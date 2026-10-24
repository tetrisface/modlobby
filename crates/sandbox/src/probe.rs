//! The fail-closed check before a confined launch: modlobby's own exe, run
//! inside the same container with the same policy, tries what a mod could
//! try and reports what it managed. A launch follows only a report where
//! everything that must be denied was.

use std::ffi::OsString;
use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::{Confinement, Os, Policy, kept_env, same_key, this_os};

/// The argument that turns modlobby into the probe.
pub const ARG: &str = "--sandbox-probe";

/// How long the probe may take; it needs well under a second.
const PATIENCE: Duration = Duration::from_secs(15);

/// `name\tbool\tdetail` lines, one per check, in this order.
const CHECKS: &[&str] = &[
	"read_canary",
	"write_inside",
	"write_outside",
	"clipboard_read",
	"child_process",
	"tcp_loopback",
	"env_keys",
];

/// What must come back `false` on `os`: everything a mod would want. A
/// child process on Linux stays inside the same namespaces, Landlock domain
/// and seccomp filter, and a relaunch from Lua keeps its write directory and
/// config through the environment, so there it is only reported.
fn must_be_denied(os: Os) -> &'static [&'static str] {
	match os {
		Os::Windows => &[
			"read_canary",
			"write_outside",
			"clipboard_read",
			"child_process",
			"tcp_loopback",
		],
		Os::Linux => &[
			"read_canary",
			"write_outside",
			"clipboard_read",
			"tcp_loopback",
		],
	}
}

/// Why a file outside the boundary may fail on `os` and still prove it: on
/// Windows the container's token is refused; on Linux the file is not in the
/// sandbox's mount namespace at all, or the root it would be written in is
/// read-only, or Landlock refused it.
fn denied_for_the_boundary(os: Os, detail: &str) -> bool {
	match os {
		Os::Windows => detail == "permission denied",
		Os::Linux => matches!(
			detail,
			"permission denied" | "entity not found" | "read-only filesystem or storage medium"
		),
	}
}

/// What the Linux probe reports when the compositor, through the proxy,
/// offers no clipboard to read.
pub const NO_CLIPBOARD: &str = "no clipboard offered";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeArgs {
	/// Where the report goes: inside the write directory.
	pub report: PathBuf,
	/// A file outside every grant, which must not be readable.
	pub canary: PathBuf,
	/// A path outside every grant, which must not be writable.
	pub outside: PathBuf,
	/// A port modlobby listens on, which must not be reachable.
	pub loopback_port: u16,
	/// The real data roots, whose private files must stay unreadable.
	pub roots: Vec<PathBuf>,
}

/// The probe's arguments from a command line, when it has them.
pub fn arg(args: impl IntoIterator<Item = OsString>) -> Option<ProbeArgs> {
	let mut args = args.into_iter().skip_while(|arg| arg != ARG);
	args.next()?;
	let report = PathBuf::from(args.next()?);
	let canary = PathBuf::from(args.next()?);
	let outside = PathBuf::from(args.next()?);
	let loopback_port = args.next()?.to_str()?.parse().ok()?;
	let roots = args.map(PathBuf::from).collect();
	Some(ProbeArgs {
		report,
		canary,
		outside,
		loopback_port,
		roots,
	})
}

/// Runs inside the container. Writes the report and prints it; never prints
/// the contents of anything it read.
pub fn run_inside(args: &ProbeArgs) {
	let mut lines = Vec::new();
	let mut check = |name: &str, (ok, detail): (bool, String)| {
		lines.push(format!("{name}\t{ok}\t{detail}"));
	};
	check("read_canary", read(&args.canary));
	let inside = args.report.with_file_name("probe-wrote.txt");
	check("write_inside", write(&inside));
	check("write_outside", write(&args.outside));
	check("clipboard_read", clipboard());
	check("child_process", child());
	check(
		"tcp_loopback",
		tcp(SocketAddr::from(([127, 0, 0, 1], args.loopback_port))),
	);
	for (index, root) in args.roots.iter().enumerate() {
		check(&format!("root_{index}"), private_files(root));
	}
	let keys: Vec<String> = std::env::vars_os()
		.map(|(key, _)| key.to_string_lossy().into_owned())
		.collect();
	check("env_keys", (true, keys.join(",")));

	let report = lines.join("\n") + "\n";
	print!("{report}");
	if let Ok(mut file) = std::fs::File::create(&args.report) {
		let _ = file.write_all(report.as_bytes());
	}
}

