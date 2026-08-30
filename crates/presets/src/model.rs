//! What a saved room setup is.
//!
//! The five sections are Chobby's, under Chobby's names, because the file this
//! interoperates with is Chobby's: `Map`, `Modoptions`,
//! `Multiplayer Battle Settings`, `Start Boxes` and `Bots`
//! (`gui_optionpresets_panel.lua:505-556`). What is ours is everything around
//! them — when a preset was made, when it last changed, and when it was last
//! used, which is the order anybody actually wants their presets in.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Unix seconds. Stored rather than derived from file times, which a copy, a
/// sync client or a backup would each quietly rewrite.
pub type Stamp = u64;

/// One saved setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Preset {
    pub name: String,
    /// The map's full name, as the battle list spells it.
    pub map: Option<String>,
    /// `!bSet` keys and their values, as strings because that is what goes on
    /// the wire. A number or a boolean in someone else's file is normalised on
    /// the way in and put back on the way out — see [`crate::chobby`].
    pub modoptions: BTreeMap<String, String>,
    /// SPADS room settings: preset, teamSize, nbTeams, autoBalance,
    /// balanceMode, locked.
    pub battle: BTreeMap<String, String>,
    /// Ally team number to its box. Kept sparse: a preset can name team 2
    /// without naming team 1, and Chobby's own file does.
    pub start_boxes: BTreeMap<u8, StartBox>,
    /// AI slots, carried through exactly as they were written.
    ///
    /// Opaque on purpose. Their shape is whatever Chobby's `AddAi` accepted at
    /// the time — library, version, ally number, colour, handicap, and a bag of
    /// AI options — and re-encoding a structure we do not fully model is how
    /// you lose somebody's carefully tuned bot setup. Round-tripped verbatim
    /// until there is a reason to read it.
    #[ts(type = "Record<string, unknown>")]
    pub bots: serde_json::Map<String, serde_json::Value>,
    pub created: Stamp,
    pub updated: Stamp,
    /// `None` until it has been applied to a room once.
    pub last_used: Option<Stamp>,
}

/// A start box in the coordinates `ADDSTARTRECT` uses: 0-200 on both axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartBox {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

impl Preset {
    /// A new preset, made now.
    pub fn new(name: impl Into<String>, now: Stamp) -> Self {
        Self {
            name: name.into(),
            map: None,
            modoptions: BTreeMap::new(),
            battle: BTreeMap::new(),
            start_boxes: BTreeMap::new(),
            bots: serde_json::Map::new(),
            created: now,
            updated: now,
            last_used: None,
        }
    }

    /// How many `!bSet` lines applying this would send, before anything is
    /// skipped for already matching. Shown in the table, where "how big is
    /// this preset" is the question a name alone does not answer.
    pub fn option_count(&self) -> usize {
        self.modoptions.len()
    }

    /// Tweak slots carry Lua, and are the reason a preset can be a megabyte.
    pub fn tweak_count(&self) -> usize {
        self.modoptions
            .keys()
            .filter(|key| key.starts_with("tweakdefs") || key.starts_with("tweakunits"))
            .filter(|key| self.modoptions.get(*key).is_some_and(|v| v.len() > 1))
            .count()
    }
}

/// The file we keep, with a version so a future shape can be told from this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub version: u32,
    pub presets: Vec<Preset>,
}

pub const VERSION: u32 = 1;

impl Default for Book {
    fn default() -> Self {
        Self {
            version: VERSION,
            presets: Vec::new(),
        }
    }
}
