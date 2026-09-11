//! Not hammering the server's login limit.
//!
//! teiserver keeps a per-account login counter in a ConCache with a 10 s TTL
//! (`application.ex:108`) and refuses a login once it reaches
//! `system.Login limit count` with `Flood protection - Please wait 20 seconds
//! and try again` (`cache_user.ex:690-700`). Two things about that cache make
//! the wait longer than the TTL says (`cache_helper.ex:140-153`):
//!
//! - it is swept every 10 s, and an entry is dropped at the second sweep after
//!   it was written, so it lives 10-20 s;
//! - it is `touch_on_read`, and the check that refuses a login reads it — so a
//!   refused attempt renews the entry for another 10-20 s.
//!
//! The counter's limit on the live server is not the default three: on
//! 2026-09-10 a login 13 s after the previous one was refused, and again one
//! 9 s after, both with nothing else on the account (`logs/modlobby.jsonl`).
//! So the rule kept here is the one that fits: **one login per entry life**,
//! 20 s after the last attempt, and 20 s more after a refusal. That is the
//! wait a restart has to respect, and it is kept on disk so a restart can —
//! the app relaunching after an update is exactly the case.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "login-state.json";

/// How long the server's counter entry can outlive the attempt that touched
/// it: the 10 s TTL, rounded up to the next 10 s sweep.
const ENTRY_LIFE: Duration = Duration::from_secs(20);

/// A second's grace on every computed wait.
///
/// Our clock and the server's are not the same clock, and being a moment early
/// costs a refusal that puts us back where we started.
const MARGIN: Duration = Duration::from_secs(1);

/// The server's counter as far as this machine has touched it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LoginState {
    /// When the last login went out, as a unix timestamp; `0` for never.
    pub last_attempt: u64,
    /// When the last refusal's renewal of the entry lapses, as a unix
    /// timestamp; `0` for none.
    pub blocked_until: u64,
}

impl LoginState {
    /// How long a login must wait, or `None` when it may go now.
    pub fn wait(&self, now: SystemTime) -> Option<Duration> {
        let now = unix(now);
        let after_attempt = match self.last_attempt {
            0 => 0,
            at => at + ENTRY_LIFE.as_secs() + MARGIN.as_secs(),
        };
        let wait = after_attempt.max(self.blocked_until).saturating_sub(now);
        (wait > 0).then(|| Duration::from_secs(wait))
    }

    /// Counts a login we are about to send, the way the server will count it.
    ///
    /// Before the answer, because the server counts before it looks at the
    /// password (`cache_user.ex:840`): a wrong password counts too.
    pub fn record_attempt(&mut self, now: SystemTime) {
        self.last_attempt = unix(now);
    }

    /// The server refused us: the check that refused read the entry, and
    /// reading renews it, so the wait starts again from now.
    pub fn record_refusal(&mut self, now: SystemTime) {
        self.blocked_until = unix(now) + ENTRY_LIFE.as_secs() + MARGIN.as_secs();
    }

    /// A login went through, so the refusal (if any) is stale. The attempt
    /// itself still stands: the server counted it.
    pub fn record_success(&mut self, now: SystemTime) {
        self.blocked_until = 0;
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
    fn a_second_login_waits_out_the_entry_the_first_one_wrote() {
        let mut state = LoginState::default();
        assert_eq!(state.wait(at(1_000)), None, "nothing counted yet");
        state.record_attempt(at(1_000));
        // The entry can live twenty seconds, plus a second of grace against
        // the two clocks disagreeing.
        assert_eq!(state.wait(at(1_000)), Some(Duration::from_secs(21)));
        assert_eq!(state.wait(at(1_013)), Some(Duration::from_secs(8)));
        assert_eq!(state.wait(at(1_020)), Some(Duration::from_secs(1)));
        assert_eq!(state.wait(at(1_021)), None, "the server has forgotten too");
    }

    #[test]
    fn a_refusal_starts_the_wait_again_from_the_refusal() {
        // What the log showed: a login, and one thirteen seconds later refused.
        let mut state = LoginState::default();
        state.record_attempt(at(2_000));
        state.record_attempt(at(2_013));
        state.record_refusal(at(2_013));
        // Reading the entry renewed it, so the wait is not what was left of
        // the first login's twenty seconds but a fresh twenty from now.
        assert_eq!(state.wait(at(2_013)), Some(Duration::from_secs(21)));
        assert_eq!(state.wait(at(2_033)), Some(Duration::from_secs(1)));
        assert_eq!(state.wait(at(2_034)), None);
    }

    #[test]
    fn a_success_clears_a_stale_refusal_but_not_the_attempt() {
        let mut state = LoginState::default();
        state.record_refusal(at(3_000));
        state.record_attempt(at(3_030));
        state.record_success(at(3_030));
        // The refusal is over; the login that succeeded still counts.
        assert_eq!(state.wait(at(3_030)), Some(Duration::from_secs(21)));
        assert_eq!(state.wait(at(3_051)), None);
    }

    #[test]
    fn quitting_between_attempts_counts_towards_the_wait() {
        // The complaint this models: the clock should not restart because the
        // app did. One login, then the app is gone for eight seconds.
        let dir = tempfile::tempdir().unwrap();
        let guard = LoginGuard::new(dir.path());
        guard.record_attempt(at(2_000));
        let restarted = LoginGuard::new(dir.path());
        assert_eq!(restarted.wait(at(2_008)), Some(Duration::from_secs(13)));
        assert_eq!(restarted.wait(at(2_021)), None, "waited out while quit");
    }

    #[test]
    fn the_file_a_previous_build_wrote_is_read_as_clear() {
        // The old shape had a count and a window; neither field is known now,
        // and an unknown file must never hold a login back.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(FILE_NAME),
            r#"{"count": 3, "windowEnds": 1789074336, "blockedUntil": 0}"#,
        )
        .unwrap();
        assert_eq!(LoginGuard::new(dir.path()).wait(at(1_789_074_000)), None);
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
