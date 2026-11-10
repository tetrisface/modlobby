//! The replays on this machine.
//!
//! BAR names a demo `<date>_<time>_<map>_<engine>.sdfz`, which carries the
//! date, map and engine. The game's length is only in the header, so the first
//! 320 bytes of each are decompressed for it: 631 replays took 200 ms warm and
//! 1.3 s from a cold disk in a debug build, which is why the list is read off
//! the main thread. The rest of the file is never touched.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::demo;

/// A replay, as far as its name and header tell us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replay {
	pub path: PathBuf,
	/// `2026-08-29T13:17:21Z`: the name's clock is UTC, and saying so lets
	/// whoever shows it put it in local time.
	pub played_at: String,
	pub map: String,
	pub engine: String,
	pub bytes: u64,
	/// `None` for a game left before it ended: the engine writes the length
	/// at game over, and a replay saved without one has zeros there.
	pub length: Option<demo::Length>,
}

/// `2026-08-29_13-17-21-351_Full Metal Plate 1.7_2026.07.04`
///
/// The map is whatever sits between the timestamp and the engine, because a
/// map name may contain underscores (`Ditched_V1`) while the fields around it
/// may not.
fn parse_name(stem: &str) -> Option<(String, String, String)> {
	let (date, rest) = stem.split_once('_')?;
	let (time, rest) = rest.split_once('_')?;
	let (map, engine) = rest.rsplit_once('_')?;
	if map.is_empty() || engine.is_empty() {
		return None;
	}

	// `13-17-21-351` — seconds are enough; the milliseconds are there to keep
	// two games started in the same second apart, not to be read.
	let clock: Vec<&str> = time.split('-').take(3).collect();
	if clock.len() != 3 {
		return None;
	}

	Some((
		format!("{date}T{}Z", clock.join(":")),
		map.to_owned(),
		engine.to_owned(),
	))
}

/// A replay as its name and the directory listing tell it, with no file
/// opened: cheap enough to ask of every replay each time someone looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
	pub path: PathBuf,
	/// As [`Replay::played_at`].
	pub played_at: String,
	pub map: String,
	pub engine: String,
	pub bytes: u64,
	/// `None` where the file system keeps no modification time.
	pub modified: Option<SystemTime>,
}

/// Every replay in `<data>/demos`, newest first, from the listing alone.
///
/// An error means the directory could not be listed, which is not the same
/// as it holding no replays: a drive that is not plugged in has not lost its
/// games.
pub fn entries(data_dir: &Path) -> std::io::Result<Vec<Entry>> {
	let mut found: Vec<Entry> = std::fs::read_dir(data_dir.join("demos"))?
		.filter_map(Result::ok)
		.filter_map(|entry| {
			let path = entry.path();
			// The engine writes a `.sdfz.cache` beside each one; its stem still
			// ends in `.sdfz`, so match on the extension rather than the name.
			if path.extension()?.to_str()? != "sdfz" {
				return None;
			}
			let stem = path.file_stem()?.to_str()?;
			let (played_at, map, engine) = parse_name(stem)?;
			let meta = entry.metadata().ok();
			let bytes = meta.as_ref().map_or(0, std::fs::Metadata::len);
			// The engine holds a game in memory and writes it out when it
			// shuts down, so a game still running, or one whose engine was
			// killed, is an empty file. Neither can be played.
			if bytes == 0 {
				return None;
			}
			Some(Entry {
				modified: meta.and_then(|meta| meta.modified().ok()),
				bytes,
				path,
				played_at,
				map,
				engine,
			})
		})
		.collect();

	// The name begins with the timestamp, so lexical order is chronological.
	found.sort_by(|a, b| b.played_at.cmp(&a.played_at));
	Ok(found)
}

