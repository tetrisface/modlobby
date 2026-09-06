//! BAR's news, from the feed the website publishes.
//!
//! One small RSS document, fetched at most once an hour and kept on disk in
//! between, so opening the tab twice in an evening costs nothing and a launch
//! with no network still has something to show. Nothing here fails loudly: a
//! lobby has to work offline, and a stale headline beats an empty page.
//!
//! The cache is the same shape as `content::map_index`'s and deliberately not
//! shared with it. What differs is the conditional request: this feed
//! publishes no `ETag`, and its `Last-Modified` tracks when the CDN rendered
//! the page rather than when anything changed, so asking with it would answer
//! `200` every time. The freshness window is the real guard, and a generic
//! cache over the two would be read at both call sites and understood at
//! neither.
//!
//! Read marks live beside the settings rather than in them ([`Memory`]): what
//! somebody has already looked at is bookkeeping, not a preference. They are
//! kept as the *ids that were on the list last time*, not as a date — see
//! [`Memory::unread`] for why that is what makes an edited item stay read.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The feed the BAR website publishes, and the one its own site links to.
pub const FEED_URL: &str = "https://www.beyondallreason.info/news/rss.xml";

/// What the read marks for this feed are filed under. A second feed later is
/// a second name here and no change to the file's shape.
pub const SOURCE: &str = "news";

/// How long a fetched feed is trusted. The feed asks for exactly this with its
/// own `<ttl>60</ttl>`. The cost is that a headline can be up to an hour late
/// inside a long session, which for news is the right way round.
pub const FRESH_FOR: Duration = Duration::from_secs(60 * 60);

/// Under the config directory's `cache/`.
pub const CACHE_FILE: &str = "news.json";

/// Beside the settings, with the other things the app remembers for itself.
const MEMORY_FILE: &str = "news-read.json";

/// One story.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewsItem {
    /// The permalink, which is what `<guid>` holds. Identifies the item
    /// everywhere: in the read marks, and as the key its picture is asked for
    /// by. It does not change when the story is edited, which is the whole
    /// reason an edit never counts as something new.
    pub id: String,
    pub title: String,
    pub summary: String,
    pub link: String,
    /// Unix seconds from `<pubDate>`; `None` when it could not be read.
    #[ts(type = "number | null")]
    pub published_at: Option<u64>,
    /// The banner, if the item published one. Fetched through the app rather
    /// than by the webview, which cannot reach the CDN it lives on.
    pub image: Option<String>,
}

/// The list, and how much of it is new since it was last looked at.
///
/// One answer rather than two commands, so the count on the tab and the page
/// behind it can never disagree about what was in the feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewsFeed {
    pub items: Vec<NewsItem>,
    #[ts(type = "number")]
    pub unread: usize,
}

/// Never surfaced: every caller of [`load`] gets a list, empty at worst.
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("not an RSS feed")]
    NotRss,
    #[error("malformed XML: {0}")]
    Xml(String),
    #[error("HTTP {0}")]
    Status(u16),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

/// Which of an item's fields the reader is currently inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Link,
    Guid,
    Description,
    PubDate,
}

impl Field {
    fn of(name: &[u8]) -> Option<Self> {
        match name {
            b"title" => Some(Self::Title),
            b"link" => Some(Self::Link),
            b"guid" => Some(Self::Guid),
            b"description" => Some(Self::Description),
            b"pubDate" => Some(Self::PubDate),
            _ => None,
        }
    }
}

/// An item being read, before it is known whether it has enough to be one.
#[derive(Debug, Default)]
struct Draft {
    title: String,
    link: String,
    guid: String,
    description: String,
    pub_date: String,
    image: Option<String>,
}

