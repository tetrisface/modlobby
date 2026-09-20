//! One actor owns the connection, the reducer, the engine child and the UI
//! transport; front ends send [`Command`]s through a [`Client`] handle. Every
//! inbound line is reduced, projected into deltas and batched: a burst that is
//! already queued becomes one `Deltas` message.

use std::collections::VecDeque;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::{Duration, Instant};

use content::DataDirs;
use lobby_core::{Effect, Session, hosting};
use lobby_ui::{
	Batcher, ContentView, Delta, DownloadStatus, EngineStatus, GameRunningView, PasteStatus, Phase,
	Projector, SKIRMISH_ROOM, ServerSnapshot, Snapshot, UiMessage, UiTransport,
};
use spring_protocol::battle::TooLong;
use spring_protocol::policy::PolicyEvent;
use spring_protocol::{
	Area, Endpoint, Envelope, Inbound, LoginRequest, ThrottlePolicy, Transport, TransportError,
	Way, server_id,
};
use tokio::io::AsyncReadExt;
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};

use crate::idle;
use crate::latency::{self, Latency};
use crate::launch;
use crate::platform::Hardware;
use crate::player_files;
use crate::reconnect;
use crate::ways::{self, Ways};

mod links;

use links::{Opened, Purpose, Server, recv_any};

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
	/// A reconnect was asked for before any login this run, or after a logout.
	#[error("nothing to reconnect with; log in first")]
	NoCredentials,
}

type Reply<T> = oneshot::Sender<Result<T, ClientError>>;
type Connected = (Transport, mpsc::Receiver<Inbound>, Way);
type ConnectFuture = Pin<Box<dyn Future<Output = Result<Connected, TransportError>> + Send>>;

/// How the runtime reaches a server; tests hand it an in-memory stream.
pub type Connector = Arc<dyn Fn(Endpoint, ThrottlePolicy) -> ConnectFuture + Send + Sync>;

/// Whether games may be fetched through a rapid master index, by its URL; the
/// refusal is for a person to read. The reading of somebody else's rapid
/// server is handed in (`content::rapid::Vetter`), so the runtime needs no
/// HTTP client and a test needs no network.
pub type Vet =
	Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;

/// With nobody to read another rapid server, only BAR's is fetched from.
fn vet_bars_only() -> Vet {
	Arc::new(|master| {
		Box::pin(async move {
			if master == recoil::RAPID_REPO_MASTER {
				return Ok(());
			}
			Err(format!(
				"nothing here can check the rapid server at {master}"
			))
		})
	})
}

