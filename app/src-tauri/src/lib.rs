//! The Tauri adapter: wires the webview to `lobby-runtime` and `settings`.
//! Nothing in here knows the protocol; commands translate calls, the channel
//! transport forwards `UiMessage`s, settings changes are emitted as events.

mod boxes;
mod commands;
mod engine;
mod flash;
mod ingame;
mod logging;
mod overlay;
mod presets;
mod screen;
mod state;
mod thumbs;
mod transport;
mod update;
mod win;

use tauri::{Emitter, Manager};

/// The overlay's slice of the settings file.
fn overlay_settings(settings: &settings::Settings) -> overlay::OverlaySettings {
    overlay::OverlaySettings {
        enabled: settings.overlay.enabled,
        hotkey: settings.overlay.hotkey.clone(),
        return_focus_to_game: settings.overlay.return_focus_to_game,
    }
}

/// Where a borderless copy of the engine config may be kept, or `None` to
/// launch against the user's settings exactly as they are.
///
/// Switched off with the overlay, because that is the only thing the copy is
/// for: someone who is not using the overlay should get the game they
/// configured, and nothing of ours should appear on disk.
/// The setting's `0` is the runtime's "never".
fn idle_timeout(settings: &settings::Settings) -> Option<std::time::Duration> {
    match settings.connection.idle_disconnect_minutes {
        0 => None,
        minutes => Some(std::time::Duration::from_secs(u64::from(minutes) * 60)),
    }
}

fn overlay_config_dir(settings: &settings::Settings) -> Option<std::path::PathBuf> {
    settings
        .overlay
        .enabled
        .then(|| settings::config_dir().join("engine"))
}

/// The live in-game socket, or nothing if it could not be bound.
///
/// Dropping it takes the widget back out of the user's data directory, which
/// is why it is held rather than leaked and why the exit handler reaches for
/// it explicitly.
pub(crate) type InGameHandle = std::sync::Arc<std::sync::Mutex<Option<ingame::InGame>>>;

/// What the in-game widget is allowed to do, and who decides.
///
/// The overlay controller answers "is this our game", so a game launched from
/// another lobby while modlobby is open is told `no` and keeps its own Escape.
struct InGameActions {
    overlay: std::sync::Arc<overlay::Controller>,
    client: lobby_runtime::Client,
}

impl ingame::Actions for InGameActions {
    fn raise(&self) -> bool {
        self.overlay.raise()
    }

    fn quit_game(&self) -> bool {
        if !self.overlay.armed_for_game() {
            return false;
        }
        // The widget is waiting on a reply, so this cannot block on the
        // runtime; the answer is "yes, this is ours and the stop is on its way".
        let client = self.client.clone();
        tauri::async_runtime::spawn(async move {
            let _ = client.stop_engine().await;
        });
        true
    }
}

/// When `run` began, for the `startup:` milestones in the log.
static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Milliseconds since the process began starting up.
pub(crate) fn since_start() -> u128 {
    STARTED
        .get()
        .map(|started| started.elapsed().as_millis())
        .unwrap_or(0)
}