/// Every replay in `<data>/demos`, newest first.
pub fn list(data_dir: &Path) -> Vec<Replay> {
	entries(data_dir)
		.unwrap_or_default()
		.into_iter()
		.map(|entry| Replay {
			length: demo::length(&entry.path).ok().and_then(demo::Length::ended),
			path: entry.path,
			played_at: entry.played_at,
			map: entry.map,
			engine: entry.engine,
			bytes: entry.bytes,
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_name_gives_up_its_date_map_and_engine() {
		assert_eq!(
			parse_name("2026-08-29_13-17-21-351_Full Metal Plate 1.7_2026.07.04"),
			Some((
				"2026-08-29T13:17:21Z".into(),
				"Full Metal Plate 1.7".into(),
				"2026.07.04".into()
			))
		);
	}

	#[test]
	fn a_map_may_contain_the_separator() {
		// Which is why the engine is taken from the right, not the map from the left.
		assert_eq!(
			parse_name("2026-08-29_09-14-10-623_Ditched_V1_2026.07.04"),
			Some((
				"2026-08-29T09:14:10Z".into(),
				"Ditched_V1".into(),
				"2026.07.04".into()
			))
		);
	}

	#[test]
	fn a_name_that_is_not_a_replay_is_skipped_rather_than_guessed_at() {
		assert_eq!(parse_name("notes"), None);
		assert_eq!(parse_name("2026-08-29_only-two-fields"), None);
		assert_eq!(parse_name("2026-08-29_13-17_map_"), None);
	}

	#[test]
	fn a_missing_demos_directory_is_simply_no_replays() {
		let dir = tempfile::tempdir().unwrap();
		assert!(list(dir.path()).is_empty());
	}

	#[test]
	fn caches_and_empty_demos_are_left_out_and_the_newest_comes_first() {
		let dir = tempfile::tempdir().unwrap();
		let demos = dir.path().join("demos");
		std::fs::create_dir_all(&demos).unwrap();
		for name in [
			"2026-08-29_09-14-10-623_Ditched_V1_2026.07.04.sdfz",
			"2026-08-29_13-17-21-351_Forge v2.3_2026.07.04.sdfz",
			"2026-08-29_13-17-21-351_Forge v2.3_2026.07.04.sdfz.cache",
			"readme.txt",
		] {
			std::fs::write(demos.join(name), b"x").unwrap();
		}
		std::fs::write(
			demos.join("2026-08-29_14-00-00-000_Forge v2.3_2026.07.04.sdfz"),
			b"",
		)
		.unwrap();

		let replays = list(dir.path());
		assert_eq!(
			replays.len(),
			2,
			"the cache, the text file and the empty demo are not replays"
		);
		assert_eq!(replays[0].map, "Forge v2.3", "newest first");
		assert_eq!(replays[1].map, "Ditched_V1");
	}

	#[test]
	fn an_entry_comes_from_the_name_and_the_directory_listing_alone() {
		let dir = tempfile::tempdir().unwrap();
		let demos = dir.path().join("demos");
		std::fs::create_dir_all(&demos).unwrap();
		// Not a demo inside: an entry is a name and a stat, and nothing is read.
		std::fs::write(
			demos.join("2026-08-29_09-14-10-623_Ditched_V1_2026.07.04.sdfz"),
			b"not gzip",
		)
		.unwrap();
		std::fs::write(
			demos.join("2026-08-29_14-00-00-000_Forge v2.3_2026.07.04.sdfz"),
			b"",
		)
		.unwrap();
		std::fs::write(
			demos.join("2026-08-29_13-17-21-351_Forge v2.3_2026.07.04.sdfz.cache"),
			b"x",
		)
		.unwrap();

		let found = entries(dir.path()).unwrap();

		assert_eq!(found.len(), 1, "the empty demo and the cache are left out");
		let entry = &found[0];
		assert_eq!(entry.played_at, "2026-08-29T09:14:10Z");
		assert_eq!(entry.map, "Ditched_V1");
		assert_eq!(entry.engine, "2026.07.04");
		assert_eq!(entry.bytes, 8);
		assert!(entry.modified.is_some());
	}

	#[test]
	fn entries_come_newest_first() {
		let dir = tempfile::tempdir().unwrap();
		let demos = dir.path().join("demos");
		std::fs::create_dir_all(&demos).unwrap();
		for name in [
			"2026-08-29_09-14-10-623_Ditched_V1_2026.07.04.sdfz",
			"2026-08-29_13-17-21-351_Forge v2.3_2026.07.04.sdfz",
		] {
			std::fs::write(demos.join(name), b"x").unwrap();
		}
		let maps: Vec<String> = entries(dir.path())
			.unwrap()
			.into_iter()
			.map(|entry| entry.map)
			.collect();
		assert_eq!(maps, ["Forge v2.3", "Ditched_V1"]);
	}

	#[test]
	fn a_demos_directory_that_cannot_be_read_is_an_error_rather_than_no_replays() {
		let dir = tempfile::tempdir().unwrap();
		assert!(entries(dir.path()).is_err());
	}

	#[test]
	fn a_finished_game_has_a_length_and_one_left_early_does_not() {
		use std::io::Write;

		let dir = tempfile::tempdir().unwrap();
		let demos = dir.path().join("demos");
		std::fs::create_dir_all(&demos).unwrap();
		// A version 5 header with the two lengths at 312 and 316, gzipped.
		let write = |name: &str, game: i32, wall: i32| {
			let mut header = vec![0_u8; 352];
			header[..15].copy_from_slice(b"spring demofile");
			header[16..20].copy_from_slice(&5_i32.to_le_bytes());
			header[312..316].copy_from_slice(&game.to_le_bytes());
			header[316..320].copy_from_slice(&wall.to_le_bytes());
			let file = std::fs::File::create(demos.join(name)).unwrap();
			let mut gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
			gz.write_all(&header).unwrap();
			gz.finish().unwrap();
		};
		write(
			"2026-08-29_10-00-00-000_Forge v2.3_2026.07.04.sdfz",
			3095,
			2266,
		);
		write("2026-08-29_11-00-00-000_Forge v2.3_2026.07.04.sdfz", 0, 0);

		let replays = list(dir.path());
		assert_eq!(replays[0].length, None, "left before the game over");
		assert_eq!(
			replays[1].length,
			Some(demo::Length {
				game: 3095,
				wall: 2266
			})
		);
	}
}