enum Command {
	Subscribe(Box<dyn UiTransport>),
	Login {
		endpoint: Endpoint,
		request: LoginRequest,
		reply: Reply<()>,
	},
	/// `None` logs out of every server.
	Logout {
		server: Option<String>,
	},
	/// Drops the remembered way into `host`, so the next connect races every way again.
	ForgetWay {
		host: String,
	},
	/// Tries the server's last login again now, ahead of the retry timer —
	/// over `endpoint` when one is given, which is how a change to the
	/// server's ports or to what it may be reached by takes hold.
	Reconnect {
		server: String,
		endpoint: Option<Endpoint>,
		reply: Reply<()>,
	},
	/// Creates an account and logs in on it, answering with the agreement the
	/// server replies to that login with.
	Register {
		endpoint: Endpoint,
		request: LoginRequest,
		email: String,
		password: String,
		reply: Reply<Vec<String>>,
	},
	/// Confirms the emailed code for an account that has just been created.
	ConfirmAgreement {
		server: String,
		code: String,
		reply: Reply<()>,
	},
	Snapshot(oneshot::Sender<Snapshot>),
	/// Opens the room with no server behind it, or replaces the one there.
	OpenSkirmish {
		room: Box<skirmish::Room>,
		reply: Reply<()>,
	},
	CloseSkirmish {
		reply: Reply<()>,
	},
	/// One change to that room. `Say` carries the console, which is the only
	/// one of them that can ask for a launch.
	Skirmish {
		act: Box<skirmish::Act>,
		reply: Reply<()>,
	},
	LaunchSkirmish {
		reply: Reply<()>,
	},
	SkirmishDownload {
		reply: Reply<()>,
	},
	/// The room itself, for whoever wants to write it down.
	SkirmishRoom(oneshot::Sender<Option<Box<skirmish::Room>>>),
	/// Puts a preset back into it.
	SkirmishPreset {
		preset: Box<presets::Preset>,
		sections: presets::Sections,
		reply: Reply<skirmish::preset::Applied>,
	},
	JoinBattle {
		server: String,
		id: u32,
		password: Option<String>,
		reply: Reply<()>,
	},
	LeaveBattle,
	Launch {
		dirs: DataDirs,
		reply: Reply<()>,
	},
	Say {
		text: String,
		reply: Reply<()>,
	},
	JoinChannel {
		server: String,
		room: String,
		key: Option<String>,
		reply: Reply<()>,
	},
	LeaveChannel {
		server: String,
		room: String,
		reply: Reply<()>,
	},
	SayChannel {
		server: String,
		room: String,
		text: String,
		reply: Reply<()>,
	},
	SayPrivate {
		server: String,
		user: String,
		text: String,
		reply: Reply<()>,
	},
	ListChannels {
		server: String,
		reply: Reply<()>,
	},
	/// Every server's friend list, asked for again.
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
	/// Drops what is left of a paste in the queue.
	CancelPaste {
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
	UpdateBot {
		name: String,
		team: u8,
		ally_team: u8,
		handicap: u8,
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
	SetAutoDownload {
		on: bool,
		reply: Reply<()>,
	},
	SetIdleTimeout {
		timeout: Option<Duration>,
		reply: Reply<()>,
	},
	/// Someone touched the window.
	Activity,
	EnginePid {
		reply: Reply<Option<u32>>,
	},
	StopEngine {
		reply: Reply<bool>,
	},
	RequestGameStatus {
		server: String,
		founder: String,
		reply: Reply<()>,
	},
	PlayReplay {
		dirs: DataDirs,
		path: String,
		reply: Reply<()>,
	},
	FriendAction {
		server: String,
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
	ReleaseSeat,
	SetDataDir(Option<PathBuf>),
	/// Each server's rapid master index, by server id; one without is BAR's.
	SetRapidMasters(BTreeMap<String, String>),
	SetVet(Vet),
	/// The disk changed under us — an engine was installed — so the room's
	/// content is worth asking about again.
	RecheckContent,
	/// Where to keep a config to launch with when the user's own settings
	/// would put the game in exclusive full screen. `None` while the
	/// overlay is switched off, which is also when nothing is written.
	SetOverlayConfigDir(Option<PathBuf>),
	SetMenuArchive(Option<recoil::MenuArchive>),
	/// Where to keep the skirmish room between runs.
	SetSkirmishPath(Option<PathBuf>),
	/// Asks a cluster manager for a room of our own; the runtime joins it
	/// when it appears. Replies with the manager asked.
	RequestPrivateHost {
		server: String,
		reply: Reply<String>,
	},
	/// Joins an empty public autohost, making it ours. Replies with its id.
	HostPublic {
		server: String,
		reply: Reply<u32>,
	},
	Shutdown,
}

/// Who asked for the latencies, and what they are for, on which server.
enum Wanted {
	Public {
		server: String,
		reply: Reply<u32>,
	},
	Private {
		server: String,
		reply: Reply<String>,
	},
}

/// The latencies a request went out to measure, back from the probe task.
struct Probe {
	measured: HashMap<Ipv4Addr, Option<Duration>>,
	wanted: Wanted,
}

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
	/// `state_dir` is where what was measured is kept between runs — host
	/// latencies, the way into each server; `None` measures afresh each run.
	pub fn spawn(policy: ThrottlePolicy, hardware: Hardware, state_dir: Option<PathBuf>) -> Self {
		let connector: Connector = Arc::new(|endpoint, policy| {
			Box::pin(async move { Transport::connect(&endpoint, policy).await })
		});
		Self::spawn_with(
			policy,
			hardware,
			connector,
			Arc::new(latency::IcmpEcho),
			state_dir,
		)
	}

	pub fn spawn_with(
		policy: ThrottlePolicy,
		hardware: Hardware,
		connector: Connector,
		latency: Arc<dyn Latency>,
		state_dir: Option<PathBuf>,
	) -> Self {
		let (tx, rx) = mpsc::channel(64);
		let runtime = Runtime::new(rx, policy, hardware, connector, latency, state_dir);
		tokio::spawn(runtime.run());
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

	/// Logs out of `server`, or of every server with `None`, and stops
	/// coming back to it.
	pub async fn logout(&self, server: Option<String>) -> Result<(), ClientError> {
		self.send(Command::Logout { server }).await
	}

	/// Forgets which way into `host` worked, so the next connect tries every way.
	pub async fn forget_way(&self, host: String) -> Result<(), ClientError> {
		self.send(Command::ForgetWay { host }).await
	}

	/// Logs in to `server` again with its last credentials, now rather than
	/// when the runtime's own retry falls due. Resolves like [`Self::login`].
	///
	/// `endpoint` is the server as it is set up now, where the caller knows:
	/// the one kept from the last login would go on trying ports that have
	/// since been changed, or refusing a plaintext that has since been allowed.
	pub async fn reconnect(
		&self,
		server: String,
		endpoint: Option<Endpoint>,
	) -> Result<(), ClientError> {
		self.ask(|reply| Command::Reconnect {
			server,
			endpoint,
			reply,
		})
		.await
	}

	/// Creates an account. Resolves when the server has accepted or refused it.
	///
	/// The account cannot log in yet: the server emails a code that
	/// [`Self::confirm_agreement`] carries back.
	/// Creates the account and logs in on the same connection.
	///
	/// Answers with the user agreement the server sends instead of a session:
	/// a new account is unverified, and the code that verifies it can only be
	/// sent on a connection that has already tried to log in. Leaving the
	/// connection open is the point — [`Self::confirm_agreement`] needs it.
	pub async fn register(
		&self,
		endpoint: Endpoint,
		request: LoginRequest,
		email: String,
		password: String,
	) -> Result<Vec<String>, ClientError> {
		self.ask(|reply| Command::Register {
			endpoint,
			request,
			email,
			password,
			reply,
		})
		.await
	}

	/// Sends the emailed agreement code for the account now logging in to `server`.
	pub async fn confirm_agreement(&self, server: String, code: String) -> Result<(), ClientError> {
		self.ask(|reply| Command::ConfirmAgreement {
			server,
			code,
			reply,
		})
		.await
	}

	pub async fn snapshot(&self) -> Result<Snapshot, ClientError> {
		let (tx, rx) = oneshot::channel();
		self.send(Command::Snapshot(tx)).await?;
		rx.await.map_err(|_| ClientError::Stopped)
	}

	/// Joins room `id` on `server`, leaving any room we are in on another.
	/// Resolves when the host accepted us as a spectator, or with its refusal.
	pub async fn join_battle(
		&self,
		server: String,
		id: u32,
		password: Option<String>,
	) -> Result<(), ClientError> {
		self.ask(|reply| Command::JoinBattle {
			server,
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
	pub async fn launch(&self, dirs: DataDirs) -> Result<(), ClientError> {
		self.ask(|reply| Command::Launch { dirs, reply }).await
	}

	pub async fn say(&self, text: String) -> Result<(), ClientError> {
		self.ask(|reply| Command::Say { text, reply }).await
	}

	pub async fn join_channel(
		&self,
		server: String,
		room: String,
		key: Option<String>,
	) -> Result<(), ClientError> {
		self.ask(|reply| Command::JoinChannel {
			server,
			room,
			key,
			reply,
		})
		.await
	}

	pub async fn leave_channel(&self, server: String, room: String) -> Result<(), ClientError> {
		self.ask(|reply| Command::LeaveChannel {
			server,
			room,
			reply,
		})
		.await
	}

	pub async fn say_channel(
		&self,
		server: String,
		room: String,
		text: String,
	) -> Result<(), ClientError> {
		self.ask(|reply| Command::SayChannel {
			server,
			room,
			text,
			reply,
		})
		.await
	}

	pub async fn say_private(
		&self,
		server: String,
		user: String,
		text: String,
	) -> Result<(), ClientError> {
		self.ask(|reply| Command::SayPrivate {
			server,
			user,
			text,
			reply,
		})
		.await
	}

	pub async fn list_channels(&self, server: String) -> Result<(), ClientError> {
		self.ask(|reply| Command::ListChannels { server, reply })
			.await
	}

	/// Asks every server for its friend list again.
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

	/// Stops a paste: what has not left the queue is dropped.
	pub async fn cancel_paste(&self) -> Result<(), ClientError> {
		self.ask(|reply| Command::CancelPaste { reply }).await
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

	/// Moves one of our AIs, or changes its bonus, colour or faction.
	///
	/// The server ignores this for an AI somebody else added, so the caller
	/// checks ownership first or asks the host in chat.
	pub async fn update_bot(
		&self,
		name: String,
		team: u8,
		ally_team: u8,
		handicap: u8,
		colour: u32,
	) -> Result<(), ClientError> {
		self.ask(|reply| Command::UpdateBot {
			name,
			team,
			ally_team,
			handicap,
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

	/// Whether joining a room fetches the game and map it needs by itself.
	pub async fn set_auto_download(&self, on: bool) -> Result<(), ClientError> {
		self.ask(|reply| Command::SetAutoDownload { on, reply })
			.await
	}

	/// How long the window may go untouched before the connection is dropped
	/// and not retried; `None` keeps it however long. Measured from the last
	/// [`Self::activity`], never while a game is running.
	pub async fn set_idle_timeout(&self, timeout: Option<Duration>) -> Result<(), ClientError> {
		self.ask(|reply| Command::SetIdleTimeout { timeout, reply })
			.await
	}

	/// Someone touched the window. Cheap, and safe to send often.
	pub async fn activity(&self) -> Result<(), ClientError> {
		self.send(Command::Activity).await
	}

	/// Marks us away on every server, so nobody waits on someone who has
	/// stepped out.
	pub async fn set_away(&self, away: bool) -> Result<(), ClientError> {
		self.ask(|reply| Command::SetAway { away, reply }).await
	}

	/// Asks a host how long its game has been going. The answer arrives as a
	/// delta, not as a return value: it comes back as a private message.
	pub async fn request_game_status(
		&self,
		server: String,
		founder: String,
	) -> Result<(), ClientError> {
		self.ask(|reply| Command::RequestGameStatus {
			server,
			founder,
			reply,
		})
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

	/// Opens the room with no server behind it, replacing any already open.
	pub async fn open_skirmish(&self, room: skirmish::Room) -> Result<(), ClientError> {
		self.ask(|reply| Command::OpenSkirmish {
			room: Box::new(room),
			reply,
		})
		.await
	}

	pub async fn close_skirmish(&self) -> Result<(), ClientError> {
		self.ask(|reply| Command::CloseSkirmish { reply }).await
	}

	/// One change to that room.
	pub async fn skirmish(&self, act: skirmish::Act) -> Result<(), ClientError> {
		self.ask(|reply| Command::Skirmish {
			act: Box::new(act),
			reply,
		})
		.await
	}

	/// Writes its script and starts the engine on it.
	pub async fn launch_skirmish(&self) -> Result<(), ClientError> {
		self.ask(|reply| Command::LaunchSkirmish { reply }).await
	}

	/// Fetches whatever of that room's content this machine lacks.
	pub async fn skirmish_download(&self) -> Result<(), ClientError> {
		self.ask(|reply| Command::SkirmishDownload { reply }).await
	}

	/// The room as it stands, for saving it as a preset.
	pub async fn skirmish_room(&self) -> Result<Option<skirmish::Room>, ClientError> {
		let (tx, rx) = oneshot::channel();
		self.send(Command::SkirmishRoom(tx)).await?;
		rx.await
			.map(|held| held.map(|room| *room))
			.map_err(|_| ClientError::Stopped)
	}

	/// Puts a preset back into it.
	pub async fn skirmish_preset(
		&self,
		preset: presets::Preset,
		sections: presets::Sections,
	) -> Result<skirmish::preset::Applied, ClientError> {
		self.ask(|reply| Command::SkirmishPreset {
			preset: Box::new(preset),
			sections,
			reply,
		})
		.await
	}

	/// Starts the engine on a replay file.
	pub async fn play_replay(&self, dirs: DataDirs, path: String) -> Result<(), ClientError> {
		self.ask(|reply| Command::PlayReplay { dirs, path, reply })
			.await
	}

	pub async fn friend_action(
		&self,
		server: String,
		action: lobby_core::FriendAction,
		user: String,
	) -> Result<(), ClientError> {
		self.ask(|reply| Command::FriendAction {
			server,
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

	/// Takes a player slot. Refused only outside a room — see
	/// [`lobby_core::SeatError`].
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

	/// Where each server's games come from: its rapid master index, by
	/// server id. A server not named here gets BAR's.
	pub async fn set_rapid_masters(
		&self,
		masters: BTreeMap<String, String>,
	) -> Result<(), ClientError> {
		self.send(Command::SetRapidMasters(masters)).await
	}

	/// Who reads a rapid server that is not BAR's before games are fetched
	/// from it. Until one is set, no such server is fetched from.
	pub async fn set_vet(&self, vet: Vet) -> Result<(), ClientError> {
		self.send(Command::SetVet(vet)).await
	}

	/// Points the content check at a data directory; `None` uses the launcher's.
	pub async fn set_data_dir(&self, data_dir: Option<PathBuf>) -> Result<(), ClientError> {
		self.send(Command::SetDataDir(data_dir)).await
	}

	/// Checks the room's content again after something was installed outside
	/// the runtime's own downloads, and fetches what is still missing.
	pub async fn recheck_content(&self) -> Result<(), ClientError> {
		self.send(Command::RecheckContent).await
	}

	/// Where a borderless config may be kept, or `None` to launch with the
	/// user's settings exactly as they are.
	/// Where to keep the skirmish room between runs.
	pub async fn set_skirmish_path(&self, path: Option<PathBuf>) -> Result<(), ClientError> {
		self.send(Command::SetSkirmishPath(path)).await
	}

	/// The LuaMenu archive to launch games against, when there is one.
	///
	/// What it buys is BAR's in-game "Lobby" button, which raises the overlay
	/// instead of quitting. Cleared when the overlay is off, since raising a
	/// lobby that will not come up is worse than the plain Quit button.
	pub async fn set_menu_archive(
		&self,
		menu: Option<recoil::MenuArchive>,
	) -> Result<(), ClientError> {
		self.send(Command::SetMenuArchive(menu)).await
	}

	pub async fn set_overlay_config_dir(&self, dir: Option<PathBuf>) -> Result<(), ClientError> {
		self.send(Command::SetOverlayConfigDir(dir)).await
	}

	/// Joins an empty public autohost; the first person in it becomes its
	/// boss, which is how a public room of your own is made. Which one is
	/// decided by latency and by which cluster has rooms to spare.
	pub async fn host_public(&self, server: String) -> Result<u32, ClientError> {
		self.ask(|reply| Command::HostPublic { server, reply })
			.await
	}

	/// Asks a cluster manager for a room of our own; the runtime joins it
	/// when it appears. Returns the manager it asked.
	pub async fn request_private_host(&self, server: String) -> Result<String, ClientError> {
		self.ask(|reply| Command::RequestPrivateHost { server, reply })
			.await
	}

	/// Leaves the room, closes the connection and stops the runtime. A running
	/// engine is left alone; it keeps its own link to the host.
	pub async fn shutdown(&self) {
		let _ = self.send(Command::Shutdown).await;
		self.tx.closed().await;
	}
}

/// What an engine was started with: the directory it writes and the copy of
/// the player's files taken just before, to compare against when it exits.
struct EngineRun {
	write: PathBuf,
	snapshot: Option<PathBuf>,
}

/// The room's game as last announced, with the secret the engine presents.
struct Game {
	view: GameRunningView,
	script_password: String,
}

enum Next {
	Command(Command),
	Inbound(String, Inbound),
	Opened(Opened),
	EngineExited(std::io::Result<ExitStatus>),
	Download(DownloadEvent),
	Probe(Probe),
	Reconnect,
	Idle,
	/// A paste's answers have dried up.
	PasteQuiet,
}

/// The most lines taken from one server before the others are looked at.
const DRAIN_MOST: usize = 256;

/// Completes when the soonest reconnection attempt falls due, and never when
/// none is wanted — so the arm simply does not fire while we are connected.
async fn sleep_until_due(wait: Option<Duration>) {
	match wait {
		Some(wait) => tokio::time::sleep(wait).await,
		None => std::future::pending().await,
	}
}

/// Completes when the window has been idle for long enough to let the server
/// go, and never without a connection to let go of or a limit to reach.
async fn sleep_until_idle(policy: &idle::Idle, connected: bool) {
	match policy.until_due(Instant::now()).filter(|_| connected) {
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
	/// Every server there is, or was this run, a session with, by id.
	servers: BTreeMap<String, Server>,
	opened_tx: mpsc::Sender<Opened>,
	opened_rx: mpsc::Receiver<Opened>,
	/// Numbers the connects, so one that comes back can be told from the
	/// one now wanted.
	attempts: u64,
	/// Whose turn it is to be listened to first; see [`recv_any`].
	turn: usize,
	engine: Option<Child>,
	/// What the running engine was started with, to look at when it exits.
	engine_run: Option<EngineRun>,
	engine_status: EngineStatus,
	game: Option<Game>,
	auto_launch: Option<DataDirs>,
	/// Whether to start the engine ourselves when the room's game starts.
	/// Pushed from settings, like the data directory.
	auto_launch_always: bool,
	/// Whether joining a room fetches its missing game and map unasked.
	auto_download: bool,
	/// Whether this machine has everything the room needs. Launching without
	/// it produces an engine that quits with a sync error.
	content_ready: bool,
	/// The join being answered, and the server it went to.
	join_reply: Option<(String, Reply<()>)>,
	/// The server the running game was reported in-game on, to report its end to.
	in_game_on: Option<String>,
	/// When the room was asked for, until its state has all arrived: the
	/// `join:` milestones in the log are measured from here.
	join_asked: Option<Instant>,
	/// Where BAR's content lives; `None` falls back to the launcher's directory.
	data_dir: Option<PathBuf>,
	/// Each server's rapid master index, by server id; see [`Self::rapid_master`].
	rapid_masters: BTreeMap<String, String>,
	vet: Vet,
	/// Where to put a config that gets the game borderless, when the user's
	/// own would not let the overlay cover it. `None` leaves their settings
	/// entirely alone, which is also what happens when they already work.
	overlay_config_dir: Option<PathBuf>,
	menu_archive: Option<recoil::MenuArchive>,
	/// The room's (engine, game, map) the content check last ran against;
	/// scanning the rapid index is too slow to repeat per message.
	checked: Option<(String, String, String)>,
	/// When to let the servers go because nobody has touched the window.
	/// Off until the app pushes a limit; the CLI has no window to watch.
	idle: idle::Idle,
	/// A multi-line paste on its way out, counted down as its writes leave.
	paste: Option<PasteProgress>,
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
	/// What the host machines answered, kept between runs in `state_dir`
	/// when there is one, so most requests probe one address rather than
	/// every cluster.
	cache: latency::Cache,
	/// The way into each server that worked last, kept beside the cache.
	ways: Ways,
	/// Where both are kept.
	state_dir: Option<PathBuf>,
	probe_tx: mpsc::Sender<Probe>,
	probe_rx: mpsc::Receiver<Probe>,
	/// Whether a room request is out measuring; a second one would only
	/// race the first for the same spare.
	probing: bool,
	projector: Projector,
	batcher: Batcher,
	/// The room with no server behind it. Not part of the session: it is still
	/// here after a logout, a dropped connection or a reconnect, which is why
	/// it lives beside `servers` rather than inside one.
	skirmish: Option<skirmish::Room>,
	/// What the skirmish room's (engine, game, map) last checked out as, and
	/// the answer. Scanning the rapid index is far too slow to repeat on every
	/// click, and a room's content only changes when what it asks for does.
	skirmish_checked: Option<((String, String, String), ContentView)>,
	/// Where that room is kept between runs. `None` keeps it nowhere, which is
	/// what the CLI and the tests want.
	skirmish_path: Option<PathBuf>,
}

/// What a pr-downloader child reports back to the runtime.
#[derive(Debug)]
enum DownloadEvent {
	Progress(recoil::Progress),
	/// `failure` is why it did not finish, for a person to read; `None` is
	/// done. Empty for a download that was stopped, which nobody is told.
	Finished {
		what: String,
		failure: Option<String>,
	},
}

/// How many of pr-downloader's last lines are kept for a failure's reason.
const TAIL_LINES: usize = 3;

/// Keeps `line` as one of the last few worth repeating: progress redraws and
/// The lines a batch of effects would send.
fn sends_in(effects: &[Effect]) -> impl Iterator<Item = &Envelope> {
	effects.iter().filter_map(|effect| match effect {
		Effect::Send(envelope) => Some(envelope),
		_ => None,
	})
}

/// A paste being sent, counted two ways: lines leaving the socket, and the
/// host's answers coming back. The second is the one the reader waits on.
#[derive(Debug, Clone)]
struct PasteProgress {
	total: u32,
	sent: u32,
	commands: u32,
	applied: u32,
	skipped: u32,
	/// Bytes across the commands, and across those answered: the bar's unit,
	/// since a tweak blob costs the host far more than a short `!bSet`.
	work: u32,
	done: u32,
	/// The commands still awaiting an answer, by weight, in the order they
	/// went out. SPADS answers in order, so each answer is the front one.
	awaiting: VecDeque<u32>,
	/// The last send or answer; a paste the host has gone quiet on ends
	/// [`PASTE_QUIET`] after it.
	last_activity: Instant,
}

/// How long after the last line left, with no answer from the host, a paste
/// is called done anyway. Chat lines and commands SPADS answers with nothing
/// would otherwise leave the bar hanging.
const PASTE_QUIET: Duration = Duration::from_secs(8);

impl PasteProgress {
	fn status(&self) -> PasteStatus {
		PasteStatus::Running {
			total: self.total,
			sent: self.sent,
			commands: self.commands,
			applied: self.applied,
			skipped: self.skipped,
			work: self.work,
			done: self.done,
		}
	}

	fn done(&self, cancelled: bool) -> PasteStatus {
		PasteStatus::Done {
			total: self.total,
			commands: self.commands,
			applied: self.applied,
			skipped: self.skipped,
			cancelled,
		}
	}

	fn answered(&self) -> bool {
		self.sent >= self.total && self.awaiting.is_empty()
	}

	/// When to give up waiting for answers, if everything has left.
	fn quiet_deadline(&self) -> Option<Instant> {
		(self.sent >= self.total).then(|| self.last_activity + PASTE_QUIET)
	}
}

/// Completes when a paste the host has stopped answering should be called
/// done, and never while there is no paste or lines are still leaving.
async fn sleep_until_paste_quiet(paste: &Option<PasteProgress>) {
	match paste.as_ref().and_then(PasteProgress::quiet_deadline) {
		Some(deadline) => tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await,
		None => std::future::pending().await,
	}
}

/// blank lines say nothing about a failure.
fn remember_tail(tail: &mut Vec<String>, line: &str) {
	let line = line.trim();
	if line.is_empty() || line.starts_with("[Progress]") {
		return;
	}
	if tail.len() == TAIL_LINES {
		tail.remove(0);
	}
	tail.push(line.to_owned());
}

/// Where `server`'s games are looked for: its own rapid master index if it
/// has one, else BAR's — which is also where a room with no server behind it
/// looks. A mod is never looked for on another server's.
fn master_for(masters: &BTreeMap<String, String>, server: Option<&str>) -> String {
	server
		.and_then(|server| masters.get(server))
		.map_or(recoil::RAPID_REPO_MASTER, String::as_str)
		.to_owned()
}

/// One pr-downloader run to its end, reporting progress on the way; the
/// failure is for a person to read.
async fn run_download(
	run: &recoil::Download,
	progress: &mpsc::Sender<DownloadEvent>,
) -> Result<(), String> {
	let mut child = crate::launch::spawn_download(run)?;
	let mut tail = Vec::new();
	if let Some(mut stdout) = child.stdout.take() {
		// Read raw rather than by line: pr-downloader redraws progress with
		// carriage returns, so a line reader would see one enormous line at
		// the end and no progress at all.
		let mut buffer = String::new();
		let mut chunk = [0_u8; 4096];
		while let Ok(read) = stdout.read(&mut chunk).await {
			if read == 0 {
				break;
			}
			buffer.push_str(&String::from_utf8_lossy(&chunk[..read]));
			for line in recoil::split_output(&mut buffer) {
				remember_tail(&mut tail, &line);
				if let Some(step) = recoil::Progress::parse(&line) {
					let _ = progress.send(DownloadEvent::Progress(step)).await;
				}
			}
		}
	}
	if matches!(child.wait().await, Ok(status) if status.success()) {
		return Ok(());
	}
	Err(unpublished(run, &tail).unwrap_or_else(|| failure_reason(&tail)))
}

/// A game run that ended at the search nobody answers: rapid did not know
/// the game, which is an answer about the server and not a broken download.
fn unpublished(run: &recoil::Download, tail: &[String]) -> Option<String> {
	let dead_end = run.search_url == recoil::NO_SEARCH_URL
		&& tail.iter().any(|line| line.contains(recoil::NO_SEARCH_URL));
	dead_end.then(|| {
		let names: Vec<&str> = run.wants.iter().map(|(_, name)| name.as_str()).collect();
		format!(
			"{} is not published by this server's rapid server ({})",
			names.join(", "),
			run.rapid_master
		)
	})
}

/// The failure as a user can report it.
fn failure_reason(tail: &[String]) -> String {
	if tail.is_empty() {
		return "pr-downloader did not finish".into();
	}
	format!("pr-downloader did not finish: {}", tail.join(" | "))
}

impl Runtime {
	fn new(
		rx: mpsc::Receiver<Command>,
		policy: ThrottlePolicy,
		hardware: Hardware,
		connector: Connector,
		latency: Arc<dyn Latency>,
		state_dir: Option<PathBuf>,
	) -> Self {
		// A short queue: progress lines arrive far faster than the front end
		// needs them, and the batcher coalesces what gets through anyway.
		let (download_tx, download_rx) = mpsc::channel(16);
		let (probe_tx, probe_rx) = mpsc::channel(1);
		let (opened_tx, opened_rx) = mpsc::channel(8);
		let cache = state_dir
			.as_deref()
			.map_or_else(latency::Cache::default, |dir| {
				latency::Cache::load(&dir.join(latency::FILE))
			});
		let ways = state_dir
			.as_deref()
			.map_or_else(Ways::default, |dir| Ways::load(&dir.join(ways::FILE)));
		Self {
			rx,
			policy,
			hardware,
			connector,
			ui: None,
			servers: BTreeMap::new(),
			opened_tx,
			opened_rx,
			attempts: 0,
			turn: 0,
			engine: None,
			engine_status: EngineStatus::Idle,
			game: None,
			auto_launch: None,
			auto_launch_always: true,
			auto_download: true,
			content_ready: false,
			join_reply: None,
			in_game_on: None,
			join_asked: None,
			data_dir: None,
			rapid_masters: BTreeMap::new(),
			vet: vet_bars_only(),
			engine_run: None,
			overlay_config_dir: None,
			menu_archive: None,
			checked: None,
			idle: idle::Idle::default(),
			paste: None,
			downloading: None,
			download_stop: None,
			download_stopping: false,
			auto_fetched: None,
			download_tx,
			download_rx,
			latency,
			cache,
			ways,
			state_dir,
			probe_tx,
			probe_rx,
			probing: false,
			skirmish: None,
			skirmish_checked: None,
			skirmish_path: None,
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
		dirs: DataDirs,
		engine_version: &str,
		skirmish: &recoil::script::Skirmish,
	) -> Result<(), ClientError> {
		if self.engine.is_some() {
			return Err(ClientError::Engine("the engine is already running".into()));
		}
		let path = dirs.write.join("modlobby-skirmish.txt");
		std::fs::create_dir_all(&dirs.write)
			.and_then(|()| std::fs::write(&path, skirmish.script()))
			.map_err(|err| ClientError::Engine(format!("writing the start script: {err}")))?;

		let launched = launch::spawn(
			&dirs,
			engine_version,
			path.to_string_lossy().into_owned(),
			self.overlay_config_dir.as_deref(),
			self.menu_archive.clone(),
		)
		.map_err(ClientError::Engine)?;
		self.started(launched, dirs.write);
		Ok(())
	}

	/// Starts the engine on a replay.
	///
	/// The engine reads the demo's own header for the engine version it needs,
	/// so the replay's name is the only hint we have about which one to use;
	/// falling back to any installed engine beats refusing to play it.
	fn play_replay(&mut self, dirs: DataDirs, path: String) -> Result<(), ClientError> {
		if self.engine.is_some() {
			return Err(ClientError::Engine("the engine is already running".into()));
		}
		let version = std::path::Path::new(&path)
			.file_stem()
			.and_then(|stem| stem.to_str())
			.and_then(|stem| stem.rsplit_once('_').map(|(_, engine)| engine.to_owned()))
			.unwrap_or_default();

		let launched = launch::spawn(
			&dirs,
			&version,
			path,
			self.overlay_config_dir.as_deref(),
			self.menu_archive.clone(),
		)
		.map_err(ClientError::Engine)?;
		self.started(launched, dirs.write);
		Ok(())
	}

	/// Bookkeeping for an engine just started, and a word to the user when it
	/// runs on a settings copy of ours rather than their own file.
	fn started(&mut self, launched: launch::Launched, write: PathBuf) {
		let pid = launched.child.id();
		self.engine = Some(launched.child);
		self.engine_run = Some(EngineRun {
			write,
			snapshot: launched.snapshot,
		});
		if let Some(config) = launched.config {
			self.batcher.push(Delta::Notice {
				level: lobby_ui::NoticeLevel::Info,
				text: format!(
					"Your engine settings ask for exclusive fullscreen, which the overlay \
                     cannot cover, so this game runs on a private copy of them: {}",
					config.display()
				),
			});
		}
		self.set_engine(EngineStatus::Running { pid });
	}

	/// Starts pr-downloader on whatever the room needs and this machine lacks.
	///
	/// The child is not awaited here: progress is streamed to the front end as
	/// it arrives, and the content check runs again when it exits, so a room
	/// that was short a map becomes joinable without anyone asking twice.
	async fn start_download(&mut self) -> Result<(), ClientError> {
		let Some(conn) = self.room().and_then(|room| self.link(&room)) else {
			return Err(ClientError::Refused("not in a room".into()));
		};
		let room = conn
			.session
			.state
			.my_battle
			.as_ref()
			.and_then(|my| conn.session.state.battles.get(&my.id))
			.ok_or_else(|| ClientError::Refused("not in a room".into()))?;
		let wanted = (
			room.engine_version.clone(),
			room.game_name.clone(),
			room.map_name.clone(),
		);
		let master = self.rapid_master(self.room().as_deref());
		self.fetch(wanted, master).await
	}

	fn rapid_master(&self, server: Option<&str>) -> String {
		master_for(&self.rapid_masters, server)
	}

	/// The same, for the room with no server behind it.
	async fn start_skirmish_download(&mut self) -> Result<(), ClientError> {
		let room = self
			.skirmish
			.as_ref()
			.ok_or_else(|| ClientError::Refused("there is no skirmish room".into()))?;
		let wanted = (room.engine.clone(), room.game.clone(), room.map.clone());
		let master = self.rapid_master(None);
		self.fetch(wanted, master).await
	}

	/// Fetches whatever of an (engine, game, map) this machine lacks, the
	/// game through `rapid_master`.
	///
	/// One run at a time: pr-downloader rewrites rapid's repo index on every
	/// run, so two at once corrupt each other's view of it.
	async fn fetch(
		&mut self,
		(engine_version, game, map): (String, String, String),
		rapid_master: String,
	) -> Result<(), ClientError> {
		if self.downloading.is_some() {
			return Err(ClientError::Refused("a download is already running".into()));
		}
		let Some(dirs) = self.data_dirs() else {
			return Err(ClientError::Refused("no BAR data directory".into()));
		};
		let library = content::Library::new(dirs.clone());
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
		let runs = crate::launch::plan_download(&dirs, &engine_version, wants, &rapid_master)
			.map_err(ClientError::Refused)?;
		let vet = Arc::clone(&self.vet);

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
			// The runs, one after the other, until one fails.
			let work = async move {
				for run in runs {
					// Somebody else's rapid server is read before anything is
					// fetched through it; BAR's own passes unread.
					if run.has_games() {
						vet(run.rapid_master.clone()).await?;
					}
					run_download(&run, &progress).await?;
				}
				Ok(())
			};
			// The stop side owns a sender the runtime drops; either the work
			// finishing or that drop ends the wait. Letting the work go is
			// what stops the child — pr-downloader leaves a partial file
			// behind, which its own resume handles on the next attempt.
			let failure = tokio::select! {
				outcome = work => outcome.err(),
				_ = stop_rx => Some(String::new()),
			};
			let _ = events.send(DownloadEvent::Finished { what, failure }).await;
		});

		Ok(())
	}

	/// Sends a room request out to measure the machines it could land on.
	/// Only what the cache does not remember is probed, plus one address it
	/// does, so what is remembered is also checked now and then. The answer
	/// comes back through [`Runtime::on_probe`], so the actor keeps reducing
	/// the server's lines meanwhile.
	fn start_probe(&mut self, ips: Vec<Ipv4Addr>, wanted: Wanted) {
		if self.probing {
			let refused = ClientError::Refused("still looking for a room".into());
			match wanted {
				Wanted::Public { reply, .. } => {
					let _ = reply.send(Err(refused));
				}
				Wanted::Private { reply, .. } => {
					let _ = reply.send(Err(refused));
				}
			}
			return;
		}
		let now = latency::unix_now();
		let due = self.cache.due(&ips, now, rand::random::<f64>());
		tracing::info!(
			probing = due.len(),
			remembered = self.cache.remembered(&ips, now),
			"host latency"
		);
		self.probing = true;
		let latency = Arc::clone(&self.latency);
		let events = self.probe_tx.clone();
		tokio::spawn(async move {
			let measured = latency::measure(latency, due, PROBE_TIMEOUT).await;
			let _ = events.send(Probe { measured, wanted }).await;
		});
	}

	/// The latencies still worth trusting.
	fn known_rtts(&self) -> lobby_core::Rtts {
		self.cache.known(latency::unix_now())
	}

	/// Picks the room, or the manager, now that the distances are known.
	/// The list is read again here: it may have moved while we measured.
	async fn on_probe(&mut self, probe: Probe) {
		self.probing = false;
		for (ip, rtt) in &probe.measured {
			tracing::debug!(%ip, ?rtt, "host latency");
		}
		self.cache.record(probe.measured, latency::unix_now());
		if let Some(dir) = self.state_dir.as_deref() {
			self.cache.save(&dir.join(latency::FILE));
		}
		let rtts = self.known_rtts();
		let roll = rand::random::<f64>();
		match probe.wanted {
			Wanted::Public { server, reply } => {
				let Some(conn) = self.link_mut(&server) else {
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
				self.apply_effects(&server, effects).await;
			}
			Wanted::Private { server, reply } => {
				let Some(conn) = self.link_mut(&server) else {
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
						self.apply_effects(&server, effects).await;
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
			DownloadEvent::Finished { what, failure } => {
				self.downloading = None;
				self.download_stop = None;
				let stopped = std::mem::take(&mut self.download_stopping);
				let status = if stopped {
					DownloadStatus::Idle
				} else if let Some(reason) = failure {
					tracing::warn!(%what, %reason, "download failed");
					self.batcher.push(Delta::Notice {
						level: lobby_ui::NoticeLevel::Warning,
						text: format!("downloading {what}: {reason}"),
					});
					DownloadStatus::Failed { what, reason }
				} else {
					DownloadStatus::Done { what }
				};
				self.batcher.push(Delta::Download(status));
				// Whatever arrived changes the answer, so ask the disk again.
				self.checked = None;
				self.refresh_content().await;
			}
		}
	}

	/// Re-checks the room's content when what it asks for changes, and tells
	/// the room whether we are synced. Nothing claims sync without a disk check.
	async fn refresh_content(&mut self) {
		if self.linked().is_empty() {
			self.checked = None;
			return;
		}
		let Some(conn) = self.room().and_then(|room| self.link(&room)) else {
			if self.checked.take().is_some() {
				self.set_synced(false).await;
			}
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
		let Some(dirs) = self.data_dirs() else {
			return;
		};
		let available = content::Library::new(dirs).check(&key.0, &key.1, &key.2);
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
			&& self.auto_download
			&& available.engine
			&& self.downloading.is_none()
			&& self.auto_fetched.as_ref() != Some(&key)
		{
			self.auto_fetched = Some(key);
			// A room that cannot fetch its own content is stuck until the
			// user does something, so they hear why.
			if let Err(error) = self.start_download().await {
				tracing::warn!(%error, "not fetching the room's content");
				self.batcher.push(Delta::Notice {
					level: lobby_ui::NoticeLevel::Warning,
					text: format!("not fetching the room's content: {error}"),
				});
			}
		}
	}

	/// Tells every session whether the content is here. Only one in a room
	/// says so to its server; the others keep it for the room they join next.
	async fn set_synced(&mut self, synced: bool) {
		for server in self.linked() {
			if let Some(conn) = self.link_mut(&server) {
				let effects = conn.session.set_synced(synced);
				self.apply_effects(&server, effects).await;
			}
		}
	}

	/// Where BAR content is: the setting or our own directory to write, every
	/// other install on the machine to read.
	fn data_dirs(&self) -> Option<DataDirs> {
		launch::data_dirs(self.data_dir.clone())
	}

	async fn run(mut self) {
		loop {
			let connected = self.servers.values().any(|server| server.link.is_some());
			let now = Instant::now();
			let retry = self
				.servers
				.values()
				.filter_map(|server| server.reconnect.until_due(now))
				.min();
			let next = tokio::select! {
				command = self.rx.recv() => match command {
					Some(command) => Next::Command(command),
					None => return,
				},
				(server, inbound) = recv_any(&mut self.servers, self.turn) => Next::Inbound(server, inbound),
				Some(opened) = self.opened_rx.recv() => Next::Opened(opened),
				status = wait_engine(&mut self.engine) => Next::EngineExited(status),
				Some(event) = self.download_rx.recv() => Next::Download(event),
				Some(probe) = self.probe_rx.recv() => Next::Probe(probe),
				() = sleep_until_due(retry) => Next::Reconnect,
				() = sleep_until_idle(&self.idle, connected) => Next::Idle,
				() = sleep_until_paste_quiet(&self.paste) => Next::PasteQuiet,
			};
			match next {
				Next::Command(Command::Shutdown) => {
					for server in self.linked() {
						self.disconnect(&server).await;
					}
					self.flush();
					return;
				}
				Next::Command(command) => self.handle_command(command).await,
				Next::Opened(opened) => self.on_opened(opened).await,
				Next::Download(event) => self.on_download(event).await,
				Next::Probe(probe) => self.on_probe(probe).await,
				Next::Reconnect => self.try_reconnect().await,
				Next::Idle => self.on_idle().await,
				Next::PasteQuiet => self.paste_quiet(),
				Next::Inbound(server, inbound) => {
					self.handle_inbound(&server, inbound).await;
					// A run at a time, then the next server gets its turn: a
					// login flood is thousands of lines, and another server's
					// room should not wait behind all of them.
					for _ in 0..DRAIN_MOST {
						let Some(more) = self.try_recv_inbound(&server) else {
							break;
						};
						self.handle_inbound(&server, more).await;
					}
					self.turn = self.turn.wrapping_add(1);
					self.refresh_content().await;
				}
				Next::EngineExited(status) => self.engine_exited(status).await,
			}
			self.flush();
		}
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
			} => {
				// Logging in is the one activity that needs no window.
				self.idle.active(Instant::now());
				self.connect(endpoint, request, Purpose::Login(reply));
			}
			Command::Logout { server } => {
				// Asked for: stop trying to come back.
				let from = match server {
					Some(server) => vec![server],
					None => self.servers.keys().cloned().collect(),
				};
				for server in from {
					self.log_out(&server).await;
				}
			}
			Command::ForgetWay { host } => self.forget_way(&host),
			// Asked for, so it goes out now: the timer's wait is for a server
			// that dropped everyone at once, not for a person watching.
			Command::Reconnect {
				server,
				endpoint,
				reply,
			} => {
				let Some(slot) = self.servers.get_mut(&server) else {
					let _ = reply.send(Err(ClientError::NoCredentials));
					return;
				};
				let Some((kept, request)) = slot.credentials.clone() else {
					let _ = reply.send(Err(ClientError::NoCredentials));
					return;
				};
				let endpoint = endpoint.unwrap_or(kept);
				slot.reconnect.attempted(Instant::now());
				self.announce_retry(&server);
				tracing::info!(server, "reconnecting on request");
				self.connect(endpoint, request, Purpose::Login(reply));
			}
			Command::Snapshot(tx) => {
				let _ = tx.send(self.snapshot());
			}
			Command::JoinBattle {
				server,
				id,
				password,
				reply,
			} => {
				let Some(conn) = self.link_mut(&server) else {
					let _ = reply.send(Err(ClientError::NotConnected));
					return;
				};
				let script_password = format!("{}{}", rand::random::<u16>(), rand::random::<u16>());
				let effects = conn
					.session
					.join_battle(id, password.as_deref(), script_password);
				self.join_reply = Some((server.clone(), reply));
				self.join_asked = Some(Instant::now());
				tracing::info!(server, id, "join: asked");
				self.apply_effects(&server, effects).await;
			}
			Command::LeaveBattle => {
				let Some(server) = self.room() else {
					return;
				};
				let Some(conn) = self.link_mut(&server) else {
					return;
				};
				let effects = conn.session.leave_battle();
				self.project_effects(&server, &effects);
				self.apply_effects(&server, effects).await;
			}
			Command::Launch { dirs, reply } => {
				let result = if self.linked().is_empty() {
					Err(ClientError::NotConnected)
				} else if self.engine.is_some() {
					Err(ClientError::Engine("already running".into()))
				} else if self.game.is_some() {
					self.launch_engine(dirs).await
				} else {
					self.auto_launch = Some(dirs);
					Ok(())
				};
				let _ = reply.send(result);
			}
			Command::Say { text, reply } => {
				// Not `run_session`: a line past the cap is `TooLong`, its own
				// error, rather than a refusal.
				let burst = self.policy.paste.burst;
				let Some(server) = self.room_or_any() else {
					let _ = reply.send(Err(ClientError::NotConnected));
					return;
				};
				let said = match self.link_mut(&server) {
					Some(conn) => conn.session.say_battle(&text, burst),
					None => {
						let _ = reply.send(Err(ClientError::NotConnected));
						return;
					}
				};
				match said {
					Ok(effects) => {
						let _ = reply.send(Ok(()));
						if let Some(Effect::PasteQueued { lines, skipped }) = effects
							.iter()
							.find(|effect| matches!(effect, Effect::PasteQueued { .. }))
						{
							let weights = sends_in(&effects)
								.filter(|envelope| {
									envelope.line.starts_with("SAYBATTLE !")
										|| envelope.line.starts_with("SAYBATTLE $")
								})
								.map(|envelope| envelope.line.len() as u32)
								.collect();
							self.paste_started(*lines, weights, *skipped);
						}
						self.apply_effects(&server, effects).await;
					}
					Err(err) => {
						let _ = reply.send(Err(err.into()));
					}
				}
			}
			Command::JoinChannel {
				server,
				room,
				key,
				reply,
			} => {
				self.run_session(&server, reply, |session| {
					session.join_channel(&room, key.as_deref())
				})
				.await;
			}
			Command::LeaveChannel {
				server,
				room,
				reply,
			} => {
				self.run_session(&server, reply, |session| session.leave_channel(&room))
					.await;
			}
			Command::SayChannel {
				server,
				room,
				text,
				reply,
			} => {
				self.run_session(&server, reply, |session| session.say_channel(&room, &text))
					.await;
			}
			Command::SayPrivate {
				server,
				user,
				text,
				reply,
			} => {
				self.run_session(&server, reply, |session| session.say_private(&user, &text))
					.await;
			}
			Command::ListChannels { server, reply } => {
				self.run_session(&server, reply, |session| {
					Ok::<_, std::convert::Infallible>(session.list_channels())
				})
				.await;
			}
			Command::OpenSkirmish { room, reply } => {
				self.open_skirmish(*room);
				let _ = reply.send(Ok(()));
			}
			Command::CloseSkirmish { reply } => {
				self.close_skirmish();
				let _ = reply.send(Ok(()));
			}
			Command::Skirmish { act, reply } => {
				let result = self.skirmish_act(*act).await;
				let _ = reply.send(result);
			}
			Command::LaunchSkirmish { reply } => {
				let result = self.launch_skirmish();
				let _ = reply.send(result);
			}
			Command::SkirmishDownload { reply } => {
				let result = self.start_skirmish_download().await;
				let _ = reply.send(result);
			}
			Command::SkirmishRoom(reply) => {
				let _ = reply.send(self.skirmish.clone().map(Box::new));
			}
			Command::SkirmishPreset {
				preset,
				sections,
				reply,
			} => {
				let result = self.skirmish_preset(&preset, sections);
				let _ = reply.send(result);
			}
			Command::PlayReplay { dirs, path, reply } => {
				let result = self.play_replay(dirs, path);
				let _ = reply.send(result);
			}
			Command::DownloadMissing { reply } => {
				let result = self.start_download().await;
				let _ = reply.send(result);
			}
			Command::CancelPaste { reply } => {
				let answer = self.cancel_paste().await;
				let _ = reply.send(answer);
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
			Command::RequestGameStatus {
				server,
				founder,
				reply,
			} => {
				self.run_session(&server, reply, |session| {
					session.request_game_status(&founder)
				})
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
			Command::SetAutoDownload { on, reply } => {
				let turned_on = on && !self.auto_download;
				self.auto_download = on;
				let _ = reply.send(Ok(()));
				// Switched on with a room already joined: fetch now, not on
				// the next room.
				if turned_on {
					self.checked = None;
					self.refresh_content().await;
				}
			}
			Command::SetIdleTimeout { timeout, reply } => {
				self.idle.set_timeout(timeout);
				let _ = reply.send(Ok(()));
			}
			Command::Activity => self.idle.active(Instant::now()),
			Command::SetAway { away, reply } => {
				self.run_everywhere(reply, |session| session.set_away(away))
					.await;
			}
			Command::Ring { user, reply } => {
				self.run_room(reply, |session| {
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
				self.run_room(reply, |session| {
					Ok::<_, std::convert::Infallible>(
						session.add_bot(&name, &ai, team, ally_team, colour),
					)
				})
				.await;
			}
			Command::UpdateBot {
				name,
				team,
				ally_team,
				handicap,
				colour,
				reply,
			} => {
				self.run_room(reply, |session| {
					Ok::<_, std::convert::Infallible>(
						session.update_bot(&name, team, ally_team, handicap, colour),
					)
				})
				.await;
			}
			Command::RemoveBot { name, reply } => {
				self.run_room(reply, |session| {
					Ok::<_, std::convert::Infallible>(session.remove_bot(&name))
				})
				.await;
			}
			Command::RefreshFriends { reply } => {
				self.run_everywhere(reply, Session::refresh_friends).await;
			}
			Command::FriendAction {
				server,
				action,
				user,
				reply,
			} => {
				self.run_session(&server, reply, |session| {
					Ok::<_, std::convert::Infallible>(session.friend_action(action, &user))
				})
				.await;
			}
			Command::SetReady { ready, reply } => {
				self.run_room(reply, |session| session.set_ready(ready))
					.await;
			}
			Command::SetSide { side, reply } => {
				self.run_room(reply, |session| session.set_side(side)).await;
			}
			Command::TakeSeat {
				team,
				ally_team,
				reply,
			} => {
				self.run_room(reply, |session| session.take_seat(team, ally_team))
					.await;
			}
			Command::SetOverlayConfigDir(dir) => self.overlay_config_dir = dir,
			Command::SetMenuArchive(menu) => self.menu_archive = menu,
			Command::SetSkirmishPath(path) => self.skirmish_path = path,
			Command::SetRapidMasters(masters) => self.rapid_masters = masters,
			Command::SetVet(vet) => self.vet = vet,
			Command::SetDataDir(data_dir) => {
				// Told on every save of the settings, of which most change
				// something else: the scan below is too slow to repeat for
				// a battle-list filter being clicked.
				if data_dir == self.data_dir {
					return;
				}
				self.data_dir = data_dir;
				// Re-check against the new directory.
				self.checked = None;
				self.refresh_content().await;
			}
			Command::RecheckContent => {
				self.checked = None;
				self.refresh_content().await;
				// The room with no server behind it keeps its own answer,
				// keyed on the same three names and cached for the same
				// reason — and `refresh_content` above returns at once when
				// there is no connection, so without this an engine put in the
				// folder by hand is found only by restarting the app.
				self.skirmish_checked = None;
				self.push_skirmish();
			}
			Command::ReleaseSeat => {
				let Some(server) = self.room() else {
					return;
				};
				let Some(conn) = self.link_mut(&server) else {
					return;
				};
				let effects = conn.session.release_seat();
				self.apply_effects(&server, effects).await;
			}
			Command::HostPublic { server, reply } => {
				let Some(conn) = self.link(&server) else {
					let _ = reply.send(Err(ClientError::NotConnected));
					return;
				};
				let ips = conn.session.spare_machines();
				self.start_probe(ips, Wanted::Public { server, reply });
			}
			Command::RequestPrivateHost { server, reply } => {
				let Some(conn) = self.link(&server) else {
					let _ = reply.send(Err(ClientError::NotConnected));
					return;
				};
				let ips = conn.session.spare_machines();
				self.start_probe(ips, Wanted::Private { server, reply });
			}
			Command::Register {
				endpoint,
				request,
				email,
				password,
				reply,
			} => self.connect(
				endpoint,
				request,
				Purpose::Register {
					email,
					password,
					reply,
				},
			),
			Command::ConfirmAgreement {
				server,
				code,
				reply,
			} => {
				// Not `run_session`: that answers as soon as the line is
				// queued, which for a code would report a wrong one as a
				// success. The server's reply is the answer — `ACCEPTED` when
				// the code was right (teiserver runs the whole login on it),
				// `DENIED Incorrect code` when it was not — so this waits
				// where a login waits.
				let Some(slot) = self.servers.get_mut(&server) else {
					let _ = reply.send(Err(ClientError::NotConnected));
					return;
				};
				let Some(conn) = slot.link.as_mut() else {
					let _ = reply.send(Err(ClientError::NotConnected));
					return;
				};
				let effects = conn.session.confirm_agreement(&code);
				slot.login_reply = Some(reply);
				self.apply_effects(&server, effects).await;
			}
			Command::Shutdown => unreachable!("handled by the run loop"),
		}
	}

	fn forget_way(&mut self, host: &str) {
		self.ways.forget(host);
		self.ways_changed();
	}

	fn ways_changed(&mut self) {
		if let Some(dir) = self.state_dir.as_deref() {
			self.ways.save(&dir.join(ways::FILE));
		}
		self.batcher.push(Delta::Ways(self.ways_view()));
	}

	/// Each remembered way as it reads: "STLS on 8200, 46 ms".
	fn ways_view(&self) -> BTreeMap<String, String> {
		self.ways
			.iter()
			.map(|(host, way)| (host.clone(), way.to_string()))
			.collect()
	}

	async fn handle_inbound(&mut self, server: &str, inbound: Inbound) {
		match inbound {
			Inbound::Message(event) => {
				let Some(conn) = self
					.servers
					.get_mut(server)
					.and_then(|slot| slot.link.as_mut())
				else {
					return;
				};
				let effects = conn.session.handle(event.clone());
				let deltas = self
					.projector
					.project(&event, &effects, &conn.session.state);
				for delta in deltas {
					self.batcher.push_for(server, delta);
				}
				if matches!(event, spring_protocol::ServerEvent::RequestBattleStatus) {
					self.note_room_state_complete();
				}
				self.apply_effects(server, effects).await;
			}
			Inbound::Policy(event) => match event {
				PolicyEvent::Delayed {
					area,
					pending,
					wait,
				} => tracing::debug!(server, ?area, pending, ?wait, "throttled"),
				PolicyEvent::Sent { area, lines, .. } => self.paste_sent(area, lines),
				other => tracing::info!(server, ?other, "policy"),
			},
			Inbound::Closed { reason } => self.connection_lost(server, reason),
		}
	}

	/// Runs one session call and applies whatever it produced. The session's
	/// own error type only ever needs to reach the caller as text.
	async fn run_session<E: std::fmt::Display>(
		&mut self,
		server: &str,
		reply: Reply<()>,
		call: impl FnOnce(&mut lobby_core::Session) -> Result<Vec<Effect>, E>,
	) {
		let Some(conn) = self.link_mut(server) else {
			let _ = reply.send(Err(ClientError::NotConnected));
			return;
		};
		match call(&mut conn.session) {
			Ok(effects) => {
				let _ = reply.send(Ok(()));
				self.apply_effects(server, effects).await;
			}
			Err(err) => {
				let _ = reply.send(Err(ClientError::Refused(err.to_string())));
			}
		}
	}

	/// A room's command, to the room's server — or, with no room, to one whose
	/// session will say there is none.
	async fn run_room<E: std::fmt::Display>(
		&mut self,
		reply: Reply<()>,
		call: impl FnOnce(&mut lobby_core::Session) -> Result<Vec<Effect>, E>,
	) {
		match self.room_or_any() {
			Some(server) => self.run_session(&server, reply, call).await,
			None => {
				let _ = reply.send(Err(ClientError::NotConnected));
			}
		}
	}

	/// One call on every server there is a link to — away, the friend list —
	/// answered once: done if any server took it.
	async fn run_everywhere(
		&mut self,
		reply: Reply<()>,
		call: impl Fn(&mut lobby_core::Session) -> Vec<Effect>,
	) {
		let linked = self.linked();
		let _ = reply.send(if linked.is_empty() {
			Err(ClientError::NotConnected)
		} else {
			Ok(())
		});
		for server in linked {
			if let Some(conn) = self.link_mut(&server) {
				let effects = call(&mut conn.session);
				self.apply_effects(&server, effects).await;
			}
		}
	}

	/// Applies what `server`'s session produced.
	async fn apply_effects(&mut self, server: &str, effects: Vec<Effect>) {
		// A queue, not a loop over the argument: joining the private room we
		// asked for produces effects of its own.
		let mut queue: std::collections::VecDeque<Effect> = effects.into();
		while let Some(effect) = queue.pop_front() {
			match effect {
				// The room a cluster manager made for us; joining it is the
				// whole point of having asked.
				Effect::PrivateHostReady { id, password } => {
					let Some(conn) = self.link_mut(server) else {
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
					self.batcher.push_for(
						server,
						Delta::Notice {
							level: lobby_ui::NoticeLevel::Info,
							text: format!("{manager} is starting a room; password {password}"),
						},
					);
				}
				Effect::Hosting { founder, alone } => {
					let rtt = self
						.link(server)
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
					self.batcher.push_for(server, Delta::Notice { level, text });
				}
				Effect::Send(envelope) => {
					if let Err(err) = self.send_line(server, envelope).await {
						self.connection_lost(server, err.to_string());
						return;
					}
				}
				Effect::Ready => {
					if let Some(slot) = self.servers.get_mut(server) {
						slot.reconnect.stop();
					}
					self.reply_login(server, Ok(()));
					// The snapshot carries `retry_in: None` for the corner.
					self.send_session(server);
					// The server volunteers nothing about friendships, so the
					// list is asked for once the login flood has settled.
					// Without this a filter that depends on it would quietly
					// match nobody.
					if let Some(conn) = self.link_mut(server) {
						let effects = conn.session.refresh_friends();
						queue.extend(effects);
					}
				}
				Effect::LoginDenied { reason } => self.refuse(server, reason).await,
				Effect::AgreementRequired { text } => {
					// Not a refusal, and never a reason to hang up: this is
					// the one connection the emailed code can be sent on.
					let slot = self.servers.entry(server.to_owned()).or_default();
					if let Some(reply) = slot.register_reply.take() {
						// Registering, so the form is already asking for the
						// code and a notice would only repeat the screen. The
						// agreement itself goes back as the answer, because
						// entering the code is what accepts it.
						let _ = reply.send(Ok(text));
					} else {
						// An older account that never confirmed. Nothing is
						// waiting on an agreement, so this has to be said.
						let login_reply = slot.login_reply.take();
						self.batcher.push_for(
                            server,
                            Delta::Notice {
                                level: lobby_ui::NoticeLevel::Warning,
                                text: "this account must confirm the emailed code before it can log in"
                                    .into(),
                            },
                        );
						if let Some(reply) = login_reply {
							let _ = reply.send(Err(ClientError::Refused(
								"confirm the code emailed to you".into(),
							)));
						}
					}
				}
				Effect::Registered => {
					// The account exists but is unverified. Logging in on this
					// same connection is what puts the server in the state
					// where `CONFIRMAGREEMENT` means anything, and the
					// agreement it answers with is what the caller is waiting
					// for. Hanging up here — as this once did — threw away the
					// connection the code had to be sent on.
					if let Some(conn) = self.link_mut(server) {
						let effects = conn.session.begin_login();
						queue.extend(effects);
					}
				}
				Effect::RegistrationDenied { reason } => {
					let slot = self.servers.entry(server.to_owned()).or_default();
					if let Some(reply) = slot.register_reply.take() {
						let _ = reply.send(Err(ClientError::Refused(reason)));
					}
					// No account was made, so there is nothing to come back to.
					slot.credentials = None;
					self.disconnect(server).await;
				}
				Effect::Redirect { host, port } => {
					self.refuse(
						server,
						format!(
							"server redirects to {host}:{}",
							port.map_or("?".into(), |p| p.to_string())
						),
					)
					.await
				}
				Effect::Disconnected { reason, flood } => {
					if flood && let Some(conn) = self.link(server) {
						let wait = Duration::from_secs_f64(self.policy.login.after_flood_secs);
						let _ = conn
							.transport
							.trip(Area::Login, Instant::now() + wait)
							.await;
					}
					self.refuse(server, format!("disconnected: {reason}")).await;
				}
				Effect::Joined { .. } => {
					self.leave_rooms_except(server).await;
					self.reply_join(server, Ok(()));
				}
				Effect::JoinFailed { reason } => {
					self.reply_join(server, Err(ClientError::Refused(reason)))
				}
				Effect::LeftBattle { .. } => self.game = None,
				Effect::GameStopped => {
					if self.game.take().is_some() {
						self.batcher.push_for(
							server,
							Delta::Alert {
								kind: lobby_ui::AlertKind::GameEnded,
								text: "your room's game has finished".into(),
							},
						);
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
						self.batcher.push_for(
							server,
							Delta::Alert {
								kind: lobby_ui::AlertKind::GameStarting,
								text: "your room's game has started".into(),
							},
						);
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
					// Nor where the engine may not join a hosted game at all:
					// the launch would be refused, and refusing it once per
					// game start is a notice nobody asked for.
					let wanted = self.auto_launch.take().or_else(|| {
						(just_started
							&& self.auto_launch_always
							&& self.content_ready && self.engine.is_none()
							&& recoil::may_join_hosted_games())
						.then(|| self.data_dirs())
						.flatten()
					});
					if let Some(dirs) = wanted
						&& let Err(err) = self.launch_engine(dirs).await
					{
						self.batcher.push(Delta::Notice {
							level: lobby_ui::NoticeLevel::Error,
							text: err.to_string(),
						});
					}
				}
				Effect::BattleChat {
					ref from,
					ref text,
					announcement: true,
				} if self.paste.is_some() => self.paste_answered(from, text),
				// Projected into deltas; the runtime itself has nothing to do.
				// Everything the projector turns into a delta on its own.
				Effect::LoggedIn { .. }
				| Effect::GameInProgress { .. }
				| Effect::Notice(_)
				| Effect::PasteQueued { .. }
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
				| Effect::Motd { .. }
				| Effect::Rung { .. }
				| Effect::ModOptionsChanged { .. }
				| Effect::VoteChanged => {}
			}
		}
	}

	fn project_effects(&mut self, server: &str, effects: &[Effect]) {
		let Some(conn) = self.servers.get(server).and_then(|slot| slot.link.as_ref()) else {
			return;
		};
		let mut deltas = Vec::new();
		self.projector
			.project_effects(effects, &conn.session.state, &mut deltas);
		for delta in deltas {
			self.batcher.push_for(server, delta);
		}
	}

	async fn send_line(&mut self, server: &str, envelope: Envelope) -> Result<(), ClientError> {
		let Some(conn) = self.link(server) else {
			return Err(ClientError::NotConnected);
		};
		conn.transport.send(envelope).await?;
		Ok(())
	}

	/// The window has gone untouched for the limit. A running game is not
	/// idleness, whatever the window says — its player is looking at the
	/// game — so that only pushes the limit out by another period.
	async fn on_idle(&mut self) {
		let now = Instant::now();
		if self.game.is_some() || self.engine.is_some() {
			self.idle.active(now);
			return;
		}
		self.idle_disconnect().await;
	}

	async fn launch_engine(&mut self, dirs: DataDirs) -> Result<(), ClientError> {
		let Some(server) = self.room() else {
			return Err(ClientError::NotConnected);
		};
		let (engine_version, url, in_game) = {
			let Some(conn) = self
				.servers
				.get_mut(&server)
				.and_then(|slot| slot.link.as_mut())
			else {
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
			let available = content::Library::new(dirs.clone()).check(
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
				self.send_line(&server, envelope).await?;
			}
		}
		self.in_game_on = Some(server);
		let launched = launch::spawn(
			&dirs,
			&engine_version,
			url,
			self.overlay_config_dir.as_deref(),
			self.menu_archive.clone(),
		)
		.map_err(ClientError::Engine)?;
		self.started(launched, dirs.write);
		Ok(())
	}

	async fn engine_exited(&mut self, status: std::io::Result<ExitStatus>) {
		self.engine = None;
		let code = status.ok().and_then(|s| s.code());
		tracing::info!(?code, "engine exited");
		self.set_engine(EngineStatus::Exited { code });
		self.check_player_files();
		if let Some(server) = self.in_game_on.take()
			&& let Some(conn) = self.link_mut(&server)
		{
			let effects = conn.session.set_in_game(false);
			self.apply_effects(&server, effects).await;
		}
	}

	/// The engine rewrites `springsettings.cfg` as it exits and has emptied it
	/// before. A file that lost most of its keys is said so, with where the
	/// copy from before the game is; putting it back is the user's call.
	fn check_player_files(&mut self) {
		let Some(run) = self.engine_run.take() else {
			return;
		};
		let Some(snapshot) = run.snapshot else {
			return;
		};
		let Some(lost) = player_files::collapsed(&run.write, &snapshot) else {
			return;
		};
		tracing::warn!(
			before = lost.before,
			after = lost.after,
			"engine settings collapsed"
		);
		self.batcher.push(Delta::Notice {
			level: lobby_ui::NoticeLevel::Warning,
			text: format!(
				"The game left {} with {} settings where it had {}. The copy taken before \
                 the game is under {}; Settings → Paths can put it back.",
				player_files::SETTINGS,
				lost.after,
				lost.before,
				snapshot.display()
			),
		});
	}

	fn set_engine(&mut self, status: EngineStatus) {
		self.engine_status = status;
		self.batcher.push(Delta::Engine(status));
	}

	/// The room's deltas go out before the answer: the front end walks into
	/// the room on the answer, and a room view without `myBattle` walks
	/// straight back out.
	fn reply_join(&mut self, server: &str, result: Result<(), ClientError>) {
		// A join on another server is not this one's to answer.
		if self.join_reply.as_ref().is_some_and(|(on, _)| on != server) {
			return;
		}
		if let Some(asked) = self.join_asked {
			tracing::info!(
				ms = asked.elapsed().as_millis() as u64,
				ok = result.is_ok(),
				"join: answered"
			);
		}
		if result.is_err() {
			self.join_asked = None;
		}
		if let Some((_, reply)) = self.join_reply.take() {
			self.flush();
			let _ = reply.send(result);
		}
	}

	/// `REQUESTBATTLESTATUS` closes the server's room-state burst.
	fn note_room_state_complete(&mut self) {
		if let Some(asked) = self.join_asked.take() {
			tracing::info!(
				ms = asked.elapsed().as_millis() as u64,
				"join: room state complete"
			);
		}
	}

	/// Opens the room with no server behind it.
	///
	/// Keeps the one already open rather than replacing it: "open the room" is
	/// a request for there to be one, and arriving at the page twice should
	/// not throw away what was set up the first time.
	fn open_skirmish(&mut self, room: skirmish::Room) {
		if self.skirmish.is_some() {
			return;
		}
		self.skirmish = Some(room);
		self.skirmish_checked = None;
		self.push_skirmish();
	}

	fn close_skirmish(&mut self) {
		self.skirmish = None;
		self.skirmish_checked = None;
		self.batcher.push(Delta::Skirmish(None));
		self.remember_skirmish();
	}

	/// Does one thing to the skirmish room and tells the front end what the
	/// room is now.
	///
	/// The room says what happened; this decides who needs to hear it. A
	/// change is worth both a line in the log and a new view; an answer is
	/// worth only the line; a click that changed nothing is worth neither.
	async fn skirmish_act(&mut self, act: skirmish::Act) -> Result<(), ClientError> {
		let Some(room) = self.skirmish.as_mut() else {
			return Err(ClientError::Engine("there is no skirmish room".into()));
		};
		let outcome = room.act(act);
		match outcome {
			skirmish::Outcome::Did(said) => {
				self.say_in_skirmish(&said);
				self.push_skirmish();
			}
			skirmish::Outcome::Said(said) => self.say_in_skirmish(&said),
			skirmish::Outcome::Nothing => {}
			skirmish::Outcome::Launch => return self.launch_skirmish(),
		}
		Ok(())
	}

	/// A line in the skirmish room's own log, which is where its console
	/// answers and where every change to it is recorded.
	fn say_in_skirmish(&mut self, text: &str) {
		let line = self.projector.said(SKIRMISH_ROOM, "setup", text);
		self.batcher.push(Delta::Chat(line));
	}

	/// Puts a preset back into the skirmish room and says what it did.
	fn skirmish_preset(
		&mut self,
		preset: &presets::Preset,
		sections: presets::Sections,
	) -> Result<skirmish::preset::Applied, ClientError> {
		let Some(room) = self.skirmish.as_mut() else {
			return Err(ClientError::Engine("there is no skirmish room".into()));
		};
		let done = skirmish::preset::apply(room, preset, sections);
		self.say_in_skirmish(&format!(
			"loaded {}: {} changed, {} already set, {} AIs",
			preset.name, done.changed, done.already_set, done.bots
		));
		self.push_skirmish();
		Ok(done)
	}

	/// Writes the skirmish room's script and starts the engine on it.
	fn launch_skirmish(&mut self) -> Result<(), ClientError> {
		let Some(room) = self.skirmish.as_ref() else {
			return Err(ClientError::Engine("there is no skirmish room".into()));
		};
		let Some(dirs) = self.data_dirs() else {
			return Err(ClientError::Engine("no data directory".into()));
		};
		// The same refusal the multiplayer path gives, for the same reason: an
		// engine that starts without the content quits with a sync error, and
		// a message naming what is missing is worth more than that.
		let available =
			content::Library::new(dirs.clone()).check(&room.engine, &room.game, &room.map);
		if !available.complete() {
			return Err(ClientError::Engine(format!(
				"this game needs content you do not have: {}",
				available.missing().join(", ")
			)));
		}
		let engine = room.engine.clone();
		let script = room.to_script();
		self.start_skirmish(dirs, &engine, &script)
	}

	/// Writes it down, so closing the lobby does not throw the setup away.
	fn remember_skirmish(&self) {
		let Some(path) = self.skirmish_path.as_ref() else {
			return;
		};
		let Some(room) = self.skirmish.as_ref() else {
			let _ = std::fs::remove_file(path);
			return;
		};
		let Ok(text) = serde_json::to_string(room) else {
			return;
		};
		if let Some(dir) = path.parent() {
			let _ = std::fs::create_dir_all(dir);
		}
		// Not worth refusing a change over: the room is right in memory either
		// way, and the next change tries again.
		if let Err(err) = std::fs::write(path, text) {
			tracing::warn!(%err, "the skirmish room was not kept");
		}
	}

	/// The room as the front end sees it, with what this machine has of it.
	fn push_skirmish(&mut self) {
		let content = self.skirmish_content();
		let Some(room) = self.skirmish.as_ref() else {
			return;
		};
		let view = room.view(content);
		self.batcher.push(Delta::Skirmish(Some(Box::new(view))));
		self.remember_skirmish();
	}

	/// Whether this machine has the skirmish room's engine, game and map.
	///
	/// Re-read only when one of the three changes: the answer comes from
	/// scanning the rapid index, which is far too slow to repeat per click.
	fn skirmish_content(&mut self) -> ContentView {
		let absent = ContentView {
			engine: false,
			game: false,
			map: false,
		};
		let Some(room) = self.skirmish.as_ref() else {
			return absent;
		};
		let key = (room.engine.clone(), room.game.clone(), room.map.clone());
		if let Some((checked, content)) = &self.skirmish_checked
			&& *checked == key
		{
			return *content;
		}
		let Some(dirs) = self.data_dirs() else {
			return absent;
		};
		let available = content::Library::new(dirs).check(&key.0, &key.1, &key.2);
		let content = ContentView {
			engine: available.engine,
			game: available.game,
			map: available.map,
		};
		self.skirmish_checked = Some((key, content));
		content
	}

	fn snapshot(&self) -> Snapshot {
		let servers = self
			.servers
			.keys()
			.filter_map(|id| self.session_snapshot(id))
			.collect();
		let mut snapshot = Snapshot {
			servers,
			engine: self.engine_status,
			..Snapshot::default()
		};
		snapshot.paste = self
			.paste
			.as_ref()
			.map_or(PasteStatus::Idle, PasteProgress::status);
		snapshot.ways = self.ways_view();
		// Joined on here rather than built into either constructor, because a
		// skirmish belongs to the machine and a snapshot describes a session.
		snapshot.skirmish = self.skirmish.as_ref().map(|room| {
			let content = self.skirmish_checked.as_ref().map_or(
				ContentView {
					engine: false,
					game: false,
					map: false,
				},
				|(_, content)| *content,
			);
			Box::new(room.view(content))
		});
		snapshot
	}

	/// A paste has been handed to the scheduler; `weights` are the bytes of
	/// its commands, in order. With nothing left to send it is done at once,
	/// which is still worth showing: the reader pasted something and should
	/// learn the room already had all of it.
	fn paste_started(&mut self, lines: usize, weights: Vec<u32>, skipped: usize) {
		let progress = PasteProgress {
			total: lines as u32,
			sent: 0,
			commands: weights.len() as u32,
			applied: 0,
			skipped: skipped as u32,
			work: weights.iter().sum(),
			done: 0,
			awaiting: weights.into(),
			last_activity: Instant::now(),
		};
		if lines == 0 {
			self.paste = None;
			self.batcher.push(Delta::Paste(progress.done(false)));
			return;
		}
		self.batcher.push(Delta::Paste(progress.status()));
		self.paste = Some(progress);
	}

	/// A battle-room write left. Any such write counts, so a `!vote` typed
	/// mid-paste nudges the count a line early; it is clamped.
	fn paste_sent(&mut self, area: Area, lines: usize) {
		if !matches!(
			area,
			Area::BattleChat | Area::BattleCommand | Area::BattlePaste
		) {
			return;
		}
		let Some(progress) = self.paste.as_mut() else {
			return;
		};
		progress.sent = (progress.sent + lines as u32).min(progress.total);
		progress.last_activity = Instant::now();
		self.paste_report();
	}

	/// The host said something about a command: the oldest one still waiting
	/// is answered.
	fn paste_answered(&mut self, from: &str, text: &str) {
		if self.paste.is_none() || !self.is_founder(from) {
			return;
		}
		let me = self
			.room()
			.and_then(|room| self.link(&room))
			.and_then(|conn| conn.session.state.me.clone())
			.unwrap_or_default();
		if !lobby_core::spads::answers_command(text, &me) {
			return;
		}
		let Some(progress) = self.paste.as_mut() else {
			return;
		};
		if let Some(weight) = progress.awaiting.pop_front() {
			progress.done += weight;
			progress.applied += 1;
		}
		progress.last_activity = Instant::now();
		self.paste_report();
	}

	/// Everything left and the host has said nothing for a while: done, with
	/// whatever count it reached.
	fn paste_quiet(&mut self) {
		if let Some(progress) = self.paste.take() {
			self.batcher.push(Delta::Paste(progress.done(false)));
		}
	}

	/// Drops what the scheduler still holds of the paste. Lines already on
	/// the wire are the host's now; their answers just stop being counted.
	async fn cancel_paste(&mut self) -> Result<(), ClientError> {
		let Some(progress) = self.paste.take() else {
			return Err(ClientError::Refused("nothing is being pasted".into()));
		};
		if let Some(conn) = self.room().and_then(|room| self.link(&room)) {
			for area in [Area::BattlePaste, Area::BattleCommand, Area::BattleChat] {
				conn.transport.cancel(area).await?;
			}
		}
		self.batcher.push(Delta::Paste(progress.done(true)));
		Ok(())
	}

	fn paste_report(&mut self) {
		let finished = self.paste.as_ref().is_some_and(PasteProgress::answered);
		let status = if finished {
			self.paste.take().map(|progress| progress.done(false))
		} else {
			self.paste.as_ref().map(PasteProgress::status)
		};
		if let Some(status) = status {
			self.batcher.push(Delta::Paste(status));
		}
	}

	/// Whether `name` hosts the room we are in.
	fn is_founder(&self, name: &str) -> bool {
		self.room()
			.and_then(|room| self.link(&room))
			.is_some_and(|conn| {
				let state = &conn.session.state;
				state
					.my_battle
					.as_ref()
					.and_then(|my| state.battles.get(&my.id))
					.is_some_and(|battle| battle.founder == name)
			})
	}

	/// A snapshot supersedes whatever deltas were waiting.
	/// Sends the whole state, dropping the pending deltas it supersedes.
	///
	/// Chat is the exception: a snapshot says nothing about what anyone said,
	/// so discarding batched chat would silently swallow whatever arrived in
	/// the moment before it — which is exactly when the message of the day and
	/// the first channel traffic land.
	fn send_snapshot(&mut self) {
		let kept = self.pending_beside_snapshot(None);
		let snapshot = self.snapshot();
		self.send_ui(UiMessage::Snapshot(Box::new(snapshot)));
		for message in kept {
			self.send_ui(message);
		}
	}

	/// `server`'s session over again, and nobody else's: what a login ends in.
	fn send_session(&mut self, server: &str) {
		let Some(session) = self.session_snapshot(server) else {
			return;
		};
		let kept = self.pending_beside_snapshot(Some(server));
		self.send_ui(UiMessage::Session(Box::new(session)));
		for message in kept {
			self.send_ui(message);
		}
	}

	/// What is pending, less what a snapshot is about to say anyway: `of`'s
	/// changes — or with `None` everybody's, the machine's included — except
	/// chat lines, the one thing a snapshot does not carry.
	fn pending_beside_snapshot(&mut self, of: Option<&str>) -> Vec<UiMessage> {
		self.batcher
			.take()
			.into_iter()
			.filter_map(|message| {
				let UiMessage::Deltas { server, deltas } = message else {
					return None;
				};
				if of.is_some() && of != server.as_deref() {
					return Some(UiMessage::Deltas { server, deltas });
				}
				let chat: Vec<Delta> = deltas
					.into_iter()
					.filter(|delta| matches!(delta, Delta::Chat(_)))
					.collect();
				(!chat.is_empty()).then_some(UiMessage::Deltas {
					server,
					deltas: chat,
				})
			})
			.collect()
	}

	fn flush(&mut self) {
		for message in self.batcher.take() {
			self.send_ui(message);
		}
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
	use spring_protocol::Security;

	#[test]
	fn a_failure_repeats_the_last_lines_pr_downloader_printed() {
		let mut tail = Vec::new();
		for line in [
			"[Progress] 10% [##   ] 1/10MB",
			"",
			"one",
			"two",
			"three",
			"[Error] no such map",
		] {
			remember_tail(&mut tail, line);
		}
		assert_eq!(tail, ["two", "three", "[Error] no such map"]);
		assert_eq!(
			failure_reason(&tail),
			"pr-downloader did not finish: two | three | [Error] no such map"
		);
	}

	#[test]
	fn a_servers_games_come_from_its_own_rapid_and_everyone_elses_from_bars() {
		let masters = BTreeMap::from([(
			"mods.example".to_owned(),
			"https://mods.example/repos.gz".to_owned(),
		)]);
		assert_eq!(
			master_for(&masters, Some("mods.example")),
			"https://mods.example/repos.gz"
		);
		assert_eq!(
			master_for(&masters, Some("server4.beyondallreason.info")),
			recoil::RAPID_REPO_MASTER
		);
		assert_eq!(master_for(&masters, None), recoil::RAPID_REPO_MASTER);
	}

	#[tokio::test]
	async fn with_nobody_to_read_another_rapid_server_only_bars_is_used() {
		let vet = vet_bars_only();
		assert_eq!(vet(recoil::RAPID_REPO_MASTER.into()).await, Ok(()));
		assert!(vet("https://mods.example/repos.gz".into()).await.is_err());
	}

	#[test]
	fn a_game_rapid_does_not_know_is_said_to_be_unpublished() {
		let theirs = "https://mods.example/repos.gz";
		let runs = recoil::Download::runs(
			std::path::Path::new("prd"),
			std::path::Path::new("data"),
			vec![(recoil::Want::Game, "Somebody's Mod v1".into())],
			theirs,
		);
		let asked_nobody = format!(
			"[Error] search():Error downloading {}?category=game&springname=x",
			recoil::NO_SEARCH_URL
		);
		assert_eq!(
			unpublished(&runs[0], &[asked_nobody]).as_deref(),
			Some(
				"Somebody's Mod v1 is not published by this server's rapid server (https://mods.example/repos.gz)"
			)
		);
		assert_eq!(unpublished(&runs[0], &["disk full".into()]), None);
	}

	#[test]
	fn a_silent_failure_still_has_a_reason() {
		assert_eq!(failure_reason(&[]), "pr-downloader did not finish");
	}

	/// A connector whose single connection ends at the returned stream: the fake server.
	fn in_memory() -> (Connector, DuplexStream) {
		let (connector, mut servers) = in_memory_many(1);
		(connector, servers.remove(0))
	}

	/// A connector good for `count` connections, each ending at its own fake server, in order.
	fn in_memory_many(count: usize) -> (Connector, Vec<DuplexStream>) {
		let (client_sides, server_sides): (Vec<_>, Vec<_>) =
			(0..count).map(|_| tokio::io::duplex(64 * 1024)).unzip();
		let client_sides = std::sync::Mutex::new(client_sides.into_iter());
		let connector: Connector = Arc::new(move |_endpoint, policy| {
			let stream = client_sides
				.lock()
				.unwrap()
				.next()
				.expect("a connection left");
			Box::pin(async move {
				let (transport, inbound) = Transport::from_stream(stream, policy);
				Ok((transport, inbound, TEST_WAY))
			})
		});
		(connector, server_sides)
	}

	/// The session's phase, as a caller from before there were several servers asks it.
	fn phase(snapshot: &Snapshot) -> Option<Phase> {
		snapshot.session().and_then(|session| session.phase)
	}

	/// The way every in-memory connection claims to have come in by.
	const TEST_WAY: Way = Way {
		port: 8200,
		security: Security::None,
		ms: 1,
	};

	type FakeServer = (
		tokio::io::Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
		tokio::io::WriteHalf<DuplexStream>,
	);

	/// Plays the server through one login of `me`: greeting, acceptance, end of the flood.
	async fn accept_login(server: DuplexStream) -> FakeServer {
		accept_login_with(server, b"").await
	}

	/// The same, with `more` of the login flood before its end — a room, say.
	async fn accept_login_with(server: DuplexStream, more: &[u8]) -> FakeServer {
		let (read, mut write) = tokio::io::split(server);
		let mut lines = BufReader::new(read).lines();
		write.write_all(b"TASSERVER 0.38 * 8201 0\n").await.unwrap();
		let sent = lines.next_line().await.unwrap().unwrap();
		assert!(sent.starts_with("LOGIN me "), "{sent}");
		write
			.write_all(b"ACCEPTED me\nADDUSER me SE 1 LuaLobby Chobby\n")
			.await
			.unwrap();
		write.write_all(more).await.unwrap();
		write.write_all(b"LOGININFOEND\n").await.unwrap();
		(lines, write)
	}

	/// A connector that reaches each host's own fake server, and never
	/// answers for a host it has none for: a server that hangs.
	fn in_memory_hosts(hosts: &[&str]) -> (Connector, HashMap<String, DuplexStream>) {
		let mut clients = HashMap::new();
		let mut servers = HashMap::new();
		for host in hosts {
			let (client, server) = tokio::io::duplex(64 * 1024);
			clients.insert((*host).to_owned(), client);
			servers.insert((*host).to_owned(), server);
		}
		let clients = std::sync::Mutex::new(clients);
		let connector: Connector = Arc::new(move |endpoint: Endpoint, policy| {
			let stream = clients.lock().unwrap().remove(&endpoint.host);
			Box::pin(async move {
				let Some(stream) = stream else {
					return std::future::pending().await;
				};
				let (transport, inbound) = Transport::from_stream(stream, policy);
				Ok((transport, inbound, TEST_WAY))
			})
		});
		(connector, servers)
	}

	fn log_in(client: &Client, host: &str) -> tokio::task::JoinHandle<Result<(), ClientError>> {
		let client = client.clone();
		let endpoint = Endpoint::new(host);
		tokio::spawn(async move {
			client
				.login(endpoint, LoginRequest::new("me", "pw", "test", "h h"))
				.await
		})
	}

	/// Every server's phase, by id, as the front end would be told it.
	async fn phases(client: &Client) -> Vec<(String, Option<Phase>)> {
		client
			.snapshot()
			.await
			.unwrap()
			.servers
			.into_iter()
			.map(|server| (server.server, server.phase))
			.collect()
	}

	/// Room 5, hosted by `host`, as a login flood lists it.
	const ROOM: &[u8] =
		b"ADDUSER host DE 2 SPADS\nBATTLEOPENED 5 0 0 host 1.2.3.4 8452 16 0 0 h R\tv\tm\tt\tg\n";

	#[tokio::test]
	async fn two_servers_are_logged_in_side_by_side() {
		let (connector, mut servers) = in_memory_hosts(&["a", "b"]);
		let client = spawn(connector);
		let a = log_in(&client, "a");
		let b = log_in(&client, "b");
		let _a = accept_login(servers.remove("a").unwrap()).await;
		let _b = accept_login(servers.remove("b").unwrap()).await;
		a.await.unwrap().unwrap();
		b.await.unwrap().unwrap();
		assert_eq!(
			phases(&client).await,
			[
				("a".to_owned(), Some(Phase::Ready)),
				("b".to_owned(), Some(Phase::Ready))
			]
		);
		client.shutdown().await;
	}

	#[tokio::test]
	async fn a_server_that_hangs_holds_nobody_else_up() {
		let (connector, mut servers) = in_memory_hosts(&["b"]);
		let client = spawn(connector);
		let _hanging = log_in(&client, "slow");
		let b = log_in(&client, "b");
		let _b = accept_login(servers.remove("b").unwrap()).await;
		tokio::time::timeout(Duration::from_secs(2), b)
			.await
			.expect("b is in without waiting on the other")
			.unwrap()
			.unwrap();
		let known = phases(&client).await;
		assert!(known.contains(&("b".to_owned(), Some(Phase::Ready))));
		client.shutdown().await;
	}

	#[tokio::test]
	async fn one_room_at_a_time_and_its_words_go_to_its_server() {
		let (connector, mut servers) = in_memory_hosts(&["a", "b"]);
		let client = spawn(connector);
		let a = log_in(&client, "a");
		let b = log_in(&client, "b");
		let (mut a_lines, mut a_write) =
			accept_login_with(servers.remove("a").unwrap(), ROOM).await;
		let (mut b_lines, mut b_write) =
			accept_login_with(servers.remove("b").unwrap(), ROOM).await;
		a.await.unwrap().unwrap();
		b.await.unwrap().unwrap();

		let joining = tokio::spawn({
			let client = client.clone();
			async move { client.join_battle("a".into(), 5, None).await }
		});
		line_starting_with(&mut a_lines, "JOINBATTLE 5").await;
		a_write
			.write_all(b"JOINBATTLE 5 h\nJOINEDBATTLE 5 me\n")
			.await
			.unwrap();
		joining.await.unwrap().unwrap();

		// A join the other server refuses costs nothing: the room is kept.
		let joining = tokio::spawn({
			let client = client.clone();
			async move { client.join_battle("b".into(), 5, None).await }
		});
		line_starting_with(&mut b_lines, "JOINBATTLE 5").await;
		b_write
			.write_all(b"JOINBATTLEFAILED wrong password\n")
			.await
			.unwrap();
		assert!(joining.await.unwrap().is_err());
		let snapshot = client.snapshot().await.unwrap();
		let (kept, _) = snapshot.room().expect("still in the first room");
		assert_eq!(kept.server, "a");

		// The same room number on the other server is another room: once
		// let in there, the first is left.
		let joining = tokio::spawn({
			let client = client.clone();
			async move { client.join_battle("b".into(), 5, None).await }
		});
		line_starting_with(&mut b_lines, "JOINBATTLE 5").await;
		b_write
			.write_all(b"JOINBATTLE 5 h\nJOINEDBATTLE 5 me\n")
			.await
			.unwrap();
		joining.await.unwrap().unwrap();
		line_starting_with(&mut a_lines, "LEAVEBATTLE").await;

		client.say("hello".into()).await.unwrap();
		assert_eq!(
			line_starting_with(&mut b_lines, "SAYBATTLE ").await,
			"SAYBATTLE hello"
		);
		let snapshot = client.snapshot().await.unwrap();
		let (room_server, room) = snapshot.room().expect("a room");
		assert_eq!((room_server.server.as_str(), room.id), ("b", 5));
		client.shutdown().await;
	}

	#[tokio::test]
	async fn away_reaches_every_server_and_logout_one_or_all() {
		let (connector, mut servers) = in_memory_hosts(&["a", "b"]);
		let client = spawn(connector);
		let a = log_in(&client, "a");
		let b = log_in(&client, "b");
		let (mut a_lines, _a_write) = accept_login(servers.remove("a").unwrap()).await;
		let (mut b_lines, _b_write) = accept_login(servers.remove("b").unwrap()).await;
		a.await.unwrap().unwrap();
		b.await.unwrap().unwrap();

		client.set_away(true).await.unwrap();
		line_starting_with(&mut a_lines, "MYSTATUS").await;
		line_starting_with(&mut b_lines, "MYSTATUS").await;

		client.logout(Some("a".into())).await.unwrap();
		assert_eq!(
			phases(&client).await,
			[("b".to_owned(), Some(Phase::Ready))],
			"logged out of one, the other stays"
		);
		client.logout(None).await.unwrap();
		assert!(phases(&client).await.is_empty(), "and then of every one");
		client.shutdown().await;
	}

	#[tokio::test]
	async fn a_drop_is_retried_for_that_server_alone() {
		let (connector, mut servers) = in_memory_hosts(&["a", "b"]);
		let client = spawn(connector);
		let a = log_in(&client, "a");
		let b = log_in(&client, "b");
		let a_server = accept_login(servers.remove("a").unwrap()).await;
		let _b = accept_login(servers.remove("b").unwrap()).await;
		a.await.unwrap().unwrap();
		b.await.unwrap().unwrap();

		drop(a_server);
		tokio::time::sleep(Duration::from_millis(100)).await;
		let snapshot = client.snapshot().await.unwrap();
		let by_id: HashMap<_, _> = snapshot
			.servers
			.iter()
			.map(|server| (server.server.as_str(), (server.phase, server.retry_in)))
			.collect();
		let (phase, retry_in) = by_id["a"];
		assert_eq!(phase, None);
		assert!(retry_in.is_some(), "a comes back on its own");
		assert_eq!(by_id["b"], (Some(Phase::Ready), None), "b never noticed");
		client.shutdown().await;
	}

	#[tokio::test]
	async fn a_connect_that_lands_after_the_logout_is_dropped() {
		let gate = Arc::new(tokio::sync::Notify::new());
		let (stream, _server) = tokio::io::duplex(1024);
		let stream = std::sync::Mutex::new(Some(stream));
		let connector: Connector = Arc::new({
			let gate = Arc::clone(&gate);
			move |_endpoint, policy| {
				let stream = stream.lock().unwrap().take().expect("one connect");
				let gate = Arc::clone(&gate);
				Box::pin(async move {
					gate.notified().await;
					let (transport, inbound) = Transport::from_stream(stream, policy);
					Ok((transport, inbound, TEST_WAY))
				})
			}
		});
		let client = spawn(connector);
		let login = log_in(&client, "a");
		tokio::time::sleep(Duration::from_millis(50)).await;
		client.logout(Some("a".into())).await.unwrap();
		gate.notify_one();

		assert!(matches!(
			login.await.unwrap(),
			Err(ClientError::Refused(reason)) if reason.contains("logged out")
		));
		assert!(phases(&client).await.is_empty(), "nothing left of it");
	}

	#[tokio::test]
	async fn a_logout_frees_the_server_for_a_new_login_at_once() {
		// The first connect never comes back; the second is answered.
		let (stream, server) = tokio::io::duplex(64 * 1024);
		let streams = std::sync::Mutex::new(vec![Some(stream), None]);
		let connector: Connector = Arc::new(move |_endpoint, policy| {
			let stream = streams.lock().unwrap().pop().expect("two connects");
			Box::pin(async move {
				let Some(stream) = stream else {
					return std::future::pending().await;
				};
				let (transport, inbound) = Transport::from_stream(stream, policy);
				Ok((transport, inbound, TEST_WAY))
			})
		});
		let client = spawn(connector);
		let _hanging = log_in(&client, "a");
		tokio::time::sleep(Duration::from_millis(50)).await;
		assert_eq!(
			phases(&client).await,
			[("a".to_owned(), Some(Phase::Connecting))],
			"a connect that is out shows as one"
		);
		client.logout(Some("a".into())).await.unwrap();

		let again = log_in(&client, "a");
		let _server = accept_login(server).await;
		again.await.unwrap().unwrap();
		assert_eq!(
			phases(&client).await,
			[("a".to_owned(), Some(Phase::Ready))]
		);
		client.shutdown().await;
	}

	fn spawn(connector: Connector) -> Client {
		Client::spawn_with(
			ThrottlePolicy::default(),
			Hardware::stub(),
			connector,
			Arc::new(latency::Unmeasured),
			None,
		)
	}

	#[tokio::test]
	async fn reconnect_on_request_logs_in_again_with_the_same_credentials() {
		let (connector, mut servers) = in_memory_many(2);
		let second = servers.pop().unwrap();
		let first = servers.pop().unwrap();
		let client = spawn(connector);
		let ui = Collector::default();
		client.subscribe(ui.clone()).await.unwrap();

		let endpoint = Endpoint::new("test");
		let login = tokio::spawn({
			let client = client.clone();
			async move {
				client
					.login(endpoint, LoginRequest::new("me", "pw", "test", "h h"))
					.await
			}
		});
		let server = accept_login(first).await;
		login.await.unwrap().unwrap();

		// The server goes away. The runtime's own retry is armed but waits.
		drop(server);
		tokio::time::sleep(Duration::from_millis(100)).await;
		assert_eq!(phase(&client.snapshot().await.unwrap()), None);

		let reconnect = tokio::spawn({
			let client = client.clone();
			async move { client.reconnect("test".into(), None).await }
		});
		let _server = accept_login(second).await;
		reconnect.await.unwrap().unwrap();
		assert_eq!(phase(&client.snapshot().await.unwrap()), Some(Phase::Ready));
		client.shutdown().await;
	}

	#[tokio::test]
	async fn a_reconnect_goes_over_the_server_as_it_is_set_up_now() {
		let (inner, mut servers) = in_memory_many(2);
		let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
		let connector: Connector = Arc::new({
			let asked = Arc::clone(&asked);
			move |endpoint: Endpoint, policy| {
				asked.lock().unwrap().push(endpoint.allow_plain);
				inner(endpoint, policy)
			}
		});
		let second = servers.pop().unwrap();
		let first = servers.pop().unwrap();
		let client = spawn(connector);
		let login = log_in(&client, "test");
		let server = accept_login(first).await;
		login.await.unwrap().unwrap();

		// Plaintext allowed since: the reconnect is told, and the login's
		// own endpoint is not what goes out again.
		drop(server);
		tokio::time::sleep(Duration::from_millis(100)).await;
		let now = Endpoint {
			allow_plain: true,
			..Endpoint::new("test")
		};
		let reconnect = tokio::spawn({
			let client = client.clone();
			async move { client.reconnect("test".into(), Some(now)).await }
		});
		let _server = accept_login(second).await;
		reconnect.await.unwrap().unwrap();
		assert_eq!(*asked.lock().unwrap(), [false, true]);
		client.shutdown().await;
	}

	#[tokio::test]
	async fn the_way_that_worked_is_tried_first_next_time_until_it_is_forgotten() {
		let (inner, mut servers) = in_memory_many(2);
		let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
		let connector: Connector = Arc::new({
			let asked = Arc::clone(&asked);
			move |endpoint: Endpoint, policy| {
				asked.lock().unwrap().push(endpoint.preferred);
				inner(endpoint, policy)
			}
		});
		let second = servers.pop().unwrap();
		let first = servers.pop().unwrap();
		let client = spawn(connector);
		client.subscribe(Collector::default()).await.unwrap();

		let login = tokio::spawn({
			let client = client.clone();
			async move {
				let request = LoginRequest::new("me", "pw", "test", "h h");
				client.login(Endpoint::new("test"), request).await
			}
		});
		let server = accept_login(first).await;
		login.await.unwrap().unwrap();
		let remembered = client.snapshot().await.unwrap().ways;
		assert_eq!(remembered.get("test"), Some(&TEST_WAY.to_string()));

		drop(server);
		tokio::time::sleep(Duration::from_millis(100)).await;
		let reconnect = tokio::spawn({
			let client = client.clone();
			async move { client.reconnect("test".into(), None).await }
		});
		let _server = accept_login(second).await;
		reconnect.await.unwrap().unwrap();
		assert_eq!(*asked.lock().unwrap(), [None, Some(TEST_WAY)]);

		client.forget_way("TEST".into()).await.unwrap();
		assert!(client.snapshot().await.unwrap().ways.is_empty());
		client.shutdown().await;
	}

	#[tokio::test]
	async fn a_session_is_filed_under_its_server() {
		let (connector, server) = in_memory();
		let client = spawn(connector);
		let ui = Collector::default();
		client.subscribe(ui.clone()).await.unwrap();
		let login = tokio::spawn({
			let client = client.clone();
			async move {
				let request = LoginRequest::new("me", "pw", "test", "h h");
				client.login(Endpoint::new(" Test "), request).await
			}
		});
		let _server = accept_login(server).await;
		login.await.unwrap().unwrap();

		let snapshot = client.snapshot().await.unwrap();
		let servers: Vec<&str> = snapshot.servers.iter().map(|s| s.server.as_str()).collect();
		assert_eq!(
			servers,
			["test"],
			"one session, under the trimmed, lowercased host"
		);
		for message in ui.take() {
			if let UiMessage::Deltas { server, deltas } = message
				&& deltas.iter().any(|delta| matches!(delta, Delta::Phase(_)))
			{
				assert_eq!(server.as_deref(), Some("test"));
			}
		}
		client.shutdown().await;
	}

	#[tokio::test]
	async fn reconnect_before_any_login_has_nothing_to_use() {
		let (connector, _server) = in_memory();
		let client = spawn(connector);
		assert!(matches!(
			client.reconnect("test".into(), None).await,
			Err(ClientError::NoCredentials)
		));
		client.shutdown().await;
	}

	/// Plays the server through a registration: the greeting, the account
	/// made, and the agreement it answers the first login with. The trailing
	/// empty `AGREEMENT ` is teiserver's own (`spring_out.ex:111-121`).
	async fn accept_registration(server: DuplexStream) -> FakeServer {
		let (read, mut write) = tokio::io::split(server);
		let mut lines = BufReader::new(read).lines();
		write.write_all(b"TASSERVER 0.38 * 8201 0\n").await.unwrap();
		let sent = lines.next_line().await.unwrap().unwrap();
		assert!(sent.starts_with("REGISTER me "), "{sent}");
		write.write_all(b"REGISTRATIONACCEPTED\n").await.unwrap();
		let sent = lines.next_line().await.unwrap().unwrap();
		assert!(sent.starts_with("LOGIN me "), "{sent}");
		write
			.write_all(
				b"AGREEMENT Read the terms at https://example/privacy\nAGREEMENT \nAGREEMENTEND\n",
			)
			.await
			.unwrap();
		(lines, write)
	}

	fn registering(client: &Client) -> tokio::task::JoinHandle<Result<Vec<String>, ClientError>> {
		let client = client.clone();
		let endpoint = Endpoint::new("test");
		tokio::spawn(async move {
			client
				.register(
					endpoint,
					LoginRequest::new("me", "pw", "test", "h h"),
					"a@b.c".into(),
					"pw".into(),
				)
				.await
		})
	}

	#[tokio::test]
	async fn registering_logs_in_on_the_same_connection_and_answers_with_the_agreement() {
		let (connector, mut servers) = in_memory_many(1);
		let client = spawn(connector);
		let made = registering(&client);
		let _server = accept_registration(servers.remove(0)).await;

		let agreement = made.await.unwrap().unwrap();
		assert_eq!(agreement, ["Read the terms at https://example/privacy", ""]);
		// Still connected: the code has nowhere else to go.
		assert_eq!(
			phase(&client.snapshot().await.unwrap()),
			Some(Phase::AwaitingLogin)
		);
		client.shutdown().await;
	}

	#[tokio::test]
	async fn a_confirmed_code_finishes_the_login_that_registering_started() {
		let (connector, mut servers) = in_memory_many(1);
		let client = spawn(connector);
		let made = registering(&client);
		let (mut lines, mut write) = accept_registration(servers.remove(0)).await;
		made.await.unwrap().unwrap();

		let confirming = tokio::spawn({
			let client = client.clone();
			async move {
				client
					.confirm_agreement("test".into(), "A1B2C3".into())
					.await
			}
		});
		let sent = lines.next_line().await.unwrap().unwrap();
		assert_eq!(sent, "CONFIRMAGREEMENT A1B2C3");
		// teiserver verifies the account and runs the whole login on it.
		write
			.write_all(b"ACCEPTED me\nADDUSER me SE 1 LuaLobby Chobby\nLOGININFOEND\n")
			.await
			.unwrap();

		confirming.await.unwrap().unwrap();
		assert_eq!(phase(&client.snapshot().await.unwrap()), Some(Phase::Ready));
		client.shutdown().await;
	}

	#[tokio::test]
	async fn a_wrong_code_answers_the_caller_rather_than_failing_silently() {
		let (connector, mut servers) = in_memory_many(1);
		let client = spawn(connector);
		let made = registering(&client);
		let (mut lines, mut write) = accept_registration(servers.remove(0)).await;
		made.await.unwrap().unwrap();

		let confirming = tokio::spawn({
			let client = client.clone();
			async move { client.confirm_agreement("test".into(), "nope".into()).await }
		});
		lines.next_line().await.unwrap().unwrap();
		write.write_all(b"DENIED Incorrect code\n").await.unwrap();

		let answer = confirming.await.unwrap();
		assert!(
			matches!(&answer, Err(ClientError::Refused(reason)) if reason == "Incorrect code"),
			"{answer:?}"
		);
		client.shutdown().await;
	}

	#[tokio::test]
	async fn a_registration_the_server_refuses_leaves_nothing_to_reconnect_with() {
		let (connector, mut servers) = in_memory_many(1);
		let client = spawn(connector);
		let made = registering(&client);

		let (read, mut write) = tokio::io::split(servers.remove(0));
		let mut lines = BufReader::new(read).lines();
		write.write_all(b"TASSERVER 0.38 * 8201 0\n").await.unwrap();
		lines.next_line().await.unwrap().unwrap();
		write
			.write_all(b"REGISTRATIONDENIED Username already taken\n")
			.await
			.unwrap();

		let answer = made.await.unwrap();
		assert!(
			matches!(&answer, Err(ClientError::Refused(r)) if r == "Username already taken"),
			"{answer:?}"
		);
		// No account was made, so nothing was left behind to come back to.
		assert!(matches!(
			client.reconnect("test".into(), None).await,
			Err(ClientError::NoCredentials)
		));
		client.shutdown().await;
	}

	#[tokio::test]
	async fn confirming_with_no_connection_says_so() {
		let (connector, _server) = in_memory();
		let client = spawn(connector);
		assert!(matches!(
			client
				.confirm_agreement("test".into(), "A1B2C3".into())
				.await,
			Err(ClientError::NotConnected)
		));
		client.shutdown().await;
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
			None,
		);
		let ui = Collector::default();
		client.subscribe(ui.clone()).await.unwrap();

		let endpoint = Endpoint::new("test");
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
			.position(|m| matches!(m, UiMessage::Session(s) if s.phase == Some(Phase::Ready)))
			.expect("the session, once ready");
		if let UiMessage::Session(session) = &messages[snapshot_at] {
			assert_eq!(session.server, "test");
			assert_eq!(session.battles.len(), 1);
			assert_eq!(session.users.len(), 2);
		}
		let after: Vec<&Delta> = messages[snapshot_at + 1..]
			.iter()
			.filter_map(|m| match m {
				UiMessage::Deltas { deltas, .. } => Some(deltas.iter()),
				_ => None,
			})
			.flatten()
			.collect();
		assert!(
			after
				.iter()
				.any(|d| matches!(d, Delta::UserAdded(u) if u.name == "bob"))
		);

		assert_eq!(
			client
				.snapshot()
				.await
				.unwrap()
				.session()
				.unwrap()
				.users
				.len(),
			3
		);
		client.shutdown().await;
	}

	#[tokio::test]
	async fn an_untouched_window_lets_the_server_go_and_stays_gone() {
		let (connector, server) = in_memory();
		let (server_read, mut server_write) = tokio::io::split(server);
		let mut server_lines = BufReader::new(server_read).lines();

		let client = Client::spawn_with(
			ThrottlePolicy::default(),
			Hardware::stub(),
			connector,
			Arc::new(latency::Unmeasured),
			None,
		);
		let ui = Collector::default();
		client.subscribe(ui.clone()).await.unwrap();
		client
			.set_idle_timeout(Some(Duration::from_millis(300)))
			.await
			.unwrap();

		let endpoint = Endpoint::new("test");
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
		server_lines.next_line().await.unwrap().unwrap();
		server_write
			.write_all(b"ACCEPTED me\nADDUSER me SE 1 modlobby\nLOGININFOEND\n")
			.await
			.unwrap();
		login.await.unwrap().unwrap();

		// Touching the window inside the limit keeps the connection.
		tokio::time::sleep(Duration::from_millis(200)).await;
		client.activity().await.unwrap();
		tokio::time::sleep(Duration::from_millis(200)).await;
		assert_eq!(
			phase(&client.snapshot().await.unwrap()),
			Some(Phase::Ready),
			"activity resets the limit"
		);

		// Left alone past it, the connection goes, with a word about why.
		tokio::time::sleep(Duration::from_millis(400)).await;
		assert_eq!(phase(&client.snapshot().await.unwrap()), None);
		let deltas: Vec<Delta> = ui
			.take()
			.into_iter()
			.filter_map(|m| match m {
				UiMessage::Deltas { deltas, .. } => Some(deltas),
				_ => None,
			})
			.flatten()
			.collect();
		assert!(deltas.iter().any(|d| matches!(
			d,
			Delta::Notice { level: lobby_ui::NoticeLevel::Info, text } if text.contains("disconnected")
		)));

		// And it does not come back: nothing the fake server reads after the
		// drop is a second LOGIN, and the phase stays down. The read is
		// bounded because the fake server holds its own half of the pipe
		// open, so the client's reader never sees an end to wait for.
		let drained = tokio::time::timeout(Duration::from_millis(300), async {
			while let Some(line) = server_lines.next_line().await.unwrap() {
				assert!(!line.starts_with("LOGIN "), "reconnected: {line}");
			}
		});
		let _ = drained.await;
		assert_eq!(phase(&client.snapshot().await.unwrap()), None);
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
			None,
		);
		let endpoint = Endpoint::new("test");
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
			async move { client.host_public("test".into()).await }
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
