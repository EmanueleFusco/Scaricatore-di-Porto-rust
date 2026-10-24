//! Optional summary generation via Gemini REST API (spec §10.4).
//!
//! Input: transcript text (cheap, fast, no audio upload needed).
//! Output: `<name>.summary.md` with:
//! - Summary (concise overview)
//! - Key Points (bulleted takeaways)
//! - Chapters (timestamped sections when timestamps are present in transcript cues; omitted otherwise)
//!
//! For very long transcripts, uses a map-reduce strategy (summarize chunks, then synthesize).

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use reqwest::StatusCode;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::services::paths;
use crate::services::transcribe::gemini::{
    DEFAULT_GEMINI_BASE_URL, DEFAULT_GEMINI_MODEL, MAX_RETRIES, classify_gemini_http_error,
};

/// Maximum chunk character length for single-shot summarization (~12,000 words).
pub const MAX_SUMMARY_CHUNK_CHARS: usize = 50_000;

/// Split text into units by sentence delimiters.
fn split_sentences(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if (bytes[i] == b'.' || bytes[i] == b'!' || bytes[i] == b'?' || bytes[i] == b';')
            && i + 1 < bytes.len()
            && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t')
        {
            parts.push(&text[start..=i]);
            start = i + 2;
            i += 1;
        }
        i += 1;
    }
    if start < text.len() {
        parts.push(&text[start..]);
    }
    parts
}

/// Slices a large block along character boundaries when no higher-level delimiters exist.
fn split_char_boundaries(text: &str, max_chunk_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cur_start = 0;
    let mut last_char_end = 0;

    for (idx, ch) in text.char_indices() {
        let ch_end = idx + ch.len_utf8();
        if ch_end - cur_start > max_chunk_chars {
            if last_char_end > cur_start {
                chunks.push(text[cur_start..last_char_end].to_string());
                cur_start = last_char_end;
            } else {
                chunks.push(text[cur_start..ch_end].to_string());
                cur_start = ch_end;
                last_char_end = cur_start;
                continue;
            }
        }
        last_char_end = ch_end;
    }

    if cur_start < text.len() {
        chunks.push(text[cur_start..].to_string());
    }

    chunks
}

/// Pack split items into chunks up to `max_chunk_chars`, recursing to `next_level`
/// whenever an individual block exceeds `max_chunk_chars`.
fn pack_parts(parts: &[&str], sep: &str, max_chunk_chars: usize, next_level: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();

    for part in parts {
        if part.is_empty() {
            continue;
        }
        if part.len() <= max_chunk_chars {
            if current_chunk.is_empty() {
                current_chunk.push_str(part);
            } else if current_chunk.len() + sep.len() + part.len() <= max_chunk_chars {
                current_chunk.push_str(sep);
                current_chunk.push_str(part);
            } else {
                chunks.push(current_chunk);
                current_chunk = part.to_string();
            }
        } else {
            if !current_chunk.is_empty() {
                chunks.push(current_chunk);
                current_chunk = String::new();
            }
            let mut sub_chunks = split_into_chunks_hierarchical(part, max_chunk_chars, next_level);
            if !sub_chunks.is_empty() {
                let last = sub_chunks.pop().unwrap_or_default();
                chunks.extend(sub_chunks);
                current_chunk = last;
            }
        }
    }

    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }

    chunks
}

