//! The boundary a modded game runs inside.
//!
//! Mod code runs in the engine on every player's machine, and the engine's
//! Lua can read any file the process can, so the only boundary that holds is
//! the operating system's. [`Confinement`] is that boundary behind one trait:
//! an AppContainer on Windows, a refusal everywhere else until each platform
//! has its own.

use std::sync::Arc;

pub mod grants;
pub mod policy;
pub mod probe;
pub mod refuse;
#[cfg(windows)]
pub mod windows;

pub use policy::{CONTENT_SUBDIRS, KEPT_ENV, Network, Policy, Root, scrubbed_env};
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
	#[cfg(windows)]
	{
		match windows::AppContainer::new() {
			Ok(container) => return Arc::new(container),
			Err(reason) => return Arc::new(Refuse { reason }),
		}
	}
	#[cfg(not(windows))]
	Arc::new(Refuse {
		reason:
			"modded games run only inside a sandbox, and this build has none for this platform yet"
				.into(),
	})
}
