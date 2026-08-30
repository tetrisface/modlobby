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
    pub paths: Paths,
    pub battle_list: BattleList,
    pub chat: Chat,
    pub notifications: Notifications,
    pub play: Play,
    pub tweaks: Tweaks,
    pub logging: Logging,
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

/// Playing rather than watching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Play {
    /// Whether a seat may be taken in a public room.
    ///
    /// Off by default, and deliberately so: in a public room a slot belongs to
    /// a real player waiting for a game, and a client that takes one to try
    /// something out has spoiled someone's evening. Turn it on when you mean
    /// to play; a room of your own (`!privatehost`) never needs it.
    pub in_public_rooms: bool,
}

/// What is worth interrupting someone for.
///
/// Only raised while the window is not focused: a toast for something already
/// on screen is noise. Chobby draws the same line, alerting only when its
/// window is in the background.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Notifications {
    pub enabled: bool,
    /// Someone messaged you directly.
    pub private_message: bool,
    /// Someone said your name in a channel or in your room.
    pub mention: bool,
    /// A friend logged in.
    pub friend_online: bool,
    /// A vote opened in your room.
    pub vote: bool,
    /// Your room's game started.
    pub game_starting: bool,
    /// Someone rang you.
    pub ring: bool,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            enabled: true,
            private_message: true,
            mention: true,
            friend_online: true,
            vote: true,
            game_starting: true,
            ring: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Chat {
    /// Lines kept per room before the oldest are dropped.
    pub max_lines: u32,
}

impl Default for Chat {
    fn default() -> Self {
        Self { max_lines: 500 }
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
