//! The job runner the queue drives (spec §3 `job.rs`).
//!
//! It was written for M1's single-video screen; M2 kept it and made it the one
//! place a download happens: [`crate::services::queue::engine::run_item`] calls
//! [`run_job`] for every queue item. It exists so the behaviour the queue has
//! to demonstrate — fallback, retry, cancel, temp cleanup — is testable without
//! any UI.
//!
//! The runner composes the pure pieces of [`crate::services::ytdlp`] and
//! [`crate::services::inspect`] with the process layer of
//! [`crate::services::process`]. It knows three things the arg builders cannot:
//!
//! * **the QuickJS→Deno fallback** (spec §5.3.2), one shot, when the classifier
//!   says `YT_CHALLENGE` or the challenge stalls for 90 s;
//! * **the one retry without `-f`** (spec §8.2) when the classifier says
//!   `FORMAT`;
//! * **the temp directory lifecycle**: `<out>/.sdp-tmp-<itemid>` is removed
//!   after the run, cancelled or not, and never touches finished outputs
//!   (spec §7.3, §4.10).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::services::inspect::{FallbackDecision, Runtimes};
use crate::services::jsruntime;
use crate::services::process::{self, ITEM_TEMP_PREFIX, LineSinks, ToolCommand};
use crate::services::ytdlp::{
    self, DownloadEnvironment, DownloadSpec, JobDirs, JobProgress, JobStage, ProgressThrottle,
    ProgressTracker, YtdlpError, YtdlpErrorCode,
};

/// How long a download may sit with the JS challenge unsolved before the
/// fallback replaces the runtime (spec §5.3.2: "no progress for 90 s while the
/// JS challenge was being solved").
pub const STALL_TIMEOUT: Duration = Duration::from_secs(90);

/// Everything a job needs that is not user options.
#[derive(Clone)]
pub struct JobEnvironment {
    pub ytdlp: PathBuf,
    /// `None` fails the job before anything is spawned: merging and audio
    /// conversion both need ffmpeg, and "two split files and exit 0" is not a
    /// result (spec §5.2.1, §9).
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
    pub runtimes: Runtimes,
    pub memory: Arc<jsruntime::RuntimeMemoryStore>,
    /// Spec §4.9: private TMP dir for the child processes.
    pub private_temp: Option<PathBuf>,
    /// Expert override for YouTube's client list; `None` = yt-dlp's defaults
    /// (the recommended state, D74).
    pub youtube_clients: Option<String>,
}

/// One job. `job_id` names the temp directory; the caller owns it.
#[derive(Clone)]
pub struct JobRequest {
    pub url: String,
    pub spec: DownloadSpec,
    pub dirs: JobDirs,
    /// 90 s in production; tests use something they can wait for.
    pub stall_timeout: Duration,
}

/// What the job left behind.
#[derive(Debug, Clone)]
pub enum JobOutcome {
    Done {
        /// The real path, from `SDP_FILE` (spec §8.3). Never guessed.
        file: PathBuf,
        /// Which runtime actually produced the file (spec §15 item 3: the UI
        /// must show it).
        runtime: jsruntime::RuntimeKind,
        elapsed_ms: u64,
        /// The height the user asked for, when the choice asked for one.
        requested_height: Option<u32>,
        /// The height yt-dlp reported it selected (`SDP_FORMAT`). `None` for
        /// audio-only and for extractors that do not declare one.
        actual_height: Option<u32>,
    },
    /// `--download-archive` matched this video ID: yt-dlp exited 0 without
    /// downloading anything (spec §6.2.4).
    SkippedArchive {
        runtime: jsruntime::RuntimeKind,
        elapsed_ms: u64,
    },
    Failed(YtdlpError),
    Cancelled,
}

/// The sink the command layer uses to emit `job:progress`.
pub type ProgressSink = Arc<dyn Fn(&JobProgress) + Send + Sync>;