fn split_into_chunks_hierarchical(text: &str, max_chunk_chars: usize, level: usize) -> Vec<String> {
    if text.len() <= max_chunk_chars {
        return vec![text.to_string()];
    }

    match level {
        0 => {
            if text.contains("\n\n") {
                let parts: Vec<&str> = text.split("\n\n").collect();
                pack_parts(&parts, "\n\n", max_chunk_chars, 1)
            } else {
                split_into_chunks_hierarchical(text, max_chunk_chars, 1)
            }
        }
        1 => {
            if text.contains('\n') {
                let parts: Vec<&str> = text.split('\n').collect();
                pack_parts(&parts, "\n", max_chunk_chars, 2)
            } else {
                split_into_chunks_hierarchical(text, max_chunk_chars, 2)
            }
        }
        2 => {
            let parts = split_sentences(text);
            if parts.len() > 1 {
                pack_parts(&parts, " ", max_chunk_chars, 3)
            } else {
                split_into_chunks_hierarchical(text, max_chunk_chars, 3)
            }
        }
        3 => {
            if text.contains(' ') {
                let parts: Vec<&str> = text.split(' ').collect();
                pack_parts(&parts, " ", max_chunk_chars, 4)
            } else {
                split_into_chunks_hierarchical(text, max_chunk_chars, 4)
            }
        }
        _ => split_char_boundaries(text, max_chunk_chars),
    }
}

/// Split transcript text into chunks around paragraph boundaries (`\n\n`),
/// falling back to single newlines (`\n`), sentence boundaries, word boundaries,
/// or character slicing when a block exceeds `max_chunk_chars`.
pub fn split_text_into_chunks(text: &str, max_chunk_chars: usize) -> Vec<String> {
    if max_chunk_chars == 0 || text.is_empty() {
        return vec![text.to_string()];
    }
    split_into_chunks_hierarchical(text, max_chunk_chars, 0)
}

/// Build the prompt for single-request summarization.
pub fn build_single_summary_prompt(text: &str, language: &str) -> String {
    let lang_instruction = if language == "auto" || language.trim().is_empty() {
        "in the same language as the transcript".to_string()
    } else {
        format!("in {language}")
    };

    let chapters_instruction = if text.contains("-->")
        || text.lines().any(|l| {
            l.contains(':')
                && (l.contains('[')
                    || l.contains('0')
                    || l.contains('1')
                    || l.contains('2')
                    || l.contains('3'))
        }) {
        "# Chapters\n\
        [Chapter-like sections with timestamps and descriptive titles based on the transcript cues (e.g. `## [mm:ss] Title` or `- mm:ss Title`)]\n\n"
    } else {
        "# Chapters\n\
        [Chapter-like sections with timestamps if timestamps exist in the transcript; otherwise omit this section]\n\n"
    };

    format!(
        "You are an expert summarizer. Summarize the following transcript {lang_instruction}.\n\
        Format your response cleanly in Markdown with the following sections:\n\
        # Summary\n\
        [A concise 2-3 paragraph overview of the content]\n\n\
        # Key Points\n\
        [Bullet points highlighting the essential arguments, takeaways and facts]\n\n\
        {chapters_instruction}\
        Transcript:\n\
        {text}"
    )
}

/// Build the prompt for the map step of long transcripts.
pub fn build_map_chunk_prompt(
    chunk: &str,
    chunk_idx: usize,
    total_chunks: usize,
    language: &str,
) -> String {
    let lang_instruction = if language == "auto" || language.trim().is_empty() {
        "in the same language as the transcript".to_string()
    } else {
        format!("in {language}")
    };

    format!(
        "Summarize Part {chunk_idx}/{total_chunks} of a larger transcript {lang_instruction}.\n\
        Extract the core ideas, discussion points and notable details concisely in bullet points (preserving timestamps for chapter-like milestones if timestamps are present in the text):\n\n\
        {chunk}"
    )
}

/// Build the prompt for the reduce step of long transcripts.
pub fn build_reduce_prompt(chunk_summaries: &[String], language: &str) -> String {
    let lang_instruction = if language == "auto" || language.trim().is_empty() {
        "in the same language as the transcript".to_string()
    } else {
        format!("in {language}")
    };

    let joined_summaries = chunk_summaries.join("\n\n---\n\n");
    format!(
        "Synthesize the following section summaries of a long transcript into a single comprehensive Markdown summary {lang_instruction}.\n\
        Structure the output with:\n\
        # Summary\n\
        [A comprehensive narrative overview connecting all parts]\n\n\
        # Key Points\n\
        [Consolidated bullet points covering the entire content]\n\n\
        # Chapters\n\
        [Consolidated chapter-like sections with timestamps and descriptive titles if timestamps exist in the section summaries; otherwise omit this section]\n\n\
        Section summaries:\n\
        {joined_summaries}"
    )
}

