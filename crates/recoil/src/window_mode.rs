//! Whether a game can be covered by another window, and what to do when it
//! cannot — without touching a file the user did not give us.
//!
//! An always-on-top window sits over a borderless game and loses to an
//! exclusive-fullscreen one, which takes the display mode with it. The engine's
//! own defaults are `Fullscreen = true`, `WindowBorderless = false`
//! (`GlobalRendering.cpp:73-74`) — exclusive — while BAR ships the opposite in
//! `springsettings.cfg:31,102`. So in practice the answer is usually already
//! the good one, and this whole module stays out of the way.
//!
//! When it is not, the fix is a launch flag, not an edit. `--config <file>` is
//! exclusive: `ConfigHandler::Instantiate` uses it *instead of* the default
//! locations (`ConfigHandler.cpp:420-422`), and `writableSource` is
//! `locations.front()` (`:87`), so the engine both reads and writes only that
//! file. Handing it a copy of our own leaves the user's `springsettings.cfg`
//! untouched — including by the engine, which otherwise rewrites it on the way
//! out. Someone trying modlobby and going back to Chobby finds their settings
//! exactly as they left them.
//!
//! The command line cannot do it alone: `--window` is inert, because
//! `SetFullScreen` assigns `fullScreen` twice and the second line discards the
//! flag (`GlobalRendering.cpp:1417-1418`). There is no environment variable —
//! the engine reads `SPRING_DATADIR` and `SPRING_ISOLATED`, and nothing that
//! reaches an individual config key.

use std::path::{Path, PathBuf};

/// How the engine will open its window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMode {
    /// A window, bordered or not. Another window can cover it.
    Windowed,
    /// Borderless full-screen — what BAR ships, and what the overlay wants.
    Borderless,
    /// A real display-mode change. Nothing reliably sits on top of this.
    Exclusive,
}

impl WindowMode {
    /// Whether an always-on-top window can be expected to cover the game.
    pub fn can_be_covered(self) -> bool {
        !matches!(self, WindowMode::Exclusive)
    }
}

/// The engine's config cascade, most specific first.
///
/// `ConfigLocater` prefers the versioned file when it exists and falls back to
/// the plain one (`ConfigLocater.cpp:54-58`), and the first definition of a
/// key wins (`ConfigHandler.cpp:415-434`).
fn candidates(data_dir: &Path, engine_version: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if !engine_version.is_empty() {
        paths.push(data_dir.join(format!("springsettings-{engine_version}.cfg")));
    }
    paths.push(data_dir.join("springsettings.cfg"));
    paths
}

/// Reads `key = value` the way the engine's config does: first definition
/// wins, `#` and `//` are comments, whitespace is not significant.
fn value_of(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(key) {
            return Some(value.trim().to_owned());
        }
    }
    None
}

fn truthy(value: &str) -> bool {
    !matches!(value.trim(), "0" | "false" | "False" | "")
}

/// How the engine in `data_dir` is configured to open its window.
pub fn window_mode(data_dir: &Path, engine_version: &str) -> WindowMode {
    let mut fullscreen = None;
    let mut borderless = None;

    for path in candidates(data_dir, engine_version) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // First file to define a key wins, so only fill what is still unknown.
        if fullscreen.is_none() {
            fullscreen = value_of(&text, "Fullscreen").map(|value| truthy(&value));
        }
        if borderless.is_none() {
            borderless = value_of(&text, "WindowBorderless").map(|value| truthy(&value));
        }
    }

    // The engine's own defaults when a key is absent everywhere.
    let fullscreen = fullscreen.unwrap_or(true);
    let borderless = borderless.unwrap_or(false);

    match (fullscreen, borderless) {
        (true, _) => WindowMode::Exclusive,
        (false, true) => WindowMode::Borderless,
        (false, false) => WindowMode::Windowed,
    }
}

/// Rewrites `text` so each key holds the wanted value, touching nothing else.
///
/// The first definition of a key is the one the engine reads, so that is the
/// one rewritten; later definitions of the same key are dropped rather than
/// left to contradict it. A key the file never mentions is appended.
fn set_keys(text: &str, wanted: &[(&str, &str)]) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut written = vec![false; wanted.len()];
    let mut out: Vec<String> = Vec::new();

    for line in text.lines() {
        let name = line
            .split_once('=')
            .map(|(name, _)| name.trim())
            .filter(|_| {
                let trimmed = line.trim_start();
                !trimmed.starts_with('#') && !trimmed.starts_with("//")
            });

        match wanted
            .iter()
            .position(|(key, _)| name.is_some_and(|name| name.eq_ignore_ascii_case(key)))
        {
            Some(index) if written[index] => {} // a later definition; the engine ignores it
            Some(index) => {
                let (key, value) = wanted[index];
                out.push(format!("{key} = {value}"));
                written[index] = true;
            }
            None => out.push(line.to_owned()),
        }
    }

    for (index, (key, value)) in wanted.iter().enumerate() {
        if !written[index] {
            out.push(format!("{key} = {value}"));
        }
    }

    let mut text = out.join(newline);
    text.push_str(newline);
    text
}

