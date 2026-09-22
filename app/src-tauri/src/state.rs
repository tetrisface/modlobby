//! What every command can reach: the runtime client, the settings store, the
//! credential store and this machine's identity.

use std::sync::Arc;

use content::launcher::BarConfig;
use lobby_runtime::{Client, Hardware, IcmpEcho, platform};
use settings::model::Builtin;
use settings::{
	CredentialStore, KeyringStore, LoginGuard, MemoryStore, RejoinMemory, Store, UpdateMemory,
	credentials,
};
use spring_protocol::{ThrottlePolicy, server_id};

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
	/// Reads a rapid server that is not BAR's before anything is fetched
	/// from it; shared with the runtime, so BAR's names are read once.
	pub rapid: std::sync::Arc<content::rapid::Vetter>,
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
	/// The local network: where the `lan` server points, and the room hosted.
	pub lan: crate::lan::Lan,
	/// Where BAR's lobby, games and maps are, as BAR's launcher config said
	/// at this start.
	pub bar: BarConfig,
	/// Whether that was fetched recently enough to stand for the run.
	bar_fresh: bool,
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
		let credentials = credential_store();
		// Before anything connects, so BAR's entry is where BAR says it is.
		let held = content::launcher::cached(&cache_dir, std::time::SystemTime::now());
		follow_bar(&settings, credentials.as_ref(), &held.applied, &held.config);
		content::launcher::mark_applied(&cache_dir, &held.config);
		let hardware = platform::detect();
		let lan = crate::lan::Lan::default();
		let client = tauri::async_runtime::block_on(async {
			Client::spawn_with(
				ThrottlePolicy::default(),
				hardware.clone(),
				crate::lan::connector(lan.target()),
				Arc::new(IcmpEcho),
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
			credentials,
			hardware,
			pve: pve::Service::new(http.clone(), pve::ENDPOINT),
			thumbs: content::map_thumb::Service::new(http.clone(), &cache_dir),
			rapid: std::sync::Arc::new(content::rapid::Vetter::new(
				http.clone(),
				held.config.rapid_master.clone(),
			)),
			http,
			map_index: tokio::sync::Mutex::new(MapIndexHeld::default()),
			news: tokio::sync::Mutex::new(None),
			widget_usage: tokio::sync::Mutex::new(WidgetUsageHeld::default()),
			game_files: Arc::new(content::game_cache::GameFileCache::new()),
			engine_downloads: tokio::sync::Mutex::new(()),
			lan,
			bar: held.config,
			bar_fresh: held.fresh,
		})
	}

	/// Asks for BAR's launcher config again once the copy on disk is a week
	/// old. What it says takes hold at the next start, before anything
	/// connects; a server moved mid-session would be a stranger thing.
	pub async fn refresh_bar_config(&self) {
		if self.bar_fresh {
			return;
		}
		let fetched = content::launcher::refresh(
			&self.http,
			content::launcher::CONFIG_URL,
			&self.settings.dir().join("cache"),
			std::time::SystemTime::now(),
		)
		.await;
		match fetched {
			Ok(config) if config != self.bar => {
				tracing::info!(
					?config,
					"BAR's launcher config changed; it takes hold at the next start"
				)
			}
			Ok(_) => tracing::debug!("BAR's launcher config: as it was"),
			Err(err) => tracing::warn!(%err, "BAR's launcher config: not refreshed"),
		}
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
			// Whose maps are BAR's decides who a map is asked of.
			let names = index.names.values().cloned().collect();
			let _ = self.client.set_bar_maps(names).await;
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
	/// A game from outside the room's rapid (`content::sources`), for the
	/// runtime's [`lobby_runtime::GameSources`]. Before rapid only the
	/// player's overrides answer; after it, modlobby's list and then the hub,
	/// whose copy is asked for again once it is a day old. BAR's own names
	/// are never looked for anywhere but BAR's rapid.
	pub async fn game_from_sources(
		&self,
		ask: lobby_runtime::GameAsk,
		progress: tokio::sync::mpsc::Sender<recoil::Progress>,
	) -> Option<Result<String, String>> {
		use content::sources;
		if self
			.rapid
			.bar_names()
			.await
			.is_ok_and(|names| names.contains(&ask.name))
		{
			return None;
		}
		let found = if ask.after_rapid {
			let cache = self.settings.dir().join("cache");
			let now = std::time::SystemTime::now();
			let (mut hub, fresh) = sources::cached(&cache, now);
			if !fresh {
				match sources::refresh(&self.http, sources::HUB_URL, &cache, now).await {
					Ok(list) => hub = list,
					Err(err) => tracing::warn!(%err, "coilbox's hub list: not refreshed"),
				}
			}
			sources::resolve(&ask.name, &[], &sources::shipped(), &hub)
		} else {
			let overrides: Vec<(String, sources::Target)> = self
				.settings
				.get()
				.games
				.overrides
				.into_iter()
				.map(|kept| (kept.name, kept.source))
				.collect();
			sources::resolve(&ask.name, &overrides, &[], &[])
		};
		if found.is_empty() {
			return None;
		}
		let games = ask.dirs.write.join("games");
		let fetched = sources::fetch(
			&self.http,
			sources::GITHUB_API,
			&found,
			&games,
			|current, total| {
				let _ = progress.try_send(recoil::Progress { current, total });
			},
		)
		.await?;
		Some(match fetched {
			Ok(fetched) => hold_to_room(fetched, ask).await,
			Err(reason) => Err(reason),
		})
	}

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

/// A game fetched from outside rapid, held to the hash its room announced:
/// kept when it has the room's checksum, set aside where the engine will not
/// load it when it has not, and kept with a word said when it could not be
/// told.
async fn hold_to_room(
	fetched: content::sources::Fetched,
	ask: lobby_runtime::GameAsk,
) -> Result<String, String> {
	use content::checksum::{Verdict, against_room};
	let set_aside = |why: String| {
		let kept = content::sources::set_aside(&fetched.path, &ask.dirs.write)
			.map(|aside| format!("set aside in {}", aside.display()))
			.unwrap_or_else(|err| format!("and could not be set aside: {err}"));
		Err(format!("{}: {why}; {kept}", fetched.from))
	};
	let Some(room) = ask.room_hash else {
		if fetched.built {
			return set_aside(
				"a build is only kept once its checksum is a room's, and there is no room".into(),
			);
		}
		return Ok(fetched.from);
	};
	let path = fetched.path.clone();
	let dirs = ask.dirs.clone();
	let engine = ask.engine.clone();
	let verdict = tokio::task::spawn_blocking(move || {
		let library = content::Library::new(dirs);
		against_room(&path, room, &|name| library.archive_named(&engine, name))
	})
	.await
	.unwrap_or_else(|err| Verdict::Unchecked(err.to_string()));
	match verdict {
		Verdict::Matches => Ok(format!("{}; checksum {room}, the room's", fetched.from)),
		Verdict::Unchecked(why) if fetched.built => set_aside(format!(
			"a build is only kept once its checksum is the room's, which could not be checked ({why})"
		)),
		Verdict::Unchecked(why) => {
			tracing::warn!(game = ask.name, %why, "not checked against the room");
			Ok(format!(
				"{} (not checked against the room: {why})",
				fetched.from
			))
		}
		Verdict::Differs { ours, room } => set_aside(format!(
			"not the room's files (checksum {ours}, the room's {room})"
		)),
	}
}

/// Moves BAR's entry with BAR's launcher config: whatever of its host, port
/// and name still says what the config said when last followed (`was`) now
/// says what it says (`now`), and a password kept for the old host is kept
/// for the new one too. What somebody set otherwise stays theirs: a host
/// typed over BAR's is somewhere they chose to go.
fn follow_bar(settings: &Store, store: &dyn CredentialStore, was: &BarConfig, now: &BarConfig) {
	if was == now {
		return;
	}
	let mut carried = Vec::new();
	let followed = settings.update(|settings| {
		let bars = settings
			.servers
			.iter_mut()
			.filter(|entry| entry.builtin == Some(Builtin::Bar));
		for entry in bars {
			if entry.host == was.host {
				entry.host.clone_from(&now.host);
				if !entry.username.is_empty() {
					carried.push(entry.username.clone());
				}
			}
			if entry.ports == [was.port] {
				entry.ports = vec![now.port];
			}
			if entry.name == was.name {
				entry.name.clone_from(&now.name);
			}
		}
	});
	if let Err(err) = followed {
		tracing::warn!(%err, "BAR's server could not follow its launcher config");
		return;
	}
	let (from, to) = (server_id(&was.host), server_id(&now.host));
	if from == to {
		return;
	}
	tracing::info!(%from, %to, "BAR's lobby moved, as its launcher config says");
	for username in carried {
		let moved = credentials::password(store, &from, &username).and_then(|kept| match kept {
			Some(password) => credentials::keep(store, &to, &username, &password),
			None => Ok(()),
		});
		if let Err(err) = moved {
			tracing::warn!(%err, %username, "the password did not move with BAR's lobby");
		}
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

#[cfg(test)]
mod tests {
	use settings::model::ServerEntry;

	use super::*;

	#[test]
	fn bars_entry_follows_the_config_but_keeps_what_was_set_by_hand() {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::open(dir.path()).unwrap();
		store
			.update(|settings| {
				settings.servers[0].username = "me".into();
				settings.servers[0].name = "Main".into();
			})
			.unwrap();
		let passwords = MemoryStore::default();
		credentials::keep(&passwords, settings::model::DEFAULT_HOST, "me", "secret").unwrap();

		let was = BarConfig::default();
		let now = BarConfig {
			host: "server5.beyondallreason.info".into(),
			port: 8300,
			name: "BAR 5".into(),
			..BarConfig::default()
		};
		follow_bar(&store, &passwords, &was, &now);

		assert_eq!(
			store.get().servers[0],
			ServerEntry {
				host: now.host.clone(),
				ports: vec![8300],
				name: "Main".into(),
				username: "me".into(),
				..ServerEntry::bar()
			}
		);
		assert_eq!(
			credentials::password(&passwords, "server5.beyondallreason.info", "me").unwrap(),
			Some("secret".into())
		);

		// A host typed over BAR's is not moved.
		store
			.update(|settings| settings.servers[0].host = "integration.example".into())
			.unwrap();
		follow_bar(&store, &passwords, &now, &was);
		assert_eq!(store.get().servers[0].host, "integration.example");
	}
}
