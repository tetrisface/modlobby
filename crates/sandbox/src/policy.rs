//! What a confined process may touch.

use std::ffi::OsString;
use std::path::PathBuf;

/// The subdirectories of a data root that hold game content and nothing
/// personal. The root itself holds the player's settings, configs and logs,
/// which stay unreadable.
pub const CONTENT_SUBDIRS: &[&str] = &["games", "maps", "packages", "pool", "rapid"];

/// The environment variables a confined engine gets. Everything else in the
/// parent's environment -- tokens, passwords, the paths of other installs --
/// is dropped. `LOCALAPPDATA`, `TEMP` and `TMP` must be present for an
/// AppContainer to start at all; Windows rewrites them to the container's
/// own folder.
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

/// The parent's environment reduced to [`KEPT_ENV`], in the parent's order.
/// Names compare case-insensitively, as Windows does.
pub fn scrubbed_env(
	parent: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
	parent
		.into_iter()
		.filter(|(key, _)| {
			let key = key.to_string_lossy();
			KEPT_ENV.iter().any(|kept| kept.eq_ignore_ascii_case(&key))
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
		let kept = scrubbed_env(vars(&[
			("GITHUB_TOKEN", "x"),
			("PATH", "p"),
			("MODLOBBY_PASSWORD", "s"),
			("TEMP", "t"),
		]));
		assert_eq!(kept, vars(&[("PATH", "p"), ("TEMP", "t")]));
	}

	#[test]
	fn env_names_match_case_insensitively() {
		assert_eq!(scrubbed_env(vars(&[("Path", "p")])), vars(&[("Path", "p")]));
	}
}
