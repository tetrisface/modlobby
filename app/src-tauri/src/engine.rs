//! Getting an engine onto a machine that has none.
//!
//! Everything else modlobby fetches goes through pr-downloader, which is the
//! right tool and handles rapid, mirrors and resume. It cannot fetch an engine,
//! because it ships inside one — so this is the one download modlobby does
//! itself, and only ever the first one.
//!
//! It is done the way pr-downloader would do it: the checksum whoever
//! published the archive gave for it is verified before anything is unpacked,
//! a download that broke off is resumed rather than restarted, and every
//! mirror named gets its turn.
//!
//! After it, `recoil::find_downloader` has something to find and the ordinary
//! content path takes over.
//!
//! # Two sources, one download
//!
//! `content::release::Source` is what decides where the archive comes from,
//! and the two are asked different questions:
//!
//! - **BAR's file index** is asked for *a named version*, because every
//!   version is in it. It answers with mirrors, a size and an md5.
//! - **The Apple Silicon port** is asked for *the current release*, because a
//!   port release carries whichever engine its author built against and there
//!   is no index to look a version up in. It answers with one asset, a size
//!   and a sha256, and the engine version is read out of the bundle after it
//!   is unpacked.
//!
//! So the version a caller asks for is a request on one path and a hope on the
//! other, and [`Installed`] carries back the version that actually arrived
//! rather than the one that was asked for. Everything between the two — the
//! resume, the mirror loop, the staging file, the progress — is shared, since
//! none of it cares what published the bytes.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use md5::{Digest, Md5};
use serde::Serialize;
use sha2::Sha256;
use tauri::{Emitter, State};
use tokio::io::AsyncWriteExt;

use crate::commands::{ApiError, Result};
use crate::state::App;

/// How far along the one download this module does is.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", tag = "phase")]
#[ts(export)]
pub enum EngineProgress {
    /// Asking where this engine lives: BAR's file index, or the Apple Silicon
    /// port's releases. One small request either way, and often the whole of
    /// it -- a port release already installed is recognised here.
    Finding,
    Downloading {
        #[ts(type = "number")]
        got: u64,
        #[ts(type = "number")]
        total: u64,
    },
    /// The archive is in hand; checking and unpacking it is not interruptible
    /// and can take a while, so it is worth saying that it is happening.
    Extracting,
    Done {
        version: String,
    },
    Failed {
        reason: String,
    },
}

/// An engine that is now on disk, and which one it turned out to be.
///
/// The version is carried rather than assumed to be the one asked for: on the
/// Apple Silicon path the request is "the current build" and the answer can
/// only be read out of the bundle afterwards. Reporting the requested version
/// there would tell a room its engine had arrived while the room went on
/// saying it was missing — the one failure that looks like the download being
/// broken when it worked.
#[derive(Debug, Clone)]
struct Installed {
    /// Where the binaries are: the engine directory, or a bundle's `MacOS`.
    bin: PathBuf,
    version: String,
}

/// Downloads and unpacks an engine into `<data>/engine/`.
///
/// Progress arrives on the `engine-download` event rather than as a return
/// value: it is a hundreds-of-megabytes download and a silent one would look
/// like a hang.
#[tauri::command]
pub async fn download_engine(
    app: State<'_, App>,
    window: tauri::Window,
    version: String,
) -> Result<String> {
    let dirs = crate::commands::data_dirs(&app)?;
    let say = |progress: EngineProgress| {
        let _ = window.emit("engine-download", progress);
    };

    // One engine download at a time: a room asking twice, or two views asking
    // for the same version, would otherwise write the same staging file. The
    // second caller finds the engine installed and returns at once.
    let _one_at_a_time = app.engine_downloads.lock().await;
    match fetch(&app.http, &dirs, &version, &say).await {
        Ok(installed) => {
            // The version that arrived, which on the Apple path need not be
            // the one that was asked for.
            say(EngineProgress::Done {
                version: installed.version,
            });
            // A room waiting on this engine can now fetch its game and map.
            let _ = app.client.recheck_content().await;
            Ok(installed.bin.to_string_lossy().into_owned())
        }
        Err(err) => {
            say(EngineProgress::Failed {
                reason: err.message.clone(),
            });
            Err(err)
        }
    }
}

