//! Where a modded game lives on this machine, and what a confined engine may
//! touch: its own write directory beside the normal one, the builds of the
//! mods it loads, and a policy over the content it reads.
//!
//! Nothing a mod writes is ever read by an unconfined launch: the mods write
//! directory is never a read directory of a normal one, and what the engine
//! scans for code or content is wiped before every modded launch.

use std::io;
use std::path::{Path, PathBuf};

use sandbox::{CONTENT_SUBDIRS, Network, Policy, Root, scrubbed_env};

/// The two directories of modded play, beside the normal data directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModsLayout {
	/// The one directory a confined engine writes: `<parent>/mods-data`.
	pub write: PathBuf,
	/// Where the builds of pinned mods go, read-only to the engine:
	/// `<parent>/mod-builds`.
	pub builds: PathBuf,
	/// The views a confined engine reads instead of the real data roots:
	/// `<parent>/mods-view/<n>`, each holding only links to one root's
	/// content subdirectories.
	pub views: PathBuf,
	/// The record of every grant made for the container, beside the data
	/// it covers: one per Windows user, whatever config dir is in use.
	pub grants: PathBuf,
}

pub fn layout(own_data_dir: &Path) -> ModsLayout {
	let parent = own_data_dir.parent().unwrap_or(own_data_dir);
	ModsLayout {
		write: parent.join("mods-data"),
		builds: parent.join("mod-builds"),
		views: parent.join("mods-view"),
		grants: parent.join("sandbox-grants.json"),
	}
}

/// Moves builds of pinned mods out of `write/games`, where modlobby put
/// them before the builds root existed and where an open launch could
/// resolve them by name, into the builds root.
/// `dirs` with the builds root among the read directories, for a content
/// check: never for a launch, where an open engine must not resolve a
/// build by name.
pub fn with_builds(dirs: &content::DataDirs) -> content::DataDirs {
	let builds = layout(&dirs.write).builds;
	let mut dirs = dirs.clone();
	if !dirs.read.contains(&builds) {
		dirs.read.push(builds);
	}
	dirs
}

pub fn adopt_builds(write: &Path, mods: &ModsLayout) -> io::Result<()> {
	let Ok(entries) = std::fs::read_dir(write.join("games")) else {
		return Ok(());
	};
	for entry in entries.filter_map(Result::ok) {
		let name = entry.file_name();
		let Some(text) = name.to_str() else {
			continue;
		};
		if !content::git::is_build_name(text) {
			continue;
		}
		let into = mods.builds.join("games");
		std::fs::create_dir_all(&into)?;
		let target = into.join(&name);
		if target.exists() {
			std::fs::remove_dir_all(entry.path())?;
			continue;
		}
		std::fs::rename(entry.path(), target)?;
	}
	Ok(())
}

/// One real data root as a confined engine sees it: a directory of ours
/// holding a link per content subdirectory the root has, and nothing else.
/// The engine opens every directory it finds under a root and makes a
/// fatal of one it cannot open; a view has none it cannot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
	pub dir: PathBuf,
	/// The real content directories the links lead to.
	pub subdirs: Vec<PathBuf>,
}

/// The views for `roots`, made afresh under `mods.views`: a root that is
/// missing or has no content subdirectory gets none. The real roots are
/// never written to; only the links are made and removed.
pub fn views(mods: &ModsLayout, roots: impl IntoIterator<Item = PathBuf>) -> io::Result<Vec<View>> {
	// std's remove_dir_all unlinks a junction rather than descending into it.
	match std::fs::remove_dir_all(&mods.views) {
		Ok(()) => {}
		Err(err) if err.kind() == io::ErrorKind::NotFound => {}
		Err(err) => return Err(err),
	}
	let mut seen: Vec<PathBuf> = Vec::new();
	let mut views = Vec::new();
	for real in roots {
		if seen.contains(&real) || !real.is_dir() {
			continue;
		}
		seen.push(real.clone());
		let subdirs: Vec<PathBuf> = CONTENT_SUBDIRS
			.iter()
			.map(|sub| real.join(sub))
			.filter(|dir| dir.is_dir())
			.collect();
		if subdirs.is_empty() {
			continue;
		}
		let dir = mods.views.join(views.len().to_string());
		std::fs::create_dir_all(&dir)?;
		for target in &subdirs {
			let name = target
				.file_name()
				.expect("a content subdirectory has a name");
			link(&dir.join(name), target)?;
		}
		views.push(View { dir, subdirs });
	}
	Ok(views)
}

