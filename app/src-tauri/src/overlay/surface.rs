//! The main window, told to change shape.
//!
//! There is one window and it morphs. A second window is not an option: the
//! runtime keeps exactly one UI transport (`Command::Subscribe` replaces it),
//! so an overlay window would take the stream from the lobby behind it.
//!
//! Overlay shape is borderless, always-on-top, off the taskbar, and covering
//! the monitor — deliberately not `set_fullscreen(true)`, which asks the OS
//! for its own fullscreen treatment and then fights the game for it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use super::seams::{ScreenId, WindowSurface};

/// What the window looked like before it became an overlay.
#[derive(Debug, Clone, Copy)]
struct Restore {
	position: PhysicalPosition<i32>,
	size: PhysicalSize<u32>,
	decorated: bool,
	maximized: bool,
	/// The origin of the monitor `position` was on, so the same place can
	/// be found on another; see [`TauriSurface::place_back`].
	before: Option<PhysicalPosition<i32>>,
	/// The origin of the monitor the overlay was last fitted to. The window
	/// on any other has been moved by hand since.
	fitted: Option<PhysicalPosition<i32>>,
}

/// A monitor's origin and extent, in physical pixels.
type Area = (PhysicalPosition<i32>, PhysicalSize<u32>);

pub struct TauriSurface {
	window: WebviewWindow,
	/// Held in memory only. Overlay geometry is never persisted, so a crash
	/// mid-overlay cannot leave a decorationless always-on-top window behind
	/// on the next start.
	restore: Mutex<Option<Restore>>,
	/// Which veiled show is current. The fallback reveal for an earlier one
	/// must not lift a veil that a later show has just drawn.
	veiled: Arc<AtomicU64>,
}

/// How long a veiled window waits for the page before showing itself
/// anyway. Two frames is what the page needs; a page that has hung -- the
/// case the overlay exists for -- never answers, and must not stay invisible.
const VEIL_PATIENCE: Duration = Duration::from_millis(250);

impl TauriSurface {
	pub fn new(window: WebviewWindow) -> Self {
		Self {
			window,
			restore: Mutex::new(None),
			veiled: Arc::new(AtomicU64::new(0)),
		}
	}

	fn enter(&self, on: Option<ScreenId>) {
		// Out of the taskbar before anything is read or set. A minimized window
		// reports itself unmaximized, sits at (-32000, -32000) with no size, and
		// takes none of the geometry below: the later `show` restores it to
		// whatever it was minimized from. From maximized that is a borderless
		// maximized window, which the toolkit holds to the work area — the
		// overlay stops at the taskbar's edge with raw game under it.
		let _ = self.window.unminimize();
		// Unmaximized next, as `screen.rs` does. A maximized window's rectangle
		// hangs past every edge of the monitor, and given back as an ordinary
		// window it stays there: the page clipped on the right, and saved that
		// way by the window-state plugin for every start after. The flag puts it
		// back to maximized instead.
		let maximized = self.window.is_maximized().unwrap_or(false);
		if maximized {
			let _ = self.window.unmaximize();
		}
		let remembered = Restore {
			position: self.window.outer_position().unwrap_or_default(),
			// Inner, because that is what `set_size` takes: an outer size given
			// back grows the window by its invisible frame on every round trip.
			size: self.window.inner_size().unwrap_or_default(),
			decorated: self.window.is_decorated().unwrap_or(true),
			maximized,
			before: self.own_monitor().map(|(origin, _)| origin),
			fitted: None,
		};
		// Kept from the first entry only: entered again while still in this
		// shape, the window would remember the overlay as what to go back to.
		self.restore
			.lock()
			.expect("overlay geometry")
			.get_or_insert(remembered);

		let _ = self.window.set_decorations(false);
		// Not something to maximize: the nav drags the overlay as it drags
		// any window, and a double-click there would otherwise shrink the
		// overlay to the work area, with the taskbar showing under the game.
		let _ = self.window.set_maximizable(false);
		// The shadow has to go with the decorations. On Windows an undecorated
		// window with a shadow keeps an invisible frame roughly 8px wide, so a
		// window told to sit at the monitor's corner actually lands shifted
		// right and down — which showed as a thin strip of raw, undimmed game
		// along the left edge of the overlay. Positioning happens after this,
		// so the coordinates land where they say.
		let _ = self.window.set_shadow(false);
		let _ = self.window.set_always_on_top(true);
		let _ = self.window.set_skip_taskbar(true);

		// The game's monitor where it is known, else the one the window is
		// on. Not the same thing: the launcher pins the engine to the primary
		// monitor, wherever the lobby was, and a lobby raised from minimized
		// is on no monitor at all until it is shown.
		let Some((origin, size)) = monitor_area(on).or_else(|| self.own_monitor()) else {
			return;
		};
		let _ = self.window.set_position(origin);
		let _ = self.window.set_size(size);
		if let Some(restore) = self.restore.lock().expect("overlay geometry").as_mut() {
			restore.fitted = Some(origin);
		}
	}

