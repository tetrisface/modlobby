//! Getting an engine onto a machine that has none.
//!
//! Everything else modlobby fetches goes through pr-downloader, which is the
//! right tool and handles rapid, mirrors and resume. It cannot fetch an engine,
//! because it ships inside one — so this is the one download modlobby does
//! itself, and only ever the first one.
//!
//! It is done the way pr-downloader would do it: the index's checksum is
//! verified before anything is unpacked, a download that broke off is resumed
//! rather than restarted, and every mirror the index named gets its turn.
//!
//! After it, `recoil::find_downloader` has something to find and the ordinary
//! content path takes over.

use std::path::{Path, PathBuf};
use std::time::Duration;

use content::fetch::{hash_file, sweep_stale_parts};
use md5::Md5;
use serde::Serialize;
use sha2::Sha256;
use tauri::{Emitter, State};

use crate::commands::{ApiError, Result};
use crate::state::App;

/// How far along the one download this module does is.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", tag = "phase")]
#[ts(export)]
pub enum EngineProgress {
	/// Asking BAR's index where this version lives.
	Finding,
	Downloading {
		#[ts(type = "number")]
		got: u64,
		#[ts(type = "number")]
		total: u64,
	},
	/// The archive is in hand; checking and unpacking it is not interruptible
	/// and can take a while, so it is worth saying that it is happening.
	Extracting,
	Done {
		version: String,
	},
	Failed {
		reason: String,
	},
}

/// Downloads and unpacks an engine into `<data>/engine/`.
///
/// An empty `version` asks for whatever Beyond All Reason plays on today: a
/// machine with nothing on it does not know which to want, and the launcher
/// config does. On a Mac there is one build whatever is asked for.
///
/// Progress arrives on the `engine-download` event rather than as a return
/// value: it is a hundreds-of-megabytes download and a silent one would look
/// like a hang.
#[tauri::command]
pub async fn download_engine(
	app: State<'_, App>,
	window: tauri::Window,
	version: String,
) -> Result<String> {
	let dirs = crate::commands::data_dirs(&app)?;
	let say = |progress: EngineProgress| {
		let _ = window.emit("engine-download", progress);
	};

	// One engine download at a time: a room asking twice, or two views asking
	// for the same version, would otherwise write the same staging file. The
	// second caller finds the engine installed and returns at once.
	let _one_at_a_time = app.engine_downloads.lock().await;
	match fetch(&app.http, &dirs, &version, &say).await {
		Ok((path, version)) => {
			say(EngineProgress::Done { version });
			// A room waiting on this engine can now fetch its game and map --
			// and a room that named no engine now has one to name.
			let _ = app.client.recheck_content().await;
			Ok(path.to_string_lossy().into_owned())
		}
		Err(err) => {
			say(EngineProgress::Failed {
				reason: err.message.clone(),
			});
			Err(err)
		}
	}
}

