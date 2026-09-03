//! The settings shape. Every field has a default so a partial file is valid;
//! unknown keys are kept on disk and ignored here.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const SCHEMA_FILE: &str = "settings.schema.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Settings {
    /// Points editors at the schema next to the file.
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub server: Server,
    pub account: Account,
    pub connection: Connection,
    pub paths: Paths,
    pub battle_list: BattleList,
    pub chat: Chat,
    pub notifications: Notifications,
    pub overlay: Overlay,
    pub play: Play,
    pub tweaks: Tweaks,
    pub logging: Logging,
    pub updates: Updates,
}

impl Settings {
    /// What a fresh install gets: the defaults plus the schema pointer.
    pub fn initial() -> Self {
        Self {
            schema: Some(format!("./{SCHEMA_FILE}")),
            ..Self::default()
        }
    }
}

/// Which teiserver to talk to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Server {
    pub host: String,
    pub port: u16,
    /// teiserver speaks TLS on 8201 and plain TCP on 8200.
    pub tls: bool,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            host: "server4.beyondallreason.info".into(),
            port: 8201,
            tls: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Account {
    pub username: String,
    /// Keep the password in the OS keyring between runs; it is never written here.
    pub remember_password: bool,
    /// Log in on startup with the remembered password. Without one there is
    /// nothing to log in with, so this does nothing on its own.
    pub auto_login: bool,
}

/// Holding on to the server, and letting it go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Connection {
    /// Minutes without a key pressed or a click in the window before the
    /// connection is dropped and not retried; `0` keeps it however long.
    ///
    /// A dropped connection otherwise comes back on its own, which is right
    /// for a window in use and wrong for one that was forgotten: it holds a
    /// seat in a room for nobody, and keeps the account logged in from a
    /// machine its owner may have left. A running game never counts as idle,
    /// whatever the lobby window sees. The window stays open, one click from
    /// logging in again.
    pub idle_disconnect_minutes: u32,
}

impl Default for Connection {
    fn default() -> Self {
        Self {
            // Long enough to read a thread of chat without touching anything;
            // short enough that a lobby left overnight is gone by morning.
            idle_disconnect_minutes: 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Paths {
    /// The BAR data directory (`engine/`, `games/`, `maps/`); the launcher's when unset.
    pub data_dir: Option<PathBuf>,
}

/// Which rooms the list shows. Chobby words these the other way round, as
/// "Filter out:" checkboxes, where ticking one means seeing less; stated
/// positively, a toggle that is on means that kind of room is in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct BattleList {
    pub show_passworded: bool,
    pub show_locked: bool,
    /// Rooms nobody has joined yet.
    pub show_empty: bool,
    /// Rooms whose game has already started.
    pub show_running: bool,
    /// Narrow the list to rooms with a friend in them. Off by default: it
    /// empties the list for anyone who has not added anybody.
    pub friends_only: bool,
    pub mode: ModeFilter,
    pub sort: BattleSort,
    /// Largest or latest first. Ignored by `BattleSort::Relevance`, which has
    /// a fixed order of its own.
    pub sort_descending: bool,
}

impl Default for BattleList {
    /// Everything, in Chobby's order. Narrowing the list is a choice someone
    /// makes, never the state they are dropped into.
    fn default() -> Self {
        Self {
            show_passworded: true,
            show_locked: true,
            show_empty: true,
            show_running: true,
            friends_only: false,
            mode: ModeFilter::default(),
            sort: BattleSort::default(),
            sort_descending: false,
        }
    }
}

/// Player-versus-what. Read off the room title, which is all the list has:
/// the server only sends a room's AI once you are in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ModeFilter {
    #[default]
    All,
    /// Only rooms that look like they are against AI.
    Pve,
    /// Only rooms that do not.
    Pvp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BattleSort {
    /// Chobby's own order: joinable first, then busy, then locked, then
    /// passworded, with player count deciding inside each band.
    #[default]
    Relevance,
    Players,
    Title,
    Map,
    Host,
}

/// What sitting down in a room you just joined should mean.
///
/// Chobby's three, under the same names it gives them
/// (`gui_settings_window.lua:906`): remember what you did last time, or always
/// one or the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JoinAs {
    Remember,
    Spectator,
    Player,
}

/// Playing rather than watching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Play {
    /// Whether a seat may be taken in a public room.
    ///
    /// On: this is a lobby, and sitting down in a room is what it is for. It
    /// stays a setting because a client driving the protocol without a person
    /// behind it should be able to say it is only watching.
    pub in_public_rooms: bool,
    /// Whether joining a room seats you.
    pub join_as: JoinAs,
    /// Whether the engine starts on its own when your room's game does.
    ///
    /// On, as it is in Chobby (`gui_settings_window.lua:888`), because a game
    /// you are in starting is the moment you want to be in it — spectating
    /// included, which is the case that otherwise means watching the room and
    /// pressing a button. Only ever fires when the content is already on disk.
    pub auto_launch: bool,
    /// Whether to ask BAR's PvE Stats service what a PvE room scores.
    ///
    /// On, because the number is the point of looking at a PvE room before
    /// joining it. It sends the map, the settings and the team size to a
    /// third-party service — never a name or an account — so it stays
    /// something that can be turned off.
    pub pve_stats: bool,
    /// What [`JoinAs::Remember`] remembers: whether you played last time.
    /// Written when you take or leave a seat, never chosen directly.
    pub last_was_player: bool,
}

