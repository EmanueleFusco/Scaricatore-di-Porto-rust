//! Offline transcription pipeline using whisper.cpp sidecar (spec §3, §10.2).
//!
//! Provides pure argument builders, progress and stdout segment parsers,
//! subtitle format (SRT and WebVTT) offset math and chunk merging,
//! ffmpeg silence-point detection and chunk boundary calculation,
//! hallucination loop guarding, and execution through `process::spawn_tool`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::services::models;
use crate::services::paths;
use crate::services::process::{self, LineSinks, ToolCommand};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Audio duration threshold above which chunking is enabled (spec §10.2: "Above 2 hours").
pub const CHUNKING_THRESHOLD_SECS: f64 = 7200.0;

/// Target chunk duration for long audio (spec §10.2: "~30-minute chunks").
pub const TARGET_CHUNK_SECS: f64 = 1800.0;

/// Silence search window around boundaries (spec §10.2: "nearest silence within ±20 s").
pub const SILENCE_WINDOW_SECS: f64 = 20.0;

/// Hard cut overlap duration when no silence point is found (spec §10.2: "2 s overlap if none").
pub const HARD_CUT_OVERLAP_SECS: f64 = 2.0;

/// Consecutive repetitions threshold for the hallucination guard (spec §10.2).
pub const HALLUCINATION_REPEAT_THRESHOLD: usize = 5;

// ---------------------------------------------------------------------------
// Types and Data Structures
// ---------------------------------------------------------------------------

/// Arguments for invoking `sdp-whisper` (spec §5.4, §10.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperArgs {
    pub whisper_path: PathBuf,
    pub model_path: PathBuf,
    pub wav_path: PathBuf,
    pub language: Option<String>,
    pub translate_to_english: bool,
    pub vad: bool,
    pub vad_model_path: Option<PathBuf>,
    pub output_base: PathBuf,
    pub threads: Option<u32>,
    pub no_gpu: bool,
}

/// A parsed live segment from whisper stdout (spec §10.2, §11.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// A detected interval of silence from `ffmpeg ... silencedetect`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SilenceInterval {
    pub start_sec: f64,
    pub end_sec: f64,
}

/// A computed boundary for chunked transcription.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkBoundary {
    pub index: usize,
    pub start_sec: f64,
    pub duration_sec: f64,
    pub timestamp_offset_ms: u64,
}

/// Output paths derived for transcript sidecars (spec §10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptPaths {
    pub txt: PathBuf,
    pub srt: PathBuf,
    pub vtt: PathBuf,
}

impl TranscriptPaths {
    pub fn all(&self) -> Vec<PathBuf> {
        vec![self.txt.clone(), self.srt.clone(), self.vtt.clone()]
    }
}

/// Detected hallucination issue in transcript text (spec §10.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HallucinationWarning {
    /// Consecutive repetition of the same segment text >= 5 times.
    RepeatedLoop { phrase: String, count: usize },
    /// Entire transcript consists only of silence tokens or whitespace.
    SuspiciousSilence,
}

/// Overall outcome of running the transcription pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptionOutcome {
    pub paths: TranscriptPaths,
    pub used_cpu_fallback: bool,
    pub hallucination_warning: Option<HallucinationWarning>,
}

// ---------------------------------------------------------------------------
// Pure Functions: Argument Builder & Parsers
// ---------------------------------------------------------------------------

/// Build argv array for `sdp-whisper` based on spec §5.4 and §10.2.
///
/// Verified flags: `-m`, `-f`, `-l`, `-tr`, `-pp`, `-otxt`, `-osrt`, `-ovtt`,
/// `-of`, `--vad`, `-vm`, `-t`, `-ng`.
pub fn build_whisper_args(args: &WhisperArgs) -> Vec<String> {
    let mut out = vec![
        "-m".to_string(),
        args.model_path.to_string_lossy().to_string(),
        "-f".to_string(),
        args.wav_path.to_string_lossy().to_string(),
    ];

    let lang = args
        .language
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("auto");
    out.push("-l".to_string());
    out.push(lang.to_string());

    if args.translate_to_english {
        out.push("-tr".to_string());
    }

    out.push("-pp".to_string());
    out.push("-otxt".to_string());
    out.push("-osrt".to_string());
    out.push("-ovtt".to_string());

    out.push("-of".to_string());
    out.push(args.output_base.to_string_lossy().to_string());

    if args.vad {
        if let Some(vad_path) = &args.vad_model_path {
            out.push("--vad".to_string());
            out.push("-vm".to_string());
            out.push(vad_path.to_string_lossy().to_string());
        }
    }

    if let Some(threads) = args.threads {
        if threads > 0 {
            out.push("-t".to_string());
            out.push(threads.to_string());
        }
    }

    if args.no_gpu {
        out.push("-ng".to_string());
    }

    out
}

/// Parse progress percentage from `-pp` lines on stderr.
///
/// Matches formats such as:
/// - `whisper_print_progress_callback: progress =  42%`
/// - `progress = 100%`
/// - `progress =   0%`
pub fn parse_whisper_progress(line: &str) -> Option<f64> {
    let idx = line.find("progress =")?;
    let after = line[idx + "progress =".len()..].trim_start();
    let num_str = after.split('%').next()?.trim();
    let val: f64 = num_str.parse().ok()?;
    if (0.0..=100.0).contains(&val) {
        Some(val)
    } else {
        None
    }
}

/// Parse segment preview lines from stdout (spec §10.2).
///
/// Matches format:
/// `[00:00:00.000 --> 00:00:04.500]   Transcribed segment text`
pub fn parse_whisper_segment(line: &str) -> Option<TranscriptSegment> {
    let trimmed = line.trim();
    if !trimmed.starts_with('[') {
        return None;
    }
    let close_bracket = trimmed.find(']')?;
    let time_part = &trimmed[1..close_bracket];
    let (start_str, end_str) = time_part.split_once("-->")?;
    let start_ms = parse_vtt_timestamp(start_str.trim())?;
    let end_ms = parse_vtt_timestamp(end_str.trim())?;
    let text = trimmed[close_bracket + 1..].trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some(TranscriptSegment {
        start_ms,
        end_ms,
        text,
    })
}