/// How long a staging file nobody is writing to is kept for resuming. A
/// download broken off last week is worth finishing; one from last month is
/// more likely an engine nobody wants any more.
const STALE_PART_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);

async fn fetch(
    http: &reqwest::Client,
    dirs: &content::DataDirs,
    version: &str,
    say: &impl Fn(EngineProgress),
) -> Result<Installed> {
    // Already there, ours or another lobby's: not an error, and not a reason
    // to download it again.
    let library = content::Library::new(dirs.clone());
    if let Some(found) = library.find_engine(version) {
        return Ok(Installed {
            bin: found.bin,
            version: version.to_owned(),
        });
    }
    let data_dir = library.write_dir().to_path_buf();
    let engine_dir = data_dir.join("engine");
    std::fs::create_dir_all(&engine_dir)
        .map_err(|err| ApiError::new("io", format!("making the engine directory: {err}")))?;
    sweep_stale_parts(&engine_dir, STALE_PART_AFTER);

    match content::release::source() {
        // A machine that will never have an engine, said before a byte moves.
        content::release::Source::Nowhere(why) => Err(ApiError::new("platform", why)),
        content::release::Source::AppleSilicon => {
            fetch_apple(http, &library, &engine_dir, say).await
        }
        content::release::Source::Index(_) => {
            fetch_from_index(http, &data_dir, &engine_dir, version, say).await
        }
    }
}

/// The engine from BAR's own file index, by version.
async fn fetch_from_index(
    http: &reqwest::Client,
    data_dir: &Path,
    engine_dir: &Path,
    version: &str,
    say: &impl Fn(EngineProgress),
) -> Result<Installed> {
    // No index entry to ask for is not a network problem, and saying so early
    // beats downloading something that cannot run here.
    let find =
        content::release::find_url(version).map_err(|no| ApiError::new(no.code(), no.reason()))?;

    say(EngineProgress::Finding);
    let index = http
        .get(find)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|err| ApiError::new("network", format!("asking BAR's file index: {err}")))?
        .text()
        .await
        .map_err(|err| ApiError::new("network", format!("reading the index answer: {err}")))?;

    let release = content::release::pick(&index).ok_or_else(|| {
        ApiError::new(
            "notFound",
            format!(
                "BAR's index has no {} build of engine {version}",
                content::release::category().unwrap_or("matching")
            ),
        )
    })?;

    // Into a staging file, because the extractor wants a path and a
    // half-written archive under `engine/` would look like an install. Dotted
    // and suffixed so nothing scanning for engines mistakes it for one.
    let staging = engine_dir.join(format!(".{}.part", release.filename));
    let target = engine_dir.join(version);

    // A transport failure leaves the staging file for the next attempt to
    // resume; a checksum failure has already removed it.
    download(
        http,
        &release.mirrors,
        release.size,
        Checksum::Md5(release.md5.as_deref()),
        &staging,
        say,
    )
    .await?;

    say(EngineProgress::Extracting);
    let unpacked = tokio::task::spawn_blocking({
        let staging = staging.clone();
        let target = target.clone();
        move || sevenz_rust2::decompress_file(&staging, &target)
    })
    .await
    .map_err(|err| ApiError::new("io", format!("unpacking: {err}")))?;

    let _ = std::fs::remove_file(&staging);
    unpacked.map_err(|err| {
        // A half-unpacked directory is worse than none: it would satisfy a
        // "which engines are installed" scan and then fail to launch.
        let _ = std::fs::remove_dir_all(&target);
        ApiError::new("archive", format!("unpacking the engine: {err}"))
    })?;
    recoil::mark_executable(&target)
        .map_err(|err| ApiError::new("io", format!("marking the engine executable: {err}")))?;

    recoil::find_engine(data_dir, version)
        .map(|engine| Installed {
            bin: engine.bin,
            version: version.to_owned(),
        })
        .ok_or_else(|| {
            ApiError::new(
                "archive",
                format!(
                    "the archive unpacked but holds no {}",
                    recoil::ENGINE_BINARY
                ),
            )
        })
}

