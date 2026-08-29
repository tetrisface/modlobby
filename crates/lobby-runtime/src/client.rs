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
    Batcher, Delta, EngineStatus, GameRunningView, Phase, Projector, Snapshot, UiMessage,
    UiTransport,
};
use spring_protocol::battle::{self, TooLong};
use spring_protocol::policy::PolicyEvent;
use spring_protocol::{
    Area, Endpoint, Envelope, Inbound, LoginRequest, ThrottlePolicy, Transport, TransportError,
};
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
    projector: Projector,
    batcher: Batcher,
}

impl Runtime {
    fn new(
        rx: mpsc::Receiver<Command>,
        policy: ThrottlePolicy,
        hardware: Hardware,
        connector: Connector,
    ) -> Self {
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
            projector: Projector::new(),
            batcher: Batcher::default(),
        }
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
            };
            match next {
                Next::Command(Command::Shutdown) => {
                    self.disconnect().await;
                    self.flush();
                    return;
                }
                Next::Command(command) => self.handle_command(command).await,
                Next::Inbound(inbound) => {
                    self.handle_inbound(inbound).await;
                    while let Some(more) = self.try_recv_inbound() {
                        self.handle_inbound(more).await;
                    }
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

    async fn apply_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Send(envelope) => {
                    if let Err(err) = self.send_line(envelope).await {
                        self.connection_lost(err.to_string());
                        return;
                    }
                }
                Effect::Ready => {
                    self.reply_login(Ok(()));
                    self.send_snapshot();
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
                Effect::LoggedIn { .. }
                | Effect::Notice(_)
                | Effect::BattleChat { .. }
                | Effect::PrivateChat { .. } => {}
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
            let engine_version = state
                .battles
                .get(&game.view.id)
                .map(|b| b.engine_version.clone())
                .ok_or_else(|| ClientError::Engine("the room is gone".into()))?;
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
        self.send_ui(UiMessage::Snapshot(snapshot));
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