/// Run one download to completion, applying the fallback and retry policies.
pub async fn run_job(
    env: &JobEnvironment,
    request: &JobRequest,
    token: CancellationToken,
    on_progress: ProgressSink,
) -> JobOutcome {
    let emitter = Emitter::new(on_progress);

    // The pre-flight check of spec §5.2.1: without ffmpeg there is no merge and
    // no Opus extraction, and letting yt-dlp "succeed" with two split files
    // would be a worse lie than a clear message. Measured 2026-10-14: with
    // `--ffmpeg-location /nonexistent`, yt-dlp exits 0 and writes `x.f278.webm`
    // + `x.f251.webm`.
    if env.ffmpeg.is_none() {
        return JobOutcome::Failed(YtdlpError::new(
            YtdlpErrorCode::FfmpegMissing,
            "ffmpeg was not found (spec §5.2.1): merging video and audio, and the opus \
             conversion, both need it. Install it, or point the app at your own copy \
             from Settings → Tools.",
        ));
    }

    let now = jsruntime::now_unix();
    let mut runtime = env.runtimes.start(env.memory.get().as_ref(), now);
    let mut fell_back = false;
    let mut omit_format = false;
    let mut format_retried = false;
    let mut degraded_retry_done = false;
    let mut impersonate_chrome = request.spec.impersonate_chrome;
    let mut impersonate_retried = false;

    emitter.emit(JobProgress::label(JobStage::Preparing, "preparing"));

    loop {
        if env.runtimes.path_of(runtime).is_none() {
            // Only reachable for "Deno only" and for Auto without a QuickJS
            // binary; Auto itself never picks a runtime it does not have.
            return JobOutcome::Failed(env.runtimes.missing_runtime_error(runtime));
        }

        let started = Instant::now();
        let attempt = run_attempt(
            env,
            request,
            runtime,
            omit_format,
            impersonate_chrome,
            token.clone(),
            &emitter,
        )
        .await;

        match attempt {
            AttemptOutcome::Finished {
                file,
                actual_height,
                format_id,
            } => {
                let requested_height = ytdlp::requested_height(&request.spec.quality);
                // A result lower than asked for, on a fresh runtime, is the
                // degraded-extraction symptom: yt-dlp's own fallback chain
                // (`…/b[height<=N]`) silently picks the muxed 360p format when
                // the separate streams are missing from that run's extraction.
                // The runtime swap is the same one-shot fallback of
                // spec §5.3.2; the wrong-quality file is removed first, so the
                // next attempt cannot resume from it.
                let degraded = ytdlp::is_degraded_selection(
                    requested_height,
                    actual_height,
                    format_id.as_deref(),
                );
                if degraded && env.runtimes.may_fallback_to_deno(runtime, fell_back) {
                    let _ = std::fs::remove_file(&file);
                    runtime = jsruntime::RuntimeKind::Deno;
                    fell_back = true;
                    continue;
                }
                if fell_back {
                    // spec §5.3.3: start with Deno for the next 24 h, or until
                    // either version changes.
                    env.memory.remember(
                        &env.runtimes.ytdlp_version,
                        env.runtimes.qjs_version(),
                        jsruntime::RuntimeKind::Deno,
                        now,
                    );
                }
                let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                return JobOutcome::Done {
                    file,
                    runtime,
                    elapsed_ms,
                    requested_height,
                    actual_height,
                };
            }
            AttemptOutcome::SkippedArchive => {
                let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                return JobOutcome::SkippedArchive {
                    runtime,
                    elapsed_ms,
                };
            }
            AttemptOutcome::Failed(error) => {
                // The one retry without `-f` (spec §8.2): the user asked for a
                // format that is not available, and yt-dlp's own fallback is the
                // best available one.
                if error.code == YtdlpErrorCode::Format && !format_retried {
                    format_retried = true;
                    omit_format = true;
                    continue;
                }
                if error.code == YtdlpErrorCode::YtChallenge {
                    match env.runtimes.fallback_decision(runtime, fell_back) {
                        FallbackDecision::Switch => {
                            runtime = jsruntime::RuntimeKind::Deno;
                            fell_back = true;
                            continue;
                        }
                        FallbackDecision::DenoMissing => {
                            return JobOutcome::Failed(
                                env.runtimes
                                    .missing_runtime_error(jsruntime::RuntimeKind::Deno),
                            );
                        }
                        FallbackDecision::No => {}
                    }
                }
                // One-shot retry with `--impersonate chrome` on ANTIBOT when
                // enabled in settings and the yt-dlp binary supports chrome
                // impersonation (spec §6.3.5, §8.5).
                if error.code == YtdlpErrorCode::Antibot
                    && request.spec.impersonate_retry
                    && !impersonate_retried
                    && !impersonate_chrome
                {
                    impersonate_retried = true;
                    if ytdlp::probe_impersonate_chrome(&env.ytdlp, env.private_temp.as_deref())
                        .await
                    {
                        impersonate_chrome = true;
                        continue;
                    }
                }
                return JobOutcome::Failed(error);
            }
            AttemptOutcome::Degraded { files } => {
                // The file that was about to be (or was) written is not what
                // the user picked: remove it and the partials, so the next
                // attempt cannot resume from the wrong bytes.
                for file in files {
                    let _ = std::fs::remove_file(file);
                }
                cleanup_job_dir(&request.dirs.temp);

                if !degraded_retry_done {
                    degraded_retry_done = true;
                    if env.runtimes.may_fallback_to_deno(runtime, fell_back) {
                        runtime = jsruntime::RuntimeKind::Deno;
                        fell_back = true;
                    }
                    // Without Deno the same runtime gets the second try: a
                    // failed challenge solve is transient, and that is exactly
                    // what a manual retry fixes.
                    continue;
                }
                return JobOutcome::Failed(YtdlpError::new(
                    YtdlpErrorCode::YtChallenge,
                    "yt-dlp selected a muxed format at a lower resolution than the one \
                     asked for (the YouTube JavaScript challenge failed in that run), so \
                     the download was stopped instead of saving the wrong file. Try \
                     again, or install Deno from Settings → Tools.",
                ));
            }
            AttemptOutcome::Stalled => match env.runtimes.fallback_decision(runtime, fell_back) {
                FallbackDecision::Switch => {
                    runtime = jsruntime::RuntimeKind::Deno;
                    fell_back = true;
                    continue;
                }
                FallbackDecision::DenoMissing => {
                    return JobOutcome::Failed(
                        env.runtimes
                            .missing_runtime_error(jsruntime::RuntimeKind::Deno),
                    );
                }
                FallbackDecision::No => {
                    return JobOutcome::Failed(YtdlpError::new(
                        YtdlpErrorCode::Network,
                        format!(
                            "the download made no progress for {} s",
                            request.stall_timeout.as_secs()
                        ),
                    ));
                }
            },
            AttemptOutcome::Cancelled => return JobOutcome::Cancelled,
        }
    }
}