/// How long a staging file nobody is writing to is kept for resuming. A
/// download broken off last week is worth finishing; one from last month is
/// more likely an engine nobody wants any more.
const STALE_PART_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The engine's binary directory, and the version it turned out to be.
async fn fetch(
	http: &reqwest::Client,
	dirs: &content::DataDirs,
	version: &str,
	say: &impl Fn(EngineProgress),
) -> Result<(PathBuf, String)> {
	let version = version.trim();
	// Already there, ours or another lobby's: not an error, and not a reason
	// to download it again.
	let library = content::Library::new(dirs.clone());
	if !version.is_empty()
		&& let Some(found) = library.find_engine(version)
	{
		return Ok((found.bin, version.to_owned()));
	}
	let data_dir = library.write_dir().to_path_buf();
	let engine_dir = data_dir.join("engine");
	std::fs::create_dir_all(&engine_dir)
		.map_err(|err| ApiError::new("io", format!("making the engine directory: {err}")))?;
	sweep_stale_parts(&engine_dir, STALE_PART_AFTER);

	say(EngineProgress::Finding);
	// Which build, where it goes, and what it will be called -- which a bundle
	// only says once unpacked, so `None` until then.
	let (version, release, target) = if content::release::one_engine() {
		// The one build there is. Having it is the answer to any version, and
		// a room that wants another cannot be given one.
		if let Some(installed) = library.installed_engines().into_iter().next() {
			if version.is_empty() || version == installed {
				let found = library.find_engine(&installed).map(|engine| engine.bin);
				return found.map(|bin| (bin, installed)).ok_or_else(|| {
					ApiError::new(
						"archive",
						"an installed engine went missing between two looks",
					)
				});
			}
			return Err(ApiError::new(
				"notFound",
				format!(
					"the Apple Silicon build here is engine {installed}, and there is no other; \
					 this room wants {version}"
				),
			));
		}
		let (tag, release) = apple_release(http).await?;
		(None, release, engine_dir.join(tag))
	} else {
		let version = if version.is_empty() {
			newest_version(http).await?
		} else {
			version.to_owned()
		};
		if let Some(found) = library.find_engine(&version) {
			return Ok((found.bin, version));
		}
		let release = index_release(http, &version).await?;
		let target = engine_dir.join(&version);
		(Some(version), release, target)
	};

	// Into a staging file, because the extractor wants a path and a
	// half-written archive under `engine/` would look like an install. Dotted
	// and suffixed so nothing scanning for engines mistakes it for one.
	let staging = engine_dir.join(format!(".{}.part", release.filename));

	// A transport failure leaves the staging file for the next attempt to
	// resume; a checksum failure has already removed it.
	download(http, &release, &staging, say).await?;

	say(EngineProgress::Extracting);
	let unpacked = tokio::task::spawn_blocking({
		let staging = staging.clone();
		let target = target.clone();
		move || unpack(&staging, &target)
	})
	.await
	.map_err(|err| ApiError::new("io", format!("unpacking: {err}")))?;

	let _ = std::fs::remove_file(&staging);
	unpacked.map_err(|err| {
		// A half-unpacked directory is worse than none: it would satisfy a
		// "which engines are installed" scan and then fail to launch.
		let _ = std::fs::remove_dir_all(&target);
		ApiError::new("archive", format!("unpacking the engine: {err}"))
	})?;
	recoil::mark_executable(&target)
		.map_err(|err| ApiError::new("io", format!("marking the engine executable: {err}")))?;

	let version = version
		.or_else(|| recoil::EngineLayout::at(&target)?.declared_version())
		.ok_or_else(|| ApiError::new("archive", "the bundle does not say which engine it is"))?;
	recoil::find_engine(&data_dir, &version)
		.map(|engine| (engine.bin, version))
		.ok_or_else(|| {
			ApiError::new(
				"archive",
				format!(
					"the archive unpacked but holds no {}",
					recoil::ENGINE_BINARY
				),
			)
		})
}

/// The engine Beyond All Reason plays on today, from its launcher config.
async fn newest_version(http: &reqwest::Client) -> Result<String> {
	let config = text(
		http,
		content::release::LAUNCHER_CONFIG_URL,
		"asking BAR which engine it plays on",
	)
	.await?;
	content::release::newest_version(&config).ok_or_else(|| {
		ApiError::new(
			"notFound",
			"BAR's launcher config names no engine for this machine",
		)
	})
}

/// The build of `version` for this machine, from BAR's file index.
async fn index_release(http: &reqwest::Client, version: &str) -> Result<content::release::Release> {
	let index = text(
		http,
		&content::release::find_url(version),
		"asking BAR's file index",
	)
	.await?;
	content::release::pick(&index).ok_or_else(|| {
		ApiError::new(
			"notFound",
			format!(
				"BAR's index has no {} build of engine {version}",
				content::release::category()
			),
		)
	})
}

/// The latest Apple Silicon build: its release tag and its zip.
async fn apple_release(http: &reqwest::Client) -> Result<(String, content::release::Release)> {
	let body = text(
		http,
		content::release::APPLE_LATEST_URL,
		"asking GitHub for the Apple Silicon build",
	)
	.await?;
	content::release::pick_apple(&body).ok_or_else(|| {
		ApiError::new(
			"notFound",
			"the Apple Silicon build's latest release has no zip",
		)
	})
}

