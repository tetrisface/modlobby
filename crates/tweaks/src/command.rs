//! The chat commands that carry a tweak. Twenty slots exist
//! (`Beyond-All-Reason/modoptions.lua:2697-2769`); the game applies every
//! `tweakdefs*` before every `tweakunits*`, ascending, with the unnumbered one
//! first (`unitdefs_post.lua:242-258`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::Kind;

/// One of the twenty modoption slots; `0` is the unnumbered `tweakdefs` / `tweakunits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "index", rename_all = "camelCase")]
#[ts(export)]
pub enum Slot {
    Defs(u8),
    Units(u8),
}

impl Slot {
    /// Every slot, in the order the game applies them.
    pub fn all() -> Vec<Slot> {
        (0..=9)
            .map(Slot::Defs)
            .chain((0..=9).map(Slot::Units))
            .collect()
    }

    pub fn kind(self) -> Kind {
        match self {
            Slot::Defs(_) => Kind::Defs,
            Slot::Units(_) => Kind::Units,
        }
    }

    /// The modoption key, e.g. `tweakdefs` or `tweakunits3`.
    pub fn key(self) -> String {
        let (name, index) = match self {
            Slot::Defs(index) => ("tweakdefs", index),
            Slot::Units(index) => ("tweakunits", index),
        };
        match index {
            0 => name.to_owned(),
            index => format!("{name}{index}"),
        }
    }

    /// Parses a modoption key, with or without the `game/modoptions/` prefix.
    pub fn parse(key: &str) -> Option<Slot> {
        let key = key.rsplit('/').next()?.to_ascii_lowercase();
        let (make, rest) = if let Some(rest) = key.strip_prefix("tweakdefs") {
            (Slot::Defs as fn(u8) -> Slot, rest)
        } else {
            (
                Slot::Units as fn(u8) -> Slot,
                key.strip_prefix("tweakunits")?,
            )
        };
        let index = if rest.is_empty() {
            0
        } else {
            rest.parse().ok().filter(|i| (1..=9).contains(i))?
        };
        Some(make(index))
    }
}

/// `!bSet <slot> <blob>` — the only form teiserver gives the 16 KB allowance,
/// and the casing Chobby uses (`liblobby/lobby/interface.lua:509-517`).
pub fn bset(slot: Slot, blob: &str) -> String {
    format!("!bSet {} {blob}", slot.key())
}

/// `!callvote bSet …` for a room where we may not set it directly. Note this
/// form is *not* given the big allowance; [`crate::Gauge`] reports the real cap.
pub fn callvote(slot: Slot, blob: &str) -> String {
    format!("!callvote bSet {} {blob}", slot.key())
}

/// Clearing a slot: Chobby sends the literal `0` (`gui_modoptions_panel.lua:493-495`).
pub fn clear(slot: Slot) -> String {
    format!("!bSet {} 0", slot.key())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_the_modoptions_and_parse_back() {
        assert_eq!(Slot::Defs(0).key(), "tweakdefs");
        assert_eq!(Slot::Units(9).key(), "tweakunits9");
        assert_eq!(Slot::all().len(), 20);
        for slot in Slot::all() {
            assert_eq!(Slot::parse(&slot.key()), Some(slot));
            assert_eq!(
                Slot::parse(&format!("game/modoptions/{}", slot.key().to_uppercase())),
                Some(slot)
            );
        }
        assert_eq!(Slot::parse("tweakdefs10"), None);
        assert_eq!(Slot::parse("map_tweaklava"), None);
    }

    #[test]
    fn commands_are_what_chobby_sends() {
        assert_eq!(bset(Slot::Defs(1), "QUJD"), "!bSet tweakdefs1 QUJD");
        assert_eq!(
            callvote(Slot::Units(0), "QUJD"),
            "!callvote bSet tweakunits QUJD"
        );
        assert_eq!(clear(Slot::Defs(0)), "!bSet tweakdefs 0");
    }
}
