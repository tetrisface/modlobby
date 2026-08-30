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
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
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

pub(crate) type Result<T> = std::result::Result<T, ApiError>;

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
    // Remembered only once the host has let us in, so a room that refused us
    // is never offered back.
    app.rejoin.remember(id);
    Ok(())
}

#[tauri::command]
pub async fn leave_battle(app: State<'_, App>) -> Result<()> {
    app.client.leave_battle().await?;
    // Leaving on purpose is the one case where we should not be asked about it
    // again; a crash never gets here, which is exactly the point.
    app.rejoin.forget();
    Ok(())
}

/// The room we were in when the app last stopped, if it stopped without
/// leaving. The caller checks it is still open before offering it.
#[tauri::command]
pub fn remembered_battle(app: State<'_, App>) -> Option<u32> {
    app.rejoin.remembered()
}

/// Drops the offer without joining anything.
#[tauri::command]
pub fn forget_battle(app: State<'_, App>) {
    app.rejoin.forget();
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
pub async fn join_channel(app: State<'_, App>, room: String, key: Option<String>) -> Result<()> {
    app.client.join_channel(room, key).await?;
    Ok(())
}

#[tauri::command]
pub async fn leave_channel(app: State<'_, App>, room: String) -> Result<()> {
    app.client.leave_channel(room).await?;
    Ok(())
}

#[tauri::command]
pub async fn say_channel(app: State<'_, App>, room: String, text: String) -> Result<()> {
    app.client.say_channel(room, text).await?;
    Ok(())
}

#[tauri::command]
pub async fn say_private(app: State<'_, App>, user: String, text: String) -> Result<()> {
    app.client.say_private(user, text).await?;
    Ok(())
}

/// Asks for the server's channel directory; it arrives as a `Directory` delta.
#[tauri::command]
pub async fn list_channels(app: State<'_, App>) -> Result<()> {
    app.client.list_channels().await?;
    Ok(())
}

/// What this machine can start a game with.
#[tauri::command]
pub fn skirmish_options(app: State<'_, App>) -> Result<SkirmishOptions> {
    let data_dir = data_dir(&app)?;
    let library = content::Library::new(&data_dir);
    Ok(SkirmishOptions {
        games: library.installed_games(),
        maps: library.installed_map_files(),
        engines: recoil::installed_engines(&data_dir),
        ais: recoil::installed_ais(&data_dir),
    })
}

/// Starts a game against AI with no server involved.
#[tauri::command]
pub async fn start_skirmish(
    app: State<'_, App>,
    game: String,
    map: String,
    engine: String,
    opponents: Vec<String>,
) -> Result<()> {
    let data_dir = data_dir(&app)?;
    if game.is_empty() || map.is_empty() || engine.is_empty() {
        return Err(ApiError::new("input", "pick a game, a map and an engine"));
    }
    // A skirmish needs no account, so someone who has never logged in still
    // needs a name to appear under.
    let username = app.settings.get().account.username;
    let player = if username.trim().is_empty() {
        "Player".to_owned()
    } else {
        username
    };

    let skirmish = recoil::script::Skirmish {
        game,
        map,
        player,
        start_pos: recoil::script::StartPos::InGame,
        opponents: opponents
            .into_iter()
            .enumerate()
            .map(|(index, short_name)| recoil::script::Ai {
                name: format!("{short_name} {}", index + 1),
                short_name,
            })
            .collect(),
        modoptions: Vec::new(),
    };
    app.client
        .start_skirmish(data_dir, engine, skirmish)
        .await?;
    Ok(())
}

/// Says whether we are ready to start. Only a player can be.
#[tauri::command]
pub async fn set_ready(app: State<'_, App>, ready: bool) -> Result<()> {
    app.client.set_ready(ready).await?;
    Ok(())
}

/// Picks a faction: 0 Armada, 1 Cortex, 2 Random, 3 Legion.
#[tauri::command]
pub async fn set_side(app: State<'_, App>, side: u8) -> Result<()> {
    app.client.set_side(side).await?;
    Ok(())
}

/// Every replay in the BAR data directory, newest first.
#[tauri::command]
pub fn list_replays(app: State<'_, App>) -> Result<Vec<ReplayView>> {
    let data_dir = data_dir(&app)?;
    Ok(content::replays::list(&data_dir)
        .into_iter()
        .map(ReplayView::from)
        .collect())
}

/// Plays a replay. The engine takes a demo file where it would take a
/// `spring://` URL, so this is the launch path with a different target.
#[tauri::command]
pub async fn play_replay(app: State<'_, App>, path: String) -> Result<()> {
    let data_dir = data_dir(&app)?;
    let replay = std::path::PathBuf::from(&path);
    // Only from the directory we listed: a path from the front end is not a
    // reason to hand the engine anything on the disk.
    if replay.parent() != Some(&data_dir.join("demos")) || !replay.is_file() {
        return Err(ApiError::new(
            "input",
            "not a replay in the demos directory",
        ));
    }
    app.client.play_replay(data_dir, path).await?;
    Ok(())
}

/// Fetches whatever the current room needs and this machine does not have.
/// Progress arrives as `Download` deltas.
#[tauri::command]
pub async fn download_missing(app: State<'_, App>) -> Result<()> {
    app.client.download_missing().await?;
    Ok(())
}

/// Asks a room's host how long its game has been going. The answer comes back
/// as a `GameStartedAgo` delta, because SPADS replies by private message.
#[tauri::command]
pub async fn request_game_status(app: State<'_, App>, founder: String) -> Result<()> {
    app.client.request_game_status(founder).await?;
    Ok(())
}

/// Records whether the last room was joined as a player, which is what the
/// `remember` join posture remembers. Written on its own so taking a seat does
/// not rewrite every other setting.
#[tauri::command]
pub fn remember_played(app: State<'_, App>, played: bool) -> Result<Settings> {
    Ok(app
        .settings
        .update(|current| current.play.last_was_player = played)?)
}

/// Flashes the taskbar entry of the running engine, if it has a window yet.
///
/// Answers whether it did, so the caller can fall back to flashing the lobby:
/// when a game is starting the engine may not have opened a window, and when
/// one has ended it may already be gone.
#[tauri::command]
pub async fn flash_engine(app: State<'_, App>) -> Result<bool> {
    let Some(pid) = app.client.engine_pid().await? else {
        return Ok(false);
    };
    Ok(crate::flash::flash_process(pid))
}

/// Marks us away, or back.
#[tauri::command]
pub async fn set_away(app: State<'_, App>, away: bool) -> Result<()> {
    app.client.set_away(away).await?;
    Ok(())
}

/// Rings someone in the room, which is how a host says the game is waiting.
#[tauri::command]
pub async fn ring(app: State<'_, App>, user: String) -> Result<()> {
    app.client.ring(user).await?;
    Ok(())
}

/// Stops the running download. What it already wrote stays on disk, and
/// pr-downloader picks up from there when it is asked again.
#[tauri::command]
pub async fn stop_download(app: State<'_, App>) -> Result<()> {
    app.client.stop_download().await?;
    Ok(())
}

/// Asks the server for the friend list and the pending requests.
#[tauri::command]
pub async fn refresh_friends(app: State<'_, App>) -> Result<()> {
    app.client.refresh_friends().await?;
    Ok(())
}

/// `request`, `accept`, `decline` or `remove`. The server announces nothing
/// when a friendship changes, so the runtime asks for the listings afterwards.
#[tauri::command]
pub async fn friend_action(app: State<'_, App>, action: String, user: String) -> Result<()> {
    let action: lobby_runtime::FriendAction =
        action
            .parse()
            .map_err(|err: lobby_runtime::UnknownFriendAction| {
                ApiError::new("input", err.to_string())
            })?;
    app.client.friend_action(action, user).await?;
    Ok(())
}

/// Sets one modoption: `!bSet <key> <value>`, in Chobby's casing.
///
/// SPADS decides what happens next, not us. `[bSet]` is granted as
/// `battle,pv:player:stopped|100:0` (`commands_default.conf`), so a player
/// below level 100 has this auto-converted into a vote by `autoCallvote`, and
/// a spectator is refused outright. The refusal arrives as chat, which is why
/// nothing here tries to predict it.
#[tauri::command]
pub async fn set_option(app: State<'_, App>, key: String, value: String) -> Result<()> {
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ApiError::new("input", "a modoption key is alphanumeric"));
    }
    // SPADS' own value pattern for a preset setting is `[A-Za-z0-9\-\_]*`
    // (`battlePresets.conf`); anything else it will not accept anyway.
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(ApiError::new(
            "input",
            "a modoption value is alphanumeric, `-`, `_` or `.`",
        ));
    }

    let command = format!("!bSet {key} {value}");
    if command.len() > spring_protocol::policy::saybattle_max_len(&command) {
        return Err(ApiError::new("tooLong", "the command is too long to send"));
    }
    app.client.say(command).await?;
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

