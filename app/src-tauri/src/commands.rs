//! The commands the webview may invoke. Each one is a thin translation to the
//! runtime client or the settings store; errors cross as `{ code, message }`.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use lobby_runtime::{ClientError, launch};
use lobby_ui::UiMessage;
use serde::Serialize;
use settings::{CredentialError, Settings};
use spring_protocol::{Endpoint, LoginRequest};
use tauri::State;
use tauri::ipc::Channel;
use tweaks::{DiffView, Kind, Prepared, Slot, TweakView};

use crate::state::App;
use crate::transport::ChannelTransport;

const LOBBY_VERSION: &str = concat!("modlobby ", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<ClientError> for ApiError {
    fn from(err: ClientError) -> Self {
        let code = match &err {
            ClientError::NotConnected => "notConnected",
            ClientError::AlreadyConnected => "alreadyConnected",
            ClientError::Transport(_) => "transport",
            ClientError::Refused(_) => "refused",
            ClientError::TooLong(_) => "tooLong",
            ClientError::Engine(_) => "engine",
            ClientError::Stopped => "stopped",
        };
        Self::new(code, err.to_string())
    }
}

impl From<settings::Error> for ApiError {
    fn from(err: settings::Error) -> Self {
        Self::new("settings", err.to_string())
    }
}

impl From<CredentialError> for ApiError {
    fn from(err: CredentialError) -> Self {
        Self::new("credentials", err.to_string())
    }
}

impl From<tweaks::Error> for ApiError {
    fn from(err: tweaks::Error) -> Self {
        let code = match &err {
            tweaks::Error::Base64(_) => "base64",
            tweaks::Error::Utf8 => "utf8",
            tweaks::Error::Lua(_) => "lua",
            tweaks::Error::Underscore(_) => "underscore",
        };
        Self::new(code, err.to_string())
    }
}

type Result<T> = std::result::Result<T, ApiError>;

/// Installs the channel the runtime streams into; a snapshot arrives at once.
#[tauri::command]
pub async fn subscribe(app: State<'_, App>, channel: Channel<UiMessage>) -> Result<()> {
    app.client.subscribe(ChannelTransport(channel)).await?;
    Ok(())
}

/// Logs in with the given password, or the remembered one. Resolves when the
/// lobby is ready. `remember` stores the password in the OS keyring, never the file.
#[tauri::command]
pub async fn login(
    app: State<'_, App>,
    username: String,
    password: Option<String>,
    remember: bool,
    auto_login: bool,
) -> Result<()> {
    if username.trim().is_empty() {
        return Err(ApiError::new("input", "a username is required"));
    }
    let password = match password.filter(|p| !p.is_empty()) {
        Some(password) => password,
        None => app
            .credentials
            .get(&username)?
            .ok_or_else(|| ApiError::new("input", "no password given or remembered"))?,
    };
    let server = app.settings.get().server;
    let endpoint = Endpoint {
        host: server.host,
        port: server.port,
        tls: server.tls,
    };
    let request = LoginRequest::new(
        &username,
        &password,
        LOBBY_VERSION,
        app.hardware.lobby_hash.clone(),
    );

    // The server counts logins whether or not they succeed; sending one it
    // would refuse only wastes the allowance.
    if let Some(wait) = app.login_guard.wait(SystemTime::now()) {
        return Err(throttled(wait));
    }
    app.login_guard.record_attempt(SystemTime::now());
    match app.client.login(endpoint, request).await {
        Ok(()) => app.login_guard.record_success(SystemTime::now()),
        Err(err) => {
            if settings::is_flood_refusal(&err.to_string()) {
                app.login_guard.record_refusal(SystemTime::now());
            }
            return Err(err.into());
        }
    }

    if remember {
        app.credentials.set(&username, &password)?;
    } else {
        app.credentials.delete(&username)?;
    }
    app.settings.update(|s| {
        s.account.username = username;
        s.account.remember_password = remember;
        // Without a remembered password there is nothing to log in with.
        s.account.auto_login = auto_login && remember;
    })?;
    Ok(())
}

/// Seconds a login must wait for teiserver's limit to lapse; 0 when clear.
#[tauri::command]
pub fn login_wait(app: State<'_, App>) -> u64 {
    app.login_guard
        .wait(SystemTime::now())
        .map_or(0, |wait| wait.as_secs())
}

fn throttled(wait: Duration) -> ApiError {
    ApiError::new(
        "throttled",
        format!(
            "teiserver allows 3 logins per 10 seconds; waiting {}s",
            wait.as_secs()
        ),
    )
}

#[tauri::command]
pub async fn logout(app: State<'_, App>) -> Result<()> {
    app.client.logout().await?;
    Ok(())
}

#[tauri::command]
pub async fn join_battle(app: State<'_, App>, id: u32, password: Option<String>) -> Result<()> {
    app.client.join_battle(id, password).await?;
    Ok(())
}

#[tauri::command]
pub async fn leave_battle(app: State<'_, App>) -> Result<()> {
    app.client.leave_battle().await?;
    Ok(())
}

/// Connects the engine to the room's game as a spectator, now or when it starts.
#[tauri::command]
pub async fn launch(app: State<'_, App>) -> Result<()> {
    let data_dir = data_dir(&app)?;
    app.client.launch(data_dir).await?;
    Ok(())
}

#[tauri::command]
pub async fn say_battle(app: State<'_, App>, text: String) -> Result<()> {
    app.client.say(text).await?;
    Ok(())
}

#[tauri::command]
pub async fn vote(app: State<'_, App>, choice: String) -> Result<()> {
    if !matches!(choice.as_str(), "y" | "n" | "b") {
        return Err(ApiError::new("input", "vote must be y, n or b"));
    }
    app.client.say(format!("!vote {choice}")).await?;
    Ok(())
}

/// Takes a player slot. The runtime refuses this in a public room — a slot
/// there belongs to someone else — so it only succeeds in a room we were given.
#[tauri::command]
pub async fn take_seat(app: State<'_, App>, team: u8, ally_team: u8) -> Result<()> {
    app.client.take_seat(team, ally_team).await?;
    Ok(())
}

#[tauri::command]
pub async fn release_seat(app: State<'_, App>) -> Result<()> {
    app.client.release_seat().await?;
    Ok(())
}

/// Asks a cluster manager for a room of our own; the runtime joins it when it
/// appears. This is the sandbox where taking a seat is allowed.
#[tauri::command]
pub async fn request_private_host(app: State<'_, App>, region: String) -> Result<String> {
    Ok(app.client.request_private_host(region).await?)
}

#[tauri::command]
pub fn get_settings(app: State<'_, App>) -> Settings {
    app.settings.get()
}

/// Replaces the settings; the file keeps the user's comments and layout.
#[tauri::command]
pub fn update_settings(app: State<'_, App>, settings: Settings) -> Result<Settings> {
    Ok(app.settings.update(|current| *current = settings)?)
}

#[tauri::command]
pub fn has_password(app: State<'_, App>, username: String) -> Result<bool> {
    Ok(app.credentials.get(&username)?.is_some())
}

#[tauri::command]
pub fn set_password(app: State<'_, App>, username: String, password: String) -> Result<()> {
    Ok(app.credentials.set(&username, &password)?)
}

#[tauri::command]
pub fn clear_password(app: State<'_, App>, username: String) -> Result<()> {
    Ok(app.credentials.delete(&username)?)
}

#[tauri::command]
pub fn open_settings_file(app: State<'_, App>) -> Result<()> {
    open(app.settings.path())
}

#[tauri::command]
pub fn open_data_dir(app: State<'_, App>) -> Result<()> {
    open(data_dir(&app)?)
}

/// Decodes a stored slot value for display: Lua, formatted Lua, name, summary.
#[tauri::command]
pub fn tweak_decode(app: State<'_, App>, blob: String, kind: Kind) -> Result<TweakView> {
    Ok(tweaks::decode(&blob, kind, &stylua(&app))?)
}

/// Formats Lua with the user's `stylua.toml`, for the editor's Format action.
#[tauri::command]
pub fn tweak_format(app: State<'_, App>, lua: String, kind: Kind) -> Result<String> {
    Ok(tweaks::lua::format(&lua, kind, &stylua(&app))?)
}

/// Minifies, encodes and measures — the gauge the editor shows. Sends nothing.
#[tauri::command]
pub fn tweak_prepare(lua: String, slot: Slot, direct: bool) -> Result<Prepared> {
    Ok(tweaks::prepare(&lua, slot, direct)?)
}

/// Prepares again here rather than trusting the webview, refuses anything the
/// server would truncate, then says it in the room.
#[tauri::command]
pub async fn tweak_send(
    app: State<'_, App>,
    lua: String,
    slot: Slot,
    direct: bool,
) -> Result<Prepared> {
    let prepared = tweaks::prepare(&lua, slot, direct)?;
    if !prepared.gauge.fits {
        return Err(ApiError::new(
            "tooLong",
            format!(
                "the command is {} characters; the server keeps {}",
                prepared.gauge.command, prepared.gauge.cap
            ),
        ));
    }
    app.client.say(prepared.command.clone()).await?;
    Ok(prepared)
}

/// Clears a slot the way Chobby does, with the literal `0`.
#[tauri::command]
pub async fn tweak_clear(app: State<'_, App>, slot: Slot) -> Result<()> {
    app.client.say(tweaks::command::clear(slot)).await?;
    Ok(())
}

/// Both sides formatted, then diffed — what a vote would change.
#[tauri::command]
pub fn tweak_diff(
    app: State<'_, App>,
    kind: Kind,
    current: String,
    proposed: String,
) -> Result<DiffView> {
    let config = stylua(&app);
    let side = |blob: &str| {
        if blob.is_empty() {
            return String::new();
        }
        tweaks::decode(blob, kind, &config).map_or_else(|_| blob.to_owned(), |view| view.formatted)
    };
    Ok(tweaks::diff::diff(&side(&current), &side(&proposed)))
}

#[tauri::command]
pub fn list_drafts(app: State<'_, App>) -> Result<Vec<String>> {
    let dir = drafts_dir(&app);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut names: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension()? == "lua")
                .then(|| path.file_stem()?.to_str().map(str::to_owned))
                .flatten()
        })
        .collect();
    names.sort();
    Ok(names)
}