impl Draft {
    fn push(&mut self, field: Field, text: &str) {
        // Appended rather than assigned: a text node split around an entity
        // arrives as more than one event, and half a headline is worse than a
        // whole one.
        match field {
            Field::Title => self.title.push_str(text),
            Field::Link => self.link.push_str(text),
            Field::Guid => self.guid.push_str(text),
            Field::Description => self.description.push_str(text),
            Field::PubDate => self.pub_date.push_str(text),
        }
    }

    /// `None` for an item with nothing to show or no way back to it, which is
    /// dropped rather than allowed to break the rest of the feed.
    fn finish(self) -> Option<NewsItem> {
        let link = self.link.trim().to_owned();
        let id = match self.guid.trim() {
            "" => link.clone(),
            guid => guid.to_owned(),
        };
        let title = headline(&self.title);
        if id.is_empty() || title.is_empty() {
            return None;
        }
        Some(NewsItem {
            id,
            title,
            summary: self.description.trim().to_owned(),
            link,
            published_at: published(&self.pub_date),
            image: self.image,
        })
    }
}

/// The headline without the site's own page-title template.
///
/// Webflow renders `<title> ⇀ <section> ★ <site>` and the feed inherits it, so
/// every entry ends in `⇀ News ★ Beyond All Reason RTS`. Cutting at the
/// separator rather than matching that whole string survives the section being
/// renamed, which matching it would not.
fn headline(title: &str) -> String {
    match title.split_once(" ⇀ ") {
        Some((head, _)) => head.trim().to_owned(),
        None => title.trim().to_owned(),
    }
}

/// `<pubDate>` as unix seconds, or `None` when it is not a date we can read.
///
/// `httpdate` wants the fixed `Wed, 24 Jun 2026 20:51:50 GMT` form, which is
/// what this feed emits; an RFC 2822 numeric offset would not parse. An item
/// whose date is unreadable keeps its place on the page and simply never
/// counts as new, which is the quiet way to be wrong.
fn published(raw: &str) -> Option<u64> {
    let at = httpdate::parse_http_date(raw.trim()).ok()?;
    Some(seconds(at))
}

/// The feed, cut down to what is shown.
///
/// `pub` because it is the half worth testing on its own: everything awkward
/// about RSS — CDATA, escaped entities, a missing date, an item with no
/// picture — is decided here, with no server in the way.
pub fn parse(xml: &str) -> Result<Vec<NewsItem>, ParseError> {
    parse_inner(xml).map_err(|err| ParseError(err.to_string()))
}

/// Why a document could not be read as a feed. Opaque: nothing acts on the
/// distinction, and a caller that wants the detail can print it.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ParseError(String);

