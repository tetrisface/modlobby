//! The lobby's own fullscreen, because the OS one stopped being trustworthy.
//!
//! The main window is transparent — that is what lets the overlay show the
//! game through a scrim — and on Windows a transparent window pays twice: it
//! loses the standard title bar, and `set_fullscreen` stops covering the
//! taskbar reliably. Both windowed-mode jobs the title bar used to do have
//! moved into the page (the nav is a drag region, the corner buttons close and
//! toggle), and fullscreen is done here by hand: decorations and shadow off,
//! the window placed over the monitor's exact rectangle. That is what a
//! borderless game does, and covering the monitor exactly is what makes the
//! shell drop the taskbar behind it — no always-on-top involved, so alt-tab
//! still works like a normal window.
//!
//! The one wrinkle is a window that is *already* monitor-sized when fullscreen
//! is asked for — a stale fullscreen restored from a previous run, say. Saving
//! that as "what windowed looks like" would make the toggle a no-op forever,
//! so a monitor-sized restore is discarded in favour of a sane centred window.

use std::sync::Mutex;

use tauri::{LogicalSize, PhysicalPosition, PhysicalSize, Position, Size};

/// What the window looked like before fullscreen, to give back.
#[derive(Debug, Clone, Copy)]
struct Restore {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    maximized: bool,
}

/// Held by the app; `Some` while the window is fullscreen.
#[derive(Default)]
pub struct Screen {
    restore: Mutex<Option<Restore>>,
}

impl Screen {
    pub fn is_fullscreen(&self) -> bool {
        self.restore.lock().expect("screen restore").is_some()
    }
}

/// Whether the window is fullscreen right now, for drawing the toggle.
#[tauri::command]
pub fn is_fullscreen(screen: tauri::State<'_, Screen>) -> bool {
    screen.is_fullscreen()
}

/// Toggles the window between fullscreen and windowed; answers the new state.
#[tauri::command]
pub fn toggle_fullscreen(
    window: tauri::Window,
    screen: tauri::State<'_, Screen>,
) -> crate::commands::Result<bool> {
    let mut held = screen.restore.lock().expect("screen restore");

    if let Some(restore) = held.take() {
        // Back on before the geometry, so the shadow's invisible frame is
        // there when the coordinates land.
        let _ = window.set_shadow(true);
        let _ = window.set_position(Position::Physical(restore.position));
        let _ = window.set_size(Size::Physical(restore.size));
        if restore.maximized {
            let _ = window.maximize();
        }
        return Ok(false);
    }

    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .ok_or_else(|| crate::commands::ApiError::new("window", "no monitor to fill"))?;

    // A maximized window is unmaximized first: its rectangle is the work
    // area, which is nobody's idea of a windowed shape, while the geometry
    // *after* unmaximizing is the real one the user last arranged. The flag is
    // kept so leaving fullscreen goes back to maximized, not to a loose window.
    let maximized = window.is_maximized().unwrap_or(false);
    if maximized {
        let _ = window.unmaximize();
    }
    let position = window.outer_position().unwrap_or_default();
    let size = window.outer_size().unwrap_or_default();

    // A window that already covers the monitor has no windowed shape worth
    // remembering — restoring to it would make the toggle do nothing, which is
    // exactly the trap being escaped. Invent a reasonable one instead.
    let covers = size.width >= monitor.size().width && size.height >= monitor.size().height;
    let restore = if covers {
        let wanted = windowed_shape(monitor.size(), monitor.scale_factor());
        Restore {
            position: PhysicalPosition::new(
                monitor.position().x + centred(monitor.size().width, wanted.width),
                monitor.position().y + centred(monitor.size().height, wanted.height),
            ),
            size: wanted,
            maximized,
        }
    } else {
        Restore {
            position,
            size,
            maximized,
        }
    };
    *held = Some(restore);

    // Clear any fullscreen the OS believes in from a previous run, so the two
    // notions of fullscreen cannot stack.
    let _ = window.set_fullscreen(false);
    let _ = window.set_shadow(false);
    let _ = window.set_position(Position::Physical(*monitor.position()));
    let _ = window.set_size(Size::Physical(*monitor.size()));
    Ok(true)
}

/// The default window size, matching `tauri.conf.json`'s `width`/`height`.
const WINDOWED: (f64, f64) = (1280.0, 800.0);

/// A windowed shape that fits, for a window that has no remembered one.
///
/// The default is the size the app opens at, but a monitor can be smaller than
/// it — a 1024×768 projector, or any monitor once the scale factor is applied.
/// Handing back a shape larger than the screen would leave the window covering
/// the monitor again, which is the very state being escaped, so it is clamped.
fn windowed_shape(monitor: &PhysicalSize<u32>, scale: f64) -> PhysicalSize<u32> {
    let wanted = LogicalSize::new(WINDOWED.0, WINDOWED.1).to_physical::<u32>(scale);
    PhysicalSize::new(
        wanted.width.min(monitor.width),
        wanted.height.min(monitor.height),
    )
}

/// The offset that centres `inner` inside `outer`.
///
/// Unsigned, so the subtraction has to be saturating: a window wider than its
/// monitor would otherwise wrap to about two billion and place the window
/// somewhere off in the coordinate space, never to be seen again.
fn centred(outer: u32, inner: u32) -> i32 {
    (outer.saturating_sub(inner) / 2) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_monitor_smaller_than_the_default_still_gets_a_window_that_fits() {
        let small = PhysicalSize::new(1024, 768);
        let shape = windowed_shape(&small, 1.0);
        assert_eq!(shape, PhysicalSize::new(1024, 768));
    }

    #[test]
    fn scaling_is_what_makes_a_roomy_monitor_too_small() {
        // A 1920×1080 panel at 200% is 960×540 logical — less than the default.
        let panel = PhysicalSize::new(1920, 1080);
        assert_eq!(windowed_shape(&panel, 2.0), PhysicalSize::new(1920, 1080));
        assert_eq!(windowed_shape(&panel, 1.0), PhysicalSize::new(1280, 800));
    }

    #[test]
    fn centring_never_wraps_around() {
        assert_eq!(centred(1920, 1280), 320);
        // The case that used to send the window two billion pixels away.
        assert_eq!(centred(1024, 1280), 0);
    }
}
