//! Flashing a window that is not ours.
//!
//! Our own window has [`requestUserAttention`][tauri], but the one worth
//! pointing at when a game starts or ends belongs to the engine, which is a
//! process we spawned and do not own the windows of. Chobby has the same
//! problem and solves it by shelling out to a PowerShell one-shot that calls
//! `FlashWindowEx` (`dist_cfg/exts/os_notifications.js`); this calls it
//! directly, with the same flags.
//!
//! [tauri]: https://docs.rs/tauri/latest/tauri/window/struct.Window.html#method.request_user_attention

/// Flashes the taskbar entry of every top-level window owned by `pid`.
///
/// Returns whether anything was flashed — a game that is loading has no window
/// yet, and one that has exited has none any more, and the caller wants to know
/// so it can fall back to flashing the lobby.
#[cfg(windows)]
pub fn flash_process(pid: u32) -> bool {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::{HWND, LPARAM, TRUE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, FLASHW_ALL, FLASHW_TIMERNOFG, FLASHWINFO, FlashWindowEx,
        GetWindowThreadProcessId, IsWindowVisible,
    };
    use windows_sys::core::BOOL;

    struct Hunt {
        wanted: u32,
        found: bool,
    }

    unsafe extern "system" fn visit(window: HWND, state: LPARAM) -> BOOL {
        // SAFETY: `state` is the `&mut Hunt` handed to `EnumWindows` below,
        // which outlives the enumeration.
        let hunt = unsafe { &mut *(state as *mut Hunt) };

        let mut owner = 0_u32;
        // SAFETY: `window` comes from the enumeration and `owner` is ours.
        unsafe { GetWindowThreadProcessId(window, &mut owner) };
        if owner != hunt.wanted {
            return TRUE;
        }
        // SAFETY: a window handle from the enumeration.
        if unsafe { IsWindowVisible(window) } == 0 {
            return TRUE;
        }

        let flash = FLASHWINFO {
            cbSize: size_of::<FLASHWINFO>() as u32,
            hwnd: window,
            // Caption and tray, and no stopping until it is looked at: the same
            // pair Chobby asks for as the literal 15.
            dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
            uCount: 0,
            dwTimeout: 0,
        };
        // SAFETY: `flash` is fully initialised and lives across the call.
        unsafe { FlashWindowEx(&flash) };
        hunt.found = true;
        TRUE
    }

    let mut hunt = Hunt {
        wanted: pid,
        found: false,
    };
    // SAFETY: `visit` matches the expected signature and `hunt` outlives the
    // call, which is synchronous.
    unsafe {
        EnumWindows(Some(visit), &raw mut hunt as *mut c_void as LPARAM);
    }
    hunt.found
}

/// Nothing to flash: the caller falls back to the lobby's own window.
///
/// macOS bounces the dock icon for the *application*, not for an arbitrary
/// process, and X11 and Wayland leave this to the window manager. Neither is
/// reachable for a child process the way `FlashWindowEx` is.
#[cfg(not(windows))]
pub fn flash_process(_pid: u32) -> bool {
    false
}
