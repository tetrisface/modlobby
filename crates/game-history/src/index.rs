//! The index: every replay seen, every game they recorded, every setting.
//!
//! Kept up to date by looking only at what changed. A replay is read when its
//! size or time differs from what the index last saw of it, so a second look
//! at an unchanged directory opens nothing. A replay that could not be read is
//! remembered as such and left alone until it changes.
//!
//! Games outlive their files: a deleted replay leaves its game in the
//! history, with no file to play. Only a directory that could be listed says
//! a file is gone; one that could not, an unplugged drive say, says nothing.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::UNIX_EPOCH;

use content::ReplayListing;
use content::demo::Head;
use content::replays::Entry;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::Error;
use crate::read::{self, Game, Parsed, Setting};
use crate::store::Store;

/// Bumped when the index changes shape; an index of another version is
/// rebuilt from the replays.
pub const VERSION: u32 = 2;

/// How many replays are read between saves on a first, long build.
const CHECKPOINT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
	pub version: u32,
	/// Replay path to what was last seen of it.
	pub files: BTreeMap<String, Seen>,
	/// By [`Game::id`].
	pub games: BTreeMap<String, Game>,
	/// By [`Game::setting`].
	pub settings: BTreeMap<String, Setting>,
}

impl Default for Index {
	fn default() -> Self {
		Self {
			version: VERSION,
			files: BTreeMap::new(),
			games: BTreeMap::new(),
			settings: BTreeMap::new(),
		}
	}
}

/// A replay file as last seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Seen {
	pub bytes: u64,
	/// Milliseconds since the epoch.
	pub modified: Option<u64>,
	/// The game it recorded; `None` when it could not be read.
	pub game: Option<String>,
}

/// Where replays come from: the disk, or a test's stand-in for it.
pub trait Source {
	fn listing(&self) -> ReplayListing;
	fn head(&self, path: &Path) -> Result<Head, content::demo::Error>;
}

impl Source for content::Library {
	fn listing(&self) -> ReplayListing {
		self.replay_listing()
	}

	fn head(&self, path: &Path) -> Result<Head, content::demo::Error> {
		content::demo::head(path)
	}
}

/// How far a look at the replays has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScanProgress {
	/// Replays read so far.
	pub done: usize,
	/// Replays this look has to read: the new and the changed.
	pub total: usize,
}

/// What reading one replay found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
	pub path: String,
	pub bytes: u64,
	pub modified: Option<u64>,
	/// `None` when it could not be read.
	pub parsed: Option<Parsed>,
}

fn millis(entry: &Entry) -> Option<u64> {
	let since = entry.modified?.duration_since(UNIX_EPOCH).ok()?;
	u64::try_from(since.as_millis()).ok()
}

fn key(path: &Path) -> String {
	path.display().to_string()
}

/// The entries the index has not seen as they are now, in the listing's
/// order, which is newest first.
pub fn fresh<'a>(index: &Index, entries: &'a [Entry]) -> Vec<&'a Entry> {
	entries
		.iter()
		.filter(|entry| {
			index
				.files
				.get(&key(&entry.path))
				.is_none_or(|seen| seen.bytes != entry.bytes || seen.modified != millis(entry))
		})
		.collect()
}

/// Whether `path` is a file of a directory the listing could read, so that
/// its absence from the listing means it is gone.
fn listed(path: &str, listing: &ReplayListing) -> bool {
	Path::new(path)
		.parent()
		.is_some_and(|parent| listing.listed.iter().any(|dir| dir == parent))
}

/// The index with what was read added, and what the listing no longer holds
/// taken out.
pub fn merge(index: &Index, listing: &ReplayListing, read: &[Read]) -> Index {
	let mut next = index.clone();

	for one in read {
		let game = one.parsed.as_ref().map(|parsed| {
			next.settings
				.entry(parsed.game.setting.clone())
				.or_insert_with(|| parsed.setting.clone());
			// A game seen before, in another file of it, keeps what it had.
			let held = next
				.games
				.entry(parsed.game.id.clone())
				.or_insert_with(|| parsed.game.clone());
			if held.replay.is_none() {
				held.replay = Some(one.path.clone());
			}
			parsed.game.id.clone()
		});
		next.files.insert(
			one.path.clone(),
			Seen {
				bytes: one.bytes,
				modified: one.modified,
				game,
			},
		);
	}

	let present: std::collections::BTreeSet<String> = listing
		.entries
		.iter()
		.map(|entry| key(&entry.path))
		.collect();
	next.files
		.retain(|path, _| present.contains(path) || !listed(path, listing));

	// A game whose file went keeps its place in the history, played from
	// another file of it where there is one.
	let files = &next.files;
	for game in next.games.values_mut() {
		if game
			.replay
			.as_ref()
			.is_some_and(|path| !files.contains_key(path) && listed(path, listing))
		{
			game.replay = files
				.iter()
				.find(|(_, seen)| seen.game.as_ref() == Some(&game.id))
				.map(|(path, _)| path.clone());
		}
	}
	next
}

