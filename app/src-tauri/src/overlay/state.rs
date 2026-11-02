//! When the lobby should sit over the game, and what that costs.
//!
//! Pure: nothing here knows about Tauri, Win32 or the hotkey plugin. Those are
//! traits the executor implements, and this decides. The parts of an overlay
//! that actually go wrong are ordering, restoring after a crash, and holding a
//! system-wide key longer than a game lasts — all of which are decisions, and
//! all of which are tested below with no window open.
//!
//! The shape is a desired-state reducer rather than a state enum with
//! transitions. Settings can change at any moment, including while a game is
//! running, and "work out what should be true and emit the difference" stays
//! correct under that where a transition table quietly does not.

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlaySettings {
	pub enabled: bool,
	/// A Tauri accelerator, e.g. `Alt+Shift+L`.
	pub hotkey: String,
	/// Whether dismissing the overlay should put the game back in front.
	pub return_focus_to_game: bool,
	/// Whether Escape in a game should raise the lobby.
	pub in_game_escape: bool,
}

impl Default for OverlaySettings {
	fn default() -> Self {
		Self {
			enabled: true,
			hotkey: "Alt+Shift+L".into(),
			return_focus_to_game: true,
			in_game_escape: true,
		}
	}
}

/// Where the lobby's window stands against the game's, read as the key is
/// pressed: it is the one fact the reducer cannot carry, since either window
/// can be dragged to another monitor at any time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
	/// On the game's monitor, or nowhere that can be told apart from it.
	Over,
	/// On a monitor of its own, where it covers nothing of the game.
	Beside {
		/// Whether the lobby has the keyboard at the moment of the press.
		focused: bool,
	},
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
	/// A game of ours started. The pid is what a window can be found by.
	EngineRunning {
		pid: Option<u32>,
	},
	EngineExited,
	/// The registered accelerator fired: the lobby up, or back to the game.
	Hotkey(Placement),
	/// The in-game Escape, or the page's own button: the lobby up, never
	/// down. Already up, it is fitted over the game again.
	Raise(Placement),
	/// The running game says it has the keyboard: the load is over and Escape
	/// is the widget's to judge.
	GameLoaded,
	/// This process is a game a lobby before this one left running. It is past
	/// loading, or as good as: it holds that lobby's tokens and will never say.
	/// By id because this comes from elsewhere than the news of it running, and
	/// may overtake it.
	TakenOver(u32),
	Settings(OverlaySettings),
	/// The page has drawn itself in its overlay dress, so the window can be
	/// seen. Until then it is shown but see-through: the first frames of a
	/// window just reshaped are whatever WebView2 had last, at the old size,
	/// and that is the "pop" between game and lobby.
	Painted,
	/// The process is on its way out. The window gets its ordinary shape
	/// back before anything remembers it, and is not shown for it.
	Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
	RegisterHotkey(String),
	UnregisterHotkey,
	/// Borderless, always on top, sized to the monitor the game is on.
	EnterOverlay,
	/// Decorations and the previous geometry back.
	LeaveOverlay,
	Show,
	/// On screen, focusable, but fully transparent until [`Effect::Reveal`]
	/// or the surface's own patience runs out.
	ShowVeiled,
	/// Make the veiled window opaque.
	Reveal,
	/// Its ordinary shape back, unseen, and out of the game's way: directly
	/// behind the window of the game named here by process id, or, with none,
	/// out of the way for the OS to say who is next. It stays on the taskbar
	/// either way, for when the game's own way back to the lobby fails.
	Lower {
		behind: Option<u32>,
	},
	/// Put the game's window in front, by process id.
	FocusEngine(u32),
	FocusSelf,
}

/// Where the window stands against a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
	/// An ordinary window, wherever the player left it. A game opens over it.
	Ordinary,
	/// Borderless, on top, sized to the game's monitor.
	Over,
	/// An ordinary window again, put out of the game's way by a press. Apart
	/// from `Ordinary` because it may need bringing back: minimized where the
	/// game's window could not be named, hidden where a platform cannot order
	/// windows, and without that a game ending leaves no window and no hotkey
	/// left to summon one with.
	Lowered,
}