/// What one attempt did.
enum AttemptOutcome {
    Finished {
        file: PathBuf,
        /// The selected height from `SDP_FORMAT`, when the template printed one.
        actual_height: Option<u32>,
        /// The selected format id (`278+251` merged, `18` muxed).
        format_id: Option<String>,
    },
    /// `--download-archive` matched this video ID: yt-dlp exited 0 without
    /// writing a file (spec §6.2.4).
    SkippedArchive,
    /// yt-dlp selected a **muxed** format lower than the one asked for: the
    /// degraded-extraction signature. The attempt is cancelled the moment
    /// `SDP_FORMAT` says so, so the wrong file is not downloaded to the end.
    Degraded {
        files: Vec<PathBuf>,
    },
    Failed(YtdlpError),
    /// The challenge watchdog fired: the runtime gets replaced, not the user
    /// bothered.
    Stalled,
    Cancelled,
}

async fn run_attempt(
    env: &JobEnvironment,
    request: &JobRequest,
    runtime: jsruntime::RuntimeKind,
    omit_format: bool,
    impersonate_chrome: bool,
    token: CancellationToken,
    emitter: &Emitter,
) -> AttemptOutcome {
    let Some(runtime_path) = env.runtimes.path_of(runtime) else {
        return AttemptOutcome::Failed(env.runtimes.missing_runtime_error(runtime));
    };

    let download_env = DownloadEnvironment {
        ffmpeg: env.ffmpeg.clone(),
        runtime: Some((runtime, runtime_path.to_path_buf())),
        tmp_env: env.private_temp.clone(),
        youtube_clients: env.youtube_clients.clone(),
    };
    let mut spec = request.spec.clone();
    spec.impersonate_chrome = impersonate_chrome;
    let args = ytdlp::download_args(
        &request.url,
        &spec,
        &request.dirs,
        &download_env,
        omit_format,
    );
    let mut command = ToolCommand::new(&env.ytdlp)
        .args(args)
        .label("yt-dlp download");
    if let Some(temp) = &env.private_temp {
        command = command.temp_dir(temp);
    }

    // The token the attempt runs under: the user's cancellation is a parent of
    // it, and the degraded-selection check cancels it directly.
    let attempt_token = token.child_token();
    let degraded = Arc::new(AtomicBool::new(false));
    let attempt = Arc::new(AttemptShared {
        tracker: Mutex::new(ProgressTracker::new(request.spec.needs_merge())),
        files: Mutex::new(Vec::new()),
        heights: Mutex::new(Vec::new()),
        format_ids: Mutex::new(Vec::new()),
        challenge_pending: Arc::new(AtomicBool::new(runtime == jsruntime::RuntimeKind::QuickJs)),
        emitter: emitter.clone(),
        requested_height: ytdlp::requested_height(&request.spec.quality),
        attempt_token: attempt_token.clone(),
        degraded: Arc::clone(&degraded),
        skipped_archive: AtomicBool::new(false),
    });

    let stdout_sink = {
        let attempt = Arc::clone(&attempt);
        process::line_sink(move |line| attempt.on_stdout_line(line))
    };
    let stderr_sink = {
        let attempt = Arc::clone(&attempt);
        process::line_sink(move |line| attempt.on_stderr_line(line))
    };

    // The stall watchdog exists only where a fallback exists: without Deno, a
    // slow extraction must be allowed to finish rather than be killed into a
    // failure the user cannot act on. It is armed only until the first download
    // line, because that is when the JS challenge is provably over — after that
    // a quiet minute is a slow network or a long merge, not a challenge.
    let watchdog = {
        let pending = Arc::clone(&attempt.challenge_pending);
        let watchdog_token = attempt_token.clone();
        let stall = request.stall_timeout;
        let armed = runtime == jsruntime::RuntimeKind::QuickJs
            && env.runtimes.fallback_decision(runtime, false) == FallbackDecision::Switch;
        armed.then(|| {
            tokio::spawn(async move {
                tokio::time::sleep(stall).await;
                if pending.load(Ordering::Relaxed) && !watchdog_token.is_cancelled() {
                    watchdog_token.cancel();
                }
            })
        })
    };

    let result = process::spawn_tool_with_sinks(
        command,
        attempt_token.clone(),
        LineSinks::new(stdout_sink, stderr_sink),
    )
    .await;
    if let Some(handle) = watchdog {
        handle.abort();
    }

    let process_result = match result {
        Ok(result) => result,
        Err(error) => {
            return AttemptOutcome::Failed(YtdlpError::new(
                YtdlpErrorCode::Unknown,
                error.to_string(),
            ));
        }
    };

    // The degraded-selection cancel comes first: it is our own, not the user's
    // and not the watchdog's.
    if degraded.load(Ordering::Relaxed) {
        return AttemptOutcome::Degraded {
            files: attempt.seen_files(),
        };
    }
    if token.is_cancelled() {
        return AttemptOutcome::Cancelled;
    }
    if attempt_token.is_cancelled() {
        // Killed by the watchdog, not by the user.
        return AttemptOutcome::Stalled;
    }
    if process_result.cancelled {
        return AttemptOutcome::Cancelled;
    }
    let file = attempt.last_file();
    if let Some(error) =
        ytdlp::classify_result(&process_result, jsruntime::is_youtube_url(&request.url))
    {
        // Spec §8.4: "Treat embedding failures as warnings, not job failures
        // (support differs per container; verify)." If an embedding option was
        // requested, the media file was already successfully downloaded/merged
        // onto disk, and yt-dlp failed only during post-processing (Unknown
        // code), tolerate the embedding failure as a non-fatal warning.
        let is_embed_warning = (request.spec.embed_metadata
            || request.spec.embed_thumbnail
            || request.spec.embed_chapters)
            && file.as_ref().is_some_and(|f| f.is_file())
            && matches!(error.code, YtdlpErrorCode::Unknown);

        if !is_embed_warning {
            return AttemptOutcome::Failed(error);
        }
    }
    match file {
        Some(file) if file.is_file() => {
            let actual_height = attempt.actual_height();
            let format_id = attempt.selected_format_id();
            // The early cancel did not get there in time and the wrong file
            // completed: same verdict, and `run_job` removes it before the
            // retry.
            if ytdlp::is_degraded_selection(
                attempt.requested_height,
                actual_height,
                format_id.as_deref(),
            ) {
                return AttemptOutcome::Degraded {
                    files: attempt.seen_files(),
                };
            }
            AttemptOutcome::Finished {
                file,
                actual_height,
                format_id,
            }
        }
        Some(file) => AttemptOutcome::Failed(YtdlpError::new(
            YtdlpErrorCode::Unknown,
            format!(
                "yt-dlp reported {} but the file is not there (spec §8.3: the path is never guessed)",
                file.display()
            ),
        )),
        None if attempt.skipped_archive.load(Ordering::Relaxed) => AttemptOutcome::SkippedArchive,
        None => AttemptOutcome::Failed(YtdlpError::new(
            YtdlpErrorCode::Unknown,
            "yt-dlp exited 0 but never printed the final path (SDP_FILE)",
        )),
    }
}

