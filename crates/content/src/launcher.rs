//! BAR's launcher config: where BAR sends its players.
//!
//! The file BAR's own launcher reads at every start. It names the lobby
//! server Chobby logs in to, and where pr-downloader finds games and maps,
//! per platform. modlobby takes the same answers from it, so BAR can move any
//! of them without a modlobby release; what modlobby was built with stands in
//! until the first fetch, and for anything the file stops saying.
//!
//! Fetched at most once a week and kept on disk raw, so a change to how it
//! is read here reads the copy already held the new way. It takes hold at the
//! next start: the settings, the runtime and the rapid vetter are each built
//! from it once, and a week's cache makes that no later than it would be.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const CONFIG_URL: &str = "https://launcher-config.beyondallreason.dev/config.json";

/// How long a fetched config is trusted.
pub const FRESH_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Under the config directory's `cache/`.
pub const CACHE_FILE: &str = "launcher-config.json";

/// The setup BAR's launcher installs on this platform, whose answers are
/// this platform's: Linux's stands for every other.
pub const SETUP: &str = if cfg!(windows) {
	"manual-win"
} else {
	"manual-linux"
};

/// What BAR's launcher config says, for one setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BarConfig {
	/// The lobby server Chobby logs in to.
	pub host: String,
	pub port: u16,
	/// What BAR calls that server.
	pub name: String,
	/// `PRD_RAPID_REPO_MASTER`: where BAR's games are.
	pub rapid_master: String,
	/// `PRD_HTTP_SEARCH_URL`: where BAR's maps are.
	pub search: String,
}

impl Default for BarConfig {
	/// What modlobby was built with; the host is `settings`' `DEFAULT_HOST`.
	fn default() -> Self {
		Self {
			host: "server4.beyondallreason.info".into(),
			port: 8200,
			name: "BAR".into(),
			rapid_master: recoil::RAPID_REPO_MASTER.into(),
			search: recoil::HTTP_SEARCH_URL.into(),
		}
	}
}

/// What `text` says for `setup`, each answer on its own: one that is missing
/// or will not do leaves the default in its place, and a file that is not
/// JSON at all is every default.
pub fn parse(text: &str, setup: &str) -> BarConfig {
	let fallback = BarConfig::default();
	let Ok(config) = serde_json::from_str::<Value>(text) else {
		return fallback;
	};
	let server = &config["json_files"]["chobby_config.json"]["server"];
	let env = config["setups"]
		.as_array()
		.and_then(|setups| {
			setups
				.iter()
				.find(|each| each["package"]["id"].as_str() == Some(setup))
		})
		.map_or(&Value::Null, |found| &found["env_variables"]);
	BarConfig {
		host: text_of(&server["address"])
			.filter(|host| is_host(host))
			.unwrap_or(fallback.host),
		port: server["port"]
			.as_u64()
			.and_then(|port| u16::try_from(port).ok())
			.filter(|port| *port > 0)
			.unwrap_or(fallback.port),
		name: text_of(&server["serverName"]).unwrap_or(fallback.name),
		rapid_master: text_of(&env["PRD_RAPID_REPO_MASTER"])
			.filter(|url| is_https(url))
			.unwrap_or(fallback.rapid_master),
		search: text_of(&env["PRD_HTTP_SEARCH_URL"])
			.filter(|url| is_https(url))
			.unwrap_or(fallback.search),
	}
}

fn text_of(value: &Value) -> Option<String> {
	value
		.as_str()
		.map(str::trim)
		.filter(|text| !text.is_empty())
		.map(str::to_owned)
}

/// A name and nothing else: what the settings would take for a host.
fn is_host(host: &str) -> bool {
	!host.contains(|c: char| c.is_whitespace() || matches!(c, '/' | ':' | '@'))
}

/// Only over https: pr-downloader fetches game code from these.
fn is_https(url: &str) -> bool {
	url.starts_with("https://")
}

/// What the cache file holds: the config as fetched, and when.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
	/// Seconds since the epoch of the last fetch.
	fetched_at: u64,
	body: String,
	/// What the settings were last brought in line with; see [`Held::applied`].
	#[serde(default)]
	applied: Option<BarConfig>,
}

/// The config held on disk, for this platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
	pub config: BarConfig,
	/// Fetched within [`FRESH_FOR`].
	pub fresh: bool,
	/// What it said when the settings last followed it -- the built-in answers
	/// until they have -- so that what still says that can be moved with it.
	pub applied: BarConfig,
}

/// The config held on disk; the built-in answers, not fresh, when there is none.
pub fn cached(cache_dir: &Path, now: SystemTime) -> Held {
	match read(&cache_dir.join(CACHE_FILE)) {
		Some(held) => Held {
			config: parse(&held.body, SETUP),
			fresh: seconds(now).saturating_sub(held.fetched_at) < FRESH_FOR.as_secs(),
			applied: held.applied.unwrap_or_default(),
		},
		None => Held {
			config: BarConfig::default(),
			fresh: false,
			applied: BarConfig::default(),
		},
	}
}

