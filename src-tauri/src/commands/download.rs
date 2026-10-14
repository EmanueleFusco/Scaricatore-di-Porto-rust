//! The temporary single-video download commands (M1).
//!
//! M2 replaces this with the queue commands of spec §3 (`queue_add`,
//! `queue_cancel`, …) and the `queue:update` event. Until then there is one job
//! at a time, identified by a `job_id`, with two events:
//!
//! * `job:progress` — the payload of spec §3, coalesced by `services/job.rs`;
//! * `job:done` — the terminal state, emitted exactly once per job.
//!
//! Everything except the job id and the events is decided by the services; this
//! module validates input, builds the environment, spawns the task and cleans
//! up (spec §3: "commands/ has no logic").

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::commands::env::resolve_job_tools;
use crate::error::{AppResult, ErrorCode, ErrorInfo};
use crate::services::job::{self, JobEnvironment, JobOutcome, JobRequest, ProgressSink};
use crate::services::process;
use crate::services::ytdlp::{
    CookieSource, DownloadSpec, FormatChoice, JobDirs, JobKind, JobProgress,
};
use crate::state::AppState;

/// What the temporary UI asks for. The advanced format table arrives in M3.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QualityRequest {
    /// Best available, muxed or separate pair.
    Best,
    /// Exact height, the original's behaviour (spec §2.9, §8.2).
    Exact { height: u32 },
    /// Audio only, Opus (spec §8.2).
    AudioOnly,
}

