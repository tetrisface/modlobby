//! Spawning the engine for a running game, and where its content lives.
//!
//! modlobby writes to a directory of its own and reads every other lobby's
//! install beside it. That is the engine's own model — one write dir, any
//! number of read dirs — and it means nothing of ours is ever half-written
//! into another lobby's tree, while a map or game they already have is never
//! fetched twice.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use content::DataDirs;
use tokio::process::Child;

/// One environment variable. A function rather than `std::env`, so the
/// directories can be computed for a machine a test describes.
type Var<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// `$name`, else `default` under `$HOME`.
fn xdg(var: Var, name: &str, default: &str) -> Option<PathBuf> {
    var(name)
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(default)))
}

/// modlobby's own data directory: where what it downloads goes.
///
/// Local rather than roaming on Windows, and XDG data rather than config on
/// Linux: it holds gigabytes of content, not settings.
pub fn own_data_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name);
    own_data_dir_in(&var)
}

fn own_data_dir_in(var: Var) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        var("LOCALAPPDATA").map(PathBuf::from)
    } else {
        xdg(var, "XDG_DATA_HOME", ".local/share")
    };
    base.map(|base| base.join("modlobby").join("data"))
}

/// Other lobbies' installs that exist on this machine, most trusted first.
pub fn installed_dirs() -> Vec<PathBuf> {
    let var = |name: &str| std::env::var_os(name);
    let candidates = if cfg!(windows) {
        windows_candidates(&var)
    } else {
        unix_candidates(&var)
    };
    candidates.into_iter().filter(|dir| dir.is_dir()).collect()
}

/// The legacy launcher's data directory leads because Chobby and its package
/// cleanup live there; the new bar-lobby's `assets` follows. Both under
/// `%LOCALAPPDATA%\Programs` (`bar-lobby/src/main/config/app.ts`).
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
/// (`bar-lobby/src/main/config/app.ts`).
fn unix_candidates(var: Var) -> Vec<PathBuf> {
    let launcher =
        xdg(var, "XDG_STATE_HOME", ".local/state").map(|state| state.join("Beyond-All-Reason"));
    let bar_lobby = var("BAR_ASSETS_PATH").map(PathBuf::from).or_else(|| {
        xdg(var, "XDG_DATA_HOME", ".local/share")
            .map(|data| data.join("BeyondAllReason").join("assets"))
    });
    [launcher, bar_lobby].into_iter().flatten().collect()
}

/// The directories the engine and every content check use: `write` is the
/// setting when given, else modlobby's own; everything else found on the
/// machine is read. `None` only on a machine with no home directory.
pub fn data_dirs(write: Option<PathBuf>) -> Option<DataDirs> {
    let write = write.or_else(own_data_dir)?;
    Some(assemble(write, installed_dirs()))
}

/// A user who points the write dir at another lobby's install gets today's
/// single-directory behaviour, not that directory twice.
fn assemble(write: PathBuf, installed: Vec<PathBuf>) -> DataDirs {
    let read = installed.into_iter().filter(|dir| *dir != write).collect();
    DataDirs { write, read }
}

/// The engine reads its settings from the write directory, so a fresh one
/// starts from another install's `springsettings.cfg` when there is one: the
/// user's resolution and keys, without a second setup. Only ever the first
/// time; after that the copy is theirs to change.
fn seed_settings(dirs: &DataDirs) {
    let ours = dirs.write.join("springsettings.cfg");
    if ours.exists() {
        return;
    }
    let Some(theirs) = dirs
        .read
        .iter()
        .map(|dir| dir.join("springsettings.cfg"))
        .find(|path| path.is_file())
    else {
        return;
    };
    let _ = std::fs::create_dir_all(&dirs.write);
    match std::fs::copy(&theirs, &ours) {
        Ok(_) => tracing::info!(from = %theirs.display(), "seeded engine settings"),
        Err(err) => tracing::warn!(%err, from = %theirs.display(), "engine settings not seeded"),
    }
}

