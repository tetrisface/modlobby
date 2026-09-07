//! The commands the webview may invoke. Each one is a thin translation to the
//! runtime client or the settings store; errors cross as `{ code, message }`.

use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use lobby_runtime::{ClientError, launch, player_files};
use lobby_ui::UiMessage;
use serde::Serialize;
use settings::{CredentialError, Settings};
use spring_protocol::{Endpoint, LoginRequest};
use tauri::State;
use tauri::ipc::Channel;
use tweaks::{DiffView, Kind, Prepared, Slot, TweakView};

use crate::state::App;
use crate::transport::ChannelTransport;

const LOBBY_VERSION: &str = env!("CARGO_PKG_VERSION");

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
            ClientError::NoCredentials => "noCredentials",
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
pub async fn subscribe(
    app: State<'_, App>,
    overlay: State<'_, std::sync::Arc<crate::overlay::Controller>>,
    channel: Channel<UiMessage>,
) -> Result<()> {
    // The front end is up and asking: the last of the startup milestones.
    tracing::debug!(ms = crate::since_start(), "startup: front end subscribed");
    app.client
        .subscribe(ChannelTransport {
            channel,
            overlay: overlay.inner().clone(),
        })
        .await?;
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

    guarded_login(&app, app.client.login(endpoint, request)).await?;

    remember_account(&app, username, &password, remember, auto_login)
}