#[cfg(windows)]
fn link(at: &Path, target: &Path) -> io::Result<()> {
	junction::create(target, at)
}

#[cfg(not(windows))]
fn link(at: &Path, target: &Path) -> io::Result<()> {
	std::os::unix::fs::symlink(target, at)
}

/// The directories that survive a reset of the mods write directory, each
/// as a directory and nothing else.
pub const KEPT_DIRS: &[&str] = &["cache", "demos", "fontcache"];
/// The files that survive, each as a file and nothing else.
pub const KEPT_FILES: &[&str] = &["infolog.txt", "uikeys.txt"];

/// Removes everything from `write` but [`KEPT_DIRS`] and [`KEPT_FILES`],
/// and a keeper that is not of its kind; makes the directory when it is
/// not there yet. An allowlist, because the engine reads code from paths
/// no list of ours would name (`LuaIntro`, a raw-first `VFS.Include`), and
/// a mod can create anything under any name.
pub fn reset(write: &Path) -> io::Result<()> {
	std::fs::create_dir_all(write)?;
	for entry in std::fs::read_dir(write)? {
		let entry = entry?;
		let name = entry.file_name();
		let name = name.to_string_lossy();
		let is_dir = entry.file_type()?.is_dir();
		let keep = (is_dir
			&& KEPT_DIRS
				.iter()
				.any(|kept| kept.eq_ignore_ascii_case(&name)))
			|| (!is_dir
				&& KEPT_FILES
					.iter()
					.any(|kept| kept.eq_ignore_ascii_case(&name)));
		if keep {
			continue;
		}
		if is_dir {
			std::fs::remove_dir_all(entry.path())?;
		} else {
			std::fs::remove_file(entry.path())?;
		}
	}
	Ok(())
}

/// What the config pins over the player's values. Sockets off is the one
/// thing between mod Lua and the network while the container has it; the
/// keys are read-only to Lua, and the engine refuses `io.open` on a file of
/// this name, so the file is rewritten before every launch all the same.
pub const PINNED: &[(&str, &str)] = &[
	("LuaSocketEnabled", "0"),
	("TCPAllowConnect", ""),
	("TCPAllowListen", ""),
	("UDPAllowListen", ""),
];

/// Writes `<write>/springsettings.cfg` afresh from the player's settings in
/// `data_dirs` with [`PINNED`] and the borderless keys over them. Where it is.
pub fn write_config(
	mods: &ModsLayout,
	data_dirs: &[&Path],
	engine_version: &str,
) -> io::Result<PathBuf> {
	let path = mods.write.join("springsettings.cfg");
	let text = recoil::window_mode::config_text(data_dirs, engine_version, PINNED);
	std::fs::write(&path, text)?;
	Ok(path)
}

/// The player's files a modded game may have: their hotkeys. Never their
/// widget configs, which hold chat history and ignore lists, and never the
/// settings file, which [`write_config`] makes.
const SEEDED: &[&str] = &["uikeys.txt"];

pub fn seed(mods: &ModsLayout, from: &Path) -> io::Result<()> {
	for name in SEEDED {
		let source = from.join(name);
		if source.is_file() {
			std::fs::copy(&source, mods.write.join(name))?;
		}
	}
	Ok(())
}

/// What a confined engine may touch for this launch: the engine's own
/// directory to execute, each view listable with its content directories
/// readable, and the mods write directory to write. The probe adds its own
/// small directory on top (`probe::ensure`).
pub fn policy(
	engine: &recoil::EngineLayout,
	views: &[View],
	mods: &ModsLayout,
	network: Network,
	grants_file: PathBuf,
) -> Policy {
	let mut exe_dirs = vec![engine.bin.clone()];
	if engine.content != engine.bin {
		exe_dirs.push(engine.content.clone());
	}
	let roots = views
		.iter()
		.map(|view| Root {
			dir: view.dir.clone(),
			subdirs: view.subdirs.clone(),
		})
		.collect();
	Policy {
		exe_dirs,
		roots,
		write: mods.write.clone(),
		network,
		env: scrubbed_env(std::env::vars_os()),
		grants_file,
	}
}

