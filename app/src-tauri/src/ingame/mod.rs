//! Escape, in the game, meaning what it means everywhere else.
//!
//! The engine gives a lobby no way in from the outside: a global hotkey works,
//! but Escape belongs to the game, and grabbing it system-wide would take it
//! away from every other program too. The only thing that can see Escape
//! *inside* a game is a widget, so there is one, whose entire job is to notice
//! Escape pressed with nothing selected and tell us.
//!
//! It deliberately draws nothing. The obvious design has the widget paint its
//! own little menu in Lua; this one just raises the lobby, which already is the
//! menu, in HTML, with the room and the chat still in it. Fewer moving parts
//! and a much better-looking result.
//!
//! It can only see it once the game has the keyboard. Until then the lobby
//! listens by itself: see [`crate::overlay::escape`].
//!
//! Telling us is the hard part, and there are three ways, tried in order:
//!
//! 1. The control socket: loopback, an ephemeral port, a token generated per
//!    run and baked into the Lua we write. It answers, so the widget consumes
//!    the key only when the lobby actually came up. BAR took the socket away
//!    from user widgets (#9547), so in a BAR of today only our menu has it.
//! 2. Our LuaMenu ([`menu`]), which BAR's own Lobby button already asks: the
//!    widget sends it the same message, and it asks us over its socket.
//! 3. A line appended to a file under `LuaUI/Config`, one of the two places
//!    BAR lets a user widget write, which [`watch_file`] polls. Inside the
//!    mods sandbox there is no socket at all and this is the only way.
//!
//! Two properties make all of it safe to install into a directory we do not
//! own:
//!
//! - It is inert unless modlobby is listening. No answer over the port, and a
//!   file nobody empties, both mean the key is *not* consumed, so a Chobby game
//!   behaves exactly as it always did.
//! - It is removed when modlobby exits cleanly, and rewritten on each launch,
//!   so a stale one from an old version never lingers.
//!
//! Any local process can reach a loopback port, and a mod can write the file;
//! both can quit a game, so being reachable is not the same as being allowed.
//! Each carries a token of its own: a mod reads the file's from the widget we
//! write beside it, and must never learn the socket's.

pub mod menu;
pub mod protocol;
pub mod widget;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use lobby_runtime::launch::LaunchFiles;
pub use protocol::{Command, parse};

/// What the in-game widget can ask for.
///
/// Each answers whether it acted. That answer is the whole reason the widget
/// waits for a reply rather than firing and forgetting: it consumes Escape
/// only when the key actually did something, so a game started from another
/// lobby while modlobby happens to be open keeps its own Escape.
pub trait Actions: Send + Sync + 'static {
	/// Raise the lobby over the game. `false` if this is not our game.
	fn raise(&self) -> bool;
	/// Stop the game, leaving the lobby up. `false` if there was none.
	fn quit_game(&self) -> bool;
	/// The game has the keyboard and judges Escape for itself from here.
	/// `false` if this is not our game.
	fn game_loaded(&self) -> bool;
	/// Whether a game of ours is running, the only time the file gets asks.
	fn game_running(&self) -> bool;
}

/// A running control socket, and where a launch of ours writes its two files.
pub struct InGame {
	pub port: u16,
	pub token: String,
	/// Guards the file channel, which a game in the sandbox can read, so it
	/// is not the socket's. Also how this run knows its files from those of
	/// another lobby sharing the data directory.
	pub file_token: String,
	installed: Option<PathBuf>,
	/// The LuaMenu archive, when one is wanted.
	menu: Option<PathBuf>,
}

impl InGame {
	/// Binds loopback and starts listening. The port is chosen by the OS.
	pub async fn start(actions: Arc<dyn Actions>) -> std::io::Result<Self> {
		let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
		let port = listener.local_addr()?.port();
		let file_token = token();
		let token = token();

		let guard = token.clone();
		tokio::spawn(async move {
			loop {
				let Ok((stream, _)) = listener.accept().await else {
					// The listener is gone; nothing will arrive again.
					return;
				};
				let actions = actions.clone();
				let guard = guard.clone();
				tokio::spawn(async move { serve(stream, guard, actions).await });
			}
		});

		Ok(Self {
			port,
			token,
			file_token,
			installed: None,
			menu: None,
		})
	}

