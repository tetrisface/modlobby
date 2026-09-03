//! Keeping the app current.
//!
//! At startup, when the setting says so, the newest release is fetched and
//! handed to the installer before anyone has logged in — the one moment a
//! restart costs nothing. Found later, with a room joined or a game running,
//! the download waits as [`Staged`] and the nav offers it instead.
//!
//! The installer does the restart: on Windows `install` hands over to NSIS
//! and exits this process, and NSIS relaunches the app with the arguments it
//! had. On Linux the AppImage is rewritten in place and `install` returns, so
//! the restart is asked for here. Nothing runs after a successful install.

use std::sync::Mutex;

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
}

#[tauri::command]
pub fn app_version() -> VersionView {
    VersionView {
        version: env!("CARGO_PKG_VERSION"),
        commit: env!("MODLOBBY_COMMIT"),
    }
}

/// How far along an update is. Emitted on the `app-update` event.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", tag = "phase")]
#[ts(export)]
pub enum UpdateProgress {
    Checking,
    UpToDate,
    Downloading {
        #[ts(type = "number")]
        got: u64,
        #[ts(type = "number")]
        total: u64,
    },
    /// Downloaded and waiting: a room is joined or a game is running, so
    /// installing now would pull the floor out. The nav offers it instead.
    Ready {
        version: String,
    },
    Failed {
        reason: String,
    },
}

/// A downloaded update, waiting for a moment when restarting costs nothing.
#[derive(Default)]
pub struct Staged(Mutex<Option<(Update, Vec<u8>)>>);

/// Whether this build may update itself. Unset means yes; `0`, `false`,
/// `off` or `no` means no startup check, and the on-demand check says why
/// it will not look. For a build that has to stay put — a local one behind
/// the released version, or one under test.
pub const AUTO_UPDATE_ENV: &str = "MODLOBBY_AUTO_UPDATE";

pub fn enabled() -> bool {
    allows(std::env::var_os(AUTO_UPDATE_ENV))
}

fn allows(value: Option<std::ffi::OsString>) -> bool {
    let Some(value) = value else {
        return true;
    };
    let value = value.to_string_lossy().trim().to_ascii_lowercase();
    !matches!(value.as_str(), "0" | "false" | "off" | "no")
}

/// How much has to arrive before the front end is told again. An installer
/// is tens of megabytes, so a megabyte is a visible step.
const REPORT_EVERY: u64 = 1024 * 1024;

/// Looks for a newer release and downloads it. Installs it at once unless a
/// room or a game would be lost, in which case it is staged and the answer
/// says so. Returns only when there is nothing to install: a successful
/// install ends the process.
#[tauri::command]
pub async fn check_update(
    app: State<'_, App>,
    staged: State<'_, Staged>,
    handle: AppHandle,
) -> Result<UpdateProgress> {
    if !enabled() {
        return Err(ApiError::new(
            "update",
            format!("updates are off: {AUTO_UPDATE_ENV} says so"),
        ));
    }

    let say = |progress: UpdateProgress| {
        let _ = handle.emit("app-update", progress);
    };

    let outcome = async {
        say(UpdateProgress::Checking);
        let Some((update, bytes)) = fetch(&handle, &say).await? else {
            return Ok(UpdateProgress::UpToDate);
        };
        if busy(&app).await {
            let version = update.version.clone();
            *staged.0.lock().expect("staged update") = Some((update, bytes));
            return Ok(UpdateProgress::Ready { version });
        }
        install(&handle, &update, &bytes)
    }
    .await;

    match &outcome {
        Ok(progress) => say(progress.clone()),
        Err(err) => say(UpdateProgress::Failed {
            reason: err.message.clone(),
        }),
    }
    outcome
}

/// Installs what [`check_update`] staged. The nav's offer, taken.
#[tauri::command]
pub fn install_update(staged: State<'_, Staged>, handle: AppHandle) -> Result<UpdateProgress> {
    let taken = staged.0.lock().expect("staged update").take();
    let Some((update, bytes)) = taken else {
        return Err(ApiError::new("update", "no update has been downloaded"));
    };
    install(&handle, &update, &bytes)
}

/// The startup check, when the setting asks for one. Quiet about being
/// offline: an update is not something to be told about failing to look for.
pub async fn at_startup(handle: AppHandle) {
    let app = handle.state::<App>();
    let staged = handle.state::<Staged>();
    match check_update(app, staged, handle.clone()).await {
        Ok(progress) => tracing::info!(?progress, "update check"),
        Err(err) => tracing::info!(reason = %err.message, "update check"),
    }
}

async fn fetch(
    handle: &AppHandle,
    say: &impl Fn(UpdateProgress),
) -> Result<Option<(Update, Vec<u8>)>> {
    let updater = handle
        .updater()
        .map_err(|err| ApiError::new("update", err.to_string()))?;
    let Some(update) = updater
        .check()
        .await
        .map_err(|err| ApiError::new("update", format!("looking for a release: {err}")))?
    else {
        return Ok(None);
    };

    let mut got = 0_u64;
    let mut reported = 0_u64;
    let bytes = update
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
        .map_err(|err| ApiError::new("update", format!("fetching {}: {err}", update.version)))?;
    Ok(Some((update, bytes)))
}

/// Whether restarting now would take something away: a room we are in, a
/// game that is running, or an engine we launched that is still alive. A
/// runtime that cannot answer has nothing to lose.
async fn busy(app: &App) -> bool {
    let in_room = match app.client.snapshot().await {
        Ok(snapshot) => snapshot.my_battle.is_some() || snapshot.game_running.is_some(),
        Err(_) => false,
    };
    in_room || matches!(app.client.engine_pid().await, Ok(Some(_)))
}

/// Hands the installer its bytes. Does not return on success: the process
/// exits and the new build comes up in its place.
fn install(handle: &AppHandle, update: &Update, bytes: &[u8]) -> Result<UpdateProgress> {
    // The exit that follows is not Tauri's, so the exit handler that takes the
    // in-game widget back out of the user's data directory will not run.
    if let Some(held) = handle.try_state::<crate::InGameHandle>() {
        drop(held.lock().expect("in-game").take());
    }
    update
        .install(bytes)
        .map_err(|err| ApiError::new("update", format!("installing {}: {err}", update.version)))?;
    // NSIS has exited this process by now. The AppImage was rewritten under
    // our feet and nothing relaunches anything, so that is done here.
    #[cfg(not(windows))]
    handle.restart();
    #[cfg(windows)]
    Ok(UpdateProgress::Ready {
        version: update.version.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::allows;

    #[test]
    fn unset_and_anything_else_mean_on() {
        assert!(allows(None));
        for value in ["1", "true", "on", "yes", "", "whatever"] {
            assert!(allows(Some(value.into())), "{value:?}");
        }
    }

    #[test]
    fn the_four_off_words_mean_off_in_any_case() {
        for value in ["0", "false", "off", "no", " OFF ", "False"] {
            assert!(!allows(Some(value.into())), "{value:?}");
        }
    }
}
