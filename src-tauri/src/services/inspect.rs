//! Link inspection: one `yt-dlp -J --flat-playlist` call classifies a URL
//! (spec §6.2).
//!
//! With `--flat-playlist` a single video returns full detail (formats), while a
//! playlist returns a light list. M1 only needs the classification and the
//! video details; the playlist entries are an M3 concern and arrive here as a
//! stub that already carries what the UI needs to say "playlist, N entries,
//! later".
//!
//! Two rules from the spec shape this file:
//!
//! * **never fail just because heights are missing** (spec §6.2, and the
//!   original's assumption is spec §12.15): an audio-only source or a generic
//!   site without heights produces an empty resolution list, which the UI shows
//!   as "Best available + Audio only";
//! * the resolution list excludes storyboards/`mhtml` (spec §6.2), which is
//!   tested against a real 4K capture where those formats are present.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::ErrorInfo;
use crate::services::jsruntime::{self, RuntimeKind, RuntimeMemory, RuntimeMemoryStore};
use crate::services::process::{self, LineSink, ToolCommand};
use crate::services::ytdlp::{self, CookieSource, YtdlpError, YtdlpErrorCode};

// ---------------------------------------------------------------------------
// Timeouts (spec §6.2)
// ---------------------------------------------------------------------------

/// A single video: 60 s.
pub const VIDEO_INSPECT_TIMEOUT: Duration = Duration::from_secs(60);
/// A playlist: 120 s, because enumeration is slower and legitimately so.
pub const PLAYLIST_INSPECT_TIMEOUT: Duration = Duration::from_secs(120);

/// Does this URL look like a playlist/channel, i.e. one that may take longer
/// than a single video to enumerate?
///
/// Deliberately a cheap, pure heuristic: the URL has to be classified *before*
/// the inspection that would tell us, and the cost of a wrong answer is only
/// which timeout applies. A `list=` query on a `watch?v=` URL counts: that is
/// the "video + list" case of spec §6.4, and the inspection has to resolve the
/// video anyway.
pub fn looks_like_playlist(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    [
        "list=",
        "/playlist",
        "/videos",
        "/shorts",
        "/streams",
        "/@",
        "/channel/",
        "/c/",
        "/user/",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub fn inspect_timeout(url: &str) -> Duration {
    if looks_like_playlist(url) {
        PLAYLIST_INSPECT_TIMEOUT
    } else {
        VIDEO_INSPECT_TIMEOUT
    }
}

// ---------------------------------------------------------------------------
// The result model (spec §6.2)
// ---------------------------------------------------------------------------

/// One format, with only the fields spec §6.2 asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FormatLite {
    pub format_id: String,
    pub ext: String,
    pub height: Option<u32>,
    pub width: Option<u32>,
    pub fps: Option<f64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub filesize: Option<u64>,
    pub protocol: Option<String>,
    pub note: Option<String>,
}

impl FormatLite {
    /// Storyboards (`sb0`, `sb3`, …) are images dressed as video. Excluded from
    /// everything the user sees (spec §6.2).
    pub fn is_storyboard(&self) -> bool {
        self.format_id.starts_with("sb")
            || self.ext == "mhtml"
            || self.protocol.as_deref() == Some("mhtml")
    }

    pub fn has_video(&self) -> bool {
        !self.is_storyboard() && self.vcodec.as_deref().is_some_and(|codec| codec != "none")
    }

    pub fn has_audio(&self) -> bool {
        !self.is_storyboard() && self.acodec.as_deref().is_some_and(|codec| codec != "none")
    }
}

/// A video with everything the temporary M1 UI needs (and the M3 UI will keep).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub duration: Option<f64>,
    /// `extractor_key`, shown as the site badge (spec §6.3.2).
    pub site: String,
    pub webpage_url: Option<String>,
    pub is_live: bool,
    pub language: Option<String>,
    pub thumbnail_url: Option<String>,
    pub formats: Vec<FormatLite>,
    /// Distinct heights of real video formats ≥ 144, descending (spec §6.2).
    pub heights: Vec<u32>,
    pub has_video: bool,
    pub has_audio: bool,
}

/// What kind of playlist a URL is (spec §6.4). The distinction is made at
/// inspection time because the UI wording differs ("playlist" vs "channel" vs
/// the endless mix warning).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaylistKind {
    Playlist,
    Channel,
    Mix,
    Other,
}

/// The M1 stub: classification and count, no entries (M3).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistStub {
    pub id: String,
    pub title: String,
    pub site: String,
    pub kind: PlaylistKind,
    pub count: Option<u32>,
    /// True when the enumeration hit `-I :1000` and more entries exist
    /// (always false in M1, where no cap is applied yet).
    pub truncated: bool,
}

