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
//!
//! Except on macOS, where there is nothing in that index to ask for and the
//! engine comes from [`crate::apple`] instead. Which of the two a machine uses
//! is [`Source`], and it is the thing to branch on rather than the category.

use serde::Deserialize;

/// Where BAR's file index lives: the same endpoint pr-downloader is handed as
/// `PRD_HTTP_SEARCH_URL`, for the same reason.
pub const FIND_URL: &str = recoil::HTTP_SEARCH_URL;

/// Where an engine for this machine comes from.
///
/// Two answers rather than one, because macOS is a different kind of problem
/// from the others. Everywhere Beyond All Reason publishes a build, the file
/// index is asked for a named version and answers with mirrors. Beyond All
/// Reason publishes no Apple build and will not: the only one that exists is
/// [`crate::apple`]'s, a third party's, fetched from its own releases and
/// carrying whichever engine its author built against.
///
/// One value rather than a category and a flag beside it, so there is no way
/// to write a caller that handles a category it does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// BAR's file index, under this category.
    Index(&'static str),
    /// The unofficial Apple Silicon build's own releases.
    AppleSilicon,
    /// Nowhere, for the reason given.
    Nowhere(&'static str),
}

impl Source {
    /// The index category, for the one source that is an index.
    pub const fn category(self) -> Option<&'static str> {
        match self {
            Self::Index(category) => Some(category),
            _ => None,
        }
    }

    /// Why nothing can be fetched here, when nothing can.
    ///
    /// The sentence rather than a `bool`, so nothing can draw the refusal
    /// without the words that explain it.
    pub const fn unavailable(self) -> Option<&'static str> {
        match self {
            Self::Nowhere(why) => Some(why),
            _ => None,
        }
    }

    /// The provenance a person should be told before it is fetched, for a
    /// source that is not Beyond All Reason's own.
    ///
    /// `None` for the index: an engine from BAR's own CDN needs no
    /// introduction, and a notice on every platform is a notice nobody reads
    /// on the one where it matters.
    pub const fn third_party(self) -> Option<&'static str> {
        match self {
            Self::AppleSilicon => Some(crate::apple::PROVENANCE),
            _ => None,
        }
    }
}

/// This machine's source.
pub const fn source() -> Source {
    if cfg!(windows) {
        Source::Index("engine_windows64")
    } else if cfg!(target_os = "macos") {
        // Apple Silicon only. There is no Intel build of the port, and
        // `engine_linux64` would fetch an ELF, unpack it, mark it executable
        // and report an engine installed -- a failure that only shows itself
        // as an exec error at the moment somebody tries to play.
        if crate::apple::supported() {
            Source::AppleSilicon
        } else {
            Source::Nowhere(crate::apple::NO_INTEL_BUILD)
        }
    } else {
        Source::Index("engine_linux64")
    }
}

/// The engine build for this machine, as BAR's index categorises them.
///
/// `None` on macOS, where the engine does not come from the index at all.
pub const fn category() -> Option<&'static str> {
    source().category()
}

/// Why a blank version is not a question worth asking.
///
/// The index answers `springname=` with a bare 404, which reaches a person as
/// a network error about a URL rather than as the missing version it is -- and
/// it is a request that could not have succeeded whatever the server did, so
/// it is one BAR's CDN should never have been asked. This says what modlobby
/// actually knows, which is not which engine to fetch.
pub const NO_VERSION: &str = "modlobby does not know which engine version to fetch, so there is nothing to ask BAR's index for. Open a battle or pick an engine version and try again.";

/// Why there is no index query to make on this machine.
///
/// Reached only by a caller that asked the index on a platform whose engine
/// does not come from it, which on Apple Silicon is a defect rather than a
/// fact about the machine -- [`source`] is what a caller is meant to branch
/// on. It still has to be a sentence, because an error with no words is one
/// nobody can report.
pub const NOT_IN_THE_INDEX: &str = "Beyond All Reason's file index publishes no engine for this machine, so there is nothing to ask it for.";

/// Why no engine can be fetched here, when none can.
///
/// `None` wherever there is a source, which since the Apple Silicon build is
/// fetched rather than installed by hand is everywhere but an Intel Mac. The
/// room reads this to decide whether to offer the download at all, and
/// `download_engine` refuses with the same words, so the two cannot disagree
/// about what this machine can do.
pub const fn no_published_engine() -> Option<&'static str> {
    source().unavailable()
}

/// The provenance to show beside the offer, when the engine is not BAR's own.
pub const fn third_party_engine() -> Option<&'static str> {
    source().third_party()
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

