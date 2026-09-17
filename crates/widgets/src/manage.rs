//! Install, update, disable and delete a widget — reversibly.
//!
//! **Reversibility is the whole design.** A widget is not just a Lua file: the
//! moment it runs it writes settings into `LuaUI/Config/BYAR.lua`, and some
//! write elsewhere as well. So an install records exactly what it put where,
//! and a delete undoes precisely that and *reports* whatever else appeared.
//! Nothing outside the ledger is removed on a guess — the alternative is a
//! delete button that occasionally eats a setting belonging to something else.
//!
//! **Installed into modlobby's own write directory.** Per the one-writable-dir
//! invariant, everything modlobby writes goes to [`Library::write_dir`]; a
//! widget installed there is visible to games modlobby launches, and not to
//! Chobby's. That is a deliberate boundary, not a limitation to work around:
//! adding to Chobby's directory is an explicit action a player takes, never a
//! side effect of clicking install here.
//!
//! **Downloads are verified with the game's own hash.** Every file carries
//! `content_hash`, base64 of its MD5, which is what `VFS.CalculateHash(data, 0)`
//! returns and what the replay telemetry reports. A file that does not match is
//! not written. This matters more than the usual integrity argument: these bytes
//! come from a third party's repository or a Discord post, and the hash is the
//! only thing tying them to the widget whose statistics the reader just read.
//!
//! **Nothing is written while a game is running.** BAR's `widgetHandler:Shutdown`
//! rewrites the config wholesale from memory when the game exits, so a config
//! edit made mid-game is silently discarded. Callers pass that state in.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::config::{ConfigError, Removed, WidgetConfig, WidgetState};
use crate::install::{Install, InstallFile};

/// Where widgets live under a write directory.
pub const WIDGETS_DIR: &str = "LuaUI/Widgets";

/// The ledger, alongside the widgets it describes.
pub const LEDGER_FILE: &str = "LuaUI/modlobby-widgets.json";

/// A widget file is a few tens of kilobytes; the largest measured is under 200.
/// This is slack, not a guess at a maximum, and it stops a misrouted response
/// being read into memory unbounded.
pub const FILE_LIMIT: usize = 4 * 1024 * 1024;

/// A hub distribution holds a widget and its assets.
pub const ARCHIVE_LIMIT: usize = 32 * 1024 * 1024;

/// Engine settings, written by the game and by widgets alike.
pub const SETTINGS_FILE: &str = "springsettings.cfg";

#[derive(Debug, thiserror::Error)]
pub enum ManageError {
    #[error("a game is running; BAR rewrites its widget config on exit and would discard this")]
    GameRunning,
    #[error("{0} cannot be installed: {1}")]
    NotInstallable(String, String),
    #[error("downloading {0}: {1}")]
    Download(String, String),
    #[error("{0} did not match its published hash and was not installed")]
    Corrupt(String),
    #[error("{0} is {1} bytes, over the limit")]
    TooLarge(String, usize),
    #[error("the archive could not be read: {0}")]
    Archive(String),
    #[error("nothing in the archive looked like a widget")]
    EmptyArchive,
    #[error("widget config: {0}")]
    Config(#[from] ConfigError),
    #[error("{0}: {1}")]
    Io(String, String),
}

/// What an install put on disk, so a delete can undo exactly it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InstalledWidget {
    /// The usage document's key, so a row can find its own record.
    pub key: String,
    /// `GetInfo().name` — how BAR's config refers to it.
    pub name: String,
    /// Paths relative to the write directory.
    pub files: Vec<String>,
    /// Base64 MD5 per file, in the same order. An update compares against
    /// these to know whether there is anything to do.
    pub hashes: Vec<String>,
    /// Where it came from, for the record and for an update.
    pub source: String,
    /// Seconds since the epoch.
    pub installed_at: u64,
    /// Engine settings keys present *before* this widget was installed.
    ///
    /// The only honest basis for attribution there is. `springsettings.cfg` is
    /// a flat list of engine settings with no record of what wrote each one, so
    /// after the fact nothing distinguishes a key this widget added from one the
    /// player set themselves. A key that was not there before the install and is
    /// there now is at least a candidate — which is worth showing, and still not
    /// worth deleting on.
    #[serde(default)]
    pub settings_before: Vec<String>,
}

