//! A game built from a commit: what a room plays when its game is a
//! developer's build that nobody published.
//!
//! A packaged build is its git tree, as the blobs are stored, with the
//! submodules checked out and the version written into `modinfo.lua` in
//! place of a placeholder. BAR's rapid build `test-31368-8379d65` was
//! compared file by file with commit `8379d65` (2026-09-22): the only
//! differences were `modinfo.lua`'s `version = "$VERSION"` and the files of
//! its `recoil-lua-library` submodule. So a build is reproducible from the
//! commit. What does not follow from that is decided by the room: a build is
//! only kept once its checksum matches the hash the room announced.
//!
//! GitHub's archives are made by `git archive`, which writes a file its
//! `.gitattributes` marks `eol=crlf` with CRLF line ends where the blob has
//! LF; [`Attributes`] undoes that, so what is unpacked is what was committed.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use regex::Regex;
use sha1::{Digest as _, Sha1};

use crate::api::{Api, Refusal};

/// The most a build unpacks to, all its files together: a repository is its
/// author's to fill and a list's entry anyone's to write, and the largest
/// real game is a fraction of this.
const MOST_UNPACKED: u64 = 8 << 30;

/// The most one file whose line ends are put back may be, since it is held
/// whole to do that: those are text, and text this large is not.
const MOST_TEXT: u64 = 256 << 20;

/// The most a `.gitattributes` may be, since it is held whole to parse.
const MOST_ATTRIBUTES: u64 = 1 << 20;

/// Set when the build that asked for an unpack is gone -- its download
/// stopped, its future dropped -- so the unpack, which runs off the async
/// threads and cannot be dropped with it, stops at its next entry.
pub struct Dropped(Arc<AtomicBool>);

impl Dropped {
	pub fn new() -> Self {
		Self(Arc::new(AtomicBool::new(false)))
	}

	/// The flag the unpack watches.
	pub fn flag(&self) -> Arc<AtomicBool> {
		Arc::clone(&self.0)
	}
}

impl Default for Dropped {
	fn default() -> Self {
		Self::new()
	}
}

impl Drop for Dropped {
	fn drop(&mut self) {
		self.0.store(true, Ordering::SeqCst);
	}
}

/// What one build may cost, all its checkouts together: the bytes written
/// (the archives' and the ones fetched raw), the files, the checkouts (the
/// repository and every submodule), and how deep submodules may nest. A
/// repository is its author's to fill; the budget is what this machine
/// agrees to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
	pub bytes: u64,
	pub files: u32,
	pub checkouts: u32,
	pub depth: u8,
}

impl Default for Budget {
	fn default() -> Self {
		Self {
			bytes: MOST_UNPACKED,
			files: 100_000,
			checkouts: 32,
			depth: 4,
		}
	}
}

/// What a build has spent so far, counted across its checkouts.
#[derive(Debug, Default)]
pub struct Spent {
	bytes: u64,
	files: u32,
	checkouts: u32,
}

impl Spent {
	fn spend_bytes(&mut self, count: u64, budget: &Budget, what: &str) -> Result<(), String> {
		self.bytes = self.bytes.saturating_add(count);
		if self.bytes > budget.bytes {
			return Err(format!(
				"{what} takes the build past {} bytes",
				budget.bytes
			));
		}
		Ok(())
	}

	fn spend_files(&mut self, count: u32, budget: &Budget, what: &str) -> Result<(), String> {
		self.files = self.files.saturating_add(count);
		if self.files > budget.files {
			return Err(format!(
				"{what} takes the build past {} files",
				budget.files
			));
		}
		Ok(())
	}

	fn spend_checkout(&mut self, budget: &Budget, repo: &str) -> Result<(), String> {
		self.checkouts += 1;
		if self.checkouts > budget.checkouts {
			return Err(format!(
				"{repo} takes the build past {} checkouts",
				budget.checkouts
			));
		}
		Ok(())
	}
}

/// What a room's version names a commit by: its last `-`-separated part,
/// when that is 7 to 40 hex digits (`test-31368-8379d65` → `8379d65`).
pub fn short_hash(version: &str) -> Option<&str> {
	let hash = version.rsplit('-').next()?;
	let hex = hash
		.chars()
		.all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
	((7..=40).contains(&hash.len()) && hex).then_some(hash)
}

/// The version placeholder a packager writes over, when a list says nothing.
pub const PLACEHOLDER: &str = "$VERSION";

/// Whether `sha` is a whole commit hash as git writes it: 40 lowercase hex
/// digits, so it names one commit and nothing can be moved under it.
pub fn is_commit(sha: &str) -> bool {
	sha.len() == 40 && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The line-end rules a repository's `.gitattributes` sets: for each path,
/// whether `git archive` wrote it with CRLF ends. The last rule that says
/// anything about line ends decides, as in git.
pub struct Attributes {
	rules: Vec<(Regex, bool)>,
}

impl Attributes {
	pub fn parse(text: &str) -> Self {
		let rules = text
			.lines()
			.filter_map(|line| {
				let line = line.trim();
				if line.is_empty() || line.starts_with('#') {
					return None;
				}
				let mut words = line.split_whitespace();
				let pattern = words.next()?;
				let crlf = words.fold(None, |crlf, attr| match attr {
					"eol=crlf" => Some(true),
					"eol=lf" | "-text" | "binary" => Some(false),
					_ => crlf,
				})?;
				Some((Regex::new(&pattern_to_regex(pattern)).ok()?, crlf))
			})
			.collect();
		Self { rules }
	}

	/// Whether `path` (from the repository's root, `/`-separated) was
	/// written with CRLF line ends.
	pub fn crlf(&self, path: &str) -> bool {
		self.rules
			.iter()
			.rfind(|(regex, _)| regex.is_match(path))
			.is_some_and(|(_, crlf)| *crlf)
	}
}

/// A gitattributes pattern as a regex over the path from the root: without a
/// `/` it matches the file's name in any directory; with one, the path from
/// the root. `*` and `?` stay within a directory, `**` crosses them.
fn pattern_to_regex(pattern: &str) -> String {
	let anchored = pattern.trim_start_matches('/');
	let mut regex = String::from(if pattern.contains('/') { "^" } else { "(^|/)" });
	let mut chars = anchored.chars().peekable();
	while let Some(c) = chars.next() {
		match c {
			'*' if chars.peek() == Some(&'*') => {
				chars.next();
				regex.push_str(".*");
			}
			'*' => regex.push_str("[^/]*"),
			'?' => regex.push_str("[^/]"),
			c => regex.push_str(&regex::escape(&c.to_string())),
		}
	}
	regex.push('$');
	regex
}

/// `text` with every CRLF made LF.
fn lf(text: &[u8]) -> Vec<u8> {
	let mut out = Vec::with_capacity(text.len());
	let mut bytes = text.iter().peekable();
	while let Some(&byte) = bytes.next() {
		if byte == b'\r' && bytes.peek() == Some(&&b'\n') {
			continue;
		}
		out.push(byte);
	}
	out
}

/// Unpacks a GitHub archive into `into` against the tree: its one top
/// folder dropped, only the files the tree names written -- a directory
/// entry makes nothing -- and every file its `.gitattributes` had written
/// with CRLF put back as committed. Nothing outside `into` is ever written,
/// and no more bytes than the budget has left.
pub fn unpack(
	zipball: &Path,
	into: &Path,
	listed: &Listed,
	budget: &Budget,
	spent: &mut Spent,
	stop: &AtomicBool,
) -> Result<(), String> {
	let file = std::fs::File::open(zipball).map_err(|err| err.to_string())?;
	let mut zip = zip::ZipArchive::new(file).map_err(|err| err.to_string())?;
	let names: Vec<String> = zip.file_names().map(str::to_owned).collect();
	let top = names
		.first()
		.and_then(|name| name.split('/').next())
		.map(|top| format!("{top}/"))
		.unwrap_or_default();
	let attributes = match names
		.iter()
		.find(|name| name.strip_prefix(&top) == Some(".gitattributes"))
	{
		Some(name) => {
			// Held whole to parse; a repository's to write, so bounded.
			let mut text = String::new();
			let mut entry = zip.by_name(name).map_err(|err| err.to_string())?;
			(&mut entry)
				.take(MOST_ATTRIBUTES + 1)
				.read_to_string(&mut text)
				.map_err(|err| format!(".gitattributes: {err}"))?;
			if text.len() as u64 > MOST_ATTRIBUTES {
				return Err(".gitattributes is larger than a mebibyte".into());
			}
			Attributes::parse(&text)
		}
		None => Attributes { rules: Vec::new() },
	};
	for at in 0..zip.len() {
		if stop.load(Ordering::SeqCst) {
			return Err("the build was stopped".into());
		}
		let mut entry = zip.by_index(at).map_err(|err| err.to_string())?;
		let Some(inside) = entry.enclosed_name() else {
			return Err(format!(
				"{} would be written outside the game",
				entry.name()
			));
		};
		let Ok(relative) = inside.strip_prefix(top.trim_end_matches('/')) else {
			continue;
		};
		if relative.as_os_str().is_empty() {
			continue;
		}
		if cfg!(windows) && !windows_writes_as_named(relative) {
			return Err(format!("{} is not a name Windows can write", entry.name()));
		}
		if entry.is_dir() {
			continue;
		}
		let as_git = relative.to_string_lossy().replace('\\', "/");
		if !listed.blobs.contains(&as_git) {
			continue;
		}
		let path = into.join(relative);
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
		}
		let crlf = attributes.crlf(&as_git);
		let left = budget.bytes.saturating_sub(spent.bytes);
		let cap = if crlf { left.min(MOST_TEXT) } else { left };
		let mut capped = (&mut entry).take(cap + 1);
		let written = if crlf {
			let mut body = Vec::new();
			capped
				.read_to_end(&mut body)
				.map_err(|err| err.to_string())?;
			std::fs::write(&path, lf(&body)).map_err(|err| err.to_string())?;
			body.len() as u64
		} else {
			let mut file = std::fs::File::create(&path).map_err(|err| err.to_string())?;
			std::io::copy(&mut capped, &mut file).map_err(|err| err.to_string())?
		};
		if written > cap {
			return Err(format!(
				"{} takes the build past {} bytes",
				entry.name(),
				budget.bytes
			));
		}
		spent.spend_bytes(written, budget, entry.name())?;
	}
	Ok(())
}

