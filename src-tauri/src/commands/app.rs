//! App metadata, diagnostics and in-app updates (spec §11.8, §13.4).
//!
//! The updater stays on the Rust side: the signing key never reaches the
//! webview, and the JS bundle only asks for a check, an install and a
//! relaunch. The updater endpoint and the public key are in `tauri.conf.json`.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::UpdaterExt;

use crate::error::{AppError, AppResult, ErrorCode, ErrorInfo};
use crate::services::diagnostics::{DiagnosticTool, DiagnosticsInput, diagnostics_text};
use crate::services::logging;
use crate::services::tools::ToolStatus;
use crate::state::{AppState, IntoAppResult};

/// Static information about this build, for the About page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub triple: String,
    pub settings_path: String,
    pub tools_dir: String,
    pub bundled_dir: String,
    pub temp_dir: Option<String>,
    pub models_dir: String,
    pub log_path: String,
}

fn info_for(app: &AppHandle, state: &AppState) -> AppInfo {
    let package = app.package_info();
    let log_path = logging::log_path(app)
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    AppInfo {
        name: package.name.clone(),
        version: package.version.to_string(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        triple: state.tools().triple().to_owned(),
        settings_path: state.settings_path().display().to_string(),
        tools_dir: state.tools().tools_dir().display().to_string(),
        bundled_dir: state.tools().bundled_dir().display().to_string(),
        temp_dir: state
            .tools()
            .temp_dir()
            .map(|dir| dir.display().to_string()),
        models_dir: state.models_dir().display().to_string(),
        log_path,
    }
}

/// Version, platform and the paths support will ask for.
#[tauri::command]
pub fn app_info(app: AppHandle, state: State<'_, AppState>) -> AppResult<AppInfo> {
    Ok(info_for(&app, &state))
}

/// One update check (spec §13.4).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub available: bool,
    pub current_version: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub date: Option<String>,
}

fn update_error(err: impl std::fmt::Display) -> ErrorInfo {
    ErrorInfo::new(ErrorCode::Update, err.to_string())
}

/// Ask the configured endpoint whether a newer release exists.
#[tauri::command]
pub async fn app_update_check(app: AppHandle) -> AppResult<UpdateCheck> {
    let current_version = app.package_info().version.to_string();
    let updater = app.updater().map_err(update_error)?;
    let update = updater.check().await.map_err(update_error)?;
    logging::append_line(
        &app,
        "INFO",
        &match &update {
            Some(update) => format!("update available: {}", update.version),
            None => "update check: already current".to_owned(),
        },
    );
    Ok(match update {
        Some(update) => UpdateCheck {
            available: true,
            current_version,
            version: Some(update.version),
            notes: update.body,
            date: update.date.map(|date| date.to_string()),
        },
        None => UpdateCheck {
            available: false,
            current_version,
            version: None,
            notes: None,
            date: None,
        },
    })
}

/// Progress of one update download, already reduced for a progress bar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub percent: Option<u8>,
}

/// Download, verify and install the available update (spec §13.4).
///
/// Refused while the queue is doing something: replacing the binary under a
/// running yt-dlp is exactly the kind of surprise a downloader must not give.
#[tauri::command]
pub async fn app_update_install(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<UpdateCheck> {
    if let Some(queue) = state.queue() {
        let summary = queue.snapshot().summary;
        if summary.is_active() {
            return Err(update_error(
                "the queue is still running; finish or pause it before updating",
            ));
        }
    }

    let updater = app.updater().map_err(update_error)?;
    let update = updater
        .check()
        .await
        .map_err(update_error)?
        .ok_or_else(|| update_error("no update is available"))?;
    let version = update.version.clone();
    logging::append_line(&app, "INFO", &format!("installing update {version}"));

    let progress_app = app.clone();
    let finish_app = app.clone();
    let mut downloaded = 0u64;
    let result = update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let percent = total
                    .filter(|total| *total > 0)
                    .map(|total| ((downloaded * 100 / total).min(100)) as u8);
                let _ = progress_app.emit(
                    "update:progress",
                    UpdateProgress {
                        downloaded,
                        total,
                        percent,
                    },
                );
            },
            move || {
                let _ = finish_app.emit("update:downloaded", ());
            },
        )
        .await;

    match result {
        Ok(()) => {
            logging::append_line(&app, "INFO", &format!("update {version} installed"));
            Ok(UpdateCheck {
                available: true,
                current_version: app.package_info().version.to_string(),
                version: Some(version),
                notes: None,
                date: None,
            })
        }
        Err(err) => {
            logging::append_line(&app, "ERROR", &format!("update {version} failed: {err}"));
            Err(update_error(err))
        }
    }
}

fn diagnostic_tools(tools: &[ToolStatus]) -> Vec<DiagnosticTool<'_>> {
    tools
        .iter()
        .map(|tool| DiagnosticTool {
            name: &tool.name,
            version: tool.version.as_deref(),
            source: tool.source.map(|source| source.as_str()),
            path: tool.path.as_deref(),
        })
        .collect()
}

/// Plain-text report for a GitHub issue (spec §11.8). Never includes secrets.
#[tauri::command]
pub async fn app_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
    bundle_type: Option<String>,
) -> AppResult<String> {
    let info = info_for(&app, &state);
    let view = crate::commands::tools::collect_tools_view(&state).await?;
    let tools = diagnostic_tools(&view.tools);
    let log_tail = logging::tail_lines(std::path::Path::new(&info.log_path), logging::TAIL_LINES)
        .unwrap_or_else(|err| format!("(could not read the log: {err})"));
    Ok(diagnostics_text(&DiagnosticsInput {
        app_name: &info.name,
        version: &info.version,
        os: &info.os,
        arch: &info.arch,
        triple: &info.triple,
        bundle_type: bundle_type.as_deref(),
        settings_path: &info.settings_path,
        tools_dir: &info.tools_dir,
        bundled_dir: &info.bundled_dir,
        temp_dir: info.temp_dir.as_deref(),
        models_dir: &info.models_dir,
        log_path: &info.log_path,
        tools: &tools,
        log_tail: &log_tail,
    }))
}

/// Reveal the log file in the system file manager, creating it if needed.
#[tauri::command]
pub fn app_open_logs(app: AppHandle) -> AppResult<String> {
    let path = logging::log_path(&app).app_result()?;
    if !path.exists() {
        logging::append_to(&path, "INFO", "log opened before any event").app_result()?;
    }
    tauri_plugin_opener::reveal_item_in_dir(&path)
        .map_err(|err| AppError::Process(format!("cannot reveal the log file: {err}")))
        .app_result()?;
    Ok(path.display().to_string())
}