	/// What a launch writes ahead of the engine, for this run's port and tokens,
	/// and noted as ours to take back from `data_dir` on the way down.
	///
	/// Written by the launch and not here, because `data_dir` may be shared with
	/// another lobby: it is the lobby that launched a game the game has to ask,
	/// and starting up beside a game must not change who that is.
	pub fn launch_files(
		&mut self,
		data_dir: &Path,
		widget_wanted: bool,
		menu_wanted: bool,
	) -> LaunchFiles {
		let socket = Some((self.port, self.token.as_str()));
		let relative = Path::new("");
		let mut files = LaunchFiles::default();
		if widget_wanted {
			self.installed = Some(widget::path(data_dir));
			files.open.push((
				widget::path(relative),
				widget::source(socket, &self.file_token),
			));
			files.mods.push((
				widget::path(relative),
				widget::source(None, &self.file_token),
			));
		}
		if widget_wanted || menu_wanted {
			// Empty: Lua's `io.open` will not make the directory the first
			// append needs.
			let file = (PathBuf::from(FILE), String::new());
			files.open.push(file.clone());
			files.mods.push(file);
		}
		if menu_wanted {
			self.menu = Some(menu::dir(data_dir));
			files.open.extend(menu::files(socket, &self.file_token));
			files.confined.extend(menu::files(None, &self.file_token));
		}
		files
	}

	/// Forgets both files without removing them.
	///
	/// For an exit while a game we launched is still running. The engine was
	/// started against the menu archive and asks for it again the moment the
	/// player quits to the menu, so taking it away is what turns that quit
	/// into a content error. Left in place, both are inert -- the widget
	/// leaves Escape alone when nobody answers, and the menu quits the engine
	/// on activation -- and the next start rewrites them anyway.
	pub fn leave_behind(&mut self) {
		self.installed = None;
		self.menu = None;
	}

	/// Takes both files back out, where they are still this run's. Called on the
	/// way down, and safe to repeat.
	pub fn uninstall(&mut self) {
		self.uninstall_with(menu::engine_running_anywhere);
	}

	fn uninstall_with(&mut self, engine_running: impl Fn() -> bool) {
		if let Some(archive) = self.menu.take()
			&& carries(&menu::main_file(&archive), &self.file_token)
		{
			remove_menu(&archive, engine_running);
		}
		if let Some(path) = self.installed.take()
			&& carries(&path, &self.file_token)
		{
			remove_widget(&path);
		}
	}
}

/// Whether the file holds `token`, which is how a run knows what it wrote from
/// what another lobby sharing the data directory wrote since.
fn carries(path: &Path, token: &str) -> bool {
	std::fs::read_to_string(path).is_ok_and(|text| text.contains(token))
}

pub use widget::FILE;

/// Watches the file the widget and the menu append to, acting on each line
/// that carries this run's token and taking the lines out once read. Emptying
/// it is also the Lua's sign that someone reads.
///
/// Polled, and only while a game of ours runs: a read costs next to nothing,
/// and a file-system watch would die with the mods write directory, which is
/// wiped before each launch.
pub fn watch_file(
	path: PathBuf,
	token: String,
	actions: Arc<dyn Actions>,
) -> tokio::task::JoinHandle<()> {
	tokio::spawn(async move {
		let mut last_act = std::time::Instant::now() - FILE_PACE;
		loop {
			while !actions.game_running() {
				tokio::time::sleep(FILE_IDLE).await;
			}
			// Written before this game began, so no ask of this game's.
			take_file(&path);
			while actions.game_running() {
				tokio::time::sleep(FILE_POLL).await;
				let Some(text) = take_file(&path) else {
					continue;
				};
				for command in text.lines().filter_map(|line| parse(line, &token)) {
					match command {
						// Can only stop the lobby listening for Escape, so a mod
						// saying it early gains nothing, and it comes about as a
						// player's first press goes in.
						Command::Loaded => {
							actions.game_loaded();
						}
						// A mod inside the game can write this file too; it gets
						// to ask this often and no more.
						Command::Raise | Command::QuitGame if last_act.elapsed() < FILE_PACE => {}
						Command::Raise => {
							last_act = std::time::Instant::now();
							actions.raise();
						}
						Command::QuitGame => {
							last_act = std::time::Instant::now();
							actions.quit_game();
						}
					}
				}
			}
		}
	})
}

