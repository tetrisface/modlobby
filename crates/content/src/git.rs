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

use std::io::Read;
use std::path::{Path, PathBuf};

use regex::Regex;

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

/// A submodule a build needs: where it goes, and the GitHub repository it is.
fn submodules(build: &Path) -> Vec<(String, String)> {
	let Ok(text) = std::fs::read_to_string(build.join(".gitmodules")) else {
		return Vec::new();
	};
	let mut found = Vec::new();
	let mut path = None;
	for line in text.lines().map(str::trim) {
		if line.starts_with('[') {
			path = None;
		} else if let Some(value) = line
			.strip_prefix("path")
			.and_then(|rest| rest.trim_start().strip_prefix('='))
		{
			path = Some(value.trim().to_owned());
		} else if let Some(value) = line
			.strip_prefix("url")
			.and_then(|rest| rest.trim_start().strip_prefix('='))
			&& let (Some(at), Some(repo)) = (path.clone(), github_repo(value.trim()))
			&& plain_relative(&at)
		{
			found.push((at, repo));
		}
	}
	found
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

/// `owner/repo` out of a GitHub clone address; `None` for anywhere else.
fn github_repo(url: &str) -> Option<String> {
	let rest = url
		.strip_prefix("https://github.com/")
		.or_else(|| url.strip_prefix("git@github.com:"))?;
	let repo = rest.trim_end_matches('/').trim_end_matches(".git");
	(repo.matches('/').count() == 1).then(|| repo.to_owned())
}

#[derive(serde::Deserialize)]
struct Commit {
	sha: String,
}

#[derive(serde::Deserialize)]
struct Content {
	sha: String,
}

async fn github_json<T: serde::de::DeserializeOwned>(
	http: &reqwest::Client,
	url: &str,
) -> Result<T, String> {
	let response = http
		.get(url)
		.header(reqwest::header::ACCEPT, "application/vnd.github+json")
		.send()
		.await
		.map_err(|err| err.to_string())?;
	if !response.status().is_success() {
		return Err(refused(url, &response));
	}
	response.json().await.map_err(|err| err.to_string())
}

/// Why GitHub's API said no, in words. Its allowance for an address without
/// an account -- 60 requests an hour -- is the usual reason, and its answer
/// says when that comes back.
pub(crate) fn refused(what: &str, response: &reqwest::Response) -> String {
	let header = |name: &str| {
		response
			.headers()
			.get(name)
			.and_then(|value| value.to_str().ok())
	};
	let status = response.status();
	let spent =
		matches!(status.as_u16(), 403 | 429) && header("x-ratelimit-remaining") == Some("0");
	if !spent {
		return format!("{what} answered {status}");
	}
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |since| since.as_secs());
	let wait = header("x-ratelimit-reset")
		.and_then(|at| at.parse::<u64>().ok())
		.map(|at| at.saturating_sub(now).div_ceil(60).max(1))
		.map(|minutes| format!(" for another {minutes} min"))
		.unwrap_or_default();
	format!("GitHub's hourly allowance of requests for this address is used up{wait}")
}

/// Builds `repo`'s commit that `version` names into a `.sdd` under `games`:
/// the commit's tree as committed, its submodules at the commits it pins,
/// and `version` written over `placeholder`. Where the build is.
///
/// ponytail: submodules one level deep; a submodule's own submodules are
/// left out, which the room's hash will say if it matters.
pub async fn build(
	http: &reqwest::Client,
	api: &str,
	repo: &str,
	version: &str,
	placeholder: &str,
	games: &Path,
	mut report: impl FnMut(u64, u64),
) -> Result<PathBuf, String> {
	let failed = |reason: String| format!("git {repo}: {reason}");
	crate::sources::releases_url(api, repo)
		.ok_or_else(|| failed("not a repository's name".into()))?;
	let hash = short_hash(version).ok_or_else(|| failed(format!("{version} names no commit")))?;
	let commit: Commit = github_json(http, &format!("{api}/repos/{repo}/commits/{hash}"))
		.await
		.map_err(failed)?;
	let short = &commit.sha[..commit.sha.len().min(12)];
	let name = format!("{}-{short}", repo.replace('/', "-"));
	let staging = games.join(format!(".{name}.sdd.part"));
	let _ = std::fs::remove_dir_all(&staging);
	std::fs::create_dir_all(&staging).map_err(|err| failed(err.to_string()))?;

	let fetch_into = |repo: String, sha: String, into: PathBuf| {
		let zipball = games.join(format!(".{}-{sha}.zip.part", repo.replace('/', "-")));
		let url = format!("{api}/repos/{repo}/zipball/{sha}");
		(url, zipball, into)
	};
	let (url, zipball, into) = fetch_into(repo.to_owned(), commit.sha.clone(), staging.clone());
	crate::fetch::resumable(http, &url, &zipball, 0, &mut report)
		.await
		.map_err(|err| failed(err.to_string()))?;
	let unpacked = unpack(&zipball, &into);
	let _ = std::fs::remove_file(&zipball);
	unpacked.map_err(failed)?;

	for (at, sub) in submodules(&staging) {
		let pinned: Content = github_json(
			http,
			&format!("{api}/repos/{repo}/contents/{at}?ref={}", commit.sha),
		)
		.await
		.map_err(|err| failed(format!("submodule {at}: {err}")))?;
		let (url, zipball, into) = fetch_into(sub.clone(), pinned.sha, staging.join(&at));
		crate::fetch::resumable(http, &url, &zipball, 0, &mut report)
			.await
			.map_err(|err| failed(format!("submodule {at}: {err}")))?;
		let unpacked = unpack(&zipball, &into);
		let _ = std::fs::remove_file(&zipball);
		unpacked.map_err(|err| failed(format!("submodule {at}: {err}")))?;
	}

	write_version(&staging, placeholder, version).map_err(failed)?;
	let built = games.join(format!("{name}.sdd"));
	let _ = std::fs::remove_dir_all(&built);
	std::fs::rename(&staging, &built).map_err(|err| failed(err.to_string()))?;
	Ok(built)
}

