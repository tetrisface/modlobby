//! The replays on this machine.
//!
//! BAR names a demo `<date>_<time>_<map>_<engine>.sdfz`, which carries
//! everything a list needs. That matters: a data directory holds thousands of
//! them, and decompressing each one to read its header would make the list
//! unusable. The header is read only for the one someone selects.

use std::path::{Path, PathBuf};

/// A replay, as far as its name tells us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replay {
    pub path: PathBuf,
    /// `2026-08-29 13:17:21`, as the name spells it.
    pub played_at: String,
    pub map: String,
    pub engine: String,
    pub bytes: u64,
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
        format!("{date} {}", clock.join(":")),
        map.to_owned(),
        engine.to_owned(),
    ))
}

/// Every replay in `<data>/demos`, newest first.
pub fn list(data_dir: &Path) -> Vec<Replay> {
    let Ok(entries) = std::fs::read_dir(data_dir.join("demos")) else {
        return Vec::new();
    };

    let mut replays: Vec<Replay> = entries
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
            Some(Replay {
                bytes: entry.metadata().map(|meta| meta.len()).unwrap_or(0),
                path,
                played_at,
                map,
                engine,
            })
        })
        .collect();

    // The name begins with the timestamp, so lexical order is chronological.
    replays.sort_by(|a, b| b.played_at.cmp(&a.played_at));
    replays
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_gives_up_its_date_map_and_engine() {
        assert_eq!(
            parse_name("2026-08-29_13-17-21-351_Full Metal Plate 1.7_2026.07.04"),
            Some((
                "2026-08-29 13:17:21".into(),
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
                "2026-08-29 09:14:10".into(),
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
    fn caches_are_left_out_and_the_newest_comes_first() {
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

        let replays = list(dir.path());
        assert_eq!(
            replays.len(),
            2,
            "the cache and the text file are not replays"
        );
        assert_eq!(replays[0].map, "Forge v2.3", "newest first");
        assert_eq!(replays[1].map, "Ditched_V1");
    }
}
