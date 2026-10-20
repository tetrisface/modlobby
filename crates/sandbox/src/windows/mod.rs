//! A Windows AppContainer around the engine.
//!
//! The container is a token the kernel checks when the process opens a file,
//! a key or a socket: by default it can read the system's directories and
//! nothing of the user's, so the policy's grants are everything the engine
//! sees. A job object takes the clipboard and the desktop away, the
//! child-process policy takes `Spring.Start` away, and the capability list
//! decides whether it has the network at all.

mod acl;
mod env;
mod job;
mod process;
mod sid;
mod winsta;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{PSID, SID_AND_ATTRIBUTES};
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};

use crate::grants::Grants;
use crate::{Confined, Confinement, Guard, Network, Policy};

/// The profile's name. Its SID is a hash of this, the same on every machine.
pub const PROFILE: &str = "modlobby.engine";

/// `SE_GROUP_ENABLED`, on a capability in the token.
const SE_GROUP_ENABLED: u32 = 0x4;

pub struct AppContainer {
	sid: sid::Sid,
}

impl AppContainer {
	pub fn new() -> Result<Self, String> {
		Ok(Self {
			sid: sid::Sid::profile(PROFILE)?,
		})
	}

	/// Everything the policy grants, and what was granted last time and is
	/// not in it any more, reconciled through the record in
	/// `policy.grants_file`.
	fn grants(&self, policy: &Policy) -> Result<(), String> {
		let sid = self.sid.as_psid();
		let plan = plan(policy);
		let wanted: Vec<PathBuf> = plan.iter().map(|(path, _, _)| path.clone()).collect();
		Grants::load(&policy.grants_file).reconcile(
			&wanted,
			|path| {
				let (_, access, inherit) = plan
					.iter()
					.find(|(planned, _, _)| planned == path)
					.expect("a wanted path is a planned one");
				acl::grant(path, sid, *access, *inherit)
			},
			|path| {
				// A directory that is gone has nothing left to revoke.
				if !path.exists() {
					return Ok(());
				}
				acl::revoke(path, sid)
			},
		)
	}
}

/// Each path the policy grants, with its access and whether it inherits:
/// the engine's directory read and execute; each root listable and its
/// content directories readable, with whatever they link to; the write
/// directory everything.
fn plan(policy: &Policy) -> Vec<(PathBuf, u32, bool)> {
	let mut plan: Vec<(PathBuf, u32, bool)> = policy
		.exe_dirs
		.iter()
		.map(|dir| (dir.clone(), acl::READ | acl::EXECUTE, true))
		.collect();
	for root in &policy.roots {
		// Inherited: a root holds only links, and a link is opened through
		// an entry of its own, which has to carry the grant. Propagation
		// stops at the link; the target gets its own grant below.
		plan.push((root.dir.clone(), acl::READ, true));
		for dir in &root.subdirs {
			if !dir.is_dir() {
				continue;
			}
			plan.extend(
				linked_dirs(dir)
					.into_iter()
					.map(|target| (target, acl::READ, true)),
			);
			plan.push((dir.clone(), acl::READ, true));
		}
	}
	plan.push((policy.write.clone(), acl::ALL, true));
	plan
}

/// The directories that links in `dir` lead to, and links one level into
/// an archive directory (`*.sdd`) under it: the BAR launcher's dev mode
/// links a game to a checkout, or makes one of links into a checkout, a
/// subdirectory each, and the engine walks into both. Nothing deeper is
/// followed, and no target that is a drive, the user's profile or above it,
/// or a `.git` directory is taken.
fn linked_dirs(dir: &Path) -> Vec<PathBuf> {
	let mut found: Vec<PathBuf> = Vec::new();
	let mut take = |path: &Path| {
		if let Some(target) = link_target(path).filter(|target| content_target(target))
			&& !found.contains(&target)
		{
			found.push(target);
		}
	};
	let Ok(entries) = std::fs::read_dir(dir) else {
		return found;
	};
	for entry in entries.filter_map(Result::ok) {
		let path = entry.path();
		let Ok(kind) = entry.file_type() else {
			continue;
		};
		if kind.is_symlink() {
			take(&path);
			continue;
		}
		let archive = kind.is_dir()
			&& path
				.extension()
				.is_some_and(|ext| ext.eq_ignore_ascii_case("sdd"));
		if !archive {
			continue;
		}
		let Ok(inside) = std::fs::read_dir(&path) else {
			continue;
		};
		for entry in inside.filter_map(Result::ok) {
			if entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
				take(&entry.path());
			}
		}
	}
	found
}

