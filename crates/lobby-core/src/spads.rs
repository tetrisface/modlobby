//! What the host bot says. SPADS has no structured protocol for votes or
//! setting changes — it announces them as `SAIDBATTLEEX` text, and every
//! client parses the strings (Chobby does it at
//! `gui_battle_room_window.lua:4720-4802`). The formats below are the literal
//! ones from `SPADS/src/spads.pl`, so a change there is a change here.
//!
//! Only the room's founder is trusted to make these announcements.

/// One thing the host told the room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Announcement {
    /// `* <user> called a vote for command "<cmd>" [!vote y, !vote n, !vote b]` (`spads.pl:8419`).
    VoteCalled { by: String, command: String },
    /// `* Vote in progress: "<cmd>" [y:1/2, n:0/1(2)] (25s remaining)` (`spads.pl:4015`).
    VoteProgress {
        command: String,
        yes: u32,
        yes_needed: u32,
        no: u32,
        no_needed: u32,
        remaining_secs: u32,
    },
    /// `* Vote for command "<cmd>" passed.|failed.` (`spads.pl:3603-3653`).
    VoteEnded { command: String, passed: bool },
    /// `* Vote cancelled by <user>` (`:8700`), or the vote's command being run
    /// directly (`:3244`), or the game starting.
    VoteCancelled,
    /// `* Battle setting changed by <user> (<key>=<value>)` (`spads.pl:8236`).
    /// The only signal when a slot is cleared: SPADS suppresses the
    /// `SETSCRIPTTAGS` for an empty value (`spads.pl:2625-2628`).
    SettingChanged {
        by: String,
        key: String,
        value: String,
    },
    /// `* BarManager|{…}`: the structured side-channel the BAR plugin adds.
    BarManager { json: String },
}

/// Who SPADS says is bossing the room, from a `BattleStateChanged` payload.
///
/// The BAR plugin reports the room's state as JSON, and `boss` is the one
/// field that says whose room it is: an empty autohost makes the first person
/// to speak up its boss, and a boss may change every setting in it. An empty
/// string means nobody.
pub fn boss(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let boss = value
        .get("BattleStateChanged")?
        .get("boss")?
        .as_str()?
        .trim();
    (!boss.is_empty()).then(|| boss.to_owned())
}

/// Parses one `SAIDBATTLEEX` line from the founder. Everything SPADS says is
/// prefixed `* ` (`spads.pl:2932`); anything unrecognised stays chat.
pub fn parse(text: &str) -> Option<Announcement> {
    let body = text.strip_prefix("* ")?;

    if let Some(json) = body.strip_prefix("BarManager|") {
        return Some(Announcement::BarManager { json: json.into() });
    }
    if let Some(rest) = body.strip_prefix("Vote in progress: ") {
        return parse_progress(rest);
    }
    if let Some(rest) = body.strip_prefix("Vote for command ") {
        let (command, rest) = quoted(rest)?;
        let passed = rest.trim_start().starts_with("passed");
        let failed = rest.trim_start().starts_with("failed");
        return (passed || failed).then_some(Announcement::VoteEnded { command, passed });
    }
    if let Some(rest) = body.strip_prefix("Battle setting changed by ") {
        let (by, rest) = rest.split_once(" (")?;
        let (key, value) = rest.strip_suffix(')')?.split_once('=')?;
        return Some(Announcement::SettingChanged {
            by: by.into(),
            key: key.to_ascii_lowercase(),
            value: value.into(),
        });
    }
    if body.starts_with("Vote cancelled by ")
        || body.starts_with("Cancelling ")
        || body.starts_with("Game starting, cancelling")
    {
        return Some(Announcement::VoteCancelled);
    }
    if let Some((by, rest)) = body.split_once(" called a vote for command ") {
        let (command, _) = quoted(rest)?;
        return Some(Announcement::VoteCalled {
            by: by.into(),
            command,
        });
    }
    None
}

/// `"<cmd>" [y:1/2, n:0/1(2)] (25s remaining)`, where the `(max)` after a
/// required count only appears while it can still fall.
fn parse_progress(rest: &str) -> Option<Announcement> {
    let (command, rest) = quoted(rest)?;
    let counts = between(rest, '[', ']')?;
    let (yes_part, no_part) = counts.split_once(", n:")?;
    let (yes, yes_needed) = fraction(yes_part.trim().strip_prefix("y:")?)?;
    let (no, no_needed) = fraction(no_part)?;
    let remaining_secs = rest
        .rsplit_once('(')
        .and_then(|(_, tail)| tail.split('s').next()?.trim().parse().ok())
        .unwrap_or(0);
    Some(Announcement::VoteProgress {
        command,
        yes,
        yes_needed,
        no,
        no_needed,
        remaining_secs,
    })
}

/// `1/2` or `0/1(2)` — the parenthesised ceiling is not what we count against.
fn fraction(text: &str) -> Option<(u32, u32)> {
    let (have, needed) = text.trim().split_once('/')?;
    let needed = needed.split('(').next()?;
    Some((have.trim().parse().ok()?, needed.trim().parse().ok()?))
}

/// The first `"…"` and whatever follows it.
fn quoted(text: &str) -> Option<(String, &str)> {
    let start = text.find('"')? + 1;
    let end = start + text[start..].find('"')?;
    Some((text[start..end].to_owned(), &text[end + 1..]))
}

