//! Battle-room lines and the bit-packed battle status.
//!
//! Layout (teiserver `spring.ex` `parse_battle_status/1`, bit 0 = LSB):
//! 0 reserved · 1 ready · 2–5 team · 6–9 ally team · 10 player (0 = spectator) ·
//! 11–17 handicap · 18–21 team bits 5–8 · 22–23 sync (1 synced, 2 unsynced, 0 bot) ·
//! 24–27 side · 28–31 ally-team bits 5–8.

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

/// The status this client reports about itself.
///
/// Only constructible as a spectator, on purpose: until the battle-room flow is
/// verified against live rooms, modlobby must never take a player slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MyBattleStatus {
    sync: Sync,
}

impl MyBattleStatus {
    pub fn spectator(sync: Sync) -> Self {
        Self { sync }
    }

    pub fn bits(self) -> u32 {
        (self.sync as u32) << 22
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn our_status_is_always_a_spectator() {
        for sync in [Sync::Bot, Sync::Synced, Sync::Unsynced] {
            let decoded = BattleStatus::from_bits(MyBattleStatus::spectator(sync).bits());
            assert!(!decoded.player);
            assert!(!decoded.ready);
            assert_eq!(decoded.sync, sync);
        }
        assert_eq!(
            MyBattleStatus::spectator(Sync::Unsynced).line(),
            "MYBATTLESTATUS 8388608 0"
        );
    }

    #[test]
    fn join_battle_always_has_three_tokens() {
        assert_eq!(join_battle(57, None, "1234"), "JOINBATTLE 57 empty 1234");
        assert_eq!(join_battle(57, Some("pw"), "1234"), "JOINBATTLE 57 pw 1234");
        assert_eq!(join_battle(57, None, "1234").split(' ').count(), 4);
    }
}
