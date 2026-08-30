//! Not hammering the server's login limit.
//!
//! teiserver counts logins per account in a cache with a 10 s TTL and a limit
//! of 3 (`application.ex:108`, `teiserver_configs.ex:392`). Every *allowed*
//! login refreshes that TTL, so the count only clears after 10 s of silence;
//! the fourth is refused with
//! `Flood protection - Please wait 20 seconds and try again`.
//!
//! Restarting the app is a login, so a few rebuilds in a row hit this. The
//! same counter is kept here, on disk so it survives a restart, and a login
//! that would be refused is held back instead of being sent.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "login-state.json";

/// teiserver's `system.Login limit count`.
const LIMIT: u32 = 3;
/// The TTL on its login-count cache, refreshed by each allowed login.
const WINDOW: Duration = Duration::from_secs(10);
/// What the server *says* to wait after refusing.
///
/// A ceiling, not the answer. `login_flood_check` blocks without touching its
/// cache (`cache_user.ex:690-700`), so the block really clears when that
/// cache entry expires — `WINDOW` after the last login it *allowed*, which is
/// usually sooner than this.
const THROTTLED_WAIT: Duration = Duration::from_secs(20);

/// A second's grace on every computed wait.
///
/// Our clock and the server's are not the same clock, and being a moment early
/// costs a refusal that puts us back where we started.
const MARGIN: Duration = Duration::from_secs(1);

/// The counter as the server keeps it, plus any refusal it has handed us.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LoginState {
    /// Logins counted in the current window.
    pub count: u32,
    /// When that count lapses, as a unix timestamp.
    pub window_ends: u64,
    /// When a refusal expires, as a unix timestamp.
    pub blocked_until: u64,
    /// The window as it stood before the attempt in flight.
    ///
    /// A refused login is the one case the server does not count, so the
    /// optimistic extension has to be undoable.
    #[serde(default)]
    pub window_before: u64,
}

impl LoginState {
    /// How long a login must wait, or `None` when it may go now.
    pub fn wait(&self, now: SystemTime) -> Option<Duration> {
        let now = unix(now);
        let blocked = self.blocked_until.saturating_sub(now);
        if blocked > 0 {
            return Some(Duration::from_secs(blocked));
        }
        // A lapsed window means the server has forgotten the count too.
        if now >= self.window_ends {
            return None;
        }
        (self.count >= LIMIT).then(|| Duration::from_secs(self.window_ends - now))
    }

    /// Counts a login we are about to send, the way the server will count it.
    ///
    /// Optimistically: `login_flood_check` runs before the password is even
    /// looked at (`cache_user.ex:749`), so anything it lets through has both
    /// counted and refreshed the cache — a wrong password included. Only a
    /// refusal is not counted, and [`Self::record_refusal`] undoes this.
    pub fn record_attempt(&mut self, now: SystemTime) {
        let now = unix(now);
        if now >= self.window_ends {
            self.count = 0;
        }
        self.count += 1;
        self.window_before = self.window_ends;
        self.window_ends = now + WINDOW.as_secs() + MARGIN.as_secs();
    }

    /// The server refused us.
    ///
    /// It refused because its count was already at the limit, and refusing did
    /// not refresh anything — so the block lifts when the entry from the last
    /// allowed login expires. Taking the "20 seconds" literally instead means
    /// waiting from the moment we were told off, which restarts the clock every
    /// time we ask.
    pub fn record_refusal(&mut self, now: SystemTime) {
        let now = unix(now);
        // Blocking touches nothing, so this attempt neither counted nor
        // refreshed: put the window back where the last counted login left it.
        self.window_ends = self.window_before;
        self.count = LIMIT;
        // The block lifts when that entry expires, which is what the window
        // already records — not twenty seconds from being told off, which
        // restarts the clock every time we ask and is why quitting the app
        // never seemed to make the wait any shorter.
        let ceiling = now + THROTTLED_WAIT.as_secs();
        // With no live window we have nothing better than what it said. That
        // is the case on a first run, or after a session whose bookkeeping
        // never reached the disk.
        let expected = if self.window_ends > now {
            self.window_ends + MARGIN.as_secs()
        } else {
            ceiling
        };
        self.blocked_until = expected.clamp(now + MARGIN.as_secs(), ceiling);
    }

    /// A login went through, so the refusal (if any) is stale.
    pub fn record_success(&mut self, now: SystemTime) {
        self.blocked_until = 0;
        // The count and the window stand: the server counted this one.
        let _ = now;
    }
}

/// Is this the server telling us we tried too often?
pub fn is_flood_refusal(reason: &str) -> bool {
    reason.to_ascii_lowercase().contains("flood protection")
}

