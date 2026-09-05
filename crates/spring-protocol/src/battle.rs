//! Battle-room lines and the bit-packed battle status.
//!
//! Layout (teiserver `spring.ex` `parse_battle_status/1`, bit 0 = LSB):
//! 0 reserved · 1 ready · 2–5 team · 6–9 ally team · 10 player (0 = spectator) ·
//! 11–17 handicap · 18–21 team bits 5–8 · 22–23 sync (1 synced, 2 unsynced, 0 bot) ·
//! 24–27 side · 28–31 ally-team bits 5–8.

use crate::paste;
use crate::policy::{Area, Envelope, saybattle_max_len};

/// Whether a client has the engine, game and map the room needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sync {
    /// Reported by host bots.
    Bot = 0,
    Synced = 1,
    #[default]
    Unsynced = 2,
}

impl Sync {
    fn from_bits(bits: u32) -> Self {
        match bits & 0b11 {
            0 => Self::Bot,
            1 => Self::Synced,
            _ => Self::Unsynced,
        }
    }
}

/// Decoded `CLIENTBATTLESTATUS` of a room member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BattleStatus {
    pub ready: bool,
    pub team: u8,
    pub ally_team: u8,
    /// `false` is a spectator.
    pub player: bool,
    pub handicap: u8,
    pub sync: Sync,
    pub side: u8,
}

impl BattleStatus {
    pub fn from_bits(bits: u32) -> Self {
        Self {
            ready: bits & (1 << 1) != 0,
            team: (((bits >> 2) & 0xF) | (((bits >> 18) & 0xF) << 4)) as u8,
            ally_team: (((bits >> 6) & 0xF) | (((bits >> 28) & 0xF) << 4)) as u8,
            player: bits & (1 << 10) != 0,
            handicap: ((bits >> 11) & 0x7F) as u8,
            sync: Sync::from_bits(bits >> 22),
            side: ((bits >> 24) & 0xF) as u8,
        }
    }
}

/// The seat this client asks for. teiserver creates every joining client as a
/// spectator (`data/client.ex:36`) and only bit 10 changes that, so
/// [`MyBattleStatus::spectator`] is the only status that can never take a slot
/// from someone. [`MyBattleStatus::player`] exists for rooms we control; the
/// decision to use it belongs to the layer that knows which room this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MyBattleStatus {
    sync: Sync,
    seat: Option<Seat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seat {
    team: u8,
    ally_team: u8,
    ready: bool,
    side: u8,
}

impl MyBattleStatus {
    pub fn spectator(sync: Sync) -> Self {
        Self { sync, seat: None }
    }

    /// Takes a player slot on `team` within `ally_team`. Callers must be sure
    /// of the room: in a public room this displaces a real player.
    pub fn player(sync: Sync, team: u8, ally_team: u8) -> Self {
        Self {
            sync,
            seat: Some(Seat {
                team,
                ally_team,
                ready: false,
                side: 0,
            }),
        }
    }

    pub fn ready(mut self, ready: bool) -> Self {
        if let Some(seat) = self.seat.as_mut() {
            seat.ready = ready;
        }
        self
    }

    pub fn side(mut self, side: u8) -> Self {
        if let Some(seat) = self.seat.as_mut() {
            seat.side = side;
        }
        self
    }

    pub fn is_player(self) -> bool {
        self.seat.is_some()
    }

    pub fn bits(self) -> u32 {
        let sync = (self.sync as u32) << 22;
        let Some(seat) = self.seat else {
            return sync;
        };
        let team = seat.team as u32;
        let ally = seat.ally_team as u32;
        sync | (u32::from(seat.ready) << 1)
            | ((team & 0xF) << 2)
            | ((ally & 0xF) << 6)
            | (1 << 10)
            | (((team >> 4) & 0xF) << 18)
            | ((seat.side as u32 & 0xF) << 24)
            | (((ally >> 4) & 0xF) << 28)
    }

    /// `MYBATTLESTATUS <bits> <team colour>` (`spring_in.ex` `do_handle("MYBATTLESTATUS", …)`).
    /// The colour is `0xBBGGRR`; a spectator has none.
    pub fn line(self) -> String {
        format!("MYBATTLESTATUS {} 0", self.bits())
    }
}

/// `JOINBATTLE <id> <password|empty> <script password>`. teiserver's regex needs
/// all three tokens (`spring_in.ex` `do_handle("JOINBATTLE", …)`); Chobby sends
/// the literal `empty` when the room has no password.
pub fn join_battle(id: u32, password: Option<&str>, script_password: &str) -> String {
    format!(
        "JOINBATTLE {id} {} {script_password}",
        password.unwrap_or("empty")
    )
}

pub const LEAVE_BATTLE: &str = "LEAVEBATTLE";