/// The unofficial Apple Silicon build, from its own releases.
///
/// The current release rather than a named version — see the module note — and
/// the current release is asked about *first*, because the answer is often
/// "you already have it". A port release that only changes the graphics driver
/// carries the engine before it, so a room asking for an engine version the
/// port has not reached yet would otherwise fetch seventy-five megabytes on
/// every ask and install something already on disk. The bundle records which
/// port built it, so that question costs one small request.
async fn fetch_apple(
    http: &reqwest::Client,
    library: &content::Library,
    engine_dir: &Path,
    say: &impl Fn(EngineProgress),
) -> Result<Installed> {
    say(EngineProgress::Finding);
    let body = http
        .get(content::apple::LATEST_RELEASE_URL)
        // GitHub's own versioned media type: the API is asked for the shape
        // this code was written against rather than for whatever is current.
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|err| {
            ApiError::new(
                "network",
                format!("asking for the current Apple Silicon build: {err}"),
            )
        })?
        .text()
        .await
        .map_err(|err| ApiError::new("network", format!("reading the releases answer: {err}")))?;

    let build = content::apple::pick(&body)
        .ok_or_else(|| ApiError::new("notFound", content::apple::NO_ASSET))?;

    // The one already installed, which is the usual answer for everything but
    // the first run.
    if let Some(found) = library.find_port(&build.port_version) {
        let version = found.declared_version().unwrap_or_default();
        tracing::info!(
            port = build.port_version,
            version,
            "the current Apple Silicon build is already installed"
        );
        return Ok(Installed {
            bin: found.bin,
            version,
        });
    }

    let staging = engine_dir.join(format!(".{}.part", build.filename));
    download(
        http,
        std::slice::from_ref(&build.url),
        build.size,
        Checksum::Sha256(build.sha256.as_deref()),
        &staging,
        say,
    )
    .await?;

    say(EngineProgress::Extracting);
    // Unpacked beside the installs rather than into its place among them: an
    // `.app` half out of a zip is still an `.app` with an engine in it, and a
    // scan finding one would report an engine that cannot start. Dotted, like
    // the staging file, so nothing scanning for engines looks inside it.
    let unpacking = engine_dir.join(format!(".{}.unpacking", build.port_version));
    let _ = std::fs::remove_dir_all(&unpacking);
    let unpacked = tokio::task::spawn_blocking({
        let staging = staging.clone();
        let unpacking = unpacking.clone();
        move || content::apple::unpack(&staging, &unpacking)
    })
    .await
    .map_err(|err| ApiError::new("io", format!("unpacking: {err}")))?;

    let _ = std::fs::remove_file(&staging);
    if let Err(err) = unpacked {
        let _ = std::fs::remove_dir_all(&unpacking);
        return Err(ApiError::new("archive", err.to_string()));
    }

    // Ad-hoc signed, and Gatekeeper's answer to that is a dialog about an
    // unidentified developer. It should not apply to an archive this process
    // fetched and extracted itself, and it is cleared rather than assumed.
    content::apple::clear_quarantine(&unpacking);

    // Only now is there something to read the engine version out of.
    let layout = recoil::EngineLayout::at(&unpacking).ok_or_else(|| {
        let _ = std::fs::remove_dir_all(&unpacking);
        ApiError::new(
            "archive",
            format!(
                "the archive unpacked but holds no {}",
                recoil::ENGINE_BINARY
            ),
        )
    })?;
    let version = layout.declared_version();
    let target = place(
        engine_dir,
        &build.install_name(version.as_deref()),
        &version,
    );
    std::fs::rename(&unpacking, &target).map_err(|err| {
        let _ = std::fs::remove_dir_all(&unpacking);
        ApiError::new("io", format!("putting the engine in place: {err}"))
    })?;

    recoil::EngineLayout::at(&target)
        .map(|engine| Installed {
            bin: engine.bin,
            // A bundle that stopped declaring its engine version is still an
            // engine; it is found by what is inside it either way, and the
            // directory name is what a person would then read it off.
            version: version.unwrap_or_else(|| build.port_version.clone()),
        })
        .ok_or_else(|| {
            ApiError::new(
                "archive",
                format!(
                    "the engine did not survive being moved into {}",
                    target.display()
                ),
            )
        })
}