#[tauri::command]
pub fn read_draft(app: State<'_, App>, name: String) -> Result<String> {
    let path = draft_path(&app, &name)?;
    std::fs::read_to_string(&path).map_err(|err| ApiError::new("draft", err.to_string()))
}

/// Drafts are plain `.lua` files beside the settings, so they can be edited,
/// backed up and version-controlled like anything else.
#[tauri::command]
pub fn save_draft(app: State<'_, App>, name: String, lua: String) -> Result<()> {
    let path = draft_path(&app, &name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| ApiError::new("draft", err.to_string()))?;
    }
    std::fs::write(&path, lua).map_err(|err| ApiError::new("draft", err.to_string()))
}

#[tauri::command]
pub fn delete_draft(app: State<'_, App>, name: String) -> Result<()> {
    let path = draft_path(&app, &name)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(ApiError::new("draft", err.to_string())),
    }
}

fn stylua(app: &App) -> tweaks::Config {
    let path = app.settings.get().tweaks.stylua_config;
    tweaks::lua::load_config(path.as_deref()).unwrap_or_default()
}

fn drafts_dir(app: &App) -> PathBuf {
    app.settings.dir().join("drafts")
}

/// Keeps a draft name to one path segment; it comes from the webview.
fn draft_path(app: &App, name: &str) -> Result<PathBuf> {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "-_ ".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = safe.trim();
    if safe.is_empty() {
        return Err(ApiError::new("input", "a draft needs a name"));
    }
    Ok(drafts_dir(app).join(format!("{safe}.lua")))
}

fn data_dir(app: &App) -> Result<PathBuf> {
    app.settings
        .get()
        .paths
        .data_dir
        .or_else(launch::default_data_dir)
        .ok_or_else(|| ApiError::new("input", "no BAR data directory; set paths.dataDir"))
}

fn open(path: PathBuf) -> Result<()> {
    tauri_plugin_opener::open_path(path, None::<&str>)
        .map_err(|err| ApiError::new("opener", err.to_string()))
}
