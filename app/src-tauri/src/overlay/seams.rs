//! The three things the overlay has to ask the operating system for.
//!
//! Traits rather than direct calls, for the reason the rest of this repo uses
//! them: the decision layer stays testable, and the parts that can only be
//! tried on a real desktop are small enough to read in one sitting.

/// The window this app already owns, told to change shape.
pub trait WindowSurface: Send + Sync {
    /// Borderless, always on top, filling the monitor the pointer is on;
    /// or back to whatever it was before.
    fn set_overlay(&self, over: bool);
    fn show(&self);
    fn hide(&self);
    fn focus(&self);
}

/// Putting a window belonging to another process in front.
pub trait ForegroundControl: Send + Sync {
    /// Brings the first visible top-level window of `pid` forward.
    fn focus(&self, pid: u32);
}

/// A system-wide accelerator, held only while a game runs.
pub trait Hotkeys: Send + Sync {
    fn register(&self, accelerator: &str);
    fn unregister(&self);
}

/// Nothing to do, for the platforms where there is nothing to be done.
///
/// macOS bounces a dock icon for the *application*, and Wayland deliberately
/// refuses to let one window raise itself over another at all — so on those
/// the overlay still shows and hides, it just cannot hand focus back, which is
/// worth logging once rather than failing.
pub struct NoForeground;

impl ForegroundControl for NoForeground {
    fn focus(&self, pid: u32) {
        tracing::debug!(pid, "no way to focus another process's window here");
    }
}
