//! Launching the Recoil engine the way spring-launcher and bar-lobby do:
//! `spring --write-dir <data> --isolation <spring://… | script.txt>`.
//!
//! No I/O beyond reading the engine directory; spawning is the caller's job so
//! the command line stays unit-testable.

pub mod script;
pub mod script_read;
pub mod window_mode;

pub use window_mode::{WindowMode, window_mode};

use std::path::{Path, PathBuf};
use std::process::Command;

/// Engine binary inside an engine directory.
pub const ENGINE_BINARY: &str = if cfg!(windows) {
    "spring.exe"
} else {
    "spring"
};

/// Where the parts of an installed engine are.
///
/// On Windows and Linux an engine directory is flat: `spring`, `pr-downloader`,
/// `AI/` and `base/` all sit in it together. A macOS build is an application
/// bundle and splits those across `Contents/MacOS` and `Contents/Resources`,
/// with the libraries it links in `Contents/Frameworks`. Everything that
/// reaches into an engine goes through this, so exactly one place knows the
/// difference and a bundle cannot be half-handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineLayout {
    /// Holds `spring` and `pr-downloader`, and is the working directory.
    pub bin: PathBuf,
    /// Holds `AI/Skirmish`, `base/`, and the engine's own content.
    pub content: PathBuf,
    /// The bundle's libraries. `None` for a flat install, which needs none.
    pub frameworks: Option<PathBuf>,
}

impl EngineLayout {
    /// Reads an engine directory, or `None` where there is no engine in it.
    pub fn at(root: &Path) -> Option<Self> {
        if root.join(ENGINE_BINARY).is_file() {
            return Some(Self {
                bin: root.to_owned(),
                content: root.to_owned(),
                frameworks: None,
            });
        }
        Self::bundle(root).or_else(|| {
            std::fs::read_dir(root)
                .ok()?
                .filter_map(Result::ok)
                .find_map(|entry| Self::bundle(&entry.path()))
        })
    }

    /// The layout of an `.app`, whether it was named as one or found inside a
    /// version directory. Recognised by the engine being where a bundle puts
    /// it rather than by the extension, so an oddly named one still works.
    fn bundle(path: &Path) -> Option<Self> {
        let contents = path.join("Contents");
        let bin = contents.join("MacOS");
        if !bin.join(ENGINE_BINARY).is_file() {
            return None;
        }
        Some(Self {
            content: contents.join("Resources"),
            frameworks: Some(contents.join("Frameworks")),
            bin,
        })
    }

    /// A flat install, where everything is in the one directory. What every
    /// platform but macOS ships, and what a hand-assembled tree looks like.
    pub fn flat(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        Self {
            bin: dir.clone(),
            content: dir,
            frameworks: None,
        }
    }

    pub fn engine(&self) -> PathBuf {
        self.bin.join(ENGINE_BINARY)
    }

    pub fn downloader(&self) -> PathBuf {
        self.bin.join(DOWNLOADER_BINARY)
    }

    /// The engine version a bundle declares, when it declares one.
    ///
    /// A bundle knows what it is -- the Apple Silicon build writes
    /// `EngineVersion` into its `Info.plist` beside its own port version -- so
    /// a person can drop the app into the engine folder under whatever name
    /// they like and be understood, rather than having to rename it to the
    /// `recoil_<version>` the launcher happens to use. A flat install has
    /// nothing to read and keeps being named by its directory.
    pub fn declared_version(&self) -> Option<String> {
        let plist = self.frameworks.as_ref()?.parent()?.join("Info.plist");
        plist_string(&std::fs::read_to_string(plist).ok()?, "EngineVersion")
    }

    /// Whether the engine and its content are in different places, which is
    /// only true of a bundle.
    pub fn bundled(&self) -> bool {
        self.frameworks.is_some()
    }

