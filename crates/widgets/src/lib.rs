//! What BAR players actually run, and how long they keep it.
//!
//! BAR's own Widget Hub says what is *offered*: a curated catalogue with cover
//! art and one-click install, already shipped and already good. What nothing
//! says is what is *used* — and that is recoverable, because every replay
//! carries it.
//!
//! Since 2025-11-13 the game's `ana_report_widgets.lua` broadcasts each
//! player's locally installed widget list at game frame 30, and the server
//! records that broadcast into the demo file. pve.bar reads it out of public
//! replays, resolves the reported hashes back to real widget sources, and
//! publishes aggregates. This crate reads those aggregates.
//!
//! **Why JSON and not the parquet the web app reads.** The same projection is
//! published in both shapes. A browser can range-read parquet in a worker
//! cheaply; a Tauri app cannot — its webview has no network access at all, so
//! every request comes through here, and pulling an Arrow stack into the binary
//! to decode tens of kilobytes would be a poor trade.
//!
//! **Everything here is an aggregate over distinct players.** The upstream
//! projection counts people, not sightings, so one enthusiast playing ten games
//! a day does not read as ten adopters; and a widget is withheld entirely below
//! a k-anonymity floor. No player is named, here or upstream.
//!
//! **Asked for once a day, and kept on disk in between.** The document is
//! rebuilt weekly, so a lobby that fetched it this morning has nothing to gain
//! from asking again tonight. It is kept in the config directory's `cache/`,
//! gzipped, and once past the freshness window the server is asked with
//! `If-None-Match` — which, for a file that changes weekly, answers a bodiless
//! `304` almost every time. Nothing here fails loudly: a stale document beats
//! none, and none is a page without numbers rather than an error.
//!
//! **Nothing retries in a loop.** Per `content::http`, the way this client
//! copes with a service having a bad minute is the last copy on disk plus a
//! wait — and when the service names that wait with `Retry-After`, that is the
//! wait honoured, not one of ours.
//!
//! The HTTP client is handed in, not built here, for the same reason as
//! [`pve`]: modlobby has one client for everything it asks BAR, so every
//! request carries the same name and shares one connection pool. It is built
//! with `gzip` and `brotli`, so the document arrives compressed and is decoded
//! before anything here sees it.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where the weekly pipeline publishes the usage document.
pub const ENDPOINT: &str = "https://d29i3oohxql6zz.cloudfront.net/widget_registry/usage.json";

/// Generous next to the ~40 KiB the document compresses to, and small enough
/// that a misrouted response cannot be read into memory unbounded.
pub const BODY_LIMIT: usize = 4 * 1024 * 1024;

/// How long a fetched document is trusted before the server is asked again.
///
/// The pipeline rebuilds weekly, so a day is already far tighter than the
/// thing it is watching; what it buys is that a lobby left open over a weekend
/// notices the new document without being restarted.
pub const FRESH_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// Under the config directory's `cache/`.
///
/// Gzipped, and named so: this is JSON published for a browser, and it
/// compresses to about a seventh of itself.
pub const CACHE_FILE: &str = "widget-usage.json.gz";

/// The longest wait a `Retry-After` is taken at its word for.
pub const RETRY_AFTER_CAP: Duration = Duration::from_secs(30);

/// The rolling windows the pipeline publishes.
///
/// They are rolling rather than calendar — "the last seven days", not "this
/// week" — anchored to the newest day of replay data rather than to the clock,
/// so the same input always produces the same answer.
pub const WINDOWS: [&str; 5] = ["7d", "30d", "90d", "365d", "all"];

/// The window a card shows until the reader picks another.
pub const DEFAULT_WINDOW: &str = "30d";

/// How a widget did over one window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WindowStats {
    /// Position within this window, 1 is most used.
    pub rank: u32,
    /// Distinct players who reported it at all.
    pub players: u32,
    /// Distinct players who had it *enabled* at least once.
    pub players_active: u32,
    /// `players_active / players`: install-and-keep rate. A widget people
    /// install and then switch off scores low here and nowhere else.
    pub retention: f64,
    pub sightings: u32,
    pub replays: u32,
    /// Days in this window that have actually been harvested.
    pub days_covered: u32,
    /// `days_covered` over the window length. Below 1.0 the window is a
    /// partial view — the pipeline backfills history a slice at a time, so a
    /// year window can legitimately hold a fortnight early on.
    pub coverage: f64,
}

impl WindowStats {
    /// Players who reported it but never had it enabled.
    pub fn players_disabled_only(&self) -> u32 {
        self.players.saturating_sub(self.players_active)
    }

