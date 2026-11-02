//! A LuaMenu archive whose whole purpose is BAR's in-game "Lobby" button.
//!
//! BAR decides between "Quit" and "Lobby" on the top bar by asking the engine
//! what menu it was started with and looking for `chobby` in the name
//! (`gui_top_bar.lua:3574`). There is no other hook: `WG.topbar` exposes no way
//! to rename or re-target a button, and the label is chosen inside a file-local
//! function. So the way to be offered the button is to be a menu whose name
//! satisfies that test.
//!
//! What the button then does is the good part, and it is why this is worth
//! doing at all. With a menu loaded, BAR skips its quit-confirmation screen
//! mid-game and sends `showLobby` straight to the menu, **leaving the game
//! running** -- which is exactly modlobby's overlay. The menu here turns that
//! message into the same `raise` its Escape widget already sends.
//!
//! After a game, BAR's "Quit" sends `ReloadForce`, and the engine drops into
//! the menu rather than exiting (`SpringApp::Reload`). Since this menu is not a
//! lobby and has nothing to show, it quits the process the moment it is
//! activated -- so the engine still ends when the player says so, and modlobby
//! still sees it end.
//!
//! The menu is also how Escape reaches modlobby since BAR took the socket away
//! from user widgets (#9547): the Escape widget sends it the same `showLobby`
//! the top bar does, and a LuaMenu keeps its socket. Where the port does not
//! answer -- inside modlobby's sandbox there is no socket at all -- the ask
//! goes as one line into the file modlobby polls, under the file channel's
//! own token.
//!
//! BAR's button fires on the press, and the lobby only comes up once the game
//! has seen the release. Windows sends a release to whichever window is under
//! the cursor, so focus taken mid-press leaves SDL (the engine's input library)
//! believing the button is still down, and the player's next clicks on that
//! button are spent correcting it.
//!
//! The archive is generated rather than shipped for the same reason the widget
//! is: the port and tokens change every run.

use std::path::{Path, PathBuf};

/// Must contain `chobby`, lowercased, or BAR offers "Quit" instead
/// (`gui_top_bar.lua:3574`). The rest of the name says whose it really is.
pub const ARCHIVE_NAME: &str = "modlobby chobby shim";

/// The archive's version, which is half of how the engine indexes it.
///
/// Fixed rather than the app's: an engine looks the menu up again, by the name
/// it was launched with, when its game ends, and every build rewrites this
/// archive when it launches a game. A release beside a dev build, or an update
/// landing under a running game, would otherwise rename it from under the engine.
const ARCHIVE_VERSION: &str = "1";

/// What to pass to `--menu`: the archive's *versioned* name.
///
/// The scanner appends the version to the name unless the name already
/// contains it (`ArchiveScanner.cpp:190-192`), and every lookup then matches
/// on that combined string -- `ArchiveFromName` compares against
/// `GetNameVersioned()` and hands back its own argument when nothing matches
/// (`:1786-1797`), which is how a bare name ends up being looked up as though
/// it were a filename and throws "Dependent archive … not found" out of `Init`
/// before a window ever opens.
///
/// BAR launches Chobby exactly this way: its launcher config passes
/// `--menu "BYAR Chobby $VERSION"` -- name and version, one space between.
///
/// Built rather than written out, so the string on the command line and the
/// one in `modinfo.lua` cannot drift apart.
pub fn menu_name() -> String {
	if ARCHIVE_NAME.contains(ARCHIVE_VERSION) {
		return ARCHIVE_NAME.to_owned();
	}
	format!("{ARCHIVE_NAME} {ARCHIVE_VERSION}")
}

/// The directory name under `games/`.
const DIR_NAME: &str = "modlobby-chobby-shim.sdd";

/// Where the archive goes.
///
/// Under `games/`, because that is one of the four roots the archive scanner
/// looks in -- `base`, `maps`, `games`, `packages`
/// (`DataDirLocater::GetDataDirRoots`). A directory anywhere else is not
/// scanned, and `--menu` naming an archive the scanner never found throws
/// during startup and the engine does not come up at all.
pub fn dir(data_dir: &Path) -> PathBuf {
	data_dir.join("games").join(DIR_NAME)
}

