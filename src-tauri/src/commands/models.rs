//! Thin IPC handlers for Whisper model management (spec §10.2, §11.6).

use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};

use crate::error::{AppError, AppResult, ErrorInfo};
use crate::services::download::Transport;
use crate::services::models::{
    self, ModelCatalogue, ModelEntry, ModelsView, cancel_model_download, delete_model, import_model,
};
use crate::state::{AppState, IntoAppResult, Settings};

/// Fetch the full Whisper models view (catalogue, installed status, active model,
/// free space, VAD status).
#[tauri::command]
pub fn models_list(state: State<'_, AppState>) -> AppResult<ModelsView> {
    let catalogue = ModelCatalogue::embedded().app_result()?;
    let models_dir = state.models_dir();
    let settings = state.settings();
    let in_flight = state.models_manager().in_flight_progress();

    let view = models::inspect_models(
        &models_dir,
        &catalogue,
        settings.active_whisper_model.as_deref(),
        &in_flight,
    );
    Ok(view)
}

/// Download a Whisper model (and VAD model if missing) in background.
#[tauri::command]
pub async fn models_download<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    model_id: String,
) -> AppResult<()> {
    let catalogue = ModelCatalogue::embedded().app_result()?;
    let _ = catalogue
        .find(&model_id)
        .ok_or_else(|| AppError::ModelNotFound(model_id.clone()))
        .app_result()?;

    if state.models_manager().is_downloading(&model_id) {
        return Ok(());
    }

    let mirror_url = state.settings().whisper_models_mirror_url;
    let manager = Arc::clone(state.models_manager());
    let models_dir = state.models_dir();
    let app_handle = app.clone();
    let mid_for_err = model_id.clone();
    let mid_for_done = model_id.clone();

    tokio::spawn(async move {
        let app_prog = app_handle.clone();
        let on_progress = models::throttled_model_progress_sink(
            std::time::Duration::from_millis(100),
            move |event| {
                let _ = app_prog.emit("models:progress", event);
            },
        );

        #[cfg(test)]
        let transport = Transport::LoopbackHttp;
        #[cfg(not(test))]
        let transport = Transport::HttpsOnly;

        match manager
            .download_model(
                &models_dir,
                &catalogue,
                &model_id,
                mirror_url.as_deref(),
                transport,
                on_progress,
            )
            .await
        {
            Ok(_) => {
                let _ = app_handle.emit("models:completed", mid_for_done);
            }
            Err(err) => {
                let err_str = err.to_string();
                if err_str.contains("the download was cancelled") {
                    let _ = app_handle.emit("models:cancelled", mid_for_done);
                    return;
                }
                #[derive(Serialize, Clone)]
                #[serde(rename_all = "camelCase")]
                struct ModelErrorPayload {
                    model_id: String,
                    error: ErrorInfo,
                }
                let _ = app_handle.emit(
                    "models:error",
                    ModelErrorPayload {
                        model_id: mid_for_err,
                        error: err.to_error_info(),
                    },
                );
            }
        }
    });

    Ok(())
}

/// Pause an in-flight download of a model (keeps .part on disk).
#[tauri::command]
pub fn models_pause_download(state: State<'_, AppState>, model_id: String) -> AppResult<bool> {
    Ok(state.models_manager().pause(&model_id))
}

/// Cancel an in-flight or paused download of a model (removes .part on disk).
#[tauri::command]
pub fn models_cancel_download(state: State<'_, AppState>, model_id: String) -> AppResult<bool> {
    let catalogue = ModelCatalogue::embedded().app_result()?;
    let models_dir = state.models_dir();
    let was_running = state.models_manager().cancel(&model_id);
    let _ = cancel_model_download(&models_dir, &catalogue, &model_id);
    Ok(was_running)
}

/// Delete an installed model from disk.
#[tauri::command]
pub fn models_delete(state: State<'_, AppState>, model_id: String) -> AppResult<ModelsView> {
    state.models_manager().cancel(&model_id);
    let catalogue = ModelCatalogue::embedded().app_result()?;
    let models_dir = state.models_dir();

    delete_model(&models_dir, &catalogue, &model_id).app_result()?;

    let mut settings = state.settings();
    if settings.active_whisper_model.as_deref() == Some(&model_id) {
        settings.active_whisper_model = None;
        let _ = state.save_settings(settings);
    }

    models_list(state)
}

