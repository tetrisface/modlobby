//! BAR tweaks: the `tweakdefs*` / `tweakunits*` modoptions carry base64url
//! Lua. This crate is the whole round trip — decode, format, edit, minify,
//! encode, size-check, diff — so no one needs an external encoder again.
//!
//! Two shapes, from `Beyond-All-Reason/gamedata/unitdefs_post.lua:231-310`:
//! `tweakdefs` is Lua **code** run with `UnitDefs` in scope; `tweakunits` is a
//! table constructor evaluated as `return <text>` and merged into existing
//! units. They differ in decoding too — see [`base64url`].

pub mod assist;
pub mod base64url;
pub mod check;
pub mod command;
pub mod diff;
pub mod gauge;
pub mod lua;
pub mod name;
pub mod outline;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use assist::{DefTags, Tag};
pub use base64url::Diagnostic;
pub use check::{Check, Problem, Symbol, check};
pub use command::Slot;
pub use diff::{ChangeOp, DiffView, Hunk};
pub use gauge::{CAP, Gauge};
pub use lua::Config;

/// Which of the two tweak shapes a payload is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Kind {
    /// `tweakdefs*`: Lua code, decoded raw.
    Defs,
    /// `tweakunits*`: a table constructor, decoded after `_` becomes `=`.
    Units,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not base64url: {0}")]
    Base64(String),
    #[error("not valid UTF-8")]
    Utf8,
    #[error("Lua: {0}")]
    Lua(String),
    /// Only tweakunits: the payload would decode to something else in game.
    #[error("{0}")]
    Underscore(String),
}

/// A slot's current value, ready to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TweakView {
    /// The decoded Lua, exactly as stored.
    pub lua: String,
    /// The same Lua through StyLua; equal to `lua` when it does not parse.
    pub formatted: String,
    /// The leading `--` comment, which is how BAR names a tweak.
    pub name: Option<String>,
    /// What Chobby's modoptions panel shows: `<length>:<hash>`.
    pub summary: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// Everything needed to send a tweak, and to decide whether it will fit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Prepared {
    pub minified: String,
    pub blob: String,
    /// The literal `!bSet <slot> <blob>`; only this form gets the server's 16 KB allowance.
    pub command: String,
    pub gauge: Gauge,
}

/// Decodes a stored slot value for display.
pub fn decode(blob: &str, kind: Kind, config: &Config) -> Result<TweakView, Error> {
    let decoded = base64url::decode(blob, kind)?;
    let lua = decoded.text;
    let formatted = lua::format(&lua, kind, config).unwrap_or_else(|_| lua.clone());
    Ok(TweakView {
        name: name::name(&lua),
        summary: name::summary(blob),
        formatted,
        lua,
        diagnostics: decoded.diagnostics,
    })
}

/// Minifies, encodes and measures — without sending anything.
pub fn prepare(lua: &str, slot: Slot, direct: bool) -> Result<Prepared, Error> {
    let kind = slot.kind();
    let minified = lua::minify(lua, kind)?;
    let blob = base64url::encode(&minified, kind)?;
    let command = if direct {
        command::bset(slot, &blob)
    } else {
        command::callvote(slot, &blob)
    };
    let gauge = Gauge::measure(lua, &minified, &blob, &command);
    Ok(Prepared {
        minified,
        blob,
        command,
        gauge,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole loop a user drives: edit Lua, prepare it, and read it back
    /// the way the room will show it.
    #[test]
    fn prepare_then_decode_survives_the_round_trip() {
        // The header shape people actually publish: a name, then credit,
        // then where it is written up.
        let lua = "-- Sphere spawner v3
-- Authors: someone
local count = 4
for i = 1, count do
  UnitDefs.armcom.metalcost = UnitDefs.armcom.metalcost - i -- inline
end
";
        let slot = Slot::Defs(1);
        let prepared = prepare(lua, slot, true).unwrap();

        assert!(prepared.command.starts_with("!bSet tweakdefs1 "));
        assert!(prepared.gauge.fits);
        assert!(prepared.gauge.minified < prepared.gauge.raw);

        let view = decode(&prepared.blob, slot.kind(), &Config::default()).unwrap();
        assert_eq!(view.name.as_deref(), Some("Sphere spawner v3"));
        assert!(view.diagnostics.is_empty());
        // The header survives whole; a comment inside the body does not.
        assert!(view.lua.contains("Authors: someone"), "credit is not noise");
        assert!(!view.lua.contains("inline"), "comments cost bytes");
        assert!(view.formatted.contains("UnitDefs.armcom.metalcost"));
        // Formatting is display only: what we would send again is unchanged.
        assert_eq!(
            prepare(&view.formatted, slot, true).unwrap().blob,
            prepared.blob
        );
    }

    #[test]
    fn oversized_payloads_report_the_overflow_rather_than_truncating() {
        let lua = format!("local t = '{}'\n", "x".repeat(20_000));
        let prepared = prepare(&lua, Slot::Units(2), true).unwrap();
        assert!(!prepared.gauge.fits);
        assert!(prepared.gauge.command > prepared.gauge.cap);
    }
}
