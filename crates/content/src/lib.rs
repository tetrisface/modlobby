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

pub mod apple;
pub mod archive;
pub mod demo;
pub mod game_cache;
pub mod http;
pub mod map_index;
pub mod map_thumb;
pub mod release;
pub mod replays;

use std::cmp::Ordering;
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

/// Where BAR content is: the one directory modlobby writes, and any it only
/// reads. The read directories are other lobbies' installs. Their files are
/// used and never touched, so a half-written download of ours cannot appear
/// in their view, and theirs can at worst be missing from ours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDirs {
    pub write: PathBuf,
    pub read: Vec<PathBuf>,
}

impl DataDirs {
    pub fn only(write: impl Into<PathBuf>) -> Self {
        Self {
            write: write.into(),
            read: Vec::new(),
        }
    }

    /// Every directory, the writable one first.
    pub fn all(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.write.as_path()).chain(self.read.iter().map(PathBuf::as_path))
    }
}

impl From<PathBuf> for DataDirs {
    fn from(write: PathBuf) -> Self {
        Self::only(write)
    }
}

impl From<&PathBuf> for DataDirs {
    fn from(write: &PathBuf) -> Self {
        Self::only(write)
    }
}

impl From<&Path> for DataDirs {
    fn from(write: &Path) -> Self {
        Self::only(write)
    }
}

/// What is installed across the data directories, read only.
#[derive(Debug, Clone)]
pub struct Library {
    dirs: DataDirs,
}

impl Library {
    pub fn new(dirs: impl Into<DataDirs>) -> Self {
        Self { dirs: dirs.into() }
    }

    pub fn dirs(&self) -> &DataDirs {
        &self.dirs
    }

    /// Where downloads go.
    pub fn write_dir(&self) -> &Path {
        &self.dirs.write
    }

    fn any_has(&self, relative: impl AsRef<Path>) -> bool {
        self.dirs.all().any(|dir| dir.join(&relative).exists())
    }

    pub fn check(&self, engine_version: &str, game: &str, map: &str) -> Availability {
        Availability {
            engine: self.has_engine(engine_version),
            game: self.has_game(game),
            map: self.has_map(map),
        }
    }

    pub fn has_engine(&self, version: &str) -> bool {
        self.find_engine(version).is_some()
    }

    /// The installed engine for `version`, ours before anyone else's.
    pub fn find_engine(&self, version: &str) -> Option<recoil::EngineLayout> {
        self.dirs
            .all()
            .find_map(|dir| recoil::find_engine(dir, version))
    }

    /// The installed engine that the Apple Silicon port release `port_version`
    /// built, wherever it is. What the download asks before fetching anything.
    pub fn find_port(&self, port_version: &str) -> Option<recoil::EngineLayout> {
        self.dirs
            .all()
            .find_map(|dir| recoil::find_port(dir, port_version))
    }

    /// Where an engine AI declares its own options, in any installed engine.
    pub fn find_ai_options(&self, version: &str, ai: &str) -> Option<PathBuf> {
        let engine = self.find_engine(version)?;
        recoil::ai_options_file(&engine.content, ai)
    }

    /// A pr-downloader to run, from any installed engine.
    pub fn find_downloader(&self, version: &str) -> Option<PathBuf> {
        self.dirs
            .all()
            .find_map(|dir| recoil::find_downloader(dir, version))
    }

    /// Every engine version installed anywhere, newest first.
    pub fn installed_engines(&self) -> Vec<String> {
        let mut versions: Vec<String> = self
            .dirs
            .all()
            .flat_map(recoil::installed_engines)
            .collect();
        versions.sort();
        versions.dedup();
        newest_first(&mut versions);
        versions
    }

    /// The skirmish AIs any installed engine ships.
    pub fn installed_ais(&self) -> Vec<String> {
        let mut ais: Vec<String> = self.dirs.all().flat_map(recoil::installed_ais).collect();
        ais.sort();
        ais.dedup();
        ais
    }

