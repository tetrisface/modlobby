//! The files in a BAR data directory that hold what the player set up.
//!
//! `springsettings.cfg` (engine settings), `uikeys.txt` (hotkeys) and
//! `LuaUI/Config/` (widget order, enabled state and each widget's own data)
//! are small, irreplaceable and known to be lost. The engine rewrites the
//! settings file in place on every change and has emptied it before —
//! spring-launcher has kept a `springsettings-backup.cfg` against exactly that
//! since 2022 (`springsettings.js`). And a write directory that lacks them
//! starts every one from defaults: BAR reads widget state with a plain
//! `loadfile` relative to the engine's working directory, which is the write
//! directory, rather than through the VFS that finds the widgets themselves
//! across every data directory (`barwidgets.lua:326`).
//!
//! So three things, all copies. A fresh write directory is seeded from another
//! install once. Every launch is preceded by a snapshot under
//! [`BACKUP_DIR`]. And a settings file that comes out of a game with most of
//! its keys gone is noticed, so the snapshot can be put back.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use content::DataDirs;

/// The engine's settings file, the one it rewrites in place.
pub const SETTINGS: &str = "springsettings.cfg";
const FILES: &[&str] = &[SETTINGS, "uikeys.txt"];
/// Directories whose files, not subdirectories, are the player's.
const DIRS: &[&str] = &["LuaUI/Config"];
/// Under the write directory; one subdirectory per snapshot, named by
/// [`stamp`].
pub const BACKUP_DIR: &str = "modlobby-backups";
/// Snapshots kept before the oldest goes.
pub const KEEP: usize = 10;
/// Fewer keys than this and a settings file is not one anyone set up: BAR's
/// own defaults write dozens on the first run, and a file the engine has
/// emptied holds a handful.
pub const HEALTHY_KEYS: usize = 20;

/// The player's files under `dir` that exist, as paths relative to it.
pub fn list(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = FILES
        .iter()
        .map(PathBuf::from)
        .filter(|rel| dir.join(rel).is_file())
        .collect();
    for sub in DIRS {
        let Ok(entries) = std::fs::read_dir(dir.join(sub)) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .map(|entry| Path::new(sub).join(entry.file_name()))
            .collect();
        files.sort();
        found.extend(files);
    }
    found
}

fn copy_all(from: &Path, to: &Path, files: &[PathBuf]) -> io::Result<usize> {
    for rel in files {
        let target = to.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from.join(rel), target)?;
    }
    Ok(files.len())
}

fn same_files(a: &Path, b: &Path, files: &[PathBuf]) -> bool {
    files.iter().all(
        |rel| match (std::fs::read(a.join(rel)), std::fs::read(b.join(rel))) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        },
    )
}

/// `2026-09-06T00-48-12Z`: sortable, readable, legal on every filesystem.
pub fn stamp(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}-{:02}-{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Howard Hinnant's `civil_from_days`, for days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The snapshots taken so far, newest first.
pub fn snapshots(write: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(write.join(BACKUP_DIR)) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs.reverse();
    dirs
}

/// Copies the player's files to `<write>/modlobby-backups/<stamp>/`, unless
/// the newest snapshot already holds the same bytes, and keeps the last
/// [`KEEP`]. The snapshot that now matches the write directory; `None` when
/// there is nothing to keep yet.
pub fn snapshot(write: &Path, now: SystemTime) -> io::Result<Option<PathBuf>> {
    let files = list(write);
    if files.is_empty() {
        return Ok(None);
    }
    if let Some(newest) = snapshots(write).into_iter().next()
        && list(&newest) == files
        && same_files(write, &newest, &files)
    {
        return Ok(Some(newest));
    }
    let dir = write.join(BACKUP_DIR).join(stamp(now));
    copy_all(write, &dir, &files)?;
    for old in snapshots(write).into_iter().skip(KEEP) {
        let _ = std::fs::remove_dir_all(old);
    }
    Ok(Some(dir))
}

/// Copies the player's files in `from` over the write directory's — another
/// install's, or a snapshot's — after snapshotting what is there now, so this
/// is never a one-way door. How many files were copied.
pub fn import(write: &Path, from: &Path, now: SystemTime) -> io::Result<usize> {
    snapshot(write, now)?;
    copy_all(from, write, &list(from))
}

/// How many `key = value` lines a settings file holds.
pub fn settings_keys(text: &str) -> usize {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("//"))
        .filter(|line| line.contains('='))
        .count()
}

fn settings_keys_in(dir: &Path) -> Option<usize> {
    std::fs::read_to_string(dir.join(SETTINGS))
        .ok()
        .map(|text| settings_keys(&text))
}

/// Whether `dir` holds a settings file someone actually set up.
pub fn has_healthy_settings(dir: &Path) -> bool {
    settings_keys_in(dir).is_some_and(|keys| keys >= HEALTHY_KEYS)
}

