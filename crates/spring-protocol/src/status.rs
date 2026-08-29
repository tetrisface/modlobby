//! `MYSTATUS`: the lobby-level flags a client controls.
//!
//! Same bit layout as `CLIENTSTATUS` ([`crate::UserStatus`]), but teiserver keeps
//! only `in_game` and `away` from it (`spring_in.ex` `do_handle("MYSTATUS", …)`);
//! rank, moderator and bot are server-owned.
//!
//! The in-game bit matters for spectating a running game: SPADS `/adduser`s a
//! mid-game joiner to the engine only after seeing it (`spads.pl` `cbClientStatus`).

pub fn my_status(in_game: bool, away: bool) -> String {
    format!("MYSTATUS {}", u32::from(in_game) | (u32::from(away) << 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UserStatus;

    #[test]
    fn bits_round_trip_through_the_clientstatus_layout() {
        assert_eq!(my_status(true, false), "MYSTATUS 1");
        assert_eq!(my_status(false, true), "MYSTATUS 2");
        assert_eq!(my_status(false, false), "MYSTATUS 0");
        let decoded = UserStatus::from_bits(3);
        assert!(decoded.in_game && decoded.away);
    }
}