    /// Whether this window has enough of its days to be worth showing as one.
    ///
    /// A 365-day window holding two weeks is not wrong, but presenting it as a
    /// year invites conclusions the data cannot carry.
    pub fn is_representative(&self) -> bool {
        self.coverage >= 0.5
    }
}

/// One widget, with its numbers per window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WidgetUsage {
    /// Stable key. Resolved widgets are `widget:<id>`; the rest get a key
    /// derived from the name and author their own client reported.
    pub key: String,
    /// Whether the reported hash was traced back to a distributable source.
    ///
    /// False does not mean unknown — the name, author and description below
    /// come from the widget's own `GetInfo()` either way. It means we cannot
    /// yet point at a file, so there is nothing to offer an install for.
    pub resolved: bool,
    /// Widget Hub id when resolved, empty otherwise.
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    #[serde(default)]
    pub windows: BTreeMap<String, WindowStats>,
}

impl WidgetUsage {
    pub fn window(&self, window: &str) -> Option<&WindowStats> {
        self.windows.get(window)
    }

    /// The best window to show for this widget, preferring `want`.
    ///
    /// A widget that is new, or rarely used, may have no row in a short window
    /// at all — the k-anonymity floor is applied inside each window, so a
    /// widget with six players this year and two this week is absent from the
    /// week. Falling back keeps it on the page instead of blanking the card.
    /// The returned name is always one of [`WINDOWS`], never the caller's
    /// string: an unknown window falls back rather than being echoed back as
    /// though it existed.
    pub fn window_or_fallback(&self, want: &str) -> Option<(&'static str, &WindowStats)> {
        let known = WINDOWS.iter().copied().find(|name| *name == want);
        if let Some(name) = known {
            if let Some(stats) = self.windows.get(name) {
                return Some((name, stats));
            }
        }
        WINDOWS
            .iter()
            .rev()
            .find_map(|name| self.windows.get(*name).map(|stats| (*name, stats)))
    }

    /// Whether this widget can be offered for install.
    pub fn is_installable(&self) -> bool {
        self.resolved && !self.id.is_empty()
    }
}

/// The published document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Usage {
    pub generated_at: String,
    pub policy_version: String,
    #[serde(default)]
    pub windows: Vec<String>,
    #[serde(default)]
    pub widgets: Vec<WidgetUsage>,
}

impl Usage {
    /// Widgets ranked for one window, most used first.
    ///
    /// Only widgets that actually have a row in that window: a missing row
    /// means the window withheld it, not that it scored zero.
    pub fn ranked(&self, window: &str) -> Vec<&WidgetUsage> {
        let mut ranked: Vec<&WidgetUsage> = self
            .widgets
            .iter()
            .filter(|widget| widget.windows.contains_key(window))
            .collect();
        ranked.sort_by_key(|widget| widget.windows[window].rank);
        ranked
    }

    pub fn find(&self, key: &str) -> Option<&WidgetUsage> {
        self.widgets.iter().find(|widget| widget.key == key)
    }

    /// Usage for a Widget Hub id, so a hub card can show what it is worth.
    pub fn by_widget_id(&self, id: &str) -> Option<&WidgetUsage> {
        if id.is_empty() {
            return None;
        }
        self.widgets.iter().find(|widget| widget.id == id)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("widget usage request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("widget usage returned HTTP {0}")]
    Status(u16),
    /// The service is turning requests away. It usually says when to come
    /// back, and that wait is the whole mechanism: ignoring it is what turns
    /// asking again into an attack.
    #[error("widget usage is busy")]
    Throttled { retry_after: Option<Duration> },
    #[error("widget usage document is {0} bytes, over the {BODY_LIMIT} byte limit")]
    TooLarge(usize),
    #[error("widget usage document could not be read: {0}")]
    Malformed(#[from] serde_json::Error),
}

impl Error {
    /// The wait this failure named, if it named one.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Error::Throttled { retry_after } => *retry_after,
            _ => None,
        }
    }
}

/// The wait a `Retry-After` header names, in the seconds form only, capped.
///
/// The same rule as [`pve::retry_after`], and a copy rather than a dependency:
/// it is four lines, and the two services are free to disagree about a cap
/// later without one of them having to be edited around the other.
pub fn retry_after(value: Option<&str>) -> Option<Duration> {
    let seconds: u64 = value?.trim().parse().ok()?;
    Some(Duration::from_secs(seconds).min(RETRY_AFTER_CAP))
}

/// What the cache file holds: the document, when it was last confirmed, and
/// the tag to confirm it with next time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
    #[serde(default)]
    etag: Option<String>,
    /// Seconds since the epoch of the last `200` or `304`.
    fetched_at: u64,
    usage: Usage,
}