/// `modtype = 5` is a menu, and `onlyLocal` keeps it out of the game list.
/// Both taken from Chobby's own `modinfo.lua`.
fn modinfo() -> String {
	format!(
		"return {{\n\
         \x20 name = '{ARCHIVE_NAME}',\n\
         \x20 shortName = 'MODLOBBY_SHIM',\n\
         \x20 description = 'Makes BAR offer Lobby instead of Quit, and raises modlobby.',\n\
         \x20 version = '{ARCHIVE_VERSION}',\n\
         \x20 modtype = 5,\n\
         \x20 onlyLocal = true,\n\
         }}\n"
	)
}

/// How long, in seconds, a line of the menu's may sit in the file before that
/// means nobody reads it. Many polls long, so a double click is never taken for
/// a lobby that has gone away.
const UNREAD_PATIENCE_SECS: u64 = 1;

/// The menu itself: no window, no drawing, one message and a wait for the
/// mouse button, and an eye on Escape where nothing else can judge it.
///
/// `LuaMenu/main.lua` is the entry point the engine loads (`LuaMenu.cpp:59`),
/// and its call-ins are plain globals rather than methods on a table.
///
/// The engine hands a menu no keys and no selection (`CLuaMenu::
/// LoadUnsyncedReadFunctions`), but it can read a key's state, which works
/// wherever the engine does and from the load screen on (`CLoadScreen::Draw`
/// updates it). So it can tell a press, and not what the press would do.
fn main_lua(socket: Option<(u16, &str)>, file_token: &str, escape_wanted: bool) -> String {
	let (port, token) = socket.unwrap_or((0, ""));
	format!(
		r#"-- modlobby: BAR's in-game Lobby button.
--
-- Written by modlobby when it launches a game, and removed once it exits with
-- no game left that could ask for it. It has no interface of its own. BAR offers a
-- "Lobby" button in place of "Quit" when a menu is loaded, and this turns that
-- button into modlobby's overlay. modlobby's Escape widget asks through here
-- too: a user widget has no socket of its own, a menu does.
--
-- If modlobby is not listening it quits the game rather than stranding anyone
-- in a menu that cannot draw itself.
--
-- The ask goes over the loopback port when one answers. Inside modlobby's
-- sandbox there is no socket, and it goes as one line into a file modlobby
-- polls instead.
--
-- It also reads Escape where the widget cannot judge it: during loading, and in
-- a game with user widgets switched off. It only reads, so the key still reaches
-- the game.

local PORT = {port}
local TOKEN = "{token}"
local FILE = "{file}"
local FILE_TOKEN = "{file_token}"
local PATIENCE = {patience}
local ESCAPE_WANTED = {escape_wanted}

local KEY_ESCAPE = 27
-- In seconds. Two presses this close mean the lobby where one might not.
local TWICE_WITHIN = 0.5

local socket = socket

-- Over the port: true for "ok", false for "no", nil when nothing answered.
local function ask_socket(verb)
	if PORT == 0 or not socket then return nil end
	local tcp = socket.tcp()
	if not tcp then return nil end
	tcp:settimeout(0.2)
	if not tcp:connect("127.0.0.1", PORT) then
		tcp:close()
		return nil
	end
	tcp:send(TOKEN .. " " .. verb .. "\n")
	local reply = tcp:receive("*l")
	tcp:close()
	if reply == nil then return nil end
	return reply == "ok"
end

-- Whether the file still holds a line. modlobby empties it as it reads, so
-- one left over means nobody is reading.
local function unread()
	local file = io.open(FILE, "r")
	if not file then return false end
	local size = file:seek("end")
	file:close()
	return size ~= nil and size > 0
end

-- When the last line went in.
local wrote_at = nil

-- A line of ours from moments ago is waiting for the next read. Any older, or
-- one we did not write, means nobody is reading.
local function waiting_its_turn()
	return wrote_at ~= nil and Spring.DiffTimers(Spring.GetTimer(), wrote_at) < PATIENCE
end

-- Into the file modlobby polls. The write directory is the game's working
-- directory.
local function ask_file(verb)
	if unread() then return waiting_its_turn() end
	local file = io.open(FILE, "a")
	if not file then
		return false
	end
	file:write(FILE_TOKEN .. " " .. verb .. "\n")
	file:close()
	wrote_at = Spring.GetTimer()
	return true
end

local function ask(verb)
	local answer = ask_socket(verb)
	if answer ~= nil then return answer end
	return ask_file(verb)
end

-- The lobby was asked for and is not up yet.
local asked = false

-- The load screen has gone and the game has the keyboard.
local game_active = false
-- modlobby's Escape widget has the keyboard too, and can see what is selected.
local widget_up = false
local escape_down = false
-- When a press that may be the first of two went in.
local first_press = nil

-- Escape going down since the last look, on its own: Shift+Escape is the
-- engine's way to abort a load.
local function escape_pressed()
	local down = Spring.GetKeyState(KEY_ESCAPE)
	local pressed = down and not escape_down
	escape_down = down
	if not pressed then return false end
	local alt, ctrl, meta, shift = Spring.GetModKeyState()
	return not (alt or ctrl or meta or shift)
end

-- Whether this press is for the lobby. In a loading game it has no other use.
-- In a running one the widget judges it. With no widget nothing here can see a
-- selection or a chat to close, so one press stays the game's, and a second
-- straight after is someone who wants out.
local function means_lobby()
	if not game_active then return true end
	if widget_up then return false end
	local now = Spring.GetTimer()
	if first_press and Spring.DiffTimers(now, first_press) < TWICE_WITHIN then
		first_press = nil
		return true
	end
	first_press = now
	return false
end

-- BAR sends this from the top bar while the game is still running, which is
-- the whole point: the lobby comes up over it rather than instead of it.
-- modlobby's Escape widget sends the same, for the same reason.
function RecvLuaMsg(msg)
	if msg == "showLobby" then
		asked = true
		return true
	end
	if msg == "modlobbyWidget" then
		widget_up = true
		return true
	end
	if msg == "modlobbyWidgetGone" then
		widget_up = false
		return true
	end
	-- "disableLobbyButton" is BAR telling a real lobby to hide its own button.
	-- There is no button here to hide.
	return false
end

local function button_down()
	local _, _, left, middle, right = Spring.GetMouseState()
	return left or middle or right
end

-- The top bar asks on the press. Waiting for the release keeps the game's
-- window in front until the game has seen it, or the next clicks on that
-- button are lost.
function Update()
	if ESCAPE_WANTED and escape_pressed() and means_lobby() then
		ask("raise")
	end
	if not asked or button_down() then return end
	asked = false
	if ask("raise") then return end
	-- modlobby is not answering, and BAR hides its own quit screen for as long
	-- as a menu is loaded (`gui_top_bar_buttons.lua:984`) -- so this button is
	-- the only way out of the game, and it just failed. Hand the player to the
	-- engine's own quit box, which the engine draws itself and which offers
	-- Quit To Menu and Quit To System.
	if Spring.SendLuaUIMsg then
		Spring.SendLuaUIMsg("modlobbyGone")
	end
end

-- The load screen is gone and the game has the keyboard. modlobby listens for
-- Escape from outside until now, in case nothing in here can reach it.
function ActivateGame()
	game_active = true
	ask("loaded")
end

-- Reached only with no game loaded: either the engine started here, or a game
-- just ended and `ReloadForce` dropped back. Neither is a state this menu has
-- anything to offer in, so it gets out of the way and lets the process end.
-- `Spring.Quit` is how Chobby does the same; `Spring.SendCommands` does not
-- exist in a LuaMenu, and a call-in that errors leaves the engine sitting on
-- a black frame it never draws.
function ActivateMenu()
	Spring.Quit()
end

-- Nothing to paint, ever.
function AllowDraw()
	return false
end
"#,
		file = super::widget::FILE,
		patience = UNREAD_PATIENCE_SECS,
	)
}

