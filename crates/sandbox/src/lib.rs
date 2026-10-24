//! The boundary a modded game runs inside.
//!
//! Mod code runs in the engine on every player's machine, and the engine's
//! Lua can read any file the process can, so the only boundary that holds is
//! the operating system's. [`Confinement`] is that boundary behind one trait:
//! an AppContainer on Windows, bubblewrap with Landlock and seccomp on
//! Linux under Wayland, a refusal everywhere else.

use std::sync::Arc;

pub mod bwrap;
pub mod grants;
pub mod inside;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod policy;
pub mod probe;
pub mod refuse;
pub mod wayland;
#[cfg(windows)]
pub mod windows;

pub use policy::{
	CONTENT_SUBDIRS, KEPT_ENV, Network, Os, Policy, Root, kept_env, same_key, scrubbed_env,
	scrubbed_env_on, this_os,
};
pub use refuse::Refuse;

/// Starts a process inside a boundary.
pub trait Confinement: Send + Sync {
	/// Starts `command` inside the boundary `policy` describes: the engine's
	/// pid, and the guard that ends it.
	fn spawn(&self, command: &std::process::Command, policy: &Policy) -> Result<Confined, String>;

	/// Why this confinement cannot run here, for the person asking; `None`
	/// when it can.
	fn unavailable(&self) -> Option<String>;
}

/// What a confinement holds a running process by.
pub trait Guard: Send + Sync {
	/// Ends the process, and everything it started. Whether it was running.
	fn kill(&self) -> bool;
	/// Blocks until the process ends; its exit code where there is one.
	fn wait(&self) -> Option<i32>;
}

/// A process running inside a boundary.
pub struct Confined {
	pub pid: u32,
	guard: Box<dyn Guard>,
}

impl Confined {
	pub fn new(pid: u32, guard: Box<dyn Guard>) -> Self {
		Self { pid, guard }
	}

	pub fn kill(&self) -> bool {
		self.guard.kill()
	}

	/// The exit code once the process has ended, off the async runtime's
	/// threads: the wait blocks.
	pub async fn wait(self: &Arc<Self>) -> Option<i32> {
		let held = Arc::clone(self);
		tokio::task::spawn_blocking(move || held.guard.wait())
			.await
			.unwrap_or(None)
	}
}

/// The confinement this platform has.
pub fn platform() -> Arc<dyn Confinement> {
	here()
}

#[cfg(windows)]
fn here() -> Arc<dyn Confinement> {
	match windows::AppContainer::new() {
		Ok(container) => Arc::new(container),
		Err(reason) => Arc::new(Refuse { reason }),
	}
}

#[cfg(target_os = "linux")]
fn here() -> Arc<dyn Confinement> {
	// The helper ships beside modlobby; the sandbox runs a copy of it.
	let helper = std::env::current_exe()
		.ok()
		.and_then(|exe| exe.parent().map(|dir| dir.join(HELPER)))
		.unwrap_or_else(|| HELPER.into());
	Arc::new(linux::Bubblewrap::new(helper))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn here() -> Arc<dyn Confinement> {
	Arc::new(Refuse {
		reason:
			"modded games run only inside a sandbox, and this build has none for this platform yet"
				.into(),
	})
}

/// The helper's file name beside modlobby's: the probe, the in-sandbox
/// exec and the Wayland proxy, with none of the app's libraries.
pub const HELPER: &str = "modlobby-sandbox";

/// What runs as the probe: modlobby itself on Windows, the helper beside it
/// on Linux, where the app's own libraries are not inside the sandbox.
pub fn probe_exe() -> std::path::PathBuf {
	let exe = std::env::current_exe().unwrap_or_default();
	if cfg!(target_os = "linux") {
		return exe.with_file_name(HELPER);
	}
	exe
}
