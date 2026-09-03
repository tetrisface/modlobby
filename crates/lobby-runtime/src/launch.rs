//! Spawning the engine for a running game.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tokio::process::Child;

/// One environment variable. A function rather than `std::env`, so the
/// candidates can be computed for a machine a test describes.
type Var<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// Where BAR's content may already be, most trusted first.
///
/// The legacy launcher's data directory leads because Chobby and its package
/// cleanup live there; the new bar-lobby's `assets` follows. Reading
/// bar-lobby's assets as the data directory accepts two things: a map the user
/// dropped into its *state* directory is not seen, and an engine launched with
/// this as its write directory leaves demos and infolog beside the assets.
fn candidates() -> Vec<PathBuf> {
    let var = |name: &str| std::env::var_os(name);
    if cfg!(windows) {
        windows_candidates(&var)
    } else {
        unix_candidates(&var)
    }
}

/// Both under `%LOCALAPPDATA%\Programs` (`bar-lobby/src/main/config/app.ts`).
fn windows_candidates(var: Var) -> Vec<PathBuf> {
    let Some(local) = var("LOCALAPPDATA") else {
        return Vec::new();
    };
    let programs = PathBuf::from(local).join("Programs");
    vec![
        programs.join("Beyond-All-Reason").join("data"),
        programs.join("BeyondAllReason").join("assets"),
    ]
}

/// The launcher writes to `$XDG_STATE_HOME/Beyond-All-Reason`
/// (`spring-launcher/src/write_path.js`); bar-lobby's assets are
/// `$BAR_ASSETS_PATH`, else `$XDG_DATA_HOME/BeyondAllReason/assets`
/// (`bar-lobby/src/main/config/app.ts`). The XDG defaults are under `$HOME`.
fn unix_candidates(var: Var) -> Vec<PathBuf> {
    let home = var("HOME").map(PathBuf::from);
    let xdg = |name: &str, default: &str| {
        var(name)
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(default)))
    };
    let launcher =
        xdg("XDG_STATE_HOME", ".local/state").map(|state| state.join("Beyond-All-Reason"));
    let bar_lobby = var("BAR_ASSETS_PATH").map(PathBuf::from).or_else(|| {
        xdg("XDG_DATA_HOME", ".local/share").map(|data| data.join("BeyondAllReason").join("assets"))
    });
    [launcher, bar_lobby].into_iter().flatten().collect()
}

/// Where the first engine goes on a bare machine: the most trusted candidate,
/// whether or not the directory exists yet. Anything that needs content that
/// is already there wants [`default_data_dir`] instead.
pub fn launcher_data_dir() -> Option<PathBuf> {
    candidates().into_iter().next()
}

/// Where BAR's content already is, so the game is not downloaded a second
/// time onto a machine that already has it.
pub fn default_data_dir() -> Option<PathBuf> {
    first_present(candidates())
}

fn first_present(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|dir| dir.is_dir())
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

    /// An environment made of the given variables and nothing else.
    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    fn paths(found: Vec<PathBuf>) -> Vec<String> {
        found
            .iter()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .collect()
    }

    #[test]
    fn windows_looks_beside_the_launcher_then_bar_lobby() {
        let found = windows_candidates(&env(&[("LOCALAPPDATA", "C:/u/AppData/Local")]));
        assert_eq!(
            paths(found),
            [
                "C:/u/AppData/Local/Programs/Beyond-All-Reason/data",
                "C:/u/AppData/Local/Programs/BeyondAllReason/assets",
            ]
        );
    }

    #[test]
    fn windows_without_localappdata_has_nowhere_to_look() {
        assert!(windows_candidates(&env(&[])).is_empty());
    }

    #[test]
    fn unix_defaults_to_the_xdg_directories_under_home() {
        let found = unix_candidates(&env(&[("HOME", "/home/dev")]));
        assert_eq!(
            paths(found),
            [
                "/home/dev/.local/state/Beyond-All-Reason",
                "/home/dev/.local/share/BeyondAllReason/assets",
            ]
        );
    }

    #[test]
    fn unix_honours_the_xdg_variables_over_home() {
        let found = unix_candidates(&env(&[
            ("HOME", "/home/dev"),
            ("XDG_STATE_HOME", "/state"),
            ("XDG_DATA_HOME", "/data"),
        ]));
        assert_eq!(
            paths(found),
            ["/state/Beyond-All-Reason", "/data/BeyondAllReason/assets"]
        );
    }

    #[test]
    fn unix_takes_bar_lobbys_own_assets_setting_first() {
        let found = unix_candidates(&env(&[
            ("HOME", "/home/dev"),
            ("BAR_ASSETS_PATH", "/games/bar"),
        ]));
        assert_eq!(paths(found)[1], "/games/bar");
    }

    #[test]
    fn unix_without_home_has_nowhere_to_look() {
        assert!(unix_candidates(&env(&[])).is_empty());
    }

    #[test]
    fn the_launcher_wins_when_both_are_installed() {
        let launcher = tempfile::tempdir().unwrap();
        let bar_lobby = tempfile::tempdir().unwrap();
        let found = first_present([
            launcher.path().to_path_buf(),
            bar_lobby.path().to_path_buf(),
        ]);
        assert_eq!(found.as_deref(), Some(launcher.path()));
    }

    #[test]
    fn a_machine_with_only_bar_lobby_uses_its_content() {
        let bar_lobby = tempfile::tempdir().unwrap();
        let missing = bar_lobby.path().join("no-launcher-here");
        let found = first_present([missing, bar_lobby.path().to_path_buf()]);
        assert_eq!(found.as_deref(), Some(bar_lobby.path()));
    }

    #[test]
    fn nothing_installed_is_nothing_found() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(first_present([dir.path().join("a")]), None);
    }
}
