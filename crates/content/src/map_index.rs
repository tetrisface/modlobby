//! BAR's published map index, kept on disk and asked for again only when it
//! may have changed.
//!
//! The index is where a map's picture lives — an imagor URL keyed by a photo
//! reference nothing else knows — and where an archive's file name maps to the
//! spring name the engine wants. It is close to a megabyte of JSON for two
//! fields, published with an `ETag`, and it changes a few times a week. So the
//! trimmed pairs are kept in the config directory, and when they are a day old
//! the server is asked with `If-None-Match`, which it usually answers with a
//! bodiless 304.
//!
//! Nothing here fails loudly: a lobby has to work with no network at all, and
//! a room with no picture still has its start-box schematic. A stale index
//! beats none, and none is an empty one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The lobby feed of `beyond-all-reason/maps-metadata`, served with an ETag
/// and a 30-minute cache lifetime; `/latest/` is the only mutable path there.
pub const INDEX_URL: &str =
    "https://maps-metadata.beyondallreason.dev/latest/lobby_maps.validated.json";
/// How long a fetched index is trusted before the server is asked again.
pub const FRESH_FOR: Duration = Duration::from_secs(24 * 60 * 60);
/// Under the config directory's `cache/`.
pub const CACHE_FILE: &str = "map-index.json";

/// The two things a lobby needs from the index.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MapIndex {
    /// Spring name, exactly as `BATTLEOPENED` reports it, to the published
    /// preview URL.
    pub images: BTreeMap<String, String>,
    /// Archive file name without its extension (`acidicquarry_5.17`) to the
    /// spring name (`AcidicQuarry 5.17`), which nothing on disk records.
    pub names: BTreeMap<String, String>,
}

impl MapIndex {
    pub fn is_empty(&self) -> bool {
        self.images.is_empty() && self.names.is_empty()
    }
}

/// What the cache file holds: the trimmed index, when it was confirmed, and
/// the tag to confirm it with next time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
    #[serde(default)]
    etag: Option<String>,
    /// Seconds since the epoch of the last 200 or 304.
    fetched_at: u64,
    index: MapIndex,
}

impl Cached {
    fn fresh(&self, now: SystemTime) -> bool {
        seconds(now).saturating_sub(self.fetched_at) < FRESH_FOR.as_secs()
    }
}

/// One entry of the feed; only the fields read are named, the rest may grow.
#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "springName")]
    spring_name: Option<String>,
    filename: Option<String>,
    images: Option<Images>,
}

#[derive(Deserialize)]
struct Images {
    preview: Option<String>,
}

/// The feed cut down to what is kept.
pub fn trim(json: &str) -> Result<MapIndex, serde_json::Error> {
    let entries: Vec<Entry> = serde_json::from_str(json)?;
    let mut index = MapIndex::default();
    for entry in entries {
        let Some(name) = entry.spring_name else {
            continue;
        };
        if let Some(preview) = entry.images.and_then(|images| images.preview) {
            index.images.insert(name.clone(), preview);
        }
        if let Some(stem) = entry.filename.as_deref().map(archive_stem) {
            index.names.insert(stem, name);
        }
    }
    Ok(index)
}

/// `acidicquarry_5.17.sd7` → `acidicquarry_5.17`, which is what a listing of
/// the maps directory has to match on.
fn archive_stem(filename: &str) -> String {
    let lower = filename.to_ascii_lowercase();
    for ext in [".sd7", ".sdz"] {
        if let Some(stem) = lower.strip_suffix(ext) {
            return stem.to_owned();
        }
    }
    lower
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("{0}")]
    Request(#[from] reqwest::Error),
    #[error("the index answered {0}")]
    Status(u16),
    #[error("the index is not the JSON expected: {0}")]
    Body(#[from] serde_json::Error),
}

enum Fetched {
    /// A 304: what was cached is still what is published.
    Unchanged,
    New {
        etag: Option<String>,
        index: MapIndex,
    },
}

/// The index, from the cache when it is fresh, from the server when it is not,
/// and from a stale cache when the server cannot be reached.
///
/// `url` is a parameter so a test can point this at a server of its own.
pub async fn load(
    client: &reqwest::Client,
    url: &str,
    cache_dir: &Path,
    now: SystemTime,
) -> MapIndex {
    let path = cache_dir.join(CACHE_FILE);
    let cached = read(&path);
    if let Some(held) = &cached
        && held.fresh(now)
    {
        return held.index.clone();
    }

    let etag = cached.as_ref().and_then(|held| held.etag.as_deref());
    match fetch(client, url, etag).await {
        Ok(Fetched::Unchanged) => {
            let Some(mut held) = cached else {
                // A 304 to a request that named no tag is the server's
                // mistake; there is nothing to show for it.
                tracing::warn!("map index: 304 with nothing cached");
                return MapIndex::default();
            };
            tracing::debug!("map index: unchanged");
            held.fetched_at = seconds(now);
            write(&path, &held);
            held.index
        }
        Ok(Fetched::New { etag, index }) => {
            tracing::debug!(maps = index.images.len(), "map index: fetched");
            write(
                &path,
                &Cached {
                    etag,
                    fetched_at: seconds(now),
                    index: index.clone(),
                },
            );
            index
        }
        Err(err) => {
            tracing::warn!(%err, stale = cached.is_some(), "map index: not refreshed");
            cached.map(|held| held.index).unwrap_or_default()
        }
    }
}

async fn fetch(client: &reqwest::Client, url: &str, etag: Option<&str>) -> Result<Fetched, Error> {
    let mut request = client.get(url);
    if let Some(etag) = etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request.send().await?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Fetched::Unchanged);
    }
    if !status.is_success() {
        return Err(Error::Status(status.as_u16()));
    }
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response.text().await?;
    Ok(Fetched::New {
        etag,
        index: trim(&body)?,
    })
}

