//! Thin IPC handlers for the settings document and download archive (spec §3,
//! §6.2.4, §11.8: `commands/` has no logic — validate, call the service,
//! return).

use std::fs;
use std::io;
use std::path::Path;

use serde::Serialize;
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

/// Summary of the playlist download archive (`<app-data>/archive.txt`, spec
/// §6.2.4, §11.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveStatusView {
    pub path: String,
    pub entries: usize,
}

/// Inspect `<app-data>/archive.txt` and return how many recorded video IDs it
/// holds (spec §11.8).
#[tauri::command]
pub fn archive_status(state: State<'_, AppState>) -> AppResult<ArchiveStatusView> {
    let path = state.tools().archive_path();
    read_archive_status(&path).app_result()
}

/// Clear `<app-data>/archive.txt` (spec §11.8 "button to clear archive").
#[tauri::command]
pub fn archive_clear(state: State<'_, AppState>) -> AppResult<ArchiveStatusView> {
    let path = state.tools().archive_path();
    clear_archive_file(&path).app_result()
}

fn read_archive_status(path: &Path) -> io::Result<ArchiveStatusView> {
    let entries = match fs::read_to_string(path) {
        Ok(raw) => raw
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .count(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => 0,
        Err(err) => return Err(err),
    };
    Ok(ArchiveStatusView {
        path: path.display().to_string(),
        entries,
    })
}

fn clear_archive_file(path: &Path) -> io::Result<ArchiveStatusView> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    Ok(ArchiveStatusView {
        path: path.display().to_string(),
        entries: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn archive_status_and_clear_handle_missing_and_populated_files() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("archive.txt");

        let empty = read_archive_status(&path).expect("missing file is 0 entries");
        assert_eq!(empty.entries, 0);

        fs::write(&path, "youtube dQw4w9WgXcQ\n\n# comment\nvimeo 12345\n").expect("write");
        let populated = read_archive_status(&path).expect("populated");
        assert_eq!(populated.entries, 2);

        let cleared = clear_archive_file(&path).expect("clear");
        assert_eq!(cleared.entries, 0);
        assert!(!path.exists());
    }
}
