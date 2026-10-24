//! Subtitles fetcher and cleaner (spec §10.1).
//!
//! Subtitles-first fast path:
//! 1. Choose languages: user setting, else video language metadata, else UI language + English.
//! 2. Fetch manual subtitles first, then automatic captions if none, via yt-dlp.
//! 3. Strip markup tags, de-duplicate rolling repeated lines (common in auto-captions),
//!    and group into paragraphs on long pauses (>= 2 s).
//! 4. Save sidecar files (`.it.srt`, `.it.auto.srt`, `.txt`) and record origin.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::services::paths;
use crate::services::process::{self, ToolCommand};

/// Default pause threshold in milliseconds to split subtitles into paragraphs.
pub const DEFAULT_PARAGRAPH_PAUSE_MS: u64 = 2000;

/// Subtitle origin (spec §10.1: "Mark the origin in the UI and in the sidecar name").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleOrigin {
    Manual,
    Auto,
}

impl SubtitleOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "subtitles_manual",
            Self::Auto => "subtitles_auto",
        }
    }
}

/// A parsed subtitle cue with timing and text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleCue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Strip XML/HTML markup tags like `<font...>`, `<b>`, `</font>`, `<c.color>`, `<00:00:01.000>`.
pub fn strip_markup_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;

    for ch in raw.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(ch);
        }
    }

    // Decode common HTML entities
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Normalize text for comparison by lowercasing and stripping non-alphanumeric chars.
fn normalize_word(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Normalize a line by lowercasing and keeping only alphanumeric chars.
fn normalize_line(l: &str) -> String {
    l.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Check if two lines match either exactly (trimmed) or normalized.
fn lines_match(a: &str, b: &str) -> bool {
    let a_trim = a.trim();
    let b_trim = b.trim();
    if a_trim == b_trim {
        return true;
    }
    let na = normalize_line(a_trim);
    let nb = normalize_line(b_trim);
    !na.is_empty() && na == nb
}

/// Find word-level overlap between the end of `words_a` and the beginning of `words_b`.
///
/// Returns the number of tokens in `words_b` that overlap with the end of `words_a`,
/// mapped back to the original index in `words_b`.
fn find_word_overlap(words_a: &[&str], words_b: &[&str]) -> usize {
    let norm_a: Vec<(usize, String)> = words_a
        .iter()
        .enumerate()
        .map(|(idx, w)| (idx, normalize_word(w)))
        .filter(|(_, w)| !w.is_empty())
        .collect();
    let norm_b: Vec<(usize, String)> = words_b
        .iter()
        .enumerate()
        .map(|(idx, w)| (idx, normalize_word(w)))
        .filter(|(_, w)| !w.is_empty())
        .collect();

    let max_k = norm_a.len().min(norm_b.len());
    for k in (1..=max_k).rev() {
        let a_slice = &norm_a[norm_a.len() - k..];
        if a_slice
            .iter()
            .zip(&norm_b[..k])
            .all(|((_, a), (_, b))| a == b)
        {
            if let (Some(last_a), Some(last_b)) = (a_slice.last(), norm_b.get(k - 1)) {
                let mut a_idx = last_a.0;
                let mut b_idx = last_b.0;
                while a_idx + 1 < words_a.len()
                    && b_idx + 1 < words_b.len()
                    && normalize_word(words_a[a_idx + 1]).is_empty()
                    && normalize_word(words_b[b_idx + 1]).is_empty()
                    && words_a[a_idx + 1] == words_b[b_idx + 1]
                {
                    a_idx += 1;
                    b_idx += 1;
                }
                return b_idx + 1;
            }
        }
    }
    0
}

/// Parse timestamp from SRT format `HH:MM:SS,mmm`.
pub fn parse_srt_timestamp(s: &str) -> Option<u64> {
    let (hms, ms_str) = s.split_once(',')?;
    let mut parts = hms.split(':');
    let h: u64 = parts.next()?.parse().ok()?;
    let m: u64 = parts.next()?.parse().ok()?;
    let s: u64 = parts.next()?.parse().ok()?;
    let ms: u64 = ms_str.parse().ok()?;
    Some(h * 3_600_000 + m * 60_000 + s * 1_000 + ms)
}

/// Parse timestamp from WebVTT format `HH:MM:SS.mmm` or `MM:SS.mmm`.
pub fn parse_vtt_timestamp(s: &str) -> Option<u64> {
    let (hms, ms_str) = s.split_once('.')?;
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, sec): (u64, u64, u64) = match parts.len() {
        2 => (0u64, parts[0].parse().ok()?, parts[1].parse().ok()?),
        3 => (
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        ),
        _ => return None,
    };
    let ms: u64 = ms_str.parse().ok()?;
    Some(h * 3_600_000 + m * 60_000 + sec * 1_000 + ms)
}

/// Parse SRT content into a list of [`SubtitleCue`]s.
pub fn parse_srt_cues(content: &str) -> Vec<SubtitleCue> {
    let normalized = content.replace("\r\n", "\n");
    let mut cues = Vec::new();

    for block in normalized.split("\n\n") {
        let trimmed = block.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.len() < 2 {
            continue;
        }

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
                let raw_text = lines[ts_idx + 1..].join("\n");
                let clean_text = strip_markup_tags(&raw_text).trim().to_string();
                if !clean_text.is_empty() {
                    cues.push(SubtitleCue {
                        start_ms: s_ms,
                        end_ms: e_ms,
                        text: clean_text,
                    });
                }
            }
        }
    }

    cues
}

