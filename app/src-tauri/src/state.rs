//! What every command can reach: the runtime client, the settings store, the
//! credential store and this machine's identity.

use std::sync::Arc;

use lobby_runtime::{Client, Hardware, platform};
use settings::{CredentialStore, KeyringStore, LoginGuard, RejoinMemory, Store, UpdateMemory};
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
    /// When a newer release was last looked for.
    pub update_memory: UpdateMemory,
    /// Saved room setups, next to the settings they sit beside.
    pub presets: presets::Store,
    /// The one client every HTTP request leaves through: pooled, and named.
    pub http: reqwest::Client,
    /// BAR's PvE Stats service, with what it has already answered this run.
    pub pve: pve::Service,
    /// BAR's map index for this run, loaded the first time anything asks.
    map_index: tokio::sync::Mutex<Option<content::map_index::MapIndex>>,
    /// The map pictures at tile size, made here and kept under `cache/`.
    pub thumbs: content::map_thumb::Service,
    /// Held for the length of an engine download, so two never overlap.
    pub engine_downloads: tokio::sync::Mutex<()>,
}

impl App {
    /// Opens the settings directory and spawns the runtime on Tauri's async runtime.
    pub fn open() -> Result<Self, settings::Error> {
        // First, because building it installs the crypto provider that every
        // TLS user in the process — the lobby transport included — relies on
        // being there; without one, rustls panics rather than guesses.
        let http = content::http::client(env!("CARGO_PKG_VERSION"));
        let settings = Store::open(settings::config_dir())?;
        let cache_dir = settings.dir().join("cache");
        let hardware = platform::detect();
        let client = tauri::async_runtime::block_on(async {
            Client::spawn(
                ThrottlePolicy::default(),
                hardware.clone(),
                Some(settings.dir().join("latency.json")),
            )
        });
        Ok(Self {
            login_guard: LoginGuard::new(settings.dir()),
            rejoin: RejoinMemory::new(settings.dir()),
            update_memory: UpdateMemory::new(settings.dir()),
            presets: presets::Store::new(settings.dir()),
            client,
            settings,
            credentials: Arc::new(KeyringStore),
            hardware,
            pve: pve::Service::new(http.clone(), pve::ENDPOINT),
            thumbs: content::map_thumb::Service::new(http.clone(), &cache_dir),
            http,
            map_index: tokio::sync::Mutex::new(None),
            engine_downloads: tokio::sync::Mutex::new(()),
        })
    }

    /// BAR's map index: each map's picture and its spring name.
    ///
    /// Loaded once per run, from the disk cache when that is fresh. An empty
    /// answer — offline on a first run — is not kept, so the next ask tries
    /// again rather than leaving the whole session without pictures.
    pub async fn map_index(&self) -> content::map_index::MapIndex {
        let mut held = self.map_index.lock().await;
        if let Some(index) = held.as_ref() {
            return index.clone();
        }
        let index = content::map_index::load(
            &self.http,
            content::map_index::INDEX_URL,
            &self.settings.dir().join("cache"),
            std::time::SystemTime::now(),
        )
        .await;
        if !index.is_empty() {
            *held = Some(index.clone());
        }
        index
    }
}
