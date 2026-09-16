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
//! The HTTP client is handed in, not built here, for the same reason as
//! [`pve`]: modlobby has one client for everything it asks BAR, so every
//! request carries the same name and shares one connection pool.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where the weekly pipeline publishes the usage document.
pub const ENDPOINT: &str = "https://d29i3oohxql6zz.cloudfront.net/widget_registry/usage.json";

/// Generous next to the ~40 KiB the document compresses to, and small enough
/// that a misrouted response cannot be read into memory unbounded.
pub const BODY_LIMIT: usize = 4 * 1024 * 1024;

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
    #[error("widget usage document is {0} bytes, over the {BODY_LIMIT} byte limit")]
    TooLarge(usize),
    #[error("widget usage document could not be read: {0}")]
    Malformed(#[from] serde_json::Error),
}

/// Reads the published usage document.
pub struct Service {
    client: reqwest::Client,
    endpoint: String,
}

impl Service {
    pub fn new(client: reqwest::Client, endpoint: impl Into<String>) -> Self {
        Self { client, endpoint: endpoint.into() }
    }

    pub fn published(client: reqwest::Client) -> Self {
        Self::new(client, ENDPOINT)
    }

    /// Fetch the document.
    ///
    /// The size is checked before parsing: this endpoint is a static object
    /// behind a CDN, so anything large is a misroute rather than a big answer,
    /// and there is no reason to hand it to a parser.
    pub async fn fetch(&self) -> Result<Usage, Error> {
        let response = self.client.get(&self.endpoint).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status(status.as_u16()));
        }
        let body = response.bytes().await?;
        if body.len() > BODY_LIMIT {
            return Err(Error::TooLarge(body.len()));
        }
        Ok(serde_json::from_slice(&body)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
        let (window, stats) = usage.widgets[0].window_or_fallback("7d").expect("a fallback");
        assert_eq!(window, "all");
        assert_eq!(stats.players, 40);
    }

    #[test]
    fn an_unknown_window_name_is_not_echoed_back() {
        let usage = parse(document(serde_json::json!([ping_wheel()])));
        let (window, _) = usage.widgets[0].window_or_fallback("since_tuesday").expect("a fallback");
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
        assert!(usage.find("widget:gui_ping_wheel").unwrap().is_installable());
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
        assert_eq!(usage.by_widget_id("gui_ping_wheel").unwrap().name, "Ping Wheel");
        assert!(usage.by_widget_id("").is_none());
    }

    #[tokio::test]
    async fn the_published_document_is_fetched() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/widget_registry/usage.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(document(serde_json::json!([ping_wheel()]))))
            .mount(&server)
            .await;
        let service = Service::new(
            content::http::client("test"),
            format!("{}/widget_registry/usage.json", server.uri()),
        );
        assert_eq!(service.fetch().await.expect("a document").widgets.len(), 1);
    }

    #[tokio::test]
    async fn a_failing_status_is_an_error_not_an_empty_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let service = Service::new(content::http::client("test"), format!("{}/usage.json", server.uri()));
        assert!(matches!(service.fetch().await, Err(Error::Status(503))));
    }

    #[tokio::test]
    async fn something_that_is_not_a_usage_document_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
            .mount(&server)
            .await;
        let service = Service::new(content::http::client("test"), format!("{}/usage.json", server.uri()));
        assert!(matches!(service.fetch().await, Err(Error::Malformed(_))));
    }
}
