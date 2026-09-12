//! Keeping the app current, without getting in the way of opening it.
//!
//! Looking and installing are two steps with a click between them. The look
//! is one small request for the release manifest, made once a day when the
//! app opens (if the setting allows) or whenever the version in the corner of
//! the nav is clicked; it downloads nothing. A newer version found becomes
//! that corner's offer. Taking the offer downloads the installer and installs
//! it at once, unless a room is joined or a game is running, in which case
//! the download waits as [`Pending::Downloaded`] and the corner offers the
//! restart instead.
//!
//! A download that waits is also kept on disk, under `updates/` beside the
//! settings, so closing the app does not throw it away: the next start finds
//! it as [`Pending::Stored`], confirms with the manifest that it is still the
//! release to install, and installs it before logging in — a fresh look, not
//! a fresh download. The manifest is asked again because the installer's
//! signature lives there, and `tauri-plugin-updater` verifies against it.
//!
//! The installer does the restart: on Windows `install` hands over to NSIS
//! and exits this process, and NSIS relaunches the app with the arguments it
//! had. On Linux the AppImage is rewritten in place and `install` returns, so
//! the restart is asked for here. Nothing runs after a successful install.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::commands::{ApiError, Result};
use crate::state::App;

/// What this build is, for the corner of the nav.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VersionView {
    /// The number the updater compares: `Cargo.toml`'s, via `CARGO_PKG_VERSION`.
    pub version: &'static str,
    /// The short commit hash, stamped by `build.rs`.
    pub commit: &'static str,
    /// Whether the engine here may be run against somebody's hosted game.
    ///
    /// False on macOS, where the only engine that exists is a third-party
    /// build its author asks not be used on the community servers. Talking in
    /// a room costs those servers nothing and is left alone; playing is what
    /// stops. This is the front end's copy of the answer, so it can draw a
    /// room you can watch rather than one whose buttons all fail —
    /// `recoil::refuse_target` is what actually enforces it.
    pub plays_online: bool,
    /// Why no engine can be fetched onto this machine, when none can.
    ///
    /// `None` wherever there is an engine to fetch, which since the Apple
    /// Silicon build is downloaded rather than installed by hand is everywhere
    /// but an Intel Mac. Where it is a sentence, it is the same fact
    /// `download_engine` refuses with, carried here so the room can decline to
    /// offer the download rather than offer it and be told.
    ///
    /// The reason rather than a `bool`, so nothing can draw the refusal
    /// without the words that explain it, and so the sentence is written once
    /// instead of once per language.
    ///
    /// Not derived from `plays_online`: they are two facts with different
    /// causes, and macOS is where they come apart — the engine is fetchable
    /// there and the community servers are still closed to it.
    pub no_published_engine: Option<&'static str>,
    /// Where the engine comes from, when it is not Beyond All Reason's own.
    ///
    /// `None` everywhere BAR publishes a build, and on macOS the one sentence
    /// somebody should read before a third party's binary is downloaded onto
    /// their machine and made executable: whose it is, that it is unaffiliated,
    /// and that the community servers are not part of what it can do.
    ///
    /// Shown beside the offer rather than behind it. An automatic install is
    /// the point — that is what this whole path is for — and the honest way to
    /// have both is to say what is being installed while it installs, not to
    /// put a dialog in front of a person who has already asked for a game.
    pub third_party_engine: Option<&'static str>,
}

#[tauri::command]
pub fn app_version() -> VersionView {
    VersionView {
        version: env!("CARGO_PKG_VERSION"),
        commit: env!("MODLOBBY_COMMIT"),
        plays_online: recoil::may_join_hosted_games(),
        no_published_engine: content::release::no_published_engine(),
        third_party_engine: content::release::third_party_engine(),
    }
}

/// How far along an update is. Emitted on the `app-update` event.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "phase"
)]
#[ts(export)]
pub enum UpdateProgress {
    Checking,
    UpToDate,
    /// A newer release exists and nothing has been fetched: the corner's offer
    /// to download and install it.
    Available {
        version: String,
    },
    Downloading {
        #[ts(type = "number")]
        got: u64,
        #[ts(type = "number")]
        total: u64,
    },
    /// Downloaded and waiting. It installs on the next start; the corner
    /// offers the restart before then, and says what stands in its way.
    Ready {
        version: String,
        /// What restarting now would take away — a room, a running game —
        /// while there is something; `None` once a click would install.
        held_by: Option<String>,
    },
    Failed {
        reason: String,
    },
}