/// Chat text longer than teiserver keeps; sending it would arrive truncated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("message is {len} characters, the server keeps {max}")]
pub struct TooLong {
    pub len: usize,
    pub max: usize,
}

/// `SAYPRIVATE <user> <text>`: a direct message, and how a cluster manager is
/// asked for a private room (`!privatehost`). teiserver's handler does not
/// slice these (`spring_in.ex`, `SAYPRIVATE`); the chat cap is kept here so a
/// long message reads the same wherever it is sent.
pub fn say_private(user: &str, text: &str) -> Result<Envelope, TooLong> {
    let len = text.chars().count();
    let max = saybattle_max_len(text);
    if len > max {
        return Err(TooLong { len, max });
    }
    Ok(Envelope::queue(
        Area::ChannelChat,
        format!("SAYPRIVATE {user} {text}"),
    ))
}

/// `RING <user>`: summons someone to the room they are already sitting in,
/// which is how you tell a player who has wandered off that the game is
/// waiting on them. It has its own throttle bucket because the server treats
/// it as the nuisance it can be.
pub fn ring(user: &str) -> Envelope {
    Envelope::queue(Area::Ring, format!("RING {user}"))
}

/// `ADDBOT <name> <status> <colour> <ai>` (`spring_in.ex` regex
/// `(\S+) (\d+) (\d+) (.+)`) — the name takes no spaces, the AI may.
///
/// The client that sends this hosts the AI: the engine on *this* machine runs
/// it when the game starts, which is why the AI has to be installed here and
/// why the status is a player seat of ours to give it.
pub fn add_bot(name: &str, ai: &str, status: MyBattleStatus, colour: u32) -> Envelope {
    Envelope::queue(
        Area::Other,
        format!("ADDBOT {name} {} {colour} {ai}", status.bits()),
    )
}

/// `REMOVEBOT <name>`. The server checks the right to remove, not us.
pub fn remove_bot(name: &str) -> Envelope {
    Envelope::queue(Area::Other, format!("REMOVEBOT {name}"))
}

/// `SAYBATTLE <text>`. `!`/`$` lines are SPADS commands and go through the
/// command bucket of the throttle policy; everything else is chat.
pub fn say_battle(text: &str) -> Result<Envelope, TooLong> {
    let len = text.chars().count();
    let max = saybattle_max_len(text);
    if len > max {
        return Err(TooLong { len, max });
    }
    let area = if text.starts_with(['!', '$']) {
        Area::BattleCommand
    } else {
        Area::BattleChat
    };
    Ok(Envelope::queue(area, format!("SAYBATTLE {text}")))
}

/// Everything pasted into the battle room, one `SAYBATTLE` per line, in the
/// order it was written.
///
/// A command line (`!`/`$`) goes whole or not at all: the server would cut it
/// silently and SPADS would run the stump. A chat line wraps at the cap
/// instead. Nothing is returned unless every line fits, so a pasted preset
/// never half-applies.
pub fn say_battle_lines(text: &str) -> Result<Vec<Envelope>, TooLong> {
    let mut envelopes = Vec::new();
    for line in paste::lines(text) {
        if line.starts_with(['!', '$']) {
            envelopes.push(say_battle(line)?);
            continue;
        }
        for piece in paste::wrap(line, saybattle_max_len(line)) {
            envelopes.push(say_battle(&piece)?);
        }
    }
    Ok(envelopes)
}

