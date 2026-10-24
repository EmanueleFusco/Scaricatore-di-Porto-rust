//! Media processing with ffmpeg: progress parsing, trim validation, argument
//! builders, hardware-to-CPU fallback, and safe file lifecycle (spec §9, §12).
//!
//! # Specification Rules
//!
//! 1. Progress parser (spec §9.1):
//!    Run with `-nostdin -hide_banner -progress pipe:1 -nostats`.
//!    Parse `out_time_us=` (microseconds; ignore `N/A`/negative values) over the
//!    expected output duration. Read stderr and check exit code.
//! 2. Encode (video) and trim (spec §9.2):
//!    Output name `<name>-encoded.mp4`.
//!    Delete original ONLY IF ffmpeg exited 0 and the output is non-empty;
//!    otherwise keep it and report error.
//!    Base command: `ffmpeg -y -nostdin -ss <start>? -t <dur>? -i <in> -c:v <venc + args> -c:a <aenc> -movflags +faststart <out>`.
//!    Put `-ss` before `-i` and use `-t (end - start)`.
//!    Trim implies Encode.
//! 3. Trim parsing & validation (spec §9.2):
//!    Accept `SS(.ms)`, `MM:SS(.ms)`, `HH:MM:SS(.ms)`. Validate `start < end`
//!    and both inside media duration. Reject garbage with an inline error.
//! 4. Audio-only encode (spec §9.4):
//!    `aac` -> `-c:a aac -b:a 192k` into `.m4a`
//!    `mp3` -> `-c:a libmp3lame -q:a 2` into `.mp3`
//!    `opus` -> `-c:a libopus -b:a 128k` into `.opus`
//!    Output `<name>-encoded.<ext>`, same delete-original and trim rules.
//! 5. Hardware encoder fallback (spec §9.3):
//!    If a hardware encoder fails at job time, retry once with the CPU encoder of
//!    the same codec family (`libx264` or `libx265`).

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::services::encoders::{
    AudioCodec, cpu_fallback_encoder, lookup_encoder, videotoolbox_fallback_args,
};
use crate::services::process::{self, ProcessResult, ToolCommand};

// ---------------------------------------------------------------------------
// Trim Parsing and Validation
// ---------------------------------------------------------------------------

/// Trim range in seconds (spec §9.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrimSpec {
    pub start: Option<f64>,
    pub end: Option<f64>,
}

impl TrimSpec {
    pub fn new(start: Option<f64>, end: Option<f64>) -> Self {
        Self { start, end }
    }

    pub fn is_empty(&self) -> bool {
        self.start.is_none() && self.end.is_none()
    }
}

/// Errors occurring during trim parsing or validation.
#[derive(Debug, Error, PartialEq)]
pub enum TrimError {
    #[error("Empty timestamp")]
    Empty,
    #[error("Invalid time format: '{0}' (expected SS, MM:SS, or HH:MM:SS)")]
    InvalidFormat(String),
    #[error("Negative time value: {0}")]
    Negative(f64),
    #[error("Start time ({0}s) must be strictly before end time ({1}s)")]
    StartNotBeforeEnd(f64, f64),
    #[error("Start time ({0}s) exceeds media duration ({1}s)")]
    StartExceedsDuration(f64, f64),
    #[error("End time ({0}s) exceeds media duration ({1}s)")]
    EndExceedsDuration(f64, f64),
}

/// Parse a time string into fractional seconds (spec §9.2).
///
/// Accepted formats:
/// - `SS` or `SS.ms` (e.g. `"45"`, `"45.5"`, `"123.456"`)
/// - `MM:SS` or `MM:SS.ms` (e.g. `"01:30"`, `"1:30.5"`)
/// - `HH:MM:SS` or `HH:MM:SS.ms` (e.g. `"00:01:30"`, `"01:02:03.450"`)
///
/// Rejects empty strings, negative numbers, and garbage.
pub fn parse_time_spec(raw: &str) -> Result<f64, TrimError> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(TrimError::Empty);
    }

    let parts: Vec<&str> = s.split(':').collect();
    let seconds = match parts.len() {
        1 => {
            // "SS" or "SS.ms"
            let sec: f64 = parts[0]
                .parse()
                .map_err(|_| TrimError::InvalidFormat(raw.to_owned()))?;
            sec
        }
        2 => {
            // "MM:SS" or "MM:SS.ms"
            let min: f64 = parts[0]
                .parse()
                .map_err(|_| TrimError::InvalidFormat(raw.to_owned()))?;
            let sec: f64 = parts[1]
                .parse()
                .map_err(|_| TrimError::InvalidFormat(raw.to_owned()))?;
            if min < 0.0 || !(0.0..60.0).contains(&sec) {
                return Err(TrimError::InvalidFormat(raw.to_owned()));
            }
            min * 60.0 + sec
        }
        3 => {
            // "HH:MM:SS" or "HH:MM:SS.ms"
            let hr: f64 = parts[0]
                .parse()
                .map_err(|_| TrimError::InvalidFormat(raw.to_owned()))?;
            let min: f64 = parts[1]
                .parse()
                .map_err(|_| TrimError::InvalidFormat(raw.to_owned()))?;
            let sec: f64 = parts[2]
                .parse()
                .map_err(|_| TrimError::InvalidFormat(raw.to_owned()))?;
            if hr < 0.0 || !(0.0..60.0).contains(&min) || !(0.0..60.0).contains(&sec) {
                return Err(TrimError::InvalidFormat(raw.to_owned()));
            }
            hr * 3600.0 + min * 60.0 + sec
        }
        _ => return Err(TrimError::InvalidFormat(raw.to_owned())),
    };

    if !seconds.is_finite() || seconds < 0.0 {
        return Err(TrimError::Negative(seconds));
    }

    Ok(seconds)
}