/// The result of one inspection.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Inspected {
    Video(Box<VideoInfo>),
    Playlist(PlaylistStub),
    Error(ErrorInfo),
}

impl Inspected {
    /// The resolution labels of spec §6.2: `"1080p"`, descending.
    pub fn resolution_labels(&self) -> Vec<String> {
        match self {
            Self::Video(video) => video.heights.iter().map(|h| format!("{h}p")).collect(),
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Raw JSON (only the needed fields)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
struct RawFormat {
    format_id: Option<serde_json::Value>,
    ext: Option<String>,
    height: Option<u32>,
    width: Option<u32>,
    fps: Option<f64>,
    vcodec: Option<String>,
    acodec: Option<String>,
    filesize: Option<u64>,
    filesize_approx: Option<u64>,
    protocol: Option<String>,
    format_note: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawInfo {
    #[serde(rename = "_type")]
    info_type: Option<String>,
    id: Option<String>,
    title: Option<String>,
    duration: Option<f64>,
    extractor_key: Option<String>,
    webpage_url: Option<String>,
    is_live: Option<bool>,
    language: Option<String>,
    thumbnail: Option<String>,
    formats: Vec<RawFormat>,
    playlist_count: Option<u32>,
    entries: Option<Vec<serde_json::Value>>,
}

/// `format_id` is usually a string but YouTube numbers some formats; accept
/// both instead of failing the whole inspection over an id.
fn format_id(value: &Option<serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Pure parsing
// ---------------------------------------------------------------------------

/// Parse the JSON produced by `-J`. Never panics, never returns `Err`: a bad
/// document becomes [`Inspected::Error`] with a readable detail.
pub fn parse_inspect_json(raw: &str) -> Inspected {
    let trimmed = raw.trim();
    let info: RawInfo = match serde_json::from_str(trimmed) {
        Ok(info) => info,
        Err(first_error) => {
            // `null` is what the pinned yt-dlp prints when an extractor failed
            // without a traceback; treat it as "no information", not as JSON
            // garbage.
            if trimmed == "null" || trimmed.is_empty() {
                return Inspected::Error(ErrorInfo {
                    code: YtdlpErrorCode::Extract.as_str().to_owned(),
                    message_key: YtdlpErrorCode::Extract.message_key().to_owned(),
                    detail: Some("yt-dlp returned no information about this URL".to_owned()),
                });
            }
            // A tool whose stdout carries a stray line before the document: find
            // the first byte that can start one and retry from there. The JSON
            // of a large video is many lines long, so "the first line that
            // starts with {" would only ever recover `{` on its own.
            if let Some(start) = trimmed.find('{')
                && let Ok(recovered) = serde_json::from_str::<RawInfo>(&trimmed[start..])
            {
                return classify_info(recovered);
            }
            return Inspected::Error(ErrorInfo {
                code: YtdlpErrorCode::Extract.as_str().to_owned(),
                message_key: YtdlpErrorCode::Extract.message_key().to_owned(),
                detail: Some(format!(
                    "could not parse the inspection JSON: {first_error}"
                )),
            });
        }
    };
    classify_info(info)
}

fn classify_info(info: RawInfo) -> Inspected {
    if info.info_type.as_deref() == Some("playlist") {
        let site = info.extractor_key.clone().unwrap_or_default();
        let url = info.webpage_url.clone().unwrap_or_default();
        let id = info.id.clone().unwrap_or_default();
        let count = info
            .playlist_count
            .or_else(|| info.entries.as_ref().map(|e| e.len() as u32));
        return Inspected::Playlist(PlaylistStub {
            kind: playlist_kind(&site, &id, &url),
            id,
            title: info.title.unwrap_or_default(),
            site,
            count,
            truncated: false,
        });
    }

    // Anything that carries formats is a video; `_type` alone is not trusted,
    // because extractors have historically omitted it.
    if info.info_type.as_deref() == Some("video") || !info.formats.is_empty() {
        let formats: Vec<FormatLite> = info
            .formats
            .iter()
            .map(|raw| FormatLite {
                format_id: format_id(&raw.format_id),
                ext: raw.ext.clone().unwrap_or_default(),
                height: raw.height,
                width: raw.width,
                fps: raw.fps,
                vcodec: raw.vcodec.clone(),
                acodec: raw.acodec.clone(),
                filesize: raw.filesize.or(raw.filesize_approx),
                protocol: raw.protocol.clone(),
                note: raw.format_note.clone(),
            })
            .collect();
        let heights = resolution_heights(&formats);
        let has_video = formats.iter().any(FormatLite::has_video);
        let has_audio = formats.iter().any(FormatLite::has_audio);
        return Inspected::Video(Box::new(VideoInfo {
            id: info.id.unwrap_or_default(),
            title: info.title.unwrap_or_default(),
            duration: info.duration,
            site: info.extractor_key.unwrap_or_default(),
            webpage_url: info.webpage_url,
            is_live: info.is_live.unwrap_or(false),
            language: info.language,
            thumbnail_url: info.thumbnail,
            formats,
            heights,
            has_video,
            has_audio,
        }));
    }

    Inspected::Error(ErrorInfo {
        code: YtdlpErrorCode::Unsupported.as_str().to_owned(),
        message_key: YtdlpErrorCode::Unsupported.message_key().to_owned(),
        detail: Some("this URL did not resolve to a video or a playlist".to_owned()),
    })
}

/// The resolution list of spec §6.2.
///
/// First pass: distinct heights ≥ 144 among formats that really have a video
/// codec, storyboards excluded, descending. If that is empty — a generic site
/// or an extractor that omits codecs — retry **without the codec condition**,
/// as the original did. If there is still nothing, the answer is an empty list:
/// the UI then offers "Best available" and "Audio only", which is exactly what
/// the spec asks for over failing.
pub fn resolution_heights(formats: &[FormatLite]) -> Vec<u32> {
    let with_codec: Vec<u32> = formats
        .iter()
        .filter(|format| format.has_video())
        .filter_map(|format| format.height)
        .filter(|height| *height >= 144)
        .collect();
    let chosen = if with_codec.is_empty() {
        formats
            .iter()
            .filter(|format| !format.is_storyboard())
            .filter_map(|format| format.height)
            .filter(|height| *height >= 144)
            .collect()
    } else {
        with_codec
    };
    let mut heights: Vec<u32> = chosen;
    heights.sort_unstable();
    heights.dedup();
    heights.reverse();
    heights
}

/// Classify a playlist URL (spec §6.4). Mixes are the `RD…` radio lists: they
/// are dynamic and effectively endless, which the UI warns about.
pub fn playlist_kind(extractor_key: &str, id: &str, url: &str) -> PlaylistKind {
    let lower_url = url.to_ascii_lowercase();
    if id.starts_with("RD") || lower_url.contains("list=rd") {
        return PlaylistKind::Mix;
    }
    if extractor_key.eq_ignore_ascii_case("youtubetab") || lower_url.contains("/@") {
        return PlaylistKind::Channel;
    }
    if extractor_key.eq_ignore_ascii_case("youtube") {
        return PlaylistKind::Playlist;
    }
    PlaylistKind::Other
}

// ---------------------------------------------------------------------------
// Running the inspection
// ---------------------------------------------------------------------------

/// What a fallback trigger should do (spec §5.3.2, §5.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackDecision {
    /// Swap to Deno for the next attempt.
    Switch,
    /// The trigger fired but Deno is not installed: the "ask once" case of
    /// spec §5.3.2, which the UI answers with an install offer.
    DenoMissing,
    /// The trigger is not actionable: quickjs-only mode, or the fallback has
    /// already run.
    No,
}

/// A resolved runtime with the version its memory key needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRuntime {
    pub path: PathBuf,
    pub version: Option<String>,
}

/// Everything the inspection needs that is not user options.
#[derive(Debug, Clone)]
pub struct Runtimes {
    pub mode: jsruntime::JsRuntimeMode,
    /// The bundled QuickJS, when it could be resolved.
    pub qjs: Option<ResolvedRuntime>,
    /// Deno, when the user installed it (it is never bundled).
    pub deno: Option<ResolvedRuntime>,
    /// The yt-dlp version the decision is keyed by.
    pub ytdlp_version: String,
}

impl Runtimes {
    /// Which runtime this attempt should use (spec §5.3.3).
    pub fn start(&self, memory: Option<&RuntimeMemory>, now: u64) -> RuntimeKind {
        jsruntime::choose_start_runtime(
            self.mode,
            memory,
            &self.ytdlp_version,
            self.qjs.as_ref().and_then(|r| r.version.as_deref()),
            self.qjs.is_some(),
            now,
        )
    }

    pub fn path_of(&self, kind: RuntimeKind) -> Option<&Path> {
        match kind {
            RuntimeKind::QuickJs => self.qjs.as_ref().map(|r| r.path.as_path()),
            RuntimeKind::Deno => self.deno.as_ref().map(|r| r.path.as_path()),
        }
    }

    /// The automatic fallback of spec §5.3.2 only exists in Auto mode.
    pub fn fallback_allowed(&self) -> bool {
        self.mode == jsruntime::JsRuntimeMode::Auto
    }

    /// May this run switch from the current runtime to Deno right now?
    ///
    /// One shot, Auto mode only, QuickJS → Deno, and only when Deno really is
    /// there (an uninstalled fallback is reported as `DENO_MISSING` so the UI
    /// can offer to install it — never attempted blindly).
    pub fn may_fallback_to_deno(&self, current: RuntimeKind, already_fell_back: bool) -> bool {
        self.fallback_decision(current, already_fell_back) == FallbackDecision::Switch
    }

    /// The three-way decision behind both callers (inspection and download).
    pub fn fallback_decision(
        &self,
        current: RuntimeKind,
        already_fell_back: bool,
    ) -> FallbackDecision {
        if !self.fallback_allowed() || already_fell_back || current != RuntimeKind::QuickJs {
            return FallbackDecision::No;
        }
        if self.deno.is_some() {
            FallbackDecision::Switch
        } else {
            FallbackDecision::DenoMissing
        }
    }

    /// The requested runtime is missing and cannot be substituted.
    pub fn missing_runtime_error(&self, kind: RuntimeKind) -> YtdlpError {
        match kind {
            RuntimeKind::Deno => YtdlpError::new(
                YtdlpErrorCode::DenoMissing,
                "QuickJS could not solve the YouTube challenge and Deno, the automatic \
                 fallback, is not installed (spec §5.3). Install it from Settings → Tools.",
            ),
            RuntimeKind::QuickJs => YtdlpError::new(
                YtdlpErrorCode::YtChallenge,
                "QuickJS is required in this mode but is not available; check Settings → Tools.",
            ),
        }
    }

    pub fn qjs_version(&self) -> Option<&str> {
        self.qjs.as_ref().and_then(|r| r.version.as_deref())
    }
}

/// The environment of an inspection: the resolved binary, the runtimes, the
/// cookies and where the memory is written.
#[derive(Clone)]
pub struct InspectEnvironment {
    pub ytdlp: PathBuf,
    pub cookies: CookieSource,
    pub runtimes: Runtimes,
    pub memory: Arc<RuntimeMemoryStore>,
    /// Spec §4.9: the private TMP dir for QuickJS.
    pub private_temp: Option<PathBuf>,
}

/// The argv array of the inspect command (spec §6.2), URL last after `--`.
pub fn inspect_args(
    url: &str,
    runtime: Option<(RuntimeKind, &Path)>,
    cookies: &CookieSource,
) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = Vec::new();
    for flag in ["--ignore-config", "-J", "--flat-playlist", "--no-warnings"] {
        args.push(flag.into());
    }
    if let Some((kind, path)) = runtime {
        args.extend(jsruntime::runtime_args(kind, path));
    }
    if let Some(extractor_args) = jsruntime::youtube_extractor_args(url) {
        args.push("--extractor-args".into());
        args.push(extractor_args.into());
    }
    args.extend(cookies.args());
    args.push("--".into());
    args.push(url.into());
    args
}

/// What an inspection returned.
#[derive(Debug)]
pub enum InspectOutcome {
    Inspected {
        inspected: Box<Inspected>,
        /// Which runtime produced it (spec §15 item 3: the UI must show it).
        runtime: RuntimeKind,
        elapsed_ms: u64,
    },
    Cancelled,
}

/// The one public video the self-test uses (spec §5.3.6).
///
/// "Me at the zoo", the first YouTube video, published 2005: chosen for
/// stability, verified live with the pinned yt-dlp and the bundled qjs on
/// 2026-10-14 (the previous candidate, `BaW_jenozKc`, answers "This video is
/// unavailable" today — a known-good id is not known-good for ever, which is
/// why the check reports elapsed time and formats instead of trusting a name).
pub const SELF_TEST_URL: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";

/// The result of a "Test YouTube extraction" run (spec §5.3.6).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfTestReport {
    pub ok: bool,
    /// `quickjs` | `deno`, when an inspection ran at all.
    pub runtime: Option<String>,
    pub elapsed_ms: u64,
    pub formats: usize,
    pub heights: Vec<u32>,
    pub error: Option<ErrorInfo>,
}

/// Run the self-test: a known public video through `-J`, reporting the runtime
/// and the elapsed time (spec §5.3.6). The fallback applies exactly as in a
/// real inspection, so the report describes the strategy the app would use.
pub async fn self_test(env: &InspectEnvironment, token: CancellationToken) -> SelfTestReport {
    match inspect(env, SELF_TEST_URL, token).await {
        InspectOutcome::Cancelled => SelfTestReport {
            ok: false,
            runtime: None,
            elapsed_ms: 0,
            formats: 0,
            heights: Vec::new(),
            error: Some(ErrorInfo {
                code: "CANCELLED".to_owned(),
                message_key: "error.cancelled".to_owned(),
                detail: None,
            }),
        },
        InspectOutcome::Inspected {
            inspected,
            runtime,
            elapsed_ms,
        } => match &*inspected {
            Inspected::Video(video) => SelfTestReport {
                ok: true,
                runtime: Some(runtime.as_str().to_owned()),
                elapsed_ms,
                formats: video.formats.len(),
                heights: video.heights.clone(),
                error: None,
            },
            Inspected::Playlist(_) => SelfTestReport {
                ok: false,
                runtime: Some(runtime.as_str().to_owned()),
                elapsed_ms,
                formats: 0,
                heights: Vec::new(),
                error: Some(ErrorInfo {
                    code: YtdlpErrorCode::Unsupported.as_str().to_owned(),
                    message_key: YtdlpErrorCode::Unsupported.message_key().to_owned(),
                    detail: Some("the self-test URL resolved to a playlist".to_owned()),
                }),
            },
            Inspected::Error(info) => SelfTestReport {
                ok: false,
                runtime: Some(runtime.as_str().to_owned()),
                elapsed_ms,
                formats: 0,
                heights: Vec::new(),
                error: Some(info.clone()),
            },
        },
    }
}

/// Inspect `url`, including the one-shot QuickJS→Deno fallback of spec §5.3.2.
///
/// Fallback triggers for an inspection: a classified `YT_CHALLENGE`, or a
/// **successful** run that found video formats but no usable heights (the
/// "zero video formats with a height" condition), which is what degraded
/// extraction looks like from out here.
pub async fn inspect(
    env: &InspectEnvironment,
    url: &str,
    token: CancellationToken,
) -> InspectOutcome {
    let now = jsruntime::now_unix();
    let mut runtime = env.runtimes.start(env.memory.get().as_ref(), now);
    let mut fell_back = false;
    let mut elapsed_ms = 0_u64;

    loop {
        let started = std::time::Instant::now();
        let outcome = run_once(env, url, runtime, token.clone()).await;
        elapsed_ms += u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match outcome {
            RunOutcome::Cancelled => return InspectOutcome::Cancelled,
            RunOutcome::Inspected(inspected) => {
                let degraded = matches!(
                    &*inspected,
                    Inspected::Video(video) if video.has_video && video.heights.is_empty()
                );
                if degraded && env.runtimes.may_fallback_to_deno(runtime, fell_back) {
                    runtime = RuntimeKind::Deno;
                    fell_back = true;
                    continue;
                }
                if fell_back {
                    remember_deno(env, now);
                }
                return InspectOutcome::Inspected {
                    inspected,
                    runtime,
                    elapsed_ms,
                };
            }
            RunOutcome::Failed(error) => {
                if error.code == YtdlpErrorCode::YtChallenge
                    && env.runtimes.may_fallback_to_deno(runtime, fell_back)
                {
                    runtime = RuntimeKind::Deno;
                    fell_back = true;
                    continue;
                }
                return InspectOutcome::Inspected {
                    inspected: Box::new(Inspected::Error(error.to_error_info())),
                    runtime,
                    elapsed_ms,
                };
            }
        }
    }
}

fn remember_deno(env: &InspectEnvironment, now: u64) {
    env.memory.remember(
        &env.runtimes.ytdlp_version,
        env.runtimes.qjs_version(),
        RuntimeKind::Deno,
        now,
    );
}

enum RunOutcome {
    Inspected(Box<Inspected>),
    Failed(YtdlpError),
    Cancelled,
}

async fn run_once(
    env: &InspectEnvironment,
    url: &str,
    runtime: RuntimeKind,
    token: CancellationToken,
) -> RunOutcome {
    let Some(runtime_path) = env.runtimes.path_of(runtime) else {
        // The chosen runtime is not installed at all (Auto never chooses a
        // missing Deno; explicit modes and Auto-without-qjs can).
        let error = YtdlpError::new(
            YtdlpErrorCode::YtChallenge,
            format!(
                "the {} runtime is not available; install it from Settings → Tools",
                runtime.as_str()
            ),
        );
        return RunOutcome::Failed(error);
    };

    let args = inspect_args(url, Some((runtime, runtime_path)), &env.cookies);
    let mut command = ToolCommand::new(&env.ytdlp)
        .args(args)
        .label("yt-dlp inspect");
    if let Some(temp) = &env.private_temp {
        command = command.temp_dir(temp);
    }

    // `-J` writes one big JSON document; collect it line by line.
    let buffer = Arc::new(Mutex::new(String::new()));
    let sink: LineSink = {
        let buffer = Arc::clone(&buffer);
        process::line_sink(move |line| {
            if let Ok(mut text) = buffer.lock() {
                text.push_str(line);
                text.push('\n');
            }
        })
    };

    let deadline = inspect_timeout(url);
    let result =
        tokio::time::timeout(deadline, process::spawn_tool(command, token.clone(), sink)).await;

    let process_result = match result {
        Ok(Ok(result)) => result,
        // Timeout: the future was dropped, and dropping a `RunningTool` kills
        // the tree (spec §4.2), so nothing is left behind.
        Err(_elapsed) => {
            return RunOutcome::Failed(YtdlpError::new(
                YtdlpErrorCode::Network,
                format!(
                    "the inspection did not finish within {} s (spec §6.2)",
                    deadline.as_secs()
                ),
            ));
        }
        Ok(Err(error)) => {
            return RunOutcome::Failed(YtdlpError::new(YtdlpErrorCode::Extract, error.to_string()));
        }
    };

    if process_result.cancelled {
        // Cancelled by the user (the caller's token) or the timeout path above.
        return RunOutcome::Cancelled;
    }

    if let Some(error) = ytdlp::classify_result(&process_result, jsruntime::is_youtube_url(url)) {
        return RunOutcome::Failed(error);
    }

    let raw = match buffer.lock() {
        Ok(text) => text.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    RunOutcome::Inspected(Box::new(parse_inspect_json(&raw)))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4K capture from the pinned yt-dlp, trimmed to the fields this module
    /// reads. Storyboards are really in it (the real capture had `sb0`…`sb3`
    /// with `mhtml`), as is a vertical format and audio-only formats.
    const YOUTUBE_4K: &str = r#"{
      "_type": "video",
      "id": "dQw4w9WgXcQ",
      "title": "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)",
      "duration": 213.0,
      "extractor_key": "Youtube",
      "webpage_url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
      "is_live": false,
      "language": null,
      "thumbnail": "https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg",
      "formats": [
        {"format_id": "233", "ext": "mp4", "vcodec": "none", "acodec": null, "protocol": "m3u8_native"},
        {"format_id": "140", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "filesize": 309288, "protocol": "https"},
        {"format_id": "251", "ext": "webm", "vcodec": "none", "acodec": "opus", "filesize": 252182, "protocol": "https"},
        {"format_id": "160", "ext": "mp4", "height": 144, "width": 256, "fps": 15, "vcodec": "avc1.4D400B", "acodec": "none", "filesize": 195278, "protocol": "https"},
        {"format_id": "278", "ext": "webm", "height": 144, "width": 256, "fps": 15, "vcodec": "vp9", "acodec": "none", "protocol": "https"},
        {"format_id": "137", "ext": "mp4", "height": 1080, "width": 1920, "fps": 30, "vcodec": "avc1.640028", "acodec": "none", "filesize": 20573888, "protocol": "https"},
        {"format_id": "248", "ext": "webm", "height": 1080, "width": 1920, "fps": 30, "vcodec": "vp9", "acodec": "none", "protocol": "https"},
        {"format_id": "271", "ext": "webm", "height": 1440, "width": 2560, "fps": 30, "vcodec": "vp9", "acodec": "none", "protocol": "https"},
        {"format_id": "313", "ext": "webm", "height": 2160, "width": 3840, "fps": 30, "vcodec": "vp9", "acodec": "none", "protocol": "https"},
        {"format_id": "598", "ext": "mp4", "height": 144, "width": 256, "fps": 15.0, "vcodec": "av01.0.00M.08", "acodec": "none", "protocol": "m3u8_native"},
        {"format_id": "18", "ext": "mp4", "height": 360, "width": 640, "fps": 30, "vcodec": "avc1.42001E", "acodec": "mp4a.40.2", "filesize_approx": 7345000, "protocol": "https"},
        {"format_id": "sb0", "ext": "mhtml", "vcodec": "none", "acodec": "none", "protocol": "mhtml"},
        {"format_id": "sb3", "ext": "mhtml", "vcodec": "none", "acodec": "none", "protocol": "mhtml"}
      ]
    }"#;

    #[test]
    fn a_video_json_becomes_the_video_model_with_the_sorted_ladder() {
        let parsed = parse_inspect_json(YOUTUBE_4K);
        let Inspected::Video(video) = &parsed else {
            panic!("expected a video, got {parsed:?}");
        };
        assert_eq!(video.id, "dQw4w9WgXcQ");
        assert_eq!(video.site, "Youtube");
        assert_eq!(video.duration, Some(213.0));
        assert!(!video.is_live);
        assert!(video.has_video);
        assert!(video.has_audio);
        assert_eq!(
            video.heights,
            vec![2160, 1440, 1080, 360, 144],
            "distinct heights, descending, 144 floor"
        );
        assert_eq!(
            parsed.resolution_labels(),
            vec!["2160p", "1440p", "1080p", "360p", "144p"]
        );
        // Storyboards are in the capture and must be filtered out silently.
        assert!(video.formats.iter().any(FormatLite::is_storyboard));
        // filesize falls back to filesize_approx (format 18 has only that).
        let f18 = video
            .formats
            .iter()
            .find(|format| format.format_id == "18")
            .expect("format 18");
        assert_eq!(f18.filesize, Some(7_345_000));
        // Both a video-only and a muxed format are exposed to the format table.
        assert!(!f18.vcodec.as_deref().unwrap_or_default().is_empty());
        assert_ne!(f18.acodec.as_deref(), Some("none"));
    }

    #[test]
    fn a_vertical_video_lists_the_right_heights_from_height_not_width() {
        // A Short: 1080 wide, 1920 tall. The resolution list is heights, and
        // confusing width for height is exactly the vertical-video bug of
        // spec §2.9.
        let raw = r#"{
          "_type": "video", "id": "short", "title": "vertical", "extractor_key": "Youtube",
          "formats": [
            {"format_id": "137", "ext": "mp4", "height": 1920, "width": 1080, "vcodec": "avc1", "acodec": "none"},
            {"format_id": "248", "ext": "webm", "height": 1080, "width": 608, "vcodec": "vp9", "acodec": "none"},
            {"format_id": "140", "ext": "m4a", "height": null, "width": null, "vcodec": "none", "acodec": "mp4a"}
          ]
        }"#;
        let Inspected::Video(video) = parse_inspect_json(raw) else {
            panic!("expected a video");
        };
        assert_eq!(video.heights, vec![1920, 1080]);
        assert!(video.has_video);
        assert!(video.has_audio);
    }