/// Whether Windows writes `path` as named: no `:`, which writes a hidden
/// stream of another file; no trailing dot or space, which it drops, so two
/// names could be one file; and no device name (`CON`, `COM1`, …), which is
/// no file at all.
fn windows_writes_as_named(path: &Path) -> bool {
	path.components().all(|part| {
		let name = part.as_os_str().to_string_lossy();
		let stem = name.split('.').next().unwrap_or_default().trim_end();
		let device = ["CON", "PRN", "AUX", "NUL"]
			.iter()
			.any(|device| stem.eq_ignore_ascii_case(device))
			|| (stem.len() == 4
				&& ["COM", "LPT"].iter().any(|port| {
					stem.get(..3)
						.is_some_and(|head| head.eq_ignore_ascii_case(port))
				}) && stem.as_bytes()[3].is_ascii_digit());
		!name.contains(':') && !name.ends_with(['.', ' ']) && !device
	})
}

/// Writes `version` over `placeholder` in the build's `modinfo.lua`, which
/// is what names the game to the engine; refused when there is none to write.
pub fn write_version(build: &Path, placeholder: &str, version: &str) -> Result<(), String> {
	let path = build.join("modinfo.lua");
	let text = std::fs::read_to_string(&path).map_err(|err| format!("modinfo.lua: {err}"))?;
	if !text.contains(placeholder) {
		return Err(format!(
			"modinfo.lua has no {placeholder} to write the version over"
		));
	}
	std::fs::write(&path, text.replace(placeholder, version))
		.map_err(|err| format!("modinfo.lua: {err}"))
}

/// The submodules a build needs: where each goes, and the GitHub repository
/// it is -- a relative address (`../lib.git`) taken from `repo`'s owner, as
/// git takes it from the superproject's.
fn submodules(build: &Path, repo: &str) -> Vec<(String, String)> {
	let Ok(text) = std::fs::read_to_string(build.join(".gitmodules")) else {
		return Vec::new();
	};
	let value = |line: &str, key: &str| {
		let rest = line.strip_prefix(key)?.trim_start().strip_prefix('=')?;
		Some(rest.trim().to_owned())
	};
	let mut sections: Vec<(Option<String>, Option<String>)> = Vec::new();
	for line in text.lines().map(str::trim) {
		if line.starts_with('[') {
			sections.push((None, None));
		} else if let Some(section) = sections.last_mut() {
			if let Some(path) = value(line, "path") {
				section.0 = Some(path);
			} else if let Some(url) = value(line, "url") {
				section.1 = Some(url);
			}
		}
	}
	sections
		.into_iter()
		.filter_map(|(path, url)| {
			let (path, url) = (path?, url?);
			let sub = github_repo(&url, repo)?;
			plain_relative(&path).then_some((path, sub))
		})
		.collect()
}

/// A path that stays inside the build and inside a URL's path: plain names
/// joined by `/`, none of them `.` or `..`. A `.gitmodules` is the
/// repository's to write, and is not trusted to say where things go.
fn plain_relative(path: &str) -> bool {
	!path.is_empty()
		&& path.split('/').all(|part| {
			!part.is_empty()
				&& part != "."
				&& part != ".."
				&& part
					.chars()
					.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
		})
}

/// `owner/repo` out of a submodule's address: a GitHub clone address, or one
/// relative to `parent` (`../lib.git`, `../../other/lib`); `None` for
/// anywhere else.
fn github_repo(url: &str, parent: &str) -> Option<String> {
	let repo = if let Some(rest) = url
		.strip_prefix("https://github.com/")
		.or_else(|| url.strip_prefix("git@github.com:"))
	{
		rest.to_owned()
	} else if url.starts_with("../") {
		let mut parts: Vec<&str> = parent.split('/').collect();
		let mut rest = url;
		while let Some(up) = rest.strip_prefix("../") {
			parts.pop()?;
			rest = up;
		}
		parts.push(rest);
		parts.join("/")
	} else {
		return None;
	};
	let repo = repo.trim_end_matches('/').trim_end_matches(".git");
	crate::sources::is_repo(repo).then(|| repo.to_owned())
}

/// A commit as GitHub describes it: its whole hash, and when it was made.
#[derive(Debug, serde::Deserialize)]
pub struct Commit {
	pub sha: String,
	#[serde(default)]
	commit: Option<CommitDetail>,
}

#[derive(Debug, serde::Deserialize)]
struct CommitDetail {
	committer: Option<Committed>,
}

#[derive(Debug, serde::Deserialize)]
struct Committed {
	date: Option<String>,
}

impl Commit {
	/// When it was committed, as GitHub writes it (`2025-10-05T13:32:09Z`).
	pub fn date(&self) -> Option<&str> {
		self.commit.as_ref()?.committer.as_ref()?.date.as_deref()
	}
}

/// A commit's whole tree, as GitHub lists it.
#[derive(serde::Deserialize)]
struct Tree {
	tree: Vec<TreeEntry>,
	#[serde(default)]
	truncated: bool,
}

#[derive(serde::Deserialize)]
struct TreeEntry {
	path: String,
	#[serde(rename = "type")]
	kind: String,
	sha: String,
	/// A blob's size in bytes, when GitHub says.
	#[serde(default)]
	size: Option<u64>,
}

/// A tree's paths, checked: the blobs, and the submodules with the commits
/// they pin.
#[derive(Debug, Clone)]
pub struct Listed {
	blobs: BTreeSet<String>,
	pins: HashMap<String, String>,
}

impl Listed {
	/// The submodules' directories, case-folded: each its own checkout's.
	fn pinned_dirs(&self) -> HashSet<String> {
		self.pins.keys().map(|path| path.to_lowercase()).collect()
	}
}

/// The blob and submodule paths of a tree, each a plain `/`-joined path of
/// normal components Windows can write, and no two the same once case is
/// folded: on Windows those would be one file, and the build would differ
/// from the tree whichever won. Checked on every platform, since a build is
/// shared.
fn listed(tree: &Tree) -> Result<Listed, String> {
	let mut blobs = BTreeSet::new();
	let mut pins = HashMap::new();
	let mut folded: HashMap<String, String> = HashMap::new();
	for entry in &tree.tree {
		if entry.kind != "blob" && entry.kind != "commit" {
			continue;
		}
		let parts: Vec<&str> = entry.path.split('/').collect();
		let plain = !entry.path.is_empty()
			&& parts
				.iter()
				.all(|part| !part.is_empty() && *part != "." && *part != "..");
		let relative: PathBuf = parts.iter().collect();
		if !plain || !windows_writes_as_named(&relative) {
			return Err(format!("{} is not a path this build can write", entry.path));
		}
		if let Some(other) = folded.insert(entry.path.to_lowercase(), entry.path.clone())
			&& other != entry.path
		{
			return Err(format!(
				"{other} and {} are one file on Windows",
				entry.path
			));
		}
		if entry.kind == "commit" {
			pins.insert(entry.path.clone(), entry.sha.clone());
		} else {
			blobs.insert(entry.path.clone());
		}
	}
	Ok(Listed { blobs, pins })
}