    /// Every replay anywhere, newest first.
    pub fn replays(&self) -> Vec<replays::Replay> {
        let mut found: Vec<replays::Replay> = self.dirs.all().flat_map(replays::list).collect();
        found.sort_by(|a, b| b.played_at.cmp(&a.played_at));
        found
    }

    /// The map archive named after its display name.
    pub fn has_map(&self, display_name: &str) -> bool {
        let stem = archive_stem(display_name);
        if stem.is_empty() {
            return false;
        }
        ARCHIVES
            .iter()
            .any(|ext| self.any_has(Path::new("maps").join(format!("{stem}.{ext}"))))
    }

    /// A rapid package whose display name matches, downloaded into `packages/`.
    ///
    /// Unpacked `games/*.sdd` are deliberately not matched by name: their
    /// directory is named for the repository (`Beyond-All-Reason.sdd`), not for
    /// the version a room asks for, so any match would be a guess. Reading the
    /// name out of each `modinfo.lua` is what that would take.
    pub fn has_game(&self, display_name: &str) -> bool {
        self.rapid_md5(display_name)
            .is_some_and(|md5| self.any_has(Path::new("packages").join(format!("{md5}.sdp"))))
    }

    /// One file out of the installed game, by the version a room reports.
    ///
    /// Rapid first, then an unpacked `games/*.sdd` for anyone running a
    /// checkout of the game itself. `None` when the game is not installed,
    /// which is a normal state and not an error.
    pub fn game_file(&self, display_name: &str, file: &str) -> Option<Vec<u8>> {
        if let Some(md5) = self.rapid_md5(display_name)
            && let Some(bytes) = self
                .dirs
                .all()
                .find_map(|dir| archive::from_package(dir, &md5, file).ok())
        {
            return Some(bytes);
        }
        self.dirs
            .all()
            .find_map(|dir| archive::from_unpacked(dir, file))
    }

    /// Every file under `prefix` in the game with this display name --
    /// `units/` for what the game can build. Empty when the game is not here.
    pub fn game_files(&self, display_name: &str, prefix: &str) -> Vec<String> {
        let wanted = prefix.to_ascii_lowercase();
        if let Some(md5) = self.rapid_md5(display_name)
            && let Some(files) = self
                .dirs
                .all()
                .find_map(|dir| archive::package_files(dir, &md5).ok())
        {
            return files
                .into_iter()
                .filter(|file| file.to_ascii_lowercase().starts_with(&wanted))
                .collect();
        }
        self.dirs
            .all()
            .map(|dir| archive::unpacked_files(dir, prefix))
            .find(|files| !files.is_empty())
            .unwrap_or_default()
    }

    /// Scans every rapid index for the version with this display name.
    fn rapid_md5(&self, display_name: &str) -> Option<String> {
        // An index line whose name field is empty would otherwise answer for
        // a room that names no game at all, and report it as installed.
        if display_name.is_empty() {
            return None;
        }
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
        for hosts in self
            .dirs
            .all()
            .filter_map(|dir| std::fs::read_dir(dir.join("rapid")).ok())
        {
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
        assert!(
            library
                .write_dir()
                .join("games")
                .join("My Mod.sdd")
                .is_dir()
        );
        assert!(!library.has_game("My Mod"));
    }

    #[test]
    fn a_name_cannot_escape_the_data_directory() {
        let (_dir, library) = library();
        assert!(!library.has_map("../../etc/passwd"));
        assert_eq!(archive_stem("a/b\\c"), "abc");
    }
}

/// By name, then newest first within a name.
///
/// Neither of the two things this orders sorts right as plain text.
///
/// Engines are dated — `2026.07.04` — but an engine built from source is
/// `local-build`, which as text comes above every date and would otherwise be
/// the one a fresh room picked.
///
/// Games are worse: the list is every rapid package on the disk, so `Beyond
/// All Reason test-31232-3649678` sits beside `BYAR Chobby test-4627-9d085d9`,
/// and "newest" across two different things means nothing. Sorting the *name*
/// up and the *version* down is what keeps each game's own versions together
/// with its newest at the top, and puts the game whose name comes first at the
/// top of the lot — which is the only ordering between two of them that is
/// about anything at all.
///
/// So: text ascending, because it is a name; numbers descending, because they
/// are a version; and a name with no numbers in it last, which is where a
/// hand-built engine belongs among the releases.
pub fn newest_first(names: &mut [String]) {
    names.sort_by(|a, b| {
        // A name that says nothing about when it is goes to the bottom.
        has_digit(b)
            .cmp(&has_digit(a))
            .then_with(|| compare(&natural(a), &natural(b)))
            .then_with(|| a.cmp(b))
    });
}

fn has_digit(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_digit())
}

