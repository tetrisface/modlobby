//! Spawning the engine for a running game, and where its content lives.
//!
//! modlobby writes to a directory of its own and reads every other lobby's
//! install beside it. That is the engine's own model — one write dir, any
//! number of read dirs — and it means nothing of ours is ever half-written
//! into another lobby's tree, while a map or game they already have is never
//! fetched twice.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use std::sync::Arc;

use content::DataDirs;
use tokio::process::Child;

use crate::mods::ModsLayout;
use crate::player_files;

/// One environment variable. A function rather than `std::env`, so the
/// directories can be computed for a machine a test describes.
type Var<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// `$name`, else `default` under `$HOME`.
fn xdg(var: Var, name: &str, default: &str) -> Option<PathBuf> {
	var(name)
		.map(PathBuf::from)
		.or_else(|| var("HOME").map(|home| PathBuf::from(home).join(default)))
}

/// modlobby's own data directory: where what it downloads goes.
///
/// Local rather than roaming on Windows, and XDG data rather than config on
/// Linux: it holds gigabytes of content, not settings. macOS keeps both in
/// `Application Support` -- it has nowhere else to put them, and this is also
/// the directory a person has to open to install an engine by hand, so it is
/// worth it being the one they expect.
pub fn own_data_dir() -> Option<PathBuf> {
	let var = |name: &str| std::env::var_os(name);
	own_data_dir_in(&var)
}

fn own_data_dir_in(var: Var) -> Option<PathBuf> {
	let base = if cfg!(windows) {
		var("LOCALAPPDATA").map(PathBuf::from)
	} else if cfg!(target_os = "macos") {
		var("HOME")
			.map(PathBuf::from)
			.map(|home| home.join("Library").join("Application Support"))
	} else {
		xdg(var, "XDG_DATA_HOME", ".local/share")
	};
	base.map(|base| base.join("modlobby").join("data"))
}

/// Other lobbies' installs that exist on this machine, most trusted first.
pub fn installed_dirs() -> Vec<PathBuf> {
	let var = |name: &str| std::env::var_os(name);
	let candidates = if cfg!(windows) {
		windows_candidates(&var)
	} else if cfg!(target_os = "macos") {
		macos_candidates(&var)
	} else {
		unix_candidates(&var)
	};
	candidates.into_iter().filter(|dir| dir.is_dir()).collect()
}

/// The legacy launcher's data directory leads because Chobby and its package
/// cleanup live there; the new bar-lobby's `assets` follows. Both under
/// `%LOCALAPPDATA%\Programs` (`bar-lobby/src/main/config/app.ts`).
fn windows_candidates(var: Var) -> Vec<PathBuf> {
	let Some(local) = var("LOCALAPPDATA") else {
		return Vec::new();
	};
	let programs = PathBuf::from(local).join("Programs");
	vec![
		programs.join("Beyond-All-Reason").join("data"),
		programs.join("BeyondAllReason").join("assets"),
	]
}

/// The Apple Silicon BAR Launcher writes to
/// `~/Library/Application Support/Beyond-All-Reason-mac`
/// (`packaging/launcher.sh`, overridable with `BAR_WRITEDIR_OVERRIDE`), and
/// keeps the base content and the skirmish AIs inside the app bundle itself.
/// Neither spring-launcher nor bar-lobby ships for macOS, so there is nothing
/// else to look for.
fn macos_candidates(var: Var) -> Vec<PathBuf> {
	let launcher = var("BAR_WRITEDIR_OVERRIDE").map(PathBuf::from).or_else(|| {
		var("HOME").map(|home| {
			PathBuf::from(home)
				.join("Library")
				.join("Application Support")
				.join("Beyond-All-Reason-mac")
		})
	});
	launcher.into_iter().collect()
}

/// The launcher writes to `$XDG_STATE_HOME/Beyond-All-Reason`
/// (`spring-launcher/src/write_path.js`); bar-lobby's assets are
/// `$BAR_ASSETS_PATH`, else `$XDG_DATA_HOME/BeyondAllReason/assets`
/// (`bar-lobby/src/main/config/app.ts`).
fn unix_candidates(var: Var) -> Vec<PathBuf> {
	let launcher =
		xdg(var, "XDG_STATE_HOME", ".local/state").map(|state| state.join("Beyond-All-Reason"));
	let bar_lobby = var("BAR_ASSETS_PATH").map(PathBuf::from).or_else(|| {
		xdg(var, "XDG_DATA_HOME", ".local/share")
			.map(|data| data.join("BeyondAllReason").join("assets"))
	});
	[launcher, bar_lobby].into_iter().flatten().collect()
}

/// The directories the engine and every content check use: `write` is the
/// setting when given, else modlobby's own; everything else found on the
/// machine is read. `None` only on a machine with no home directory.
pub fn data_dirs(write: Option<PathBuf>) -> Option<DataDirs> {
	let write = write.or_else(own_data_dir)?;
	let dirs = assemble(write, installed_dirs());
	// A build left in the write directory by an older modlobby is one an
	// open launch could resolve by name; it goes to the builds root, which
	// only a confined launch reads (`mods::with_builds` for checks).
	let mods = crate::mods::layout(&dirs.write);
	if let Err(err) = crate::mods::adopt_builds(&dirs.write, &mods) {
		tracing::warn!(%err, "a mod build could not be moved to the builds root");
	}
	Some(dirs)
}