fn between(text: &str, open: char, close: char) -> Option<&str> {
    let start = text.find(open)? + open.len_utf8();
    let end = start + text[start..].find(close)?;
    Some(&text[start..end])
}

/// What a vote would do if it passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Proposal {
    /// `bSet <key> <value>`: the modoption a vote wants to change.
    SetOption { key: String, value: String },
    /// Anything else — a map change, `forcestart`, and so on.
    Other,
}

impl Proposal {
    /// SPADS echoes the command with the caller's own casing (`spads.pl:3026`).
    pub fn parse(command: &str) -> Proposal {
        let mut parts = command.split_whitespace();
        let is_bset = parts.next().is_some_and(|w| w.eq_ignore_ascii_case("bset"));
        let (Some(key), true) = (parts.next(), is_bset) else {
            return Proposal::Other;
        };
        Proposal::SetOption {
            key: key.to_ascii_lowercase(),
            value: parts.next().unwrap_or_default().to_owned(),
        }
    }
}

/// A vote the room is holding right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoteState {
    pub command: String,
    pub by: Option<String>,
    pub proposal: Proposal,
    pub yes: u32,
    pub yes_needed: u32,
    pub no: u32,
    pub no_needed: u32,
    pub remaining_secs: u32,
}

impl VoteState {
    pub fn called(by: String, command: String) -> Self {
        Self {
            proposal: Proposal::parse(&command),
            command,
            by: Some(by),
            yes: 1,
            yes_needed: 0,
            no: 0,
            no_needed: 0,
            remaining_secs: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_boss_is_read_off_the_state_payload() {
        use super::boss;
        // What a live room actually sends.
        let json = r#"{"BattleStateChanged": {"locked": "unlocked", "teamSize": "4", "boss": "idifixnl"}}"#;
        assert_eq!(boss(json), Some("idifixnl".into()));

        // A room with nobody in charge says so with an empty string.
        assert_eq!(boss(r#"{"BattleStateChanged": {"boss": ""}}"#), None);

        // Anything else is not a claim about who is in charge.
        assert_eq!(boss(r#"{"onVoteStart": {}}"#), None);
        assert_eq!(boss("not json"), None);
    }

    use super::*;

    #[test]
    fn the_vote_lifecycle_spads_actually_prints() {
        let blob = "bG9jYWwgZm9v";
        assert_eq!(
            parse(&format!(
                "* Bob called a vote for command \"bSet tweakdefs1 {blob}\" [!vote y, !vote n, !vote b]"
            )),
            Some(Announcement::VoteCalled {
                by: "Bob".into(),
                command: format!("bSet tweakdefs1 {blob}")
            })
        );
        assert_eq!(
            parse("* Vote in progress: \"set map DSDR 4.0\" [y:1/2, n:0/1(2)] (25s remaining)"),
            Some(Announcement::VoteProgress {
                command: "set map DSDR 4.0".into(),
                yes: 1,
                yes_needed: 2,
                no: 0,
                no_needed: 1,
                remaining_secs: 25
            })
        );
        assert_eq!(
            parse("* Vote for command \"forcestart\" passed."),
            Some(Announcement::VoteEnded {
                command: "forcestart".into(),
                passed: true
            })
        );
        assert_eq!(
            parse("* Vote for command \"forcestart\" failed (delay expired)."),
            Some(Announcement::VoteEnded {
                command: "forcestart".into(),
                passed: false
            })
        );
        for cancelled in [
            "* Vote cancelled by Bob",
            "* Cancelling \"set map x\" vote (command executed directly by Bob)",
            "* Game starting, cancelling \"set map x\" vote",
        ] {
            assert_eq!(
                parse(cancelled),
                Some(Announcement::VoteCancelled),
                "{cancelled}"
            );
        }
    }

    #[test]
    fn setting_changes_and_the_bar_side_channel() {
        assert_eq!(
            parse("* Battle setting changed by Bob (TweakDefs1=QUJD)"),
            Some(Announcement::SettingChanged {
                by: "Bob".into(),
                key: "tweakdefs1".into(),
                value: "QUJD".into()
            })
        );
        // Clearing a slot: the only place this shows up.
        assert!(matches!(
            parse("* Battle setting changed by Bob (tweakdefs1=)"),
            Some(Announcement::SettingChanged { value, .. }) if value.is_empty()
        ));
        assert_eq!(
            parse("* BarManager|{\"onVoteStart\": {}}"),
            Some(Announcement::BarManager {
                json: "{\"onVoteStart\": {}}".into()
            })
        );
    }

    #[test]
    fn ordinary_chat_is_not_an_announcement() {
        assert_eq!(parse("* Hi tetrisface! Current battle type is coop."), None);
        assert_eq!(parse("hello"), None);
        assert_eq!(parse("* Vote in progress: nonsense"), None);
    }

    #[test]
    fn a_tweak_vote_is_recognised_whatever_the_caller_typed() {
        assert_eq!(
            Proposal::parse("bSet TweakDefs1 QUJD"),
            Proposal::SetOption {
                key: "tweakdefs1".into(),
                value: "QUJD".into()
            }
        );
        assert_eq!(Proposal::parse("set map DSDR"), Proposal::Other);
        assert_eq!(
            Proposal::parse("bset tweakdefs1"),
            Proposal::SetOption {
                key: "tweakdefs1".into(),
                value: String::new()
            },
            "clearing a slot"
        );
    }
}
