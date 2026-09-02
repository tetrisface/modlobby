//! One actor owns the connection, the reducer, the engine child and the UI
//! transport; front ends send [`Command`]s through a [`Client`] handle. Every
//! inbound line is reduced, projected into deltas and batched: a burst that is
//! already queued becomes one `Deltas` message.

use std::collections::HashMap;
use std::future::Future;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lobby_core::{Effect, Session, hosting};
use lobby_ui::{
    Batcher, Delta, DownloadStatus, EngineStatus, GameRunningView, Phase, Projector, Snapshot,
    UiMessage, UiTransport,
};
use spring_protocol::battle::{self, TooLong};
use spring_protocol::policy::PolicyEvent;
use spring_protocol::{
    Area, Endpoint, Envelope, Inbound, LoginRequest, ThrottlePolicy, Transport, TransportError,
};
use tokio::io::AsyncReadExt;
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};

use crate::latency::{self, Latency};
use crate::launch;
use crate::platform::Hardware;
use crate::reconnect;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("not connected")]
    NotConnected,
    #[error("already connected")]
    AlreadyConnected,
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// The server or host said no: login denied, join refused, disconnected.
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    TooLong(#[from] TooLong),
    #[error("engine: {0}")]
    Engine(String),
    #[error("client stopped")]
    Stopped,
}

type Reply<T> = oneshot::Sender<Result<T, ClientError>>;
type Connected = (Transport, mpsc::Receiver<Inbound>);
type ConnectFuture = Pin<Box<dyn Future<Output = Result<Connected, TransportError>> + Send>>;

/// How the runtime reaches a server; tests hand it an in-memory stream.
pub type Connector = Arc<dyn Fn(Endpoint, ThrottlePolicy) -> ConnectFuture + Send + Sync>;

enum Command {
    Subscribe(Box<dyn UiTransport>),
    Login {
        endpoint: Endpoint,
        request: LoginRequest,
        reply: Reply<()>,
    },
    Logout,
    /// Creates an account on a connection of its own, then hangs up.
    Register {
        endpoint: Endpoint,
        request: LoginRequest,
        email: String,
        password: String,
        reply: Reply<()>,
    },
    /// Confirms the emailed code for an account that has just been created.
    ConfirmAgreement {
        code: String,
        reply: Reply<()>,
    },
    Snapshot(oneshot::Sender<Snapshot>),
    JoinBattle {
        id: u32,
        password: Option<String>,
        reply: Reply<()>,
    },
    LeaveBattle,
    Launch {
        data_dir: PathBuf,
        reply: Reply<()>,
    },
    Say {
        text: String,
        reply: Reply<()>,
    },
    JoinChannel {
        room: String,
        key: Option<String>,
        reply: Reply<()>,
    },
    LeaveChannel {
        room: String,
        reply: Reply<()>,
    },
    SayChannel {
        room: String,
        text: String,
        reply: Reply<()>,
    },
    SayPrivate {
        user: String,
        text: String,
        reply: Reply<()>,
    },
    ListChannels {
        reply: Reply<()>,
    },
    RefreshFriends {
        reply: Reply<()>,
    },
    /// Fetches whatever the current room is missing.
    DownloadMissing {
        reply: Reply<()>,
    },
    StopDownload {
        reply: Reply<()>,
    },
    Ring {
        user: String,
        reply: Reply<()>,
    },
    AddBot {
        name: String,
        ai: String,
        team: u8,
        ally_team: u8,
        colour: u32,
        reply: Reply<()>,
    },
    RemoveBot {
        name: String,
        reply: Reply<()>,
    },
    SetAway {
        away: bool,
        reply: Reply<()>,
    },
    SetAutoLaunch {
        always: bool,
        reply: Reply<()>,
    },
    EnginePid {
        reply: Reply<Option<u32>>,
    },
    StopEngine {
        reply: Reply<bool>,
    },
    RequestGameStatus {
        founder: String,
        reply: Reply<()>,
    },
    PlayReplay {
        data_dir: PathBuf,
        path: String,
        reply: Reply<()>,
    },
    StartSkirmish {
        data_dir: PathBuf,
        engine_version: String,
        skirmish: Box<recoil::script::Skirmish>,
        reply: Reply<()>,
    },
    FriendAction {
        action: lobby_core::FriendAction,
        user: String,
        reply: Reply<()>,
    },
    TakeSeat {
        team: u8,
        ally_team: u8,
        reply: Reply<()>,
    },
    SetReady {
        ready: bool,
        reply: Reply<()>,
    },
    SetSide {
        side: u8,
        reply: Reply<()>,
    },
    AllowPublicSeat(bool),
    ReleaseSeat,
    SetDataDir(Option<PathBuf>),
    /// Where to keep a config to launch with when the user's own settings
    /// would put the game in exclusive full screen. `None` while the
    /// overlay is switched off, which is also when nothing is written.
    SetOverlayConfigDir(Option<PathBuf>),
    /// Asks a cluster manager for a room of our own; the runtime joins it
    /// when it appears. Replies with the manager asked.
    RequestPrivateHost {
        reply: Reply<String>,
    },
    /// Joins an empty public autohost, making it ours. Replies with its id.
    HostPublic {
        reply: Reply<u32>,
    },
    Shutdown,
}

/// Who asked for the latencies, and what they are for.
enum Wanted {
    Public(Reply<u32>),
    Private(Reply<String>),
}

/// The latencies a request went out to measure, back from the probe task.
struct Probe {
    measured: HashMap<Ipv4Addr, Option<Duration>>,
    wanted: Wanted,
}

/// How long a measured latency stays good for. The machines do not move;
/// this is about noticing a route that changed, not tracking jitter.
const RTT_TTL: Duration = Duration::from_secs(600);
/// How long to wait for one echo. Hosts are within a few hundred ms of
/// anywhere; longer only delays the answer for a machine that is down.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// Handle to the runtime; cheap to clone.
#[derive(Clone)]
pub struct Client {
    tx: mpsc::Sender<Command>,
}

impl Client {
    /// Spawns the runtime on the current tokio runtime, connecting over TCP/TLS.
    pub fn spawn(policy: ThrottlePolicy, hardware: Hardware) -> Self {
        let connector: Connector = Arc::new(|endpoint, policy| {
            Box::pin(async move { Transport::connect(&endpoint, policy).await })
        });
        Self::spawn_with(policy, hardware, connector, Arc::new(latency::IcmpEcho))
    }

    pub fn spawn_with(
        policy: ThrottlePolicy,
        hardware: Hardware,
        connector: Connector,
        latency: Arc<dyn Latency>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(Runtime::new(rx, policy, hardware, connector, latency).run());
        Self { tx }
    }

    async fn send(&self, command: Command) -> Result<(), ClientError> {
        self.tx
            .send(command)
            .await
            .map_err(|_| ClientError::Stopped)
    }