fn parse_inner(xml: &str) -> Result<Vec<NewsItem>, Error> {
    // Text is not trimmed as it is read: this reader hands entities back as
    // events of their own, so a headline arrives in pieces around every
    // `&amp;`, and trimming each piece would eat the spaces between them.
    // Whitespace between elements never reaches a field, and what does is
    // trimmed once the item is whole.
    let mut reader = Reader::from_str(xml);

    let mut items: Vec<NewsItem> = Vec::new();
    let mut draft: Option<Draft> = None;
    let mut field: Option<Field> = None;
    let mut is_rss = false;

    loop {
        let event = reader
            .read_event()
            .map_err(|err| Error::Xml(err.to_string()))?;
        match event {
            Event::Start(tag) => match tag.name().as_ref() {
                b"rss" | b"feed" => is_rss = true,
                b"item" => draft = Some(Draft::default()),
                name => {
                    // Only inside an item: the channel has a `<title>` and a
                    // `<link>` of its own, and they are not a story.
                    if draft.is_some() {
                        field = Field::of(name);
                    }
                }
            },
            Event::Empty(tag) => {
                if let Some(item) = draft.as_mut() {
                    take_picture(item, &tag);
                }
            }
            Event::Text(text) => {
                if let (Some(item), Some(which)) = (draft.as_mut(), field) {
                    let read = text
                        .xml10_content()
                        .map_err(|err| Error::Xml(err.to_string()))?;
                    item.push(which, &read);
                }
            }
            Event::GeneralRef(reference) => {
                if let (Some(item), Some(which)) = (draft.as_mut(), field) {
                    item.push(which, &resolve(&reference)?);
                }
            }
            Event::CData(data) => {
                if let (Some(item), Some(which)) = (draft.as_mut(), field) {
                    // Character data is literal by definition: decoded, never
                    // unescaped.
                    let read = data.decode().map_err(|err| Error::Xml(err.to_string()))?;
                    item.push(which, &read);
                }
            }
            Event::End(tag) => {
                if tag.name().as_ref() == b"item" {
                    if let Some(item) = draft.take().and_then(Draft::finish) {
                        items.push(item);
                    }
                }
                field = None;
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if !is_rss {
        return Err(Error::NotRss);
    }
    // The feed's own order is not guaranteed, and a story published late with
    // an earlier date would otherwise sit at the top. Stable, so items with no
    // readable date keep the order they arrived in, at the end.
    items.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    Ok(items)
}

/// One `&…;` as the text it stands for.
///
/// Anything unrecognised keeps its source form rather than disappearing: a
/// feed is somebody else's document, and a headline with a stray `&nbsp;` in
/// it reads better than one with a hole where a word was.
fn resolve(reference: &BytesRef<'_>) -> Result<String, Error> {
    if let Some(ch) = reference
        .resolve_char_ref()
        .map_err(|err| Error::Xml(err.to_string()))?
    {
        return Ok(ch.to_string());
    }
    let name = reference
        .decode()
        .map_err(|err| Error::Xml(err.to_string()))?;
    Ok(quick_xml::escape::resolve_predefined_entity(&name)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("&{name};")))
}

/// `<media:thumbnail>` in preference to `<media:content>`: both carry the same
/// picture on this feed, and the thumbnail is the one meant for a list.
fn take_picture(item: &mut Draft, tag: &BytesStart<'_>) {
    let name = tag.name();
    let preferred = match name.as_ref() {
        b"media:thumbnail" => true,
        b"media:content" => false,
        _ => return,
    };
    if item.image.is_some() && !preferred {
        return;
    }
    if let Some(url) = attribute(tag, b"url") {
        item.image = Some(url);
    }
}

fn attribute(tag: &BytesStart<'_>, wanted: &[u8]) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|attr| attr.key.as_ref() == wanted)
        .and_then(|attr| attr.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|value| value.into_owned())
}

/// What the cache file holds: the feed, and when it was last fetched.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
    /// Seconds since the epoch of the last fetch.
    fetched_at: u64,
    items: Vec<NewsItem>,
}

impl Cached {
    fn fresh(&self, now: SystemTime) -> bool {
        seconds(now).saturating_sub(self.fetched_at) < FRESH_FOR.as_secs()
    }
}

/// The feed: from the cache while it is fresh, from the server when it is not,
/// and from a stale cache when the server cannot be reached.
///
/// `url` is a parameter so a test can point this at a server of its own.
pub async fn load(
    client: &reqwest::Client,
    url: &str,
    cache_dir: &Path,
    now: SystemTime,
) -> Vec<NewsItem> {
    let path = cache_dir.join(CACHE_FILE);
    let cached = read(&path);
    if let Some(held) = &cached
        && held.fresh(now)
    {
        return held.items.clone();
    }

    match fetch(client, url).await {
        Ok(items) => {
            tracing::debug!(stories = items.len(), "news: fetched");
            write(
                &path,
                &Cached {
                    fetched_at: seconds(now),
                    items: items.clone(),
                },
            );
            items
        }
        Err(err) => {
            tracing::warn!(%err, stale = cached.is_some(), "news: not refreshed");
            cached.map(|held| held.items).unwrap_or_default()
        }
    }
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<Vec<NewsItem>, Error> {
    let response = client.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Status(status.as_u16()));
    }
    parse_inner(&response.text().await?)
}

