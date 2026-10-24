//! bubblewrap around the engine, on Linux under Wayland.
//!
//! bwrap builds a root of nothing but what the policy names (see
//! [`crate::bwrap`]), in namespaces of its own; inside it modlobby's helper
//! restricts itself with Landlock and seccomp and becomes the engine (see
//! [`crate::inside`]); the engine sees the compositor only through the proxy
//! that hides the clipboard (see [`crate::wayland`]), started beside it as a
//! process of its own so a lobby restart leaves a running game its display.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::bwrap::{Session, System, session_refusal};
use crate::inside::{PROXY, Rules, rules_args};
use crate::{Confined, Confinement, Guard, Network, Policy};

/// Inside the sandbox: the runtime directory holding the sockets, and the
/// home, both made fresh.
const RUNTIME: &str = "/run/sandbox";
const HOME: &str = "/home/modlobby";

/// The `/etc` entries libraries and drivers read; nothing of the user's.
const ETC: &[&str] = &[
	"ld.so.cache",
	"ld.so.conf",
	"ld.so.conf.d",
	"fonts",
	"alsa",
	"pulse",
	"localtime",
	"resolv.conf",
	"hosts",
	"nsswitch.conf",
	"drirc",
	"vulkan",
	"glvnd",
	"alternatives",
];

/// The `/sys` subtrees a GL driver walks to find its device.
const SYS: &[&str] = &["block", "bus", "class", "dev", "devices"];

/// The top-level directories a merged-`/usr` host has as links.
const TOP: &[&str] = &["bin", "lib", "lib64", "lib32", "libx32", "sbin"];

pub struct Bubblewrap {
	/// modlobby's helper beside its own executable; copied out before use,
	/// since an AppImage's files vanish with the AppImage.
	helper: PathBuf,
}

impl Bubblewrap {
	pub fn new(helper: PathBuf) -> Self {
		Self { helper }
	}
}

/// `command` as the leader of a process group of its own: a Ctrl-C or a
/// hang-up meant for the lobby's terminal must not reach a running game.
fn detached(command: &mut Command) -> &mut Command {
	use std::os::unix::process::CommandExt as _;
	command.process_group(0)
}

/// bwrap on the `PATH`.
fn bwrap() -> Option<PathBuf> {
	let path = std::env::var_os("PATH")?;
	std::env::split_paths(&path)
		.map(|dir| dir.join("bwrap"))
		.find(|candidate| candidate.is_file())
}

/// Whether bwrap may make a user namespace here: once a run, a sandbox that
/// does nothing.
fn trial(bwrap: &Path) -> Result<(), String> {
	static TRIED: OnceLock<Result<(), String>> = OnceLock::new();
	TRIED
		.get_or_init(|| {
			let out = Command::new(bwrap)
				.args(["--unshare-user", "--ro-bind", "/", "/", "--", "/bin/true"])
				.stdin(Stdio::null())
				.output()
				.map_err(|err| err.to_string())?;
			if out.status.success() {
				Ok(())
			} else {
				Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
			}
		})
		.clone()
}

impl Confinement for Bubblewrap {
	fn unavailable(&self) -> Option<String> {
		if let Some(refusal) = session_refusal(&|key| std::env::var_os(key)) {
			return Some(refusal);
		}
		if !self.helper.is_file() {
			return Some(format!(
				"modlobby's sandbox helper is missing at {}",
				self.helper.display()
			));
		}
		let Some(bwrap) = bwrap() else {
			return Some(
				"bubblewrap (bwrap) is not installed, and modded games on Linux run inside it"
					.into(),
			);
		};
		trial(&bwrap)
			.err()
			.map(|why| format!("this system does not let bubblewrap make a user namespace: {why}"))
	}