/// An update the look found, and what has been done about it so far.
enum Pending {
    Found(Update),
    Downloaded(Update, Vec<u8>),
    /// An earlier run's download, on disk. No `Update` yet: the manifest has
    /// to be asked again for one before it can be installed.
    Stored {
        version: String,
        path: PathBuf,
    },
}

/// The update between the look and the install, and the directory a
/// download waits in between runs.
pub struct Staged {
    dir: PathBuf,
    held: Mutex<Option<Pending>>,
}

/// What a kept download is called: the version, so a start can tell whether
/// it is still worth anything without opening it.
const STORED_PREFIX: &str = "modlobby-";
const STORED_SUFFIX: &str = ".update";

impl Staged {
    /// Opens `updates/` under the config directory and picks up a download an
    /// earlier run left there. One for the version running now is what that
    /// run installed, and is removed; anything else waits for the manifest
    /// to say whether it is still the release to install.
    pub fn open(config_dir: &Path) -> Self {
        Self::open_as(config_dir, env!("CARGO_PKG_VERSION"))
    }

    fn open_as(config_dir: &Path, running: &str) -> Self {
        let dir = config_dir.join("updates");
        let mut stored = None;
        for path in stored_files(&dir) {
            let version = stored_version(&path);
            if version == running || stored.is_some() {
                remove(&path);
                continue;
            }
            tracing::info!(version, path = %path.display(), "update kept from an earlier run");
            stored = Some(Pending::Stored { version, path });
        }
        Self {
            dir,
            held: Mutex::new(stored),
        }
    }

    /// The version of a download waiting on disk, if one is.
    pub fn stored_version(&self) -> Option<String> {
        match self.held.lock().expect("staged update").as_ref() {
            Some(Pending::Stored { version, .. }) => Some(version.clone()),
            _ => None,
        }
    }

    /// Keeps a download for a later run. Losable: a download that cannot be
    /// written is still installable from memory this run, and is fetched
    /// again next time, which is what happened before anything was kept.
    fn keep(&self, version: &str, bytes: &[u8]) {
        let path = self.dir.join(format!(
            "{STORED_PREFIX}{}{STORED_SUFFIX}",
            version.replace(['/', '\\'], "_")
        ));
        let tmp = path.with_extension("part");
        let written = std::fs::create_dir_all(&self.dir)
            .and_then(|()| std::fs::write(&tmp, bytes))
            .and_then(|()| std::fs::rename(&tmp, &path));
        match written {
            Ok(()) => {
                tracing::info!(version, path = %path.display(), "update kept for the next start")
            }
            Err(err) => tracing::warn!(%err, path = %path.display(), "could not keep the update"),
        }
    }

    /// Removes every kept download: installed, or no longer the release.
    fn discard(&self) {
        for path in stored_files(&self.dir) {
            remove(&path);
        }
    }
}

fn stored_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(STORED_PREFIX) && name.ends_with(STORED_SUFFIX)
                })
        })
        .collect();
    paths.sort();
    paths
}

fn stored_version(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(STORED_PREFIX))
        .and_then(|name| name.strip_suffix(STORED_SUFFIX))
        .unwrap_or_default()
        .to_owned()
}

fn remove(path: &Path) {
    if let Err(err) = std::fs::remove_file(path) {
        tracing::warn!(%err, path = %path.display(), "could not remove a kept update");
    }
}

/// How often the app looks on its own.
pub const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Whether this build may update itself. Unset means yes in a release and no
/// in a `tauri dev` run, which is always a local build the released one would
/// replace; `0`, `false`, `off` or `no` means no look at all, anything else
/// means look, and the on-demand look says why it will not. For a build that
/// has to stay put — a local one behind the released version, or one under
/// test — or, set on, for a dev run testing the update round trip.
pub const AUTO_UPDATE_ENV: &str = "MODLOBBY_AUTO_UPDATE";

pub fn enabled() -> bool {
    allows(std::env::var_os(AUTO_UPDATE_ENV), !tauri::is_dev())
}

fn allows(value: Option<std::ffi::OsString>, unset: bool) -> bool {
    let Some(value) = value else {
        return unset;
    };
    let value = value.to_string_lossy().trim().to_ascii_lowercase();
    !matches!(value.as_str(), "0" | "false" | "off" | "no")
}

/// How much has to arrive before the front end is told again. An installer
/// is tens of megabytes, so a megabyte is a visible step.
const REPORT_EVERY: u64 = 1024 * 1024;

