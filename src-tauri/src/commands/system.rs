//! System integration commands (spec §3, §9.3, §9.5).
//!
//! Provides `open_path` and `reveal_path` (via `tauri-plugin-opener`), and
//! `encoders_probe` (probing hardware and CPU video encoders).

use std::fs;
use std::path::Path;

use tauri::State;
use tokio_util::sync::CancellationToken;

use crate::error::{AppResult, ErrorCode, ErrorInfo};
use crate::services::encoders::{self, EncoderInfo};
use crate::services::tools;
use crate::state::AppState;

/// Open a file with the system default player/application (spec §9.5).
///
/// Failure to open never fails the item.
#[tauri::command]
pub async fn open_path(path: String) -> AppResult<()> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(ErrorInfo::new(
            ErrorCode::Io,
            format!("path does not exist: {path}"),
        ));
    }
    tauri_plugin_opener::open_path(&path, None::<&str>)
        .map_err(|err| ErrorInfo::new(ErrorCode::Io, format!("failed to open path: {err}")))
}

/// Reveal a file in the system file manager (spec §9.5, §11.5).
#[tauri::command]
pub async fn reveal_path(path: String) -> AppResult<()> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(ErrorInfo::new(
            ErrorCode::Io,
            format!("path does not exist: {path}"),
        ));
    }
    tauri_plugin_opener::reveal_item_in_dir(&path)
        .map_err(|err| ErrorInfo::new(ErrorCode::Io, format!("failed to reveal path: {err}")))
}

/// Probe available video encoders (spec §9.3, cached by ffmpeg path/mtime/version).
#[tauri::command]
pub async fn encoders_probe(
    state: State<'_, AppState>,
    force: Option<bool>,
) -> AppResult<Vec<EncoderInfo>> {
    let settings = state.settings();
    let Some(ffmpeg) = crate::commands::env::resolve_cached(&state, &settings, tools::FFMPEG).await
    else {
        return Err(ErrorInfo::new(
            ErrorCode::Unsupported,
            "ffmpeg is not installed",
        ));
    };

    let mtime = fs::metadata(&ffmpeg.path).and_then(|m| m.modified()).ok();
    let token = CancellationToken::new();

    encoders::probe_all_encoders(
        &ffmpeg.path,
        mtime,
        ffmpeg.version.as_deref(),
        state.encoders(),
        token,
        force.unwrap_or(false),
    )
    .await
    .map_err(|err| err.to_error_info())
}
