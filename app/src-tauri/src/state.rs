//! What every command can reach: the runtime client, the settings store, the
//! credential store and this machine's identity.

use std::sync::Arc;

use lobby_runtime::{Client, Hardware, platform};
use settings::{CredentialStore, KeyringStore, LoginGuard, Store};
use spring_protocol::ThrottlePolicy;

pub struct App {
    pub client: Client,
    pub settings: Store,
    pub credentials: Arc<dyn CredentialStore>,
    pub hardware: Hardware,
    /// Keeps us under teiserver's login limit across restarts.
    pub login_guard: LoginGuard,
}

impl App {
    /// Opens the settings directory and spawns the runtime on Tauri's async runtime.
    pub fn open() -> Result<Self, settings::Error> {
        let settings = Store::open(settings::config_dir())?;
        let hardware = platform::detect();
        let client = tauri::async_runtime::block_on(async {
            Client::spawn(ThrottlePolicy::default(), hardware.clone())
        });
        Ok(Self {
            login_guard: LoginGuard::new(settings.dir()),
            client,
            settings,
            credentials: Arc::new(KeyringStore),
            hardware,
        })
    }
}