fn disabled() -> ApiError {
    ApiError::new(
        "update",
        format!("updates are off: {AUTO_UPDATE_ENV} says so, or is unset in a dev run"),
    )
}

/// Looks for a newer release. Downloads nothing: the answer is `Available`
/// with the version, `UpToDate`, or `Ready` when that version has already
/// been downloaded and is waiting for a restart. A completed look is
/// remembered so the daily one knows when it is due.
#[tauri::command]
pub async fn check_update(
    app: State<'_, App>,
    staged: State<'_, Staged>,
    handle: AppHandle,
) -> Result<UpdateProgress> {
    if !enabled() {
        return Err(disabled());
    }

    let say = |progress: UpdateProgress| {
        let _ = handle.emit("app-update", progress);
    };
    say(UpdateProgress::Checking);

    let outcome = match look(&handle).await {
        Ok(found) => {
            app.update_memory.record(SystemTime::now());
            let held_by = busy(&app).await;
            Ok(settle(&staged, found, held_by))
        }
        Err(err) => Err(err),
    };

    match &outcome {
        Ok(progress) => say(progress.clone()),
        Err(err) => say(UpdateProgress::Failed {
            reason: err.message.clone(),
        }),
    }
    outcome
}

/// Reconciles what the manifest says with what is held: the same version
/// already downloaded stays downloaded and is `Ready`; anything else the
/// look found replaces it as `Available`; nothing found clears it.
fn settle(staged: &Staged, found: Option<Update>, held_by: Option<&str>) -> UpdateProgress {
    let mut held = staged.held.lock().expect("staged update");
    match (found, held.take()) {
        (None, _) => {
            staged.discard();
            UpdateProgress::UpToDate
        }
        (Some(update), Some(Pending::Downloaded(done, bytes)))
            if done.version == update.version =>
        {
            let version = done.version.clone();
            *held = Some(Pending::Downloaded(done, bytes));
            ready(version, held_by)
        }
        (Some(update), Some(Pending::Stored { version, path })) if version == update.version => {
            *held = Some(Pending::Stored {
                version: version.clone(),
                path,
            });
            ready(version, held_by)
        }
        (Some(update), _) => {
            // Whatever was kept is not this release.
            staged.discard();
            let version = update.version.clone();
            *held = Some(Pending::Found(update));
            UpdateProgress::Available { version }
        }
    }
}

fn ready(version: String, held_by: Option<&str>) -> UpdateProgress {
    UpdateProgress::Ready {
        version,
        held_by: held_by.map(str::to_owned),
    }
}

/// Takes the corner's offer: downloads what the look found and installs it,
/// or stages it as `Ready` when a room or a game would be lost. Installs at
/// once what an earlier click — or an earlier run — already downloaded.
/// Returns only when there is nothing to install: a successful install ends
/// the process.
#[tauri::command]
pub async fn install_update(
    app: State<'_, App>,
    staged: State<'_, Staged>,
    handle: AppHandle,
) -> Result<UpdateProgress> {
    let taken = staged.held.lock().expect("staged update").take();
    let Some(pending) = taken else {
        return Err(ApiError::new(
            "update",
            "no update has been found; look for one first",
        ));
    };

    let say = |progress: UpdateProgress| {
        let _ = handle.emit("app-update", progress);
    };

    let (update, bytes) = match pending {
        Pending::Downloaded(update, bytes) => (update, bytes),
        Pending::Found(update) => match download(&update, &say).await {
            Ok(bytes) => {
                staged.keep(&update.version, &bytes);
                (update, bytes)
            }
            Err(err) => {
                // Still found, still on offer; the next click tries again.
                *staged.held.lock().expect("staged update") = Some(Pending::Found(update));
                say(UpdateProgress::Failed {
                    reason: err.message.clone(),
                });
                return Err(err);
            }
        },
        Pending::Stored { version, path } => match reopen(&staged, &handle, version, path).await {
            Ok(Reopened::Installable(update, bytes)) => (*update, bytes),
            Ok(Reopened::Otherwise(progress)) => {
                say(progress.clone());
                return Ok(progress);
            }
            Err(err) => {
                say(UpdateProgress::Failed {
                    reason: err.message.clone(),
                });
                return Err(err);
            }
        },
    };

    if let Some(held_by) = busy(&app).await {
        let version = update.version.clone();
        *staged.held.lock().expect("staged update") = Some(Pending::Downloaded(update, bytes));
        let progress = ready(version, Some(held_by));
        say(progress.clone());
        return Ok(progress);
    }

    let outcome = install(&handle, &staged, &update, &bytes);
    if let Err(err) = &outcome {
        say(UpdateProgress::Failed {
            reason: err.message.clone(),
        });
    }
    outcome
}