/// Removes every file under `into` that the tree does not name, and every
/// directory that leaves empty; a submodule's directory is left for its own
/// checkout. Names are compared case-folded, since a file system that folds
/// them put `Units/` and `units/` in one directory. How many files went.
fn keep_only(into: &Path, listed: &Listed) -> Result<u32, String> {
	fn walk(
		dir: &Path,
		relative: &str,
		blobs: &HashSet<String>,
		pinned: &HashSet<String>,
		removed: &mut u32,
	) -> Result<bool, String> {
		let mut left = false;
		for entry in std::fs::read_dir(dir).map_err(|err| err.to_string())? {
			let entry = entry.map_err(|err| err.to_string())?;
			let name = entry.file_name().to_string_lossy().into_owned();
			let path = if relative.is_empty() {
				name
			} else {
				format!("{relative}/{name}")
			};
			let folded = path.to_lowercase();
			let is_dir = entry.file_type().map_err(|err| err.to_string())?.is_dir();
			if is_dir {
				// A submodule's place is its own checkout's.
				if pinned.contains(&folded) {
					left = true;
					continue;
				}
				if walk(&entry.path(), &path, blobs, pinned, removed)? {
					left = true;
				} else {
					std::fs::remove_dir(entry.path()).map_err(|err| err.to_string())?;
				}
			} else if blobs.contains(&folded) {
				left = true;
			} else {
				std::fs::remove_file(entry.path()).map_err(|err| err.to_string())?;
				*removed += 1;
			}
		}
		Ok(left)
	}
	let blobs: HashSet<String> = listed
		.blobs
		.iter()
		.map(|path| path.to_lowercase())
		.collect();
	let mut removed = 0;
	walk(into, "", &blobs, &listed.pinned_dirs(), &mut removed)?;
	Ok(removed)
}

/// Holds the build on the disk to its tree: as many files as the tree
/// names, outside the submodules' directories. Fewer means two of the
/// tree's names were one file on this file system -- a short name, a
/// folding the tree's own check does not know -- and the build is not the
/// commit.
fn matches_tree(into: &Path, listed: &Listed) -> Result<(), String> {
	fn count(dir: &Path, relative: &str, pinned: &HashSet<String>) -> Result<usize, String> {
		let mut files = 0;
		for entry in std::fs::read_dir(dir).map_err(|err| err.to_string())? {
			let entry = entry.map_err(|err| err.to_string())?;
			let name = entry.file_name().to_string_lossy().into_owned();
			let path = if relative.is_empty() {
				name
			} else {
				format!("{relative}/{name}")
			};
			if entry.file_type().map_err(|err| err.to_string())?.is_dir() {
				if !pinned.contains(&path.to_lowercase()) {
					files += count(&entry.path(), &path, pinned)?;
				}
			} else {
				files += 1;
			}
		}
		Ok(files)
	}
	let files = count(into, "", &listed.pinned_dirs())?;
	let named = listed.blobs.len();
	if files != named {
		return Err(format!(
			"the build holds {files} file{} where its tree names {named}: two of its names are one file here",
			if files == 1 { "" } else { "s" }
		));
	}
	Ok(())
}

/// Builds `repo`'s commit that `version` names into a `.sdd` under `games`:
/// the commit's tree as committed, its submodules -- and theirs -- at the
/// commits they pin, and `version` written over `placeholder`. Where the
/// build is.
pub async fn build(
	http: &reqwest::Client,
	api: &Api,
	repo: &str,
	version: &str,
	placeholder: &str,
	games: &Path,
	report: impl FnMut(u64, u64),
) -> Result<PathBuf, String> {
	let failed = |reason: String| format!("git {repo}: {reason}");
	crate::sources::releases_url(&api.github, repo)
		.ok_or_else(|| failed("not a repository's name".into()))?;
	let hash = short_hash(version).ok_or_else(|| failed(format!("{version} names no commit")))?;
	let commit = commit_of(http, api, repo, hash).await.map_err(failed)?;
	build_commit(
		http,
		api,
		repo,
		&commit.sha,
		Some((placeholder, version)),
		games,
		&Budget::default(),
		report,
	)
	.await
}

/// What a build of `repo` at `sha` is called under `games/`:
/// `github-owner-repo-<12 digits of the commit>.sdd`, which says where it
/// came from beside anything built from elsewhere later. One name per
/// commit, and the engine finds an archive by its file name too, so a room
/// can load a mutator by this and never mean another copy of the same mod.
pub fn build_name(repo: &str, sha: &str) -> String {
	format!(
		"github-{}-{}.sdd",
		repo.replace('/', "-"),
		&sha[..sha.len().min(12)]
	)
}

/// Whether a commit is on the branch a mod should come from, as GitHub tells
/// it through its compare API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
	/// On the branch: its tip, or behind it.
	OnBranch(String),
	/// Not on it: ahead of it, diverged from it, or unknown to GitHub, in
	/// GitHub's word.
	OffBranch { branch: String, status: String },
	/// GitHub could not be asked, and why.
	Unknown(String),
}

/// Whether `name` has [`build_name`]'s shape. Only a build has one, so no
/// such name is ever asked of rapid or a list.
pub fn is_build_name(name: &str) -> bool {
	name.strip_prefix("github-")
		.and_then(|rest| rest.strip_suffix(".sdd"))
		.and_then(|rest| rest.rsplit_once('-'))
		.is_some_and(|(repo, sha)| {
			!repo.is_empty()
				&& sha.len() == 12
				&& sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
		})
}

/// Whether `text` can name a branch, a tag or a commit to GitHub: nothing
/// empty, nothing that climbs, only letters, digits, `-`, `_`, `.` and `/`.
fn is_reference(text: &str) -> bool {
	!text.is_empty()
		&& !text.contains("..")
		&& text
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
}