fn read(path: &Path) -> (bool, String) {
	match std::fs::read(path) {
		Ok(bytes) => (true, format!("{} bytes", bytes.len())),
		Err(err) => (false, err.kind().to_string()),
	}
}

fn write(path: &Path) -> (bool, String) {
	match std::fs::write(path, b"probe") {
		Ok(()) => {
			let _ = std::fs::remove_file(path);
			(true, "written".into())
		}
		Err(err) => (false, err.kind().to_string()),
	}
}

#[cfg(windows)]
fn clipboard() -> (bool, String) {
	use windows::Win32::System::DataExchange::{CloseClipboard, OpenClipboard};
	unsafe {
		match OpenClipboard(None) {
			Ok(()) => {
				let _ = CloseClipboard();
				(true, "opened".into())
			}
			Err(err) => (false, err.to_string()),
		}
	}
}

/// Whether the compositor the sandbox hands out offers a clipboard: the
/// registry, read once over `WAYLAND_DISPLAY`, holds none of the globals
/// that would hand one out.
#[cfg(target_os = "linux")]
fn clipboard() -> (bool, String) {
	match wayland_globals() {
		Ok(globals) => match globals
			.iter()
			.find(|global| crate::wayland::HIDDEN.contains(&global.as_str()))
		{
			Some(offered) => (true, offered.clone()),
			None => (false, NO_CLIPBOARD.into()),
		},
		Err(why) => (false, format!("the display could not be asked: {why}")),
	}
}

/// The interfaces the compositor advertises: `get_registry`, then `sync`,
/// and every `global` until the sync's `done`.
#[cfg(target_os = "linux")]
fn wayland_globals() -> Result<Vec<String>, String> {
	use std::io::Read as _;
	let display = std::env::var_os("WAYLAND_DISPLAY").ok_or("no WAYLAND_DISPLAY")?;
	let path = match std::path::Path::new(&display) {
		path if path.is_absolute() => path.to_path_buf(),
		name => PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").ok_or("no XDG_RUNTIME_DIR")?)
			.join(name),
	};
	let mut socket =
		std::os::unix::net::UnixStream::connect(&path).map_err(|err| err.to_string())?;
	socket
		.set_read_timeout(Some(Duration::from_secs(3)))
		.map_err(|err| err.to_string())?;
	let request = |object: u32, opcode: u32, new_id: u32| -> Vec<u8> {
		let mut out = object.to_ne_bytes().to_vec();
		out.extend(((12_u32 << 16) | opcode).to_ne_bytes());
		out.extend(new_id.to_ne_bytes());
		out
	};
	const REGISTRY: u32 = 2;
	const CALLBACK: u32 = 3;
	let mut asked = request(1, 1, REGISTRY);
	asked.extend(request(1, 0, CALLBACK));
	socket.write_all(&asked).map_err(|err| err.to_string())?;
	let mut held: Vec<u8> = Vec::new();
	let mut globals = Vec::new();
	let mut chunk = [0_u8; 4096];
	loop {
		while held.len() >= 8 {
			let word =
				|at: usize| u32::from_ne_bytes(held[at..at + 4].try_into().unwrap_or([0; 4]));
			let (object, header) = (word(0), word(4));
			let size = (header >> 16) as usize;
			if size < 8 {
				return Err("the display sent what is not Wayland".into());
			}
			if held.len() < size {
				break;
			}
			let opcode = header & 0xffff;
			if object == CALLBACK && opcode == 0 {
				return Ok(globals);
			}
			if object == 1 && opcode == 0 {
				return Err("the display refused the request".into());
			}
			if object == REGISTRY && opcode == 0 && size >= 16 {
				let len = word(12) as usize;
				let end = (16 + len.saturating_sub(1)).min(size);
				globals.push(String::from_utf8_lossy(&held[16..end]).into_owned());
			}
			held.drain(..size);
		}
		let read = socket.read(&mut chunk).map_err(|err| err.to_string())?;
		if read == 0 {
			return Err("the display closed the connection".into());
		}
		held.extend_from_slice(&chunk[..read]);
	}
}