fn compare(left: &[Run], right: &[Run]) -> Ordering {
    for pair in left.iter().zip(right.iter()) {
        let order = match pair {
            (Run::Text(a), Run::Text(b)) => a.cmp(b),
            (Run::Number(a), Run::Number(b)) => b.cmp(a),
            // A name that opens with its version outranks one behind a prefix,
            // so `2026.07.04` is offered above `recoil_2025.04.04`.
            (Run::Number(_), Run::Text(_)) => Ordering::Less,
            (Run::Text(_), Run::Number(_)) => Ordering::Greater,
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}

/// A name as the runs it is made of: text compared as text, digits as digits.
///
/// `u64` because a build number is at most a few million and a date has no
/// component that could overflow; anything longer than a `u64` is not a
/// version and falls back to being compared as the text it is.
fn natural(name: &str) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut rest = name;
    while !rest.is_empty() {
        let digits = rest.starts_with(|c: char| c.is_ascii_digit());
        let end = rest
            .find(|c: char| c.is_ascii_digit() != digits)
            .unwrap_or(rest.len());
        let (run, tail) = rest.split_at(end);
        runs.push(match run.parse::<u64>() {
            Ok(number) if digits => Run::Number(number),
            _ => Run::Text(run.to_ascii_lowercase()),
        });
        rest = tail;
    }
    runs
}

/// A stretch of one kind.
#[derive(Debug, PartialEq, Eq)]
enum Run {
    Text(String),
    Number(u64),
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
                if self.any_has(Path::new("packages").join(format!("{md5}.sdp"))) {
                    names.push(name.to_owned());
                }
            }
        }
        names.sort();
        names.dedup();
        newest_first(&mut names);
        names
    }

    /// The archive file name of every installed map, without its extension.
    ///
    /// This is the lowercased, underscored form — `acidicquarry_5.17` — not
    /// the spring name the engine wants in a start script. Recovering the
    /// capitalisation is the caller's problem, because nothing on disk records it.
    pub fn installed_map_files(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .dirs
            .all()
            .filter_map(|dir| std::fs::read_dir(dir.join("maps")).ok())
            .flatten()
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
        names.dedup();
        names
    }
}

#[cfg(test)]
mod union_tests {
    use super::*;

    /// Ours is empty; theirs has an engine, a map and a replay.
    fn split() -> (tempfile::TempDir, tempfile::TempDir, Library) {
        let ours = tempfile::tempdir().unwrap();
        let theirs = tempfile::tempdir().unwrap();
        let engine = theirs.path().join("engine").join("recoil_2026.07.04");
        std::fs::create_dir_all(&engine).unwrap();
        std::fs::write(engine.join(recoil::ENGINE_BINARY), b"").unwrap();
        std::fs::create_dir_all(theirs.path().join("maps")).unwrap();
        std::fs::write(
            theirs.path().join("maps").join("supreme_isthmus_v2.1.sd7"),
            b"",
        )
        .unwrap();
        std::fs::create_dir_all(theirs.path().join("demos")).unwrap();
        std::fs::write(
            theirs
                .path()
                .join("demos")
                .join("2026-08-29_13-17-21-351_Supreme Isthmus v2.1_2026.07.04.sdfz"),
            b"",
        )
        .unwrap();
        let library = Library::new(DataDirs {
            write: ours.path().to_path_buf(),
            read: vec![theirs.path().to_path_buf()],
        });
        (ours, theirs, library)
    }