/// Where to put a freshly unpacked bundle, given what it wants to be called.
///
/// Almost always the name itself. The exception is a port release that carries
/// the engine version one already installed does: `recoil_2026.07.04` is then
/// taken, by an older build of the same engine. Replacing that is the right
/// answer — it is a newer build of the thing in it — but only when what is
/// there really is a bundle of that engine and not, say, a directory somebody
/// assembled by hand. Anything else keeps its place and the new one goes in
/// beside it under a name of its own.
fn place(engine_dir: &Path, name: &str, version: &Option<String>) -> PathBuf {
    let target = engine_dir.join(name);
    let Some(existing) = recoil::EngineLayout::at(&target) else {
        // Nothing there, or nothing with an engine in it.
        let _ = std::fs::remove_dir_all(&target);
        return target;
    };
    let same_engine = version.is_some() && existing.declared_version() == *version;
    if existing.bundled() && same_engine {
        tracing::info!(
            path = %target.display(),
            "replacing an older build of the same engine"
        );
        let _ = std::fs::remove_dir_all(&target);
        return target;
    }
    let beside = engine_dir.join(format!("{name}-{}", uniquely()));
    tracing::info!(
        path = %beside.display(),
        "an engine is already installed under {name}; the new one goes beside it"
    );
    beside
}

/// Enough to tell two installs apart in a directory listing, without a clock
/// dependency for one name.
fn uniquely() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

/// What whoever published the archive says it should hash to.
///
/// BAR's index gives an md5, GitHub a sha256; both are absent on old enough
/// entries. One type rather than two `Option<&str>` parameters, because
/// "which of these two is set" is exactly the question a call site would get
/// wrong once and never notice — an unverified download is silent.
#[derive(Debug, Clone, Copy)]
enum Checksum<'a> {
    Md5(Option<&'a str>),
    Sha256(Option<&'a str>),
}

/// Which hash, with the expected value left behind.
///
/// The reading loop runs on a blocking thread, which borrows nothing; this is
/// the part of a [`Checksum`] that can go there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hash {
    Md5,
    Sha256,
}

impl Hash {
    const fn name(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha256 => "sha256",
        }
    }

    /// The archive's hash, read in chunks: an engine is hundreds of megabytes
    /// and reading one into memory to hash it is hundreds of megabytes of
    /// memory on the machine least likely to have them spare.
    fn of(self, path: &Path) -> std::io::Result<String> {
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        let mut md5 = Md5::new();
        let mut sha256 = Sha256::new();
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            match self {
                Self::Md5 => md5.update(&chunk[..read]),
                Self::Sha256 => sha256.update(&chunk[..read]),
            }
        }
        Ok(match self {
            Self::Md5 => hex(&md5.finalize()),
            Self::Sha256 => hex(&sha256.finalize()),
        })
    }
}

/// Tries every mirror, in its order, and verifies what arrived.
async fn download(
    http: &reqwest::Client,
    mirrors: &[String],
    size: u64,
    sum: Checksum<'_>,
    into: &Path,
    say: &impl Fn(EngineProgress),
) -> Result<()> {
    let mut last = None;
    for mirror in mirrors {
        match download_from(http, mirror, into, size, say).await {
            Ok(()) => return verify(into, sum).await,
            Err(err) => {
                tracing::warn!(mirror, reason = %err.message, "engine mirror failed");
                last = Some(err);
            }
        }
    }
    Err(last.unwrap_or_else(|| ApiError::new("notFound", "nothing named a mirror to fetch from")))
}

/// How much has to arrive before the front end is told again.
///
/// Chunks arrive in tens of kilobytes, so one event each would be tens of
/// thousands of IPC messages for one engine — a progress bar nobody can see
/// moving that fast, at the cost of the UI thread that has to drain them.
const REPORT_EVERY: u64 = 4 * 1024 * 1024;

