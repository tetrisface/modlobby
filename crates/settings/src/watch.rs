//! Live reload. Editors save as temp-file-plus-rename, so the directory is
//! watched rather than the file; events are debounced and our own writes are
//! skipped by content hash.

use std::path::Path;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::file::{FILE_NAME, Store};

const DEBOUNCE: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Starts watching; must be called on a tokio runtime.
    pub fn watch(&self) -> Result<Watch, notify::Error> {
        let (raw_tx, mut raw_rx) = mpsc::unbounded_channel::<()>();
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
        tokio::spawn(async move {
            while raw_rx.recv().await.is_some() {
                // Coalesce the burst an editor produces for one save.
                tokio::time::sleep(DEBOUNCE).await;
                while raw_rx.try_recv().is_ok() {}
                let event = match store.reload() {
                    Ok(Some(settings)) => SettingsEvent::Changed(settings),
                    Ok(None) => continue,
                    Err(err) => SettingsEvent::Invalid(err.to_string()),
                };
                if tx.send(event).await.is_err() {
                    return;
                }
            }
        });
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