/// Parse WebVTT content into a list of [`SubtitleCue`]s.
pub fn parse_vtt_cues(content: &str) -> Vec<SubtitleCue> {
    let normalized = content.replace("\r\n", "\n");
    let mut cues = Vec::new();

    for block in normalized.split("\n\n") {
        let trimmed = block.trim();
        if trimmed.is_empty() || trimmed == "WEBVTT" || trimmed.starts_with("WEBVTT\n") {
            continue;
        }
        let lines: Vec<&str> = trimmed.lines().collect();
        let ts_idx = if lines[0].contains("-->") {
            0
        } else if lines.len() > 1 && lines[1].contains("-->") {
            1
        } else {
            continue;
        };

        let ts_line = lines[ts_idx];
        if let Some((start_part, rest)) = ts_line.split_once("-->") {
            let start_ts_str = start_part.trim();
            let mut rest_tokens = rest.split_whitespace();
            if let Some(end_ts_str) = rest_tokens.next() {
                if let (Some(s_ms), Some(e_ms)) = (
                    parse_vtt_timestamp(start_ts_str),
                    parse_vtt_timestamp(end_ts_str),
                ) {
                    let raw_text = lines[ts_idx + 1..].join("\n");
                    let clean_text = strip_markup_tags(&raw_text).trim().to_string();
                    if !clean_text.is_empty() {
                        cues.push(SubtitleCue {
                            start_ms: s_ms,
                            end_ms: e_ms,
                            text: clean_text,
                        });
                    }
                }
            }
        }
    }

    cues
}

/// De-duplicate rolling repeated lines from raw cues and assemble paragraphs (spec §10.1).
///
/// Handles typical auto-caption patterns:
/// 1. Rolling multi-line windows where line 1 of cue N+1 repeats line 2 of cue N.
/// 2. Incremental word building where cue N+1 contains cue N's text as a prefix.
/// 3. Overlapping word boundary phrases.
/// 4. Paragraph separation on pauses longer than `pause_threshold_ms`.
pub fn clean_subtitle_cues(cues: &[SubtitleCue], pause_threshold_ms: u64) -> String {
    if cues.is_empty() {
        return String::new();
    }

    let mut paragraphs: Vec<String> = Vec::new();
    let mut current_paragraph = String::new();
    let mut prev_cue: Option<&SubtitleCue> = None;
    let mut prev_tokens: Vec<String> = Vec::new();
    let mut recent_lines: Vec<String> = Vec::new();

    for cue in cues {
        // Check for long pause -> start new paragraph
        if let Some(prev) = prev_cue {
            if cue.start_ms > prev.end_ms
                && (cue.start_ms - prev.end_ms) >= pause_threshold_ms
                && !current_paragraph.trim().is_empty()
            {
                paragraphs.push(current_paragraph.trim().to_string());
                current_paragraph.clear();
                prev_tokens.clear();
                recent_lines.clear();
            }
        }

        // Clean cue text: split by lines, de-duplicate within multi-line cues and across rolling cues
        let cue_lines: Vec<&str> = cue
            .text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        let mut kept_lines: Vec<&str> = Vec::new();
        for (idx, line) in cue_lines.iter().enumerate() {
            if idx > 0 && lines_match(line, cue_lines[idx - 1]) {
                // Skip identical repeated lines within the same cue
                continue;
            }
            if recent_lines.iter().any(|r| lines_match(line, r)) {
                // Skip lines repeated from recent rolling cues
                continue;
            }
            kept_lines.push(line);
        }

        // Update rolling lines window with lines from this cue
        for line in &cue_lines {
            if !recent_lines.iter().any(|r| lines_match(line, r)) {
                recent_lines.push((*line).to_string());
            }
        }
        if recent_lines.len() > 4 {
            recent_lines.drain(0..recent_lines.len() - 4);
        }

        // Flatten kept cue lines into words
        let mut cue_words: Vec<&str> = Vec::new();
        for line in kept_lines {
            cue_words.extend(line.split_whitespace());
        }

        if cue_words.is_empty() {
            prev_cue = Some(cue);
            continue;
        }

        // Check word overlap with previous cue
        let prev_words_str: Vec<&str> = prev_tokens.iter().map(|s| s.as_str()).collect();
        let overlap = find_word_overlap(&prev_words_str, &cue_words);

        let new_words = &cue_words[overlap..];
        if !new_words.is_empty() {
            for word in new_words {
                if !current_paragraph.is_empty() && !current_paragraph.ends_with(' ') {
                    current_paragraph.push(' ');
                }
                current_paragraph.push_str(word);
            }
        }

        prev_tokens = cue_words.iter().map(|w| (*w).to_string()).collect();
        prev_cue = Some(cue);
    }

    if !current_paragraph.trim().is_empty() {
        paragraphs.push(current_paragraph.trim().to_string());
    }

    paragraphs.join("\n\n")
}