/// Installs the download an earlier run kept, before this one logs in.
/// `None` when nothing was kept, or when this build does not update itself;
/// otherwise what `install_update` answers when it does not install — the
/// manifest moved on, or could not be reached — so the front end can carry
/// on with the login.
#[tauri::command]
pub async fn resume_update(
    app: State<'_, App>,
    staged: State<'_, Staged>,
    handle: AppHandle,
) -> Result<Option<UpdateProgress>> {
    if !enabled() || staged.stored_version().is_none() {
        return Ok(None);
    }
    install_update(app, staged, handle).await.map(Some)
}

enum Reopened {
    /// Boxed for the size: an `Update` carries the whole manifest response.
    Installable(Box<Update>, Vec<u8>),
    Otherwise(UpdateProgress),
}

/// Turns a kept download back into something installable: the manifest for
/// the `Update` (and the signature in it), the file for the bytes. A
/// manifest that has moved on makes the kept file worthless, and a file
/// that cannot be read is fetched again as if never kept.
async fn reopen(
    staged: &Staged,
    handle: &AppHandle,
    version: String,
    path: PathBuf,
) -> Result<Reopened> {
    let found = match look(handle).await {
        Ok(found) => found,
        Err(err) => {
            // Offline, most likely: the file keeps waiting.
            *staged.held.lock().expect("staged update") = Some(Pending::Stored { version, path });
            return Err(err);
        }
    };
    let Some(update) = found else {
        staged.discard();
        return Ok(Reopened::Otherwise(UpdateProgress::UpToDate));
    };
    if update.version != version {
        staged.discard();
        let version = update.version.clone();
        *staged.held.lock().expect("staged update") = Some(Pending::Found(update));
        return Ok(Reopened::Otherwise(UpdateProgress::Available { version }));
    }
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Reopened::Installable(Box::new(update), bytes)),
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "kept update unreadable; fetching again");
            staged.discard();
            let version = update.version.clone();
            *staged.held.lock().expect("staged update") = Some(Pending::Found(update));
            Ok(Reopened::Otherwise(UpdateProgress::Available { version }))
        }
    }
}

/// The daily look, when it is due. Quiet about being offline: an update is
/// not something to be told about failing to look for. Not while a download
/// waits on disk: the start that found it is installing it.
pub async fn daily(handle: AppHandle) {
    let app = handle.state::<App>();
    let staged = handle.state::<Staged>();
    if staged.stored_version().is_some() {
        tracing::debug!("update check: a kept download is being resumed, not looking");
        return;
    }
    if !app.update_memory.due(SystemTime::now(), EVERY) {
        tracing::debug!("update check: looked within the day, not again");
        return;
    }
    match check_update(app, staged, handle.clone()).await {
        Ok(progress) => tracing::info!(?progress, "update check"),
        Err(err) => tracing::info!(reason = %err.message, "update check"),
    }
}

/// The release manifest, compared with this build. One small request.
async fn look(handle: &AppHandle) -> Result<Option<Update>> {
    let updater = handle
        .updater()
        .map_err(|err| ApiError::new("update", err.to_string()))?;
    updater
        .check()
        .await
        .map_err(|err| ApiError::new("update", format!("looking for a release: {err}")))
}

async fn download(update: &Update, say: &impl Fn(UpdateProgress)) -> Result<Vec<u8>> {
    let mut got = 0_u64;
    let mut reported = 0_u64;
    say(UpdateProgress::Downloading { got: 0, total: 0 });
    update
        .download(
            |chunk, total| {
                got += chunk as u64;
                let total = total.unwrap_or(0);
                if got - reported >= REPORT_EVERY {
                    reported = got;
                    say(UpdateProgress::Downloading { got, total });
                }
            },
            || {},
        )
        .await
        .map_err(|err| ApiError::new("update", format!("fetching {}: {err}", update.version)))
}