/// Whether `text` reads as a commit hash: seven or more hex digits and
/// nothing else, as git abbreviates them.
fn looks_like_hash(text: &str) -> bool {
	text.len() >= 7 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Where the commits GitHub has put on a branch are kept, under the API
/// cache: `"<repo>@<sha>": "<branch>"`, never forgotten, since a commit on a
/// branch stays on it.
pub const TRACKS_FILE: &str = "tracks.json";

fn tracks_read(path: &Path) -> BTreeMap<String, String> {
	std::fs::read_to_string(path)
		.ok()
		.and_then(|text| serde_json::from_str(&text).ok())
		.unwrap_or_default()
}

/// Temp file and rename, so a crash never leaves half a record.
fn tracks_write(path: &Path, tracks: &BTreeMap<String, String>) {
	let Ok(text) = serde_json::to_string_pretty(tracks) else {
		return;
	};
	if let Some(parent) = path.parent() {
		let _ = std::fs::create_dir_all(parent);
	}
	let fresh = path.with_extension("json.tmp");
	if std::fs::write(&fresh, text).is_ok() {
		let _ = std::fs::rename(&fresh, path);
	}
}

/// Whether `sha` is on `branch` of `repo` -- the repository's default
/// branch when none is named -- as GitHub's compare tells it: `identical`
/// or `behind` is on it, `ahead` or `diverged` is not, and a commit GitHub
/// does not know is off every branch. A positive answer is kept in
/// [`TRACKS_FILE`] under the API cache and never asked for again.
pub async fn on_branch(
	http: &reqwest::Client,
	api: &Api,
	repo: &str,
	sha: &str,
	branch: Option<&str>,
) -> Reach {
	if let Some(branch) = branch {
		if !is_reference(branch) {
			return Reach::Unknown(format!("{branch:?} is not a branch name"));
		}
		// A hash is not a branch: GitHub would compare the commit with itself
		// and call it identical, whatever branch it is or is not on.
		if looks_like_hash(branch) {
			return Reach::OffBranch {
				branch: branch.to_owned(),
				status: "not a branch".into(),
			};
		}
	}
	let key = format!("{repo}@{sha}");
	let tracks = api.cache.as_deref().map(|dir| dir.join(TRACKS_FILE));
	if let Some(path) = &tracks
		&& let Some(branch) = tracks_read(path).remove(&key)
	{
		return Reach::OnBranch(branch);
	}
	let branch = match branch {
		Some(branch) => {
			// Only a branch GitHub has: a tag or a pull request ref is a
			// well-formed name that is nobody's branch.
			let url = format!("{}/repos/{repo}/branches/{branch}", api.github);
			match api.fetch(http, &url, "the branch").await {
				Ok(_) => branch.to_owned(),
				Err(Refusal {
					status: Some(reqwest::StatusCode::NOT_FOUND),
					..
				}) => {
					return Reach::OffBranch {
						branch: branch.to_owned(),
						status: "not a branch".into(),
					};
				}
				Err(refusal) => return Reach::Unknown(refusal.text),
			}
		}
		None => {
			let url = format!("{}/repos/{repo}", api.github);
			match api
				.json::<serde_json::Value>(http, &url, "the repository")
				.await
			{
				Ok(answer) => match answer.get("default_branch").and_then(|b| b.as_str()) {
					Some(branch) => branch.to_owned(),
					None => return Reach::Unknown("the repository names no default branch".into()),
				},
				Err(reason) => return Reach::Unknown(reason),
			}
		}
	};
	let url = format!("{}/repos/{repo}/compare/{branch}...{sha}", api.github);
	let status = match api.fetch(http, &url, "the comparison").await {
		Ok(body) => serde_json::from_str::<serde_json::Value>(&body)
			.ok()
			.and_then(|answer| answer.get("status")?.as_str().map(str::to_owned)),
		Err(Refusal {
			status: Some(reqwest::StatusCode::NOT_FOUND),
			..
		}) => {
			return Reach::OffBranch {
				branch,
				status: "unknown to GitHub".into(),
			};
		}
		Err(refusal) => return Reach::Unknown(refusal.text),
	};
	match status.as_deref() {
		Some("identical" | "behind") => {
			if let Some(path) = &tracks {
				let mut kept = tracks_read(path);
				kept.insert(key, branch.clone());
				tracks_write(path, &kept);
			}
			Reach::OnBranch(branch)
		}
		Some(status @ ("ahead" | "diverged")) => Reach::OffBranch {
			branch,
			status: status.to_owned(),
		},
		Some(other) => Reach::Unknown(format!(
			"GitHub answered a status it does not document: {other}"
		)),
		None => Reach::Unknown("GitHub's comparison had no status".into()),
	}
}

/// The commit `reference` names in `repo`: a branch, a tag, `HEAD` for the
/// default branch, or a commit hash as short as GitHub can tell apart.
pub async fn commit_of(
	http: &reqwest::Client,
	api: &Api,
	repo: &str,
	reference: &str,
) -> Result<Commit, String> {
	if !is_reference(reference) {
		return Err(format!("{reference:?} is not a branch, tag or commit"));
	}
	api.json(
		http,
		&format!("{}/repos/{repo}/commits/{reference}", api.github),
		"the commit",
	)
	.await
}

/// Builds `repo` at `sha`, a whole commit hash, into a `.sdd` under `games`:
/// the commit's tree as committed and its submodules -- and theirs -- at the
/// commits they pin, with `stamp`'s version written over its placeholder
/// where there is one. What a mutator's host pins is built this way, as it
/// stands. Where the build is.
#[allow(clippy::too_many_arguments)]
pub async fn build_commit(
	http: &reqwest::Client,
	api: &Api,
	repo: &str,
	sha: &str,
	stamp: Option<(&str, &str)>,
	games: &Path,
	budget: &Budget,
	mut report: impl FnMut(u64, u64),
) -> Result<PathBuf, String> {
	let failed = |reason: String| format!("git {repo}: {reason}");
	crate::sources::releases_url(&api.github, repo)
		.ok_or_else(|| failed("not a repository's name".into()))?;
	if !is_commit(sha) {
		return Err(failed(format!("{sha} is not a whole commit hash")));
	}
	let name = build_name(repo, sha);
	let staging = games.join(format!(".{name}.part"));
	let _ = std::fs::remove_dir_all(&staging);
	std::fs::create_dir_all(&staging).map_err(|err| failed(err.to_string()))?;

	let mut spent = Spent::default();
	let mut pending = vec![(repo.to_owned(), sha.to_owned(), staging.clone(), 0_u8)];
	let built = async {
		while let Some((at_repo, sha, into, depth)) = pending.pop() {
			if depth > budget.depth {
				return Err(failed(format!(
					"{at_repo} nests submodules deeper than {}",
					budget.depth
				)));
			}
			let pinned = checkout(
				http,
				api,
				&at_repo,
				&sha,
				&into,
				games,
				budget,
				&mut spent,
				&mut report,
			)
			.await
			.map_err(|err| failed(format!("{at_repo} at {}: {err}", &sha[..sha.len().min(7)])))?;
			for (at, sub) in submodules(&into, &at_repo) {
				let Some(sub_sha) = pinned.get(&at) else {
					return Err(failed(format!(
						"{at_repo} names a submodule at {at} and pins nothing there"
					)));
				};
				pending.push((sub, sub_sha.clone(), into.join(&at), depth + 1));
			}
		}
		if let Some((placeholder, version)) = stamp {
			write_version(&staging, placeholder, version).map_err(failed)?;
		}
		Ok(())
	}
	.await;
	if let Err(err) = built {
		// Nothing half-built stays: the next build of this name starts clean
		// either way, and the sweep has less to find.
		let _ = std::fs::remove_dir_all(&staging);
		return Err(err);
	}
	let built = games.join(&name);
	let _ = std::fs::remove_dir_all(&built);
	std::fs::rename(&staging, &built).map_err(|err| failed(err.to_string()))?;
	Ok(built)
}

/// One repository's commit into `into`: its tree listed first, which says
/// what the build is and what it will cost before a byte is downloaded;
/// then GitHub's archive of it unpacked against that tree, and every file
/// proved against the blob the commit names. What `git archive` left out
/// (`export-ignore`) or wrote otherwise (`export-subst`, line ends the root
/// `.gitattributes` does not explain) is fetched as committed. The
/// submodules it pins, by path.
///
/// ponytail: files fetched one at a time; a repository whose archive differs
/// from its tree in thousands of files builds slowly.
#[allow(clippy::too_many_arguments)]
async fn checkout(
	http: &reqwest::Client,
	api: &Api,
	repo: &str,
	sha: &str,
	into: &Path,
	games: &Path,
	budget: &Budget,
	spent: &mut Spent,
	report: &mut impl FnMut(u64, u64),
) -> Result<HashMap<String, String>, String> {
	spent.spend_checkout(budget, repo)?;
	let tree: Tree = api
		.json(
			http,
			&format!("{}/repos/{repo}/git/trees/{sha}?recursive=1", api.github),
			"the tree",
		)
		.await?;
	if tree.truncated {
		return Err("the tree is too large to list whole, so its files cannot be proved".into());
	}
	let listed = listed(&tree)?;
	spent.spend_files(
		u32::try_from(listed.blobs.len()).unwrap_or(u32::MAX),
		budget,
		"the tree",
	)?;
	let sized: u64 = tree
		.tree
		.iter()
		.filter(|entry| entry.kind == "blob")
		.filter_map(|entry| entry.size)
		.sum();
	if sized > budget.bytes.saturating_sub(spent.bytes) {
		return Err(format!(
			"the tree would take the build past {} bytes",
			budget.bytes
		));
	}
	let zipball = games.join(format!(".{}-{sha}.zip.part", repo.replace('/', "-")));
	let url = format!("{}/repos/{repo}/zipball/{sha}", api.github);
	crate::fetch::resumable(http, &url, &zipball, 0, &mut *report)
		.await
		.map_err(|err| err.to_string())?;
	// Off the async threads: an archive is thousands of files. The guard
	// lives across the await, so a build dropped meanwhile stops the unpack.
	let unpacked = {
		let guard = Dropped::new();
		let stop = guard.flag();
		let zipball = zipball.clone();
		let into = into.to_path_buf();
		let budget = *budget;
		let named = listed.clone();
		let mut taken = std::mem::take(spent);
		let joined = tokio::task::spawn_blocking(move || {
			let outcome = unpack(&zipball, &into, &named, &budget, &mut taken, &stop);
			(outcome, taken)
		})
		.await;
		drop(guard);
		match joined {
			Ok((outcome, taken)) => {
				*spent = taken;
				outcome
			}
			Err(err) => Err(format!("the unpack task: {err}")),
		}
	};
	let _ = std::fs::remove_file(&zipball);
	unpacked?;
	for entry in &tree.tree {
		if entry.kind == "blob" {
			prove(http, api, repo, sha, into, entry, budget, spent).await?;
		}
	}
	// What the archive held that the tree does not name goes: the build is
	// the tree, nothing else.
	let dropped = keep_only(into, &listed)?;
	if dropped > 0 {
		tracing::info!(
			repo,
			sha,
			dropped,
			"archive files the tree does not name were dropped"
		);
	}
	matches_tree(into, &listed)?;
	Ok(listed.pins)
}

/// A file as committed: kept when its blob hash is the tree's, else fetched
/// from where GitHub serves it raw, and held to the tree there too.
#[allow(clippy::too_many_arguments)]
async fn prove(
	http: &reqwest::Client,
	api: &Api,
	repo: &str,
	sha: &str,
	into: &Path,
	entry: &TreeEntry,
	budget: &Budget,
	spent: &mut Spent,
) -> Result<(), String> {
	let parts: Vec<&str> = entry.path.split('/').collect();
	let inside = parts
		.iter()
		.all(|part| !part.is_empty() && *part != "." && *part != "..");
	let relative: PathBuf = parts.iter().collect();
	if !inside || (cfg!(windows) && !windows_writes_as_named(&relative)) {
		return Err(format!("{} is not a path this build can write", entry.path));
	}
	let path = into.join(&relative);
	if blob_hash_of(&path).is_ok_and(|held| held == entry.sha) {
		return Ok(());
	}
	let mut url = reqwest::Url::parse(&api.raw).map_err(|err| err.to_string())?;
	url.path_segments_mut()
		.map_err(|()| format!("{} is not a web address", api.raw))?
		.pop_if_empty()
		.extend(repo.split('/'))
		.push(sha)
		.extend(&parts);
	let bytes = http
		.get(url)
		.send()
		.await
		.and_then(reqwest::Response::error_for_status)
		.map_err(|err| format!("{}: {err}", entry.path))?
		.bytes()
		.await
		.map_err(|err| format!("{}: {err}", entry.path))?;
	if blob_hash(&bytes) != entry.sha {
		return Err(format!(
			"{} is not as committed even where GitHub serves it raw",
			entry.path
		));
	}
	// Fetched raw, these bytes cost what the archive's did.
	spent.spend_bytes(bytes.len() as u64, budget, &entry.path)?;
	tracing::info!(
		repo,
		path = entry.path,
		"fetched as committed; the archive had it otherwise"
	);
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
	}
	std::fs::write(&path, &bytes).map_err(|err| format!("{}: {err}", entry.path))
}

