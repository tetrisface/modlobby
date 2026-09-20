//! Drives `lobby-core` against a live server and reports through a
//! [`UiTransport`](lobby_ui::UiTransport). The CLI and the Tauri app are both
//! thin callers of [`Client`].

pub mod client;
pub mod idle;
mod json_file;
pub mod latency;
pub mod launch;
mod misses;
pub mod platform;
pub mod player_files;
pub mod reconnect;
mod ways;

pub use client::{Ask, Client, ClientError, Connector, FromHost};
pub use latency::{IcmpEcho, Latency, Unmeasured};
/// Re-exported so callers do not need `lobby-core` just to name an action.
pub use lobby_core::{FriendAction, UnknownFriendAction};
pub use platform::Hardware;