/// The state both reader tasks write into.
struct AttemptShared {
    tracker: Mutex<ProgressTracker>,
    files: Mutex<Vec<PathBuf>>,
    /// Heights of the formats yt-dlp said it selected (`SDP_FORMAT`).
    heights: Mutex<Vec<u32>>,
    /// Format ids of the same lines; `+` means a merge (see
    /// [`ytdlp::is_degraded_selection`]).
    format_ids: Mutex<Vec<String>>,
    challenge_pending: Arc<AtomicBool>,
    emitter: Emitter,
    /// What the user's choice asked for; the `SDP_FORMAT` check needs it.
    requested_height: Option<u32>,
    /// Cancels the attempt when the selected format is the degraded one.
    attempt_token: CancellationToken,
    /// Set when the cancel above was because of a degraded selection (as
    /// opposed to the user or the watchdog).
    degraded: Arc<AtomicBool>,
    /// Set when `--download-archive` prints its skip line on stdout.
    skipped_archive: AtomicBool,
}

impl AttemptShared {
    fn on_stdout_line(&self, line: &str) {
        if ytdlp::is_archive_skip_line(line) {
            self.challenge_pending.store(false, Ordering::Relaxed);
            self.skipped_archive.store(true, Ordering::Relaxed);
            return;
        }
        if let Some(last) = ytdlp::parse_file_line(line) {
            if let Ok(mut files) = self.files.lock() {
                files.push(last);
            }
            return;
        }
        if let Some(format) = ytdlp::parse_format_line(line) {
            // A format line with no height is the audio stream of an audio-only
            // job: nothing to compare there, but the id still says "muxed".
            if let Ok(mut ids) = self.format_ids.lock() {
                ids.push(format.format_id.clone());
            }
            if let Some(height) = format.height
                && let Ok(mut heights) = self.heights.lock()
            {
                heights.push(height);
            }
            self.challenge_pending.store(false, Ordering::Relaxed);
            // The decisive check, before the bytes: what exactly will be
            // downloaded? A muxed format at a lower height than the one asked
            // for is the degraded extraction, and it must not be finalized.
            if ytdlp::is_degraded_selection(
                self.requested_height,
                format.height,
                Some(format.format_id.as_str()),
            ) {
                self.degraded.store(true, Ordering::Relaxed);
                self.attempt_token.cancel();
            }
            return;
        }
        if let Some(parsed) = ytdlp::parse_download_line(line) {
            // The first download line proves the extraction phase is over.
            self.challenge_pending.store(false, Ordering::Relaxed);
            let progress = match self.tracker.lock() {
                Ok(mut tracker) => tracker.on_download(&parsed),
                Err(poisoned) => poisoned.into_inner().on_download(&parsed),
            };
            self.emitter.emit(progress);
        }
    }