/// One small answer, read whole; `doing` says what was being asked in the error.
async fn text(http: &reqwest::Client, url: &str, doing: &str) -> Result<String> {
	http.get(url)
		.send()
		.await
		.and_then(reqwest::Response::error_for_status)
		.map_err(|err| ApiError::new("network", format!("{doing}: {err}")))?
		.text()
		.await
		.map_err(|err| ApiError::new("network", format!("{doing}: reading the answer: {err}")))
}

/// Unpacks an engine archive into `target`, by what it is: BAR's index serves
/// 7z, GitHub's release is a zip.
fn unpack(archive: &Path, target: &Path) -> std::result::Result<(), String> {
	let zipped = archive
		.file_name()
		.and_then(|name| name.to_str())
		.is_some_and(|name| name.ends_with(".zip.part"));
	if zipped {
		unzip(archive, target).map_err(|err| err.to_string())
	} else {
		sevenz_rust2::decompress_file(archive, target).map_err(|err| err.to_string())
	}
}

/// Unpacks a zip into `target`, keeping the executable bits and symlinks the
/// archive carries -- an app bundle is nothing without either -- and leaving
/// out the `__MACOSX/` resource forks a Mac's zip adds beside every file.
fn unzip(archive: &Path, target: &Path) -> std::io::Result<()> {
	let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
	for index in 0..zip.len() {
		let mut member = zip.by_index(index)?;
		// `enclosed_name` refuses a member that would land outside `target`.
		let Some(relative) = member.enclosed_name() else {
			continue;
		};
		if relative.starts_with("__MACOSX") {
			continue;
		}
		let path = target.join(relative);
		if member.is_dir() {
			std::fs::create_dir_all(&path)?;
			continue;
		}
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}
		#[cfg(unix)]
		if member.is_symlink() {
			let mut to = String::new();
			std::io::Read::read_to_string(&mut member, &mut to)?;
			std::os::unix::fs::symlink(to, &path)?;
			continue;
		}
		let mut file = std::fs::File::create(&path)?;
		std::io::copy(&mut member, &mut file)?;
		#[cfg(unix)]
		if let Some(mode) = member.unix_mode() {
			use std::os::unix::fs::PermissionsExt;
			std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
		}
	}
	Ok(())
}

/// Tries every mirror the index named, in its order, and verifies what arrived.
async fn download(
	http: &reqwest::Client,
	release: &content::release::Release,
	into: &Path,
	say: &impl Fn(EngineProgress),
) -> Result<()> {
	let mut last = None;
	for mirror in &release.mirrors {
		match download_from(http, mirror, into, release.size, say).await {
			Ok(()) => return verify(into, release).await,
			Err(err) => {
				tracing::warn!(mirror, reason = %err.message, "engine mirror failed");
				last = Some(err);
			}
		}
	}
	Err(last.unwrap_or_else(|| ApiError::new("notFound", "the index named no mirror")))
}

/// Streams the archive to `into`, picking up where a previous attempt left
/// off (`content::fetch::resumable`).
async fn download_from(
	http: &reqwest::Client,
	url: &str,
	into: &Path,
	expected: u64,
	say: &impl Fn(EngineProgress),
) -> Result<()> {
	content::fetch::resumable(http, url, into, expected, |got, total| {
		say(EngineProgress::Downloading { got, total });
	})
	.await
	.map_err(|err| match err {
		content::fetch::Error::Network(reason) => ApiError::new("network", reason),
		content::fetch::Error::Io(reason) => ApiError::new("io", reason),
		content::fetch::Error::Status(status) => {
			ApiError::new("network", format!("the mirror answered {status}"))
		}
	})
}