/// Whether a start script might load mutators: any mention of a mutator
/// key, however it is spelled, laid out or commented. The engine's own
/// parser skips comments before a key and loads what it finds, so a script
/// we cannot read with certainty is treated as modded.
pub fn loads_mutators(script: &str) -> bool {
	script.to_ascii_lowercase().contains("mutator")
}

#[cfg(test)]
mod tests {
	use super::*;

	fn touch(path: &Path) {
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, b"x").unwrap();
	}

	#[test]
	fn the_layout_sits_beside_the_normal_data_dir() {
		let mods = layout(Path::new("C:/u/AppData/Local/modlobby/data"));
		assert_eq!(
			mods.write,
			PathBuf::from("C:/u/AppData/Local/modlobby/mods-data")
		);
		assert_eq!(
			mods.builds,
			PathBuf::from("C:/u/AppData/Local/modlobby/mod-builds")
		);
	}

	#[test]
	fn the_reset_removes_what_the_engine_scans_and_keeps_the_rest() {
		let dir = tempfile::tempdir().unwrap();
		let write = dir.path().join("mods-data");
		for rel in [
			"LuaUI/Widgets/x.lua",
			"games/a.sdd/modinfo.lua",
			"maps/m.sd7",
			"cache/ArchiveCache22.lua",
			"demos/d.sdfz",
			"infolog.txt",
			"springsettings.cfg",
			"uikeys.txt",
		] {
			touch(&write.join(rel));
		}
		reset(&write).unwrap();
		assert!(!write.join("LuaUI").exists());
		assert!(!write.join("games").exists());
		assert!(!write.join("maps").exists());
		assert!(
			!write.join("springsettings.cfg").exists(),
			"written afresh after the reset"
		);
		assert!(write.join("cache/ArchiveCache22.lua").is_file());
		assert!(write.join("demos/d.sdfz").is_file());
		assert!(write.join("infolog.txt").is_file());
		assert!(write.join("uikeys.txt").is_file());
	}

	#[test]
	fn a_reset_of_a_dir_that_does_not_exist_yet_creates_it() {
		let dir = tempfile::tempdir().unwrap();
		let write = dir.path().join("mods-data");
		reset(&write).unwrap();
		assert!(write.is_dir());
	}

	#[test]
	fn the_config_pins_sockets_off_and_borderless_over_the_players_values() {
		let dir = tempfile::tempdir().unwrap();
		let theirs = dir.path().join("data");
		std::fs::create_dir_all(&theirs).unwrap();
		std::fs::write(
			theirs.join("springsettings.cfg"),
			"LuaSocketEnabled = 1\nFullscreen = 1\nname = ann\nMaxParticles = 5000\n",
		)
		.unwrap();
		let mods = layout(&theirs);
		std::fs::create_dir_all(&mods.write).unwrap();
		let config = write_config(&mods, &[&theirs], "2026.09.01").unwrap();
		assert_eq!(config, mods.write.join("springsettings.cfg"));
		let text = std::fs::read_to_string(&config).unwrap();
		let value = |key: &str| {
			text.lines()
				.find_map(|line| line.split_once('=').filter(|(k, _)| k.trim() == key))
				.map(|(_, v)| v.trim().to_owned())
		};
		assert_eq!(value("LuaSocketEnabled").as_deref(), Some("0"));
		assert_eq!(value("TCPAllowConnect").as_deref(), Some(""));
		assert_eq!(value("Fullscreen").as_deref(), Some("0"));
		assert_eq!(value("WindowBorderless").as_deref(), Some("1"));
		assert_eq!(
			value("MaxParticles").as_deref(),
			Some("5000"),
			"their other settings carry over"
		);
	}

	#[test]
	fn the_config_is_written_afresh_every_time() {
		let dir = tempfile::tempdir().unwrap();
		let mods = layout(&dir.path().join("data"));
		std::fs::create_dir_all(&mods.write).unwrap();
		std::fs::write(
			mods.write.join("springsettings.cfg"),
			"LuaSocketEnabled = 1\n",
		)
		.unwrap();
		write_config(&mods, &[], "2026.09.01").unwrap();
		let text = std::fs::read_to_string(mods.write.join("springsettings.cfg")).unwrap();
		assert!(text.contains("LuaSocketEnabled = 0"), "{text}");
		assert!(!text.contains("LuaSocketEnabled = 1"), "{text}");
	}

	#[test]
	fn the_seed_copies_hotkeys_and_never_the_widget_configs() {
		let dir = tempfile::tempdir().unwrap();
		let theirs = dir.path().join("data");
		touch(&theirs.join("uikeys.txt"));
		touch(&theirs.join("springsettings.cfg"));
		touch(&theirs.join("LuaUI/Config/BYAR.lua"));
		let mods = layout(&theirs);
		std::fs::create_dir_all(&mods.write).unwrap();
		seed(&mods, &theirs).unwrap();
		assert!(mods.write.join("uikeys.txt").is_file());
		assert!(
			!mods.write.join("LuaUI").exists(),
			"BYAR.lua holds chat history"
		);
		assert!(
			!mods.write.join("springsettings.cfg").exists(),
			"the config is written, not seeded"
		);
	}

	fn engine_in(dir: &Path) -> recoil::EngineLayout {
		recoil::EngineLayout {
			bin: dir.join("engine/2026.09.01"),
			content: dir.join("engine/2026.09.01"),
			frameworks: None,
		}
	}

	#[test]
	fn a_missing_root_is_left_out_of_the_policy() {
		let dir = tempfile::tempdir().unwrap();
		let data = dir.path().join("data");
		std::fs::create_dir_all(data.join("games")).unwrap();
		let mods = layout(&data);
		let views = views(&mods, [data.clone(), dir.path().join("gone")]).unwrap();
		let policy = policy(
			&engine_in(dir.path()),
			&views,
			&mods,
			sandbox::Network::None,
			dir.path().join("g.json"),
		);
		assert_eq!(policy.roots.len(), 1);
		assert_eq!(policy.roots[0].subdirs, vec![data.join("games")]);
		assert!(
			policy.roots[0].dir.starts_with(&mods.views),
			"the engine reads the view, not the root"
		);
	}

	#[test]
	fn a_root_contributes_only_the_content_subdirs_it_has() {
		let dir = tempfile::tempdir().unwrap();
		let data = dir.path().join("data");
		std::fs::create_dir_all(data.join("games")).unwrap();
		std::fs::create_dir_all(data.join("pool")).unwrap();
		std::fs::create_dir_all(data.join("LuaMenu/Config")).unwrap();
		let mods = layout(&data);
		let views = views(&mods, [data.clone()]).unwrap();
		let policy = policy(
			&engine_in(dir.path()),
			&views,
			&mods,
			sandbox::Network::None,
			dir.path().join("g.json"),
		);
		assert_eq!(
			policy.roots[0].subdirs,
			vec![data.join("games"), data.join("pool")]
		);
	}

	#[test]
	fn the_policy_writes_only_the_mods_dir_and_reads_the_builds() {
		let dir = tempfile::tempdir().unwrap();
		let data = dir.path().join("data");
		std::fs::create_dir_all(&data).unwrap();
		let mods = layout(&data);
		std::fs::create_dir_all(mods.builds.join("games")).unwrap();
		let views = views(&mods, [data.clone(), mods.builds.clone()]).unwrap();
		let policy = policy(
			&engine_in(dir.path()),
			&views,
			&mods,
			sandbox::Network::Outbound,
			dir.path().join("g.json"),
		);
		assert_eq!(policy.write, mods.write);
		assert!(
			policy
				.roots
				.iter()
				.any(|root| root.subdirs == vec![mods.builds.join("games")])
		);
		assert_eq!(policy.network, sandbox::Network::Outbound);
		assert!(
			policy
				.exe_dirs
				.contains(&dir.path().join("engine/2026.09.01"))
		);
	}

	#[test]
	fn a_script_loading_mutators_is_recognised_however_it_is_spelled() {
		assert!(loads_mutators(
			"[game]\n{\n\tMutator0 = github-x-y.sdd;\n}\n"
		));
		assert!(loads_mutators("[GAME]{mutator0=x;}"));
		assert!(
			loads_mutators("[game]\n{\n\tmutator1=x;\n}\n"),
			"any mutator key counts: the engine, not our parser, decides what loads"
		);
		assert!(!loads_mutators("[game]\n{\n\tgametype = BAR;\n}\n"));
	}

	/// The engine's own parser skips comments before a key and loads what
	/// it finds; a script we cannot read with certainty is treated as modded.
	#[test]
	fn a_script_that_might_load_mutators_counts_as_modded() {
		assert!(loads_mutators("[game]{/*x*/mutator0=github-a-b-c.sdd;}"));
		assert!(loads_mutators(
			"[game]\n{\n\t// mutator0 = x;\n\tgametype = g;\n}\n"
		));
		assert!(
			loads_mutators("[game]\n{\n\tMUTATOR1=x;\n}\n"),
			"any mutator key at all"
		);
		assert!(!loads_mutators(
			"[game]\n{\n\tgametype = Beyond All Reason;\n}\n"
		));
	}

	#[test]
	fn the_layout_keeps_the_grants_record_beside_the_mods_dir() {
		let mods = layout(Path::new("C:/u/AppData/Local/modlobby/data"));
		assert_eq!(
			mods.grants,
			PathBuf::from("C:/u/AppData/Local/modlobby/sandbox-grants.json")
		);
	}

	/// Only what is known to be safe survives a reset: a mod can create a
	/// directory under any name, and the engine reads code from paths no
	/// list of ours would name (`LuaIntro`, a raw-first `VFS.Include`).
	#[test]
	fn the_reset_keeps_only_the_allowed_entries_and_fixes_their_kind() {
		let dir = tempfile::tempdir().unwrap();
		let write = dir.path().join("mods-data");
		for rel in [
			"LuaIntro/main.lua",
			"Terraform Brush/x.lua",
			"cache/ArchiveCache22.lua",
			"demos/d.sdfz",
			"fontcache/f.bin",
			"infolog.txt",
			"uikeys.txt",
			"springsettings.cfg",
			"modlobby-ingame.txt",
			"ruins_blueprints_temp.txt",
		] {
			touch(&write.join(rel));
		}
		// A file where a kept directory belongs, planted to break the next reset.
		std::fs::remove_dir_all(write.join("demos")).unwrap();
		std::fs::write(write.join("demos"), b"not a dir").unwrap();
		reset(&write).unwrap();
		let mut left: Vec<String> = std::fs::read_dir(&write)
			.unwrap()
			.map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
			.collect();
		left.sort();
		assert_eq!(
			left,
			vec!["cache", "fontcache", "infolog.txt", "uikeys.txt"]
		);
		assert!(write.join("cache/ArchiveCache22.lua").is_file());
		assert!(
			!write.join("demos").exists(),
			"a file in a directory's place is gone"
		);
	}

	/// Builds made before the builds root existed sit in the normal write
	/// dir's `games/`, where an open launch could resolve them by name.
	#[test]
	fn builds_left_in_the_write_dir_are_moved_into_the_builds_root() {
		let dir = tempfile::tempdir().unwrap();
		let data = dir.path().join("data");
		let stray = data
			.join("games")
			.join("github-owner-repo-0123456789ab.sdd");
		touch(&stray.join("modinfo.lua"));
		touch(
			&data
				.join("games")
				.join("Some Game v1.sdd")
				.join("modinfo.lua"),
		);
		let mods = layout(&data);
		adopt_builds(&data, &mods).unwrap();
		assert!(!stray.exists());
		assert!(
			mods.builds
				.join("games")
				.join("github-owner-repo-0123456789ab.sdd")
				.join("modinfo.lua")
				.is_file()
		);
		assert!(
			data.join("games").join("Some Game v1.sdd").is_dir(),
			"a game is not a build"
		);
	}

	/// A content check sees the builds root; asking twice adds it once.
	#[test]
	fn with_builds_adds_the_builds_root_once() {
		let dir = tempfile::tempdir().unwrap();
		let dirs = content::DataDirs::only(dir.path().join("data"));
		let once = with_builds(&dirs);
		let twice = with_builds(&once);
		assert_eq!(once.read, [layout(&dirs.write).builds]);
		assert_eq!(twice.read, once.read);
		assert!(!dirs.read.iter().any(|read| read.ends_with("mod-builds")));
	}
}
