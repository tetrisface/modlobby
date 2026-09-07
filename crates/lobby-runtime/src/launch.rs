//! Spawning the engine for a running game, and where its content lives.
//!
//! modlobby writes to a directory of its own and reads every other lobby's
//! install beside it. That is the engine's own model — one write dir, any
//! number of read dirs — and it means nothing of ours is ever half-written
//! into another lobby's tree, while a map or game they already have is never
//! fetched twice.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use content::DataDirs;
use tokio::process::Child;

use crate::player_files;

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
/// Linux: it holds gigabytes of content, not settings. macOS keeps both in
/// `Application Support` -- it has nowhere else to put them, and this is also
/// the directory a person has to open to install an engine by hand, so it is
/// worth it being the one they expect.
pub fn own_data_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name);
    own_data_dir_in(&var)
}

fn own_data_dir_in(var: Var) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        var("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        var("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library").join("Application Support"))
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
    } else if cfg!(target_os = "macos") {
        macos_candidates(&var)
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

/// The Apple Silicon BAR Launcher writes to
/// `~/Library/Application Support/Beyond-All-Reason-mac`
/// (`packaging/launcher.sh`, overridable with `BAR_WRITEDIR_OVERRIDE`), and
/// keeps the base content and the skirmish AIs inside the app bundle itself.
/// Neither spring-launcher nor bar-lobby ships for macOS, so there is nothing
/// else to look for.
fn macos_candidates(var: Var) -> Vec<PathBuf> {
    let launcher = var("BAR_WRITEDIR_OVERRIDE").map(PathBuf::from).or_else(|| {
        var("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Beyond-All-Reason-mac")
        })
    });
    launcher.into_iter().collect()
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

/// An engine that has been started, and what it was started with.
pub struct Launched {
    pub child: Child,
    /// The player's files as they were just before, when there were any to
    /// keep: what to compare against when the game is over.
    pub snapshot: Option<PathBuf>,
    /// The private settings copy handed over with `--config`, when the user's
    /// own would have put the game in exclusive fullscreen. Worth telling them,
    /// since what they change in-game lands there rather than in their file.
    pub config: Option<PathBuf>,
}

/// Finds `engine_version` in any data directory and starts it on `target`
/// (a `spring://` URL or a start script), writing to `dirs.write`.
///
/// A fresh write directory is seeded with the player's files from another
/// install first, and the files are snapshotted before every launch, since
/// the engine rewrites its settings on the way out and has lost them before.
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
    menu: Option<recoil::MenuArchive>,
) -> Result<Launched, String> {
    let engine = content::Library::new(dirs.clone())
        .find_engine(engine_version)
        .ok_or_else(|| {
            format!(
                "no engine {engine_version} with {} under {}",
                recoil::ENGINE_BINARY,
                dirs.write.join("engine").display()
            )
        })?;
    player_files::seed(dirs);
    // Neither failure below is worth refusing to play over: without the
    // snapshot a loss goes unnoticed, without the config the overlay cannot
    // cover the game.
    let snapshot = player_files::snapshot(&dirs.write, SystemTime::now())
        .inspect_err(|err| tracing::warn!(%err, "player's files not snapshotted"))
        .ok()
        .flatten();
    let config = overlay_config_dir.and_then(|dir| {
        recoil::window_mode::borderless_config(&dirs.write, engine_version, dir)
            .inspect_err(|err| tracing::warn!(%err, "no borderless config; overlay may not show"))
            .ok()
            .flatten()
    });
    if let Some(config) = &config {
        tracing::info!(
            config = %config.display(),
            "their settings ask for exclusive fullscreen; launching on a private copy"
        );
    }

    let launch = recoil::Launch {
        engine,
        data_dir: dirs.write.clone(),
        read_dirs: dirs.read.clone(),
        target,
        config: config.clone(),
        menu,
    };
    tracing::info!(engine = %launch.engine.engine().display(), "launching");
    let child = tokio::process::Command::from(launch.command())
        .spawn()
        .map_err(|err| format!("spawning the engine: {err}"))?;
    Ok(Launched {
        child,
        snapshot,
        config,
    })
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

    /// The one BAR install a Mac can have: the Apple Silicon launcher's own
    /// write directory, where its downloaded maps and games are.
    #[test]
    fn a_mac_reuses_the_apple_silicon_launchers_content() {
        let vars = env(&[("HOME", "/Users/ann")]);
        assert_eq!(
            macos_candidates(&vars),
            vec![PathBuf::from(
                "/Users/ann/Library/Application Support/Beyond-All-Reason-mac"
            )]
        );
    }

    /// The launcher honours this, so following it finds a moved install.
    #[test]
    fn a_moved_mac_install_is_followed() {
        let vars = env(&[
            ("HOME", "/Users/ann"),
            ("BAR_WRITEDIR_OVERRIDE", "/Volumes/Games/bar"),
        ]);
        assert_eq!(
            macos_candidates(&vars),
            vec![PathBuf::from("/Volumes/Games/bar")]
        );
    }

    #[test]
    fn a_mac_with_no_home_offers_nothing_rather_than_a_bare_path() {
        let vars = env(&[]);
        assert!(macos_candidates(&vars).is_empty());
    }

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
}