    /// What a bundled engine needs in its environment to draw anything.
    ///
    /// The macOS build has no native OpenGL 4.6, so it renders through zink on
    /// Vulkan on Metal and needs to be told so: the driver, the Vulkan ICD to
    /// load and the version to claim. Its own launcher script sets exactly this
    /// before running the engine, and an engine started without it comes up
    /// blank. A flat install needs none of it, so this is empty there and the
    /// same code path serves every platform.
    ///
    /// MoltenVK is preferred over KosmicKrisp, which needs Metal 4 and so
    /// macOS 26; MoltenVK works everywhere the build does. The two MVK settings
    /// are the bundle author's, against a command pool that is not thread safe
    /// under threaded submission and argument buffers that hold plain pointers.
    pub fn environment(&self) -> Vec<(&'static str, std::ffi::OsString)> {
        let Some(frameworks) = self.frameworks.as_ref() else {
            return Vec::new();
        };
        let icds = self.content.join("vulkan").join("icd.d");
        let icd = ["moltenvk_icd.json", "kosmickrisp_mesa_icd.aarch64.json"]
            .into_iter()
            .map(|name| icds.join(name))
            .find(|path| path.is_file());
        let mut env: Vec<(&'static str, std::ffi::OsString)> = vec![
            ("EGL_PLATFORM", "surfaceless".into()),
            ("GALLIUM_DRIVER", "zink".into()),
            ("MESA_LOADER_DRIVER_OVERRIDE", "zink".into()),
            ("MESA_GL_VERSION_OVERRIDE", "4.6".into()),
            ("DYLD_FALLBACK_LIBRARY_PATH", frameworks.clone().into()),
        ];
        if let Some(icd) = icd {
            let moltenvk = icd
                .file_name()
                .is_some_and(|name| name == "moltenvk_icd.json");
            env.push(("VK_ICD_FILENAMES", icd.clone().into()));
            env.push(("VK_DRIVER_FILES", icd.into()));
            if moltenvk {
                env.push(("MVK_CONFIG_USE_COMMAND_POOLING", "0".into()));
                env.push(("MVK_CONFIG_USE_METAL_ARGUMENT_BUFFERS", "0".into()));
            }
        }
        env
    }
}

/// The string value of `key` in an XML property list.
///
/// A plist is XML, but the two lines that matter here are
/// `<key>Name</key><string>value</string>` and nothing about them needs a
/// parser: a whole XML dependency to read one version out of one file would be
/// the larger mistake. Whitespace and a line break between the two tags are
/// allowed for, since Apple's own tools write it both ways.
fn plist_string(plist: &str, key: &str) -> Option<String> {
    let after = plist.split_once(&format!("<key>{key}</key>"))?.1;
    let open = after.find("<string>")? + "<string>".len();
    let close = after[open..].find("</string>")? + open;
    // Only when the value really is the next tag, not some later key's.
    if after[..open].contains("<key>") {
        return None;
    }
    Some(after[open..close].trim().to_owned())
}

/// What the engine is handed to join somebody else's hosted game, rather than
/// a start script or a replay of its own.
pub const JOIN_SCHEME: &str = "spring://";

/// Whether the engine on this platform may be run against a hosted game.
///
/// False on macOS. Beyond All Reason publishes no Apple engine, so the only
/// one that exists is a third-party build whose author asks that it not reach
/// the community servers until they approve it — and enforces that by
/// neutering Chobby's server address, which modlobby never reads. Chatting in
/// a room costs the servers nothing and is left alone; *playing* is what this
/// stops, and it stops it at the one place the engine is started rather than
/// by hiding buttons, so no path can arrive at it by another route.
pub const fn may_join_hosted_games() -> bool {
    cfg!(not(target_os = "macos"))
}

/// Whether this target is somebody else's hosted game.
pub fn is_hosted_game(target: &str) -> bool {
    target.starts_with(JOIN_SCHEME)
}

/// Why the engine must not be started on `target`, when it must not.
///
/// Split from the platform answer so it can be exercised both ways from any
/// machine: an invariant only ever tested on the platform it fires on is one
/// nobody notices breaking.
pub fn refuse_target(target: &str, may_join: bool) -> Option<String> {
    if !is_hosted_game(target) || may_join {
        return None;
    }
    Some(
        "this build cannot join hosted games: the only Beyond All Reason engine for macOS is a          third-party build its author asks not be used on the community servers. Skirmish          against AI and replays still run."
            .to_owned(),
    )
}