/// Execute a text-only prompt against Gemini `generateContent`.
pub async fn call_gemini_generate_text(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
    token: &CancellationToken,
) -> Result<String, AppError> {
    let base = base_url.trim_end_matches('/');
    let model_id = model.strip_prefix("models/").unwrap_or(model);
    let gen_url = format!("{base}/v1beta/models/{model_id}:generateContent");

    let payload = serde_json::json!({
        "contents": [
            {
                "parts": [
                    { "text": prompt }
                ]
            }
        ]
    });

    for attempt in 0..=MAX_RETRIES {
        if token.is_cancelled() {
            return Err(AppError::Process("Cancelled".to_string()));
        }

        let resp_res = tokio::select! {
            biased;
            _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
            res = client
                .post(&gen_url)
                .header("x-goog-api-key", api_key)
                .header("Content-Type", "application/json")
                .json(&payload)
                .send() => res,
        };

        match resp_res {
            Ok(resp) => {
                let status = resp.status();
                if (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                    && attempt < MAX_RETRIES
                {
                    let delay = Duration::from_millis(500 * (1 << attempt));
                    tokio::select! {
                        _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
                        _ = tokio::time::sleep(delay) => continue,
                    }
                }
                if !status.is_success() {
                    let text = tokio::select! {
                        biased;
                        _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
                        res = resp.text() => res.unwrap_or_default(),
                    };
                    return Err(classify_gemini_http_error(
                        status,
                        &text,
                        Some(model),
                        "Gemini summary request",
                    ));
                }

                let json: serde_json::Value = tokio::select! {
                    biased;
                    _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
                    res = resp.json() => res,
                }
                .map_err(|e| AppError::Process(e.to_string()))?;

                if let Some(block_reason) = json
                    .pointer("/promptFeedback/blockReason")
                    .and_then(|v| v.as_str())
                {
                    return Err(AppError::GeminiBlocked(block_reason.to_string()));
                }

                let candidates = json.get("candidates").and_then(|v| v.as_array());
                let Some(first_candidate) = candidates.and_then(|c| c.first()) else {
                    return Err(AppError::Process(
                        "Gemini returned empty candidates list for summary".to_string(),
                    ));
                };

                let mut out_text = String::new();
                if let Some(parts) = first_candidate
                    .pointer("/content/parts")
                    .and_then(|v| v.as_array())
                {
                    for part in parts {
                        if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                            out_text.push_str(t);
                        }
                    }
                }

                return Ok(out_text);
            }
            Err(e) => {
                if attempt < MAX_RETRIES {
                    let delay = Duration::from_millis(500 * (1 << attempt));
                    tokio::select! {
                        _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
                        _ = tokio::time::sleep(delay) => continue,
                    }
                }
                return Err(AppError::Process(format!(
                    "Network error generating summary with Gemini: {e}"
                )));
            }
        }
    }

    Err(AppError::Process(
        "Gemini summary exceeded max retries".into(),
    ))
}

/// Request configuration for summary generation.
///
/// Custom `Debug` redacts `api_key` so request structs can be logged safely (spec §10.5).
#[derive(Clone)]
pub struct SummaryRequest {
    pub transcript_text: String,
    pub language: Option<String>,
    pub model: Option<String>,
    pub api_key: String,
    pub output_dir: PathBuf,
    pub base_filename: String,
    pub transcripts_subfolder: bool,
    pub base_url: Option<String>,
}

impl std::fmt::Debug for SummaryRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SummaryRequest")
            .field("transcript_text", &self.transcript_text)
            .field("language", &self.language)
            .field("model", &self.model)
            .field("api_key", &"[REDACTED]")
            .field("output_dir", &self.output_dir)
            .field("base_filename", &self.base_filename)
            .field("transcripts_subfolder", &self.transcripts_subfolder)
            .field("base_url", &self.base_url)
            .finish()
    }
}