#[derive(Debug)]
pub struct Overlay {
	settings: OverlaySettings,
	/// `Some` while a game of ours is running; the inner value is its pid.
	engine: Option<Option<u32>>,
	/// The accelerator currently held, so a changed hotkey re-registers once
	/// and an unchanged one does nothing.
	registered: Option<String>,
	window: Standing,
	/// The game known to be past loading. By id, so it needs no clearing: the
	/// next game is simply not this one.
	loaded: Option<u32>,
}

impl Default for Overlay {
	fn default() -> Self {
		Self {
			settings: OverlaySettings::default(),
			engine: None,
			registered: None,
			window: Standing::Ordinary,
			loaded: None,
		}
	}
}

impl Overlay {
	pub fn new(settings: OverlaySettings) -> Self {
		Self {
			settings,
			..Self::default()
		}
	}

	/// Whether the window is currently sitting over a game.
	pub fn is_over(&self) -> bool {
		self.window == Standing::Over
	}

	/// Whether a game *we* launched is running and the overlay is switched on.
	///
	/// The in-game widget asks this before it consumes Escape: a game started
	/// from another lobby while modlobby happens to be open must keep its own
	/// Escape, and only this side knows whose game it is.
	pub fn armed_for_game(&self) -> bool {
		self.armed()
	}

	/// The running game's process id, where one is known.
	pub fn engine_pid(&self) -> Option<u32> {
		self.engine.flatten()
	}

	/// The game that cannot hear Escape for itself yet, for the lobby to listen
	/// on its behalf (see [`super::escape`]): ours, with Escape wanted, not yet
	/// said to have the keyboard, and with a process id to tell it is in front.
	pub fn loading_game(&self) -> Option<u32> {
		if !self.settings.enabled || !self.settings.in_game_escape {
			return None;
		}
		self.engine_pid().filter(|pid| self.loaded != Some(*pid))
	}

	pub fn step(&mut self, input: Input) -> Vec<Effect> {
		let mut out = Vec::new();
		match input {
			Input::EngineRunning { pid } => self.engine = Some(pid),
			Input::EngineExited => self.engine = None,
			Input::Settings(settings) => self.settings = settings,
			Input::GameLoaded => self.loaded = self.engine_pid().or(self.loaded),
			Input::TakenOver(pid) => self.loaded = Some(pid),
			Input::Hotkey(placement) => {
				// A key that fired after the game ended, or while the feature
				// is off, is not an instruction — it is a race with
				// unregistering.
				if !self.armed() {
					return out;
				}
				if let Placement::Beside { focused } = placement {
					self.beside(focused, &mut out);
				} else if !self.is_over() {
					self.over(&mut out);
				} else {
					self.lower(&mut out);
				}
				return out;
			}
			Input::Raise(placement) => {
				if !self.armed() {
					return out;
				}
				match placement {
					// Raising asks for the lobby, whoever had the keyboard.
					Placement::Beside { .. } => self.beside(false, &mut out),
					Placement::Over => self.over(&mut out),
				}
				return out;
			}
			Input::Painted => {
				// A report from a page that is no longer over a game -- the
				// hotkey was pressed twice inside two frames -- is stale.
				if self.is_over() {
					out.push(Effect::Reveal);
				}
				return out;
			}
			Input::Shutdown => {
				// Nothing to sit over any more, and no window to bring back
				// either: it is about to go. Counted as ordinary so the
				// reconciliation below does not show it on the way out.
				self.engine = None;
				if self.window == Standing::Lowered {
					self.window = Standing::Ordinary;
				}
			}
		}
		out
	}

	fn armed(&self) -> bool {
		self.settings.enabled && self.engine.is_some()
	}