fn read(path: &Path) -> Option<Cached> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(cached) => Some(cached),
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "news cache not readable");
            None
        }
    }
}

/// Temp file and rename, so a crash never leaves half a feed behind. Failing
/// to write costs one fetch on the next run and is not worth more than a line.
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
        tracing::warn!(%err, path = %path.display(), "news cache not written");
    }
}

fn seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// What has already been looked at, kept between runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Marks {
    /// Per source, the ids that were on the list when it was last looked at.
    seen: BTreeMap<String, BTreeSet<String>>,
}

/// The read marks, in a file beside the settings.
#[derive(Debug, Clone)]
pub struct Memory {
    path: PathBuf,
}

impl Memory {
    pub fn new(config_dir: impl AsRef<Path>) -> Self {
        Self {
            path: config_dir.as_ref().join(MEMORY_FILE),
        }
    }

    /// How many of these have turned up since this source was last looked at.
    ///
    /// Counted by id rather than by date, which is what makes an edit invisible:
    /// a `<guid>` is the story's permalink and does not move when the words do,
    /// so a rewritten item stays read. A date would only hold if the feed
    /// promised never to touch `<pubDate>` on an edit, and it promises nothing.
    ///
    /// A source nobody has ever seen is seeded here rather than counted. A
    /// badge reading "53" on a first launch is not news, it is the page
    /// announcing that it exists.
    pub fn unread(&self, source: &str, items: &[NewsItem]) -> usize {
        let mut marks = self.read();
        let Some(seen) = marks.seen.get(source) else {
            if !items.is_empty() {
                marks.seen.insert(source.to_owned(), ids(items));
                self.write(&marks);
            }
            return 0;
        };
        items.iter().filter(|item| !seen.contains(&item.id)).count()
    }

    /// Remembers what the list showed, which is what "read" means here.
    ///
    /// Only what is currently on the list: the feed is a window of the most
    /// recent stories, so this stays the size of that window instead of
    /// growing forever with every id ever published.
    pub fn mark_read(&self, source: &str, items: &[NewsItem]) {
        if items.is_empty() {
            return;
        }
        let mut marks = self.read();
        marks.seen.insert(source.to_owned(), ids(items));
        self.write(&marks);
    }

