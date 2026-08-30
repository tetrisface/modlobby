//! Finding the windows another process owns.
//!
//! Two features need this and they need the same half of it: flashing the
//! engine's taskbar entry when a game starts, and putting the engine's window
//! back in front when the overlay gets out of the way. The enumeration is the
//! part that is easy to get subtly wrong — the callback contract, the pid
//! comparison, the visibility filter — so it lives once.

#![cfg(windows)]

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{HWND, LPARAM, TRUE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
};
use windows_sys::core::BOOL;

struct Hunt {
    wanted: u32,
    found: Vec<HWND>,
}

unsafe extern "system" fn visit(window: HWND, state: LPARAM) -> BOOL {
    // SAFETY: `state` is the `&mut Hunt` handed to `EnumWindows` below, which
    // outlives the enumeration — it is synchronous.
    let hunt = unsafe { &mut *(state as *mut Hunt) };

    let mut owner = 0_u32;
    // SAFETY: `window` comes from the enumeration and `owner` is ours.
    unsafe { GetWindowThreadProcessId(window, &mut owner) };
    // SAFETY: a window handle from the enumeration.
    if owner == hunt.wanted && unsafe { IsWindowVisible(window) } != 0 {
        hunt.found.push(window);
    }
    // Keep going: a process may own several, and the first is not always the
    // one anybody means.
    TRUE
}

/// Every visible top-level window belonging to `pid`, in z-order.
///
/// Empty is an ordinary answer, not a failure: a game that is still loading
/// has no window yet, and one that has exited has none any more.
pub fn visible_windows_of(pid: u32) -> Vec<HWND> {
    let mut hunt = Hunt {
        wanted: pid,
        found: Vec::new(),
    };
    // SAFETY: `visit` matches the expected signature and `hunt` outlives this
    // synchronous call.
    unsafe {
        EnumWindows(Some(visit), &raw mut hunt as *mut c_void as LPARAM);
    }
    hunt.found
}
