//! IPC command handlers for Gemini API key and model management (spec §10.3, §10.5).

use std::path::Path;
use std::time::Duration;

use tauri::State;

use crate::error::{AppError, AppResult, ErrorInfo};
use crate::services::secrets::{self, GeminiKeyStatus, KeySource};
use crate::services::transcribe::gemini::{self, DEFAULT_GEMINI_BASE_URL, GeminiModelInfo};
use crate::state::AppState;

fn config_dir(state: &AppState) -> &Path {
    state.settings_path().parent().unwrap_or(Path::new(""))
}

/// Retrieve the Gemini key status (has_key, source). Never leaks the secret key (spec §10.5).
#[tauri::command]
pub fn gemini_key_get_status(state: State<'_, AppState>) -> AppResult<GeminiKeyStatus> {
    Ok(secrets::get_gemini_key_status(config_dir(&state)))
}

/// Save the Gemini API key to OS keyring (or fallback 0600 file) (spec §10.5).
#[tauri::command]
pub fn gemini_key_save(state: State<'_, AppState>, key: String) -> AppResult<KeySource> {
    secrets::set_gemini_api_key(config_dir(&state), &key)
        .map_err(|e| ErrorInfo::new(e.code(), e.to_string()))
}

/// Delete the Gemini API key from keyring and fallback file.
#[tauri::command]
pub fn gemini_key_delete(state: State<'_, AppState>) -> AppResult<()> {
    secrets::delete_gemini_api_key(config_dir(&state))
        .map_err(|e| ErrorInfo::new(e.code(), e.to_string()))
}

/// Test whether the provided key (or saved key if None) is valid against the Gemini API.
#[tauri::command]
pub async fn gemini_key_test(state: State<'_, AppState>, key: Option<String>) -> AppResult<bool> {
    let api_key = match key {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => match secrets::get_gemini_api_key(config_dir(&state)) {
            Some(k) => k,
            None => {
                return Err(ErrorInfo::new(
                    crate::error::ErrorCode::GeminiKeyMissing,
                    "No Gemini API key configured to test",
                ));
            }
        },
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Process(e.to_string()))
        .map_err(|e| ErrorInfo::new(e.code(), e.to_string()))?;

    gemini::list_gemini_models(&client, DEFAULT_GEMINI_BASE_URL, &api_key)
        .await
        .map(|_| true)
        .map_err(|e| ErrorInfo::new(e.code(), e.to_string()))
}

/// Query dynamic list of available models from `GET /v1beta/models` (spec §10.3).
#[tauri::command]
pub async fn gemini_models_list(state: State<'_, AppState>) -> AppResult<Vec<GeminiModelInfo>> {
    let api_key = secrets::get_gemini_api_key(config_dir(&state)).ok_or_else(|| {
        ErrorInfo::new(
            crate::error::ErrorCode::GeminiKeyMissing,
            "Gemini API key is not configured",
        )
    })?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Process(e.to_string()))
        .map_err(|e| ErrorInfo::new(e.code(), e.to_string()))?;

    gemini::list_gemini_models(&client, DEFAULT_GEMINI_BASE_URL, &api_key)
        .await
        .map_err(|e| ErrorInfo::new(e.code(), e.to_string()))
}
