//! Escape heard from outside the game, whatever state its Lua is in.
//!
//! The widget is what judges Escape, since only the game knows whether there
//! is a selection to drop or a chat to close. It can only do that once the game
//! has the keyboard, and until then our menu reads the key instead
//! (`ingame::menu`). Both need Lua that runs and reaches us, and the menu gets no
//! turn at all in the seconds before the load screen (`CPreGame` updates no Lua).
//!
//! So the lobby also listens by itself until the game says it has the keyboard.
//! A game that never says is one whose Lua cannot reach us, where nothing else
//! could hear the key, so the listening goes on: Escape then raises the lobby
//! even with units selected, which beats it doing nothing.
//!
//! Any of them can fire alongside another. Raising twice is still raising.
//!
//! It only looks, so the key reaches the game as it always did, and it asks
//! after these few keys by name with nothing hooked, so no other key is ever
//! seen.

use std::sync::Arc;
use std::time::Duration;

use super::Controller;
use super::seams::{ForegroundControl, Keyboard, Keys};

/// How often the key is looked at while a game loads: the delay it adds.
const LOOK: Duration = Duration::from_millis(50);
/// How often to look for a game starting. A game has no window to press
/// anything at for most of this.
const IDLE: Duration = Duration::from_secs(1);

/// Turns looks at the keyboard into presses meant for the game.
#[derive(Debug, Default)]
struct Presses {
	game_in_front: bool,
	escape: bool,
}

impl Presses {
	/// One look; whether it found Escape freshly pressed at the game.
	///
	/// The game has to have been in front at the look before as well. The press
	/// that dismisses the overlay lands on the lobby and hands the game the
	/// keyboard while the key is still down, and must not read as a press at the
	/// game that raises it straight back. Modifiers are read as the key goes
	/// down only: Shift+Escape is the engine's own way to abort a load, and the
	/// Shift being let go first does not make it a press of Escape.
	fn look(&mut self, game_in_front: bool, keys: Keys) -> bool {
		let pressed =
			game_in_front && self.game_in_front && keys.escape && !self.escape && !keys.modifier;
		self.game_in_front = game_in_front;
		self.escape = keys.escape;
		pressed
	}
}

pub struct Listener {
	controller: Arc<Controller>,
	foreground: Arc<dyn ForegroundControl>,
	keyboard: Arc<dyn Keyboard>,
	game: Option<u32>,
	presses: Presses,
}

impl Listener {
	pub fn new(
		controller: Arc<Controller>,
		foreground: Arc<dyn ForegroundControl>,
		keyboard: Arc<dyn Keyboard>,
	) -> Self {
		Self {
			controller,
			foreground,
			keyboard,
			game: None,
			presses: Presses::default(),
		}
	}

	pub async fn run(mut self) {
		loop {
			let wait = self.look();
			tokio::time::sleep(wait).await;
		}
	}

	/// One look, answering how long until the next.
	fn look(&mut self) -> Duration {
		let game = self.controller.loading_game();
		if game != self.game {
			self.game = game;
			self.presses = Presses::default();
		}
		let Some(pid) = game else {
			return IDLE;
		};
		let in_front = self.foreground.in_front(pid);
		if self.presses.look(in_front, self.keyboard.keys()) && !self.controller.is_over() {
			tracing::info!(pid, "Escape heard from outside the game");
			self.controller.raise();
		}
		LOOK
	}
}

/// The keyboard as the operating system reports it. Nothing to ask elsewhere:
/// X11 could answer, Wayland by design will not.
pub struct Windows;

#[cfg(windows)]
impl Keyboard for Windows {
	fn keys(&self) -> Keys {
		use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
			GetAsyncKeyState, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
		};
		// SAFETY: takes a key code and touches no memory.
		let state = |key: u16| unsafe { GetAsyncKeyState(i32::from(key)) };
		Keys {
			// The top bit is down now, the bottom one went down since the last
			// look: a quick tap can fall between two looks.
			escape: state(VK_ESCAPE) != 0,
			modifier: [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
				.into_iter()
				.any(|key| state(key) < 0),
		}
	}
}

