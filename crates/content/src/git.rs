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

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use regex::Regex;
use sha1::{Digest as _, Sha1};

use crate::api::Api;

/// The most a build unpacks to, all its files together: a repository is its
/// author's to fill and a list's entry anyone's to write, and the largest
/// real game is a fraction of this.
const MOST_UNPACKED: u64 = 8 << 30;

/// The most one file whose line ends are put back may be, since it is held
/// whole to do that: those are text, and text this large is not.
const MOST_TEXT: u64 = 256 << 20;

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

/// Unpacks a GitHub archive into `into`: its one top folder dropped, and
/// every file its `.gitattributes` had written with CRLF put back as
/// committed. Nothing outside `into` is ever written, and no more than
/// [`MOST_UNPACKED`] in all.
pub fn unpack(zipball: &Path, into: &Path) -> Result<(), String> {
	let file = std::fs::File::open(zipball).map_err(|err| err.to_string())?;
	let mut zip = zip::ZipArchive::new(file).map_err(|err| err.to_string())?;
	let names: Vec<String> = zip.file_names().map(str::to_owned).collect();
	let top = names
		.first()
		.and_then(|name| name.split('/').next())
		.map(|top| format!("{top}/"))
		.unwrap_or_default();
	let attributes = names
		.iter()
		.find(|name| name.strip_prefix(&top) == Some(".gitattributes"))
		.and_then(|name| {
			let mut text = String::new();
			zip.by_name(name).ok()?.read_to_string(&mut text).ok()?;
			Some(Attributes::parse(&text))
		})
		.unwrap_or(Attributes { rules: Vec::new() });
	let mut left = MOST_UNPACKED;
	for at in 0..zip.len() {
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
		let path = into.join(relative);
		if entry.is_dir() {
			std::fs::create_dir_all(&path).map_err(|err| err.to_string())?;
			continue;
		}
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
		}
		let as_git = relative.to_string_lossy().replace('\\', "/");
		let crlf = attributes.crlf(&as_git);
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
				"{} unpacks to more than a game would ({} MiB)",
				entry.name(),
				cap >> 20
			));
		}
		left -= written;
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

/// The commit `reference` names in `repo`: a branch, a tag, `HEAD` for the
/// default branch, or a commit hash as short as GitHub can tell apart.
pub async fn commit_of(
	http: &reqwest::Client,
	api: &Api,
	repo: &str,
	reference: &str,
) -> Result<Commit, String> {
	let plain = !reference.is_empty()
		&& !reference.contains("..")
		&& reference
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'));
	if !plain {
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
pub async fn build_commit(
	http: &reqwest::Client,
	api: &Api,
	repo: &str,
	sha: &str,
	stamp: Option<(&str, &str)>,
	games: &Path,
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

	let mut pending = vec![(repo.to_owned(), sha.to_owned(), staging.clone())];
	while let Some((at_repo, sha, into)) = pending.pop() {
		let pinned = checkout(http, api, &at_repo, &sha, &into, games, &mut report)
			.await
			.map_err(|err| failed(format!("{at_repo} at {}: {err}", &sha[..sha.len().min(7)])))?;
		for (at, sub) in submodules(&into, &at_repo) {
			let Some(sub_sha) = pinned.get(&at) else {
				return Err(failed(format!(
					"{at_repo} names a submodule at {at} and pins nothing there"
				)));
			};
			pending.push((sub, sub_sha.clone(), into.join(&at)));
		}
	}

	if let Some((placeholder, version)) = stamp {
		write_version(&staging, placeholder, version).map_err(failed)?;
	}
	let built = games.join(&name);
	let _ = std::fs::remove_dir_all(&built);
	std::fs::rename(&staging, &built).map_err(|err| failed(err.to_string()))?;
	Ok(built)
}

/// One repository's commit into `into`: GitHub's archive of it unpacked,
/// then every file proved against the blob the commit names. What `git
/// archive` left out (`export-ignore`) or wrote otherwise (`export-subst`,
/// line ends the root `.gitattributes` does not explain) is fetched as
/// committed. The submodules it pins, by path.
///
/// ponytail: files fetched one at a time; a repository whose archive differs
/// from its tree in thousands of files builds slowly.
async fn checkout(
	http: &reqwest::Client,
	api: &Api,
	repo: &str,
	sha: &str,
	into: &Path,
	games: &Path,
	report: &mut impl FnMut(u64, u64),
) -> Result<HashMap<String, String>, String> {
	let zipball = games.join(format!(".{}-{sha}.zip.part", repo.replace('/', "-")));
	let url = format!("{}/repos/{repo}/zipball/{sha}", api.github);
	crate::fetch::resumable(http, &url, &zipball, 0, &mut *report)
		.await
		.map_err(|err| err.to_string())?;
	let unpacked = unpack(&zipball, into);
	let _ = std::fs::remove_file(&zipball);
	unpacked?;

	let tree: Tree = api
		.json(
			http,
			&format!("{}/repos/{repo}/git/trees/{sha}?recursive=1", api.github),
			"the tree",
		)
		.await?;
	if tree.truncated {
		tracing::warn!(
			repo,
			sha,
			"the tree is too large to list whole; its files are not proved"
		);
	}
	let mut pinned = HashMap::new();
	for entry in tree.tree {
		match entry.kind.as_str() {
			"commit" => {
				pinned.insert(entry.path, entry.sha);
			}
			"blob" => prove(http, api, repo, sha, into, &entry).await?,
			_ => {}
		}
	}
	Ok(pinned)
}

/// A file as committed: kept when its blob hash is the tree's, else fetched
/// from where GitHub serves it raw, and held to the tree there too.
async fn prove(
	http: &reqwest::Client,
	api: &Api,
	repo: &str,
	sha: &str,
	into: &Path,
	entry: &TreeEntry,
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
		assert!(unpack(&zip, &dir.path().join("into")).is_err());
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
}
