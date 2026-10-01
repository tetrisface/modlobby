//! Engines that outlived the lobby that started them. A lobby can end under a
//! running game -- a dev rebuild, a crash, a quit -- and the engine plays on,
//! so the lobby that comes back takes it over rather than drawing the room as
//! though nothing were running.

use std::path::{Path, PathBuf};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// A running process, as far as taking one over cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
	pub pid: u32,
	pub parent: Option<u32>,
	/// Seconds since the epoch.
	pub started: u64,
	pub exe: Option<PathBuf>,
}

/// The engine among `processes` nobody owns any more: started from an engine
/// under one of `roots` (each data directory's `engine/`), whose parent has
/// gone, or has handed its number on to a newer process. An engine whose
/// lobby or launcher is still running is theirs and never taken.
// ponytail: an orphan another lobby started from the same install is taken
// too; its command line (our `--menu`, our script names) tells them apart if
// that ever happens.
pub fn orphaned_engine(processes: &[Process], roots: &[PathBuf]) -> Option<u32> {
	let owned = |engine: &Process| {
		engine
			.parent
			.and_then(|parent| processes.iter().find(|p| p.pid == parent))
			.is_some_and(|parent| parent.started <= engine.started)
	};
	processes
		.iter()
		.filter(|p| p.exe.as_deref().is_some_and(|exe| is_engine(exe, roots)))
		.find(|engine| !owned(engine))
		.map(|engine| engine.pid)
}

fn is_engine(exe: &Path, roots: &[PathBuf]) -> bool {
	exe.file_name()
		.is_some_and(|name| name == recoil::ENGINE_BINARY)
		&& roots.iter().any(|root| exe.starts_with(root))
}

/// Every process running now.
pub fn processes() -> Vec<Process> {
	let mut system = System::new();
	system.refresh_processes_specifics(
		ProcessesToUpdate::All,
		true,
		ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
	);
	system
		.processes()
		.values()
		.map(|p| Process {
			pid: p.pid().as_u32(),
			parent: p.parent().map(Pid::as_u32),
			started: p.start_time(),
			exe: p.exe().map(Path::to_path_buf),
		})
		.collect()
}

/// Whether process `pid` is running.
pub fn alive(pid: u32) -> bool {
	one(pid).process(Pid::from_u32(pid)).is_some()
}

/// Ends process `pid`; whether there was one to end.
pub fn kill(pid: u32) -> bool {
	one(pid)
		.process(Pid::from_u32(pid))
		.is_some_and(sysinfo::Process::kill)
}

fn one(pid: u32) -> System {
	let mut system = System::new();
	system.refresh_processes_specifics(
		ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
		true,
		ProcessRefreshKind::nothing(),
	);
	system
}

#[cfg(test)]
mod tests {
	use super::*;

	fn process(pid: u32, parent: Option<u32>, started: u64, exe: &str) -> Process {
		Process {
			pid,
			parent,
			started,
			exe: Some(PathBuf::from(exe)),
		}
	}

	fn engine_at(dir: &str) -> String {
		format!("{dir}/engine/recoil_2026.07.04/{}", recoil::ENGINE_BINARY)
	}

	#[test]
	fn only_an_engine_of_ours_with_nobody_left_to_own_it_is_taken() {
		let roots = [PathBuf::from("/data/engine")];
		let ours = engine_at("/data");
		let lobby = "/apps/lobby";
		// A lobby that is still running keeps its engine.
		let owned = [
			process(10, None, 100, lobby),
			process(11, Some(10), 105, &ours),
		];
		assert_eq!(orphaned_engine(&owned, &roots), None);
		// Its lobby gone, the engine is ours to take.
		assert_eq!(orphaned_engine(&owned[1..], &roots), Some(11));
		// So is one whose parent's number now belongs to something newer.
		let reused = [
			process(10, None, 200, lobby),
			process(11, Some(10), 105, &ours),
		];
		assert_eq!(orphaned_engine(&reused, &roots), Some(11));
		// An engine from some other install, or anything else, is not.
		let elsewhere = [process(11, Some(10), 105, &engine_at("/other"))];
		assert_eq!(orphaned_engine(&elsewhere, &roots), None);
		let not_engine = [process(
			11,
			Some(10),
			105,
			"/data/engine/recoil/pr-downloader",
		)];
		assert_eq!(orphaned_engine(&not_engine, &roots), None);
	}

	#[test]
	fn a_process_is_alive_until_it_is_killed() {
		let mut waits = if cfg!(windows) {
			let mut ping = std::process::Command::new("ping");
			ping.args(["-n", "30", "127.0.0.1"]);
			ping
		} else {
			let mut sleep = std::process::Command::new("sleep");
			sleep.arg("30");
			sleep
		};
		let mut child = waits.stdout(std::process::Stdio::null()).spawn().unwrap();
		let pid = child.id();
		assert!(alive(pid));
		assert!(kill(pid));
		child.wait().unwrap();
		assert!(!alive(pid));
	}
}