/// A user who points the write dir at another lobby's install gets today's
/// single-directory behaviour, not that directory twice.
fn assemble(write: PathBuf, installed: Vec<PathBuf>) -> DataDirs {
	let read = installed.into_iter().filter(|dir| *dir != write).collect();
	DataDirs { write, read }
}

/// An engine that has been started: as a child of ours, or inside a
/// confinement that holds it by its pid and job.
pub enum Started {
	Child(Child),
	Confined(Arc<sandbox::Confined>),
}

impl Started {
	pub fn id(&self) -> Option<u32> {
		match self {
			Self::Child(child) => child.id(),
			Self::Confined(confined) => Some(confined.pid),
		}
	}
}

/// An engine that has been started, and what it was started with.
pub struct Launched {
	pub child: Started,
	/// The player's files as they were just before, when there were any to
	/// keep: what to compare against when the game is over.
	pub snapshot: Option<PathBuf>,
	/// The private settings copy handed over with `--config`, when the user's
	/// own would have put the game in exclusive fullscreen. Worth telling them,
	/// since what they change in-game lands there rather than in their file.
	pub config: Option<PathBuf>,
	/// The mods a confined game loads, as the key what it leaves of the
	/// player's settings is kept under; `None` for an open game or a replay.
	pub set: Option<crate::modsettings::ModSet>,
}

/// How an engine is started: as any game; as any game but reading the mod
/// builds too, because the player trusts every mod it loads with their own
/// rights; or inside a confinement, because the game loads mods, which is
/// code from anywhere.
pub enum Mode<'a> {
	Open,
	Trusted {
		mods: &'a ModsLayout,
		/// The lobby's builds, every one trusted outside the sandbox: all an
		/// open engine may resolve beyond the player's own content.
		builds: Vec<PathBuf>,
	},
	Confined {
		confinement: Arc<dyn sandbox::Confinement>,
		mods: &'a ModsLayout,
		network: sandbox::Network,
		/// modlobby's own exe, which doubles as the probe.
		self_exe: PathBuf,
		/// Where the probe's verdict is kept between launches.
		probe_cache: PathBuf,
		/// Where the grants made for the container are recorded.
		grants_file: PathBuf,
		/// Files written into the mods write directory after its reset, by
		/// path relative to it: the in-game escape widget.
		after_reset: Vec<(PathBuf, String)>,
		/// The mods the lobby loads, as the key their settings are kept
		/// under; `None` for a replay, which has none to keep.
		set: Option<crate::modsettings::ModSet>,
	},
}

/// The OS build the probe's verdict is keyed on, beside modlobby's version.
pub fn os_build() -> String {
	sysinfo::System::os_version().unwrap_or_default()
}

/// Finds `engine_version` in any data directory and starts it on `target`
/// (a `spring://` URL or a start script), writing to `dirs.write`.
///
/// A fresh write directory is seeded with the player's files from another
/// install first, and the files are snapshotted before every launch, since
/// the engine rewrites its settings on the way out and has lost them before.
///
/// `overlay_config_dir` is where a borderless copy of the user's settings may
/// be kept, when they have the overlay on and their own settings would put the
/// game in exclusive full screen. Passing `None`, or having settings that
/// already work, launches against their configuration untouched.
pub fn spawn_open(
	dirs: &DataDirs,
	engine_version: &str,
	target: String,
	overlay_config_dir: Option<&Path>,
	menu: Option<recoil::MenuArchive>,
) -> Result<Launched, String> {
	start_open(prepare_open(
		dirs,
		engine_version,
		target,
		overlay_config_dir,
		menu,
		None,
	)?)
}

/// An open launch made ready: the command to run, and what to remember of
/// it once it runs.
struct Prepared {
	launch: recoil::Launch,
	snapshot: Option<PathBuf>,
	config: Option<PathBuf>,
}

/// Everything [`spawn_open`] does before the engine starts. With `builds`,
/// that directory is read too: a view of the lobby's builds, for a game the
/// player trusts with their own rights.
fn prepare_open(
	dirs: &DataDirs,
	engine_version: &str,
	target: String,
	overlay_config_dir: Option<&Path>,
	menu: Option<recoil::MenuArchive>,
	builds: Option<&Path>,
) -> Result<Prepared, String> {
	let engine = find_engine(dirs, engine_version, &target)?;
	player_files::seed(dirs);
	// Neither failure below is worth refusing to play over: without the
	// snapshot a loss goes unnoticed, without the config the overlay cannot
	// cover the game.
	let snapshot = player_files::snapshot(&dirs.write, SystemTime::now())
		.inspect_err(|err| tracing::warn!(%err, "player's files not snapshotted"))
		.ok()
		.flatten();
	let config = overlay_config_dir.and_then(|dir| {
		let data_dirs = settings_dirs(dirs, &engine);
		let data_dirs: Vec<&Path> = data_dirs.iter().map(PathBuf::as_path).collect();
		recoil::window_mode::borderless_config(&data_dirs, engine_version, dir)
			.inspect_err(|err| tracing::warn!(%err, "no borderless config; overlay may not show"))
			.ok()
			.flatten()
	});
	if let Some(config) = &config {
		tracing::info!(
			config = %config.display(),
			"their settings ask for exclusive fullscreen; launching on a private copy"
		);
	}

	let mut read_dirs = dirs.read.clone();
	if let Some(builds) = builds
		&& !read_dirs.iter().any(|dir| dir == builds)
	{
		read_dirs.push(builds.to_path_buf());
	}
	let launch = recoil::Launch {
		engine,
		data_dir: dirs.write.clone(),
		read_dirs,
		target,
		config: config.clone(),
		menu,
	};
	Ok(Prepared {
		launch,
		snapshot,
		config,
	})
}