/// Whether a link target may be granted: a directory that is not a drive
/// root, not the user's profile or any directory above it, and not `.git`.
fn content_target(target: &Path) -> bool {
	if !target.is_dir() || target.parent().is_none() {
		return false;
	}
	if target
		.file_name()
		.is_some_and(|name| name.eq_ignore_ascii_case(".git"))
	{
		return false;
	}
	let profile = std::env::var_os("USERPROFILE")
		.map(PathBuf::from)
		.and_then(|profile| std::fs::canonicalize(profile).ok())
		.map(plain);
	match profile {
		Some(profile) => !profile.starts_with(target),
		None => true,
	}
}

/// Where a link leads, canonical; `None` for anything that is not a link to
/// somewhere that exists.
fn link_target(path: &Path) -> Option<PathBuf> {
	let target = std::fs::read_link(path).ok()?;
	let target = if target.is_relative() {
		path.parent()?.join(target)
	} else {
		target
	};
	std::fs::canonicalize(&target).ok().map(plain)
}

/// A canonical path without the `\?\` prefix, which the ACL calls take
/// but the grants record need not carry.
fn plain(path: PathBuf) -> PathBuf {
	let text = path.to_string_lossy();
	match text.strip_prefix(r"\\?\") {
		Some(short) if !short.starts_with("UNC") => PathBuf::from(short),
		_ => path,
	}
}

/// The policy's environment with what the command itself set on top: the
/// launch names the read directories through `SPRING_DATADIR`.
fn environment(command: &std::process::Command, policy: &Policy) -> Vec<(OsString, OsString)> {
	let mut env = policy.env.clone();
	for (key, value) in command.get_envs() {
		let Some(value) = value else {
			continue;
		};
		env.retain(|(held, _)| !held.eq_ignore_ascii_case(key));
		env.push((key.to_owned(), value.to_owned()));
	}
	env
}

impl Confinement for AppContainer {
	fn spawn(&self, command: &std::process::Command, policy: &Policy) -> Result<Confined, String> {
		let env = environment(command, policy);
		process::check_env(&env)?;
		self.grants(policy)?;
		let sid = self.sid.as_psid();

		let mut internet = match policy.network {
			Network::Outbound => sid::internet_client()?,
			Network::None => Vec::new(),
		};
		let mut capabilities = Vec::new();
		if !internet.is_empty() {
			capabilities.push(SID_AND_ATTRIBUTES {
				Sid: PSID(internet.as_mut_ptr() as _),
				Attributes: SE_GROUP_ENABLED,
			});
		}
		let args: Vec<&OsStr> = command.get_args().collect();
		let mut line = env::command_line(command.get_program(), &args);
		let block = env::environment_block(&env);

		winsta::set(sid, true)?;
		let started = match process::create(
			sid,
			&mut capabilities,
			&mut line,
			&block,
			command.get_current_dir(),
		) {
			Ok(started) => started,
			Err(err) => {
				let _ = winsta::set(sid, false);
				return Err(err);
			}
		};
		let job = match job::Job::new().and_then(|job| job.assign(started.process).map(|()| job)) {
			Ok(job) => job,
			Err(err) => {
				started.abandon();
				let _ = winsta::set(sid, false);
				return Err(err);
			}
		};
		started.resume();
		Ok(Confined::new(
			started.pid,
			Box::new(Guarded {
				process: Process(started.process),
				job,
				sid: self.sid.clone(),
			}),
		))
	}

	fn unavailable(&self) -> Option<String> {
		None
	}
}

struct Process(HANDLE);

// A process handle is a kernel object; the pointer is an opaque id.
unsafe impl Send for Process {}
unsafe impl Sync for Process {}

impl Drop for Process {
	fn drop(&mut self) {
		unsafe {
			let _ = CloseHandle(self.0);
		}
	}
}

/// Holds the engine: its process handle, its job, and the window-station
/// entries that go when it does.
struct Guarded {
	process: Process,
	job: job::Job,
	sid: sid::Sid,
}

impl Guard for Guarded {
	fn kill(&self) -> bool {
		self.job.terminate()
	}

	fn wait(&self) -> Option<i32> {
		unsafe {
			WaitForSingleObject(self.process.0, INFINITE);
			let mut code = 0u32;
			GetExitCodeProcess(self.process.0, &mut code)
				.ok()
				.map(|()| code as i32)
		}
	}
}

