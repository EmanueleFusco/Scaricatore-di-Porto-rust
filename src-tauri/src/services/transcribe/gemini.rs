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
        if let Some(path) = self.temp_audio_file.take() {
            let _ = tokio::fs::remove_file(path).await;
        }

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
    }
}

impl Drop for GeminiFileGuard {
    fn drop(&mut self) {
        if let Some(path) = self.temp_audio_file.take() {
            let _ = std::fs::remove_file(path);
        }

        if let Some(file_name) = self.remote_file_name.take() {
            let client = self.client.clone();
            let base_url = self.base_url.clone();
            let api_key = self.api_key.clone();

            tokio::spawn(async move {
                let base = base_url.trim_end_matches('/');
                let url = format!("{base}/v1beta/{file_name}");
                let _ = client
                    .delete(&url)
                    .header("x-goog-api-key", &api_key)
                    .send()
                    .await;
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

/// Check if an HTTP 400 response body indicates an invalid or malformed Gemini API key.
pub fn is_invalid_api_key_body(body: &str) -> bool {
    let upper = body.to_ascii_uppercase();
    upper.contains("API_KEY_INVALID")
        || upper.contains("API KEY NOT VALID")
        || (upper.contains("INVALID_ARGUMENT") && upper.contains("API KEY"))
}

/// Centralised classification of non-success Gemini HTTP status codes (spec §10.3 item 7).
pub fn classify_gemini_http_error(
    status: StatusCode,
    body: &str,
    model: Option<&str>,
    context: &str,
) -> AppError {
    if status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || (status == StatusCode::BAD_REQUEST && is_invalid_api_key_body(body))
    {
        return AppError::GeminiKeyInvalid(body.to_string());
    }
    if status == StatusCode::NOT_FOUND {
        if let Some(m) = model {
            return AppError::GeminiModelNotFound(m.to_string());
        }
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return AppError::GeminiQuotaExceeded(body.to_string());
    }
    AppError::Process(format!("{context} failed with HTTP {status}: {body}"))
}

/// Map an ISO language code (e.g. "it", "en", "es", "fr", "de") or regional tag ("it-IT")
/// to its English language name for Gemini prompts (spec §10.3).
pub fn map_language_name(code_or_name: &str) -> &str {
    let trimmed = code_or_name.trim();
    let primary = trimmed
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    match primary.as_str() {
        "it" | "ita" | "italian" => "Italian",
        "en" | "eng" | "english" => "English",
        "es" | "spa" | "spanish" => "Spanish",
        "fr" | "fra" | "fre" | "french" => "French",
        "de" | "deu" | "ger" | "german" => "German",
        "pt" | "por" | "portuguese" => "Portuguese",
        "nl" | "nld" | "dut" | "dutch" => "Dutch",
        "ru" | "rus" | "russian" => "Russian",
        "ja" | "jpn" | "japanese" => "Japanese",
        "zh" | "zho" | "chi" | "chinese" => "Chinese",
        "ko" | "kor" | "korean" => "Korean",
        "ar" | "ara" | "arabic" => "Arabic",
        "hi" | "hin" | "hindi" => "Hindi",
        "pl" | "pol" | "polish" => "Polish",
        "uk" | "ukr" | "ukrainian" => "Ukrainian",
        "tr" | "tur" | "turkish" => "Turkish",
        "sv" | "swe" | "swedish" => "Swedish",
        "da" | "dan" | "danish" => "Danish",
        "no" | "nor" | "norwegian" => "Norwegian",
        "fi" | "fin" | "finnish" => "Finnish",
        "el" | "ell" | "gre" | "greek" => "Greek",
        "cs" | "ces" | "cze" | "czech" => "Czech",
        "ro" | "ron" | "rum" | "romanian" => "Romanian",
        "hu" | "hun" | "hungarian" => "Hungarian",
        _ => trimmed,
    }
}

/// Resolve the target language for Gemini transcription prompt (spec §10.3).
///
/// If `translate_to_english` is true, the target language is `"English"`.
/// If translation was not requested, returns `None` so the original-language
/// prompt is used (even when `transcript_language` contains the audio's detected
/// or selected language code).
pub fn resolve_gemini_target_language(
    translate_to_english: bool,
    explicit_target_language: Option<&str>,
) -> Option<String> {
    if translate_to_english {
        Some("English".to_string())
    } else {
        explicit_target_language
            .filter(|l| {
                let trimmed = l.trim();
                !trimmed.is_empty()
                    && !trimmed.eq_ignore_ascii_case("auto")
                    && !trimmed.eq_ignore_ascii_case("original")
                    && !trimmed.eq_ignore_ascii_case("original language")
            })
            .map(|l| map_language_name(l).to_string())
    }
}

/// Helper to build verbatim prompt (spec §10.3).
pub fn build_transcription_prompt(target_language: Option<&str>) -> String {
    match target_language {
        Some(lang) => {
            let trimmed = lang.trim();
            if trimmed.is_empty()
                || trimmed.eq_ignore_ascii_case("auto")
                || trimmed.eq_ignore_ascii_case("original")
                || trimmed.eq_ignore_ascii_case("original language")
            {
                "Transcribe the following audio file into its original language. Include accurate punctuation.".to_string()
            } else {
                let mapped = map_language_name(trimmed);
                format!("Transcribe the following audio file, and then translate the entire transcription into {mapped}.")
            }
        }
        None => "Transcribe the following audio file into its original language. Include accurate punctuation.".to_string(),
    }
}

pub fn build_audio_prep_command(
    ffmpeg_path: &Path,
    input_media: &Path,
    output: &Path,
    codec: &str,
    bitrate: Option<&str>,
    format: Option<&str>,
    temp_dir: &Path,
) -> ToolCommand {
    let mut cmd = ToolCommand::new(ffmpeg_path)
        .temp_dir(temp_dir)
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
        .arg(codec);
    if let Some(br) = bitrate {
        cmd = cmd.arg("-b:a").arg(br);
    }
    if let Some(fmt) = format {
        cmd = cmd.arg("-f").arg(fmt);
    }
    cmd.arg(output.as_os_str())
}

/// Estimate the free disk space needed for Gemini audio extraction in bytes.
///
/// Uses worst-case fallback (16 kHz mono 16-bit uncompressed WAV / FLAC) = 32,000 bytes/sec (~115 MB/hour)
/// with a 25% safety margin, minimum 50 MB, or 200 MB default headroom when duration is unknown (spec §10.3).
pub fn estimate_audio_prep_needed_bytes(duration_sec: Option<f64>) -> u64 {
    if let Some(dur) = duration_sec {
        ((dur * 32_000.0 * 1.25) as u64).max(50 * 1024 * 1024)
    } else {
        200 * 1024 * 1024
    }
}

/// Maximum audio duration supported by Gemini Files API per prompt (9.5 hours = 34,200s, spec §10.3 item 1).
pub const GEMINI_MAX_AUDIO_DURATION_SECS: f64 = 9.5 * 3600.0;

/// Threshold above which Gemini single-turn transcription may hit model token limits (~60 minutes, spec §10.3 item 4).
pub const GEMINI_LONG_AUDIO_THRESHOLD_SECS: f64 = 60.0 * 60.0;

/// Outcome of generating text with Gemini (spec §10.3 item 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeminiGenerateOutcome {
    pub text: String,
    pub truncated: bool,
    pub finish_reason: Option<String>,
}

/// Overall outcome of running Gemini transcription (spec §10.3).
#[derive(Debug, Clone, PartialEq)]
pub struct GeminiTranscribeOutcome {
    pub txt_path: PathBuf,
    pub truncated: bool,
    pub finish_reason: Option<String>,
    pub duration_sec: Option<f64>,
    pub duration_warning: bool,
}

impl std::ops::Deref for GeminiTranscribeOutcome {
    type Target = PathBuf;
    fn deref(&self) -> &Self::Target {
        &self.txt_path
    }
}

impl AsRef<Path> for GeminiTranscribeOutcome {
    fn as_ref(&self) -> &Path {
        &self.txt_path
    }
}

impl PartialEq<PathBuf> for GeminiTranscribeOutcome {
    fn eq(&self, other: &PathBuf) -> bool {
        &self.txt_path == other
    }
}

impl PartialEq<GeminiTranscribeOutcome> for PathBuf {
    fn eq(&self, other: &GeminiTranscribeOutcome) -> bool {
        self == &other.txt_path
    }
}

impl PartialEq<Path> for GeminiTranscribeOutcome {
    fn eq(&self, other: &Path) -> bool {
        self.txt_path.as_path() == other
    }
}

impl PartialEq<GeminiTranscribeOutcome> for Path {
    fn eq(&self, other: &GeminiTranscribeOutcome) -> bool {
        self == other.txt_path.as_path()
    }
}

/// Verify that `temp_dir` has enough free disk space for audio preparation (spec §10.3 item 1).
pub fn check_audio_prep_disk_space(temp_dir: &Path, needed: u64) -> Result<(), AppError> {
    let free_space = crate::services::paths::free_disk_space(temp_dir).unwrap_or(0);
    if free_space > 0 && free_space < needed {
        return Err(AppError::DiskFull {
            needed,
            available: free_space,
        });
    }
    Ok(())
}

/// Prepare audio for Gemini upload via ffmpeg (spec §10.3).
///
/// Converts to 16 kHz mono Opus in an Ogg container (`audio/ogg`) at 24 kbps.
/// Falls back to MP3 (`audio/mpeg`), FLAC (`audio/flac`), or WAV (`audio/wav`) if Opus encoding fails.
pub async fn prepare_audio_for_gemini(
    ffmpeg_path: &Path,
    input_media: &Path,
    temp_dir: &Path,
    token: CancellationToken,
) -> Result<(PathBuf, &'static str), AppError> {
    if !temp_dir.exists() {
        fs::create_dir_all(temp_dir).map_err(AppError::Io)?;
    }

    // Check free disk space in temp_dir before running ffmpeg (spec §10.3 item 1).
    let duration_sec =
        if let Some(ffprobe) = crate::services::ffmpeg::find_ffprobe(ffmpeg_path, None) {
            crate::services::ffmpeg::probe_file_duration(
                &ffprobe,
                input_media,
                Some(temp_dir),
                token.clone(),
            )
            .await
        } else {
            None
        };
    let needed = estimate_audio_prep_needed_bytes(duration_sec);
    check_audio_prep_disk_space(temp_dir, needed)?;

    // Reject audio exceeding Gemini's 9.5-hour limit (spec §10.3 item 1).
    if let Some(dur) = duration_sec {
        if dur > GEMINI_MAX_AUDIO_DURATION_SECS {
            return Err(AppError::Process(format!(
                "Audio duration ({:.1} hours) exceeds Gemini's 9.5-hour limit",
                dur / 3600.0
            )));
        }
    }

    // 1. Try Opus in Ogg container
    let ogg_path = temp_dir.join("gemini_audio.ogg");
    let _ = fs::remove_file(&ogg_path);

    let cmd = build_audio_prep_command(
        ffmpeg_path,
        input_media,
        &ogg_path,
        "libopus",
        Some("24k"),
        Some("ogg"),
        temp_dir,
    );

    let ogg_res = process::spawn_tool(cmd, token.clone(), process::discard_lines()).await;
    match ogg_res {
        Ok(result) if !result.cancelled && result.exit_code == Some(0) && ogg_path.is_file() => {
            let len = fs::metadata(&ogg_path).map(|m| m.len()).unwrap_or(0);
            if len > 0 {
                return Ok((ogg_path, "audio/ogg"));
            }
        }
        _ => {}
    }
    let _ = fs::remove_file(&ogg_path);

    if token.is_cancelled() {
        return Err(AppError::Process("Cancelled".to_string()));
    }

    // 2. Fallback to MP3
    let mp3_path = temp_dir.join("gemini_audio.mp3");
    let _ = fs::remove_file(&mp3_path);

    let cmd = build_audio_prep_command(
        ffmpeg_path,
        input_media,
        &mp3_path,
        "libmp3lame",
        Some("32k"),
        None,
        temp_dir,
    );

    let mp3_res = process::spawn_tool(cmd, token.clone(), process::discard_lines()).await;
    match mp3_res {
        Ok(result) if !result.cancelled && result.exit_code == Some(0) && mp3_path.is_file() => {
            let len = fs::metadata(&mp3_path).map(|m| m.len()).unwrap_or(0);
            if len > 0 {
                return Ok((mp3_path, "audio/mpeg"));
            }
        }
        _ => {}
    }
    let _ = fs::remove_file(&mp3_path);

    if token.is_cancelled() {
        return Err(AppError::Process("Cancelled".to_string()));
    }

    // 3. Fallback to FLAC (spec §10.3 item 1)
    let flac_path = temp_dir.join("gemini_audio.flac");
    let _ = fs::remove_file(&flac_path);

    let cmd = build_audio_prep_command(
        ffmpeg_path,
        input_media,
        &flac_path,
        "flac",
        None,
        None,
        temp_dir,
    );

    let flac_res = process::spawn_tool(cmd, token.clone(), process::discard_lines()).await;
    match flac_res {
        Ok(result) if !result.cancelled && result.exit_code == Some(0) && flac_path.is_file() => {
            let len = fs::metadata(&flac_path).map(|m| m.len()).unwrap_or(0);
            if len > 0 {
                return Ok((flac_path, "audio/flac"));
            }
        }
        _ => {}
    }
    let _ = fs::remove_file(&flac_path);

    if token.is_cancelled() {
        return Err(AppError::Process("Cancelled".to_string()));
    }

    // 4. Fallback to 16 kHz WAV
    let wav_path = temp_dir.join("gemini_audio.wav");
    let _ = fs::remove_file(&wav_path);

    let cmd = build_audio_prep_command(
        ffmpeg_path,
        input_media,
        &wav_path,
        "pcm_s16le",
        None,
        None,
        temp_dir,
    );

    let wav_res = process::spawn_tool(cmd, token.clone(), process::discard_lines()).await;
    match wav_res {
        Ok(result) if !result.cancelled && result.exit_code == Some(0) && wav_path.is_file() => {
            let len = fs::metadata(&wav_path).map(|m| m.len()).unwrap_or(0);
            if len > 0 {
                return Ok((wav_path, "audio/wav"));
            }
            let _ = fs::remove_file(&wav_path);
            Err(AppError::Process(
                "ffmpeg produced empty audio file".to_string(),
            ))
        }
        Ok(result) if result.cancelled => {
            let _ = fs::remove_file(&wav_path);
            Err(AppError::Process("Cancelled".to_string()))
        }
        Ok(result) => {
            let _ = fs::remove_file(&wav_path);
            Err(AppError::Process(format!(
                "ffmpeg failed to extract audio for Gemini: exit code {:?}",
                result.exit_code
            )))
        }
        Err(err) => {
            let _ = fs::remove_file(&wav_path);
            Err(err)
        }
    }
}

/// Build the `x-goog-api-key` header value, marked sensitive so `reqwest` debug
/// logging redacts it (spec §10.3, §10.5).
fn gemini_api_key_header(api_key: &str) -> Result<HeaderValue, AppError> {
    let mut value = HeaderValue::from_str(api_key).map_err(|e| AppError::Process(e.to_string()))?;
    value.set_sensitive(true);
    Ok(value)
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
        headers.insert("x-goog-api-key", gemini_api_key_header(api_key)?);
        headers.insert(
            "X-Goog-Upload-Protocol",
            HeaderValue::from_static("resumable"),
        );
        headers.insert("X-Goog-Upload-Command", HeaderValue::from_static("start"));
        headers.insert(
            "X-Goog-Upload-Header-Content-Length",
            HeaderValue::from(file_len),
        );
        headers.insert(
            "X-Goog-Upload-Header-Content-Type",
            HeaderValue::from_str(mime_type)
                .map_err(|e| AppError::Process(format!("Invalid MIME header: {e}")))?,
        );
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));

        let resp_res = tokio::select! {
            biased;
            _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
            res = client
                .post(&init_url)
                .headers(headers)
                .json(&init_body)
                .send() => res,
        };

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
                    let text = tokio::select! {
                        biased;
                        _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
                        res = resp.text() => res.unwrap_or_default(),
                    };
                    if status == StatusCode::BAD_REQUEST && text.trim().is_empty() {
                        return Err(AppError::GeminiKeyInvalid(text));
                    }
                    return Err(classify_gemini_http_error(
                        status,
                        &text,
                        None,
                        "Gemini upload init",
                    ));
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
        headers.insert("x-goog-api-key", gemini_api_key_header(api_key)?);
        headers.insert("X-Goog-Upload-Offset", HeaderValue::from_static("0"));
        headers.insert(
            "X-Goog-Upload-Command",
            HeaderValue::from_static("upload, finalize"),
        );
        headers.insert("Content-Length", HeaderValue::from(file_len));

        let resp_res = tokio::select! {
            biased;
            _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
            res = client
                .post(&upload_url)
                .headers(headers)
                .body(body)
                .send() => res,
        };

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
                    let text = tokio::select! {
                        biased;
                        _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
                        res = resp.text() => res.unwrap_or_default(),
                    };
                    return Err(classify_gemini_http_error(
                        status,
                        &text,
                        None,
                        "Gemini file body upload",
                    ));
                }

                let json: serde_json::Value = tokio::select! {
                    biased;
                    _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
                    res = resp.json() => res,
                }
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
    let mut transient_failures = 0usize;

    loop {
        if token.is_cancelled() {
            return Err(AppError::Process("Cancelled".to_string()));
        }
        if start_time.elapsed() >= timeout {
            return Err(AppError::Process(
                "Gemini file processing timed out after 10 minutes".to_string(),
            ));
        }

        let resp_res = tokio::select! {
            biased;
            _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
            res = client
                .get(&status_url)
                .header("x-goog-api-key", api_key)
                .send() => res,
        };

        match resp_res {
            Ok(resp) => {
                let status = resp.status();
                if status == StatusCode::NOT_FOUND {
                    return Err(AppError::Process(format!(
                        "Gemini file '{file_name}' not found"
                    )));
                }
                if !status.is_success() {
                    let text = tokio::select! {
                        biased;
                        _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
                        res = resp.text() => res.unwrap_or_default(),
                    };
                    if (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                        && transient_failures < MAX_RETRIES
                    {
                        wait_backoff(transient_failures, token).await?;
                        transient_failures += 1;
                        continue;
                    }
                    return Err(classify_gemini_http_error(
                        status,
                        &text,
                        None,
                        "Gemini file status polling",
                    ));
                }
                transient_failures = 0;
                let json: serde_json::Value = tokio::select! {
                    biased;
                    _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
                    res = resp.json() => res,
                }
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
            Err(e) => {
                if transient_failures < MAX_RETRIES {
                    wait_backoff(transient_failures, token).await?;
                    transient_failures += 1;
                    continue;
                }
                return Err(AppError::Process(format!(
                    "Network error polling Gemini file status: {e}"
                )));
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
) -> Result<GeminiGenerateOutcome, AppError> {
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
                    wait_backoff(attempt, token).await?;
                    continue;
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
                        "Gemini content generation",
                    ));
                }

                let json: serde_json::Value = tokio::select! {
                    biased;
                    _ = token.cancelled() => return Err(AppError::Process("Cancelled".to_string())),
                    res = resp.json() => res,
                }
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
                let finish_reason = first_candidate
                    .get("finishReason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let truncated = finish_reason
                    .as_deref()
                    .map(|fr| fr != "STOP")
                    .unwrap_or(false);

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

                if finish_reason.as_deref() == Some("SAFETY") && out_text.is_empty() {
                    return Err(AppError::GeminiBlocked("SAFETY".to_string()));
                }

                return Ok(GeminiGenerateOutcome {
                    text: out_text,
                    truncated,
                    finish_reason,
                });
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
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(classify_gemini_http_error(
            status,
            &text,
            None,
            "Failed to list Gemini models",
        ));
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
///
/// Custom `Debug` redacts `api_key` so request structs can be logged safely (spec §10.5).
#[derive(Clone)]
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
    pub duration_sec: Option<f64>,
}

impl std::fmt::Debug for GeminiTranscribeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeminiTranscribeRequest")
            .field("ffmpeg_path", &self.ffmpeg_path)
            .field("input_media", &self.input_media)
            .field("temp_dir", &self.temp_dir)
            .field("output_dir", &self.output_dir)
            .field("base_filename", &self.base_filename)
            .field("transcripts_subfolder", &self.transcripts_subfolder)
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("target_language", &self.target_language)
            .field("base_url", &self.base_url)
            .field("poll_interval", &self.poll_interval)
            .field("duration_sec", &self.duration_sec)
            .finish()
    }
}

/// Run full Gemini transcription pipeline (spec §10.3).
///
/// Output: produces `<name>.txt` only (spec §10.3: "Output: .txt only. SRT/VTT come only from Whisper or subtitles").
pub async fn run_gemini_transcription(
    req: GeminiTranscribeRequest,
    token: CancellationToken,
    progress_sink: Option<GeminiProgressSink>,
) -> Result<GeminiTranscribeOutcome, AppError> {
    // Missing key must fail before extracting audio or uploading (spec §10.3 item 7).
    if req.api_key.trim().is_empty() {
        return Err(AppError::GeminiKeyMissing);
    }

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

    // 0. Probe duration and check limits (spec §10.3 items 1 & 4)
    let duration_sec = if let Some(d) = req.duration_sec {
        Some(d)
    } else if let Some(ffprobe) = crate::services::ffmpeg::find_ffprobe(&req.ffmpeg_path, None) {
        crate::services::ffmpeg::probe_file_duration(
            &ffprobe,
            &req.input_media,
            Some(&req.temp_dir),
            token.clone(),
        )
        .await
    } else {
        None
    };

    if let Some(dur) = duration_sec {
        if dur > GEMINI_MAX_AUDIO_DURATION_SECS {
            return Err(AppError::Process(format!(
                "Audio duration ({:.1} hours) exceeds Gemini's 9.5-hour limit",
                dur / 3600.0
            )));
        }
    }

    let duration_warning = duration_sec
        .map(|dur| dur >= GEMINI_LONG_AUDIO_THRESHOLD_SECS)
        .unwrap_or(false);

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
    let gen_res = generate_transcription(
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
    paths::atomic_write(&txt_path, gen_res.text.as_bytes())?;

    if let Some(sink) = &progress_sink {
        sink(1.0, "transcribing");
    }

    Ok(GeminiTranscribeOutcome {
        txt_path,
        truncated: gen_res.truncated,
        finish_reason: gen_res.finish_reason,
        duration_sec,
        duration_warning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_transcribe_request_debug_redacts_api_key() {
        let req = GeminiTranscribeRequest {
            ffmpeg_path: PathBuf::from("/tmp/ffmpeg"),
            input_media: PathBuf::from("/tmp/input.mkv"),
            temp_dir: PathBuf::from("/tmp"),
            output_dir: PathBuf::from("/tmp"),
            base_filename: "video".to_string(),
            transcripts_subfolder: false,
            api_key: "super-secret-test-key".to_string(),
            model: DEFAULT_GEMINI_MODEL.to_string(),
            target_language: None,
            base_url: None,
            poll_interval: None,
            duration_sec: None,
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

    #[test]
    fn gemini_api_key_header_is_sensitive() {
        let header = gemini_api_key_header("super-secret-test-key").expect("valid header value");
        assert!(
            header.is_sensitive(),
            "x-goog-api-key header must be marked sensitive"
        );
        assert_eq!(
            header.to_str().expect("visible value"),
            "super-secret-test-key"
        );
    }

    #[tokio::test]
    async fn run_gemini_transcription_fails_fast_when_api_key_is_blank() {
        let dir = tempfile::tempdir().expect("tempdir");
        let req = GeminiTranscribeRequest {
            ffmpeg_path: dir.path().join("missing-ffmpeg"),
            input_media: dir.path().join("missing-input.mkv"),
            temp_dir: dir.path().to_path_buf(),
            output_dir: dir.path().to_path_buf(),
            base_filename: "video".to_string(),
            transcripts_subfolder: false,
            api_key: "   ".to_string(),
            model: DEFAULT_GEMINI_MODEL.to_string(),
            target_language: None,
            base_url: Some("http://127.0.0.1:1".to_string()),
            poll_interval: Some(Duration::from_millis(1)),
            duration_sec: Some(60.0),
        };

        let err = run_gemini_transcription(req, CancellationToken::new(), None)
            .await
            .expect_err("blank api_key must fail before audio preparation");

        assert!(
            matches!(err, AppError::GeminiKeyMissing),
            "expected GeminiKeyMissing, got {err:?}"
        );
        assert!(
            !dir.path().join("gemini_audio.ogg").exists()
                && !dir.path().join("gemini_audio.mp3").exists()
                && !dir.path().join("gemini_audio.flac").exists()
                && !dir.path().join("gemini_audio.wav").exists(),
            "no audio file must be prepared when api_key is blank"
        );
    }

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

    #[test]
    fn gemini_m5_prompt_and_language_mapping() {
        // ISO codes and regional tags must map to English language names in translation prompt
        assert_eq!(
            build_transcription_prompt(Some("it")),
            "Transcribe the following audio file, and then translate the entire transcription into Italian."
        );
        assert_eq!(
            build_transcription_prompt(Some("it-IT")),
            "Transcribe the following audio file, and then translate the entire transcription into Italian."
        );
        assert_eq!(
            build_transcription_prompt(Some("en")),
            "Transcribe the following audio file, and then translate the entire transcription into English."
        );
        assert_eq!(
            build_transcription_prompt(Some("es")),
            "Transcribe the following audio file, and then translate the entire transcription into Spanish."
        );
        assert_eq!(
            build_transcription_prompt(Some("fr")),
            "Transcribe the following audio file, and then translate the entire transcription into French."
        );
        assert_eq!(
            build_transcription_prompt(Some("de")),
            "Transcribe the following audio file, and then translate the entire transcription into German."
        );

        // Original language / auto strings must use original-language prompt
        assert_eq!(
            build_transcription_prompt(None),
            "Transcribe the following audio file into its original language. Include accurate punctuation."
        );
        assert_eq!(
            build_transcription_prompt(Some("auto")),
            "Transcribe the following audio file into its original language. Include accurate punctuation."
        );
        assert_eq!(
            build_transcription_prompt(Some("Original Language")),
            "Transcribe the following audio file into its original language. Include accurate punctuation."
        );

        // Target language resolution:
        // When translate_to_english is false / not set:
        // Must use original-language prompt (target_language: None), even if transcript_language was auto-populated with "it"
        let no_translate = resolve_gemini_target_language(false, None);
        assert_eq!(no_translate, None);
        assert_eq!(
            build_transcription_prompt(no_translate.as_deref()),
            "Transcribe the following audio file into its original language. Include accurate punctuation."
        );

        // When translate_to_english is true:
        // Must translate to English
        let translate_en = resolve_gemini_target_language(true, None);
        assert_eq!(translate_en, Some("English".to_string()));
        assert_eq!(
            build_transcription_prompt(translate_en.as_deref()),
            "Transcribe the following audio file, and then translate the entire transcription into English."
        );

        // Explicit target language override (when translate_to_english is false)
        let explicit_target = resolve_gemini_target_language(false, Some("it"));
        assert_eq!(explicit_target, Some("Italian".to_string()));
        let explicit_auto = resolve_gemini_target_language(false, Some("auto"));
        assert_eq!(explicit_auto, None);
    }

    #[test]
    fn gemini_file_guard_drop_removes_temp_audio_file_synchronously() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let temp_audio = temp_dir.path().join("test_audio.ogg");
        fs::write(&temp_audio, b"audio data").expect("write temp audio");
        assert!(temp_audio.exists());

        let client = reqwest::Client::new();
        let mut guard = GeminiFileGuard::new(client, "http://127.0.0.1:9", "dummy_key");
        guard.set_temp_audio_file(temp_audio.clone());
        drop(guard);

        // Immediately after drop, the temp file must be gone synchronously without waiting for tokio network DELETE.
        assert!(
            !temp_audio.exists(),
            "temp audio file should be deleted synchronously on drop"
        );
    }

    #[test]
    fn gemini_audio_prep_commands_set_temp_dir() {
        let temp_dir = Path::new("/private/tmp/job-1");
        let cmd = build_audio_prep_command(
            Path::new("/tools/ffmpeg"),
            Path::new("/in/video.mp4"),
            Path::new("/private/tmp/job-1/audio.ogg"),
            "libopus",
            Some("24k"),
            Some("ogg"),
            temp_dir,
        );
        assert_eq!(cmd.env, process::temp_env(temp_dir));
    }

    #[test]
    fn gemini_audio_prep_needed_bytes_estimation() {
        // Unknown duration -> 200 MB default headroom
        assert_eq!(estimate_audio_prep_needed_bytes(None), 200 * 1024 * 1024);

        // Short duration (10s) -> 10 * 32,000 * 1.25 = 400,000 bytes -> clamped to 50 MB minimum
        assert_eq!(
            estimate_audio_prep_needed_bytes(Some(10.0)),
            50 * 1024 * 1024
        );

        // 1 hour (3600s) -> 3600 * 32,000 * 1.25 = 144,000,000 bytes
        assert_eq!(estimate_audio_prep_needed_bytes(Some(3600.0)), 144_000_000);
    }

    #[test]
    fn check_audio_prep_disk_space_fails_when_available_is_insufficient() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let free = crate::services::paths::free_disk_space(temp_dir.path()).unwrap_or(0);
        if free > 0 {
            // Demanding 100 GB more than current free space will always trigger DiskFull.
            let res = check_audio_prep_disk_space(
                temp_dir.path(),
                free.saturating_add(100 * 1024 * 1024 * 1024),
            );
            match res {
                Err(AppError::DiskFull { needed, available }) => {
                    assert!(needed > available);
                }
                other => panic!("expected DiskFull error, got: {:?}", other),
            }

            // Demanding a minimal amount (1 byte) must succeed.
            assert!(check_audio_prep_disk_space(temp_dir.path(), 1).is_ok());
        }
    }

    #[test]
    fn gemini_duration_limits_and_outcome_traits() {
        assert_eq!(GEMINI_MAX_AUDIO_DURATION_SECS, 9.5 * 3600.0);
        assert_eq!(GEMINI_LONG_AUDIO_THRESHOLD_SECS, 3600.0);

        let path = PathBuf::from("/tmp/test.txt");
        let outcome = GeminiTranscribeOutcome {
            txt_path: path.clone(),
            truncated: true,
            finish_reason: Some("MAX_TOKENS".to_string()),
            duration_sec: Some(4000.0),
            duration_warning: true,
        };

        // Deref to PathBuf
        assert_eq!(*outcome, path);
        // AsRef<Path>
        assert_eq!(outcome.as_ref(), path.as_path());
        assert!(outcome.truncated);
        assert!(outcome.duration_warning);
    }

    #[tokio::test]
    async fn m8_gemini_http_errors_and_polling_and_upload_quota_classified_properly() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let base_url = format!("http://127.0.0.1:{port}");

        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut req_line = String::new();
                if reader.read_line(&mut req_line).is_err() || req_line.is_empty() {
                    continue;
                }
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err()
                        || line == "\r\n"
                        || line == "\n"
                        || line.is_empty()
                    {
                        break;
                    }
                }

                let body_400_key = r#"{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT","details":[{"reason":"API_KEY_INVALID"}]}}"#;
                let body_429 = r#"{"error":{"code":429,"message":"Resource has been exhausted.","status":"RESOURCE_EXHAUSTED"}}"#;

                let resp = if req_line.contains("GET /v1beta/models")
                    || req_line.contains("POST /v1beta/models/bad-key-model:generateContent")
                    || req_line.contains("GET /v1beta/files/bad-key-file")
                {
                    format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body_400_key.len(),
                        body_400_key
                    )
                } else if req_line.contains("POST /upload/v1beta/files") {
                    format!(
                        "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body_429.len(),
                        body_429
                    )
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_string()
                };
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let client = reqwest::Client::new();
        let token = CancellationToken::new();

        // 1a. list_gemini_models on HTTP 400 API_KEY_INVALID -> GeminiKeyInvalid
        let list_res = list_gemini_models(&client, &base_url, "invalid-key").await;
        assert!(
            matches!(list_res, Err(AppError::GeminiKeyInvalid(_))),
            "expected GeminiKeyInvalid from list_gemini_models on 400, got: {list_res:?}"
        );

        // 1b. generate_transcription on HTTP 400 API_KEY_INVALID -> GeminiKeyInvalid
        let gen_res = generate_transcription(
            &client,
            &base_url,
            "invalid-key",
            "bad-key-model",
            "prompt",
            "https://example.com/file",
            "audio/ogg",
            &token,
        )
        .await;
        assert!(
            matches!(gen_res, Err(AppError::GeminiKeyInvalid(_))),
            "expected GeminiKeyInvalid from generate_transcription on 400, got: {gen_res:?}"
        );

        // 1c. call_gemini_generate_text (summary) on HTTP 400 API_KEY_INVALID -> GeminiKeyInvalid
        let sum_res = crate::services::transcribe::summary::call_gemini_generate_text(
            &client,
            &base_url,
            "invalid-key",
            "bad-key-model",
            "prompt",
            &token,
        )
        .await;
        assert!(
            matches!(sum_res, Err(AppError::GeminiKeyInvalid(_))),
            "expected GeminiKeyInvalid from call_gemini_generate_text on 400, got: {sum_res:?}"
        );

        // 2. poll_file_active on HTTP 400 fails immediately with GeminiKeyInvalid instead of sleeping until timeout
        let poll_start = tokio::time::Instant::now();
        let poll_res = poll_file_active(
            &client,
            &base_url,
            "invalid-key",
            "files/bad-key-file",
            Duration::from_secs(5),
            Duration::from_secs(10),
            &token,
        )
        .await;
        assert!(
            poll_start.elapsed() < Duration::from_secs(1),
            "poll_file_active should fail immediately on non-retryable 400, not sleep"
        );
        assert!(
            matches!(poll_res, Err(AppError::GeminiKeyInvalid(_))),
            "expected GeminiKeyInvalid from poll_file_active on 400, got: {poll_res:?}"
        );

        // 3. upload_file_resumable on HTTP 429 -> GeminiQuotaExceeded
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let dummy_audio = temp_dir.path().join("audio.ogg");
        fs::write(&dummy_audio, b"ogg-data").expect("write audio");
        let upload_res = upload_file_resumable(
            &client,
            &base_url,
            "valid-key",
            &dummy_audio,
            "audio/ogg",
            &token,
        )
        .await;
        assert!(
            matches!(upload_res, Err(AppError::GeminiQuotaExceeded(_))),
            "expected GeminiQuotaExceeded from upload_file_resumable on 429, got: {upload_res:?}"
        );
    }

    #[test]
    fn upload_file_resumable_contains_no_unwrap_in_source() {
        let source = include_str!("gemini.rs");
        let start = source
            .find("pub async fn upload_file_resumable")
            .expect("upload_file_resumable fn exists");
        let end = source[start..]
            .find("pub async fn list_gemini_models")
            .expect("list_gemini_models fn exists");
        let fn_body = &source[start..start + end];
        assert!(
            !fn_body.contains(".unwrap()"),
            "upload_file_resumable must not contain .unwrap() in non-test code (m2, AGENTS.md §6)"
        );
    }

    #[tokio::test]
    async fn upload_file_resumable_handles_invalid_mime_header_without_unwrap_panic() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let dummy_audio = temp_dir.path().join("audio.ogg");
        fs::write(&dummy_audio, b"ogg-data").expect("write audio");
        let client = reqwest::Client::new();
        let token = CancellationToken::new();

        let res = upload_file_resumable(
            &client,
            "http://127.0.0.1:1",
            "fake-key",
            &dummy_audio,
            "audio/ogg\r\ninvalid",
            &token,
        )
        .await;

        assert!(res.is_err(), "Must return Err for invalid MIME header");
        let err = res.err().unwrap();
        assert!(
            matches!(err, AppError::Process(ref msg) if msg.contains("Invalid MIME header")),
            "Error must be AppError::Process reporting Invalid MIME header, got: {err:?}"
        );
    }
}