/// Starts what [`prepare_open`] made ready.
fn start_open(prepared: Prepared) -> Result<Launched, String> {
	let Prepared {
		launch,
		snapshot,
		config,
	} = prepared;
	tracing::info!(engine = %launch.engine.engine().display(), "launching");
	let child = tokio::process::Command::from(launch.command())
		.spawn()
		.map_err(|err| format!("spawning the engine: {err}"))?;
	Ok(Launched {
		child: Started::Child(child),
		snapshot,
		config,
		set: None,
	})
}

/// [`spawn_open`], or the confined launch `mode` asks for: the mods write
/// directory reset and seeded, its config written with sockets off, the
/// probe run inside the same container, and only then the engine, reading
/// every data directory and writing nowhere else.
pub async fn spawn(
	dirs: &DataDirs,
	engine_version: &str,
	target: String,
	overlay_config_dir: Option<&Path>,
	menu: Option<recoil::MenuArchive>,
	mode: Mode<'_>,
) -> Result<Launched, String> {
	let (confinement, mods, network, self_exe, probe_cache, grants_file, after_reset, set) =
		match mode {
			Mode::Open => {
				return spawn_open(dirs, engine_version, target, overlay_config_dir, menu);
			}
			Mode::Trusted { mods, builds } => {
				let view = crate::mods::trusted_view(mods, &builds)
					.map_err(|err| format!("making the view of the lobby's builds: {err}"))?;
				return start_open(prepare_open(
					dirs,
					engine_version,
					target,
					overlay_config_dir,
					menu,
					Some(&view),
				)?);
			}
			Mode::Confined {
				confinement,
				mods,
				network,
				self_exe,
				probe_cache,
				grants_file,
				after_reset,
				set,
			} => (
				confinement,
				mods,
				network,
				self_exe,
				probe_cache,
				grants_file,
				after_reset,
				set,
			),
		};
	if let Some(reason) = confinement.unavailable() {
		return Err(reason);
	}
	let engine = find_engine(dirs, engine_version, &target)?;
	let failed = |what: &str, err: std::io::Error| format!("{what}: {err}");
	crate::mods::reset(&mods.write).map_err(|err| failed("resetting the mods directory", err))?;
	crate::mods::seed(mods, &dirs.write)
		.map_err(|err| failed("seeding the mods directory", err))?;
	// What the last game of these mods left of the player's settings goes
	// over the open seed, each file through its reader once more.
	let mut theirs = None;
	if let Some(set) = &set
		&& let Some(capture) = crate::modsettings::find(&mods.settings, set)
	{
		match crate::modsettings::seed_from(&capture, &mods.write) {
			Ok(seeded) => theirs = seeded.config,
			Err(err) => tracing::warn!(%err, "the mods' kept settings were not seeded"),
		}
	}
	std::fs::write(
		mods.write.join(crate::mods::CONFINED_MARK),
		"This game runs inside modlobby's sandbox.\n",
	)
	.map_err(|err| failed("marking the mods directory", err))?;
	let data_dirs = settings_dirs(dirs, &engine);
	let data_dirs: Vec<&Path> = data_dirs.iter().map(PathBuf::as_path).collect();
	let config = crate::mods::write_config(mods, &data_dirs, engine_version, theirs.as_deref())
		.map_err(|err| failed("writing the mods config", err))?;
	for (relative, text) in &after_reset {
		let path = mods.write.join(relative);
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)
				.map_err(|err| failed("preparing the mods directory", err))?;
		}
		std::fs::write(&path, text)
			.map_err(|err| failed("writing into the mods directory", err))?;
	}

	// What went in, kept beside what the game will leave: the diff the
	// player can inspect, and what a discard goes back to.
	if let Some(set) = &set
		&& let Err(err) = crate::modsettings::baseline(&mods.settings, set, &mods.write)
	{
		tracing::warn!(%err, "the mods' settings baseline was not written");
	}

	// A replay is a file the engine opens itself, and the replays directory
	// holds every game's chat: the one being played goes in as a copy, and
	// the engine is pointed at that.
	let target = if Path::new(&target).is_file() {
		let demos = mods.write.join("demos");
		std::fs::create_dir_all(&demos)
			.map_err(|err| failed("preparing the mods replays directory", err))?;
		let name = Path::new(&target)
			.file_name()
			.ok_or("a replay with no file name")?;
		let copy = demos.join(name);
		std::fs::copy(&target, &copy).map_err(|err| failed("copying the replay", err))?;
		copy.to_string_lossy().into_owned()
	} else {
		target
	};

	// The engine reads views of the data roots -- links to their content and
	// nothing else -- never the roots themselves.
	let roots = dirs
		.all()
		.map(Path::to_path_buf)
		.chain(std::iter::once(mods.builds.clone()));
	let views =
		crate::mods::views(mods, roots).map_err(|err| failed("making the content views", err))?;
	let launch = recoil::Launch {
		engine,
		data_dir: mods.write.clone(),
		read_dirs: views.iter().map(|view| view.dir.clone()).collect(),
		target,
		config: Some(config),
		menu,
	};
	let policy = crate::mods::policy(&launch.engine, &views, mods, network, grants_file);
	sandbox::probe::ensure(
		&confinement,
		&policy,
		&self_exe,
		&probe_cache,
		&os_build(),
		env!("CARGO_PKG_VERSION"),
	)
	.await?;
	tracing::info!(engine = %launch.engine.engine().display(), "launching confined");
	// Granting is slow on a large content tree; off the async threads.
	let confined = {
		let command = launch.command();
		let policy = policy.clone();
		tokio::task::spawn_blocking(move || confinement.spawn(&command, &policy))
			.await
			.map_err(|err| format!("the launch task: {err}"))??
	};
	Ok(Launched {
		child: Started::Confined(Arc::new(confined)),
		snapshot: None,
		config: None,
		set,
	})
}