/// Looks at the replays again: reads what is new or changed, newest first,
/// writes the blobs, and saves the index as it goes. Blobs are written
/// before the index that names them, so a saved index never refers to a
/// blob that is not there.
pub fn refresh(
	index: &Index,
	store: &Store,
	source: &impl Source,
	mut progress: impl FnMut(ScanProgress),
) -> Result<Index, Error> {
	let listing = source.listing();
	let todo = fresh(index, &listing.entries);
	let total = todo.len();

	let mut current = merge(index, &listing, &[]);
	let mut done = 0;
	for batch in todo.chunks(CHECKPOINT) {
		let read: Vec<Read> = batch
			.iter()
			.map(|entry| {
				let parsed = source
					.head(&entry.path)
					.ok()
					.map(|head| read::read(&entry.path, &head));
				done += 1;
				progress(ScanProgress { done, total });
				Read {
					path: key(&entry.path),
					bytes: entry.bytes,
					modified: millis(entry),
					parsed,
				}
			})
			.collect();
		for (id, text) in read
			.iter()
			.filter_map(|one| one.parsed.as_ref())
			.flat_map(|parsed| &parsed.blobs)
		{
			store.put_blob(id, text)?;
		}
		current = merge(&current, &listing, &read);
		store.save(&current)?;
	}

	if total == 0 && current != *index {
		store.save(&current)?;
	}
	Ok(current)
}

#[cfg(test)]
mod tests {
	use std::cell::Cell;
	use std::path::PathBuf;
	use std::time::{Duration, SystemTime};

	use super::*;

	/// Replays that exist only in memory, counting every one opened.
	#[derive(Default)]
	struct Fake {
		listing: ReplayListing,
		heads: BTreeMap<PathBuf, Head>,
		opened: Cell<usize>,
	}

	impl Source for Fake {
		fn listing(&self) -> ReplayListing {
			self.listing.clone()
		}

		fn head(&self, path: &Path) -> Result<Head, content::demo::Error> {
			self.opened.set(self.opened.get() + 1);
			self.heads
				.get(path)
				.cloned()
				.ok_or(content::demo::Error::NotADemo)
		}
	}

	fn demos() -> PathBuf {
		PathBuf::from("data").join("demos")
	}