	fn spawn(&self, command: &Command, policy: &Policy) -> Result<Confined, String> {
		if let Some(refusal) = self.unavailable() {
			return Err(refusal);
		}
		let bwrap = bwrap().ok_or("bubblewrap is gone")?;
		let run = policy
			.write
			.parent()
			.ok_or("the mods write directory has no parent")?
			.join("sandbox-run");
		std::fs::create_dir_all(&run)
			.map_err(|err| format!("the sandbox's run directory: {err}"))?;
		let helper = crate::probe::copied(&self.helper, &run.join("bin"))?;

		let proxy = Proxy::start(&helper, &run)?;
		let mut sockets = vec![(proxy.listen.clone(), Path::new(RUNTIME).join("wayland-0"))];
		let pulse = pulse_socket();
		if let Some(pulse) = &pulse {
			sockets.push((
				pulse.clone(),
				Path::new(RUNTIME).join("pulse").join("native"),
			));
		}
		let session = Session {
			system: system(),
			devices: devices(),
			sockets,
			runtime: RUNTIME.into(),
			home: HOME.into(),
		};
		let env = environment(command, policy, pulse.is_some());
		let rules = rules(policy, &session, &helper);
		let mut line: Vec<OsString> = vec![command.get_program().to_owned()];
		line.extend(command.get_args().map(OsStr::to_owned));
		let args = crate::bwrap::args(policy, &session, &helper, &rules_args(&rules), &line, &env);

		let log = std::fs::File::create(run.join("bwrap.log"))
			.map_err(|err| format!("the sandbox's log: {err}"))?;
		let child = detached(&mut Command::new(&bwrap))
			.args(args)
			.stdin(Stdio::null())
			.stdout(log.try_clone().map_err(|err| err.to_string())?)
			.stderr(log)
			.spawn()
			.map_err(|err| format!("starting bubblewrap: {err}"))?;
		let outer = child.id();
		let (init, engine) = inner_pids(outer);
		Ok(Confined::new(
			engine.unwrap_or(outer),
			Box::new(Held {
				child: Mutex::new(child),
				outer,
				init,
				engine,
				proxy: Mutex::new(Some(proxy.child)),
			}),
		))
	}
}

/// The proxy's process and the socket it listens on.
struct Proxy {
	child: Child,
	listen: PathBuf,
}

impl Proxy {
	/// The helper as the Wayland proxy for this launch, once its socket is
	/// there; a socket of its own each launch, since the last one lingers a
	/// while after its game.
	fn start(helper: &Path, run: &Path) -> Result<Self, String> {
		let upstream = upstream().ok_or("no Wayland display to hand the game")?;
		let stamp = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map(|since| since.as_nanos())
			.unwrap_or_default();
		let listen = run.join(format!("wayland-{stamp}"));
		let log = std::fs::File::create(run.join("wayland-proxy.log"))
			.map_err(|err| format!("the proxy's log: {err}"))?;
		let mut child = detached(&mut Command::new(helper))
			.arg(PROXY)
			.arg(&listen)
			.arg(&upstream)
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(log)
			.spawn()
			.map_err(|err| format!("starting the Wayland proxy: {err}"))?;
		let started = Instant::now();
		while !listen.exists() {
			if started.elapsed() > Duration::from_secs(3) {
				let _ = child.kill();
				return Err("the Wayland proxy did not start".into());
			}
			std::thread::sleep(Duration::from_millis(20));
		}
		Ok(Self { child, listen })
	}
}

/// The compositor's socket: `WAYLAND_DISPLAY` as a path, or under
/// `XDG_RUNTIME_DIR`.
fn upstream() -> Option<PathBuf> {
	let display = PathBuf::from(std::env::var_os("WAYLAND_DISPLAY").filter(|d| !d.is_empty())?);
	if display.is_absolute() {
		return Some(display);
	}
	Some(PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join(display))
}

