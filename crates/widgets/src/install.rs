//! Where a widget's bytes come from, and what to check them against.
//!
//! The usage document says what a widget is and how many people run it. This is
//! the other half: what a one-click install actually does.
//!
//! **A link for every widget, a download for some.** Roughly half of what the
//! pipeline publishes has no traceable source at all — a hash nobody has seen in
//! a repository, a gist or the Discord channel — and of what is traceable, not
//! all of it may be redistributed. Both cases keep their row and say why, which
//! is the whole reason [`Install`] is always present rather than optional.
//!
//! **The licence decides, upstream.** A widget whose `GetInfo()` does not grant
//! redistribution has no `url` here and its Lua is not in the published bundle
//! either — the decision is made where the bytes are, not here. This end only
//! renders the consequence.
//!
//! **Every file carries the hash BAR itself would compute.** `content_hash` is
//! base64 of the MD5 of the file, which is exactly `VFS.CalculateHash(data, 0)`
//! — the same value the game broadcasts in a replay. So a client verifies a
//! download with the game's own algorithm, and a widget that verifies is
//! bit-identical to the one those players were measured running.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where a widget's bytes live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum InstallKind {
    /// BAR's own Widget Hub. A zip to unpack.
    Hub,
    /// A GitHub repository, pinned to the commit that was scanned.
    Github,
    /// A gist, pinned to the revision that was scanned.
    Gist,
    /// The Discord widget channel. These bytes are served by pve.bar, because
    /// Discord's own attachment links are signed and expire.
    Discord,
    /// No source was ever found for this widget's hash.
    #[default]
    None,
}

/// One file of a widget, and how to check it arrived intact.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InstallFile {
    /// Path as published, which may carry the directories it sat under
    /// upstream. [`Self::file_name`] is what it installs as.
    pub path: String,
    /// Base64 MD5 — `VFS.CalculateHash(data, 0)`, the game's own value.
    pub content_hash: String,
    /// Empty for a hub archive member, which is not separately addressable,
    /// and for anything the licence withheld.
    #[serde(default)]
    pub url: String,
}

impl InstallFile {
    /// What this installs as, with any upstream directories dropped.
    ///
    /// A repository keeps its widgets at `Widgets/community/<id>/<id>.lua`;
    /// BAR wants `<id>.lua` in `LuaUI/Widgets`. Path separators are stripped
    /// rather than sanitised, so nothing a published document says can write
    /// outside the directory it was pointed at.
    pub fn file_name(&self) -> Option<&str> {
        let name = self.path.rsplit(['/', '\\']).next()?.trim();
        let safe = !name.is_empty()
            && name != "."
            && name != ".."
            && name.ends_with(".lua")
            && !name.contains(':');
        safe.then_some(name)
    }
}

/// How to install one widget, or why it cannot be offered.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Install {
    #[serde(default)]
    pub kind: InstallKind,
    /// What to download. Empty when withheld or unknown.
    #[serde(default)]
    pub url: String,
    /// Whether `url` is an archive to unpack rather than a single Lua file.
    #[serde(default)]
    pub archive: bool,
    /// A page a reader can open: repository, gist or Discord thread. Present
    /// far more often than `url`.
    #[serde(default)]
    pub page: String,
    /// Verbatim from the widget's `GetInfo()`. Spelling varies wildly.
    #[serde(default)]
    pub license: String,
    /// Whether that licence grants redistribution.
    #[serde(default)]
    pub permissive: bool,
    /// Why there is no download, when there is none.
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub files: Vec<InstallFile>,
}

impl Install {
    /// Whether a download can be offered.
    pub fn is_installable(&self) -> bool {
        !self.url.is_empty() && !self.files.is_empty()
    }

    /// Whether there is at least a page to send the reader to.
    pub fn is_linkable(&self) -> bool {
        !self.page.is_empty()
    }

    /// The files to fetch individually. Empty for an archive.
    pub fn downloadable_files(&self) -> impl Iterator<Item = &InstallFile> {
        self.files.iter().filter(|file| !file.url.is_empty())
    }

    /// What a row should say in place of a download button.
    ///
    /// Never empty when there is no download: a button that does nothing and
    /// says nothing is worse than no button.
    pub fn unavailable_because(&self) -> Option<&str> {
        if self.is_installable() {
            return None;
        }
        Some(match () {
            _ if !self.reason.is_empty() => self.reason.as_str(),
            _ if self.kind == InstallKind::None => "no known source",
            _ => "no download available",
        })
    }
}
