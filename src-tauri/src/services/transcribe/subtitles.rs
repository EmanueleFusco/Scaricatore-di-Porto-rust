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

/// Find word-level overlap between the end of `words_a` and the beginning of `words_b`.
fn find_word_overlap(words_a: &[&str], words_b: &[&str]) -> usize {
    let norm_a: Vec<String> = words_a
        .iter()
        .map(|w| normalize_word(w))
        .filter(|w| !w.is_empty())
        .collect();
    let norm_b: Vec<String> = words_b
        .iter()
        .map(|w| normalize_word(w))
        .filter(|w| !w.is_empty())
        .collect();

    let max_k = norm_a.len().min(norm_b.len());
    for k in (1..=max_k).rev() {
        if norm_a[norm_a.len() - k..] == norm_b[..k] {
            return k;
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
            }
        }

        // Clean cue text: split by lines, de-duplicate within multi-line cues
        let cue_lines: Vec<&str> = cue
            .text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        // Flatten cue lines into words
        let mut cue_words: Vec<&str> = Vec::new();
        for (idx, line) in cue_lines.iter().enumerate() {
            if idx > 0 && line == &cue_lines[idx - 1] {
                // Skip identical repeated lines within the same cue
                continue;
            }
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
    pub temp_dir: PathBuf,
    pub output_dir: PathBuf,
    pub base_filename: String,
    pub transcripts_subfolder: bool,
}

/// Result of subtitle fetching and extraction.
#[derive(Debug, Clone)]
pub struct SubtitleFetchOutcome {
    pub srt_file: PathBuf,
    pub txt_file: PathBuf,
    pub origin: SubtitleOrigin,
    pub language: String,
}

/// Build sub-langs argument for yt-dlp (spec §10.1: exclude -live_chat, prefer -orig for YouTube).
pub fn build_sub_langs_arg(requested_lang: Option<&str>, auto_subs: bool) -> String {
    let lang = requested_lang.unwrap_or("auto").trim();
    if lang == "auto" || lang.is_empty() {
        if auto_subs {
            "it-orig,it,en-orig,en,all,-live_chat".to_string()
        } else {
            "it,en,all,-live_chat".to_string()
        }
    } else if auto_subs {
        format!("{lang}-orig,{lang},-live_chat")
    } else {
        format!("{lang},-live_chat")
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

async fn run_ytdlp_sub_fetch(
    req: &SubtitleFetchRequest,
    auto_subs: bool,
    token: &CancellationToken,
) -> Result<Option<(PathBuf, String)>, AppError> {
    // Clear any previous sub files in temp_dir
    clean_temp_sub_files(&req.temp_dir);

    let sub_langs = build_sub_langs_arg(req.language.as_deref(), auto_subs);
    let output_tmpl = req.temp_dir.join("sub.%(ext)s");

    let mut cmd = ToolCommand::new(&req.ytdlp_path)
        .arg("--skip-download")
        .arg("--ignore-config");
    if auto_subs {
        cmd = cmd.arg("--write-auto-subs");
    } else {
        cmd = cmd.arg("--write-subs").arg("--no-write-auto-subs");
    }
    let cmd = cmd
        .arg("--sub-langs")
        .arg(&sub_langs)
        .arg("--sub-format")
        .arg("srt/vtt/best")
        .arg("--convert-subs")
        .arg("srt")
        .arg("-o")
        .arg(output_tmpl.to_string_lossy().as_ref())
        .url(&req.url);

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

    // Pick the best match (e.g. matching requested language or first found)
    let chosen = found[0].clone();
    let lang = extract_lang_from_sub_filename(&chosen).unwrap_or_else(|| "auto".to_string());
    Ok(Some((chosen, lang)))
}

fn extract_lang_from_sub_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    // Typically `sub.<lang>` or `sub.<lang>-orig`
    if let Some((_, lang)) = stem.split_once('.') {
        Some(lang.to_string())
    } else {
        None
    }
}

fn clean_temp_sub_files(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.file_stem()
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

    // Sidecar naming: `.it.srt` or `.it.auto.srt`
    let srt_name = match origin {
        SubtitleOrigin::Manual => format!("{}.{}.srt", req.base_filename, lang),
        SubtitleOrigin::Auto => format!("{}.{}.auto.srt", req.base_filename, lang),
    };
    let txt_name = format!("{}.txt", req.base_filename);

    let target_srt = target_dir.join(srt_name);
    let target_txt = target_dir.join(txt_name);

    fs::copy(&sub_path, &target_srt).map_err(AppError::Io)?;
    fs::write(&target_txt, cleaned_text.as_bytes()).map_err(AppError::Io)?;

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
        assert_eq!(build_sub_langs_arg(Some("it"), false), "it,-live_chat");
        assert_eq!(
            build_sub_langs_arg(Some("it"), true),
            "it-orig,it,-live_chat"
        );
        assert_eq!(build_sub_langs_arg(None, false), "it,en,all,-live_chat");
        assert_eq!(
            build_sub_langs_arg(None, true),
            "it-orig,it,en-orig,en,all,-live_chat"
        );
    }
}