/// Parse optional trim strings and validate the range (spec §9.2, §12.11).
/// A malformed value is an error, never silently treated as "no trim".
pub fn parse_and_validate_trim(
    start: Option<&str>,
    end: Option<&str>,
    media_duration: Option<f64>,
) -> Result<(Option<f64>, Option<f64>), TrimError> {
    let start = start.map(parse_time_spec).transpose()?;
    let end = end.map(parse_time_spec).transpose()?;
    validate_trim(start, end, media_duration)?;
    Ok((start, end))
}

/// Validate a trim range against itself and the media duration (spec §9.2).
pub fn validate_trim(
    start: Option<f64>,
    end: Option<f64>,
    media_duration: Option<f64>,
) -> Result<(), TrimError> {
    if let Some(s) = start {
        if s < 0.0 {
            return Err(TrimError::Negative(s));
        }
    }
    if let Some(e) = end {
        if e <= 0.0 {
            return Err(TrimError::Negative(e));
        }
    }
    if let (Some(s), Some(e)) = (start, end) {
        if s >= e {
            return Err(TrimError::StartNotBeforeEnd(s, e));
        }
    }
    if let Some(total) = media_duration {
        if total > 0.0 {
            if let Some(s) = start {
                if s > total {
                    return Err(TrimError::StartExceedsDuration(s, total));
                }
            }
            if let Some(e) = end {
                if e > total {
                    return Err(TrimError::EndExceedsDuration(e, total));
                }
            }
        }
    }
    Ok(())
}

/// Calculate the expected duration in seconds for progress tracking (spec §9.2).
/// Expected duration = `(end or total) - start`.
pub fn calculate_expected_duration(
    trim: Option<&TrimSpec>,
    total_duration: Option<f64>,
) -> Option<f64> {
    let start = trim.and_then(|t| t.start).unwrap_or(0.0);
    let end = trim.and_then(|t| t.end).or(total_duration);
    end.map(|e| (e - start).max(0.0))
}

/// Format a duration in seconds into a string suitable for ffmpeg `-ss` or `-t`.
pub fn format_duration_arg(seconds: f64) -> String {
    format!("{:.3}", seconds)
}

// ---------------------------------------------------------------------------
// Progress Parser
// ---------------------------------------------------------------------------

/// Parsed progress event from ffmpeg's `-progress pipe:1` (spec §9.1).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FfmpegProgress {
    /// Position reached in seconds.
    pub out_time_sec: f64,
    /// Completion percentage (0.0 .. 100.0) if expected duration is known.
    pub percent: Option<f64>,
    /// Processing speed multiplier (e.g. `2.5` for 2.5x).
    pub speed: Option<f64>,
    /// Estimated time remaining in seconds.
    pub eta_sec: Option<u64>,
    /// True if ffmpeg emitted `progress=end`.
    pub is_end: bool,
}

/// Stateful tracker that parses `-progress pipe:1` lines and emits [`FfmpegProgress`].
pub struct FfmpegProgressTracker {
    expected_duration_sec: Option<f64>,
    current_time_sec: f64,
    current_speed: Option<f64>,
}

impl FfmpegProgressTracker {
    pub fn new(expected_duration_sec: Option<f64>) -> Self {
        Self {
            expected_duration_sec,
            current_time_sec: 0.0,
            current_speed: None,
        }
    }

    /// Parse a single line from ffmpeg `-progress pipe:1`.
    ///
    /// Returns `Some(FfmpegProgress)` when a milestone key (like `out_time_us` or `progress`)
    /// is received.
    pub fn handle_line(&mut self, line: &str) -> Option<FfmpegProgress> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut parts = trimmed.splitn(2, '=');
        let key = parts.next()?.trim();
        let value = parts.next()?.trim();