/// Git's name for a file's contents: SHA-1 over `blob <length>\0` and them.
fn blob_hash(bytes: &[u8]) -> String {
	let mut hasher = Sha1::new();
	hasher.update(format!("blob {}\0", bytes.len()));
	hasher.update(bytes);
	crate::fetch::hex(&hasher.finalize())
}

/// [`blob_hash`] of a file on the disk, read in chunks.
fn blob_hash_of(path: &Path) -> std::io::Result<String> {
	let mut file = std::fs::File::open(path)?;
	let mut hasher = Sha1::new();
	hasher.update(format!("blob {}\0", file.metadata()?.len()));
	let mut chunk = [0_u8; 64 * 1024];
	loop {
		let read = file.read(&mut chunk)?;
		if read == 0 {
			break;
		}
		hasher.update(&chunk[..read]);
	}
	Ok(crate::fetch::hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
	use std::io::Write;

	use wiremock::matchers::{method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;

	/// A zipball as GitHub makes one: every path under `<repo>-<sha>/`.
	fn zipball(top: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
		let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
		let stored = zip::write::SimpleFileOptions::default();
		zip.add_directory(format!("{top}/"), stored).unwrap();
		for (name, body) in files {
			zip.start_file(format!("{top}/{name}"), stored).unwrap();
			zip.write_all(body).unwrap();
		}
		zip.finish().unwrap().into_inner()
	}

	#[test]
	fn only_a_builds_name_has_a_builds_shape() {
		let sha = "9108a17078f79d09925edc305ec83bc06c3a7cb3";
		assert!(is_build_name(&build_name("tetrisface/sphere-spawner", sha)));
		for name in [
			"sphere spawner mod v1.0.0",
			"github-tetrisface-sphere-spawner-9108a17.sdd",
			"github--9108a17078f7.sdd",
			"github-dev-sphere-9108A17078F7.sdd",
			"github-dev-sphere-9108a17078f7.sdz",
		] {
			assert!(!is_build_name(name), "{name}");
		}
	}

	#[test]
	fn a_version_names_its_commit_by_its_last_part() {
		assert_eq!(short_hash("test-31368-8379d65"), Some("8379d65"));
		assert_eq!(short_hash("test-12065-72535dc"), Some("72535dc"));
		assert_eq!(short_hash("0.1.86"), None);
		assert_eq!(short_hash("test-1-abc"), None, "too short");
		assert_eq!(
			short_hash("test-1-ABCDEF0"),
			None,
			"git writes hashes lowercase"
		);
	}

	#[test]
	fn crlf_is_undone_where_the_attributes_asked_for_it_and_nowhere_else() {
		// BAR's own.
		let attributes = Attributes::parse(
			"# Declare files that will always have CRLF line endings on checkout.\n*.lua text eol=crlf\n*.txt text eol=crlf\n/docs/*.md eol=crlf\nkeep.lua -text\n",
		);
		assert!(attributes.crlf("modinfo.lua"));
		assert!(attributes.crlf("units/armcom.lua"));
		assert!(attributes.crlf("docs/readme.md"));
		assert!(!attributes.crlf("docs/deeper/readme.md"));
		assert!(
			!attributes.crlf("sub/keep.lua"),
			"a later rule says otherwise"
		);
		assert!(!attributes.crlf("image.png"));
		assert_eq!(lf(b"a\r\nb\r\n\rc\n"), b"a\nb\n\rc\n");
	}

	/// GitHub's tree for a commit: each file with its blob hash, each
	/// submodule with the commit it pins.
	fn tree(files: &[(&str, &[u8])], pins: &[(&str, &str)]) -> String {
		let mut entries: Vec<String> = files
			.iter()
			.map(|(path, body)| {
				format!(
					r#"{{ "path": "{path}", "type": "blob", "sha": "{}" }}"#,
					blob_hash(body)
				)
			})
			.collect();
		entries.extend(pins.iter().map(|(path, sha)| {
			format!(r#"{{ "path": "{path}", "type": "commit", "sha": "{sha}" }}"#)
		}));
		format!(
			r#"{{ "tree": [{}], "truncated": false }}"#,
			entries.join(",")
		)
	}

	fn client() -> reqwest::Client {
		crate::http::client("test")
	}

	async fn serve(server: &MockServer, at: String, body: ResponseTemplate) {
		Mock::given(method("GET"))
			.and(path(at))
			.respond_with(body)
			.mount(server)
			.await;
	}

	#[tokio::test]
	async fn a_commit_is_built_as_committed_with_its_submodules_and_its_version() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		std::fs::create_dir(&games).unwrap();
		let server = MockServer::start().await;
		let sha = "8379d65aaaa00000000000000000000000000000";
		let lib_sha = "24d521d0cbe7c860a53c360d545aadceaaae3f17";
		let deep_sha = "d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0";
		let ok = |body: String| ResponseTemplate::new(200).set_body_string(body);
		let bytes = |body: Vec<u8>| ResponseTemplate::new(200).set_body_bytes(body);

		let gitattributes: &[u8] =
			b"*.lua text eol=crlf\ndocs/ export-ignore\nstamp.txt export-subst\n";
		let gitmodules: &[u8] =
			b"[submodule \"lib\"]\n\turl = https://github.com/dev/lib.git\n\tpath = lib\n";
		let modinfo: &[u8] = b"name = 'Game'\nversion = \"$VERSION\",\n";
		serve(
			&server,
			"/repos/dev/Game/commits/8379d65".into(),
			ok(format!(r#"{{ "sha": "{sha}" }}"#)),
		)
		.await;
		serve(
			&server,
			format!("/repos/dev/Game/zipball/{sha}"),
			bytes(zipball(
				"dev-Game-8379d65",
				&[
					(".gitattributes", gitattributes),
					(".gitmodules", gitmodules),
					(
						"modinfo.lua",
						b"name = 'Game'\r\nversion = \"$VERSION\",\r\n",
					),
					("units/a.lua", b"return 1\r\n"),
					// `git archive` wrote the commit into it; the tree has the
					// placeholder.
					("stamp.txt", b"8379d65\n"),
				],
			)),
		)
		.await;
		serve(
			&server,
			format!("/repos/dev/Game/git/trees/{sha}"),
			ok(tree(
				&[
					(".gitattributes", gitattributes),
					(".gitmodules", gitmodules),
					("modinfo.lua", modinfo),
					("units/a.lua", b"return 1\n"),
					("stamp.txt", b"$Format:%h$\n"),
					("docs/notes.txt", b"left out of the archive\n"),
				],
				&[("lib", lib_sha)],
			)),
		)
		.await;
		serve(
			&server,
			format!("/raw/dev/Game/{sha}/stamp.txt"),
			ok("$Format:%h$\n".into()),
		)
		.await;
		serve(
			&server,
			format!("/raw/dev/Game/{sha}/docs/notes.txt"),
			ok("left out of the archive\n".into()),
		)
		.await;

		// The submodule has its own, named relative to it.
		let lib_modules: &[u8] = b"[submodule \"deep\"]\n\tpath = deep\n\turl = ../deep.git\n";
		serve(
			&server,
			format!("/repos/dev/lib/zipball/{lib_sha}"),
			bytes(zipball(
				"dev-lib-24d521d",
				&[(".gitmodules", lib_modules), ("init.lua", b"return {}\n")],
			)),
		)
		.await;
		serve(
			&server,
			format!("/repos/dev/lib/git/trees/{lib_sha}"),
			ok(tree(
				&[(".gitmodules", lib_modules), ("init.lua", b"return {}\n")],
				&[("deep", deep_sha)],
			)),
		)
		.await;
		serve(
			&server,
			format!("/repos/dev/deep/zipball/{deep_sha}"),
			bytes(zipball("dev-deep-d0d0d0d", &[("x.lua", b"return 2\n")])),
		)
		.await;
		serve(
			&server,
			format!("/repos/dev/deep/git/trees/{deep_sha}"),
			ok(tree(&[("x.lua", b"return 2\n")], &[])),
		)
		.await;

		let client = crate::http::client("test");
		let built = build(
			&client,
			&Api::at(&server.uri()),
			"dev/Game",
			"test-5-8379d65",
			PLACEHOLDER,
			&games,
			|_, _| {},
		)
		.await
		.unwrap();
		assert_eq!(built, games.join("github-dev-Game-8379d65aaaa0.sdd"));
		let read = |path: &str| std::fs::read_to_string(built.join(path)).unwrap();
		assert_eq!(
			read("modinfo.lua"),
			"name = 'Game'\nversion = \"test-5-8379d65\",\n"
		);
		assert_eq!(read("units/a.lua"), "return 1\n");
		assert_eq!(read("stamp.txt"), "$Format:%h$\n");
		assert_eq!(read("docs/notes.txt"), "left out of the archive\n");
		assert_eq!(read("lib/init.lua"), "return {}\n");
		assert_eq!(read("lib/deep/x.lua"), "return 2\n");
		assert_eq!(
			crate::map_name::game_of_archive(&built).as_deref(),
			Some("Game test-5-8379d65")
		);
		let leftovers: Vec<_> = std::fs::read_dir(&games)
			.unwrap()
			.filter_map(Result::ok)
			.map(|entry| entry.file_name())
			.collect();
		assert_eq!(leftovers.len(), 1, "no staging or zip left: {leftovers:?}");
	}

	#[test]
	fn a_blob_is_named_as_git_names_it() {
		// `git hash-object` of an empty file and of "hello\n".
		assert_eq!(blob_hash(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
		assert_eq!(
			blob_hash(b"hello\n"),
			"ce013625030ba8dba906f756967f9e9ca394464a"
		);
	}

	#[test]
	fn only_names_windows_writes_as_named_pass() {
		for fine in [
			"luarules/gadgets/unit_x.lua",
			"icons/comet.png",
			"COM",
			"LPTX.txt",
			"abé.lua",
		] {
			assert!(windows_writes_as_named(Path::new(fine)), "{fine}");
		}
		for bad in [
			"units/aux.lua",
			"con.d/x",
			"NUL",
			"com1.txt",
			"x/Lpt9",
			"a:stream",
			"trail.",
			"space ",
		] {
			assert!(!windows_writes_as_named(Path::new(bad)), "{bad}");
		}
	}

	#[test]
	fn a_path_that_climbs_out_is_refused_and_a_version_needs_its_placeholder() {
		let dir = tempfile::tempdir().unwrap();
		let zip = dir.path().join("evil.zip");
		std::fs::write(&zip, zipball("top", &[("../../escaped.lua", b"x")])).unwrap();
		assert!(
			unpack(
				&zip,
				&dir.path().join("into"),
				&listed(&entries(&["escaped.lua"])).unwrap(),
				&Budget::default(),
				&mut Spent::default(),
				&AtomicBool::new(false)
			)
			.is_err()
		);
		assert!(!dir.path().join("escaped.lua").exists());

		std::fs::write(dir.path().join("modinfo.lua"), "version = '1.0'").unwrap();
		assert!(write_version(dir.path(), PLACEHOLDER, "test-1-abcdef0").is_err());

		// A .gitmodules is the repository's to write: only plain paths into
		// the build, and only GitHub repositories, are taken from it.
		std::fs::write(
			dir.path().join(".gitmodules"),
			"[submodule \"a\"]\n\tpath = lib/core\n\turl = https://github.com/dev/core.git\n\
			 [submodule \"b\"]\n\tpath = ../../out\n\turl = https://github.com/dev/evil.git\n\
			 [submodule \"c\"]\n\tpath = x?ref=main\n\turl = https://github.com/dev/evil.git\n\
			 [submodule \"d\"]\n\tpath = elsewhere\n\turl = https://gitlab.com/dev/other.git\n\
			 [submodule \"e\"]\n\turl = ../../them/shared\n\tpath = shared\n\
			 [submodule \"f\"]\n\tpath = far\n\turl = ../../../too/far\n",
		)
		.unwrap();
		assert_eq!(
			submodules(dir.path(), "dev/Game"),
			[
				("lib/core".to_owned(), "dev/core".to_owned()),
				("shared".to_owned(), "them/shared".to_owned()),
			]
		);
	}

	/// A GitHub that answers the repository's default branch and one
	/// comparison; `compare` is the body of the comparison's answer.
	async fn github(server: &MockServer, compare: ResponseTemplate) {
		serve(
			server,
			"/repos/dev/sphere".into(),
			ResponseTemplate::new(200).set_body_string(r#"{ "default_branch": "main" }"#),
		)
		.await;
		serve(
			server,
			format!("/repos/dev/sphere/compare/main...{TRACKED}"),
			compare,
		)
		.await;
	}

	const TRACKED: &str = "9108a17078f79d09925edc305ec83bc06c3a7cb3";

	fn status(word: &str) -> ResponseTemplate {
		ResponseTemplate::new(200).set_body_string(format!(r#"{{ "status": "{word}" }}"#))
	}

	#[tokio::test]
	async fn a_commit_behind_or_identical_to_its_branch_is_on_it() {
		for word in ["behind", "identical"] {
			let server = MockServer::start().await;
			github(&server, status(word)).await;
			let reach = on_branch(
				&client(),
				&Api::at(&server.uri()),
				"dev/sphere",
				TRACKED,
				None,
			)
			.await;
			assert_eq!(reach, Reach::OnBranch("main".into()), "{word}");
		}
	}

	#[tokio::test]
	async fn a_commit_ahead_or_diverged_is_off_it() {
		for word in ["ahead", "diverged"] {
			let server = MockServer::start().await;
			github(&server, status(word)).await;
			let reach = on_branch(
				&client(),
				&Api::at(&server.uri()),
				"dev/sphere",
				TRACKED,
				None,
			)
			.await;
			assert_eq!(
				reach,
				Reach::OffBranch {
					branch: "main".into(),
					status: word.into()
				}
			);
		}
	}

	#[tokio::test]
	async fn a_commit_github_does_not_know_is_off_every_branch() {
		let server = MockServer::start().await;
		github(&server, ResponseTemplate::new(404).set_body_string("{}")).await;
		let reach = on_branch(
			&client(),
			&Api::at(&server.uri()),
			"dev/sphere",
			TRACKED,
			None,
		)
		.await;
		assert_eq!(
			reach,
			Reach::OffBranch {
				branch: "main".into(),
				status: "unknown to GitHub".into()
			}
		);
	}

	#[tokio::test]
	async fn an_answer_github_refuses_asks_rather_than_passes() {
		let server = MockServer::start().await;
		github(
			&server,
			ResponseTemplate::new(403)
				.insert_header("x-ratelimit-remaining", "0")
				.insert_header("x-ratelimit-reset", "9999999999"),
		)
		.await;
		let reach = on_branch(
			&client(),
			&Api::at(&server.uri()),
			"dev/sphere",
			TRACKED,
			None,
		)
		.await;
		match reach {
			Reach::Unknown(reason) => assert!(reason.contains("allowance"), "{reason}"),
			other => panic!("{other:?}"),
		}
	}

	#[tokio::test]
	async fn the_hosts_track_replaces_the_default_branch() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(path("/repos/dev/sphere"))
			.respond_with(ResponseTemplate::new(200).set_body_string("{}"))
			.expect(0)
			.mount(&server)
			.await;
		serve(
			&server,
			"/repos/dev/sphere/branches/release".into(),
			ResponseTemplate::new(200).set_body_string(r#"{ "name": "release" }"#),
		)
		.await;
		serve(
			&server,
			format!("/repos/dev/sphere/compare/release...{TRACKED}"),
			status("behind"),
		)
		.await;
		let reach = on_branch(
			&client(),
			&Api::at(&server.uri()),
			"dev/sphere",
			TRACKED,
			Some("release"),
		)
		.await;
		assert_eq!(reach, Reach::OnBranch("release".into()));
	}

	#[tokio::test]
	async fn a_positive_answer_is_kept_and_asked_for_once() {
		let dir = tempfile::tempdir().unwrap();
		let server = MockServer::start().await;
		for at in [
			"/repos/dev/sphere".to_owned(),
			format!("/repos/dev/sphere/compare/main...{TRACKED}"),
		] {
			let body = if at.contains("compare") {
				status("identical")
			} else {
				ResponseTemplate::new(200).set_body_string(r#"{ "default_branch": "main" }"#)
			};
			Mock::given(method("GET"))
				.and(path(at))
				.respond_with(body)
				.expect(1)
				.mount(&server)
				.await;
		}
		let api = Api {
			cache: Some(dir.path().to_path_buf()),
			..Api::at(&server.uri())
		};
		for _ in 0..2 {
			let reach = on_branch(&client(), &api, "dev/sphere", TRACKED, None).await;
			assert_eq!(reach, Reach::OnBranch("main".into()));
		}
		assert!(dir.path().join(TRACKS_FILE).is_file());
	}

	#[tokio::test]
	async fn a_track_that_is_not_a_branch_name_is_not_asked() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(200).set_body_string("{}"))
			.expect(0)
			.mount(&server)
			.await;
		let reach = on_branch(
			&client(),
			&Api::at(&server.uri()),
			"dev/sphere",
			TRACKED,
			Some("../x"),
		)
		.await;
		assert!(matches!(reach, Reach::Unknown(_)), "{reach:?}");
	}

	/// A host naming the commit itself as the branch to check would have
	/// GitHub compare a commit with itself and call it identical. A hash is
	/// not a branch name, and is never asked about.
	#[tokio::test]
	async fn a_track_naming_the_commit_itself_is_not_a_branch() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(200).set_body_string("{}"))
			.expect(0)
			.mount(&server)
			.await;
		let reach = on_branch(
			&client(),
			&Api::at(&server.uri()),
			"dev/sphere",
			TRACKED,
			Some(TRACKED),
		)
		.await;
		assert!(
			matches!(&reach, Reach::OffBranch { status, .. } if status == "not a branch"),
			"{reach:?}"
		);
		let short = on_branch(
			&client(),
			&Api::at(&server.uri()),
			"dev/sphere",
			TRACKED,
			Some("9108a17"),
		)
		.await;
		assert!(
			matches!(&short, Reach::OffBranch { status, .. } if status == "not a branch"),
			"{short:?}"
		);
	}

	/// A track that is a well-formed name GitHub has no branch for -- a tag,
	/// a pull request ref -- is off every branch, and nothing is compared.
	#[tokio::test]
	async fn a_track_github_has_no_branch_for_is_off_every_branch() {
		let server = MockServer::start().await;
		serve(
			&server,
			"/repos/dev/sphere/branches/pull/7/head".into(),
			ResponseTemplate::new(404).set_body_string("{}"),
		)
		.await;
		Mock::given(method("GET"))
			.and(path(format!(
				"/repos/dev/sphere/compare/pull/7/head...{TRACKED}"
			)))
			.respond_with(status("identical"))
			.expect(0)
			.mount(&server)
			.await;
		let reach = on_branch(
			&client(),
			&Api::at(&server.uri()),
			"dev/sphere",
			TRACKED,
			Some("pull/7/head"),
		)
		.await;
		assert_eq!(
			reach,
			Reach::OffBranch {
				branch: "pull/7/head".into(),
				status: "not a branch".into()
			}
		);
	}

	// ------------------------------------------------------------ budgets

	const BUILD_SHA: &str = "8379d65aaaa00000000000000000000000000000";
	const LIB_SHA: &str = "24d521d0cbe7c860a53c360d545aadceaaae3f17";
	const DEEP_SHA: &str = "d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0";

	fn ok(body: String) -> ResponseTemplate {
		ResponseTemplate::new(200).set_body_string(body)
	}

	/// A repository's commit on the fake GitHub: its zipball and its tree,
	/// with `pins` as submodule entries.
	async fn repo_at(
		server: &MockServer,
		repo: &str,
		sha: &str,
		files: &[(&str, &[u8])],
		pins: &[(&str, &str)],
	) {
		let top = format!("{}-{}", repo.replace('/', "-"), &sha[..7]);
		serve(
			server,
			format!("/repos/{repo}/zipball/{sha}"),
			ResponseTemplate::new(200).set_body_bytes(zipball(&top, files)),
		)
		.await;
		serve(
			server,
			format!("/repos/{repo}/git/trees/{sha}"),
			ok(tree(files, pins)),
		)
		.await;
	}

	fn staging_left(games: &Path) -> bool {
		std::fs::read_dir(games)
			.unwrap()
			.filter_map(Result::ok)
			.any(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
	}

	#[tokio::test]
	async fn a_build_past_its_file_count_is_refused() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		std::fs::create_dir(&games).unwrap();
		let server = MockServer::start().await;
		repo_at(
			&server,
			"dev/Game",
			BUILD_SHA,
			&[
				(
					"modinfo.lua",
					b"name = 'Game'
",
				),
				(
					"units/a.lua",
					b"return 1
",
				),
				(
					"units/b.lua",
					b"return 2
",
				),
			],
			&[],
		)
		.await;
		let budget = Budget {
			files: 2,
			..Budget::default()
		};
		let err = build_commit(
			&client(),
			&Api::at(&server.uri()),
			"dev/Game",
			BUILD_SHA,
			None,
			&games,
			&budget,
			|_, _| {},
		)
		.await
		.unwrap_err();
		assert!(err.contains("files"), "{err}");
		assert!(!games.join("github-dev-Game-8379d65aaaa0.sdd").exists());
		assert!(
			!staging_left(&games),
			"the staging directory was left behind"
		);
	}

	#[tokio::test]
	async fn submodules_deeper_than_the_budget_are_refused() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		std::fs::create_dir(&games).unwrap();
		let server = MockServer::start().await;
		let game_modules: &[u8] = b"[submodule \"lib\"]
	path = lib
	url = ../lib.git
";
		let lib_modules: &[u8] = b"[submodule \"deep\"]
	path = deep
	url = ../deep.git
";
		repo_at(
			&server,
			"dev/Game",
			BUILD_SHA,
			&[
				(".gitmodules", game_modules),
				(
					"modinfo.lua",
					b"name = 'Game'
",
				),
			],
			&[("lib", LIB_SHA)],
		)
		.await;
		repo_at(
			&server,
			"dev/lib",
			LIB_SHA,
			&[
				(".gitmodules", lib_modules),
				(
					"init.lua",
					b"return {}
",
				),
			],
			&[("deep", DEEP_SHA)],
		)
		.await;
		// `dev/deep` is never served: a build that asks for it has already
		// gone past the limit.
		let budget = Budget {
			depth: 1,
			..Budget::default()
		};
		let err = build_commit(
			&client(),
			&Api::at(&server.uri()),
			"dev/Game",
			BUILD_SHA,
			None,
			&games,
			&budget,
			|_, _| {},
		)
		.await
		.unwrap_err();
		assert!(err.contains("deeper"), "{err}");
		assert!(!staging_left(&games));
	}

	#[test]
	fn a_gitattributes_past_a_mebibyte_refuses_the_unpack() {
		let dir = tempfile::tempdir().unwrap();
		let huge = vec![b'#'; (MOST_ATTRIBUTES + 1) as usize];
		let zip = dir.path().join("big.zip");
		std::fs::write(
			&zip,
			zipball(
				"dev-Game-8379d65",
				&[
					(".gitattributes", &huge),
					(
						"a.lua",
						b"return 1
",
					),
				],
			),
		)
		.unwrap();
		let into = dir.path().join("into");
		let mut spent = Spent::default();
		let err = unpack(
			&zip,
			&into,
			&listed(&entries(&[".gitattributes", "a.lua"])).unwrap(),
			&Budget::default(),
			&mut spent,
			&AtomicBool::new(false),
		)
		.unwrap_err();
		assert!(err.contains(".gitattributes"), "{err}");
	}

	#[tokio::test]
	async fn raw_fetched_bytes_spend_the_budget() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		std::fs::create_dir(&games).unwrap();
		let server = MockServer::start().await;
		// The archive has one version of the file, the tree names another:
		// the committed one is fetched raw, and those bytes cost too.
		let archived = vec![b'A'; 64 * 1024];
		let committed = vec![b'B'; 64 * 1024];
		serve(
			&server,
			format!("/repos/dev/Game/zipball/{BUILD_SHA}"),
			ResponseTemplate::new(200).set_body_bytes(zipball(
				"dev-Game-8379d65",
				&[
					(
						"modinfo.lua",
						b"name = 'Game'
",
					),
					("big.bin", &archived),
				],
			)),
		)
		.await;
		serve(
			&server,
			format!("/repos/dev/Game/git/trees/{BUILD_SHA}"),
			ok(tree(
				&[
					(
						"modinfo.lua",
						b"name = 'Game'
",
					),
					("big.bin", &committed),
				],
				&[],
			)),
		)
		.await;
		serve(
			&server,
			format!("/raw/dev/Game/{BUILD_SHA}/big.bin"),
			ResponseTemplate::new(200).set_body_bytes(committed.clone()),
		)
		.await;
		let budget = Budget {
			bytes: 100 * 1024,
			..Budget::default()
		};
		let err = build_commit(
			&client(),
			&Api::at(&server.uri()),
			"dev/Game",
			BUILD_SHA,
			None,
			&games,
			&budget,
			|_, _| {},
		)
		.await
		.unwrap_err();
		assert!(err.contains("bytes"), "{err}");
		assert!(!staging_left(&games));
	}

	// ------------------------------------------------------ the tree rules

	fn entries(paths: &[&str]) -> Tree {
		Tree {
			tree: paths
				.iter()
				.map(|path| TreeEntry {
					path: (*path).to_owned(),
					kind: "blob".into(),
					sha: "0".repeat(40),
					size: None,
				})
				.collect(),
			truncated: false,
		}
	}

	#[tokio::test]
	async fn a_truncated_tree_refuses_the_build() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		std::fs::create_dir(&games).unwrap();
		let server = MockServer::start().await;
		serve(
			&server,
			format!("/repos/dev/Game/zipball/{BUILD_SHA}"),
			ResponseTemplate::new(200).set_body_bytes(zipball(
				"dev-Game-8379d65",
				&[(
					"modinfo.lua",
					b"name = 'Game'
",
				)],
			)),
		)
		.await;
		serve(
			&server,
			format!("/repos/dev/Game/git/trees/{BUILD_SHA}"),
			ok(r#"{ "tree": [], "truncated": true }"#.into()),
		)
		.await;
		let err = build_commit(
			&client(),
			&Api::at(&server.uri()),
			"dev/Game",
			BUILD_SHA,
			None,
			&games,
			&Budget::default(),
			|_, _| {},
		)
		.await
		.unwrap_err();
		assert!(err.contains("cannot be proved"), "{err}");
		assert!(!games.join("github-dev-Game-8379d65aaaa0.sdd").exists());
		assert!(!staging_left(&games));
	}

	#[tokio::test]
	async fn a_file_the_tree_does_not_name_is_dropped() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		std::fs::create_dir(&games).unwrap();
		let server = MockServer::start().await;
		serve(
			&server,
			format!("/repos/dev/Game/zipball/{BUILD_SHA}"),
			ResponseTemplate::new(200).set_body_bytes(zipball(
				"dev-Game-8379d65",
				&[
					(
						"modinfo.lua",
						b"name = 'Game'
",
					),
					(
						"units/a.lua",
						b"return 1
",
					),
					(
						"extra.lua",
						b"return 'not committed'
",
					),
					(
						"units/deeper/extra2.lua",
						b"return 2
",
					),
				],
			)),
		)
		.await;
		serve(
			&server,
			format!("/repos/dev/Game/git/trees/{BUILD_SHA}"),
			ok(tree(
				&[
					(
						"modinfo.lua",
						b"name = 'Game'
",
					),
					(
						"units/a.lua",
						b"return 1
",
					),
				],
				&[],
			)),
		)
		.await;
		let built = build_commit(
			&client(),
			&Api::at(&server.uri()),
			"dev/Game",
			BUILD_SHA,
			None,
			&games,
			&Budget::default(),
			|_, _| {},
		)
		.await
		.unwrap();
		assert!(built.join("modinfo.lua").is_file());
		assert!(built.join("units").join("a.lua").is_file());
		assert!(
			!built.join("extra.lua").exists(),
			"an archive extra survived"
		);
		assert!(
			!built.join("units").join("deeper").exists(),
			"an emptied directory survived"
		);
	}

	#[test]
	fn two_paths_one_name_on_windows_refuse_the_build() {
		let err = listed(&entries(&["Units/a.lua", "units/A.lua"])).unwrap_err();
		assert!(err.contains("one file on Windows"), "{err}");
		assert!(listed(&entries(&["units/a.lua", "units/b.lua"])).is_ok());
	}

	#[test]
	fn a_path_that_climbs_or_is_a_device_is_refused() {
		for bad in [
			"../x",
			"a/./b",
			"a//b",
			"COM1.lua",
			"units/",
			"/units/a.lua",
		] {
			assert!(listed(&entries(&[bad])).is_err(), "{bad}");
		}
	}

	// ---------------------------------------------------- stopping a build

	/// A build dropped mid-unpack -- the download stopped -- stops the unpack
	/// at its next entry rather than filling the disk in the background.
	#[test]
	fn an_unpack_stops_when_its_build_is_dropped() {
		let dir = tempfile::tempdir().unwrap();
		let zip = dir.path().join("many.zip");
		let files: Vec<(String, Vec<u8>)> = (0..20)
			.map(|i| {
				(
					format!("f{i}.lua"),
					b"return 1
"
					.to_vec(),
				)
			})
			.collect();
		let refs: Vec<(&str, &[u8])> = files
			.iter()
			.map(|(name, body)| (name.as_str(), body.as_slice()))
			.collect();
		std::fs::write(&zip, zipball("dev-Game-8379d65", &refs)).unwrap();
		let into = dir.path().join("into");
		let stop = std::sync::atomic::AtomicBool::new(true);
		let named: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
		let err = unpack(
			&zip,
			&into,
			&listed(&entries(&named)).unwrap(),
			&Budget::default(),
			&mut Spent::default(),
			&stop,
		)
		.unwrap_err();
		assert!(err.contains("stopped"), "{err}");
		let written = std::fs::read_dir(&into)
			.map(|entries| entries.count())
			.unwrap_or(0);
		assert!(written <= 1, "{written} files written after the stop");
	}

	#[test]
	fn a_dropped_guard_sets_its_flag() {
		let guard = Dropped::new();
		let flag = guard.flag();
		assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
		drop(guard);
		assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
	}

	/// `Units/` and `units/` are one directory on Windows; both files are the
	/// tree's and stay, whichever spelling the directory ended up with.
	#[test]
	fn mixed_case_directories_survive_keep_only() {
		let dir = tempfile::tempdir().unwrap();
		let into = dir.path().join("into");
		for rel in ["Units/a.lua", "units/b.lua"] {
			let path = into.join(rel);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(
				path,
				b"return 1
",
			)
			.unwrap();
		}
		let listed = listed(&entries(&["Units/a.lua", "units/b.lua"])).unwrap();
		assert_eq!(keep_only(&into, &listed).unwrap(), 0);
		assert!(into.join("Units").join("a.lua").is_file());
		assert!(into.join("units").join("b.lua").is_file());
	}

	/// The archive is unpacked against the tree: a file the tree does not
	/// name is never written, and a directory entry makes nothing -- a
	/// thousand of them cost no inode.
	#[test]
	fn an_unpack_writes_only_what_the_tree_names() {
		let dir = tempfile::tempdir().unwrap();
		let zip = dir.path().join("extras.zip");
		let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
		let stored = zip::write::SimpleFileOptions::default();
		writer.add_directory("top/", stored).unwrap();
		for at in 0..50 {
			writer
				.add_directory(format!("top/bomb{at}/"), stored)
				.unwrap();
		}
		writer.start_file("top/a.lua", stored).unwrap();
		writer.write_all(b"return 1\n").unwrap();
		writer.start_file("top/extra.lua", stored).unwrap();
		writer.write_all(b"return 2\n").unwrap();
		std::fs::write(&zip, writer.finish().unwrap().into_inner()).unwrap();
		let into = dir.path().join("into");
		let listed = listed(&entries(&["a.lua"])).unwrap();
		let mut spent = Spent::default();
		unpack(
			&zip,
			&into,
			&listed,
			&Budget::default(),
			&mut spent,
			&AtomicBool::new(false),
		)
		.unwrap();
		assert!(into.join("a.lua").is_file());
		assert!(
			!into.join("extra.lua").exists(),
			"an archive file the tree does not name was written"
		);
		assert_eq!(
			std::fs::read_dir(&into).unwrap().count(),
			1,
			"an archive directory entry was made"
		);
	}

	/// A build is held to its tree on the disk too: fewer files than the
	/// tree names means two of its names were one file here.
	#[test]
	fn a_build_short_of_its_tree_is_refused() {
		let dir = tempfile::tempdir().unwrap();
		let into = dir.path().join("into");
		std::fs::create_dir_all(into.join("lib").join("core")).unwrap();
		std::fs::write(into.join("a.lua"), b"return 1\n").unwrap();
		let mut listed = listed(&entries(&["a.lua", "b.lua"])).unwrap();
		listed.pins.insert("lib/core".into(), "0".repeat(40));
		let err = matches_tree(&into, &listed).unwrap_err();
		assert!(err.contains("1 file") && err.contains("2"), "{err}");
		std::fs::write(into.join("b.lua"), b"return 2\n").unwrap();
		// A submodule's checkout is its own tree's, not this one's.
		std::fs::write(into.join("lib").join("core").join("c.lua"), b"return 3\n").unwrap();
		matches_tree(&into, &listed).unwrap();
	}

	#[tokio::test]
	async fn a_tree_past_the_file_budget_is_refused_before_its_download() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		std::fs::create_dir(&games).unwrap();
		let server = MockServer::start().await;
		let files: [(&str, &[u8]); 3] = [
			("modinfo.lua", b"name = 'Game'\n"),
			("units/a.lua", b"return 1\n"),
			("units/b.lua", b"return 2\n"),
		];
		serve(
			&server,
			format!("/repos/dev/Game/git/trees/{BUILD_SHA}"),
			ok(tree(&files, &[])),
		)
		.await;
		Mock::given(method("GET"))
			.and(path(format!("/repos/dev/Game/zipball/{BUILD_SHA}")))
			.respond_with(
				ResponseTemplate::new(200).set_body_bytes(zipball("dev-Game-8379d65", &files)),
			)
			.expect(0)
			.mount(&server)
			.await;
		let budget = Budget {
			files: 2,
			..Budget::default()
		};
		let err = build_commit(
			&client(),
			&Api::at(&server.uri()),
			"dev/Game",
			BUILD_SHA,
			None,
			&games,
			&budget,
			|_, _| {},
		)
		.await
		.unwrap_err();
		assert!(err.contains("files"), "{err}");
		assert!(!staging_left(&games));
	}

	#[tokio::test]
	async fn a_tree_past_the_byte_budget_is_refused_before_its_download() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		std::fs::create_dir(&games).unwrap();
		let server = MockServer::start().await;
		serve(
			&server,
			format!("/repos/dev/Game/git/trees/{BUILD_SHA}"),
			ok(format!(
				r#"{{ "tree": [{{ "path": "big.bin", "type": "blob", "sha": "{}", "size": 200000 }}], "truncated": false }}"#,
				"0".repeat(40)
			)),
		)
		.await;
		Mock::given(method("GET"))
			.and(path(format!("/repos/dev/Game/zipball/{BUILD_SHA}")))
			.respond_with(
				ResponseTemplate::new(200).set_body_bytes(zipball("dev-Game-8379d65", &[])),
			)
			.expect(0)
			.mount(&server)
			.await;
		let budget = Budget {
			bytes: 100 * 1024,
			..Budget::default()
		};
		let err = build_commit(
			&client(),
			&Api::at(&server.uri()),
			"dev/Game",
			BUILD_SHA,
			None,
			&games,
			&budget,
			|_, _| {},
		)
		.await
		.unwrap_err();
		assert!(err.contains("bytes"), "{err}");
		assert!(!staging_left(&games));
	}
}
