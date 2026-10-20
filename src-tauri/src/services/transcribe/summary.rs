//! Optional summary generation via Gemini REST API (spec §10.4).
//!
//! Input: transcript text (cheap, fast, no audio upload needed).
//! Output: `<name>.summary.md` with:
//! - Summary (concise overview)
//! - Key Points (bulleted takeaways)
//! - Chapters (if timestamps exist)
//!
//! For very long transcripts, uses a map-reduce strategy (summarize chunks, then synthesize).

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use reqwest::StatusCode;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::services::transcribe::gemini::{
    DEFAULT_GEMINI_BASE_URL, DEFAULT_GEMINI_MODEL, MAX_RETRIES,
};

/// Maximum chunk character length for single-shot summarization (~12,000 words).
pub const MAX_SUMMARY_CHUNK_CHARS: usize = 50_000;

/// Split transcript text into chunks around paragraph boundaries (`\n\n`).
pub fn split_text_into_chunks(text: &str, max_chunk_chars: usize) -> Vec<String> {
    if text.len() <= max_chunk_chars {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current_chunk = String::new();

    for paragraph in text.split("\n\n") {
        if current_chunk.is_empty() {
            current_chunk.push_str(paragraph);
        } else if current_chunk.len() + paragraph.len() + 2 <= max_chunk_chars {
            current_chunk.push_str("\n\n");
            current_chunk.push_str(paragraph);
        } else {
            chunks.push(current_chunk);
            current_chunk = paragraph.to_string();
        }
    }

    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }

    chunks
}

/// Build the prompt for single-request summarization.
pub fn build_single_summary_prompt(text: &str, language: &str) -> String {
    let lang_instruction = if language == "auto" || language.trim().is_empty() {
        "in the same language as the transcript".to_string()
    } else {
        format!("in {language}")
    };

    format!(
        "You are an expert summarizer. Summarize the following transcript {lang_instruction}.\n\
        Format your response cleanly in Markdown with the following sections:\n\
        # Summary\n\
        [A concise 2-3 paragraph overview of the content]\n\n\
        # Key Points\n\
        [Bullet points highlighting the essential arguments, takeaways and facts]\n\n\
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
        Extract the core ideas, discussion points and notable details concisely in bullet points:\n\n\
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

        let resp_res = client
            .post(&gen_url)
            .header("x-goog-api-key", api_key)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await;

        match resp_res {
            Ok(resp) => {
                let status = resp.status();
                if status == StatusCode::NOT_FOUND {
                    return Err(AppError::GeminiModelNotFound(model.to_string()));
                }
                if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(AppError::GeminiKeyInvalid(text));
                }
                if status == StatusCode::TOO_MANY_REQUESTS {
                    let text = resp.text().await.unwrap_or_default();
                    if attempt < MAX_RETRIES {
                        let delay = Duration::from_millis(500 * (1 << attempt));
                        tokio::select! {
                            _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
                            _ = tokio::time::sleep(delay) => continue,
                        }
                    }
                    return Err(AppError::GeminiQuotaExceeded(text));
                }
                if status.is_server_error() && attempt < MAX_RETRIES {
                    let delay = Duration::from_millis(500 * (1 << attempt));
                    tokio::select! {
                        _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
                        _ = tokio::time::sleep(delay) => continue,
                    }
                }
                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(AppError::Process(format!(
                        "Gemini summary request failed with HTTP {status}: {text}"
                    )));
                }

                let json: serde_json::Value = resp
                    .json()
                    .await
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
#[derive(Debug, Clone)]
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

/// Generate `<name>.summary.md` from the transcript text (spec §10.4).
pub async fn generate_summary(
    req: SummaryRequest,
    token: CancellationToken,
) -> Result<PathBuf, AppError> {
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

    let summary_path = target_dir.join(format!("{}.summary.md", req.base_filename));
    fs::write(&summary_path, summary_md.as_bytes()).map_err(AppError::Io)?;

    Ok(summary_path)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