#[cfg(not(any(windows, target_os = "linux")))]
fn clipboard() -> (bool, String) {
	(false, "not checked on this platform".into())
}

fn child() -> (bool, String) {
	let program = if cfg!(windows) { "cmd.exe" } else { "true" };
	let mut command = std::process::Command::new(program);
	if cfg!(windows) {
		command.args(["/c", "exit", "0"]);
	}
	match command.status() {
		Ok(status) => (true, status.to_string()),
		// 367 is ERROR_CHILD_PROCESS_BLOCKED, the one answer that proves
		// the policy; any other failure is the probe's own problem.
		Err(err) if err.raw_os_error() == Some(367) => (false, "blocked (os error 367)".into()),
		Err(err) => (false, err.to_string()),
	}
}

/// Whether a root's private files can be read: its settings, or the
/// directory Chobby keeps its saved password in.
fn private_files(root: &Path) -> (bool, String) {
	match std::fs::read(root.join("springsettings.cfg")) {
		Ok(bytes) => return (true, format!("settings {} bytes", bytes.len())),
		Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
			return (false, err.kind().to_string());
		}
		Err(_) => {}
	}
	match std::fs::read_dir(root.join("LuaMenu").join("Config")) {
		Ok(entries) => (true, format!("LuaMenu/Config {} entries", entries.count())),
		Err(err) => (false, err.kind().to_string()),
	}
}

fn tcp(addr: SocketAddr) -> (bool, String) {
	match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
		Ok(_) => (true, "connected".into()),
		Err(err) => (false, err.kind().to_string()),
	}
}

/// A report as the probe wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report(pub Vec<(String, bool, String)>);

pub fn parse(text: &str) -> Report {
	Report(
		text.lines()
			.filter_map(|line| {
				let mut parts = line.splitn(3, '\t');
				let name = parts.next()?.to_owned();
				let ok = parts.next()? == "true";
				let detail = parts.next().unwrap_or("").to_owned();
				Some((name, ok, detail))
			})
			.collect(),
	)
}

/// `Ok` when every check came back as the boundary demands: denied for
/// the reason the boundary gives, not for want of a file or a program; else
/// the names of the checks that did not, a missing one among them.
pub fn verdict(report: &Report) -> Result<(), Vec<String>> {
	verdict_on(report, this_os())
}

/// [`verdict`] by `os`'s rules.
pub fn verdict_on(report: &Report, os: Os) -> Result<(), Vec<String>> {
	let find = |name: &str| report.0.iter().find(|(held, _, _)| held == name);
	let mut failed: Vec<String> = CHECKS
		.iter()
		.filter(|name| {
			let Some((_, ok, detail)) = find(name) else {
				return true;
			};
			match **name {
				"write_inside" => !*ok,
				"env_keys" => detail
					.split(',')
					.filter(|key| !key.is_empty())
					.any(|key| !kept_env(os).iter().any(|kept| same_key(os, kept, key))),
				"read_canary" | "write_outside" => *ok || !denied_for_the_boundary(os, detail),
				"child_process" if os == Os::Linux => false,
				"child_process" => *ok || !detail.starts_with("blocked"),
				"clipboard_read" if os == Os::Linux => *ok || detail != NO_CLIPBOARD,
				"tcp_loopback" => {
					*ok || !matches!(
						detail.as_str(),
						"timed out" | "connection refused" | "permission denied"
					)
				}
				name if must_be_denied(os).contains(&name) => *ok,
				_ => false,
			}
		})
		.map(|name| (*name).to_owned())
		.collect();
	failed.extend(
		report
			.0
			.iter()
			.filter(|(name, ok, _)| name.starts_with("root_") && *ok)
			.map(|(name, _, _)| name.clone()),
	);
	if failed.is_empty() {
		Ok(())
	} else {
		Err(failed)
	}
}

