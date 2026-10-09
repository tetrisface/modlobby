//! Talking to Windows itself: the windows another process owns, and a box
//! for what has to be said before there is a window of our own.
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
	EnumWindows, GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible,
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

/// The monitor `window` is on, as a handle two windows on the same one share.
///
/// `None` for a window on no monitor: one minimized, which sits at
/// (-32000, -32000), or one dragged off every screen.
pub fn monitor_of(window: HWND) -> Option<isize> {
	use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromWindow};

	// SAFETY: any handle is accepted; a dead one answers null like an
	// off-screen one.
	let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONULL) };
	(!monitor.is_null()).then_some(monitor as isize)
}

/// A monitor's rectangle, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
	pub left: i32,
	pub top: i32,
	pub width: u32,
	pub height: u32,
}

/// The rectangle of a monitor named by [`monitor_of`]. `None` for a handle
/// the OS no longer knows, which is a monitor unplugged since.
pub fn monitor_rect(monitor: isize) -> Option<Rect> {
	use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};

	let mut info = MONITORINFO {
		cbSize: size_of::<MONITORINFO>() as u32,
		..Default::default()
	};
	// SAFETY: `info` is sized as the call requires and outlives it; a dead
	// handle answers zero rather than writing anything.
	if unsafe { GetMonitorInfoW(monitor as _, &mut info) } == 0 {
		return None;
	}
	let area = info.rcMonitor;
	Some(Rect {
		left: area.left,
		top: area.top,
		width: (area.right - area.left).max(0) as u32,
		height: (area.bottom - area.top).max(0) as u32,
	})
}

/// Whether the window in front belongs to `pid`.
pub fn owns_foreground(pid: u32) -> bool {
	let mut owner = 0_u32;
	// SAFETY: both calls accept a null window, which leaves `owner` at 0 and
	// so matches no process we spawned.
	unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut owner) };
	owner == pid
}

/// A modal error box with nothing behind it.
pub fn alert(title: &str, message: &str) {
	use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

	let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
	let (title, message) = (wide(title), wide(message));
	// SAFETY: both strings are NUL-terminated and outlive the call, which
	// blocks until the box is dismissed; a null owner window is accepted.
	unsafe {
		MessageBoxW(
			std::ptr::null_mut(),
			message.as_ptr(),
			title.as_ptr(),
			MB_OK | MB_ICONERROR,
		)
	};
}
