//! Spawning the engine for a running game.

use std::path::{Path, PathBuf};

use tokio::process::Child;

/// Where the BAR launcher keeps its data on Windows, install or no install.
///
/// The path exists as an answer even when the directory does not: fetching the
/// first engine onto a bare machine has to put it *somewhere*, and this is the
/// somewhere the launcher itself would have chosen. Anything that needs content
/// that is already there wants [`default_data_dir`] instead.
pub fn launcher_data_dir() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(local)
            .join("Programs")
            .join("Beyond-All-Reason")
            .join("data"),
    )
}

/// Where the new bar-lobby keeps its content on Windows.
///
/// Engines, packages, pool, the rapid index and every map its pr-downloader
/// fetched live under the install's `assets` directory
/// (`bar-lobby/src/main/config/app.ts`). Two things are accepted by reading it
/// as the data directory: a map the user dropped into bar-lobby's state
/// directory (`%APPDATA%\\BeyondAllReason\\data\\maps`) is not seen, and an
/// engine launched with this as its write directory leaves its demos and
/// infolog beside bar-lobby's assets rather than in its state.
pub fn bar_lobby_assets_dir() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(local)
            .join("Programs")
            .join("BeyondAllReason")
            .join("assets"),
    )
}

/// Where BAR's content already is: the legacy launcher's data directory when
/// it exists, else bar-lobby's, else nothing. Whichever is found, the game is
/// not downloaded a second time onto a machine that already has it. The
/// launcher comes first because Chobby and its package cleanup live there.
pub fn default_data_dir() -> Option<PathBuf> {
    first_present([launcher_data_dir(), bar_lobby_assets_dir()])
}

fn first_present(candidates: [Option<PathBuf>; 2]) -> Option<PathBuf> {
    candidates.into_iter().flatten().find(|dir| dir.is_dir())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launcher_wins_when_both_are_installed() {
        let launcher = tempfile::tempdir().unwrap();
        let bar_lobby = tempfile::tempdir().unwrap();
        let found = first_present([
            Some(launcher.path().to_path_buf()),
            Some(bar_lobby.path().to_path_buf()),
        ]);
        assert_eq!(found.as_deref(), Some(launcher.path()));
    }

    #[test]
    fn a_machine_with_only_bar_lobby_uses_its_content() {
        let bar_lobby = tempfile::tempdir().unwrap();
        let missing = bar_lobby.path().join("no-launcher-here");
        let found = first_present([Some(missing), Some(bar_lobby.path().to_path_buf())]);
        assert_eq!(found.as_deref(), Some(bar_lobby.path()));
    }

    #[test]
    fn nothing_installed_is_nothing_found() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(first_present([Some(dir.path().join("a")), None]), None);
    }
}
