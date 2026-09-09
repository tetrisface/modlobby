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
//! The archive is generated rather than shipped for the same reason the widget
//! is: the port and token change every run.

use std::path::{Path, PathBuf};

/// Must contain `chobby`, lowercased, or BAR offers "Quit" instead
/// (`gui_top_bar.lua:3574`). The rest of the name says whose it really is.
pub const ARCHIVE_NAME: &str = "modlobby chobby shim";

/// The archive's version, which is half of how the engine indexes it.
const ARCHIVE_VERSION: &str = env!("CARGO_PKG_VERSION");

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

/// The menu itself: no window, no drawing, two messages.
///
/// `LuaMenu/main.lua` is the entry point the engine loads (`LuaMenu.cpp:59`),
/// and its call-ins are plain globals rather than methods on a table.
fn main_lua(port: u16, token: &str) -> String {
    format!(
        r#"-- modlobby: BAR's in-game Lobby button.
--
-- Written by modlobby when it launches a game and removed when it exits. It
-- has no interface of its own. BAR offers a "Lobby" button in place of "Quit"
-- when a menu is loaded, and this turns that button into modlobby's overlay.
--
-- If modlobby is not listening it quits the game rather than stranding anyone
-- in a menu that cannot draw itself.

local PORT = {port}
local TOKEN = "{token}"

local socket = socket

local function ask(verb)
	if not socket then return false end
	local tcp = socket.tcp()
	if not tcp then return false end
	tcp:settimeout(0.2)
	if not tcp:connect("127.0.0.1", PORT) then
		tcp:close()
		return false
	end
	tcp:send(TOKEN .. " " .. verb .. "\n")
	local reply = tcp:receive("*l")
	tcp:close()
	return reply == "ok"
end

-- BAR sends this from the top bar while the game is still running, which is
-- the whole point: the lobby comes up over it rather than instead of it.
function RecvLuaMsg(msg)
	if msg == "showLobby" then
		return ask("raise")
	end
	-- "disableLobbyButton" is BAR telling a real lobby to hide its own button.
	-- There is no button here to hide.
	return false
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
"#
    )
}

/// Writes the archive, replacing any older one. The archive's name, for `--menu`.
pub fn install(data_dir: &Path, port: u16, token: &str) -> std::io::Result<PathBuf> {
    let root = dir(data_dir);
    std::fs::create_dir_all(root.join("LuaMenu"))?;
    std::fs::write(root.join("modinfo.lua"), modinfo())?;
    std::fs::write(root.join("LuaMenu").join("main.lua"), main_lua(port, token))?;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn it_is_written_where_the_scanner_looks() {
        let root = std::env::temp_dir().join("modlobby-menu-test");
        let _ = std::fs::remove_dir_all(&root);

        let written = install(&root, 1234, "secret").unwrap();

        // `games/` is one of the four roots `GetDataDirRoots` returns.
        assert_eq!(written.parent().unwrap().file_name().unwrap(), "games");
        // `modinfo.lua` is what the scanner indexes, and `recoil::MenuArchive`
        // refuses to pass `--menu` without one.
        assert!(written.join("modinfo.lua").is_file());

        let lua = std::fs::read_to_string(written.join("LuaMenu").join("main.lua")).unwrap();
        assert!(lua.contains("local PORT = 1234"));
        assert!(lua.contains(r#"local TOKEN = "secret""#));
        assert!(lua.contains("function RecvLuaMsg"));
        assert!(lua.contains("function ActivateMenu"));
        // The one quit call a LuaMenu has. `SendCommands` is not in its
        // environment, and the engine sat black on the error.
        assert!(lua.contains("Spring.Quit()"));
        assert!(!lua.contains("SendCommands("));

        let info = std::fs::read_to_string(written.join("modinfo.lua")).unwrap();
        assert!(info.contains("modtype = 5"), "a menu, not a game");
        assert!(info.contains(ARCHIVE_NAME));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
