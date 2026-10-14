//! Scaricatore Di Porto — Tauri builder, plugins and process lifetime
//! (spec §3 `main.rs / lib.rs`).

pub mod commands;
pub mod error;
pub mod services;
pub mod state;

use tauri::Manager;

use crate::services::process;
use crate::state::AppState;

/// Exit hook: cancel every running child process before the process goes away
/// (spec §4.2 "On app exit cancel all running items").
///
/// `kill_all_running` walks the registry of live process trees. On Windows the
/// Job Objects carry `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so even a crash of
/// the app kills the trees; on Unix this explicit pass is what leaves no
/// orphan behind.
fn on_exit() {
    process::kill_all_running();
}

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
            let state = AppState::load(&handle)?;
            // Spec §5.2.1: copy the bundled yt-dlp into the app data dir on
            // first launch, so the copy is the thing that runs and the thing
            // `-U` can replace. Best effort: without it the app still works on
            // the bundled binary.
            commands::tools::bootstrap_first_run(state.tools());
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::settings_get,
            commands::settings::settings_save,
            commands::tools::tools_status,
            commands::tools::tools_test,
            commands::tools::tools_set_path,
            commands::tools::tools_set_ytdlp_channel,
            commands::tools::tools_update_ytdlp,
            commands::tools::tools_install,
            commands::tools::tools_complete_setup,
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
