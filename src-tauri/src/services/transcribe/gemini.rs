//! Gemini Cloud Transcription REST client (spec §10.3).
//!
//! Direct REST integration with Google Generative Language API without SDK:
//! - Resumable audio upload streamed from disk (audio/ogg with opus, falling back to mp3/wav).
//! - Active polling with timeout (10 min) and cancellation.
//! - Content generation with verbatim prompts and blockReason/finishReason handling.
//! - RAII guard that deletes the remote cloud file on success, error, or cancel.
//! - Dynamic model discovery via GET /v1beta/models with -latest aliases as defaults.
//! - Backoff retry on 429 (rate/quota) and 5xx server errors.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::services::paths;
use crate::services::process::{self, ToolCommand};

pub const DEFAULT_GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-flash-latest";
pub const FALLBACK_GEMINI_MODEL: &str = "gemini-flash-lite-latest";

/// Maximum time to wait for a file to reach the ACTIVE state (spec §10.3: "10 min").
pub const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(600);
/// Polling interval for file status (spec §10.3: "original: every 5 s").
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Maximum retries on 429 or 5xx responses (spec §10.3: "max 3").
pub const MAX_RETRIES: usize = 3;

/// Callback sink for Gemini progress updates (`0.0` to `1.0` with stage label).
pub type GeminiProgressSink = Arc<dyn Fn(f64, &str) + Send + Sync>;

/// Gemini model information from `GET /v1beta/models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiModelInfo {
    pub id: String,
    pub display_name: String,
    pub description: String,
}

/// Information about an uploaded file returned by the Files API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadedFile {
    pub name: String,
    pub uri: String,
    pub state: String,
}

/// RAII cleanup guard that ensures the remote file on Gemini Files API and the
/// local temporary audio file are always deleted on success, error, or cancellation.
pub struct GeminiFileGuard {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    remote_file_name: Option<String>,
    temp_audio_file: Option<PathBuf>,
}