impl InstalledWidget {
    /// Whether the published files differ from what is on disk.
    pub fn is_outdated(&self, install: &Install) -> bool {
        let published: Vec<&str> = install
            .files
            .iter()
            .map(|file| file.content_hash.as_str())
            .collect();
        published.is_empty() || published != self.hashes
    }
}

/// What modlobby has installed, and what BAR's own config says about it.
///
/// Two different questions, deliberately kept apart. The ledger says what
/// *modlobby* put on disk, and is what delete is allowed to act on. The config
/// says what *BAR* knows about, whoever installed it — so a widget the player
/// added by hand can still be switched off from here, and one modlobby
/// installed can still be missing from the config until the game has loaded it
/// once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct WidgetStatus {
    /// Usage keys modlobby installed, with what it wrote.
    pub installed: Vec<InstalledWidget>,
    /// Every widget named in `BYAR.lua`, ours or not.
    pub configured: Vec<WidgetState>,
    /// Whether a game is running, which is when config edits are refused.
    pub locked: bool,
    /// Where installs land, so the interface can say it rather than imply it.
    pub write_dir: String,
    /// Every widget file BAR would load, whoever put it there.
    #[serde(default)]
    pub local: Vec<crate::local::LocalWidget>,
}

/// Everything modlobby has installed, keyed by usage key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub widgets: BTreeMap<String, InstalledWidget>,
}

impl Ledger {
    pub fn read(write_dir: &Path) -> Self {
        let path = write_dir.join(LEDGER_FILE);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_else(|err| {
            // A ledger we cannot read is worse than none: it would make delete
            // claim ownership of nothing while the files stay. Say so loudly
            // and carry on with an empty one rather than failing the page.
            tracing::warn!(?path, %err, "widget ledger unreadable; treating as empty");
            Self::default()
        })
    }

    pub fn write(&self, write_dir: &Path) -> Result<(), ManageError> {
        let path = write_dir.join(LEDGER_FILE);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| ManageError::Io(parent.display().to_string(), err.to_string()))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|err| ManageError::Io(LEDGER_FILE.to_owned(), err.to_string()))?;
        std::fs::write(&path, text)
            .map_err(|err| ManageError::Io(path.display().to_string(), err.to_string()))
    }
}

/// What a delete actually did, and what it deliberately left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Deleted {
    /// Files removed, relative to the write directory.
    pub files: Vec<String>,
    /// What came out of `BYAR.lua`.
    pub config: Removed,
    /// Things this widget appears to have touched that were **not** removed,
    /// because nothing on disk records which widget wrote them.
    ///
    /// Shown, never acted on. `springsettings.cfg` keys are unattributed, so
    /// attributing one is a guess — and a guess that deletes is the kind of
    /// bug a player discovers weeks later.
    pub residue: Vec<String>,
}

/// Fetch, verify and write one widget's files.
///
/// Returns the ledger entry rather than writing it, so a caller can record the
/// install and the config change together or not at all.
pub async fn install(
    http: &reqwest::Client,
    key: &str,
    name: &str,
    install: &Install,
    write_dir: &Path,
    now: u64,
) -> Result<InstalledWidget, ManageError> {
    if !install.is_installable() {
        return Err(ManageError::NotInstallable(
            name.to_owned(),
            install
                .unavailable_because()
                .unwrap_or("no download available")
                .to_owned(),
        ));
    }
    let fetched = if install.archive {
        from_archive(http, &install.url, &install.files).await?
    } else {
        from_files(http, install).await?
    };
    if fetched.is_empty() {
        return Err(ManageError::EmptyArchive);
    }

    let dir = write_dir.join(WIDGETS_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|err| ManageError::Io(dir.display().to_string(), err.to_string()))?;

    let mut files = Vec::new();
    let mut hashes = Vec::new();
    for (file_name, data, hash) in fetched {
        let path = dir.join(&file_name);
        std::fs::write(&path, &data)
            .map_err(|err| ManageError::Io(path.display().to_string(), err.to_string()))?;
        files.push(format!("{WIDGETS_DIR}/{file_name}"));
        hashes.push(hash);
    }
    Ok(InstalledWidget {
        key: key.to_owned(),
        name: name.to_owned(),
        files,
        hashes,
        source: install.url.clone(),
        installed_at: now,
        settings_before: settings_keys(write_dir),
    })
}