impl Drop for Guarded {
	fn drop(&mut self) {
		let _ = winsta::set(self.sid.as_psid(), false);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Root;

	/// The BAR launcher's dev mode links `games/BYAR-Chobby.sdd` to a checkout
	/// elsewhere; the engine walks into it, so the target is content too.
	#[test]
	fn a_linked_archive_directory_is_granted_at_its_target() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("data");
		let games = root.join("games");
		std::fs::create_dir_all(&games).unwrap();
		let elsewhere = dir.path().join("checkout").join("BYAR-Chobby");
		std::fs::create_dir_all(&elsewhere).unwrap();
		let link = games.join("BYAR-Chobby.sdd");
		let made = std::process::Command::new("cmd")
			.args(["/c", "mklink", "/J"])
			.arg(&link)
			.arg(&elsewhere)
			.output()
			.unwrap();
		assert!(
			made.status.success(),
			"{}",
			String::from_utf8_lossy(&made.stderr)
		);
		let policy = Policy {
			exe_dirs: Vec::new(),
			roots: vec![Root {
				dir: root.clone(),
				subdirs: vec![games.clone()],
			}],
			write: dir.path().join("mods"),
			network: Network::None,
			env: Vec::new(),
			grants_file: dir.path().join("grants.json"),
		};
		let planned: Vec<PathBuf> = plan(&policy).into_iter().map(|(path, _, _)| path).collect();
		let target = std::fs::canonicalize(&elsewhere).unwrap();
		assert!(
			planned
				.iter()
				.any(|path| std::fs::canonicalize(path).ok() == Some(target.clone())),
			"{planned:?}"
		);
	}

	/// The launcher's dev-mode game is a directory of junctions, one per
	/// subdirectory, into a checkout: a link deep inside an archive directory
	/// is content the engine walks into just the same.
	#[test]
	fn a_link_deep_inside_an_archive_directory_is_granted_too() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("data");
		let archive = root.join("games").join("Game.sdd");
		std::fs::create_dir_all(&archive).unwrap();
		let checkout = dir.path().join("checkout").join("anims");
		std::fs::create_dir_all(&checkout).unwrap();
		let made = std::process::Command::new("cmd")
			.args(["/c", "mklink", "/J"])
			.arg(archive.join("anims"))
			.arg(&checkout)
			.output()
			.unwrap();
		assert!(
			made.status.success(),
			"{}",
			String::from_utf8_lossy(&made.stderr)
		);
		let policy = Policy {
			exe_dirs: Vec::new(),
			roots: vec![Root {
				dir: root.clone(),
				subdirs: vec![root.join("games")],
			}],
			write: dir.path().join("mods"),
			network: Network::None,
			env: Vec::new(),
			grants_file: dir.path().join("grants.json"),
		};
		let planned: Vec<PathBuf> = plan(&policy).into_iter().map(|(path, _, _)| path).collect();
		let target = std::fs::canonicalize(&checkout).unwrap();
		assert!(
			planned
				.iter()
				.any(|path| std::fs::canonicalize(path).ok() == Some(target.clone())),
			"{planned:?}"
		);
	}

	/// Links are followed in a content directory and one level into an
	/// archive directory, never further, and never to anything that holds
	/// the user's profile, a data root or a drive.
	#[test]
	fn link_following_is_bounded() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("data");
		let games = root.join("games");
		let archive = games.join("Game.sdd");
		std::fs::create_dir_all(&archive).unwrap();
		let checkout = dir.path().join("checkout");
		std::fs::create_dir_all(checkout.join("anims")).unwrap();
		std::fs::create_dir_all(checkout.join("deeper").join("more")).unwrap();
		let junction = |at: &Path, to: &Path| {
			let made = std::process::Command::new("cmd")
				.args(["/c", "mklink", "/J"])
				.arg(at)
				.arg(to)
				.output()
				.unwrap();
			assert!(
				made.status.success(),
				"{}",
				String::from_utf8_lossy(&made.stderr)
			);
		};
		// one level inside an archive: followed
		junction(&archive.join("anims"), &checkout.join("anims"));
		// a link inside a link target: not followed
		junction(
			&checkout.join("anims").join("more"),
			&checkout.join("deeper").join("more"),
		);
		// a link to a drive root: refused
		junction(&games.join("Drive.sdd"), Path::new("C:\\"));
		// a link to the profile: refused
		let profile = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap();
		junction(&games.join("Home.sdd"), &profile);
		let policy = Policy {
			exe_dirs: Vec::new(),
			roots: vec![Root {
				dir: root.clone(),
				subdirs: vec![games.clone()],
			}],
			write: dir.path().join("mods"),
			network: Network::None,
			env: Vec::new(),
			grants_file: dir.path().join("grants.json"),
		};
		let planned: Vec<PathBuf> = plan(&policy)
			.into_iter()
			.filter_map(|(path, _, _)| std::fs::canonicalize(path).ok())
			.collect();
		let has = |p: &Path| planned.contains(&std::fs::canonicalize(p).unwrap());
		assert!(has(&checkout.join("anims")), "{planned:?}");
		assert!(
			!has(&checkout.join("deeper").join("more")),
			"a link in a target: {planned:?}"
		);
		assert!(!has(Path::new("C:\\")), "a drive root: {planned:?}");
		assert!(!has(&profile), "the profile: {planned:?}");
	}
}
