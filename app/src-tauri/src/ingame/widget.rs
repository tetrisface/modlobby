//! The Lua we write into the game, generated so the port and token are ours.

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

/// The widget, with this run's port and token in it.
///
/// Written rather than shipped as a static file because the token changes
/// every run: a widget left behind by a crash is not merely inert, it is
/// talking to a port that no longer answers with a secret that no longer works.
pub fn source(port: u16, token: &str) -> String {
    // `{port}` and `{token}` are the only things interpolated; the rest is
    // literal Lua, kept readable so anyone can see what we put in their game.
    format!(
        r#"-- modlobby: Escape opens the lobby.
--
-- Written by modlobby when it launches a game and removed when it exits. It
-- draws nothing and sends nothing but a keypress notification to a loopback
-- port that modlobby is listening on, guarded by a token generated for this
-- run.
--
-- If modlobby is not listening it does nothing at all -- including not
-- consuming the key -- so a game launched from any other lobby behaves
-- exactly as it would without this file.

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

local PORT = {port}
local TOKEN = "{token}"

local socket = socket
local GetSelectedUnitsCount = Spring.GetSelectedUnitsCount

local KEY_ESCAPE = 27
local connection = nil

-- Connecting costs a round trip on loopback, which is nothing, but doing it
-- inside the keypress would still stall a frame if the port were gone. The
-- connection is kept and only rebuilt when it breaks.
local function connect()
	if not socket then
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

-- Sends one line and waits briefly for the answer.
--
-- The answer is the point. modlobby replies "ok" only when the game we are in
-- is one it launched and the overlay is switched on; anything else is "no",
-- and the key goes back to the game untouched. Waiting costs a loopback round
-- trip on an Escape press, which is not a frame anyone will notice.
local function ask(verb)
	if not connection then
		connection = connect()
	end
	if not connection then
		return false
	end

	local sent = connection:send(TOKEN .. " " .. verb .. "\n")
	if not sent then
		-- The lobby was restarted under us; one reconnect, then give up.
		connection:close()
		connection = connect()
		if not connection then
			return false
		end
		sent = connection:send(TOKEN .. " " .. verb .. "\n")
		if not sent then
			return false
		end
	end

	local reply = connection:receive("*l")
	if not reply then
		connection:close()
		connection = nil
		return false
	end
	return reply == "ok"
end

function widget:Initialize()
	connection = connect()
	if not connection then
		-- Nothing to talk to. Staying loaded is harmless and lets a lobby
		-- started after the game still be reached on the next press.
		Spring.Echo("modlobby: not running; Escape left alone")
	end
end

function widget:Shutdown()
	if connection then
		connection:close()
		connection = nil
	end
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
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_lands_where_bar_looks_for_user_widgets() {
        let path = path(Path::new("C:/bar"));
        assert!(path.ends_with("LuaUI/Widgets/modlobby_escape.lua"));
    }

    #[test]
    fn the_port_and_token_are_this_runs() {
        let source = source(51234, "cafebabe");
        assert!(source.contains("local PORT = 51234"));
        assert!(source.contains(r#"local TOKEN = "cafebabe""#));
    }

    #[test]
    fn the_braces_survive_being_a_format_string() {
        // `GetInfo` returns a table, and a `{` that did not make it through
        // would be a Lua syntax error the game reports and we never see.
        let source = source(1, "x");
        assert!(source.contains("return {\n\t\tname = \"modlobby Escape\""));
        assert!(!source.contains("{{"), "no doubled braces left over");
    }

    #[test]
    fn escape_is_only_taken_when_it_would_do_nothing_else() {
        let source = source(1, "x");
        // The three guards that keep this from stealing the game's own key.
        assert!(source.contains("GetSelectedUnitsCount() > 0"));
        assert!(source.contains("isRepeat"));
        assert!(source.contains("return ask(\"raise\")"));
    }
}