	fn entry(name: &str, bytes: u64, modified: u64) -> Entry {
		Entry {
			path: demos().join(name),
			played_at: format!("2026-11-0{}T10:00:00Z", name.len() % 9),
			map: "Forge".into(),
			engine: "2026.07.04".into(),
			bytes,
			modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(modified)),
		}
	}

	fn head(id: u8, modoptions: &str) -> Head {
		Head {
			game_id: (id != 0).then_some([id; 16]),
			started: 1_762_000_000 + u64::from(id),
			length: None,
			script: format!(
				"[GAME]\n{{\n\tmapname=Forge;\n\t[MODOPTIONS]\n\t{{\n{modoptions}\t}}\n}}\n"
			),
		}
	}

	impl Fake {
		fn with(mut self, entry: Entry, head: Option<Head>) -> Self {
			if let Some(head) = head {
				self.heads.insert(entry.path.clone(), head);
			}
			self.listing.entries.push(entry);
			if self.listing.listed.is_empty() {
				self.listing.listed.push(demos());
			}
			self
		}
	}

	fn store() -> (tempfile::TempDir, Store) {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::new(dir.path().join("history"));
		(dir, store)
	}

	#[test]
	fn a_second_look_at_unchanged_replays_opens_nothing() {
		let (_dir, store) = store();
		let source = Fake::default()
			.with(entry("a.sdfz", 10, 1), Some(head(1, "\t\ta=1;\n")))
			.with(entry("b.sdfz", 10, 1), Some(head(2, "\t\ta=2;\n")));

		let first = refresh(&Index::default(), &store, &source, |_| {}).unwrap();
		assert_eq!(source.opened.get(), 2);
		assert_eq!(first.games.len(), 2);
		assert_eq!(first.settings.len(), 2);

		let second = refresh(&first, &store, &source, |_| {}).unwrap();
		assert_eq!(source.opened.get(), 2, "nothing read the second time");
		assert_eq!(second, first);
	}

	#[test]
	fn a_replay_whose_size_or_time_changed_is_read_again() {
		let (_dir, store) = store();
		let before = Fake::default().with(entry("a.sdfz", 10, 1), Some(head(1, "")));
		let index = refresh(&Index::default(), &store, &before, |_| {}).unwrap();

		let after = Fake::default().with(entry("a.sdfz", 10, 2), Some(head(1, "")));
		refresh(&index, &store, &after, |_| {}).unwrap();
		assert_eq!(after.opened.get(), 1);
	}

	#[test]
	fn a_replay_that_cannot_be_read_is_left_alone_until_it_changes() {
		let (_dir, store) = store();
		let broken = Fake::default().with(entry("a.sdfz", 10, 1), None);
		let index = refresh(&Index::default(), &store, &broken, |_| {}).unwrap();
		assert!(index.games.is_empty());
		assert_eq!(index.files[&key(&demos().join("a.sdfz"))].game, None);

		refresh(&index, &store, &broken, |_| {}).unwrap();
		assert_eq!(
			broken.opened.get(),
			1,
			"not retried while it is the same file"
		);
	}

	#[test]
	fn a_game_recorded_twice_is_one_game() {
		let (_dir, store) = store();
		// A rejoin writes another demo of the same game.
		let source = Fake::default()
			.with(entry("a.sdfz", 10, 1), Some(head(1, "")))
			.with(entry("b.sdfz", 20, 1), Some(head(1, "")));
		let index = refresh(&Index::default(), &store, &source, |_| {}).unwrap();
		assert_eq!(index.games.len(), 1);
		assert_eq!(index.files.len(), 2);
	}

	#[test]
	fn games_without_an_id_are_told_apart_by_their_script() {
		let (_dir, store) = store();
		let source = Fake::default()
			.with(entry("a.sdfz", 10, 1), Some(head(0, "\t\ta=1;\n")))
			.with(entry("b.sdfz", 10, 1), Some(head(0, "\t\ta=2;\n")))
			.with(entry("c.sdfz", 10, 1), Some(head(0, "\t\ta=1;\n")));
		let index = refresh(&Index::default(), &store, &source, |_| {}).unwrap();
		assert_eq!(index.games.len(), 2);
	}

	#[test]
	fn a_deleted_replay_keeps_its_game_without_a_file() {
		let (_dir, store) = store();
		let source = Fake::default().with(entry("a.sdfz", 10, 1), Some(head(1, "")));
		let index = refresh(&Index::default(), &store, &source, |_| {}).unwrap();

		let emptied = Fake {
			listing: ReplayListing {
				listed: vec![demos()],
				entries: Vec::new(),
			},
			..Fake::default()
		};
		let index = refresh(&index, &store, &emptied, |_| {}).unwrap();
		assert!(index.files.is_empty());
		assert_eq!(index.games.len(), 1);
		assert_eq!(index.games.values().next().unwrap().replay, None);
	}

	#[test]
	fn a_directory_that_could_not_be_listed_loses_nothing() {
		let (_dir, store) = store();
		let source = Fake::default().with(entry("a.sdfz", 10, 1), Some(head(1, "")));
		let index = refresh(&Index::default(), &store, &source, |_| {}).unwrap();

		let unplugged = Fake::default();
		let after = refresh(&index, &store, &unplugged, |_| {}).unwrap();
		assert_eq!(after, index);
	}

	#[test]
	fn the_index_is_saved_with_its_blobs() {
		let (_dir, store) = store();
		let blob = tweaks::base64url::encode("-- Mine\nlocal x = 1\n", tweaks::Kind::Defs).unwrap();
		let source = Fake::default().with(
			entry("a.sdfz", 10, 1),
			Some(head(1, &format!("\t\ttweakdefs={blob};\n"))),
		);
		let index = refresh(&Index::default(), &store, &source, |_| {}).unwrap();

		assert_eq!(store.load().unwrap(), index);
		let id = &index.settings.values().next().unwrap().blobs["tweakdefs"].id;
		assert_eq!(store.blob(id).unwrap(), blob);
	}

	#[test]
	fn progress_counts_the_replays_to_read() {
		let (_dir, store) = store();
		let source = Fake::default()
			.with(entry("a.sdfz", 10, 1), Some(head(1, "")))
			.with(entry("b.sdfz", 10, 1), None);
		let mut seen = Vec::new();
		refresh(&Index::default(), &store, &source, |step| seen.push(step)).unwrap();
		assert_eq!(seen.last(), Some(&ScanProgress { done: 2, total: 2 }));
	}

	#[test]
	fn the_newest_replay_is_read_first() {
		let index = Index::default();
		let entries = [entry("new.sdfz", 1, 1), entry("old.sdfz", 1, 1)];
		let order: Vec<&Path> = fresh(&index, &entries)
			.into_iter()
			.map(|entry| entry.path.as_path())
			.collect();
		assert_eq!(order, [demos().join("new.sdfz"), demos().join("old.sdfz")]);
	}

	#[test]
	fn read_is_the_same_as_reading_the_head() {
		// `merge` takes the setting `read` made, unchanged.
		let parsed = read::read(Path::new("a"), &head(1, "\t\ta=1;\n"));
		let index = merge(
			&Index::default(),
			&ReplayListing::default(),
			&[Read {
				path: "a".into(),
				bytes: 1,
				modified: None,
				parsed: Some(parsed.clone()),
			}],
		);
		assert_eq!(index.settings[&parsed.game.setting], parsed.setting);
		assert_eq!(index.games[&parsed.game.id], parsed.game);
	}
}
