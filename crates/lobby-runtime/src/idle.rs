//! When to let go of the server because nobody is here.
//!
//! A lobby left open holds a seat on the server, and [`crate::reconnect`]
//! makes it hold on through drops. That is right for a window someone is
//! using and wrong for one they forgot: an idle lobby keeps a room's player
//! count honest for nobody, and its account logged in from a machine its
//! owner may have walked away from. So after a while without anyone touching
//! the window, the connection is dropped and not retried; the window stays,
//! one click from logging in again.
//!
//! What counts as touching the window is the front end's to decide and to
//! report through [`Idle::active`]; this is only the arithmetic, kept apart
//! from the socket so it can be tested without one.

use std::time::{Duration, Instant};

/// How long without activity before the connection is dropped, and when the
/// window was last touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Idle {
    /// `None` keeps the connection however long nobody is here.
    timeout: Option<Duration>,
    last_active: Instant,
}

impl Default for Idle {
    fn default() -> Self {
        Self {
            timeout: None,
            last_active: Instant::now(),
        }
    }
}

impl Idle {
    /// Sets or clears the limit. Measured from the last activity, not from
    /// now: a limit lowered on a window already idle past it is due at once.
    pub fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.timeout = timeout;
    }

    /// Someone is here.
    pub fn active(&mut self, now: Instant) {
        self.last_active = now;
    }

    /// How long until the connection should be dropped, or `None` when it
    /// never should be.
    pub fn until_due(&self, now: Instant) -> Option<Duration> {
        let timeout = self.timeout?;
        Some((self.last_active + timeout).saturating_duration_since(now))
    }

    /// Whether the connection should be dropped now.
    pub fn due(&self, now: Instant) -> bool {
        self.until_due(now) == Some(Duration::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn nothing_is_due_without_a_limit() {
        let now = Instant::now();
        let mut idle = Idle::default();
        idle.active(now);
        assert_eq!(idle.until_due(at(now, 1_000_000)), None);
        assert!(!idle.due(at(now, 1_000_000)));
    }

    #[test]
    fn the_limit_counts_from_the_last_activity() {
        let now = Instant::now();
        let mut idle = Idle::default();
        idle.active(now);
        idle.set_timeout(Some(Duration::from_secs(60)));

        assert_eq!(idle.until_due(at(now, 20)), Some(Duration::from_secs(40)));
        assert!(!idle.due(at(now, 59)));
        assert!(idle.due(at(now, 60)));

        idle.active(at(now, 59));
        assert!(!idle.due(at(now, 60)));
        assert!(idle.due(at(now, 119)));
    }

    #[test]
    fn a_limit_lowered_past_the_idle_time_is_due_at_once() {
        let now = Instant::now();
        let mut idle = Idle::default();
        idle.active(now);
        idle.set_timeout(Some(Duration::from_secs(3600)));
        assert!(!idle.due(at(now, 600)));

        idle.set_timeout(Some(Duration::from_secs(300)));
        assert!(idle.due(at(now, 600)));

        idle.set_timeout(None);
        assert!(!idle.due(at(now, 600)));
    }
}