impl GeminiFileGuard {
    pub fn new(
        client: reqwest::Client,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            client,
            base_url: base_url.into(),
            api_key: api_key.into(),
            remote_file_name: None,
            temp_audio_file: None,
        }
    }

    pub fn set_remote_file(&mut self, name: impl Into<String>) {
        self.remote_file_name = Some(name.into());
    }

    pub fn set_temp_audio_file(&mut self, path: PathBuf) {
        self.temp_audio_file = Some(path);
    }

    /// Explicitly delete remote file and local temp file asynchronously.
    pub async fn cleanup(&mut self) {
        if let Some(file_name) = self.remote_file_name.take() {
            let base = self.base_url.trim_end_matches('/');
            let url = format!("{base}/v1beta/{file_name}");
            let _ = self
                .client
                .delete(&url)
                .header("x-goog-api-key", &self.api_key)
                .send()
                .await;
        }

        if let Some(path) = self.temp_audio_file.take() {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

impl Drop for GeminiFileGuard {
    fn drop(&mut self) {
        if self.remote_file_name.is_some() || self.temp_audio_file.is_some() {
            let client = self.client.clone();
            let base_url = self.base_url.clone();
            let api_key = self.api_key.clone();
            let remote_file = self.remote_file_name.take();
            let temp_file = self.temp_audio_file.take();

            tokio::spawn(async move {
                if let Some(file_name) = remote_file {
                    let base = base_url.trim_end_matches('/');
                    let url = format!("{base}/v1beta/{file_name}");
                    let _ = client
                        .delete(&url)
                        .header("x-goog-api-key", &api_key)
                        .send()
                        .await;
                }
                if let Some(path) = temp_file {
                    let _ = tokio::fs::remove_file(path).await;
                }
            });
        }
    }
}

/// Sleep for backoff with cancellation support.
async fn wait_backoff(attempt: usize, token: &CancellationToken) -> Result<(), AppError> {
    let delay_ms = 500u64 * (1u64 << attempt.min(5));
    tokio::select! {
        _ = token.cancelled() => Err(AppError::Process("Cancelled".to_string())),
        _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => Ok(()),
    }
}

/// Helper to build verbatim prompt (spec §10.3).
pub fn build_transcription_prompt(target_language: Option<&str>) -> String {
    match target_language {
        Some(lang) if !lang.trim().is_empty() && lang != "auto" && lang != "Original Language" => {
            format!("Transcribe the following audio file, and then translate the entire transcription into {lang}.")
        }
        _ => "Transcribe the following audio file into its original language. Include accurate punctuation.".to_string(),
    }
}

/// Prepare audio for Gemini upload via ffmpeg (spec §10.3).
///
/// Converts to 16 kHz mono Opus in an Ogg container (`audio/ogg`) at 24 kbps.
/// Falls back to MP3 (`audio/mpeg`) or WAV (`audio/wav`) if Opus encoding fails.
pub async fn prepare_audio_for_gemini(
    ffmpeg_path: &Path,
    input_media: &Path,
    temp_dir: &Path,
    token: CancellationToken,
) -> Result<(PathBuf, &'static str), AppError> {
    if !temp_dir.exists() {
        fs::create_dir_all(temp_dir).map_err(AppError::Io)?;
    }

    // 1. Try Opus in Ogg container
    let ogg_path = temp_dir.join("gemini_audio.ogg");
    if ogg_path.exists() {
        let _ = fs::remove_file(&ogg_path);
    }

    let mut cmd = ToolCommand::new(ffmpeg_path)
        .arg("-nostdin")
        .arg("-y")
        .arg("-i")
        .arg(input_media.as_os_str())
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-c:a")
        .arg("libopus")
        .arg("-b:a")
        .arg("24k")
        .arg("-f")
        .arg("ogg")
        .arg(ogg_path.as_os_str());

    let result = process::spawn_tool(cmd, token.clone(), process::discard_lines()).await?;
    if !result.cancelled && result.exit_code == Some(0) && ogg_path.is_file() {
        let len = fs::metadata(&ogg_path).map(|m| m.len()).unwrap_or(0);
        if len > 0 {
            return Ok((ogg_path, "audio/ogg"));
        }
    }

    if token.is_cancelled() {
        return Err(AppError::Process("Cancelled".to_string()));
    }

    // 2. Fallback to MP3
    let mp3_path = temp_dir.join("gemini_audio.mp3");
    if mp3_path.exists() {
        let _ = fs::remove_file(&mp3_path);
    }

    cmd = ToolCommand::new(ffmpeg_path)
        .arg("-nostdin")
        .arg("-y")
        .arg("-i")
        .arg(input_media.as_os_str())
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-c:a")
        .arg("libmp3lame")
        .arg("-b:a")
        .arg("32k")
        .arg(mp3_path.as_os_str());

    let result = process::spawn_tool(cmd, token.clone(), process::discard_lines()).await?;
    if !result.cancelled && result.exit_code == Some(0) && mp3_path.is_file() {
        let len = fs::metadata(&mp3_path).map(|m| m.len()).unwrap_or(0);
        if len > 0 {
            return Ok((mp3_path, "audio/mpeg"));
        }
    }

    if token.is_cancelled() {
        return Err(AppError::Process("Cancelled".to_string()));
    }

    // 3. Fallback to 16 kHz WAV
    let wav_path = temp_dir.join("gemini_audio.wav");
    if wav_path.exists() {
        let _ = fs::remove_file(&wav_path);
    }

    cmd = ToolCommand::new(ffmpeg_path)
        .arg("-nostdin")
        .arg("-y")
        .arg("-i")
        .arg(input_media.as_os_str())
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg(wav_path.as_os_str());

    let result = process::spawn_tool(cmd, token.clone(), process::discard_lines()).await?;
    if result.cancelled {
        return Err(AppError::Process("Cancelled".to_string()));
    }
    if result.exit_code != Some(0) || !wav_path.is_file() {
        return Err(AppError::Process(format!(
            "ffmpeg failed to extract audio for Gemini: exit code {:?}",
            result.exit_code
        )));
    }

    Ok((wav_path, "audio/wav"))
}

/// Upload audio file to Gemini Files API using the resumable upload protocol (spec §10.3).
///
/// Streams file bytes directly from disk without reading the entire file into RAM.
pub async fn upload_file_resumable(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    audio_path: &Path,
    mime_type: &str,
    token: &CancellationToken,
) -> Result<UploadedFile, AppError> {
    let file_metadata = fs::metadata(audio_path).map_err(AppError::Io)?;
    let file_len = file_metadata.len();
    let display_name = audio_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio.ogg".to_string());

    let base = base_url.trim_end_matches('/');
    let init_url = format!("{base}/upload/v1beta/files");

    // Step 1: Initiate resumable upload session with retry
    let mut upload_url: Option<String> = None;
    for attempt in 0..=MAX_RETRIES {
        if token.is_cancelled() {
            return Err(AppError::Process("Cancelled".to_string()));
        }

        let init_body = serde_json::json!({
            "file": {
                "display_name": display_name
            }
        });

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-goog-api-key",
            HeaderValue::from_str(api_key).map_err(|e| AppError::Process(e.to_string()))?,
        );
        headers.insert(
            "X-Goog-Upload-Protocol",
            HeaderValue::from_static("resumable"),
        );
        headers.insert("X-Goog-Upload-Command", HeaderValue::from_static("start"));
        headers.insert(
            "X-Goog-Upload-Header-Content-Length",
            HeaderValue::from_str(&file_len.to_string()).unwrap(),
        );
        headers.insert(
            "X-Goog-Upload-Header-Content-Type",
            HeaderValue::from_str(mime_type).unwrap(),
        );
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));

        let resp_res = client
            .post(&init_url)
            .headers(headers)
            .json(&init_body)
            .send()
            .await;

        match resp_res {
            Ok(resp) => {
                let status = resp.status();
                if status == StatusCode::UNAUTHORIZED
                    || status == StatusCode::FORBIDDEN
                    || status == StatusCode::BAD_REQUEST
                {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(AppError::GeminiKeyInvalid(text));
                }
                if (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                    && attempt < MAX_RETRIES
                {
                    wait_backoff(attempt, token).await?;
                    continue;
                }
                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(AppError::Process(format!(
                        "Gemini upload init failed with HTTP {status}: {text}"
                    )));
                }

                // Header is case-insensitive in reqwest
                if let Some(val) = resp.headers().get("x-goog-upload-url") {
                    if let Ok(url_str) = val.to_str() {
                        upload_url = Some(url_str.to_string());
                        break;
                    }
                }
                return Err(AppError::Process(
                    "Gemini upload initiation missing x-goog-upload-url header".into(),
                ));
            }
            Err(e) => {
                if attempt < MAX_RETRIES {
                    wait_backoff(attempt, token).await?;
                    continue;
                }
                return Err(AppError::Process(format!(
                    "Network error contacting Gemini: {e}"
                )));
            }
        }
    }

    let upload_url = upload_url
        .ok_or_else(|| AppError::Process("Failed to obtain Gemini resumable upload URL".into()))?;

    // Step 2: Stream bytes from disk to upload_url
    for attempt in 0..=MAX_RETRIES {
        if token.is_cancelled() {
            return Err(AppError::Process("Cancelled".to_string()));
        }

        let file = tokio::fs::File::open(audio_path)
            .await
            .map_err(AppError::Io)?;
        let stream = tokio_util::io::ReaderStream::new(file);
        let body = reqwest::Body::wrap_stream(stream);

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-goog-api-key",
            HeaderValue::from_str(api_key).map_err(|e| AppError::Process(e.to_string()))?,
        );
        headers.insert("X-Goog-Upload-Offset", HeaderValue::from_static("0"));
        headers.insert(
            "X-Goog-Upload-Command",
            HeaderValue::from_static("upload, finalize"),
        );
        headers.insert(
            "Content-Length",
            HeaderValue::from_str(&file_len.to_string()).unwrap(),
        );

        let resp_res = client
            .post(&upload_url)
            .headers(headers)
            .body(body)
            .send()
            .await;

        match resp_res {
            Ok(resp) => {
                let status = resp.status();
                if (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                    && attempt < MAX_RETRIES
                {
                    wait_backoff(attempt, token).await?;
                    continue;
                }
                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(AppError::Process(format!(
                        "Gemini file body upload failed with HTTP {status}: {text}"
                    )));
                }

                let json: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| AppError::Process(e.to_string()))?;
                let file_obj = json.get("file").ok_or_else(|| {
                    AppError::Process("Gemini upload response missing 'file' field".into())
                })?;
                let name = file_obj
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let uri = file_obj
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let state = file_obj
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if name.is_empty() || uri.is_empty() {
                    return Err(AppError::Process(
                        "Gemini file object missing name or uri".into(),
                    ));
                }

                return Ok(UploadedFile { name, uri, state });
            }
            Err(e) => {
                if attempt < MAX_RETRIES {
                    wait_backoff(attempt, token).await?;
                    continue;
                }
                return Err(AppError::Process(format!(
                    "Network error uploading audio to Gemini: {e}"
                )));
            }
        }
    }

    Err(AppError::Process(
        "Gemini audio upload exceeded max retries".into(),
    ))
}