/// Derive final transcript output paths next to media file or in `transcripts/` subfolder.
///
/// If any of `<stem>.txt`, `<stem>.srt`, or `<stem>.vtt` already exists on disk,
/// picks the first unused suffix ` (1)`, ` (2)`, etc. so all three outputs share
/// a collision-free base stem (spec §10.2).
pub fn derive_transcript_paths(media_path: &Path, transcripts_subfolder: bool) -> TranscriptPaths {
    let parent = media_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "transcript".to_string());

    let target_dir = if transcripts_subfolder {
        parent.join("transcripts")
    } else {
        parent.to_path_buf()
    };

    let candidate_paths = |name: &str| -> TranscriptPaths {
        TranscriptPaths {
            txt: target_dir.join(format!("{name}.txt")),
            srt: target_dir.join(format!("{name}.srt")),
            vtt: target_dir.join(format!("{name}.vtt")),
        }
    };

    let mut current = candidate_paths(&stem);
    if !current.txt.exists() && !current.srt.exists() && !current.vtt.exists() {
        return current;
    }

    let mut counter = 1u64;
    loop {
        current = candidate_paths(&format!("{stem} ({counter})"));
        if !current.txt.exists() && !current.srt.exists() && !current.vtt.exists() {
            return current;
        }
        counter += 1;
    }
}

// ---------------------------------------------------------------------------
// Pure Functions: Subtitle Timestamps & Chunk Merging
// ---------------------------------------------------------------------------

/// Parse SRT timestamp `HH:MM:SS,mmm` to milliseconds.
pub fn parse_srt_timestamp(s: &str) -> Option<u64> {
    let s = s.trim();
    let (hms, millis_str) = s.split_once(',')?;
    let millis: u64 = millis_str.trim().parse().ok()?;
    let mut parts = hms.split(':');
    let h: u64 = parts.next()?.parse().ok()?;
    let m: u64 = parts.next()?.parse().ok()?;
    let sec: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(h * 3_600_000 + m * 60_000 + sec * 1_000 + millis)
}

/// Format milliseconds to SRT timestamp `HH:MM:SS,mmm`.
pub fn format_srt_timestamp(ms: u64) -> String {
    let total_secs = ms / 1_000;
    let millis = ms % 1_000;
    let seconds = total_secs % 60;
    let total_mins = total_secs / 60;
    let minutes = total_mins % 60;
    let hours = total_mins / 60;
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}

/// Parse WebVTT timestamp `HH:MM:SS.mmm` or `MM:SS.mmm` to milliseconds.
pub fn parse_vtt_timestamp(s: &str) -> Option<u64> {
    let s = s.trim();
    let (hms, millis_str) = s.split_once('.')?;
    let millis: u64 = millis_str.trim().parse().ok()?;
    let parts: Vec<&str> = hms.split(':').collect();
    match parts.len() {
        2 => {
            let m: u64 = parts[0].parse().ok()?;
            let sec: u64 = parts[1].parse().ok()?;
            Some(m * 60_000 + sec * 1_000 + millis)
        }
        3 => {
            let h: u64 = parts[0].parse().ok()?;
            let m: u64 = parts[1].parse().ok()?;
            let sec: u64 = parts[2].parse().ok()?;
            Some(h * 3_600_000 + m * 60_000 + sec * 1_000 + millis)
        }
        _ => None,
    }
}

