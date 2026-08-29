//! The seam between the runtime and whatever renders the lobby.

use std::sync::{Arc, Mutex};

use crate::model::UiMessage;

/// The front end is gone; the runtime drops the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiClosed;

pub trait UiTransport: Send + 'static {
    fn send(&self, message: UiMessage) -> Result<(), UiClosed>;
}

/// Test double: keeps every message.
#[derive(Debug, Clone, Default)]
pub struct Collector(Arc<Mutex<Vec<UiMessage>>>);

impl Collector {
    pub fn take(&self) -> Vec<UiMessage> {
        std::mem::take(&mut self.0.lock().expect("collector lock"))
    }
}

impl UiTransport for Collector {
    fn send(&self, message: UiMessage) -> Result<(), UiClosed> {
        self.0.lock().expect("collector lock").push(message);
        Ok(())
    }
}
