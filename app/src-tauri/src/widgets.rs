//! What BAR players actually run, and one-click management of it.
//!
//! The numbers come from pve.bar's weekly projection over public replays —
//! aggregates over distinct players, never named, withheld below a
//! k-anonymity floor. See the `widgets` crate for what is published and why.
//!
//! **Installs go to modlobby's own write directory**, per the one-writable-dir
//! invariant. A widget installed there is visible to games modlobby launches
//! and not to Chobby's — putting one where Chobby would see it is an explicit
//! action a player takes, never a side effect of clicking install here.
//!
//! **Nothing touches the config while a game is running.** BAR's
//! `widgetHandler:Shutdown` rewrites `BYAR.lua` wholesale from memory when the
//! game exits, so an edit made mid-game is discarded without a word. Every
//! command that writes checks first, and no answer counts as running.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::State;
use widgets::config::WidgetConfig;
use widgets::manage::{
    Deleted, InstalledWidget, Ledger, WidgetStatus, read_config, residue_of, write_config,
};

use crate::commands::{ApiError, Result, data_dirs};
use crate::state::App;

/// The published usage document, or `None` when it could not be read.
///
/// Deliberately not a `Result`: usage decorates a widget list rather than
/// carrying it, so a page that renders without the numbers is a better outcome
/// than one that refuses to render. The state layer logs the reason.
#[tauri::command]
pub async fn widget_usage(app: State<'_, App>) -> std::result::Result<Option<widgets::Usage>, ()> {
    Ok(app.widget_usage().await)
}

/// How long to wait for the runtime to say whether a game is up.
const ENGINE_ANSWER: Duration = Duration::from_millis(500);

/// Whether a game is running, asked without blocking.
///
/// The crate root has a synchronous `engine_running` for the exit handler, and
/// it cannot be reused here: it blocks on the async runtime, which from inside
/// a command — already running on that runtime — panics rather than answers.
///
/// **No answer counts as running.** The cost of being wrong is asymmetric: a
/// refused edit is a button the player presses again in a minute, while an edit
/// made under a live game is silently discarded by `widgetHandler:Shutdown` and
/// looks like modlobby lying about what it did.
async fn game_running(app: &App) -> bool {
    let client = app.client.clone();
    match tokio::time::timeout(ENGINE_ANSWER, client.engine_pid()).await {
        Ok(Ok(pid)) => pid.is_some(),
        Ok(Err(_)) | Err(_) => true,
    }
}

/// What modlobby has installed and what the game's config says.
///
/// The file scan covers every data directory the engine is given, not only
/// modlobby's: a player's own widgets usually live in BAR's directory — often a
/// symlink into a git checkout — and those are exactly the files a same-named
/// published widget collides with.
#[tauri::command]
pub async fn widget_installed(app: State<'_, App>) -> Result<WidgetStatus> {
    let dirs = data_dirs(&app)?;
    let write_dir = content::Library::new(dirs.clone()).write_dir().to_owned();
    let configured = read_config(&write_dir)
        .and_then(|config| config.states().ok())
        .unwrap_or_default();
    let scanned: Vec<(PathBuf, bool)> = std::iter::once((dirs.write.clone(), true))
        .chain(dirs.read.iter().map(|dir| (dir.clone(), false)))
        .collect();
    // Reading a few dozen widget files is disk work, kept off the async runtime.
    let local = tokio::task::spawn_blocking(move || widgets::local::scan(&scanned))
        .await
        .unwrap_or_default();
    Ok(WidgetStatus {
        installed: Ledger::read(&write_dir).widgets.into_values().collect(),
        configured,
        locked: game_running(&app).await,
        write_dir: write_dir.display().to_string(),
        local,
    })
}

/// Download a widget, verify it, and write it into modlobby's write directory.
///
/// Install does **not** enable: BAR adds a new widget to its own config the
/// first time it loads one, and writing an `order` entry here would be guessing
/// at a name the file does not have yet. What install owes the reader is the
/// file and an honest record of having put it there.
#[tauri::command]
pub async fn widget_install(
    app: State<'_, App>,
    key: String,
    name: String,
    install: widgets::Install,
) -> Result<InstalledWidget> {
    let write_dir = write_dir(&app)?;
    let entry = widgets::manage::install(&app.http, &key, &name, &install, &write_dir, now())
        .await
        .map_err(manage_error)?;
    let mut ledger = Ledger::read(&write_dir);
    ledger.widgets.insert(key, entry.clone());
    ledger.write(&write_dir).map_err(manage_error)?;
    tracing::info!(name, files = entry.files.len(), "widget installed");
    Ok(entry)
}

