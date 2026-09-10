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
//! The installer does the restart: on Windows `install` hands over to NSIS
//! and exits this process, and NSIS relaunches the app with the arguments it
//! had. On Linux the AppImage is rewritten in place and `install` returns, so
//! the restart is asked for here. Nothing runs after a successful install.

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
    /// `None` everywhere Beyond All Reason publishes a build. Where it does
    /// not, it is `content::release::NOT_PUBLISHED_HERE` — the same fact
    /// `download_engine` refuses with, carried here so the room can decline to
    /// offer the download rather than offer it and be told. The engine that
    /// runs there arrived by hand, so it is named rather than chosen: a picker
    /// over it could only list what somebody had already put on the disk.
    ///
    /// The reason rather than a `bool`, so nothing can draw the refusal
    /// without the words that explain it, and so the sentence is written once
    /// instead of once per language.
    ///
    /// Not derived from `plays_online`: they are two facts with one cause
    /// today, and they come apart the moment the Apple Silicon build's author
    /// is approved — the servers would open while BAR's index still published
    /// no Apple build, and a room deriving one from the other would go back to
    /// offering a 404.
    pub no_published_engine: Option<&'static str>,
}

#[tauri::command]
pub fn app_version() -> VersionView {
    VersionView {
        version: env!("CARGO_PKG_VERSION"),
        commit: env!("MODLOBBY_COMMIT"),
        plays_online: recoil::may_join_hosted_games(),
        no_published_engine: content::release::no_published_engine(),
    }
}

/// How far along an update is. Emitted on the `app-update` event.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", tag = "phase")]
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
    /// Downloaded and waiting: a room is joined or a game is running, so
    /// installing now would pull the floor out. The corner offers the restart.
    Ready {
        version: String,
    },
    Failed {
        reason: String,
    },
}

/// An update the look found, and what has been done about it so far.
enum Pending {
    Found(Update),
    Downloaded(Update, Vec<u8>),
}

/// The update between the look and the install.
#[derive(Default)]
pub struct Staged(Mutex<Option<Pending>>);

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
        return Err(ApiError::new(
            "update",
            format!("updates are off: {AUTO_UPDATE_ENV} says so, or is unset in a dev run"),
        ));
    }

    let say = |progress: UpdateProgress| {
        let _ = handle.emit("app-update", progress);
    };
    say(UpdateProgress::Checking);

    let outcome = look(&handle).await.map(|found| {
        app.update_memory.record(SystemTime::now());
        let mut held = staged.0.lock().expect("staged update");
        match (found, held.take()) {
            (None, _) => UpdateProgress::UpToDate,
            // The same version already downloaded and waiting: nothing to
            // fetch again, and the restart is still the offer.
            (Some(update), Some(Pending::Downloaded(done, bytes)))
                if done.version == update.version =>
            {
                let version = done.version.clone();
                *held = Some(Pending::Downloaded(done, bytes));
                UpdateProgress::Ready { version }
            }
            (Some(update), _) => {
                let version = update.version.clone();
                *held = Some(Pending::Found(update));
                UpdateProgress::Available { version }
            }
        }
    });

    match &outcome {
        Ok(progress) => say(progress.clone()),
        Err(err) => say(UpdateProgress::Failed {
            reason: err.message.clone(),
        }),
    }
    outcome
}

/// Takes the corner's offer: downloads what the look found and installs it,
/// or stages it as `Ready` when a room or a game would be lost. Installs at
/// once what an earlier click already downloaded. Returns only when there is
/// nothing to install: a successful install ends the process.
#[tauri::command]
pub async fn install_update(
    app: State<'_, App>,
    staged: State<'_, Staged>,
    handle: AppHandle,
) -> Result<UpdateProgress> {
    let taken = staged.0.lock().expect("staged update").take();
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
            Ok(bytes) => (update, bytes),
            Err(err) => {
                // Still found, still on offer; the next click tries again.
                *staged.0.lock().expect("staged update") = Some(Pending::Found(update));
                say(UpdateProgress::Failed {
                    reason: err.message.clone(),
                });
                return Err(err);
            }
        },
    };

    if busy(&app).await {
        let version = update.version.clone();
        *staged.0.lock().expect("staged update") = Some(Pending::Downloaded(update, bytes));
        let progress = UpdateProgress::Ready { version };
        say(progress.clone());
        return Ok(progress);
    }

    let outcome = install(&handle, &update, &bytes);
    if let Err(err) = &outcome {
        say(UpdateProgress::Failed {
            reason: err.message.clone(),
        });
    }
    outcome
}

/// The daily look, when it is due. Quiet about being offline: an update is
/// not something to be told about failing to look for.
pub async fn daily(handle: AppHandle) {
    let app = handle.state::<App>();
    if !app.update_memory.due(SystemTime::now(), EVERY) {
        tracing::debug!("update check: looked within the day, not again");
        return;
    }
    let staged = handle.state::<Staged>();
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

    /// The two answers cannot drift on the one platform where either of them
    /// is the uncommon one, which is the only platform nobody develops on.
    #[test]
    fn the_refusal_is_carried_exactly_where_there_is_no_category() {
        let view = super::app_version();
        assert_eq!(
            view.no_published_engine.is_some(),
            content::release::category().is_none()
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
}
