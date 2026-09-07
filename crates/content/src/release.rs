//! Finding an engine build to download.
//!
//! There is a chicken and egg in getting a machine from nothing to a playable
//! game: pr-downloader fetches the game and the maps, but pr-downloader ships
//! *inside* an engine, so the engine cannot come from it. It comes from BAR's
//! file index instead — the same host pr-downloader itself searches:
//!
//! ```text
//! https://files-cdn.beyondallreason.dev/find?category=engine_windows64&springname=<version>
//! ```
//!
//! which answers with a JSON array whose first entry carries the mirrors. This
//! module is the part that can be decided without a network: which URL to ask,
//! and what the answer means.

use serde::Deserialize;

/// Where BAR's file index lives: the same endpoint pr-downloader is handed as
/// `PRD_HTTP_SEARCH_URL`, for the same reason.
pub const FIND_URL: &str = recoil::HTTP_SEARCH_URL;

/// The engine build for this machine, as BAR's index categorises them.
///
/// `None` on macOS, and that is the point: BAR publishes no Apple build, and
/// answering `engine_linux64` there would fetch an ELF, unpack it, mark it
/// executable and report an engine installed -- a failure that only shows
/// itself as an exec error at the moment somebody tries to play. A missing
/// category is a refusal the caller can explain.
pub const fn category() -> Option<&'static str> {
    if cfg!(windows) {
        Some("engine_windows64")
    } else if cfg!(target_os = "macos") {
        None
    } else {
        Some("engine_linux64")
    }
}

/// Why there is no engine to download here, in words for a person.
///
/// The whole instruction, because this answers a request somebody actually
/// made: an error has no buttons beside it, so it has to carry both where the
/// engine comes from and where it goes.
pub const NO_CATEGORY: &str = "Beyond All Reason publishes no macOS engine, so modlobby cannot fetch one. Install an Apple Silicon build by hand -- the BAR Launcher app from github.com/Vandomas/RecoilEngine-AppleSilicon -- into the engine folder of the data directory, and modlobby will find the engine inside it.";

/// The same fact for a room, which has the buttons this sentence would
/// otherwise have to describe.
///
/// Two spellings of one refusal is how a room and a runtime come to disagree
/// about what a machine can do, so both live here and both come from
/// [`category`]. What differs is only who is being told: [`NO_CATEGORY`]
/// answers a request that was made, this one is said before anyone makes it.
pub const NOT_PUBLISHED_HERE: &str = "Beyond All Reason publishes no engine for this machine, so modlobby cannot fetch one. An Apple Silicon build goes into the engine folder by hand.";

/// Why no engine can be fetched for `category`, when none can.
///
/// The absence [`category`] returns, in words. Both callers that have to
/// explain themselves to a person -- the download that refuses, and the room
/// that would otherwise offer it -- would each have to work out for themselves
/// that a missing category is what "no Apple build" looks like from here, and
/// a fact spelled out twice is how the room comes to offer what the runtime
/// refuses.
///
/// Takes the category rather than reading it, the way `recoil::refuse_target`
/// takes the platform answer: an invariant only ever tested on the platform it
/// fires on is one nobody notices breaking.
pub const fn no_published_engine_for(category: Option<&str>) -> Option<&'static str> {
    match category {
        Some(_) => None,
        None => Some(NOT_PUBLISHED_HERE),
    }
}

/// This machine's own answer, for a caller with no category in hand.
pub const fn no_published_engine() -> Option<&'static str> {
    no_published_engine_for(category())
}

/// One entry from the index. Only the fields worth acting on are read; the
/// index carries more and may grow.
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub filename: String,
    /// Where it can be fetched from, best first.
    #[serde(default)]
    pub mirrors: Vec<String>,
    /// Bytes, for a progress bar that means something.
    #[serde(default)]
    pub size: u64,
    /// Of the whole archive, lowercase hex. The index has carried one for
    /// every engine so far; an entry without it is unpacked unverified.
    #[serde(default)]
    pub md5: Option<String>,
}

