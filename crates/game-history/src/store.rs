//! Where the history is kept: `index.json.gz`, and `blobs/<id>` for each
//! distinct tweak blob, written once.
//!
//! Kept beside the settings rather than in a cache, because it outlives the
//! replays it was read from: a rebuild brings back only the games whose
//! replays are still on the disk. So an index is never written over unless it
//! was read, or was not there, or was kept aside first.
//!
//! The blobs are named by their hash, so a blob already there says what a new
//! one would, and writing one twice changes nothing. They are written before
//! the index that names them: a crash between the two leaves a blob nothing
//! names, never a name with no blob.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::Error;
use crate::index::{Index, VERSION};

const INDEX: &str = "index.json.gz";
const BLOBS: &str = "blobs";

#[derive(Debug, Clone)]
pub struct Store {
	dir: PathBuf,
}

/// An index from its saved bytes, if they are one.
fn decode(bytes: &[u8]) -> Option<Index> {
	let mut json = Vec::new();
	flate2::read::GzDecoder::new(bytes)
		.read_to_end(&mut json)
		.ok()?;
	serde_json::from_slice(&json).ok()
}

/// Whether `id` is a blob id, before it is let anywhere near a path.
fn is_blob_id(id: &str) -> bool {
	id.len() == 64
		&& id
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl Store {
	pub fn new(dir: impl Into<PathBuf>) -> Self {
		Self { dir: dir.into() }
	}

	pub fn dir(&self) -> &Path {
		&self.dir
	}

	/// The saved index.
	///
	/// None saved yet is an empty one. So is one this build cannot read, of
	/// another version or damaged; that file is kept aside as
	/// `index.json.gz.old` rather than lost to the rebuild. A file that cannot
	/// be opened at all, locked by a scanner say, is an error, so that nothing
	/// saves an empty index over the games it holds.
	pub fn load(&self) -> Result<Index, Error> {
		let path = self.dir.join(INDEX);
		let bytes = match std::fs::read(&path) {
			Ok(bytes) => bytes,
			Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Index::default()),
			Err(err) => return Err(err.into()),
		};
		match decode(&bytes) {
			Some(index) if index.version == VERSION => Ok(index),
			_ => {
				std::fs::rename(&path, self.dir.join(format!("{INDEX}.old")))?;
				Ok(Index::default())
			}
		}
	}

	/// Saves the index whole: written beside the old one, then moved over it,
	/// so a crash mid-write leaves the old index rather than half of the new.
	pub fn save(&self, index: &Index) -> Result<(), Error> {
		std::fs::create_dir_all(&self.dir)?;
		let json = serde_json::to_vec(index)?;
		let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
		gz.write_all(&json)?;
		let partial = self.dir.join(format!("{INDEX}.partial"));
		std::fs::write(&partial, gz.finish()?)?;
		std::fs::rename(&partial, self.dir.join(INDEX))?;
		Ok(())
	}

	fn blob_path(&self, id: &str) -> Result<PathBuf, Error> {
		if !is_blob_id(id) {
			return Err(Error::BadId(id.to_owned()));
		}
		Ok(self.dir.join(BLOBS).join(id))
	}

	/// Keeps a blob. One already kept is not written again: its name is its
	/// hash, so whatever is there already says the same.
	pub fn put_blob(&self, id: &str, text: &str) -> Result<(), Error> {
		let path = self.blob_path(id)?;
		if path.is_file() {
			return Ok(());
		}
		std::fs::create_dir_all(self.dir.join(BLOBS))?;
		let partial = path.with_extension("partial");
		std::fs::write(&partial, text)?;
		std::fs::rename(&partial, &path)?;
		Ok(())
	}

	pub fn blob(&self, id: &str) -> Result<String, Error> {
		Ok(std::fs::read_to_string(self.blob_path(id)?)?)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn store() -> (tempfile::TempDir, Store) {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::new(dir.path().join("history"));
		(dir, store)
	}

	fn id(fill: char) -> String {
		std::iter::repeat_n(fill, 64).collect()
	}

	#[test]
	fn an_index_comes_back_as_it_was_saved() {
		let (_dir, store) = store();
		let mut index = Index::default();
		index.files.insert(
			"a.sdfz".into(),
			crate::index::Seen {
				bytes: 3,
				modified: Some(4),
				game: None,
			},
		);
		store.save(&index).unwrap();
		assert_eq!(store.load().unwrap(), index);
	}

	#[test]
	fn no_index_yet_is_an_empty_one() {
		let (_dir, store) = store();
		assert_eq!(store.load().unwrap(), Index::default());
	}

	#[test]
	fn an_index_of_another_version_is_an_empty_one() {
		let (_dir, store) = store();
		let mut index = Index {
			version: VERSION + 1,
			..Index::default()
		};
		index.files.insert(
			"a.sdfz".into(),
			crate::index::Seen {
				bytes: 3,
				modified: None,
				game: None,
			},
		);
		store.save(&index).unwrap();
		assert_eq!(store.load().unwrap(), Index::default());
		assert!(
			store.dir().join(format!("{INDEX}.old")).is_file(),
			"kept aside, not lost to the rebuild"
		);
	}

	#[test]
	fn an_index_this_build_cannot_read_is_kept_aside_for_an_empty_one() {
		let (_dir, store) = store();
		std::fs::create_dir_all(store.dir()).unwrap();
		std::fs::write(store.dir().join(INDEX), b"not gzip").unwrap();
		assert_eq!(store.load().unwrap(), Index::default());
		assert_eq!(
			std::fs::read(store.dir().join(format!("{INDEX}.old"))).unwrap(),
			b"not gzip"
		);
	}

	/// A file a scanner has locked reads as an error, so that nobody saves an
	/// empty index over the games it holds. A directory in its place is the
	/// portable way to make opening it fail.
	#[test]
	fn an_index_that_cannot_be_opened_is_an_error_rather_than_none() {
		let (_dir, store) = store();
		std::fs::create_dir_all(store.dir().join(INDEX)).unwrap();
		assert!(matches!(store.load(), Err(Error::Io(_))));
	}

	#[test]
	fn a_blob_is_written_once() {
		let (_dir, store) = store();
		store.put_blob(&id('a'), "first").unwrap();
		store.put_blob(&id('a'), "second").unwrap();
		assert_eq!(store.blob(&id('a')).unwrap(), "first");
	}

	#[test]
	fn anything_but_a_blob_id_is_refused_before_it_reaches_a_path() {
		let (_dir, store) = store();
		for bad in ["../index.json.gz", "..", &id('A'), &id('g'), "abc"] {
			assert!(
				matches!(store.blob(bad), Err(Error::BadId(_))),
				"{bad} is not an id"
			);
			assert!(matches!(store.put_blob(bad, "x"), Err(Error::BadId(_))));
		}
	}
}
