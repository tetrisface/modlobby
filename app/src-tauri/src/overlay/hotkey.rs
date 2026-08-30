//! The system-wide key, held only while a game runs.
//!
//! A global accelerator beats whatever has focus — that is the point, and also
//! the hazard: while it is registered it takes that combination away from the
//! game. So it is registered when a game starts and released the moment one
//! ends, and a lobby sitting idle owns no key at all.

use std::sync::Mutex;

use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use super::seams::Hotkeys;

pub struct GlobalHotkey {
    app: AppHandle,
    /// What is currently held, so unregistering names the same shortcut and a
    /// changed accelerator does not leak the old one.
    held: Mutex<Option<Shortcut>>,
}

impl GlobalHotkey {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            held: Mutex::new(None),
        }
    }
}

impl Hotkeys for GlobalHotkey {
    fn register(&self, accelerator: &str) {
        let shortcut: Shortcut = match accelerator.parse() {
            Ok(shortcut) => shortcut,
            Err(err) => {
                // A hand-edited settings file can say anything. Refusing to
                // arm is better than refusing to run.
                tracing::warn!(accelerator, %err, "not a usable shortcut");
                return;
            }
        };

        self.unregister();
        match self.app.global_shortcut().register(shortcut) {
            Ok(()) => *self.held.lock().expect("shortcut") = Some(shortcut),
            // Another application may already own it; the lobby is still
            // perfectly usable without the overlay.
            Err(err) => tracing::warn!(accelerator, %err, "could not take that shortcut"),
        }
    }

    fn unregister(&self) {
        let Some(shortcut) = self.held.lock().expect("shortcut").take() else {
            return;
        };
        if let Err(err) = self.app.global_shortcut().unregister(shortcut) {
            tracing::warn!(%err, "could not release the shortcut");
        }
    }
}
