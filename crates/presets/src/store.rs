//! The file of presets, and the operations on it.
//!
//! Every write is atomic and every write keeps a backup. A preset is somebody's
//! evening of tuning a room, and this file is the only copy of the ones that
//! never went to Chobby.

use std::path::{Path, PathBuf};

use crate::chobby;
use crate::model::{Book, Preset, Stamp, VERSION};

pub const FILE_NAME: &str = "presets.json";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not readable as presets: {message}")]
    Invalid { path: PathBuf, message: String },
    #[error("no preset called {0}")]
    Unknown(String),
    #[error("a preset called {0} already exists")]
    Duplicate(String),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Presets on disk.
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            path: dir.as_ref().join(FILE_NAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Everything saved. A file that is not there yet is simply no presets.
    pub fn load(&self) -> Result<Book, Error> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Book::default()),
            Err(err) => return Err(io(&self.path)(err)),
        };
        if text.trim().is_empty() {
            return Ok(Book::default());
        }
        serde_json::from_str(&text).map_err(|err| Error::Invalid {
            path: self.path.clone(),
            message: err.to_string(),
        })
    }

    /// Replaces the file, keeping the version it was written under.
    pub fn save(&self, book: &Book) -> Result<(), Error> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let text = serde_json::to_string_pretty(&Book {
            version: VERSION,
            presets: book.presets.clone(),
        })
        .map_err(|err| Error::Invalid {
            path: self.path.clone(),
            message: err.to_string(),
        })?;

        // The previous file, kept: this is the only copy of any preset that was
        // never exported.
        let _ = std::fs::copy(&self.path, self.path.with_extension("json.bak"));

        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, &text).map_err(io(&temporary))?;
        std::fs::rename(&temporary, &self.path).map_err(io(&self.path))
    }

    /// Adds a preset, or replaces one of the same name while keeping the date
    /// it was first made — which is what "updated" means.
    pub fn put(&self, mut preset: Preset, now: Stamp) -> Result<Book, Error> {
        let mut book = self.load()?;
        preset.updated = now;
        match book
            .presets
            .iter()
            .position(|held| held.name == preset.name)
        {
            Some(at) => {
                preset.created = book.presets[at].created;
                preset.last_used = preset.last_used.or(book.presets[at].last_used);
                book.presets[at] = preset;
            }
            None => {
                preset.created = now;
                book.presets.push(preset);
            }
        }
        self.save(&book)?;
        Ok(book)
    }

    pub fn remove(&self, name: &str) -> Result<Book, Error> {
        let mut book = self.load()?;
        let before = book.presets.len();
        book.presets.retain(|preset| preset.name != name);
        if book.presets.len() == before {
            return Err(Error::Unknown(name.to_owned()));
        }
        self.save(&book)?;
        Ok(book)
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<Book, Error> {
        let mut book = self.load()?;
        if book.presets.iter().any(|preset| preset.name == to) {
            return Err(Error::Duplicate(to.to_owned()));
        }
        let held = book
            .presets
            .iter_mut()
            .find(|preset| preset.name == from)
            .ok_or_else(|| Error::Unknown(from.to_owned()))?;
        held.name = to.to_owned();
        self.save(&book)?;
        Ok(book)
    }

    /// Stamps a preset as used, which is what the table sorts by.
    pub fn touch(&self, name: &str, now: Stamp) -> Result<Book, Error> {
        let mut book = self.load()?;
        let held = book
            .presets
            .iter_mut()
            .find(|preset| preset.name == name)
            .ok_or_else(|| Error::Unknown(name.to_owned()))?;
        held.last_used = Some(now);
        self.save(&book)?;
        Ok(book)
    }

    /// Brings in everything from one of Chobby's files.
    ///
    /// A name we already hold is kept, not overwritten: ours carries history
    /// and possibly edits, and an import should never be the thing that loses
    /// them. The count of what was skipped is returned so the front end can
    /// say so rather than pretending everything arrived.
    pub fn import_chobby(&self, path: &Path, now: Stamp) -> Result<(Book, usize), Error> {
        let text = std::fs::read_to_string(path).map_err(io(path))?;
        let incoming = chobby::read(&text, now).map_err(|err| Error::Invalid {
            path: path.to_path_buf(),
            message: err.to_string(),
        })?;

        let mut book = self.load()?;
        let mut skipped = 0;
        for preset in incoming {
            if book.presets.iter().any(|held| held.name == preset.name) {
                skipped += 1;
                continue;
            }
            book.presets.push(preset);
        }
        self.save(&book)?;
        Ok((book, skipped))
    }

    /// Writes presets back into one of Chobby's files, leaving its other
    /// entries alone.
    pub fn export_chobby(&self, path: &Path, names: &[String]) -> Result<usize, Error> {
        let book = self.load()?;
        let chosen: Vec<Preset> = if names.is_empty() {
            book.presets.clone()
        } else {
            book.presets
                .iter()
                .filter(|preset| names.contains(&preset.name))
                .cloned()
                .collect()
        };
        if let Some(missing) = names
            .iter()
            .find(|name| !book.presets.iter().any(|preset| &preset.name == *name))
        {
            return Err(Error::Unknown(missing.clone()));
        }

        let existing = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(io(path)(err)),
        };
        let merged = chobby::merge_into(&existing, &chosen).map_err(|err| Error::Invalid {
            path: path.to_path_buf(),
            message: err.to_string(),
        })?;

        // Their file, so their backup too.
        if !existing.is_empty() {
            let _ = std::fs::write(path.with_extension("json.modlobby.bak"), &existing);
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, &merged).map_err(io(&temporary))?;
        std::fs::rename(&temporary, path).map_err(io(path))?;
        Ok(chosen.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StartBox;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        (dir, store)
    }

    #[test]
    fn a_missing_file_is_no_presets_rather_than_an_error() {
        let (_dir, store) = store();
        assert_eq!(store.load().unwrap().presets.len(), 0);
    }

    #[test]
    fn saving_over_a_name_keeps_the_day_it_was_made() {
        let (_dir, store) = store();
        store.put(Preset::new("raptors", 100), 100).unwrap();

        let mut changed = Preset::new("raptors", 999);
        changed.map = Some("Comet Catcher".into());
        let book = store.put(changed, 500).unwrap();

        assert_eq!(book.presets.len(), 1);
        assert_eq!(book.presets[0].created, 100, "made then, not now");
        assert_eq!(book.presets[0].updated, 500);
        assert_eq!(book.presets[0].map.as_deref(), Some("Comet Catcher"));
    }

    #[test]
    fn saving_over_a_name_keeps_when_it_was_last_used() {
        let (_dir, store) = store();
        store.put(Preset::new("raptors", 100), 100).unwrap();
        store.touch("raptors", 200).unwrap();
        let book = store.put(Preset::new("raptors", 300), 300).unwrap();
        assert_eq!(book.presets[0].last_used, Some(200));
    }

    #[test]
    fn renaming_refuses_to_land_on_a_name_already_taken() {
        let (_dir, store) = store();
        store.put(Preset::new("a", 1), 1).unwrap();
        store.put(Preset::new("b", 1), 1).unwrap();
        assert!(matches!(
            store.rename("a", "b"),
            Err(Error::Duplicate(name)) if name == "b"
        ));
        // And the file is untouched by the refusal.
        assert_eq!(store.load().unwrap().presets.len(), 2);
    }

    #[test]
    fn importing_never_overwrites_a_preset_we_already_have() {
        let (dir, store) = store();
        let mut ours = Preset::new("shared", 100);
        ours.map = Some("ours".into());
        store.put(ours, 100).unwrap();

        let theirs = dir.path().join("optionsPresets.json");
        std::fs::write(
            &theirs,
            r#"{"shared": {"Map": "theirs"}, "new one": {"Map": "also theirs"}}"#,
        )
        .unwrap();

        let (book, skipped) = store.import_chobby(&theirs, 700).unwrap();
        assert_eq!(skipped, 1, "the name we already had");
        assert_eq!(book.presets.len(), 2);
        let shared = book.presets.iter().find(|p| p.name == "shared").unwrap();
        assert_eq!(shared.map.as_deref(), Some("ours"), "ours survived");
    }

    #[test]
    fn exporting_leaves_their_other_presets_alone() {
        let (dir, store) = store();
        let mut ours = Preset::new("mine", 100);
        ours.start_boxes.insert(
            0,
            StartBox {
                left: 0,
                top: 0,
                right: 50,
                bottom: 200,
            },
        );
        store.put(ours, 100).unwrap();

        let theirs = dir.path().join("optionsPresets.json");
        std::fs::write(&theirs, r#"{"untouched": {"Map": "keep me"}}"#).unwrap();

        assert_eq!(store.export_chobby(&theirs, &[]).unwrap(), 1);
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&theirs).unwrap()).unwrap();
        assert_eq!(written["untouched"]["Map"], "keep me");
        assert!(written["mine"]["Start Boxes"].is_array());
        // Their file was backed up before we touched it.
        assert!(theirs.with_extension("json.modlobby.bak").exists());
    }

    #[test]
    fn exporting_a_name_that_is_not_there_says_so_rather_than_writing_nothing() {
        let (dir, store) = store();
        let theirs = dir.path().join("optionsPresets.json");
        assert!(matches!(
            store.export_chobby(&theirs, &["ghost".into()]),
            Err(Error::Unknown(name)) if name == "ghost"
        ));
    }

    #[test]
    fn a_write_keeps_the_previous_file() {
        let (_dir, store) = store();
        store.put(Preset::new("first", 1), 1).unwrap();
        store.put(Preset::new("second", 2), 2).unwrap();
        let backup = store.path().with_extension("json.bak");
        let kept: Book = serde_json::from_str(&std::fs::read_to_string(backup).unwrap()).unwrap();
        assert_eq!(kept.presets.len(), 1, "the file as it was before");
    }
}
