//! Putting the game's window back in front, and saying where it is.
//!
//! Getting our own window out of the way is not the same as handing the
//! keyboard back: it stays with us, or goes to the next window in the z-order,
//! and that is not reliably the game. This names the window we mean by the
//! process id we spawned.
//!
//! It works because it is called from the foreground process — Windows only
//! honours `SetForegroundWindow` from one, which is exactly what we are at the
//! moment the overlay is being dismissed.

use super::seams::{ForegroundControl, ScreenId, WindowId};

pub struct Windows;

#[cfg(windows)]
impl ForegroundControl for Windows {
	fn focus(&self, pid: u32) {
		use windows_sys::Win32::UI::WindowsAndMessaging::{
			SW_RESTORE, SetForegroundWindow, ShowWindow,
		};

		let Some(window) = crate::win::visible_windows_of(pid).into_iter().next() else {
			// A game still loading has no window yet. Not an error, and not
			// worth interrupting anyone over.
			tracing::debug!(pid, "no window to bring forward yet");
			return;
		};

		// SAFETY: a live handle from the enumeration.
		let raised = unsafe { SetForegroundWindow(window) };
		if raised == 0 {
			// Windows refuses this from a process it does not consider
			// foreground. Restoring is the weaker request it usually grants.
			tracing::warn!(pid, "could not raise the game; restoring instead");
			// SAFETY: same handle.
			unsafe { ShowWindow(window, SW_RESTORE) };
		}
	}

	fn screen_of(&self, pid: u32) -> Option<ScreenId> {
		let window = crate::win::visible_windows_of(pid).into_iter().next()?;
		crate::win::monitor_of(window).map(ScreenId)
	}

	fn window_of(&self, pid: u32) -> Option<WindowId> {
		let window = crate::win::visible_windows_of(pid).into_iter().next()?;
		Some(WindowId(window as isize))
	}

	fn in_front(&self, pid: u32) -> bool {
		crate::win::owns_foreground(pid)
	}
}

#[cfg(not(windows))]
impl ForegroundControl for Windows {
	fn focus(&self, pid: u32) {
		tracing::debug!(
			pid,
			"raising another process's window is not available here"
		);
	}

	fn screen_of(&self, _pid: u32) -> Option<ScreenId> {
		None
	}

	fn window_of(&self, _pid: u32) -> Option<WindowId> {
		None
	}

	fn in_front(&self, _pid: u32) -> bool {
		false
	}
}