    async fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Command) -> Result<T, ClientError> {
        let (reply, rx) = oneshot::channel();
        self.send(make(reply)).await?;
        rx.await.map_err(|_| ClientError::Stopped)?
    }

    /// Installs the front end; it receives a snapshot immediately.
    pub async fn subscribe(&self, transport: impl UiTransport) -> Result<(), ClientError> {
        self.send(Command::Subscribe(Box::new(transport))).await
    }

    /// Resolves once the login flood is over (the lobby is ready), or with the refusal.
    pub async fn login(
        &self,
        endpoint: Endpoint,
        request: LoginRequest,
    ) -> Result<(), ClientError> {
        self.ask(|reply| Command::Login {
            endpoint,
            request,
            reply,
        })
        .await
    }

    pub async fn logout(&self) -> Result<(), ClientError> {
        self.send(Command::Logout).await
    }

    /// Creates an account. Resolves when the server has accepted or refused it.
    ///
    /// The account cannot log in yet: the server emails a code that
    /// [`Self::confirm_agreement`] carries back.
    pub async fn register(
        &self,
        endpoint: Endpoint,
        request: LoginRequest,
        email: String,
        password: String,
    ) -> Result<(), ClientError> {
        self.ask(|reply| Command::Register {
            endpoint,
            request,
            email,
            password,
            reply,
        })
        .await
    }

    /// Sends the emailed agreement code for the account now logging in.
    pub async fn confirm_agreement(&self, code: String) -> Result<(), ClientError> {
        self.ask(|reply| Command::ConfirmAgreement { code, reply })
            .await
    }

    pub async fn snapshot(&self) -> Result<Snapshot, ClientError> {
        let (tx, rx) = oneshot::channel();
        self.send(Command::Snapshot(tx)).await?;
        rx.await.map_err(|_| ClientError::Stopped)
    }

    /// Resolves when the host accepted us as a spectator, or with its refusal.
    pub async fn join_battle(&self, id: u32, password: Option<String>) -> Result<(), ClientError> {
        self.ask(|reply| Command::JoinBattle {
            id,
            password,
            reply,
        })
        .await
    }

    pub async fn leave_battle(&self) -> Result<(), ClientError> {
        self.send(Command::LeaveBattle).await
    }

    /// Connects the engine to the room's game now, or as soon as it is running.
    pub async fn launch(&self, data_dir: PathBuf) -> Result<(), ClientError> {
        self.ask(|reply| Command::Launch { data_dir, reply }).await
    }

    pub async fn say(&self, text: String) -> Result<(), ClientError> {
        self.ask(|reply| Command::Say { text, reply }).await
    }

    pub async fn join_channel(&self, room: String, key: Option<String>) -> Result<(), ClientError> {
        self.ask(|reply| Command::JoinChannel { room, key, reply })
            .await
    }

    pub async fn leave_channel(&self, room: String) -> Result<(), ClientError> {
        self.ask(|reply| Command::LeaveChannel { room, reply })
            .await
    }

    pub async fn say_channel(&self, room: String, text: String) -> Result<(), ClientError> {
        self.ask(|reply| Command::SayChannel { room, text, reply })
            .await
    }

    pub async fn say_private(&self, user: String, text: String) -> Result<(), ClientError> {
        self.ask(|reply| Command::SayPrivate { user, text, reply })
            .await
    }

    pub async fn list_channels(&self) -> Result<(), ClientError> {
        self.ask(|reply| Command::ListChannels { reply }).await
    }

    pub async fn refresh_friends(&self) -> Result<(), ClientError> {
        self.ask(|reply| Command::RefreshFriends { reply }).await
    }

    /// Fetches the game and map the current room needs and we do not have.
    pub async fn download_missing(&self) -> Result<(), ClientError> {
        self.ask(|reply| Command::DownloadMissing { reply }).await
    }

    /// Stops the running download, leaving whatever it already wrote.
    pub async fn stop_download(&self) -> Result<(), ClientError> {
        self.ask(|reply| Command::StopDownload { reply }).await
    }

    /// Rings someone, which is how you tell a player the room is waiting.
    pub async fn ring(&self, user: String) -> Result<(), ClientError> {
        self.ask(|reply| Command::Ring { user, reply }).await
    }

    /// Adds an AI to the room; it runs on this machine when the game starts.
    pub async fn add_bot(
        &self,
        name: String,
        ai: String,
        team: u8,
        ally_team: u8,
        colour: u32,
    ) -> Result<(), ClientError> {
        self.ask(|reply| Command::AddBot {
            name,
            ai,
            team,
            ally_team,
            colour,
            reply,
        })
        .await
    }

    pub async fn remove_bot(&self, name: String) -> Result<(), ClientError> {
        self.ask(|reply| Command::RemoveBot { name, reply }).await
    }

    /// Whether the engine starts on its own when the room's game does.
    pub async fn set_auto_launch(&self, always: bool) -> Result<(), ClientError> {
        self.ask(|reply| Command::SetAutoLaunch { always, reply })
            .await
    }

    /// Marks us away, so nobody waits on someone who has stepped out.
    pub async fn set_away(&self, away: bool) -> Result<(), ClientError> {
        self.ask(|reply| Command::SetAway { away, reply }).await
    }

    /// Asks a host how long its game has been going. The answer arrives as a
    /// delta, not as a return value: it comes back as a private message.
    pub async fn request_game_status(&self, founder: String) -> Result<(), ClientError> {
        self.ask(|reply| Command::RequestGameStatus { founder, reply })
            .await
    }

    /// The running engine's process id, for whoever needs to point at its
    /// window. `None` when no engine of ours is running.
    pub async fn engine_pid(&self) -> Result<Option<u32>, ClientError> {
        self.ask(|reply| Command::EnginePid { reply }).await
    }

    /// Stops the running game, if there is one. Answers whether there was.
    ///
    /// Asking the process to stop rather than telling the engine to quit: we
    /// have no channel into a running game, and a game being ended on purpose
    /// has nothing to save.
    pub async fn stop_engine(&self) -> Result<bool, ClientError> {
        self.ask(|reply| Command::StopEngine { reply }).await
    }

    /// Starts a game against AI with no server involved.
    pub async fn start_skirmish(
        &self,
        data_dir: PathBuf,
        engine_version: String,
        skirmish: recoil::script::Skirmish,
    ) -> Result<(), ClientError> {
        self.ask(|reply| Command::StartSkirmish {
            data_dir,
            engine_version,
            skirmish: Box::new(skirmish),
            reply,
        })
        .await
    }

    /// Starts the engine on a replay file.
    pub async fn play_replay(&self, data_dir: PathBuf, path: String) -> Result<(), ClientError> {
        self.ask(|reply| Command::PlayReplay {
            data_dir,
            path,
            reply,
        })
        .await
    }

    pub async fn friend_action(
        &self,
        action: lobby_core::FriendAction,
        user: String,
    ) -> Result<(), ClientError> {
        self.ask(|reply| Command::FriendAction {
            action,
            user,
            reply,
        })
        .await
    }

    /// Says whether we are ready to start. Only a player can be.
    pub async fn set_ready(&self, ready: bool) -> Result<(), ClientError> {
        self.ask(|reply| Command::SetReady { ready, reply }).await
    }

    /// Picks a faction: 0 Armada, 1 Cortex, 2 Random, 3 Legion.
    pub async fn set_side(&self, side: u8) -> Result<(), ClientError> {
        self.ask(|reply| Command::SetSide { side, reply }).await
    }

    /// Whether a seat may be taken in a public room at all.
    pub async fn allow_public_seat(&self, allow: bool) -> Result<(), ClientError> {
        self.send(Command::AllowPublicSeat(allow)).await
    }

    /// Takes a player slot. Refused unless the room is passworded — see
    /// [`lobby_core::SeatError`]; in a public room the slot is someone else's.
    pub async fn take_seat(&self, team: u8, ally_team: u8) -> Result<(), ClientError> {
        self.ask(|reply| Command::TakeSeat {
            team,
            ally_team,
            reply,
        })
        .await
    }

    pub async fn release_seat(&self) -> Result<(), ClientError> {
        self.send(Command::ReleaseSeat).await
    }

    /// Points the content check at a data directory; `None` uses the launcher's.
    pub async fn set_data_dir(&self, data_dir: Option<PathBuf>) -> Result<(), ClientError> {
        self.send(Command::SetDataDir(data_dir)).await
    }

    /// Where a borderless config may be kept, or `None` to launch with the
    /// user's settings exactly as they are.
    pub async fn set_overlay_config_dir(&self, dir: Option<PathBuf>) -> Result<(), ClientError> {
        self.send(Command::SetOverlayConfigDir(dir)).await
    }

    /// Joins an empty public autohost; the first person in it becomes its
    /// boss, which is how a public room of your own is made. Which one is
    /// decided by latency and by which cluster has rooms to spare.
    pub async fn host_public(&self) -> Result<u32, ClientError> {
        self.ask(|reply| Command::HostPublic { reply }).await
    }

    /// Asks a cluster manager for a room of our own; the runtime joins it
    /// when it appears. Returns the manager it asked.
    pub async fn request_private_host(&self) -> Result<String, ClientError> {
        self.ask(|reply| Command::RequestPrivateHost { reply })
            .await
    }

    /// Leaves the room, closes the connection and stops the runtime. A running
    /// engine is left alone; it keeps its own link to the host.
    pub async fn shutdown(&self) {
        let _ = self.send(Command::Shutdown).await;
        self.tx.closed().await;
    }
}

struct Connection {
    transport: Transport,
    inbound: mpsc::Receiver<Inbound>,
    session: Session,
}

/// The room's game as last announced, with the secret the engine presents.
struct Game {
    view: GameRunningView,
    script_password: String,
}

enum Next {
    Command(Command),
    Inbound(Inbound),
    EngineExited(std::io::Result<ExitStatus>),
    Download(DownloadEvent),
    Probe(Probe),
    Reconnect,
}

