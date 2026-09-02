//! What every command can reach: the runtime client, the settings store, the
//! credential store and this machine's identity.

use std::sync::Arc;

use lobby_runtime::{Client, Hardware, platform};
use settings::{CredentialStore, KeyringStore, LoginGuard, RejoinMemory, Store};
use spring_protocol::ThrottlePolicy;

pub struct App {
    pub client: Client,
    pub settings: Store,
    pub credentials: Arc<dyn CredentialStore>,
    pub hardware: Hardware,
    /// Keeps us under teiserver's login limit across restarts.
    pub login_guard: LoginGuard,
    /// The room to offer back after a restart.
    pub rejoin: RejoinMemory,
    /// Saved room setups, next to the settings they sit beside.
    pub presets: presets::Store,
    /// The one client every HTTP request leaves through: pooled, and named.
    pub http: reqwest::Client,
    /// BAR's PvE Stats service, with what it has already answered this run.
    pub pve: pve::Service,
    /// BAR's map index for this run, loaded the first time anything asks.
    pub map_index: tokio::sync::Mutex<Option<content::map_index::MapIndex>>,
}

impl App {
    /// Opens the settings directory and spawns the runtime on Tauri's async runtime.
    pub fn open() -> Result<Self, settings::Error> {
        // First, because building it installs the crypto provider that every
        // TLS user in the process — the lobby transport included — relies on
        // being there; without one, rustls panics rather than guesses.
        let http = content::http::client(env!("CARGO_PKG_VERSION"));
        let settings = Store::open(settings::config_dir())?;
        let hardware = platform::detect();
        let client = tauri::async_runtime::block_on(async {
            Client::spawn(ThrottlePolicy::default(), hardware.clone())
        });
        Ok(Self {
            login_guard: LoginGuard::new(settings.dir()),
            rejoin: RejoinMemory::new(settings.dir()),
            presets: presets::Store::new(settings.dir()),
            client,
            settings,
            credentials: Arc::new(KeyringStore),
            hardware,
            pve: pve::Service::new(http.clone(), pve::ENDPOINT),
            http,
            map_index: tokio::sync::Mutex::new(None),
        })
    }
}
