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

/// Puts `window` directly under `above` in the z-order, whichever process owns
/// that one. Nothing else about either changes, and who has the keyboard least
/// of all: that stays with whoever has it. Whether it took.
pub fn place_behind(window: HWND, above: HWND) -> bool {
	use windows_sys::Win32::UI::WindowsAndMessaging::{
		SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
	};

	// SAFETY: any handle is accepted; a dead one answers zero.
	unsafe {
		SetWindowPos(
			window,
			above,
			0,
			0,
			0,
			0,
			SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
		) != 0
	}
}

/// Tells the compositor to draw `window`, a window of this process, or not.
/// Everything else goes on as usual, and unlike the layered style this is not
/// something a change of the window's styles can undo. Whether it took: a
/// session without a compositor does not.
pub fn cloak(window: HWND, cloaked: bool) -> bool {
	use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};

	let cloaked = BOOL::from(cloaked);
	// SAFETY: `cloaked` is the size given and outlives the call; a dead handle
	// answers an error.
	unsafe {
		DwmSetWindowAttribute(
			window,
			DWMWA_CLOAK as u32,
			(&raw const cloaked).cast(),
			size_of::<BOOL>() as u32,
		) >= 0
	}
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

#[cfg(test)]
mod tests {
	use std::os::windows::io::AsRawHandle;
	use std::ptr::{null, null_mut};