/// Completes when a reconnection attempt falls due, and never when none is
/// wanted — so the arm simply does not fire while we are connected.
async fn sleep_until_due(policy: &reconnect::Reconnect) {
    match policy.until_due(Instant::now()) {
        Some(wait) => tokio::time::sleep(wait).await,
        None => std::future::pending().await,
    }
}

struct Runtime {
    rx: mpsc::Receiver<Command>,
    policy: ThrottlePolicy,
    hardware: Hardware,
    connector: Connector,
    ui: Option<Box<dyn UiTransport>>,
    conn: Option<Connection>,
    engine: Option<Child>,
    engine_status: EngineStatus,
    game: Option<Game>,
    auto_launch: Option<PathBuf>,
    /// Whether to start the engine ourselves when the room's game starts.
    /// Pushed from settings, like the data directory.
    auto_launch_always: bool,
    /// Whether this machine has everything the room needs. Launching without
    /// it produces an engine that quits with a sync error.
    content_ready: bool,
    login_reply: Option<Reply<()>>,
    /// Waiting on `REGISTRATIONACCEPTED`/`REGISTRATIONDENIED`.
    register_reply: Option<Reply<()>>,
    join_reply: Option<Reply<()>>,
    /// Where BAR's content lives; `None` falls back to the launcher's directory.
    data_dir: Option<PathBuf>,
    /// Where to put a config that gets the game borderless, when the user's
    /// own would not let the overlay cover it. `None` leaves their settings
    /// entirely alone, which is also what happens when they already work.
    overlay_config_dir: Option<PathBuf>,
    /// The room's (engine, game, map) the content check last ran against;
    /// scanning the rapid index is too slow to repeat per message.
    checked: Option<(String, String, String)>,
    /// How to log in again, kept from the last successful attempt so a drop
    /// can be recovered from without the user typing anything.
    credentials: Option<(spring_protocol::Endpoint, LoginRequest)>,
    reconnect: reconnect::Reconnect,
    /// Carried across reconnects, since the session is rebuilt each time.
    allow_public_seat: bool,
    /// What a running pr-downloader was asked for, or `None` when none is.
    downloading: Option<String>,
    /// Stops the running child. Dropping it is what the child watches for.
    download_stop: Option<oneshot::Sender<()>>,
    /// Set while a stop we asked for is still on its way, so the child dying
    /// is reported as stopped rather than as a failure.
    download_stopping: bool,
    /// The room contents we have already fetched for once. A map the CDN does
    /// not have would otherwise be retried forever, since every failed
    /// download ends in another content check.
    auto_fetched: Option<(String, String, String)>,
    download_tx: mpsc::Sender<DownloadEvent>,
    download_rx: mpsc::Receiver<DownloadEvent>,
    latency: Arc<dyn Latency>,
    /// What the host machines answered, and when, so a second room in the
    /// same session does not wait on the same probes again.
    rtts: HashMap<Ipv4Addr, (Instant, Option<Duration>)>,
    probe_tx: mpsc::Sender<Probe>,
    probe_rx: mpsc::Receiver<Probe>,
    /// Whether a room request is out measuring; a second one would only
    /// race the first for the same spare.
    probing: bool,
    projector: Projector,
    batcher: Batcher,
}

/// What a pr-downloader child reports back to the runtime.
#[derive(Debug)]
enum DownloadEvent {
    Progress(recoil::Progress),
    Finished { what: String, ok: bool },
}

impl Runtime {
    fn new(
        rx: mpsc::Receiver<Command>,
        policy: ThrottlePolicy,
        hardware: Hardware,
        connector: Connector,
        latency: Arc<dyn Latency>,
    ) -> Self {
        // A short queue: progress lines arrive far faster than the front end
        // needs them, and the batcher coalesces what gets through anyway.
        let (download_tx, download_rx) = mpsc::channel(16);
        let (probe_tx, probe_rx) = mpsc::channel(1);
        Self {
            rx,
            policy,
            hardware,
            connector,
            ui: None,
            conn: None,
            engine: None,
            engine_status: EngineStatus::Idle,
            game: None,
            auto_launch: None,
            auto_launch_always: true,
            content_ready: false,
            login_reply: None,
            register_reply: None,
            join_reply: None,
            data_dir: None,
            overlay_config_dir: None,
            checked: None,
            credentials: None,
            reconnect: reconnect::Reconnect::default(),
            allow_public_seat: false,
            downloading: None,
            download_stop: None,
            download_stopping: false,
            auto_fetched: None,
            download_tx,
            download_rx,
            latency,
            rtts: HashMap::new(),
            probe_tx,
            probe_rx,
            probing: false,
            projector: Projector::new(),
            batcher: Batcher::default(),
        }
    }

    /// Writes the start script and starts the engine on it.
    ///
    /// The script goes in the data directory the engine is already reading
    /// under `--isolation`, so it needs no extra path allowance, and it is
    /// overwritten each time rather than accumulating.
    fn start_skirmish(
        &mut self,
        data_dir: PathBuf,
        engine_version: &str,
        skirmish: &recoil::script::Skirmish,
    ) -> Result<(), ClientError> {
        if self.engine.is_some() {
            return Err(ClientError::Engine("the engine is already running".into()));
        }
        let path = data_dir.join("modlobby-skirmish.txt");
        std::fs::write(&path, skirmish.script())
            .map_err(|err| ClientError::Engine(format!("writing the start script: {err}")))?;

        let child = launch::spawn(
            &data_dir,
            engine_version,
            path.to_string_lossy().into_owned(),
            self.overlay_config_dir.as_deref(),
        )
        .map_err(ClientError::Engine)?;
        let pid = child.id();
        self.engine = Some(child);
        self.set_engine(EngineStatus::Running { pid });
        Ok(())
    }

    /// Starts the engine on a replay.
    ///
    /// The engine reads the demo's own header for the engine version it needs,
    /// so the replay's name is the only hint we have about which one to use;
    /// falling back to any installed engine beats refusing to play it.
    fn play_replay(&mut self, data_dir: PathBuf, path: String) -> Result<(), ClientError> {
        if self.engine.is_some() {
            return Err(ClientError::Engine("the engine is already running".into()));
        }
        let version = std::path::Path::new(&path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.rsplit_once('_').map(|(_, engine)| engine.to_owned()))
            .unwrap_or_default();

        let child = launch::spawn(
            &data_dir,
            &version,
            path,
            self.overlay_config_dir.as_deref(),
        )
        .map_err(ClientError::Engine)?;
        let pid = child.id();
        self.engine = Some(child);
        self.set_engine(EngineStatus::Running { pid });
        Ok(())
    }

    /// Starts pr-downloader on whatever the room needs and this machine lacks.
    ///
    /// The child is not awaited here: progress is streamed to the front end as
    /// it arrives, and the content check runs again when it exits, so a room
    /// that was short a map becomes joinable without anyone asking twice.
    async fn start_download(&mut self) -> Result<(), ClientError> {
        if self.downloading.is_some() {
            return Err(ClientError::Refused("a download is already running".into()));
        }
        let Some(conn) = self.conn.as_ref() else {
            return Err(ClientError::NotConnected);
        };
        let room = conn
            .session
            .state
            .my_battle
            .as_ref()
            .and_then(|my| conn.session.state.battles.get(&my.id))
            .ok_or_else(|| ClientError::Refused("not in a room".into()))?;
        let (engine_version, game, map) = (
            room.engine_version.clone(),
            room.game_name.clone(),
            room.map_name.clone(),
        );

        let Some(data_dir) = self.data_dir() else {
            return Err(ClientError::Refused("no BAR data directory".into()));
        };
        let library = content::Library::new(&data_dir);
        let mut wants = Vec::new();
        if !library.has_game(&game) {
            wants.push((recoil::Want::Game, game.clone()));
        }
        if !library.has_map(&map) {
            wants.push((recoil::Want::Map, map.clone()));
        }
        if wants.is_empty() {
            return Err(ClientError::Refused("nothing is missing".into()));
        }

        let what = wants
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let mut child = crate::launch::spawn_download(&data_dir, &engine_version, wants)
            .map_err(ClientError::Refused)?;
        let stdout = child.stdout.take();

        let (stop_tx, stop_rx) = oneshot::channel();
        self.downloading = Some(what.clone());
        self.download_stop = Some(stop_tx);
        self.download_stopping = false;
        self.batcher.push(Delta::Download(DownloadStatus::Running {
            what: what.clone(),
            current: 0,
            total: 0,
        }));

        let events = self.download_tx.clone();
        tokio::spawn(async move {
            let progress = events.clone();
            let pump = async move {
                if let Some(mut stdout) = stdout {
                    // Read raw rather than by line: pr-downloader redraws progress
                    // with carriage returns, so a line reader would see one
                    // enormous line at the end and no progress at all.
                    let mut buffer = String::new();
                    let mut chunk = [0_u8; 4096];
                    while let Ok(read) = stdout.read(&mut chunk).await {
                        if read == 0 {
                            break;
                        }
                        buffer.push_str(&String::from_utf8_lossy(&chunk[..read]));
                        for line in recoil::split_output(&mut buffer) {
                            if let Some(step) = recoil::Progress::parse(&line) {
                                let _ = progress.send(DownloadEvent::Progress(step)).await;
                            }
                        }
                    }
                }
            };
            // The stop side owns a sender the runtime drops; either the pump
            // finishing or that drop ends the wait, and only then is the child
            // asked to stop — pr-downloader leaves a partial file behind, which
            // its own resume handles on the next attempt.
            tokio::pin!(pump);
            tokio::select! {
                () = &mut pump => {}
                _ = stop_rx => {
                    let _ = child.start_kill();
                }
            }
            let status = child.wait().await;
            let _ = events
                .send(DownloadEvent::Finished {
                    what,
                    ok: matches!(status, Ok(status) if status.success()),
                })
                .await;
        });

        Ok(())
    }