/// Automatically detect format (SRT or WebVTT) and clean to plain text.
pub fn clean_subtitles(raw_content: &str) -> String {
    let trimmed = raw_content.trim();
    if trimmed.starts_with("WEBVTT") {
        let cues = parse_vtt_cues(raw_content);
        clean_subtitle_cues(&cues, DEFAULT_PARAGRAPH_PAUSE_MS)
    } else {
        let cues = parse_srt_cues(raw_content);
        clean_subtitle_cues(&cues, DEFAULT_PARAGRAPH_PAUSE_MS)
    }
}

/// Request parameters for subtitle fetching.
#[derive(Debug, Clone)]
pub struct SubtitleFetchRequest {
    pub ytdlp_path: PathBuf,
    pub url: String,
    pub language: Option<String>,
    pub video_language: Option<String>,
    pub ui_language: Option<String>,
    pub temp_dir: PathBuf,
    pub output_dir: PathBuf,
    pub base_filename: String,
    pub transcripts_subfolder: bool,
    pub ffmpeg_location: Option<PathBuf>,
    pub js_runtime_args: Vec<std::ffi::OsString>,
    pub cookies: crate::services::ytdlp::CookieSource,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    pub custom_headers: Vec<String>,
    pub proxy: Option<String>,
    pub limit_rate: Option<String>,
}

impl Default for SubtitleFetchRequest {
    fn default() -> Self {
        Self {
            ytdlp_path: PathBuf::new(),
            url: String::new(),
            language: None,
            video_language: None,
            ui_language: None,
            temp_dir: PathBuf::new(),
            output_dir: PathBuf::new(),
            base_filename: String::new(),
            transcripts_subfolder: false,
            ffmpeg_location: None,
            js_runtime_args: Vec::new(),
            cookies: crate::services::ytdlp::CookieSource::None,
            user_agent: None,
            referer: None,
            custom_headers: Vec::new(),
            proxy: None,
            limit_rate: None,
        }
    }
}

/// Result of subtitle fetching and extraction.
#[derive(Debug, Clone)]
pub struct SubtitleFetchOutcome {
    pub srt_file: PathBuf,
    pub txt_file: PathBuf,
    pub origin: SubtitleOrigin,
    pub language: String,
}

/// Normalize language tags (e.g. "it-IT" -> "it", "en_US" -> "en").
pub fn normalize_language_tag(lang: &str) -> Option<String> {
    let trimmed = lang.trim();
    if trimmed.is_empty() {
        return None;
    }
    let primary = trimmed
        .split(['-', '_'])
        .next()?
        .trim()
        .to_ascii_lowercase();
    if primary.is_empty() || primary == "auto" {
        None
    } else {
        Some(primary)
    }
}

/// Resolve subtitle languages in preference order according to spec §10.1:
/// (1) user's chosen language,
/// (2) video's language metadata,
/// (3) app UI language + English ("it,en" or "en,it").
pub fn resolve_subtitle_languages(
    requested_lang: Option<&str>,
    video_lang: Option<&str>,
    ui_lang: Option<&str>,
) -> Vec<String> {
    if let Some(req) = requested_lang.and_then(normalize_language_tag) {
        return vec![req];
    }
    if let Some(vid) = video_lang.and_then(normalize_language_tag) {
        return vec![vid];
    }
    let ui = ui_lang
        .and_then(normalize_language_tag)
        .unwrap_or_else(|| "it".to_string());
    if ui == "en" {
        vec!["en".to_string(), "it".to_string()]
    } else {
        vec![ui, "en".to_string()]
    }
}

/// Build sub-langs argument for yt-dlp (spec §10.1: exclude -live_chat, prefer -orig for YouTube).
pub fn build_sub_langs_arg(
    requested_lang: Option<&str>,
    video_lang: Option<&str>,
    ui_lang: Option<&str>,
    auto_subs: bool,
) -> String {
    let langs = resolve_subtitle_languages(requested_lang, video_lang, ui_lang);
    if auto_subs {
        let mut parts = Vec::new();
        for lang in &langs {
            parts.push(format!("{lang}-orig"));
            parts.push(lang.clone());
        }
        parts.push("-live_chat".to_string());
        parts.join(",")
    } else {
        let mut parts = langs;
        parts.push("-live_chat".to_string());
        parts.join(",")
    }
}