        match key {
            "out_time_us" => {
                // Microseconds: ignore N/A or negative values
                if let Ok(us) = value.parse::<i64>() {
                    if us >= 0 {
                        self.current_time_sec = us as f64 / 1_000_000.0;
                        return Some(self.build_progress(false));
                    }
                }
                None
            }
            "out_time_ms" => {
                // Some ffmpeg builds emit out_time_ms in microseconds
                if let Ok(us) = value.parse::<i64>() {
                    if us >= 0 {
                        self.current_time_sec = us as f64 / 1_000_000.0;
                        return Some(self.build_progress(false));
                    }
                }
                None
            }
            "speed" => {
                // e.g. "2.45x" or "N/A"
                if let Some(stripped) = value.strip_suffix('x') {
                    if let Ok(speed) = stripped.trim().parse::<f64>() {
                        if speed > 0.0 {
                            self.current_speed = Some(speed);
                        }
                    }
                }
                None
            }
            "progress" => {
                if value == "end" {
                    Some(self.build_progress(true))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn build_progress(&self, is_end: bool) -> FfmpegProgress {
        let percent = if is_end {
            Some(100.0)
        } else if let Some(total) = self.expected_duration_sec {
            if total > 0.0 {
                Some(((self.current_time_sec / total) * 100.0).clamp(0.0, 100.0))
            } else {
                None
            }
        } else {
            None
        };

        let eta_sec = if is_end {
            Some(0)
        } else if let (Some(total), Some(speed)) = (self.expected_duration_sec, self.current_speed)
        {
            if total > self.current_time_sec && speed > 0.0 {
                let remaining = total - self.current_time_sec;
                Some((remaining / speed).round() as u64)
            } else {
                None
            }
        } else {
            None
        };

        FfmpegProgress {
            out_time_sec: self.current_time_sec,
            percent,
            speed: self.current_speed,
            eta_sec,
            is_end,
        }
    }
}

// ---------------------------------------------------------------------------
// Argument Builders
// ---------------------------------------------------------------------------

/// Options for a video encode / trim operation (spec §9.2).
#[derive(Debug, Clone, PartialEq)]
pub struct VideoEncodeOptions {
    pub input: PathBuf,
    pub output: PathBuf,
    pub video_encoder: String,
    pub audio_codec: Option<AudioCodec>,
    pub trim: Option<TrimSpec>,
    pub embed_thumbnail: bool,
}

/// Options for an audio-only encode / trim operation (spec §9.4).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioEncodeOptions {
    pub input: PathBuf,
    pub output: PathBuf,
    pub audio_codec: AudioCodec,
    pub trim: Option<TrimSpec>,
    pub embed_thumbnail: bool,
}

/// Derive the encoded output path following the spec: `<name>-encoded.<ext>`.
///
/// Example: `/path/to/Video.webm` -> `/path/to/Video-encoded.mp4`
pub fn derive_encoded_output_path(input: &Path, target_ext: &str) -> PathBuf {
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    let mut name = input
        .file_stem()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("output"));
    name.push(format!("-encoded.{target_ext}"));
    parent.join(name)
}

/// The temporary file ffmpeg writes to before the atomic rename to `output`
/// (spec §7.3, §15.12). It keeps the extension, which ffmpeg needs to pick
/// the muxer: `Video-encoded.mp4` -> `Video-encoded.sdp-part.mp4`.
pub fn partial_output_path(output: &Path) -> PathBuf {
    let mut name = output
        .file_stem()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("output"));
    name.push(".sdp-part");
    if let Some(ext) = output.extension() {
        name.push(".");
        name.push(ext);
    }
    output.with_file_name(name)
}

/// Build video encode args with optional custom encoder arguments (e.g. VideoToolbox fallback args).
pub fn build_video_encode_args_with_custom_encoder(
    options: &VideoEncodeOptions,
    encoder_id: &str,
    custom_args: Option<&[&str]>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "-y".into(),
        "-nostdin".into(),
        "-hide_banner".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
    ];

    // Trim arguments before -i (spec §9.2)
    if let Some(trim) = &options.trim {
        if let Some(start) = trim.start {
            if start > 0.0 {
                args.push("-ss".into());
                args.push(format_duration_arg(start).into());
            }
        }
        if let Some(end) = trim.end {
            let start = trim.start.unwrap_or(0.0);
            let dur = (end - start).max(0.0);
            args.push("-t".into());
            args.push(format_duration_arg(dur).into());
        }
    }

    args.push("-i".into());
    args.push(options.input.as_os_str().to_os_string());

    if options.embed_thumbnail {
        // Map primary video, audio, and attached thumbnail picture
        args.push("-map".into());
        args.push("0:V:0?".into());
        args.push("-map".into());
        args.push("0:a?".into());
        args.push("-map".into());
        args.push("0:disp:attached_pic?".into());
    }

    let v_stream_target = if options.embed_thumbnail {
        "-c:v:0"
    } else {
        "-c:v"
    };

    // Video encoder arguments
    if let Some(custom) = custom_args {
        args.push(v_stream_target.into());
        args.push(encoder_id.into());
        for extra in custom {
            args.push((*extra).into());
        }
    } else if let Some(def) = lookup_encoder(encoder_id) {
        args.push(v_stream_target.into());
        args.push(def.id.into());
        for extra in def.args {
            args.push((*extra).into());
        }
    } else {
        // Fallback to libx264 if unknown
        args.push(v_stream_target.into());
        args.push("libx264".into());
        args.push("-preset".into());
        args.push("medium".into());
        args.push("-crf".into());
        args.push("20".into());
        args.push("-pix_fmt".into());
        args.push("yuv420p".into());
    }

    // Audio codec
    if let Some(audio) = options.audio_codec {
        for arg in audio.ffmpeg_args() {
            args.push((*arg).into());
        }
    } else {
        // If no explicit audio re-encode requested, copy original audio
        args.push("-c:a".into());
        args.push("copy".into());
    }

    if options.embed_thumbnail {
        // Preserve attached picture disposition without re-encoding it with the video encoder.
        // Stream 0 is the primary video stream (encoded above with -c:v:0); stream 1 in the output
        // is the attached picture stream.
        args.push("-c:v:1".into());
        args.push("copy".into());
        args.push("-disposition:v:1".into());
        args.push("attached_pic".into());
    }

    // Faststart for web streaming & quick metadata reading
    args.push("-movflags".into());
    args.push("+faststart".into());

    args.push(options.output.as_os_str().to_os_string());

    args
}

