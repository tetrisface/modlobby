//! The lobby as an overlay while a game runs.
//!
//! One global hotkey raises this window over the engine and gives the game
//! focus back on the next press. There is no second webview and no Lua inside
//! the engine: the runtime keeps exactly one UI transport, so a second window
//! would take the stream from the first — the main window changes shape
//! instead.
//!
//! [`state`] decides; everything that touches the operating system is a trait
//! it knows nothing about.

pub mod controller;
pub mod foreground;
pub mod hotkey;
pub mod seams;
pub mod state;
pub mod surface;

pub use controller::Controller;
pub use state::OverlaySettings;