/// The keys `springsettings.cfg` holds right now.
///
/// Read as keys rather than as a whole-file copy: the file is also written by
/// the engine on every run, so comparing contents would report the whole file
/// as changed every time. What matters is which settings exist.
pub fn settings_keys(write_dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(write_dir.join(SETTINGS_FILE)) else {
        return Vec::new();
    };
    let mut keys: Vec<String> = text
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
                return None;
            }
            let (key, _) = line.split_once('=')?;
            Some(key.trim().to_owned())
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Engine settings that appeared since a widget was installed.
///
/// A candidate list, not an attribution: any widget run in between could have
/// written any of these, and so could the player. It is the closest thing to
/// evidence that exists, which is why delete shows it and leaves it alone.
pub fn settings_since(entry: &InstalledWidget, write_dir: &Path) -> Vec<String> {
    let before: std::collections::BTreeSet<&str> =
        entry.settings_before.iter().map(String::as_str).collect();
    settings_keys(write_dir)
        .into_iter()
        .filter(|key| !before.contains(key.as_str()))
        .collect()
}

/// Remove a widget's files and its config entries, reporting what is left.
///
/// `residue` is whatever else names this widget in files modlobby does not own.
/// It is reported and not touched: nothing on disk says which widget wrote a
/// given `springsettings.cfg` key, so removing one is a guess.
pub fn delete(
    entry: &InstalledWidget,
    write_dir: &Path,
    config: &mut WidgetConfig,
    residue: Vec<String>,
) -> Result<Deleted, ManageError> {
    let mut removed = Vec::new();
    for relative in &entry.files {
        let path = write_dir.join(relative);
        match std::fs::remove_file(&path) {
            Ok(()) => removed.push(relative.clone()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(ManageError::Io(path.display().to_string(), err.to_string())),
        }
    }
    let config_removed = config.remove(&entry.name)?;
    Ok(Deleted {
        files: removed,
        config: config_removed,
        residue,
    })
}

type Fetched = Vec<(String, Vec<u8>, String)>;

async fn from_files(http: &reqwest::Client, install: &Install) -> Result<Fetched, ManageError> {
    let mut fetched = Vec::new();
    for file in install.downloadable_files() {
        let Some(name) = file.file_name() else {
            continue;
        };
        let data = get(http, &file.url, FILE_LIMIT).await?;
        verify(name, &data, &file.content_hash)?;
        fetched.push((name.to_owned(), data, file.content_hash.clone()));
    }
    Ok(fetched)
}

/// Unpack a hub distribution, keeping only members the document vouches for.
///
/// An archive is bytes from elsewhere containing arbitrary paths, so nothing in
/// it is trusted to say where it goes: a member is matched to a published file
/// by its *hash*, and installs under that file's name. A member the document
/// does not list is not written, whatever it claims to be.
async fn from_archive(
    http: &reqwest::Client,
    url: &str,
    files: &[InstallFile],
) -> Result<Fetched, ManageError> {
    let data = get(http, url, ARCHIVE_LIMIT).await?;
    let mut archive = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|err| ManageError::Archive(err.to_string()))?;
    let wanted: BTreeMap<&str, &InstallFile> = files
        .iter()
        .filter_map(|file| file.file_name().map(|_| (file.content_hash.as_str(), file)))
        .collect();

    let mut fetched = Vec::new();
    for index in 0..archive.len() {
        let mut member = archive
            .by_index(index)
            .map_err(|err| ManageError::Archive(err.to_string()))?;
        if !member.is_file() || member.size() as usize > FILE_LIMIT {
            continue;
        }
        let mut bytes = Vec::new();
        member
            .read_to_end(&mut bytes)
            .map_err(|err| ManageError::Archive(err.to_string()))?;
        let hash = hash_of(&bytes);
        let Some(file) = wanted.get(hash.as_str()) else {
            continue;
        };
        let Some(name) = file.file_name() else {
            continue;
        };
        fetched.push((name.to_owned(), bytes, hash));
    }
    Ok(fetched)
}

