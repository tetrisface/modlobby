//! The main window, told to change shape.
//!
//! There is one window and it morphs. A second window is not an option: the
//! runtime keeps exactly one UI transport (`Command::Subscribe` replaces it),
//! so an overlay window would take the stream from the lobby behind it.
//!
//! Overlay shape is borderless, always-on-top, off the taskbar, and covering
//! the monitor — deliberately not `set_fullscreen(true)`, which asks the OS
//! for its own fullscreen treatment and then fights the game for it.

use std::sync::Mutex;

use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use super::seams::WindowSurface;

/// What the window looked like before it became an overlay.
#[derive(Debug, Clone, Copy)]
struct Restore {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    decorated: bool,
}

pub struct TauriSurface {
    window: WebviewWindow,
    /// Held in memory only. Overlay geometry is never persisted, so a crash
    /// mid-overlay cannot leave a decorationless always-on-top window behind
    /// on the next start.
    restore: Mutex<Option<Restore>>,
}

impl TauriSurface {
    pub fn new(window: WebviewWindow) -> Self {
        Self {
            window,
            restore: Mutex::new(None),
        }
    }

    fn enter(&self) {
        let remembered = Restore {
            position: self.window.outer_position().unwrap_or_default(),
            size: self.window.outer_size().unwrap_or_default(),
            decorated: self.window.is_decorated().unwrap_or(true),
        };
        *self.restore.lock().expect("overlay geometry") = Some(remembered);

        let _ = self.window.set_decorations(false);
        let _ = self.window.set_always_on_top(true);
        let _ = self.window.set_skip_taskbar(true);

        // The monitor the window is on, which after a game launch is the one
        // the game is on — the engine takes the same screen the lobby was on.
        if let Ok(Some(monitor)) = self.window.current_monitor() {
            let _ = self.window.set_position(*monitor.position());
            let _ = self.window.set_size(*monitor.size());
        }
    }

    fn leave(&self) {
        let remembered = self.restore.lock().expect("overlay geometry").take();
        let _ = self.window.set_always_on_top(false);
        let _ = self.window.set_skip_taskbar(false);

        let Some(remembered) = remembered else {
            // Nothing recorded: at least give the frame back rather than
            // leaving a borderless window nobody can move.
            let _ = self.window.set_decorations(true);
            return;
        };
        let _ = self.window.set_decorations(remembered.decorated);
        let _ = self.window.set_position(remembered.position);
        let _ = self.window.set_size(remembered.size);
    }
}

impl WindowSurface for TauriSurface {
    fn set_overlay(&self, over: bool) {
        if over {
            self.enter()
        } else {
            self.leave()
        }
        // The page dresses differently over a game — a centred card on a
        // see-through scrim, Esc and click-outside to leave — so it is told.
        // A webview that reloads mid-overlay asks `overlay_active` instead.
        let _ = self.window.emit("overlay", over);
    }

    fn show(&self) {
        let _ = self.window.show();
        let _ = self.window.unminimize();
    }

    fn hide(&self) {
        let _ = self.window.hide();
    }

    fn focus(&self) {
        let _ = self.window.set_focus();
    }
}

/// The main window, or nothing if it has gone.
pub fn main_window(app: &tauri::AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}