#[cfg(test)]
mod tests {
	use std::io::Write;

	use wiremock::matchers::{method, path, query_param};
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

	#[tokio::test]
	async fn a_commit_is_built_as_committed_with_its_submodule_and_its_version() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		std::fs::create_dir(&games).unwrap();
		let api = MockServer::start().await;
		let sha = "8379d65aaaa0000000000000000000000000000";
		let sub_sha = "24d521d0cbe7c860a53c360d545aadceaaae3f17";
		Mock::given(method("GET"))
			.and(path("/repos/dev/Game/commits/8379d65"))
			.respond_with(
				ResponseTemplate::new(200).set_body_string(format!(r#"{{ "sha": "{sha}" }}"#)),
			)
			.mount(&api)
			.await;
		Mock::given(method("GET"))
			.and(path(format!("/repos/dev/Game/zipball/{sha}")))
			.respond_with(ResponseTemplate::new(200).set_body_bytes(zipball(
				"dev-Game-8379d65",
				&[
					(".gitattributes", b"*.lua text eol=crlf\n"),
					(".gitmodules", b"[submodule \"lib\"]\n\tpath = lib\n\turl = https://github.com/dev/lib.git\n"),
					("modinfo.lua", b"name = 'Game'\r\nversion = \"$VERSION\",\r\n"),
					("units/a.lua", b"return 1\r\n"),
				],
			)))
			.mount(&api)
			.await;
		Mock::given(method("GET"))
			.and(path("/repos/dev/Game/contents/lib"))
			.and(query_param("ref", sha))
			.respond_with(
				ResponseTemplate::new(200)
					.set_body_string(format!(r#"{{ "type": "submodule", "sha": "{sub_sha}" }}"#)),
			)
			.mount(&api)
			.await;
		Mock::given(method("GET"))
			.and(path(format!("/repos/dev/lib/zipball/{sub_sha}")))
			.respond_with(
				ResponseTemplate::new(200)
					.set_body_bytes(zipball("dev-lib-24d521d", &[("init.lua", b"return {}\n")])),
			)
			.mount(&api)
			.await;

		let client = crate::http::client("test");
		let built = build(
			&client,
			&api.uri(),
			"dev/Game",
			"test-5-8379d65",
			PLACEHOLDER,
			&games,
			|_, _| {},
		)
		.await
		.unwrap();
		assert_eq!(built, games.join("dev-Game-8379d65aaaa0.sdd"));
		assert_eq!(
			std::fs::read_to_string(built.join("modinfo.lua")).unwrap(),
			"name = 'Game'\nversion = \"test-5-8379d65\",\n"
		);
		assert_eq!(
			std::fs::read(built.join("units").join("a.lua")).unwrap(),
			b"return 1\n"
		);
		assert_eq!(
			std::fs::read(built.join("lib").join("init.lua")).unwrap(),
			b"return {}\n"
		);
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

	#[tokio::test]
	async fn a_spent_allowance_is_said_as_one() {
		let server = MockServer::start().await;
		let reset = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_secs()
			+ 600;
		Mock::given(method("GET"))
			.respond_with(
				ResponseTemplate::new(403)
					.insert_header("x-ratelimit-remaining", "0")
					.insert_header("x-ratelimit-reset", reset.to_string().as_str()),
			)
			.mount(&server)
			.await;
		let client = crate::http::client("test");
		let said = github_json::<Commit>(&client, &format!("{}/repos/a/b/commits/c", server.uri()))
			.await
			.err()
			.unwrap();
		assert_eq!(
			said,
			"GitHub's hourly allowance of requests for this address is used up for another 10 min"
		);
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
			 [submodule \"d\"]\n\tpath = elsewhere\n\turl = https://gitlab.com/dev/other.git\n",
		)
		.unwrap();
		assert_eq!(
			submodules(dir.path()),
			[("lib/core".to_owned(), "dev/core".to_owned())]
		);
	}
}