async fn get(http: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>, ManageError> {
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|err| ManageError::Download(url.to_owned(), err.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(ManageError::Download(
            url.to_owned(),
            format!("HTTP {}", status.as_u16()),
        ));
    }
    if let Some(length) = response.content_length()
        && length as usize > limit
    {
        return Err(ManageError::TooLarge(url.to_owned(), length as usize));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|err| ManageError::Download(url.to_owned(), err.to_string()))?;
    if bytes.len() > limit {
        return Err(ManageError::TooLarge(url.to_owned(), bytes.len()));
    }
    Ok(bytes.to_vec())
}

/// `VFS.CalculateHash(data, 0)`: base64 of the MD5 digest.
pub fn hash_of(data: &[u8]) -> String {
    BASE64.encode(Md5::digest(data))
}

fn verify(name: &str, data: &[u8], expected: &str) -> Result<(), ManageError> {
    // A published hash may be of the text-mode form -- BAR reads with
    // `io.open(f, "r")`, so Windows collapses CRLF before hashing -- while the
    // bytes on the server are the raw ones. Both are the same widget.
    let raw = hash_of(data);
    if raw == expected {
        return Ok(());
    }
    let unix: Vec<u8> = collapse_crlf(data);
    if unix != data && hash_of(&unix) == expected {
        return Ok(());
    }
    tracing::warn!(name, expected, got = %raw, "widget file failed verification");
    Err(ManageError::Corrupt(name.to_owned()))
}

pub(crate) fn collapse_crlf(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut at = 0;
    while at < data.len() {
        if data[at] == b'\r' && data.get(at + 1) == Some(&b'\n') {
            at += 1;
            continue;
        }
        out.push(data[at]);
        at += 1;
    }
    out
}

/// Where a widget's files would be installed, for a caller that needs the path.
pub fn widget_path(write_dir: &Path, file_name: &str) -> PathBuf {
    write_dir.join(WIDGETS_DIR).join(file_name)
}

/// Files outside modlobby's ownership that mention a widget by name.
///
/// Reported, never removed. Nothing on disk records which widget wrote a given
/// `springsettings.cfg` key — the file is a flat list of engine settings with no
/// provenance — so attributing one is a guess, and a guess that deletes is the
/// kind of bug a player discovers weeks later. Naming what was left behind lets
/// them decide; removing it on their behalf does not.
pub fn residue_of(entry: &InstalledWidget, write_dir: &Path) -> Vec<String> {
    const WATCHED: [&str; 2] = [SETTINGS_FILE, "LuaUI/Config/BYAR_ui.lua"];
    let mut found: Vec<String> = WATCHED
        .iter()
        .filter(|relative| {
            std::fs::read_to_string(write_dir.join(relative))
                .is_ok_and(|text| text.contains(&entry.name))
        })
        .map(|relative| format!("{relative} mentions this widget by name"))
        .collect();
    // Weaker evidence than a name match but wider: a widget that writes a key
    // named nothing like itself shows up here and nowhere else.
    let appeared = settings_since(entry, write_dir);
    if !appeared.is_empty() {
        found.push(format!(
            "{} engine setting{} appeared since it was installed: {}",
            appeared.len(),
            if appeared.len() == 1 { "" } else { "s" },
            appeared.join(", ")
        ));
    }
    found
}

/// Write the widget config through a temporary file in the same directory.
///
/// A half-written `BYAR.lua` is not a degraded config, it is a Lua syntax error
/// — BAR would start with every widget setting gone, and there is no copy.
/// A rename within one directory is atomic on both platforms, so the file on
/// disk is either wholly the old one or wholly the new one.
pub fn write_config(path: &Path, text: &str) -> Result<(), ManageError> {
    let io = |err: std::io::Error| ManageError::Io(path.display().to_string(), err.to_string());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let temporary = path.with_extension("lua.modlobby-new");
    std::fs::write(&temporary, text).map_err(io)?;
    std::fs::rename(&temporary, path).map_err(io)
}

/// Read the widget config, or `None` when BAR has not written one yet.
pub fn read_config(write_dir: &Path) -> Option<WidgetConfig> {
    std::fs::read_to_string(write_dir.join(crate::config::CONFIG_PATH))
        .ok()
        .map(WidgetConfig::parse)
}

/// Whether a widget in the ledger still has every file it was installed with.
///
/// A file removed by hand is not an error, but it does mean the ledger no
/// longer describes the disk, and an update is the honest answer rather than a
/// delete that reports files it never found.
pub fn is_intact(entry: &InstalledWidget, write_dir: &Path) -> bool {
    entry
        .files
        .iter()
        .all(|relative| write_dir.join(relative).exists())
}
