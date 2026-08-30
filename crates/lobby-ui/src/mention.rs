//! Whether a line is talking to you.
//!
//! Chat moves fast enough in a busy channel that a line naming you goes past
//! unseen, which is how people miss "shall we start?" addressed to them by
//! name. Everything here is a plain string question so it can be tested
//! without a server.

/// Whether `text` names `me`.
///
/// Case-insensitive, because nobody types a name the way it is registered, and
/// bounded by what a name can contain: BAR names are letters, digits and
/// underscores, plus the brackets clans wrap around them. A match flanked by
/// one of those characters is part of a longer word — `Sky` should not light
/// up for `Skywalker` — while punctuation, whitespace and the ends of the line
/// all count as boundaries, so `Sky:`, `@Sky` and `(Sky)` all land.
pub fn mentions(text: &str, me: &str) -> bool {
    if me.is_empty() {
        return false;
    }
    let haystack = text.to_lowercase();
    let needle = me.to_lowercase();
    let bytes = haystack.as_bytes();

    haystack.match_indices(&needle).any(|(at, hit)| {
        let before = bytes[..at].iter().next_back().copied();
        let after = bytes.get(at + hit.len()).copied();
        !part_of_a_word(before) && !part_of_a_word(after)
    })
}

fn part_of_a_word(byte: Option<u8>) -> bool {
    byte.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_found_however_it_is_typed() {
        assert!(mentions("sky are you ready?", "Sky"));
        assert!(mentions("SKY?", "sky"));
        assert!(mentions("hey Sky", "Sky"));
        assert!(mentions("@Sky, start it", "Sky"));
        assert!(mentions("(sky) go", "Sky"));
    }

    #[test]
    fn a_longer_word_containing_the_name_is_not_a_mention() {
        assert!(!mentions("skywalker is afk", "Sky"));
        assert!(!mentions("bluesky", "Sky"));
        assert!(!mentions("sky_bot said no", "Sky"));
    }

    #[test]
    fn a_clan_tag_is_part_of_the_name_and_still_matches_around_it() {
        assert!(mentions("nice one [CoW]Sky", "[CoW]Sky"));
        // The bare name still counts: people drop the tag when talking to you.
        assert!(mentions("sky gg", "Sky"));
    }

    #[test]
    fn nothing_matches_when_we_do_not_know_who_we_are() {
        assert!(!mentions("anything at all", ""));
    }
}
