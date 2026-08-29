//! Live reload. Editors save as temp-file-plus-rename, so the directory is
//! watched rather than the file; events are debounced and our own writes are
//! skipped by content hash.

use std::path::Path;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::file::{FILE_NAME, Store};

const DEBOUNCE: Duration = Duration::from_millis(200);

/// Serialised for the front end as `{ changed: Settings }` or `{ invalid: string }`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SettingsEvent {
    /// The user changed the file and it parsed.
    Changed(crate::Settings),
    /// The user changed the file and it did not parse; the previous settings stay in force.
    Invalid(String),
}

/// A running watcher; dropping it stops the events.
pub struct Watch {
    _watcher: RecommendedWatcher,
    pub events: mpsc::Receiver<SettingsEvent>,
}

impl Store {
    /// Starts watching. Debouncing and re-reading happen on a plain thread —
    /// the file work is blocking anyway — so this can be called from anywhere,
    /// including a Tauri setup hook where no tokio runtime is entered yet.
    pub fn watch(&self) -> Result<Watch, notify::Error> {
        let (raw_tx, raw_rx) = std::sync::mpsc::channel::<()>();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else {
                    return;
                };
                if event.paths.iter().any(|p| is_settings_file(p)) {
                    let _ = raw_tx.send(());
                }
            })?;
        watcher.watch(self.dir(), RecursiveMode::NonRecursive)?;

        let (tx, events) = mpsc::channel(8);
        let store = self.clone();
        std::thread::Builder::new()
            .name("settings-watch".into())
            .spawn(move || {
                while raw_rx.recv().is_ok() {
                    // Coalesce the burst an editor produces for one save.
                    while raw_rx.recv_timeout(DEBOUNCE).is_ok() {}
                    let event = match store.reload() {
                        Ok(Some(settings)) => SettingsEvent::Changed(settings),
                        Ok(None) => continue,
                        Err(err) => SettingsEvent::Invalid(err.to_string()),
                    };
                    if tx.blocking_send(event).is_err() {
                        return;
                    }
                }
            })
            .expect("spawning the settings watch thread");
        Ok(Watch {
            _watcher: watcher,
            events,
        })
    }
}

fn is_settings_file(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn external_edits_arrive_and_own_writes_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut watch = store.watch().unwrap();

        store.update(|s| s.chat.max_lines = 5).unwrap();
        let external = "{ \"chat\": { \"maxLines\": 77 } }";
        std::fs::write(store.path(), external).unwrap();

        let event = tokio::time::timeout(Duration::from_secs(5), watch.events.recv())
            .await
            .expect("an event within 5 s")
            .expect("watcher alive");
        let SettingsEvent::Changed(settings) = event else {
            panic!("expected Changed, got {event:?}")
        };
        assert_eq!(settings.chat.max_lines, 77);
        assert_eq!(store.get().chat.max_lines, 77);

        std::fs::write(store.path(), "{ broken").unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), watch.events.recv())
            .await
            .expect("an event within 5 s")
            .expect("watcher alive");
        assert!(matches!(event, SettingsEvent::Invalid(_)));
        assert_eq!(store.get().chat.max_lines, 77);
    }
}