impl QualityRequest {
    fn into_spec(self, cookies: CookieSource) -> DownloadSpec {
        match self {
            Self::Best => DownloadSpec {
                kind: JobKind::Video,
                quality: FormatChoice::Best,
                cookies,
            },
            Self::Exact { height } => {
                // Sanity bound: a height of 0 or 100000 is a UI bug, and the
                // selector it would build is nonsense.
                let height = height.clamp(144, 4320);
                DownloadSpec {
                    kind: JobKind::Video,
                    quality: FormatChoice::Exact { height },
                    cookies,
                }
            }
            Self::AudioOnly => DownloadSpec {
                kind: JobKind::Audio,
                quality: FormatChoice::AudioOnly,
                cookies,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStarted {
    pub job_id: String,
    /// Where the file will be written, so the UI can show it right away.
    pub output_dir: String,
}

/// The `job:progress` payload: spec §3 plus the job id.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgressEvent {
    pub job_id: String,
    #[serde(flatten)]
    pub progress: JobProgress,
}

/// The `job:done` payload.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDoneEvent {
    pub job_id: String,
    /// `done` | `failed` | `cancelled`.
    pub status: &'static str,
    pub file: Option<String>,
    /// `quickjs` | `deno` — spec §15 item 3: the UI shows the runtime used.
    pub runtime: Option<String>,
    pub elapsed_ms: Option<u64>,
    /// What the user asked for and what yt-dlp reported selecting
    /// (`SDP_FORMAT`); the UI warns when the second is lower than the first.
    pub requested_height: Option<u32>,
    pub actual_height: Option<u32>,
    pub error: Option<ErrorInfo>,
}

/// Start one download. Returns as soon as the job is registered; progress and
/// completion arrive as events.
#[tauri::command]
pub async fn download_start<R: tauri::Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    url: String,
    quality: QualityRequest,
    output_dir: Option<String>,
) -> AppResult<DownloadStarted> {
    let url = url.trim().to_owned();
    if !process::is_allowed_url(&url) {
        return Err(refuse(format!(
            "refusing to download {url:?}: only http:// and https:// URLs are accepted (spec §4.6)"
        )));
    }

    let settings = state.settings();
    let tools = resolve_job_tools(&state, &settings).await?;

    let requested = output_dir
        .filter(|raw| !raw.trim().is_empty())
        .map(PathBuf::from);
    let dir = crate::services::paths::output_dir_for(
        requested,
        settings.default_output_dir.clone(),
        crate::services::paths::default_output_dir(&app),
    )
    .ok_or_else(|| refuse("no output folder: pick one in Settings, or set a default in your OS"))?;
    crate::services::paths::ensure_dir(&dir).map_err(|err| {
        ErrorInfo::new(
            ErrorCode::Io,
            format!(
                "the output folder {} cannot be created: {err}",
                dir.display()
            ),
        )
    })?;

    let cookies = crate::services::ytdlp::cookie_source(
        settings.cookies_from_browser.as_deref(),
        settings.cookies_file.as_deref(),
        crate::services::ytdlp::CfgOs::current(),
    );
    let spec = quality.into_spec(cookies);

    let job_id = next_job_id();
    let temp = process::item_temp_dir(&dir, &job_id);
    let request = JobRequest {
        url,
        spec,
        dirs: JobDirs {
            home: dir.clone(),
            temp: temp.clone(),
        },
        stall_timeout: job::STALL_TIMEOUT,
    };
    let environment = JobEnvironment {
        ytdlp: tools.ytdlp.path,
        ffmpeg: tools.ffmpeg,
        runtimes: tools.runtimes,
        memory: state.runtime_memory().clone(),
        private_temp: tools.private_temp,
        youtube_clients: settings.youtube_player_clients.clone(),
    };

    let token = state.start_job(job_id.clone());
    let output_dir_display = dir.display().to_string();
    let started_id = job_id.clone();

    // The task outlives this borrow of `State`, so it reaches the state through
    // the app handle instead of capturing the reference.
    tauri::async_runtime::spawn(async move {
        let events = app.clone();
        let events_done = app.clone();
        let sink_id = started_id.clone();
        let sink: ProgressSink = Arc::new(move |progress: &JobProgress| {
            let _ = events.emit(
                "job:progress",
                JobProgressEvent {
                    job_id: sink_id.clone(),
                    progress: progress.clone(),
                },
            );
        });

        let outcome = job::run_job(&environment, &request, token, sink).await;
        // The job's temp dir goes, whatever happened (spec §7.3); finished
        // outputs and the .part leftovers of a pause are not touched here.
        job::cleanup_job_dir(&temp);

        let done = match outcome {
            JobOutcome::Done {
                file,
                runtime,
                elapsed_ms,
                requested_height,
                actual_height,
            } => JobDoneEvent {
                job_id: started_id.clone(),
                status: "done",
                file: Some(file.display().to_string()),
                runtime: Some(runtime.as_str().to_owned()),
                elapsed_ms: Some(elapsed_ms),
                requested_height,
                actual_height,
                error: None,
            },
            JobOutcome::Failed(error) => JobDoneEvent {
                job_id: started_id.clone(),
                status: "failed",
                file: None,
                runtime: None,
                elapsed_ms: None,
                requested_height: None,
                actual_height: None,
                error: Some(error.to_error_info()),
            },
            JobOutcome::Cancelled => JobDoneEvent {
                job_id: started_id.clone(),
                status: "cancelled",
                file: None,
                runtime: None,
                elapsed_ms: None,
                requested_height: None,
                actual_height: None,
                error: None,
            },
        };

        if let Some(state) = events_done.try_state::<AppState>() {
            state.finish_job(&started_id);
        }
        let _ = events_done.emit("job:done", done);
    });

    Ok(DownloadStarted {
        job_id,
        output_dir: output_dir_display,
    })
}

/// Cancel the running job. `false` means it was already gone (a stale click in
/// the UI, not an error).
#[tauri::command]
pub fn download_cancel(state: State<'_, AppState>, job_id: String) -> AppResult<bool> {
    Ok(state.cancel_job(&job_id))
}

/// Remember that the user declined the Deno install offer (spec §5.3.2: "ask
/// once, remember the answer").
#[tauri::command]
pub fn download_dismiss_deno_offer(state: State<'_, AppState>) -> AppResult<()> {
    let mut settings = state.settings();
    settings.deno_offer_dismissed = true;
    state
        .save_settings(settings)
        .map_err(|err| err.to_error_info())?;
    Ok(())
}

/// Unique, filesystem-safe job id. The counter makes two starts in the same
/// millisecond distinguishable; the millisecond makes a restart harmless.
fn next_job_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    format!("job-{millis}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn refuse(detail: impl Into<String>) -> ErrorInfo {
    ErrorInfo::new(ErrorCode::Unsupported, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_requests_become_the_documented_selectors() {
        let spec = QualityRequest::Best.into_spec(CookieSource::None);
        assert_eq!(spec.kind, JobKind::Video);
        assert_eq!(spec.quality, FormatChoice::Best);

        let spec = QualityRequest::Exact { height: 720 }.into_spec(CookieSource::None);
        assert_eq!(spec.quality, FormatChoice::Exact { height: 720 });

        let spec = QualityRequest::AudioOnly.into_spec(CookieSource::None);
        assert_eq!(spec.kind, JobKind::Audio);
        assert_eq!(spec.quality, FormatChoice::AudioOnly);

        // A nonsensical height is clamped to something buildable instead of
        // producing a selector no release will ever match.
        let spec = QualityRequest::Exact { height: 0 }.into_spec(CookieSource::None);
        assert_eq!(spec.quality, FormatChoice::Exact { height: 144 });
    }

    #[test]
    fn job_ids_are_unique_and_filesystem_safe() {
        let first = next_job_id();
        let second = next_job_id();
        assert_ne!(first, second);
        for id in [first, second] {
            assert!(id.starts_with("job-"));
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "{id} is not safe in a directory name"
            );
        }
    }
}