    /// Sends a room request out to measure the machines it could land on.
    /// Only addresses not measured lately are probed; the rest are already
    /// known. The answer comes back through [`Runtime::on_probe`], so the
    /// actor keeps reducing the server's lines meanwhile.
    fn start_probe(&mut self, ips: Vec<Ipv4Addr>, wanted: Wanted) {
        if self.probing {
            let refused = ClientError::Refused("still looking for a room".into());
            match wanted {
                Wanted::Public(reply) => {
                    let _ = reply.send(Err(refused));
                }
                Wanted::Private(reply) => {
                    let _ = reply.send(Err(refused));
                }
            }
            return;
        }
        let now = Instant::now();
        let stale: Vec<Ipv4Addr> = ips
            .into_iter()
            .filter(|ip| {
                self.rtts
                    .get(ip)
                    .is_none_or(|(at, _)| now.duration_since(*at) > RTT_TTL)
            })
            .collect();
        self.probing = true;
        let latency = Arc::clone(&self.latency);
        let events = self.probe_tx.clone();
        tokio::spawn(async move {
            let measured = latency::measure(latency, stale, PROBE_TIMEOUT).await;
            let _ = events.send(Probe { measured, wanted }).await;
        });
    }

    /// The latencies still worth trusting.
    fn known_rtts(&self) -> lobby_core::Rtts {
        let now = Instant::now();
        self.rtts
            .iter()
            .filter(|(_, (at, _))| now.duration_since(*at) <= RTT_TTL)
            .filter_map(|(ip, (_, rtt))| Some((*ip, (*rtt)?)))
            .collect()
    }

    /// Picks the room, or the manager, now that the distances are known.
    /// The list is read again here: it may have moved while we measured.
    async fn on_probe(&mut self, probe: Probe) {
        self.probing = false;
        let now = Instant::now();
        for (ip, rtt) in probe.measured {
            tracing::debug!(%ip, ?rtt, "host latency");
            self.rtts.insert(ip, (now, rtt));
        }
        let rtts = self.known_rtts();
        let roll = rand::random::<f64>();
        match probe.wanted {
            Wanted::Public(reply) => {
                let Some(conn) = self.conn.as_mut() else {
                    let _ = reply.send(Err(ClientError::NotConnected));
                    return;
                };
                let spares = conn.session.spare_rooms();
                let Some(id) = hosting::pick(&spares, &rtts, roll) else {
                    let _ = reply.send(Err(ClientError::Refused(
                        "no empty autohost right now".into(),
                    )));
                    return;
                };
                let script_password = format!("{}{}", rand::random::<u16>(), rand::random::<u16>());
                let effects = conn.session.host_public(id, script_password);
                let _ = reply.send(Ok(id));
                self.apply_effects(effects).await;
            }
            Wanted::Private(reply) => {
                let Some(conn) = self.conn.as_mut() else {
                    let _ = reply.send(Err(ClientError::NotConnected));
                    return;
                };
                let Some(manager) = conn.session.pick_cluster_manager(&rtts, roll) else {
                    let _ = reply.send(Err(ClientError::Refused(
                        "no cluster manager online".into(),
                    )));
                    return;
                };
                let manager = manager.to_owned();
                match conn.session.request_private_host(&manager) {
                    Ok(effects) => {
                        let _ = reply.send(Ok(manager));
                        self.apply_effects(effects).await;
                    }
                    Err(err) => {
                        let _ = reply.send(Err(ClientError::Refused(err.to_string())));
                    }
                }
            }
        }
    }

    /// Reports a download's progress, and re-checks content when it ends.
    async fn on_download(&mut self, event: DownloadEvent) {
        match event {
            DownloadEvent::Progress(progress) => {
                let Some(what) = self.downloading.clone() else {
                    return;
                };
                self.batcher.push(Delta::Download(DownloadStatus::Running {
                    what,
                    current: progress.current,
                    total: progress.total,
                }));
            }
            DownloadEvent::Finished { what, ok } => {
                self.downloading = None;
                self.download_stop = None;
                let stopped = std::mem::take(&mut self.download_stopping);
                self.batcher.push(Delta::Download(if stopped {
                    DownloadStatus::Idle
                } else if ok {
                    DownloadStatus::Done { what }
                } else {
                    DownloadStatus::Failed {
                        what,
                        reason: "pr-downloader did not finish".into(),
                    }
                }));
                // Whatever arrived changes the answer, so ask the disk again.
                self.checked = None;
                self.refresh_content().await;
            }
        }
    }

    /// Re-checks the room's content when what it asks for changes, and tells
    /// the room whether we are synced. Nothing claims sync without a disk check.
    async fn refresh_content(&mut self) {
        let Some(conn) = self.conn.as_ref() else {
            self.checked = None;
            return;
        };
        let room = conn
            .session
            .state
            .my_battle
            .as_ref()
            .and_then(|my| conn.session.state.battles.get(&my.id));
        let Some(room) = room else {
            if self.checked.take().is_some() {
                self.set_synced(false).await;
            }
            return;
        };
        let key = (
            room.engine_version.clone(),
            room.game_name.clone(),
            room.map_name.clone(),
        );
        if self.checked.as_ref() == Some(&key) {
            return;
        }
        let Some(data_dir) = self.data_dir() else {
            return;
        };
        let available = content::Library::new(data_dir).check(&key.0, &key.1, &key.2);
        self.checked = Some(key.clone());
        self.batcher.push(Delta::Content {
            engine: available.engine,
            game: available.game,
            map: available.map,
        });
        self.content_ready = available.complete();
        self.set_synced(available.complete()).await;

        // Joining a room you have no map for is a request for the map: there is
        // nothing else to do in it. Only what pr-downloader can fetch, only
        // when nothing else is running, and only once per room — a name the
        // CDN does not carry would otherwise be retried by every content check
        // that a failed download itself provokes.
        let fetchable = !available.game || !available.map;
        if fetchable
            && available.engine
            && self.downloading.is_none()
            && self.auto_fetched.as_ref() != Some(&key)
        {
            self.auto_fetched = Some(key);
            if let Err(error) = self.start_download().await {
                tracing::debug!(%error, "not fetching the room's content");
            }
        }
    }

    async fn set_synced(&mut self, synced: bool) {
        let Some(conn) = self.conn.as_mut() else {
            return;
        };
        let effects = conn.session.set_synced(synced);
        self.apply_effects(effects).await;
    }

    /// Where BAR keeps its content: the setting when given, else the launcher's.
    fn data_dir(&self) -> Option<PathBuf> {
        self.data_dir.clone().or_else(launch::default_data_dir)
    }