/// The query for one engine version on this platform, where there is one.
pub fn find_url(version: &str) -> Option<String> {
    // The version can carry characters that matter in a query string; BAR's
    // own versions are dotted digits, but encoding is still the correct thing.
    Some(format!(
        "{FIND_URL}?category={}&springname={}",
        category()?,
        urlencode(version)
    ))
}

/// The build to fetch, or `None` when the index knows of none.
///
/// An entry with no mirrors is no use: it names a file nothing can reach.
pub fn pick(body: &str) -> Option<Release> {
    let releases: Vec<Release> = serde_json::from_str(body).ok()?;
    releases
        .into_iter()
        .find(|release| !release.mirrors.is_empty())
}

/// Percent-encodes everything that is not unreserved, which is all a query
/// value needs and avoids a URL-building dependency for one string.
fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written against a room that offers a download the runtime refuses.
    ///
    /// Both arms go through `_for`, so the uncommon one is exercised from
    /// every machine rather than only from the one it fires on. The last line
    /// is the invariant rather than a restatement of the first: `find_url` is
    /// `None` only by way of `category()?`, so the reason the room reads and
    /// the URL the downloader builds have to agree or one of them is lying.
    #[test]
    fn the_reason_and_the_category_are_one_answer() {
        assert_eq!(no_published_engine_for(Some("engine_linux64")), None);
        assert_eq!(no_published_engine_for(None), Some(NOT_PUBLISHED_HERE));
        assert_eq!(no_published_engine().is_none(), category().is_some());
        assert_eq!(
            find_url("2026.07.04").is_none(),
            no_published_engine().is_some()
        );
    }

    #[test]
    fn the_query_names_this_platform_and_the_version() {
        let Some(url) = find_url("2026.07.04") else {
            return;
        };
        assert!(url.starts_with(FIND_URL));
        assert!(url.contains("springname=2026.07.04"));
        assert!(url.contains(if cfg!(windows) {
            "category=engine_windows64"
        } else {
            "category=engine_linux64"
        }));
    }

    #[test]
    fn a_version_with_awkward_characters_is_encoded() {
        let Some(url) = find_url("2026.07.04 rc/1") else {
            return;
        };
        assert!(url.contains("2026.07.04%20rc%2F1"));
        assert!(!url.contains(' '));
    }

    #[test]
    fn the_first_entry_with_a_mirror_is_the_one_to_fetch() {
        let body = r#"[
            {"filename":"a.7z","mirrors":[],"size":1},
            {"filename":"b.7z","mirrors":["https://x/b.7z"],"size":123}
        ]"#;
        let release = pick(body).expect("a release");
        assert_eq!(release.filename, "b.7z");
        assert_eq!(release.size, 123);
        assert_eq!(release.mirrors[0], "https://x/b.7z");
    }

    #[test]
    fn an_index_that_knows_nothing_is_not_an_error_here() {
        // The index answers an unknown version with an empty array, which is
        // an answer rather than a failure — the caller says "no such engine".
        assert!(pick("[]").is_none());
        assert!(pick("not json").is_none());
        assert!(pick(r#"[{"filename":"a.7z","mirrors":[]}]"#).is_none());
    }

    #[test]
    fn fields_the_index_grows_are_ignored_rather_than_fatal() {
        let body = r#"[{"filename":"a.7z","mirrors":["u"],"size":9,"tags":["y"],"path":"engine"}]"#;
        let release = pick(body).expect("a release");
        assert_eq!(release.filename, "a.7z");
        assert_eq!(release.md5, None);
    }

    #[test]
    fn the_checksum_is_kept_for_the_download_to_verify() {
        let body =
            r#"[{"filename":"a.7z","mirrors":["u"],"md5":"87c91c5c81898622d6870708d05150b1"}]"#;
        assert_eq!(
            pick(body).and_then(|r| r.md5),
            Some("87c91c5c81898622d6870708d05150b1".into())
        );
    }
}
