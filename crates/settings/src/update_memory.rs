//! When the app last looked for a newer release, and how soon it looks again.
//!
//! The look is one small request for the release manifest, and once a day is
//! plenty for a lobby that is opened most evenings; a restart loop while
//! developing should not become a request per restart. A session that went
//! wrong is the exception: the fix for it may already be out, so the next look
//! is due an hour after the last, then two, then three, back up to the day.
//! The next clean session drops straight back to daily. Nothing about what
//! went wrong is sent anywhere -- the client only looks sooner.
//!
//! The record is kept beside the settings, not inside them: it is
//! bookkeeping, not a preference.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "update-check.json";

pub const HOUR: Duration = Duration::from_secs(60 * 60);
pub const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// Where the ladder stops: an hour a step, so this many steps is the day.
const LAST_STEP: u32 = 24;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Record {
    /// Unix seconds of the last completed look, `0` for never.
    last_checked: u64,
    /// Set when a session begins, cleared when it ends. Still set when the
    /// next one begins means the last never got to end: a crash, a kill, the
    /// power.
    running: bool,
    /// This session raised an error or panicked.
    trouble: bool,
    /// Hours between looks while backing off after a bad session; `0` is the
    /// daily cadence.
    steps: u32,
}

#[derive(Debug, Clone)]
pub struct UpdateMemory {
    path: PathBuf,
}

impl UpdateMemory {
    pub fn new(config_dir: impl AsRef<Path>) -> Self {
        Self {
            path: config_dir.as_ref().join(FILE_NAME),
        }
    }

    /// Unix seconds of the last completed look, or `None` for never.
    pub fn last_checked(&self) -> Option<u64> {
        let last = self.read().last_checked;
        (last > 0).then_some(last)
    }

    /// Whether `every` has passed since the last look. Never looked is due;
    /// a clock set back is not, which errs towards quiet.
    pub fn due(&self, now: SystemTime, every: Duration) -> bool {
        let Some(last) = self.last_checked() else {
            return true;
        };
        unix(now).saturating_sub(last) >= every.as_secs()
    }

    /// How long after the last look the next one is due.
    // ponytail: a crash *in* the update path climbs this ladder too, and so
    // looks more often; the ceiling and the clean-session reset bound it.
    pub fn interval(&self) -> Duration {
        match self.read().steps {
            0 => DAY,
            steps => HOUR * steps.min(LAST_STEP),
        }
    }

    /// A session begins. Returns whether the last one went wrong -- it never
    /// got to end, or it raised an error -- which starts the ladder, or keeps
    /// climbing it; a clean one resets it to the day.
    pub fn began(&self) -> bool {
        let mut went_wrong = false;
        self.change(|record| {
            went_wrong = record.running || record.trouble;
            record.steps = if went_wrong { record.steps.max(1) } else { 0 };
            record.running = true;
            record.trouble = false;
        });
        went_wrong
    }

    /// The session ended the way it meant to: quit, or handed to an installer.
    pub fn ended(&self) {
        self.change(|record| record.running = false);
    }

    /// Something went wrong this session, so the next one looks sooner.
    pub fn note_trouble(&self) {
        self.change(|record| record.trouble = true);
    }

    /// Losable: a look that cannot be recorded happens again at the next
    /// start that finds it due, which costs one small request and nothing else.
    pub fn record(&self, now: SystemTime) {
        self.change(|record| {
            record.last_checked = unix(now);
            if record.steps > 0 {
                record.steps = (record.steps + 1).min(LAST_STEP);
            }
        });
    }