    async fn run(mut self) {
        loop {
            let next = tokio::select! {
                command = self.rx.recv() => match command {
                    Some(command) => Next::Command(command),
                    None => return,
                },
                inbound = recv_inbound(&mut self.conn) => Next::Inbound(inbound),
                status = wait_engine(&mut self.engine) => Next::EngineExited(status),
                Some(event) = self.download_rx.recv() => Next::Download(event),
                Some(probe) = self.probe_rx.recv() => Next::Probe(probe),
                () = sleep_until_due(&self.reconnect) => Next::Reconnect,
            };
            match next {
                Next::Command(Command::Shutdown) => {
                    self.disconnect().await;
                    self.flush();
                    return;
                }
                Next::Command(command) => self.handle_command(command).await,
                Next::Download(event) => self.on_download(event).await,
                Next::Probe(probe) => self.on_probe(probe).await,
                Next::Reconnect => self.try_reconnect().await,
                Next::Inbound(inbound) => {
                    self.handle_inbound(inbound).await;
                    while let Some(more) = self.try_recv_inbound() {
                        self.handle_inbound(more).await;
                    }
                    self.refresh_content().await;
                }
                Next::EngineExited(status) => self.engine_exited(status).await,
            }
            self.flush();
        }
    }

    fn try_recv_inbound(&mut self) -> Option<Inbound> {
        self.conn.as_mut()?.inbound.try_recv().ok()
    }

    async fn handle_command(&mut self, command: Command) {
        match command {
            Command::Subscribe(transport) => {
                self.ui = Some(transport);
                self.send_snapshot();
            }
            Command::Login {
                endpoint,
                request,
                reply,
            } => self.connect(endpoint, request, reply).await,
            Command::Logout => {
                // Asked for: stop trying to come back.
                self.credentials = None;
                self.reconnect.stop();
                self.disconnect().await;
            }
            Command::Snapshot(tx) => {
                let _ = tx.send(self.snapshot());
            }
            Command::JoinBattle {
                id,
                password,
                reply,
            } => {
                let Some(conn) = self.conn.as_mut() else {
                    let _ = reply.send(Err(ClientError::NotConnected));
                    return;
                };
                let script_password = format!("{}{}", rand::random::<u16>(), rand::random::<u16>());
                let effects = conn
                    .session
                    .join_battle(id, password.as_deref(), script_password);
                self.join_reply = Some(reply);
                self.apply_effects(effects).await;
            }
            Command::LeaveBattle => {
                let Some(conn) = self.conn.as_mut() else {
                    return;
                };
                let effects = conn.session.leave_battle();
                self.project_effects(&effects);
                self.apply_effects(effects).await;
            }
            Command::Launch { data_dir, reply } => {
                let result = if self.conn.is_none() {
                    Err(ClientError::NotConnected)
                } else if self.engine.is_some() {
                    Err(ClientError::Engine("already running".into()))
                } else if self.game.is_some() {
                    self.launch_engine(data_dir).await
                } else {
                    self.auto_launch = Some(data_dir);
                    Ok(())
                };
                let _ = reply.send(result);
            }
            Command::Say { text, reply } => {
                let result = match battle::say_battle(&text) {
                    Ok(envelope) => self.send_line(envelope).await,
                    Err(err) => Err(err.into()),
                };
                let _ = reply.send(result);
            }
            Command::JoinChannel { room, key, reply } => {
                self.run_session(reply, |session| session.join_channel(&room, key.as_deref()))
                    .await;
            }
            Command::LeaveChannel { room, reply } => {
                self.run_session(reply, |session| session.leave_channel(&room))
                    .await;
            }
            Command::SayChannel { room, text, reply } => {
                self.run_session(reply, |session| session.say_channel(&room, &text))
                    .await;
            }
            Command::SayPrivate { user, text, reply } => {
                self.run_session(reply, |session| session.say_private(&user, &text))
                    .await;
            }
            Command::ListChannels { reply } => {
                self.run_session(reply, |session| {
                    Ok::<_, std::convert::Infallible>(session.list_channels())
                })
                .await;
            }
            Command::StartSkirmish {
                data_dir,
                engine_version,
                skirmish,
                reply,
            } => {
                let result = self.start_skirmish(data_dir, &engine_version, &skirmish);
                let _ = reply.send(result);
            }
            Command::PlayReplay {
                data_dir,
                path,
                reply,
            } => {
                let result = self.play_replay(data_dir, path);
                let _ = reply.send(result);
            }
            Command::DownloadMissing { reply } => {
                let result = self.start_download().await;
                let _ = reply.send(result);
            }
            Command::StopDownload { reply } => {
                let stopped = self.download_stop.take().is_some();
                self.download_stopping = stopped;
                let answer = if stopped {
                    Ok(())
                } else {
                    Err(ClientError::Refused("nothing is downloading".into()))
                };
                let _ = reply.send(answer);
            }
            Command::RequestGameStatus { founder, reply } => {
                self.run_session(reply, |session| session.request_game_status(&founder))
                    .await;
            }
            Command::EnginePid { reply } => {
                let _ = reply.send(Ok(self.engine.as_ref().and_then(Child::id)));
            }
            Command::StopEngine { reply } => {
                // The child is left in place: the exit is noticed by the same
                // `wait_engine` arm that handles a game closing itself, so
                // there is one path to "the game ended" rather than two.
                let running = match self.engine.as_mut() {
                    Some(child) => child.start_kill().is_ok(),
                    None => false,
                };
                let _ = reply.send(Ok(running));
            }
            Command::SetAutoLaunch { always, reply } => {
                self.auto_launch_always = always;
                let _ = reply.send(Ok(()));
            }
            Command::SetAway { away, reply } => {
                self.run_session(reply, |session| {
                    Ok::<_, std::convert::Infallible>(session.set_away(away))
                })
                .await;
            }
            Command::Ring { user, reply } => {
                self.run_session(reply, |session| {
                    Ok::<_, std::convert::Infallible>(session.ring(&user))
                })
                .await;
            }
            Command::AddBot {
                name,
                ai,
                team,
                ally_team,
                colour,
                reply,
            } => {
                self.run_session(reply, |session| {
                    Ok::<_, std::convert::Infallible>(
                        session.add_bot(&name, &ai, team, ally_team, colour),
                    )
                })
                .await;
            }
            Command::RemoveBot { name, reply } => {
                self.run_session(reply, |session| {
                    Ok::<_, std::convert::Infallible>(session.remove_bot(&name))
                })
                .await;
            }
            Command::RefreshFriends { reply } => {
                self.run_session(reply, |session| {
                    Ok::<_, std::convert::Infallible>(session.refresh_friends())
                })
                .await;
            }
            Command::FriendAction {
                action,
                user,
                reply,
            } => {
                self.run_session(reply, |session| {
                    Ok::<_, std::convert::Infallible>(session.friend_action(action, &user))
                })
                .await;
            }
            Command::SetReady { ready, reply } => {
                self.run_session(reply, |session| session.set_ready(ready))
                    .await;
            }
            Command::SetSide { side, reply } => {
                self.run_session(reply, |session| session.set_side(side))
                    .await;
            }
            Command::AllowPublicSeat(allow) => {
                self.allow_public_seat = allow;
                if let Some(conn) = self.conn.as_mut() {
                    conn.session.allow_public_seat(allow);
                }
            }
            Command::TakeSeat {
                team,
                ally_team,
                reply,
            } => {
                let Some(conn) = self.conn.as_mut() else {
                    let _ = reply.send(Err(ClientError::NotConnected));
                    return;
                };
                match conn.session.take_seat(team, ally_team) {
                    Ok(effects) => {
                        let _ = reply.send(Ok(()));
                        self.apply_effects(effects).await;
                    }
                    Err(err) => {
                        let _ = reply.send(Err(ClientError::Refused(err.to_string())));
                    }
                }
            }
            Command::SetOverlayConfigDir(dir) => self.overlay_config_dir = dir,
            Command::SetDataDir(data_dir) => {
                self.data_dir = data_dir;
                // Re-check against the new directory.
                self.checked = None;
                self.refresh_content().await;
            }
            Command::ReleaseSeat => {
                let Some(conn) = self.conn.as_mut() else {
                    return;
                };
                let effects = conn.session.release_seat();
                self.apply_effects(effects).await;
            }
            Command::HostPublic { reply } => {
                let Some(conn) = self.conn.as_ref() else {
                    let _ = reply.send(Err(ClientError::NotConnected));
                    return;
                };
                let ips = conn
                    .session
                    .spare_rooms()
                    .into_iter()
                    .filter_map(|room| room.ip)
                    .collect();
                self.start_probe(ips, Wanted::Public(reply));
            }
            Command::RequestPrivateHost { reply } => {
                let Some(conn) = self.conn.as_ref() else {
                    let _ = reply.send(Err(ClientError::NotConnected));
                    return;
                };
                let ips = conn.session.cluster_ips().into_values().collect();
                self.start_probe(ips, Wanted::Private(reply));
            }
            Command::Register {
                endpoint,
                request,
                email,
                password,
                reply,
            } => {
                self.register(endpoint, request, email, password, reply)
                    .await
            }
            Command::ConfirmAgreement { code, reply } => {
                self.run_session(reply, |session| {
                    Ok::<_, std::convert::Infallible>(session.confirm_agreement(&code))
                })
                .await;
            }
            Command::Shutdown => unreachable!("handled by the run loop"),
        }
    }