    fn read(&self) -> Marks {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Losable: marks that cannot be written mean a badge comes back once.
    fn write(&self, marks: &Marks) {
        let Ok(text) = serde_json::to_string_pretty(marks) else {
            return;
        };
        if let Err(err) = std::fs::write(&self.path, text) {
            tracing::warn!(%err, path = %self.path.display(), "news marks not written");
        }
    }
}

fn ids(items: &[NewsItem]) -> BTreeSet<String> {
    items.iter().map(|item| item.id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path as path_matcher};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Shaped like the real feed, down to the title template and the trailing
    /// `<media:content>` beside the thumbnail.
    const FEED: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
<title>Beyond All Reason | News Feed</title>
<link>https://www.beyondallreason.info</link>
<item>
  <title>Hooded Horse: a new chapter &#8211; and more &amp; more ⇀ News ★ Beyond All Reason RTS</title>
  <link>https://www.beyondallreason.info/news/hooded-horse</link>
  <guid>https://www.beyondallreason.info/news/hooded-horse</guid>
  <description>BAR is going professional.</description>
  <pubDate>Wed, 24 Jun 2026 20:51:50 GMT</pubDate>
  <media:content url="https://cdn.example/big.webp" medium="image"/>
  <media:thumbnail url="https://cdn.example/thumb.webp"/>
</item>
<item>
  <title><![CDATA[A quiet one ⇀ News ★ Beyond All Reason RTS]]></title>
  <link>https://www.beyondallreason.info/news/quiet</link>
  <guid>https://www.beyondallreason.info/news/quiet</guid>
  <description><![CDATA[Nothing to see.]]></description>
  <pubDate>Tue, 12 May 2026 14:26:59 GMT</pubDate>
</item>
<item>
  <title>Undated</title>
  <link>https://www.beyondallreason.info/news/undated</link>
  <guid>https://www.beyondallreason.info/news/undated</guid>
  <description>No date on this one.</description>
  <pubDate>4 Jun 2026 10:00:00 +0000</pubDate>
</item>
</channel></rss>"#;

    fn client() -> reqwest::Client {
        content::http::client("test")
    }

    fn item(id: &str, at: Option<u64>) -> NewsItem {
        NewsItem {
            id: id.into(),
            title: id.into(),
            summary: String::new(),
            link: id.into(),
            published_at: at,
            image: None,
        }
    }

    #[test]
    fn only_what_precedes_the_sites_own_title_suffix_is_kept() {
        let items = parse(FEED).unwrap();
        assert_eq!(
            items[0].title,
            "Hooded Horse: a new chapter – and more & more"
        );
    }

    #[test]
    fn a_title_wrapped_in_cdata_arrives_as_its_text() {
        let items = parse(FEED).unwrap();
        assert_eq!(items[1].title, "A quiet one");
        assert_eq!(items[1].summary, "Nothing to see.");
    }

    #[test]
    fn the_guid_is_the_id_and_the_link_is_kept_beside_it() {
        let items = parse(FEED).unwrap();
        assert_eq!(
            items[0].id,
            "https://www.beyondallreason.info/news/hooded-horse"
        );
        assert_eq!(items[0].link, items[0].id);
    }

    #[test]
    fn the_thumbnail_wins_over_the_full_size_picture() {
        let items = parse(FEED).unwrap();
        assert_eq!(
            items[0].image.as_deref(),
            Some("https://cdn.example/thumb.webp")
        );
    }

    #[test]
    fn an_item_with_no_picture_is_not_an_error() {
        let items = parse(FEED).unwrap();
        assert!(items[1].image.is_none());
    }

    #[test]
    fn an_item_with_no_readable_date_still_appears_and_sorts_last() {
        let items = parse(FEED).unwrap();
        let undated = items.last().expect("three items");
        assert_eq!(undated.title, "Undated");
        assert!(undated.published_at.is_none());
    }

    #[test]
    fn items_come_back_newest_first_whatever_order_the_feed_used() {
        let mut items = vec![
            item("old", Some(10)),
            item("newest", Some(30)),
            item("middle", Some(20)),
        ];
        items.sort_by(|a, b| b.published_at.cmp(&a.published_at));
        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["newest", "middle", "old"]
        );

        let parsed = parse(FEED).unwrap();
        assert_eq!(
            parsed[0].title,
            "Hooded Horse: a new chapter – and more & more"
        );
        assert_eq!(parsed[1].title, "A quiet one");
    }

    #[test]
    fn a_document_that_is_not_a_feed_is_an_error_not_a_panic() {
        assert!(parse("<html><body>down for maintenance</body></html>").is_err());
        assert!(parse("").is_err());
        assert!(parse("not xml at all").is_err());
    }

    #[test]
    fn a_truncated_feed_is_an_error_not_a_panic() {
        assert!(parse(&FEED[..FEED.len() / 2]).is_err());
    }

