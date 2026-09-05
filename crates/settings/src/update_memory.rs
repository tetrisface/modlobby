//! When the app last looked for a newer release.
//!
//! The look is one small request for the release manifest, and once a day is
//! plenty for a lobby that is opened most evenings; a restart loop while
//! developing should not become a request per restart. The time is kept beside
//! the settings, not inside them: it is bookkeeping, not a preference.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "update-check.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Record {
    /// Unix seconds of the last completed look, `0` for never.
    last_checked: u64,
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
        let text = std::fs::read_to_string(&self.path).ok()?;
        let record: Record = serde_json::from_str(&text).ok()?;
        (record.last_checked > 0).then_some(record.last_checked)
    }

    /// Whether `every` has passed since the last look. Never looked is due;
    /// a clock set back is not, which errs towards quiet.
    pub fn due(&self, now: SystemTime, every: Duration) -> bool {
        let Some(last) = self.last_checked() else {
            return true;
        };
        unix(now).saturating_sub(last) >= every.as_secs()
    }

    /// Losable: a look that cannot be recorded happens again tomorrow, which
    /// costs one small request and nothing else.
    pub fn record(&self, now: SystemTime) {
        let record = Record {
            last_checked: unix(now),
        };
        let Ok(text) = serde_json::to_string_pretty(&record) else {
            return;
        };
        if let Err(err) = std::fs::write(&self.path, text) {
            tracing::warn!(%err, path = %self.path.display(), "could not record the update check");
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

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

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
}
