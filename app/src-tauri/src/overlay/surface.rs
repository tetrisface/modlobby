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
        // Kept from the first entry only: entered again while still in this
        // shape, the window would remember the overlay as what to go back to.
        self.restore
            .lock()
            .expect("overlay geometry")
            .get_or_insert(remembered);

        let _ = self.window.set_decorations(false);
        // The shadow has to go with the decorations. On Windows an undecorated
        // window with a shadow keeps an invisible frame roughly 8px wide, so a
        // window told to sit at the monitor's corner actually lands shifted
        // right and down — which showed as a thin strip of raw, undimmed game
        // along the left edge of the overlay. Positioning happens after this,
        // so the coordinates land where they say.
        let _ = self.window.set_shadow(false);
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
        // Back on, unconditionally: a decorated window ignores it, and every
        // ordinary window wants its shadow.
        let _ = self.window.set_shadow(true);

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

    /// What the window is now, as the toolkit believes it and as the OS has
    /// it. The two can disagree: the toolkit keeps its own copy of the flags
    /// and skips a change it thinks is already made, so a shape that went
    /// wrong is only visible with both side by side.
    fn log_shape(&self, over: bool) {
        let believed = (
            self.window.is_decorated().ok(),
            self.window.is_always_on_top().ok(),
            self.window.is_visible().ok(),
        );
        let (style, ex_style) = self.os_styles();
        tracing::info!(
            over,
            decorated = ?believed.0,
            on_top = ?believed.1,
            visible = ?believed.2,
            style = format_args!("{style:#x}"),
            ex_style = format_args!("{ex_style:#x}"),
            "overlay: window shaped"
        );
    }

    /// `GWL_STYLE` and `GWL_EXSTYLE` straight from the window, so the log can
    /// say what the OS has rather than what was asked for.
    #[cfg(windows)]
    fn os_styles(&self) -> (isize, isize) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GWL_STYLE, GetWindowLongPtrW,
        };
        let Ok(hwnd) = self.window.hwnd() else {
            return (0, 0);
        };
        // SAFETY: a live handle to our own window.
        unsafe {
            (
                GetWindowLongPtrW(hwnd.0 as _, GWL_STYLE),
                GetWindowLongPtrW(hwnd.0 as _, GWL_EXSTYLE),
            )
        }
    }

    #[cfg(not(windows))]
    fn os_styles(&self) -> (isize, isize) {
        (0, 0)
    }
}

impl WindowSurface for TauriSurface {
    fn set_overlay(&self, over: bool) {
        if over {
            self.enter()
        } else {
            self.leave()
        }
        self.log_shape(over);
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
