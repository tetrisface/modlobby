//! The Tauri adapter: wires the webview to `lobby-runtime` and `settings`.
//! Nothing in here knows the protocol; commands translate calls, the channel
//! transport forwards `UiMessage`s, settings changes are emitted as events.

mod commands;
mod flash;
mod logging;
mod overlay;
mod presets;
mod state;
mod transport;
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
fn overlay_config_dir(settings: &settings::Settings) -> Option<std::path::PathBuf> {
    settings
        .overlay
        .enabled
        .then(|| settings::config_dir().join("engine"))
}

pub fn run() {
    let app = state::App::open().unwrap_or_else(|err| panic!("starting modlobby: {err}"));
    // Held for the life of the process: dropping it stops the file writer.
    let _logging = logging::start(app.settings.dir(), &app.settings.get().logging.filter);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        // Where the window was and how big it was, kept between runs — a lobby
        // is a window you arrange once and then live with.
        .plugin(tauri_plugin_window_state::Builder::default().build())
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

            let mut watch = app.settings.watch()?;
            let handle = tauri_app.handle().clone();
            let client = app.client.clone();
            let data_dir = app.settings.get().paths.data_dir;
            let in_public = app.settings.get().play.in_public_rooms;
            let auto_launch = app.settings.get().play.auto_launch;
            let engine_config = overlay_config_dir(&app.settings.get());
            tauri::async_runtime::spawn(async move {
                // The content check needs to know where BAR keeps its files,
                // both now and whenever the setting changes.
                let _ = client.set_data_dir(data_dir).await;
                let _ = client.allow_public_seat(in_public).await;
                let _ = client.set_auto_launch(auto_launch).await;
                let _ = client.set_overlay_config_dir(engine_config).await;
                while let Some(event) = watch.recv().await {
                    if let settings::SettingsEvent::Changed(settings) = &event {
                        let _ = client.set_data_dir(settings.paths.data_dir.clone()).await;
                        let _ = client
                            .allow_public_seat(settings.play.in_public_rooms)
                            .await;
                        let _ = client.set_auto_launch(settings.play.auto_launch).await;
                        let _ = client
                            .set_overlay_config_dir(overlay_config_dir(settings))
                            .await;
                        controller.settings_changed(overlay_settings(settings));
                    }
                    let _ = handle.emit("settings", &event);
                }
            });
            tauri_app.manage(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::subscribe,
            commands::login,
            commands::logout,
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
            commands::stop_download,
            commands::ring,
            commands::set_away,
            commands::overlay_active,
            commands::overlay_toggle,
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
            commands::list_drafts,
            commands::read_draft,
            commands::save_draft,
            commands::delete_draft,
        ])
        .run(tauri::generate_context!())
        .expect("running modlobby");
}