/// Re-download a widget whose published files no longer match what is on disk.
///
/// The same path as install, deliberately: an update is an install over the top,
/// and treating it as a separate operation is how the two drift apart.
#[tauri::command]
pub async fn widget_update(
    app: State<'_, App>,
    key: String,
    name: String,
    install: widgets::Install,
) -> Result<InstalledWidget> {
    widget_install(app, key, name, install).await
}

/// Switch a widget off without losing its settings.
#[tauri::command]
pub async fn widget_disable(app: State<'_, App>, name: String) -> Result<bool> {
    edit_config(&app, |config| {
        config.disable(&name).map_err(|err| {
            ApiError::new(
                if matches!(err, widgets::config::ConfigError::ForcedOn(_)) {
                    "forcedOn"
                } else {
                    "widgetConfig"
                },
                err.to_string(),
            )
        })
    })
    .await
}

/// Switch a widget back on, at the end of the load order.
#[tauri::command]
pub async fn widget_enable(app: State<'_, App>, name: String) -> Result<bool> {
    edit_config(&app, |config| {
        config
            .enable(&name)
            .map_err(|err| ApiError::new("widgetConfig", err.to_string()))
    })
    .await
}

/// Remove a widget: its files, and the settings it left in BAR's config.
///
/// Reports anything else that names it and was **not** removed. Nothing on disk
/// records which widget wrote a given `springsettings.cfg` key, so attributing
/// one is a guess — and a guess that deletes is a bug a player finds weeks
/// later. Residue is shown, not acted on.
#[tauri::command]
pub async fn widget_delete(app: State<'_, App>, key: String) -> Result<Deleted> {
    let write_dir = write_dir(&app)?;
    if game_running(&app).await {
        return Err(ApiError::new(
            "gameRunning",
            "a game is running; BAR rewrites its widget config on exit and would discard this",
        ));
    }
    let mut ledger = Ledger::read(&write_dir);
    let entry = ledger.widgets.get(&key).cloned().ok_or_else(|| {
        ApiError::new(
            "notInstalled",
            "modlobby has no record of installing this widget, so it will not remove files it does not own",
        )
    })?;

    let path = write_dir.join(widgets::config::CONFIG_PATH);
    let mut config = read_config(&write_dir).unwrap_or_else(|| WidgetConfig::parse(String::new()));
    let residue = residue_of(&entry, &write_dir);
    let deleted =
        widgets::manage::delete(&entry, &write_dir, &mut config, residue).map_err(manage_error)?;
    if deleted.config.order_entry || deleted.config.settings {
        write_config(&path, config.as_str()).map_err(manage_error)?;
    }
    ledger.widgets.remove(&key);
    ledger.write(&write_dir).map_err(manage_error)?;
    tracing::info!(
        name = entry.name,
        files = deleted.files.len(),
        residue = deleted.residue.len(),
        "widget deleted"
    );
    Ok(deleted)
}

async fn edit_config(
    app: &App,
    edit: impl FnOnce(&mut WidgetConfig) -> Result<bool>,
) -> Result<bool> {
    let write_dir = write_dir(app)?;
    if game_running(app).await {
        return Err(ApiError::new(
            "gameRunning",
            "a game is running; BAR rewrites its widget config on exit and would discard this",
        ));
    }
    let path = write_dir.join(widgets::config::CONFIG_PATH);
    let mut config = read_config(&write_dir).ok_or_else(|| {
        ApiError::new(
            "noWidgetConfig",
            "BAR has not written its widget config yet; launch a game once first",
        )
    })?;
    let changed = edit(&mut config)?;
    if changed {
        write_config(&path, config.as_str()).map_err(manage_error)?;
    }
    Ok(changed)
}

fn write_dir(app: &App) -> Result<PathBuf> {
    Ok(content::Library::new(data_dirs(app)?)
        .write_dir()
        .to_owned())
}

fn manage_error(err: widgets::manage::ManageError) -> ApiError {
    use widgets::manage::ManageError as E;
    let code = match &err {
        E::GameRunning => "gameRunning",
        E::NotInstallable(..) => "notInstallable",
        E::Download(..) => "network",
        E::Corrupt(_) => "corrupt",
        E::TooLarge(..) => "tooLarge",
        E::Archive(_) | E::EmptyArchive => "archive",
        E::UnsafePath(_) => "unsafePath",
        E::Config(_) => "widgetConfig",
        E::Io(..) => "io",
    };
    ApiError::new(code, err.to_string())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}