/// Format milliseconds to WebVTT timestamp `HH:MM:SS.mmm`.
pub fn format_vtt_timestamp(ms: u64) -> String {
    let total_secs = ms / 1_000;
    let millis = ms % 1_000;
    let seconds = total_secs % 60;
    let total_mins = total_secs / 60;
    let minutes = total_mins % 60;
    let hours = total_mins / 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

/// Offset timestamps in an SRT block and renumber cue index starting at `start_index`.
///
/// Returns the offset SRT text and the next available cue index.
pub fn offset_srt_content(
    content: &str,
    offset_ms: u64,
    mut start_index: usize,
) -> (String, usize) {
    let normalized = content.replace("\r\n", "\n");
    let mut out = String::new();
    let blocks: Vec<&str> = normalized.split("\n\n").collect();

    for block in blocks {
        let trimmed = block.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.len() < 2 {
            continue;
        }

        // Look for timestamp line (either line 0 or line 1)
        let ts_idx = if lines[0].contains("-->") {
            0
        } else if lines[1].contains("-->") {
            1
        } else {
            continue;
        };

        let ts_line = lines[ts_idx];
        if let Some((start_str, end_str)) = ts_line.split_once("-->") {
            if let (Some(s_ms), Some(e_ms)) = (
                parse_srt_timestamp(start_str.trim()),
                parse_srt_timestamp(end_str.trim()),
            ) {
                let new_start = format_srt_timestamp(s_ms + offset_ms);
                let new_end = format_srt_timestamp(e_ms + offset_ms);

                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(&format!("{start_index}\n{new_start} --> {new_end}\n"));
                start_index += 1;

                for line in &lines[ts_idx + 1..] {
                    out.push_str(line);
                    out.push('\n');
                }
                // Strip trailing newline from block text
                if out.ends_with('\n') {
                    out.pop();
                }
            }
        }
    }

    (out, start_index)
}

/// Offset timestamps in WebVTT content by `offset_ms`.
pub fn offset_vtt_content(content: &str, offset_ms: u64) -> String {
    let normalized = content.replace("\r\n", "\n");
    let mut out = String::new();
    let mut in_header = true;

    for line in normalized.lines() {
        if in_header {
            if line.trim().is_empty() {
                in_header = false;
            }
            continue;
        }

        if line.contains("-->") {
            let (start_part, rest) = line.split_once("-->").unwrap();
            let start_ts_str = start_part.trim();
            // Rest may contain end timestamp followed by optional cue settings (e.g. `align:start`)
            let mut rest_tokens = rest.split_whitespace();
            if let Some(end_ts_str) = rest_tokens.next() {
                if let (Some(s_ms), Some(e_ms)) = (
                    parse_vtt_timestamp(start_ts_str),
                    parse_vtt_timestamp(end_ts_str),
                ) {
                    let new_start = format_vtt_timestamp(s_ms + offset_ms);
                    let new_end = format_vtt_timestamp(e_ms + offset_ms);
                    let settings: Vec<&str> = rest_tokens.collect();
                    if settings.is_empty() {
                        out.push_str(&format!("{new_start} --> {new_end}\n"));
                    } else {
                        out.push_str(&format!(
                            "{new_start} --> {new_end} {}\n",
                            settings.join(" ")
                        ));
                    }
                    continue;
                }
            }
        }

        out.push_str(line);
        out.push('\n');
    }

    out.trim_end().to_string()
}

/// Merge multiple SRT contents with corresponding millisecond offsets.
pub fn merge_srt_contents(chunks: &[(&str, u64)]) -> String {
    let mut merged = String::new();
    let mut next_index = 1;

    for (content, offset_ms) in chunks {
        if content.trim().is_empty() {
            continue;
        }
        let (offsetted, new_index) = offset_srt_content(content, *offset_ms, next_index);
        if !offsetted.is_empty() {
            if !merged.is_empty() {
                merged.push_str("\n\n");
            }
            merged.push_str(&offsetted);
            next_index = new_index;
        }
    }

    if !merged.is_empty() {
        merged.push('\n');
    }
    merged
}

/// Merge multiple WebVTT contents with corresponding millisecond offsets.
pub fn merge_vtt_contents(chunks: &[(&str, u64)]) -> String {
    let mut cues = Vec::new();

    for (content, offset_ms) in chunks {
        let offsetted = offset_vtt_content(content, *offset_ms);
        let trimmed = offsetted.trim();
        if !trimmed.is_empty() {
            cues.push(trimmed.to_string());
        }
    }

    if cues.is_empty() {
        "WEBVTT\n".to_string()
    } else {
        format!("WEBVTT\n\n{}\n", cues.join("\n\n"))
    }
}

/// Merge multiple plain text transcripts with double newlines between chunks.
pub fn merge_txt_contents(chunks: &[&str]) -> String {
    let parts: Vec<&str> = chunks
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("{}\n", parts.join("\n\n"))
    }
}

// ---------------------------------------------------------------------------
// Pure Functions: Silence Detection & Chunk Boundaries
// ---------------------------------------------------------------------------

/// Parse `ffmpeg -af silencedetect` lines from stderr.
///
/// Output format typically includes lines like:
/// `[silencedetect @ 0x...] silence_start: 1795.4`
/// `[silencedetect @ 0x...] silence_end: 1798.1 | silence_duration: 2.7`
pub fn parse_silencedetect(stderr: &str) -> Vec<SilenceInterval> {
    let mut intervals = Vec::new();
    let mut current_start: Option<f64> = None;

    for line in stderr.lines() {
        if let Some(idx) = line.find("silence_start:") {
            let after = line[idx + "silence_start:".len()..].trim();
            let token = after.split_whitespace().next().unwrap_or("");
            if let Ok(val) = token.parse::<f64>() {
                current_start = Some(val);
            }
        } else if let Some(idx) = line.find("silence_end:") {
            let after = line[idx + "silence_end:".len()..].trim();
            let token = after.split('|').next().unwrap_or("").trim();
            if let Ok(end_val) = token.parse::<f64>() {
                if let Some(start_val) = current_start.take() {
                    if end_val >= start_val {
                        intervals.push(SilenceInterval {
                            start_sec: start_val,
                            end_sec: end_val,
                        });
                    }
                }
            }
        }
    }

    intervals
}

