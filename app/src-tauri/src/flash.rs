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

/// Flashes the taskbar entry of every visible top-level window owned by `pid`.
///
/// Returns whether anything was flashed — a game that is loading has no window
/// yet, and one that has exited has none any more, and the caller wants to know
/// so it can fall back to flashing the lobby.
#[cfg(windows)]
pub fn flash_process(pid: u32) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FLASHW_ALL, FLASHW_TIMERNOFG, FLASHWINFO, FlashWindowEx,
    };

    let windows = crate::win::visible_windows_of(pid);
    for window in &windows {
        let flash = FLASHWINFO {
            cbSize: size_of::<FLASHWINFO>() as u32,
            hwnd: *window,
            // Caption and tray, and no stopping until it is looked at: the same
            // pair Chobby asks for as the literal 15.
            dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
            uCount: 0,
            dwTimeout: 0,
        };
        // SAFETY: `flash` is fully initialised and lives across the call.
        unsafe { FlashWindowEx(&flash) };
    }
    !windows.is_empty()
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