/// The installs worth copying from, most trusted first.
pub fn sources(dirs: &DataDirs) -> Vec<PathBuf> {
    dirs.read
        .iter()
        .filter(|dir| has_healthy_settings(dir))
        .cloned()
        .collect()
}

/// A write directory without a settings file takes the player's files from
/// the first install that has healthy ones, never over a file already there.
/// Only ever the first time; after that the copies are theirs to change. The
/// directory seeded from.
pub fn seed(dirs: &DataDirs) -> Option<PathBuf> {
    if dirs.write.join(SETTINGS).exists() {
        return None;
    }
    let from = sources(dirs).into_iter().next()?;
    let files: Vec<PathBuf> = list(&from)
        .into_iter()
        .filter(|rel| !dirs.write.join(rel).exists())
        .collect();
    let _ = std::fs::create_dir_all(&dirs.write);
    match copy_all(&from, &dirs.write, &files) {
        Ok(count) => {
            tracing::info!(from = %from.display(), files = count, "seeded the player's files");
            Some(from)
        }
        Err(err) => {
            tracing::warn!(%err, from = %from.display(), "player's files not seeded");
            None
        }
    }
}

/// A settings file that lost most of its keys during a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Collapse {
    pub before: usize,
    pub after: usize,
}

/// Whether the settings file lost more than half its keys since `snapshot`.
/// A file that was never healthy has nothing to lose.
pub fn collapsed(write: &Path, snapshot: &Path) -> Option<Collapse> {
    let before = settings_keys_in(snapshot)?;
    let after = settings_keys_in(write).unwrap_or(0);
    (before >= HEALTHY_KEYS && after * 2 < before).then_some(Collapse { before, after })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    /// What a settings file looks like after someone has played.
    fn healthy() -> String {
        let mut text = String::from("Fullscreen = 0\nWindowBorderless = 1\n");
        for n in 0..HEALTHY_KEYS {
            text.push_str(&format!("Key{n} = {n}\n"));
        }
        text
    }

    /// An install with every kind of player file.
    fn install(text: &str) -> tempfile::TempDir {
        let home = dir();
        std::fs::write(home.path().join(SETTINGS), text).unwrap();
        std::fs::write(home.path().join("uikeys.txt"), "bind a b\n").unwrap();
        let config = home.path().join("LuaUI/Config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("BYAR.lua"), "return {}\n").unwrap();
        std::fs::create_dir_all(config.join("nested")).unwrap();
        home
    }

    fn read(dir: &Path, rel: &str) -> String {
        std::fs::read_to_string(dir.join(rel)).unwrap()
    }

    #[test]
    fn the_stamp_is_utc_and_sorts_like_time() {
        // 2026-09-03T21:25:33Z, from a logged event.
        assert_eq!(stamp(at(1_788_470_733)), "2026-09-03T21-25-33Z");
        assert_eq!(stamp(at(0)), "1970-01-01T00-00-00Z");
        assert_eq!(stamp(at(951_782_400)), "2000-02-29T00-00-00Z");
        assert!(stamp(at(1_788_470_733)) < stamp(at(1_788_470_734)));
    }

    #[test]
    fn the_players_files_are_the_settings_hotkeys_and_widget_state() {
        let home = install(&healthy());
        let found: Vec<String> = list(home.path())
            .iter()
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(
            found,
            ["springsettings.cfg", "uikeys.txt", "LuaUI/Config/BYAR.lua"],
            "files only; the nested directory is not ours"
        );
        assert!(list(dir().path()).is_empty());
    }

    #[test]
    fn a_snapshot_is_a_copy_and_an_unchanged_one_is_not_taken_twice() {
        let home = install(&healthy());
        let first = snapshot(home.path(), at(1_000)).unwrap().unwrap();
        assert_eq!(read(&first, SETTINGS), healthy());
        assert_eq!(read(&first, "LuaUI/Config/BYAR.lua"), "return {}\n");

        let again = snapshot(home.path(), at(2_000)).unwrap().unwrap();
        assert_eq!(again, first, "nothing changed, so nothing to keep");

        std::fs::write(home.path().join("uikeys.txt"), "bind c d\n").unwrap();
        let third = snapshot(home.path(), at(3_000)).unwrap().unwrap();
        assert_ne!(third, first);
        assert_eq!(snapshots(home.path()), [third, first]);
    }

    #[test]
    fn an_empty_write_dir_has_nothing_to_snapshot() {
        let home = dir();
        assert_eq!(snapshot(home.path(), at(1)).unwrap(), None);
        assert!(snapshots(home.path()).is_empty());
    }

    #[test]
    fn only_the_last_ten_snapshots_are_kept() {
        let home = install(&healthy());
        for n in 0..(KEEP + 3) {
            std::fs::write(home.path().join("uikeys.txt"), format!("bind {n}\n")).unwrap();
            snapshot(home.path(), at(1_000 * n as u64)).unwrap();
        }
        let kept = snapshots(home.path());
        assert_eq!(kept.len(), KEEP);
        assert_eq!(read(&kept[0], "uikeys.txt"), format!("bind {}\n", KEEP + 2));
    }

    #[test]
    fn a_fresh_write_dir_takes_everything_from_a_healthy_install() {
        let theirs = install(&healthy());
        let ours = dir();
        let dirs = DataDirs {
            write: ours.path().join("data"),
            read: vec![theirs.path().to_path_buf()],
        };

        assert_eq!(seed(&dirs), Some(theirs.path().to_path_buf()));
        assert_eq!(read(&dirs.write, SETTINGS), healthy());
        assert_eq!(read(&dirs.write, "uikeys.txt"), "bind a b\n");
        assert_eq!(read(&dirs.write, "LuaUI/Config/BYAR.lua"), "return {}\n");

        // Theirs changes later; ours is ours now.
        std::fs::write(theirs.path().join(SETTINGS), "Fullscreen = 1\n").unwrap();
        assert_eq!(seed(&dirs), None);
        assert_eq!(read(&dirs.write, SETTINGS), healthy());
    }

    #[test]
    fn a_degraded_install_is_not_worth_seeding_from() {
        let theirs = install("MiniMapDrawPings = 0\nWater = 4\nsnd_volbattle = 13\n");
        let ours = dir();
        let dirs = DataDirs {
            write: ours.path().join("data"),
            read: vec![theirs.path().to_path_buf()],
        };
        assert_eq!(seed(&dirs), None);
        assert!(!dirs.write.join(SETTINGS).exists());
        assert!(sources(&dirs).is_empty());
    }

    #[test]
    fn seeding_never_overwrites_a_file_already_there() {
        let theirs = install(&healthy());
        let ours = dir();
        let config = ours.path().join("LuaUI/Config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("BYAR.lua"), "mine\n").unwrap();
        let dirs = DataDirs {
            write: ours.path().to_path_buf(),
            read: vec![theirs.path().to_path_buf()],
        };
        seed(&dirs);
        assert_eq!(read(ours.path(), "LuaUI/Config/BYAR.lua"), "mine\n");
        assert_eq!(read(ours.path(), SETTINGS), healthy());
    }

    #[test]
    fn importing_snapshots_first_then_copies_over() {
        let theirs = install(&healthy());
        let ours = install("Fullscreen = 1\n");

        let copied = import(ours.path(), theirs.path(), at(1_000)).unwrap();
        assert_eq!(copied, 3);
        assert_eq!(read(ours.path(), SETTINGS), healthy());

        let kept = snapshots(ours.path());
        assert_eq!(kept.len(), 1);
        assert_eq!(read(&kept[0], SETTINGS), "Fullscreen = 1\n");

        // And a snapshot goes back the same way.
        import(ours.path(), &kept[0], at(2_000)).unwrap();
        assert_eq!(read(ours.path(), SETTINGS), "Fullscreen = 1\n");
    }

    #[test]
    fn key_counting_reads_the_file_the_way_the_engine_does() {
        assert_eq!(
            settings_keys("# comment\n// another\n\nA = 1\n  B=2  \nnot a setting\n"),
            2
        );
    }

    #[test]
    fn losing_most_keys_is_a_collapse_and_losing_a_few_is_not() {
        let home = install(&healthy());
        let before = snapshot(home.path(), at(1)).unwrap().unwrap();
        assert_eq!(collapsed(home.path(), &before), None);

        std::fs::write(home.path().join(SETTINGS), "Water = 4\nA = 1\nB = 2\n").unwrap();
        assert_eq!(
            collapsed(home.path(), &before),
            Some(Collapse {
                before: HEALTHY_KEYS + 2,
                after: 3
            })
        );

        std::fs::remove_file(home.path().join(SETTINGS)).unwrap();
        assert_eq!(
            collapsed(home.path(), &before).map(|lost| lost.after),
            Some(0),
            "a missing file is the worst case of the same thing"
        );

        // Trimmed by a third: the engine drops keys at their default, and that
        // is not what this is for.
        let mut trimmed = healthy();
        trimmed.truncate(trimmed.len() * 2 / 3);
        std::fs::write(home.path().join(SETTINGS), trimmed).unwrap();
        assert_eq!(collapsed(home.path(), &before), None);
    }

    #[test]
    fn a_snapshot_that_was_never_healthy_has_nothing_to_lose() {
        let home = install("Fullscreen = 1\n");
        let before = snapshot(home.path(), at(1)).unwrap().unwrap();
        std::fs::write(home.path().join(SETTINGS), "").unwrap();
        assert_eq!(collapsed(home.path(), &before), None);
    }
}
