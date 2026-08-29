//! `UiTransport` over a Tauri channel: one ordered stream into the webview.

use lobby_ui::{UiClosed, UiMessage, UiTransport};
use tauri::ipc::Channel;

pub struct ChannelTransport(pub Channel<UiMessage>);

impl UiTransport for ChannelTransport {
    fn send(&self, message: UiMessage) -> Result<(), UiClosed> {
        self.0.send(message).map_err(|_| UiClosed)
    }
}