/// Joins an empty public autohost in a region, which makes it your room.
#[tauri::command]
pub async fn host_public(app: State<'_, App>, region: String) -> Result<u32> {
    Ok(app.client.host_public(region).await?)
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

/// Records which channels to rejoin next time. Written on its own rather than
/// through the whole settings object, so a join never races a setting the user
/// is editing in the file at the same moment.
#[tauri::command]
pub fn remember_channels(app: State<'_, App>, channels: Vec<String>) -> Result<Settings> {
    Ok(app
        .settings
        .update(|current| current.chat.channels = channels)?)
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

/// The webview's console, written into the same file as everything else so a
/// UI error and the protocol traffic around it sit on one timeline.
#[tauri::command]
pub fn log_message(level: String, message: String) {
    match level.as_str() {
        "error" => tracing::error!(target: "webview", "{message}"),
        "warn" => tracing::warn!(target: "webview", "{message}"),
        "debug" => tracing::debug!(target: "webview", "{message}"),
        _ => tracing::info!(target: "webview", "{message}"),
    }
}

#[tauri::command]
pub fn open_log_dir(app: State<'_, App>) -> Result<()> {
    open(app.settings.dir().join("logs"))
}

#[tauri::command]
pub fn open_settings_file(app: State<'_, App>) -> Result<()> {
    open(app.settings.path())
}

#[tauri::command]
pub fn open_data_dir(app: State<'_, App>) -> Result<()> {
    open(data_dir(&app)?)
}

/// Opens a link from chat in the system browser.
///
/// Only `http` and `https`: chat is text other people wrote, and handing an
/// arbitrary scheme to the shell would let anyone in a channel decide what
/// this machine opens.
#[tauri::command]
pub fn open_url(url: String) -> Result<()> {
    let allowed = url.starts_with("https://") || url.starts_with("http://");
    // Quotes and control characters have no business in a URL and are how a
    // crafted line would try to break out of whatever opens it.
    let suspicious = url.chars().any(char::is_control) || url.contains('"') || url.contains("'");
    if !allowed || suspicious {
        return Err(ApiError::new("input", "only http and https links open"));
    }
    tauri_plugin_opener::open_url(url, None::<&str>)
        .map_err(|err| ApiError::new("opener", err.to_string()))
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

/// A replay as the front end lists it.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReplayView {
    pub path: String,
    pub played_at: String,
    pub map: String,
    pub engine: String,
    #[ts(type = "number")]
    pub bytes: u64,
}

impl From<content::replays::Replay> for ReplayView {
    fn from(replay: content::replays::Replay) -> Self {
        Self {
            path: replay.path.to_string_lossy().into_owned(),
            played_at: replay.played_at,
            map: replay.map,
            engine: replay.engine,
            bytes: replay.bytes,
        }
    }
}

/// What a skirmish can be built from on this machine.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SkirmishOptions {
    pub games: Vec<String>,
    /// Archive file names, lowercased and underscored.
    pub maps: Vec<String>,
    pub engines: Vec<String>,
    pub ais: Vec<String>,
}
