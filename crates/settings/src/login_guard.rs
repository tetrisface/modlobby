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
/// What the server tells us to wait after refusing.
const THROTTLED_WAIT: Duration = Duration::from_secs(20);

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
    pub fn record_attempt(&mut self, now: SystemTime) {
        let now = unix(now);
        if now >= self.window_ends {
            self.count = 0;
        }
        self.count += 1;
        self.window_ends = now + WINDOW.as_secs();
    }

    /// The server refused us; hold off for as long as it asked.
    pub fn record_refusal(&mut self, now: SystemTime) {
        self.blocked_until = unix(now) + THROTTLED_WAIT.as_secs();
    }

    /// A login went through, so the refusal (if any) is stale.
    pub fn record_success(&mut self, now: SystemTime) {
        self.blocked_until = 0;
        // The count still stands: the server counted this one too.
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
        // The window was refreshed by the third, so it runs to 1_016.
        assert_eq!(state.wait(at(1_007)), Some(Duration::from_secs(9)));
        assert_eq!(state.wait(at(1_015)), Some(Duration::from_secs(1)));
        assert_eq!(state.wait(at(1_016)), None, "the server has forgotten too");

        // And the count starts over.
        state.record_attempt(at(1_016));
        assert_eq!(state.wait(at(1_017)), None);
    }

    #[test]
    fn a_refusal_is_honoured_for_the_twenty_seconds_it_asks_for() {
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
    fn the_count_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let guard = LoginGuard::new(dir.path());
        for second in [500, 501, 502] {
            assert_eq!(guard.wait(at(second)), None);
            guard.record_attempt(at(second));
        }
        // A fresh guard reads the same file, which is the whole point.
        let restarted = LoginGuard::new(dir.path());
        assert_eq!(restarted.wait(at(503)), Some(Duration::from_secs(9)));
        assert_eq!(restarted.wait(at(512)), None);
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