/// Import a local `.bin` Whisper model file from disk into the models directory.
#[tauri::command]
pub fn models_import(
    state: State<'_, AppState>,
    source_path: String,
    target_model_id: Option<String>,
) -> AppResult<ModelEntry> {
    let catalogue = ModelCatalogue::embedded().app_result()?;
    let models_dir = state.models_dir();
    let path = Path::new(&source_path);

    let imported =
        import_model(&models_dir, &catalogue, path, target_model_id.as_deref()).app_result()?;

    let mut settings = state.settings();
    if settings.active_whisper_model.is_none() {
        settings.active_whisper_model = Some(imported.id.clone());
        let _ = state.save_settings(settings);
    }

    Ok(imported)
}

/// Set the active Whisper model in settings.
#[tauri::command]
pub fn models_set_active(state: State<'_, AppState>, model_id: String) -> AppResult<Settings> {
    let catalogue = ModelCatalogue::embedded().app_result()?;
    if catalogue.find(&model_id).is_none() {
        return Err(AppError::ModelNotFound(model_id).to_error_info());
    }

    let mut settings = state.settings();
    settings.active_whisper_model = Some(model_id);
    state.save_settings(settings).app_result()
}

/// Open the models directory in the OS file manager.
#[tauri::command]
pub fn models_open_folder(state: State<'_, AppState>) -> AppResult<()> {
    let dir = state.models_dir();
    fs::create_dir_all(&dir).app_result()?;
    tauri_plugin_opener::open_path(dir.to_string_lossy().as_ref(), None::<&str>)
        .map_err(|err| AppError::Io(std::io::Error::other(err.to_string())).to_error_info())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::sync::mpsc;
    use std::time::Duration;
    use tauri::{Listener, Manager};
    use tempfile::tempdir;

    #[tokio::test(flavor = "multi_thread")]
    async fn models_download_throttles_rapid_chunk_progress_events() {
        let dir = tempdir().unwrap();
        let state = AppState::from_path(dir.path().join("settings.json")).unwrap();
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        app.manage(state);

        // Pre-create VAD file so models_download proceeds directly to downloading the model
        let catalogue = ModelCatalogue::embedded().unwrap();
        let models_dir = app.state::<AppState>().models_dir();
        fs::create_dir_all(&models_dir).unwrap();
        fs::write(models_dir.join(&catalogue.vad.file), b"vad-dummy").unwrap();

        // Start a mock TCP server sending 25 rapid chunks of 100 bytes each
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);

        let server_handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(match stream.try_clone() {
                    Ok(c) => c,
                    Err(_) => break,
                });
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Err(_) | Ok(0) => break,
                        Ok(_) => {}
                    }
                    if line.trim().is_empty() {
                        break;
                    }
                }
                let resp = "HTTP/1.1 200 OK\r\nContent-Length: 2500\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();

                // Send 25 small chunks rapidly across 50ms (< 100ms throttle interval)
                for _ in 0..25 {
                    let _ = stream.write_all(&[0x42; 100]);
                    let _ = stream.flush();
                    std::thread::sleep(Duration::from_millis(2));
                }
                let _ = stream.shutdown(std::net::Shutdown::Write);
                drop(stream);
            }
        });

        // Set mirror url to local test server
        let mut settings = app.state::<AppState>().settings();
        settings.whisper_models_mirror_url = Some(format!("http://127.0.0.1:{port}"));
        app.state::<AppState>().save_settings(settings).unwrap();

        let progress_events = Arc::new(Mutex::new(Vec::new()));
        let p_events = Arc::clone(&progress_events);
        app.listen("models:progress", move |event| {
            p_events.lock().unwrap().push(event.payload().to_string());
        });

        let (done_tx, done_rx) = mpsc::channel();
        let d_tx1 = done_tx.clone();
        app.listen("models:completed", move |_| {
            let _ = d_tx1.send(());
        });
        let d_tx2 = done_tx.clone();
        app.listen("models:error", move |_| {
            let _ = d_tx2.send(());
        });

        models_download(
            app.handle().clone(),
            app.state::<AppState>(),
            "tiny-q5_1".to_string(),
        )
        .await
        .unwrap();

        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("download task must finish or error");

        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = std::net::TcpStream::connect(("127.0.0.1", port));
        let _ = server_handle.join();

        let count = progress_events.lock().unwrap().len();
        // 25 rapid chunks were sent in <50ms.
        // Unthrottled code emits an IPC event on every single chunk (>= 25 events).
        // Throttled code coalesces intermediate chunk events to 100ms intervals (<= 3 events).
        assert!(
            count <= 3,
            "models:progress emitted unthrottled on every TCP read chunk: got {count} events for 25 rapid chunks (expected <= 3)"
        );
    }
}
