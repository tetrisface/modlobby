//! What turns decisions into window calls.
//!
//! The only place that knows all three seams. Everything it does comes from
//! [`super::state`], so the ordering questions — hide before focusing the
//! game, restore before showing after a crash — are settled in tests rather
//! than here.

use std::sync::{Arc, Mutex};

use lobby_ui::{Delta, EngineStatus, UiMessage};

use super::seams::{ForegroundControl, Hotkeys, WindowSurface};
use super::state::{Effect, Input, Overlay, OverlaySettings};

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
        self.drive(Input::Hotkey);
    }

    pub fn settings_changed(&self, settings: OverlaySettings) {
        self.drive(Input::Settings(settings));
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

    /// Raises the lobby if it is not already up. Unlike the hotkey this never
    /// lowers it: the in-game Escape is a one-way door out of the game.
    pub fn raise(&self) -> bool {
        if !self.armed_for_game() {
            return false;
        }
        if self.is_over() {
            return true;
        }
        self.hotkey();
        true
    }

    fn drive(&self, input: Input) {
        // The lock is released before anything touches a window: a window call
        // can re-enter through an event handler, and holding this across one
        // would deadlock the next hotkey press.
        let effects = {
            let mut overlay = self.overlay.lock().expect("overlay");
            super::state::step(&mut overlay, input)
        };
        for effect in effects {
            self.apply(effect);
        }
    }

    fn apply(&self, effect: Effect) {
        match effect {
            Effect::RegisterHotkey(accelerator) => self.hotkeys.register(&accelerator),
            Effect::UnregisterHotkey => self.hotkeys.unregister(),
            Effect::EnterOverlay => self.surface.set_overlay(true),
            Effect::LeaveOverlay => self.surface.set_overlay(false),
            Effect::Show => self.surface.show(),
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
            UiMessage::Deltas(deltas) => {
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

    use super::*;

    #[derive(Default)]
    struct Spy {
        over: AtomicBool,
        shown: AtomicBool,
        registered: AtomicBool,
    }

    impl WindowSurface for Spy {
        fn set_overlay(&self, over: bool) {
            self.over.store(over, Ordering::SeqCst);
        }
        fn show(&self) {
            self.shown.store(true, Ordering::SeqCst);
        }
        fn hide(&self) {
            self.shown.store(false, Ordering::SeqCst);
        }
        fn focus(&self) {}
    }

    impl ForegroundControl for Spy {
        fn focus(&self, _pid: u32) {}
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
        let spy = Arc::new(Spy::default());
        let controller = Controller::new(
            OverlaySettings::default(),
            spy.clone(),
            spy.clone(),
            spy.clone(),
        );
        (spy, controller)
    }

    #[test]
    fn a_running_engine_in_the_stream_arms_the_hotkey() {
        let (spy, controller) = controller();
        controller.observe(&UiMessage::Deltas(vec![Delta::Engine(
            EngineStatus::Running { pid: Some(7) },
        )]));
        assert!(spy.registered.load(Ordering::SeqCst));
        assert!(!spy.over.load(Ordering::SeqCst), "armed, not imposed");
    }

    #[test]
    fn the_engine_exiting_releases_the_key_and_restores_the_window() {
        let (spy, controller) = controller();
        controller.engine_running(Some(7));
        controller.hotkey();
        assert!(spy.over.load(Ordering::SeqCst));

        controller.observe(&UiMessage::Deltas(vec![Delta::Engine(
            EngineStatus::Exited { code: Some(0) },
        )]));
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