/// The engine `engine_version` in any data directory, for `target`.
fn find_engine(
	dirs: &DataDirs,
	engine_version: &str,
	target: &str,
) -> Result<recoil::EngineLayout, String> {
	// The one place the engine is ever started, so the one place this has to
	// hold. A skirmish and a replay are this machine's own business; joining
	// somebody's hosted game is what the Apple build's author asked nobody to
	// do with it, and refusing here means no button, route or future caller
	// can get around it by accident.
	if let Some(refusal) = recoil::refuse_target(target, recoil::may_join_hosted_games()) {
		return Err(refusal);
	}
	content::Library::new(dirs.clone())
		.find_engine(engine_version)
		.ok_or_else(|| {
			format!(
				"no engine {engine_version} with {} under {}",
				recoil::ENGINE_BINARY,
				dirs.write.join("engine").display()
			)
		})
}

/// The engine's own order for its settings: the write dir, the directory it
/// runs from (`--isolation`, `DataDirLocater::LocateDataDirs` level 2a), then
/// `SPRING_DATADIR`.
fn settings_dirs(dirs: &DataDirs, engine: &recoil::EngineLayout) -> Vec<PathBuf> {
	[dirs.write.clone(), engine.bin.clone()]
		.into_iter()
		.chain(dirs.read.iter().cloned())
		.collect()
}

/// The pr-downloader runs that fetch everything in `wants`, to be run one
/// after the other: it rewrites rapid's repo index on every run, so two at
/// once corrupt each other's view of it. They write to `dirs.write` only;
/// what the read directories already hold was left out of `wants` by the
/// caller.
///
/// Games are looked for in `rapid_master` — the room's server's — and nowhere
/// else; see [`recoil::Download::runs`].
pub fn plan_download(
	dirs: &DataDirs,
	engine_version: &str,
	wants: Vec<(recoil::Want, String)>,
	rapid_master: &str,
	map_searches: &[String],
) -> Result<Vec<recoil::Download>, String> {
	let binary = content::Library::new(dirs.clone())
		.find_downloader(engine_version)
		.ok_or_else(|| {
			format!(
				"no engine under {} ships {}; install an engine first",
				dirs.write.join("engine").display(),
				recoil::DOWNLOADER_BINARY
			)
		})?;
	// What a room's mutators need goes to the builds root, which only a
	// confined launch reads; the rest where it always went.
	let (mutators, rest): (Vec<_>, Vec<_>) = wants
		.into_iter()
		.partition(|(want, _)| *want == recoil::Want::Mutator);
	let mut runs = recoil::Download::runs(&binary, &dirs.write, rest, rapid_master, map_searches);
	if !mutators.is_empty() {
		let builds = crate::mods::layout(&dirs.write).builds;
		runs.extend(recoil::Download::runs(
			&binary,
			&builds,
			mutators,
			rapid_master,
			&[],
		));
	}
	Ok(runs)
}