/// Finds `engine_version` in any data directory and starts it on `target`
/// (a `spring://` URL or a start script), writing to `dirs.write`.
///
/// `overlay_config_dir` is where a borderless copy of the user's settings may
/// be kept, when they have the overlay on and their own settings would put the
/// game in exclusive full screen. Passing `None`, or having settings that
/// already work, launches against their configuration untouched.
pub fn spawn(
    dirs: &DataDirs,
    engine_version: &str,
    target: String,
    overlay_config_dir: Option<&Path>,
) -> Result<Child, String> {
    let engine_dir = content::Library::new(dirs.clone())
        .find_engine(engine_version)
        .ok_or_else(|| {
            format!(
                "no engine {engine_version} with {} under {}",
                recoil::ENGINE_BINARY,
                dirs.write.join("engine").display()
            )
        })?;
    seed_settings(dirs);
    // A failure here is not worth refusing to play over: the game still
    // runs, the overlay just cannot cover it.
    let config = overlay_config_dir.and_then(|dir| {
        recoil::window_mode::borderless_config(&dirs.write, engine_version, dir)
            .inspect_err(|err| tracing::warn!(%err, "no borderless config; overlay may not show"))
            .ok()
            .flatten()
    });

    let launch = recoil::Launch {
        engine_dir,
        data_dir: dirs.write.clone(),
        read_dirs: dirs.read.clone(),
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
/// on every run, so two at once corrupt each other's view of it. It writes to
/// `dirs.write` only; what the read directories already hold was left out of
/// `wants` by the caller.
pub fn spawn_download(
    dirs: &DataDirs,
    engine_version: &str,
    wants: Vec<(recoil::Want, String)>,
) -> Result<Child, String> {
    let binary = content::Library::new(dirs.clone())
        .find_downloader(engine_version)
        .ok_or_else(|| {
            format!(
                "no engine under {} ships {}; install an engine first",
                dirs.write.join("engine").display(),
                recoil::DOWNLOADER_BINARY
            )
        })?;
    let download = recoil::Download {
        binary,
        data_dir: dirs.write.clone(),
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
    fn our_own_directory_is_local_data_not_config() {
        let found = own_data_dir_in(&env(&[
            ("LOCALAPPDATA", "C:/u/AppData/Local"),
            ("HOME", "/home/dev"),
        ]))
        .unwrap();
        let expected = if cfg!(windows) {
            "C:/u/AppData/Local/modlobby/data"
        } else {
            "/home/dev/.local/share/modlobby/data"
        };
        assert_eq!(paths(vec![found]), [expected]);
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
        assert_eq!(own_data_dir_in(&env(&[])), None);
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
    fn every_install_found_is_read_and_only_ours_is_written() {
        let dirs = assemble(
            PathBuf::from("/ours"),
            vec![PathBuf::from("/launcher"), PathBuf::from("/bar-lobby")],
        );
        assert_eq!(dirs.write, PathBuf::from("/ours"));
        assert_eq!(
            dirs.read,
            [PathBuf::from("/launcher"), PathBuf::from("/bar-lobby")]
        );
    }

    #[test]
    fn pointing_the_write_dir_at_an_install_does_not_read_it_twice() {
        let dirs = assemble(
            PathBuf::from("/launcher"),
            vec![PathBuf::from("/launcher"), PathBuf::from("/bar-lobby")],
        );
        assert_eq!(dirs.read, [PathBuf::from("/bar-lobby")]);
    }

    #[test]
    fn a_fresh_write_dir_starts_from_their_engine_settings() {
        let ours = tempfile::tempdir().unwrap();
        let theirs = tempfile::tempdir().unwrap();
        std::fs::write(
            theirs.path().join("springsettings.cfg"),
            "XResolution = 1920\n",
        )
        .unwrap();
        let dirs = DataDirs {
            write: ours.path().join("data"),
            read: vec![theirs.path().to_path_buf()],
        };

        seed_settings(&dirs);
        let seeded = std::fs::read_to_string(dirs.write.join("springsettings.cfg")).unwrap();
        assert_eq!(seeded, "XResolution = 1920\n");

        // Theirs changes later; ours is ours now.
        std::fs::write(
            theirs.path().join("springsettings.cfg"),
            "XResolution = 800\n",
        )
        .unwrap();
        seed_settings(&dirs);
        let kept = std::fs::read_to_string(dirs.write.join("springsettings.cfg")).unwrap();
        assert_eq!(kept, "XResolution = 1920\n");
    }
}