/// Streams the archive to `into`, picking up where a previous attempt left off.
///
/// Written to disk chunk by chunk rather than collected first: an engine is a
/// few hundred megabytes, and the machine most likely to be running this is the
/// one that just discovered it has no engine at all. What is already on disk
/// is asked for with `Range`; a mirror that answers 206 continues it, one that
/// answers 200 did not understand and starts over.
async fn download_from(
    http: &reqwest::Client,
    url: &str,
    into: &Path,
    expected: u64,
    say: &impl Fn(EngineProgress),
) -> Result<()> {
    let have = tokio::fs::metadata(into)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0);
    // The archive as it is on the mirror, byte for byte: the offset resumed
    // from, the size the index quoted and the checksum it gave are all of the
    // stored file, and the client otherwise asks for gzip on every request.
    // A mirror that compressed the answer would make a resume append the
    // wrong bytes at the wrong offset, and the checksum would then reject
    // the whole download rather than the mirror.
    let mut request = http
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut response = request
        .send()
        .await
        .map_err(|err| ApiError::new("network", format!("fetching the engine: {err}")))?;

    let status = response.status();
    let io = |err| ApiError::new("io", format!("opening the archive: {err}"));
    let (mut file, mut got) = if status == reqwest::StatusCode::PARTIAL_CONTENT && have > 0 {
        tracing::info!(have, "resuming the engine archive");
        let file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(into)
            .await
            .map_err(io)?;
        (file, have)
    } else if status.is_success() {
        (tokio::fs::File::create(into).await.map_err(io)?, 0)
    } else {
        return Err(ApiError::new(
            "network",
            format!("the mirror answered {status}"),
        ));
    };

    // The index carries a size, but the response's own is the one that matches
    // what is arriving — and after a resume it counts only the remainder.
    let total = match response.content_length() {
        Some(remaining) => got + remaining,
        None => expected.max(got),
    };
    let mut reported = got;

    // `chunk` rather than a stream, so reqwest needs no extra feature and this
    // needs no futures crate for one loop.
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| ApiError::new("network", format!("the download stopped: {err}")))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|err| ApiError::new("io", format!("writing the archive: {err}")))?;
        got += chunk.len() as u64;
        if got - reported >= REPORT_EVERY {
            reported = got;
            say(EngineProgress::Downloading { got, total });
        }
    }

    file.flush()
        .await
        .map_err(|err| ApiError::new("io", format!("finishing the archive: {err}")))?;

    // The bar should read full before extraction starts, whatever the last
    // reporting threshold happened to land on.
    say(EngineProgress::Downloading { got, total });
    Ok(())
}

