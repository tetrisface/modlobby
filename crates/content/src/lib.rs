//! Whether this machine can actually play a given room.
//!
//! A lobby that claims to be synced when it is not is worse than one that
//! admits it: SPADS will start a game the client cannot join. So every check
//! here is exact — a file that either exists or does not — and anything that
//! cannot be resolved counts as missing.
//!
//! - **Engine**: a directory under `<data>/engine` holding the binary.
//! - **Game**: rapid maps a version's display name to an MD5
//!   (`<data>/rapid/<repo>/<name>/versions.gz`, `tag,md5,depends,name`), and
//!   the package is present when `<data>/packages/<md5>.sdp` exists.
//! - **Map**: the display name lowercased with spaces as underscores, which is
//!   how BAR names the archives (`Supreme Isthmus v2.1` →
//!   `supreme_isthmus_v2.1.sd7`).

pub mod archive;
pub mod demo;
pub mod release;
pub mod replays;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Archive extensions BAR ships maps and games in; `.sdd` is an unpacked directory.
const ARCHIVES: [&str; 3] = ["sd7", "sdz", "sdd"];

/// What a room needs, and whether we have it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Availability {
    pub engine: bool,
    pub game: bool,
    pub map: bool,
}

impl Availability {
    /// Only then may we tell the room we are synced.
    pub fn complete(&self) -> bool {
        self.engine && self.game && self.map
    }

    /// What is missing, for a message worth reading.
    pub fn missing(&self) -> Vec<&'static str> {
        [
            ("engine", self.engine),
            ("game", self.game),
            ("map", self.map),
        ]
        .into_iter()
        .filter(|(_, have)| !have)
        .map(|(what, _)| what)
        .collect()
    }
}

/// The BAR data directory, read only.
#[derive(Debug, Clone)]
pub struct Library {
    data_dir: PathBuf,
}

impl Library {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn check(&self, engine_version: &str, game: &str, map: &str) -> Availability {
        Availability {
            engine: self.has_engine(engine_version),
            game: self.has_game(game),
            map: self.has_map(map),
        }
    }

    pub fn has_engine(&self, version: &str) -> bool {
        recoil::find_engine(&self.data_dir, version).is_some()
    }

    /// The map archive named after its display name.
    pub fn has_map(&self, display_name: &str) -> bool {
        let stem = archive_stem(display_name);
        if stem.is_empty() {
            return false;
        }
        ARCHIVES.iter().any(|ext| {
            self.data_dir
                .join("maps")
                .join(format!("{stem}.{ext}"))
                .exists()
        })
    }

    /// A rapid package whose display name matches, downloaded into `packages/`.
    ///
    /// Unpacked `games/*.sdd` are deliberately not matched by name: their
    /// directory is named for the repository (`Beyond-All-Reason.sdd`), not for
    /// the version a room asks for, so any match would be a guess. Reading the
    /// name out of each `modinfo.lua` is what that would take.
    pub fn has_game(&self, display_name: &str) -> bool {
        self.rapid_md5(display_name).is_some_and(|md5| {
            self.data_dir
                .join("packages")
                .join(format!("{md5}.sdp"))
                .exists()
        })
    }

    /// One file out of the installed game, by the version a room reports.
    ///
    /// Rapid first, then an unpacked `games/*.sdd` for anyone running a
    /// checkout of the game itself. `None` when the game is not installed,
    /// which is a normal state and not an error.
    pub fn game_file(&self, display_name: &str, file: &str) -> Option<Vec<u8>> {
        if let Some(md5) = self.rapid_md5(display_name)
            && let Ok(bytes) = archive::from_package(&self.data_dir, &md5, file)
        {
            return Some(bytes);
        }
        archive::from_unpacked(&self.data_dir, file)
    }

    /// Every file under `prefix` in the game with this display name --
    /// `units/` for what the game can build. Empty when the game is not here.
    pub fn game_files(&self, display_name: &str, prefix: &str) -> Vec<String> {
        let wanted = prefix.to_ascii_lowercase();
        if let Some(md5) = self.rapid_md5(display_name)
            && let Ok(files) = archive::package_files(&self.data_dir, &md5)
        {
            return files
                .into_iter()
                .filter(|file| file.to_ascii_lowercase().starts_with(&wanted))
                .collect();
        }
        archive::unpacked_files(&self.data_dir, prefix)
    }

    /// Scans every rapid index for the version with this display name.
    fn rapid_md5(&self, display_name: &str) -> Option<String> {
        for index in self.rapid_indexes() {
            let Ok(file) = std::fs::File::open(&index) else {
                continue;
            };
            for line in BufReader::new(GzDecoder::new(file))
                .lines()
                .map_while(Result::ok)
            {
                // tag,md5,depends,name
                let mut fields = line.split(',');
                let (Some(_tag), Some(md5), Some(_depends), Some(name)) =
                    (fields.next(), fields.next(), fields.next(), fields.next())
                else {
                    continue;
                };
                if name.trim() == display_name {
                    return Some(md5.to_owned());
                }
            }
        }
        None
    }

    /// `<data>/rapid/<repo host>/<repo>/versions.gz`.
    fn rapid_indexes(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let Ok(hosts) = std::fs::read_dir(self.data_dir.join("rapid")) else {
            return found;
        };
        for host in hosts.filter_map(Result::ok) {
            let Ok(repos) = std::fs::read_dir(host.path()) else {
                continue;
            };
            for repo in repos.filter_map(Result::ok) {
                let index = repo.path().join("versions.gz");
                if index.is_file() {
                    found.push(index);
                }
            }
        }
        found
    }
}