/// Poll file status on Gemini Files API until state is ACTIVE, failed, or timed out (spec §10.3).
pub async fn poll_file_active(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    file_name: &str,
    poll_interval: Duration,
    timeout: Duration,
    token: &CancellationToken,
) -> Result<(), AppError> {
    let base = base_url.trim_end_matches('/');
    let status_url = format!("{base}/v1beta/{file_name}");
    let start_time = tokio::time::Instant::now();

    loop {
        if token.is_cancelled() {
            return Err(AppError::Process("Cancelled".to_string()));
        }
        if start_time.elapsed() >= timeout {
            return Err(AppError::Process(
                "Gemini file processing timed out after 10 minutes".to_string(),
            ));
        }

        let resp_res = client
            .get(&status_url)
            .header("x-goog-api-key", api_key)
            .send()
            .await;

        match resp_res {
            Ok(resp) => {
                let status = resp.status();
                if status == StatusCode::NOT_FOUND {
                    return Err(AppError::Process(format!(
                        "Gemini file '{file_name}' not found"
                    )));
                }
                if status.is_success() {
                    let json: serde_json::Value = resp
                        .json()
                        .await
                        .map_err(|e| AppError::Process(e.to_string()))?;
                    let state = json.get("state").and_then(|v| v.as_str()).unwrap_or("");
                    if state == "ACTIVE" {
                        return Ok(());
                    }
                    if state == "FAILED" {
                        let err_msg = json.get("error").map(|e| e.to_string()).unwrap_or_default();
                        return Err(AppError::Process(format!(
                            "Gemini cloud audio processing failed: {err_msg}"
                        )));
                    }
                }
            }
            Err(_) => {
                // Transient network jitter during polling is tolerated until timeout
            }
        }

        tokio::select! {
            _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
            _ = tokio::time::sleep(poll_interval) => {}
        }
    }
}