/// The audio server's socket, when there is one: `PULSE_SERVER` as
/// `unix:<path>`, else the usual place under `XDG_RUNTIME_DIR`.
fn pulse_socket() -> Option<PathBuf> {
	if let Some(server) = std::env::var("PULSE_SERVER").ok()
		&& let Some(path) = server.strip_prefix("unix:")
	{
		return Some(PathBuf::from(path)).filter(|path| path.exists());
	}
	let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
	Some(runtime.join("pulse").join("native")).filter(|path| path.exists())
}

/// `/usr`, the merged-`/usr` links or directories, the `/etc` entries and
/// `/sys` subtrees this host has.
fn system() -> Vec<System> {
	let mut pieces = vec![System::Bind("/usr".into())];
	for top in TOP {
		let at = Path::new("/").join(top);
		match std::fs::read_link(&at) {
			Ok(to) => pieces.push(System::Link { at, to }),
			Err(_) if at.is_dir() => pieces.push(System::Bind(at)),
			Err(_) => {}
		}
	}
	let existing = |base: &str, names: &[&str]| -> Vec<System> {
		names
			.iter()
			.map(|name| Path::new(base).join(name))
			.filter(|path| path.exists())
			.map(System::Bind)
			.collect()
	};
	pieces.extend(existing("/etc", ETC));
	pieces.extend(existing("/sys", SYS));
	pieces
}

/// The GPU's nodes this host has: the DRM nodes, WSL's, NVIDIA's.
fn devices() -> Vec<PathBuf> {
	let mut nodes: Vec<PathBuf> = ["/dev/dri", "/dev/dxg"]
		.iter()
		.map(PathBuf::from)
		.filter(|path| path.exists())
		.collect();
	if let Ok(entries) = std::fs::read_dir("/dev") {
		let mut nvidia: Vec<PathBuf> = entries
			.filter_map(Result::ok)
			.filter(|entry| entry.file_name().to_string_lossy().starts_with("nvidia"))
			.map(|entry| entry.path())
			.collect();
		nvidia.sort();
		nodes.extend(nvidia);
	}
	nodes
}

/// The engine's whole environment: the policy's, the command's own (the
/// data directories), and the sandbox's -- its runtime directory, the
/// proxied display, the audio socket, its home and caches, and the write
/// directory as `SPRING_WRITEDIR` and `SPRING_ISOLATED`, so an engine
/// started again from Lua, without its arguments, keeps them.
fn environment(command: &Command, policy: &Policy, pulse: bool) -> Vec<(OsString, OsString)> {
	let write = policy.write.as_os_str().to_owned();
	let mut ours: Vec<(OsString, OsString)> = vec![
		("XDG_RUNTIME_DIR".into(), RUNTIME.into()),
		("WAYLAND_DISPLAY".into(), "wayland-0".into()),
		("SDL_VIDEODRIVER".into(), "wayland".into()),
		("HOME".into(), HOME.into()),
		(
			"XDG_CACHE_HOME".into(),
			policy.write.join("cache").into_os_string(),
		),
		("SPRING_WRITEDIR".into(), write.clone()),
		("SPRING_ISOLATED".into(), write),
	];
	if pulse {
		let socket = Path::new(RUNTIME).join("pulse").join("native");
		let mut server = OsString::from("unix:");
		server.push(socket);
		ours.push(("PULSE_SERVER".into(), server));
	}
	let set_by_us = |key: &OsStr| ours.iter().any(|(held, _)| held == key);
	let mut env: Vec<(OsString, OsString)> = policy
		.env
		.iter()
		.filter(|(key, _)| !set_by_us(key))
		.cloned()
		.collect();
	for (key, value) in command.get_envs() {
		if let Some(value) = value
			&& !set_by_us(key)
		{
			env.retain(|(held, _)| held != key);
			env.push((key.to_owned(), value.to_owned()));
		}
	}
	env.extend(ours);
	env
}

