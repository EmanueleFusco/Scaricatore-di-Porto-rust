//! The inspect IPC command (spec §6.2): thin, as `commands/` must be.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::env::resolve_job_tools;
use crate::error::{AppResult, ErrorCode, ErrorInfo};
use crate::services::inspect::{
    self, InspectEnvironment, InspectOptions, InspectOutcome, Inspected,
};
use crate::services::process;
use crate::services::ytdlp::{self, CfgOs};
use crate::state::AppState;

/// Optional per-inspection flags from the frontend (spec §6.2, §6.4):
/// pagination (`item_range`), forcing single-video mode on `watch?v=…&list=…`
/// (`no_playlist`), or per-link network headers from "More options".
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InspectRequestOptions {
    pub item_range: Option<(u32, u32)>,
    pub no_playlist: Option<bool>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub custom_headers: Vec<String>,
}

impl InspectRequestOptions {
    fn into_service_options(self) -> InspectOptions {
        InspectOptions {
            item_range: self.item_range,
            no_playlist: self.no_playlist.unwrap_or(false),
            impersonate_chrome: false,
            proxy: None,
            referer: self.referer,
            user_agent: self.user_agent,
            custom_headers: self.custom_headers,
        }
    }
}

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
    options: Option<InspectRequestOptions>,
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
    let service_opts = options.unwrap_or_default().into_service_options();

    let token = state.start_inspection(request_id);
    let outcome = inspect::inspect_with_options(&environment, &url, &service_opts, token).await;
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

/// How many URLs the batch inspection runs at once (spec §6.2: "Concurrency:
/// inspect up to 3 URLs in parallel when many are pasted").
pub const BATCH_CONCURRENCY: usize = 3;

/// One answer of [`inspect_urls`], in the order the URLs were sent.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchInspectView {
    pub url: String,
    /// `null` when the URL was refused or the inspection failed; the message is
    /// in `error` and the frontend shows it inline on that row (spec §11.3:
    /// "Errors are inline per link with a Retry/Details action, never a modal").
    pub inspected: Option<Inspected>,
    pub error: Option<ErrorInfo>,
}

/// Inspect several URLs, up to [`BATCH_CONCURRENCY`] at a time.
///
/// This is the path the Add bar uses when many links are pasted, and it is
/// **not** the same as calling `inspect_url` in a loop:
///
/// * each URL gets its own request id and its own cancellation token, so one
///   slow link cannot cancel another — the "cancel the previous inspection" rule
///   of spec §6.2 exists for the *single* box, where the previous URL really is
///   obsolete;
/// * the answers come back **in order**, so row 3 of the Add bar is row 3 of the
///   answer, whatever finished first;
/// * the tool resolution happens once for the batch instead of once per URL
///   (D75's cache makes that cheap, but once is once).
#[tauri::command]
pub async fn inspect_urls(
    state: State<'_, AppState>,
    request_id: u64,
    urls: Vec<String>,
    options: Option<InspectRequestOptions>,
) -> AppResult<Vec<BatchInspectView>> {
    let wanted: Vec<String> = urls
        .into_iter()
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty())
        .collect();

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
    let service_opts = options.unwrap_or_default().into_service_options();

    let mut answers: Vec<Option<BatchInspectView>> = (0..wanted.len()).map(|_| None).collect();
    let mut next = 0;
    while next < wanted.len() {
        let end = (next + BATCH_CONCURRENCY).min(wanted.len());
        let batch: Vec<(usize, String)> = wanted[next..end]
            .iter()
            .enumerate()
            .map(|(offset, url)| (next + offset, url.clone()))
            .collect();
        next = end;

        let mut set = tokio::task::JoinSet::new();
        for (index, url) in batch {
            let environment = environment.clone();
            let opts = service_opts.clone();
            set.spawn(async move { (index, inspect_one(&environment, &opts, url).await) });
        }
        while let Some(joined) = set.join_next().await {
            // A panicking inspection loses its own row, not the batch: that
            // answer stays `None` and the frontend shows the row as failed.
            if let Ok((index, answer)) = joined
                && let Some(slot) = answers.get_mut(index)
            {
                *slot = Some(answer);
            }
        }
    }

    // The single-url path is what owns the "one inspection at a time" slot; a
    // finished batch leaves nothing registered behind it.
    state.finish_inspection(request_id);
    Ok(answers.into_iter().flatten().collect())
}

/// Inspect one URL inside a batch: refusal, tool failure and cancellation all
/// become an answer for that row instead of an error for the whole batch.
async fn inspect_one(
    environment: &InspectEnvironment,
    options: &InspectOptions,
    url: String,
) -> BatchInspectView {
    if !process::is_allowed_url(&url) {
        return BatchInspectView {
            inspected: None,
            error: Some(refuse(format!(
                "refusing to inspect {url:?}: only http:// and https:// URLs are accepted (spec §4.6)"
            ))),
            url,
        };
    }
    let token = tokio_util::sync::CancellationToken::new();
    match inspect::inspect_with_options(environment, &url, options, token).await {
        InspectOutcome::Inspected { inspected, .. } => BatchInspectView {
            url,
            inspected: Some(*inspected),
            error: None,
        },
        InspectOutcome::Cancelled => BatchInspectView {
            url,
            inspected: None,
            error: Some(ErrorInfo {
                code: "CANCELLED".to_owned(),
                message_key: "error.cancelled".to_owned(),
                detail: None,
            }),
        },
    }
}

/// A refused URL becomes an answer for its own row instead of failing the batch.
fn refuse(detail: impl Into<String>) -> ErrorInfo {
    ErrorInfo::new(ErrorCode::Unsupported, detail)
}