impl Default for Play {
    fn default() -> Self {
        Self {
            in_public_rooms: true,
            join_as: JoinAs::Remember,
            auto_launch: true,
            pve_stats: true,
            // Nothing remembered yet, and this is a lobby.
            last_was_player: true,
        }
    }
}

/// The lobby raised over a running game by a hotkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Overlay {
    pub enabled: bool,
    /// A Tauri accelerator, e.g. `Alt+Shift+L`.
    ///
    /// A global hotkey beats the focused window, so a collision does not
    /// merely conflict — it silently eats a game action for as long as a game
    /// is running. The default was checked against BAR's shipped binds: `sc_l`
    /// is taken plain and with Shift (cycling fire state), and the whole game
    /// binds only two Alt+Shift combinations, neither of them this one.
    pub hotkey: String,
    /// Whether hiding the overlay should put the game back in front.
    pub return_focus_to_game: bool,
    /// Whether Escape inside a game should raise the lobby.
    ///
    /// The engine gives an outside program no way to see Escape, so this is
    /// the one feature that puts a file of ours in the BAR data directory: a
    /// small widget in `LuaUI/Widgets/`. It draws nothing, it is removed when
    /// modlobby exits, and it leaves the key alone unless modlobby answers —
    /// so a game launched from Chobby behaves exactly as it always did. It is
    /// still someone else's directory, which is why it is a setting.
    pub in_game_escape: bool,
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            enabled: true,
            hotkey: "Alt+Shift+L".into(),
            return_focus_to_game: true,
            in_game_escape: true,
        }
    }
}

/// How loudly to say that something happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Alert {
    /// Say nothing.
    Off,
    /// A message in the lobby's own corner, which you see when you look.
    Lobby,
    /// A desktop notification and a flashing taskbar entry, and only while
    /// the window is in the background: a toast for something already on
    /// screen is noise, which is the line Chobby draws too. Never the
    /// lobby's corner — that is what `Lobby` is for, and a choice that did
    /// both would not be a choice.
    Desktop,
}

/// Accepts the `true`/`false` this used to be.
///
/// Every setting under `notifications` was a boolean before there was anywhere
/// but the desktop to put one. A file written then must keep working: an
/// unreadable value here is not a field that falls back to its default, it is
/// a settings file that will not parse at all.
impl<'de> Deserialize<'de> for Alert {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Written {
            Named(String),
            Legacy(bool),
        }

        Ok(match Written::deserialize(deserializer)? {
            Written::Legacy(true) => Alert::Desktop,
            Written::Legacy(false) => Alert::Off,
            Written::Named(name) => match name.to_ascii_lowercase().as_str() {
                "off" | "none" | "false" => Alert::Off,
                "lobby" => Alert::Lobby,
                _ => Alert::Desktop,
            },
        })
    }
}

