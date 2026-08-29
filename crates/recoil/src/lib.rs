//! Launching the Recoil engine the way spring-launcher and bar-lobby do:
//! `spring --write-dir <data> --isolation <spring://… | script.txt>`.
//!
//! No I/O beyond reading the engine directory; spawning is the caller's job so
//! the command line stays unit-testable.

pub mod script;

use std::path::{Path, PathBuf};
use std::process::Command;

/// Engine binary inside an engine directory.
pub const ENGINE_BINARY: &str = if cfg!(windows) {
    "spring.exe"
} else {
    "spring"
};

/// `spring://<user>:<script password>@<host>:<port>` — what Chobby hands the
/// engine to join a hosted game (`liblobby/lobby/lobby.lua` `ConnectToBattle`,
/// parsed in `rts/System/SpringApp.cpp`).
pub fn spring_url(username: &str, script_password: &str, host: &str, port: u16) -> String {
    format!("spring://{username}:{script_password}@{host}:{port}")
}

/// The directory under `<data>/engine` holding `version`, as the BAR launcher
/// names them (`2026.07.04` → `recoil_2026.07.04`), and only if the binary is there.
pub fn find_engine(data_dir: &Path, version: &str) -> Option<PathBuf> {
    let suffix = format!("_{version}");
    std::fs::read_dir(data_dir.join("engine"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            (name == version || name.ends_with(&suffix)) && path.join(ENGINE_BINARY).is_file()
        })
}

/// One engine invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub engine_dir: PathBuf,
    /// The BAR data directory (`--write-dir`); `--isolation` keeps the engine from reading anything else.
    pub data_dir: PathBuf,
    /// A `spring://` URL or a start-script path.
    pub target: String,
}

impl Launch {
    /// Mirrors `bar-lobby/src/main/game/game.ts`.
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(self.engine_dir.join(ENGINE_BINARY));
        cmd.current_dir(&self.engine_dir)
            .arg("--write-dir")
            .arg(&self.data_dir)
            .arg("--isolation")
            .arg(&self.target);
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_matches_chobby() {
        assert_eq!(
            spring_url("me", "4242", "1.2.3.4", 8452),
            "spring://me:4242@1.2.3.4:8452"
        );
    }

    #[test]
    fn finds_engine_by_version_suffix_with_binary() {
        let root = std::env::temp_dir().join(format!("recoil-test-{}", std::process::id()));
        let engine = root.join("engine").join("recoil_2026.07.04");
        std::fs::create_dir_all(&engine).unwrap();
        std::fs::write(engine.join(ENGINE_BINARY), b"").unwrap();
        std::fs::create_dir_all(root.join("engine").join("recoil_2025.04.01")).unwrap();

        assert_eq!(find_engine(&root, "2026.07.04"), Some(engine));
        assert_eq!(find_engine(&root, "2025.04.01"), None, "no binary");
        assert_eq!(find_engine(&root, "1999.01.01"), None);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn command_mirrors_bar_lobby() {
        let launch = Launch {
            engine_dir: "C:/e".into(),
            data_dir: "C:/d".into(),
            target: "spring://me:1@h:2".into(),
        };
        let cmd = launch.command();
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            ["--write-dir", "C:/d", "--isolation", "spring://me:1@h:2"]
        );
        assert!(cmd.get_program().to_string_lossy().ends_with(ENGINE_BINARY));
    }
}

/// pr-downloader binary inside an engine directory. It ships as part of an
/// engine, so a data directory with no engine cannot fetch anything.
pub const DOWNLOADER_BINARY: &str = if cfg!(windows) {
    "pr-downloader.exe"
} else {
    "pr-downloader"
};

/// The pr-downloader to use: the one beside `version` if that engine is
/// installed, otherwise any engine's, since the binary does not care which
/// engine it came from.
pub fn find_downloader(data_dir: &Path, version: &str) -> Option<PathBuf> {
    let preferred = find_engine(data_dir, version).map(|dir| dir.join(DOWNLOADER_BINARY));
    if let Some(path) = preferred
        && path.is_file()
    {
        return Some(path);
    }

    std::fs::read_dir(data_dir.join("engine"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(DOWNLOADER_BINARY))
        .find(|path| path.is_file())
}

/// What to fetch. pr-downloader takes a game by rapid tag or name, and a map by
/// its spring name — the same strings a room reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    Game,
    Map,
}