/// What a verdict covers: this policy and no other. Another root set or
/// network mode is another boundary.
pub fn fingerprint(policy: &Policy) -> String {
	use std::hash::{Hash, Hasher};
	let mut hasher = std::collections::hash_map::DefaultHasher::new();
	format!("{:?}", policy.network).hash(&mut hasher);
	policy.exe_dirs.hash(&mut hasher);
	for root in &policy.roots {
		root.dir.hash(&mut hasher);
		root.subdirs.hash(&mut hasher);
	}
	policy.write.hash(&mut hasher);
	format!("{:016x}", hasher.finish())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Cached {
	version: String,
	os_build: String,
	policy: String,
	holds: bool,
}

fn cached(cache: &Path, os_build: &str, version: &str, policy: &str) -> bool {
	std::fs::read_to_string(cache)
		.ok()
		.and_then(|text| serde_json::from_str::<Cached>(&text).ok())
		.is_some_and(|held| {
			held.holds
				&& held.os_build == os_build
				&& held.version == version
				&& held.policy == policy
		})
}

/// Proves the boundary holds before an engine is started inside it: a
/// cached verdict for this modlobby and this OS build, else a probe run
/// through `confinement` under `policy`. `policy.exe_dirs` must cover
/// `self_exe`'s directory. `Err` says why a launch may not go ahead.
pub async fn ensure(
	confinement: &Arc<dyn Confinement>,
	policy: &Policy,
	self_exe: &Path,
	cache: &Path,
	os_build: &str,
	version: &str,
) -> Result<(), String> {
	ensure_with(
		confinement,
		policy,
		self_exe,
		cache,
		os_build,
		version,
		PATIENCE,
	)
	.await
}

/// [`ensure`] with the probe's patience spelled out; the tests give it less.
pub async fn ensure_with(
	confinement: &Arc<dyn Confinement>,
	policy: &Policy,
	self_exe: &Path,
	cache: &Path,
	os_build: &str,
	version: &str,
	patience: Duration,
) -> Result<(), String> {
	let policy_print = fingerprint(policy);
	if cached(cache, os_build, version, &policy_print) {
		return Ok(());
	}
	let outside_dir = policy
		.write
		.parent()
		.ok_or("the mods write directory has no parent to probe against")?;
	let canary = outside_dir.join("sandbox-canary.txt");
	let outside = outside_dir.join("sandbox-outside.txt");
	let report = policy.write.join("probe-report.txt");
	std::fs::create_dir_all(&policy.write).map_err(|err| err.to_string())?;
	std::fs::write(&canary, b"canary").map_err(|err| format!("planting the canary: {err}"))?;
	let _ = std::fs::remove_file(&outside);
	let _ = std::fs::remove_file(&report);
	let listener = TcpListener::bind("127.0.0.1:0").map_err(|err| err.to_string())?;
	let port = listener.local_addr().map_err(|err| err.to_string())?.port();

	// Its own copy of modlobby's exe in a directory holding nothing else:
	// the grant covers that directory, never the one modlobby runs from,
	// which in a developer's checkout is a target tree of a million files.
	let probe_dir = cache
		.parent()
		.ok_or("the probe cache has no directory")?
		.join("sandbox-probe");
	let probe_exe = copied(self_exe, &probe_dir)?;
	let mut probe_policy = policy.clone();
	probe_policy.exe_dirs.push(probe_dir);
	let mut command = std::process::Command::new(&probe_exe);
	command
		.arg(ARG)
		.arg(&report)
		.arg(&canary)
		.arg(&outside)
		.arg(port.to_string())
		.current_dir(&policy.write);
	// The real roots are the parents of the content directories the views
	// link to; their private files must stay unreadable.
	let mut real_roots: Vec<PathBuf> = Vec::new();
	for root in &policy.roots {
		for sub in &root.subdirs {
			if let Some(parent) = sub.parent().map(Path::to_path_buf)
				&& !real_roots.contains(&parent)
			{
				real_roots.push(parent);
			}
		}
	}
	for real in &real_roots {
		command.arg(real);
	}
	let outcome = run(confinement, command, &probe_policy, &report, patience).await;
	let _ = std::fs::remove_file(&canary);
	let _ = std::fs::remove_file(&outside);
	drop(listener);
	outcome?;
	let held = Cached {
		version: version.to_owned(),
		os_build: os_build.to_owned(),
		policy: policy_print,
		holds: true,
	};
	if let Some(parent) = cache.parent() {
		let _ = std::fs::create_dir_all(parent);
	}
	std::fs::write(cache, serde_json::to_string(&held).unwrap_or_default())
		.map_err(|err| format!("recording the probe's verdict: {err}"))
}

/// `exe` copied into `dir` under its own name, when the copy is missing or
/// differs in size or modification time. Where the copy is.
pub(crate) fn copied(exe: &Path, dir: &Path) -> Result<PathBuf, String> {
	let failed = |err: std::io::Error| format!("copying modlobby for the probe: {err}");
	let name = exe.file_name().ok_or("modlobby's exe has no file name")?;
	let copy = dir.join(name);
	let same = |path: &Path| {
		std::fs::metadata(path)
			.ok()
			.map(|meta| (meta.len(), meta.modified().ok()))
	};
	if same(exe).is_some() && same(exe) == same(&copy) {
		return Ok(copy);
	}
	std::fs::create_dir_all(dir).map_err(failed)?;
	std::fs::copy(exe, &copy).map_err(failed)?;
	Ok(copy)
}

async fn run(
	confinement: &Arc<dyn Confinement>,
	command: std::process::Command,
	policy: &Policy,
	report: &Path,
	patience: Duration,
) -> Result<(), String> {
	// Granting is slow on a large content tree; off the async threads.
	let confined = {
		let confinement = Arc::clone(confinement);
		let policy = policy.clone();
		tokio::task::spawn_blocking(move || confinement.spawn(&command, &policy))
			.await
			.map_err(|err| format!("the probe's launch task: {err}"))??
	};
	let confined = Arc::new(confined);
	if tokio::time::timeout(patience, confined.wait())
		.await
		.is_err()
	{
		confined.kill();
		return Err(format!(
			"the sandbox probe did not finish in {:.2} s",
			patience.as_secs_f64()
		));
	}
	let text = std::fs::read_to_string(report)
		.map_err(|err| format!("the sandbox probe left no report: {err}"))?;
	verdict(&parse(&text))
		.map_err(|failed| format!("the sandbox did not hold: {}", failed.join(", ")))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Confined, Guard, Network, Os};
	use std::sync::atomic::{AtomicBool, Ordering};
	use std::sync::{Condvar, Mutex};

	const HOLDS: &str = "read_canary\tfalse\tpermission denied\nwrite_inside\ttrue\twritten\nwrite_outside\tfalse\tpermission denied\nclipboard_read\tfalse\tdenied\nchild_process\tfalse\tblocked\ntcp_loopback\tfalse\ttimed out\nenv_keys\ttrue\tPATH,TEMP\n";

	#[test]
	fn a_report_that_holds_passes() {
		assert_eq!(verdict_on(&parse(HOLDS), Os::Windows), Ok(()));
	}

	/// Denied for the wrong reason is not denied: a canary that was never
	/// there, or a child that was not found, proves nothing.
	#[test]
	fn a_denial_for_the_wrong_reason_fails_closed() {
		let not_there = HOLDS.replace(
			"read_canary\tfalse\tpermission denied",
			"read_canary\tfalse\tentity not found",
		);
		assert_eq!(
			verdict_on(&parse(&not_there), Os::Windows),
			Err(vec!["read_canary".to_owned()])
		);
		let no_cmd = HOLDS.replace(
			"child_process\tfalse\tblocked",
			"child_process\tfalse\tprogram not found",
		);
		assert_eq!(
			verdict_on(&parse(&no_cmd), Os::Windows),
			Err(vec!["child_process".to_owned()])
		);
	}

	/// A root whose private files are readable -- an install under Program
	/// Files, which every AppContainer may read -- cannot be made safe.
	#[test]
	fn a_root_whose_settings_are_readable_fails_closed() {
		let leaky = HOLDS.to_owned() + "root_0\ttrue\tsettings 1200 bytes\n";
		assert_eq!(
			verdict_on(&parse(&leaky), Os::Windows),
			Err(vec!["root_0".to_owned()])
		);
		let fine = HOLDS.to_owned()
			+ "root_0\tfalse\tpermission denied\nroot_1\tfalse\tentity not found\n";
		assert_eq!(verdict_on(&parse(&fine), Os::Windows), Ok(()));
	}

	#[test]
	fn a_report_where_the_canary_was_read_fails_closed() {
		let leaked = HOLDS.replace("read_canary\tfalse", "read_canary\ttrue");
		assert_eq!(
			verdict_on(&parse(&leaked), Os::Windows),
			Err(vec!["read_canary".to_owned()])
		);
	}

	#[test]
	fn a_report_with_a_key_outside_the_allowlist_fails_closed() {
		let leaked = HOLDS.replace("PATH,TEMP", "PATH,GITHUB_TOKEN");
		assert_eq!(
			verdict_on(&parse(&leaked), Os::Windows),
			Err(vec!["env_keys".to_owned()])
		);
	}

	#[test]
	fn a_report_missing_a_check_fails_closed() {
		let short = "read_canary\tfalse\tdenied\n";
		let failed = verdict_on(&parse(short), Os::Windows).unwrap_err();
		assert!(failed.contains(&"write_inside".to_owned()), "{failed:?}");
		assert!(failed.contains(&"tcp_loopback".to_owned()), "{failed:?}");
	}

	#[test]
	fn the_probe_arguments_are_read_back_from_a_command_line() {
		let args = [
			"modlobby.exe",
			ARG,
			"C:/w/report.txt",
			"C:/canary.txt",
			"C:/outside.txt",
			"5123",
		]
		.map(OsString::from);
		assert_eq!(
			arg(args),
			Some(ProbeArgs {
				report: "C:/w/report.txt".into(),
				canary: "C:/canary.txt".into(),
				outside: "C:/outside.txt".into(),
				loopback_port: 5123,
				roots: Vec::new(),
			})
		);
		assert_eq!(arg(["modlobby.exe".into()]), None);
	}

	/// A guard whose process never ends on its own; `kill` ends it.
	struct Hang {
		killed: Arc<AtomicBool>,
		ended: Arc<(Mutex<bool>, Condvar)>,
	}

	impl Guard for Hang {
		fn kill(&self) -> bool {
			self.killed.store(true, Ordering::SeqCst);
			let (ended, wake) = &*self.ended;
			*ended.lock().unwrap() = true;
			wake.notify_all();
			true
		}

		fn wait(&self) -> Option<i32> {
			let (ended, wake) = &*self.ended;
			let mut done = ended.lock().unwrap();
			while !*done {
				done = wake.wait(done).unwrap();
			}
			None
		}
	}

	struct Hanging {
		killed: Arc<AtomicBool>,
	}

	impl Confinement for Hanging {
		fn spawn(
			&self,
			_command: &std::process::Command,
			_policy: &Policy,
		) -> Result<Confined, String> {
			Ok(Confined::new(
				1,
				Box::new(Hang {
					killed: self.killed.clone(),
					ended: Arc::new((Mutex::new(false), Condvar::new())),
				}),
			))
		}

		fn unavailable(&self) -> Option<String> {
			None
		}
	}

	struct NeverSpawns;

	impl Confinement for NeverSpawns {
		fn spawn(
			&self,
			_command: &std::process::Command,
			_policy: &Policy,
		) -> Result<Confined, String> {
			Err("spawned".into())
		}

		fn unavailable(&self) -> Option<String> {
			None
		}
	}

	/// A stand-in for modlobby's exe: the probe copies it, so it must exist.
	fn exe_in(dir: &Path) -> PathBuf {
		let exe = dir.join("modlobby.exe");
		std::fs::write(&exe, b"exe").unwrap();
		exe
	}

	fn policy_in(dir: &Path) -> Policy {
		Policy {
			exe_dirs: vec![dir.join("bin")],
			roots: Vec::new(),
			write: dir.join("mods"),
			network: Network::None,
			env: Vec::new(),
			grants_file: dir.join("grants.json"),
		}
	}

	#[tokio::test]
	async fn a_probe_that_never_reports_fails_closed() {
		let dir = tempfile::tempdir().unwrap();
		let killed = Arc::new(AtomicBool::new(false));
		let hanging: Arc<dyn Confinement> = Arc::new(Hanging {
			killed: killed.clone(),
		});
		let err = ensure_with(
			&hanging,
			&policy_in(dir.path()),
			&exe_in(dir.path()),
			&dir.path().join("probe.json"),
			"build",
			"0.0.0",
			Duration::from_millis(50),
		)
		.await
		.unwrap_err();
		assert!(err.contains("did not finish"), "{err}");
		assert!(killed.load(Ordering::SeqCst), "the hung probe was ended");
		assert!(!dir.path().join("probe.json").exists(), "nothing cached");
	}

	#[tokio::test]
	async fn a_cached_verdict_for_the_same_version_and_build_is_trusted() {
		let dir = tempfile::tempdir().unwrap();
		let cache = dir.path().join("probe.json");
		let policy = policy_in(dir.path());
		std::fs::write(
			&cache,
			format!(
				r#"{{"version":"0.1.44","os_build":"26200","policy":"{}","holds":true}}"#,
				fingerprint(&policy)
			),
		)
		.unwrap();
		let ok = ensure(
			&(Arc::new(NeverSpawns) as Arc<dyn Confinement>),
			&policy_in(dir.path()),
			Path::new("C:/modlobby.exe"),
			&cache,
			"26200",
			"0.1.44",
		)
		.await;
		assert_eq!(ok, Ok(()));
	}

	/// The verdict covers one boundary: other roots or another network mode
	/// are another boundary.
	#[tokio::test]
	async fn a_cached_verdict_for_another_policy_is_not_trusted() {
		let dir = tempfile::tempdir().unwrap();
		let cache = dir.path().join("probe.json");
		let mut policy = policy_in(dir.path());
		std::fs::write(
			&cache,
			format!(
				r#"{{"version":"0.1.44","os_build":"26200","policy":"{}","holds":true}}"#,
				fingerprint(&policy)
			),
		)
		.unwrap();
		policy.network = Network::Outbound;
		let err = ensure(
			&(Arc::new(NeverSpawns) as Arc<dyn Confinement>),
			&policy,
			&exe_in(dir.path()),
			&cache,
			"26200",
			"0.1.44",
		)
		.await
		.unwrap_err();
		assert!(err.contains("spawned"), "{err}");
	}

	#[tokio::test]
	async fn a_cached_verdict_for_another_version_is_not_trusted() {
		let dir = tempfile::tempdir().unwrap();
		let cache = dir.path().join("probe.json");
		std::fs::write(
			&cache,
			r#"{"version":"0.1.43","os_build":"26200","holds":true}"#,
		)
		.unwrap();
		let err = ensure(
			&(Arc::new(NeverSpawns) as Arc<dyn Confinement>),
			&policy_in(dir.path()),
			&exe_in(dir.path()),
			&cache,
			"26200",
			"0.1.44",
		)
		.await
		.unwrap_err();
		assert!(err.contains("spawned"), "{err}");
	}

	// ------------------------------------------------------------- Linux

	/// What a bwrap sandbox reports when it holds: the canary is not in its
	/// mount namespace, the root it is in is read-only, a child stays
	/// inside, and the compositor offers no clipboard.
	const HOLDS_ON_LINUX: &str = "read_canary\tfalse\tentity not found\nwrite_inside\ttrue\twritten\nwrite_outside\tfalse\tread-only filesystem or storage medium\nclipboard_read\tfalse\tno clipboard offered\nchild_process\ttrue\texit status: 0\ntcp_loopback\tfalse\tpermission denied\nenv_keys\ttrue\tPATH,HOME,WAYLAND_DISPLAY,XDG_RUNTIME_DIR,PULSE_SERVER,SPRING_WRITEDIR\n";

	#[test]
	fn on_linux_a_hidden_file_is_denied_by_absence() {
		assert_eq!(verdict_on(&parse(HOLDS_ON_LINUX), Os::Linux), Ok(()));
		let denied = HOLDS_ON_LINUX
			.replace("entity not found", "permission denied")
			.replace(
				"read-only filesystem or storage medium",
				"permission denied",
			);
		assert_eq!(verdict_on(&parse(&denied), Os::Linux), Ok(()));
		// Windows has no namespace to hide a file in: absence proves nothing.
		let on_windows = verdict_on(&parse(HOLDS_ON_LINUX), Os::Windows).unwrap_err();
		assert!(
			on_windows.contains(&"read_canary".to_owned()),
			"{on_windows:?}"
		);
		assert!(
			on_windows.contains(&"write_outside".to_owned()),
			"{on_windows:?}"
		);
	}

	/// A child of the engine stays inside the same sandbox on Linux, and a
	/// relaunch keeps the write dir and config through the environment.
	#[test]
	fn on_linux_a_child_process_is_allowed() {
		assert_eq!(verdict_on(&parse(HOLDS_ON_LINUX), Os::Linux), Ok(()));
		let on_windows = verdict_on(&parse(HOLDS_ON_LINUX), Os::Windows).unwrap_err();
		assert!(
			on_windows.contains(&"child_process".to_owned()),
			"{on_windows:?}"
		);
	}

	/// The clipboard is denied by the proxy hiding it, and the probe must
	/// have looked: a probe that could not check proves nothing.
	#[test]
	fn on_linux_the_clipboard_must_be_shown_hidden() {
		let unchecked =
			HOLDS_ON_LINUX.replace("no clipboard offered", "not checked on this platform");
		assert_eq!(
			verdict_on(&parse(&unchecked), Os::Linux),
			Err(vec!["clipboard_read".to_owned()])
		);
		let offered = HOLDS_ON_LINUX.replace(
			"clipboard_read\tfalse\tno clipboard offered",
			"clipboard_read\ttrue\twl_data_device_manager",
		);
		assert_eq!(
			verdict_on(&parse(&offered), Os::Linux),
			Err(vec!["clipboard_read".to_owned()])
		);
	}

	#[test]
	fn on_linux_a_key_outside_the_allowlist_fails_closed() {
		let leaked = HOLDS_ON_LINUX.replace("PATH,HOME", "PATH,GITHUB_TOKEN");
		assert_eq!(
			verdict_on(&parse(&leaked), Os::Linux),
			Err(vec!["env_keys".to_owned()])
		);
		// Linux names are case-sensitive: `Path` is not `PATH`.
		let cased = HOLDS_ON_LINUX.replace("PATH,HOME", "Path,HOME");
		assert_eq!(
			verdict_on(&parse(&cased), Os::Linux),
			Err(vec!["env_keys".to_owned()])
		);
	}
}
