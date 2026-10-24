//! What a confined process may touch.

use std::ffi::OsString;
use std::path::PathBuf;

/// The subdirectories of a data root that hold game content and nothing
/// personal. The root itself holds the player's settings, configs and logs,
/// which stay unreadable.
pub const CONTENT_SUBDIRS: &[&str] = &["games", "maps", "packages", "pool", "rapid"];

/// The platforms a boundary is built for. A report is judged by the rules of
/// the platform it came from: a file hidden by a mount namespace is proof on
/// Linux and nothing on Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
	Windows,
	Linux,
}

/// The platform this build runs on, as far as boundaries go.
pub const fn this_os() -> Os {
	if cfg!(windows) {
		Os::Windows
	} else {
		Os::Linux
	}
}

/// The environment variables a confined engine on Windows gets. Everything
/// else in the parent's environment -- tokens, passwords, the paths of
/// other installs -- is dropped. `LOCALAPPDATA`, `TEMP` and `TMP` must be
/// present for an AppContainer to start at all; Windows rewrites them to
/// the container's own folder.
pub const KEPT_ENV: &[&str] = &[
	"SystemRoot",
	"SystemDrive",
	"windir",
	"PATH",
	"LOCALAPPDATA",
	"TEMP",
	"TMP",
	"PROCESSOR_ARCHITECTURE",
	"NUMBER_OF_PROCESSORS",
];

/// The environment a confined engine on Linux may hold: what the sandbox
/// sets for it -- its runtime directory, the proxied display, the audio
/// socket, its home and caches, and the engine's own directories, so a
/// relaunch from Lua keeps them -- and the locale. Never `DISPLAY`: X11
/// would hand it every other window's keys.
const KEPT_ENV_LINUX: &[&str] = &[
	"PATH",
	"HOME",
	"LANG",
	"LC_ALL",
	"XDG_RUNTIME_DIR",
	"XDG_CACHE_HOME",
	"WAYLAND_DISPLAY",
	"PULSE_SERVER",
	"SDL_VIDEODRIVER",
	"SPRING_DATADIR",
	"SPRING_WRITEDIR",
	"SPRING_ISOLATED",
	"__GL_SHADER_DISK_CACHE_PATH",
	"MESA_SHADER_CACHE_DIR",
	// bwrap sets it to the directory it starts the command in.
	"PWD",
];

/// What a confined engine on `os` may find in its environment.
pub fn kept_env(os: Os) -> &'static [&'static str] {
	match os {
		Os::Windows => KEPT_ENV,
		Os::Linux => KEPT_ENV_LINUX,
	}
}

/// Whether two environment names are one on `os`: Windows ignores case,
/// Linux does not.
pub fn same_key(os: Os, a: &str, b: &str) -> bool {
	match os {
		Os::Windows => a.eq_ignore_ascii_case(b),
		Os::Linux => a == b,
	}
}

/// A data root the engine reads: `dir` is listable, and holds nothing but
/// links to `subdirs`, the content directories that are readable. The
/// engine opens every directory it finds under a root and makes a fatal of
/// one it cannot open, so a root it sees must hold nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
	pub dir: PathBuf,
	pub subdirs: Vec<PathBuf>,
}

/// Whether the process may reach the network at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
	/// A game nobody joins over the network: a skirmish, a replay.
	None,
	/// Outbound only, for a game on a server.
	Outbound,
}

/// Everything a confined process may touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
	/// Read and execute: the engine's own directory, and modlobby's for the
	/// probe that runs before it.
	pub exe_dirs: Vec<PathBuf>,
	pub roots: Vec<Root>,
	/// The one directory it may write.
	pub write: PathBuf,
	pub network: Network,
	/// Its whole environment, already scrubbed.
	pub env: Vec<(OsString, OsString)>,
	/// Where the grants made for it are recorded, so they can be revoked.
	pub grants_file: PathBuf,
}

/// The parent's environment reduced to what this platform keeps, in the
/// parent's order.
pub fn scrubbed_env(
	parent: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
	scrubbed_env_on(parent, this_os())
}

/// [`scrubbed_env`] by `os`'s rules: its list, its way of comparing names.
pub fn scrubbed_env_on(
	parent: impl IntoIterator<Item = (OsString, OsString)>,
	os: Os,
) -> Vec<(OsString, OsString)> {
	parent
		.into_iter()
		.filter(|(key, _)| {
			let key = key.to_string_lossy();
			kept_env(os).iter().any(|kept| same_key(os, kept, &key))
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn vars(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
		pairs
			.iter()
			.map(|(key, value)| (OsString::from(key), OsString::from(value)))
			.collect()
	}

	#[test]
	fn the_environment_keeps_only_what_the_engine_needs() {
		let kept = scrubbed_env_on(
			vars(&[
				("GITHUB_TOKEN", "x"),
				("PATH", "p"),
				("MODLOBBY_PASSWORD", "s"),
				("TEMP", "t"),
			]),
			Os::Windows,
		);
		assert_eq!(kept, vars(&[("PATH", "p"), ("TEMP", "t")]));
	}

	#[test]
	fn env_names_match_case_insensitively() {
		assert_eq!(
			scrubbed_env_on(vars(&[("Path", "p")]), Os::Windows),
			vars(&[("Path", "p")])
		);
	}

	#[test]
	fn the_linux_environment_keeps_the_display_and_audio() {
		let linux = kept_env(Os::Linux);
		for key in [
			"PATH",
			"HOME",
			"LANG",
			"XDG_RUNTIME_DIR",
			"WAYLAND_DISPLAY",
			"PULSE_SERVER",
			"SDL_VIDEODRIVER",
			"SPRING_DATADIR",
			"SPRING_WRITEDIR",
			"SPRING_ISOLATED",
			"PWD",
		] {
			assert!(linux.contains(&key), "{key}");
		}
		assert!(!linux.contains(&"DISPLAY"), "X11 is never handed in");
		assert!(!linux.contains(&"DBUS_SESSION_BUS_ADDRESS"));
		assert!(kept_env(Os::Windows).contains(&"LOCALAPPDATA"));
		let kept = scrubbed_env_on(
			vars(&[
				("WAYLAND_DISPLAY", "wayland-0"),
				("DISPLAY", ":0"),
				("GITHUB_TOKEN", "x"),
				("Path", "p"),
			]),
			Os::Linux,
		);
		assert_eq!(kept, vars(&[("WAYLAND_DISPLAY", "wayland-0")]));
	}
}