/// Attempt to fetch subtitles via yt-dlp (spec §10.1).
///
/// Tries manual subtitles first; if none are found, tries automatic captions.
/// Returns `Ok(Some(outcome))` if subtitles were retrieved, `Ok(None)` if no subtitles exist.
pub async fn fetch_subtitles(
    req: SubtitleFetchRequest,
    token: CancellationToken,
) -> Result<Option<SubtitleFetchOutcome>, AppError> {
    if !req.temp_dir.exists() {
        fs::create_dir_all(&req.temp_dir).map_err(AppError::Io)?;
    }

    struct SubTempGuard<'a>(&'a Path);
    impl<'a> Drop for SubTempGuard<'a> {
        fn drop(&mut self) {
            clean_temp_sub_files(self.0);
        }
    }
    let _guard = SubTempGuard(&req.temp_dir);

    // 1. Try manual subtitles first
    let manual_res = run_ytdlp_sub_fetch(&req, false, &token).await?;
    if let Some((sub_path, lang)) = manual_res {
        return process_downloaded_subtitles(&req, sub_path, SubtitleOrigin::Manual, lang);
    }

    if token.is_cancelled() {
        return Err(AppError::Process("Cancelled".to_string()));
    }

    // 2. Try automatic captions
    let auto_res = run_ytdlp_sub_fetch(&req, true, &token).await?;
    if let Some((sub_path, lang)) = auto_res {
        return process_downloaded_subtitles(&req, sub_path, SubtitleOrigin::Auto, lang);
    }

    Ok(None)
}

pub fn build_sub_fetch_command(req: &SubtitleFetchRequest, auto_subs: bool) -> ToolCommand {
    let sub_langs = build_sub_langs_arg(
        req.language.as_deref(),
        req.video_language.as_deref(),
        req.ui_language.as_deref(),
        auto_subs,
    );
    let output_tmpl = req.temp_dir.join("sub.%(ext)s");

    let mut cmd = ToolCommand::new(&req.ytdlp_path)
        .temp_dir(&req.temp_dir)
        .arg("--skip-download")
        .arg("--ignore-config");

    // Encoding (Spec §4.8)
    cmd = cmd.args(crate::services::ytdlp::ENCODING_ARGS);

    // Subtitle conversion / ffmpeg location (Spec §4.8)
    if let Some(ffmpeg) = &req.ffmpeg_location {
        cmd = cmd.arg("--ffmpeg-location").arg(ffmpeg.as_os_str());
    }

    // JS runtime args (Spec §5.3.7)
    for arg in &req.js_runtime_args {
        cmd = cmd.arg(arg);
    }

    if auto_subs {
        cmd = cmd.arg("--write-auto-subs");
    } else {
        cmd = cmd.arg("--write-subs").arg("--no-write-auto-subs");
    }
    cmd = cmd
        .arg("--sub-langs")
        .arg(&sub_langs)
        .arg("--sub-format")
        .arg("srt/vtt/best")
        .arg("--convert-subs")
        .arg("srt")
        .arg("-o")
        .arg(output_tmpl.to_string_lossy().as_ref());

    // Cookies (Spec §8.6)
    for cookie_arg in req.cookies.args() {
        cmd = cmd.arg(cookie_arg);
    }

    // Proxy (Spec §8.2, §11.8)
    if let Some(proxy) = req
        .proxy
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        cmd = cmd.arg("--proxy").arg(proxy);
    }

    // Rate limit (Spec §8.2, §11.8)
    if let Some(rate) = req
        .limit_rate
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        cmd = cmd.arg("--limit-rate").arg(rate);
    }

    // Referer (Spec §6.3.4, §8.2)
    if let Some(referer) = req
        .referer
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        cmd = cmd.arg("--referer").arg(referer);
    }

    // User Agent (Spec §6.3.4, §8.2)
    if let Some(ua) = req
        .user_agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        cmd = cmd.arg("--user-agent").arg(ua);
    }

    // Custom headers (Spec §6.3.4, §8.2)
    for header in &req.custom_headers {
        let trimmed = header.trim();
        if !trimmed.is_empty() {
            cmd = cmd.arg("--add-headers").arg(trimmed);
        }
    }

    cmd.url(&req.url)
}

async fn run_ytdlp_sub_fetch(
    req: &SubtitleFetchRequest,
    auto_subs: bool,
    token: &CancellationToken,
) -> Result<Option<(PathBuf, String)>, AppError> {
    // Clear any previous sub files in temp_dir
    clean_temp_sub_files(&req.temp_dir);

    let langs = resolve_subtitle_languages(
        req.language.as_deref(),
        req.video_language.as_deref(),
        req.ui_language.as_deref(),
    );
    let cmd = build_sub_fetch_command(req, auto_subs);

    let result = process::spawn_tool(cmd, token.clone(), process::discard_lines()).await?;
    if result.cancelled {
        return Err(AppError::Process("Cancelled".to_string()));
    }

    // Search for produced .srt files in temp_dir
    let entries = fs::read_dir(&req.temp_dir).map_err(AppError::Io)?;
    let mut found: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().is_some_and(|ext| ext == "srt") {
            found.push(p);
        }
    }

    if found.is_empty() {
        return Ok(None);
    }

    let preference_tags: Vec<String> = if auto_subs {
        langs
            .iter()
            .flat_map(|l| [format!("{l}-orig"), l.clone()])
            .collect()
    } else {
        langs
    };

    found.sort_by(|a, b| {
        let rank_a = sub_file_rank(a, &preference_tags);
        let rank_b = sub_file_rank(b, &preference_tags);
        rank_a.cmp(&rank_b)
    });

    let chosen = found[0].clone();
    let lang = extract_lang_from_sub_filename(&chosen).unwrap_or_else(|| "auto".to_string());
    Ok(Some((chosen, lang)))
}