/// Why there is no index query to make, when there is none.
///
/// Two reasons, and they are not the same kind of thing: one is a fact about
/// the machine that will still be true tomorrow, the other is modlobby not
/// knowing its own mind. They are told apart here so the error carries which,
/// rather than both arriving as "no".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoQuery {
    /// No version to ask about.
    Version,
    /// No build published for this machine.
    Platform,
}

impl NoQuery {
    /// The tag the error carries: a machine that will never have an engine is
    /// a different case from one that momentarily does not know which, and a
    /// caller that wants to treat them differently can.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Platform => "platform",
        }
    }

    pub const fn reason(self) -> &'static str {
        match self {
            Self::Version => NO_VERSION,
            // The index's own refusal, which is not the same sentence as "this
            // machine can have no engine": on Apple Silicon the index has
            // nothing and the machine still gets one.
            Self::Platform => match source().unavailable() {
                Some(why) => why,
                None => NOT_IN_THE_INDEX,
            },
        }
    }
}

/// The query for one engine version on this platform, or why there is none.
///
/// Both refusals live here because both end at the same request, and a request
/// that cannot succeed is one to not send. The blank version is checked first
/// on purpose: it is a defect in modlobby rather than a fact about the machine,
/// and answering it with the macOS refusal on macOS is how a defect comes to be
/// invisible on the one platform whose users would report it.
pub fn find_url(version: &str) -> Result<String, NoQuery> {
    // Trimmed for the query as well as for the check: a padded version is
    // still one the index would 404, and a 404 is what this is here to stop.
    let version = version.trim();
    if version.is_empty() {
        return Err(NoQuery::Version);
    }
    // The version can carry characters that matter in a query string; BAR's
    // own versions are dotted digits, but encoding is still the correct thing.
    Ok(format!(
        "{FIND_URL}?category={}&springname={}",
        category().ok_or(NoQuery::Platform)?,
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
    /// Every arm goes through [`Source`], so the uncommon ones are exercised
    /// from every machine rather than only from the one they fire on. The
    /// invariant is that one value decides all three questions -- whether a
    /// download is offered, which URL it builds, and what a person is told
    /// about where the engine comes from -- because a room that works them out
    /// separately is a room that offers what the runtime refuses.
    #[test]
    fn one_source_answers_the_offer_the_url_and_the_provenance() {
        let index = Source::Index("engine_linux64");
        assert_eq!(index.category(), Some("engine_linux64"));
        assert_eq!(index.unavailable(), None);
        assert_eq!(index.third_party(), None, "BAR's own CDN needs no notice");

        // Fetchable, and from somewhere that is not Beyond All Reason.
        assert_eq!(Source::AppleSilicon.category(), None);
        assert_eq!(Source::AppleSilicon.unavailable(), None);
        assert_eq!(
            Source::AppleSilicon.third_party(),
            Some(crate::apple::PROVENANCE)
        );

        // The only machine with nothing to fetch carries its own reason.
        let nowhere = Source::Nowhere(crate::apple::NO_INTEL_BUILD);
        assert_eq!(nowhere.category(), None);
        assert_eq!(nowhere.unavailable(), Some(crate::apple::NO_INTEL_BUILD));
        assert_eq!(nowhere.third_party(), None);

        // And this machine's answers all come from the one value.
        assert_eq!(no_published_engine(), source().unavailable());
        assert_eq!(third_party_engine(), source().third_party());
        assert_eq!(category(), source().category());
    }

    /// macOS gained a source; it must not have gained an index query with it.
    ///
    /// Asserted on every platform through [`Source`] rather than only where it
    /// fires: `find_url` builds the query for BAR's CDN, and Apple Silicon is
    /// the case where an engine *is* fetchable and that query still must not
    /// be sent. A caller reaching it anyway gets a sentence about the index,
    /// not the "no engine for this machine" that would be a lie there.
    #[test]
    fn the_apple_build_is_fetchable_without_being_in_bars_index() {
        assert!(Source::AppleSilicon.unavailable().is_none());
        assert!(Source::AppleSilicon.category().is_none());
        assert_eq!(
            find_url("2026.07.04").err(),
            source()
                .category()
                .map_or(Some(NoQuery::Platform), |_| None)
        );
        // The Intel Mac's reason is about the machine; every other platform
        // reaching `Platform` is a defect, and says so about the index.
        assert_eq!(
            NoQuery::Platform.reason(),
            source().unavailable().unwrap_or(NOT_IN_THE_INDEX)
        );
    }

    #[test]
    fn the_query_names_this_platform_and_the_version() {
        let Ok(url) = find_url("2026.07.04") else {
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
        let Ok(url) = find_url("2026.07.04 rc/1") else {
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