/// `Supreme Isthmus v2.1` → `supreme_isthmus_v2.1`, the archive naming BAR uses.
fn archive_stem(display_name: &str) -> String {
    display_name
        .trim()
        .to_lowercase()
        .replace(' ', "_")
        // A name is a file name here; anything that could escape the directory
        // must not survive.
        .replace(['/', '\\'], "")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn library() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let engine = root.join("engine").join("recoil_2026.07.04");
        std::fs::create_dir_all(&engine).unwrap();
        std::fs::write(engine.join(recoil::ENGINE_BINARY), b"").unwrap();
        std::fs::create_dir_all(root.join("maps")).unwrap();
        std::fs::write(root.join("maps").join("supreme_isthmus_v2.1.sd7"), b"").unwrap();
        std::fs::create_dir_all(root.join("games").join("My Mod.sdd")).unwrap();
        std::fs::create_dir_all(root.join("packages")).unwrap();
        std::fs::write(root.join("packages").join("abc123.sdp"), b"").unwrap();

        let repo = root.join("rapid").join("repos-cdn.example").join("byar");
        std::fs::create_dir_all(&repo).unwrap();
        let mut index = flate2::write::GzEncoder::new(
            std::fs::File::create(repo.join("versions.gz")).unwrap(),
            flate2::Compression::fast(),
        );
        writeln!(
            index,
            "byar:git:aaa,abc123,,Beyond All Reason test-31115-21dbf79"
        )
        .unwrap();
        writeln!(
            index,
            "byar:git:bbb,notdownloaded,,Beyond All Reason test-99999-ffffff"
        )
        .unwrap();
        index.finish().unwrap();

        let library = Library::new(root);
        (dir, library)
    }

    #[test]
    fn a_room_we_can_play_reports_complete() {
        let (_dir, library) = library();
        let check = library.check(
            "2026.07.04",
            "Beyond All Reason test-31115-21dbf79",
            "Supreme Isthmus v2.1",
        );
        assert_eq!(
            check,
            Availability {
                engine: true,
                game: true,
                map: true
            }
        );
        assert!(check.complete());
        assert!(check.missing().is_empty());
    }

    #[test]
    fn anything_unresolved_counts_as_missing() {
        let (_dir, library) = library();
        // Known to rapid, but the package was never downloaded.
        let check = library.check(
            "2025.01.01",
            "Beyond All Reason test-99999-ffffff",
            "Some Other Map 1.0",
        );
        assert_eq!(
            check,
            Availability {
                engine: false,
                game: false,
                map: false
            }
        );
        assert!(!check.complete());
        assert_eq!(check.missing(), ["engine", "game", "map"]);
        // A game rapid has never heard of is missing, not an error.
        assert!(!library.has_game("Something Invented"));
    }

    /// A known gap, asserted so it is a decision rather than a surprise: an
    /// unpacked mod is invisible here until `modinfo.lua` is read.
    #[test]
    fn an_unpacked_sdd_is_not_matched_by_name() {
        let (_dir, library) = library();
        assert!(library.data_dir().join("games").join("My Mod.sdd").is_dir());
        assert!(!library.has_game("My Mod"));
    }

    #[test]
    fn a_name_cannot_escape_the_data_directory() {
        let (_dir, library) = library();
        assert!(!library.has_map("../../etc/passwd"));
        assert_eq!(archive_stem("a/b\\c"), "abc");
    }
}

impl Library {
    /// Every game version whose package is actually on the disk, newest name
    /// first. Rapid lists far more than is installed, so the `.sdp` decides.
    pub fn installed_games(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for index in self.rapid_indexes() {
            let Ok(file) = std::fs::File::open(&index) else {
                continue;
            };
            for line in BufReader::new(GzDecoder::new(file))
                .lines()
                .map_while(Result::ok)
            {
                // tag,md5,depends,name
                let mut fields = line.split(',');
                let (Some(_tag), Some(md5), Some(_depends), Some(name)) =
                    (fields.next(), fields.next(), fields.next(), fields.next())
                else {
                    continue;
                };
                let name = name.trim();
                if name.is_empty() {
                    continue;
                }
                if self
                    .data_dir
                    .join("packages")
                    .join(format!("{md5}.sdp"))
                    .exists()
                {
                    names.push(name.to_owned());
                }
            }
        }
        names.sort();
        names.dedup();
        names.reverse();
        names
    }

    /// The archive file name of every installed map, without its extension.
    ///
    /// This is the lowercased, underscored form — `acidicquarry_5.17` — not
    /// the spring name the engine wants in a start script. Recovering the
    /// capitalisation is the caller's problem, because nothing on disk records it.
    pub fn installed_map_files(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.data_dir.join("maps")) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                let extension = path.extension()?.to_str()?;
                if extension != "sd7" && extension != "sdz" {
                    return None;
                }
                Some(path.file_stem()?.to_str()?.to_owned())
            })
            .collect();
        names.sort();
        names
    }
}

#[cfg(test)]
mod listing_tests {
    use super::*;

    #[test]
    fn only_maps_are_listed_and_only_by_stem() {
        let dir = tempfile::tempdir().unwrap();
        let maps = dir.path().join("maps");
        std::fs::create_dir_all(&maps).unwrap();
        for name in [
            "acidicquarry_5.17.sd7",
            "acidicquarry_5.17.sd7.md5.gz",
            "old_map.sdz",
            "notes.txt",
        ] {
            std::fs::write(maps.join(name), b"x").unwrap();
        }

        assert_eq!(
            Library::new(dir.path()).installed_map_files(),
            vec!["acidicquarry_5.17", "old_map"]
        );
    }

    #[test]
    fn a_directory_with_no_maps_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Library::new(dir.path()).installed_map_files().is_empty());
        assert!(Library::new(dir.path()).installed_games().is_empty());
    }
}
