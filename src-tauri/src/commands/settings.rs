//! Thin IPC handlers for the settings document (spec §3: `commands/` has no
//! logic — validate, call the service, return).

use tauri::State;

use crate::error::AppResult;
use crate::state::{AppState, IntoAppResult, Settings, SettingsView, settings_view};

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> AppResult<SettingsView> {
    settings_view(&state)
}

/// Persist `settings`, normalise it, and return what was actually written so
/// the UI can show the effective values.
#[tauri::command]
pub fn settings_save(state: State<'_, AppState>, settings: Settings) -> AppResult<SettingsView> {
    state.save_settings(settings).app_result()?;
    settings_view(&state)
}