/// Build the FFmpeg command line arguments for a video encode (spec §9.2).
///
/// Base command: `ffmpeg -y -nostdin -ss <start>? -t <dur>? -i <in> -c:v <venc + args> -c:a <aenc> -movflags +faststart <out>`.
pub fn build_video_encode_args(options: &VideoEncodeOptions) -> Vec<OsString> {
    build_video_encode_args_with_custom_encoder(options, &options.video_encoder, None)
}

/// Build the FFmpeg command line arguments for an audio encode (spec §9.4).
pub fn build_audio_encode_args(options: &AudioEncodeOptions) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "-y".into(),
        "-nostdin".into(),
        "-hide_banner".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
    ];

    // Trim arguments before -i (spec §9.2)
    if let Some(trim) = &options.trim {
        if let Some(start) = trim.start {
            if start > 0.0 {
                args.push("-ss".into());
                args.push(format_duration_arg(start).into());
            }
        }
        if let Some(end) = trim.end {
            let start = trim.start.unwrap_or(0.0);
            let dur = (end - start).max(0.0);
            args.push("-t".into());
            args.push(format_duration_arg(dur).into());
        }
    }

    args.push("-i".into());
    args.push(options.input.as_os_str().to_os_string());

    if options.embed_thumbnail && matches!(options.audio_codec, AudioCodec::Aac | AudioCodec::Mp3) {
        // Preserve embedded thumbnail for audio containers supporting attached_pic (m4a, mp3)
        args.push("-map".into());
        args.push("0:a:0?".into());
        args.push("-map".into());
        args.push("0:disp:attached_pic?".into());
        for arg in options.audio_codec.ffmpeg_args() {
            args.push((*arg).into());
        }
        args.push("-c:v".into());
        args.push("copy".into());
        args.push("-disposition:v".into());
        args.push("attached_pic".into());
    } else {
        // Audio only: strip video
        args.push("-vn".into());
        for arg in options.audio_codec.ffmpeg_args() {
            args.push((*arg).into());
        }
    }

    args.push(options.output.as_os_str().to_os_string());

    args
}

// ---------------------------------------------------------------------------
// Execution & Job Lifecycle
// ---------------------------------------------------------------------------

/// Request for an encode or trim job.
#[derive(Debug, Clone)]
pub enum EncodeJobKind {
    Video(VideoEncodeOptions),
    Audio(AudioEncodeOptions),
}

impl EncodeJobKind {
    pub fn input(&self) -> &Path {
        match self {
            Self::Video(o) => &o.input,
            Self::Audio(o) => &o.input,
        }
    }

    pub fn output(&self) -> &Path {
        match self {
            Self::Video(o) => &o.output,
            Self::Audio(o) => &o.output,
        }
    }

    pub fn trim(&self) -> Option<&TrimSpec> {
        match self {
            Self::Video(o) => o.trim.as_ref(),
            Self::Audio(o) => o.trim.as_ref(),
        }
    }
}

