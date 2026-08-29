//! Launching the Recoil engine the way spring-launcher and bar-lobby do:
//! `spring --write-dir <data> --isolation <spring://… | script.txt>`.
//!
//! No I/O beyond reading the engine directory; spawning is the caller's job so
//! the command line stays unit-testable.

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