/// Calculate boundaries for chunking long audio (spec §10.2).
///
/// - Audio <= 2 hours: single boundary.
/// - Audio > 2 hours: split at ~30 min boundaries. For each boundary, search for
///   silence within `±20 s`. If found, cut in middle of silence. If not found,
///   hard cut with 2 s overlap.
pub fn calculate_chunk_boundaries(
    total_duration_sec: f64,
    silence_intervals: &[SilenceInterval],
) -> Vec<ChunkBoundary> {
    if total_duration_sec <= 0.0 {
        return Vec::new();
    }
    if total_duration_sec <= CHUNKING_THRESHOLD_SECS {
        return vec![ChunkBoundary {
            index: 1,
            start_sec: 0.0,
            duration_sec: total_duration_sec,
            timestamp_offset_ms: 0,
        }];
    }

    let mut boundaries = Vec::new();
    let mut current_start = 0.0;
    let mut chunk_index = 1;

    while current_start + TARGET_CHUNK_SECS < total_duration_sec {
        let ideal_cut = current_start + TARGET_CHUNK_SECS;
        let window_start = ideal_cut - SILENCE_WINDOW_SECS;
        let window_end = ideal_cut + SILENCE_WINDOW_SECS;

        // Find silence interval overlapping window closest to ideal_cut
        let matching_silence = silence_intervals
            .iter()
            .filter(|s| s.end_sec >= window_start && s.start_sec <= window_end)
            .min_by(|a, b| {
                let mid_a = (a.start_sec + a.end_sec) / 2.0;
                let mid_b = (b.start_sec + b.end_sec) / 2.0;
                (mid_a - ideal_cut)
                    .abs()
                    .partial_cmp(&(mid_b - ideal_cut).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        let (cut_point, overlap) = if let Some(silence) = matching_silence {
            let mid = ((silence.start_sec + silence.end_sec) / 2.0).clamp(window_start, window_end);
            (mid, 0.0)
        } else {
            // Hard cut with 2 s overlap if none (spec §10.2)
            (ideal_cut, HARD_CUT_OVERLAP_SECS)
        };

        let chunk_duration =
            ((cut_point + overlap).min(total_duration_sec) - current_start).max(0.1);
        boundaries.push(ChunkBoundary {
            index: chunk_index,
            start_sec: current_start,
            duration_sec: chunk_duration,
            timestamp_offset_ms: (current_start * 1000.0) as u64,
        });

        chunk_index += 1;
        current_start = cut_point;
    }

    // Final trailing chunk
    if current_start < total_duration_sec {
        boundaries.push(ChunkBoundary {
            index: chunk_index,
            start_sec: current_start,
            duration_sec: total_duration_sec - current_start,
            timestamp_offset_ms: (current_start * 1000.0) as u64,
        });
    }

    boundaries
}

// ---------------------------------------------------------------------------
// Pure Functions: Hallucination Guard
// ---------------------------------------------------------------------------

/// Detect obvious hallucination loops or suspicious silence-only transcripts (spec §10.2).
pub fn detect_hallucination(transcript_text: &str) -> Option<HallucinationWarning> {
    let lines: Vec<&str> = transcript_text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    if lines.is_empty() {
        return Some(HallucinationWarning::SuspiciousSilence);
    }

    // Check for suspicious silence-only markers
    let all_silence = lines.iter().all(|line| {
        let lowered = line.to_ascii_lowercase();
        lowered.contains("[blank_audio]")
            || lowered.contains("[silence]")
            || lowered.contains("(silence)")
            || lowered.contains("(music)")
            || lowered.contains("[music]")
            || lowered.contains("(applause)")
    });
    if all_silence {
        return Some(HallucinationWarning::SuspiciousSilence);
    }

    // Check for consecutive loop repetition >= 5 times
    let mut repeat_count = 1;
    let mut last_norm = String::new();

    for line in lines {
        let norm: String = line
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        let norm = norm.split_whitespace().collect::<Vec<_>>().join(" ");
        if norm.is_empty() {
            continue;
        }

        if norm == last_norm {
            repeat_count += 1;
            if repeat_count >= HALLUCINATION_REPEAT_THRESHOLD {
                return Some(HallucinationWarning::RepeatedLoop {
                    phrase: line.to_string(),
                    count: repeat_count,
                });
            }
        } else {
            repeat_count = 1;
            last_norm = norm;
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Pipeline Execution: Audio Preparation & Whisper Tool Spawning
// ---------------------------------------------------------------------------

/// Callback type for live segment preview and progress percentage updates.
pub type TranscribeProgressSink = Arc<dyn Fn(f64, Option<&str>) + Send + Sync>;

/// Prepare a 16 kHz mono WAV file using the user's ffmpeg (spec §10.2).
///
/// Pre-flight: checks available disk space (approx 115 MB / hour).
pub async fn prepare_16khz_wav(
    ffmpeg_path: &Path,
    input_media: &Path,
    output_wav: &Path,
    temp_dir: Option<&Path>,
    duration_sec: Option<f64>,
    token: &CancellationToken,
) -> Result<(), AppError> {
    let check_dir = output_wav.parent().unwrap_or(output_wav);
    let free_space = models::free_disk_space(check_dir).unwrap_or(0);

    // 16 kHz mono 16-bit PCM = 32,000 bytes/sec
    let needed = if let Some(dur) = duration_sec {
        ((dur * 32_000.0 * 1.25) as u64).max(50 * 1024 * 1024)
    } else {
        200 * 1024 * 1024 // 200 MB default headroom
    };

    if free_space > 0 && free_space < needed {
        return Err(AppError::DiskFull {
            needed,
            available: free_space,
        });
    }

    if let Some(parent) = output_wav.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut cmd = ToolCommand::new(ffmpeg_path)
        .arg("-y")
        .arg("-nostdin")
        .arg("-i")
        .arg(input_media)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(output_wav)
        .label("ffmpeg wav extraction");

    if let Some(td) = temp_dir {
        cmd = cmd.temp_dir(td);
    }

    let result = process::spawn_tool(cmd, token.clone(), process::discard_lines()).await?;
    if result.cancelled {
        let _ = fs::remove_file(output_wav);
        return Err(AppError::Process("Audio preparation cancelled".to_string()));
    }
    if !result.success() {
        let _ = fs::remove_file(output_wav);
        let detail = result.stderr_tail.join("\n");
        return Err(AppError::Process(format!(
            "ffmpeg failed to extract 16 kHz WAV: {detail}"
        )));
    }

    Ok(())
}

/// Run ffmpeg silencedetect on the WAV file to find split points.
pub async fn detect_audio_silence(
    ffmpeg_path: &Path,
    wav_path: &Path,
    temp_dir: Option<&Path>,
    token: &CancellationToken,
) -> Vec<SilenceInterval> {
    let mut cmd = ToolCommand::new(ffmpeg_path)
        .arg("-nostdin")
        .arg("-i")
        .arg(wav_path)
        .args(["-af", "silencedetect=noise=-30dB:d=0.5", "-f", "null", "-"])
        .label("ffmpeg silencedetect");

    if let Some(td) = temp_dir {
        cmd = cmd.temp_dir(td);
    }

    match process::spawn_tool(cmd, token.clone(), process::discard_lines()).await {
        Ok(result) => {
            let stderr_text = result.stderr_tail.join("\n");
            parse_silencedetect(&stderr_text)
        }
        Err(_) => Vec::new(),
    }
}

/// Split a piece of the WAV into a chunk WAV with ffmpeg copy.
pub async fn extract_wav_chunk(
    ffmpeg_path: &Path,
    full_wav: &Path,
    start_sec: f64,
    duration_sec: f64,
    output_chunk_wav: &Path,
    temp_dir: Option<&Path>,
    token: &CancellationToken,
) -> Result<(), AppError> {
    if let Some(parent) = output_chunk_wav.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut cmd = ToolCommand::new(ffmpeg_path)
        .arg("-y")
        .arg("-nostdin")
        .arg("-ss")
        .arg(format!("{start_sec:.3}"))
        .arg("-t")
        .arg(format!("{duration_sec:.3}"))
        .arg("-i")
        .arg(full_wav)
        .args(["-c", "copy"])
        .arg(output_chunk_wav)
        .label("ffmpeg chunk split");

    if let Some(td) = temp_dir {
        cmd = cmd.temp_dir(td);
    }

    let res = process::spawn_tool(cmd, token.clone(), process::discard_lines()).await?;
    if res.cancelled {
        let _ = fs::remove_file(output_chunk_wav);
        return Err(AppError::Process("Chunk extraction cancelled".to_string()));
    }
    if !res.success() {
        let _ = fs::remove_file(output_chunk_wav);
        return Err(AppError::Process(format!(
            "ffmpeg failed to split audio chunk: {}",
            res.stderr_tail.join("\n")
        )));
    }
    Ok(())
}

/// Run a single `sdp-whisper` invocation.
///
/// Returns (exit_success, cancelled, stderr_tail).
pub async fn execute_whisper_attempt(
    args: &WhisperArgs,
    temp_dir: Option<&Path>,
    token: &CancellationToken,
    progress_sink: Option<TranscribeProgressSink>,
    percent_offset: f64,
    percent_scale: f64,
) -> Result<process::ProcessResult, AppError> {
    let argv = build_whisper_args(args);
    let mut cmd = ToolCommand::new(&args.whisper_path)
        .args(argv.iter().map(String::as_str))
        .label("sdp-whisper");

    if let Some(td) = temp_dir {
        cmd = cmd.temp_dir(td);
    }

    let stdout_sink: process::LineSink = match progress_sink.as_ref() {
        Some(sink) => {
            let sink = Arc::clone(sink);
            Arc::new(move |line: &str| {
                if let Some(seg) = parse_whisper_segment(line) {
                    sink(percent_offset, Some(&seg.text));
                }
            })
        }
        None => process::discard_lines(),
    };

    let stderr_sink: process::LineSink = match progress_sink.as_ref() {
        Some(sink) => {
            let sink = Arc::clone(sink);
            Arc::new(move |line: &str| {
                if let Some(p) = parse_whisper_progress(line) {
                    let scaled = percent_offset + (p * percent_scale);
                    sink(scaled, None);
                }
            })
        }
        None => process::discard_lines(),
    };

    let sinks = LineSinks::new(stdout_sink, stderr_sink);

    process::spawn_tool_with_sinks(cmd, token.clone(), sinks).await
}

/// Options requesting a transcription run.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscribeRequest {
    pub ffmpeg_path: PathBuf,
    pub whisper_path: PathBuf,
    pub model_path: PathBuf,
    pub vad_model_path: Option<PathBuf>,
    pub input_media: PathBuf,
    pub language: Option<String>,
    pub translate_to_english: bool,
    pub skip_silence_vad: bool,
    pub whisper_threads: u32,
    pub transcripts_subfolder: bool,
    pub duration_sec: Option<f64>,
    pub temp_dir: Option<PathBuf>,
}

/// Full offline transcription pipeline (spec §10.2).
///
/// 1. Prepares 16 kHz mono WAV via ffmpeg.
/// 2. If > 2 hours, splits into ~30-min chunks at silence points.
/// 3. Executes `sdp-whisper` (retrying once with `-ng` if GPU build fails).
/// 4. Merges SRT/VTT with timestamp offsets and collects output files.
/// 5. Validates hallucination loops and cleans temporary WAV/chunk files.
pub async fn run_transcribe(
    req: TranscribeRequest,
    token: CancellationToken,
    progress_sink: Option<TranscribeProgressSink>,
) -> Result<TranscriptionOutcome, AppError> {
    let parent_dir = req
        .temp_dir
        .as_deref()
        .or_else(|| req.input_media.parent())
        .unwrap_or_else(|| Path::new("."));

    let rand_suffix: u32 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_millis() % 1_000_000) as u32)
        .unwrap_or(0);

    let full_wav = parent_dir.join(format!("whisper_prep_{rand_suffix}.wav"));

    // Step 1: Prepare 16 kHz WAV
    prepare_16khz_wav(
        &req.ffmpeg_path,
        &req.input_media,
        &full_wav,
        req.temp_dir.as_deref(),
        req.duration_sec,
        &token,
    )
    .await?;

    let effective_duration = req.duration_sec.unwrap_or_else(|| {
        // Fallback: estimate from WAV file size (32,000 bytes/sec for 16kHz mono 16-bit)
        let size = fs::metadata(&full_wav).map(|m| m.len()).unwrap_or(0);
        size as f64 / 32_000.0
    });

    let mut silence_intervals = Vec::new();
    if effective_duration > CHUNKING_THRESHOLD_SECS {
        silence_intervals =
            detect_audio_silence(&req.ffmpeg_path, &full_wav, req.temp_dir.as_deref(), &token)
                .await;
    }

    let boundaries = calculate_chunk_boundaries(effective_duration, &silence_intervals);
    let total_chunks = boundaries.len().max(1);

    let target_dir = if req.transcripts_subfolder {
        req.input_media
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("transcripts")
    } else {
        req.input_media
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };
    fs::create_dir_all(&target_dir)?;

    let mut chunk_txt_data: Vec<String> = Vec::new();
    let mut chunk_srt_data: Vec<(String, u64)> = Vec::new();
    let mut chunk_vtt_data: Vec<(String, u64)> = Vec::new();
    let mut used_cpu_fallback = false;

    for (idx, boundary) in boundaries.iter().enumerate() {
        if token.is_cancelled() {
            let _ = fs::remove_file(&full_wav);
            return Err(AppError::Process("Transcription cancelled".to_string()));
        }

        let chunk_wav = if total_chunks == 1 {
            full_wav.clone()
        } else {
            let c_wav = parent_dir.join(format!("chunk_{idx}_{rand_suffix}.wav"));
            extract_wav_chunk(
                &req.ffmpeg_path,
                &full_wav,
                boundary.start_sec,
                boundary.duration_sec,
                &c_wav,
                req.temp_dir.as_deref(),
                &token,
            )
            .await?;
            c_wav
        };

        let chunk_out_base = parent_dir.join(format!("chunk_{idx}_{rand_suffix}_out"));
        let percent_offset = (idx as f64 * 100.0) / total_chunks as f64;
        let percent_scale = 1.0 / total_chunks as f64;

        let mut whisper_args = WhisperArgs {
            whisper_path: req.whisper_path.clone(),
            model_path: req.model_path.clone(),
            wav_path: chunk_wav.clone(),
            language: req.language.clone(),
            translate_to_english: req.translate_to_english,
            vad: req.skip_silence_vad && req.vad_model_path.is_some(),
            vad_model_path: req.vad_model_path.clone(),
            output_base: chunk_out_base.clone(),
            threads: (req.whisper_threads > 0).then_some(req.whisper_threads),
            no_gpu: false,
        };

        let mut res = execute_whisper_attempt(
            &whisper_args,
            req.temp_dir.as_deref(),
            &token,
            progress_sink.clone(),
            percent_offset,
            percent_scale,
        )
        .await?;

        if res.cancelled {
            clean_whisper_partials(&chunk_out_base);
            if total_chunks > 1 {
                let _ = fs::remove_file(&chunk_wav);
            }
            let _ = fs::remove_file(&full_wav);
            return Err(AppError::Process("Transcription cancelled".to_string()));
        }

        // Retry once with -ng on non-zero exit (spec §10.2 item 6)
        if !res.success() && !whisper_args.no_gpu {
            whisper_args.no_gpu = true;
            res = execute_whisper_attempt(
                &whisper_args,
                req.temp_dir.as_deref(),
                &token,
                progress_sink.clone(),
                percent_offset,
                percent_scale,
            )
            .await?;
            if res.success() {
                used_cpu_fallback = true;
            }
        }

        if !res.success() {
            clean_whisper_partials(&chunk_out_base);
            if total_chunks > 1 {
                let _ = fs::remove_file(&chunk_wav);
            }
            let _ = fs::remove_file(&full_wav);
            return Err(AppError::Process(format!(
                "sdp-whisper failed: {}",
                res.stderr_tail.join("\n")
            )));
        }

        // Collect outputs from this chunk
        let txt_file = PathBuf::from(format!("{}.txt", chunk_out_base.display()));
        let srt_file = PathBuf::from(format!("{}.srt", chunk_out_base.display()));
        let vtt_file = PathBuf::from(format!("{}.vtt", chunk_out_base.display()));

        if let Ok(txt) = fs::read_to_string(&txt_file) {
            chunk_txt_data.push(txt);
        }
        if let Ok(srt) = fs::read_to_string(&srt_file) {
            chunk_srt_data.push((srt, boundary.timestamp_offset_ms));
        }
        if let Ok(vtt) = fs::read_to_string(&vtt_file) {
            chunk_vtt_data.push((vtt, boundary.timestamp_offset_ms));
        }

        clean_whisper_partials(&chunk_out_base);
        if total_chunks > 1 {
            let _ = fs::remove_file(&chunk_wav);
        }
    }

    let _ = fs::remove_file(&full_wav);

    // Merge chunk contents
    let txt_slices: Vec<&str> = chunk_txt_data.iter().map(String::as_str).collect();
    let final_txt = merge_txt_contents(&txt_slices);

    let srt_slices: Vec<(&str, u64)> = chunk_srt_data
        .iter()
        .map(|(s, off)| (s.as_str(), *off))
        .collect();
    let final_srt = merge_srt_contents(&srt_slices);

    let vtt_slices: Vec<(&str, u64)> = chunk_vtt_data
        .iter()
        .map(|(s, off)| (s.as_str(), *off))
        .collect();
    let final_vtt = merge_vtt_contents(&vtt_slices);

    // Derive collision-free paths immediately before writing (spec §10.2)
    let final_paths = derive_transcript_paths(&req.input_media, req.transcripts_subfolder);

    // Write final merged files atomically
    paths::atomic_write(&final_paths.txt, final_txt.as_bytes())?;
    paths::atomic_write(&final_paths.srt, final_srt.as_bytes())?;
    paths::atomic_write(&final_paths.vtt, final_vtt.as_bytes())?;

    // Hallucination check
    let hallucination_warning = detect_hallucination(&final_txt);

    if let Some(sink) = &progress_sink {
        sink(100.0, None);
    }

    Ok(TranscriptionOutcome {
        paths: final_paths,
        used_cpu_fallback,
        hallucination_warning,
    })
}

fn clean_whisper_partials(base: &Path) {
    let _ = fs::remove_file(format!("{}.txt", base.display()));
    let _ = fs::remove_file(format!("{}.srt", base.display()));
    let _ = fs::remove_file(format!("{}.vtt", base.display()));
}

// ---------------------------------------------------------------------------
// Unit Tests (Pure functions)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_whisper_args_includes_all_pinned_flags() {
        let args = WhisperArgs {
            whisper_path: PathBuf::from("/bin/sdp-whisper"),
            model_path: PathBuf::from("/models/ggml-turbo.bin"),
            wav_path: PathBuf::from("/tmp/audio.wav"),
            language: Some("it".to_string()),
            translate_to_english: true,
            vad: true,
            vad_model_path: Some(PathBuf::from("/models/silero.bin")),
            output_base: PathBuf::from("/out/video"),
            threads: Some(4),
            no_gpu: true,
        };

        let argv = build_whisper_args(&args);
        assert_eq!(
            argv,
            vec![
                "-m",
                "/models/ggml-turbo.bin",
                "-f",
                "/tmp/audio.wav",
                "-l",
                "it",
                "-tr",
                "-pp",
                "-otxt",
                "-osrt",
                "-ovtt",
                "-of",
                "/out/video",
                "--vad",
                "-vm",
                "/models/silero.bin",
                "-t",
                "4",
                "-ng",
            ]
        );
    }

    #[test]
    fn build_whisper_args_omits_optional_flags_when_disabled() {
        let args = WhisperArgs {
            whisper_path: PathBuf::from("/bin/sdp-whisper"),
            model_path: PathBuf::from("/models/ggml-turbo.bin"),
            wav_path: PathBuf::from("/tmp/audio.wav"),
            language: None,
            translate_to_english: false,
            vad: false,
            vad_model_path: None,
            output_base: PathBuf::from("/out/video"),
            threads: None,
            no_gpu: false,
        };

        let argv = build_whisper_args(&args);
        assert_eq!(
            argv,
            vec![
                "-m",
                "/models/ggml-turbo.bin",
                "-f",
                "/tmp/audio.wav",
                "-l",
                "auto",
                "-pp",
                "-otxt",
                "-osrt",
                "-ovtt",
                "-of",
                "/out/video",
            ]
        );
    }

    #[test]
    fn parse_whisper_progress_extracts_percentage() {
        assert_eq!(
            parse_whisper_progress("whisper_print_progress_callback: progress =  42%"),
            Some(42.0)
        );
        assert_eq!(parse_whisper_progress("progress = 100%"), Some(100.0));
        assert_eq!(parse_whisper_progress("progress =   0%"), Some(0.0));
        assert_eq!(parse_whisper_progress("some other log line"), None);
        assert_eq!(parse_whisper_progress("progress = 150%"), None);
    }

    #[test]
    fn parse_whisper_segment_extracts_timestamps_and_text() {
        let line = "[00:01:23.456 --> 00:01:28.900]   Hello from Whisper transcription!";
        let seg = parse_whisper_segment(line).expect("must parse");
        assert_eq!(seg.start_ms, 83456);
        assert_eq!(seg.end_ms, 88900);
        assert_eq!(seg.text, "Hello from Whisper transcription!");

        // 2-part timestamp MM:SS.mmm
        let line_short = "[01:23.456 --> 01:28.900]   Short timestamp format";
        let seg_short = parse_whisper_segment(line_short).expect("must parse short");
        assert_eq!(seg_short.start_ms, 83456);
        assert_eq!(seg_short.text, "Short timestamp format");

        assert_eq!(parse_whisper_segment("random output text"), None);
        assert_eq!(parse_whisper_segment("[00:00.000 --> 00:01.000]   "), None);
    }

    #[test]
    fn srt_timestamp_roundtrip() {
        assert_eq!(parse_srt_timestamp("01:23:45,678"), Some(5_025_678));
        assert_eq!(format_srt_timestamp(5_025_678), "01:23:45,678");
        assert_eq!(parse_srt_timestamp("invalid"), None);
    }

    #[test]
    fn vtt_timestamp_roundtrip() {
        assert_eq!(parse_vtt_timestamp("01:23:45.678"), Some(5_025_678));
        assert_eq!(format_vtt_timestamp(5_025_678), "01:23:45.678");
        assert_eq!(parse_vtt_timestamp("23:45.678"), Some(1_425_678));
    }

    #[test]
    fn offset_srt_content_updates_timestamps_and_index() {
        let input =
            "1\n00:00:01,000 --> 00:00:03,000\nHello\n\n2\n00:00:04,500 --> 00:00:06,000\nWorld";
        // 10,000 ms offset, start index 5
        let (offsetted, next_idx) = offset_srt_content(input, 10_000, 5);
        assert_eq!(next_idx, 7);
        assert_eq!(
            offsetted,
            "5\n00:00:11,000 --> 00:00:13,000\nHello\n\n6\n00:00:14,500 --> 00:00:16,000\nWorld"
        );
    }

    #[test]
    fn merge_srt_contents_merges_chunks_seamlessly() {
        let chunk1 = "1\n00:00:01,000 --> 00:00:02,000\nChunk 1 text";
        let chunk2 = "1\n00:00:00,500 --> 00:00:02,500\nChunk 2 text";
        let merged = merge_srt_contents(&[(chunk1, 0), (chunk2, 60_000)]);
        assert_eq!(
            merged,
            "1\n00:00:01,000 --> 00:00:02,000\nChunk 1 text\n\n2\n00:01:00,500 --> 00:01:02,500\nChunk 2 text\n"
        );
    }

    #[test]
    fn merge_vtt_contents_merges_chunks_with_webvtt_header() {
        let chunk1 = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nChunk 1 text";
        let chunk2 = "WEBVTT\n\n00:00:00.500 --> 00:00:02.500\nChunk 2 text";
        let merged = merge_vtt_contents(&[(chunk1, 0), (chunk2, 60_000)]);
        assert_eq!(
            merged,
            "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nChunk 1 text\n\n00:01:00.500 --> 00:01:02.500\nChunk 2 text\n"
        );
    }

    #[test]
    fn merge_txt_contents_joins_non_empty_parts() {
        let c1 = "First chunk.";
        let c2 = "Second chunk.";
        assert_eq!(
            merge_txt_contents(&[c1, "", "  ", c2]),
            "First chunk.\n\nSecond chunk.\n"
        );
    }

    #[test]
    fn parse_silencedetect_extracts_intervals() {
        let stderr = r#"
[silencedetect @ 0x55d7b055] silence_start: 1795.4
[silencedetect @ 0x55d7b055] silence_end: 1798.1 | silence_duration: 2.7
[silencedetect @ 0x55d7b055] silence_start: 3590.0
[silencedetect @ 0x55d7b055] silence_end: 3594.2 | silence_duration: 4.2
"#;
        let intervals = parse_silencedetect(stderr);
        assert_eq!(intervals.len(), 2);
        assert_eq!(intervals[0].start_sec, 1795.4);
        assert_eq!(intervals[0].end_sec, 1798.1);
        assert_eq!(intervals[1].start_sec, 3590.0);
        assert_eq!(intervals[1].end_sec, 3594.2);
    }

    #[test]
    fn calculate_chunk_boundaries_single_chunk_under_2_hours() {
        let b = calculate_chunk_boundaries(7199.0, &[]);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].start_sec, 0.0);
        assert_eq!(b[0].duration_sec, 7199.0);
        assert_eq!(b[0].timestamp_offset_ms, 0);
    }

    #[test]
    fn calculate_chunk_boundaries_splits_at_silence_for_long_audio() {
        let silences = vec![
            SilenceInterval {
                start_sec: 1798.0,
                end_sec: 1802.0,
            },
            SilenceInterval {
                start_sec: 3599.0,
                end_sec: 3601.0,
            },
        ];
        let total = 8000.0; // > 2 hours (7200s)
        let b = calculate_chunk_boundaries(total, &silences);
        assert!(b.len() >= 3);
        // Cut 1 near 1800 at mid (1800.0)
        assert_eq!(b[0].start_sec, 0.0);
        assert_eq!(b[0].duration_sec, 1800.0);
        // Cut 2 starts at 1800.0
        assert_eq!(b[1].start_sec, 1800.0);
        assert_eq!(b[1].timestamp_offset_ms, 1_800_000);
    }

    #[test]
    fn calculate_chunk_boundaries_hard_cut_with_overlap_when_no_silence() {
        let total = 8000.0;
        let b = calculate_chunk_boundaries(total, &[]);
        assert!(b.len() >= 3);
        assert_eq!(b[0].start_sec, 0.0);
        // Hard cut with 2 s overlap
        assert_eq!(b[0].duration_sec, 1802.0);
        assert_eq!(b[1].start_sec, 1800.0);
        assert_eq!(b[1].timestamp_offset_ms, 1_800_000);
    }

    #[test]
    fn detect_hallucination_finds_repetition_loops() {
        let mut loop_text = String::new();
        for _ in 0..6 {
            loop_text.push_str("Grazie per aver guardato il video.\n");
        }
        let warn = detect_hallucination(&loop_text);
        assert!(matches!(
            warn,
            Some(HallucinationWarning::RepeatedLoop { count, .. }) if count >= 5
        ));

        let normal_text = "Prima frase del discorso.\nSeconda frase diversa.\nTerza frase finale.";
        assert_eq!(detect_hallucination(normal_text), None);
    }

    #[test]
    fn detect_hallucination_finds_suspicious_silence() {
        assert_eq!(
            detect_hallucination("   \n\n  "),
            Some(HallucinationWarning::SuspiciousSilence)
        );
        assert_eq!(
            detect_hallucination("[BLANK_AUDIO]\n[BLANK_AUDIO]\n[BLANK_AUDIO]"),
            Some(HallucinationWarning::SuspiciousSilence)
        );
    }

    #[test]
    fn derive_transcript_paths_formats_filenames() {
        let media = PathBuf::from("/downloads/My Video [123].mp4");
        let direct = derive_transcript_paths(&media, false);
        assert_eq!(direct.txt, PathBuf::from("/downloads/My Video [123].txt"));
        assert_eq!(direct.srt, PathBuf::from("/downloads/My Video [123].srt"));
        assert_eq!(direct.vtt, PathBuf::from("/downloads/My Video [123].vtt"));

        let subfolder = derive_transcript_paths(&media, true);
        assert_eq!(
            subfolder.txt,
            PathBuf::from("/downloads/transcripts/My Video [123].txt")
        );
    }

    #[test]
    fn derive_transcript_paths_avoids_overwriting_existing_files() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let media = temp.path().join("lecture.mp4");

        // When no files exist, direct stem is used
        let paths0 = derive_transcript_paths(&media, false);
        assert_eq!(paths0.txt, temp.path().join("lecture.txt"));
        assert_eq!(paths0.srt, temp.path().join("lecture.srt"));
        assert_eq!(paths0.vtt, temp.path().join("lecture.vtt"));

        // If .srt exists, suffix (1) is used for all three outputs
        fs::write(&paths0.srt, "existing srt").expect("write srt");
        let paths1 = derive_transcript_paths(&media, false);
        assert_eq!(paths1.txt, temp.path().join("lecture (1).txt"));
        assert_eq!(paths1.srt, temp.path().join("lecture (1).srt"));
        assert_eq!(paths1.vtt, temp.path().join("lecture (1).vtt"));

        // If .txt exists for suffix (1), suffix (2) is used
        fs::write(&paths1.txt, "existing txt 1").expect("write txt 1");
        let paths2 = derive_transcript_paths(&media, false);
        assert_eq!(paths2.txt, temp.path().join("lecture (2).txt"));
        assert_eq!(paths2.srt, temp.path().join("lecture (2).srt"));
        assert_eq!(paths2.vtt, temp.path().join("lecture (2).vtt"));

        // Subfolder mode checks existing files in transcripts/
        let sub_paths0 = derive_transcript_paths(&media, true);
        assert_eq!(
            sub_paths0.txt,
            temp.path().join("transcripts").join("lecture.txt")
        );
        fs::create_dir_all(temp.path().join("transcripts")).expect("create transcripts");
        fs::write(
            temp.path().join("transcripts").join("lecture.vtt"),
            "existing vtt",
        )
        .expect("write vtt");
        let sub_paths1 = derive_transcript_paths(&media, true);
        assert_eq!(
            sub_paths1.txt,
            temp.path().join("transcripts").join("lecture (1).txt")
        );
        assert_eq!(
            sub_paths1.srt,
            temp.path().join("transcripts").join("lecture (1).srt")
        );
        assert_eq!(
            sub_paths1.vtt,
            temp.path().join("transcripts").join("lecture (1).vtt")
        );
    }
}