impl Want {
    fn flag(self) -> &'static str {
        match self {
            Self::Game => "--download-game",
            Self::Map => "--download-map",
        }
    }
}

/// One pr-downloader invocation.
///
/// Everything it fetches goes in one invocation rather than one each: it
/// rewrites rapid's repo index every time it runs, so two at once fight over
/// the same file, and it parallelises within a single run anyway
/// (`bar-lobby/src/main/content/pr-downloader.ts:70`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub binary: PathBuf,
    /// `--filesystem-writepath`: the BAR data directory.
    pub data_dir: PathBuf,
    pub wants: Vec<(Want, String)>,
}

/// Where pr-downloader looks for BAR's content.
///
/// Without these it falls back to springrts.com, which does not carry BAR and
/// fails with nothing more useful than `Error occurred while downloading: 1`.
/// The values are BAR's own published endpoints
/// (`bar-lobby/src/main/json/model/config.ts`).
pub const RAPID_REPO_MASTER: &str = "https://repos-cdn.beyondallreason.dev/repos.gz";
pub const HTTP_SEARCH_URL: &str = "https://files-cdn.beyondallreason.dev/find";
/// pr-downloader prefers rapid's streamer, and BAR's returns an HTTP error:
/// `streamer.cgi?<md5>` fails with "Couldn't download files for <md5>". BAR
/// ships `prdRapidUseStreamer` defaulting to `"false"` for this reason.
pub const RAPID_USE_STREAMER: &str = "false";

impl Download {
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.env("PRD_RAPID_REPO_MASTER", RAPID_REPO_MASTER)
            .env("PRD_HTTP_SEARCH_URL", HTTP_SEARCH_URL)
            .env("PRD_RAPID_USE_STREAMER", RAPID_USE_STREAMER)
            .arg("--filesystem-writepath")
            .arg(&self.data_dir);
        for (want, name) in &self.wants {
            cmd.arg(want.flag()).arg(name);
        }
        cmd
    }
}

/// Splits pr-downloader's output into lines.
///
/// It redraws progress with carriage returns rather than newlines, so a reader
/// that only splits on `\n` sees one enormous line at the end and no progress
/// at all. Both count as a break here.
pub fn split_output(buffer: &mut String) -> Vec<String> {
    let Some(end) = buffer.rfind(['\r', '\n']) else {
        return Vec::new();
    };
    let complete: String = buffer.drain(..=end).collect();
    complete
        .split(['\r', '\n'])
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// How far along a download is, read off a `[Progress]` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub current: u64,
    pub total: u64,
}

impl Progress {
    /// pr-downloader ends a progress line with `<current>/<total>`. A total of
    /// zero or one means it has not worked out the size yet, which bar-lobby
    /// also skips rather than reporting as a percentage of nothing.
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        if !line.starts_with("[Progress]") {
            return None;
        }
        let (current, total) = line.rsplit_once(' ')?.1.split_once('/')?;
        let progress = Self {
            current: current.trim().parse().ok()?,
            total: total.trim().parse().ok()?,
        };
        (progress.total > 1).then_some(progress)
    }

    pub fn fraction(self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.current as f64 / self.total as f64
    }
}

#[cfg(test)]
mod download_tests {
    use super::*;

