//! The bubblewrap command line a confined engine starts under on Linux.
//!
//! Pure: every path it binds comes in through the [`Policy`] and the
//! [`Session`], which the Linux confinement reads off the host. Each view,
//! content directory and executable directory is bound read-only at its own
//! path, so the paths the engine is told are the paths it finds; the mods
//! write directory is the one writable place, beside a private `/tmp` and
//! home that vanish with the sandbox.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::{Network, Policy};

/// A piece of the host the sandbox's root is built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum System {
	/// Bound read-only at its own path.
	Bind(PathBuf),
	/// A symbolic link at `at` to `to`, as a merged-`/usr` host has `/lib`.
	Link { at: PathBuf, to: PathBuf },
}

/// What the sandbox takes from the session it runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
	/// `/usr`, its merged-`/usr` links, the `/etc` entries and `/sys`
	/// subtrees libraries and drivers read.
	pub system: Vec<System>,
	/// The GPU nodes there are.
	pub devices: Vec<PathBuf>,
	/// Sockets bound in from the host, `(host, inside)`: the Wayland
	/// proxy's, and the audio server's.
	pub sockets: Vec<(PathBuf, PathBuf)>,
	/// The sandbox's `XDG_RUNTIME_DIR`, made inside, holding the sockets.
	pub runtime: PathBuf,
	/// The sandbox's home: private, empty, gone with it.
	pub home: PathBuf,
}

/// bwrap's arguments: `helper --sandbox-exec <landlock> -- <command>` run in
/// a fresh root built from `policy` and `session`, with exactly `env`.
pub fn args(
	policy: &Policy,
	session: &Session,
	helper: &Path,
	landlock: &[OsString],
	command: &[OsString],
	env: &[(OsString, OsString)],
) -> Vec<OsString> {
	let mut out: Vec<OsString> = [
		"--unshare-user",
		"--unshare-pid",
		"--unshare-ipc",
		"--unshare-uts",
		"--unshare-cgroup-try",
		"--new-session",
	]
	.map(OsString::from)
	.to_vec();
	if policy.network == Network::None {
		out.push("--unshare-net".into());
	}
	let flag = |out: &mut Vec<OsString>, name: &str, values: &[&Path]| {
		out.push(name.into());
		out.extend(values.iter().map(|value| value.as_os_str().to_owned()));
	};
	for piece in &session.system {
		match piece {
			System::Bind(path) => flag(&mut out, "--ro-bind", &[path, path]),
			System::Link { at, to } => flag(&mut out, "--symlink", &[to, at]),
		}
	}
	flag(&mut out, "--proc", &[Path::new("/proc")]);
	flag(&mut out, "--dev", &[Path::new("/dev")]);
	for device in &session.devices {
		flag(&mut out, "--dev-bind", &[device, device]);
	}
	flag(&mut out, "--tmpfs", &[Path::new("/tmp")]);
	flag(&mut out, "--tmpfs", &[&session.home]);
	flag(&mut out, "--dir", &[&session.runtime]);
	for (host, inside) in &session.sockets {
		flag(&mut out, "--ro-bind", &[host, inside]);
	}
	// Each read-only place once, at its own path: the views, the content
	// their links point at, the engine's directory and the helper's.
	let mut read_only: Vec<&Path> = Vec::new();
	let roots = policy
		.roots
		.iter()
		.flat_map(|root| std::iter::once(&root.dir).chain(&root.subdirs));
	let helper_dir = helper.parent().unwrap_or(helper);
	for path in roots
		.chain(&policy.exe_dirs)
		.map(PathBuf::as_path)
		.chain(std::iter::once(helper_dir))
	{
		if !read_only.contains(&path) {
			read_only.push(path);
		}
	}
	for path in read_only {
		flag(&mut out, "--ro-bind", &[path, path]);
	}
	flag(&mut out, "--bind", &[&policy.write, &policy.write]);
	flag(&mut out, "--chdir", &[&policy.write]);
	out.push("--clearenv".into());
	for (key, value) in env {
		out.extend(["--setenv".into(), key.clone(), value.clone()]);
	}
	// Last of the mounts: the root the mount points were made in is
	// read-only from here on, so nothing can be written outside the mods
	// directory, `/tmp` and the home.
	flag(&mut out, "--remount-ro", &[Path::new("/")]);
	out.push("--".into());
	out.push(helper.as_os_str().to_owned());
	out.push(crate::inside::EXEC.into());
	out.extend(landlock.iter().cloned());
	out.push("--".into());
	out.extend(command.iter().cloned());
	out
}

