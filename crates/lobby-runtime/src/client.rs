//! One actor owns the connection, the reducer, the engine child and the UI
//! transport; front ends send [`Command`]s through a [`Client`] handle. Every
//! inbound line is reduced, projected into deltas and batched: a burst that is
//! already queued becomes one `Deltas` message.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lobby_core::{Effect, Session};
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

use crate::launch;
use crate::platform::Hardware;

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
    ReleaseSeat,
    SetDataDir(Option<PathBuf>),
    RequestPrivateHost {
        region: String,
        reply: Reply<String>,
    },
    Shutdown,
}

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
        Self::spawn_with(policy, hardware, connector)
    }

    pub fn spawn_with(policy: ThrottlePolicy, hardware: Hardware, connector: Connector) -> Self {
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(Runtime::new(rx, policy, hardware, connector).run());
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

    /// Asks a cluster manager in `region` for a room of our own; the runtime
    /// joins it when it appears. Returns the manager it asked.
    pub async fn request_private_host(&self, region: String) -> Result<String, ClientError> {
        self.ask(|reply| Command::RequestPrivateHost { region, reply })
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
    login_reply: Option<Reply<()>>,
    join_reply: Option<Reply<()>>,
    /// Where BAR's content lives; `None` falls back to the launcher's directory.
    data_dir: Option<PathBuf>,
    /// The room's (engine, game, map) the content check last ran against;
    /// scanning the rapid index is too slow to repeat per message.
    checked: Option<(String, String, String)>,
    /// What a running pr-downloader was asked for, or `None` when none is.
    downloading: Option<String>,
    download_tx: mpsc::Sender<DownloadEvent>,
    download_rx: mpsc::Receiver<DownloadEvent>,
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
    ) -> Self {
        // A short queue: progress lines arrive far faster than the front end
        // needs them, and the batcher coalesces what gets through anyway.
        let (download_tx, download_rx) = mpsc::channel(16);
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
            login_reply: None,
            join_reply: None,
            data_dir: None,
            checked: None,
            downloading: None,
            download_tx,
            download_rx,
            projector: Projector::new(),
            batcher: Batcher::default(),
        }
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

        self.downloading = Some(what.clone());
        self.batcher.push(Delta::Download(DownloadStatus::Running {
            what: what.clone(),
            current: 0,
            total: 0,
        }));

        let events = self.download_tx.clone();
        tokio::spawn(async move {
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
                        if let Some(progress) = recoil::Progress::parse(&line) {
                            let _ = events.send(DownloadEvent::Progress(progress)).await;
                        }
                    }
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
                self.batcher.push(Delta::Download(if ok {
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
        self.checked = Some(key);
        self.batcher.push(Delta::Content {
            engine: available.engine,
            game: available.game,
            map: available.map,
        });
        self.set_synced(available.complete()).await;
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
            };
            match next {
                Next::Command(Command::Shutdown) => {
                    self.disconnect().await;
                    self.flush();
                    return;
                }
                Next::Command(command) => self.handle_command(command).await,
                Next::Download(event) => self.on_download(event).await,
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
            Command::Logout => self.disconnect().await,
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
            Command::DownloadMissing { reply } => {
                let result = self.start_download().await;
                let _ = reply.send(result);
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
            Command::RequestPrivateHost { region, reply } => {
                let Some(conn) = self.conn.as_mut() else {
                    let _ = reply.send(Err(ClientError::NotConnected));
                    return;
                };
                let Some(manager) = conn.session.cluster_managers(&region).first().copied() else {
                    let _ = reply.send(Err(ClientError::Refused(format!(
                        "no cluster manager online for {region}"
                    ))));
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
            Command::Shutdown => unreachable!("handled by the run loop"),
        }
    }

    async fn connect(&mut self, endpoint: Endpoint, request: LoginRequest, reply: Reply<()>) {
        if self.conn.is_some() {
            let _ = reply.send(Err(ClientError::AlreadyConnected));
            return;
        }
        self.batcher.push(Delta::Phase(Some(Phase::Connecting)));
        self.flush();
        match (self.connector)(endpoint, self.policy.clone()).await {
            Ok((transport, inbound)) => {
                let session = Session::new(
                    request,
                    self.hardware.properties.clone(),
                    self.hardware.machine_hash.clone(),
                );
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
                Effect::Send(envelope) => {
                    if let Err(err) = self.send_line(envelope).await {
                        self.connection_lost(err.to_string());
                        return;
                    }
                }
                Effect::Ready => {
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
                    self.refuse("the account must accept the user agreement first".into())
                        .await
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
                Effect::GameRunning {
                    id,
                    ip,
                    port,
                    script_password,
                } => {
                    self.game = Some(Game {
                        view: GameRunningView { id, ip, port },
                        script_password,
                    });
                    if let Some(data_dir) = self.auto_launch.take()
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
    async fn refuse(&mut self, reason: String) {
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
        self.reply_login(Err(ClientError::Refused(reason.clone())));
        self.reply_join(Err(ClientError::Refused(reason.clone())));
        self.conn = None;
        self.game = None;
        self.auto_launch = None;
        self.batcher.push(Delta::Notice {
            level: lobby_ui::NoticeLevel::Error,
            text: format!("connection lost: {reason}"),
        });
        self.batcher.push(Delta::Phase(None));
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
        let child = launch::spawn(&data_dir, &engine_version, url).map_err(ClientError::Engine)?;
        self.engine = Some(child);
        self.set_engine(EngineStatus::Running);
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
    fn send_snapshot(&mut self) {
        self.batcher.take();
        let snapshot = self.snapshot();
        self.send_ui(UiMessage::Snapshot(Box::new(snapshot)));
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

        let client = Client::spawn_with(ThrottlePolicy::default(), Hardware::stub(), connector);
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
}