/// Checks the archive against the checksum it was published with, when there
/// was one.
///
/// A mismatch discards the file: a resumed download that went wrong, or a
/// mirror serving something else, and either way not something to unpack and
/// then find out about at launch. It matters more on the Apple Silicon path
/// than on BAR's own, since what arrives there is a third party's code that
/// this process is about to make executable — so the sha256 GitHub took of the
/// asset is checked before anything is unpacked from it.
async fn verify(path: &Path, sum: Checksum<'_>) -> Result<()> {
    let (kind, expected) = match sum {
        Checksum::Md5(expected) => (Hash::Md5, expected),
        Checksum::Sha256(expected) => (Hash::Sha256, expected),
    };
    let Some(expected) = expected else {
        tracing::info!(
            kind = kind.name(),
            "no checksum was published; unpacking unverified"
        );
        return Ok(());
    };
    let actual = tokio::task::spawn_blocking({
        let path = path.to_path_buf();
        move || kind.of(&path)
    })
    .await
    .map_err(|err| ApiError::new("io", format!("checking the archive: {err}")))?
    .map_err(|err| ApiError::new("io", format!("reading the archive back: {err}")))?;

    if actual.eq_ignore_ascii_case(expected) {
        tracing::info!(
            kind = kind.name(),
            checksum = actual,
            "engine archive verified"
        );
        return Ok(());
    }
    let _ = std::fs::remove_file(path);
    Err(ApiError::new(
        "archive",
        format!(
            "the archive's {} ({actual}) is not the published one ({expected}); it was discarded",
            kind.name()
        ),
    ))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Removes staging files older than `after`, so a download abandoned long ago
/// does not sit under `engine/` forever. A recent one is left for resuming.
fn sweep_stale_parts(engine_dir: &Path, after: Duration) {
    let Ok(entries) = std::fs::read_dir(engine_dir) else {
        return;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with('.') && name.ends_with(".part")) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > after);
        if stale {
            tracing::info!(path = %path.display(), "removing an abandoned engine download");
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use content::release::Release;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn md5_of(bytes: &[u8]) -> String {
        hex(&Md5::digest(bytes))
    }

    fn release(mirrors: Vec<String>, md5: Option<&str>) -> Release {
        Release {
            filename: "x.7z".into(),
            mirrors,
            size: 6,
            md5: md5.map(str::to_owned),
        }
    }

    /// The download as a room's engine fetch makes it: BAR's index, an md5.
    async fn fetch_indexed(release: &Release, into: &std::path::Path) -> Result<()> {
        download(
            &content::http::client("test"),
            &release.mirrors,
            release.size,
            Checksum::Md5(release.md5.as_deref()),
            into,
            &quiet,
        )
        .await
    }

    fn quiet(_: EngineProgress) {}

    #[tokio::test]
    async fn a_broken_off_download_is_resumed_and_then_verified() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/x.7z"))
            .and(header("Range", "bytes=3-"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(b"def".to_vec()))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join(".x.7z.part");
        std::fs::write(&staging, b"abc").unwrap();

        let release = release(
            vec![format!("{}/x.7z", server.uri())],
            Some(&md5_of(b"abcdef")),
        );
        fetch_indexed(&release, &staging).await.unwrap();
        assert_eq!(std::fs::read(&staging).unwrap(), b"abcdef");
    }

    #[tokio::test]
    async fn a_mirror_that_ignores_the_range_starts_over() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abcdef".to_vec()))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join(".x.7z.part");
        std::fs::write(&staging, b"abc").unwrap();

        let release = release(
            vec![format!("{}/x.7z", server.uri())],
            Some(&md5_of(b"abcdef")),
        );
        fetch_indexed(&release, &staging).await.unwrap();
        assert_eq!(std::fs::read(&staging).unwrap(), b"abcdef");
    }

    #[tokio::test]
    async fn a_checksum_mismatch_discards_the_archive() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abcdef".to_vec()))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join(".x.7z.part");

        let release = release(
            vec![format!("{}/x.7z", server.uri())],
            Some(&md5_of(b"something else")),
        );
        let err = fetch_indexed(&release, &staging).await.unwrap_err();
        assert_eq!(err.code, "archive");
        assert!(!staging.exists(), "nothing downstream may find it");
    }

    #[tokio::test]
    async fn the_next_mirror_is_tried_when_the_first_fails() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/down/x.7z"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/up/x.7z"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abcdef".to_vec()))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join(".x.7z.part");

        let release = release(
            vec![
                format!("{}/down/x.7z", server.uri()),
                format!("{}/up/x.7z", server.uri()),
            ],
            Some(&md5_of(b"abcdef")),
        );
        fetch_indexed(&release, &staging).await.unwrap();
        assert_eq!(std::fs::read(&staging).unwrap(), b"abcdef");
    }

    #[tokio::test]
    async fn every_mirror_failing_is_the_last_ones_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .expect(2)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join(".x.7z.part");
        let release = release(
            vec![
                format!("{}/a/x.7z", server.uri()),
                format!("{}/b/x.7z", server.uri()),
            ],
            None,
        );
        let err = fetch_indexed(&release, &staging).await.unwrap_err();
        assert_eq!(err.code, "network");
        assert!(err.message.contains("404"));
    }

    /// The headline path, against the real releases and a real network.
    ///
    /// Everything else here is mocked on purpose -- a test that fetches
    /// seventy-five megabytes is not one to run on every `cargo test`. But the
    /// Apple Silicon install is the one path where every step is somebody
    /// else's: GitHub's API shape, their checksum, their zip, their bundle,
    /// and a `Info.plist` we read a version out of. Mocking all of that would
    /// only assert that this file agrees with itself.
    ///
    /// So it is asked for real, behind `--ignored`, on the machine it is for:
    ///
    /// ```text
    /// cargo test -p modlobby-app -- --ignored --nocapture apple
    /// ```
    #[tokio::test]
    #[ignore = "downloads the real Apple Silicon engine; macOS only"]
    async fn the_apple_silicon_engine_installs_from_its_releases() {
        if !content::apple::supported() {
            println!("not an Apple Silicon Mac; nothing to install");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let dirs = content::DataDirs::only(dir.path());
        let http = content::http::client(env!("CARGO_PKG_VERSION"));

        let seen = std::sync::Mutex::new(Vec::new());
        let say = |progress: EngineProgress| {
            let phase = match &progress {
                EngineProgress::Finding => "finding".to_owned(),
                EngineProgress::Downloading { got, total } => {
                    format!("downloading {got}/{total}")
                }
                EngineProgress::Extracting => "extracting".to_owned(),
                EngineProgress::Done { version } => format!("done {version}"),
                EngineProgress::Failed { reason } => format!("failed {reason}"),
            };
            seen.lock().unwrap().push(phase);
        };

        // The version asked for is a hope on this path, not a request: the
        // port carries whichever engine its author built against.
        let installed = fetch(&http, &dirs, "2026.07.04", &say)
            .await
            .expect("an engine");

        assert!(
            installed.bin.join(recoil::ENGINE_BINARY).is_file(),
            "the engine is where it was said to be"
        );
        assert!(!installed.version.is_empty());
        let library = content::Library::new(dirs.clone());
        assert!(
            library.has_engine(&installed.version),
            "and is found by the version it reported"
        );
        assert_eq!(library.installed_engines(), [installed.version.clone()]);
        assert!(
            library.find_downloader(&installed.version).is_some(),
            "pr-downloader comes with it, which is what fetches everything else"
        );
        assert!(
            !library.installed_ais().is_empty(),
            "and the skirmish AIs, which is what there is to play against"
        );

        let phases = seen.lock().unwrap().clone();
        assert_eq!(phases.first().map(String::as_str), Some("finding"));
        assert!(
            phases.iter().any(|phase| phase == "extracting"),
            "{phases:?}"
        );
        assert!(
            phases.iter().any(|phase| phase.starts_with("downloading")),
            "a silent download of this size looks like a hang: {phases:?}"
        );

        // Asked again, it recognises the build it just installed and fetches
        // nothing -- which is what stops a room wanting a newer engine than
        // the port has reached from downloading it on every ask.
        seen.lock().unwrap().clear();
        let again = fetch(&http, &dirs, "2099.01.01", &say)
            .await
            .expect("the one already installed");
        assert_eq!(again.version, installed.version);
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.starts_with("downloading")),
            "the current build was already here: {:?}",
            seen.lock().unwrap()
        );

        println!(
            "engine {} installed under {}",
            installed.version,
            installed.bin.display()
        );
    }

    #[test]
    fn only_old_staging_files_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join(".old.7z.part");
        let recent = dir.path().join(".recent.7z.part");
        let engine = dir.path().join(".hidden-but-not-a-part");
        for path in [&old, &recent, &engine] {
            std::fs::write(path, b"x").unwrap();
        }
        let long_ago = std::time::SystemTime::now() - STALE_PART_AFTER * 2;
        for path in [&old, &engine] {
            // Writable, because Windows will not date a file opened read-only.
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(long_ago)
                .unwrap();
        }

        sweep_stale_parts(dir.path(), STALE_PART_AFTER);

        assert!(!old.exists());
        assert!(recent.exists(), "a recent one is resumed, not removed");
        assert!(engine.exists(), "only `.…part` files are ours to remove");
    }
}