/// Starts one run. Killed if dropped: a stopped download is its task let go.
pub fn spawn_download(download: &recoil::Download) -> Result<Child, String> {
	tracing::info!(
		binary = %download.binary.display(),
		rapid = download.rapid_master,
		"downloading content"
	);
	tokio::process::Command::from(download.command())
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.kill_on_drop(true)
		.spawn()
		.map_err(|err| format!("spawning pr-downloader: {err}"))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The one BAR install a Mac can have: the Apple Silicon launcher's own
	/// write directory, where its downloaded maps and games are.
	#[test]
	fn a_mac_reuses_the_apple_silicon_launchers_content() {
		let vars = env(&[("HOME", "/Users/ann")]);
		assert_eq!(
			macos_candidates(&vars),
			vec![PathBuf::from(
				"/Users/ann/Library/Application Support/Beyond-All-Reason-mac"
			)]
		);
	}

	/// The launcher honours this, so following it finds a moved install.
	#[test]
	fn a_moved_mac_install_is_followed() {
		let vars = env(&[
			("HOME", "/Users/ann"),
			("BAR_WRITEDIR_OVERRIDE", "/Volumes/Games/bar"),
		]);
		assert_eq!(
			macos_candidates(&vars),
			vec![PathBuf::from("/Volumes/Games/bar")]
		);
	}

	#[test]
	fn a_mac_with_no_home_offers_nothing_rather_than_a_bare_path() {
		let vars = env(&[]);
		assert!(macos_candidates(&vars).is_empty());
	}

	/// An environment made of the given variables and nothing else.
	fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
		move |name| {
			vars.iter()
				.find(|(key, _)| *key == name)
				.map(|(_, value)| OsString::from(value))
		}
	}

	fn paths(found: Vec<PathBuf>) -> Vec<String> {
		found
			.iter()
			.map(|path| path.to_string_lossy().replace('\\', "/"))
			.collect()
	}

	#[test]
	fn our_own_directory_is_local_data_not_config() {
		let found = own_data_dir_in(&env(&[
			("LOCALAPPDATA", "C:/u/AppData/Local"),
			("HOME", "/home/dev"),
		]))
		.unwrap();
		let expected = if cfg!(windows) {
			"C:/u/AppData/Local/modlobby/data"
		} else if cfg!(target_os = "macos") {
			"/home/dev/Library/Application Support/modlobby/data"
		} else {
			"/home/dev/.local/share/modlobby/data"
		};
		assert_eq!(paths(vec![found]), [expected]);
	}

	#[test]
	fn windows_looks_beside_the_launcher_then_bar_lobby() {
		let found = windows_candidates(&env(&[("LOCALAPPDATA", "C:/u/AppData/Local")]));
		assert_eq!(
			paths(found),
			[
				"C:/u/AppData/Local/Programs/Beyond-All-Reason/data",
				"C:/u/AppData/Local/Programs/BeyondAllReason/assets",
			]
		);
	}

	#[test]
	fn windows_without_localappdata_has_nowhere_to_look() {
		assert!(windows_candidates(&env(&[])).is_empty());
		assert_eq!(own_data_dir_in(&env(&[])), None);
	}

	#[test]
	fn unix_defaults_to_the_xdg_directories_under_home() {
		let found = unix_candidates(&env(&[("HOME", "/home/dev")]));
		assert_eq!(
			paths(found),
			[
				"/home/dev/.local/state/Beyond-All-Reason",
				"/home/dev/.local/share/BeyondAllReason/assets",
			]
		);
	}

	#[test]
	fn unix_honours_the_xdg_variables_over_home() {
		let found = unix_candidates(&env(&[
			("HOME", "/home/dev"),
			("XDG_STATE_HOME", "/state"),
			("XDG_DATA_HOME", "/data"),
		]));
		assert_eq!(
			paths(found),
			["/state/Beyond-All-Reason", "/data/BeyondAllReason/assets"]
		);
	}

	#[test]
	fn unix_takes_bar_lobbys_own_assets_setting_first() {
		let found = unix_candidates(&env(&[
			("HOME", "/home/dev"),
			("BAR_ASSETS_PATH", "/games/bar"),
		]));
		assert_eq!(paths(found)[1], "/games/bar");
	}

	#[test]
	fn unix_without_home_has_nowhere_to_look() {
		assert!(unix_candidates(&env(&[])).is_empty());
	}

	#[test]
	fn every_install_found_is_read_and_only_ours_is_written() {
		let dirs = assemble(
			PathBuf::from("/ours"),
			vec![PathBuf::from("/launcher"), PathBuf::from("/bar-lobby")],
		);
		assert_eq!(dirs.write, PathBuf::from("/ours"));
		assert_eq!(
			dirs.read,
			[PathBuf::from("/launcher"), PathBuf::from("/bar-lobby")]
		);
	}

	#[test]
	fn pointing_the_write_dir_at_an_install_does_not_read_it_twice() {
		let dirs = assemble(
			PathBuf::from("/launcher"),
			vec![PathBuf::from("/launcher"), PathBuf::from("/bar-lobby")],
		);
		assert_eq!(dirs.read, [PathBuf::from("/bar-lobby")]);
	}
}

#[cfg(test)]
mod confined {
	use super::*;
	use std::sync::{Arc, Mutex};

	/// A confinement that records the policy it was handed and spawns nothing.
	struct Recording {
		seen: Arc<Mutex<Option<sandbox::Policy>>>,
	}

	impl sandbox::Confinement for Recording {
		fn spawn(
			&self,
			_command: &std::process::Command,
			policy: &sandbox::Policy,
		) -> Result<sandbox::Confined, String> {
			*self.seen.lock().unwrap() = Some(policy.clone());
			Err("recorded, not started".into())
		}