    /// Opens a connection whose only purpose is to create an account.
    ///
    /// Separate from `connect` because it must not become a logged-in session:
    /// the server answers `REGISTER` and leaves the connection unauthenticated,
    /// and someone registering is by definition not logged in yet.
    async fn register(
        &mut self,
        endpoint: Endpoint,
        request: LoginRequest,
        email: String,
        password: String,
        reply: Reply<()>,
    ) {
        if self.conn.is_some() {
            let _ = reply.send(Err(ClientError::AlreadyConnected));
            return;
        }
        match (self.connector)(endpoint, self.policy.clone()).await {
            Ok((transport, inbound)) => {
                let session = Session::new(
                    request,
                    self.hardware.properties.clone(),
                    self.hardware.machine_hash.clone(),
                )
                .registering(email, password);
                self.conn = Some(Connection {
                    transport,
                    inbound,
                    session,
                });
                self.register_reply = Some(reply);
            }
            Err(err) => {
                let _ = reply.send(Err(err.into()));
            }
        }
    }

    async fn connect(&mut self, endpoint: Endpoint, request: LoginRequest, reply: Reply<()>) {
        if self.conn.is_some() {
            let _ = reply.send(Err(ClientError::AlreadyConnected));
            return;
        }
        self.credentials = Some((endpoint.clone(), request.clone()));
        self.batcher.push(Delta::Phase(Some(Phase::Connecting)));
        self.flush();
        match (self.connector)(endpoint, self.policy.clone()).await {
            Ok((transport, inbound)) => {
                let mut session = Session::new(
                    request,
                    self.hardware.properties.clone(),
                    self.hardware.machine_hash.clone(),
                );
                // A fresh session starts guarded; the setting has to be
                // reapplied, or a reconnect would quietly revoke the choice.
                session.allow_public_seat(self.allow_public_seat);
                self.conn = Some(Connection {
                    transport,
                    inbound,
                    session,
                });
                self.login_reply = Some(reply);
            }
            Err(err) => {
                self.batcher.push(Delta::Phase(None));
                let _ = reply.send(Err(err.into()));
            }
        }
    }

    async fn handle_inbound(&mut self, inbound: Inbound) {
        match inbound {
            Inbound::Message(event) => {
                let Some(conn) = self.conn.as_mut() else {
                    return;
                };
                let effects = conn.session.handle(event.clone());
                let deltas = self
                    .projector
                    .project(&event, &effects, &conn.session.state);
                for delta in deltas {
                    self.batcher.push(delta);
                }
                self.apply_effects(effects).await;
            }
            Inbound::Policy(event) => match event {
                PolicyEvent::Delayed {
                    area,
                    pending,
                    wait,
                } => tracing::debug!(?area, pending, ?wait, "throttled"),
                other => tracing::info!(?other, "policy"),
            },
            Inbound::Closed { reason } => self.connection_lost(reason),
        }
    }

    /// Runs one session call and applies whatever it produced. The session's
    /// own error type only ever needs to reach the caller as text.
    async fn run_session<E: std::fmt::Display>(
        &mut self,
        reply: Reply<()>,
        call: impl FnOnce(&mut lobby_core::Session) -> Result<Vec<Effect>, E>,
    ) {
        let Some(conn) = self.conn.as_mut() else {
            let _ = reply.send(Err(ClientError::NotConnected));
            return;
        };
        match call(&mut conn.session) {
            Ok(effects) => {
                let _ = reply.send(Ok(()));
                self.apply_effects(effects).await;
            }
            Err(err) => {
                let _ = reply.send(Err(ClientError::Refused(err.to_string())));
            }
        }
    }

    async fn apply_effects(&mut self, effects: Vec<Effect>) {
        // A queue, not a loop over the argument: joining the private room we
        // asked for produces effects of its own.
        let mut queue: std::collections::VecDeque<Effect> = effects.into();
        while let Some(effect) = queue.pop_front() {
            match effect {
                // The room a cluster manager made for us; joining it is the
                // whole point of having asked.
                Effect::PrivateHostReady { id, password } => {
                    let Some(conn) = self.conn.as_mut() else {
                        continue;
                    };
                    let script_password =
                        format!("{}{}", rand::random::<u16>(), rand::random::<u16>());
                    let effects = conn
                        .session
                        .join_battle(id, Some(&password), script_password);
                    queue.extend(effects);
                }
                Effect::PrivateHostOffered { manager, password } => {
                    self.batcher.push(Delta::Notice {
                        level: lobby_ui::NoticeLevel::Info,
                        text: format!("{manager} is starting a room; password {password}"),
                    });
                }
                Effect::Hosting { founder, alone } => {
                    let rtt = self
                        .conn
                        .as_ref()
                        .and_then(|conn| {
                            conn.session
                                .state
                                .battles
                                .values()
                                .find(|b| b.founder == founder)
                        })
                        .and_then(|battle| battle.ip.parse().ok())
                        .and_then(|ip| self.known_rtts().get(&ip).copied())
                        .map_or(String::new(), |rtt| format!(" ({} ms)", rtt.as_millis()));
                    let (level, text) = if alone {
                        (
                            lobby_ui::NoticeLevel::Info,
                            format!("joined {founder}{rtt}; you boss it"),
                        )
                    } else {
                        (
                            lobby_ui::NoticeLevel::Warning,
                            format!(
                                "{founder} was not empty by the time we got in; nobody was bossed"
                            ),
                        )
                    };
                    self.batcher.push(Delta::Notice { level, text });
                }
                Effect::Send(envelope) => {
                    if let Err(err) = self.send_line(envelope).await {
                        self.connection_lost(err.to_string());
                        return;
                    }
                }
                Effect::Ready => {
                    self.reconnect.stop();
                    self.reply_login(Ok(()));
                    self.send_snapshot();
                    // The server volunteers nothing about friendships, so the
                    // list is asked for once the login flood has settled.
                    // Without this a filter that depends on it would quietly
                    // match nobody.
                    if let Some(conn) = self.conn.as_mut() {
                        let effects = conn.session.refresh_friends();
                        queue.extend(effects);
                    }
                }
                Effect::LoginDenied { reason } => self.refuse(reason).await,
                Effect::AgreementRequired { .. } => {
                    // Not a refusal: an account that has just been created is
                    // expected to land here, and the code it needs is in an
                    // email. Hanging up would throw away the very connection
                    // that can confirm it.
                    self.batcher.push(Delta::Notice {
                        level: lobby_ui::NoticeLevel::Warning,
                        text: "this account must confirm the emailed code before it can log in"
                            .into(),
                    });
                    if let Some(reply) = self.login_reply.take() {
                        let _ = reply.send(Err(ClientError::Refused(
                            "confirm the code emailed to you".into(),
                        )));
                    }
                }
                Effect::Registered => {
                    if let Some(reply) = self.register_reply.take() {
                        let _ = reply.send(Ok(()));
                    }
                    // The connection has done its one job.
                    self.disconnect().await;
                }
                Effect::RegistrationDenied { reason } => {
                    if let Some(reply) = self.register_reply.take() {
                        let _ = reply.send(Err(ClientError::Refused(reason)));
                    }
                    self.disconnect().await;
                }
                Effect::Redirect { host, port } => {
                    self.refuse(format!(
                        "server redirects to {host}:{}",
                        port.map_or("?".into(), |p| p.to_string())
                    ))
                    .await
                }
                Effect::Disconnected { reason, flood } => {
                    if flood && let Some(conn) = &self.conn {
                        let wait = Duration::from_secs_f64(self.policy.login.after_flood_secs);
                        let _ = conn
                            .transport
                            .trip(Area::Login, Instant::now() + wait)
                            .await;
                    }
                    self.refuse(format!("disconnected: {reason}")).await;
                }
                Effect::Joined { .. } => self.reply_join(Ok(())),
                Effect::JoinFailed { reason } => self.reply_join(Err(ClientError::Refused(reason))),
                Effect::LeftBattle { .. } => self.game = None,
                Effect::GameStopped => {
                    if self.game.take().is_some() {
                        self.batcher.push(Delta::Alert {
                            kind: lobby_ui::AlertKind::GameEnded,
                            text: "your room's game has finished".into(),
                        });
                    }
                }
                Effect::GameRunning {
                    id,
                    ip,
                    port,
                    script_password,
                    just_started,
                } => {
                    // Raised once here rather than in the projector: the
                    // effect fires whenever the host's status changes, and
                    // `self.game` already tracks whether it is news. Walking
                    // into a game already under way is not news of a start.
                    if just_started && self.game.is_none() {
                        self.batcher.push(Delta::Alert {
                            kind: lobby_ui::AlertKind::GameStarting,
                            text: "your room's game has started".into(),
                        });
                    }
                    self.game = Some(Game {
                        view: GameRunningView { id, ip, port },
                        script_password,
                    });
                    // Either somebody asked to launch before the game existed,
                    // or a game started around us and the setting says that is
                    // reason enough.
                    //
                    // `just_started` is the whole distinction: joining a room
                    // whose game is already under way reports the same running
                    // game, and launching on that would drop anyone who
                    // wandered in to look straight into a match they had not
                    // asked to watch. Chobby draws the same line — it offers
                    // to watch, and starts on its own only when the game does.
                    // Never without the content either: that only produces an
                    // engine that quits with a sync error.
                    let wanted = self.auto_launch.take().or_else(|| {
                        (just_started
                            && self.auto_launch_always
                            && self.content_ready
                            && self.engine.is_none())
                        .then(|| self.data_dir())
                        .flatten()
                    });
                    if let Some(data_dir) = wanted
                        && let Err(err) = self.launch_engine(data_dir).await
                    {
                        self.batcher.push(Delta::Notice {
                            level: lobby_ui::NoticeLevel::Error,
                            text: err.to_string(),
                        });
                    }
                }
                // Projected into deltas; the runtime itself has nothing to do.
                // Everything the projector turns into a delta on its own.
                Effect::LoggedIn { .. }
                | Effect::GameInProgress { .. }
                | Effect::Notice(_)
                | Effect::BattleChat { .. }
                | Effect::PrivateChat { .. }
                | Effect::ChannelChat { .. }
                | Effect::ChannelJoined { .. }
                | Effect::ChannelJoinFailed { .. }
                | Effect::ChannelLeft { .. }
                | Effect::ChannelChanged { .. }
                | Effect::ChannelsListed
                | Effect::FriendsChanged
                | Effect::BossChanged
                | Effect::ServerSaid { .. }
                | Effect::Rung { .. }
                | Effect::ModOptionsChanged { .. }
                | Effect::VoteChanged => {}
            }
        }
    }