/// Call `models/<model>:generateContent` to transcribe the active cloud audio file (spec §10.3).
#[allow(clippy::too_many_arguments)]
pub async fn generate_transcription(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
    file_uri: &str,
    mime_type: &str,
    token: &CancellationToken,
) -> Result<String, AppError> {
    let base = base_url.trim_end_matches('/');
    let model_id = model.strip_prefix("models/").unwrap_or(model);
    let gen_url = format!("{base}/v1beta/models/{model_id}:generateContent");

    let payload = serde_json::json!({
        "contents": [
            {
                "parts": [
                    { "text": prompt },
                    {
                        "file_data": {
                            "mime_type": mime_type,
                            "file_uri": file_uri
                        }
                    }
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
                        wait_backoff(attempt, token).await?;
                        continue;
                    }
                    return Err(AppError::GeminiQuotaExceeded(text));
                }
                if status.is_server_error() && attempt < MAX_RETRIES {
                    wait_backoff(attempt, token).await?;
                    continue;
                }
                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(AppError::Process(format!(
                        "Gemini content generation failed with HTTP {status}: {text}"
                    )));
                }

                let json: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| AppError::Process(e.to_string()))?;

                // Check prompt feedback blockReason
                if let Some(block_reason) = json
                    .pointer("/promptFeedback/blockReason")
                    .and_then(|v| v.as_str())
                {
                    return Err(AppError::GeminiBlocked(block_reason.to_string()));
                }

                // Check candidates
                let candidates = json.get("candidates").and_then(|v| v.as_array());
                let Some(first_candidate) = candidates.and_then(|c| c.first()) else {
                    return Err(AppError::Process(
                        "Gemini returned empty candidates list".to_string(),
                    ));
                };

                // Check finish reason
                if let Some(finish_reason) =
                    first_candidate.get("finishReason").and_then(|v| v.as_str())
                {
                    if finish_reason == "MAX_TOKENS" {
                        eprintln!("Gemini transcription truncated at max tokens limit");
                    }
                }

                // Collect text parts
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
                    wait_backoff(attempt, token).await?;
                    continue;
                }
                return Err(AppError::Process(format!(
                    "Network error generating content with Gemini: {e}"
                )));
            }
        }
    }

    Err(AppError::Process(
        "Gemini generateContent exceeded max retries".into(),
    ))
}

