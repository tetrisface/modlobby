//! The three things the overlay has to ask the operating system for.
//!
//! Traits rather than direct calls, for the reason the rest of this repo uses
//! them: the decision layer stays testable, and the parts that can only be
//! tried on a real desktop are small enough to read in one sitting.

/// A monitor, by whatever the platform names one: equal for two windows on
/// the same one, and nothing more is read into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenId(pub isize);

/// The window this app already owns, told to change shape.
pub trait WindowSurface: Send + Sync {
	/// Borderless, always on top, filling the monitor `on` -- the game's --
	/// or, where none is known, the one the window is on. Asked again while
	/// already in this shape, it fits the window to that monitor again and
	/// keeps the shape it first recorded to go back to.
	fn enter_overlay(&self, on: Option<ScreenId>);
	/// Back to whatever it was before.
	fn leave_overlay(&self);
	fn show(&self);
	/// Shown and focusable, but transparent until [`Self::reveal`]. A page
	/// that never reports back is not left invisible: the surface reveals on
	/// its own after a short wait.
	fn show_veiled(&self);
	fn reveal(&self);
	fn hide(&self);
	fn focus(&self);
	/// The monitor the window is on, where the platform can say.
	fn screen(&self) -> Option<ScreenId>;
	fn is_focused(&self) -> bool;
}

/// Putting a window belonging to another process in front.
pub trait ForegroundControl: Send + Sync {
	/// Brings the first visible top-level window of `pid` forward.
	fn focus(&self, pid: u32);
	/// The monitor that window is on, where the platform can say and the
	/// process has a window yet.
	fn screen_of(&self, pid: u32) -> Option<ScreenId>;
}

/// A system-wide accelerator, held only while a game runs.
pub trait Hotkeys: Send + Sync {
	fn register(&self, accelerator: &str);
	fn unregister(&self);
}