	fn leave(&self) {
		let remembered = self.restore.lock().expect("overlay geometry").take();
		let _ = self.window.set_always_on_top(false);
		let _ = self.window.set_skip_taskbar(false);
		let _ = self.window.set_maximizable(true);
		// Back on, unconditionally: a decorated window ignores it, and every
		// ordinary window wants its shadow.
		let _ = self.window.set_shadow(true);

		// Nothing recorded means nothing was changed: the window is frameless
		// by design, so there is no frame to give back.
		let Some(remembered) = remembered else {
			return;
		};
		let _ = self.window.set_decorations(remembered.decorated);
		let _ = self.window.set_position(self.place_back(&remembered));
		let _ = self.window.set_size(remembered.size);
		if remembered.maximized {
			let _ = self.window.maximize();
		}
	}

	/// Where the window goes back to: where it was, unless it has been moved
	/// to another monitor since it was fitted over the game -- by hand, with
	/// Win+Shift+Arrow -- in which case the same place on that monitor, kept
	/// inside it. A window somebody moved is a window they wanted there.
	fn place_back(&self, remembered: &Restore) -> PhysicalPosition<i32> {
		let (Some((origin, extent)), Some(before), Some(fitted)) =
			(self.own_monitor(), remembered.before, remembered.fitted)
		else {
			return remembered.position;
		};
		if origin == fitted {
			return remembered.position;
		}
		let room = |span: u32, size: u32| span.saturating_sub(size) as i32;
		PhysicalPosition::new(
			origin.x
				+ (remembered.position.x - before.x)
					.clamp(0, room(extent.width, remembered.size.width)),
			origin.y
				+ (remembered.position.y - before.y)
					.clamp(0, room(extent.height, remembered.size.height)),
		)
	}

	/// The monitor holding most of the window, as the toolkit has it.
	fn own_monitor(&self) -> Option<Area> {
		let monitor = self.window.current_monitor().ok().flatten()?;
		Some((*monitor.position(), *monitor.size()))
	}

	/// What the window is now, as the toolkit believes it and as the OS has
	/// it. The two can disagree: the toolkit keeps its own copy of the flags
	/// and skips a change it thinks is already made, so a shape that went
	/// wrong is only visible with both side by side.
	fn log_shape(&self, over: bool) {
		let believed = (
			self.window.is_decorated().ok(),
			self.window.is_always_on_top().ok(),
			self.window.is_visible().ok(),
		);
		let (style, ex_style) = self.os_styles();
		tracing::info!(
			over,
			decorated = ?believed.0,
			on_top = ?believed.1,
			visible = ?believed.2,
			style = format_args!("{style:#x}"),
			ex_style = format_args!("{ex_style:#x}"),
			"overlay: window shaped"
		);
	}

	/// `GWL_STYLE` and `GWL_EXSTYLE` straight from the window, so the log can
	/// say what the OS has rather than what was asked for.
	#[cfg(windows)]
	fn os_styles(&self) -> (isize, isize) {
		use windows_sys::Win32::UI::WindowsAndMessaging::{
			GWL_EXSTYLE, GWL_STYLE, GetWindowLongPtrW,
		};
		let Ok(hwnd) = self.window.hwnd() else {
			return (0, 0);
		};
		// SAFETY: a live handle to our own window.
		unsafe {
			(
				GetWindowLongPtrW(hwnd.0 as _, GWL_STYLE),
				GetWindowLongPtrW(hwnd.0 as _, GWL_EXSTYLE),
			)
		}
	}

	#[cfg(not(windows))]
	fn os_styles(&self) -> (isize, isize) {
		(0, 0)
	}