/// Notes that the settings now follow `config`.
pub fn mark_applied(cache_dir: &Path, config: &BarConfig) {
	let path = cache_dir.join(CACHE_FILE);
	if let Some(mut held) = read(&path)
		&& held.applied.as_ref() != Some(config)
	{
		held.applied = Some(config.clone());
		write(&path, &held);
	}
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error(transparent)]
	Http(#[from] reqwest::Error),
	#[error("HTTP {0}")]
	Status(u16),
	#[error("not JSON: {0}")]
	NotJson(#[from] serde_json::Error),
}

/// Fetches the config, keeps it, and says what it says for this platform.
/// `url` is a parameter so a test can point this at a server of its own.
pub async fn refresh(
	client: &reqwest::Client,
	url: &str,
	cache_dir: &Path,
	now: SystemTime,
) -> Result<BarConfig, Error> {
	let response = client.get(url).send().await?;
	let status = response.status();
	if !status.is_success() {
		return Err(Error::Status(status.as_u16()));
	}
	let body = response.text().await?;
	// Not kept unless it is JSON: a captive portal's page is not a config.
	serde_json::from_str::<Value>(&body)?;
	let config = parse(&body, SETUP);
	let path = cache_dir.join(CACHE_FILE);
	let applied = read(&path).and_then(|held| held.applied);
	write(
		&path,
		&Cached {
			fetched_at: seconds(now),
			body,
			applied,
		},
	);
	Ok(config)
}

fn read(path: &Path) -> Option<Cached> {
	let text = std::fs::read_to_string(path).ok()?;
	match serde_json::from_str(&text) {
		Ok(cached) => Some(cached),
		Err(err) => {
			tracing::warn!(%err, path = %path.display(), "launcher config cache not readable");
			None
		}
	}
}

/// Temp file and rename, so a crash never leaves half a config behind.
fn write(path: &Path, cached: &Cached) {
	let written = (|| {
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}
		let tmp: PathBuf = path.with_extension("json.tmp");
		std::fs::write(&tmp, serde_json::to_vec(cached)?)?;
		std::fs::rename(&tmp, path)
	})();
	if let Err(err) = written {
		tracing::warn!(%err, path = %path.display(), "launcher config cache not written");
	}
}

fn seconds(time: SystemTime) -> u64 {
	time.duration_since(UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

#[cfg(test)]
mod tests {
	use wiremock::matchers::{method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;

	/// The parts of BAR's config this reads, as published on 2026-09-21, with
	/// the Windows setup moved elsewhere to tell the two apart.
	const PUBLISHED: &str = r#"{
		"setups": [
			{ "package": { "id": "manual-linux" }, "env_variables": {
				"PRD_HTTP_SEARCH_URL": "https://files-cdn.beyondallreason.dev/find",
				"PRD_RAPID_USE_STREAMER": "false",
				"PRD_RAPID_REPO_MASTER": "https://repos-cdn.beyondallreason.dev/repos.gz" } },
			{ "package": { "id": "manual-win" }, "env_variables": {
				"PRD_HTTP_SEARCH_URL": "https://files.example/find",
				"PRD_RAPID_REPO_MASTER": "https://repos.example/repos.gz" } }
		],
		"json_files": { "chobby_config.json": {
			"server": { "address": "server4.beyondallreason.info", "port": 8200, "protocol": "spring", "serverName": "BAR" },
			"game": "byar" } }
	}"#;

	#[test]
	fn each_setup_reads_its_own_and_the_lobby_is_everyones() {
		let linux = parse(PUBLISHED, "manual-linux");
		assert_eq!(linux, BarConfig::default());
		let windows = parse(PUBLISHED, "manual-win");
		assert_eq!(windows.rapid_master, "https://repos.example/repos.gz");
		assert_eq!(windows.search, "https://files.example/find");
		assert_eq!(windows.host, "server4.beyondallreason.info");
		assert_eq!(windows.port, 8200);
		assert_eq!(windows.name, "BAR");
	}

	#[test]
	fn an_answer_that_will_not_do_leaves_the_built_in_one() {
		let moved = PUBLISHED
			.replace(
				"server4.beyondallreason.info",
				"server5.beyondallreason.info",
			)
			.replace("https://repos-cdn", "http://repos-cdn")
			.replace("8200", "0");
		let read = parse(&moved, "manual-linux");
		assert_eq!(read.host, "server5.beyondallreason.info");
		assert_eq!(
			read.rapid_master,
			BarConfig::default().rapid_master,
			"not over https"
		);
		assert_eq!(read.port, 8200);
		assert_eq!(parse("<html>", "manual-linux"), BarConfig::default());
		assert_eq!(
			parse(PUBLISHED, "manual-mac").search,
			BarConfig::default().search
		);
	}

	#[tokio::test]
	async fn a_fetched_config_is_kept_for_a_week_and_a_failed_one_changes_nothing() {
		let dir = tempfile::tempdir().unwrap();
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(path("/config.json"))
			.respond_with(ResponseTemplate::new(200).set_body_string(PUBLISHED))
			.expect(2)
			.mount(&server)
			.await;
		let url = format!("{}/config.json", server.uri());
		let client = crate::http::client("test");
		let now = SystemTime::now();

		assert!(!cached(dir.path(), now).fresh);
		refresh(&client, &url, dir.path(), now).await.unwrap();
		assert!(cached(dir.path(), now).fresh);
		assert!(!cached(dir.path(), now + FRESH_FOR).fresh);

		// What the settings followed is kept across a refresh.
		let moved = BarConfig {
			host: "server5.beyondallreason.info".into(),
			..BarConfig::default()
		};
		mark_applied(dir.path(), &moved);
		refresh(&client, &url, dir.path(), now).await.unwrap();
		assert_eq!(cached(dir.path(), now).applied, moved);

		let broken = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
			.expect(1)
			.mount(&broken)
			.await;
		let refused = refresh(&client, &broken.uri(), dir.path(), now).await;
		assert!(matches!(refused, Err(Error::NotJson(_))));
		assert!(
			cached(dir.path(), now).fresh,
			"the good copy is still there"
		);
	}
}