pub fn rank_subtitle_files(files: &[PathBuf], preference_tags: &[String]) -> Vec<PathBuf> {
    let mut sorted = files.to_vec();
    sorted.sort_by(|a, b| {
        let rank_a = sub_file_rank(a, preference_tags);
        let rank_b = sub_file_rank(b, preference_tags);
        rank_a.cmp(&rank_b)
    });
    sorted
}

fn sub_file_rank(path: &Path, preference_tags: &[String]) -> (usize, String) {
    let filename = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let raw_tag = stem.split_once('.').map(|(_, tag)| tag).unwrap_or("");

    // 1. Exact match in preference_tags
    if let Some(pos) = preference_tags
        .iter()
        .position(|t| t.eq_ignore_ascii_case(raw_tag))
    {
        return (pos, filename);
    }

    // 2. Normalized base language match (preserving -orig if present)
    let is_orig = raw_tag.ends_with("-orig");
    let base = raw_tag.strip_suffix("-orig").unwrap_or(raw_tag);
    if let Some(norm_base) = normalize_language_tag(base) {
        let candidate = if is_orig {
            format!("{norm_base}-orig")
        } else {
            norm_base
        };
        if let Some(pos) = preference_tags
            .iter()
            .position(|t| t.eq_ignore_ascii_case(&candidate))
        {
            return (pos, filename);
        }
    }

    (usize::MAX, filename)
}

fn extract_lang_from_sub_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    // Typically `sub.<lang>` or `sub.<lang>-orig`
    if let Some((_, raw_lang)) = stem.split_once('.') {
        let stripped = raw_lang.strip_suffix("-orig").unwrap_or(raw_lang);
        let lang = normalize_language_tag(stripped).unwrap_or_else(|| stripped.to_string());
        Some(lang)
    } else {
        None
    }
}

fn clean_temp_sub_files(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.file_name()
                    .is_some_and(|s| s.to_string_lossy().starts_with("sub"))
            {
                let _ = fs::remove_file(p);
            }
        }
    }
}

