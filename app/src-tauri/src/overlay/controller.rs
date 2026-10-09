//! What turns decisions into window calls.
//!
//! The only place that knows all three seams. Everything it does comes from
//! [`super::state`], so the ordering questions — hide before focusing the
//! game, restore before showing after a crash — are settled in tests rather
//! than here.

use std::sync::{Arc, Mutex};

use lobby_ui::{Delta, EngineStatus, UiMessage};

use super::seams::{ForegroundControl, Hotkeys, WindowSurface};
use super::state::{Effect, Input, Overlay, OverlaySettings, Placement};

pub struct Controller {
	overlay: Mutex<Overlay>,
	surface: Arc<dyn WindowSurface>,
	foreground: Arc<dyn ForegroundControl>,
	hotkeys: Arc<dyn Hotkeys>,
}

impl Controller {
	pub fn new(
		settings: OverlaySettings,
		surface: Arc<dyn WindowSurface>,
		foreground: Arc<dyn ForegroundControl>,
		hotkeys: Arc<dyn Hotkeys>,
	) -> Self {
		Self {
			overlay: Mutex::new(Overlay::new(settings)),
			surface,
			foreground,
			hotkeys,
		}
	}

	pub fn engine_running(&self, pid: Option<u32>) {
		self.drive(Input::EngineRunning { pid });
	}

	pub fn engine_exited(&self) {
		self.drive(Input::EngineExited);
	}

	pub fn hotkey(&self) {
		let engine = self.overlay.lock().expect("overlay").engine_pid();
		self.drive(Input::Hotkey(self.placement(engine)));
	}

	/// Where the window stands against the game's: beside it only when both
	/// monitors are known and differ. Not knowing means over, which is what
	/// every press meant before anyone asked.
	fn placement(&self, engine: Option<u32>) -> Placement {
		let (Some(pid), Some(ours)) = (engine, self.surface.screen()) else {
			return Placement::Over;
		};
		match self.foreground.screen_of(pid) {
			Some(theirs) if theirs != ours => Placement::Beside {
				focused: self.surface.is_focused(),
			},
			_ => Placement::Over,
		}
	}

	/// The page has painted its overlay dress; the veiled window can be seen.
	pub fn painted(&self) {
		self.drive(Input::Painted);
	}

	pub fn settings_changed(&self, settings: OverlaySettings) {
		self.drive(Input::Settings(settings));
	}

	/// The process is exiting: the window gets its ordinary shape back first,
	/// since the shape it is closed in is the shape it opens in next time.
	pub fn shut_down(&self) {
		self.drive(Input::Shutdown);
	}

	/// Whether the window is currently over a game, for the front end to show
	/// a different face.
	pub fn is_over(&self) -> bool {
		self.overlay.lock().expect("overlay").is_over()
	}

	/// Whether a game of ours is running with the overlay switched on.
	pub fn armed_for_game(&self) -> bool {
		self.overlay.lock().expect("overlay").armed_for_game()
	}

	/// Raises the lobby, and fits it over the game again if it is already
	/// up. Unlike the hotkey this never lowers it: the in-game Escape is a
	/// one-way door out of the game.
	pub fn raise(&self) -> bool {
		if !self.armed_for_game() {
			return false;
		}
		let engine = self.overlay.lock().expect("overlay").engine_pid();
		self.drive(Input::Raise(self.placement(engine)));
		true
	}

	fn drive(&self, input: Input) {
		// The lock is released before anything touches a window: a window call
		// can re-enter through an event handler, and holding this across one
		// would deadlock the next hotkey press.
		let (effects, engine) = {
			let mut overlay = self.overlay.lock().expect("overlay");
			let effects = super::state::step(&mut overlay, input.clone());
			tracing::info!(?input, ?effects, over = overlay.is_over(), "overlay");
			(effects, overlay.engine_pid())
		};
		for effect in effects {
			self.apply(effect, engine);
		}
	}