    fn project_effects(&mut self, effects: &[Effect]) {
        let Some(conn) = self.conn.as_ref() else {
            return;
        };
        let mut deltas = Vec::new();
        self.projector
            .project_effects(effects, &conn.session.state, &mut deltas);
        for delta in deltas {
            self.batcher.push(delta);
        }
    }

    async fn send_line(&mut self, envelope: Envelope) -> Result<(), ClientError> {
        let Some(conn) = self.conn.as_ref() else {
            return Err(ClientError::NotConnected);
        };
        conn.transport.send(envelope).await?;
        Ok(())
    }

    /// The server said no (before or after login): answer whoever waits, then drop the link.
    /// A refusal we can do nothing about, except when it is the server telling
    /// us to wait — which is the one refusal worth answering by waiting.
    async fn refuse(&mut self, reason: String) {
        let transient = reason.to_ascii_lowercase().contains("flood protection");
        if transient && self.credentials.is_some() {
            self.reconnect
                .disconnected(Instant::now(), false, rand::random::<f64>());
            tracing::info!(reason, "login refused as flooding; will wait and retry");
        }
        self.reply_login(Err(ClientError::Refused(reason.clone())));
        self.reply_join(Err(ClientError::Refused(reason)));
        if let Some(conn) = self.conn.take() {
            conn.transport.shutdown().await;
        }
        self.game = None;
        self.auto_launch = None;
        self.batcher.push(Delta::Phase(None));
    }

    fn connection_lost(&mut self, reason: String) {
        tracing::warn!(reason, "connection lost");
        // A drop while a game is running gets the longer wait: it was probably
        // the server, and everyone in that game is about to try at once.
        let in_game = self.game.is_some() || self.engine.is_some();
        if self.credentials.is_some() {
            self.reconnect
                .disconnected(Instant::now(), in_game, rand::random::<f64>());
        }
        self.reply_login(Err(ClientError::Refused(reason.clone())));
        self.reply_join(Err(ClientError::Refused(reason.clone())));
        self.conn = None;
        self.game = None;
        self.auto_launch = None;
        let text = if self.reconnect.is_armed() {
            format!("connection lost: {reason} — trying again shortly")
        } else {
            format!("connection lost: {reason}")
        };
        self.batcher.push(Delta::Notice {
            level: lobby_ui::NoticeLevel::Error,
            text,
        });
        self.batcher.push(Delta::Phase(None));
    }

    /// Tries the last known credentials again.
    async fn try_reconnect(&mut self) {
        let Some((endpoint, request)) = self.credentials.clone() else {
            self.reconnect.stop();
            return;
        };
        self.reconnect.attempted(Instant::now());
        tracing::info!("reconnecting");
        let (tx, _rx) = oneshot::channel();
        self.connect(endpoint, request, tx).await;
    }

