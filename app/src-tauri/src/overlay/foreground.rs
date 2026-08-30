//! Putting the game's window back in front.
//!
//! Hiding our own window is not the same as handing the keyboard back: the
//! next window in the z-order gets it, and that is not reliably the game. This
//! names the window we mean by the process id we spawned.
//!
//! It works because it is called from the foreground process — Windows only
//! honours `SetForegroundWindow` from one, which is exactly what we are at the
//! moment the overlay is being dismissed.

use super::seams::ForegroundControl;

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
}

#[cfg(not(windows))]
impl ForegroundControl for Windows {
    fn focus(&self, pid: u32) {
        tracing::debug!(
            pid,
            "raising another process's window is not available here"
        );
    }
}
