//! Will it fit? teiserver keeps only the first 16 385 characters of a
//! `!bSet tweak…` chat line (`spring_in.ex:1244-1258`, an inclusive slice), and
//! silently truncates the rest — which corrupts the base64 without any error.
//! The cap covers the whole command, slot name included.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Characters teiserver keeps of a `!bSet tweakdefs…` / `!bSet tweakunits…` line.
pub const CAP: usize = 16_385;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Gauge {
    /// Bytes of the Lua as edited.
    pub raw: usize,
    /// Bytes after minification.
    pub minified: usize,
    /// Characters of the base64url payload.
    pub blob: usize,
    /// Characters of the whole chat command — what the cap applies to.
    pub command: usize,
    pub cap: usize,
    pub fits: bool,
}

impl Gauge {
    pub fn measure(raw: &str, minified: &str, blob: &str, command: &str) -> Self {
        let length = command.chars().count();
        Self {
            raw: raw.len(),
            minified: minified.len(),
            blob: blob.chars().count(),
            command: length,
            cap: spring_protocol::policy::saybattle_max_len(command),
            fits: length <= spring_protocol::policy::saybattle_max_len(command),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kind, Slot, command};

    #[test]
    fn the_cap_covers_the_whole_command_at_exactly_16385() {
        let slot = Slot::Defs(1);
        let prefix = command::bset(slot, "");
        let blob = "A".repeat(CAP - prefix.chars().count());
        let command = command::bset(slot, &blob);
        assert_eq!(command.chars().count(), CAP);
        let gauge = Gauge::measure("", "", &blob, &command);
        assert_eq!(gauge.cap, CAP);
        assert!(gauge.fits);

        let over = command::bset(slot, &format!("{blob}A"));
        assert!(!Gauge::measure("", "", "", &over).fits);
    }

    /// A vote is capped at 257 characters: the allowance is only for `!bSet`.
    #[test]
    fn a_callvote_gets_the_short_allowance() {
        let blob = "A".repeat(300);
        let vote = command::callvote(Slot::Units(0), &blob);
        let gauge = Gauge::measure("", "", &blob, &vote);
        assert_eq!(gauge.cap, 257);
        assert!(!gauge.fits);
        assert_eq!(Slot::Units(0).kind(), Kind::Units);
    }
}
