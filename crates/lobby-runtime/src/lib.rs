//! Drives `lobby-core` against a live server and reports through a
//! [`UiTransport`](lobby_ui::UiTransport). The CLI and the Tauri app are both
//! thin callers of [`Client`].

pub mod client;
pub mod idle;
pub mod latency;
pub mod launch;
pub mod platform;
pub mod reconnect;

pub use client::{Client, ClientError, Connector};
pub use latency::{IcmpEcho, Latency, Unmeasured};
/// Re-exported so callers do not need `lobby-core` just to name an action.
pub use lobby_core::{FriendAction, UnknownFriendAction};
pub use platform::Hardware;
