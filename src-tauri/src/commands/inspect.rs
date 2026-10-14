//! The inspect IPC command (spec §6.2): thin, as `commands/` must be.

use serde::Serialize;
use tauri::State;

use crate::commands::env::resolve_job_tools;
use crate::error::{AppResult, ErrorCode, ErrorInfo};
use crate::services::inspect::{self, InspectEnvironment, InspectOutcome, Inspected};
use crate::services::process;
use crate::services::ytdlp::{self, CfgOs};
use crate::state::AppState;

/// Answer of one inspection. `request_id` is echoed back so the frontend can
/// drop stale answers (spec §6.2): URLs change faster than yt-dlp replies, and
/// an old title appearing under a new URL is exactly the bug spec §12.12
/// records from the original.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectView {
    pub request_id: u64,
    pub inspected: Inspected,
}

/// Inspect one URL. The previous inspection is cancelled first (spec §6.2).
#[tauri::command]
pub async fn inspect_url(
    state: State<'_, AppState>,
    request_id: u64,
    url: String,
) -> AppResult<InspectView> {
    let url = url.trim().to_owned();
    if !process::is_allowed_url(&url) {
        return Err(refuse(format!(
            "refusing to inspect {url:?}: only http:// and https:// URLs are accepted (spec §4.6)"
        )));
    }

    let settings = state.settings();
    let tools = resolve_job_tools(&state, &settings).await?;
    let environment = InspectEnvironment {
        ytdlp: tools.ytdlp.path,
        cookies: ytdlp::cookie_source(
            settings.cookies_from_browser.as_deref(),
            settings.cookies_file.as_deref(),
            CfgOs::current(),
        ),
        runtimes: tools.runtimes,
        memory: state.runtime_memory().clone(),
        private_temp: tools.private_temp,
        youtube_clients: settings.youtube_player_clients.clone(),
    };

    let token = state.start_inspection(request_id);
    let outcome = inspect::inspect(&environment, &url, token).await;
    state.finish_inspection(request_id);

    match outcome {
        InspectOutcome::Inspected { inspected, .. } => Ok(InspectView {
            request_id,
            inspected: *inspected,
        }),
        // The user cleared the box or typed something else: the frontend drops
        // this answer by request id, so the command only has to say so.
        InspectOutcome::Cancelled => Err(ErrorInfo {
            code: "CANCELLED".to_owned(),
            message_key: "error.cancelled".to_owned(),
            detail: None,
        }),
    }
}

/// Cancel the running inspection; used when the URL box is cleared (spec §6.2).
#[tauri::command]
pub fn inspect_cancel(state: State<'_, AppState>) {
    state.cancel_inspection();
}

fn refuse(detail: impl Into<String>) -> ErrorInfo {
    ErrorInfo::new(ErrorCode::Unsupported, detail)
}
