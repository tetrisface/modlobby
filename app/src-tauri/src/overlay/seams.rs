//! The things the overlay has to ask the operating system for.
//!
//! Traits rather than direct calls, for the reason the rest of this repo uses
//! them: the decision layer stays testable, and the parts that can only be
//! tried on a real desktop are small enough to read in one sitting.

/// A monitor, by whatever the platform names one: equal for two windows on
/// the same one, and nothing more is read into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenId(pub isize);

/// A window of another process, by whatever the platform names one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowId(pub isize);

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
	/// Its ordinary shape back, unseen, and out of the way: directly behind
	/// the window `behind` -- the game's -- or, with none to name, wherever
	/// the platform leaves a window nobody is looking at. Where it can, it
	/// stays somewhere the player can get it back from without any help.
	fn lower(&self, behind: Option<WindowId>);
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
	/// That window itself, where the platform can name it and the process has
	/// one yet.
	fn window_of(&self, pid: u32) -> Option<WindowId>;
	/// Whether the window in front is that process's, so the keyboard is its.
	fn in_front(&self, pid: u32) -> bool;
}

/// One look at the keys Escape depends on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Keys {
	/// Down now, or pressed since the last look.
	pub escape: bool,
	/// Shift, Ctrl, Alt or the Windows key, down now.
	pub modifier: bool,
}

/// The keyboard, read and never taken: every key still goes where it was going.
pub trait Keyboard: Send + Sync {
	fn keys(&self) -> Keys;
}

/// A system-wide accelerator, held only while a game runs.
pub trait Hotkeys: Send + Sync {
	fn register(&self, accelerator: &str);
	fn unregister(&self);
}
