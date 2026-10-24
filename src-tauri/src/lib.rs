//! Scaricatore Di Porto — Tauri builder, plugins and process lifetime
//! (spec §3 `main.rs / lib.rs`).

// Raising `rust-version` to 1.90 for tauri-plugin-updater switched on Clippy's
// let-chain rewrite for every nested `if let` in the tree. Those rewrites are
// unrelated to the update feature and would bury it in churn; the lint stays
// off until a dedicated cleanup.
#![allow(clippy::collapsible_if)]

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

/// WebKitGTK's default DMA-BUF renderer can die at startup on some Wayland
/// compositors (for example KDE + NVIDIA) with
/// `Gdk-Message: Error 71 (Errore di protocollo) dispatching to Wayland display`.
/// Spec §13.3 prescribes disabling it as the fallback. This is applied before the
/// webview is created, and an explicit `WEBKIT_DISABLE_DMABUF_RENDERER` value is
/// always respected.
#[cfg(any(target_os = "linux", test))]
fn is_wayland_session(wayland_display: Option<&str>, xdg_session_type: Option<&str>) -> bool {
    wayland_display.is_some_and(|value| !value.is_empty())
        || xdg_session_type.is_some_and(|value| value.eq_ignore_ascii_case("wayland"))
}

#[cfg(target_os = "linux")]
fn nvidia_driver_present() -> bool {
    // The DMA-BUF crash tracked here is specific to NVIDIA's proprietary driver
    // stack (WebKit bug 280210). Restricting the fallback to NVIDIA avoids
    // disabling the faster renderer on AMD/Intel systems where it works.
    std::path::Path::new("/proc/driver/nvidia/version").exists()
        || std::path::Path::new("/dev/nvidia0").exists()
}

#[cfg(target_os = "linux")]
fn apply_webkit_dma_buf_fallback() {
    let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
    let xdg_session_type = std::env::var("XDG_SESSION_TYPE").ok();
    let user_override = std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").ok();

    if user_override.is_none()
        && is_wayland_session(wayland_display.as_deref(), xdg_session_type.as_deref())
        && nvidia_driver_present()
    {
        // SAFETY: this runs before Tauri creates the GTK/WebKitGTK backend, and
        // `set_var` is only called once on this startup path.
        unsafe {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
}

#[cfg(target_os = "linux")]
fn ensure_linux_desktop_integration<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    use std::fs;
    if let Ok(data_dir) = crate::services::paths::app_data_dir(handle) {
        let icon_dir = data_dir.join("icons");
        let _ = fs::create_dir_all(&icon_dir);
        let icon_path = icon_dir.join("icon.png");
        let icon_bytes = include_bytes!("../icons/icon.png");
        let _ = fs::write(&icon_path, icon_bytes);

        if let Some(home) = std::env::var_os("HOME") {
            let apps_dir = std::path::PathBuf::from(home).join(".local/share/applications");
            let _ = fs::create_dir_all(&apps_dir);
            let icon_str = icon_path.to_string_lossy();
            let exec_str = std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "scaricatore-di-porto".to_string());

            let entries = [
                (
                    "com.scaricatorediporto.app.desktop",
                    "com.scaricatorediporto.app",
                ),
                ("scaricatore-di-porto.desktop", "scaricatore-di-porto"),
            ];

            for (filename, wm_class) in entries {
                let desktop_content = format!(
                    "[Desktop Entry]\n\
                     Type=Application\n\
                     Name=Scaricatore Di Porto\n\
                     Comment=Downloader and transcriber\n\
                     Exec=\"{}\" %U\n\
                     Icon={}\n\
                     Terminal=false\n\
                     Categories=Utility;Network;\n\
                     StartupNotify=true\n\
                     StartupWMClass={}\n",
                    exec_str, icon_str, wm_class
                );
                let _ = fs::write(apps_dir.join(filename), desktop_content);
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    apply_webkit_dma_buf_fallback();

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
        // Spec §13.4: signed in-app updates, endpoint and public key in
        // `tauri.conf.json`. The private key never exists in the repository.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Relaunch after an update, from the webview.
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let handle = app.handle().clone();
            #[cfg(target_os = "linux")]
            ensure_linux_desktop_integration(&handle);
            if let Some(window) = app.get_webview_window("main") {
                if let Some(icon) = app.default_window_icon() {
                    let _ = window.set_icon(icon.clone());
                }
            }
            let state = AppState::load(&handle)?;
            crate::services::logging::append_line(
                &handle,
                "INFO",
                &format!("app started, version {}", handle.package_info().version),
            );
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
            commands::app::app_info,
            commands::app::app_update_check,
            commands::app::app_update_install,
            commands::app::app_diagnostics,
            commands::app::app_open_logs,
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
            commands::queue::queue_transcribe_item,
            commands::queue::queue_add_local_file,
            commands::tools::tools_status,
            commands::tools::tools_test,
            commands::tools::tools_set_path,
            commands::tools::tools_set_ytdlp_channel,
            commands::tools::tools_update_ytdlp,
            commands::tools::tools_install,
            commands::tools::tools_cancel_install,
            commands::tools::tools_self_test,
            commands::tools::tools_complete_setup,
            commands::system::open_path,
            commands::system::reveal_path,
            commands::system::encoders_probe,
            commands::models::models_list,
            commands::models::models_download,
            commands::models::models_pause_download,
            commands::models::models_cancel_download,
            commands::models::models_delete,
            commands::models::models_import,
            commands::models::models_set_active,
            commands::models::models_open_folder,
            commands::gemini::gemini_key_get_status,
            commands::gemini::gemini_key_save,
            commands::gemini::gemini_key_delete,
            commands::gemini::gemini_key_test,
            commands::gemini::gemini_models_list,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_wayland_session_from_display() {
        assert!(is_wayland_session(Some("wayland-1"), None));
    }

    #[test]
    fn is_wayland_session_from_xdg_type() {
        assert!(is_wayland_session(None, Some("wayland")));
    }

    #[test]
    fn is_wayland_session_is_case_insensitive() {
        assert!(is_wayland_session(None, Some("Wayland")));
    }

    #[test]
    fn is_wayland_session_rejects_empty_display() {
        assert!(!is_wayland_session(Some(""), None));
    }

    #[test]
    fn is_wayland_session_rejects_x11() {
        assert!(!is_wayland_session(None, Some("x11")));
    }
}