	/// The shape is on, or off; the page is told either way. It dresses
	/// differently over a game — a centred card on a see-through scrim, Esc
	/// and click-outside to leave. A webview that reloads mid-overlay asks
	/// `overlay_active` instead.
	fn dressed(&self, over: bool) {
		self.log_shape(over);
		let _ = self.window.emit("overlay", over);
	}

	/// The monitor the window is on, by the handle the game's window is
	/// looked up by too, so the two compare. A minimized window is on none.
	#[cfg(windows)]
	fn monitor(&self) -> Option<ScreenId> {
		let hwnd = self.window.hwnd().ok()?;
		crate::win::monitor_of(hwnd.0 as _).map(ScreenId)
	}

	/// Elsewhere the game's monitor cannot be found at all, so there is
	/// nothing for ours to be compared with.
	#[cfg(not(windows))]
	fn monitor(&self) -> Option<ScreenId> {
		None
	}
}

/// A window's alpha, through the layered-window style.
///
/// The toolkit has no opacity call, and it rewrites `GWL_EXSTYLE` from its
/// own cache whenever a flag changes, so the layered bit is put on after
/// `show()` and taken off again once the window is opaque -- the toolkit
/// never learns it was there, and it never has a chance to clear it early.
#[cfg(windows)]
fn set_alpha(window: &WebviewWindow, alpha: u8) {
	use windows_sys::Win32::UI::WindowsAndMessaging::{
		GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW,
		WS_EX_LAYERED,
	};
	let Ok(hwnd) = window.hwnd() else {
		return;
	};
	let hwnd = hwnd.0 as _;
	let layered = WS_EX_LAYERED as isize;
	// SAFETY: a live handle to our own window; plain style bits.
	unsafe {
		let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
		SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | layered);
		SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
		if alpha == u8::MAX {
			SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !layered);
		}
	}
}

#[cfg(not(windows))]
fn set_alpha(_window: &WebviewWindow, _alpha: u8) {}

/// The extent of a monitor named by the game's window, where the platform
/// can say.
#[cfg(windows)]
fn monitor_area(screen: Option<ScreenId>) -> Option<Area> {
	let area = crate::win::monitor_rect(screen?.0)?;
	Some((
		PhysicalPosition::new(area.left, area.top),
		PhysicalSize::new(area.width, area.height),
	))
}

#[cfg(not(windows))]
fn monitor_area(_screen: Option<ScreenId>) -> Option<Area> {
	None
}

impl WindowSurface for TauriSurface {
	fn enter_overlay(&self, on: Option<ScreenId>) {
		self.enter(on);
		self.dressed(true);
	}

	fn leave_overlay(&self) {
		self.leave();
		self.dressed(false);
	}

	fn show(&self) {
		let _ = self.window.show();
		let _ = self.window.unminimize();
	}

	fn show_veiled(&self) {
		let generation = self.veiled.fetch_add(1, Ordering::SeqCst) + 1;
		let _ = self.window.show();
		let _ = self.window.unminimize();
		set_alpha(&self.window, 0);
		// The page answers with `overlay_painted` after two frames.
		let _ = self.window.emit("overlay-veiled", ());

		let window = self.window.clone();
		let veiled = Arc::clone(&self.veiled);
		std::thread::spawn(move || {
			std::thread::sleep(VEIL_PATIENCE);
			if veiled.load(Ordering::SeqCst) == generation {
				tracing::info!("overlay: page did not report painting; revealing anyway");
				set_alpha(&window, u8::MAX);
			}
		});
	}

	fn reveal(&self) {
		// Counts as a new generation so the pending fallback finds nothing
		// left to do, rather than revealing twice and saying the page was late.
		self.veiled.fetch_add(1, Ordering::SeqCst);
		set_alpha(&self.window, u8::MAX);
	}

	fn hide(&self) {
		let _ = self.window.hide();
	}

	fn focus(&self) {
		let _ = self.window.set_focus();
	}

	fn screen(&self) -> Option<ScreenId> {
		self.monitor()
	}

	fn is_focused(&self) -> bool {
		self.window.is_focused().unwrap_or(false)
	}
}

/// The main window, or nothing if it has gone.
pub fn main_window(app: &tauri::AppHandle) -> Option<WebviewWindow> {
	app.get_webview_window("main")
}
