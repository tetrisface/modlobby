//! The room to offer back after a restart.
//!
//! A lobby client that crashes mid-game leaves the player in a room the server
//! still has them in. Chobby remembers the room on join, forgets it on leave,
//! and offers it back at the next login if it is still open
//! (`gui_battle_rejoin.lua`). This is that memory.
//!
//! It lives beside the settings rather than inside them: it is state the app
//! keeps for itself, not a preference anyone would want to hand-edit.

use std::path::{Path, PathBuf};

const FILE_NAME: &str = "rejoin.json";

#[derive(Debug, Clone)]
pub struct RejoinMemory {
    path: PathBuf,
}

impl RejoinMemory {
    pub fn new(config_dir: impl AsRef<Path>) -> Self {
        Self {
            path: config_dir.as_ref().join(FILE_NAME),
        }
    }

    /// The room we were last in, if we did not leave it deliberately.
    pub fn remembered(&self) -> Option<u32> {
        std::fs::read_to_string(&self.path)
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    pub fn remember(&self, battle: u32) {
        self.write(&battle.to_string());
    }

    pub fn forget(&self) {
        // Removing the file rather than blanking it, so a directory someone
        // has cleaned out behaves the same as one that never had the file.
        if let Err(err) = std::fs::remove_file(&self.path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%err, path = %self.path.display(), "could not forget the last room");
        }
    }

    /// Losable: failing to remember a room costs an offer to rejoin it, which
    /// is not worth failing a join over.
    fn write(&self, text: &str) {
        if let Err(err) = std::fs::write(&self.path, text) {
            tracing::warn!(%err, path = %self.path.display(), "could not remember the room");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_room_is_remembered_until_it_is_left() {
        let dir = tempfile::tempdir().unwrap();
        let memory = RejoinMemory::new(dir.path());
        assert_eq!(memory.remembered(), None);

        memory.remember(4231);
        assert_eq!(memory.remembered(), Some(4231));

        // A fresh handle reads the same file, which is the whole point: the
        // offer has to survive the process going away.
        assert_eq!(RejoinMemory::new(dir.path()).remembered(), Some(4231));

        memory.forget();
        assert_eq!(memory.remembered(), None);
    }

    #[test]
    fn forgetting_twice_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let memory = RejoinMemory::new(dir.path());
        memory.forget();
        memory.forget();
        assert_eq!(memory.remembered(), None);
    }

    #[test]
    fn a_file_that_is_not_a_room_id_remembers_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let memory = RejoinMemory::new(dir.path());
        std::fs::write(dir.path().join(FILE_NAME), "not a number").unwrap();
        assert_eq!(memory.remembered(), None);
    }
}