/// What restarting now would take away: a room we are in, a game that is
/// running, or an engine we launched that is still alive. `None` when
/// nothing would be lost. A runtime that cannot answer has nothing to lose.
async fn busy(app: &App) -> Option<&'static str> {
    if let Ok(snapshot) = app.client.snapshot().await {
        if snapshot.game_running.is_some() {
            return Some("the game that is running");
        }
        if snapshot.my_battle.is_some() {
            return Some("the room you are in");
        }
    }
    matches!(app.client.engine_pid().await, Ok(Some(_)))
        .then_some("the engine that is still running")
}

/// Hands the installer its bytes. Does not return on success: the process
/// exits and the new build comes up in its place.
fn install(
    handle: &AppHandle,
    staged: &Staged,
    update: &Update,
    bytes: &[u8],
) -> Result<UpdateProgress> {
    // The exit that follows is not Tauri's, so the exit handler that takes the
    // in-game widget back out of the user's data directory will not run.
    if let Some(held) = handle.try_state::<crate::InGameHandle>() {
        drop(held.lock().expect("in-game").take());
    }
    update
        .install(bytes)
        .map_err(|err| ApiError::new("update", format!("installing {}: {err}", update.version)))?;
    // NSIS has exited this process by now; the kept file is for the next
    // start to recognise as its own version and remove. The AppImage was
    // rewritten under our feet and nothing relaunches anything, so that is
    // done here, and the kept file has served.
    #[cfg(not(windows))]
    {
        staged.discard();
        handle.restart();
    }
    #[cfg(windows)]
    {
        let _ = staged;
        Ok(UpdateProgress::Ready {
            version: update.version.clone(),
            held_by: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Staged, allows};

    /// The window's copy of where the engine comes from cannot drift from the
    /// runtime's, on the one platform where either answer is the uncommon one
    /// — which is the only platform nobody develops on.
    ///
    /// The pairing is the thing being asserted, not the values: a build that
    /// can fetch an engine must not also carry a sentence saying it cannot,
    /// and one that fetches a third party's must carry the sentence saying so.
    #[test]
    fn the_window_is_told_the_same_source_the_runtime_would_use() {
        let view = super::app_version();
        let source = content::release::source();
        assert_eq!(view.no_published_engine, source.unavailable());
        assert_eq!(view.third_party_engine, source.third_party());
        assert!(
            view.no_published_engine.is_none() || view.third_party_engine.is_none(),
            "a machine with nothing to fetch has no provenance to show for it"
        );
    }

    #[test]
    fn unset_takes_the_build_default() {
        assert!(allows(None, true));
        assert!(!allows(None, false));
    }

    #[test]
    fn anything_but_the_off_words_means_on_even_in_a_dev_run() {
        for value in ["1", "true", "on", "yes", "", "whatever"] {
            assert!(allows(Some(value.into()), false), "{value:?}");
        }
    }

    #[test]
    fn the_four_off_words_mean_off_in_any_case() {
        for value in ["0", "false", "off", "no", " OFF ", "False"] {
            assert!(!allows(Some(value.into()), true), "{value:?}");
        }
    }

    #[test]
    fn a_kept_download_is_found_by_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let staged = Staged::open_as(dir.path(), "0.1.10");
        assert_eq!(staged.stored_version(), None, "nothing kept yet");

        staged.keep("0.1.11", b"installer");
        let restarted = Staged::open_as(dir.path(), "0.1.10");
        assert_eq!(restarted.stored_version(), Some("0.1.11".into()));
        assert!(!dir.path().join("updates/modlobby-0.1.11.part").exists());
    }

    #[test]
    fn the_start_that_installed_it_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        Staged::open_as(dir.path(), "0.1.10").keep("0.1.11", b"installer");
        let updated = Staged::open_as(dir.path(), "0.1.11");
        assert_eq!(updated.stored_version(), None);
        assert!(!dir.path().join("updates/modlobby-0.1.11.update").exists());
    }

    #[test]
    fn discarding_leaves_nothing_for_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let staged = Staged::open_as(dir.path(), "0.1.10");
        staged.keep("0.1.11", b"installer");
        staged.discard();
        assert_eq!(Staged::open_as(dir.path(), "0.1.10").stored_version(), None);
    }

    #[test]
    fn a_name_that_is_not_a_kept_download_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let updates = dir.path().join("updates");
        std::fs::create_dir_all(&updates).unwrap();
        std::fs::write(updates.join("notes.txt"), "mine").unwrap();
        let staged = Staged::open_as(dir.path(), "0.1.10");
        assert_eq!(staged.stored_version(), None);
        staged.discard();
        assert!(updates.join("notes.txt").exists());
    }
}