/// A pasted direct message, one `SAYPRIVATE` per line, long lines wrapped.
pub fn say_private_lines(user: &str, text: &str) -> Result<Vec<Envelope>, TooLong> {
    paste::lines(text)
        .into_iter()
        .flat_map(|line| paste::wrap(line, crate::chat::SAY_MAX_LEN))
        .map(|piece| say_private(user, &piece))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_bot_is_a_seated_ready_player_line() {
        let status = MyBattleStatus::player(Sync::Synced, 3, 1).ready(true);
        let envelope = add_bot("BARb1", "BARb", status, 255);
        // ready(1<<1) | team 3(<<2) | ally 1(<<6) | player(1<<10) | synced(1<<22)
        let bits = (1 << 1) | (3 << 2) | (1 << 6) | (1 << 10) | (1 << 22);
        assert_eq!(envelope.line, format!("ADDBOT BARb1 {bits} 255 BARb"));
        assert_eq!(remove_bot("BARb1").line, "REMOVEBOT BARb1");
    }

    #[test]
    fn say_battle_picks_the_bucket_and_enforces_the_cap() {
        let chat = say_battle("hello").unwrap();
        assert_eq!(
            (chat.area, chat.line.as_str()),
            (Area::BattleChat, "SAYBATTLE hello")
        );
        assert_eq!(say_battle("!vote y").unwrap().area, Area::BattleCommand);
        assert_eq!(
            say_battle("$welcome-message hi").unwrap().area,
            Area::BattleCommand
        );

        assert!(say_battle(&"x".repeat(257)).is_ok());
        assert_eq!(
            say_battle(&"x".repeat(258)),
            Err(TooLong { len: 258, max: 257 })
        );
        let prefix = "!bSet tweakdefs1 ";
        let blob = "A".repeat(16_385 - prefix.len());
        assert!(say_battle(&format!("{prefix}{blob}")).is_ok());
        assert!(say_battle(&format!("{prefix}{blob}A")).is_err());
    }

    #[test]
    fn a_paste_is_one_message_per_line_with_commands_kept_whole() {
        let lines = |text: &str| -> Vec<String> {
            say_battle_lines(text)
                .unwrap()
                .into_iter()
                .map(|e| e.line)
                .collect()
        };
        assert_eq!(
            lines("!preset custom\r\n\n  !bSet a 1  \nthanks all"),
            [
                "SAYBATTLE !preset custom",
                "SAYBATTLE !bSet a 1",
                "SAYBATTLE thanks all",
            ]
        );
        assert!(lines("\n \n").is_empty());

        // Chat wraps at the cap; the pieces are all sent as chat.
        let wall = format!("{} {}", "a".repeat(200), "b".repeat(200));
        let sent = say_battle_lines(&wall).unwrap();
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(|e| e.area == Area::BattleChat));
        assert_eq!(sent[0].line, format!("SAYBATTLE {}", "a".repeat(200)));

        // A command over its cap refuses the whole paste: nothing half-applies.
        let over = format!("!preset custom\n!bSet welcome {}", "x".repeat(300));
        assert_eq!(say_battle_lines(&over), Err(TooLong { len: 314, max: 257 }));
    }

    #[test]
    fn a_private_paste_wraps_like_chat() {
        let sent = say_private_lines("bob", &format!("hi\n{}", "y".repeat(258))).unwrap();
        assert_eq!(sent.len(), 3);
        assert_eq!(sent[0].line, "SAYPRIVATE bob hi");
        assert_eq!(sent[1].line, format!("SAYPRIVATE bob {}", "y".repeat(257)));
        assert_eq!(sent[2].line, "SAYPRIVATE bob y");
    }

    #[test]
    fn decodes_teiserver_layout() {
        // ready, team 3, ally 1, player, handicap 5, synced, side 2
        let bits = (1 << 1) | (3 << 2) | (1 << 6) | (1 << 10) | (5 << 11) | (1 << 22) | (2 << 24);
        assert_eq!(
            BattleStatus::from_bits(bits),
            BattleStatus {
                ready: true,
                team: 3,
                ally_team: 1,
                player: true,
                handicap: 5,
                sync: Sync::Synced,
                side: 2
            }
        );
        let wide = BattleStatus::from_bits((1 << 18) | (1 << 28));
        assert_eq!((wide.team, wide.ally_team), (16, 16));
    }

    #[test]
    fn a_spectator_status_can_never_carry_a_seat() {
        for sync in [Sync::Bot, Sync::Synced, Sync::Unsynced] {
            let status = MyBattleStatus::spectator(sync);
            let decoded = BattleStatus::from_bits(status.bits());
            assert!(!status.is_player());
            assert!(!decoded.player);
            assert!(!decoded.ready);
            assert_eq!(decoded.sync, sync);
            // The modifiers only apply to a seat, so they cannot create one.
            assert_eq!(status.ready(true).side(3).bits(), status.bits());
        }
        assert_eq!(
            MyBattleStatus::spectator(Sync::Unsynced).line(),
            "MYBATTLESTATUS 8388608 0"
        );
    }

    /// The encoder must round-trip through the decoder teiserver shares.
    #[test]
    fn a_player_status_round_trips_including_the_wide_team_bits() {
        let status = MyBattleStatus::player(Sync::Synced, 3, 1)
            .ready(true)
            .side(2);
        let decoded = BattleStatus::from_bits(status.bits());
        assert_eq!(
            decoded,
            BattleStatus {
                ready: true,
                team: 3,
                ally_team: 1,
                player: true,
                handicap: 0,
                sync: Sync::Synced,
                side: 2
            }
        );
        // Teams above 15 use the extension bits at 18-21 and 28-31.
        let wide = BattleStatus::from_bits(MyBattleStatus::player(Sync::Synced, 20, 17).bits());
        assert_eq!((wide.team, wide.ally_team), (20, 17));
        assert!(wide.player && !wide.ready);
    }

    #[test]
    fn join_battle_always_has_three_tokens() {
        assert_eq!(join_battle(57, None, "1234"), "JOINBATTLE 57 empty 1234");
        assert_eq!(join_battle(57, Some("pw"), "1234"), "JOINBATTLE 57 pw 1234");
        assert_eq!(join_battle(57, None, "1234").split(' ').count(), 4);
    }
}