	fn apply(&self, effect: Effect, engine: Option<u32>) {
		match effect {
			Effect::RegisterHotkey(accelerator) => self.hotkeys.register(&accelerator),
			Effect::UnregisterHotkey => self.hotkeys.unregister(),
			// Over the game's monitor, which is not always the lobby's: a
			// lobby raised from minimized has none of its own to speak of.
			Effect::EnterOverlay => self
				.surface
				.enter_overlay(engine.and_then(|pid| self.foreground.screen_of(pid))),
			Effect::LeaveOverlay => self.surface.leave_overlay(),
			Effect::Show => self.surface.show(),
			Effect::ShowVeiled => self.surface.show_veiled(),
			Effect::Reveal => self.surface.reveal(),
			Effect::Hide => self.surface.hide(),
			Effect::FocusSelf => self.surface.focus(),
			Effect::FocusEngine(pid) => self.foreground.focus(pid),
		}
	}

	/// Reads engine news out of the stream on its way to the webview.
	///
	/// Tapped here rather than sent from the front end so that arming does not
	/// depend on a webview being alive and responsive — a page that has hung
	/// is exactly when being able to raise the lobby matters.
	pub fn observe(&self, message: &UiMessage) {
		match message {
			UiMessage::Snapshot(snapshot) => self.note(&snapshot.engine),
			// A session's: the engine is the machine's.
			UiMessage::Session(_) => {}
			UiMessage::Deltas { deltas, .. } => {
				for delta in deltas {
					if let Delta::Engine(status) = delta {
						self.note(status);
					}
				}
			}
		}
	}

	fn note(&self, status: &EngineStatus) {
		match status {
			EngineStatus::Running { pid } => self.engine_running(*pid),
			EngineStatus::Idle | EngineStatus::Exited { .. } => self.engine_exited(),
		}
	}
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::{AtomicBool, Ordering};

	use super::super::seams::ScreenId;
	use super::*;

	#[derive(Default)]
	struct Spy {
		over: AtomicBool,
		shown: AtomicBool,
		registered: AtomicBool,
		focused_engine: AtomicBool,
		/// The monitors the lobby and the game are on, where the spy knows;
		/// a test moves either by writing here.
		ours: Mutex<Option<ScreenId>>,
		theirs: Mutex<Option<ScreenId>>,
		/// The monitor the overlay was last fitted to.
		fitted_to: Mutex<Option<ScreenId>>,
	}

	impl Spy {
		fn on(ours: Option<ScreenId>, theirs: Option<ScreenId>) -> Self {
			Self {
				ours: Mutex::new(ours),
				theirs: Mutex::new(theirs),
				..Self::default()
			}
		}
	}

	impl WindowSurface for Spy {
		fn enter_overlay(&self, on: Option<ScreenId>) {
			self.over.store(true, Ordering::SeqCst);
			*self.fitted_to.lock().unwrap() = on;
		}
		fn leave_overlay(&self) {
			self.over.store(false, Ordering::SeqCst);
		}
		fn show(&self) {
			self.shown.store(true, Ordering::SeqCst);
		}
		fn show_veiled(&self) {
			self.shown.store(true, Ordering::SeqCst);
		}
		fn reveal(&self) {}
		fn hide(&self) {
			self.shown.store(false, Ordering::SeqCst);
		}
		fn focus(&self) {}
		fn screen(&self) -> Option<ScreenId> {
			*self.ours.lock().unwrap()
		}
		fn is_focused(&self) -> bool {
			true
		}
	}

	impl ForegroundControl for Spy {
		fn focus(&self, _pid: u32) {
			self.focused_engine.store(true, Ordering::SeqCst);
		}
		fn screen_of(&self, _pid: u32) -> Option<ScreenId> {
			*self.theirs.lock().unwrap()
		}
	}

	impl Hotkeys for Spy {
		fn register(&self, _accelerator: &str) {
			self.registered.store(true, Ordering::SeqCst);
		}
		fn unregister(&self) {
			self.registered.store(false, Ordering::SeqCst);
		}
	}

