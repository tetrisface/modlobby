//! `UiTransport` over a Tauri channel: one ordered stream into the webview.
//!
//! The overlay listens in on the way past. Every message the front end sees
//! goes through here, so this is where the engine's comings and goings can be
//! noticed without asking the webview to report them — which matters because
//! a page that has stopped responding is exactly when raising the lobby over
//! the game is worth having.

use std::sync::Arc;

use lobby_ui::{UiClosed, UiMessage, UiTransport};
use tauri::ipc::Channel;

use crate::overlay::Controller;

pub struct ChannelTransport {
    pub channel: Channel<UiMessage>,
    pub overlay: Arc<Controller>,
}

impl UiTransport for ChannelTransport {
    fn send(&self, message: UiMessage) -> Result<(), UiClosed> {
        self.overlay.observe(&message);
        self.channel.send(message).map_err(|_| UiClosed)
    }
}