    fn on_stderr_line(&self, line: &str) {
        if ytdlp::is_archive_skip_line(line) {
            self.challenge_pending.store(false, Ordering::Relaxed);
            self.skipped_archive.store(true, Ordering::Relaxed);
            return;
        }
        if let Some((status, name)) = ytdlp::parse_postprocess_line(line) {
            self.challenge_pending.store(false, Ordering::Relaxed);
            let progress = match self.tracker.lock() {
                Ok(mut tracker) => tracker.on_postprocess(&status, &name),
                Err(poisoned) => poisoned.into_inner().on_postprocess(&status, &name),
            };
            if let Some(progress) = progress {
                self.emitter.emit(progress);
            }
        }
    }

    /// The last `SDP_FILE` path. A merge writes one; a split download would
    /// print one per stream, and the caller refused to start without ffmpeg, so
    /// the last one is the final output in both shapes.
    fn last_file(&self) -> Option<PathBuf> {
        match self.files.lock() {
            Ok(files) => files.last().cloned(),
            Err(poisoned) => poisoned.into_inner().last().cloned(),
        }
    }

    /// The height yt-dlp reported it selected: the largest one seen, which is
    /// the video stream when there was a merge.
    fn actual_height(&self) -> Option<u32> {
        match self.heights.lock() {
            Ok(heights) => heights.iter().copied().max(),
            Err(poisoned) => poisoned.into_inner().iter().copied().max(),
        }
    }

