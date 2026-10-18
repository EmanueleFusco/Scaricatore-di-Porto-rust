//! Thin IPC handlers for Whisper model management (spec §10.2, §11.6).

use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult, ErrorInfo};
use crate::services::download::Transport;
use crate::services::models::{
    self, ModelCatalogue, ModelEntry, ModelsView, delete_model, import_model,
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
pub async fn models_download(
    app: AppHandle,
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
        let on_progress = move |event| {
            let _ = app_prog.emit("models:progress", event);
        };

        match manager
            .download_model(
                &models_dir,
                &catalogue,
                &model_id,
                mirror_url.as_deref(),
                Transport::HttpsOnly,
                on_progress,
            )
            .await
        {
            Ok(_) => {
                let _ = app_handle.emit("models:completed", mid_for_done);
            }
            Err(err) => {
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

/// Cancel an in-flight download of a model.
#[tauri::command]
pub fn models_cancel_download(state: State<'_, AppState>, model_id: String) -> AppResult<bool> {
    Ok(state.models_manager().cancel(&model_id))
}

/// Delete an installed model from disk.
#[tauri::command]
pub fn models_delete(state: State<'_, AppState>, model_id: String) -> AppResult<ModelsView> {
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
