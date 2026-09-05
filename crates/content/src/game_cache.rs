//! Files read out of an installed game, kept for the length of the run.
//!
//! Reading one file out of a rapid package means decompressing the index,
//! finding the version's `.sdp`, and pulling the blob out of the pool: tens of
//! milliseconds, paid again for every room that runs the same game. A rapid
//! version's content never changes, so the bytes are kept the first time and
//! handed back from memory after that. An unpacked `games/*.sdd` checkout can
//! change under a running lobby and is read fresh every time; so is a game
//! that is not installed, since it may well be by the next ask.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::Library;

/// Bytes by (game display name, file path within the game).
type Held = HashMap<(String, String), Arc<[u8]>>;

#[derive(Debug, Default)]
pub struct GameFileCache {
    held: Mutex<Held>,
}

impl GameFileCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// One file out of the game with this display name, as `Library::game_file`
    /// reads it, from memory when it has been read before.
    pub fn game_file(&self, library: &Library, game: &str, file: &str) -> Option<Arc<[u8]>> {
        self.get_or_read(
            game,
            file,
            || library.game_file(game, file),
            || library.has_game(game),
        )
    }

    /// `read` when the cache has nothing for the key; kept only when `keep`
    /// says the source is immutable. The lock is not held across `read`, so
    /// a slow disk stalls nobody else — two first asks may both read, which
    /// costs a read and nothing more.
    fn get_or_read(
        &self,
        game: &str,
        file: &str,
        read: impl FnOnce() -> Option<Vec<u8>>,
        keep: impl FnOnce() -> bool,
    ) -> Option<Arc<[u8]>> {
        let key = (game.to_owned(), file.to_owned());
        if let Some(bytes) = self.lock().get(&key) {
            return Some(bytes.clone());
        }
        let bytes: Arc<[u8]> = read()?.into();
        if keep() {
            self.lock().insert(key, bytes.clone());
        }
        Some(bytes)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Held> {
        // The map is only ever inserted into; a panic while holding the lock
        // leaves it whole, so poisoning is no reason to stop answering.
        self.held
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// A reader that counts how often it was asked, answering `bytes`.
    fn counting<'a>(
        reads: &'a Cell<u32>,
        bytes: Option<&'static [u8]>,
    ) -> impl FnOnce() -> Option<Vec<u8>> + 'a {
        move || {
            reads.set(reads.get() + 1);
            bytes.map(<[u8]>::to_vec)
        }
    }

    #[test]
    fn a_packaged_file_is_read_once_per_run() {
        let cache = GameFileCache::new();
        let reads = Cell::new(0);

        let first = cache.get_or_read("BAR 1", "luaai.lua", counting(&reads, Some(b"a")), || true);
        let again = cache.get_or_read("BAR 1", "luaai.lua", counting(&reads, Some(b"b")), || true);

        assert_eq!(first.as_deref(), Some(&b"a"[..]));
        assert_eq!(again.as_deref(), Some(&b"a"[..]));
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn the_key_is_the_game_and_the_file_together() {
        let cache = GameFileCache::new();
        let reads = Cell::new(0);

        cache.get_or_read("BAR 1", "luaai.lua", counting(&reads, Some(b"a")), || true);
        cache.get_or_read("BAR 2", "luaai.lua", counting(&reads, Some(b"b")), || true);
        let other = cache.get_or_read(
            "BAR 1",
            "modoptions.lua",
            counting(&reads, Some(b"c")),
            || true,
        );

        assert_eq!(other.as_deref(), Some(&b"c"[..]));
        assert_eq!(reads.get(), 3);
    }

    #[test]
    fn a_checkout_is_read_every_time() {
        let cache = GameFileCache::new();
        let reads = Cell::new(0);

        cache.get_or_read("dev", "luaai.lua", counting(&reads, Some(b"a")), || false);
        let again = cache.get_or_read("dev", "luaai.lua", counting(&reads, Some(b"b")), || false);

        assert_eq!(again.as_deref(), Some(&b"b"[..]));
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn a_game_that_is_not_installed_yet_is_asked_for_again() {
        let cache = GameFileCache::new();
        let reads = Cell::new(0);

        let missing = cache.get_or_read("BAR 1", "luaai.lua", counting(&reads, None), || true);
        let installed =
            cache.get_or_read("BAR 1", "luaai.lua", counting(&reads, Some(b"a")), || true);

        assert!(missing.is_none());
        assert_eq!(installed.as_deref(), Some(&b"a"[..]));
        assert_eq!(reads.get(), 2);
    }
}
