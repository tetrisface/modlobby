//! Spawning the engine for a running game.

use std::path::{Path, PathBuf};

use tokio::process::Child;

/// Where the BAR launcher keeps its data on Windows.
pub fn default_data_dir() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let dir = PathBuf::from(local)
        .join("Programs")
        .join("Beyond-All-Reason")
        .join("data");
    dir.is_dir().then_some(dir)
}

/// Finds `engine_version` under `data_dir/engine` and starts it on `target`
/// (a `spring://` URL or a start script).
///
/// `overlay_config_dir` is where a borderless copy of the user's settings may
/// be kept, when they have the overlay on and their own settings would put the
/// game in exclusive full screen. Passing `None`, or having settings that
/// already work, launches against their configuration untouched.
pub fn spawn(
    data_dir: &Path,
    engine_version: &str,
    target: String,
    overlay_config_dir: Option<&Path>,
) -> Result<Child, String> {
    let engine_dir = recoil::find_engine(data_dir, engine_version).ok_or_else(|| {
        format!(
            "no engine {engine_version} with {} under {}",
            recoil::ENGINE_BINARY,
            data_dir.join("engine").display()
        )
    })?;
    // A failure here is not worth refusing to play over: the game still
    // runs, the overlay just cannot cover it.
    let config = overlay_config_dir.and_then(|dir| {
        recoil::window_mode::borderless_config(data_dir, engine_version, dir)
            .inspect_err(|err| tracing::warn!(%err, "no borderless config; overlay may not show"))
            .ok()
            .flatten()
    });

    let launch = recoil::Launch {
        engine_dir,
        data_dir: data_dir.to_path_buf(),
        target,
        config,
    };
    tracing::info!(engine = %launch.engine_dir.display(), "launching");
    tokio::process::Command::from(launch.command())
        .spawn()
        .map_err(|err| format!("spawning the engine: {err}"))
}

/// Starts pr-downloader on everything in `wants`, with its output on a pipe.
///
/// One invocation for the whole set: pr-downloader rewrites rapid's repo index
/// on every run, so two at once corrupt each other's view of it.
pub fn spawn_download(
    data_dir: &Path,
    engine_version: &str,
    wants: Vec<(recoil::Want, String)>,
) -> Result<Child, String> {
    let binary = recoil::find_downloader(data_dir, engine_version).ok_or_else(|| {
        format!(
            "no engine under {} ships {}; install an engine first",
            data_dir.join("engine").display(),
            recoil::DOWNLOADER_BINARY
        )
    })?;
    let download = recoil::Download {
        binary,
        data_dir: data_dir.to_path_buf(),
        wants,
    };
    tracing::info!(binary = %download.binary.display(), "downloading content");
    tokio::process::Command::from(download.command())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| format!("spawning pr-downloader: {err}"))
}