#[cfg(not(windows))]
impl Keyboard for Windows {
	fn keys(&self) -> Keys {
		Keys::default()
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Mutex;
	use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

	use super::super::OverlaySettings;
	use super::super::seams::{Hotkeys, ScreenId, WindowId, WindowSurface};
	use super::*;

	const UP: Keys = Keys {
		escape: false,
		modifier: false,
	};
	const ESCAPE: Keys = Keys {
		escape: true,
		modifier: false,
	};
	const SHIFT: Keys = Keys {
		escape: false,
		modifier: true,
	};
	const SHIFT_ESCAPE: Keys = Keys {
		escape: true,
		modifier: true,
	};

	/// Looks in a row at a game in front, answering how many read as presses.
	fn presses_at_the_game(looks: &[Keys]) -> usize {
		let mut presses = Presses::default();
		looks
			.iter()
			.filter(|keys| presses.look(true, **keys))
			.count()
	}

	#[test]
	fn a_press_at_the_game_counts_once_however_long_it_is_held() {
		assert_eq!(presses_at_the_game(&[UP, ESCAPE]), 1);
		assert_eq!(presses_at_the_game(&[UP, ESCAPE, ESCAPE, ESCAPE, UP]), 1);
	}

	#[test]
	fn every_press_counts_when_they_come_quickly() {
		assert_eq!(
			presses_at_the_game(&[UP, ESCAPE, UP, ESCAPE, UP, ESCAPE]),
			3
		);
	}

	#[test]
	fn a_key_already_down_at_the_first_look_was_not_pressed_at_this_game() {
		assert_eq!(presses_at_the_game(&[ESCAPE, ESCAPE, UP]), 0);
	}

	#[test]
	fn shift_escape_stays_the_engines_way_to_abort_a_load() {
		assert_eq!(presses_at_the_game(&[UP, SHIFT_ESCAPE, UP]), 0);
		assert_eq!(
			presses_at_the_game(&[SHIFT, SHIFT_ESCAPE, ESCAPE, ESCAPE, UP]),
			0,
			"letting go of Shift first does not make it a press of Escape"
		);
	}

	#[test]
	fn a_press_at_another_program_is_that_programs() {
		let mut presses = Presses::default();
		assert!(!presses.look(false, UP));
		assert!(!presses.look(false, ESCAPE));
		assert!(!presses.look(false, UP));

		// Switched away since the last look: the game had it then, not now.
		let mut switched_away = Presses::default();
		assert!(!switched_away.look(true, UP));
		assert!(!switched_away.look(false, ESCAPE));
	}

	/// The press that dismisses the overlay is still down when the game gets
	/// the keyboard back, however the looks fall around it.
	#[test]
	fn the_press_that_hands_the_game_back_does_not_raise_the_lobby_again() {
		// Seen on the lobby first.
		let mut seen_on_the_lobby = Presses::default();
		assert!(!seen_on_the_lobby.look(false, UP));
		assert!(!seen_on_the_lobby.look(false, ESCAPE));
		assert!(!seen_on_the_lobby.look(true, ESCAPE));
		assert!(!seen_on_the_lobby.look(true, UP));

		// Handed back quicker than one look: first seen with the game in front.
		let mut seen_on_the_game = Presses::default();
		assert!(!seen_on_the_game.look(false, UP));
		assert!(!seen_on_the_game.look(true, ESCAPE));
		assert!(!seen_on_the_game.look(true, ESCAPE));
		assert!(!seen_on_the_game.look(true, UP));

		// And the next one is a press like any other.
		assert!(seen_on_the_game.look(true, ESCAPE));
	}

	#[derive(Default)]
	struct Stage {
		raises: AtomicUsize,
		in_front: AtomicBool,
		keys: Mutex<Keys>,
		looks_at_keys: AtomicUsize,
	}

	impl Stage {
		fn press(&self, keys: Keys) {
			*self.keys.lock().unwrap() = keys;
		}
	}

	impl WindowSurface for Stage {
		fn enter_overlay(&self, _on: Option<ScreenId>) {
			self.raises.fetch_add(1, Ordering::SeqCst);
		}
		fn leave_overlay(&self) {}
		fn show(&self) {}
		fn show_veiled(&self) {}
		fn reveal(&self) {}
		fn lower(&self, _behind: Option<WindowId>) {}
		fn focus(&self) {}
		fn screen(&self) -> Option<ScreenId> {
			None
		}
		fn is_focused(&self) -> bool {
			false
		}
	}

	impl ForegroundControl for Stage {
		fn focus(&self, _pid: u32) {}
		fn screen_of(&self, _pid: u32) -> Option<ScreenId> {
			None
		}
		fn window_of(&self, _pid: u32) -> Option<WindowId> {
			None
		}
		fn in_front(&self, _pid: u32) -> bool {
			self.in_front.load(Ordering::SeqCst)
		}
	}

	impl Hotkeys for Stage {
		fn register(&self, _accelerator: &str) {}
		fn unregister(&self) {}
	}

	impl Keyboard for Stage {
		fn keys(&self) -> Keys {
			self.looks_at_keys.fetch_add(1, Ordering::SeqCst);
			*self.keys.lock().unwrap()
		}
	}

	fn on_stage() -> (Arc<Stage>, Arc<Controller>, Listener) {
		let stage = Arc::new(Stage::default());
		stage.in_front.store(true, Ordering::SeqCst);
		let controller = Arc::new(Controller::new(
			OverlaySettings::default(),
			stage.clone(),
			stage.clone(),
			stage.clone(),
		));
		let listener = Listener::new(controller.clone(), stage.clone(), stage.clone());
		(stage, controller, listener)
	}

	#[test]
	fn escape_during_a_load_raises_the_lobby() {
		let (stage, controller, mut listener) = on_stage();
		controller.engine_running(Some(7));

		assert_eq!(listener.look(), LOOK);
		stage.press(ESCAPE);
		listener.look();

		assert_eq!(stage.raises.load(Ordering::SeqCst), 1);
		assert!(controller.is_over());
	}

	#[test]
	fn once_the_game_has_the_keyboard_the_key_is_not_even_looked_at() {
		let (stage, controller, mut listener) = on_stage();
		controller.engine_running(Some(7));
		listener.look();
		let looks = stage.looks_at_keys.load(Ordering::SeqCst);

		assert!(controller.game_loaded());
		stage.press(ESCAPE);

		assert_eq!(listener.look(), IDLE);
		assert_eq!(stage.raises.load(Ordering::SeqCst), 0);
		assert_eq!(stage.looks_at_keys.load(Ordering::SeqCst), looks);
	}

	/// The menu reads the same press at the game's frame rate, usually sooner.
	#[test]
	fn a_lobby_already_up_is_not_raised_again() {
		let (stage, controller, mut listener) = on_stage();
		controller.engine_running(Some(7));
		listener.look();

		controller.raise();
		stage.press(ESCAPE);
		listener.look();

		assert_eq!(stage.raises.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn with_no_game_the_key_is_not_looked_at() {
		let (stage, _controller, mut listener) = on_stage();
		assert_eq!(listener.look(), IDLE);
		assert_eq!(stage.looks_at_keys.load(Ordering::SeqCst), 0);
	}

	#[test]
	fn a_game_that_never_says_goes_on_being_listened_for() {
		let (stage, controller, mut listener) = on_stage();
		controller.engine_running(Some(7));
		listener.look();
		stage.press(ESCAPE);
		listener.look();
		assert_eq!(stage.raises.load(Ordering::SeqCst), 1);

		// Back at the game, long after any load would be over.
		controller.hotkey();
		for keys in [UP, ESCAPE] {
			stage.press(keys);
			listener.look();
		}
		assert_eq!(stage.raises.load(Ordering::SeqCst), 2);
	}

	#[test]
	fn a_game_taken_over_keeps_its_escape() {
		let (stage, controller, mut listener) = on_stage();
		controller.engine_running(Some(7));
		controller.taken_over(7);
		listener.look();
		stage.press(ESCAPE);
		listener.look();
		assert_eq!(stage.raises.load(Ordering::SeqCst), 0);
	}

	/// What was seen of the last game says nothing of this one's window.
	#[test]
	fn a_new_game_gets_a_look_of_its_own_before_a_press_can_count() {
		let (stage, controller, mut listener) = on_stage();
		controller.engine_running(Some(7));
		listener.look();

		controller.engine_exited();
		listener.look();
		controller.engine_running(Some(8));
		stage.press(ESCAPE);
		listener.look();
		assert_eq!(stage.raises.load(Ordering::SeqCst), 0);

		stage.press(UP);
		listener.look();
		stage.press(ESCAPE);
		listener.look();
		assert_eq!(stage.raises.load(Ordering::SeqCst), 1);
	}
}
