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
//! which answers with a JSON array whose first entry carries the mirrors.
//!
//! BAR publishes no macOS build. The one that exists is an Apple Silicon port
//! published on GitHub, and there is only ever one of it, so a Mac asks that
//! repository for its latest release rather than the index for a version.
//!
//! A machine with nothing on it does not know which version to want either:
//! BAR's launcher config says which engine the game currently plays on, and
//! that is what an empty version resolves to.
//!
//! This module is the part that can be decided without a network: which URL to
//! ask, and what the answer means.

use serde::Deserialize;

/// Where BAR's file index lives: the same endpoint pr-downloader is handed as
/// `PRD_HTTP_SEARCH_URL`, for the same reason.
pub const FIND_URL: &str = recoil::HTTP_SEARCH_URL;

/// Where the official launcher reads which engine and game to install. Its
/// `setups[].launch.engine` is the engine the game is currently played on.
pub const LAUNCHER_CONFIG_URL: &str = "https://launcher-config.beyondallreason.dev/config.json";

/// The only Beyond All Reason engine for Apple Silicon, whose author has
/// allowed modlobby to fetch it. GitHub's API, for the asset list and its
/// checksums; the page itself has neither.
pub const APPLE_LATEST_URL: &str =
	"https://api.github.com/repos/Vandomas/RecoilEngine-AppleSilicon/releases/latest";

/// Whether the engine here is the one Apple Silicon build, fetched whatever
/// version is asked for. Everywhere else BAR's index is asked by version.
pub const fn one_engine() -> bool {
	cfg!(target_os = "macos")
}

/// The engine build for this machine, as BAR's index categorises them.
///
/// Never asked on macOS: [`one_engine`] machines do not go to the index.
pub const fn category() -> &'static str {
	if cfg!(windows) {
		"engine_windows64"
	} else {
		"engine_linux64"
	}
}

/// The platform as the launcher config names it.
const fn launcher_platform() -> &'static str {
	if cfg!(windows) { "win32" } else { "linux" }
}

/// One entry from the index, or one release asset from GitHub. Only the fields
/// worth acting on are read; both carry more and may grow.
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
	/// What GitHub gives instead of an md5.
	#[serde(default)]
	pub sha256: Option<String>,
}

/// The query for one engine version on this platform.
pub fn find_url(version: &str) -> String {
	format!(
		"{FIND_URL}?category={}&springname={}",
		category(),
		urlencode(version.trim())
	)
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

/// The engine the game is played on today, from the launcher config: the
/// first setup for this platform, minus the `recoil_` its folder name carries.
pub fn newest_version(config: &str) -> Option<String> {
	#[derive(Deserialize)]
	struct Config {
		setups: Vec<Setup>,
	}
	#[derive(Deserialize)]
	struct Setup {
		package: Package,
		launch: Launch,
	}
	#[derive(Deserialize)]
	struct Package {
		platform: String,
	}
	#[derive(Deserialize)]
	struct Launch {
		engine: String,
	}
	let config: Config = serde_json::from_str(config).ok()?;
	config
		.setups
		.into_iter()
		.find(|setup| setup.package.platform == launcher_platform())
		.map(|setup| {
			let engine = setup.launch.engine;
			engine
				.strip_prefix("recoil_")
				.map_or(engine.clone(), str::to_owned)
		})
}

/// The Apple Silicon build from a GitHub release: its tag, and the zip asset
/// as a [`Release`]. The dmg beside it is for people; the zip unpacks.
pub fn pick_apple(body: &str) -> Option<(String, Release)> {
	#[derive(Deserialize)]
	struct GithubRelease {
		tag_name: String,
		assets: Vec<Asset>,
	}
	#[derive(Deserialize)]
	struct Asset {
		name: String,
		browser_download_url: String,
		#[serde(default)]
		size: u64,
		/// `sha256:<hex>`, on every asset GitHub has hashed.
		#[serde(default)]
		digest: Option<String>,
	}
	let release: GithubRelease = serde_json::from_str(body).ok()?;
	let asset = release
		.assets
		.into_iter()
		.find(|asset| asset.name.ends_with(".zip"))?;
	Some((
		release.tag_name,
		Release {
			filename: asset.name,
			mirrors: vec![asset.browser_download_url],
			size: asset.size,
			md5: None,
			sha256: asset
				.digest
				.and_then(|digest| digest.strip_prefix("sha256:").map(str::to_owned)),
		},
	))
}

/// Percent-encodes everything that is not unreserved, which is all a query
/// value needs and avoids a URL-building dependency for one string.
fn urlencode(value: &str) -> String {
	value
		.bytes()
		.map(|byte| match byte {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
				char::from(byte).to_string()
			}
			other => format!("%{other:02X}"),
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_query_names_this_platform_and_the_version() {
		let url = find_url("2026.07.04");
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
		let url = find_url("2026.07.04 rc/1");
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

	/// The launcher config as it is served, trimmed to what is read: a test
	/// engine setup sits beside the real one, and the other platform's comes
	/// first, so the first setup is the wrong answer twice over.
	const LAUNCHER_CONFIG: &str = r#"{"title":"BAR","setups":[
        {"package":{"id":"manual-other","platform":"nothing"},"launch":{"engine":"recoil_2000.01.01"}},
        {"package":{"id":"manual-linux","platform":"linux"},"launch":{"engine":"recoil_2026.07.04","start_args":[]}},
        {"package":{"id":"manual-win","platform":"win32"},"launch":{"engine":"recoil_2026.07.04"}},
        {"package":{"id":"manual-linux-test-engine","platform":"linux"},"launch":{"engine":"recoil_2026.07.03"}}
    ]}"#;

	#[test]
	fn the_newest_version_is_the_launchers_for_this_platform() {
		assert_eq!(
			newest_version(LAUNCHER_CONFIG).as_deref(),
			Some("2026.07.04")
		);
		assert_eq!(newest_version("{}"), None);
		assert_eq!(newest_version("not json"), None);
	}

	/// A GitHub release as the API answers, trimmed likewise: the dmg is
	/// listed first, and the digest carries its algorithm.
	const GITHUB_RELEASE: &str = r#"{"tag_name":"v0.15.1","name":"v0.15.1","assets":[
        {"name":"BAR-Launcher-v0.15.1.dmg","browser_download_url":"https://x/v0.15.1/BAR-Launcher-v0.15.1.dmg","size":77,"digest":"sha256:aa"},
        {"name":"BAR-Launcher-v0.15.1.zip","browser_download_url":"https://x/v0.15.1/BAR-Launcher-v0.15.1.zip","size":75353432,"digest":"sha256:14a856fa"}
    ]}"#;

	#[test]
	fn the_apple_build_is_the_zip_with_its_sha256() {
		let (tag, release) = pick_apple(GITHUB_RELEASE).expect("a release");
		assert_eq!(tag, "v0.15.1");
		assert_eq!(release.filename, "BAR-Launcher-v0.15.1.zip");
		assert_eq!(
			release.mirrors,
			["https://x/v0.15.1/BAR-Launcher-v0.15.1.zip"]
		);
		assert_eq!(release.size, 75353432);
		assert_eq!(release.sha256.as_deref(), Some("14a856fa"));
		assert_eq!(release.md5, None);
		assert!(pick_apple(r#"{"tag_name":"v1","assets":[]}"#).is_none());
	}
}