/// `spring://<user>:<script password>@<host>:<port>` — what Chobby hands the
/// engine to join a hosted game (`liblobby/lobby/lobby.lua` `ConnectToBattle`,
/// parsed in `rts/System/SpringApp.cpp`).
pub fn spring_url(username: &str, script_password: &str, host: &str, port: u16) -> String {
    format!("{JOIN_SCHEME}{username}:{script_password}@{host}:{port}")
}

/// The engine under `<data>/engine` holding `version`, as the BAR launcher
/// names them (`2026.07.04` → `recoil_2026.07.04`), and only if it has a binary.
pub fn find_engine(data_dir: &Path, version: &str) -> Option<EngineLayout> {
    let suffix = format!("_{version}");
    std::fs::read_dir(data_dir.join("engine"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(|path| {
            let named = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let named = named == version || named.ends_with(&suffix);
            EngineLayout::at(&path).map(|layout| (named, layout))
        })
        .find(|(named, layout)| *named || layout.declared_version().as_deref() == Some(version))
        .map(|(_, layout)| layout)
}

/// Every engine installed under `<data>/engine`, in directory order.
fn engine_layouts(data_dir: &Path) -> Vec<(std::ffi::OsString, EngineLayout)> {
    let Ok(entries) = std::fs::read_dir(data_dir.join("engine")) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            EngineLayout::at(&entry.path()).map(|layout| (entry.file_name(), layout))
        })
        .collect()
}

/// One engine invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub engine: EngineLayout,
    /// The BAR data directory (`--write-dir`); `--isolation` keeps the engine from reading anything else.
    pub data_dir: PathBuf,
    /// Directories the engine may read and never writes: other lobbies'
    /// installs. `SPRING_DATADIR` is honoured even under `--isolation`
    /// (`DataDirLocater::LocateDataDirs`, level 3), and only the first
    /// directory located, the write dir, is ever written to.
    pub read_dirs: Vec<PathBuf>,
    /// A `spring://` URL or a start-script path.
    pub target: String,
    /// `--config`, when the user's own settings would defeat the overlay.
    ///
    /// Exclusive: the engine reads and writes only this file, which is how
    /// their `springsettings.cfg` survives a modlobby launch untouched. See
    /// [`window_mode::borderless_config`].
    pub config: Option<PathBuf>,
    /// `--menu`, when there is a menu archive to launch against.
    pub menu: Option<MenuArchive>,
}

/// A LuaMenu archive to start the engine with.
///
/// Carries where it is as well as what it is called, because the two have to
/// agree before the name reaches the command line: `--menu` naming an archive
/// the scanner did not find throws a `content_error` out of `Init`
/// (`VFSHandler.cpp:215`) and the engine never opens a window. A game that
/// will not start is a far worse failure than a missing button, so the check
/// happens here, at the point of use, rather than wherever the archive was
/// written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuArchive {
    /// The `name` from its `modinfo.lua`, which is what the scanner indexes.
    pub name: String,
    pub dir: PathBuf,
}

impl MenuArchive {
    /// Whether the engine will find it. Only `modinfo.lua` matters: without
    /// one the scanner does not index the directory at all.
    pub fn found(&self) -> bool {
        self.dir.join("modinfo.lua").is_file()
    }
}