	/// Beside the game the lobby covers nothing: there is no game to get out
	/// of the way of and no screen to take, so it is an ordinary window.
	/// Given its shape back if it is in the overlay's -- fitted over the
	/// game and moved off that monitor since, by hand or by the game moving
	/// -- and then the keyboard is passed: to the game from a lobby that has
	/// it, else to the lobby.
	fn beside(&mut self, focused: bool, out: &mut Vec<Effect>) {
		if self.is_over() {
			out.push(Effect::LeaveOverlay);
		}
		let lowered = self.window == Standing::Lowered;
		self.window = Standing::Ordinary;
		if lowered || !focused {
			out.push(Effect::Show);
			out.push(Effect::FocusSelf);
		} else if let Some(Some(pid)) = self.engine {
			out.push(Effect::FocusEngine(pid));
		}
	}

	/// Over the game the lobby is fitted to the game's monitor on every
	/// showing, not only the first: dragged partly off it, it goes back over
	/// the game. The surface keeps the first shape it was given to go back to.
	/// A window not over it yet is shown through a veil, so the dress is on
	/// before anyone sees it.
	fn over(&mut self, out: &mut Vec<Effect>) {
		let seen = self.is_over();
		self.window = Standing::Over;
		out.push(Effect::EnterOverlay);
		if !seen {
			out.push(Effect::ShowVeiled);
		}
		out.push(Effect::FocusSelf);
	}

	/// Back to the game, with the lobby waiting directly behind it. Getting
	/// out of the way is not the same as handing the game the keyboard, so
	/// that is a step of its own, and with nobody named to give it to the
	/// window does not go behind the game either: it would keep the keyboard
	/// where nobody can see it.
	fn lower(&mut self, out: &mut Vec<Effect>) {
		self.window = Standing::Lowered;
		let game = self
			.engine
			.flatten()
			.filter(|_| self.settings.return_focus_to_game);
		out.push(Effect::Lower { behind: game });
		if let Some(pid) = game {
			out.push(Effect::FocusEngine(pid));
		}
	}

	/// Applies the consequences of whatever just changed.
	///
	/// Split from [`Self::step`] only because the borrow checker prefers it;
	/// the rule is one sentence: hold the hotkey exactly while a game is
	/// running and the feature is on, and never leave the window borderless
	/// and on top once there is no game under it.
	fn reconcile(&mut self) -> Vec<Effect> {
		let mut out = Vec::new();
		let wanted = self.armed().then(|| self.settings.hotkey.clone());

		if self.registered != wanted {
			if self.registered.is_some() {
				out.push(Effect::UnregisterHotkey);
			}
			if let Some(accel) = &wanted {
				out.push(Effect::RegisterHotkey(accel.clone()));
			}
			self.registered = wanted;
		}

		// Nothing to sit over any more. Whether we were showing over the game
		// or lowered out of its way, the window has to come back as an ordinary
		// one — otherwise a crashed game leaves a decorationless, always-on-top
		// window, or none at all, with no hotkey left to summon it.
		if !self.armed() {
			match self.window {
				Standing::Over => out.push(Effect::LeaveOverlay),
				Standing::Lowered => out.push(Effect::Show),
				Standing::Ordinary => {}
			}
			self.window = Standing::Ordinary;
		}
		out
	}
}