/// Keeps the account a session was just opened as: the password in the OS
/// keyring, the rest beside it in the settings file.
///
/// Shared by logging in and by confirming a new account's code, because both
/// end in a session and there is one thing worth remembering about either.
/// Written only after the server has said yes, so a failed attempt leaves
/// nothing behind.
fn remember_account(
    app: &App,
    username: String,
    password: &str,
    remember: bool,
    auto_login: bool,
) -> Result<()> {
    if remember {
        app.credentials.set(&username, password)?;
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

/// Tries the last login again, now rather than when the runtime's own retry
/// falls due. Fails with `noCredentials` when this run has nothing to try.
#[tauri::command]
pub async fn reconnect(app: State<'_, App>) -> Result<()> {
    guarded_login(&app, app.client.reconnect()).await
}

/// One login attempt under the flood guard, whichever command makes it. The
/// server counts logins whether or not they succeed; sending one it would
/// refuse only wastes the allowance.
async fn guarded_login(
    app: &App,
    attempt: impl Future<Output = std::result::Result<(), ClientError>>,
) -> Result<()> {
    if let Some(wait) = app.login_guard.wait(SystemTime::now()) {
        return Err(throttled(wait));
    }
    app.login_guard.record_attempt(SystemTime::now());
    match attempt.await {
        Ok(()) => {
            app.login_guard.record_success(SystemTime::now());
            Ok(())
        }
        Err(err) => {
            if settings::is_flood_refusal(&err.to_string()) {
                app.login_guard.record_refusal(SystemTime::now());
            }
            Err(err.into())
        }
    }
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

/// Creates an account and logs in on it, answering with the user agreement.
///
/// A new account is unverified: the server emails a code, and answers the
/// login that follows with its agreement rather than with a session.
/// [`confirm_agreement`] finishes the job, on the connection this leaves open.
#[tauri::command]
pub async fn register(
    app: State<'_, App>,
    username: String,
    password: String,
    email: String,
) -> Result<Vec<String>> {
    if let Some(problem) = spring_protocol::login::name_problem(&username) {
        return Err(ApiError::new("input", problem));
    }
    if password.is_empty() {
        return Err(ApiError::new("input", "a password is required"));
    }
    if !email.contains('@') || email.contains(' ') {
        return Err(ApiError::new("input", "an email address is required"));
    }

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

    // No login guard here. `REGISTER` never reaches teiserver's flood check —
    // `spring_in.ex:335` goes straight to `CacheUser.register_user_with_md5`
    // — and that check is keyed by account, so the login that follows starts
    // from zero on an account that did not exist a moment ago. Counting it
    // here throttled nothing but that login.
    Ok(app
        .client
        .register(endpoint, request, email, password)
        .await?)
}

/// Sends the emailed code, which is what finishes a new account's first login.
///
/// The server verifies the account and completes the login on the same
/// connection (`spring_in.ex:353` ends in `do_login_accepted`), so this
/// resolves the way a login does — and, like a login, it is where the account
/// becomes the one this machine remembers.
#[tauri::command]
pub async fn confirm_agreement(
    app: State<'_, App>,
    username: String,
    password: String,
    code: String,
    remember: bool,
    auto_login: bool,
) -> Result<()> {
    if code.trim().is_empty() {
        return Err(ApiError::new("input", "the emailed code is required"));
    }
    app.client.confirm_agreement(code.trim().to_owned()).await?;
    remember_account(&app, username, &password, remember, auto_login)
}

/// Why a username would be refused, answered without asking the server.
///
/// The rules are teiserver's own and purely mechanical, so a typo costs
/// neither a round trip nor one of the three logins it allows per ten seconds.
/// Whether the name is taken only the server can say, and it does.
#[tauri::command]
pub fn name_problem(username: String) -> Option<String> {
    spring_protocol::login::name_problem(&username)
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
    app.client.launch(data_dirs(&app)?).await?;
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
    let library = content::Library::new(data_dirs(&app)?);
    Ok(SkirmishOptions {
        games: library.installed_games(),
        maps: library.installed_map_files(),
        engines: library.installed_engines(),
        ais: library.installed_ais(),
    })
}

/// What an engine AI declares it can be told.
///
/// `AIOptions.lua` is the same `local options = { … }` table a game's
/// `modoptions.lua` is, so the same parser reads it and the same rows draw it.
/// An empty answer means the AI ships no options, or is not installed for the
/// engine asked about — neither is a failure.
#[tauri::command]
pub async fn ai_options(
    app: State<'_, App>,
    engine: String,
    ai: String,
) -> Result<Vec<modoptions::ModOption>> {
    let Some(dirs) = data_dirs_of(&app) else {
        return Ok(Vec::new());
    };
    disk_work(move || {
        let Some(path) = content::Library::new(dirs).find_ai_options(&engine, &ai) else {
            return Ok(Vec::new());
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(Vec::new());
        };
        modoptions::parse(&text).map_err(|err| ApiError::new("aiOptions", err.to_string()))
    })
    .await?
}

/// Where the skirmish room is kept between runs.
pub fn skirmish_path(app: &App) -> std::path::PathBuf {
    app.settings.dir().join("skirmish.json")
}

/// The room as it was left last time.
///
/// A file that will not parse is ignored rather than complained about: it is a
/// convenience, the room it describes is one nobody has asked for yet, and the
/// next change overwrites it.
fn remembered_skirmish(app: &State<'_, App>) -> Option<skirmish::Room> {
    let text = std::fs::read_to_string(skirmish_path(app)).ok()?;
    serde_json::from_str(&text)
        .inspect_err(|err| tracing::debug!(%err, "the kept skirmish room could not be read"))
        .ok()
}

/// A map's spring name, which is what a start script names it by.
///
/// What is on disk is the archive's file name -- lowercased and underscored --
/// and nothing there records the capitalisation, so it comes from BAR's
/// published index, the same one the minimaps do. Offline the file name is
/// used as-is: it is the best guess there is, and it is what the old skirmish
/// form did.
async fn spring_name(app: &State<'_, App>, map: String) -> String {
    if map.is_empty() {
        return map;
    }
    let index = app.map_index().await;
    index.names.get(&map).cloned().unwrap_or(map)
}

/// The name to play under.
///
/// A skirmish needs no account, so someone who has never logged in still needs
/// something to appear as.
fn player_name(app: &State<'_, App>) -> String {
    let username = app.settings.get().account.username;
    if username.trim().is_empty() {
        "Player".to_owned()
    } else {
        username
    }
}

/// Opens the room with no server behind it, on whatever this machine has.
///
/// The newest of each is the useful guess: it is what a person who installed
/// BAR yesterday wants, and every part of it can be changed in the room.
#[tauri::command]
pub async fn skirmish_open(
    app: State<'_, App>,
    game: Option<String>,
    map: Option<String>,
    engine: Option<String>,
) -> Result<()> {
    // The one left from last time, if nothing has been asked for and nothing
    // is open. Read here rather than in the runtime so that restoring and
    // building a fresh one are the same decision, made once.
    if game.is_none() && map.is_none() && engine.is_none() {
        if app.client.skirmish_room().await?.is_some() {
            return Ok(());
        }
        if let Some(room) = remembered_skirmish(&app) {
            app.client.open_skirmish(room).await?;
            return Ok(());
        }
    }
    let library = content::Library::new(data_dirs(&app)?);
    // The newest, which is what both lists are ordered by and what somebody
    // who installed BAR yesterday wants. Every part of it can be changed in
    // the room afterwards.
    let newest = |asked: Option<String>, from: Vec<String>| {
        asked
            .filter(|asked| !asked.is_empty())
            .or_else(|| from.into_iter().next())
            .unwrap_or_default()
    };
    let room = skirmish::Room::new(
        player_name(&app),
        newest(game, library.installed_games()),
        // Maps have no newest; the list is alphabetical and the first of it is
        // at least the same one every time.
        spring_name(&app, newest(map, library.installed_map_files())).await,
        newest(engine, library.installed_engines()),
    );
    app.client.open_skirmish(room).await?;
    Ok(())
}

#[tauri::command]
pub async fn skirmish_close(app: State<'_, App>) -> Result<()> {
    app.client.close_skirmish().await?;
    Ok(())
}

/// One change to that room. Every way it can change comes through here, which
/// is what makes its log a complete account of what happened to it.
#[tauri::command]
pub async fn skirmish_act(app: State<'_, App>, act: skirmish::Act) -> Result<()> {
    app.client.skirmish(act).await?;
    Ok(())
}

/// Writes its script and starts the engine on it.
#[tauri::command]
pub async fn skirmish_launch(app: State<'_, App>) -> Result<()> {
    app.client.launch_skirmish().await?;
    Ok(())
}

/// Fetches whatever of the skirmish room's engine, game and map is missing.
#[tauri::command]
pub async fn skirmish_download_missing(app: State<'_, App>) -> Result<()> {
    app.client.skirmish_download().await?;
    Ok(())
}

/// Puts a tweak in one of the room's slots.
///
/// Prepared exactly as it would be for a room on the server, then handed to
/// the same `!bSet` the console takes — so what the editor copies to the
/// clipboard and what it sends here are one command, not two code paths.
///
/// The length gauge is not enforced. Its cap is the server's chat limit, and
/// there is no chat here: a tweak too big to say in a room still fits in a
/// start script perfectly well.
#[tauri::command]
pub async fn skirmish_tweak_send(
    app: State<'_, App>,
    lua: String,
    slot: Slot,
    direct: bool,
) -> Result<Prepared> {
    let prepared = tweaks::prepare(&lua, slot, direct)?;
    app.client
        .skirmish(skirmish::Act::Say {
            text: prepared.command.clone(),
        })
        .await?;
    Ok(prepared)
}

/// Clears a slot the way Chobby does, with the literal `0`.
#[tauri::command]
pub async fn skirmish_tweak_clear(app: State<'_, App>, slot: Slot) -> Result<()> {
    app.client
        .skirmish(skirmish::Act::Say {
            text: tweaks::command::clear(slot),
        })
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

/// Every replay in any BAR data directory, newest first.
#[tauri::command]
pub fn list_replays(app: State<'_, App>) -> Result<Vec<ReplayView>> {
    Ok(content::Library::new(data_dirs(&app)?)
        .replays()
        .into_iter()
        .map(ReplayView::from)
        .collect())
}

/// Plays a replay. The engine takes a demo file where it would take a
/// `spring://` URL, so this is the launch path with a different target.
#[tauri::command]
pub async fn play_replay(app: State<'_, App>, path: String) -> Result<()> {
    let dirs = data_dirs(&app)?;
    let replay = std::path::PathBuf::from(&path);
    // Only from a directory we listed: a path from the front end is not a
    // reason to hand the engine anything on the disk.
    let listed = dirs
        .all()
        .any(|dir| replay.parent() == Some(dir.join("demos").as_path()));
    if !listed || !replay.is_file() {
        return Err(ApiError::new(
            "input",
            "not a replay in the demos directory",
        ));
    }
    app.client.play_replay(dirs, path).await?;
    Ok(())
}

/// Fetches whatever the current room needs and this machine does not have.
/// Progress arrives as `Download` deltas.
#[tauri::command]
pub async fn download_missing(app: State<'_, App>) -> Result<()> {
    app.client.download_missing().await?;
    Ok(())
}

/// Looks at what is installed again, for an engine that arrived from outside
/// this app.
///
/// Not the same as [`download_missing`], which fetches with pr-downloader out
/// of the named engine and so can do nothing at all when the engine is what is
/// missing. The answer is cached against the room's engine, game and map
/// because it comes of scanning the rapid index, which is far too slow to
/// repeat per click; a bundle dropped into the engine folder by hand changes
/// none of those three names, so nothing invalidates the cache and only being
/// asked finds it.
#[tauri::command]
pub async fn recheck_content(app: State<'_, App>) -> Result<()> {
    app.client.recheck_content().await?;
    Ok(())
}

/// BAR's map index: each map's picture and its spring name.
#[tauri::command]
pub async fn map_index(app: State<'_, App>) -> Result<content::map_index::MapIndex> {
    Ok(app.map_index().await)
}

/// BAR's news, and how much of it has turned up since it was last looked at.
///
/// Both in one answer so the count on the tab and the page behind it cannot
/// disagree about what the feed held.
#[tauri::command]
pub async fn news(app: State<'_, App>) -> Result<news::NewsFeed> {
    let items = app.news().await;
    let unread = app.news_read.unread(news::SOURCE, &items);
    Ok(news::NewsFeed { items, unread })
}

/// Remembers what the list showed, which is what clears the count.
#[tauri::command]
pub async fn mark_news_read(app: State<'_, App>) -> Result<()> {
    app.news_read.mark_read(news::SOURCE, &app.news().await);
    Ok(())
}

/// Makes the pictures of `maps` at `tiles` ahead of time, in the order given,
/// so a room joined from the list shows its map at once rather than after a
/// download and a decode. One worker, so it never takes more than a core; a
/// map the index has no picture for is left out. See `content::map_thumb`.
#[tauri::command]
pub async fn warm_map_pictures(
    app: State<'_, App>,
    maps: Vec<String>,
    tiles: Vec<content::map_thumb::Tile>,
) -> Result<()> {
    let index = app.map_index().await;
    let mut seen = std::collections::HashSet::new();
    let jobs = maps
        .iter()
        .filter_map(|name| index.images.get(name))
        .filter(|url| seen.insert(url.as_str()))
        .map(|url| content::map_thumb::Job {
            url: url.clone(),
            tiles: tiles.clone(),
        })
        .collect();
    app.thumbs.warm(jobs);
    Ok(())
}

/// Asks a room's host how long its game has been going. The answer comes back
/// as a `GameStartedAgo` delta, because SPADS replies by private message.
#[tauri::command]
pub async fn request_game_status(app: State<'_, App>, founder: String) -> Result<()> {
    app.client.request_game_status(founder).await?;
    Ok(())
}

/// BAR's modoption table, read out of the installed game.
///
/// Not shipped with the app. The names and descriptions in `modoptions.lua`
/// are BAR's writing under GPL v2, and every player already has the file — so
/// this reads theirs rather than redistributing a copy, which is what
/// bar-lobby and Chobby both do. It also means the table always matches the
/// game version the room is actually running.
///
/// An empty answer means the game is not installed yet; the room falls back to
/// showing the settings it can see without their descriptions.
#[tauri::command]
pub async fn game_modoptions(
    app: State<'_, App>,
    game: String,
) -> Result<Vec<modoptions::ModOption>> {
    let Some(dirs) = data_dirs_of(&app) else {
        return Ok(Vec::new());
    };
    let cache = app.game_files.clone();
    disk_work(move || {
        let library = content::Library::new(dirs);
        let Some(bytes) = cache.game_file(&library, &game, "modoptions.lua") else {
            return Ok(Vec::new());
        };
        modoptions::parse(&String::from_utf8_lossy(&bytes))
            .map_err(|err| ApiError::new("modoptions", err.to_string()))
    })
    .await?
}

/// An AI a room can be given.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiChoice {
    /// What `ADDBOT` names it: an engine AI's directory, or a Lua AI's `name`.
    pub name: String,
    /// The game's one-line description; empty for engine AIs.
    pub desc: String,
}

/// The AIs a room running `game` can be given: what the installed engines
/// ship, then what the game implements in Lua.
///
/// The engine's `AI/Skirmish/` only holds the AIs compiled against it (BARb,
/// NullAI…). BAR's Scavengers and Raptors are Lua AIs, declared by the game in
/// its `luaai.lua`; that is read out of the installed game the way
/// `game_modoptions` reads its option table. Until the game is installed only
/// the engine's list is offered, and a `luaai.lua` this cannot read is logged
/// rather than allowed to hide the engine's AIs.
#[tauri::command]
pub async fn game_ais(app: State<'_, App>, game: String) -> Result<Vec<AiChoice>> {
    let Some(dirs) = data_dirs_of(&app) else {
        return Ok(Vec::new());
    };
    let cache = app.game_files.clone();
    disk_work(move || {
        let library = content::Library::new(dirs);
        let mut choices: Vec<AiChoice> = library
            .installed_ais()
            .into_iter()
            .map(|name| AiChoice {
                name,
                desc: String::new(),
            })
            .collect();

        let Some(bytes) = cache.game_file(&library, &game, "luaai.lua") else {
            return choices;
        };
        let lua_ais = match modoptions::luaai::parse(&String::from_utf8_lossy(&bytes)) {
            Ok(ais) => ais,
            Err(err) => {
                tracing::warn!(%game, %err, "the game's luaai.lua could not be read");
                return choices;
            }
        };
        for ai in lua_ais {
            if choices.iter().all(|choice| choice.name != ai.name) {
                choices.push(AiChoice {
                    name: ai.name,
                    desc: ai.desc,
                });
            }
        }
        choices
    })
    .await
}

/// Runs a read of the installed game off the main thread. Opening a rapid
/// package costs tens of milliseconds, and a sync command pays them on the
/// thread that paints the window.
async fn disk_work<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|err| ApiError::new("disk", err.to_string()))
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

/// Whether the window is currently sitting over a running game.
#[tauri::command]
pub fn overlay_active(overlay: State<'_, std::sync::Arc<crate::overlay::Controller>>) -> bool {
    overlay.is_over()
}

/// Stops the running game, leaving the lobby up. Answers whether there was one.
#[tauri::command]
pub async fn stop_game(app: State<'_, App>) -> Result<bool> {
    Ok(app.client.stop_engine().await?)
}

/// Ends the game and closes modlobby, which is what a game's own Quit does.
///
/// The engine is stopped first and its exit is not waited for: the process is
/// already going away, and waiting would only risk hanging on a game that is
/// slow to die.
#[tauri::command]
pub async fn quit_all(app: State<'_, App>, handle: tauri::AppHandle) -> Result<()> {
    let _ = app.client.stop_engine().await;
    handle.exit(0);
    Ok(())
}

/// Closes the lobby, leaving a running game alone.
///
/// Different from [`quit_all`] on purpose: the game is its own process and
/// closing the lobby window is not a statement about it — the same bargain
/// every other lobby offers.
#[tauri::command]
pub fn shutdown(handle: tauri::AppHandle) {
    handle.exit(0);
}

/// The same thing the hotkey does, for people who would rather click.
#[tauri::command]
pub fn overlay_toggle(overlay: State<'_, std::sync::Arc<crate::overlay::Controller>>) {
    overlay.hotkey();
}

/// Marks us away, or back.
#[tauri::command]
pub async fn set_away(app: State<'_, App>, away: bool) -> Result<()> {
    app.client.set_away(away).await?;
    Ok(())
}

/// Someone touched the window; the idle disconnect counts from here.
#[tauri::command]
pub async fn activity(app: State<'_, App>) -> Result<()> {
    app.client.activity().await?;
    Ok(())
}

/// Adds an AI to the room. It runs on this machine when the game starts.
#[tauri::command]
pub async fn add_bot(
    app: State<'_, App>,
    name: String,
    ai: String,
    team: u8,
    ally_team: u8,
    colour: u32,
) -> Result<()> {
    app.client
        .add_bot(name, ai, team, ally_team, colour)
        .await?;
    Ok(())
}

/// Moves one of our AIs, or changes its bonus, colour or faction.
///
/// Only for an AI we added: the server drops the message for anyone else's,
/// silently, so the caller asks the host in chat instead.
#[tauri::command]
pub async fn update_bot(
    app: State<'_, App>,
    name: String,
    team: u8,
    ally_team: u8,
    handicap: u8,
    colour: u32,
) -> Result<()> {
    app.client
        .update_bot(name, team, ally_team, handicap, colour)
        .await?;
    Ok(())
}

/// Removes an AI by name; whether we may is the server's call.
#[tauri::command]
pub async fn remove_bot(app: State<'_, App>, name: String) -> Result<()> {
    app.client.remove_bot(name).await?;
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

/// Stops a paste. What has not left the queue is dropped; what has is the
/// host's already.
#[tauri::command]
pub async fn cancel_paste(app: State<'_, App>) -> Result<()> {
    app.client.cancel_paste().await?;
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
pub async fn request_private_host(app: State<'_, App>) -> Result<String> {
    Ok(app.client.request_private_host().await?)
}

/// Joins an empty public autohost, which makes it your room. The runtime
/// picks one by latency and by which cluster has rooms to spare.
#[tauri::command]
pub async fn host_public(app: State<'_, App>) -> Result<u32> {
    Ok(app.client.host_public().await?)
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
    let dirs = data_dirs(&app)?;
    // Opening a directory that is not there yet opens nothing; make it.
    std::fs::create_dir_all(&dirs.write)
        .map_err(|err| ApiError::new("io", format!("making the data directory: {err}")))?;
    open(dirs.write)
}

/// The folder an engine is dropped into, for the machine where that is how one
/// arrives.
///
/// Made if it is not there, for the same reason [`open_data_dir`] makes its
/// own: a message that names a folder and then opens nothing is worse than no
/// button at all, and on a machine that has never had an engine nothing has
/// created it yet.
#[tauri::command]
pub fn open_engine_dir(app: State<'_, App>) -> Result<()> {
    let engine = data_dirs(&app)?.write.join("engine");
    std::fs::create_dir_all(&engine)
        .map_err(|err| ApiError::new("io", format!("making the engine directory: {err}")))?;
    open(engine)
}

/// The player's files — engine settings, hotkeys, widget state — as the
/// Settings page shows them: where they can be copied from, and the copies
/// taken before each launch.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayerFilesView {
    /// The data directory the engine writes, where the files live.
    pub write: String,
    /// Other installs on this machine whose settings are worth copying.
    pub sources: Vec<String>,
    /// Snapshot directories, newest first; each is named for when it was
    /// taken, in UTC.
    pub snapshots: Vec<String>,
}

fn shown(paths: Vec<PathBuf>) -> Vec<String> {
    paths
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

#[tauri::command]
pub async fn player_files(app: State<'_, App>) -> Result<PlayerFilesView> {
    let dirs = data_dirs(&app)?;
    disk_work(move || PlayerFilesView {
        write: dirs.write.to_string_lossy().into_owned(),
        sources: shown(player_files::sources(&dirs)),
        snapshots: shown(player_files::snapshots(&dirs.write)),
    })
    .await
}

/// Copies another install's player files over ours, or puts a snapshot back,
/// after snapshotting what is there now. `from` must be one of the places
/// [`player_files`] offers. Refused while a game runs, since the engine
/// rewrites these files as it exits. How many files were copied.
#[tauri::command]
pub async fn import_player_files(app: State<'_, App>, from: String) -> Result<u32> {
    let dirs = data_dirs(&app)?;
    if app.client.engine_pid().await?.is_some() {
        return Err(ApiError::new(
            "engine",
            "wait for the game to end; it writes these files as it exits",
        ));
    }
    let from = PathBuf::from(from);
    disk_work(move || {
        let offered = player_files::sources(&dirs)
            .into_iter()
            .chain(player_files::snapshots(&dirs.write))
            .any(|dir| dir == from);
        if !offered {
            return Err(ApiError::new(
                "input",
                "not an install or a snapshot the settings can be copied from",
            ));
        }
        player_files::import(&dirs.write, &from, SystemTime::now())
            .map(|count| count as u32)
            .map_err(|err| ApiError::new("io", format!("copying the player's files: {err}")))
    })
    .await?
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

/// The stack a thread doing Lua work gets.
///
/// StyLua and full_moon walk a tweak's tables recursively, and the first time
/// twenty slots were decoded on joining a room, a fourteen kilobyte
/// `tweakunits` table went past the main thread's megabyte in a debug build.
/// A thread of our own, sized for the deepest table anyone publishes, costs
/// one spawn per call -- and the calls are debounced.
const LUA_STACK: usize = 64 << 20;

/// Runs Lua work off the main thread, on a stack with room to recurse.
async fn lua_work<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::Builder::new()
            .stack_size(LUA_STACK)
            .spawn(work)
            .map_err(|err| ApiError::new("lua", err.to_string()))?
            .join()
            .map_err(|_| ApiError::new("lua", "the Lua worker panicked"))
    })
    .await
    .map_err(|err| ApiError::new("lua", err.to_string()))?
}

/// Decodes a stored slot value for display: Lua, formatted Lua, name, summary.
#[tauri::command]
pub async fn tweak_decode(app: State<'_, App>, blob: String, kind: Kind) -> Result<TweakView> {
    let config = stylua(&app);
    Ok(lua_work(move || tweaks::decode(&blob, kind, &config)).await??)
}

/// Formats Lua with the user's `stylua.toml`, for the editor's Format action.
#[tauri::command]
pub async fn tweak_format(app: State<'_, App>, lua: String, kind: Kind) -> Result<String> {
    let config = stylua(&app);
    Ok(lua_work(move || tweaks::lua::format(&lua, kind, &config)).await??)
}

/// Minifies, encodes and measures — the gauge the editor shows. Sends nothing.
#[tauri::command]
pub async fn tweak_prepare(lua: String, slot: Slot, direct: bool) -> Result<Prepared> {
    Ok(lua_work(move || tweaks::prepare(&lua, slot, direct)).await??)
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
pub async fn tweak_diff(
    app: State<'_, App>,
    kind: Kind,
    current: String,
    proposed: String,
) -> Result<DiffView> {
    let config = stylua(&app);
    lua_work(move || {
        let side = |blob: &str| {
            if blob.is_empty() {
                return String::new();
            }
            tweaks::decode(blob, kind, &config)
                .map_or_else(|_| blob.to_owned(), |view| view.formatted)
        };
        tweaks::diff::diff(&side(&current), &side(&proposed))
    })
    .await
}

/// The units a game has, for completing and checking `tweakunits` keys.
#[tauri::command]
pub async fn game_unit_names(app: State<'_, App>, game: String) -> Result<Vec<String>> {
    let Some(dirs) = data_dirs_of(&app) else {
        return Ok(Vec::new());
    };
    disk_work(move || {
        let files = content::Library::new(dirs).game_files(&game, "units/");
        tweaks::assist::unit_names(&files)
    })
    .await
}

/// The engine's weapon tags, dumped by the installed engine once and kept.
///
/// `spring --list-def-tags` starts the engine far enough to register every
/// tag and prints them as JSON. It takes a second or two, so the dump is
/// cached beside the settings, one file per engine version.
#[tauri::command]
pub async fn engine_def_tags(app: State<'_, App>, version: String) -> Result<tweaks::DefTags> {
    let cache = app
        .settings
        .dir()
        .join("cache")
        .join(format!("deftags-{}.json", safe_name(&version)));
    let json = match std::fs::read_to_string(&cache) {
        Ok(text) => text,
        Err(_) => {
            let dirs = data_dirs_of(&app).ok_or_else(|| {
                ApiError::new("deftags", "no data directory to find an engine in")
            })?;
            let engine = content::Library::new(dirs)
                .find_engine(&version)
                .ok_or_else(|| {
                    ApiError::new("deftags", format!("engine {version} is not installed"))
                })?;
            let text = tauri::async_runtime::spawn_blocking(move || dump_def_tags(&engine.bin))
                .await
                .map_err(|err| ApiError::new("deftags", err.to_string()))??;
            if let Some(parent) = cache.parent() {
                // The cache is a convenience; failing to keep it costs a rerun.
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&cache, &text);
            text
        }
    };
    tweaks::assist::parse_def_tags(&json).map_err(|err| ApiError::new("deftags", err.to_string()))
}

fn dump_def_tags(engine_dir: &std::path::Path) -> Result<String> {
    let mut command = std::process::Command::new(engine_dir.join(recoil::ENGINE_BINARY));
    command.current_dir(engine_dir).arg("--list-def-tags");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
        .output()
        .map_err(|err| ApiError::new("deftags", format!("running the engine: {err}")))?;
    if !output.status.success() {
        return Err(ApiError::new(
            "deftags",
            format!("the engine exited with {}", output.status),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A version string as one file name segment.
fn safe_name(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The data directories, for the commands where having none is an empty
/// answer rather than an error.
fn data_dirs_of(app: &App) -> Option<content::DataDirs> {
    launch::data_dirs(app.settings.get().paths.data_dir)
}

/// Where the Lua stops making sense, and what it names: markers and an outline.
#[tauri::command]
pub async fn tweak_check(lua: String, kind: Kind) -> Result<tweaks::Check> {
    lua_work(move || tweaks::check(&lua, kind)).await
}

/// Two pieces of Lua, formatted alike and diffed -- what the editor compares.
#[tauri::command]
pub async fn tweak_diff_text(
    app: State<'_, App>,
    kind: Kind,
    left: String,
    right: String,
) -> Result<DiffView> {
    let config = stylua(&app);
    lua_work(move || {
        let side =
            |lua: &str| tweaks::lua::format(lua, kind, &config).unwrap_or_else(|_| lua.to_owned());
        tweaks::diff::diff(&side(&left), &side(&right))
    })
    .await
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
    tweaks::lua::load_config(path.as_deref()).unwrap_or_else(|_| tweaks::lua::default_config())
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

/// Where BAR content is: the directory we write, chosen or our own, and every
/// other lobby's install to read. Fails only on a machine with no home
/// directory, which is the one case a setting can fix.
pub(crate) fn data_dirs(app: &App) -> Result<content::DataDirs> {
    launch::data_dirs(app.settings.get().paths.data_dir).ok_or_else(|| {
        ApiError::new(
            "input",
            "no home directory to keep BAR content under; set paths.dataDir",
        )
    })
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
