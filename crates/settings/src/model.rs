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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Paths {
    /// The BAR data directory (`engine/`, `games/`, `maps/`); the launcher's when unset.
    pub data_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct BattleList {
    pub hide_passworded: bool,
    pub hide_locked: bool,
    pub hide_empty: bool,
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
