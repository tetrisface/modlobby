//! The Lua we write into the game, generated so the port and tokens are ours.

use std::path::{Path, PathBuf};

/// Where BAR loads user widgets from.
///
/// `barwidgets.lua:447` scans `LuaUI/Widgets/` in `VFS.RAW`, which is the
/// write directory — the same one we pass as `--write-dir`.
pub fn path(data_dir: &Path) -> PathBuf {
	data_dir
		.join("LuaUI")
		.join("Widgets")
		.join("modlobby_escape.lua")
}

/// The file the widget and the menu append to when nothing else reaches us,
/// relative to the write directory, which is the game's working directory.
///
/// Under `LuaUI/Config/` because that is one of the two places BAR lets a
/// user widget write (`barwidgets.lua`, `userWritableFolders`); a file at the
/// root of the write directory is refused.
pub const FILE: &str = "LuaUI/Config/modlobby-ingame.txt";

/// [`FILE`] inside a write directory, for the watcher.
pub fn file(write_dir: &Path) -> PathBuf {
	write_dir.join(FILE)
}

/// The widget, with this run's secrets in it.
///
/// `socket` is the port and token of the control socket, or `None` for a
/// copy that must not carry them: the one written into the mods sandbox,
/// where any mod can read the file and no connection could be made anyway.
/// `file_token` guards the file channel.
///
/// Written rather than shipped as a static file because the tokens change
/// every run: a widget left behind by a crash is not merely inert, it is
/// talking to a port that no longer answers with a secret that no longer works.
pub fn source(socket: Option<(u16, &str)>, file_token: &str) -> String {
	let (port, token) = socket.unwrap_or((0, ""));
	// The `{name}` placeholders are the only things interpolated; the rest is
	// literal Lua, kept readable so anyone can see what we put in their game.
	format!(
		r#"-- modlobby: Escape opens the lobby.
--
-- Written by modlobby when it launches a game and removed when it exits. It
-- draws nothing. Escape pressed with nothing selected is told to modlobby in
-- the first of three ways this game offers:
--
--   1. the loopback port modlobby listens on, guarded by a token for this run.
--      BAR gives user widgets no socket any more (#9547), and a copy written
--      into the mods sandbox is given no port at all;
--   2. modlobby's own LuaMenu, which keeps its socket and already turns BAR's
--      Lobby button into the same request. It tells us back when nobody
--      answered;
--   3. one line appended to a file under LuaUI/Config, which modlobby polls.
--
-- The key is consumed only when the ask got through, so a game launched from
-- any other lobby keeps its own Escape.

function widget:GetInfo()
	return {{
		name = "modlobby Escape",
		desc = "Escape with nothing selected opens the modlobby window",
		author = "modlobby",
		date = "2026",
		license = "GNU GPL, v2 or later",
		layer = 0,
		enabled = true,
	}}
end

-- 0: this copy was given no port.
local PORT = {port}
local TOKEN = "{token}"
-- Lowercased, as BAR's top bar also reads the menu's name.
local MENU = "{menu}"
local FILE = "{file}"
local FILE_TOKEN = "{file_token}"

local socket = socket
local GetSelectedUnitsCount = Spring.GetSelectedUnitsCount

local KEY_ESCAPE = 27
local connection = nil
-- The menu has been told this widget judges the key from here.
local told_menu = false

-- Connecting costs a round trip on loopback, which is nothing, but doing it
-- inside the keypress would still stall a frame if the port were gone. The
-- connection is kept and only rebuilt when it breaks.
local function connect()
	if PORT == 0 or not socket then
		return nil
	end
	local tcp = socket.tcp()
	if not tcp then
		return nil
	end
	tcp:settimeout(0.05)
	local ok = tcp:connect("127.0.0.1", PORT)
	if not ok then
		tcp:close()
		return nil
	end
	return tcp
end

-- Over the port: true for "ok", false for "no", nil when nothing answered.
--
-- modlobby replies "ok" only when the game we are in is one it launched and
-- the overlay is switched on; "no" hands the key back to the game untouched.
local function ask_socket(verb)
	if not connection then
		connection = connect()
	end
	if not connection then
		return nil
	end

	local sent = connection:send(TOKEN .. " " .. verb .. "\n")
	if not sent then
		-- The lobby was restarted under us; one reconnect, then give up.
		connection:close()
		connection = connect()
		if not connection then
			return nil
		end
		sent = connection:send(TOKEN .. " " .. verb .. "\n")
		if not sent then
			return nil
		end
	end

	local reply = connection:receive("*l")
	if not reply then
		connection:close()
		connection = nil
		return nil
	end
	return reply == "ok"
end

-- Whether the engine runs modlobby's menu, which has the socket we lack.
local function menu_loaded()
	if not Spring.GetMenuName or not Spring.SendLuaMenuMsg then
		return false
	end
	local name = Spring.GetMenuName()
	return type(name) == "string" and string.find(string.lower(name), MENU, 1, true) ~= nil
end

-- Through the menu: the same message BAR's Lobby button sends it. The menu
-- asks modlobby, and sends "modlobbyGone" back to us when nobody took it
-- (see RecvLuaMsg). nil when the engine runs another menu.
local function ask_menu(verb)
	if verb ~= "raise" or not menu_loaded() then
		return nil
	end
	Spring.SendLuaMenuMsg("showLobby")
	return true
end

-- Whether the file still holds a line. modlobby empties it as it reads, so
-- one left over means nobody is reading, and the key is not ours to take.
local function unread()
	local file = io.open(FILE, "r")
	if not file then
		return false
	end
	local size = file:seek("end")
	file:close()
	return size ~= nil and size > 0
end

-- Into the file modlobby polls. There is no answer to wait for.
local function ask_file(verb)
	if unread() then
		return false
	end
	local file = io.open(FILE, "a")
	if not file then
		return false
	end
	file:write(FILE_TOKEN .. " " .. verb .. "\n")
	file:close()
	return true
end

local function ask(verb)
	local answer = ask_socket(verb)
	if answer ~= nil then
		return answer
	end
	answer = ask_menu(verb)
	if answer ~= nil then
		return answer
	end
	return ask_file(verb)
end

-- The first way open to us, for the infolog.
local function way()
	if connection then
		return "the port"
	end
	if menu_loaded() then
		return "the menu"
	end
	return FILE
end

function widget:Initialize()
	connection = connect()
	Spring.Echo("modlobby: Escape asks through " .. way())
end

function widget:Shutdown()
	if connection then
		connection:close()
		connection = nil
	end
	if told_menu and menu_loaded() then
		Spring.SendLuaMenuMsg("modlobbyWidgetGone")
	end
end

-- The first update is the game having the keyboard, and the menu watching for
-- Escape can leave it to us, who can see what is selected. Not from Initialize:
-- the message is a direct call into the menu, and Initialize runs on the thread
-- that loads the game.
function widget:Update()
	if told_menu then
		return
	end
	told_menu = true
	if menu_loaded() then
		Spring.SendLuaMenuMsg("modlobbyWidget")
	end
end

-- The menu says its ask found nobody home.
--
-- BAR replaces its quit screen with the Lobby button for as long as a menu is
-- loaded, so a crashed modlobby leaves a running game with no way out but
-- Alt+F4. The engine's own quit box is drawn by the engine rather than by
-- LuaUI, is always available, and offers Quit To Menu and Quit To System.
function widget:RecvLuaMsg(msg)
	if msg ~= "modlobbyGone" then
		return false
	end
	Spring.Echo("modlobby is not running; opening the engine quit menu")
	Spring.SendCommands("QuitMenu")
	return true
end

-- The game is about to have the keyboard, and modlobby stops listening for
-- Escape on its behalf. The menu says so too; only one has to get through.
function widget:GamePreload()
	ask("loaded")
end

function widget:KeyPress(key, mods, isRepeat)
	if key ~= KEY_ESCAPE or isRepeat then
		return false
	end
	-- Escape already means "drop what I am holding". Only when it would
	-- otherwise do nothing does it mean "show me the lobby".
	if GetSelectedUnitsCount() > 0 then
		return false
	end
	if mods and (mods.alt or mods.ctrl or mods.shift or mods.meta) then
		return false
	end
	-- Consumed only if it actually got through, so the game keeps its own
	-- Escape whenever modlobby is not there.
	return ask("raise")
end
"#,
		menu = super::menu::ARCHIVE_NAME.to_lowercase(),
		file = FILE,
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	const SOCKET: Option<(u16, &str)> = Some((51234, "cafebabe"));

	#[test]
	fn it_lands_where_bar_looks_for_user_widgets() {
		let path = path(Path::new("C:/bar"));
		assert!(path.ends_with("LuaUI/Widgets/modlobby_escape.lua"));
	}

	/// BAR lets a user widget write under `LuaUI/Config/` and `LuaUI/Widgets/`
	/// only; the file at the root of the write directory was refused.
	#[test]
	fn the_file_is_where_a_user_widget_may_write() {
		assert!(FILE.to_lowercase().starts_with("luaui/config/"));
		assert!(file(Path::new("C:/bar")).ends_with("LuaUI/Config/modlobby-ingame.txt"));
	}

	#[test]
	fn the_port_and_tokens_are_this_runs() {
		let source = source(SOCKET, "f11e");
		assert!(source.contains("local PORT = 51234"));
		assert!(source.contains(r#"local TOKEN = "cafebabe""#));
		assert!(source.contains(r#"local FILE_TOKEN = "f11e""#));
	}

	/// The copy for the mods sandbox: any mod can read it, so it carries no
	/// socket secret, and the Lua never tries the port.
	#[test]
	fn a_copy_without_a_socket_carries_no_port_or_token() {
		let source = source(None, "f11e");
		assert!(source.contains("local PORT = 0"));
		assert!(source.contains(r#"local TOKEN = """#));
		assert!(source.contains("if PORT == 0 or not socket then"));
		assert!(source.contains(r#"local FILE_TOKEN = "f11e""#));
	}

	#[test]
	fn the_braces_survive_being_a_format_string() {
		// `GetInfo` returns a table, and a `{` that did not make it through
		// would be a Lua syntax error the game reports and we never see.
		let source = source(SOCKET, "x");
		assert!(source.contains("return {\n\t\tname = \"modlobby Escape\""));
		assert!(!source.contains("{{"), "no doubled braces left over");
	}

	#[test]
	fn escape_is_only_taken_when_it_would_do_nothing_else() {
		let source = source(SOCKET, "x");
		// The three guards that keep this from stealing the game's own key.
		assert!(source.contains("GetSelectedUnitsCount() > 0"));
		assert!(source.contains("isRepeat"));
		assert!(source.contains("return ask(\"raise\")"));
	}

	/// The three ways, in order: a reply over the port settles it, the menu
	/// is asked when the port did not answer, and the file is the last resort.
	#[test]
	fn the_port_the_menu_and_the_file_are_tried_in_that_order() {
		let source = source(SOCKET, "x");
		let socket = source.find("local answer = ask_socket(verb)").unwrap();
		let menu = source.find("answer = ask_menu(verb)").unwrap();
		let file = source.find("return ask_file(verb)").unwrap();
		assert!(socket < menu && menu < file);
		// "no" over the port is an answer, not a reason to try elsewhere.
		assert!(source.contains("if answer ~= nil then\n\t\treturn answer"));
	}

	/// The menu leaves a running game's Escape to this widget for as long as it
	/// is told the widget is up. The message is a direct call into the menu's Lua,
	/// so it comes from `Update`, on the game's main thread, and never from
	/// `Initialize`, which runs on the thread that loads the game.
	#[test]
	fn the_menu_is_told_the_widget_is_up_from_the_games_own_thread_and_when_it_goes() {
		let source = source(SOCKET, "x");
		let declared = source.find("local told_menu = false").unwrap();
		let initialize = source.find("function widget:Initialize()").unwrap();
		let shutdown = source.find("function widget:Shutdown()").unwrap();
		let update = source.find("function widget:Update()").unwrap();
		let up = source
			.find(r#"Spring.SendLuaMenuMsg("modlobbyWidget")"#)
			.unwrap();
		let gone = source
			.find(r#"Spring.SendLuaMenuMsg("modlobbyWidgetGone")"#)
			.unwrap();
		assert!(
			declared < shutdown,
			"a Lua local is declared before its use"
		);
		assert!(initialize < shutdown && shutdown < gone);
		assert!(update < up);
		assert!(!source[initialize..shutdown].contains("SendLuaMenuMsg"));
	}

	/// The menu says it too, and only one has to get through. This is the one
	/// for a game whose menu did not load.
	#[test]
	fn the_game_taking_the_keyboard_is_told_to_the_lobby() {
		let source = source(SOCKET, "x");
		let ask = source.find("local function ask(verb)").unwrap();
		let told = source
			.find("function widget:GamePreload()\n\task(\"loaded\")")
			.unwrap();
		assert!(ask < told, "a Lua local has to be declared before its use");
		// Only a press goes through the menu; this goes on to the file.
		assert!(source.contains(r#"if verb ~= "raise" or not menu_loaded() then"#));
	}

	/// The menu's name is what the widget looks for, lowercased the way BAR's
	/// top bar reads it, and the message is the one that button sends.
	#[test]
	fn the_menu_is_asked_with_the_lobby_buttons_own_message() {
		let source = source(SOCKET, "x");
		assert!(source.contains(&format!(
			r#"local MENU = "{}""#,
			super::super::menu::ARCHIVE_NAME.to_lowercase()
		)));
		assert!(source.contains("string.find(string.lower(name), MENU, 1, true)"));
		assert!(source.contains(r#"Spring.SendLuaMenuMsg("showLobby")"#));
	}

	/// A line still in the file means modlobby is not reading it: the key
	/// then stays the game's rather than vanishing into a file nobody reads.
	#[test]
	fn the_file_is_appended_to_only_while_someone_empties_it() {
		let source = source(SOCKET, "f11e");
		assert!(source.contains(&format!(r#"local FILE = "{FILE}""#)));
		assert!(source.contains(r#"io.open(FILE, "a")"#));
		assert!(source.contains(r#"FILE_TOKEN .. " " .. verb"#));
		let unread = source.find("if unread() then\n\t\treturn false").unwrap();
		let append = source.find(r#"io.open(FILE, "a")"#).unwrap();
		assert!(unread < append);
	}

	/// The way out of a game whose lobby died: BAR hides its own quit screen
	/// while a menu is loaded, so the engine's quit box is all that is left.
	#[test]
	fn a_dead_lobby_still_leaves_a_way_out_of_the_game() {
		let source = source(SOCKET, "x");
		assert!(source.contains("function widget:RecvLuaMsg"));
		assert!(source.contains(r#"msg ~= "modlobbyGone""#));
		assert!(source.contains(r#"Spring.SendCommands("QuitMenu")"#));
	}
}