/// Find ffprobe executable path, either from explicit config, adjacent to ffmpeg, or on PATH.
pub fn find_ffprobe(ffmpeg_path: &Path, env_ffprobe: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = env_ffprobe {
        if p.is_file() {
            return Some(p.to_path_buf());
        }
    }
    let sibling_exe = ffmpeg_path.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    if sibling_exe.is_file() {
        return Some(sibling_exe);
    }
    let sibling = ffmpeg_path.with_file_name("ffprobe");
    if sibling.is_file() {
        return Some(sibling);
    }
    let tool_name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(tool_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Probe media file duration in seconds using ffprobe (spec §9.1).
pub async fn probe_file_duration(
    ffprobe_path: &Path,
    input_file: &Path,
    temp_dir: Option<&Path>,
    token: CancellationToken,
) -> Option<f64> {
    let mut cmd = ToolCommand::new(ffprobe_path)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            "--",
        ])
        .arg(input_file)
        .label("ffprobe-duration");

    if let Some(dir) = temp_dir {
        cmd = cmd.temp_dir(dir);
    }

    let stdout_lines = Arc::new(std::sync::Mutex::new(Vec::new()));
    let stdout_sink = {
        let lines = Arc::clone(&stdout_lines);
        process::line_sink(move |line| {
            if let Ok(mut l) = lines.lock() {
                l.push(line.to_owned());
            }
        })
    };

    let probe_token = token.child_token();
    let run = process::spawn_tool(cmd, probe_token.clone(), stdout_sink);
    let outcome = match tokio::time::timeout(Duration::from_secs(10), run).await {
        Ok(result) => result,
        Err(_) => {
            probe_token.cancel();
            return None;
        }
    };

    match outcome {
        Ok(result) if result.success() => {
            let output = stdout_lines.lock().ok()?.join("\n");
            let val = output.trim().parse::<f64>().ok()?;
            if val > 0.0 && val.is_finite() {
                Some(val)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Request for executing an encode job.
#[derive(Debug, Clone)]
pub struct EncodeRequest {
    pub ffmpeg_path: PathBuf,
    pub kind: EncodeJobKind,
    pub keep_original: bool,
    pub total_duration: Option<f64>,
    pub temp_dir: Option<PathBuf>,
}

/// Progress callback sink for encoding.
pub type EncodeProgressSink = Arc<dyn Fn(&FfmpegProgress) + Send + Sync + 'static>;

/// Outcome of an encode job.
#[derive(Debug, Clone, PartialEq)]
pub enum EncodeOutcome {
    Success {
        output_file: PathBuf,
        used_cpu_fallback: bool,
    },
    Cancelled,
    Failed {
        error: String,
        stderr_tail: Vec<String>,
    },
}

/// Run an encode / trim job with ffmpeg, implementing hardware-to-CPU fallback
/// and safe file lifecycle (spec §9.2, §9.3).
pub async fn run_encode(
    mut request: EncodeRequest,
    token: CancellationToken,
    sink: EncodeProgressSink,
) -> EncodeOutcome {
    // ffmpeg writes to a partial file; only a complete encode is renamed onto
    // the final name, so a cancel/pause/failure never leaves a corrupt
    // `-encoded` file nor truncates an existing one (spec §7.3, §15.12).
    let (input, final_output) = match &mut request.kind {
        EncodeJobKind::Video(o) => {
            let fin = std::mem::replace(&mut o.output, PathBuf::new());
            o.output = partial_output_path(&fin);
            (o.input.clone(), fin)
        }
        EncodeJobKind::Audio(o) => {
            let fin = std::mem::replace(&mut o.output, PathBuf::new());
            o.output = partial_output_path(&fin);
            (o.input.clone(), fin)
        }
    };
    let keep_original = request.keep_original;
    let partial = partial_output_path(&final_output);

    match run_encode_attempts(request, token, sink).await {
        EncodeOutcome::Success {
            used_cpu_fallback, ..
        } => match fs::rename(&partial, &final_output) {
            Ok(()) => {
                // Delete original only if requested and successful
                if !keep_original && input != final_output {
                    let _ = fs::remove_file(&input);
                }
                EncodeOutcome::Success {
                    output_file: final_output,
                    used_cpu_fallback,
                }
            }
            Err(err) => {
                let _ = fs::remove_file(&partial);
                EncodeOutcome::Failed {
                    error: format!("could not move the encoded file into place: {err}"),
                    stderr_tail: Vec::new(),
                }
            }
        },
        other => {
            let _ = fs::remove_file(&partial);
            other
        }
    }
}

/// The ffmpeg attempts (with hardware-to-CPU fallback) against the output
/// path in `request`, which [`run_encode`] points at the partial file.
async fn run_encode_attempts(
    request: EncodeRequest,
    token: CancellationToken,
    sink: EncodeProgressSink,
) -> EncodeOutcome {
    let output = match &request.kind {
        EncodeJobKind::Video(o) => o.output.clone(),
        EncodeJobKind::Audio(o) => o.output.clone(),
    };

    let temp_dir = request.temp_dir.as_deref();

    let mut total_duration = request.total_duration;
    if total_duration.is_none() {
        if let Some(ffprobe) = find_ffprobe(&request.ffmpeg_path, None) {
            let input = match &request.kind {
                EncodeJobKind::Video(v) => &v.input,
                EncodeJobKind::Audio(a) => &a.input,
            };
            if let Some(dur) = probe_file_duration(&ffprobe, input, temp_dir, token.clone()).await {
                total_duration = Some(dur);
            }
        }
    }

    let expected_dur = calculate_expected_duration(request.kind.trim(), total_duration);

    // Initial attempt
    let (outcome, used_fallback) = match &request.kind {
        EncodeJobKind::Video(video_opts) => {
            let mut res = execute_single_encode(
                &request.ffmpeg_path,
                &build_video_encode_args(video_opts),
                expected_dur,
                token.clone(),
                sink.clone(),
                temp_dir,
            )
            .await;

            if matches!(
                res,
                Ok(ProcessResult {
                    exit_code: Some(0),
                    ..
                })
            ) {
                (Ok(()), false)
            } else if matches!(
                res,
                Ok(ProcessResult {
                    cancelled: true,
                    ..
                })
            ) {
                return EncodeOutcome::Cancelled;
            } else {
                // If videotoolbox failed with -q:v, retry once with -b:v fallback before CPU fallback (spec §9.3)
                if let Some(fallback_args) = videotoolbox_fallback_args(&video_opts.video_encoder) {
                    if token.is_cancelled() {
                        return EncodeOutcome::Cancelled;
                    }
                    let vt_args = build_video_encode_args_with_custom_encoder(
                        video_opts,
                        &video_opts.video_encoder,
                        Some(fallback_args),
                    );
                    res = execute_single_encode(
                        &request.ffmpeg_path,
                        &vt_args,
                        expected_dur,
                        token.clone(),
                        sink.clone(),
                        temp_dir,
                    )
                    .await;
                }

                if matches!(
                    res,
                    Ok(ProcessResult {
                        exit_code: Some(0),
                        ..
                    })
                ) {
                    (Ok(()), false)
                } else if matches!(
                    res,
                    Ok(ProcessResult {
                        cancelled: true,
                        ..
                    })
                ) {
                    return EncodeOutcome::Cancelled;
                } else if let Some(cpu_codec) = cpu_fallback_encoder(&video_opts.video_encoder) {
                    // Check if hardware encoder failed and CPU fallback is available (spec §9.3)
                    let mut fallback_opts = video_opts.clone();
                    fallback_opts.video_encoder = cpu_codec.to_owned();

                    let fallback_res = execute_single_encode(
                        &request.ffmpeg_path,
                        &build_video_encode_args(&fallback_opts),
                        expected_dur,
                        token.clone(),
                        sink.clone(),
                        temp_dir,
                    )
                    .await;

                    match fallback_res {
                        Ok(ProcessResult {
                            exit_code: Some(0), ..
                        }) => (Ok(()), true),
                        Ok(ProcessResult {
                            cancelled: true, ..
                        }) => return EncodeOutcome::Cancelled,
                        Ok(ProcessResult {
                            exit_code,
                            stderr_tail,
                            ..
                        }) => (
                            Err((
                                format!("FFmpeg failed with exit code {:?}", exit_code),
                                stderr_tail,
                            )),
                            true,
                        ),
                        Err(e) => (Err((e.to_string(), Vec::new())), true),
                    }
                } else {
                    let (msg, tail) = match res {
                        Ok(ProcessResult {
                            exit_code,
                            stderr_tail,
                            ..
                        }) => (
                            format!("FFmpeg failed with exit code {:?}", exit_code),
                            stderr_tail,
                        ),
                        Err(e) => (e.to_string(), Vec::new()),
                    };
                    (Err((msg, tail)), false)
                }
            }
        }
        EncodeJobKind::Audio(audio_opts) => {
            let res = execute_single_encode(
                &request.ffmpeg_path,
                &build_audio_encode_args(audio_opts),
                expected_dur,
                token.clone(),
                sink.clone(),
                temp_dir,
            )
            .await;

            match res {
                Ok(ProcessResult {
                    exit_code: Some(0), ..
                }) => (Ok(()), false),
                Ok(ProcessResult {
                    cancelled: true, ..
                }) => return EncodeOutcome::Cancelled,
                Ok(ProcessResult {
                    exit_code,
                    stderr_tail,
                    ..
                }) => (
                    Err((
                        format!("FFmpeg failed with exit code {:?}", exit_code),
                        stderr_tail,
                    )),
                    false,
                ),
                Err(e) => (Err((e.to_string(), Vec::new())), false),
            }
        }
    };

    match outcome {
        Ok(()) => {
            // Check output file exists and is non-empty (spec §9.2)
            let is_valid_output = fs::metadata(&output).map(|m| m.len() > 0).unwrap_or(false);

            if is_valid_output {
                EncodeOutcome::Success {
                    output_file: output,
                    used_cpu_fallback: used_fallback,
                }
            } else {
                // Output was empty or missing: keep original file!
                if output.exists() {
                    let _ = fs::remove_file(&output);
                }
                EncodeOutcome::Failed {
                    error: "FFmpeg produced empty output file".to_owned(),
                    stderr_tail: Vec::new(),
                }
            }
        }
        Err((err, stderr_tail)) => {
            // Keep original file! Delete broken partial output if present (spec §9.2)
            if output.exists() {
                let _ = fs::remove_file(&output);
            }
            EncodeOutcome::Failed {
                error: err,
                stderr_tail,
            }
        }
    }
}

/// Execute a single invocation of ffmpeg and pipe stdout to the tracker.
async fn execute_single_encode(
    ffmpeg_path: &Path,
    args: &[OsString],
    expected_dur: Option<f64>,
    token: CancellationToken,
    sink: EncodeProgressSink,
    temp_dir: Option<&Path>,
) -> Result<ProcessResult, AppError> {
    let mut cmd = ToolCommand::new(ffmpeg_path)
        .args(args.to_vec())
        .label("ffmpeg-encode");
    if let Some(dir) = temp_dir {
        cmd = cmd.temp_dir(dir);
    }

    let tracker = Arc::new(std::sync::Mutex::new(FfmpegProgressTracker::new(
        expected_dur,
    )));

    let tracker_sink = Arc::clone(&tracker);
    let on_stdout = Arc::new(move |line: &str| {
        if let Ok(mut t) = tracker_sink.lock() {
            if let Some(prog) = t.handle_line(line) {
                sink(&prog);
            }
        }
    });

    process::spawn_tool(cmd, token, on_stdout).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_validate_trim_rejects_bogus_or_inverted_ranges() {
        assert!(parse_and_validate_trim(Some("abc"), Some("01:00"), None).is_err());
        assert!(parse_and_validate_trim(Some("abc"), Some("xyz"), None).is_err());
        assert!(parse_and_validate_trim(Some("02:00"), Some("01:00"), None).is_err());
        assert!(parse_and_validate_trim(None, Some("02:00"), Some(60.0)).is_err());
        assert_eq!(
            parse_and_validate_trim(Some("00:10"), Some("00:20"), Some(60.0)),
            Ok((Some(10.0), Some(20.0)))
        );
        assert_eq!(parse_and_validate_trim(None, None, None), Ok((None, None)));
    }

    #[test]
    fn parses_valid_time_specs() {
        assert_eq!(parse_time_spec("45").unwrap(), 45.0);
        assert_eq!(parse_time_spec("45.500").unwrap(), 45.5);
        assert_eq!(parse_time_spec("01:30").unwrap(), 90.0);
        assert_eq!(parse_time_spec("1:30.25").unwrap(), 90.25);
        assert_eq!(parse_time_spec("00:01:30").unwrap(), 90.0);
        assert_eq!(parse_time_spec("01:00:00").unwrap(), 3600.0);
        assert_eq!(parse_time_spec("01:02:03.5").unwrap(), 3723.5);
    }

    #[test]
    fn rejects_invalid_time_specs() {
        assert_eq!(parse_time_spec(""), Err(TrimError::Empty));
        assert_eq!(parse_time_spec("   "), Err(TrimError::Empty));
        assert!(matches!(
            parse_time_spec("-10"),
            Err(TrimError::Negative(_))
        ));
        assert!(matches!(
            parse_time_spec("invalid"),
            Err(TrimError::InvalidFormat(_))
        ));
        assert!(matches!(
            parse_time_spec("01:65"),
            Err(TrimError::InvalidFormat(_))
        ));
        assert!(matches!(
            parse_time_spec("1:2:3:4"),
            Err(TrimError::InvalidFormat(_))
        ));
    }

    #[test]
    fn validates_trim_boundaries() {
        assert_eq!(validate_trim(Some(10.0), Some(20.0), Some(60.0)), Ok(()));
        assert_eq!(validate_trim(Some(0.0), Some(60.0), Some(60.0)), Ok(()));

        assert_eq!(
            validate_trim(Some(20.0), Some(10.0), Some(60.0)),
            Err(TrimError::StartNotBeforeEnd(20.0, 10.0))
        );
        assert_eq!(
            validate_trim(Some(10.0), Some(10.0), Some(60.0)),
            Err(TrimError::StartNotBeforeEnd(10.0, 10.0))
        );
        assert_eq!(
            validate_trim(Some(70.0), None, Some(60.0)),
            Err(TrimError::StartExceedsDuration(70.0, 60.0))
        );
        assert_eq!(
            validate_trim(None, Some(70.0), Some(60.0)),
            Err(TrimError::EndExceedsDuration(70.0, 60.0))
        );
    }

    #[test]
    fn calculates_expected_duration_correctly() {
        let t1 = TrimSpec::new(Some(10.0), Some(40.0));
        assert_eq!(
            calculate_expected_duration(Some(&t1), Some(100.0)),
            Some(30.0)
        );

        let t2 = TrimSpec::new(Some(10.0), None);
        assert_eq!(
            calculate_expected_duration(Some(&t2), Some(100.0)),
            Some(90.0)
        );

        let t3 = TrimSpec::new(None, Some(40.0));
        assert_eq!(
            calculate_expected_duration(Some(&t3), Some(100.0)),
            Some(40.0)
        );

        assert_eq!(calculate_expected_duration(None, Some(100.0)), Some(100.0));
    }

    #[test]
    fn progress_tracker_parses_lines() {
        let mut tracker = FfmpegProgressTracker::new(Some(100.0));

        assert_eq!(tracker.handle_line("frame=10"), None);
        assert_eq!(tracker.handle_line("fps=25.0"), None);
        assert_eq!(tracker.handle_line("speed=2.0x"), None);

        let p1 = tracker
            .handle_line("out_time_us=25000000")
            .expect("progress parsed");
        assert_eq!(p1.out_time_sec, 25.0);
        assert_eq!(p1.percent, Some(25.0));
        assert_eq!(p1.speed, Some(2.0));
        // Remaining 75s at 2x speed = 37.5s -> round to 38s
        assert_eq!(p1.eta_sec, Some(38));

        let p_end = tracker.handle_line("progress=end").expect("end progress");
        assert!(p_end.is_end);
        assert_eq!(p_end.percent, Some(100.0));
        assert_eq!(p_end.eta_sec, Some(0));
    }

    #[test]
    fn derives_encoded_output_path() {
        let in_path = PathBuf::from("/path/to/My Video.webm");
        let out_path = derive_encoded_output_path(&in_path, "mp4");
        assert_eq!(out_path, PathBuf::from("/path/to/My Video-encoded.mp4"));

        let audio_path = PathBuf::from("/path/to/Song.opus");
        let audio_out = derive_encoded_output_path(&audio_path, "m4a");
        assert_eq!(audio_out, PathBuf::from("/path/to/Song-encoded.m4a"));
    }

    #[test]
    fn partial_output_keeps_extension_and_folder() {
        assert_eq!(
            partial_output_path(Path::new("/path/to/My Video-encoded.mp4")),
            PathBuf::from("/path/to/My Video-encoded.sdp-part.mp4")
        );
    }

    #[test]
    fn builds_video_encode_args_with_trim() {
        let opts = VideoEncodeOptions {
            input: PathBuf::from("/media/input.mkv"),
            output: PathBuf::from("/media/output-encoded.mp4"),
            video_encoder: "libx264".to_string(),
            audio_codec: Some(AudioCodec::Aac),
            trim: Some(TrimSpec::new(Some(15.0), Some(45.0))),
            embed_thumbnail: false,
        };

        let args = build_video_encode_args(&opts);
        let args_str: Vec<String> = args
            .into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();

        // -ss and -t must be before -i
        let ss_idx = args_str.iter().position(|r| r == "-ss").expect("has -ss");
        let t_idx = args_str.iter().position(|r| r == "-t").expect("has -t");
        let i_idx = args_str.iter().position(|r| r == "-i").expect("has -i");

        assert!(ss_idx < i_idx, "-ss before -i");
        assert!(t_idx < i_idx, "-t before -i");
        assert_eq!(args_str[ss_idx + 1], "15.000");
        assert_eq!(args_str[t_idx + 1], "30.000"); // 45 - 15 = 30s duration

        assert!(args_str.contains(&"-c:v".to_string()));
        assert!(args_str.contains(&"libx264".to_string()));
        assert!(args_str.contains(&"-movflags".to_string()));
        assert!(args_str.contains(&"+faststart".to_string()));
        assert_eq!(
            args_str.last().map(|s| s.as_str()),
            Some("/media/output-encoded.mp4")
        );
    }

    #[test]
    fn builds_video_encode_args_with_embed_thumbnail() {
        let opts = VideoEncodeOptions {
            input: PathBuf::from("/media/input.mkv"),
            output: PathBuf::from("/media/output-encoded.mp4"),
            video_encoder: "libx264".to_string(),
            audio_codec: Some(AudioCodec::Aac),
            trim: None,
            embed_thumbnail: true,
        };

        let args = build_video_encode_args(&opts);
        let args_str: Vec<String> = args
            .into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();

        assert!(args_str.contains(&"0:V:0?".to_string()));
        assert!(args_str.contains(&"0:disp:attached_pic?".to_string()));
        assert!(args_str.contains(&"-c:v:0".to_string()));
        assert!(args_str.contains(&"-c:v:1".to_string()));
        assert!(args_str.contains(&"-disposition:v:1".to_string()));
    }

    #[test]
    fn builds_audio_encode_args_with_trim() {
        let opts = AudioEncodeOptions {
            input: PathBuf::from("/media/input.mkv"),
            output: PathBuf::from("/media/output-encoded.mp3"),
            audio_codec: AudioCodec::Mp3,
            trim: Some(TrimSpec::new(Some(5.0), Some(15.0))),
            embed_thumbnail: false,
        };

        let args = build_audio_encode_args(&opts);
        let args_str: Vec<String> = args
            .into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();

        assert!(args_str.contains(&"-vn".to_string()));
        assert!(args_str.contains(&"-c:a".to_string()));
        assert!(args_str.contains(&"libmp3lame".to_string()));
        assert_eq!(
            args_str.last().map(|s| s.as_str()),
            Some("/media/output-encoded.mp3")
        );
    }

    #[test]
    fn builds_audio_encode_args_with_embed_thumbnail() {
        let opts = AudioEncodeOptions {
            input: PathBuf::from("/media/input.mkv"),
            output: PathBuf::from("/media/output-encoded.m4a"),
            audio_codec: AudioCodec::Aac,
            trim: None,
            embed_thumbnail: true,
        };

        let args = build_audio_encode_args(&opts);
        let args_str: Vec<String> = args
            .into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();

        assert!(!args_str.contains(&"-vn".to_string()));
        assert!(args_str.contains(&"0:a:0?".to_string()));
        assert!(args_str.contains(&"0:disp:attached_pic?".to_string()));
        assert!(args_str.contains(&"-c:v".to_string()));
        assert!(args_str.contains(&"-disposition:v".to_string()));
    }
}
