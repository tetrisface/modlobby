//! The Tauri adapter: wires the webview to `lobby-runtime` and `settings`.
//! Nothing in here knows the protocol; commands translate calls, the channel
//! transport forwards `UiMessage`s, settings changes are emitted as events.

mod commands;
mod logging;
mod state;
mod transport;

use tauri::{Emitter, Manager};

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
        .setup(move |tauri_app| {
            let mut watch = app.settings.watch()?;
            let handle = tauri_app.handle().clone();
            let client = app.client.clone();
            let data_dir = app.settings.get().paths.data_dir;
            let in_public = app.settings.get().play.in_public_rooms;
            tauri::async_runtime::spawn(async move {
                // The content check needs to know where BAR keeps its files,
                // both now and whenever the setting changes.
                let _ = client.set_data_dir(data_dir).await;
                let _ = client.allow_public_seat(in_public).await;
                while let Some(event) = watch.events.recv().await {
                    if let settings::SettingsEvent::Changed(settings) = &event {
                        let _ = client.set_data_dir(settings.paths.data_dir.clone()).await;
                        let _ = client
                            .allow_public_seat(settings.play.in_public_rooms)
                            .await;
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
