//! What every command can reach: the runtime client, the settings store, the
//! credential store and this machine's identity.

use std::sync::Arc;

use lobby_runtime::{Client, Hardware, platform};
use settings::{
    CredentialStore, KeyringStore, LoginGuard, MemoryStore, RejoinMemory, Store, UpdateMemory,
};
use spring_protocol::ThrottlePolicy;

/// How long a map index that could not be fetched is not asked for again.
///
/// Long enough that a server having a bad minute is not asked for a megabyte
/// thirty times, short enough that the pictures come back in the same sitting.
const MAP_INDEX_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// How long a widget-usage document that could not be fetched is not asked
/// for again, when the service did not name a wait of its own.
///
/// The page is a tab somebody can click back onto; without this, a service
/// that is down is asked once per visit.
const WIDGET_USAGE_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// The map index for this run: what was loaded, or when loading last failed.
#[derive(Default)]
struct MapIndexHeld {
    index: Option<content::map_index::MapIndex>,
    failed_at: Option<std::time::Instant>,
}

/// The widget usage for this run: what was loaded, or when loading last failed
/// and how long it earned before being asked again.
#[derive(Default)]
struct WidgetUsageHeld {
    usage: Option<widgets::Usage>,
    failed: Option<(std::time::Instant, std::time::Duration)>,
}

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
    /// The pve.bar stats service, with what it has already answered this run.
    pub pve: pve::Service,
    /// BAR's map index for this run, loaded the first time anything asks.
    map_index: tokio::sync::Mutex<MapIndexHeld>,
    /// What of BAR's news has already been read, kept between runs.
    pub news_read: news::Memory,
    /// The news for this run, loaded the first time anything asks.
    news: tokio::sync::Mutex<Option<Vec<news::NewsItem>>>,
    /// What BAR players actually run, loaded the first time anything asks.
    /// The document is rebuilt weekly and cached on disk between runs, so once
    /// a run is plenty.
    widget_usage: tokio::sync::Mutex<WidgetUsageHeld>,
    /// The map pictures at tile size, made here and kept under `cache/`.
    pub thumbs: content::map_thumb::Service,
    /// Files read out of installed games — `modoptions.lua`, `luaai.lua` —
    /// kept for the run, since a rapid version never changes. Shared, so a
    /// command can read through it off the main thread.
    pub game_files: Arc<content::game_cache::GameFileCache>,
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
                Some(settings.dir().to_path_buf()),
            )
        });
        Ok(Self {
            login_guard: LoginGuard::new(settings.dir()),
            rejoin: RejoinMemory::new(settings.dir()),
            update_memory: UpdateMemory::new(settings.dir()),
            presets: presets::Store::new(settings.dir()),
            news_read: news::Memory::new(settings.dir()),
            client,
            settings,
            credentials: credential_store(),
            hardware,
            pve: pve::Service::new(http.clone(), pve::ENDPOINT),
            thumbs: content::map_thumb::Service::new(http.clone(), &cache_dir),
            http,
            map_index: tokio::sync::Mutex::new(MapIndexHeld::default()),
            news: tokio::sync::Mutex::new(None),
            widget_usage: tokio::sync::Mutex::new(WidgetUsageHeld::default()),
            game_files: Arc::new(content::game_cache::GameFileCache::new()),
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
        if let Some(index) = held.index.as_ref() {
            return index.clone();
        }
        // An empty index is not kept, so a run that started offline gets the
        // pictures once the network is back. But it is asked for by every
        // picture on the screen, and a server that is down would otherwise be
        // asked for a megabyte of JSON thirty times per battle list until it
        // came back. A failure is remembered for a while instead.
        if let Some(failed) = held.failed_at
            && failed.elapsed() < MAP_INDEX_RETRY_AFTER
        {
            return content::map_index::MapIndex::default();
        }
        let index = content::map_index::load(
            &self.http,
            content::map_index::INDEX_URL,
            &self.settings.dir().join("cache"),
            std::time::SystemTime::now(),
        )
        .await;
        if index.is_empty() {
            held.failed_at = Some(std::time::Instant::now());
        } else {
            held.index = Some(index.clone());
        }
        index
    }

    /// The published widget-usage document, held for the run.
    ///
    /// The fetch itself is the cheap part — [`widgets::load`] answers from
    /// disk while its copy is fresh and asks the server with `If-None-Match`
    /// when it is not — so this holds the parsed document rather than the
    /// bytes, and remembers a failure so a service that is down is not asked
    /// again on every visit to the page.
    ///
    /// A failure returns `None` rather than an error: usage is decoration on a
    /// widget list, and a page that renders without the numbers is a better
    /// outcome than one that refuses to render.
    pub async fn widget_usage(&self) -> Option<widgets::Usage> {
        let mut held = self.widget_usage.lock().await;
        if let Some(usage) = held.usage.as_ref() {
            return Some(usage.clone());
        }
        // The wait the service named is the one kept; ours only stands in when
        // it named none.
        if let Some((failed_at, hold)) = held.failed
            && failed_at.elapsed() < hold
        {
            return None;
        }
        let loaded = widgets::load(
            &self.http,
            widgets::ENDPOINT,
            &self.settings.dir().join("cache"),
            std::time::SystemTime::now(),
        )
        .await;
        match loaded.usage {
            Some(usage) => {
                held.usage = Some(usage.clone());
                held.failed = None;
                Some(usage)
            }
            None => {
                let hold = loaded.retry_after.unwrap_or(WIDGET_USAGE_RETRY_AFTER);
                tracing::warn!(?hold, "widget usage unavailable");
                held.failed = Some((std::time::Instant::now(), hold));
                None
            }
        }
    }

    /// BAR's news, newest first.
    ///
    /// Loaded once per run, from the disk cache while that is inside the hour
    /// it is trusted for. An empty answer — offline, or a first run with no
    /// network — is not kept, so the next ask tries again rather than leaving
    /// the whole session with an empty page.
    pub async fn news(&self) -> Vec<news::NewsItem> {
        let mut held = self.news.lock().await;
        if let Some(items) = held.as_ref() {
            return items.clone();
        }
        let items = news::load(
            &self.http,
            news::FEED_URL,
            &self.settings.dir().join("cache"),
            std::time::SystemTime::now(),
        )
        .await;
        if !items.is_empty() {
            *held = Some(items.clone());
        }
        items
    }
}

/// Set to `memory` to keep passwords for this run only, in memory.
///
/// The OS keyring is per user, not per `MODLOBBY_CONFIG_DIR`: a second
/// instance on a scratch config still reads, overwrites and deletes the real
/// one's passwords, and a login with "remember" off deletes one. A test run
/// sets this so it can log in without touching anything that outlives it.
pub const CREDENTIALS_ENV: &str = "MODLOBBY_CREDENTIALS";

fn credential_store() -> Arc<dyn CredentialStore> {
    if std::env::var(CREDENTIALS_ENV).is_ok_and(|store| store == "memory") {
        tracing::warn!(
            "{CREDENTIALS_ENV}=memory: passwords are kept in memory for this run and forgotten with it"
        );
        return Arc::new(MemoryStore::default());
    }
    Arc::new(KeyringStore)
}