/// Generate `<name>.summary.md` from the transcript text (spec §10.4).
pub async fn generate_summary(
    req: SummaryRequest,
    token: CancellationToken,
) -> Result<PathBuf, AppError> {
    // Missing key must fail before any HTTP request or output write (spec §10.3 item 7).
    if req.api_key.trim().is_empty() {
        return Err(AppError::GeminiKeyMissing);
    }

    if req.transcript_text.trim().is_empty() {
        return Err(AppError::Process(
            "Cannot summarize empty transcript".to_string(),
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| AppError::Process(e.to_string()))?;

    let base_url = req
        .base_url
        .unwrap_or_else(|| DEFAULT_GEMINI_BASE_URL.to_string());
    let model = req
        .model
        .unwrap_or_else(|| DEFAULT_GEMINI_MODEL.to_string());
    let lang = req.language.unwrap_or_else(|| "auto".to_string());

    let chunks = split_text_into_chunks(&req.transcript_text, MAX_SUMMARY_CHUNK_CHARS);

    let summary_md = if chunks.len() <= 1 {
        // Single-shot summarization
        let prompt = build_single_summary_prompt(&req.transcript_text, &lang);
        call_gemini_generate_text(&client, &base_url, &req.api_key, &model, &prompt, &token).await?
    } else {
        // Map-reduce summarization
        let mut chunk_summaries = Vec::with_capacity(chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            if token.is_cancelled() {
                return Err(AppError::Process("Cancelled".to_string()));
            }
            let map_prompt = build_map_chunk_prompt(chunk, i + 1, chunks.len(), &lang);
            let part_summary = call_gemini_generate_text(
                &client,
                &base_url,
                &req.api_key,
                &model,
                &map_prompt,
                &token,
            )
            .await?;
            chunk_summaries.push(part_summary);
        }

        let reduce_prompt = build_reduce_prompt(&chunk_summaries, &lang);
        call_gemini_generate_text(
            &client,
            &base_url,
            &req.api_key,
            &model,
            &reduce_prompt,
            &token,
        )
        .await?
    };

    // Save `<name>.summary.md` next to media (or in `transcripts/` if requested)
    let target_dir = if req.transcripts_subfolder {
        req.output_dir.join("transcripts")
    } else {
        req.output_dir.clone()
    };
    if !target_dir.exists() {
        fs::create_dir_all(&target_dir).map_err(AppError::Io)?;
    }

    let free_stem = paths::collision_free_stem(&req.base_filename, |stem| {
        target_dir.join(format!("{stem}.summary.md")).exists()
    });
    let summary_path = target_dir.join(format!("{free_stem}.summary.md"));
    paths::atomic_write(&summary_path, summary_md.as_bytes())?;

    Ok(summary_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_request_debug_redacts_api_key() {
        let req = SummaryRequest {
            transcript_text: "some transcript".to_string(),
            language: None,
            model: None,
            api_key: "super-secret-test-key".to_string(),
            output_dir: PathBuf::from("/tmp"),
            base_filename: "video".to_string(),
            transcripts_subfolder: false,
            base_url: None,
        };

        let dbg = format!("{req:?}");
        assert!(
            !dbg.contains("super-secret-test-key"),
            "api_key leaked in Debug output: {dbg}"
        );
        assert!(
            dbg.contains("[REDACTED]"),
            "expected redaction marker: {dbg}"
        );
    }

    #[tokio::test]
    async fn generate_summary_fails_fast_when_api_key_is_blank() {
        let dir = tempfile::tempdir().expect("tempdir");
        let req = SummaryRequest {
            transcript_text: "Some transcript text to summarize.".to_string(),
            language: None,
            model: None,
            api_key: String::new(),
            output_dir: dir.path().to_path_buf(),
            base_filename: "video".to_string(),
            transcripts_subfolder: false,
            base_url: Some("http://127.0.0.1:1".to_string()),
        };

        let err = generate_summary(req, CancellationToken::new())
            .await
            .expect_err("blank api_key must fail before any HTTP request");

        assert!(
            matches!(err, AppError::GeminiKeyMissing),
            "expected GeminiKeyMissing, got {err:?}"
        );
        assert!(
            !dir.path().join("video.summary.md").exists(),
            "no summary file must be written when api_key is blank"
        );
    }

    #[test]
    fn split_text_into_chunks_respects_paragraph_boundaries() {
        let p1 = "First paragraph here with some text.";
        let p2 = "Second paragraph with more words and insights.";
        let p3 = "Third paragraph wrapping up the ideas.";
        let full = format!("{p1}\n\n{p2}\n\n{p3}");

        // Threshold large enough for single chunk
        let c1 = split_text_into_chunks(&full, 200);
        assert_eq!(c1.len(), 1);
        assert_eq!(c1[0], full);

        // Threshold forcing split
        let c2 = split_text_into_chunks(&full, 50);
        assert_eq!(c2.len(), 3);
        assert_eq!(c2[0], p1);
        assert_eq!(c2[1], p2);
        assert_eq!(c2[2], p3);
    }

    #[test]
    fn single_summary_prompt_includes_language_and_sections() {
        let prompt = build_single_summary_prompt("Sample transcript", "Italian");
        assert!(prompt.contains("in Italian"));
        assert!(prompt.contains("# Summary"));
        assert!(prompt.contains("# Key Points"));
        assert!(prompt.contains("Sample transcript"));
    }

    #[test]
    fn split_text_into_chunks_splits_whisper_single_newline_transcripts() {
        let line = "Segment of whisper transcription without double newlines.\n";
        let full = line.repeat(200); // 58 * 200 = 11,600 chars, zero "\n\n"
        let chunks = split_text_into_chunks(&full, 1_000);
        assert!(
            chunks.len() > 1,
            "must split into multiple chunks, got {}",
            chunks.len()
        );
        for chunk in &chunks {
            assert!(chunk.len() <= 1_000, "chunk len {} > 1000", chunk.len());
        }
    }

    #[test]
    fn split_text_into_chunks_splits_oversized_paragraph_on_sentences_and_words() {
        let sentence = "This is a sentence inside a very long single paragraph. ";
        let full = sentence.repeat(50); // ~2800 chars, no "\n\n" and no "\n"
        let chunks = split_text_into_chunks(&full, 500);
        assert!(
            chunks.len() > 1,
            "must split into multiple chunks, got {}",
            chunks.len()
        );
        for chunk in &chunks {
            assert!(chunk.len() <= 500, "chunk len {} > 500", chunk.len());
        }
    }

    #[test]
    fn summary_prompts_request_chapters_with_timestamps() {
        let timed_transcript =
            "1\n00:00:01,000 --> 00:00:04,000\nHello\n2\n00:00:04,000 --> 00:00:10,000\nWorld";
        let prompt = build_single_summary_prompt(timed_transcript, "English");
        assert!(
            prompt.contains("Chapters"),
            "single summary prompt must ask for Chapters section: {}",
            prompt
        );
        assert!(
            prompt.contains("timestamps"),
            "single summary prompt must mention timestamps: {}",
            prompt
        );

        let map_prompt = build_map_chunk_prompt(timed_transcript, 1, 2, "English");
        assert!(
            map_prompt.contains("timestamps"),
            "map prompt must mention preserving timestamps: {}",
            map_prompt
        );

        let reduce_prompt = build_reduce_prompt(
            &["Part 1 summary with milestone 00:00:01".to_string()],
            "English",
        );
        assert!(
            reduce_prompt.contains("Chapters"),
            "reduce prompt must ask for Chapters section: {}",
            reduce_prompt
        );
    }
}