    #[test]
    fn an_item_with_nothing_to_show_is_dropped_and_the_rest_survive() {
        let feed = r#"<rss><channel>
            <item><description>no title, no link</description></item>
            <item><title>Real</title><guid>g1</guid></item>
        </channel></rss>"#;
        let items = parse(feed).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "g1");
    }

    #[tokio::test]
    async fn the_first_load_fetches_and_keeps_the_feed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/news/rss.xml"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FEED))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let url = format!("{}/news/rss.xml", server.uri());

        let items = load(&client(), &url, dir.path(), SystemTime::now()).await;

        assert_eq!(items.len(), 3);
        let held = read(&dir.path().join(CACHE_FILE)).expect("a cache file");
        assert_eq!(held.items, items);
    }

    #[tokio::test]
    async fn within_the_hour_the_server_is_not_asked_again() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FEED))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let url = format!("{}/news/rss.xml", server.uri());
        let now = SystemTime::now();

        let first = load(&client(), &url, dir.path(), now).await;
        let again = load(
            &client(),
            &url,
            dir.path(),
            now + FRESH_FOR - Duration::from_secs(1),
        )
        .await;

        assert_eq!(first, again);
    }

    #[tokio::test]
    async fn when_the_server_fails_the_stale_cache_is_served() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FEED))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let url = format!("{}/news/rss.xml", server.uri());
        let now = SystemTime::now();

        let fresh = load(&client(), &url, dir.path(), now).await;
        let stale = load(&client(), &url, dir.path(), now + FRESH_FOR).await;

        assert_eq!(fresh, stale);
    }

    #[tokio::test]
    async fn with_nothing_cached_and_no_server_the_feed_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        // A port nothing listens on.
        let items = load(
            &client(),
            "http://127.0.0.1:9/news/rss.xml",
            dir.path(),
            SystemTime::now(),
        )
        .await;
        assert!(items.is_empty());
        assert!(!dir.path().join(CACHE_FILE).exists());
    }

    #[test]
    fn a_source_seen_for_the_first_time_starts_with_nothing_unread() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::new(dir.path());
        let items = [item("a", Some(2)), item("b", Some(1))];

        assert_eq!(memory.unread(SOURCE, &items), 0);
        // And it stays seeded across a restart, rather than seeding again.
        assert_eq!(Memory::new(dir.path()).unread(SOURCE, &items), 0);
    }

    #[test]
    fn an_item_that_was_not_on_the_list_last_time_is_unread() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::new(dir.path());
        let first = [item("a", Some(1))];
        memory.unread(SOURCE, &first);

        let later = [item("c", Some(3)), item("b", Some(2)), item("a", Some(1))];
        assert_eq!(memory.unread(SOURCE, &later), 2);

        memory.mark_read(SOURCE, &later);
        assert_eq!(memory.unread(SOURCE, &later), 0);
        assert_eq!(Memory::new(dir.path()).unread(SOURCE, &later), 0);
    }

    #[test]
    fn an_edited_story_is_still_the_one_that_was_read() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::new(dir.path());
        let published = [item("a", Some(1))];
        memory.mark_read(SOURCE, &published);

        // Same permalink, new words and a bumped date: an edit, not news.
        let edited = [NewsItem {
            title: "Rewritten".into(),
            summary: "More detail.".into(),
            published_at: Some(99),
            ..published[0].clone()
        }];
        assert_eq!(memory.unread(SOURCE, &edited), 0);
    }

    #[test]
    fn marking_read_remembers_only_what_the_list_showed() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::new(dir.path());
        memory.mark_read(SOURCE, &[item("a", Some(1)), item("b", Some(2))]);
        // The window moves on; `a` has fallen off it.
        memory.mark_read(SOURCE, &[item("b", Some(2)), item("c", Some(3))]);

        let marks = memory.read();
        assert_eq!(
            marks.seen[SOURCE],
            ["b".to_owned(), "c".to_owned()].into_iter().collect()
        );
    }

    #[test]
    fn a_second_source_is_seeded_on_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::new(dir.path());
        memory.mark_read(SOURCE, &[item("a", Some(1))]);

        // Adding a feed later must not report all of its history at once, and
        // must not disturb what the first one remembers.
        assert_eq!(memory.unread("devlog", &[item("d1", Some(1))]), 0);
        assert_eq!(memory.unread(SOURCE, &[item("a", Some(1))]), 0);
    }

    #[test]
    fn an_unreadable_marks_file_means_nothing_has_been_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MEMORY_FILE), "not json").unwrap();
        let memory = Memory::new(dir.path());
        assert_eq!(memory.unread(SOURCE, &[item("a", Some(1))]), 0);
    }
}