fn process_downloaded_subtitles(
    req: &SubtitleFetchRequest,
    sub_path: PathBuf,
    origin: SubtitleOrigin,
    lang: String,
) -> Result<Option<SubtitleFetchOutcome>, AppError> {
    let srt_content = fs::read_to_string(&sub_path).map_err(AppError::Io)?;
    let cleaned_text = clean_subtitles(&srt_content);

    // Determine target folder
    let target_dir = if req.transcripts_subfolder {
        req.output_dir.join("transcripts")
    } else {
        req.output_dir.clone()
    };
    if !target_dir.exists() {
        fs::create_dir_all(&target_dir).map_err(AppError::Io)?;
    }

    // Derive collision-free paths (spec §10.1, Regression F.2)
    let candidate_paths = |stem: &str| -> (PathBuf, PathBuf) {
        let srt_name = match origin {
            SubtitleOrigin::Manual => format!("{stem}.{lang}.srt"),
            SubtitleOrigin::Auto => format!("{stem}.{lang}.auto.srt"),
        };
        let txt_name = format!("{stem}.txt");
        (target_dir.join(srt_name), target_dir.join(txt_name))
    };

    let free_stem = paths::collision_free_stem(&req.base_filename, |stem| {
        let (srt_p, txt_p) = candidate_paths(stem);
        srt_p.exists() || txt_p.exists()
    });

    let (target_srt, target_txt) = candidate_paths(&free_stem);

    paths::atomic_write(&target_srt, srt_content.as_bytes())?;
    paths::atomic_write(&target_txt, cleaned_text.as_bytes())?;

    // Clean up temp file
    let _ = fs::remove_file(sub_path);

    Ok(Some(SubtitleFetchOutcome {
        srt_file: target_srt,
        txt_file: target_txt,
        origin,
        language: lang,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_markup_tags_removes_html_and_vtt_tags() {
        let input = "<font color=\"#ffff00\">Hello</font> <b>world</b> <c.yellow>test</c> &amp; &lt;more&gt;";
        let clean = strip_markup_tags(input);
        assert_eq!(clean, "Hello world test & <more>");
    }

    #[test]
    fn parse_srt_cues_extracts_clean_cues() {
        let srt = "1\n00:00:01,000 --> 00:00:03,500\n<b>Hello</b> world\n\n2\n00:00:04,000 --> 00:00:06,000\nSecond cue\n";
        let cues = parse_srt_cues(srt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[0].end_ms, 3500);
        assert_eq!(cues[0].text, "Hello world");
        assert_eq!(cues[1].start_ms, 4000);
        assert_eq!(cues[1].end_ms, 6000);
        assert_eq!(cues[1].text, "Second cue");
    }

    #[test]
    fn clean_subtitle_cues_removes_rolling_repeated_lines() {
        // Typical YouTube ASR rolling lines:
        // Cue 1: Line 1 + Line 2
        // Cue 2: Line 2 + Line 3
        // Cue 3: Line 3 + Line 4
        let cues = vec![
            SubtitleCue {
                start_ms: 1000,
                end_ms: 2500,
                text: "Benvenuti a tutti\nin questo nuovo video".to_string(),
            },
            SubtitleCue {
                start_ms: 2500,
                end_ms: 4000,
                text: "in questo nuovo video\noggi parleremo di Rust".to_string(),
            },
            SubtitleCue {
                start_ms: 4000,
                end_ms: 5500,
                text: "oggi parleremo di Rust\ne delle sue meraviglie".to_string(),
            },
        ];

        let cleaned = clean_subtitle_cues(&cues, 2000);
        assert_eq!(
            cleaned,
            "Benvenuti a tutti in questo nuovo video oggi parleremo di Rust e delle sue meraviglie"
        );
    }

    #[test]
    fn clean_subtitle_cues_removes_rolling_word_overlap() {
        let cues = vec![
            SubtitleCue {
                start_ms: 1000,
                end_ms: 2000,
                text: "welcome to our".to_string(),
            },
            SubtitleCue {
                start_ms: 2000,
                end_ms: 3000,
                text: "to our channel today".to_string(),
            },
            SubtitleCue {
                start_ms: 3000,
                end_ms: 4000,
                text: "channel today we are".to_string(),
            },
        ];

        let cleaned = clean_subtitle_cues(&cues, 2000);
        assert_eq!(cleaned, "welcome to our channel today we are");
    }

    #[test]
    fn clean_subtitle_cues_creates_paragraphs_on_long_pauses() {
        let cues = vec![
            SubtitleCue {
                start_ms: 1000,
                end_ms: 2500,
                text: "First sentence here.".to_string(),
            },
            // Pause of 3.5 seconds (6000 - 2500 = 3500 >= 2000)
            SubtitleCue {
                start_ms: 6000,
                end_ms: 8000,
                text: "Second paragraph starts here.".to_string(),
            },
        ];

        let cleaned = clean_subtitle_cues(&cues, 2000);
        assert_eq!(
            cleaned,
            "First sentence here.\n\nSecond paragraph starts here."
        );
    }

    #[test]
    fn build_sub_langs_arg_formats_correctly() {
        assert_eq!(
            build_sub_langs_arg(Some("it"), None, None, false),
            "it,-live_chat"
        );
        assert_eq!(
            build_sub_langs_arg(Some("it"), None, None, true),
            "it-orig,it,-live_chat"
        );
        // Default when requested is None and UI is Italian or unspecified: "it,en" without ",all"
        assert_eq!(
            build_sub_langs_arg(None, None, None, false),
            "it,en,-live_chat"
        );
        assert_eq!(
            build_sub_langs_arg(None, None, None, true),
            "it-orig,it,en-orig,en,-live_chat"
        );
    }

    #[test]
    fn subtitles_m1_language_normalization_and_orig_stripping() {
        // 1. Tag normalisation: "it-IT" -> "it", "en_US" -> "en"
        assert_eq!(
            build_sub_langs_arg(Some("it-IT"), None, None, false),
            "it,-live_chat"
        );
        assert_eq!(
            build_sub_langs_arg(Some("en_US"), None, None, true),
            "en-orig,en,-live_chat"
        );

        // 2. Removal of ",all" and fallback to video language metadata or UI language
        let fallback_default = build_sub_langs_arg(None, None, None, false);
        assert!(!fallback_default.contains("all"));
        assert_eq!(fallback_default, "it,en,-live_chat");

        // Fallback to video metadata language when requested_lang is "auto" or None
        assert_eq!(
            build_sub_langs_arg(Some("auto"), Some("de-DE"), Some("en"), false),
            "de,-live_chat"
        );
        assert_eq!(
            build_sub_langs_arg(None, Some("es_ES"), None, true),
            "es-orig,es,-live_chat"
        );

        // Fallback to UI language when requested and video lang are None/"auto"
        assert_eq!(
            build_sub_langs_arg(None, None, Some("en"), false),
            "en,it,-live_chat"
        );
        assert_eq!(
            build_sub_langs_arg(Some("auto"), None, Some("en"), true),
            "en-orig,en,it-orig,it,-live_chat"
        );

        // 3. Stripping of -orig in extract_lang_from_sub_filename
        assert_eq!(
            extract_lang_from_sub_filename(Path::new("sub.it-orig.srt")),
            Some("it".to_string())
        );
        assert_eq!(
            extract_lang_from_sub_filename(Path::new("sub.it.srt")),
            Some("it".to_string())
        );
        assert_eq!(
            extract_lang_from_sub_filename(Path::new("sub.en-US-orig.srt")),
            Some("en".to_string())
        );
        assert_eq!(extract_lang_from_sub_filename(Path::new("sub.srt")), None);

        // 4. Deterministic ranking preferring -orig over translated and preferred over arbitrary
        let files = vec![
            PathBuf::from("/tmp/sub.ar.srt"),
            PathBuf::from("/tmp/sub.it.srt"),
            PathBuf::from("/tmp/sub.it-orig.srt"),
            PathBuf::from("/tmp/sub.en.srt"),
        ];
        let preference = vec![
            "it-orig".to_string(),
            "it".to_string(),
            "en-orig".to_string(),
            "en".to_string(),
        ];
        let ranked = rank_subtitle_files(&files, &preference);
        assert_eq!(
            ranked,
            vec![
                PathBuf::from("/tmp/sub.it-orig.srt"),
                PathBuf::from("/tmp/sub.it.srt"),
                PathBuf::from("/tmp/sub.en.srt"),
                PathBuf::from("/tmp/sub.ar.srt"),
            ]
        );
    }

    #[test]
    fn process_downloaded_subtitles_does_not_overwrite_existing_files() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let out_dir = temp.path().join("out");
        fs::create_dir_all(&out_dir).expect("create out");
        let temp_dir = temp.path().join("tmp");
        fs::create_dir_all(&temp_dir).expect("create tmp");

        let existing_txt = out_dir.join("lecture.txt");
        let existing_srt = out_dir.join("lecture.en.srt");
        fs::write(&existing_txt, "user text").expect("write existing txt");
        fs::write(&existing_srt, "user srt").expect("write existing srt");

        let sub_temp = temp_dir.join("sub.en.srt");
        fs::write(&sub_temp, "1\n00:00:01,000 --> 00:00:02,000\nSub text\n").expect("write sub");

        let req = SubtitleFetchRequest {
            ytdlp_path: PathBuf::from("fake_ytdlp"),
            url: "https://example.com".to_string(),
            language: Some("en".to_string()),
            video_language: None,
            ui_language: None,
            temp_dir,
            output_dir: out_dir.clone(),
            base_filename: "lecture".to_string(),
            transcripts_subfolder: false,
            ..Default::default()
        };

        let outcome =
            process_downloaded_subtitles(&req, sub_temp, SubtitleOrigin::Manual, "en".to_string())
                .expect("process subtitles")
                .expect("outcome");

        assert_eq!(
            fs::read_to_string(&existing_txt).expect("read txt"),
            "user text",
            "existing .txt must not be overwritten"
        );
        assert_eq!(
            fs::read_to_string(&existing_srt).expect("read srt"),
            "user srt",
            "existing .srt must not be overwritten"
        );

        assert_eq!(outcome.txt_file, out_dir.join("lecture (1).txt"));
        assert_eq!(outcome.srt_file, out_dir.join("lecture (1).en.srt"));
        assert!(outcome.txt_file.is_file());
        assert!(outcome.srt_file.is_file());
    }

    #[tokio::test]
    async fn fetch_subtitles_guard_cleans_temp_sub_files_on_failure() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let sub_temp = temp_dir.path().join("sub.it.srt");
        fs::write(&sub_temp, b"1\n00:00:01,000 --> 00:00:02,000\nHello\n").expect("write sub");
        assert!(sub_temp.exists());

        let req = SubtitleFetchRequest {
            ytdlp_path: PathBuf::from("nonexistent_ytdlp_binary_xyz"),
            url: "https://example.com/watch?v=123".to_string(),
            output_dir: temp_dir.path().to_path_buf(),
            temp_dir: temp_dir.path().to_path_buf(),
            base_filename: "test_video".to_string(),
            language: None,
            video_language: None,
            ui_language: None,
            transcripts_subfolder: false,
            ..Default::default()
        };

        let token = CancellationToken::new();
        let _ = fetch_subtitles(req, token).await;

        // Any sub.* files in temp_dir must be cleaned up by RAII guard
        assert!(
            !sub_temp.exists(),
            "temp sub files must be cleaned up on exit of fetch_subtitles"
        );
    }

    #[test]
    fn subtitles_fetch_command_includes_required_flags_and_temp_dir() {
        use std::ffi::OsString;

        let req = SubtitleFetchRequest {
            ytdlp_path: PathBuf::from("/bin/yt-dlp"),
            url: "https://www.youtube.com/watch?v=123".to_string(),
            language: Some("en".to_string()),
            video_language: None,
            ui_language: Some("it".to_string()),
            temp_dir: PathBuf::from("/private/tmp/job-1"),
            output_dir: PathBuf::from("/downloads"),
            base_filename: "test".to_string(),
            transcripts_subfolder: false,
            ffmpeg_location: Some(PathBuf::from("/tools/ffmpeg")),
            js_runtime_args: vec![
                OsString::from("--no-js-runtimes"),
                OsString::from("--js-runtimes"),
                OsString::from("quickjs:/tools/qjs"),
            ],
            cookies: crate::services::ytdlp::CookieSource::Browser("chrome".to_string()),
            user_agent: Some("CustomUA/1.0".to_string()),
            referer: Some("https://example.com/ref".to_string()),
            custom_headers: vec!["X-Custom: 1".to_string()],
            proxy: Some("http://proxy.example.com:8080".to_string()),
            limit_rate: Some("500K".to_string()),
        };

        let cmd = build_sub_fetch_command(&req, false);

        // 1. Private temp dir (Spec §4.9)
        assert_eq!(cmd.env, process::temp_env(&req.temp_dir));

        let argv: Vec<String> = cmd
            .argv()
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect();

        // 2. Encoding utf-8 (Spec §4.8)
        let enc_pos = argv.iter().position(|a| a == "--encoding");
        assert!(enc_pos.is_some(), "missing --encoding");
        assert_eq!(
            argv.get(enc_pos.unwrap() + 1).map(|s| s.as_str()),
            Some("utf-8")
        );

        // 3. ffmpeg location (Spec §4.8)
        let ff_pos = argv.iter().position(|a| a == "--ffmpeg-location");
        assert!(ff_pos.is_some(), "missing --ffmpeg-location");
        assert_eq!(
            argv.get(ff_pos.unwrap() + 1).map(|s| s.as_str()),
            Some("/tools/ffmpeg")
        );

        // 4. JS runtime args (Spec §5.3.7)
        assert!(argv.contains(&"--no-js-runtimes".to_string()));
        assert!(argv.contains(&"--js-runtimes".to_string()));
        assert!(argv.contains(&"quickjs:/tools/qjs".to_string()));

        // 5. Auth and network flags
        assert!(argv.contains(&"--cookies-from-browser".to_string()));
        assert!(argv.contains(&"chrome".to_string()));
        assert!(argv.contains(&"--proxy".to_string()));
        assert!(argv.contains(&"http://proxy.example.com:8080".to_string()));
        assert!(argv.contains(&"--limit-rate".to_string()));
        assert!(argv.contains(&"500K".to_string()));
        assert!(argv.contains(&"--referer".to_string()));
        assert!(argv.contains(&"https://example.com/ref".to_string()));
        assert!(argv.contains(&"--user-agent".to_string()));
        assert!(argv.contains(&"CustomUA/1.0".to_string()));
        assert!(argv.contains(&"--add-headers".to_string()));
        assert!(argv.contains(&"X-Custom: 1".to_string()));
    }

    #[test]
    fn subtitles_m3_rollups_and_punctuation_overlap() {
        // 1. Punctuation in overlap prefix must not distort slice index
        let vtt = "\
WEBVTT
Kind: captions
Language: en

00:00:01.000 --> 00:00:03.000 align:start position:0%
hello -<00:00:01.500><c> world</c><00:00:02.000><c> this</c>

00:00:03.000 --> 00:00:05.000 align:start position:0%
hello - world this
is<00:00:03.500><c> a</c><00:00:04.000><c> test</c>

00:00:05.000 --> 00:00:06.500 align:start position:0%
is a test
";
        let cleaned = clean_subtitles(vtt);
        assert_eq!(cleaned, "hello - world this is a test");

        // 2. Multi-line roll-up cue `Line 1\nLine 2` followed by freeze to `Line 1`
        let cues = vec![
            SubtitleCue {
                start_ms: 1000,
                end_ms: 2000,
                text: "First line\nSecond line".to_string(),
            },
            SubtitleCue {
                start_ms: 2000,
                end_ms: 3000,
                text: "First line".to_string(),
            },
            SubtitleCue {
                start_ms: 3000,
                end_ms: 4000,
                text: "Second line\nThird line".to_string(),
            },
        ];
        let cleaned_cues = clean_subtitle_cues(&cues, 2000);
        assert_eq!(cleaned_cues, "First line Second line Third line");

        // 3. Multi-line roll-up block where line 1 was emitted two cues earlier
        let cues_rollup = vec![
            SubtitleCue {
                start_ms: 1000,
                end_ms: 2000,
                text: "Line 1".to_string(),
            },
            SubtitleCue {
                start_ms: 2000,
                end_ms: 3000,
                text: "Line 1\nLine 2".to_string(),
            },
            SubtitleCue {
                start_ms: 3000,
                end_ms: 4000,
                text: "Line 1\nLine 2\nLine 3".to_string(),
            },
        ];
        let cleaned_rollup = clean_subtitle_cues(&cues_rollup, 2000);
        assert_eq!(cleaned_rollup, "Line 1 Line 2 Line 3");
    }

    #[test]
    fn clean_vtt_with_inline_word_timestamps_and_punctuation_rollups() {
        let vtt = "\
WEBVTT
Kind: captions
Language: en

00:00:01.000 --> 00:00:03.000 align:start position:0%
hello -<00:00:01.520><c> world,</c><00:00:02.000><c> this</c>

00:00:03.000 --> 00:00:05.000 align:start position:0%
hello - world, this
is<00:00:03.500><c> a</c><00:00:04.000><c> test</c><00:00:04.500><c> !</c>

00:00:05.000 --> 00:00:06.500 align:start position:0%
is a test !
";
        let cleaned = clean_subtitles(vtt);
        assert_eq!(cleaned, "hello - world, this is a test !");
    }
}