/// Query list of available models from `GET /v1beta/models` (spec §10.3).
pub async fn list_gemini_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<GeminiModelInfo>, AppError> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/v1beta/models");

    let resp = client
        .get(&url)
        .header("x-goog-api-key", api_key)
        .send()
        .await
        .map_err(|e| AppError::Process(format!("Network error fetching Gemini models: {e}")))?;

    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::GeminiKeyInvalid(text));
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Process(format!(
            "Failed to list Gemini models: HTTP {status}: {text}"
        )));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Process(e.to_string()))?;
    let mut out_models: Vec<GeminiModelInfo> = Vec::new();

    if let Some(models) = json.get("models").and_then(|v| v.as_array()) {
        for m in models {
            let methods = m
                .get("supportedGenerationMethods")
                .and_then(|v| v.as_array());
            let supports_generate = methods.is_some_and(|arr| {
                arr.iter()
                    .any(|method| method.as_str() == Some("generateContent"))
            });

            if supports_generate {
                let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let id = name.strip_prefix("models/").unwrap_or(name).to_string();
                let display_name = m
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string();
                let description = m
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if !id.is_empty() {
                    out_models.push(GeminiModelInfo {
                        id,
                        display_name,
                        description,
                    });
                }
            }
        }
    }

    // Always ensure default -latest aliases are present
    let has_flash_latest = out_models.iter().any(|m| m.id == DEFAULT_GEMINI_MODEL);
    if !has_flash_latest {
        out_models.insert(
            0,
            GeminiModelInfo {
                id: DEFAULT_GEMINI_MODEL.to_string(),
                display_name: "Gemini Flash (Latest)".to_string(),
                description: "Recommended general-purpose fast model alias".to_string(),
            },
        );
    }

    let has_flash_lite_latest = out_models.iter().any(|m| m.id == FALLBACK_GEMINI_MODEL);
    if !has_flash_lite_latest {
        out_models.insert(
            1,
            GeminiModelInfo {
                id: FALLBACK_GEMINI_MODEL.to_string(),
                display_name: "Gemini Flash Lite (Latest)".to_string(),
                description: "Lightweight fast model alias".to_string(),
            },
        );
    }

    // Sort so that -latest aliases appear at the top of the list (spec §10.3)
    out_models.sort_by(|a, b| {
        let a_latest = a.id.ends_with("-latest");
        let b_latest = b.id.ends_with("-latest");
        b_latest.cmp(&a_latest)
    });

    Ok(out_models)
}