    async fn disconnect(&mut self) {
        let Some(mut conn) = self.conn.take() else {
            return;
        };
        let leaving = conn.session.leave_battle();
        let in_room = !leaving.is_empty();
        if in_room {
            for effect in leaving {
                if let Effect::Send(envelope) = effect {
                    let _ = conn.transport.send(envelope).await;
                }
            }
            // Let the writer flush LEAVEBATTLE before the socket goes away.
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        conn.transport.shutdown().await;
        self.game = None;
        self.auto_launch = None;
        if in_room {
            self.batcher.push(Delta::MyBattle(None));
            self.batcher.push(Delta::GameRunning(None));
        }
        self.batcher.push(Delta::Phase(None));
    }

    async fn launch_engine(&mut self, data_dir: PathBuf) -> Result<(), ClientError> {
        let (engine_version, url, in_game) = {
            let Some(conn) = self.conn.as_mut() else {
                return Err(ClientError::NotConnected);
            };
            let Some(game) = self.game.as_ref() else {
                return Err(ClientError::Engine("no game is running".into()));
            };
            let state = &conn.session.state;
            let me = state.me.clone().unwrap_or_default();
            let room = state
                .battles
                .get(&game.view.id)
                .ok_or_else(|| ClientError::Engine("the room is gone".into()))?;
            // Better a message naming what is missing than an engine that
            // starts and cannot join.
            let available = content::Library::new(&data_dir).check(
                &room.engine_version,
                &room.game_name,
                &room.map_name,
            );
            if !available.complete() {
                return Err(ClientError::Engine(format!(
                    "this room needs content you do not have: {}",
                    available.missing().join(", ")
                )));
            }
            let engine_version = room.engine_version.clone();
            let url = recoil::spring_url(&me, &game.script_password, &game.view.ip, game.view.port);
            (engine_version, url, conn.session.set_in_game(true))
        };
        // SPADS /adduser's us to the running game only once the lobby shows us in
        // game; the engine needs a few seconds to reach the host.
        for effect in in_game {
            if let Effect::Send(envelope) = effect {
                self.send_line(envelope).await?;
            }
        }
        let child = launch::spawn(
            &data_dir,
            &engine_version,
            url,
            self.overlay_config_dir.as_deref(),
        )
        .map_err(ClientError::Engine)?;
        let pid = child.id();
        self.engine = Some(child);
        self.set_engine(EngineStatus::Running { pid });
        Ok(())
    }

    async fn engine_exited(&mut self, status: std::io::Result<ExitStatus>) {
        self.engine = None;
        let code = status.ok().and_then(|s| s.code());
        tracing::info!(?code, "engine exited");
        self.set_engine(EngineStatus::Exited { code });
        if let Some(conn) = self.conn.as_mut() {
            let effects = conn.session.set_in_game(false);
            self.apply_effects(effects).await;
        }
    }

    fn set_engine(&mut self, status: EngineStatus) {
        self.engine_status = status;
        self.batcher.push(Delta::Engine(status));
    }

    fn reply_login(&mut self, result: Result<(), ClientError>) {
        if let Some(reply) = self.login_reply.take() {
            let _ = reply.send(result);
        }
    }

    fn reply_join(&mut self, result: Result<(), ClientError>) {
        if let Some(reply) = self.join_reply.take() {
            let _ = reply.send(result);
        }
    }

    fn snapshot(&self) -> Snapshot {
        match &self.conn {
            Some(conn) => Snapshot::from_state(
                &conn.session.state,
                self.game.as_ref().map(|g| g.view.clone()),
                self.engine_status,
            ),
            None => Snapshot {
                engine: self.engine_status,
                ..Snapshot::disconnected()
            },
        }
    }

    /// A snapshot supersedes whatever deltas were waiting.
    /// Sends the whole state, dropping the pending deltas it supersedes.
    ///
    /// Chat is the exception: a snapshot says nothing about what anyone said,
    /// so discarding batched chat would silently swallow whatever arrived in
    /// the moment before it — which is exactly when the message of the day and
    /// the first channel traffic land.
    fn send_snapshot(&mut self) {
        let kept: Vec<Delta> = self
            .batcher
            .take()
            .into_iter()
            .filter(|delta| matches!(delta, Delta::Chat(_)))
            .collect();
        let snapshot = self.snapshot();
        self.send_ui(UiMessage::Snapshot(Box::new(snapshot)));
        if !kept.is_empty() {
            self.send_ui(UiMessage::Deltas(kept));
        }
    }

    fn flush(&mut self) {
        if self.batcher.is_empty() {
            return;
        }
        let deltas = self.batcher.take();
        self.send_ui(UiMessage::Deltas(deltas));
    }

    fn send_ui(&mut self, message: UiMessage) {
        let Some(ui) = self.ui.as_ref() else {
            return;
        };
        if ui.send(message).is_err() {
            tracing::info!("ui transport closed");
            self.ui = None;
        }
    }
}

/// Resolves with the next inbound item; never, when there is no connection.
async fn recv_inbound(conn: &mut Option<Connection>) -> Inbound {
    match conn {
        Some(conn) => conn.inbound.recv().await.unwrap_or(Inbound::Closed {
            reason: "transport task ended".into(),
        }),
        None => std::future::pending().await,
    }
}

/// Resolves when the launched engine exits; never, when none was launched.
async fn wait_engine(engine: &mut Option<Child>) -> std::io::Result<ExitStatus> {
    match engine {
        Some(child) => child.wait().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use lobby_ui::Collector;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

    use super::*;

    /// A connector whose single connection ends at the returned stream: the fake server.
    fn in_memory() -> (Connector, DuplexStream) {
        let (client_side, server_side) = tokio::io::duplex(64 * 1024);
        let client_side = std::sync::Mutex::new(Some(client_side));
        let connector: Connector = Arc::new(move |_endpoint, policy| {
            let stream = client_side.lock().unwrap().take().expect("one connection");
            Box::pin(async move { Ok(Transport::from_stream(stream, policy)) })
        });
        (connector, server_side)
    }

    #[tokio::test]
    async fn login_flood_yields_one_snapshot_then_deltas() {
        let (connector, server) = in_memory();
        let (server_read, mut server_write) = tokio::io::split(server);
        let mut server_lines = BufReader::new(server_read).lines();

        let client = Client::spawn_with(
            ThrottlePolicy::default(),
            Hardware::stub(),
            connector,
            Arc::new(latency::Unmeasured),
        );
        let ui = Collector::default();
        client.subscribe(ui.clone()).await.unwrap();

        let endpoint = Endpoint::parse("test:8200", false).unwrap();
        let login = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .login(endpoint, LoginRequest::new("me", "pw", "test", "h h"))
                    .await
            }
        });

        server_write
            .write_all(b"TASSERVER 0.38 * 8201 0\n")
            .await
            .unwrap();
        let sent = server_lines.next_line().await.unwrap().unwrap();
        assert!(sent.starts_with("LOGIN me "), "{sent}");
        server_write
            .write_all(
                b"ACCEPTED me\nADDUSER me SE 1 LuaLobby Chobby\nADDUSER host GB 2 SPADS\n\
                  BATTLEOPENED 5 0 0 host 1.2.3.4 8452 16 0 0 h R\tv\tm\tt\tg\nLOGININFOEND\n",
            )
            .await
            .unwrap();
        login.await.unwrap().unwrap();

        server_write
            .write_all(b"ADDUSER bob DE 3 x\n")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let messages = ui.take();
        let snapshot_at = messages
            .iter()
            .position(|m| matches!(m, UiMessage::Snapshot(s) if s.phase == Some(Phase::Ready)))
            .expect("ready snapshot");
        if let UiMessage::Snapshot(s) = &messages[snapshot_at] {
            assert_eq!(s.battles.len(), 1);
            assert_eq!(s.users.len(), 2);
        }
        let after: Vec<&Delta> = messages[snapshot_at + 1..]
            .iter()
            .filter_map(|m| match m {
                UiMessage::Deltas(d) => Some(d.iter()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(
            after
                .iter()
                .any(|d| matches!(d, Delta::UserAdded(u) if u.name == "bob"))
        );

        assert_eq!(client.snapshot().await.unwrap().users.len(), 3);
        client.shutdown().await;
    }

    /// The next line the client sends that starts with `prefix`, skipping
    /// whatever else it had queued.
    async fn line_starting_with(
        lines: &mut tokio::io::Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
        prefix: &str,
    ) -> String {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let line = lines.next_line().await.unwrap().expect("client hung up");
                if line.starts_with(prefix) {
                    return line;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no line starting with {prefix}"))
    }

    #[tokio::test]
    async fn hosting_a_public_room_takes_the_spare_and_bosses_it() {
        let (connector, server) = in_memory();
        let (server_read, mut server_write) = tokio::io::split(server);
        let mut server_lines = BufReader::new(server_read).lines();

        let client = Client::spawn_with(
            ThrottlePolicy::default(),
            Hardware::stub(),
            connector,
            Arc::new(latency::Unmeasured),
        );
        let endpoint = Endpoint::parse("test:8200", false).unwrap();
        let login = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .login(endpoint, LoginRequest::new("me", "pw", "test", "h h"))
                    .await
            }
        });
        server_write
            .write_all(b"TASSERVER 0.38 * 8201 0\n")
            .await
            .unwrap();
        line_starting_with(&mut server_lines, "LOGIN ").await;
        // One busy room and one spare, on the same cluster.
        server_write
            .write_all(
                b"ACCEPTED me\nADDUSER me SE 1 LuaLobby Chobby\n\
                  ADDUSER Host[EU1][001] DE 2 SPADS\nADDUSER Host[EU1][002] DE 3 SPADS\n\
                  ADDUSER alice DE 4 x\n\
                  BATTLEOPENED 5 0 0 Host[EU1][001] 1.2.3.4 8452 16 0 0 h R\tv\tm\tt\tg\n\
                  BATTLEOPENED 6 0 0 Host[EU1][002] 1.2.3.4 8452 16 0 0 h R\tv\tm\tt\tg\n\
                  JOINEDBATTLE 5 alice\nLOGININFOEND\n",
            )
            .await
            .unwrap();
        login.await.unwrap().unwrap();

        let hosting = tokio::spawn({
            let client = client.clone();
            async move { client.host_public().await }
        });
        let join = line_starting_with(&mut server_lines, "JOINBATTLE ").await;
        assert!(join.starts_with("JOINBATTLE 6 empty "), "{join}");
        assert_eq!(hosting.await.unwrap().unwrap(), 6);

        server_write
            .write_all(b"JOINBATTLE 6 h\nJOINEDBATTLE 6 me\n")
            .await
            .unwrap();
        assert_eq!(
            line_starting_with(&mut server_lines, "SAYBATTLE ").await,
            "SAYBATTLE !boss me"
        );
        assert_eq!(
            line_starting_with(&mut server_lines, "SAYBATTLE ").await,
            "SAYBATTLE !preset custom"
        );
        client.shutdown().await;
    }
}