	use windows_sys::Win32::Foundation::{GENERIC_ALL, WAIT_OBJECT_0};
	use windows_sys::Win32::Graphics::Dwm::{
		DWM_CLOAKED_APP, DWMWA_CLOAKED, DwmGetWindowAttribute,
	};
	use windows_sys::Win32::System::StationsAndDesktops::{CreateDesktopW, SetThreadDesktop};
	use windows_sys::Win32::System::Threading::WaitForSingleObject;
	use windows_sys::Win32::UI::Input::Ime::{ImmDisableIME, ImmGetDefaultIMEWnd};
	use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetActiveWindow, SetActiveWindow};
	use windows_sys::Win32::UI::WindowsAndMessaging::{
		CreateWindowExW, GW_HWNDNEXT, GWL_EXSTYLE, GetWindow, GetWindowLongPtrW, IsZoomed,
		SW_MAXIMIZE, SW_RESTORE, SW_SHOWNORMAL, SetWindowLongPtrW, ShowWindow, WS_OVERLAPPEDWINDOW,
	};

	use super::*;

	fn wide(text: &str) -> Vec<u16> {
		text.encode_utf16().chain(Some(0)).collect()
	}

	/// How long a test on its own desktop may take, its thread's exit included.
	/// They take milliseconds.
	const PRIVATE_DESKTOP_LIMIT_MS: u32 = 30_000;

	/// Runs `test` on a thread of its own, on a Windows desktop of its own:
	/// windows made there can be shown, maximized and activated with nothing
	/// happening on the screen anyone is using.
	///
	/// The thread has no input method. A first window otherwise brings the
	/// system's text services along, whose teardown can hang the thread's exit
	/// with a core spinning, and with it `cargo test` and the pre-push hook.
	/// The wait is bounded too, so whatever else hangs fails the test instead.
	fn on_a_private_desktop(name: &str, test: impl FnOnce() + Send + 'static) {
		let name = wide(&format!("modlobby-test-{}-{name}", std::process::id()));
		let thread = std::thread::spawn(move || {
			// SAFETY: 0 names this thread, which has no window yet: the call
			// only counts before the first. The name is NUL-terminated and
			// outlives the call; the nulls ask for the defaults. A thread that
			// owns no window yet may change desktop. The desktop goes with the
			// process.
			unsafe {
				assert_ne!(ImmDisableIME(0), 0, "no input method on this thread");
				let desktop = CreateDesktopW(name.as_ptr(), null(), null(), 0, GENERIC_ALL, null());
				assert!(!desktop.is_null(), "a desktop of our own");
				assert_ne!(SetThreadDesktop(desktop), 0, "and this thread on it");
			}
			test();
		});
		// SAFETY: the handle is the thread's own and lives as long as `thread`.
		let ended =
			unsafe { WaitForSingleObject(thread.as_raw_handle(), PRIVATE_DESKTOP_LIMIT_MS) };
		assert_eq!(
			ended, WAIT_OBJECT_0,
			"the test on its own desktop, and its thread's exit, within {PRIVATE_DESKTOP_LIMIT_MS} ms"
		);
		thread.join().expect("the test on its own desktop");
	}

	fn a_window() -> HWND {
		let (class, title) = (wide("STATIC"), wide("modlobby test"));
		// SAFETY: both strings are NUL-terminated and outlive the call, and
		// `STATIC` is a class the system has registered.
		let window = unsafe {
			CreateWindowExW(
				0,
				class.as_ptr(),
				title.as_ptr(),
				WS_OVERLAPPEDWINDOW,
				0,
				0,
				400,
				300,
				null_mut(),
				null_mut(),
				null_mut(),
				null(),
			)
		};
		assert!(!window.is_null(), "a window to test with");
		// SAFETY: a live window of this thread.
		assert!(
			unsafe { ImmGetDefaultIMEWnd(window) }.is_null(),
			"no input method came along, see `on_a_private_desktop`"
		);
		window
	}

	/// The next window down that can be seen. Hidden windows, such as the
	/// helpers an input method brings along outside these tests, can sit
	/// between a window and anything it is placed under.
	fn next_seen_below(window: HWND) -> HWND {
		let mut below = window;
		loop {
			// SAFETY: a live window from this thread, or one the system lists.
			below = unsafe { GetWindow(below, GW_HWNDNEXT) };
			if below.is_null() || unsafe { IsWindowVisible(below) } != 0 {
				return below;
			}
		}
	}

	fn cloaked(window: HWND) -> bool {
		let mut how = 0_u32;
		// SAFETY: `how` is the size asked for and outlives the call.
		let answered = unsafe {
			DwmGetWindowAttribute(
				window,
				DWMWA_CLOAKED as u32,
				(&raw mut how).cast(),
				size_of::<u32>() as u32,
			)
		};
		assert!(answered >= 0, "the compositor answers: {answered:#x}");
		how & DWM_CLOAKED_APP != 0
	}

	#[test]
	fn a_window_placed_behind_another_is_directly_under_it_and_stays_active() {
		on_a_private_desktop("behind", || {
			let bottom = a_window();
			let middle = a_window();
			let top = a_window();
			// SAFETY: live windows of this thread.
			unsafe {
				ShowWindow(bottom, SW_SHOWNORMAL);
				ShowWindow(middle, SW_SHOWNORMAL);
				ShowWindow(top, SW_SHOWNORMAL);
				SetActiveWindow(top);
			}

			assert!(place_behind(top, bottom));

			// SAFETY: as above.
			unsafe {
				assert_eq!(next_seen_below(bottom), top, "right under it");
				assert_eq!(
					GetActiveWindow(),
					top,
					"who has the keyboard is a separate step"
				);
			}
		});
	}

	/// What the surface leans on. The toolkit restores and maximizes a window
	/// on its way behind a game, both of which bring it to the front, and then
	/// rewrites its extended style, which takes the layered bit an alpha veil
	/// needs. A cloak has to hold through all of it.
	#[test]
	fn a_cloak_holds_through_a_maximize_a_restore_and_a_style_rewrite() {
		on_a_private_desktop("cloak", || {
			let window = a_window();
			// SAFETY: a live window of this thread.
			unsafe { ShowWindow(window, SW_SHOWNORMAL) };
			assert!(!cloaked(window));

			assert!(cloak(window, true), "the compositor takes the request");
			assert!(cloaked(window));

			// SAFETY: as above; the style goes back as it was read.
			unsafe {
				ShowWindow(window, SW_MAXIMIZE);
				assert_ne!(IsZoomed(window), 0, "really maximized");
				assert!(cloaked(window), "through a maximize");

				ShowWindow(window, SW_RESTORE);
				assert_eq!(IsZoomed(window), 0, "really restored");
				assert!(cloaked(window), "through a restore");

				let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
				SetWindowLongPtrW(window, GWL_EXSTYLE, style);
				assert!(cloaked(window), "through a style rewrite");
			}

			assert!(cloak(window, false));
			assert!(!cloaked(window));
		});
	}
}