/// What is worth saying something about, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Notifications {
    /// Someone messaged you directly.
    pub private_message: Alert,
    /// Someone said your name in a channel or in your room.
    pub mention: Alert,
    /// Someone rang you.
    pub ring: Alert,
    /// A friend logged in.
    pub friend_online: Alert,
    /// A vote opened in your room.
    pub vote: Alert,
    /// Your room's game started.
    pub game_starting: Alert,
    /// Your room's game finished.
    pub game_ended: Alert,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            // Addressed to you by name: worth pulling you back for.
            private_message: Alert::Desktop,
            mention: Alert::Desktop,
            ring: Alert::Desktop,
            game_starting: Alert::Desktop,
            // True, but not worth taking over the screen for.
            friend_online: Alert::Lobby,
            vote: Alert::Lobby,
            game_ended: Alert::Lobby,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Chat {
    /// Whether to drop the host's machine-readable lines rather than show them.
    ///
    /// SPADS rides structured state on battle chat as `BarManager|{…}`, which
    /// this already parses into the room. Chobby hides the same lines behind
    /// the same default, calling it "filter bot chatter"
    /// (`gui_settings_window.lua:1146`).
    pub filter_host_chatter: bool,
    /// Lines kept per room before the oldest are dropped.
    pub max_lines: u32,
    /// Channels to rejoin at login. The server forgets you were in them the
    /// moment you disconnect, so remembering is the client's job — and keeping
    /// it here means you can also just write one in.
    pub channels: Vec<String>,
}

impl Default for Chat {
    fn default() -> Self {
        Self {
            filter_host_chatter: true,
            max_lines: 3000,
            // Where the server puts everyone, and where the announcements are.
            channels: vec!["main".into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Tweaks {
    /// A `stylua.toml` to format tweak Lua with; StyLua's defaults when unset.
    pub stylua_config: Option<PathBuf>,
    /// Slot offered when exporting a tweak, e.g. `tweakdefs1`.
    pub default_slot: String,
}

impl Default for Tweaks {
    fn default() -> Self {
        Self {
            stylua_config: None,
            default_slot: "tweakdefs1".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Logging {
    /// A `tracing` filter, e.g. `info,spring::rx=trace`.
    pub filter: String,
}

impl Default for Logging {
    fn default() -> Self {
        Self {
            filter: "info".into(),
        }
    }
}

/// Keeping the app current.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Updates {
    /// Whether to fetch the newest release on startup and restart into it.
    ///
    /// On, because an out-of-date lobby is one that quietly talks to a server
    /// that has moved on, and startup is the one moment a restart costs
    /// nothing. An update found while a room is joined or a game is running
    /// waits in the nav for a click, whatever this says.
    pub automatic: bool,
}

impl Default for Updates {
    fn default() -> Self {
        Self { automatic: true }
    }
}

/// The JSON Schema editors use for completion, pretty-printed.
pub fn schema_json() -> String {
    let schema = schemars::schema_for!(Settings);
    serde_json::to_string_pretty(&schema).expect("schema serialises")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_fills_in_defaults() {
        let s: Settings = serde_json::from_str(r#"{"chat":{"maxLines":7}}"#).unwrap();
        assert_eq!(s.chat.max_lines, 7);
        assert_eq!(s.server, Server::default());
        assert_eq!(s.tweaks.default_slot, "tweakdefs1");
        assert_eq!(s.connection.idle_disconnect_minutes, 60);
    }

    #[test]
    fn a_file_written_before_alerts_had_places_still_parses() {
        // What every one of these was until there was somewhere other than the
        // desktop to put a notification. A file like this must not be the
        // reason the app refuses to start.
        let s: Settings = serde_json::from_str(
            r#"{"notifications":{"enabled":true,"privateMessage":true,"vote":false}}"#,
        )
        .unwrap();
        assert_eq!(s.notifications.private_message, Alert::Desktop);
        assert_eq!(s.notifications.vote, Alert::Off);
        // A field that was never written keeps its default rather than the
        // reading of whatever the file happened to say about its neighbours.
        assert_eq!(s.notifications.game_ended, Alert::Lobby);
    }

    #[test]
    fn a_place_is_read_by_name_however_it_is_written() {
        let s: Settings = serde_json::from_str(
            r#"{"notifications":{"mention":"lobby","ring":"OFF","vote":"desktop"}}"#,
        )
        .unwrap();
        assert_eq!(s.notifications.mention, Alert::Lobby);
        assert_eq!(s.notifications.ring, Alert::Off);
        assert_eq!(s.notifications.vote, Alert::Desktop);
    }

    /// `schema/settings.schema.json` is what editors read; keep it in sync.
    /// Regenerate with `SETTINGS_WRITE_SCHEMA=1 cargo test -p settings`.
    #[test]
    fn committed_schema_is_current() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/schema/",
            "settings.schema.json"
        );
        let current = schema_json();
        if std::env::var_os("SETTINGS_WRITE_SCHEMA").is_some() {
            std::fs::write(path, &current).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(path).unwrap_or_default();
        assert_eq!(
            committed.trim(),
            current.trim(),
            "schema drifted; regenerate"
        );
    }
}
