//! One actor owns the connection, the reducer, the engine child and the UI
//! transport; front ends send [`Command`]s through a [`Client`] handle. Every
//! inbound line is reduced, projected into deltas and batched: a burst that is
//! already queued becomes one `Deltas` message.

use std::collections::VecDeque;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};

use crate::idle;
use crate::latency::{self, Latency};
use crate::launch;
use crate::misses::{self, Misses};
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
	/// The account has not confirmed the code emailed to it, so the server
	/// answered the login with its agreement: the server's own words, which
	/// say where the code went and what entering it agrees to. The connection
	/// stays up for the code.
	#[error("{}", unverified(.0))]
	Unverified(Vec<String>),
	/// A registration refused because the name is an account already, which
	/// may be the person's own: made earlier, and never confirmed.
	#[error("{0}")]
	NameTaken(String),
}

fn unverified(agreement: &[String]) -> String {
	let said: Vec<&str> = agreement
		.iter()
		.map(|line| line.trim())
		.filter(|line| !line.is_empty())
		.collect();
	if said.is_empty() {
		return "this account must confirm the code emailed to it".into();
	}
	said.join("\n")
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

/// The last resort for a map: the room's own host, which is playing it and
/// so has the file. Handed in for the same reason [`Vet`] is -- the runtime
/// has no business knowing what a LAN room is -- and `None` from a room that
/// has no such host, which is every room on a server.
///
/// Takes the map's name only to say what it is fetching; who is asked, and
/// what is proved to them, is the business of whatever answers this.
pub type FromHost = Arc<
	dyn Fn(
			Ask,
			mpsc::Sender<recoil::Progress>,
		) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
		+ Send
		+ Sync,
>;

/// What the room's host is asked for, and what proves the asking.
///
/// The credentials come from here rather than being looked up by whoever
/// answers: the script password is the runtime's to keep -- it never reaches
/// the front end or a view -- and this hands it to one caller for one fetch.
#[derive(Debug, Clone)]
pub struct Ask {
	/// The room's server, so an answerer can tell the one room it serves
	/// from every other.
	pub server: Option<String>,
	/// The map, as the room names it.
	pub map: String,
	/// Us, in that room.
	pub me: String,
	/// What we gave at `JOINBATTLE`; the host knows it and nobody else does.
	pub script_password: String,
}

/// No host to ask, which is every room but one on the local network.
fn no_host() -> FromHost {
	Arc::new(|_, _| Box::pin(async { Err("this room has no host to ask".to_owned()) }))
}

/// A game from somewhere other than the room's server's rapid: the player's
/// override, modlobby's list, coilbox's hub (`content::sources`). Handed in
/// for the same reason [`Vet`] is. `None` is nothing to try; `Some(Ok(from))`
/// says where the game came from, for a person to read.
pub type GameSources = Arc<
	dyn Fn(
			GameAsk,
			mpsc::Sender<recoil::Progress>,
			RapidRun,
		) -> Pin<Box<dyn Future<Output = Option<Result<String, String>>> + Send>>
		+ Send
		+ Sync,
>;

/// pr-downloader run for the asked game against another rapid master, vetted
/// as the room's is and filling the same bar: what a list's rapid entry is
/// fetched by, handed to [`GameSources`] since only the runtime runs it.
pub type RapidRun =
	Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;

/// What [`GameSources`] is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameAsk {
	/// The game, as the room names it.
	pub name: String,
	/// Where downloads go (a game under the write directory's `games/`),
	/// and every directory a dependency may already be in.
	pub dirs: DataDirs,
	/// Before the room's rapid is tried only the player's override answers,
	/// which is their word and replaces rapid; after, the lists.
	pub after_rapid: bool,
	/// The engine the room plays on, whose base content the game's checksum
	/// takes in.
	pub engine: String,
	/// What the room announced for its game (`JOINBATTLE`), which a copy
	/// from outside rapid is held to; `None` with no room to ask.
	pub room_hash: Option<u32>,
	/// For a mutator, the checksum its host announced for it, which a copy
	/// from anywhere is held to in place of `room_hash`.
	pub checksum: Option<String>,
	/// For a mutator, the commit its host pinned it to: built from GitHub as
	/// it stands there, before anything else is asked.
	pub pin: Option<lobby_core::GitPin>,
}

/// Nowhere but rapid, until something better is handed in.
fn rapid_only() -> GameSources {
	Arc::new(|_, _, _| Box::pin(async { None }))
}

/// Runs `fetch` with its progress going where pr-downloader's goes, so a
/// handover fills the same bar rather than leaving it stopped.
async fn pumped<T>(
	progress: &mpsc::Sender<DownloadEvent>,
	fetch: impl FnOnce(mpsc::Sender<recoil::Progress>) -> Pin<Box<dyn Future<Output = T> + Send>>,
) -> T {
	let (tx, mut rx) = mpsc::channel::<recoil::Progress>(8);
	let pump = tokio::spawn({
		let progress = progress.clone();
		async move {
			while let Some(step) = rx.recv().await {
				let _ = progress.send(DownloadEvent::Progress(step)).await;
			}
		}
	});
	let outcome = fetch(tx).await;
	pump.abort();
	outcome
}

/// The game a run wants, and why it did not come, if it did not: the
/// player's override instead of rapid; else the room's rapid, read first if
/// it is not BAR's; and after rapid fails for any reason, the lists.
async fn fetch_game(
	run: &recoil::Download,
	room: &Room,
	vet: &Vet,
	sources: &GameSources,
	progress: &mpsc::Sender<DownloadEvent>,
) -> Option<String> {
	let (want, name) = run
		.wants
		.iter()
		.find(|(want, _)| *want != recoil::Want::Map)
		.cloned()?;
	// Only the room's game has the room's hash; a mutator has the checksum
	// and the commit its host announced for it.
	let room_hash = room.hash.filter(|_| want == recoil::Want::Game);
	let mutator = room
		.mutators
		.iter()
		.find(|mutator| want == recoil::Want::Mutator && mutator.name == name);
	let ask = |after_rapid| GameAsk {
		name: name.clone(),
		dirs: room.dirs.clone(),
		after_rapid,
		engine: room.engine.clone(),
		room_hash,
		checksum: mutator.and_then(|mutator| mutator.checksum.clone()),
		pin: mutator.and_then(|mutator| mutator.source.clone()),
	};
	let elsewhere = rapid_elsewhere(run, vet, progress);
	if let Some(got) = ask_sources(sources, ask(false), progress, elsewhere.clone()).await {
		return got.err();
	}
	let rapid = match vet(run.rapid_master.clone()).await {
		Ok(()) => run_download(run, progress).await,
		Err(refused) => Err(refused),
	};
	let reason = rapid.err()?;
	match ask_sources(sources, ask(true), progress, elsewhere).await {
		None => Some(reason),
		Some(Ok(_)) => None,
		Some(Err(more)) => Some(format!("{reason}; {more}")),
	}
}

/// Everything a game run wants, one name at a time -- the room's game, then
/// the mutators it loads -- each the way [`fetch_game`] fetches, so one that
/// cannot be had does not cost the others. Why any did not come, if one did
/// not.
async fn fetch_games(
	run: &recoil::Download,
	room: &Room,
	vet: &Vet,
	sources: &GameSources,
	progress: &mpsc::Sender<DownloadEvent>,
) -> Option<String> {
	let mut failures = Vec::new();
	for wanted in run
		.wants
		.iter()
		.filter(|(want, _)| *want != recoil::Want::Map)
	{
		let one = recoil::Download {
			wants: vec![wanted.clone()],
			..run.clone()
		};
		failures.extend(fetch_game(&one, room, vet, sources, progress).await);
	}
	(!failures.is_empty()).then(|| failures.join("; "))
}

/// What a download is for, beyond what it wants: the room's engine and the
/// hash it announced, and where content lives.
#[derive(Debug, Clone)]
struct Room {
	dirs: DataDirs,
	engine: String,
	hash: Option<u32>,
	/// What the room announced for each mutator it loads.
	mutators: Vec<lobby_core::Mutator>,
}

/// `run`'s game from another rapid master: vetted, and nothing else of the
/// run's fetched with it.
fn rapid_elsewhere(
	run: &recoil::Download,
	vet: &Vet,
	progress: &mpsc::Sender<DownloadEvent>,
) -> RapidRun {
	let (run, vet, progress) = (run.clone(), vet.clone(), progress.clone());
	Arc::new(move |master: String| {
		let (mut run, vet, progress) = (run.clone(), vet.clone(), progress.clone());
		Box::pin(async move {
			vet(master.clone()).await?;
			run.rapid_master = master;
			run.wants.retain(|(want, _)| *want != recoil::Want::Map);
			run_download(&run, &progress).await
		})
	})
}