/// How often the file is read during a game: the delay it adds to Escape and
/// the Lobby button.
const FILE_POLL: std::time::Duration = std::time::Duration::from_millis(50);
/// How often to look for a game starting. A game takes far longer than this to
/// load before it can ask for anything.
const FILE_IDLE: std::time::Duration = std::time::Duration::from_secs(1);
/// How often the file may ask for anything.
const FILE_PACE: std::time::Duration = std::time::Duration::from_secs(1);
/// The most of the file that is read: a line is a few dozen bytes, and the
/// game may write the file.
const FILE_MOST: u64 = 4 * 1024;

/// The front of the file, decoded lossily, and the file emptied whatever
/// it held; `None` when there was nothing.
fn take_file(path: &Path) -> Option<String> {
	use std::io::Read;
	let mut head = Vec::new();
	std::fs::File::open(path)
		.ok()?
		.take(FILE_MOST)
		.read_to_end(&mut head)
		.ok()?;
	if head.is_empty() {
		return None;
	}
	if let Err(err) = std::fs::File::create(path) {
		tracing::warn!(%err, path = %path.display(), "the in-game file could not be emptied");
	}
	Some(String::from_utf8_lossy(&head).into_owned())
}

/// Takes out what an earlier run left and this one will not rewrite: a killed
/// run never reaches its exit handler, and the menu is kept for as long as any
/// engine runs.
pub fn sweep(data_dir: &Path, widget_wanted: bool, menu_wanted: bool) {
	sweep_with(
		data_dir,
		widget_wanted,
		menu_wanted,
		menu::engine_running_anywhere,
	);
}

fn sweep_with(
	data_dir: &Path,
	widget_wanted: bool,
	menu_wanted: bool,
	engine_running: impl Fn() -> bool,
) {
	let path = widget::path(data_dir);
	if !widget_wanted && path.is_file() {
		remove_widget(&path);
	}
	let archive = menu::dir(data_dir);
	if !menu_wanted && archive.is_dir() {
		remove_menu(&archive, engine_running);
	}
}

/// Any engine may have been started against the menu -- by an earlier run of
/// ours, say -- and asks for it again when its game ends. Finding it gone is a
/// content error in the player's face, so it stays until no engine runs.
fn remove_menu(archive: &Path, engine_running: impl Fn() -> bool) {
	if engine_running() {
		tracing::info!(path = %archive.display(), "leaving the in-game menu for a running engine");
		return;
	}
	if let Err(err) = std::fs::remove_dir_all(archive) {
		tracing::warn!(%err, path = %archive.display(), "leaving the in-game menu behind");
	}
}

/// A widget we cannot remove is inert anyway: it fails to connect and stops
/// consuming the key. One taken from under a game costs nothing either, since
/// the engine read it when the game began.
fn remove_widget(path: &Path) {
	if let Err(err) = std::fs::remove_file(path) {
		tracing::warn!(%err, path = %path.display(), "leaving the in-game widget behind");
	}
}

impl Drop for InGame {
	fn drop(&mut self) {
		self.uninstall();
	}
}