	fn controller() -> (Arc<Spy>, Controller) {
		controller_with(Spy::default())
	}

	fn controller_with(spy: Spy) -> (Arc<Spy>, Controller) {
		let spy = Arc::new(spy);
		let controller = Controller::new(
			OverlaySettings::default(),
			spy.clone(),
			spy.clone(),
			spy.clone(),
		);
		(spy, controller)
	}

	#[test]
	fn a_game_on_another_monitor_is_handed_the_keyboard_rather_than_covered() {
		let (spy, controller) = controller_with(Spy::on(Some(ScreenId(1)), Some(ScreenId(2))));
		controller.engine_running(Some(7));
		// The spy says the lobby has focus, so the press goes to the game.
		controller.hotkey();
		assert!(
			!spy.over.load(Ordering::SeqCst),
			"the window keeps its shape"
		);
		assert!(spy.focused_engine.load(Ordering::SeqCst));
	}

	#[test]
	fn a_monitor_nobody_can_name_counts_as_the_games() {
		let (spy, controller) = controller_with(Spy::on(Some(ScreenId(1)), None));
		controller.engine_running(Some(7));
		controller.hotkey();
		assert!(spy.over.load(Ordering::SeqCst));
	}

	#[test]
	fn the_overlay_is_fitted_to_the_games_monitor_not_the_lobbys() {
		// A minimized lobby is on no monitor; the game is on its own.
		let (spy, controller) = controller_with(Spy::on(None, Some(ScreenId(2))));
		controller.engine_running(Some(7));
		controller.hotkey();
		assert!(spy.over.load(Ordering::SeqCst));
		assert_eq!(*spy.fitted_to.lock().unwrap(), Some(ScreenId(2)));
	}

	#[test]
	fn moved_onto_another_monitor_the_next_press_gives_the_window_its_shape_back() {
		let (spy, controller) = controller_with(Spy::on(Some(ScreenId(1)), Some(ScreenId(1))));
		controller.engine_running(Some(7));
		controller.hotkey();
		assert!(spy.over.load(Ordering::SeqCst));

		// Win+Shift+Arrow, while the overlay is up.
		*spy.ours.lock().unwrap() = Some(ScreenId(2));
		controller.hotkey();
		assert!(!spy.over.load(Ordering::SeqCst), "an ordinary window again");
		assert!(spy.focused_engine.load(Ordering::SeqCst));
	}

	#[test]
	fn a_running_engine_in_the_stream_arms_the_hotkey() {
		let (spy, controller) = controller();
		controller.observe(&UiMessage::Deltas {
			server: None,
			deltas: vec![Delta::Engine(EngineStatus::Running { pid: Some(7) })],
		});
		assert!(spy.registered.load(Ordering::SeqCst));
		assert!(!spy.over.load(Ordering::SeqCst), "armed, not imposed");
	}

	#[test]
	fn the_engine_exiting_releases_the_key_and_restores_the_window() {
		let (spy, controller) = controller();
		controller.engine_running(Some(7));
		controller.hotkey();
		assert!(spy.over.load(Ordering::SeqCst));

		controller.observe(&UiMessage::Deltas {
			server: None,
			deltas: vec![Delta::Engine(EngineStatus::Exited { code: Some(0) })],
		});
		assert!(!spy.registered.load(Ordering::SeqCst));
		assert!(!spy.over.load(Ordering::SeqCst));
		assert!(spy.shown.load(Ordering::SeqCst), "and it is on screen");
	}

	#[test]
	fn a_snapshot_arms_as_readily_as_a_delta() {
		// A webview that reloaded mid-game gets a snapshot, not a delta, and
		// the overlay has to arm from it or the hotkey silently stops working.
		let (spy, controller) = controller();
		let snapshot = lobby_ui::Snapshot {
			engine: EngineStatus::Running { pid: Some(11) },
			..lobby_ui::Snapshot::default()
		};
		controller.observe(&UiMessage::Snapshot(Box::new(snapshot)));
		assert!(spy.registered.load(Ordering::SeqCst));
	}
}