/// The file of the archive at `archive` that is written anew for every run.
pub fn main_file(archive: &Path) -> PathBuf {
	archive.join("LuaMenu").join("main.lua")
}

/// The archive's files, by path relative to the write directory.
///
/// `socket` is the port and token of the control socket, or `None` for a copy
/// that must not carry them: the one a game in the mods sandbox reads, where
/// any mod can read the file and no connection could be made anyway.
/// `escape_wanted` is whether Escape should raise the lobby: the menu is there
/// for BAR's Lobby button either way.
pub fn files(
	socket: Option<(u16, &str)>,
	file_token: &str,
	escape_wanted: bool,
) -> Vec<(PathBuf, String)> {
	let root = dir(Path::new(""));
	vec![
		(root.join("modinfo.lua"), modinfo()),
		(
			main_file(&root),
			main_lua(socket, file_token, escape_wanted),
		),
	]
}

/// The file token in a menu of ours, which is the one its game asks under.
pub fn file_token_in(main_lua: &str) -> Option<&str> {
	main_lua.lines().find_map(|line| {
		line.strip_prefix("local FILE_TOKEN = \"")?
			.strip_suffix('"')
	})
}

/// Whether any engine runs on this machine, whoever started it: one launched
/// by an earlier run of ours that was killed still asks for the archive when
/// its game ends. Unsure reads as yes, because the archive left behind is
/// inert and taken from under an engine is a content error.
// ponytail: without tasklist/pgrep the archive is never removed; the next
// start rewrites it and the uninstaller takes it.
pub fn engine_running_anywhere() -> bool {
	probe().unwrap_or(true)
}