impl Launch {
    /// Mirrors `bar-lobby/src/main/game/game.ts`.
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(self.engine.engine());
        cmd.current_dir(&self.engine.bin)
            .arg("--write-dir")
            .arg(&self.data_dir)
            .arg("--isolation");
        // `join_paths` uses the same separator the engine splits on: `;` on
        // Windows, `:` elsewhere. A bundled engine carries its own base
        // content and skirmish AIs inside it, so the bundle is a read
        // directory of its own -- last, because ours comes first.
        let read_dirs = self
            .read_dirs
            .iter()
            .cloned()
            .chain(self.engine.bundled().then(|| self.engine.content.clone()));
        if let Ok(read_dirs) = std::env::join_paths(read_dirs)
            && !read_dirs.is_empty()
        {
            cmd.env("SPRING_DATADIR", read_dirs);
        }
        // Blank without it, on a build that renders through zink.
        for (key, value) in self.engine.environment() {
            cmd.env(key, value);
        }
        if let Some(config) = &self.config {
            cmd.arg("--config").arg(config);
        }
        if let Some(menu) = self.menu.as_ref().filter(|menu| menu.found()) {
            cmd.arg("--menu").arg(&menu.name);
        }
        cmd.arg(&self.target);
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a macOS-shaped engine: an `.app` inside the version directory,
    /// with the binary, the content and the libraries in their three places.
    fn bundled_engine(root: &Path) -> PathBuf {
        let app = root
            .join("engine")
            .join("recoil_2026.07.04")
            .join("BAR Launcher.app");
        let contents = app.join("Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::create_dir_all(contents.join("Resources").join("vulkan").join("icd.d")).unwrap();
        std::fs::create_dir_all(contents.join("Frameworks")).unwrap();
        std::fs::write(contents.join("MacOS").join(ENGINE_BINARY), b"").unwrap();
        std::fs::write(
            contents
                .join("Resources")
                .join("vulkan")
                .join("icd.d")
                .join("moltenvk_icd.json"),
            b"{}",
        )
        .unwrap();
        app
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("recoil-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    /// The real Info.plist the Apple Silicon build ships, trimmed to the keys
    /// that matter here -- note the port version comes first, so reading the
    /// wrong one is a live mistake.
    const BUNDLE_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>launcher</string>
  <key>CFBundleShortVersionString</key><string>0.15.0</string>
  <key>EngineVersion</key><string>2026.07.04</string>
  <key>PortVersion</key><string>0.15.0</string>
</dict></plist>"#;

    #[test]
    fn a_bundle_says_which_engine_it_holds() {
        assert_eq!(
            plist_string(BUNDLE_PLIST, "EngineVersion").as_deref(),
            Some("2026.07.04")
        );
        assert_eq!(
            plist_string(BUNDLE_PLIST, "CFBundleShortVersionString").as_deref(),
            Some("0.15.0")
        );
        assert_eq!(plist_string(BUNDLE_PLIST, "Nothing"), None);
        // A key whose value is missing must not borrow the next key's.
        assert_eq!(
            plist_string(
                "<key>Alone</key><key>Other</key><string>x</string>",
                "Alone"
            ),
            None
        );
    }

    /// Dropped in under any name and still understood, which is the whole
    /// point: nobody should have to rename an app to `recoil_2026.07.04`.
    #[test]
    fn a_bundle_is_found_by_the_version_it_declares_not_its_folder() {
        let root = scratch("declared");
        let app = bundled_engine(&root);
        // Rename the version directory to something meaningless.
        let odd = root.join("engine").join("dropped-here");
        std::fs::rename(app.parent().unwrap(), &odd).unwrap();
        std::fs::write(
            odd.join("BAR Launcher.app")
                .join("Contents")
                .join("Info.plist"),
            BUNDLE_PLIST,
        )
        .unwrap();

        assert_eq!(installed_engines(&root), ["2026.07.04"]);
        assert!(
            find_engine(&root, "2026.07.04").is_some(),
            "found by what it says it is"
        );
        assert!(find_engine(&root, "2020.01.01").is_none());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_flat_engine_keeps_everything_in_one_place() {
        let root = scratch("flat");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(ENGINE_BINARY), b"").unwrap();

        let layout = EngineLayout::at(&root).expect("an engine");
        assert_eq!(layout, EngineLayout::flat(&root));
        assert!(!layout.bundled());
        assert!(
            layout.environment().is_empty(),
            "nothing to say about drawing on a platform with its own OpenGL"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The macOS build is an application bundle, so the binary, the content and
    /// the libraries are three different directories.
    #[test]
    fn a_bundled_engine_is_found_inside_its_app() {
        let root = scratch("bundle");
        let app = bundled_engine(&root);

        let layout = find_engine(&root, "2026.07.04").expect("an engine in the bundle");
        assert_eq!(
            layout.engine(),
            app.join("Contents").join("MacOS").join(ENGINE_BINARY)
        );
        assert_eq!(
            layout.downloader(),
            app.join("Contents").join("MacOS").join(DOWNLOADER_BINARY)
        );
        assert_eq!(layout.content, app.join("Contents").join("Resources"));
        assert!(layout.bundled());
        // And it is listed as an installed version like any other.
        assert_eq!(installed_engines(&root), ["2026.07.04"]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Without this the engine comes up blank: the macOS build has no native
    /// OpenGL 4.6 and renders through zink on Vulkan on Metal.
    #[test]
    fn a_bundled_engine_carries_its_graphics_environment() {
        let root = scratch("env");
        let app = bundled_engine(&root);
        let layout = find_engine(&root, "2026.07.04").unwrap();

        let env: std::collections::HashMap<_, _> = layout.environment().into_iter().collect();
        assert_eq!(env.get("GALLIUM_DRIVER").unwrap(), "zink");
        assert_eq!(env.get("MESA_GL_VERSION_OVERRIDE").unwrap(), "4.6");
        assert_eq!(
            env.get("DYLD_FALLBACK_LIBRARY_PATH").unwrap(),
            app.join("Contents").join("Frameworks").as_os_str()
        );
        // MoltenVK is preferred, and brings its own two corrections with it.
        assert!(
            env.get("VK_ICD_FILENAMES")
                .unwrap()
                .to_string_lossy()
                .ends_with("moltenvk_icd.json")
        );
        assert_eq!(env.get("MVK_CONFIG_USE_COMMAND_POOLING").unwrap(), "0");

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The bundle carries the base content and the skirmish AIs inside it, so
    /// the engine has to be told to read from itself.
    #[test]
    fn a_bundle_is_a_read_directory_of_its_own() {
        let root = scratch("readdirs");
        let app = bundled_engine(&root);
        let launch = Launch {
            engine: find_engine(&root, "2026.07.04").unwrap(),
            data_dir: root.join("write"),
            read_dirs: vec![root.join("theirs")],
            target: "script.txt".into(),
            config: None,
            menu: None,
        };

        let cmd = launch.command();
        let (_, datadir) = cmd
            .get_envs()
            .filter_map(|(k, v)| v.map(|v| (k, v)))
            .find(|(k, _)| *k == "SPRING_DATADIR")
            .expect("read directories");
        let dirs: Vec<PathBuf> = std::env::split_paths(datadir).collect();
        assert_eq!(
            dirs,
            vec![root.join("theirs"), app.join("Contents").join("Resources")],
            "ours first, the bundle last"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A directory of configuration with no library for this platform is an AI
    /// that fails at game start; not offering it is the honest answer.
    #[test]
    fn an_ai_without_a_library_is_not_offered() {
        let root = scratch("ais");
        bundled_engine(&root);
        let skirmish = root
            .join("engine")
            .join("recoil_2026.07.04")
            .join("BAR Launcher.app")
            .join("Contents")
            .join("Resources")
            .join("AI")
            .join("Skirmish");
        std::fs::create_dir_all(skirmish.join("BARb").join("stable")).unwrap();
        std::fs::create_dir_all(skirmish.join("Ghost").join("stable")).unwrap();
        std::fs::write(skirmish.join("BARb").join("stable").join(AI_LIBRARY), b"").unwrap();

        assert_eq!(installed_ais(&root), ["BARb"], "Ghost has no library");

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The invariant the macOS support rests on: a hosted game never starts
    /// the engine where the engine may not join one, and everything local
    /// still does.
    #[test]
    fn a_hosted_game_is_refused_where_the_engine_may_not_join_one() {
        let url = spring_url("me", "4242", "1.2.3.4", 8452);
        assert!(is_hosted_game(&url));
        assert!(refuse_target(&url, false).is_some());
        assert!(
            refuse_target(&url, true).is_none(),
            "allowed everywhere else"
        );
    }

    #[test]
    fn a_skirmish_and_a_replay_are_this_machines_own_business() {
        for target in [
            "C:/data/modlobby-skirmish.txt",
            "/Users/ann/Library/Application Support/modlobby/data/demos/x.sdfz",
        ] {
            assert!(!is_hosted_game(target));
            assert!(
                refuse_target(target, false).is_none(),
                "{target} needs no server"
            );
        }
    }

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

        assert_eq!(
            find_engine(&root, "2026.07.04"),
            Some(EngineLayout::flat(engine))
        );
        assert_eq!(find_engine(&root, "2025.04.01"), None, "no binary");
        assert_eq!(find_engine(&root, "1999.01.01"), None);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn command_mirrors_bar_lobby() {
        let launch = Launch {
            engine: EngineLayout::flat("C:/e"),
            data_dir: "C:/d".into(),
            read_dirs: Vec::new(),
            target: "spring://me:1@h:2".into(),
            config: None,
            menu: None,
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
        assert!(
            cmd.get_envs().all(|(key, _)| key != "SPRING_DATADIR"),
            "nothing to read from, nothing to tell the engine"
        );
    }

    #[test]
    fn other_installs_are_handed_to_the_engine_as_read_only_data_dirs() {
        let launch = Launch {
            engine: EngineLayout::flat("C:/e"),
            data_dir: "C:/d".into(),
            read_dirs: vec!["/launcher".into(), "/bar-lobby".into()],
            target: "spring://me:1@h:2".into(),
            config: None,
            menu: None,
        };
        let cmd = launch.command();
        let datadir = cmd
            .get_envs()
            .find(|(key, _)| *key == "SPRING_DATADIR")
            .and_then(|(_, value)| value)
            .expect("SPRING_DATADIR");
        let separator = if cfg!(windows) { ";" } else { ":" };
        assert_eq!(
            datadir.to_string_lossy(),
            format!("/launcher{separator}/bar-lobby")
        );
        // The write dir is still the only `--write-dir`.
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[..2], ["--write-dir", "C:/d"]);
    }

    /// The engine dies during startup on a menu it cannot find, so a name only
    /// reaches the command line once its archive is on disk.
    #[test]
    fn a_menu_the_scanner_would_not_find_is_left_off() {
        let root = std::env::temp_dir().join("modlobby-menu-arg-test");
        let archive = root.join("games").join("shim.sdd");
        let _ = std::fs::remove_dir_all(&root);

        let menu = MenuArchive {
            name: "modlobby chobby shim".into(),
            dir: archive.clone(),
        };
        assert!(!menu.found(), "nothing written yet");

        let launch = Launch {
            engine: EngineLayout::flat("C:/e"),
            data_dir: "C:/d".into(),
            read_dirs: Vec::new(),
            target: "spring://me:1@h:2".into(),
            config: None,
            menu: Some(menu.clone()),
        };
        let args = |launch: &Launch| -> Vec<String> {
            launch
                .command()
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        assert!(!args(&launch).contains(&"--menu".to_string()));

        std::fs::create_dir_all(&archive).unwrap();
        std::fs::write(archive.join("modinfo.lua"), "return {}").unwrap();
        assert!(menu.found());
        let with = args(&launch);
        assert!(with.contains(&"--menu".to_string()));
        assert!(with.contains(&"modlobby chobby shim".to_string()));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_borderless_config_goes_on_the_command_line_before_the_target() {
        let launch = Launch {
            engine: EngineLayout::flat("C:/e"),
            data_dir: "C:/d".into(),
            read_dirs: Vec::new(),
            target: "spring://me:1@h:2".into(),
            config: Some("C:/mine/springsettings.cfg".into()),
            menu: None,
        };
        let args: Vec<String> = launch
            .command()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--write-dir",
                "C:/d",
                "--isolation",
                "--config",
                "C:/mine/springsettings.cfg",
                "spring://me:1@h:2",
            ]
        );
    }
}

/// What a loadable skirmish AI is called on this platform.
pub const AI_LIBRARY: &str = if cfg!(windows) {
    "SkirmishAI.dll"
} else if cfg!(target_os = "macos") {
    "libSkirmishAI.dylib"
} else {
    "libSkirmishAI.so"
};

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
    let preferred = find_engine(data_dir, version).map(|engine| engine.downloader());
    if let Some(path) = preferred
        && path.is_file()
    {
        return Some(path);
    }

    engine_layouts(data_dir)
        .into_iter()
        .map(|(_, engine)| engine.downloader())
        .find(|path| path.is_file())
}

/// Gives the two binaries modlobby spawns their executable bit back.
///
/// An engine archive is 7z, which carries Windows attributes only, so an
/// unpacked `spring` and `pr-downloader` arrive as plain files on Unix. The
/// shared objects beside them are `dlopen`ed and need only reading. A binary
/// the archive did not ship is not an error here; launching reports that.
#[cfg(unix)]
pub fn mark_executable(engine_dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for binary in [ENGINE_BINARY, DOWNLOADER_BINARY] {
        let path = engine_dir.join(binary);
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        std::fs::set_permissions(&path, permissions)?;
    }
    Ok(())
}

/// Windows has no executable bit; the archive's files run as unpacked.
#[cfg(not(unix))]
pub fn mark_executable(_engine_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
mod executable_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_unpacked_binaries_become_executable() {
        let engine = tempfile::tempdir().unwrap();
        for name in [ENGINE_BINARY, "libunitsync.so"] {
            let path = engine.path().join(name);
            std::fs::write(&path, b"").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }

        mark_executable(engine.path()).unwrap();

        assert_eq!(mode(&engine.path().join(ENGINE_BINARY)), 0o755);
        assert_eq!(mode(&engine.path().join("libunitsync.so")), 0o644);
        assert!(!engine.path().join(DOWNLOADER_BINARY).exists());
    }
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

/// The variable pr-downloader reads its own request rate limit from.
pub const MAX_REQS_ENV: &str = "PRD_MAX_HTTP_REQS_PER_SEC";

/// The most requests a second modlobby will make of BAR's servers through
/// pr-downloader.
///
/// A guard against a runaway, not a "never faster than this" -- and the
/// arithmetic that separates the two is worth having in view. With the
/// streamer off a game arrives as one HTTP request per rapid pool file: a BAR
/// install is some 36 000 of them averaging 56 KB, so a limit on request
/// *starts* is a limit on bandwidth of about 56 KB times the number. At 200 a
/// second that was 11 MB/s, a throttle on any fibre line. At 3 000 it is
/// 170 MB/s, about 1.3 Gbit/s -- past what a residential line does, so a
/// first install runs at whatever the link allows -- while a retry storm or a
/// loop reinvoking the downloader is still bounded to something the CDN can
/// absorb. bar-lobby sets no limit at all, so this makes modlobby gentler than
/// the official client without making it slower.
pub const MAX_HTTP_REQS_PER_SEC: u32 = 3_000;

/// The limit to hand the child, never raising one somebody already set.
///
/// Not a plain minimum, because pr-downloader spells "no limit" `0`
/// (`Throttler::get_token` returns true whenever the rate is zero), and the
/// smallest of `0` and a ceiling is no ceiling. It also reads anything it
/// cannot parse as unlimited -- `getMaxReqsPerSecLimit` returns `false` from a
/// function returning `unsigned` -- so a value we do not understand is replaced
/// rather than passed on, and the child always gets a number it will honour.
///
/// Takes the setting rather than reading it, so both the raising and the
/// lowering can be tested without a process to set variables on.
pub fn max_reqs_per_sec(existing: Option<&str>) -> u32 {
    existing
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|limit| *limit > 0)
        .map_or(MAX_HTTP_REQS_PER_SEC, |limit| {
            limit.min(MAX_HTTP_REQS_PER_SEC)
        })
}

impl Download {
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.env("PRD_RAPID_REPO_MASTER", RAPID_REPO_MASTER)
            .env("PRD_HTTP_SEARCH_URL", HTTP_SEARCH_URL)
            .env("PRD_RAPID_USE_STREAMER", RAPID_USE_STREAMER)
            .env(
                MAX_REQS_ENV,
                max_reqs_per_sec(std::env::var(MAX_REQS_ENV).ok().as_deref()).to_string(),
            )
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
        // A rate limit always goes over, and it is never pr-downloader's
        // spelling of "unlimited" -- which is what an absent or unreadable one
        // would leave it with.
        let limit: u32 = env
            .iter()
            .find(|(key, _)| key == MAX_REQS_ENV)
            .and_then(|(_, value)| value.as_deref())
            .expect("a rate limit is handed over")
            .parse()
            .expect("a number pr-downloader can read");
        assert!((1..=MAX_HTTP_REQS_PER_SEC).contains(&limit));
    }

    /// The ceiling is one, not a floor and not a replacement: somebody who has
    /// asked for less traffic than modlobby would make keeps their answer.
    #[test]
    fn the_request_rate_is_capped_without_raising_a_lower_one() {
        // Every way of saying "no limit" to pr-downloader becomes the ceiling:
        // unset, its own zero, and anything it would fail to parse -- which it
        // treats as zero rather than as an error.
        assert_eq!(max_reqs_per_sec(None), MAX_HTTP_REQS_PER_SEC);
        assert_eq!(max_reqs_per_sec(Some("0")), MAX_HTTP_REQS_PER_SEC);
        assert_eq!(max_reqs_per_sec(Some("")), MAX_HTTP_REQS_PER_SEC);
        assert_eq!(
            max_reqs_per_sec(Some("as fast as you like")),
            MAX_HTTP_REQS_PER_SEC
        );
        assert_eq!(max_reqs_per_sec(Some("-1")), MAX_HTTP_REQS_PER_SEC);
        // Above it comes down, below it is left alone.
        assert_eq!(max_reqs_per_sec(Some("100000")), MAX_HTTP_REQS_PER_SEC);
        assert_eq!(max_reqs_per_sec(Some(" 5 ")), 5);
        assert_eq!(max_reqs_per_sec(Some("1")), 1);
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
    let mut versions: Vec<String> = engine_layouts(data_dir)
        .into_iter()
        .filter_map(|(dir, layout)| {
            if let Some(declared) = layout.declared_version() {
                return Some(declared);
            }
            let name = dir.to_str()?.to_owned();
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

/// Where an engine AI declares its own options.
///
/// `<engine>/AI/Skirmish/<name>/<version>/AIOptions.lua`, of which the version
/// is a directory the AI names itself (`stable`, `0.1`). The first found is
/// taken: an engine ships one build of each AI, and a second would be an
/// install somebody assembled by hand.
///
/// The file is the same `local options = { … }` table a game's `modoptions.lua`
/// is, so whatever reads one reads the other.
pub fn ai_options_file(content_dir: &Path, ai: &str) -> Option<PathBuf> {
    let versions = std::fs::read_dir(content_dir.join("AI").join("Skirmish").join(ai)).ok()?;
    versions
        .filter_map(Result::ok)
        .map(|version| version.path().join("AIOptions.lua"))
        .find(|path| path.is_file())
}

/// The skirmish AIs any installed engine ships.
pub fn installed_ais(data_dir: &Path) -> Vec<String> {
    let mut ais: Vec<String> = engine_layouts(data_dir)
        .into_iter()
        .flat_map(|(_, engine)| {
            std::fs::read_dir(engine.content.join("AI").join("Skirmish"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter(|ai| has_ai_library(&ai.path()))
                .filter_map(|ai| ai.file_name().to_str().map(str::to_owned))
        })
        .collect();
    ais.sort();
    ais.dedup();
    ais
}

/// Whether an AI directory holds a library this platform can load.
///
/// The directory alone means nothing: an engine can ship `AI/Skirmish/BARb`
/// with configuration and scripts and no build for the platform in hand, and
/// listing it then offers a choice that fails at game start rather than at the
/// picker. The engine loads `SkirmishAI.dll` / `libSkirmishAI.dylib` /
/// `libSkirmishAI.so` from the AI's own version directory
/// (`AI/Interfaces/C/src/Interface.cpp`).
fn has_ai_library(ai_dir: &Path) -> bool {
    let Ok(versions) = std::fs::read_dir(ai_dir) else {
        return false;
    };
    versions
        .filter_map(Result::ok)
        .any(|version| version.path().join(AI_LIBRARY).is_file())
}