/// Checks the archive against the checksum its source gave, when it gave one:
/// BAR's index an md5, GitHub a sha256.
///
/// A mismatch discards the file: a resumed download that went wrong, or a
/// mirror serving something else, and either way not something to unpack and
/// then find out about at launch.
async fn verify(path: &Path, release: &content::release::Release) -> Result<()> {
	let (expected, sha) = match (&release.sha256, &release.md5) {
		(Some(sha256), _) => (sha256.as_str(), true),
		(None, Some(md5)) => (md5.as_str(), false),
		(None, None) => {
			tracing::info!("the index gave no checksum; unpacking unverified");
			return Ok(());
		}
	};
	let actual = tokio::task::spawn_blocking({
		let path = path.to_path_buf();
		move || {
			if sha {
				hash_file::<Sha256>(&path)
			} else {
				hash_file::<Md5>(&path)
			}
		}
	})
	.await
	.map_err(|err| ApiError::new("io", format!("checking the archive: {err}")))?
	.map_err(|err| ApiError::new("io", format!("reading the archive back: {err}")))?;

	if actual.eq_ignore_ascii_case(expected) {
		tracing::info!(checksum = actual, "engine archive verified");
		return Ok(());
	}
	let _ = std::fs::remove_file(path);
	Err(ApiError::new(
		"archive",
		format!(
			"the archive's checksum ({actual}) is not the source's ({expected}); it was discarded"
		),
	))
}

#[cfg(test)]
mod tests {
	use super::*;
	use content::fetch::hex;
	use content::release::Release;
	use md5::Digest;
	use wiremock::matchers::{header, method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	fn md5_of(bytes: &[u8]) -> String {
		hex(&Md5::digest(bytes))
	}

	fn release(mirrors: Vec<String>, md5: Option<&str>) -> Release {
		Release {
			filename: "x.7z".into(),
			mirrors,
			size: 6,
			md5: md5.map(str::to_owned),
			sha256: None,
		}
	}

	fn quiet(_: EngineProgress) {}

	/// The shape GitHub's zip of the Apple Silicon build has: an app bundle,
	/// whose binaries are only binaries with their mode, beside a `__MACOSX/`
	/// tree of resource forks that would be junk on the disk.
	#[test]
	fn a_zip_unpacks_without_its_resource_forks_and_keeps_the_executable_bit() {
		use std::io::Write;
		use zip::write::SimpleFileOptions;
		let dir = tempfile::tempdir().unwrap();
		let archive = dir.path().join(".x.zip.part");
		let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
		let plain = SimpleFileOptions::default();
		zip.add_directory("BAR Launcher.app/Contents/MacOS/", plain)
			.unwrap();
		zip.start_file(
			"BAR Launcher.app/Contents/MacOS/spring",
			plain.unix_permissions(0o755),
		)
		.unwrap();
		zip.write_all(b"engine").unwrap();
		zip.start_file("__MACOSX/BAR Launcher.app/._Contents", plain)
			.unwrap();
		zip.write_all(b"fork").unwrap();
		zip.finish().unwrap();

		let target = dir.path().join("v0.15.1");
		unpack(&archive, &target).unwrap();
		let spring = target.join("BAR Launcher.app/Contents/MacOS/spring");
		assert_eq!(std::fs::read(&spring).unwrap(), b"engine");
		assert!(!target.join("__MACOSX").exists());
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			let mode = std::fs::metadata(&spring).unwrap().permissions().mode();
			assert_eq!(
				mode & 0o111,
				0o111,
				"an engine that cannot be run is no engine"
			);
		}
	}