#[cfg(windows)]
fn probe() -> Option<bool> {
	use std::os::windows::process::CommandExt;
	const CREATE_NO_WINDOW: u32 = 0x0800_0000;
	let filter = format!("IMAGENAME eq {}", recoil::ENGINE_BINARY);
	let output = std::process::Command::new("tasklist")
		.args(["/FI", &filter, "/FO", "CSV", "/NH"])
		.creation_flags(CREATE_NO_WINDOW)
		.output()
		.ok()?;
	output
		.status
		.success()
		.then(|| listed(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(not(windows))]
fn probe() -> Option<bool> {
	let status = std::process::Command::new("pgrep")
		.args(["-x", recoil::ENGINE_BINARY])
		.stdout(std::process::Stdio::null())
		.status()
		.ok()?;
	// 1 is "no match"; anything past it is pgrep failing.
	match status.code() {
		Some(0) => Some(true),
		Some(1) => Some(false),
		_ => None,
	}
}

/// `tasklist /FO CSV /NH` prints one quoted row per match and, when there is
/// none, a sentence in the system's language. Only the image name is never
/// translated.
#[cfg(any(windows, test))]
fn listed(stdout: &str) -> bool {
	let quoted = format!("\"{}\"", recoil::ENGINE_BINARY);
	stdout
		.lines()
		.any(|line| line.to_ascii_lowercase().starts_with(&quoted))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn an_engine_is_read_from_its_row_not_from_the_sentence() {
		let engine = recoil::ENGINE_BINARY;
		assert!(listed(&format!(
			"\"{engine}\",\"4242\",\"Console\",\"1\",\"812\u{a0}344 K\"\r\n"
		)));
		assert!(!listed(
			"INFO: No tasks are running which match the specified criteria.\r\n"
		));
		assert!(!listed(
			"INFORMATION: Det finns inga aktiviteter som matchar angivna villkor.\r\n"
		));
		assert!(!listed(
			"\"spring-headless.exe\",\"4242\",\"Console\",\"1\",\"1 K\"\r\n"
		));
	}

	#[test]
	fn the_name_is_what_bar_looks_for() {
		// `gui_top_bar.lua:3574` lowercases the menu name and searches it for
		// "chobby". Ours has to survive that or the button says Quit.
		assert!(ARCHIVE_NAME.to_lowercase().contains("chobby"));
		assert!(menu_name().to_lowercase().contains("chobby"));
	}

	/// The bug that stopped the engine starting: `--menu` was handed the bare
	/// name, and the scanner indexes the versioned one.
	#[test]
	fn the_menu_argument_carries_the_version_the_scanner_indexes() {
		assert_eq!(menu_name(), format!("{ARCHIVE_NAME} {ARCHIVE_VERSION}"));
	}

	/// Whatever goes on the command line is what `modinfo.lua` declares.
	#[test]
	fn modinfo_agrees_with_the_menu_argument() {
		let info = modinfo();
		assert!(info.contains(&format!("name = '{ARCHIVE_NAME}'")));
		assert!(info.contains(&format!("version = '{ARCHIVE_VERSION}'")));
	}

	fn the_file<'a>(files: &'a [(PathBuf, String)], name: &Path) -> &'a str {
		files
			.iter()
			.find(|(path, _)| path == name)
			.map(|(_, text)| text.as_str())
			.unwrap_or_else(|| panic!("{name:?} in {files:?}"))
	}

	/// The copy for the sandbox: any game in it can read the file, so it holds
	/// no socket secret, and the Lua never tries the port.
	#[test]
	fn a_copy_without_a_socket_carries_no_port_or_token() {
		let archive = dir(Path::new(""));
		let files = files(None, "filesecret", true);
		let lua = the_file(&files, &archive.join("LuaMenu").join("main.lua"));
		assert!(lua.contains("local PORT = 0"));
		assert!(lua.contains(r#"local TOKEN = """#));
		assert!(lua.contains("if PORT == 0 or not socket then return nil end"));
		assert!(lua.contains(r#"local FILE_TOKEN = "filesecret""#));
	}

	#[test]
	fn it_is_written_where_the_scanner_looks() {
		let archive = dir(Path::new(""));
		let files = files(Some((1234, "secret")), "filesecret", true);

		// `games/` is one of the four roots `GetDataDirRoots` returns.
		assert_eq!(archive.parent().unwrap().file_name().unwrap(), "games");
		// `modinfo.lua` is what the scanner indexes, and `recoil::MenuArchive`
		// refuses to pass `--menu` without one.
		let info = the_file(&files, &archive.join("modinfo.lua"));

		let lua = the_file(&files, &archive.join("LuaMenu").join("main.lua"));
		assert!(lua.contains("local PORT = 1234"));
		assert!(lua.contains(r#"local TOKEN = "secret""#));
		assert!(lua.contains(r#"local FILE_TOKEN = "filesecret""#));
		assert!(lua.contains("function RecvLuaMsg"));
		assert!(lua.contains("function ActivateMenu"));
		// The dead-lobby way out: nobody is listening, so the player is told
		// through LuaUI, which can open the engine's quit box.
		assert!(lua.contains(r#"Spring.SendLuaUIMsg("modlobbyGone")"#));
		// The one quit call a LuaMenu has. `SendCommands` is not in its
		// environment, and the engine sat black on the error.
		assert!(lua.contains("Spring.Quit()"));
		assert!(!lua.contains("SendCommands("));

		assert!(info.contains("modtype = 5"), "a menu, not a game");
		assert!(info.contains(ARCHIVE_NAME));
	}

	/// `CLoadScreen`'s destructor hands the game the keyboard and calls this.
	#[test]
	fn the_game_taking_the_keyboard_is_told_to_the_lobby() {
		let lua = main_lua(Some((1234, "secret")), "filesecret", true);
		let ask = lua.find("local function ask(verb)").unwrap();
		let told = lua
			.find("function ActivateGame()\n\tgame_active = true\n\task(\"loaded\")")
			.unwrap();
		assert!(ask < told, "a Lua local has to be declared before its use");
	}

	/// What a lobby taking a game over reads, since the game asks under it.
	#[test]
	fn the_menu_on_disk_says_which_token_its_game_asks_under() {
		let lua = main_lua(None, "filesecret", true);
		assert_eq!(file_token_in(&lua), Some("filesecret"));
		assert_eq!(file_token_in("-- nothing of ours"), None);
	}

	#[test]
	fn the_menu_leaves_the_key_alone_where_escape_is_switched_off() {
		assert!(main_lua(None, "x", true).contains("local ESCAPE_WANTED = true"));
		assert!(main_lua(None, "x", false).contains("local ESCAPE_WANTED = false"));
	}

	/// How a press plays out takes an engine to run. These are the rules it
	/// rests on: one press while loading, the widget's call in a running game,
	/// and two quick ones where there is no widget to make that call.
	#[test]
	fn a_press_means_the_lobby_by_what_can_judge_it() {
		let lua = main_lua(None, "x", true);
		assert!(lua.contains("Spring.GetKeyState(KEY_ESCAPE)"));
		assert!(lua.contains("Spring.GetModKeyState()"));
		let loading = lua.find("if not game_active then return true end").unwrap();
		let widget = lua.find("if widget_up then return false end").unwrap();
		let twice = lua.find("< TWICE_WITHIN").unwrap();
		assert!(loading < widget && widget < twice);
		assert!(lua.contains(r#"msg == "modlobbyWidget""#));
		assert!(lua.contains(r#"msg == "modlobbyWidgetGone""#));

		let update = lua.find("function Update()").unwrap();
		let read = lua
			.find("if ESCAPE_WANTED and escape_pressed() and means_lobby() then")
			.unwrap();
		let button = lua
			.find("if not asked or button_down() then return end")
			.unwrap();
		assert!(
			update < read && read < button,
			"before the button's early out"
		);
	}

	/// The press only asks; the release is what shows the lobby. Focus taken
	/// while the button is down is the bug that made the Lobby button need
	/// three clicks.
	#[test]
	fn the_lobby_comes_up_once_the_game_has_seen_the_release() {
		let lua = main_lua(Some((1234, "secret")), "filesecret", true);
		let message = lua.find("function RecvLuaMsg").unwrap();
		let update = lua.find("function Update").unwrap();
		assert!(message < update);
		assert!(
			!lua[message..update].contains(r#"ask("raise")"#),
			"nothing is raised from the message, which arrives on the press"
		);
		let wait = lua
			.find("if not asked or button_down() then return end")
			.unwrap();
		let raise = lua.find(r#"if ask("raise") then return end"#).unwrap();
		assert!(update < wait && wait < raise);
		assert!(lua.contains("Spring.GetMouseState()"));
	}

	/// A second click before the next read finds the first line still there,
	/// and that is not a lobby that has gone: it opened the engine's quit box
	/// behind the overlay.
	#[test]
	fn a_line_of_ours_from_moments_ago_is_not_a_lobby_gone() {
		let lua = main_lua(Some((1234, "secret")), "filesecret", true);
		assert!(lua.contains(&format!("local PATIENCE = {UNREAD_PATIENCE_SECS}")));
		assert!(lua.contains("wrote_at = Spring.GetTimer()"));
		assert!(lua.contains("Spring.DiffTimers(Spring.GetTimer(), wrote_at) < PATIENCE"));
		assert!(
			std::time::Duration::from_secs(UNREAD_PATIENCE_SECS) >= crate::ingame::FILE_POLL * 10,
			"the patience has to outlast many reads"
		);
	}

	/// Where the port does not answer -- inside the sandbox there is no socket
	/// at all -- the button appends its ask to the file modlobby polls, under
	/// the file channel's own token. A reply of "no" over the port is an
	/// answer, and never falls through to the file.
	#[test]
	fn without_an_answer_over_the_port_the_lobby_button_writes_the_file() {
		let lua = main_lua(Some((1234, "secret")), "filesecret", true);
		assert!(lua.contains(&format!(
			r#"local FILE = "{}""#,
			crate::ingame::widget::FILE
		)));
		assert!(lua.contains(r#"io.open(FILE, "a")"#));
		assert!(lua.contains(r#"FILE_TOKEN .. " " .. verb"#));
		let socket = lua.find("local answer = ask_socket(verb)").unwrap();
		let file = lua.find("return ask_file(verb)").unwrap();
		assert!(
			socket < file,
			"the port first, the file when it did not answer"
		);
		assert!(lua.contains("if answer ~= nil then return answer end"));
		// A line nobody has taken out means nobody is reading: the player is
		// handed the engine's quit box rather than a file nobody watches.
		let unread = lua
			.find("if unread() then return waiting_its_turn() end")
			.unwrap();
		let append = lua.find(r#"io.open(FILE, "a")"#).unwrap();
		assert!(unread < append);
		assert!(lua.contains(r#"Spring.SendLuaUIMsg("modlobbyGone")"#));
		assert!(
			!lua.contains("CONFINED"),
			"the sandbox mark is not what decides"
		);
	}
}
