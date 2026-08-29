//! The Tauri adapter: wires the webview to `lobby-runtime` and `settings`.
//! Nothing in here knows the protocol; commands translate calls, the channel
//! transport forwards `UiMessage`s, settings changes are emitted as events.

mod commands;
mod state;
mod transport;

use tauri::{Emitter, Manager};
use tracing_subscriber::EnvFilter;

pub fn run() {
    let app = state::App::open().unwrap_or_else(|err| panic!("starting modlobby: {err}"));
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(app.settings.get().logging.filter)),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(move |tauri_app| {
            let mut watch = app.settings.watch()?;
            let handle = tauri_app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(event) = watch.events.recv().await {
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
            commands::join_battle,
            commands::leave_battle,
            commands::launch,
            commands::say_battle,
            commands::vote,
            commands::get_settings,
            commands::update_settings,
            commands::has_password,
            commands::set_password,
            commands::clear_password,
            commands::open_settings_file,
            commands::open_data_dir,
        ])
        .run(tauri::generate_context!())
        .expect("running modlobby");
}
