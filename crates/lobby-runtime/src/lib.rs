//! Drives `lobby-core` against a live server and reports through a
//! [`UiTransport`](lobby_ui::UiTransport). The CLI and the Tauri app are both
//! thin callers of [`Client`].

pub mod client;
pub mod launch;
pub mod platform;

pub use client::{Client, ClientError, Connector};
pub use platform::Hardware;
