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

// The executor that applies these effects is the next piece. Until it exists
// the decision layer is unreachable from the app — but not untested, which is
// the point of building it in this order.
#![allow(dead_code)]

pub mod state;

pub use state::{Effect, Input, Overlay, OverlaySettings, step};