fn read(path: &Path) -> Option<Cached> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text)
        .inspect_err(
            |err| tracing::warn!(%err, path = %path.display(), "map index cache unreadable"),
        )
        .ok()
}

/// Temp file and rename, so a crash never leaves half an index behind. A
/// failure to write costs one fetch on the next run and is not worth more
/// than a log line.
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
        tracing::warn!(%err, path = %path.display(), "map index cache not written");
    }
}

fn seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const FEED: &str = r#"[
        {"springName": "AcidicQuarry 5.17", "filename": "acidicquarry_5.17.sd7",
         "images": {"preview": "https://maps.example/i/fit-in/1024x1024/a.jpg"}, "mapWidth": 12},
        {"springName": "No Picture 1", "filename": "no_picture_1.sdz"},
        {"filename": "nameless.sd7", "images": {"preview": "https://maps.example/x"}}
    ]"#;

    fn client() -> reqwest::Client {
        crate::http::client("test")
    }

    #[test]
    fn only_the_two_fields_are_kept_and_the_stem_is_the_file_name() {
        let index = trim(FEED).unwrap();
        assert_eq!(
            index.images.get("AcidicQuarry 5.17").map(String::as_str),
            Some("https://maps.example/i/fit-in/1024x1024/a.jpg")
        );
        assert_eq!(
            index.names.get("acidicquarry_5.17").map(String::as_str),
            Some("AcidicQuarry 5.17")
        );
        // A map with no picture still names its archive.
        assert!(!index.images.contains_key("No Picture 1"));
        assert_eq!(
            index.names.get("no_picture_1").map(String::as_str),
            Some("No Picture 1")
        );
        // An entry with no spring name is no use to anyone.
        assert!(!index.images.values().any(|url| url.ends_with("/x")));
        assert!(!index.names.contains_key("nameless"));
    }

    #[test]
    fn a_feed_that_is_not_a_list_is_an_error_not_a_panic() {
        assert!(trim("{}").is_err());
        assert!(trim("not json").is_err());
    }

    #[tokio::test]
    async fn the_first_load_fetches_and_remembers_the_tag() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/latest/lobby_maps.validated.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("ETag", "\"v1\"")
                    .set_body_string(FEED),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let url = format!("{}/latest/lobby_maps.validated.json", server.uri());

        let index = load(&client(), &url, dir.path(), SystemTime::now()).await;
        assert_eq!(index.images.len(), 1);

        let held = read(&dir.path().join(CACHE_FILE)).expect("a cache file");
        assert_eq!(held.etag.as_deref(), Some("\"v1\""));
        assert_eq!(held.index, index);
    }

    #[tokio::test]
    async fn within_a_day_the_server_is_not_asked() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FEED))
            .expect(0)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let held = Cached {
            etag: Some("\"v1\"".into()),
            fetched_at: seconds(now) - 60,
            index: trim(FEED).unwrap(),
        };
        write(&dir.path().join(CACHE_FILE), &held);

        let index = load(&client(), &server.uri(), dir.path(), now).await;
        assert_eq!(index, held.index);
    }

    #[tokio::test]
    async fn a_stale_cache_asks_with_its_tag_and_a_304_keeps_it() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("If-None-Match", "\"v1\""))
            .respond_with(ResponseTemplate::new(304))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let stale = seconds(now) - 2 * FRESH_FOR.as_secs();
        write(
            &dir.path().join(CACHE_FILE),
            &Cached {
                etag: Some("\"v1\"".into()),
                fetched_at: stale,
                index: trim(FEED).unwrap(),
            },
        );

        let index = load(&client(), &server.uri(), dir.path(), now).await;
        assert_eq!(index, trim(FEED).unwrap());
        let held = read(&dir.path().join(CACHE_FILE)).unwrap();
        assert!(held.fetched_at > stale, "the 304 counts as a confirmation");
        assert_eq!(held.etag.as_deref(), Some("\"v1\""));
    }

    #[tokio::test]
    async fn when_the_server_fails_the_stale_cache_is_served() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let stale = seconds(now) - 2 * FRESH_FOR.as_secs();
        write(
            &dir.path().join(CACHE_FILE),
            &Cached {
                etag: None,
                fetched_at: stale,
                index: trim(FEED).unwrap(),
            },
        );

        let index = load(&client(), &server.uri(), dir.path(), now).await;
        assert_eq!(index, trim(FEED).unwrap());
        let held = read(&dir.path().join(CACHE_FILE)).unwrap();
        assert_eq!(held.fetched_at, stale, "a failure confirms nothing");
    }

    #[tokio::test]
    async fn with_nothing_cached_and_no_server_the_index_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        // A port nothing listens on.
        let index = load(
            &client(),
            "http://127.0.0.1:9/latest/lobby_maps.validated.json",
            dir.path(),
            SystemTime::now(),
        )
        .await;
        assert!(index.is_empty());
        assert!(!dir.path().join(CACHE_FILE).exists());
    }
}
