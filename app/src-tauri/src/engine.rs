//! Getting an engine onto a machine that has none.
//!
//! Everything else modlobby fetches goes through pr-downloader, which is the
//! right tool and handles rapid, mirrors and resume. It cannot fetch an engine,
//! because it ships inside one — so this is the one download modlobby does
//! itself, and only ever the first one.
//!
//! After it, `recoil::find_downloader` has something to find and the ordinary
//! content path takes over.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{Emitter, State};

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
    /// The archive is in hand; unpacking is not interruptible and can take a
    /// while, so it is worth saying that it is happening.
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

    match fetch(&data_dir, &version, &say).await {
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

async fn fetch(data_dir: &Path, version: &str, say: &impl Fn(EngineProgress)) -> Result<PathBuf> {
    // Already there: not an error, and not a reason to download it again.
    if let Some(found) = recoil::find_engine(data_dir, version) {
        return Ok(found);
    }

    say(EngineProgress::Finding);
    let index = reqwest::get(content::release::find_url(version))
        .await
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
    let mirror = release
        .mirrors
        .first()
        .expect("pick kept only mirrored entries");

    // Into a temporary file, because the extractor wants a path and a
    // half-written archive under `engine/` would look like an install.
    let staging = data_dir
        .join("engine")
        .join(format!(".{}.part", release.filename));
    let target = data_dir.join("engine").join(version);
    if let Some(parent) = staging.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| ApiError::new("io", format!("making the engine directory: {err}")))?;
    }

    if let Err(err) = download(mirror, &staging, release.size, say).await {
        // Nothing downstream should find a partial archive lying around.
        let _ = std::fs::remove_file(&staging);
        return Err(err);
    }

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

/// How much has to arrive before the front end is told again.
///
/// Chunks arrive in tens of kilobytes, so one event each would be tens of
/// thousands of IPC messages for one engine — a progress bar nobody can see
/// moving that fast, at the cost of the UI thread that has to drain them.
const REPORT_EVERY: u64 = 4 * 1024 * 1024;

/// Streams the archive to `into`, reporting as it goes.
///
/// Written to disk chunk by chunk rather than collected first: an engine is a
/// few hundred megabytes, and the machine most likely to be running this is the
/// one that just discovered it has no engine at all.
async fn download(
    url: &str,
    into: &Path,
    expected: u64,
    say: &impl Fn(EngineProgress),
) -> Result<()> {
    let mut response = reqwest::get(url)
        .await
        .map_err(|err| ApiError::new("network", format!("fetching the engine: {err}")))?;
    if !response.status().is_success() {
        return Err(ApiError::new(
            "network",
            format!("the mirror answered {}", response.status()),
        ));
    }

    // The index carries a size, but the response's own is the one that matches
    // what is arriving.
    let total = response.content_length().unwrap_or(expected);
    let mut got = 0_u64;
    let mut reported = 0_u64;

    let mut file = tokio::fs::File::create(into)
        .await
        .map_err(|err| ApiError::new("io", format!("opening the archive: {err}")))?;

    // `chunk` rather than a stream, so reqwest needs no extra feature and this
    // needs no futures crate for one loop.
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| ApiError::new("network", format!("the download stopped: {err}")))?
    {
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|err| ApiError::new("io", format!("writing the archive: {err}")))?;
        got += chunk.len() as u64;
        if got - reported >= REPORT_EVERY {
            reported = got;
            say(EngineProgress::Downloading { got, total });
        }
    }

    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|err| ApiError::new("io", format!("finishing the archive: {err}")))?;

    // The bar should read full before extraction starts, whatever the last
    // reporting threshold happened to land on.
    say(EngineProgress::Downloading { got, total });
    Ok(())
}