	#[tokio::test]
	async fn a_github_sha256_is_checked_the_way_the_indexs_md5_is() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(200).set_body_bytes(b"abcdef".to_vec()))
			.mount(&server)
			.await;
		let dir = tempfile::tempdir().unwrap();
		let staging = dir.path().join(".x.zip.part");
		let mut release = release(vec![format!("{}/x.zip", server.uri())], None);
		release.sha256 = Some(hex(&Sha256::digest(b"abcdef")));
		download(&content::http::client("test"), &release, &staging, &quiet)
			.await
			.unwrap();
		assert!(staging.exists());

		release.sha256 = Some(hex(&Sha256::digest(b"something else")));
		let err = download(&content::http::client("test"), &release, &staging, &quiet)
			.await
			.unwrap_err();
		assert_eq!(err.code, "archive");
		assert!(!staging.exists());
	}

	#[tokio::test]
	async fn a_broken_off_download_is_resumed_and_then_verified() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(path("/x.7z"))
			.and(header("Range", "bytes=3-"))
			.respond_with(ResponseTemplate::new(206).set_body_bytes(b"def".to_vec()))
			.expect(1)
			.mount(&server)
			.await;
		let dir = tempfile::tempdir().unwrap();
		let staging = dir.path().join(".x.7z.part");
		std::fs::write(&staging, b"abc").unwrap();

		let release = release(
			vec![format!("{}/x.7z", server.uri())],
			Some(&md5_of(b"abcdef")),
		);
		download(&content::http::client("test"), &release, &staging, &quiet)
			.await
			.unwrap();
		assert_eq!(std::fs::read(&staging).unwrap(), b"abcdef");
	}

	#[tokio::test]
	async fn a_mirror_that_ignores_the_range_starts_over() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(200).set_body_bytes(b"abcdef".to_vec()))
			.mount(&server)
			.await;
		let dir = tempfile::tempdir().unwrap();
		let staging = dir.path().join(".x.7z.part");
		std::fs::write(&staging, b"abc").unwrap();

		let release = release(
			vec![format!("{}/x.7z", server.uri())],
			Some(&md5_of(b"abcdef")),
		);
		download(&content::http::client("test"), &release, &staging, &quiet)
			.await
			.unwrap();
		assert_eq!(std::fs::read(&staging).unwrap(), b"abcdef");
	}

	#[tokio::test]
	async fn a_checksum_mismatch_discards_the_archive() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(200).set_body_bytes(b"abcdef".to_vec()))
			.mount(&server)
			.await;
		let dir = tempfile::tempdir().unwrap();
		let staging = dir.path().join(".x.7z.part");

		let release = release(
			vec![format!("{}/x.7z", server.uri())],
			Some(&md5_of(b"something else")),
		);
		let err = download(&content::http::client("test"), &release, &staging, &quiet)
			.await
			.unwrap_err();
		assert_eq!(err.code, "archive");
		assert!(!staging.exists(), "nothing downstream may find it");
	}

	#[tokio::test]
	async fn the_next_mirror_is_tried_when_the_first_fails() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(path("/down/x.7z"))
			.respond_with(ResponseTemplate::new(503))
			.expect(1)
			.mount(&server)
			.await;
		Mock::given(method("GET"))
			.and(path("/up/x.7z"))
			.respond_with(ResponseTemplate::new(200).set_body_bytes(b"abcdef".to_vec()))
			.expect(1)
			.mount(&server)
			.await;
		let dir = tempfile::tempdir().unwrap();
		let staging = dir.path().join(".x.7z.part");

		let release = release(
			vec![
				format!("{}/down/x.7z", server.uri()),
				format!("{}/up/x.7z", server.uri()),
			],
			Some(&md5_of(b"abcdef")),
		);
		download(&content::http::client("test"), &release, &staging, &quiet)
			.await
			.unwrap();
		assert_eq!(std::fs::read(&staging).unwrap(), b"abcdef");
	}

	#[tokio::test]
	async fn every_mirror_failing_is_the_last_ones_error() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(404))
			.expect(2)
			.mount(&server)
			.await;
		let dir = tempfile::tempdir().unwrap();
		let staging = dir.path().join(".x.7z.part");
		let release = release(
			vec![
				format!("{}/a/x.7z", server.uri()),
				format!("{}/b/x.7z", server.uri()),
			],
			None,
		);
		let err = download(&content::http::client("test"), &release, &staging, &quiet)
			.await
			.unwrap_err();
		assert_eq!(err.code, "network");
		assert!(err.message.contains("404"));
	}

	#[test]
	fn only_old_staging_files_are_swept() {
		let dir = tempfile::tempdir().unwrap();
		let old = dir.path().join(".old.7z.part");
		let recent = dir.path().join(".recent.7z.part");
		let engine = dir.path().join(".hidden-but-not-a-part");
		for path in [&old, &recent, &engine] {
			std::fs::write(path, b"x").unwrap();
		}
		let long_ago = std::time::SystemTime::now() - STALE_PART_AFTER * 2;
		for path in [&old, &engine] {
			// Writable, because Windows will not date a file opened read-only.
			std::fs::OpenOptions::new()
				.write(true)
				.open(path)
				.unwrap()
				.set_modified(long_ago)
				.unwrap();
		}

		sweep_stale_parts(dir.path(), STALE_PART_AFTER);

		assert!(!old.exists());
		assert!(recent.exists(), "a recent one is resumed, not removed");
		assert!(engine.exists(), "only `.…part` files are ours to remove");
	}
}
