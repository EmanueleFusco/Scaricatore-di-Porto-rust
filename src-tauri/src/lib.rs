//! Scaricatore Di Porto — Tauri builder, plugins and process lifetime
//! (spec §3 `main.rs / lib.rs`).

pub mod commands;
pub mod error;
pub mod services;
pub mod state;

use tauri::Manager;

use crate::state::AppState;

/// Exit hook: cancel every running child process before the process goes away
/// (spec §4.2 "On app exit cancel all running items").
///
/// M0-B adds `services/process.rs` and registers its cancel-all registry here;
/// until then there is nothing to cancel, so this is intentionally empty.
fn on_exit() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        // Must be the FIRST plugin: a second launch has to be intercepted
        // before any other plugin starts doing work (spec §3 single-instance).
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(AppState::load(&handle)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::settings_get,
            commands::settings::settings_save,
        ])
        .build(tauri::generate_context!());

    match builder {
        Ok(app) => app.run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = event {
                on_exit();
            }
        }),
        Err(err) => {
            // Nothing sensible left to do: the app cannot open a window.
            eprintln!("scaricatore-di-porto: cannot start: {err}");
            std::process::exit(1);
        }
    }
}
