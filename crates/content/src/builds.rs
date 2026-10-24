//! What a build root collects and should not keep: staging left by a build
//! that never finished, games set aside because they were not the room's,
//! and builds nobody has asked for in a long while.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A `.part` -- staging a build never finished, or a zipball it never
/// unpacked -- is a build that stopped; a week is longer than any download.
pub const STALE_PART_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// A game set aside because its checksum was not the room's is kept a
/// month, in case the room comes back to it; then it is nobody's.
pub const STALE_QUARANTINE_AFTER: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// A build neither made nor launched in three months is rebuilt in a
/// minute if a lobby asks again; kept, it is disk nobody asked for.
pub const STALE_BUILD_AFTER: Duration = Duration::from_secs(90 * 24 * 60 * 60);

/// Where the builds lobbies launched are recorded, by name and the time: a
/// build in use is not stale, whenever it was made.
const USED: &str = ".used.json";

/// Records each of `builds`, by its name, as launched at `now`.
pub fn used(games: &Path, builds: &[PathBuf], now: SystemTime) -> io::Result<()> {
	let mut record = used_read(games);
	let at = now
		.duration_since(SystemTime::UNIX_EPOCH)
		.map(|since| since.as_secs())
		.unwrap_or(0);
	for name in builds.iter().filter_map(|build| build.file_name()) {
		record.insert(name.to_string_lossy().into_owned(), at);
	}
	used_write(games, &record)
}

fn used_read(games: &Path) -> BTreeMap<String, u64> {
	std::fs::read_to_string(games.join(USED))
		.ok()
		.and_then(|text| serde_json::from_str(&text).ok())
		.unwrap_or_default()
}

fn used_write(games: &Path, record: &BTreeMap<String, u64>) -> io::Result<()> {
	std::fs::create_dir_all(games)?;
	let text = serde_json::to_string_pretty(record).map_err(io::Error::other)?;
	std::fs::write(games.join(USED), text)
}

/// Removes what is stale under `games` -- the builds root's `games/` -- and
/// in `data_dir`'s quarantine, judged at `now` by each entry's modified
/// time, and for a build by when a lobby last launched it too: `.part`
/// staging and zipballs past [`STALE_PART_AFTER`], quarantined games past
/// [`STALE_QUARANTINE_AFTER`], builds (by their name's shape) past
/// [`STALE_BUILD_AFTER`]. Nothing else is touched; what cannot be removed
/// is logged and left. What went.
pub fn sweep(games: &Path, data_dir: &Path, now: SystemTime) -> Vec<PathBuf> {
	let mut gone = Vec::new();
	let record = used_read(games);
	for entry in entries(games) {
		let name = entry.file_name().to_string_lossy().into_owned();
		let is_part = name.starts_with('.') && name.ends_with(".part");
		let (window, launched) = if is_part {
			(STALE_PART_AFTER, None)
		} else if crate::git::is_build_name(&name) {
			let launched = record
				.get(&name)
				.map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(*secs));
			(STALE_BUILD_AFTER, launched)
		} else {
			continue;
		};
		if older_than(&entry.path(), launched, now, window) {
			remove(&entry.path(), &mut gone);
		}
	}
	for entry in entries(&data_dir.join(crate::sources::QUARANTINE)) {
		if older_than(&entry.path(), None, now, STALE_QUARANTINE_AFTER) {
			remove(&entry.path(), &mut gone);
		}
	}
	// The record keeps only the builds still here.
	let kept: BTreeMap<String, u64> = record
		.iter()
		.filter(|(name, _)| games.join(name).exists())
		.map(|(name, at)| (name.clone(), *at))
		.collect();
	if kept.len() != record.len()
		&& let Err(err) = used_write(games, &kept)
	{
		tracing::warn!(%err, "the record of builds in use was not pruned");
	}
	gone
}

fn entries(dir: &Path) -> Vec<std::fs::DirEntry> {
	std::fs::read_dir(dir)
		.map(|entries| entries.filter_map(Result::ok).collect())
		.unwrap_or_default()
}

/// Whether `path` was last modified -- and last `launched`, when it was --
/// more than `window` before `now`; a time that cannot be read, or lies
/// ahead of `now`, is not stale.
fn older_than(
	path: &Path,
	launched: Option<SystemTime>,
	now: SystemTime,
	window: Duration,
) -> bool {
	let modified = std::fs::metadata(path)
		.and_then(|meta| meta.modified())
		.ok();
	let latest = match (modified, launched) {
		(Some(modified), Some(launched)) => Some(modified.max(launched)),
		(modified, launched) => modified.or(launched),
	};
	latest
		.and_then(|at| now.duration_since(at).ok())
		.is_some_and(|age| age > window)
}