    #[test]
    fn an_ultrawide_2to1_height_is_kept_as_is() {
        let raw = r#"{
          "_type": "video", "id": "uw", "title": "ultrawide", "extractor_key": "Youtube",
          "formats": [
            {"format_id": "a", "ext": "webm", "height": 1440, "width": 2880, "vcodec": "vp9", "acodec": "none"},
            {"format_id": "b", "ext": "mp4", "height": 1080, "width": 2160, "vcodec": "avc1", "acodec": "none"}
          ]
        }"#;
        let Inspected::Video(video) = parse_inspect_json(raw) else {
            panic!("expected a video");
        };
        assert_eq!(video.heights, vec![1440, 1080]);
    }

    #[test]
    fn an_audio_only_source_has_an_empty_resolution_list_and_does_not_fail() {
        let raw = r#"{
          "_type": "video", "id": "podcast", "title": "an episode",
          "extractor_key": "Soundcloud", "duration": 3600,
          "formats": [
            {"format_id": "a1", "ext": "mp3", "vcodec": "none", "acodec": "mp3", "protocol": "https"}
          ]
        }"#;
        let parsed = parse_inspect_json(raw);
        let Inspected::Video(video) = &parsed else {
            panic!("expected a video");
        };
        assert!(video.heights.is_empty(), "no heights is not an error");
        assert!(video.has_audio);
        assert!(!video.has_video);
        assert!(parsed.resolution_labels().is_empty());
    }

    #[test]
    fn a_generic_site_without_video_codecs_falls_back_to_heights_alone() {
        // The original's fallback (spec §6.2): no format declares a codec, but
        // a height exists.
        let raw = r#"{
          "_type": "video", "id": "g", "title": "generic", "extractor_key": "Generic",
          "formats": [
            {"format_id": "1", "ext": "mp4", "height": 720},
            {"format_id": "2", "ext": "mp4", "height": 480}
          ]
        }"#;
        let Inspected::Video(video) = parse_inspect_json(raw) else {
            panic!("expected a video");
        };
        assert_eq!(video.heights, vec![720, 480]);
    }

    #[test]
    fn a_format_with_an_unknown_height_never_hides_the_known_ones() {
        let raw = r#"{
          "_type": "video", "id": "x", "title": "t", "extractor_key": "Generic",
          "formats": [
            {"format_id": "1", "ext": "mp4", "height": null, "vcodec": "h264", "acodec": "aac"},
            {"format_id": "2", "ext": "mp4", "height": 480, "vcodec": "h264", "acodec": "none"}
          ]
        }"#;
        let Inspected::Video(video) = parse_inspect_json(raw) else {
            panic!("expected a video");
        };
        assert_eq!(video.heights, vec![480]);
    }

    #[test]
    fn numeric_format_ids_do_not_break_the_document() {
        let raw = r#"{
          "_type": "video", "id": "n", "title": "t", "extractor_key": "Generic",
          "formats": [{"format_id": 22, "ext": "mp4", "height": 720, "vcodec": "h264", "acodec": "aac"}]
        }"#;
        let Inspected::Video(video) = parse_inspect_json(raw) else {
            panic!("expected a video");
        };
        assert_eq!(video.formats[0].format_id, "22");
    }

    #[test]
    fn a_playlist_becomes_the_stub_with_its_kind_and_count() {
        let raw = r#"{
          "_type": "playlist",
          "id": "PLabcdefghijklmnop",
          "title": "A playlist",
          "extractor_key": "Youtube",
          "webpage_url": "https://www.youtube.com/playlist?list=PLabcdefghijklmnop",
          "playlist_count": 42,
          "entries": [{"id": "a"}, {"id": "b"}]
        }"#;
        let Inspected::Playlist(playlist) = parse_inspect_json(raw) else {
            panic!("expected a playlist");
        };
        assert_eq!(playlist.kind, PlaylistKind::Playlist);
        assert_eq!(playlist.count, Some(42));
        assert_eq!(playlist.title, "A playlist");
        assert!(!playlist.truncated);
    }

    #[test]
    fn playlist_kinds_are_classified() {
        assert_eq!(
            playlist_kind(
                "Youtube",
                "PL123",
                "https://www.youtube.com/playlist?list=PL123"
            ),
            PlaylistKind::Playlist
        );
        assert_eq!(
            playlist_kind(
                "Youtube",
                "RDabc",
                "https://www.youtube.com/watch?v=x&list=RDabc"
            ),
            PlaylistKind::Mix
        );
        assert_eq!(
            playlist_kind(
                "YoutubeTab",
                "UC123",
                "https://www.youtube.com/@name/videos"
            ),
            PlaylistKind::Channel
        );
        assert_eq!(
            playlist_kind("Vimeo", "album", "https://vimeo.com/album/1"),
            PlaylistKind::Other
        );
    }

    #[test]
    fn garbage_and_null_produce_a_readable_error_instead_of_a_panic() {
        for raw in ["", "null", "{ this is not json", "[]"] {
            match parse_inspect_json(raw) {
                Inspected::Error(info) => {
                    assert!(info.detail.is_some(), "a detail is always present");
                }
                other => panic!("{raw:?} must be an error, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_document_with_a_stray_line_before_it_is_recovered() {
        let raw = format!("some warning line\n{YOUTUBE_4K}");
        let Inspected::Video(video) = parse_inspect_json(&raw) else {
            panic!("expected a video");
        };
        assert_eq!(video.id, "dQw4w9WgXcQ");
    }

    #[test]
    fn the_timeout_follows_the_url_shape() {
        assert_eq!(
            inspect_timeout("https://www.youtube.com/watch?v=x"),
            VIDEO_INSPECT_TIMEOUT
        );
        assert_eq!(
            inspect_timeout("https://www.youtube.com/playlist?list=PL1"),
            PLAYLIST_INSPECT_TIMEOUT
        );
        assert_eq!(
            inspect_timeout("https://www.youtube.com/@name/videos"),
            PLAYLIST_INSPECT_TIMEOUT
        );
        assert_eq!(
            inspect_timeout("https://www.youtube.com/watch?v=x&list=PL1"),
            PLAYLIST_INSPECT_TIMEOUT
        );
        assert!(!looks_like_playlist("https://vimeo.com/12345"));
    }

    #[test]
    fn inspect_args_put_the_url_last_and_respect_the_runtime_order() {
        let args = inspect_args(
            "https://youtu.be/x",
            Some((RuntimeKind::QuickJs, Path::new("/opt/qjs"))),
            &CookieSource::None,
        );
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        for required in ["--ignore-config", "-J", "--flat-playlist", "--no-warnings"] {
            assert!(rendered.contains(&required.to_owned()), "{rendered:?}");
        }
        let clear = rendered.iter().position(|a| a == "--no-js-runtimes");
        let enable = rendered.iter().position(|a| a == "--js-runtimes");
        assert!(clear < enable);
        assert_eq!(rendered[rendered.len() - 2], "--");
        assert_eq!(rendered[rendered.len() - 1], "https://youtu.be/x");
        assert!(rendered.contains(&jsruntime::YOUTUBE_PLAYER_CLIENTS.to_owned()));

        // A non-YouTube link gets no YouTube extractor args.
        let args = inspect_args("https://vimeo.com/1", None, &CookieSource::None);
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(!rendered.contains(&jsruntime::YOUTUBE_PLAYER_CLIENTS.to_owned()));
        assert!(!rendered.contains(&"--js-runtimes".to_owned()));
    }

    #[test]
    fn the_inspection_carries_the_cookies_it_was_given() {
        let args = inspect_args(
            "https://youtu.be/x",
            None,
            &CookieSource::Browser("firefox".into()),
        );
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(
            rendered
                .windows(2)
                .any(|pair| pair == ["--cookies-from-browser", "firefox"])
        );
    }
}