    #[test]
    fn another_installs_content_counts_as_ours_to_read() {
        let (_ours, theirs, library) = split();
        assert!(library.has_engine("2026.07.04"));
        assert!(library.has_map("Supreme Isthmus v2.1"));
        assert_eq!(
            library.find_engine("2026.07.04").unwrap(),
            recoil::EngineLayout::flat(theirs.path().join("engine").join("recoil_2026.07.04"))
        );
        assert_eq!(library.installed_engines(), ["2026.07.04"]);
        assert_eq!(library.installed_map_files(), ["supreme_isthmus_v2.1"]);
        assert_eq!(library.replays().len(), 1);
    }

    #[test]
    fn our_own_copy_wins_over_theirs() {
        let (ours, _theirs, library) = split();
        let engine = ours.path().join("engine").join("2026.07.04");
        std::fs::create_dir_all(&engine).unwrap();
        std::fs::write(engine.join(recoil::ENGINE_BINARY), b"").unwrap();
        assert_eq!(
            library.find_engine("2026.07.04").unwrap(),
            recoil::EngineLayout::flat(engine)
        );
        assert_eq!(library.installed_engines(), ["2026.07.04"], "listed once");
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

    fn ordered(names: &[&str]) -> Vec<String> {
        let mut held: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
        newest_first(&mut held);
        held
    }

    #[test]
    fn engines_come_newest_first_with_a_hand_built_one_last() {
        // As text `local-build` outranks every date, which would make it the
        // one a fresh room picked.
        assert_eq!(
            ordered(&["2025.04.01", "local-build", "2026.07.04", "2026.07.03"]),
            ["2026.07.04", "2026.07.03", "2025.04.01", "local-build"]
        );
    }

    #[test]
    fn a_build_number_is_compared_as_a_number_not_as_text() {
        // The day BAR's build number gains a digit, text ordering puts the
        // newest game at the bottom.
        assert_eq!(
            ordered(&[
                "Beyond All Reason test-9999-aaa",
                "Beyond All Reason test-100000-bbb",
                "Beyond All Reason test-31232-ccc",
            ]),
            [
                "Beyond All Reason test-100000-bbb",
                "Beyond All Reason test-31232-ccc",
                "Beyond All Reason test-9999-aaa",
            ]
        );
    }

    #[test]
    fn a_date_outranks_a_prefixed_one_rather_than_sorting_under_its_prefix() {
        assert_eq!(
            ordered(&["recoil_2025.04.04", "2026.07.04"]),
            ["2026.07.04", "recoil_2025.04.04"]
        );
    }

    #[test]
    fn names_with_nothing_to_go_on_are_at_least_alphabetical() {
        assert_eq!(
            ordered(&["local-build", "another-build"]),
            ["another-build", "local-build"]
        );
    }

    /// The games list is every rapid package on the disk, so BAR sits beside
    /// the Chobby menu archive. "Newest" across the two means nothing; what
    /// keeps a room from opening on the lobby menu is ordering the name up and
    /// the version down.
    #[test]
    fn each_games_versions_stay_together_with_its_newest_on_top() {
        assert_eq!(
            ordered(&[
                "BYAR Chobby test-4627-9d085d9",
                "Beyond All Reason test-31221-719595a",
                "BYAR Chobby test-4600-aaaaaaa",
                "Beyond All Reason test-31232-3649678",
            ]),
            [
                "Beyond All Reason test-31232-3649678",
                "Beyond All Reason test-31221-719595a",
                "BYAR Chobby test-4627-9d085d9",
                "BYAR Chobby test-4600-aaaaaaa",
            ]
        );
    }
}