async fn serve(stream: TcpStream, token: String, actions: Arc<dyn Actions>) {
	let mut reader = BufReader::new(stream);
	let mut line = String::new();
	// One connection may carry several presses; the widget keeps it open.
	loop {
		line.clear();
		match reader.read_line(&mut line).await {
			Ok(0) | Err(_) => return,
			Ok(_) => {}
		}
		let acted = match parse(&line, &token) {
			Some(Command::Raise) => actions.raise(),
			Some(Command::QuitGame) => actions.quit_game(),
			Some(Command::Loaded) => actions.game_loaded(),
			// A bad token is the interesting case and says nothing back: a
			// reply would tell a guesser whether it was close.
			None => return,
		};
		let reply: &[u8] = if acted { b"ok\n" } else { b"no\n" };
		if reader.get_mut().write_all(reply).await.is_err() {
			return;
		}
	}
}

/// A per-run secret, baked into the widget we write and required on the wire.
pub fn token() -> String {
	// Four 32-bit draws rather than 32 nibbles: the same 128 bits, and it uses
	// the one `rand` entry point the rest of this workspace already uses.
	(0..4).fold(String::new(), |mut token, _| {
		use std::fmt::Write;
		let _ = write!(token, "{:08x}", rand::random::<u32>());
		token
	})
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

	use super::*;

	#[derive(Default)]
	struct Spy {
		raised: AtomicUsize,
		quit: AtomicUsize,
		loaded: AtomicUsize,
		/// What the app would answer: whether this is our game.
		answer: bool,
		running: AtomicBool,
	}

	fn spy(answer: bool) -> Arc<Spy> {
		Arc::new(Spy {
			answer,
			running: AtomicBool::new(true),
			..Spy::default()
		})
	}

	/// A watcher past the emptying that begins each game, so that a line from
	/// here on is this game's.
	async fn watching(path: &Path, spy: &Arc<Spy>) -> tokio::task::JoinHandle<()> {
		let watcher = watch_file(path.to_path_buf(), "8f3c1d9e".into(), spy.clone());
		tokio::task::yield_now().await;
		watcher
	}

	/// Waits for `done`, for longer than the slowest thing a watcher waits on.
	async fn until(done: impl Fn() -> bool) {
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
		while !done() && std::time::Instant::now() < deadline {
			tokio::time::sleep(std::time::Duration::from_millis(20)).await;
		}
	}

	/// The file as the game would find it in a write directory, its
	/// directory made as a launch makes it.
	fn file_in(write_dir: &Path) -> PathBuf {
		let path = widget::file(write_dir);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		path
	}

	impl Actions for Spy {
		fn raise(&self) -> bool {
			self.raised.fetch_add(1, Ordering::SeqCst);
			self.answer
		}
		fn quit_game(&self) -> bool {
			self.quit.fetch_add(1, Ordering::SeqCst);
			self.answer
		}
		fn game_loaded(&self) -> bool {
			self.loaded.fetch_add(1, Ordering::SeqCst);
			self.answer
		}
		fn game_running(&self) -> bool {
			self.running.load(Ordering::SeqCst)
		}
	}

	#[tokio::test]
	async fn a_game_says_over_the_port_that_it_has_loaded() {
		let spy = spy(true);
		let ingame = InGame::start(spy.clone()).await.unwrap();

		let reply = ask(ingame.port, &format!("{} loaded\n", ingame.token)).await;

		assert_eq!(reply, "ok");
		assert_eq!(spy.loaded.load(Ordering::SeqCst), 1);
		assert_eq!(spy.raised.load(Ordering::SeqCst), 0);
	}

	/// The widget and the menu both say it, about as a player's first Escape
	/// goes in, and saying it can only stop the lobby listening: it is not paced
	/// with what the file may ask for, and does not use that up.
	#[tokio::test]
	async fn a_game_says_through_the_file_that_it_has_loaded_and_a_press_still_gets_through() {
		let home = tempfile::tempdir().unwrap();
		let path = file_in(home.path());
		let spy = spy(true);
		let watcher = watching(&path, &spy).await;
		// Past the emptying that begins each game.
		tokio::time::sleep(FILE_POLL * 2).await;

		std::fs::write(
			&path,
			"8f3c1d9e loaded\n8f3c1d9e loaded\n8f3c1d9e raise\ndeadbeef loaded\n",
		)
		.unwrap();
		until(|| spy.raised.load(Ordering::SeqCst) == 1).await;
		watcher.abort();

		assert_eq!(spy.loaded.load(Ordering::SeqCst), 2);
		assert_eq!(spy.raised.load(Ordering::SeqCst), 1);
	}

	/// Sends one line and reads the answer back.
	async fn ask(port: u16, line: &str) -> String {
		let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
		let mut stream = BufReader::new(stream);
		stream.get_mut().write_all(line.as_bytes()).await.unwrap();
		let mut reply = String::new();
		stream.read_line(&mut reply).await.unwrap();
		reply.trim().to_owned()
	}

	#[tokio::test]
	async fn a_press_with_the_right_token_raises_the_lobby() {
		let spy = spy(true);
		let ingame = InGame::start(spy.clone()).await.unwrap();

		let reply = ask(ingame.port, &format!("{} raise\n", ingame.token)).await;
		assert_eq!(reply, "ok");
		assert_eq!(spy.raised.load(Ordering::SeqCst), 1);
	}

	#[tokio::test]
	async fn a_game_that_is_not_ours_is_told_so_and_keeps_its_escape() {
		// The widget consumes the key only on `ok`. Answering `no` is what
		// leaves a Chobby game's Escape alone while modlobby happens to run.
		let spy = spy(false);
		let ingame = InGame::start(spy.clone()).await.unwrap();

		let reply = ask(ingame.port, &format!("{} raise\n", ingame.token)).await;
		assert_eq!(reply, "no");
		assert_eq!(spy.raised.load(Ordering::SeqCst), 1, "asked, and declined");
	}

	#[tokio::test]
	async fn something_else_on_the_port_gets_nowhere() {
		let spy = spy(true);
		let ingame = InGame::start(spy.clone()).await.unwrap();

		let mut stream = TcpStream::connect(("127.0.0.1", ingame.port))
			.await
			.unwrap();
		// Anyone can reach a loopback port; reaching it is not permission.
		stream.write_all(b"0000 quit\nalso quit\n").await.unwrap();
		stream.flush().await.unwrap();

		tokio::time::sleep(std::time::Duration::from_millis(60)).await;
		assert_eq!(spy.quit.load(Ordering::SeqCst), 0);
		assert_eq!(spy.raised.load(Ordering::SeqCst), 0);
	}

	fn a_run(port: u16, token: &str, file_token: &str) -> InGame {
		InGame {
			port,
			token: token.into(),
			file_token: file_token.into(),
			installed: None,
			menu: None,
		}
	}

	/// What a launch of this run leaves on disk: the widget and the archive.
	fn launch_from(ingame: &mut InGame, home: &Path) -> (PathBuf, PathBuf) {
		let files = ingame.launch_files(home, true, true);
		lobby_runtime::launch::write_files(home, &files.open).unwrap();
		(widget::path(home), menu::dir(home))
	}

	#[test]
	fn nothing_is_on_disk_until_a_launch_puts_it_there() {
		let home = tempfile::tempdir().unwrap();
		let mut ingame = a_run(4242, "abc", "filesecret");

		let files = ingame.launch_files(home.path(), true, true);

		assert!(
			!files.open.is_empty(),
			"handed over for the launch to write"
		);
		assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
		// And an exit without a launch has nothing to take back.
		ingame.uninstall_with(|| false);
	}

	#[test]
	fn both_files_are_written_and_taken_back_out() {
		let home = tempfile::tempdir().unwrap();
		let mut ingame = a_run(4242, "abc", "filesecret");

		let (path, menu) = launch_from(&mut ingame, home.path());
		assert!(path.is_file());
		let source = std::fs::read_to_string(&path).unwrap();
		assert!(source.contains("4242"), "the port is baked in");
		assert!(source.contains("abc"), "and so is the token");
		assert!(source.contains("filesecret"), "and the file channel's");
		assert!(
			widget::file(home.path()).parent().unwrap().is_dir(),
			"the file's directory is made, since Lua's io.open will not"
		);

		ingame.uninstall_with(|| false);
		assert!(!path.exists());
		assert!(!menu.exists());
		// Removing twice is what a drop after an explicit stop does.
		ingame.uninstall_with(|| false);
	}

	/// Two lobbies sharing a data directory, a release and a dev build: the
	/// files belong to whoever launched last, and the other's exit, which a dev
	/// build's restart is, must not take them from under that game.
	#[test]
	fn a_lobby_takes_back_only_the_files_that_are_its_own() {
		let home = tempfile::tempdir().unwrap();
		let mut ours = a_run(4242, "abc", "filesecret");
		let mut theirs = a_run(5353, "def", "theirsecret");
		let (path, menu) = launch_from(&mut ours, home.path());

		// Beside us and never launching, its exit touches nothing.
		theirs.launch_files(home.path(), true, true);
		theirs.uninstall_with(|| false);
		assert!(path.is_file());
		assert!(menu.join("modinfo.lua").is_file());

		// Once it has launched the files say so, and our exit leaves them.
		launch_from(&mut theirs, home.path());
		ours.uninstall_with(|| false);
		assert!(std::fs::read_to_string(&path).unwrap().contains("5353"));
		assert!(menu.join("LuaMenu").join("main.lua").is_file());

		theirs.uninstall_with(|| false);
		assert!(!path.exists());
		assert!(!menu.exists());
	}

	/// A game in the sandbox can read everything it is handed, and the
	/// socket's token must never be among it. Loopback is shut in there, but a
	/// secret that is not present needs no such argument.
	#[test]
	fn what_a_sandboxed_game_reads_carries_no_socket_secret() {
		let home = tempfile::tempdir().unwrap();
		let mut ingame = a_run(4242, "sockettoken", "filesecret");

		let files = ingame.launch_files(home.path(), true, true);

		for (path, text) in files.confined.iter().chain(&files.mods) {
			assert!(!text.contains("sockettoken"), "{path:?}");
			assert!(!text.contains("4242"), "{path:?}");
		}
		let (_, menu) = files
			.confined
			.iter()
			.find(|(path, _)| path.ends_with("main.lua"))
			.expect("the menu goes in for a confined game too");
		assert!(menu.contains("local PORT = 0"));
		assert!(menu.contains(r#"local FILE_TOKEN = "filesecret""#));
		// The open copy goes where the confined one does, and has the socket.
		let (path, open) = files
			.open
			.iter()
			.find(|(path, _)| path.ends_with("main.lua"))
			.unwrap();
		assert!(open.contains("sockettoken"));
		assert!(files.confined.iter().any(|(same, _)| same == path));
	}

	#[test]
	fn only_what_is_wanted_is_handed_over() {
		let home = tempfile::tempdir().unwrap();
		let mut ingame = a_run(4242, "abc", "filesecret");
		let names = |files: &[(PathBuf, String)]| {
			files
				.iter()
				.map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
				.collect::<Vec<_>>()
		};

		assert_eq!(
			ingame.launch_files(home.path(), false, false),
			LaunchFiles::default()
		);

		let widget_only = ingame.launch_files(home.path(), true, false);
		assert!(names(&widget_only.open).contains(&"modlobby_escape.lua".to_owned()));
		assert!(!names(&widget_only.open).contains(&"main.lua".to_owned()));
		assert!(widget_only.confined.is_empty());

		// The menu falls back on the same file, so its directory goes in too.
		let menu_only = ingame.launch_files(home.path(), false, true);
		assert!(!names(&menu_only.open).contains(&"modlobby_escape.lua".to_owned()));
		assert!(names(&menu_only.open).contains(&"main.lua".to_owned()));
		assert!(names(&menu_only.open).contains(&"modlobby-ingame.txt".to_owned()));
		assert!(names(&menu_only.mods).contains(&"modlobby-ingame.txt".to_owned()));
		assert!(!names(&menu_only.mods).contains(&"modlobby_escape.lua".to_owned()));
	}

	/// The bug: an exit took the menu from under a game an earlier run of ours
	/// had started, and the game's end was a content error.
	#[test]
	fn a_running_engine_keeps_the_menu() {
		let home = tempfile::tempdir().unwrap();
		let mut ingame = a_run(4242, "abc", "filesecret");
		let (widget, menu) = launch_from(&mut ingame, home.path());

		ingame.uninstall_with(|| true);

		assert!(menu.join("modinfo.lua").is_file());
		assert!(!widget.exists(), "the widget was read when the game began");
	}

	#[test]
	fn leftovers_nobody_wants_are_swept_once_no_engine_runs() {
		let home = tempfile::tempdir().unwrap();
		let mut ingame = a_run(4242, "abc", "filesecret");
		let (widget, menu) = launch_from(&mut ingame, home.path());
		// A killed run: nothing is taken out.
		ingame.leave_behind();

		sweep_with(home.path(), true, false, || true);
		assert!(widget.is_file(), "wanted, so the next launch rewrites it");
		assert!(menu.is_dir(), "an engine still runs");

		sweep_with(home.path(), false, false, || false);
		assert!(!widget.exists());
		assert!(!menu.exists());
	}

	#[test]
	fn left_behind_for_a_running_game_both_files_stay() {
		let home = tempfile::tempdir().unwrap();
		let mut ingame = a_run(4242, "abc", "filesecret");
		let (widget, menu) = launch_from(&mut ingame, home.path());

		ingame.leave_behind();
		drop(ingame);

		// The engine that was started against them may ask for them again.
		assert!(widget.is_file());
		assert!(menu.join("modinfo.lua").is_file());
		assert!(menu.join("LuaMenu").join("main.lua").is_file());
	}

	#[tokio::test]
	async fn a_line_appended_to_the_file_raises_the_lobby_once() {
		let dir = tempfile::tempdir().unwrap();
		let path = file_in(dir.path());
		let spy = spy(true);
		let watcher = watching(&path, &spy).await;
		std::fs::write(&path, "8f3c1d9e raise\n").unwrap();
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
		while spy.raised.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
			tokio::time::sleep(std::time::Duration::from_millis(50)).await;
		}
		assert_eq!(spy.raised.load(Ordering::SeqCst), 1);
		tokio::time::sleep(std::time::Duration::from_millis(600)).await;
		assert_eq!(
			spy.raised.load(Ordering::SeqCst),
			1,
			"a line is acted on once"
		);
		assert_eq!(
			std::fs::read_to_string(&path).unwrap(),
			"",
			"read lines are taken out"
		);
		watcher.abort();
	}

	#[tokio::test]
	async fn a_line_with_the_wrong_token_in_the_file_does_nothing() {
		let dir = tempfile::tempdir().unwrap();
		let path = file_in(dir.path());
		let spy = spy(true);
		let watcher = watching(&path, &spy).await;
		std::fs::write(&path, "deadbeef raise\n").unwrap();
		tokio::time::sleep(std::time::Duration::from_millis(700)).await;
		assert_eq!(spy.raised.load(Ordering::SeqCst), 0);
		watcher.abort();
	}

	#[tokio::test]
	async fn a_huge_escape_file_is_read_in_part_and_emptied() {
		let dir = tempfile::tempdir().unwrap();
		let path = file_in(dir.path());
		let spy = spy(true);
		let watcher = watching(&path, &spy).await;
		let mut huge = b"8f3c1d9e raise\n".to_vec();
		huge.extend(std::iter::repeat_n(b'x', 2 * 1024 * 1024));
		std::fs::write(&path, &huge).unwrap();
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
		while spy.raised.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
			tokio::time::sleep(std::time::Duration::from_millis(50)).await;
		}
		assert_eq!(spy.raised.load(Ordering::SeqCst), 1);
		assert_eq!(
			std::fs::metadata(&path).unwrap().len(),
			0,
			"emptied whatever its size"
		);
		watcher.abort();
	}

	#[tokio::test]
	async fn bytes_that_are_not_utf8_do_not_stall_the_watcher() {
		let dir = tempfile::tempdir().unwrap();
		let path = file_in(dir.path());
		let spy = spy(true);
		let watcher = watching(&path, &spy).await;
		std::fs::write(&path, b"\xff\xfe junk\n8f3c1d9e raise\n").unwrap();
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
		while spy.raised.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
			tokio::time::sleep(std::time::Duration::from_millis(50)).await;
		}
		assert_eq!(spy.raised.load(Ordering::SeqCst), 1);
		assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
		watcher.abort();
	}

	/// A mod that writes the file can only ask so often.
	#[tokio::test]
	async fn with_no_game_running_the_file_is_left_as_it_is() {
		let dir = tempfile::tempdir().unwrap();
		let path = file_in(dir.path());
		let spy = spy(true);
		spy.running.store(false, Ordering::SeqCst);
		let watcher = watching(&path, &spy).await;
		std::fs::write(&path, "8f3c1d9e raise\n").unwrap();
		tokio::time::sleep(FILE_POLL * 4).await;
		assert_eq!(spy.raised.load(Ordering::SeqCst), 0);
		assert_eq!(
			std::fs::read_to_string(&path).unwrap(),
			"8f3c1d9e raise\n",
			"a line left in the file is how the Lua knows nobody reads"
		);
		watcher.abort();
	}

	#[tokio::test]
	async fn a_line_from_before_the_game_is_dropped_and_the_games_own_raises() {
		let dir = tempfile::tempdir().unwrap();
		let path = file_in(dir.path());
		let spy = spy(true);
		spy.running.store(false, Ordering::SeqCst);
		std::fs::write(&path, "8f3c1d9e raise\n").unwrap();
		let watcher = watching(&path, &spy).await;

		spy.running.store(true, Ordering::SeqCst);
		until(|| std::fs::read_to_string(&path).unwrap().is_empty()).await;
		assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
		tokio::time::sleep(FILE_POLL * 4).await;
		assert_eq!(spy.raised.load(Ordering::SeqCst), 0, "not this game's ask");

		std::fs::write(&path, "8f3c1d9e raise\n").unwrap();
		until(|| spy.raised.load(Ordering::SeqCst) == 1).await;
		assert_eq!(spy.raised.load(Ordering::SeqCst), 1);
		watcher.abort();
	}

	#[tokio::test]
	async fn the_game_ending_stops_the_reading() {
		let dir = tempfile::tempdir().unwrap();
		let path = file_in(dir.path());
		let spy = spy(true);
		let watcher = watching(&path, &spy).await;
		spy.running.store(false, Ordering::SeqCst);
		tokio::time::sleep(FILE_POLL * 2).await;

		std::fs::write(&path, "8f3c1d9e raise\n").unwrap();
		tokio::time::sleep(FILE_POLL * 4).await;
		assert_eq!(spy.raised.load(Ordering::SeqCst), 0);
		assert_ne!(std::fs::read_to_string(&path).unwrap(), "");
		watcher.abort();
	}

	#[tokio::test]
	async fn raise_lines_are_rate_limited() {
		let dir = tempfile::tempdir().unwrap();
		let path = file_in(dir.path());
		let spy = spy(true);
		let watcher = watching(&path, &spy).await;
		std::fs::write(&path, "8f3c1d9e raise\n".repeat(50)).unwrap();
		tokio::time::sleep(std::time::Duration::from_millis(700)).await;
		assert_eq!(
			spy.raised.load(Ordering::SeqCst),
			1,
			"fifty lines in one read raise once"
		);
		watcher.abort();
	}
}
