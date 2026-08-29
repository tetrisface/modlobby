//! When to try the server again after losing it.
//!
//! Reconnecting at once does not work and is not polite. teiserver keeps a
//! session for a while after a drop and refuses a second login until it has
//! flushed, and a server that restarts drops everyone at the same instant — so
//! a fixed delay would have the whole population knock in unison. Chobby waits
//! 25-60 s in the lobby and 60-300 s if the drop happened mid-game, jittered
//! per client (`liblobby/lobby/lobby.lua:2055-2088`). This is that policy,
//! separated from the socket so its arithmetic can be tested without one.

use std::time::{Duration, Instant};

/// The floor before a first attempt, and the spread added on top of it.
const LOBBY_FLOOR: Duration = Duration::from_secs(25);
const LOBBY_SPREAD: Duration = Duration::from_secs(35);
/// Longer in game: the drop was probably the server, and a game is still
/// running that we do not want to disturb by hammering.
const GAME_FLOOR: Duration = Duration::from_secs(60);
const GAME_SPREAD: Duration = Duration::from_secs(240);
/// Between attempts once the first one has been made.
const RETRY_EVERY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Waiting {
    /// The earliest a first attempt may be made.
    first_allowed: Instant,
    /// When the last attempt went out, if one has.
    attempted: Option<Instant>,
}

/// Whether and when to try again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconnect {
    waiting: Option<Waiting>,
}

impl Reconnect {
    /// Arms the policy after a drop. `jitter` is a fresh number in `0.0..1.0`,
    /// drawn once per disconnect so two clients dropped together come back at
    /// different moments.
    pub fn disconnected(&mut self, now: Instant, in_game: bool, jitter: f64) {
        let (floor, spread) = if in_game {
            (GAME_FLOOR, GAME_SPREAD)
        } else {
            (LOBBY_FLOOR, LOBBY_SPREAD)
        };
        let jitter = jitter.clamp(0.0, 1.0);
        self.waiting = Some(Waiting {
            first_allowed: now + floor + spread.mul_f64(jitter),
            attempted: None,
        });
    }

    /// Stops trying: we are connected, or the user asked us to stop.
    pub fn stop(&mut self) {
        self.waiting = None;
    }

    pub fn is_armed(&self) -> bool {
        self.waiting.is_some()
    }

    /// How long until the next attempt is due, or `None` when none is wanted.
    pub fn until_due(&self, now: Instant) -> Option<Duration> {
        let waiting = self.waiting.as_ref()?;
        let due = match waiting.attempted {
            Some(attempted) => waiting.first_allowed.max(attempted + RETRY_EVERY),
            None => waiting.first_allowed,
        };
        Some(due.saturating_duration_since(now))
    }

    /// Whether an attempt should go out now.
    pub fn due(&self, now: Instant) -> bool {
        self.until_due(now) == Some(Duration::ZERO)
    }

    /// Records that an attempt has just been made, so the next waits again.
    pub fn attempted(&mut self, now: Instant) {
        if let Some(waiting) = self.waiting.as_mut() {
            waiting.attempted = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn nothing_is_due_until_it_has_been_armed() {
        let now = Instant::now();
        let policy = Reconnect::default();
        assert!(!policy.is_armed());
        assert_eq!(policy.until_due(now), None);
        assert!(!policy.due(now));
    }

    #[test]
    fn a_lobby_drop_waits_between_twenty_five_and_sixty_seconds() {
        let now = Instant::now();

        let mut soonest = Reconnect::default();
        soonest.disconnected(now, false, 0.0);
        assert!(!soonest.due(at(now, 24)));
        assert!(soonest.due(at(now, 25)));

        let mut latest = Reconnect::default();
        latest.disconnected(now, true, 1.0);
        // In game the window is 60-300s, so the longest wait is five minutes.
        assert!(!latest.due(at(now, 299)));
        assert!(latest.due(at(now, 300)));
    }

    #[test]
    fn two_clients_dropped_together_do_not_come_back_together() {
        let now = Instant::now();
        let mut first = Reconnect::default();
        let mut second = Reconnect::default();
        first.disconnected(now, false, 0.1);
        second.disconnected(now, false, 0.9);
        assert_ne!(first.until_due(now), second.until_due(now));
    }

    #[test]
    fn after_an_attempt_the_next_one_waits_again() {
        let now = Instant::now();
        let mut policy = Reconnect::default();
        policy.disconnected(now, false, 0.0);

        assert!(policy.due(at(now, 25)));
        policy.attempted(at(now, 25));
        assert!(!policy.due(at(now, 30)), "not immediately after trying");
        assert!(policy.due(at(now, 55)), "thirty seconds later");
    }

    #[test]
    fn connecting_disarms_it() {
        let now = Instant::now();
        let mut policy = Reconnect::default();
        policy.disconnected(now, false, 0.0);
        policy.stop();
        assert!(!policy.is_armed());
        assert!(!policy.due(at(now, 600)));
    }

    #[test]
    fn a_jitter_outside_its_range_cannot_shorten_the_floor() {
        let now = Instant::now();
        let mut policy = Reconnect::default();
        policy.disconnected(now, false, -5.0);
        assert!(!policy.due(at(now, 24)));
        assert!(policy.due(at(now, 25)));
    }
}