/// High-level transcription request structure.
#[derive(Debug, Clone)]
pub struct GeminiTranscribeRequest {
    pub ffmpeg_path: PathBuf,
    pub input_media: PathBuf,
    pub temp_dir: PathBuf,
    pub output_dir: PathBuf,
    pub base_filename: String,
    pub transcripts_subfolder: bool,
    pub api_key: String,
    pub model: String,
    pub target_language: Option<String>,
    pub base_url: Option<String>,
    pub poll_interval: Option<Duration>,
}

/// Run full Gemini transcription pipeline (spec §10.3).
///
/// Output: produces `<name>.txt` only (spec §10.3: "Output: .txt only. SRT/VTT come only from Whisper or subtitles").
pub async fn run_gemini_transcription(
    req: GeminiTranscribeRequest,
    token: CancellationToken,
    progress_sink: Option<GeminiProgressSink>,
) -> Result<PathBuf, AppError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| AppError::Process(e.to_string()))?;

    let base_url = req
        .base_url
        .unwrap_or_else(|| DEFAULT_GEMINI_BASE_URL.to_string());
    let mut guard = GeminiFileGuard::new(client.clone(), &base_url, &req.api_key);

    if let Some(sink) = &progress_sink {
        sink(0.05, "transcribing");
    }

    // 1. Prepare audio
    let (audio_file, mime_type) = prepare_audio_for_gemini(
        &req.ffmpeg_path,
        &req.input_media,
        &req.temp_dir,
        token.clone(),
    )
    .await?;
    guard.set_temp_audio_file(audio_file.clone());

    if let Some(sink) = &progress_sink {
        sink(0.20, "transcribing");
    }

    // 2. Resumable upload
    let uploaded = upload_file_resumable(
        &client,
        &base_url,
        &req.api_key,
        &audio_file,
        mime_type,
        &token,
    )
    .await?;
    guard.set_remote_file(uploaded.name.clone());

    if let Some(sink) = &progress_sink {
        sink(0.50, "transcribing");
    }

    // 3. Poll until ACTIVE
    let poll_interval = req.poll_interval.unwrap_or(DEFAULT_POLL_INTERVAL);
    poll_file_active(
        &client,
        &base_url,
        &req.api_key,
        &uploaded.name,
        poll_interval,
        DEFAULT_POLL_TIMEOUT,
        &token,
    )
    .await?;

    if let Some(sink) = &progress_sink {
        sink(0.75, "transcribing");
    }

    // 4. Generate content
    let prompt = build_transcription_prompt(req.target_language.as_deref());
    let transcript = generate_transcription(
        &client,
        &base_url,
        &req.api_key,
        &req.model,
        &prompt,
        &uploaded.uri,
        mime_type,
        &token,
    )
    .await?;

    // 5. Clean up remote and temp files via RAII guard
    guard.cleanup().await;

    // 6. Save transcript .txt
    let target_dir = if req.transcripts_subfolder {
        req.output_dir.join("transcripts")
    } else {
        req.output_dir.clone()
    };
    if !target_dir.exists() {
        fs::create_dir_all(&target_dir).map_err(AppError::Io)?;
    }

    let free_stem = paths::collision_free_stem(&req.base_filename, |stem| {
        target_dir.join(format!("{stem}.txt")).exists()
    });
    let txt_path = target_dir.join(format!("{free_stem}.txt"));
    paths::atomic_write(&txt_path, transcript.as_bytes())?;

    if let Some(sink) = &progress_sink {
        sink(1.0, "transcribing");
    }

    Ok(txt_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_transcription_prompts() {
        let orig = build_transcription_prompt(None);
        assert_eq!(
            orig,
            "Transcribe the following audio file into its original language. Include accurate punctuation."
        );

        let orig_explicit = build_transcription_prompt(Some("Original Language"));
        assert_eq!(
            orig_explicit,
            "Transcribe the following audio file into its original language. Include accurate punctuation."
        );

        let target = build_transcription_prompt(Some("Italian"));
        assert_eq!(
            target,
            "Transcribe the following audio file, and then translate the entire transcription into Italian."
        );

        let target_en = build_transcription_prompt(Some("English"));
        assert_eq!(
            target_en,
            "Transcribe the following audio file, and then translate the entire transcription into English."
        );
    }
}
