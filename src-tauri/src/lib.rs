//! Scaricatore Di Porto — Tauri builder, plugins and process lifetime
//! (spec §3 `main.rs / lib.rs`).

pub mod commands;
pub mod error;
pub mod services;
pub mod state;

use std::sync::Arc;

use tauri::Emitter;
use tauri::Manager;

use crate::services::notify::IdleNotifier as OsIdleNotifier;
use crate::services::process;
use crate::services::queue::engine::{EngineConfig, QueueEngine};
use crate::services::queue::events::EventSink;
use crate::state::AppState;

/// Exit hook: cancel every running child process before the process goes away
/// (spec §4.2 "On app exit cancel all running items").
///
/// The queue's tokens go first, so the items record themselves as cancelled and
/// their temp directories are cleaned; `kill_all_running` then walks the
/// registry of live process trees. On Windows the Job Objects carry
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so even a crash of the app kills the
/// trees; on Unix this explicit pass is what leaves no orphan behind.
///
/// The order matters and is now enforced by the engine as well: `shutdown()`
/// stops the scheduler from starting anything, so nothing can be spawned
/// between this pass and the end of the process (D96).
fn on_exit(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<AppState>()
        && let Some(queue) = state.queue()
    {
        queue.shutdown();
    }
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
        // Spec §7.7: the notification when the queue becomes idle.
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let state = AppState::load(&handle)?;
            // Spec §5.2.1: copy the bundled yt-dlp into the app data dir on
            // first launch, so the copy is the thing that runs and the thing
            // `-U` can replace. Best effort: without it the app still works on
            // the bundled binary.
            commands::tools::bootstrap_first_run(state.tools());

            let settings = state.settings();
            let queue = build_queue(&handle, &state);
            state.set_queue(queue.clone());
            queue.start();

            // Spec §4.10: remove the stale `.sdp-tmp-*` directories, but only
            // those belonging to no item of the queue — an interrupted item
            // keeps its `.part` files and resumes from them (spec §7.4).
            let mut dirs = queue.live_output_dirs();
            if let Some(dir) = settings
                .default_output_dir
                .clone()
                .or_else(|| crate::services::paths::default_output_dir(&handle))
            {
                dirs.push(dir);
            }
            dirs.sort();
            dirs.dedup();
            let owned = queue.owned_temp_names();
            let mut removed = 0;
            for dir in dirs {
                removed += crate::services::job::cleanup_stale_temp_dirs(&dir, &owned);
            }
            if removed > 0 {
                eprintln!(
                    "scaricatore-di-porto: removed {removed} stale temp director{}",
                    if removed == 1 { "y" } else { "ies" },
                );
            }

            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::settings_get,
            commands::settings::settings_save,
            commands::settings::archive_status,
            commands::settings::archive_clear,
            commands::inspect::inspect_url,
            commands::inspect::inspect_urls,
            commands::inspect::inspect_cancel,
            commands::download::download_start,
            commands::download::download_cancel,
            commands::download::download_dismiss_deno_offer,
            commands::queue::queue_list,
            commands::queue::queue_add,
            commands::queue::queue_cancel,
            commands::queue::queue_pause,
            commands::queue::queue_resume,
            commands::queue::queue_retry,
            commands::queue::queue_remove,
            commands::queue::queue_pause_all,
            commands::queue::queue_resume_all,
            commands::queue::queue_retry_failed,
            commands::queue::queue_reorder,
            commands::queue::queue_move_to_top,
            commands::queue::queue_clear_finished,
            commands::queue::queue_upsert_group,
            commands::queue::queue_pause_group,
            commands::queue::queue_resume_group,
            commands::queue::queue_retry_group_failed,
            commands::queue::queue_remove_group,
            commands::queue::queue_lanes,
            commands::queue::read_links_file,
            commands::queue::app_set_language,
            commands::tools::tools_status,
            commands::tools::tools_test,
            commands::tools::tools_set_path,
            commands::tools::tools_set_ytdlp_channel,
            commands::tools::tools_update_ytdlp,
            commands::tools::tools_install,
            commands::tools::tools_cancel_install,
            commands::tools::tools_self_test,
            commands::tools::tools_complete_setup,
        ])
        .build(tauri::generate_context!());

    match builder {
        Ok(app) => app.run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = event {
                on_exit(app);
            }
        }),
        Err(err) => {
            // Nothing sensible left to do: the app cannot open a window.
            eprintln!("scaricatore-di-porto: cannot start: {err}");
            std::process::exit(1);
        }
    }
}

/// Open `queue.json` and wire the engine to the app (spec §7.4).
fn build_queue(handle: &tauri::AppHandle, state: &AppState) -> QueueEngine {
    let settings = state.settings();
    let path = crate::services::queue::store::queue_file(handle).unwrap_or_else(|err| {
        eprintln!("scaricatore-di-porto: no app data dir for the queue: {err}");
        std::env::temp_dir().join(crate::services::queue::store::QUEUE_FILE)
    });

    let events = handle.clone();
    let sink: EventSink = Arc::new(move |update| {
        let _ = events.emit("queue:update", update);
    });

    // `LanguageHolder` is a cloneable handle over one shared slot, so the
    // notifier and the `app_set_language` command really do share the same
    // value: the webview resolves the language properly (spec §11.9) and pushes
    // it, and the notification follows.
    let notifier: Arc<dyn crate::services::queue::engine::IdleNotifier> =
        Arc::new(OsIdleNotifier::new(handle.clone(), state.ui_lang().clone()));

    QueueEngine::open(EngineConfig {
        path,
        provider: commands::queue::provider(handle),
        sink,
        lanes: settings.lane_config(),
        notify_on_idle: settings.notify_on_idle,
        notifier: Some(notifier),
    })
}