fn unix(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The counter, kept beside the settings so a restart cannot forget it.
#[derive(Debug, Clone)]
pub struct LoginGuard {
    path: PathBuf,
}

impl LoginGuard {
    pub fn new(config_dir: impl AsRef<Path>) -> Self {
        Self {
            path: config_dir.as_ref().join(FILE_NAME),
        }
    }

    pub fn load(&self) -> LoginState {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// A losable counter: if it cannot be written the login still proceeds,
    /// because refusing to log in over a bookkeeping failure helps nobody.
    fn store(&self, state: &LoginState) {
        let Ok(text) = serde_json::to_string_pretty(state) else {
            return;
        };
        if let Err(err) = std::fs::write(&self.path, text) {
            tracing::warn!(%err, path = %self.path.display(), "could not record the login count");
        }
    }

    pub fn wait(&self, now: SystemTime) -> Option<Duration> {
        self.load().wait(now)
    }

    pub fn record_attempt(&self, now: SystemTime) {
        let mut state = self.load();
        state.record_attempt(now);
        self.store(&state);
    }

    pub fn record_refusal(&self, now: SystemTime) {
        let mut state = self.load();
        state.record_refusal(now);
        self.store(&state);
    }

    pub fn record_success(&self, now: SystemTime) {
        let mut state = self.load();
        state.record_success(now);
        self.store(&state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn the_fourth_login_in_a_window_is_held_back() {
        let mut state = LoginState::default();
        assert_eq!(state.wait(at(1_000)), None);
        for second in [1_000, 1_003, 1_006] {
            assert_eq!(state.wait(at(second)), None, "three are allowed");
            state.record_attempt(at(second));
        }
        // The window was refreshed by the third, so it runs to 1_016 plus a
        // second of grace against the two clocks disagreeing.
        assert_eq!(state.wait(at(1_007)), Some(Duration::from_secs(10)));
        assert_eq!(state.wait(at(1_016)), Some(Duration::from_secs(1)));
        assert_eq!(state.wait(at(1_017)), None, "the server has forgotten too");

        // And the count starts over.
        state.record_attempt(at(1_017));
        assert_eq!(state.wait(at(1_018)), None);
    }

    #[test]
    fn a_refusal_with_nothing_to_go_on_believes_the_twenty_seconds() {
        let mut state = LoginState::default();
        state.record_refusal(at(2_000));
        assert_eq!(state.wait(at(2_000)), Some(Duration::from_secs(20)));
        assert_eq!(state.wait(at(2_019)), Some(Duration::from_secs(1)));
        assert_eq!(state.wait(at(2_020)), None);

        // Logging in successfully clears a stale refusal.
        state.record_refusal(at(3_000));
        state.record_success(at(3_005));
        assert_eq!(state.wait(at(3_005)), None);
    }

    #[test]
    fn a_refusal_waits_for_the_window_rather_than_the_stated_twenty() {
        // Three logins get through, so the server's entry expires at 1_011.
        let mut state = LoginState::default();
        for second in [1_000, 1_001, 1_002] {
            state.record_attempt(at(second));
        }
        // A fourth we sent anyway — another client, a clock that disagreed.
        state.record_attempt(at(1_003));
        state.record_refusal(at(1_003));

        // The refusal did not refresh anything, so the wait runs to where the
        // third login left the window, not twenty seconds from now.
        // The third login left the window at 1_013; the refused fourth moved
        // it and was rolled back. Eleven seconds, not the twenty it was told.
        assert_eq!(state.wait(at(1_003)), Some(Duration::from_secs(11)));
        assert_eq!(state.wait(at(1_013)), Some(Duration::from_secs(1)));
        assert_eq!(state.wait(at(1_014)), None);
    }

    #[test]
    fn quitting_between_attempts_counts_towards_the_wait() {
        // The complaint this models: the clock should not restart because the
        // app did. Three logins, then the app is gone for eight seconds.
        let dir = tempfile::tempdir().unwrap();
        let guard = LoginGuard::new(dir.path());
        for second in [2_000, 2_001, 2_002] {
            guard.record_attempt(at(second));
        }
        let restarted = LoginGuard::new(dir.path());
        assert_eq!(restarted.wait(at(2_010)), Some(Duration::from_secs(3)));
        assert_eq!(restarted.wait(at(2_013)), None, "waited out while quit");
    }

    #[test]
    fn the_count_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let guard = LoginGuard::new(dir.path());
        for second in [500, 501, 502] {
            assert_eq!(guard.wait(at(second)), None);
            guard.record_attempt(at(second));
        }
        // A fresh guard reads the same file, which is the whole point.
        let restarted = LoginGuard::new(dir.path());
        assert_eq!(restarted.wait(at(503)), Some(Duration::from_secs(10)));
        assert_eq!(restarted.wait(at(513)), None);
    }

    #[test]
    fn only_the_flood_refusal_counts_as_one() {
        assert!(is_flood_refusal(
            "Flood protection - Please wait 20 seconds and try again"
        ));
        assert!(is_flood_refusal("flood protection"));
        assert!(!is_flood_refusal("Invalid username or password"));
        assert!(!is_flood_refusal("Account is banned"));
    }

    #[test]
    fn a_missing_or_broken_file_never_blocks_a_login() {
        let dir = tempfile::tempdir().unwrap();
        let guard = LoginGuard::new(dir.path());
        assert_eq!(guard.wait(at(1)), None, "no file yet");
        std::fs::write(dir.path().join(FILE_NAME), "not json").unwrap();
        assert_eq!(guard.wait(at(1)), None, "unreadable counts as clear");
    }
}