impl Cached {
    fn fresh(&self, now: SystemTime) -> bool {
        seconds(now).saturating_sub(self.fetched_at) < FRESH_FOR.as_secs()
    }
}

/// What a load came back with: a document if there is one to show, and the
/// wait the service named if it named one.
///
/// A failure is not an error here, for the same reason the command that calls
/// this returns an option: usage decorates a widget list rather than carrying
/// it. The wait is passed up so a caller can hold off for exactly as long as
/// it was asked to rather than guessing.
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    pub usage: Option<Usage>,
    pub retry_after: Option<Duration>,
}

#[derive(Debug)]
enum Fetched {
    /// A `304`: what is cached is still what is published.
    Unchanged,
    New {
        etag: Option<String>,
        usage: Usage,
    },
}

/// The usage document: from the cache while it is fresh, from the server when
/// it is not, and from a stale cache when the server cannot be reached.
///
/// `url` is a parameter so a test can point this at a server of its own.
pub async fn load(
    client: &reqwest::Client,
    url: &str,
    cache_dir: &Path,
    now: SystemTime,
) -> Loaded {
    let path = cache_dir.join(CACHE_FILE);
    let cached = read(&path);
    if let Some(held) = &cached
        && held.fresh(now)
    {
        return Loaded {
            usage: Some(held.usage.clone()),
            retry_after: None,
        };
    }

    let etag = cached.as_ref().and_then(|held| held.etag.as_deref());
    match fetch(client, url, etag).await {
        Ok(Fetched::Unchanged) => {
            let Some(mut held) = cached else {
                // A 304 to a request that named no tag is the server's
                // mistake; there is nothing to show for it.
                tracing::warn!("widget usage: 304 with nothing cached");
                return Loaded::default();
            };
            tracing::debug!("widget usage: unchanged");
            held.fetched_at = seconds(now);
            write(&path, &held);
            Loaded {
                usage: Some(held.usage),
                retry_after: None,
            }
        }
        Ok(Fetched::New { etag, usage }) => {
            tracing::debug!(widgets = usage.widgets.len(), "widget usage: fetched");
            write(
                &path,
                &Cached {
                    etag,
                    fetched_at: seconds(now),
                    usage: usage.clone(),
                },
            );
            Loaded {
                usage: Some(usage),
                retry_after: None,
            }
        }
        Err(error) => {
            let retry_after = error.retry_after();
            tracing::warn!(%error, stale = cached.is_some(), "widget usage: not refreshed");
            Loaded {
                usage: cached.map(|held| held.usage),
                retry_after,
            }
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
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
    {
        let said = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok());
        return Err(Error::Throttled {
            retry_after: retry_after(said),
        });
    }
    if !status.is_success() {
        return Err(Error::Status(status.as_u16()));
    }
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    // The size is checked before parsing: this endpoint is a static object
    // behind a CDN, so anything large is a misroute rather than a big answer,
    // and there is no reason to hand it to a parser. The client decompresses
    // before this point, so the limit is on what would actually be parsed.
    let body = response.bytes().await?;
    if body.len() > BODY_LIMIT {
        return Err(Error::TooLarge(body.len()));
    }
    Ok(Fetched::New {
        etag,
        usage: serde_json::from_slice(&body)?,
    })
}

/// The cached document, or `None` when there is not a readable one.
///
/// Anything that will not read back — a file half written before a kill, one
/// left by a version whose shape differed — is treated as no cache at all.
/// That costs one fetch, which is the only sensible price for bytes nobody can
/// parse.
fn read(path: &Path) -> Option<Cached> {
    let compressed = std::fs::read(path).ok()?;
    let mut json = String::new();
    flate2::read::GzDecoder::new(compressed.as_slice())
        .read_to_string(&mut json)
        .inspect_err(
            |err| tracing::warn!(%err, path = %path.display(), "widget usage cache unreadable"),
        )
        .ok()?;
    serde_json::from_str(&json)
        .inspect_err(
            |err| tracing::warn!(%err, path = %path.display(), "widget usage cache not understood"),
        )
        .ok()
}

/// Temp file and rename, so a crash never leaves half a document behind. A
/// failure to write costs one fetch on the next run and is not worth more than
/// a log line.
fn write(path: &Path, cached: &Cached) {
    let written = (|| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&serde_json::to_vec(cached)?)?;
        let tmp: PathBuf = path.with_extension("gz.tmp");
        std::fs::write(&tmp, encoder.finish()?)?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(err) = written {
        tracing::warn!(%err, path = %path.display(), "widget usage cache not written");
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
    use wiremock::matchers::{header, header_regex, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A moment, for the freshness window. Which moment never matters; what
    /// matters is the distance between two of them.
    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    const NOW: u64 = 1_800_000_000;

    fn client() -> reqwest::Client {
        content::http::client("test")
    }

    /// A cache as a previous run would have left it.
    fn seed(dir: &Path, etag: Option<&str>, fetched_at: u64, usage: Usage) {
        write(
            &dir.join(CACHE_FILE),
            &Cached {
                etag: etag.map(str::to_owned),
                fetched_at,
                usage,
            },
        );
    }

    fn stats(rank: u32, players: u32, active: u32, coverage: f64) -> serde_json::Value {
        serde_json::json!({
            "rank": rank,
            "players": players,
            "players_active": active,
            "retention": if players == 0 { 0.0 } else { active as f64 / players as f64 },
            "sightings": players * 3,
            "replays": players * 2,
            "days_covered": (coverage * 30.0) as u32,
            "coverage": coverage,
        })
    }

    fn document(widgets: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "generated_at": "2026-09-16T04:00:00+00:00",
            "policy_version": "pve_widget_harvest_v1_prefix_262144",
            "windows": ["7d", "30d", "90d", "365d", "all"],
            "widgets": widgets,
        })
    }

    fn ping_wheel() -> serde_json::Value {
        serde_json::json!({
            "key": "widget:gui_ping_wheel",
            "resolved": true,
            "id": "gui_ping_wheel",
            "name": "Ping Wheel",
            "author": "Errrrrrr",
            "description": "A radial ping menu",
            "windows": { "30d": stats(1, 120, 100, 1.0), "all": stats(1, 400, 320, 1.0) },
        })
    }

    fn unresolved() -> serde_json::Value {
        serde_json::json!({
            "key": "unresolved:abc123",
            "resolved": false,
            "id": "",
            "name": "Flea Transport",
            "author": "[teh]Teddy",
            "description": "",
            "windows": { "all": stats(2, 40, 30, 1.0) },
        })
    }

    fn parse(value: serde_json::Value) -> Usage {
        serde_json::from_value(value).expect("a usage document")
    }

    #[test]
    fn a_published_document_parses() {
        let usage = parse(document(serde_json::json!([ping_wheel()])));
        assert_eq!(usage.widgets.len(), 1);
        assert_eq!(usage.widgets[0].name, "Ping Wheel");
        assert_eq!(usage.widgets[0].window("30d").unwrap().players, 120);
    }

    #[test]
    fn ranking_follows_the_window_not_the_document_order() {
        let mut second = ping_wheel();
        second["key"] = "widget:gui_other".into();
        second["id"] = "gui_other".into();
        second["name"] = "Other".into();
        second["windows"]["30d"] = stats(2, 90, 80, 1.0);
        let usage = parse(document(serde_json::json!([second, ping_wheel()])));
        let ranked = usage.ranked("30d");
        assert_eq!(ranked[0].name, "Ping Wheel");
        assert_eq!(ranked[1].name, "Other");
    }

    #[test]
    fn a_widget_absent_from_a_window_is_not_ranked_as_zero() {
        // The k-anonymity floor is applied inside each window, so a missing row
        // means "withheld here", not "used by nobody".
        let usage = parse(document(serde_json::json!([unresolved()])));
        assert!(usage.ranked("30d").is_empty());
        assert_eq!(usage.ranked("all").len(), 1);
    }

    #[test]
    fn a_widget_missing_the_wanted_window_falls_back_rather_than_blanking() {
        let usage = parse(document(serde_json::json!([unresolved()])));
        let (window, stats) = usage.widgets[0]
            .window_or_fallback("7d")
            .expect("a fallback");
        assert_eq!(window, "all");
        assert_eq!(stats.players, 40);
    }

    #[test]
    fn an_unknown_window_name_is_not_echoed_back() {
        let usage = parse(document(serde_json::json!([ping_wheel()])));
        let (window, _) = usage.widgets[0]
            .window_or_fallback("since_tuesday")
            .expect("a fallback");
        assert!(WINDOWS.contains(&window));
    }

    #[test]
    fn retention_separates_kept_from_switched_off() {
        let usage = parse(document(serde_json::json!([ping_wheel()])));
        let stats = usage.widgets[0].window("30d").unwrap();
        assert_eq!(stats.players_disabled_only(), 20);
        assert!((stats.retention - 100.0 / 120.0).abs() < 1e-9);
    }

    #[test]
    fn a_partly_harvested_window_is_not_representative() {
        // A year window holding a fortnight is honest data, but presenting it
        // as a year would invite conclusions it cannot carry.
        let mut widget = ping_wheel();
        widget["windows"]["365d"] = stats(1, 400, 300, 0.04);
        let usage = parse(document(serde_json::json!([widget])));
        assert!(!usage.widgets[0].window("365d").unwrap().is_representative());
        assert!(usage.widgets[0].window("30d").unwrap().is_representative());
    }

    #[test]
    fn only_a_resolved_widget_can_be_offered_for_install() {
        let usage = parse(document(serde_json::json!([ping_wheel(), unresolved()])));
        assert!(
            usage
                .find("widget:gui_ping_wheel")
                .unwrap()
                .is_installable()
        );
        assert!(!usage.find("unresolved:abc123").unwrap().is_installable());
    }

    #[test]
    fn an_unresolved_widget_still_carries_the_name_its_own_client_reported() {
        let usage = parse(document(serde_json::json!([unresolved()])));
        let widget = &usage.widgets[0];
        assert!(!widget.resolved);
        assert_eq!(widget.name, "Flea Transport");
        assert_eq!(widget.author, "[teh]Teddy");
    }

    #[test]
    fn a_hub_id_finds_its_usage() {
        let usage = parse(document(serde_json::json!([ping_wheel(), unresolved()])));
        assert_eq!(
            usage.by_widget_id("gui_ping_wheel").unwrap().name,
            "Ping Wheel"
        );
        assert!(usage.by_widget_id("").is_none());
    }

    #[tokio::test]
    async fn the_published_document_is_fetched_and_kept() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/widget_registry/usage.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("ETag", "\"week-38\"")
                    .set_body_json(document(serde_json::json!([ping_wheel()]))),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("a cache directory");

        let loaded = load(
            &client(),
            &format!("{}/widget_registry/usage.json", server.uri()),
            dir.path(),
            at(NOW),
        )
        .await;

        assert_eq!(loaded.usage.expect("a document").widgets.len(), 1);
        // And it is on disk, gzipped, with the tag to confirm it by next time.
        let held = read(&dir.path().join(CACHE_FILE)).expect("a cache");
        assert_eq!(held.etag.as_deref(), Some("\"week-38\""));
        assert_eq!(held.fetched_at, NOW);
        assert_eq!(held.usage.widgets[0].name, "Ping Wheel");
    }

    #[tokio::test]
    async fn a_fresh_cache_is_not_a_request_at_all() {
        // The point of the whole arrangement: opening the page twice in an
        // evening asks the service nothing.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("a cache directory");
        seed(
            dir.path(),
            Some("\"week-38\""),
            NOW - 60,
            parse(document(serde_json::json!([ping_wheel()]))),
        );

        let loaded = load(
            &client(),
            &format!("{}/usage.json", server.uri()),
            dir.path(),
            at(NOW),
        )
        .await;

        assert_eq!(loaded.usage.expect("the cached document").widgets.len(), 1);
    }

    #[tokio::test]
    async fn a_stale_cache_is_confirmed_rather_than_downloaded_again() {
        // A weekly document asked for daily is a 304 six times out of seven,
        // and a 304 has no body to pay for.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("If-None-Match", "\"week-38\""))
            .respond_with(ResponseTemplate::new(304))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("a cache directory");
        let stale = NOW - FRESH_FOR.as_secs() - 1;
        seed(
            dir.path(),
            Some("\"week-38\""),
            stale,
            parse(document(serde_json::json!([ping_wheel()]))),
        );

        let loaded = load(
            &client(),
            &format!("{}/usage.json", server.uri()),
            dir.path(),
            at(NOW),
        )
        .await;

        assert_eq!(
            loaded.usage.expect("the confirmed document").widgets.len(),
            1
        );
        // Confirmed now, so the next day is quiet too.
        let held = read(&dir.path().join(CACHE_FILE)).expect("a cache");
        assert_eq!(held.fetched_at, NOW);
        assert_eq!(held.etag.as_deref(), Some("\"week-38\""));
    }

    #[tokio::test]
    async fn a_document_that_changed_replaces_the_cached_one() {
        let server = MockServer::start().await;
        let mut moved = ping_wheel();
        moved["name"] = "Ping Wheel Mk II".into();
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("ETag", "\"week-39\"")
                    .set_body_json(document(serde_json::json!([moved]))),
            )
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("a cache directory");
        seed(
            dir.path(),
            Some("\"week-38\""),
            NOW - FRESH_FOR.as_secs() - 1,
            parse(document(serde_json::json!([ping_wheel()]))),
        );

        let loaded = load(
            &client(),
            &format!("{}/usage.json", server.uri()),
            dir.path(),
            at(NOW),
        )
        .await;

        assert_eq!(
            loaded.usage.expect("a document").widgets[0].name,
            "Ping Wheel Mk II"
        );
        let held = read(&dir.path().join(CACHE_FILE)).expect("a cache");
        assert_eq!(held.etag.as_deref(), Some("\"week-39\""));
        assert_eq!(held.usage.widgets[0].name, "Ping Wheel Mk II");
    }

    #[tokio::test]
    async fn a_service_that_is_down_is_answered_from_the_stale_cache() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("a cache directory");
        seed(
            dir.path(),
            None,
            NOW - FRESH_FOR.as_secs() - 1,
            parse(document(serde_json::json!([ping_wheel()]))),
        );

        let loaded = load(
            &client(),
            &format!("{}/usage.json", server.uri()),
            dir.path(),
            at(NOW),
        )
        .await;

        // Last week's numbers are worth more than an empty page.
        assert_eq!(loaded.usage.expect("the stale document").widgets.len(), 1);
        assert!(loaded.retry_after.is_none());
    }

    #[tokio::test]
    async fn a_busy_service_hands_back_the_wait_it_named() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "7"))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("a cache directory");

        let loaded = load(
            &client(),
            &format!("{}/usage.json", server.uri()),
            dir.path(),
            at(NOW),
        )
        .await;

        assert!(loaded.usage.is_none());
        assert_eq!(loaded.retry_after, Some(Duration::from_secs(7)));
    }

    #[test]
    fn a_wait_is_taken_at_its_word_only_so_far() {
        assert_eq!(retry_after(Some("7")), Some(Duration::from_secs(7)));
        assert_eq!(retry_after(Some(" 12 ")), Some(Duration::from_secs(12)));
        assert_eq!(retry_after(Some("99999")), Some(RETRY_AFTER_CAP));
        // The HTTP-date form is legal and not read here; an unreadable wait is
        // no wait rather than a guess.
        assert_eq!(retry_after(Some("Wed, 21 Oct 2026 07:28:00 GMT")), None);
        assert_eq!(retry_after(None), None);
    }

    #[tokio::test]
    async fn the_document_is_asked_for_compressed() {
        // 40 KiB of JSON over a connection somebody is playing a game on. The
        // client's `gzip`/`brotli` features are what make this header appear,
        // so this test is really about nobody dropping them from the manifest.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header_regex("accept-encoding", "gzip"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(document(serde_json::json!([ping_wheel()]))),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("a cache directory");

        let loaded = load(
            &client(),
            &format!("{}/usage.json", server.uri()),
            dir.path(),
            at(NOW),
        )
        .await;

        assert!(loaded.usage.is_some());
    }

    #[tokio::test]
    async fn a_cache_file_nobody_can_read_is_no_cache_rather_than_a_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(document(serde_json::json!([ping_wheel()]))),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("a cache directory");
        std::fs::write(dir.path().join(CACHE_FILE), b"not gzip, not json").expect("a bad cache");

        let loaded = load(
            &client(),
            &format!("{}/usage.json", server.uri()),
            dir.path(),
            at(NOW),
        )
        .await;

        assert_eq!(loaded.usage.expect("a fetched document").widgets.len(), 1);
    }

    #[tokio::test]
    async fn something_that_is_not_a_usage_document_is_not_written_to_the_cache() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("a cache directory");

        let loaded = load(
            &client(),
            &format!("{}/usage.json", server.uri()),
            dir.path(),
            at(NOW),
        )
        .await;

        assert!(loaded.usage.is_none());
        assert!(!dir.path().join(CACHE_FILE).exists());
    }

    #[tokio::test]
    async fn a_failing_status_is_an_error_rather_than_an_empty_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let failed = fetch(&client(), &format!("{}/usage.json", server.uri()), None)
            .await
            .expect_err("a 503 is not a document");
        assert!(matches!(failed, Error::Throttled { retry_after: None }));
    }
}