/// Removes `path`. A directory under a live name is renamed to a `.part`
/// name first, so a removal that fails partway -- a file held open --
/// leaves no half of it where a lobby would find it, only staging the next
/// sweep takes; what cannot be renamed is in use, and left whole.
fn remove(path: &Path, gone: &mut Vec<PathBuf>) {
	let name = path
		.file_name()
		.map(|name| name.to_string_lossy().into_owned())
		.unwrap_or_default();
	let is_part = name.starts_with('.') && name.ends_with(".part");
	let mut doomed = path.to_path_buf();
	if path.is_dir() && !is_part {
		doomed = path.with_file_name(format!(".{name}.part"));
		if let Err(err) = std::fs::rename(path, &doomed) {
			tracing::warn!(path = %path.display(), %err, "stale, but in use; left");
			return;
		}
	}
	let removed = if doomed.is_dir() {
		std::fs::remove_dir_all(&doomed)
	} else {
		std::fs::remove_file(&doomed)
	};
	match removed {
		Ok(()) => gone.push(path.to_path_buf()),
		Err(err) => tracing::warn!(path = %path.display(), %err, "stale, but could not be removed"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::{Duration, SystemTime};

	fn days(count: u64) -> Duration {
		Duration::from_secs(count * 86_400)
	}

	fn touch(path: &Path) {
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, b"").unwrap();
	}

	#[test]
	fn stale_parts_and_quarantined_games_are_swept() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		let data = dir.path().join("data");
		touch(
			&games
				.join(".github-dev-a-000000000000.sdd.part")
				.join("x.lua"),
		);
		touch(&games.join(".dev-a-0000.zip.part"));
		touch(
			&games
				.join("github-dev-b-000000000000.sdd")
				.join("modinfo.lua"),
		);
		touch(&data.join(crate::sources::QUARANTINE).join("Game 1.sdz"));
		let now = SystemTime::now();
		// A day on, nothing is stale yet.
		assert!(sweep(&games, &data, now + days(1)).is_empty());
		// A month on, the parts and the quarantined game go; the build stays.
		let gone = sweep(&games, &data, now + days(31));
		assert_eq!(gone.len(), 3, "{gone:?}");
		assert!(!games.join(".github-dev-a-000000000000.sdd.part").exists());
		assert!(!games.join(".dev-a-0000.zip.part").exists());
		assert!(
			!data
				.join(crate::sources::QUARANTINE)
				.join("Game 1.sdz")
				.exists()
		);
		assert!(games.join("github-dev-b-000000000000.sdd").is_dir());
	}

	#[test]
	fn a_build_older_than_the_window_is_swept_and_a_newer_one_kept() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		let data = dir.path().join("data");
		touch(
			&games
				.join("github-dev-b-000000000000.sdd")
				.join("modinfo.lua"),
		);
		touch(&games.join("Game.sdd").join("modinfo.lua"));
		let now = SystemTime::now();
		assert!(sweep(&games, &data, now + days(89)).is_empty());
		let gone = sweep(&games, &data, now + days(91));
		assert_eq!(gone, [games.join("github-dev-b-000000000000.sdd")]);
		assert!(
			games.join("Game.sdd").is_dir(),
			"a game that is not a build is never the sweep's"
		);
		// Sweeping a root that is not there is nothing to do.
		assert!(sweep(&dir.path().join("nowhere"), &data, now).is_empty());
	}

	/// A build the sweep cannot remove whole is left whole under its name,
	/// or gone from it: never half of one where a lobby would find it.
	#[cfg(windows)]
	#[test]
	fn a_build_held_open_is_whole_or_gone() {
		use std::os::windows::fs::OpenOptionsExt as _;
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		let data = dir.path().join("data");
		let live = games.join("github-dev-b-000000000000.sdd");
		touch(&live.join("a.lua"));
		touch(&live.join("b.lua"));
		let _held = std::fs::OpenOptions::new()
			.read(true)
			.share_mode(0)
			.open(live.join("b.lua"))
			.unwrap();
		let now = SystemTime::now();
		let _ = sweep(&games, &data, now + days(91));
		assert!(
			!live.exists() || (live.join("a.lua").is_file() && live.join("b.lua").is_file()),
			"half a build under its live name"
		);
	}

	#[test]
	fn a_build_launched_lately_is_kept_whenever_it_was_made() {
		let dir = tempfile::tempdir().unwrap();
		let games = dir.path().join("games");
		let data = dir.path().join("data");
		let build = games.join("github-dev-b-000000000000.sdd");
		touch(&build.join("modinfo.lua"));
		let now = SystemTime::now();
		used(
			&games,
			&[build.clone(), games.join("Game.sdd")],
			now + days(60),
		)
		.unwrap();
		assert!(
			sweep(&games, &data, now + days(120)).is_empty(),
			"launched 60 days ago, made 120 ago"
		);
		assert_eq!(
			sweep(&games, &data, now + days(151)),
			std::slice::from_ref(&build)
		);
		assert!(!build.exists());
	}
}