    /// The format id of the last `SDP_FORMAT` line seen.
    fn selected_format_id(&self) -> Option<String> {
        match self.format_ids.lock() {
            Ok(ids) => ids.last().cloned(),
            Err(poisoned) => poisoned.into_inner().last().cloned(),
        }
    }

    /// Every file path yt-dlp reported (`SDP_FILE`), for cleanup after a
    /// rejected attempt.
    fn seen_files(&self) -> Vec<PathBuf> {
        match self.files.lock() {
            Ok(files) => files.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

/// Coalesces progress to ~10/s, but never hides a stage change or a label change
/// (spec §7.6, §11.7).
#[derive(Clone)]
struct Emitter {
    sink: ProgressSink,
    state: Arc<Mutex<EmitterState>>,
}

struct EmitterState {
    throttle: ProgressThrottle,
    last: Option<(JobStage, String)>,
}

impl Emitter {
    fn new(sink: ProgressSink) -> Self {
        Self {
            sink,
            state: Arc::new(Mutex::new(EmitterState {
                throttle: ProgressThrottle::new(Duration::from_millis(100)),
                last: None,
            })),
        }
    }

    fn emit(&self, progress: JobProgress) {
        let key = (progress.stage, progress.label.clone());
        let now = Instant::now();
        let should_send = {
            let mut state = match self.state.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            if state.last.as_ref() != Some(&key) {
                state.last = Some(key);
                // A stage change must go out even inside the coalescing window,
                // and it still counts as this window's emission.
                state.throttle.mark_emitted(now);
                true
            } else {
                state.throttle.should_emit(now)
            }
        };
        if should_send {
            // Outside the lock: the sink emits a Tauri event.
            (self.sink)(&progress);
        }
    }
}

// ---------------------------------------------------------------------------
// Temp directories (spec §4.9, §4.10, §7.3)
// ---------------------------------------------------------------------------

/// Remove a job's temp dir; failure is not fatal (best effort, like the spec's
/// "remove temp dirs" — a locked file on Windows must not fail the job).
pub fn cleanup_job_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

/// Remove stale `.sdp-tmp-*` directories under an output folder (spec §4.10).
///
/// `owned` holds the item ids the queue still owns (queued, running, paused,
/// interrupted): spec §4.10 says to remove "only those belonging to no
/// queued/interrupted item", and spec §7.4 wants an interrupted item to resume
/// from its `.part` files — which live inside its temp directory. Anything not
/// carrying the prefix is never touched: the output folder is the user's.
pub fn cleanup_stale_temp_dirs(output_dir: &Path, owned: &[String]) -> usize {
    let Ok(entries) = fs::read_dir(output_dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(id) = name.strip_prefix(ITEM_TEMP_PREFIX) else {
            continue;
        };
        if owned.iter().any(|owned| owned == id) {
            continue;
        }
        if path.is_dir() && fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn cleanup_removes_only_prefixed_directories_that_nobody_owns() {
        let dir = TempDir::new().expect("temp dir");
        let keep = dir.path().join("finished-video.mp4");
        fs::write(&keep, b"x").expect("write");
        fs::create_dir(dir.path().join(".sdp-tmp-a")).expect("mkdir");
        fs::create_dir(dir.path().join(".sdp-tmp-b")).expect("mkdir");
        fs::create_dir(dir.path().join("something-else")).expect("mkdir");

        // `b` belongs to an interrupted item, so it must survive (spec §7.4).
        let removed = cleanup_stale_temp_dirs(dir.path(), &["b".to_owned()]);
        assert_eq!(removed, 1);
        assert!(!dir.path().join(".sdp-tmp-a").exists());
        assert!(
            dir.path().join(".sdp-tmp-b").exists(),
            "an interrupted item resumes from its own temp dir"
        );
        assert!(dir.path().join("something-else").exists());
        assert!(keep.exists(), "finished outputs are never touched");
    }

    #[test]
    fn cleanup_of_a_missing_directory_is_a_no_op() {
        let dir = TempDir::new().expect("temp dir");
        assert_eq!(cleanup_stale_temp_dirs(&dir.path().join("nope"), &[]), 0);
        cleanup_job_dir(&dir.path().join("nope"));
    }

    #[test]
    fn the_emitter_coalesces_but_never_loses_a_stage_change() {
        let seen: Arc<Mutex<Vec<JobProgress>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: ProgressSink = {
            let seen = Arc::clone(&seen);
            Arc::new(move |progress: &JobProgress| {
                if let Ok(mut lines) = seen.lock() {
                    lines.push(progress.clone());
                }
            })
        };
        let emitter = Emitter::new(sink);

        // Same stage and label: only the first one is guaranteed (the throttle
        // may or may not let the rest through, but they are identical anyway).
        for _ in 0..50 {
            emitter.emit(JobProgress {
                stage: JobStage::Downloading,
                percent: Some(1.0),
                speed_bps: None,
                eta_sec: None,
                label: "video".into(),
                sub_index: Some(1),
                sub_total: Some(2),
            });
        }
        emitter.emit(JobProgress::label(JobStage::PostProcessing, "merging"));

        let seen = seen.lock().expect("lines").clone();
        assert_eq!(seen.len(), 2, "stage changes are never coalesced away");
        assert_eq!(seen[0].stage, JobStage::Downloading);
        assert_eq!(seen[1].label, "merging");
    }
}