/// Why this session cannot hold a modded game, read from its environment;
/// `None` for a Wayland session.
pub fn session_refusal(env: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Option<String> {
	let set = |key: &str| env(key).filter(|value| !value.is_empty());
	if set("XDG_SESSION_TYPE").is_some_and(|kind| kind.eq_ignore_ascii_case("x11")) {
		return Some(
			"this is an X11 session: modded games run only under Wayland, where a game 			 cannot read the keys or pixels of other windows"
				.into(),
		);
	}
	if set("WAYLAND_DISPLAY").is_none() {
		return Some(
			"no Wayland display: modded games run only under Wayland, where a game 			 cannot read the keys or pixels of other windows"
				.into(),
		);
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Root;

	fn policy(network: Network) -> Policy {
		Policy {
			exe_dirs: vec!["/data/engine/2026.09".into()],
			roots: vec![Root {
				dir: "/cfg/mods-view/0".into(),
				subdirs: vec!["/data/games".into(), "/data/maps".into()],
			}],
			write: "/cfg/mods-data".into(),
			network,
			env: Vec::new(),
			grants_file: "/cfg/sandbox-grants.json".into(),
		}
	}

	fn session() -> Session {
		Session {
			system: vec![
				System::Bind("/usr".into()),
				System::Link {
					at: "/lib".into(),
					to: "usr/lib".into(),
				},
				System::Bind("/etc/ld.so.cache".into()),
			],
			devices: vec!["/dev/dri".into()],
			sockets: vec![(
				"/cfg/sandbox-run/wayland-0".into(),
				"/run/sandbox/wayland-0".into(),
			)],
			runtime: "/run/sandbox".into(),
			home: "/home/modlobby".into(),
		}
	}

	fn built(network: Network) -> Vec<String> {
		args(
			&policy(network),
			&session(),
			Path::new("/cfg/sandbox-run/bin/modlobby-sandbox"),
			&["--write".into(), "/cfg/mods-data".into()],
			&["/data/engine/2026.09/spring".into(), "--isolation".into()],
			&[
				("SPRING_WRITEDIR".into(), "/cfg/mods-data".into()),
				("WAYLAND_DISPLAY".into(), "wayland-0".into()),
			],
		)
		.into_iter()
		.map(|arg| arg.to_string_lossy().into_owned())
		.collect()
	}

	/// The arguments of every `flag` among bwrap's options, `count` each.
	fn options(args: &[String], flag: &str, count: usize) -> Vec<Vec<String>> {
		let end = args.iter().position(|arg| arg == "--").unwrap();
		args[..end]
			.iter()
			.enumerate()
			.filter(|(_, arg)| *arg == flag)
			.map(|(at, _)| args[at + 1..at + 1 + count].to_vec())
			.collect()
	}

	#[test]
	fn only_the_write_directory_is_writable() {
		let args = built(Network::Outbound);
		assert_eq!(
			options(&args, "--bind", 2),
			[["/cfg/mods-data", "/cfg/mods-data"]]
		);
		let read_only = options(&args, "--ro-bind", 2);
		for path in [
			"/usr",
			"/etc/ld.so.cache",
			"/cfg/mods-view/0",
			"/data/games",
			"/data/maps",
			"/data/engine/2026.09",
			"/cfg/sandbox-run/bin",
		] {
			assert!(
				read_only.contains(&vec![path.to_owned(), path.to_owned()]),
				"{path} is not bound read-only at its own path: {read_only:?}"
			);
		}
		assert!(read_only.contains(&vec![
			"/cfg/sandbox-run/wayland-0".to_owned(),
			"/run/sandbox/wayland-0".to_owned()
		]));
		assert_eq!(options(&args, "--dev-bind", 2), [["/dev/dri", "/dev/dri"]]);
		assert_eq!(options(&args, "--symlink", 2), [["usr/lib", "/lib"]]);
		// Nothing of the host's but what was named: no blanket binds.
		assert!(!read_only.contains(&vec!["/".to_owned(), "/".to_owned()]));
		assert!(options(&args, "--bind-try", 2).is_empty());
	}

	/// The root holding the mount points is read-only once everything is
	/// mounted; `/tmp` and the home are mounts of their own and stay
	/// writable, and vanish with the sandbox.
	#[test]
	fn the_root_is_read_only_after_everything_is_mounted() {
		let args = built(Network::Outbound);
		let end = args.iter().position(|arg| arg == "--").unwrap();
		assert_eq!(&args[end - 2..end], ["--remount-ro", "/"]);
		let tmpfs = options(&args, "--tmpfs", 1);
		assert!(tmpfs.contains(&vec!["/tmp".to_owned()]), "{tmpfs:?}");
		assert!(
			tmpfs.contains(&vec!["/home/modlobby".to_owned()]),
			"{tmpfs:?}"
		);
		assert_eq!(options(&args, "--chdir", 1), [["/cfg/mods-data"]]);
	}

	#[test]
	fn the_environment_is_cleared_and_set() {
		let args = built(Network::Outbound);
		let clear = args.iter().position(|arg| arg == "--clearenv").unwrap();
		let first_set = args.iter().position(|arg| arg == "--setenv").unwrap();
		assert!(
			clear < first_set,
			"the environment is cleared before it is set"
		);
		assert_eq!(
			options(&args, "--setenv", 2),
			[
				["SPRING_WRITEDIR", "/cfg/mods-data"],
				["WAYLAND_DISPLAY", "wayland-0"]
			]
		);
	}

	#[test]
	fn a_game_without_the_network_unshares_it() {
		assert!(built(Network::None).contains(&"--unshare-net".to_owned()));
		assert!(!built(Network::Outbound).contains(&"--unshare-net".to_owned()));
	}

	/// A new session, so the game cannot type into the terminal modlobby
	/// was started from; every namespace but the network its own; and no
	/// tie to modlobby's life, since a lobby restart must not end a match.
	#[test]
	fn the_session_is_new_and_namespaces_are_closed() {
		let args = built(Network::Outbound);
		for flag in [
			"--new-session",
			"--unshare-user",
			"--unshare-pid",
			"--unshare-ipc",
			"--unshare-uts",
			"--unshare-cgroup-try",
		] {
			assert!(args.contains(&flag.to_owned()), "{flag}");
		}
		assert!(!args.contains(&"--die-with-parent".to_owned()));
		assert!(!args.contains(&"--share-net".to_owned()));
	}

	#[test]
	fn the_helper_runs_first_and_execs_the_command() {
		let args = built(Network::None);
		let end = args.iter().position(|arg| arg == "--").unwrap();
		assert_eq!(
			&args[end..],
			[
				"--",
				"/cfg/sandbox-run/bin/modlobby-sandbox",
				"--sandbox-exec",
				"--write",
				"/cfg/mods-data",
				"--",
				"/data/engine/2026.09/spring",
				"--isolation",
			]
		);
	}

	fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
		move |key| {
			pairs
				.iter()
				.find(|(held, _)| *held == key)
				.map(|(_, value)| OsString::from(value))
		}
	}

	/// X11 lets any window read every other window's keys and pixels, so a
	/// sandbox cannot hold a game that gets one: an X11 session refuses, and
	/// so does one with no Wayland display to proxy.
	#[test]
	fn an_x11_session_refuses() {
		let x11 = session_refusal(&env_of(&[
			("XDG_SESSION_TYPE", "x11"),
			("WAYLAND_DISPLAY", "wayland-0"),
			("DISPLAY", ":0"),
		]))
		.unwrap();
		assert!(x11.contains("X11"), "{x11}");
		let no_wayland =
			session_refusal(&env_of(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "")])).unwrap();
		assert!(no_wayland.contains("Wayland"), "{no_wayland}");
	}

	#[test]
	fn a_wayland_session_does_not() {
		assert_eq!(
			session_refusal(&env_of(&[
				("XDG_SESSION_TYPE", "wayland"),
				("WAYLAND_DISPLAY", "wayland-0"),
				("DISPLAY", ":0"),
			])),
			None
		);
		// WSLg sets no session type; its Wayland display is what counts.
		assert_eq!(
			session_refusal(&env_of(&[("WAYLAND_DISPLAY", "wayland-0")])),
			None
		);
	}
}