/// The user's settings as the engine would resolve them, flattened into one
/// file's worth of text.
///
/// `--config` is exclusive, so a copy has to carry everything, not just the
/// keys we care about. Later files contribute only the keys earlier ones did
/// not mention, which is the cascade's own first-definition-wins rule.
fn flattened(data_dir: &Path, engine_version: &str) -> String {
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<String> = Vec::new();

    for path in candidates(data_dir, engine_version) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }
            let Some((name, _)) = trimmed.split_once('=') else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            if seen.contains(&name) {
                continue;
            }
            seen.push(name);
            out.push(trimmed.to_owned());
        }
    }

    out.push(String::new());
    out.join("\n")
}

/// Where modlobby keeps the config it launches with, and what seeded it.
fn ours(config_dir: &Path) -> (PathBuf, PathBuf) {
    (
        config_dir.join("springsettings.cfg"),
        config_dir.join("springsettings.seed.cfg"),
    )
}

/// A config file to launch with that gets the game borderless, leaving the
/// user's own alone. `None` when their settings already allow an overlay.
///
/// Reseeded from their settings whenever those change, and otherwise left as
/// the engine last wrote it — so a graphics option changed inside a
/// modlobby-launched game survives the next launch, while a change made in
/// Chobby wins, being the more recent word on the subject.
pub fn borderless_config(
    data_dir: &Path,
    engine_version: &str,
    config_dir: &Path,
) -> std::io::Result<Option<PathBuf>> {
    if window_mode(data_dir, engine_version).can_be_covered() {
        return Ok(None);
    }

    let (config, seed) = ours(config_dir);
    let theirs = flattened(data_dir, engine_version);

    let unchanged = std::fs::read_to_string(&seed).is_ok_and(|last| last == theirs);
    if unchanged && config.is_file() {
        return Ok(Some(config));
    }

    std::fs::create_dir_all(config_dir)?;
    std::fs::write(&seed, &theirs)?;
    std::fs::write(
        &config,
        set_keys(&theirs, &[("Fullscreen", "0"), ("WindowBorderless", "1")]),
    )?;
    Ok(Some(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn what_bar_ships_can_be_covered() {
        let home = dir();
        std::fs::write(
            home.path().join("springsettings.cfg"),
            "Fullscreen = 0\nWindowBorderless = 1\n",
        )
        .unwrap();
        assert_eq!(
            window_mode(home.path(), "2026.07.04"),
            WindowMode::Borderless
        );
        assert!(window_mode(home.path(), "2026.07.04").can_be_covered());
    }

    #[test]
    fn no_config_at_all_is_the_engines_default_which_cannot_be_covered() {
        let home = dir();
        // `Fullscreen` defaults to true in the engine, so the honest answer for
        // a machine that has never run the game is "exclusive".
        assert_eq!(
            window_mode(home.path(), "2026.07.04"),
            WindowMode::Exclusive
        );
        assert!(!window_mode(home.path(), "2026.07.04").can_be_covered());
    }

    #[test]
    fn the_versioned_file_wins_where_it_speaks() {
        let home = dir();
        std::fs::write(
            home.path().join("springsettings.cfg"),
            "Fullscreen = 1\nWindowBorderless = 0\n",
        )
        .unwrap();
        std::fs::write(
            home.path().join("springsettings-2026.07.04.cfg"),
            "Fullscreen = 0\n",
        )
        .unwrap();

        // The versioned file settles `Fullscreen`; the plain one still answers
        // for `WindowBorderless`, which it did not mention.
        assert_eq!(window_mode(home.path(), "2026.07.04"), WindowMode::Windowed);
        // And a different engine version does not read that file at all.
        assert_eq!(
            window_mode(home.path(), "2025.01.01"),
            WindowMode::Exclusive
        );
    }

    #[test]
    fn comments_and_spacing_are_read_the_way_the_engine_reads_them() {
        let home = dir();
        std::fs::write(
            home.path().join("springsettings.cfg"),
            "# a comment\n// another\n\n   Fullscreen=0   \nWINDOWBORDERLESS = true\n",
        )
        .unwrap();
        assert_eq!(window_mode(home.path(), ""), WindowMode::Borderless);
    }

    #[test]
    fn the_first_definition_of_a_key_is_the_one_that_counts() {
        let home = dir();
        std::fs::write(
            home.path().join("springsettings.cfg"),
            "Fullscreen = 0\nWindowBorderless = 1\nFullscreen = 1\n",
        )
        .unwrap();
        assert_eq!(window_mode(home.path(), ""), WindowMode::Borderless);
    }

    #[test]
    fn nothing_is_launched_with_when_the_settings_already_work() {
        let home = dir();
        let mine = dir();
        std::fs::write(
            home.path().join("springsettings.cfg"),
            "Fullscreen = 0\nWindowBorderless = 1\n",
        )
        .unwrap();

        let config = borderless_config(home.path(), "2026.07.04", mine.path()).unwrap();
        assert_eq!(config, None, "no copy, no flag, nothing to explain");
        assert!(!mine.path().join("springsettings.cfg").exists());
    }

    #[test]
    fn the_copy_carries_their_settings_and_never_touches_their_file() {
        let home = dir();
        let mine = dir();
        let theirs = home.path().join("springsettings.cfg");
        let original = "Fullscreen = 1\r\nVSync = 2\r\nShadows = 1\r\n";
        std::fs::write(&theirs, original).unwrap();

        let config = borderless_config(home.path(), "2026.07.04", mine.path())
            .unwrap()
            .expect("a config to launch with");

        let copy = std::fs::read_to_string(&config).unwrap();
        assert!(copy.contains("Fullscreen = 0"));
        assert!(copy.contains("WindowBorderless = 1"));
        assert!(copy.contains("VSync = 2"), "and everything else they set");
        assert!(copy.contains("Shadows = 1"));

        // The whole point: going back to Chobby finds it as they left it.
        assert_eq!(std::fs::read_to_string(&theirs).unwrap(), original);
    }

    #[test]
    fn the_versioned_file_is_what_gets_copied_when_it_is_what_the_engine_reads() {
        let home = dir();
        let mine = dir();
        std::fs::write(
            home.path().join("springsettings.cfg"),
            "Fullscreen = 0\nVSync = 2\n",
        )
        .unwrap();
        std::fs::write(
            home.path().join("springsettings-2026.07.04.cfg"),
            "Fullscreen = 1\n",
        )
        .unwrap();

        let config = borderless_config(home.path(), "2026.07.04", mine.path())
            .unwrap()
            .expect("the versioned file forces fullscreen, so a copy is needed");

        let copy = std::fs::read_to_string(config).unwrap();
        assert_eq!(copy.matches("Fullscreen").count(), 1);
        assert!(copy.contains("Fullscreen = 0"));
        // The plain file still contributes the keys the versioned one is silent
        // about, exactly as the engine's cascade would.
        assert!(copy.contains("VSync = 2"));
    }

    #[test]
    fn a_setting_changed_inside_a_game_survives_the_next_launch() {
        let home = dir();
        let mine = dir();
        std::fs::write(home.path().join("springsettings.cfg"), "Fullscreen = 1\n").unwrap();

        let config = borderless_config(home.path(), "", mine.path())
            .unwrap()
            .unwrap();
        // The engine writes to the file it was handed; pretend it did.
        std::fs::write(
            &config,
            "Fullscreen = 0\nWindowBorderless = 1\nShadows = 2\n",
        )
        .unwrap();

        let again = borderless_config(home.path(), "", mine.path())
            .unwrap()
            .unwrap();
        assert_eq!(again, config);
        assert!(
            std::fs::read_to_string(&again)
                .unwrap()
                .contains("Shadows = 2"),
            "reseeding here would silently undo what they changed in-game"
        );
    }

    #[test]
    fn changing_a_setting_in_chobby_is_the_more_recent_word_and_wins() {
        let home = dir();
        let mine = dir();
        let theirs = home.path().join("springsettings.cfg");
        std::fs::write(&theirs, "Fullscreen = 1\nShadows = 1\n").unwrap();

        let config = borderless_config(home.path(), "", mine.path())
            .unwrap()
            .unwrap();
        assert!(
            std::fs::read_to_string(&config)
                .unwrap()
                .contains("Shadows = 1")
        );

        std::fs::write(&theirs, "Fullscreen = 1\nShadows = 3\n").unwrap();
        let again = borderless_config(home.path(), "", mine.path())
            .unwrap()
            .unwrap();

        let copy = std::fs::read_to_string(&again).unwrap();
        assert!(copy.contains("Shadows = 3"));
        assert!(copy.contains("Fullscreen = 0"), "and still borderless");
    }
}