/// Asks `sources`, and says where the game came from when it came.
async fn ask_sources(
	sources: &GameSources,
	ask: GameAsk,
	progress: &mpsc::Sender<DownloadEvent>,
	rapid: RapidRun,
) -> Option<Result<String, String>> {
	let game = ask.name.clone();
	let got = pumped(progress, |tx| sources(ask, tx, rapid)).await?;
	if let Ok(from) = &got {
		let from = from.clone();
		let _ = progress.send(DownloadEvent::Sourced { game, from }).await;
	}
	Some(got)
}

/// Where BAR publishes its games and maps: its rapid master index and its
/// search, as its launcher config names them. Until told, the addresses
/// modlobby was built with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarContent {
	pub rapid_master: String,
	pub search: String,
}

impl Default for BarContent {
	fn default() -> Self {
		Self {
			rapid_master: recoil::RAPID_REPO_MASTER.to_owned(),
			search: recoil::HTTP_SEARCH_URL.to_owned(),
		}
	}
}

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
	/// Starts the engine as the game server of the room we are in, on a
	/// script somebody else wrote from that room, and tells the room so.
	LaunchHosted {
		dirs: DataDirs,
		engine_version: String,
		script: String,
		reply: Reply<()>,
	},
	/// Whether idling is to be left alone: a hosted room is not idleness
	/// even while its host waits for people to arrive.
	KeepAwake(bool),
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
		/// A ready to follow, once the server has answered with the seat.
		ready: bool,
		reply: Reply<()>,
	},
	SetReady {
		ready: bool,
		reply: Reply<()>,
	},
	SetPreReady {
		on: bool,
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
	/// Each server's own map search (`find`), by server id.
	SetMapSearches(BTreeMap<String, String>),
	/// The spring names of BAR's maps; see [`map_searches_for`].
	SetBarMaps(BTreeSet<String>),
	SetBarContent(BarContent),
	SetVet(Vet),
	/// Where a map comes from when no search had it; see [`FromHost`].
	SetFromHost(FromHost),
	/// Where a game comes from when it is not the room's rapid's.
	SetGameSources(GameSources),
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

	/// Starts the engine as the host of the room we are in, on `script`.
	pub async fn launch_hosted(
		&self,
		dirs: DataDirs,
		engine_version: String,
		script: String,
	) -> Result<(), ClientError> {
		self.ask(|reply| Command::LaunchHosted {
			dirs,
			engine_version,
			script,
			reply,
		})
		.await
	}

	pub async fn keep_awake(&self, on: bool) -> Result<(), ClientError> {
		self.send(Command::KeepAwake(on)).await
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

	/// Arms a ready given in advance, or takes it back: it answers the
	/// server's next automatic unready once, then is spent.
	pub async fn set_pre_ready(&self, on: bool) -> Result<(), ClientError> {
		self.ask(|reply| Command::SetPreReady { on, reply }).await
	}

	/// Picks a faction: 0 Armada, 1 Cortex, 2 Random, 3 Legion.
	pub async fn set_side(&self, side: u8) -> Result<(), ClientError> {
		self.ask(|reply| Command::SetSide { side, reply }).await
	}

	/// Takes a player slot. Refused only outside a room — see
	/// [`lobby_core::SeatError`].
	pub async fn take_seat(&self, team: u8, ally_team: u8, ready: bool) -> Result<(), ClientError> {
		self.ask(|reply| Command::TakeSeat {
			team,
			ally_team,
			ready,
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

	/// Where each server's own maps are asked for: its `find`, by server id.
	/// Asked only for a map BAR does not have.
	pub async fn set_map_searches(
		&self,
		searches: BTreeMap<String, String>,
	) -> Result<(), ClientError> {
		self.send(Command::SetMapSearches(searches)).await
	}

	/// BAR's maps by spring name, which decide who a map is asked of. Until
	/// they are known a map is asked of its server's search, else of BAR's.
	pub async fn set_bar_maps(&self, names: BTreeSet<String>) -> Result<(), ClientError> {
		self.send(Command::SetBarMaps(names)).await
	}

	/// Where BAR's own games and maps are, as BAR's launcher config says.
	pub async fn set_bar_content(&self, bar: BarContent) -> Result<(), ClientError> {
		self.send(Command::SetBarContent(bar)).await
	}

	/// Who reads a rapid server that is not BAR's before games are fetched
	/// from it. Until one is set, no such server is fetched from.
	pub async fn set_vet(&self, vet: Vet) -> Result<(), ClientError> {
		self.send(Command::SetVet(vet)).await
	}

	/// Who to ask for a map that no search had. See [`FromHost`].
	pub async fn set_from_host(&self, from_host: FromHost) -> Result<(), ClientError> {
		self.send(Command::SetFromHost(from_host)).await
	}

	/// Where games come from besides the room's rapid; see [`GameSources`].
	pub async fn set_game_sources(&self, sources: GameSources) -> Result<(), ClientError> {
		self.send(Command::SetGameSources(sources)).await
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
	/// Whether the room's game and map here are the ones it plays, worked out.
	Checked(Checked),
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

/// `wait` from now in Unix milliseconds: the page's clock, for a deadline it
/// draws down to.
fn unix_ms_after(wait: std::time::Duration) -> u64 {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default();
	(now + wait).as_millis() as u64
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
	/// Whether the idle limit is to be ignored, for as long as somebody says so.
	keep_awake: bool,
	/// When the room was asked for, until its state has all arrived: the
	/// `join:` milestones in the log are measured from here.
	join_asked: Option<Instant>,
	/// Where BAR's content lives; `None` falls back to the launcher's directory.
	data_dir: Option<PathBuf>,
	/// Each server's rapid master index, by server id; see [`Self::rapid_master`].
	rapid_masters: BTreeMap<String, String>,
	map_searches: BTreeMap<String, String>,
	bar_maps: BTreeSet<String>,
	bar: BarContent,
	/// Maps a search said it does not have, kept beside the ways.
	misses: Misses,
	vet: Vet,
	from_host: FromHost,
	game_sources: GameSources,
	/// Where to put a config that gets the game borderless, when the user's
	/// own would not let the overlay cover it. `None` leaves their settings
	/// entirely alone, which is also what happens when they already work.
	overlay_config_dir: Option<PathBuf>,
	menu_archive: Option<recoil::MenuArchive>,
	/// The room's engine, game, map and mutators the content check last ran
	/// against; scanning the rapid index is too slow to repeat per message.
	checked: Option<RoomContent>,
	/// The room (server, battle) whose host has been told we load mutators,
	/// so it is told once.
	greeted_mutator_host: Option<(String, u32)>,
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
	auto_fetched: Option<RoomContent>,
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
	check_tx: mpsc::Sender<Checked>,
	check_rx: mpsc::Receiver<Checked>,
	/// The room's content and what it was held to, as last pushed, so a
	/// reloaded window has them from its snapshot.
	content_view: Option<ContentView>,
	content_check: lobby_ui::ContentCheckView,
	/// What the last content check was for, so an answer for a room since
	/// left is let go.
	check_for: Option<CheckKey>,
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

/// A room's engine, and its game and map by name, each with the hash the
/// room announced for it -- `None` for one that is not here or had none --
/// and the mutators it loads, each with whether it is here.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckKey {
	engine: String,
	game: (String, Option<u32>),
	map: (String, Option<u32>),
	mutators: Vec<(lobby_core::Mutator, bool)>,
}

/// What a room asks this machine for: its engine, game and map by name, and
/// the mutators it loads on top.
type RoomContent = (String, String, String, Vec<lobby_core::Mutator>);

/// A content check's answer.
#[derive(Debug)]
struct Checked {
	key: CheckKey,
	view: lobby_ui::ContentCheckView,
}

/// Where a mutator the room loads is here. One its host pinned to a commit
/// is the build of that commit, found by the file name the room loads it by,
/// so no other copy of the same mod passes for it; any other is found by the
/// name inside it.
fn mutator_archive(
	library: &content::Library,
	engine: &str,
	mutator: &lobby_core::Mutator,
) -> Option<PathBuf> {
	let Some(pin) = &mutator.source else {
		return library.archive_named(engine, &mutator.name);
	};
	let built = content::git::build_name(&pin.repo, &pin.commit);
	if mutator.name != built {
		tracing::warn!(name = %mutator.name, %built, "a pinned mutator the room names otherwise than its build");
		return None;
	}
	library.archive_file(&built)
}

/// Whether the room has said all a fetch needs to know of a mutator. A host
/// sends a room's tags a line at a time, and between two lines a build can be
/// named with no commit yet, or beside the commit of the mutator that held
/// its place before. Asked for then, it would be looked for by its file name
/// where only a build has one, or built from the wrong commit.
fn announced_whole(mutator: &lobby_core::Mutator) -> bool {
	match &mutator.source {
		Some(pin) => mutator.name == content::git::build_name(&pin.repo, &pin.commit),
		None => !content::git::is_build_name(&mutator.name),
	}
}

/// A mutator as the front end shows it: by the name inside it where that is
/// known, else the repository it is built from, else the name the room
/// loads it by.
fn mutator_view(
	mutator: &lobby_core::Mutator,
	here: bool,
	inside: Option<(String, Option<String>)>,
	check: Option<lobby_ui::CheckView>,
) -> lobby_ui::MutatorView {
	let (title, description) = inside.unzip();
	lobby_ui::MutatorView {
		title: title
			.or_else(|| mutator.source.as_ref().map(|pin| pin.repo.clone()))
			.unwrap_or_else(|| mutator.name.clone()),
		description: description.flatten(),
		name: mutator.name.clone(),
		here,
		check,
		source: mutator
			.source
			.as_ref()
			.map(|pin| format!("github:{}@{}", pin.repo, pin.commit)),
		date: mutator.date.clone(),
	}
}

/// The room's game and map here held to the room's hashes: what
/// `content::checksum` makes of each, as the front end shows it.
fn check_content(dirs: DataDirs, key: &CheckKey) -> lobby_ui::ContentCheckView {
	use content::checksum::Verdict;
	use lobby_ui::CheckView;
	let library = content::Library::new(dirs);
	let find = |name: &str| library.archive_named(&key.engine, name);
	let view = |verdict, hash| match verdict {
		Verdict::Matches => CheckView::Same { hash },
		Verdict::Differs { ours, room } => CheckView::Differs { ours, room },
		Verdict::Unchecked(why) => CheckView::Unchecked { why },
	};
	let held = |path: Option<PathBuf>, hash: Option<u32>| {
		let (path, hash) = (path?, hash?);
		Some(view(
			content::checksum::against_room(&path, hash, &find),
			hash,
		))
	};
	let announced = |mutator: &lobby_core::Mutator| {
		let checksum = mutator.checksum.as_deref()?;
		let path = mutator_archive(&library, &key.engine, mutator)?;
		let hash = content::checksum::announced_hash(checksum)?;
		Some(view(
			content::checksum::against_announced(&path, checksum),
			hash,
		))
	};
	lobby_ui::ContentCheckView {
		game: held(library.archive_named(&key.engine, &key.game.0), key.game.1),
		map: held(library.map_archive(&key.map.0), key.map.1),
		mutators: key
			.mutators
			.iter()
			.map(|(mutator, here)| {
				let inside = mutator_archive(&library, &key.engine, mutator)
					.and_then(|path| content::map_name::game_and_description(&path));
				mutator_view(mutator, *here, inside, announced(mutator))
			})
			.collect(),
	}
}

/// What a pr-downloader child reports back to the runtime.
#[derive(Debug)]
enum DownloadEvent {
	Progress(recoil::Progress),
	/// `search` answered that it has no `map`.
	Missed {
		search: String,
		map: String,
	},
	/// `game` came from somewhere other than rapid, and `from` says where.
	Sourced {
		game: String,
		from: String,
	},
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
fn master_for(masters: &BTreeMap<String, String>, server: Option<&str>, bars: &str) -> String {
	server
		.and_then(|server| masters.get(server))
		.map_or(bars, String::as_str)
		.to_owned()
}

/// One pr-downloader run to its end, reporting progress on the way; the
/// failure is for a person to read.
async fn run_download(
	run: &recoil::Download,
	progress: &mpsc::Sender<DownloadEvent>,
) -> Result<(), String> {
	let mut child = crate::launch::spawn_download(run)?;
	// Read beside stdout, or a full pipe would stall the child.
	let errors = child.stderr.take().map(|stderr| {
		tokio::spawn(async move {
			let mut said = Vec::new();
			let mut lines = BufReader::new(stderr).lines();
			while let Ok(Some(line)) = lines.next_line().await {
				if pr_downloader_said(&line) {
					said.push(line);
				}
			}
			said
		})
	});
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
	// Last, so what went wrong ends the message.
	if let Some(errors) = errors {
		for line in errors.await.unwrap_or_default() {
			remember_tail(&mut tail, &line);
		}
	}
	if let Some(map) = missed(run, &tail) {
		let search = run.search_url.clone();
		let _ = progress.send(DownloadEvent::Missed { search, map }).await;
	}
	Err(unpublished(run, &tail).unwrap_or_else(|| failure_reason(&tail)))
}

/// Whether a stderr line is pr-downloader's own: its `[Error]` and `[Warn]`
/// lines go there (as of the 2026.09.01 engine), including the search that
/// tells an unpublished game apart. curl's tracing (`* Trying …`) goes there
/// too, and says nothing a person needs.
fn pr_downloader_said(line: &str) -> bool {
	line.starts_with('[')
}

/// What a download that wanted a game and a map came to. The game's failure
/// is the one to tell, and says what became of the map beside it.
fn fetched(game: Option<String>, map: Result<(), String>) -> Result<(), String> {
	match (game, map) {
		(Some(game), Err(map)) => Err(format!("{game}; {map}")),
		(Some(game), Ok(())) => Err(game),
		(None, map) => map,
	}
}

/// The map a map run's search answered "no such thing" for: a 404, which is
/// an answer. Anything else — no network, a server down — is not, and is not
/// remembered as one.
fn missed(run: &recoil::Download, tail: &[String]) -> Option<String> {
	let not_found = !run.has_games() && tail.iter().any(|line| line.contains("error: 404"));
	let (_, map) = run.wants.first()?;
	not_found.then(|| map.clone())
}

/// Who a map is asked of, in the order they are asked, until one answers.
///
/// BAR's search for one of BAR's maps; the room's server's own for any other,
/// so neither is asked about the other's; then springfiles, which is nobody's
/// server and has what no server publishes.
///
/// A third-party search is still never asked for one of BAR's names, so it
/// cannot put its own map under one -- springfiles included. That is why the
/// fallback needs to *know* the name is not BAR's rather than merely not find
/// it in the list: with `bar_maps` empty the list has not loaded, every name
/// is unknown, and the old answer stands.
///
/// Never empty, so a map is always looked for somewhere. Before this, a
/// custom map on a server naming no search was refused outright, which is
/// every map in a room on the LAN.
///
/// ponytail: a map is trusted as far as the search that named it -- pr-
/// downloader checks the md5 that same search gave, and nothing checks one
/// search's answer against another's. https at least.
fn map_searches_for(
	bar_maps: &BTreeSet<String>,
	searches: &BTreeMap<String, String>,
	server: Option<&str>,
	map: &str,
	bars: &str,
) -> Vec<String> {
	let own = server.and_then(|server| searches.get(server));
	if bar_maps.contains(map) || (own.is_none() && bar_maps.is_empty()) {
		return vec![bars.to_owned()];
	}
	let mut asked = Vec::new();
	match own {
		Some(search) if search.starts_with("https://") => asked.push(search.clone()),
		Some(search) => tracing::warn!(%search, "map search is not https; not asked"),
		None => {}
	}
	// Only for a name we know is not BAR's: an unloaded list is not evidence.
	if !bar_maps.is_empty() {
		asked.push(recoil::SPRINGFILES_SEARCH_URL.to_owned());
	}
	asked.dedup();
	asked
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
		let (check_tx, check_rx) = mpsc::channel(1);
		let (opened_tx, opened_rx) = mpsc::channel(8);
		let cache = state_dir
			.as_deref()
			.map_or_else(latency::Cache::default, |dir| {
				latency::Cache::load(&dir.join(latency::FILE))
			});
		let misses = state_dir
			.as_deref()
			.map_or_else(Misses::default, |dir| Misses::load(&dir.join(misses::FILE)));
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
			keep_awake: false,
			join_asked: None,
			data_dir: None,
			rapid_masters: BTreeMap::new(),
			map_searches: BTreeMap::new(),
			bar_maps: BTreeSet::new(),
			bar: BarContent::default(),
			misses,
			vet: vet_bars_only(),
			from_host: no_host(),
			game_sources: rapid_only(),
			engine_run: None,
			overlay_config_dir: None,
			menu_archive: None,
			checked: None,
			greeted_mutator_host: None,
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
			check_tx,
			check_rx,
			content_view: None,
			content_check: lobby_ui::ContentCheckView::default(),
			check_for: None,
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

	/// Starts the engine as the game server of the room we are in.
	///
	/// The same script path a skirmish takes, with one thing first: the room
	/// is told we are in game, because that bit going up is what starts every
	/// guest's engine. It goes out before the spawn, as it does when joining,
	/// and comes back down with the engine's exit either way.
	async fn launch_hosted(
		&mut self,
		dirs: DataDirs,
		engine_version: &str,
		script: &str,
	) -> Result<(), ClientError> {
		if self.engine.is_some() {
			return Err(ClientError::Engine("the engine is already running".into()));
		}
		let Some(server) = self.room() else {
			return Err(ClientError::NotConnected);
		};
		let in_game = self
			.servers
			.get_mut(&server)
			.and_then(|slot| slot.link.as_mut())
			.map(|conn| conn.session.set_in_game(true))
			.ok_or(ClientError::NotConnected)?;
		for effect in in_game {
			if let Effect::Send(envelope) = effect {
				self.send_line(&server, envelope).await?;
			}
		}
		self.in_game_on = Some(server);
		let path = dirs.write.join("modlobby-hosted.txt");
		std::fs::create_dir_all(&dirs.write)
			.and_then(|()| std::fs::write(&path, script))
			.map_err(|err| ClientError::Engine(format!("writing the start script: {err}")))?;
		match launch::spawn(
			&dirs,
			engine_version,
			path.to_string_lossy().into_owned(),
			self.overlay_config_dir.as_deref(),
			self.menu_archive.clone(),
		) {
			Ok(launched) => {
				self.started(launched, dirs.write);
				Ok(())
			}
			Err(reason) => {
				// The bit went up for a game that never started; take it
				// back down, or every guest's engine would try to join it.
				if let Some(server) = self.in_game_on.take()
					&& let Some(conn) = self.link_mut(&server)
				{
					let effects = conn.session.set_in_game(false);
					self.apply_effects(&server, effects).await;
				}
				Err(ClientError::Engine(reason))
			}
		}
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
	///
	/// `by_hand` is a person asking, which a remembered miss never holds back.
	async fn start_download(&mut self, by_hand: bool) -> Result<(), ClientError> {
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
		let mutators = conn
			.session
			.state
			.my_battle
			.as_ref()
			.map(lobby_core::MyBattle::mutators)
			.unwrap_or_default();
		self.fetch(wanted, mutators, self.room(), by_hand).await
	}

	/// Who `map` is asked of, or why nobody is.
	///
	/// A miss is remembered per search, so one that said no today drops out
	/// of the chain and the rest are still asked; only when every one of them
	/// has said no is the fetch held back. Asking by hand forgets them all,
	/// which is what Download is for.
	fn map_searches(
		&mut self,
		server: Option<&str>,
		map: &str,
		by_hand: bool,
	) -> Result<Vec<String>, String> {
		let asked = map_searches_for(
			&self.bar_maps,
			&self.map_searches,
			server,
			map,
			&self.bar.search,
		);
		if by_hand {
			// `count`, not `any`: every one is forgotten, and a short circuit
			// would leave the rest remembered.
			let forgot = asked
				.iter()
				.filter(|search| self.misses.forget(search, map))
				.count();
			if forgot > 0 {
				self.save_misses();
			}
			return Ok(asked);
		}
		let now = latency::unix_now();
		let fresh: Vec<String> = asked
			.into_iter()
			.filter(|search| !self.misses.holds(search, map, now))
			.collect();
		if fresh.is_empty() {
			return Err(format!(
				"{map} was not found earlier today, so it is not looked for again unasked; Download asks again"
			));
		}
		Ok(fresh)
	}

	fn save_misses(&self) {
		if let Some(dir) = self.state_dir.as_deref() {
			self.misses.save(&dir.join(misses::FILE));
		}
	}

	fn rapid_master(&self, server: Option<&str>) -> String {
		master_for(&self.rapid_masters, server, &self.bar.rapid_master)
	}

	/// The same, for the room with no server behind it.
	async fn start_skirmish_download(&mut self) -> Result<(), ClientError> {
		let room = self
			.skirmish
			.as_ref()
			.ok_or_else(|| ClientError::Refused("there is no skirmish room".into()))?;
		let wanted = (room.engine.clone(), room.game.clone(), room.map.clone());
		self.fetch(wanted, Vec::new(), None, true).await
	}

	/// Fetches whatever of an (engine, game, map) and the room's `mutators`
	/// this machine lacks, for a room on `server`: the game and the mutators
	/// through that server's rapid, then the lists; the map from whoever can
	/// have it.
	///
	/// One run at a time: pr-downloader rewrites rapid's repo index on every
	/// run, so two at once corrupt each other's view of it.
	async fn fetch(
		&mut self,
		(engine_version, game, map): (String, String, String),
		mutators: Vec<lobby_core::Mutator>,
		server: Option<String>,
		by_hand: bool,
	) -> Result<(), ClientError> {
		if self.downloading.is_some() {
			return Err(ClientError::Refused("a download is already running".into()));
		}
		let Some(dirs) = self.data_dirs() else {
			return Err(ClientError::Refused("no BAR data directory".into()));
		};
		let library = content::Library::new(dirs.clone());
		let rapid_master = self.rapid_master(server.as_deref());
		let mut wants = Vec::new();
		// Why what is missing is not asked for.
		let mut refused = Vec::new();
		if !library.has_game(&game) {
			// A room that names no game yet -- a first run -- is asking for
			// BAR; its name is adopted once the download has given it one.
			let want = if game.is_empty() {
				recoil::BAR_GAME_TAG.to_owned()
			} else {
				game.clone()
			};
			if recoil::stale_copy(&want) && rapid_master != self.bar.rapid_master {
				refused.push(format!(
					"{want} is only fetched from BAR's rapid: {rapid_master} has an old copy of it with other contents"
				));
			} else {
				wants.push((recoil::Want::Game, want));
			}
		}
		wants.extend(
			mutators
				.iter()
				.filter(|mutator| announced_whole(mutator))
				.filter(|mutator| mutator_archive(&library, &engine_version, mutator).is_none())
				.map(|mutator| (recoil::Want::Mutator, mutator.name.clone())),
		);
		let mut map_searches = vec![self.bar.search.clone()];
		// No map named is no map to ask for; the picker fetches whichever is chosen.
		if !map.is_empty() && !library.has_map(&map) {
			match self.map_searches(server.as_deref(), &map, by_hand) {
				Ok(searches) => {
					map_searches = searches;
					wants.push((recoil::Want::Map, map.clone()));
				}
				Err(reason) => refused.push(reason),
			}
		}
		if wants.is_empty() {
			return Err(ClientError::Refused(if refused.is_empty() {
				"nothing is missing".into()
			} else {
				refused.join("; ")
			}));
		}
		// What can come still comes; the rest's refusals are said beside it.
		for reason in refused {
			self.batcher.push(Delta::Notice {
				level: lobby_ui::NoticeLevel::Warning,
				text: reason,
			});
		}

		let what = wants
			.iter()
			.map(|(_, name)| name.as_str())
			.collect::<Vec<_>>()
			.join(", ");
		let runs = crate::launch::plan_download(
			&dirs,
			&engine_version,
			wants,
			&rapid_master,
			&map_searches,
		)
		.map_err(ClientError::Refused)?;
		let vet = Arc::clone(&self.vet);
		let from_host = Arc::clone(&self.from_host);
		let sources = Arc::clone(&self.game_sources);
		let room = Room {
			dirs: dirs.clone(),
			engine: engine_version.clone(),
			hash: server
				.as_deref()
				.and_then(|at| self.link(at))
				.and_then(|conn| conn.session.state.my_battle.as_ref())
				.and_then(|my| content::checksum::parse_room_hash(&my.game_hash)),
			mutators,
		};

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
		// Gathered before the runs move into the task, for the last resort:
		// who we are in this room, and what the host will know us by.
		let ask = server
			.as_deref()
			.and_then(|at| self.link(at))
			.map(|conn| Ask {
				server: server.clone(),
				map: map.clone(),
				me: conn.session.state.me.clone().unwrap_or_default(),
				script_password: conn
					.session
					.state
					.my_battle
					.as_ref()
					.map(|my| my.script_password.clone())
					.unwrap_or_default(),
			});
		tokio::spawn(async move {
			let progress = events.clone();
			// The runs, one after the other.
			let work = async move {
				// The map's runs are the same map from each search in turn:
				// alternatives, not more work, so the first that answers ends
				// it and only every one of them failing is a failure. A game
				// has one source and must work -- but a game that cannot be
				// had does not cost the map, which comes from elsewhere.
				let mut found_map = false;
				let mut no_map = None;
				let mut no_game = None;
				for run in runs {
					if run.has_games() {
						no_game = fetch_games(&run, &room, &vet, &sources, &progress).await;
						continue;
					}
					if found_map {
						continue;
					}
					match run_download(&run, &progress).await {
						Ok(()) => found_map = true,
						Err(reason) => no_map = Some(reason),
					}
				}
				let map = match no_map {
					// Every search said no. The room's own host is playing
					// the map, so it has the file even where nobody else
					// publishes it -- which is every custom map on a LAN.
					Some(reason) if !found_map => match ask {
						Some(ask) => pumped(&progress, |tx| from_host(ask, tx))
							.await
							.map_err(|from| format!("{reason}; and {from}")),
						None => Err(reason),
					},
					_ => Ok(()),
				};
				fetched(no_game, map)
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
			DownloadEvent::Missed { search, map } => {
				self.misses.remember(&search, &map, latency::unix_now());
				self.save_misses();
			}
			DownloadEvent::Sourced { game, from } => {
				tracing::info!(%game, %from, "game from outside rapid");
				self.batcher.push(Delta::Notice {
					level: lobby_ui::NoticeLevel::Info,
					text: format!("{game}: {from}"),
				});
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
				self.look_again().await;
			}
		}
	}

	/// Asks the disk again for both rooms, because something arrived or
	/// somebody asked.
	///
	/// The room with no server behind it keeps its own answer, keyed on the
	/// same three names and cached for the same reason — and `refresh_content`
	/// returns at once when there is no connection, so without the second half
	/// an engine put in the folder by hand is found only by restarting the app.
	/// A room opened on a machine with nothing on it names nothing, so it is
	/// also given what has arrived since, and that is written down.
	async fn look_again(&mut self) {
		self.checked = None;
		self.refresh_content().await;
		let dirs = self.data_dirs();
		let adopted = match (self.skirmish.as_mut(), dirs) {
			(Some(room), Some(dirs)) => {
				let library = content::Library::new(dirs);
				room.adopt(&library.installed_engines(), &library.installed_games())
			}
			_ => false,
		};
		if adopted {
			self.remember_skirmish();
		}
		self.skirmish_checked = None;
		self.push_skirmish();
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
		let my = conn.session.state.my_battle.as_ref();
		let room = my.and_then(|my| conn.session.state.battles.get(&my.id));
		let game_hash = my.and_then(|my| content::checksum::parse_room_hash(&my.game_hash));
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
			my.map(lobby_core::MyBattle::mutators).unwrap_or_default(),
		);
		if self.checked.as_ref() == Some(&key) {
			return;
		}
		let Some(dirs) = self.data_dirs() else {
			return;
		};
		let library = content::Library::new(dirs.clone());
		let available = library.check(&key.0, &key.1, &key.2);
		let mutators: Vec<(lobby_core::Mutator, bool)> = key
			.3
			.iter()
			.map(|mutator| {
				let here = mutator_archive(&library, &key.0, mutator).is_some();
				(mutator.clone(), here)
			})
			.collect();
		let mutators_here = mutators.iter().all(|(_, here)| *here);
		let mutators_to_fetch = mutators
			.iter()
			.any(|(mutator, here)| !here && announced_whole(mutator));
		let map_hash = content::checksum::parse_room_hash(&room.map_hash);
		self.start_check(
			CheckKey {
				engine: key.0.clone(),
				game: (key.1.clone(), game_hash.filter(|_| available.game)),
				map: (key.2.clone(), map_hash.filter(|_| available.map)),
				mutators,
			},
			dirs,
		);
		self.checked = Some(key.clone());
		self.content_view = Some(ContentView {
			engine: available.engine,
			game: available.game,
			map: available.map,
		});
		self.batcher.push(Delta::Content {
			engine: available.engine,
			game: available.game,
			map: available.map,
		});
		// A mutator missing is a game that cannot start: the engine stops at
		// an archive it cannot find.
		self.content_ready = available.complete() && mutators_here;
		self.set_synced(self.content_ready).await;

		// Joining a room you have no map for is a request for the map: there is
		// nothing else to do in it. Only what pr-downloader can fetch, only
		// when nothing else is running, and only once per room — a name the
		// CDN does not carry would otherwise be retried by every content check
		// that a failed download itself provokes.
		let fetchable = !available.game || !available.map || mutators_to_fetch;
		if fetchable
			&& self.auto_download
			&& available.engine
			&& self.downloading.is_none()
			&& self.auto_fetched.as_ref() != Some(&key)
		{
			self.auto_fetched = Some(key);
			// A room that cannot fetch its own content is stuck until the
			// user does something, so they hear why.
			if let Err(error) = self.start_download(false).await {
				tracing::warn!(%error, "not fetching the room's content");
				self.batcher.push(Delta::Notice {
					level: lobby_ui::NoticeLevel::Warning,
					text: format!("not fetching the room's content: {error}"),
				});
			}
		}
	}

	/// Tells a mutator host, once per room, that this lobby loads what it
	/// announces: the host tells everybody else what the room loads and where
	/// a lobby that loads it is, and holds the start while such a player is
	/// seated. Said to any host whose tags say it runs mutators, whatever it
	/// is called; the tags are the protocol, not a client name.
	async fn greet_mutator_host(&mut self) {
		let room = self.room();
		let greet = room.as_deref().and_then(|server| {
			let state = &self.link(server)?.session.state;
			let my = state.my_battle.as_ref()?;
			if !my.hosts_mutators() || self.greeted_mutator_host == Some((server.to_owned(), my.id))
			{
				return None;
			}
			let founder = state.battles.get(&my.id)?.founder.clone();
			Some((server.to_owned(), my.id, founder))
		});
		let Some((server, id, founder)) = greet else {
			return;
		};
		self.greeted_mutator_host = Some((server.clone(), id));
		let effects = self
			.link_mut(&server)
			.and_then(|conn| {
				conn.session
					.say_private(&founder, lobby_core::MUTATORS_SUPPORTED)
					.ok()
			})
			.unwrap_or_default();
		self.apply_effects(&server, effects).await;
	}

	/// Holds the room's game and map, where they are here, to the hashes the
	/// room announced -- off the actor, since reading a game takes seconds;
	/// the answer comes back as [`Next::Checked`].
	fn start_check(&mut self, key: CheckKey, dirs: DataDirs) {
		let checking = |hash: Option<u32>| hash.map(|_| lobby_ui::CheckView::Checking);
		let view = lobby_ui::ContentCheckView {
			game: checking(key.game.1),
			map: checking(key.map.1),
			mutators: key
				.mutators
				.iter()
				.map(|(mutator, here)| {
					let checking = (*here && mutator.checksum.is_some())
						.then_some(lobby_ui::CheckView::Checking);
					mutator_view(mutator, *here, None, checking)
				})
				.collect(),
		};
		let nothing_to_hold = view.game.is_none()
			&& view.map.is_none()
			&& view.mutators.iter().all(|mutator| mutator.check.is_none());
		self.content_check = view.clone();
		self.batcher.push(Delta::ContentCheck(view));
		if nothing_to_hold {
			self.check_for = None;
			return;
		}
		self.check_for = Some(key.clone());
		let answer = self.check_tx.clone();
		tokio::task::spawn_blocking(move || {
			let view = check_content(dirs, &key);
			let _ = answer.blocking_send(Checked { key, view });
		});
	}

	/// A content check's answer. A game or map that is not the room's is one
	/// the game would desync on -- the engine only warns
	/// (`PreGame.cpp:636-646`) -- so the room hears we are not synced, which
	/// keeps its host from starting with us, as SpringLobby's hash check did.
	/// Held to BAR's own rooms on 2026-09-22: rapid games and maps matched.
	async fn on_checked(&mut self, checked: Checked) {
		if self.check_for.as_ref() != Some(&checked.key) {
			return;
		}
		let mut unsynced = false;
		let parts = [
			(&checked.key.game.0, &checked.view.game),
			(&checked.key.map.0, &checked.view.map),
		]
		.into_iter()
		.chain(
			checked
				.view
				.mutators
				.iter()
				.map(|mutator| (&mutator.name, &mutator.check)),
		);
		for (name, view) in parts {
			match view {
				Some(lobby_ui::CheckView::Differs { ours, room }) => {
					tracing::warn!(%name, ours, room, "not the room's files");
					unsynced = true;
					self.batcher.push(Delta::Notice {
						level: lobby_ui::NoticeLevel::Warning,
						text: format!(
							"{name} here is not the room's (checksum {ours}, the room's {room}); the room is told you are not synced"
						),
					});
				}
				Some(lobby_ui::CheckView::Unchecked { why }) => {
					tracing::info!(%name, %why, "not checked against the room");
				}
				_ => {}
			}
		}
		if unsynced {
			self.content_ready = false;
			self.set_synced(false).await;
		}
		self.content_check = checked.view.clone();
		self.batcher.push(Delta::ContentCheck(checked.view));
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
				Some(checked) = self.check_rx.recv() => Next::Checked(checked),
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
				Next::Checked(checked) => self.on_checked(checked).await,
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
					self.greet_mutator_host().await;
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
			Command::LaunchHosted {
				dirs,
				engine_version,
				script,
				reply,
			} => {
				let result = self.launch_hosted(dirs, &engine_version, &script).await;
				let _ = reply.send(result);
			}
			Command::KeepAwake(on) => self.keep_awake = on,
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
				let result = self.start_download(true).await;
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
			Command::SetPreReady { on, reply } => {
				self.run_room(reply, |session| session.set_pre_ready(on))
					.await;
			}
			Command::SetSide { side, reply } => {
				self.run_room(reply, |session| session.set_side(side)).await;
			}
			Command::TakeSeat {
				team,
				ally_team,
				ready,
				reply,
			} => {
				self.run_room(reply, |session| session.take_seat(team, ally_team, ready))
					.await;
			}
			Command::SetOverlayConfigDir(dir) => self.overlay_config_dir = dir,
			Command::SetMenuArchive(menu) => self.menu_archive = menu,
			Command::SetSkirmishPath(path) => self.skirmish_path = path,
			Command::SetRapidMasters(masters) => self.rapid_masters = masters,
			Command::SetMapSearches(searches) => self.map_searches = searches,
			Command::SetBarMaps(names) => self.bar_maps = names,
			Command::SetBarContent(bar) => self.bar = bar,
			Command::SetVet(vet) => self.vet = vet,
			Command::SetFromHost(from_host) => self.from_host = from_host,
			Command::SetGameSources(sources) => self.game_sources = sources,
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
			Command::RecheckContent => self.look_again().await,
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
				} => {
					tracing::debug!(server, ?area, pending, ?wait, "throttled");
					if area == Area::BattleStatus {
						let until = unix_ms_after(wait);
						let effects: Vec<Effect> = self
							.link_mut(server)
							.and_then(|conn| conn.session.status_held(until))
							.into_iter()
							.collect();
						self.apply_effects(server, effects).await;
					}
				}
				PolicyEvent::Sent { area, lines, .. } => {
					self.paste_sent(area, lines);
					if area == Area::BattleStatus {
						let effects: Vec<Effect> = self
							.link_mut(server)
							.and_then(|conn| conn.session.status_sent())
							.into_iter()
							.collect();
						self.apply_effects(server, effects).await;
					}
				}
				// The status it replaced never leaves, so no answer to it comes.
				PolicyEvent::Coalesced {
					area: Area::BattleStatus,
					..
				} => {
					tracing::info!(server, "policy: a status replaced one still waiting");
					if let Some(conn) = self.link_mut(server) {
						conn.session.status_replaced();
					}
				}
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
							let _ = reply.send(Err(ClientError::Unverified(text)));
						}
					}
				}
				Effect::Registered => {
					// The account exists but is unverified, and only a login
					// binds the emailed code to a connection: the agreement
					// that login is answered with is what the caller waits
					// for. It goes out on a connection of its own. uberserver
					// has put this one in its agreement state, where `LOGIN`
					// is refused (`Insufficient rights`) and `CONFIRMAGREEMENT`
					// has no account to confirm; teiserver answers an
					// unverified login with the agreement on any connection
					// (`spring_in.ex:292`).
					let slot = self.servers.entry(server.to_owned()).or_default();
					let reply = slot.register_reply.take();
					let credentials = slot.credentials.clone();
					self.disconnect(server).await;
					if let (Some(reply), Some((endpoint, request))) = (reply, credentials) {
						self.connect(endpoint, request, Purpose::Agreement(reply));
					}
					return;
				}
				Effect::RegistrationDenied { reason } => {
					let slot = self.servers.entry(server.to_owned()).or_default();
					if let Some(reply) = slot.register_reply.take() {
						let refused = if spring_protocol::login::name_taken(&reason) {
							ClientError::NameTaken(reason)
						} else {
							ClientError::Refused(reason)
						};
						let _ = reply.send(Err(refused));
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
						view: GameRunningView {
							id,
							ip,
							port,
							added: just_started,
						},
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
					let may_join = self
						.game
						.as_ref()
						.is_some_and(|game| recoil::may_join_hosted_game_at(&game.view.ip));
					let wanted = self.auto_launch.take().or_else(|| {
						(just_started
							&& self.auto_launch_always
							&& self.content_ready && self.engine.is_none()
							&& may_join)
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
				| Effect::RoomChanged
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
		if self.game.is_some() || self.engine.is_some() || self.keep_awake {
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
			// starts and cannot join -- or one that stops at a mutator it
			// cannot find.
			let library = content::Library::new(dirs.clone());
			let available = library.check(&room.engine_version, &room.game_name, &room.map_name);
			let mut missing: Vec<String> =
				available.missing().into_iter().map(str::to_owned).collect();
			missing.extend(
				state
					.my_battle
					.iter()
					.flat_map(lobby_core::MyBattle::mutators)
					.filter(|mutator| {
						mutator_archive(&library, &room.engine_version, mutator).is_none()
					})
					.map(|mutator| {
						let named = mutator
							.source
							.as_ref()
							.map_or(&mutator.name, |pin| &pin.repo);
						format!("the mod {named}")
					}),
			);
			if !missing.is_empty() {
				return Err(ClientError::Engine(format!(
					"this room needs content you do not have: {}",
					missing.join(", ")
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
		if let Some(game) = self.game.as_mut().filter(|game| !game.view.added) {
			game.view.added = true;
			self.batcher
				.push_for(&server, Delta::GameRunning(Some(game.view.clone())));
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
		snapshot.content = self.content_view;
		snapshot.content_check = self.content_check.clone();
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

	/// What the 2026.09.01 engine's pr-downloader printed to stderr for a game
	/// springrts' rapid does not publish, joining a SplinterFaction room on
	/// lobby.recoilengine.org (2026-09-22).
	#[test]
	fn pr_downloaders_own_lines_are_kept_and_curls_tracing_is_not() {
		let stderr = [
			"*   Trying 127.0.0.1:1...",
			"* connect to 127.0.0.1 port 1 failed: Connection refused",
			"[Error] /build/src/tools/pr-downloader/src/Downloader/Http/HttpDownloader.cpp:220:search():Error downloading http://127.0.0.1:1/nobody-is-asked?category=game&springname=SplinterFaction%200.1.86",
		];
		let kept: Vec<&str> = stderr
			.into_iter()
			.filter(|line| pr_downloader_said(line))
			.collect();
		assert_eq!(kept.len(), 1);

		let run = recoil::Download {
			binary: "prd".into(),
			data_dir: "data".into(),
			wants: vec![(recoil::Want::Game, "SplinterFaction 0.1.86".into())],
			rapid_master: "https://repos.springrts.com/repos.gz".into(),
			search_url: recoil::NO_SEARCH_URL.into(),
		};
		let tail: Vec<String> = kept.into_iter().map(str::to_owned).collect();
		assert_eq!(
			unpublished(&run, &tail).as_deref(),
			Some(
				"SplinterFaction 0.1.86 is not published by this server's rapid server (https://repos.springrts.com/repos.gz)"
			)
		);
	}

	#[test]
	fn the_rooms_game_is_held_to_its_hash_wherever_it_is_installed() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path().join("games").join("Game.sdd");
		std::fs::create_dir_all(&game).unwrap();
		// modtype 4: no implicit "Spring content v1" to find, so no engine is needed.
		std::fs::write(
			game.join("modinfo.lua"),
			"name = 'Game'\nversion = '1'\nmodtype = 4\n",
		)
		.unwrap();
		let dirs = || DataDirs::only(dir.path());
		let ours = content::checksum::room_hash(&content::checksum::single(&game).unwrap());

		let key = |game: &str, hash: Option<u32>| CheckKey {
			engine: "2026.09.01".into(),
			game: (game.into(), hash),
			map: ("Nowhere 1".into(), Some(7)),
			mutators: Vec::new(),
		};
		use lobby_ui::{CheckView, ContentCheckView};
		assert_eq!(
			check_content(dirs(), &key("Game 1", Some(ours))),
			ContentCheckView {
				game: Some(CheckView::Same { hash: ours }),
				map: None,
				mutators: Vec::new(),
			}
		);
		assert_eq!(
			check_content(dirs(), &key("Game 1", Some(ours ^ 1))).game,
			Some(CheckView::Differs {
				ours,
				room: ours ^ 1
			})
		);
		assert_eq!(check_content(dirs(), &key("Game 1", None)).game, None);
		assert_eq!(
			check_content(dirs(), &key("Absent 2", Some(ours))),
			ContentCheckView::default()
		);
	}

	/// A mutator's copy here is held to the checksum its host announced, its
	/// own files only; one that is not here is said to be missing, unchecked.
	#[test]
	fn a_mutator_is_held_to_the_checksum_its_host_announced() {
		let dir = tempfile::tempdir().unwrap();
		let sphere = dir.path().join("games").join("sphere.sdd");
		std::fs::create_dir_all(&sphere).unwrap();
		std::fs::write(
			sphere.join("modinfo.lua"),
			"name = 'Sphere'\nversion = 'v1'\nmutator = '1'\nmodtype = 1\n",
		)
		.unwrap();
		let single = content::checksum::single(&sphere).unwrap();
		let hex = |sum: &[u8]| sum.iter().map(|b| format!("{b:02x}")).collect::<String>();
		let ours = content::checksum::room_hash(&single);
		let mut other = single;
		other[0] ^= 1;

		let mutator = |name: &str, checksum: &str| lobby_core::Mutator {
			name: name.into(),
			checksum: Some(checksum.into()),
			source: None,
			date: None,
		};
		let key = CheckKey {
			engine: "2026.09.01".into(),
			game: ("Game 1".into(), None),
			map: ("Nowhere 1".into(), None),
			mutators: vec![
				(mutator("Sphere v1", &hex(&single)), true),
				(mutator("Sphere v1", &hex(&other)), true),
				(mutator("Absent v1", &hex(&single)), false),
			],
		};
		use lobby_ui::{CheckView, MutatorView};
		assert_eq!(
			check_content(DataDirs::only(dir.path()), &key).mutators,
			[
				MutatorView {
					name: "Sphere v1".into(),
					title: "Sphere v1".into(),
					description: None,
					here: true,
					check: Some(CheckView::Same { hash: ours }),
					source: None,
					date: None,
				},
				MutatorView {
					name: "Sphere v1".into(),
					title: "Sphere v1".into(),
					description: None,
					here: true,
					check: Some(CheckView::Differs {
						ours,
						room: ours ^ 1
					}),
					source: None,
					date: None,
				},
				MutatorView {
					name: "Absent v1".into(),
					title: "Absent v1".into(),
					description: None,
					here: false,
					check: None,
					source: None,
					date: None,
				},
			]
		);
	}

	/// A mutator pinned to a commit is its build of that commit, found by the
	/// file name the room loads it by: another copy of the same mod, with the
	/// same name inside it, does not pass for it.
	#[test]
	fn a_mutator_half_announced_is_not_asked_for() {
		let sha = "9108a17078f79d09925edc305ec83bc06c3a7cb3";
		let pin = |repo: &str| lobby_core::GitPin {
			repo: repo.into(),
			commit: sha.into(),
		};
		let mutator = |name: String, source| lobby_core::Mutator {
			name,
			checksum: None,
			source,
			date: None,
		};
		let built = content::git::build_name("dev/sphere", sha);
		assert!(announced_whole(&mutator(
			built.clone(),
			Some(pin("dev/sphere"))
		)));
		assert!(announced_whole(&mutator("Tiny maps v1".into(), None)));
		assert!(
			!announced_whole(&mutator(built.clone(), None)),
			"no commit yet"
		);
		assert!(
			!announced_whole(&mutator(built, Some(pin("dev/tanks")))),
			"its neighbour's commit"
		);
	}

	#[test]
	fn a_pinned_mutator_is_its_build_and_no_other_copy() {
		let sha = "9108a17078f79d09925edc305ec83bc06c3a7cb3";
		let pin = lobby_core::GitPin {
			repo: "dev/sphere".into(),
			commit: sha.into(),
		};
		let built = content::git::build_name(&pin.repo, sha);
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		let modinfo = "name = 'Sphere'\nversion = 'v1'\nmutator = '1'\n";
		std::fs::create_dir_all(games.join("sphere.sdd")).unwrap();
		std::fs::write(games.join("sphere.sdd").join("modinfo.lua"), modinfo).unwrap();
		let mutator = lobby_core::Mutator {
			name: built.clone(),
			checksum: None,
			source: Some(pin),
			date: Some("2025-10-05T13:32:09Z".into()),
		};
		let library = content::Library::new(DataDirs::only(dir.path()));
		assert_eq!(mutator_archive(&library, "2026.09.01", &mutator), None);

		std::fs::create_dir_all(games.join(&built)).unwrap();
		std::fs::write(games.join(&built).join("modinfo.lua"), modinfo).unwrap();
		assert_eq!(
			mutator_archive(&library, "2026.09.01", &mutator),
			Some(games.join(&built))
		);
		let key = CheckKey {
			engine: "2026.09.01".into(),
			game: ("Game 1".into(), None),
			map: ("Nowhere 1".into(), None),
			mutators: vec![(mutator, true)],
		};
		assert_eq!(
			check_content(DataDirs::only(dir.path()), &key).mutators,
			[lobby_ui::MutatorView {
				name: built,
				title: "Sphere v1".into(),
				description: None,
				here: true,
				check: None,
				source: Some(format!("github:dev/sphere@{sha}")),
				date: Some("2025-10-05T13:32:09Z".into()),
			}]
		);
	}

	fn room() -> Room {
		Room {
			dirs: DataDirs::only("data"),
			engine: "2026.09.01".into(),
			hash: Some(1_521_219_441),
			mutators: Vec::new(),
		}
	}

	fn game_run() -> recoil::Download {
		recoil::Download {
			binary: "no-such-pr-downloader".into(),
			data_dir: "data".into(),
			wants: vec![(recoil::Want::Game, "SplinterFaction 0.1.86".into())],
			rapid_master: "https://repos.springrts.com/repos.gz".into(),
			search_url: recoil::NO_SEARCH_URL.into(),
		}
	}

	/// Sources that answer `before` ahead of rapid and `after` behind it,
	/// and note what they were asked.
	fn sources(
		before: Option<Result<String, String>>,
		after: Option<Result<String, String>>,
		asked: Arc<std::sync::Mutex<Vec<GameAsk>>>,
	) -> GameSources {
		Arc::new(move |ask: GameAsk, _, _| {
			let answer = if ask.after_rapid {
				after.clone()
			} else {
				before.clone()
			};
			asked.lock().unwrap().push(ask);
			Box::pin(async move { answer })
		})
	}

	/// A rapid server that is refused before pr-downloader would run.
	fn refused_rapid(tried: Arc<std::sync::atomic::AtomicBool>) -> Vet {
		Arc::new(move |_| {
			tried.store(true, std::sync::atomic::Ordering::Relaxed);
			Box::pin(async { Err("rapid said no".to_owned()) })
		})
	}

	#[tokio::test]
	async fn a_lists_rapid_entry_is_run_against_its_master_and_vetted_as_the_rooms_is() {
		let (tx, _events) = mpsc::channel(8);
		let vetted = Arc::new(std::sync::Mutex::new(Vec::new()));
		let vet: Vet = {
			let vetted = Arc::clone(&vetted);
			Arc::new(move |master| {
				vetted.lock().unwrap().push(master);
				Box::pin(async { Err("not that one".to_owned()) })
			})
		};
		let elsewhere: GameSources = Arc::new(|ask: GameAsk, _, rapid| {
			Box::pin(async move {
				ask.after_rapid.then_some(())?;
				Some(
					rapid("https://elsewhere/repos.gz".into())
						.await
						.map(|()| "rapid".into()),
				)
			})
		});
		let failure = fetch_game(&game_run(), &room(), &vet, &elsewhere, &tx).await;
		assert_eq!(failure, Some("not that one; not that one".into()));
		assert_eq!(
			*vetted.lock().unwrap(),
			[
				"https://repos.springrts.com/repos.gz",
				"https://elsewhere/repos.gz"
			]
		);
	}

	#[tokio::test]
	async fn an_override_replaces_rapid_rather_than_following_it() {
		let (tx, mut events) = mpsc::channel(8);
		let asked = Arc::default();
		let tried = Arc::default();
		let found = sources(Some(Ok("github fork/SF".into())), None, Arc::clone(&asked));
		let failure = fetch_game(
			&game_run(),
			&room(),
			&refused_rapid(Arc::clone(&tried)),
			&found,
			&tx,
		)
		.await;
		assert_eq!(failure, None);
		assert!(
			!tried.load(std::sync::atomic::Ordering::Relaxed),
			"rapid never asked"
		);
		assert_eq!(asked.lock().unwrap().len(), 1);
		assert!(matches!(
			events.try_recv(),
			Ok(DownloadEvent::Sourced { from, .. }) if from == "github fork/SF"
		));
	}

	#[tokio::test]
	async fn the_lists_are_asked_once_rapid_fails_and_say_why_if_they_fail_too() {
		let (tx, _events) = mpsc::channel(8);
		let tried = Arc::default();

		let asked = Arc::default();
		let found = sources(None, Some(Ok("github SF/SF".into())), Arc::clone(&asked));
		assert_eq!(
			fetch_game(
				&game_run(),
				&room(),
				&refused_rapid(Arc::clone(&tried)),
				&found,
				&tx
			)
			.await,
			None
		);
		let asked: Vec<bool> = asked
			.lock()
			.unwrap()
			.iter()
			.map(|ask| ask.after_rapid)
			.collect();
		assert_eq!(asked, [false, true]);

		let none = sources(
			None,
			Some(Err("no release has 0.1.86".into())),
			Arc::default(),
		);
		assert_eq!(
			fetch_game(
				&game_run(),
				&room(),
				&refused_rapid(Arc::clone(&tried)),
				&none,
				&tx
			)
			.await,
			Some("rapid said no; no release has 0.1.86".into())
		);
		assert_eq!(
			fetch_game(
				&game_run(),
				&room(),
				&refused_rapid(tried),
				&rapid_only(),
				&tx
			)
			.await,
			Some("rapid said no".into())
		);
	}

	#[test]
	fn a_game_that_cannot_be_had_is_told_with_what_became_of_the_map() {
		assert_eq!(fetched(None, Ok(())), Ok(()));
		assert_eq!(fetched(None, Err("no map".into())), Err("no map".into()));
		assert_eq!(
			fetched(Some("no game".into()), Ok(())),
			Err("no game".into())
		);
		assert_eq!(
			fetched(Some("no game".into()), Err("no map".into())),
			Err("no game; no map".into())
		);
	}

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
			master_for(&masters, Some("mods.example"), recoil::RAPID_REPO_MASTER),
			"https://mods.example/repos.gz"
		);
		assert_eq!(
			master_for(
				&masters,
				Some("server4.beyondallreason.info"),
				recoil::RAPID_REPO_MASTER
			),
			recoil::RAPID_REPO_MASTER
		);
		assert_eq!(
			master_for(&masters, None, recoil::RAPID_REPO_MASTER),
			recoil::RAPID_REPO_MASTER
		);
	}

	#[test]
	fn a_map_is_asked_only_of_whoever_can_have_it() {
		let bars = BTreeSet::from(["Supreme Isthmus v2.1".to_owned()]);
		let theirs = BTreeMap::from([(
			"mods.example".to_owned(),
			"https://mods.example/find".to_owned(),
		)]);
		let ask = |bars: &BTreeSet<String>, server, map| {
			map_searches_for(bars, &theirs, server, map, recoil::HTTP_SEARCH_URL)
		};
		let mods = Some("mods.example");
		let bar = recoil::HTTP_SEARCH_URL;
		let files = recoil::SPRINGFILES_SEARCH_URL;

		// One of BAR's names goes to BAR alone. Nowhere else is asked, so
		// nowhere else can answer with its own map under that name.
		assert_eq!(ask(&bars, mods, "Supreme Isthmus v2.1"), [bar]);
		assert_eq!(ask(&bars, None, "Supreme Isthmus v2.1"), [bar]);

		// A name that is not BAR's: the room's own server first, since it is
		// the one that published the room, then the public index.
		assert_eq!(
			ask(&bars, mods, "Bathtub Brawl V2"),
			["https://mods.example/find", files]
		);
		// BAR's own server names no map search, so only the fallback is left
		// -- and BAR is still not asked for a map it does not have.
		assert_eq!(
			ask(
				&bars,
				Some("server4.beyondallreason.info"),
				"Bathtub Brawl V2"
			),
			[files]
		);
		// A room on the LAN, whose server publishes nothing at all: the
		// refusal this replaced is what a custom map used to get.
		assert_eq!(ask(&bars, Some("lan"), "Frosty Cove v1.13"), [files]);

		// BAR's maps not known yet: every name is unknown, which is not the
		// same as known not to be BAR's, so the fallback stays out of it.
		let unknown = BTreeSet::new();
		assert_eq!(
			ask(&unknown, mods, "Anything"),
			["https://mods.example/find"]
		);
		assert_eq!(ask(&unknown, None, "Anything"), [bar]);

		// A search that is not https is not asked; the fallback still is.
		let plain = BTreeMap::from([(
			"mods.example".to_owned(),
			"http://mods.example/find".to_owned(),
		)]);
		assert_eq!(
			map_searches_for(
				&bars,
				&plain,
				mods,
				"Bathtub Brawl V2",
				recoil::HTTP_SEARCH_URL
			),
			[files]
		);

		// A server naming springfiles itself is not asked twice.
		let same = BTreeMap::from([("mods.example".to_owned(), files.to_owned())]);
		assert_eq!(
			map_searches_for(
				&bars,
				&same,
				mods,
				"Bathtub Brawl V2",
				recoil::HTTP_SEARCH_URL
			),
			[files]
		);
	}

	#[test]
	fn only_a_404_from_a_map_search_is_a_miss() {
		let runs = |want| {
			recoil::Download::runs(
				std::path::Path::new("prd"),
				std::path::Path::new("data"),
				vec![(want, "Nowhere v1".into())],
				recoil::RAPID_REPO_MASTER,
				&["https://mods.example/find".to_owned()],
			)
		};
		let not_found =
			["DownloadUrl():Error in curl (The requested URL returned error: 404)".to_owned()];
		assert_eq!(
			missed(&runs(recoil::Want::Map)[0], &not_found).as_deref(),
			Some("Nowhere v1")
		);
		assert_eq!(
			missed(
				&runs(recoil::Want::Map)[0],
				&["Could not resolve host".into()]
			),
			None
		);
		assert_eq!(missed(&runs(recoil::Want::Game)[0], &not_found), None);
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
			&[recoil::HTTP_SEARCH_URL.to_owned()],
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
		pin: None,
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

	/// Plays the server through a registration: the greeting and the account
	/// made on the first connection, which is then let go; on the second, the
	/// agreement the first login is answered with. The trailing empty
	/// `AGREEMENT ` is teiserver's own (`spring_out.ex:111-121`).
	async fn accept_registration(made_on: DuplexStream, logged_in_on: DuplexStream) -> FakeServer {
		let (read, mut write) = tokio::io::split(made_on);
		let mut lines = BufReader::new(read).lines();
		write.write_all(b"TASSERVER 0.38 * 8201 0\n").await.unwrap();
		let sent = lines.next_line().await.unwrap().unwrap();
		assert!(sent.starts_with("REGISTER me "), "{sent}");
		write.write_all(b"REGISTRATIONACCEPTED\n").await.unwrap();
		// Nothing more on this one: uberserver would refuse a login here.
		assert_eq!(lines.next_line().await.unwrap(), None, "hung up");
		agree(logged_in_on).await
	}

	/// The server answering an unverified account's login with its agreement.
	async fn agree(server: DuplexStream) -> FakeServer {
		let (read, mut write) = tokio::io::split(server);
		let mut lines = BufReader::new(read).lines();
		write.write_all(b"TASSERVER 0.38 * 8201 0\n").await.unwrap();
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
	async fn registering_logs_in_on_a_fresh_connection_and_answers_with_the_agreement() {
		let (connector, mut servers) = in_memory_many(2);
		let client = spawn(connector);
		let made = registering(&client);
		let _server = accept_registration(servers.remove(0), servers.remove(0)).await;

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
		let (connector, mut servers) = in_memory_many(2);
		let client = spawn(connector);
		let made = registering(&client);
		let (mut lines, mut write) =
			accept_registration(servers.remove(0), servers.remove(0)).await;
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
		let (connector, mut servers) = in_memory_many(2);
		let client = spawn(connector);
		let made = registering(&client);
		let (mut lines, mut write) =
			accept_registration(servers.remove(0), servers.remove(0)).await;
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
			matches!(&answer, Err(ClientError::NameTaken(r)) if r == "Username already taken"),
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
	async fn an_unconfirmed_account_logging_in_is_answered_with_the_agreement_and_kept_on() {
		let (connector, mut servers) = in_memory_many(1);
		let client = spawn(connector);
		let login = tokio::spawn({
			let client = client.clone();
			async move {
				let request = LoginRequest::new("me", "pw", "test", "h h");
				client.login(Endpoint::new("test"), request).await
			}
		});
		let (mut lines, _write) = agree(servers.remove(0)).await;

		let answer = login.await.unwrap();
		let Err(unverified @ ClientError::Unverified(_)) = answer else {
			panic!("{answer:?}");
		};
		// The server's words, blank lines left out, are what the person reads.
		assert_eq!(
			unverified.to_string(),
			"Read the terms at https://example/privacy"
		);
		// Still connected, for the code.
		let _confirming = tokio::spawn({
			let client = client.clone();
			async move { client.confirm_agreement("test".into(), "9953".into()).await }
		});
		assert_eq!(
			lines.next_line().await.unwrap().unwrap(),
			"CONFIRMAGREEMENT 9953"
		);
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

	/// Hosting a room's game: the in-game bit goes out before the engine is
	/// started, since it is what starts every guest's engine — and comes
	/// back down when ours could not start, or they would all be joining
	/// nothing.
	#[tokio::test]
	async fn a_hosted_launch_says_in_game_first_and_takes_it_back_on_failure() {
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
		let login = log_in(&client, "test");
		server_write
			.write_all(b"TASSERVER 0.38 * 8201 0\n")
			.await
			.unwrap();
		line_starting_with(&mut server_lines, "LOGIN ").await;
		server_write
			.write_all(b"ACCEPTED me\nADDUSER me SE 1 LuaLobby Chobby\n")
			.await
			.unwrap();
		server_write.write_all(ROOM).await.unwrap();
		server_write.write_all(b"LOGININFOEND\n").await.unwrap();
		login.await.unwrap().unwrap();

		let join = tokio::spawn({
			let client = client.clone();
			async move { client.join_battle("test".into(), 5, None).await }
		});
		line_starting_with(&mut server_lines, "JOINBATTLE ").await;
		server_write
			.write_all(b"JOINBATTLE 5 h\nJOINEDBATTLE 5 me\nREQUESTBATTLESTATUS\n")
			.await
			.unwrap();
		join.await.unwrap().unwrap();
		line_starting_with(&mut server_lines, "MYBATTLESTATUS ").await;

		let dir = tempfile::tempdir().unwrap();
		let result = client
			.launch_hosted(
				DataDirs::only(dir.path()),
				"0.0.0".into(),
				"[game] {}\n".into(),
			)
			.await;
		assert!(matches!(result, Err(ClientError::Engine(_))), "{result:?}");
		assert_eq!(
			line_starting_with(&mut server_lines, "MYSTATUS ").await,
			"MYSTATUS 1"
		);
		assert_eq!(
			line_starting_with(&mut server_lines, "MYSTATUS ").await,
			"MYSTATUS 0"
		);
		assert_eq!(
			std::fs::read_to_string(dir.path().join("modlobby-hosted.txt")).unwrap(),
			"[game] {}\n"
		);
		client.shutdown().await;
	}
}