		fn unavailable(&self) -> Option<String> {
			None
		}
	}

	fn engine_in(dir: &Path) -> DataDirs {
		let data = dir.join("data");
		let bin = data.join("engine").join("0.0.0");
		std::fs::create_dir_all(&bin).unwrap();
		std::fs::write(bin.join(recoil::ENGINE_BINARY), b"").unwrap();
		std::fs::create_dir_all(data.join("games")).unwrap();
		DataDirs::only(data)
	}

	fn mode<'a>(
		confinement: Arc<dyn sandbox::Confinement>,
		mods: &'a crate::mods::ModsLayout,
		dir: &Path,
		set: Option<crate::modsettings::ModSet>,
	) -> Mode<'a> {
		// The probe runs from a copy of modlobby's exe; an empty file stands
		// in for it, since the recording confinement starts nothing.
		std::fs::write(dir.join("modlobby.exe"), b"").unwrap();
		Mode::Confined {
			confinement,
			mods,
			network: sandbox::Network::None,
			self_exe: dir.join("modlobby.exe"),
			probe_cache: dir.join("probe.json"),
			grants_file: dir.join("grants.json"),
			after_reset: vec![(
				PathBuf::from("LuaUI/Widgets/modlobby_escape.lua"),
				"-- escape".into(),
			)],
			set,
		}
	}

	#[tokio::test]
	async fn a_confined_launch_never_reads_the_normal_write_dir_as_writable() {
		let dir = tempfile::tempdir().unwrap();
		let dirs = engine_in(dir.path());
		let mods = crate::mods::layout(&dirs.write);
		// The player's own copy of the escape widget talks to the socket; the
		// seed carries it along, and the file version written after the reset
		// has to win.
		let players = dirs.write.join("LuaUI").join("Widgets");
		std::fs::create_dir_all(&players).unwrap();
		std::fs::write(players.join("modlobby_escape.lua"), "-- socket version").unwrap();
		// No cached verdict: the probe runs through the same recording
		// confinement, which records the policy and refuses, so nothing
		// is started and the policy is still what the engine would get.
		let seen = Arc::new(Mutex::new(None));
		let recording = Recording { seen: seen.clone() };
		let err = spawn(
			&dirs,
			"0.0.0",
			"script.txt".into(),
			None,
			None,
			mode(Arc::new(recording), &mods, dir.path(), None),
		)
		.await
		.err()
		.expect("refused");
		assert!(err.contains("recorded, not started"), "{err}");
		let policy = seen
			.lock()
			.unwrap()
			.clone()
			.expect("the confinement was asked");
		assert_eq!(policy.write, mods.write);
		assert!(
			policy
				.roots
				.iter()
				.any(|root| root.subdirs.contains(&dirs.write.join("games"))),
			"the normal data dir's content is read through a view: {:?}",
			policy.roots
		);
		assert!(
			!policy.roots.iter().any(|root| root.dir == dirs.write),
			"the normal data dir itself is never a root of the engine's"
		);
		assert!(
			mods.write.join("springsettings.cfg").is_file(),
			"the config was written before the launch"
		);
		assert_eq!(
			std::fs::read_to_string(mods.write.join("LuaUI/Widgets/modlobby_escape.lua")).unwrap(),
			"-- escape",
			"the escape widget landed after the reset, over the player's copy"
		);
		assert!(
			mods.write.join(crate::mods::CONFINED_MARK).is_file(),
			"the mods directory is marked as the sandbox's for the game's Lua"
		);
	}

	#[tokio::test]
	async fn a_platform_without_a_sandbox_refuses_a_modded_launch_with_its_reason() {
		let dir = tempfile::tempdir().unwrap();
		let dirs = engine_in(dir.path());
		let mods = crate::mods::layout(&dirs.write);
		let refuse = sandbox::Refuse {
			reason: "no sandbox on this platform".into(),
		};
		let err = spawn(
			&dirs,
			"0.0.0",
			"script.txt".into(),
			None,
			None,
			mode(Arc::new(refuse), &mods, dir.path(), None),
		)
		.await
		.err()
		.expect("refused");
		assert_eq!(err, "no sandbox on this platform");
		assert!(
			!mods.write.exists(),
			"nothing was prepared for a launch that cannot happen"
		);
	}

	/// A replay is a file the engine opens itself, and the replays directory
	/// holds every game's chat: the one being played is copied into the mods
	/// directory and the engine is pointed at the copy.
	#[tokio::test]
	async fn a_replay_target_is_copied_into_the_mods_dir() {
		let dir = tempfile::tempdir().unwrap();
		let dirs = engine_in(dir.path());
		let mods = crate::mods::layout(&dirs.write);
		let demos = dirs.write.join("demos");
		std::fs::create_dir_all(&demos).unwrap();
		let replay = demos.join("2026-10-18_game_0.0.0.sdfz");
		std::fs::write(&replay, b"demo bytes").unwrap();
		let seen = Arc::new(Mutex::new(None));
		let recording = Recording { seen: seen.clone() };
		let _ = spawn(
			&dirs,
			"0.0.0",
			replay.to_string_lossy().into_owned(),
			None,
			None,
			mode(Arc::new(recording), &mods, dir.path(), None),
		)
		.await;
		let copy = mods.write.join("demos").join("2026-10-18_game_0.0.0.sdfz");
		assert_eq!(std::fs::read(&copy).unwrap(), b"demo bytes");
	}

	/// An open launch must never resolve a mod build: the builds root is
	/// read only through a confined launch's views.
	#[test]
	fn the_builds_root_is_not_a_read_dir_of_an_open_launch() {
		let dir = tempfile::tempdir().unwrap();
		let dirs = data_dirs(Some(dir.path().join("data"))).unwrap();
		assert!(
			!dirs.read.iter().any(|read| read.ends_with("mod-builds")),
			"{:?}",
			dirs.read
		);
	}

	#[tokio::test]
	async fn a_confined_launch_reads_the_builds_root() {
		let dir = tempfile::tempdir().unwrap();
		let dirs = engine_in(dir.path());
		let mods = crate::mods::layout(&dirs.write);
		std::fs::create_dir_all(mods.builds.join("games")).unwrap();
		let seen = Arc::new(Mutex::new(None));
		let recording = Recording { seen: seen.clone() };
		let _ = spawn(
			&dirs,
			"0.0.0",
			"script.txt".into(),
			None,
			None,
			mode(Arc::new(recording), &mods, dir.path(), None),
		)
		.await;
		let policy = seen
			.lock()
			.unwrap()
			.clone()
			.expect("the confinement was asked");
		assert!(
			policy
				.roots
				.iter()
				.any(|root| root.subdirs.contains(&mods.builds.join("games"))),
			"{:?}",
			policy.roots
		);
	}

	/// What a room's mutators need is fetched into the builds root, where
	/// only a confined launch reads; the room's game and map go where they
	/// always did.
	#[test]
	fn mutator_downloads_land_in_the_builds_root() {
		let dir = tempfile::tempdir().unwrap();
		let dirs = engine_in(dir.path());
		std::fs::write(
			dirs.write
				.join("engine")
				.join("0.0.0")
				.join(recoil::DOWNLOADER_BINARY),
			b"",
		)
		.unwrap();
		let runs = plan_download(
			&dirs,
			"0.0.0",
			vec![
				(recoil::Want::Game, "Game v1".into()),
				(recoil::Want::Mutator, "Mod v1".into()),
			],
			"https://rapid.example/repos.gz",
			&[],
		)
		.unwrap();
		let mods = crate::mods::layout(&dirs.write);
		let of = |want: recoil::Want| {
			runs.iter()
				.find(|run| run.wants.iter().any(|(held, _)| *held == want))
				.map(|run| run.data_dir.clone())
				.expect("a run for it")
		};
		assert_eq!(of(recoil::Want::Game), dirs.write);
		assert_eq!(of(recoil::Want::Mutator), mods.builds);
	}

	/// A launch trusted with its mods is an open engine that reads a view
	/// holding exactly the lobby's builds -- never the builds root, where the
	/// host's start script could pick any build ever made.
	#[test]
	fn a_trusted_launch_reads_a_view_of_exactly_the_lobbys_builds() {
		let dir = tempfile::tempdir().unwrap();
		let dirs = engine_in(dir.path());
		let mods = crate::mods::layout(&dirs.write);
		let a = mods
			.builds
			.join("games")
			.join("github-dev-a-000000000000.sdd");
		let b = mods
			.builds
			.join("games")
			.join("github-dev-b-000000000000.sdd");
		for build in [&a, &b] {
			std::fs::create_dir_all(build).unwrap();
			std::fs::write(build.join("modinfo.lua"), "return {}").unwrap();
		}
		let view = crate::mods::trusted_view(&mods, std::slice::from_ref(&a)).unwrap();
		let trusted =
			prepare_open(&dirs, "0.0.0", "script.txt".into(), None, None, Some(&view)).unwrap();
		assert_eq!(trusted.launch.data_dir, dirs.write);
		assert!(
			trusted.launch.read_dirs.contains(&view),
			"{:?}",
			trusted.launch.read_dirs
		);
		assert!(!trusted.launch.read_dirs.contains(&mods.builds));
		let mut linked: Vec<String> = std::fs::read_dir(view.join("games"))
			.unwrap()
			.map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
			.collect();
		linked.sort();
		assert_eq!(linked, ["github-dev-a-000000000000.sdd"]);
		assert!(
			view.join("games")
				.join("github-dev-a-000000000000.sdd")
				.join("modinfo.lua")
				.is_file()
		);
		let open = prepare_open(&dirs, "0.0.0", "script.txt".into(), None, None, None).unwrap();
		assert!(!open.launch.read_dirs.contains(&mods.builds));
	}

	// ------------------------------------------- settings kept per mod set

	fn sphere() -> lobby_core::Mutator {
		lobby_core::Mutator {
			name: "github-dev-sphere-000000000000.sdd".into(),
			checksum: None,
			source: Some(lobby_core::GitPin {
				repo: "dev/sphere".into(),
				commit: "0".repeat(40),
			}),
			date: None,
			track: None,
		}
	}

	fn plant(dir: &Path, rel: &str, text: &str) {
		let path = dir.join(rel);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, text).unwrap();
	}

	/// The open seed: the player's settings and widget selection as their
	/// ordinary games have them.
	fn open_seed(dirs: &DataDirs) {
		plant(
			&dirs.write,
			"springsettings.cfg",
			"Fullscreen = 1\nVSync = 1\n",
		);
		plant(
			&dirs.write,
			"LuaUI/Config/BYAR.lua",
			"return {\n\tallowUserWidgets = true,\n\torder = {\n\t\tChat = 1,\n\t},\n}\n",
		);
	}

	#[tokio::test]
	async fn a_set_played_before_seeds_from_its_capture() {
		let dir = tempfile::tempdir().unwrap();
		let dirs = engine_in(dir.path());
		let mods = crate::mods::layout(&dirs.write);
		open_seed(&dirs);
		let set = crate::modsettings::ModSet::of(&[sphere()]);
		// What the last game of this set left behind.
		let left = dir.path().join("left");
		plant(&left, "springsettings.cfg", "Fullscreen = 0\nVSync = 1\n");
		plant(
			&left,
			"LuaUI/Config/BYAR.lua",
			"return {\n\torder = {\n\t\tChat = 7,\n\t},\n}\n",
		);
		crate::modsettings::capture(&mods.settings, &set, &left, "2026-10-22T10:00:00Z").unwrap();
		let seen = Arc::new(Mutex::new(None));
		let recording = Recording { seen: seen.clone() };
		let _ = spawn(
			&dirs,
			"0.0.0",
			"script.txt".into(),
			None,
			None,
			mode(Arc::new(recording), &mods, dir.path(), Some(set)),
		)
		.await;
		let config = std::fs::read_to_string(mods.write.join("springsettings.cfg")).unwrap();
		assert!(config.contains("Fullscreen = 0"), "{config}");
		assert!(config.contains("LuaSocketEnabled = 0"), "{config}");
		let widgets = std::fs::read_to_string(mods.write.join("LuaUI/Config/BYAR.lua")).unwrap();
		assert!(widgets.contains("Chat = 7"), "{widgets}");
	}

	#[tokio::test]
	async fn a_rejected_capture_falls_back_to_the_open_seed() {
		let dir = tempfile::tempdir().unwrap();
		let dirs = engine_in(dir.path());
		let mods = crate::mods::layout(&dirs.write);
		open_seed(&dirs);
		let set = crate::modsettings::ModSet::of(&[sphere()]);
		let left = dir.path().join("left");
		plant(&left, "springsettings.cfg", "Fullscreen = 0\n");
		let capture =
			crate::modsettings::capture(&mods.settings, &set, &left, "2026-10-22T10:00:00Z")
				.unwrap();
		// Planted after the capture, around its reader: not data.
		plant(
			&capture.after(),
			"LuaUI/Config/BYAR.lua",
			"return { order = other }\n",
		);
		let seen = Arc::new(Mutex::new(None));
		let recording = Recording { seen: seen.clone() };
		let _ = spawn(
			&dirs,
			"0.0.0",
			"script.txt".into(),
			None,
			None,
			mode(Arc::new(recording), &mods, dir.path(), Some(set)),
		)
		.await;
		let widgets = std::fs::read_to_string(mods.write.join("LuaUI/Config/BYAR.lua")).unwrap();
		assert!(
			widgets.contains("Chat = 1"),
			"the open seed stands: {widgets}"
		);
		assert!(!widgets.contains("other"), "{widgets}");
		let config = std::fs::read_to_string(mods.write.join("springsettings.cfg")).unwrap();
		assert!(
			config.contains("Fullscreen = 0"),
			"the rest of the capture still seeds: {config}"
		);
	}

	#[tokio::test]
	async fn a_baseline_is_written_at_a_sets_first_launch() {
		let dir = tempfile::tempdir().unwrap();
		let dirs = engine_in(dir.path());
		let mods = crate::mods::layout(&dirs.write);
		open_seed(&dirs);
		let set = crate::modsettings::ModSet::of(&[sphere()]);
		let seen = Arc::new(Mutex::new(None));
		let recording = Recording { seen: seen.clone() };
		let _ = spawn(
			&dirs,
			"0.0.0",
			"script.txt".into(),
			None,
			None,
			mode(Arc::new(recording), &mods, dir.path(), Some(set.clone())),
		)
		.await;
		let capture = crate::modsettings::Capture {
			dir: mods.settings.join(set.id()),
		};
		assert_eq!(capture.set().map(|(kept, _)| kept), Some(set));
		let before = std::fs::read_to_string(capture.before().join("springsettings.cfg")).unwrap();
		// The baseline is what the engine was given: the open seed with the
		// launch's own keys pinned over it, so Fullscreen reads pinned, VSync theirs.
		assert!(before.contains("VSync = 1"), "{before}");
		assert!(before.contains("Fullscreen = 0"), "{before}");
		assert!(
			!before.contains("LuaSocketEnabled"),
			"a pinned key is never kept: {before}"
		);
		assert!(capture.before().join("LuaUI/Config/BYAR.lua").is_file());
	}
}
