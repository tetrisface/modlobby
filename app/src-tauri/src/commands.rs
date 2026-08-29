//! The commands the webview may invoke. Each one is a thin translation to the
//! runtime client or the settings store; errors cross as `{ code, message }`.

use std::path::PathBuf;

use lobby_runtime::{ClientError, launch};
use lobby_ui::UiMessage;
use serde::Serialize;
use settings::{CredentialError, Settings};
use spring_protocol::{Endpoint, LoginRequest};
use tauri::State;
use tauri::ipc::Channel;

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
    app.client.login(endpoint, request).await?;

    if remember {
        app.credentials.set(&username, &password)?;
    } else {
        app.credentials.delete(&username)?;
    }
    app.settings.update(|s| {
        s.account.username = username;
        s.account.remember_password = remember;
    })?;
    Ok(())
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