    #[test]
    fn one_invocation_carries_every_asset() {
        let download = Download {
            binary: "C:/e/pr-downloader.exe".into(),
            data_dir: "C:/bar".into(),
            wants: vec![
                (Want::Game, "Beyond All Reason test-31115".into()),
                (Want::Map, "Supreme Isthmus v2.1".into()),
            ],
        };
        let cmd = download.command();
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "--filesystem-writepath",
                "C:/bar",
                "--download-game",
                "Beyond All Reason test-31115",
                "--download-map",
                "Supreme Isthmus v2.1",
            ]
        );
    }

    #[test]
    fn the_cdn_is_named_because_the_default_one_has_no_bar_content() {
        let download = Download {
            binary: "prd".into(),
            data_dir: "C:/bar".into(),
            wants: vec![(Want::Map, "Pinewood_Derby_V1".into())],
        };
        let cmd = download.command();
        let env: Vec<_> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(env.contains(&(
            "PRD_RAPID_REPO_MASTER".into(),
            Some(RAPID_REPO_MASTER.into())
        )));
        assert!(env.contains(&("PRD_HTTP_SEARCH_URL".into(), Some(HTTP_SEARCH_URL.into()))));
        // Without this the rapid streamer is preferred, and BAR's fails.
        assert!(env.contains(&(
            "PRD_RAPID_USE_STREAMER".into(),
            Some(RAPID_USE_STREAMER.into())
        )));
    }

    #[test]
    fn progress_redrawn_with_carriage_returns_still_splits() {
        // What pr-downloader actually writes: one line, many updates.
        let mut buffer = String::from("[Progress] 10% 1/10\r[Progress] 50% 5/10\r");
        let lines = split_output(&mut buffer);
        assert_eq!(
            lines
                .iter()
                .filter_map(|l| Progress::parse(l))
                .collect::<Vec<_>>(),
            vec![
                Progress {
                    current: 1,
                    total: 10
                },
                Progress {
                    current: 5,
                    total: 10
                },
            ]
        );

        // A partial update is held back until its terminator arrives.
        buffer.push_str("[Progress] 60% 6/1");
        assert!(split_output(&mut buffer).is_empty());
        buffer.push_str("0\n");
        assert_eq!(
            split_output(&mut buffer)
                .iter()
                .filter_map(|l| Progress::parse(l))
                .collect::<Vec<_>>(),
            vec![Progress {
                current: 6,
                total: 10
            }]
        );
    }

    #[test]
    fn progress_is_read_off_the_trailing_byte_counts() {
        assert_eq!(
            Progress::parse("[Progress] 45% [==========>          ] 4500/10000"),
            Some(Progress {
                current: 4500,
                total: 10000
            })
        );
        assert_eq!(
            Progress::parse("[Progress] 45% [=====] 4500/10000").map(Progress::fraction),
            Some(0.45)
        );
    }

    #[test]
    fn a_size_it_does_not_know_yet_is_not_progress() {
        // Reporting a percentage of nothing would show a full bar at the start.
        assert_eq!(Progress::parse("[Progress] 0% [ ] 0/0"), None);
        assert_eq!(Progress::parse("[Progress] 0% [ ] 0/1"), None);
        assert_eq!(Progress::parse("Downloading something 1/2"), None);
        assert_eq!(Progress::parse(""), None);
    }
}

/// Every engine version installed, newest name first.
pub fn installed_engines(data_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(data_dir.join("engine")) else {
        return Vec::new();
    };
    let mut versions: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join(ENGINE_BINARY).is_file())
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            // The launcher names them `recoil_<version>`; anything else is
            // taken as the version itself, which is how `find_engine` reads them.
            Some(
                name.rsplit_once('_')
                    .map_or(name.clone(), |(_, v)| v.to_owned()),
            )
        })
        .collect();
    versions.sort();
    versions.dedup();
    versions.reverse();
    versions
}

/// The skirmish AIs any installed engine ships.
pub fn installed_ais(data_dir: &Path) -> Vec<String> {
    let Ok(engines) = std::fs::read_dir(data_dir.join("engine")) else {
        return Vec::new();
    };
    let mut ais: Vec<String> = engines
        .filter_map(Result::ok)
        .flat_map(|engine| {
            std::fs::read_dir(engine.path().join("AI").join("Skirmish"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter(|ai| ai.path().is_dir())
                .filter_map(|ai| ai.file_name().to_str().map(str::to_owned))
        })
        .collect();
    ais.sort();
    ais.dedup();
    ais
}