pub fn run() {
    let started = *STARTED.get_or_init(std::time::Instant::now);
    // Before anything reads the environment: the config dir and the update
    // switch both come from it. A dev run finds the repo's `.env` by walking
    // up from the working directory; an installed app has none and skips it.
    let dotenv = dotenvy::dotenv();
    let app = state::App::open().unwrap_or_else(|err| panic!("starting modlobby: {err}"));
    let opened = started.elapsed().as_millis();
    // Held for the life of the process: dropping it stops the file writer.
    let _logging = logging::start(app.settings.dir(), &app.settings.get().logging.filter);
    tracing::debug!(ms = opened, "startup: app opened");
    tracing::debug!(ms = since_start(), "startup: logging up");
    match dotenv {
        Ok(path) => tracing::info!(path = %path.display(), "loaded .env"),
        Err(err) if err.not_found() => {}
        Err(err) => tracing::warn!(%err, "ignoring .env"),
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Where the window was and how big it was, kept between runs — a lobby
        // is a window you arrange once and then live with. Fullscreen is
        // deliberately not part of it: the lobby's fullscreen is its own
        // (`screen.rs`), and an OS-level fullscreen restored at startup is
        // exactly the stuck state that made the toggle look dead.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        - tauri_plugin_window_state::StateFlags::FULLSCREEN,
                )
                .build(),
        )
        .plugin(
            // The handler fires on press and release; only one of those is an
            // instruction.
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    use tauri_plugin_global_shortcut::ShortcutState;
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if let Some(overlay) = app.try_state::<std::sync::Arc<overlay::Controller>>() {
                        overlay.hotkey();
                    }
                })
                .build(),
        )
        .setup(move |tauri_app| {
            // The overlay needs a window, so it is built here rather than in
            // `App::open`, and Tauri holds it beside the app state.
            let controller = std::sync::Arc::new(overlay::Controller::new(
                overlay_settings(&app.settings.get()),
                std::sync::Arc::new(overlay::surface::TauriSurface::new(
                    overlay::surface::main_window(tauri_app.handle())
                        .ok_or("the main window is missing")?,
                )),
                std::sync::Arc::new(overlay::foreground::Windows),
                std::sync::Arc::new(overlay::hotkey::GlobalHotkey::new(
                    tauri_app.handle().clone(),
                )),
            ));
            tauri_app.manage(controller.clone());
            tauri_app.manage(screen::Screen::default());

            // Escape inside a game. The listener is bound before anything is
            // written, so the widget always names a port that answers.
            let actions = std::sync::Arc::new(InGameActions {
                overlay: controller.clone(),
                client: app.client.clone(),
            });
            // The widget goes in the directory we write; the engine reads
            // `LuaUI/Widgets` from every data directory it is given.
            let widget_dir = lobby_runtime::launch::data_dirs(app.settings.get().paths.data_dir)
                .map(|dirs| dirs.write);
            let want_widget = app.settings.get().overlay.in_game_escape;
            // Held by the app so it lives as long as the process, and so the
            // exit handler can drop it — dropping is what removes the widget.
            let held: InGameHandle = std::sync::Arc::new(std::sync::Mutex::new(None));
            tauri_app.manage(held.clone());
            tauri::async_runtime::spawn(async move {
                match ingame::InGame::start(actions).await {
                    Ok(mut ingame) => {
                        if let (true, Some(dir)) = (want_widget, widget_dir.as_deref()) {
                            match ingame.install(dir) {
                                Ok(path) => tracing::info!(
                                    path = %path.display(),
                                    port = ingame.port,
                                    "in-game Escape widget installed"
                                ),
                                Err(err) => tracing::warn!(%err, "could not install the widget"),
                            }
                        }
                        *held.lock().expect("in-game") = Some(ingame);
                    }
                    Err(err) => tracing::warn!(%err, "no in-game control socket"),
                }
            });

            let mut watch = app.settings.watch()?;
            let handle = tauri_app.handle().clone();
            let client = app.client.clone();
            let data_dir = app.settings.get().paths.data_dir;
            let in_public = app.settings.get().play.in_public_rooms;
            let auto_launch = app.settings.get().play.auto_launch;
            let auto_download = app.settings.get().play.auto_download;
            let engine_config = overlay_config_dir(&app.settings.get());
            let idle = idle_timeout(&app.settings.get());
            tauri::async_runtime::spawn(async move {
                // The content check needs to know where BAR keeps its files,
                // both now and whenever the setting changes.
                let _ = client.set_data_dir(data_dir).await;
                let _ = client.allow_public_seat(in_public).await;
                let _ = client.set_auto_launch(auto_launch).await;
                let _ = client.set_auto_download(auto_download).await;
                let _ = client.set_overlay_config_dir(engine_config).await;
                let _ = client.set_idle_timeout(idle).await;
                while let Some(event) = watch.recv().await {
                    if let settings::SettingsEvent::Changed(settings) = &event {
                        let _ = client.set_data_dir(settings.paths.data_dir.clone()).await;
                        let _ = client
                            .allow_public_seat(settings.play.in_public_rooms)
                            .await;
                        let _ = client.set_auto_launch(settings.play.auto_launch).await;
                        let _ = client.set_auto_download(settings.play.auto_download).await;
                        let _ = client
                            .set_overlay_config_dir(overlay_config_dir(settings))
                            .await;
                        let _ = client.set_idle_timeout(idle_timeout(settings)).await;
                        controller.settings_changed(overlay_settings(settings));
                    }
                    let _ = handle.emit("settings", &event);
                }
            });
            let check_updates = app.settings.get().updates.automatic && update::enabled();
            tauri_app.manage(update::Staged::default());
            tauri_app.manage(app);
            // A look, not a download: one small request when it is due, and
            // the corner of the nav says what it found.
            if check_updates {
                tauri::async_runtime::spawn(update::daily(tauri_app.handle().clone()));
            }
            tracing::debug!(ms = since_start(), "startup: setup done");
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol(thumbs::SCHEME, thumbs::serve)
        .invoke_handler(tauri::generate_handler![
            commands::subscribe,
            commands::login,
            commands::logout,
            commands::reconnect,
            commands::register,
            commands::confirm_agreement,
            commands::login_wait,
            commands::log_message,
            commands::open_log_dir,
            commands::join_battle,
            commands::leave_battle,
            commands::remembered_battle,
            commands::forget_battle,
            commands::launch,
            commands::say_battle,
            commands::vote,
            commands::set_option,
            commands::join_channel,
            commands::leave_channel,
            commands::say_channel,
            commands::say_private,
            commands::list_channels,
            commands::download_missing,
            commands::map_index,
            commands::warm_map_pictures,
            commands::stop_download,
            commands::cancel_paste,
            commands::ring,
            commands::add_bot,
            commands::remove_bot,
            commands::set_away,
            commands::activity,
            commands::overlay_active,
            commands::overlay_toggle,
            commands::stop_game,
            commands::quit_all,
            commands::shutdown,
            screen::is_fullscreen,
            screen::toggle_fullscreen,
            boxes::start_boxes,
            boxes::decode_boxes,
            engine::download_engine,
            update::app_version,
            update::check_update,
            update::install_update,
            commands::flash_engine,
            commands::remember_played,
            commands::game_modoptions,
            presets::pve_score,
            presets::list_presets,
            presets::chobby_presets_path,
            presets::save_preset,
            presets::preset_from_replay,
            presets::delete_preset,
            presets::rename_preset,
            presets::plan_preset,
            presets::apply_preset,
            presets::import_presets,
            presets::export_presets,
            commands::request_game_status,
            commands::remember_channels,
            commands::skirmish_options,
            commands::start_skirmish,
            commands::list_replays,
            commands::play_replay,
            commands::refresh_friends,
            commands::friend_action,
            commands::get_settings,
            commands::update_settings,
            commands::has_password,
            commands::set_password,
            commands::clear_password,
            commands::open_settings_file,
            commands::open_data_dir,
            commands::open_url,
            commands::take_seat,
            commands::set_ready,
            commands::set_side,
            commands::release_seat,
            commands::request_private_host,
            commands::host_public,
            commands::tweak_decode,
            commands::tweak_format,
            commands::tweak_prepare,
            commands::tweak_send,
            commands::tweak_clear,
            commands::tweak_diff,
            commands::tweak_check,
            commands::game_unit_names,
            commands::engine_def_tags,
            commands::tweak_diff_text,
            commands::list_drafts,
            commands::read_draft,
            commands::save_draft,
            commands::delete_draft,
        ])
        .build(tauri::generate_context!())
        .expect("building modlobby")
        .run(|handle, event| {
            // Leaving a widget behind that talks to a port nobody answers is
            // harmless — it stops consuming Escape — but tidying up is the
            // whole promise, so it is done on the way out.
            if matches!(event, tauri::RunEvent::Exit)
                && let Some(held) = handle.try_state::<InGameHandle>()
            {
                drop(held.lock().expect("in-game").take());
            }
        });
}
