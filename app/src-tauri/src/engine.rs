//! Getting an engine onto a machine that has none.
//!
//! Everything else modlobby fetches goes through pr-downloader, which is the
//! right tool and handles rapid, mirrors and resume. It cannot fetch an engine,
//! because it ships inside one — so this is the one download modlobby does
//! itself, and only ever the first one.
//!
//! It is done the way pr-downloader would do it: the index's checksum is
//! verified before anything is unpacked, a download that broke off is resumed
//! rather than restarted, and every mirror the index named gets its turn.
//!
//! After it, `recoil::find_downloader` has something to find and the ordinary
//! content path takes over.

use std::path::{Path, PathBuf};
use std::time::Duration;

use md5::{Digest, Md5};
use serde::Serialize;
use tauri::{Emitter, State};
use tokio::io::AsyncWriteExt;

use crate::commands::{ApiError, Result};
use crate::state::App;

/// How far along the one download this module does is.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", tag = "phase")]
#[ts(export)]
pub enum EngineProgress {
    /// Asking BAR's index where this version lives.
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

/// Downloads and unpacks an engine into `<data>/engine/<version>`.
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
    let data_dir = crate::commands::data_dir_or_default(&app)?;
    let say = |progress: EngineProgress| {
        let _ = window.emit("engine-download", progress);
    };

    match fetch(&app.http, &data_dir, &version, &say).await {
        Ok(path) => {
            say(EngineProgress::Done {
                version: version.clone(),
            });
            Ok(path.to_string_lossy().into_owned())
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
    data_dir: &Path,
    version: &str,
    say: &impl Fn(EngineProgress),
) -> Result<PathBuf> {
    // Already there: not an error, and not a reason to download it again.
    if let Some(found) = recoil::find_engine(data_dir, version) {
        return Ok(found);
    }
    let engine_dir = data_dir.join("engine");
    std::fs::create_dir_all(&engine_dir)
        .map_err(|err| ApiError::new("io", format!("making the engine directory: {err}")))?;
    sweep_stale_parts(&engine_dir, STALE_PART_AFTER);

    say(EngineProgress::Finding);
    let index = http
        .get(content::release::find_url(version))
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
                content::release::category()
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
    download(http, &release, &staging, say).await?;

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

    recoil::find_engine(data_dir, version).ok_or_else(|| {
        ApiError::new(
            "archive",
            format!(
                "the archive unpacked but holds no {}",
                recoil::ENGINE_BINARY
            ),
        )
    })
}

/// Tries every mirror the index named, in its order, and verifies what arrived.
async fn download(
    http: &reqwest::Client,
    release: &content::release::Release,
    into: &Path,
    say: &impl Fn(EngineProgress),
) -> Result<()> {
    let mut last = None;
    for mirror in &release.mirrors {
        match download_from(http, mirror, into, release.size, say).await {
            Ok(()) => return verify(into, release.md5.as_deref()).await,
            Err(err) => {
                tracing::warn!(mirror, reason = %err.message, "engine mirror failed");
                last = Some(err);
            }
        }
    }
    Err(last.unwrap_or_else(|| ApiError::new("notFound", "the index named no mirror")))
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
    let mut request = http.get(url);
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

/// Checks the archive against the index's checksum, when it gave one.
///
/// A mismatch discards the file: a resumed download that went wrong, or a
/// mirror serving something else, and either way not something to unpack and
/// then find out about at launch.
async fn verify(path: &Path, expected: Option<&str>) -> Result<()> {
    let Some(expected) = expected else {
        tracing::info!("the index gave no checksum; unpacking unverified");
        return Ok(());
    };
    let actual = tokio::task::spawn_blocking({
        let path = path.to_path_buf();
        move || -> std::io::Result<String> {
            use std::io::Read;
            let mut file = std::fs::File::open(path)?;
            let mut hasher = Md5::new();
            let mut chunk = [0_u8; 64 * 1024];
            loop {
                let read = file.read(&mut chunk)?;
                if read == 0 {
                    break;
                }
                hasher.update(&chunk[..read]);
            }
            Ok(hex(&hasher.finalize()))
        }
    })
    .await
    .map_err(|err| ApiError::new("io", format!("checking the archive: {err}")))?
    .map_err(|err| ApiError::new("io", format!("reading the archive back: {err}")))?;

    if actual.eq_ignore_ascii_case(expected) {
        tracing::info!(md5 = actual, "engine archive verified");
        return Ok(());
    }
    let _ = std::fs::remove_file(path);
    Err(ApiError::new(
        "archive",
        format!(
            "the archive's checksum ({actual}) is not the index's ({expected}); it was discarded"
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
        download(&content::http::client("test"), &release, &staging, &quiet)
            .await
            .unwrap();
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
        download(&content::http::client("test"), &release, &staging, &quiet)
            .await
            .unwrap();
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
        let err = download(&content::http::client("test"), &release, &staging, &quiet)
            .await
            .unwrap_err();
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
        download(&content::http::client("test"), &release, &staging, &quiet)
            .await
            .unwrap();
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
        let err = download(&content::http::client("test"), &release, &staging, &quiet)
            .await
            .unwrap_err();
        assert_eq!(err.code, "network");
        assert!(err.message.contains("404"));
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