    fn read(&self) -> Record {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn change(&self, edit: impl FnOnce(&mut Record)) {
        let mut record = self.read();
        edit(&mut record);
        let Ok(text) = serde_json::to_string_pretty(&record) else {
            return;
        };
        if let Err(err) = std::fs::write(&self.path, text) {
            tracing::warn!(%err, path = %self.path.display(), "could not write the update record");
        }
    }
}

fn unix(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_looked_is_due() {
        let dir = tempfile::tempdir().unwrap();
        let memory = UpdateMemory::new(dir.path());
        assert_eq!(memory.last_checked(), None);
        assert!(memory.due(SystemTime::now(), DAY));
    }

    #[test]
    fn a_look_today_is_not_due_until_tomorrow() {
        let dir = tempfile::tempdir().unwrap();
        let memory = UpdateMemory::new(dir.path());
        let now = SystemTime::now();
        memory.record(now);
        assert!(!memory.due(now, DAY));
        assert!(!memory.due(now + DAY - Duration::from_secs(1), DAY));
        assert!(memory.due(now + DAY, DAY));
        // A fresh handle reads the same file: the point is surviving a restart.
        assert!(!UpdateMemory::new(dir.path()).due(now, DAY));
    }

    #[test]
    fn a_clock_set_back_waits_rather_than_looking() {
        let dir = tempfile::tempdir().unwrap();
        let memory = UpdateMemory::new(dir.path());
        let now = SystemTime::now();
        memory.record(now);
        assert!(!memory.due(now - DAY, DAY));
    }

    #[test]
    fn a_file_that_is_not_a_record_means_never() {
        let dir = tempfile::tempdir().unwrap();
        let memory = UpdateMemory::new(dir.path());
        std::fs::write(dir.path().join(FILE_NAME), "not json").unwrap();
        assert_eq!(memory.last_checked(), None);
        assert!(memory.due(SystemTime::now(), DAY));
    }

    #[test]
    fn a_session_that_ends_keeps_the_day() {
        let dir = tempfile::tempdir().unwrap();
        let memory = UpdateMemory::new(dir.path());
        assert!(!memory.began());
        memory.ended();
        assert!(!memory.began());
        assert_eq!(memory.interval(), DAY);
    }

    #[test]
    fn a_session_that_never_ended_makes_the_next_look_an_hour_away() {
        let dir = tempfile::tempdir().unwrap();
        let memory = UpdateMemory::new(dir.path());
        memory.began();
        // No `ended`: the process went without saying so.
        assert!(UpdateMemory::new(dir.path()).began());
        assert_eq!(memory.interval(), HOUR);
    }

    #[test]
    fn an_error_in_a_session_that_ended_does_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let memory = UpdateMemory::new(dir.path());
        memory.began();
        memory.note_trouble();
        memory.ended();
        assert!(memory.began());
        assert_eq!(memory.interval(), HOUR);
    }

    #[test]
    fn each_look_adds_an_hour_until_the_day() {
        let dir = tempfile::tempdir().unwrap();
        let memory = UpdateMemory::new(dir.path());
        memory.note_trouble();
        memory.began();
        let now = SystemTime::now();
        for hours in 2..=LAST_STEP {
            memory.record(now);
            assert_eq!(memory.interval(), HOUR * hours);
        }
        memory.record(now);
        assert_eq!(memory.interval(), DAY);
    }

    #[test]
    fn another_bad_session_climbs_on_rather_than_starting_over() {
        let dir = tempfile::tempdir().unwrap();
        let memory = UpdateMemory::new(dir.path());
        memory.note_trouble();
        memory.began();
        memory.record(SystemTime::now());
        memory.record(SystemTime::now());
        memory.note_trouble();
        assert!(memory.began());
        assert_eq!(memory.interval(), HOUR * 3);
    }

    #[test]
    fn a_clean_session_drops_back_to_the_day() {
        let dir = tempfile::tempdir().unwrap();
        let memory = UpdateMemory::new(dir.path());
        memory.note_trouble();
        memory.began();
        memory.record(SystemTime::now());
        memory.ended();
        assert!(!memory.began());
        assert_eq!(memory.interval(), DAY);
    }

    #[test]
    fn a_record_from_before_the_ladder_is_a_clean_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), r#"{"lastChecked": 1700000000}"#).unwrap();
        let memory = UpdateMemory::new(dir.path());
        assert_eq!(memory.last_checked(), Some(1_700_000_000));
        assert!(!memory.began());
        assert_eq!(memory.interval(), DAY);
    }
}