/// What Landlock lets the process inside do, under bwrap's root: read the
/// content and the system, execute the system's programs and libraries, the
/// engine and the helper, and write the mods directory, `/tmp`, its home
/// and its devices.
fn rules(policy: &Policy, session: &Session, helper: &Path) -> Rules {
	let mut read: Vec<PathBuf> = policy
		.roots
		.iter()
		.flat_map(|root| std::iter::once(root.dir.clone()).chain(root.subdirs.iter().cloned()))
		.collect();
	read.extend(session.system.iter().filter_map(|piece| match piece {
		System::Bind(path) if !path.starts_with("/usr") => Some(path.clone()),
		_ => None,
	}));
	read.extend([PathBuf::from("/proc"), session.runtime.clone()]);
	let mut exec = vec![PathBuf::from("/usr")];
	exec.extend(session.system.iter().filter_map(|piece| match piece {
		System::Bind(path) if TOP.iter().any(|top| path == &Path::new("/").join(top)) => {
			Some(path.clone())
		}
		_ => None,
	}));
	exec.extend(policy.exe_dirs.iter().cloned());
	exec.extend(helper.parent().map(Path::to_path_buf));
	let write = vec![
		policy.write.clone(),
		PathBuf::from("/tmp"),
		session.home.clone(),
		PathBuf::from("/dev"),
	];
	Rules {
		read,
		exec,
		write,
		online: policy.network == Network::Outbound,
	}
}

/// The host pids of the sandbox's first process and of the engine below
/// it, when they show within a few seconds: the engine is what an adopting
/// lobby finds by its binary, and ending the first process ends every
/// process in the sandbox.
fn inner_pids(outer: u32) -> (Option<u32>, Option<u32>) {
	let started = Instant::now();
	let child_of = |pid: u32| -> Option<u32> {
		let listed = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).ok()?;
		listed.split_whitespace().next()?.parse().ok()
	};
	let mut init = None;
	while started.elapsed() < Duration::from_secs(3) {
		init = init.or_else(|| child_of(outer));
		if let Some(init) = init
			&& let Some(engine) = child_of(init)
		{
			return (Some(init), Some(engine));
		}
		std::thread::sleep(Duration::from_millis(20));
	}
	(init, None)
}

/// bwrap as the guard holds it: its own process to wait on, the pids to end
/// it by, and the proxy to stop with it.
struct Held {
	child: Mutex<Child>,
	outer: u32,
	init: Option<u32>,
	engine: Option<u32>,
	proxy: Mutex<Option<Child>>,
}

impl Held {
	fn stop_proxy(&self) {
		if let Ok(mut proxy) = self.proxy.lock()
			&& let Some(mut child) = proxy.take()
		{
			let _ = child.kill();
			let _ = child.wait();
		}
	}
}

impl Guard for Held {
	fn kill(&self) -> bool {
		// The sandbox's first process takes every process in it along; the
		// engine and bwrap itself are ended too, in case it was not found.
		let mut ended = false;
		for pid in [self.init, self.engine, Some(self.outer)]
			.into_iter()
			.flatten()
		{
			// SAFETY: kill(2) with a pid of a process we started.
			ended |= unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) } == 0;
		}
		self.stop_proxy();
		ended
	}

	fn wait(&self) -> Option<i32> {
		let status = self.child.lock().ok()?.wait().ok();
		self.stop_proxy();
		status?.code()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The proxy and bwrap lead process groups of their own: a Ctrl-C or a
	/// hang-up meant for the lobby's terminal does not reach the game.
	#[test]
	fn a_detached_process_leads_its_own_group() {
		let mut command = Command::new("/bin/sh");
		command.args(["-c", "cut -d' ' -f5 /proc/self/stat"]);
		let child = detached(&mut command)
			.stdout(Stdio::piped())
			.spawn()
			.unwrap();
		let pid = child.id();
		let out = child.wait_with_output().unwrap();
		let group: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
		assert_eq!(group, pid);
	}

	/// A stable id of the machine is no game's business.
	#[test]
	fn the_machine_id_is_not_bound() {
		assert!(!ETC.contains(&"machine-id"));
	}
}