/// The reducer as the executor drives it.
///
/// Wraps [`Overlay::step`] so the reconciliation always runs, which is the
/// part it would be easy to forget at one call site out of three.
pub fn step(overlay: &mut Overlay, input: Input) -> Vec<Effect> {
	let mut out = overlay.step(input);
	out.extend(overlay.reconcile());
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	fn armed() -> Overlay {
		let mut overlay = Overlay::new(OverlaySettings::default());
		let effects = step(&mut overlay, Input::EngineRunning { pid: Some(42) });
		assert_eq!(effects, vec![Effect::RegisterHotkey("Alt+Shift+L".into())]);
		overlay
	}

	#[test]
	fn a_game_is_loading_until_it_says_it_has_the_keyboard() {
		let mut overlay = armed();
		assert_eq!(overlay.loading_game(), Some(42));

		assert_eq!(step(&mut overlay, Input::GameLoaded), vec![]);
		assert_eq!(overlay.loading_game(), None);

		// The same news again -- a page reloading is sent a snapshot -- is not
		// a new game.
		step(&mut overlay, Input::EngineRunning { pid: Some(42) });
		assert_eq!(overlay.loading_game(), None);
	}

	#[test]
	fn the_next_game_loads_all_over_again() {
		let mut overlay = armed();
		step(&mut overlay, Input::GameLoaded);
		step(&mut overlay, Input::EngineExited);
		assert_eq!(overlay.loading_game(), None, "no game, nothing loading");

		step(&mut overlay, Input::EngineRunning { pid: Some(43) });
		assert_eq!(overlay.loading_game(), Some(43));
	}

	/// Taken over from a lobby before this one: its Lua holds that lobby's
	/// tokens, so it will never say, and it is most likely deep into a game.
	#[test]
	fn a_game_taken_over_is_not_loading_whichever_news_arrives_first() {
		let mut told_first = Overlay::new(OverlaySettings::default());
		step(&mut told_first, Input::TakenOver(42));
		step(&mut told_first, Input::EngineRunning { pid: Some(42) });
		assert_eq!(told_first.loading_game(), None);

		let mut seen_first = armed();
		step(&mut seen_first, Input::TakenOver(42));
		assert_eq!(seen_first.loading_game(), None);
	}

	#[test]
	fn nothing_is_loading_for_a_lobby_with_nothing_to_do_about_it() {
		let idle = Overlay::new(OverlaySettings::default());
		assert_eq!(idle.loading_game(), None);

		let mut no_escape = armed();
		step(
			&mut no_escape,
			Input::Settings(OverlaySettings {
				in_game_escape: false,
				..OverlaySettings::default()
			}),
		);
		assert_eq!(no_escape.loading_game(), None);

		let mut overlay_off = armed();
		step(
			&mut overlay_off,
			Input::Settings(OverlaySettings {
				enabled: false,
				..OverlaySettings::default()
			}),
		);
		assert_eq!(overlay_off.loading_game(), None);

		// With no process to name there is no telling whether it is in front.
		let mut no_pid = Overlay::new(OverlaySettings::default());
		step(&mut no_pid, Input::EngineRunning { pid: None });
		assert_eq!(no_pid.loading_game(), None);
	}

	#[test]
	fn word_of_a_game_with_none_running_is_forgotten() {
		let mut overlay = Overlay::new(OverlaySettings::default());
		step(&mut overlay, Input::GameLoaded);
		step(&mut overlay, Input::EngineRunning { pid: Some(42) });
		assert_eq!(overlay.loading_game(), Some(42));
	}

	#[test]
	fn a_game_starting_takes_the_key_but_does_not_take_the_screen() {
		let mut overlay = Overlay::new(OverlaySettings::default());
		let effects = step(&mut overlay, Input::EngineRunning { pid: Some(42) });
		// Launching a game is a request to play it. The overlay is offered,
		// not imposed.
		assert_eq!(effects, vec![Effect::RegisterHotkey("Alt+Shift+L".into())]);
		assert!(!overlay.is_over());
	}

	#[test]
	fn the_hotkey_raises_the_lobby_and_puts_it_back() {
		let mut overlay = armed();

		assert_eq!(
			step(&mut overlay, Input::Hotkey(Placement::Over)),
			vec![Effect::EnterOverlay, Effect::ShowVeiled, Effect::FocusSelf]
		);
		assert!(overlay.is_over());
		assert_eq!(step(&mut overlay, Input::Painted), vec![Effect::Reveal]);

		assert_eq!(
			step(&mut overlay, Input::Hotkey(Placement::Over)),
			vec![Effect::Lower { behind: Some(42) }, Effect::FocusEngine(42)]
		);
		assert!(!overlay.is_over());

		// Lowered in its ordinary shape, so fitted over the game again and
		// shown through the veil, as on the first press.
		assert_eq!(
			step(&mut overlay, Input::Hotkey(Placement::Over)),
			vec![Effect::EnterOverlay, Effect::ShowVeiled, Effect::FocusSelf]
		);
		assert!(overlay.is_over());
	}

	#[test]
	fn raising_never_lowers_and_fits_a_window_already_up_over_the_game_again() {
		let mut overlay = armed();
		assert_eq!(
			step(&mut overlay, Input::Raise(Placement::Over)),
			vec![Effect::EnterOverlay, Effect::ShowVeiled, Effect::FocusSelf]
		);
		// Up already, and dragged partly off the game by hand: back over it,
		// with no veil over a window already on screen.
		assert_eq!(
			step(&mut overlay, Input::Raise(Placement::Over)),
			vec![Effect::EnterOverlay, Effect::FocusSelf]
		);
		assert!(overlay.is_over());
	}

	#[test]
	fn a_paint_report_reveals_only_a_window_still_over_the_game() {
		let mut overlay = armed();
		// Never raised: nothing to reveal.
		assert_eq!(step(&mut overlay, Input::Painted), vec![]);

		step(&mut overlay, Input::Hotkey(Placement::Over));
		step(&mut overlay, Input::Hotkey(Placement::Over));
		// Raised and hidden again before the page got two frames in: the
		// report is about a window nobody can see, and revealing it would
		// undo nothing but must not be mistaken for a show either.
		assert_eq!(step(&mut overlay, Input::Painted), vec![]);
	}

	#[test]
	fn shutting_down_over_a_game_gives_the_shape_back() {
		let mut overlay = armed();
		step(&mut overlay, Input::Hotkey(Placement::Over));
		assert!(overlay.is_over());

		// What the window looks like at exit is what gets remembered for the
		// next start, so it must not be the overlay's shape.
		assert_eq!(
			step(&mut overlay, Input::Shutdown),
			vec![Effect::UnregisterHotkey, Effect::LeaveOverlay]
		);
	}

	#[test]
	fn shutting_down_while_lowered_does_not_bring_the_window_forward() {
		let mut overlay = armed();
		step(&mut overlay, Input::Hotkey(Placement::Over));
		step(&mut overlay, Input::Hotkey(Placement::Over));

		// In its ordinary shape already, and a window brought forward for the
		// last few milliseconds is a flash nobody asked for.
		assert_eq!(
			step(&mut overlay, Input::Shutdown),
			vec![Effect::UnregisterHotkey]
		);
	}

	#[test]
	fn a_game_ending_while_the_overlay_is_up_gives_an_ordinary_window_back() {
		let mut overlay = armed();
		step(&mut overlay, Input::Hotkey(Placement::Over));

		assert_eq!(
			step(&mut overlay, Input::EngineExited),
			vec![Effect::UnregisterHotkey, Effect::LeaveOverlay]
		);
		assert!(!overlay.is_over());
	}

	#[test]
	fn a_game_ending_while_the_lobby_is_lowered_brings_it_back() {
		let mut overlay = armed();
		step(&mut overlay, Input::Hotkey(Placement::Over));
		step(&mut overlay, Input::Hotkey(Placement::Over));
		assert!(!overlay.is_over());

		// Minimized, or hidden where a platform cannot order windows: without
		// the Show there is no window and no hotkey left to summon one with.
		// Its shape it has already.
		assert_eq!(
			step(&mut overlay, Input::EngineExited),
			vec![Effect::UnregisterHotkey, Effect::Show]
		);

		// Over and done with: the next game starts from an ordinary window.
		step(&mut overlay, Input::EngineRunning { pid: Some(7) });
		assert_eq!(
			step(&mut overlay, Input::EngineExited),
			vec![Effect::UnregisterHotkey]
		);
	}

	#[test]
	fn turning_it_off_while_lowered_brings_the_window_back() {
		let mut overlay = armed();
		step(&mut overlay, Input::Hotkey(Placement::Over));
		step(&mut overlay, Input::Hotkey(Placement::Over));

		let effects = step(
			&mut overlay,
			Input::Settings(OverlaySettings {
				enabled: false,
				..OverlaySettings::default()
			}),
		);
		assert_eq!(effects, vec![Effect::UnregisterHotkey, Effect::Show]);
	}

	#[test]
	fn raising_a_lowered_lobby_goes_through_the_veil_every_time() {
		let mut overlay = armed();
		let raised = vec![Effect::EnterOverlay, Effect::ShowVeiled, Effect::FocusSelf];
		for _ in 0..3 {
			assert_eq!(step(&mut overlay, Input::Raise(Placement::Over)), raised);
			assert_eq!(step(&mut overlay, Input::Painted), vec![Effect::Reveal]);
			step(&mut overlay, Input::Hotkey(Placement::Over));
			assert!(!overlay.is_over());
		}
	}

	#[test]
	fn the_key_is_held_only_while_a_game_is() {
		let mut overlay = armed();
		assert_eq!(
			step(&mut overlay, Input::EngineExited),
			vec![Effect::UnregisterHotkey]
		);
		// A lobby sitting idle should not own a system-wide key.
		assert_eq!(step(&mut overlay, Input::Hotkey(Placement::Over)), vec![]);
	}

	#[test]
	fn arming_again_while_already_armed_changes_nothing() {
		let mut overlay = armed();
		assert_eq!(
			step(&mut overlay, Input::EngineRunning { pid: Some(42) }),
			vec![]
		);
		assert_eq!(
			step(&mut overlay, Input::EngineRunning { pid: Some(7) }),
			vec![]
		);
	}

	#[test]
	fn changing_the_hotkey_mid_game_re_registers_exactly_once() {
		let mut overlay = armed();
		let effects = step(
			&mut overlay,
			Input::Settings(OverlaySettings {
				hotkey: "Ctrl+Alt+O".into(),
				..OverlaySettings::default()
			}),
		);
		assert_eq!(
			effects,
			vec![
				Effect::UnregisterHotkey,
				Effect::RegisterHotkey("Ctrl+Alt+O".into())
			]
		);
		// Saying the same thing again is not a change.
		assert_eq!(
			step(
				&mut overlay,
				Input::Settings(OverlaySettings {
					hotkey: "Ctrl+Alt+O".into(),
					..OverlaySettings::default()
				})
			),
			vec![]
		);
	}

	#[test]
	fn turning_it_off_mid_overlay_hands_the_window_back() {
		let mut overlay = armed();
		step(&mut overlay, Input::Hotkey(Placement::Over));

		let effects = step(
			&mut overlay,
			Input::Settings(OverlaySettings {
				enabled: false,
				..OverlaySettings::default()
			}),
		);
		assert_eq!(
			effects,
			vec![Effect::UnregisterHotkey, Effect::LeaveOverlay]
		);
	}

	#[test]
	fn turning_it_on_mid_game_arms_without_a_relaunch() {
		let mut overlay = Overlay::new(OverlaySettings {
			enabled: false,
			..OverlaySettings::default()
		});
		assert_eq!(
			step(&mut overlay, Input::EngineRunning { pid: Some(9) }),
			vec![]
		);

		assert_eq!(
			step(&mut overlay, Input::Settings(OverlaySettings::default())),
			vec![Effect::RegisterHotkey("Alt+Shift+L".into())]
		);
	}

	#[test]
	fn a_game_whose_pid_we_lost_still_lowers_rather_than_refusing_to() {
		let mut overlay = Overlay::new(OverlaySettings::default());
		step(&mut overlay, Input::EngineRunning { pid: None });
		step(&mut overlay, Input::Hotkey(Placement::Over));

		// No pid means nothing to go behind or to focus, but getting out of
		// the way is still the more useful half of the answer.
		assert_eq!(
			step(&mut overlay, Input::Hotkey(Placement::Over)),
			vec![Effect::Lower { behind: None }]
		);
	}

	#[test]
	fn a_lobby_on_its_own_monitor_keeps_its_shape_and_only_passes_the_keyboard() {
		let mut overlay = armed();
		let beside = |focused| Input::Hotkey(Placement::Beside { focused });

		// From the game: the lobby comes forward as the window it already is.
		assert_eq!(
			step(&mut overlay, beside(false)),
			vec![Effect::Show, Effect::FocusSelf]
		);
		assert!(!overlay.is_over());
		// And nothing is waiting to be revealed.
		assert_eq!(step(&mut overlay, Input::Painted), vec![]);

		// From the lobby: back to the game, with the lobby left where it is.
		assert_eq!(
			step(&mut overlay, beside(true)),
			vec![Effect::FocusEngine(42)]
		);
		assert!(!overlay.is_over());

		// The game ending finds nothing to give back.
		assert_eq!(
			step(&mut overlay, Input::EngineExited),
			vec![Effect::UnregisterHotkey]
		);
	}

	#[test]
	fn moved_onto_another_monitor_the_overlay_becomes_an_ordinary_window_there() {
		let mut overlay = armed();
		step(&mut overlay, Input::Hotkey(Placement::Over));
		assert!(overlay.is_over());

		// Win+Shift+Arrow took the overlay to a monitor with no game under
		// it. The press gives the window its shape back where it is, and as
		// the lobby had the keyboard, hands it to the game.
		assert_eq!(
			step(
				&mut overlay,
				Input::Hotkey(Placement::Beside { focused: true })
			),
			vec![Effect::LeaveOverlay, Effect::FocusEngine(42)]
		);
		assert!(!overlay.is_over());
		// An ordinary window from here on: the next press only raises it.
		assert_eq!(
			step(
				&mut overlay,
				Input::Hotkey(Placement::Beside { focused: false })
			),
			vec![Effect::Show, Effect::FocusSelf]
		);
		// And raising from the game never hands the keyboard back.
		assert_eq!(
			step(
				&mut overlay,
				Input::Raise(Placement::Beside { focused: true })
			),
			vec![Effect::Show, Effect::FocusSelf]
		);
	}

	#[test]
	fn lowered_behind_a_game_that_moved_away_the_lobby_comes_forward_as_it_is() {
		let mut overlay = armed();
		step(&mut overlay, Input::Hotkey(Placement::Over));
		step(&mut overlay, Input::Hotkey(Placement::Over));
		assert!(!overlay.is_over());

		assert_eq!(
			step(
				&mut overlay,
				Input::Hotkey(Placement::Beside { focused: false })
			),
			vec![Effect::Show, Effect::FocusSelf]
		);
		// Nothing left to give back when the game ends.
		assert_eq!(
			step(&mut overlay, Input::EngineExited),
			vec![Effect::UnregisterHotkey]
		);
	}

	#[test]
	fn a_game_without_a_pid_cannot_be_handed_the_keyboard_from_beside_it() {
		let mut overlay = Overlay::new(OverlaySettings::default());
		step(&mut overlay, Input::EngineRunning { pid: None });
		assert_eq!(
			step(
				&mut overlay,
				Input::Hotkey(Placement::Beside { focused: true })
			),
			vec![]
		);
	}

	#[test]
	fn someone_who_would_rather_keep_focus_is_not_given_the_game() {
		let mut overlay = Overlay::new(OverlaySettings {
			return_focus_to_game: false,
			..OverlaySettings::default()
		});
		step(&mut overlay, Input::EngineRunning { pid: Some(42) });
		step(&mut overlay, Input::Hotkey(Placement::Over));
		// Not behind the game, which would leave the keyboard in a window
		// nobody can see: out of the way, for the OS to say who is next.
		assert_eq!(
			step(&mut overlay, Input::Hotkey(Placement::Over)),
			vec![Effect::Lower { behind: None }]
		);
	}
}
