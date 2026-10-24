//! Integration tests for Gemini Cloud Transcription, Subtitles First, and Summaries (Phase M6).
//!
//! Covers (spec §10.1, §10.3, §10.4, §10.5, §15 items 10 & 11):
//! - Subtitle cleaner: removes rolling repeated lines, cleans tags, groups paragraphs on pauses.
//! - Gemini REST client without SDK:
//!   - Resumable upload streamed from disk.
//!   - Polling until ACTIVE with timeout and cancel.
//!   - generateContent verbatim prompts.
//!   - RAII cleanup guard that DELETEs remote file on success, error, and cancel.
//!   - 429 quota backoff retry.
//!   - Retired model (404) returns GeminiModelNotFound.
//!   - Dynamic models list from GET /v1beta/models.
//! - Text-only summary with map-reduce for long transcripts.
//! - Queue engine transcript origin tracking and engine ordering.

#![allow(clippy::collapsible_if)]

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use scaricatore_di_porto_lib::error::{AppError, ErrorInfo};
use scaricatore_di_porto_lib::services::job::JobEnvironment;
use scaricatore_di_porto_lib::services::jsruntime::{JsRuntimeMode, RuntimeMemoryStore};
use scaricatore_di_porto_lib::services::queue::engine::{
    AddRequest, BoxFuture, EngineConfig, IdleNotifier, QueueEngine, ToolProvider,
};
use scaricatore_di_porto_lib::services::queue::model::{
    CookieChoice, ItemOptions, ItemState, Kind, QualityChoice, QueueSummary,
};
use scaricatore_di_porto_lib::services::transcribe::gemini::{
    GeminiFileGuard, GeminiProgressSink, GeminiTranscribeOutcome, GeminiTranscribeRequest,
    list_gemini_models, poll_file_active, prepare_audio_for_gemini, run_gemini_transcription,
    upload_file_resumable,
};
use scaricatore_di_porto_lib::services::transcribe::subtitles::clean_subtitles;
use scaricatore_di_porto_lib::services::transcribe::summary::{
    MAX_SUMMARY_CHUNK_CHARS, SummaryRequest, generate_summary, split_text_into_chunks,
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// Locates the `fake_tool` example built by `cargo test`.
fn fake_tool_source() -> PathBuf {
    let profile_dir = std::env::current_exe()
        .expect("the test binary has a path")
        .parent()
        .and_then(Path::parent)
        .expect("the test binary lives in <profile>/deps/")
        .to_path_buf();
    let mut path = profile_dir.join("examples").join("fake_tool");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    assert!(
        path.is_file(),
        "the fake_tool example was not built next to the tests (looked for {}).\n\
         Run the suite as `cargo test`.",
        path.display()
    );
    path
}

// ---------------------------------------------------------------------------
// Mock Gemini HTTP Server
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    #[allow(dead_code)]
    body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MockBehavior {
    Normal,
    FailGenerateContent404,
    FailGenerateContent500,
    RateLimitOnceThenSucceed,
    SlowPoll,
    PollFailed,
    PollStuckProcessing,
    HangGenerateContent,
    TruncatedMaxTokens,
    BlockedPromptFeedback,
}

struct MockGeminiServer {
    port: u16,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    #[allow(dead_code)]
    behavior: Arc<Mutex<MockBehavior>>,
    rate_limit_counter: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockGeminiServer {
    fn start(behavior: MockBehavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let behavior = Arc::new(Mutex::new(behavior));
        let rate_limit_counter = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));

        let t_requests = Arc::clone(&requests);
        let t_behavior = Arc::clone(&behavior);
        let t_rate_limit = Arc::clone(&rate_limit_counter);
        let t_stop = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            for stream in listener.incoming() {
                if t_stop.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                serve_gemini_request(stream, port, &t_requests, &t_behavior, &t_rate_limit);
            }
        });

        Self {
            port,
            requests,
            behavior,
            rate_limit_counter,
            stop,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn recorded_requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    async fn wait_for_delete(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            let reqs = self.recorded_requests();
            if reqs
                .iter()
                .any(|r| r.method == "DELETE" && r.path.contains("mock-file-123"))
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }
}

impl Drop for MockGeminiServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn serve_gemini_request(
    mut stream: TcpStream,
    port: u16,
    requests: &Mutex<Vec<RecordedRequest>>,
    behavior: &Mutex<MockBehavior>,
    rate_limit_counter: &AtomicUsize,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    });

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
        return;
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();

    let mut headers = HashMap::new();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line == "\n" || line.is_empty()
        {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let k_norm = k.trim().to_ascii_lowercase();
            let v_norm = v.trim().to_string();
            if k_norm == "content-length" {
                content_length = v_norm.parse().unwrap_or(0);
            }
            headers.insert(k_norm, v_norm);
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }

    requests.lock().unwrap().push(RecordedRequest {
        method: method.clone(),
        path: path.clone(),
        headers: headers.clone(),
        body: body.clone(),
    });

    let cur_behavior = behavior.lock().unwrap().clone();

    // Route response based on method and path
    if method == "POST" && path.starts_with("/upload/v1beta/files") {
        // Step 1: Resumable upload start -> return upload URL
        let upload_url = format!("http://127.0.0.1:{}/resumable-upload/mock-file-123", port);
        let resp = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nx-goog-upload-url: {}\r\nContent-Length: 0\r\n\r\n",
            upload_url
        );
        let _ = stream.write_all(resp.as_bytes());
    } else if method == "POST" && path.starts_with("/resumable-upload/mock-file-123") {
        // Step 2: Upload streamed bytes -> return UploadedFile JSON
        let json = r#"{"file":{"name":"files/mock-file-123","uri":"https://generativelanguage.googleapis.com/v1beta/files/mock-file-123","state":"PROCESSING"}}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            json.len(),
            json
        );
        let _ = stream.write_all(resp.as_bytes());
    } else if method == "GET" && path.starts_with("/v1beta/files/mock-file-123") {
        if cur_behavior == MockBehavior::SlowPoll {
            thread::sleep(Duration::from_millis(300));
        }
        let json = match cur_behavior {
            MockBehavior::PollFailed => {
                r#"{"name":"files/mock-file-123","uri":"https://generativelanguage.googleapis.com/v1beta/files/mock-file-123","state":"FAILED","error":{"code":400,"message":"Audio stream corrupted"}}"#
            }
            MockBehavior::PollStuckProcessing => {
                r#"{"name":"files/mock-file-123","uri":"https://generativelanguage.googleapis.com/v1beta/files/mock-file-123","state":"PROCESSING"}"#
            }
            _ => {
                r#"{"name":"files/mock-file-123","uri":"https://generativelanguage.googleapis.com/v1beta/files/mock-file-123","state":"ACTIVE"}"#
            }
        };
        let resp = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            json.len(),
            json
        );
        let _ = stream.write_all(resp.as_bytes());
    } else if method == "POST" && path.contains(":generateContent") {
        // Step 4: generateContent
        match cur_behavior {
            MockBehavior::FailGenerateContent404 => {
                let json = r#"{"error":{"code":404,"message":"models/gemini-2.5-flash is not found for API version v1beta, or is not supported for generateContent","status":"NOT_FOUND"}}"#;
                let resp = format!(
                    "HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json.len(),
                    json
                );
                let _ = stream.write_all(resp.as_bytes());
            }
            MockBehavior::FailGenerateContent500 => {
                let json = r#"{"error":{"code":500,"message":"Internal server error","status":"INTERNAL"}}"#;
                let resp = format!(
                    "HTTP/1.1 500 Internal Server Error\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json.len(),
                    json
                );
                let _ = stream.write_all(resp.as_bytes());
            }
            MockBehavior::RateLimitOnceThenSucceed => {
                let count = rate_limit_counter.fetch_add(1, Ordering::SeqCst);
                if count == 0 {
                    let json = r#"{"error":{"code":429,"message":"Resource has been exhausted (e.g. check quota).","status":"RESOURCE_EXHAUSTED"}}"#;
                    let resp = format!(
                        "HTTP/1.1 429 Too Many Requests\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        json.len(),
                        json
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else {
                    let json = r#"{"candidates":[{"content":{"parts":[{"text":"Transcribed text after quota backoff."}]},"finishReason":"STOP"}]}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        json.len(),
                        json
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
            }
            MockBehavior::HangGenerateContent => {
                // Simulate a long-running in-flight HTTP request (e.g. 5 seconds)
                thread::sleep(Duration::from_secs(5));
                let json = r#"{"candidates":[{"content":{"parts":[{"text":"Late response"}]},"finishReason":"STOP"}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json.len(),
                    json
                );
                let _ = stream.write_all(resp.as_bytes());
            }
            MockBehavior::TruncatedMaxTokens => {
                let json = serde_json::json!({
                    "candidates": [
                        {
                            "content": {
                                "parts": [
                                    { "text": "This is a partial transcript truncated by token limit." }
                                ]
                            },
                            "finishReason": "MAX_TOKENS"
                        }
                    ]
                })
                .to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json.len(),
                    json
                );
                let _ = stream.write_all(resp.as_bytes());
            }
            MockBehavior::BlockedPromptFeedback => {
                let json = serde_json::json!({
                    "promptFeedback": {
                        "blockReason": "SAFETY"
                    }
                })
                .to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json.len(),
                    json
                );
                let _ = stream.write_all(resp.as_bytes());
            }
            MockBehavior::Normal
            | MockBehavior::SlowPoll
            | MockBehavior::PollFailed
            | MockBehavior::PollStuckProcessing => {
                // If it's a summary request or transcription request:
                let body_str = String::from_utf8_lossy(&body);
                let text_resp = if body_str.contains("summary") || body_str.contains("Riassumi") {
                    "## Summary\n\n- Key point 1\n- Key point 2"
                } else {
                    "This is the transcribed text from mock Gemini API."
                };
                let json = serde_json::json!({
                    "candidates": [
                        {
                            "content": {
                                "parts": [
                                    { "text": text_resp }
                                ]
                            },
                            "finishReason": "STOP"
                        }
                    ]
                })
                .to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json.len(),
                    json
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        }
    } else if method == "DELETE" && path.starts_with("/v1beta/files/mock-file-123") {
        // Step 5: DELETE file cleanup
        let json = r#"{"status":"DELETED"}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            json.len(),
            json
        );
        let _ = stream.write_all(resp.as_bytes());
    } else if method == "GET" && path.starts_with("/v1beta/models") {
        // Models list
        let json = r#"{
            "models": [
                {
                    "name": "models/gemini-2.5-flash",
                    "displayName": "Gemini 2.5 Flash (Deprecated)",
                    "description": "Old model",
                    "supportedGenerationMethods": ["generateContent"]
                },
                {
                    "name": "models/gemini-flash-latest",
                    "displayName": "Gemini Flash Latest",
                    "description": "Recommended latest model",
                    "supportedGenerationMethods": ["generateContent"]
                },
                {
                    "name": "models/gemini-flash-lite-latest",
                    "displayName": "Gemini Flash Lite Latest",
                    "description": "Lightweight model",
                    "supportedGenerationMethods": ["generateContent"]
                },
                {
                    "name": "models/text-embedding-004",
                    "displayName": "Embedding Model",
                    "description": "Embedding only",
                    "supportedGenerationMethods": ["embedContent"]
                }
            ]
        }"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            json.len(),
            json
        );
        let _ = stream.write_all(resp.as_bytes());
    } else {
        let resp = "HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes());
    }
}

// ---------------------------------------------------------------------------
// Test Fixture
// ---------------------------------------------------------------------------

struct Fixture {
    dir: TempDir,
    fake_ffmpeg: PathBuf,
    #[allow(dead_code)]
    fake_ffprobe: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");

        let fake_ffmpeg = bin_dir.join(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        });
        fs::copy(fake_tool_source(), &fake_ffmpeg).expect("copy fake_tool as ffmpeg");

        let fake_ffprobe = bin_dir.join(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
        fs::copy(fake_tool_source(), &fake_ffprobe).expect("copy fake_tool as ffprobe");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&fake_ffmpeg).expect("metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&fake_ffmpeg, perms).expect("chmod +x");

            let mut probe_perms = fs::metadata(&fake_ffprobe).expect("metadata").permissions();
            probe_perms.set_mode(0o755);
            fs::set_permissions(&fake_ffprobe, probe_perms).expect("chmod +x");
        }

        Self {
            dir,
            fake_ffmpeg,
            fake_ffprobe,
        }
    }

    fn create_dummy_media(&self, name: &str) -> PathBuf {
        let p = self.dir.path().join(name);
        fs::write(&p, b"RIFF....WAVEfmt ....data....").expect("write dummy media");
        p
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn subtitle_cleaner_removes_rolling_duplicates_and_splits_paragraphs() {
    let srt_content = "\
1\n\
00:00:01,000 --> 00:00:02,500\n\
Benvenuti a questa\n\
\n\
2\n\
00:00:02,500 --> 00:00:04,000\n\
Benvenuti a questa presentazione del nuovo\n\
\n\
3\n\
00:00:04,000 --> 00:00:06,000\n\
presentazione del nuovo <font color=\"red\">software</font>.\n\
\n\
4\n\
00:00:10,000 --> 00:00:12,000\n\
Passiamo ora al secondo argomento.\n\
";

    let cleaned = clean_subtitles(srt_content);

    // 1. Should deduplicate rolling text cleanly
    assert!(
        cleaned.contains("Benvenuti a questa presentazione del nuovo software."),
        "rolling repetitions should be eliminated, got:\n{cleaned}"
    );
    // 2. HTML tags should be stripped
    assert!(!cleaned.contains("<font"), "HTML tags must be stripped");
    // 3. Pause between 00:00:06 and 00:00:10 is 4.0s (>= 2s) -> paragraph break \n\n
    assert!(
        cleaned.contains("\n\nPassiamo ora al secondo argomento."),
        "long pause must create a paragraph break"
    );
}

#[tokio::test]
async fn gemini_transcription_end_to_end_with_mock_server_and_raii_delete_guard() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample.mp4");
    let server = MockGeminiServer::start(MockBehavior::Normal);

    let output_dir = fix.dir.path().join("out");
    fs::create_dir_all(&output_dir).expect("out dir");
    let temp_dir = fix.dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    let progress_calls = Arc::new(AtomicUsize::new(0));
    let progress_calls_clone = Arc::clone(&progress_calls);
    let sink: GeminiProgressSink = Arc::new(move |_pct, _lbl| {
        progress_calls_clone.fetch_add(1, Ordering::SeqCst);
    });

    let req = GeminiTranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        input_media: media,
        temp_dir: temp_dir.clone(),
        output_dir: output_dir.clone(),
        base_filename: "sample".to_string(),
        transcripts_subfolder: false,
        api_key: "test-fake-key-123".to_string(),
        model: "gemini-flash-latest".to_string(),
        target_language: None,
        base_url: Some(server.base_url()),
        poll_interval: Some(Duration::from_millis(50)),
        duration_sec: None,
    };

    let token = CancellationToken::new();
    let res = run_gemini_transcription(req, token, Some(sink)).await;
    assert!(res.is_ok(), "transcription should succeed: {:?}", res.err());

    let txt_path = res.unwrap();
    assert!(txt_path.is_file(), "output .txt file must exist");
    let text = fs::read_to_string(&txt_path).expect("read txt");
    assert!(text.contains("This is the transcribed text from mock Gemini API."));

    // Verify progress sink was called
    assert!(progress_calls.load(Ordering::SeqCst) > 0);

    // Verify mock server received all expected requests, especially DELETE
    let requests = server.recorded_requests();
    let delete_req = requests
        .iter()
        .find(|r| r.method == "DELETE" && r.path.contains("mock-file-123"));
    assert!(
        delete_req.is_some(),
        "DELETE /v1beta/files/mock-file-123 must be called by RAII guard!"
    );
    assert_eq!(
        delete_req.unwrap().headers.get("x-goog-api-key"),
        Some(&"test-fake-key-123".to_string()),
        "x-goog-api-key header must be present on DELETE request"
    );

    // Verify temp audio file was cleaned up
    let temp_entries: Vec<_> = fs::read_dir(&temp_dir).unwrap().collect();
    assert!(
        temp_entries.is_empty(),
        "temp directory must be empty after RAII cleanup, found: {:?}",
        temp_entries
    );
}

#[tokio::test]
async fn gemini_transcription_maps_iso_code_to_full_language_name_in_prompt() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample_it.mp4");
    let server = MockGeminiServer::start(MockBehavior::Normal);

    let output_dir = fix.dir.path().join("out_it");
    fs::create_dir_all(&output_dir).expect("out dir");
    let temp_dir = fix.dir.path().join("tmp_it");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    let req = GeminiTranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        input_media: media,
        temp_dir: temp_dir.clone(),
        output_dir: output_dir.clone(),
        base_filename: "sample_it".to_string(),
        transcripts_subfolder: false,
        api_key: "test-fake-key-123".to_string(),
        model: "gemini-flash-latest".to_string(),
        target_language: Some("it".to_string()),
        base_url: Some(server.base_url()),
        poll_interval: Some(Duration::from_millis(50)),
        duration_sec: None,
    };

    let token = CancellationToken::new();
    let res = run_gemini_transcription(req, token, None).await;
    assert!(res.is_ok(), "transcription should succeed: {:?}", res.err());

    let requests = server.recorded_requests();
    let gen_req = requests
        .iter()
        .find(|r| r.method == "POST" && r.path.contains(":generateContent"))
        .expect("generateContent request must be recorded");

    let body_json: serde_json::Value =
        serde_json::from_slice(&gen_req.body).expect("parse generateContent JSON");
    let prompt_text = body_json["contents"][0]["parts"][0]["text"]
        .as_str()
        .expect("prompt text string");

    assert_eq!(
        prompt_text,
        "Transcribe the following audio file, and then translate the entire transcription into Italian."
    );
}

#[tokio::test]
async fn gemini_raii_delete_guard_on_generation_error() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample_error.mp4");
    let server = MockGeminiServer::start(MockBehavior::FailGenerateContent500);

    let output_dir = fix.dir.path().join("out_err");
    fs::create_dir_all(&output_dir).expect("out dir");
    let temp_dir = fix.dir.path().join("tmp_err");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    let req = GeminiTranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        input_media: media,
        temp_dir: temp_dir.clone(),
        output_dir: output_dir.clone(),
        base_filename: "sample_error".to_string(),
        transcripts_subfolder: false,
        api_key: "test-fake-key-error".to_string(),
        model: "gemini-flash-latest".to_string(),
        target_language: None,
        base_url: Some(server.base_url()),
        poll_interval: Some(Duration::from_millis(50)),
        duration_sec: None,
    };

    let token = CancellationToken::new();
    let res = run_gemini_transcription(req, token, None).await;
    assert!(res.is_err(), "transcription must fail on 500 error");

    // Even though it failed, DELETE must have been called by RAII guard!
    let deleted = server.wait_for_delete(Duration::from_secs(1)).await;
    assert!(
        deleted,
        "DELETE must be called by RAII guard on generation error!"
    );
}

#[tokio::test]
async fn gemini_raii_delete_guard_on_cancellation() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample_cancel.mp4");
    let server = MockGeminiServer::start(MockBehavior::SlowPoll);

    let output_dir = fix.dir.path().join("out_cancel");
    fs::create_dir_all(&output_dir).expect("out dir");
    let temp_dir = fix.dir.path().join("tmp_cancel");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    let token = CancellationToken::new();
    let cancel_token = token.clone();

    // Cancel after 50ms so it cancels while polling ACTIVE
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_token.cancel();
    });

    let req = GeminiTranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        input_media: media,
        temp_dir: temp_dir.clone(),
        output_dir: output_dir.clone(),
        base_filename: "sample_cancel".to_string(),
        transcripts_subfolder: false,
        api_key: "test-fake-key-cancel".to_string(),
        model: "gemini-flash-latest".to_string(),
        target_language: None,
        base_url: Some(server.base_url()),
        poll_interval: Some(Duration::from_millis(50)),
        duration_sec: None,
    };

    let res = run_gemini_transcription(req, token, None).await;
    assert!(res.is_err());
    assert!(
        matches!(res.err().unwrap(), AppError::Process(msg) if msg.contains("Cancelled")),
        "error must be cancellation"
    );

    // Verify DELETE was triggered on cancellation
    let deleted = server.wait_for_delete(Duration::from_secs(1)).await;
    assert!(
        deleted,
        "DELETE must be called by RAII guard upon cancellation!"
    );
}

#[tokio::test]
async fn gemini_429_quota_backoff_retry_succeeds() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample_429.mp4");
    let server = MockGeminiServer::start(MockBehavior::RateLimitOnceThenSucceed);

    let output_dir = fix.dir.path().join("out_429");
    fs::create_dir_all(&output_dir).expect("out dir");
    let temp_dir = fix.dir.path().join("tmp_429");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    let req = GeminiTranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        input_media: media,
        temp_dir: temp_dir.clone(),
        output_dir: output_dir.clone(),
        base_filename: "sample_429".to_string(),
        transcripts_subfolder: false,
        api_key: "test-fake-key-429".to_string(),
        model: "gemini-flash-latest".to_string(),
        target_language: None,
        base_url: Some(server.base_url()),
        poll_interval: Some(Duration::from_millis(50)),
        duration_sec: None,
    };

    let token = CancellationToken::new();
    let res = run_gemini_transcription(req, token, None).await;
    assert!(
        res.is_ok(),
        "transcription should succeed after 429 backoff retry"
    );
    assert_eq!(server.rate_limit_counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn gemini_retired_model_404_returns_model_not_found() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample_404.mp4");
    let server = MockGeminiServer::start(MockBehavior::FailGenerateContent404);

    let output_dir = fix.dir.path().join("out_404");
    fs::create_dir_all(&output_dir).expect("out dir");
    let temp_dir = fix.dir.path().join("tmp_404");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    let req = GeminiTranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        input_media: media,
        temp_dir: temp_dir.clone(),
        output_dir: output_dir.clone(),
        base_filename: "sample_404".to_string(),
        transcripts_subfolder: false,
        api_key: "test-fake-key-404".to_string(),
        model: "gemini-2.5-flash".to_string(),
        target_language: None,
        base_url: Some(server.base_url()),
        poll_interval: Some(Duration::from_millis(50)),
        duration_sec: None,
    };

    let token = CancellationToken::new();
    let res = run_gemini_transcription(req, token, None).await;
    assert!(res.is_err());
    match res.err().unwrap() {
        AppError::GeminiModelNotFound(m) => {
            assert_eq!(m, "gemini-2.5-flash");
        }
        other => panic!("expected GeminiModelNotFound, got: {:?}", other),
    }
}

#[tokio::test]
async fn gemini_list_models_filters_and_sorts_latest_aliases() {
    let server = MockGeminiServer::start(MockBehavior::Normal);
    let client = reqwest::Client::new();
    let models = list_gemini_models(&client, &server.base_url(), "test-key")
        .await
        .expect("list models");

    // Only models with "generateContent" should be kept (embedding model filtered out)
    assert_eq!(models.len(), 3);
    assert!(!models.iter().any(|m| m.id.contains("embedding")));

    // -latest aliases should be placed first
    assert!(models[0].id.ends_with("-latest"));
    assert!(models[1].id.ends_with("-latest"));
    assert_eq!(models[0].id, "gemini-flash-latest");
    assert_eq!(models[1].id, "gemini-flash-lite-latest");
}

#[tokio::test]
async fn gemini_summary_single_and_map_reduce() {
    let fix = Fixture::new();
    let server = MockGeminiServer::start(MockBehavior::Normal);

    let output_dir = fix.dir.path().join("summary_out");
    fs::create_dir_all(&output_dir).expect("summary out");

    // 1. Short text -> single prompt
    let short_req = SummaryRequest {
        transcript_text: "Short transcript for testing single prompt summary.".to_string(),
        language: Some("it".to_string()),
        model: Some("gemini-flash-latest".to_string()),
        api_key: "fake-key".to_string(),
        output_dir: output_dir.clone(),
        base_filename: "short_test".to_string(),
        transcripts_subfolder: false,
        base_url: Some(server.base_url()),
    };

    let token = CancellationToken::new();
    let short_path = generate_summary(short_req, token.clone())
        .await
        .expect("generate short summary");
    assert!(short_path.is_file());
    assert_eq!(short_path.file_name().unwrap(), "short_test.summary.md");

    // 2. Long text exceeding MAX_SUMMARY_CHUNK_CHARS -> map-reduce
    let paragraph =
        "Questo è un paragrafo lungo che contiene informazioni dettagliate sulla conferenza.\n\n";
    let repeats = (MAX_SUMMARY_CHUNK_CHARS / paragraph.len()) + 5;
    let long_transcript = paragraph.repeat(repeats);

    let chunks = split_text_into_chunks(&long_transcript, MAX_SUMMARY_CHUNK_CHARS);
    assert!(
        chunks.len() >= 2,
        "must be split into at least 2 chunks for map-reduce"
    );

    let long_req = SummaryRequest {
        transcript_text: long_transcript,
        language: Some("it".to_string()),
        model: Some("gemini-flash-latest".to_string()),
        api_key: "fake-key".to_string(),
        output_dir: output_dir.clone(),
        base_filename: "long_test".to_string(),
        transcripts_subfolder: false,
        base_url: Some(server.base_url()),
    };

    let long_path = generate_summary(long_req, token)
        .await
        .expect("generate map-reduce summary");
    assert!(long_path.is_file());
    assert_eq!(long_path.file_name().unwrap(), "long_test.summary.md");
}

#[tokio::test]
async fn gemini_summary_splits_whisper_single_newline_and_requests_chapters() {
    let fix = Fixture::new();
    let server = MockGeminiServer::start(MockBehavior::Normal);

    let output_dir = fix.dir.path().join("summary_whisper_out");
    fs::create_dir_all(&output_dir).expect("summary out");

    // 1. Whisper transcript without "\n\n" exceeding MAX_SUMMARY_CHUNK_CHARS
    let line = "00:01:00 Questo è un segmento whisper senza doppi ritorni a capo.\n";
    let repeats = (MAX_SUMMARY_CHUNK_CHARS / line.len()) + 5;
    let whisper_transcript = line.repeat(repeats);

    let chunks = split_text_into_chunks(&whisper_transcript, MAX_SUMMARY_CHUNK_CHARS);
    assert!(
        chunks.len() >= 2,
        "whisper transcript with single newlines must be split into >= 2 chunks"
    );
    for chunk in &chunks {
        assert!(chunk.len() <= MAX_SUMMARY_CHUNK_CHARS);
    }

    let req = SummaryRequest {
        transcript_text: whisper_transcript,
        language: Some("it".to_string()),
        model: Some("gemini-flash-latest".to_string()),
        api_key: "fake-key".to_string(),
        output_dir: output_dir.clone(),
        base_filename: "whisper_summary".to_string(),
        transcripts_subfolder: false,
        base_url: Some(server.base_url()),
    };

    let token = CancellationToken::new();
    let path = generate_summary(req, token)
        .await
        .expect("generate summary for whisper transcript");
    assert!(path.is_file());
    assert_eq!(path.file_name().unwrap(), "whisper_summary.summary.md");

    // Verify recorded prompt asks for Chapters with timestamps
    let requests = server.recorded_requests();
    let gen_reqs: Vec<_> = requests
        .iter()
        .filter(|r| r.method == "POST" && r.path.contains(":generateContent"))
        .collect();
    assert!(gen_reqs.len() >= 2, "must perform map-reduce requests");

    // Map prompt must ask to preserve timestamps
    let first_body = String::from_utf8_lossy(&gen_reqs[0].body);
    assert!(
        first_body.contains("timestamps"),
        "map prompt should mention timestamps: {}",
        first_body
    );

    // Reduce prompt must request # Chapters
    let reduce_body = String::from_utf8_lossy(&gen_reqs.last().unwrap().body);
    assert!(
        reduce_body.contains("# Chapters"),
        "reduce prompt should include # Chapters: {}",
        reduce_body
    );
}

// ---------------------------------------------------------------------------
// Engine ordering test
// ---------------------------------------------------------------------------

struct QueueFakeTools {
    whisper: Option<PathBuf>,
    ffmpeg: PathBuf,
    models_dir: PathBuf,
    config_dir: Option<PathBuf>,
    default_model: String,
    private_temp: Option<PathBuf>,
}

impl ToolProvider for QueueFakeTools {
    fn environment(&self, options: ItemOptions) -> BoxFuture<Result<JobEnvironment, ErrorInfo>> {
        fs::create_dir_all(&options.output_dir).ok();
        let env = JobEnvironment {
            ytdlp: self.ffmpeg.clone(),
            ffmpeg: Some(self.ffmpeg.clone()),
            ffprobe: scaricatore_di_porto_lib::services::ffmpeg::find_ffprobe(&self.ffmpeg, None),
            whisper: self.whisper.clone(),
            models_dir: Some(self.models_dir.clone()),
            config_dir: self.config_dir.clone(),
            whisper_threads: 2,
            skip_silence_vad: false,
            default_whisper_model: Some(self.default_model.clone()),
            default_transcript_language: None,
            default_transcription_engine_order: None,
            default_gemini_model: None,
            default_summarize: false,
            default_summary_language: None,
            runtimes: scaricatore_di_porto_lib::services::inspect::Runtimes {
                mode: JsRuntimeMode::QuickJsOnly,
                qjs: None,
                deno: None,
                ytdlp_version: "2026.01.01".to_string(),
            },
            memory: Arc::new(RuntimeMemoryStore::load(self.models_dir.join("mem.json"))),
            private_temp: self.private_temp.clone(),
            youtube_clients: None,
        };
        Box::pin(std::future::ready(Ok(env)))
    }
}

struct DummyNotifier;
impl IdleNotifier for DummyNotifier {
    fn notify(&self, _summary: &QueueSummary) {}
}

#[tokio::test]
async fn queue_engine_fails_subtitles_only_when_local_or_no_subs() {
    let fix = Fixture::new();
    let local_file = fix.create_dummy_media("local_no_subs.mp4");

    let queue_path = fix.dir.path().join("queue_subs_only.json");
    let tools = Arc::new(QueueFakeTools {
        whisper: None,
        ffmpeg: fix.fake_ffmpeg.clone(),
        models_dir: fix.dir.path().join("models"),
        config_dir: None,
        default_model: "large-v3-turbo-q5_0".to_string(),
        private_temp: None,
    });

    let config = EngineConfig {
        path: queue_path,
        provider: tools,
        sink: Arc::new(|_update| {}),
        lanes: scaricatore_di_porto_lib::services::queue::model::LaneConfig::default(),
        notify_on_idle: false,
        notifier: Some(Arc::new(DummyNotifier)),
    };

    let engine = QueueEngine::open(config);
    engine.start();

    let req = AddRequest {
        url: format!("local://{}", local_file.display()),
        title: Some("Local No Subs".to_string()),
        site: Some("local".to_string()),
        duration_sec: Some(10),
        options: ItemOptions {
            kind: Kind::Video,
            quality: QualityChoice::Best,
            output_dir: fix.dir.path().join("out"),
            cookies: CookieChoice::None,
            runtime_mode: JsRuntimeMode::QuickJsOnly,
            transcribe: true,
            transcription_engine: Some("subtitles_only".to_string()),
            ..ItemOptions::default()
        },
        group_id: None,
        file: Some(local_file.clone()),
    };

    let outcome = engine.add(vec![req], true);
    let item_id = outcome.added[0].clone();

    let mut failed = false;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let snapshot = engine.snapshot();
        if let Some(item) = snapshot.items.iter().find(|i| i.id == item_id) {
            if item.state == ItemState::Failed {
                failed = true;
                let error = item.error.as_ref().expect("error present");
                assert_eq!(error.code, "NO_SUBTITLES");
                assert_eq!(error.message_key, "error.ytdlp.noSubtitles");
                break;
            }
        }
    }
    assert!(
        failed,
        "item with subtitles_only on local media should fail with NO_SUBTITLES"
    );
}

#[tokio::test]
async fn transcription_and_summary_do_not_overwrite_existing_files() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("lecture.mp4");
    let server = MockGeminiServer::start(MockBehavior::Normal);

    let out_dir = fix.dir.path().join("out");
    fs::create_dir_all(&out_dir).expect("out dir");
    let temp_dir = fix.dir.path().join("tmp");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    let existing_txt = out_dir.join("lecture.txt");
    let existing_summary = out_dir.join("lecture.summary.md");
    fs::write(&existing_txt, "original user transcript").expect("write existing txt");
    fs::write(&existing_summary, "original user summary").expect("write existing summary");

    let req = GeminiTranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        input_media: media,
        temp_dir: temp_dir.clone(),
        output_dir: out_dir.clone(),
        base_filename: "lecture".to_string(),
        transcripts_subfolder: false,
        api_key: "test-fake-key-123".to_string(),
        model: "gemini-flash-latest".to_string(),
        target_language: None,
        base_url: Some(server.base_url()),
        poll_interval: Some(Duration::from_millis(50)),
        duration_sec: None,
    };

    let token = CancellationToken::new();
    let res = run_gemini_transcription(req, token.clone(), None).await;
    assert!(res.is_ok(), "transcription should succeed: {:?}", res.err());
    let txt_path = res.unwrap();

    let sum_req = SummaryRequest {
        transcript_text: "Some transcript text".to_string(),
        language: None,
        model: None,
        api_key: "test-fake-key-123".to_string(),
        output_dir: out_dir.clone(),
        base_filename: "lecture".to_string(),
        transcripts_subfolder: false,
        base_url: Some(server.base_url()),
    };

    let sum_res = generate_summary(sum_req, token).await;
    assert!(
        sum_res.is_ok(),
        "summary should succeed: {:?}",
        sum_res.err()
    );
    let summary_path = sum_res.unwrap();

    assert_eq!(
        fs::read_to_string(&existing_txt).expect("read existing txt"),
        "original user transcript",
        "existing .txt must never be silently overwritten"
    );
    assert_eq!(
        fs::read_to_string(&existing_summary).expect("read existing summary"),
        "original user summary",
        "existing .summary.md must never be silently overwritten"
    );
    assert_eq!(txt_path, out_dir.join("lecture (1).txt"));
    assert_eq!(summary_path, out_dir.join("lecture (1).summary.md"));
    assert!(txt_path.is_file());
    assert!(summary_path.is_file());
}

#[tokio::test]
async fn cancelling_gemini_in_flight_http_aborts_promptly_and_does_not_swallow_summary_cancel() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("hang_cancel.mp4");
    let server = MockGeminiServer::start(MockBehavior::HangGenerateContent);

    let out_dir = fix.dir.path().join("out_hang");
    fs::create_dir_all(&out_dir).expect("out dir");
    let temp_dir = fix.dir.path().join("tmp_hang");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    // Part 1: Cancelling an in-flight generateContent HTTP request must abort within <= 2s
    // (server sleeps 5s before responding).
    let token = CancellationToken::new();
    let cancel_token = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        cancel_token.cancel();
    });

    let req = GeminiTranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        input_media: media.clone(),
        temp_dir: temp_dir.clone(),
        output_dir: out_dir.clone(),
        base_filename: "hang_cancel".to_string(),
        transcripts_subfolder: false,
        api_key: "test-fake-key-hang".to_string(),
        model: "gemini-flash-latest".to_string(),
        target_language: None,
        base_url: Some(server.base_url()),
        poll_interval: Some(Duration::from_millis(20)),
        duration_sec: None,
    };

    let start = tokio::time::Instant::now();
    let res = run_gemini_transcription(req, token, None).await;
    let elapsed = start.elapsed();

    assert!(res.is_err(), "cancelled transcription must return Err");
    assert!(
        elapsed < Duration::from_secs(2),
        "in-flight HTTP request must abort within <= 2s on cancel, took {:?}",
        elapsed
    );

    // Part 2: Cancelling summary generation in the queue engine must not be swallowed as a warning
    // that marks the item Done (e.g. when shutdown() cancels running tokens during summary).
    let bin_dir = fix.dir.path().join("bin");
    let fake_whisper = bin_dir.join(if cfg!(windows) {
        "sdp-whisper.exe"
    } else {
        "sdp-whisper"
    });
    fs::copy(&fix.fake_ffmpeg, &fake_whisper).expect("copy sdp-whisper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&fake_whisper).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fake_whisper, perms).expect("chmod +x");
    }
    fs::write(bin_dir.join("fake_whisper_sleep.txt"), "150").expect("write whisper sleep");

    let models_dir = fix.dir.path().join("models");
    fs::create_dir_all(&models_dir).expect("models dir");
    fs::write(
        models_dir.join("ggml-large-v3-turbo-q5_0.bin"),
        b"fake model",
    )
    .expect("write model");

    let config_dir = fix.dir.path().join("config");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::write(
        config_dir.join("gemini_api_key"),
        "fake-gemini-key-for-summary",
    )
    .expect("write key");

    let queue_path = fix.dir.path().join("queue_cancel_summary.json");
    let tools = Arc::new(QueueFakeTools {
        whisper: Some(fake_whisper),
        ffmpeg: fix.fake_ffmpeg.clone(),
        models_dir,
        config_dir: Some(config_dir),
        default_model: "large-v3-turbo-q5_0".to_string(),
        private_temp: None,
    });

    let config = EngineConfig {
        path: queue_path,
        provider: tools,
        sink: Arc::new(|_update| {}),
        lanes: scaricatore_di_porto_lib::services::queue::model::LaneConfig::default(),
        notify_on_idle: false,
        notifier: Some(Arc::new(DummyNotifier)),
    };

    let engine = QueueEngine::open(config);
    engine.start();

    let add_req = AddRequest {
        url: format!("local://{}", media.display()),
        title: Some("Cancel During Summary".to_string()),
        site: Some("local".to_string()),
        duration_sec: Some(10),
        options: ItemOptions {
            kind: Kind::Video,
            quality: QualityChoice::Best,
            output_dir: out_dir.clone(),
            cookies: CookieChoice::None,
            runtime_mode: JsRuntimeMode::QuickJsOnly,
            transcribe: true,
            transcription_engine: Some("whisper_only".to_string()),
            summarize: Some(true),
            summary_language: None,
            // Pass a model name with an embedded space so the local URL fails immediately without hitting the network.
            gemini_model: Some("invalid model for offline test".to_string()),
            ..ItemOptions::default()
        },
        group_id: None,
        file: Some(media),
    };

    let outcome = engine.add(vec![add_req], true);
    let item_id = outcome.added[0].clone();

    // Wait until Whisper finishes and item enters "Generating summary...", then call shutdown()
    // which cancels running tokens without setting item.control = Cancel.
    // Without the fix in engine.rs:2167, Err(e) during summary is caught as a warning and
    // run_transcribe_stage returns Ok, turning the cancelled item into Done!
    let mut shutdown_sent = false;
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(5)).await;
        let snapshot = engine.snapshot();
        if let Some(item) = snapshot.items.iter().find(|i| i.id == item_id) {
            if item.progress.percent == Some(95.0) {
                assert_eq!(
                    item.transcript_preview.as_deref(),
                    Some("queue.status.generatingSummary"),
                    "transcript_preview during summary must use i18n key queue.status.generatingSummary rather than hard-coded English"
                );
                engine.shutdown();
                shutdown_sent = true;
                break;
            }
        }
    }
    if !shutdown_sent {
        let snapshot = engine.snapshot();
        let item = snapshot.items.iter().find(|i| i.id == item_id);
        panic!(
            "item did not reach Generating summary... stage, got: {:?}",
            item
        );
    }

    let mut final_state = None;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let snapshot = engine.snapshot();
        if let Some(item) = snapshot.items.iter().find(|i| i.id == item_id) {
            if matches!(
                item.state,
                ItemState::Cancelled | ItemState::Done | ItemState::Failed
            ) {
                final_state = Some(item.state);
                break;
            }
        }
    }

    assert_eq!(
        final_state,
        Some(ItemState::Cancelled),
        "cancelling during summary generation must transition item to Cancelled, not Done"
    );
}

#[tokio::test]
async fn gemini_audio_prep_falls_back_to_flac_when_opus_and_mp3_fail() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample.mp4");
    let temp_dir = fix.dir.path().join("tmp_prep_flac");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    // Make fake ffmpeg fail for libopus and libmp3lame
    let bin_dir = fix.fake_ffmpeg.parent().unwrap();
    fs::write(bin_dir.join("fail_encode.txt"), "libopus,libmp3lame")
        .expect("write fail_encode.txt");

    let token = CancellationToken::new();
    let res = prepare_audio_for_gemini(&fix.fake_ffmpeg, &media, &temp_dir, token).await;

    let (audio_file, mime) = res.expect("audio prep should succeed with flac fallback");
    assert_eq!(mime, "audio/flac", "MIME type for FLAC must be audio/flac");
    assert_eq!(
        audio_file.file_name().and_then(|n| n.to_str()),
        Some("gemini_audio.flac"),
        "audio file must be gemini_audio.flac"
    );
    assert!(audio_file.is_file(), "flac output file must exist on disk");
}

#[tokio::test]
async fn gemini_audio_prep_checks_disk_space_and_fails_with_disk_full() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample.mp4");
    let temp_dir = fix.dir.path().join("tmp_prep_disk");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    // Configure fake ffprobe to report a massive duration (e.g. 100,000,000s -> ~4 TB needed)
    let bin_dir = fix.fake_ffmpeg.parent().unwrap();
    fs::write(bin_dir.join("fake_ffprobe_duration.txt"), "100000000.0")
        .expect("write fake_ffprobe_duration");

    let token = CancellationToken::new();
    let res = prepare_audio_for_gemini(&fix.fake_ffmpeg, &media, &temp_dir, token).await;

    match res {
        Err(AppError::DiskFull { needed, available }) => {
            assert!(
                needed > available,
                "needed space ({needed}) must exceed available space ({available})"
            );
        }
        other => panic!("expected AppError::DiskFull, got: {:?}", other),
    }
}

#[tokio::test]
async fn gemini_audio_prep_rejects_audio_exceeding_gemini_duration_limit() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample_oversized.mp4");
    let temp_dir = fix.dir.path().join("tmp_prep_oversized");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    // Configure fake ffprobe to report 35,000s (> 9.5 hours = 34,200s)
    let bin_dir = fix.fake_ffmpeg.parent().unwrap();
    fs::write(bin_dir.join("fake_ffprobe_duration.txt"), "35000.0")
        .expect("write fake_ffprobe_duration");

    let token = CancellationToken::new();
    let res = prepare_audio_for_gemini(&fix.fake_ffmpeg, &media, &temp_dir, token).await;

    match res {
        Err(AppError::Process(msg)) => {
            assert!(
                msg.contains("9.5-hour limit") || msg.contains("exceeds"),
                "expected duration limit error message, got: {msg}"
            );
        }
        other => panic!(
            "expected AppError::Process rejecting audio > 9.5 hours, got: {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn gemini_handles_max_tokens_truncation_and_long_audio_warning() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample_trunc.mp4");
    let server = MockGeminiServer::start(MockBehavior::TruncatedMaxTokens);

    let output_dir = fix.dir.path().join("out_trunc");
    fs::create_dir_all(&output_dir).expect("out dir");
    let temp_dir = fix.dir.path().join("tmp_trunc");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    let req = GeminiTranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        input_media: media,
        temp_dir: temp_dir.clone(),
        output_dir: output_dir.clone(),
        base_filename: "sample_trunc".to_string(),
        transcripts_subfolder: false,
        api_key: "test-fake-key-123".to_string(),
        model: "gemini-flash-latest".to_string(),
        target_language: None,
        base_url: Some(server.base_url()),
        poll_interval: Some(Duration::from_millis(50)),
        duration_sec: Some(4000.0), // > 60 minutes (3600s)
    };

    let token = CancellationToken::new();
    let res = run_gemini_transcription(req, token, None).await;
    assert!(res.is_ok(), "transcription should succeed: {:?}", res.err());

    let outcome: GeminiTranscribeOutcome = res.unwrap();
    assert!(
        outcome.truncated,
        "outcome must be marked truncated when finishReason is MAX_TOKENS"
    );
    assert_eq!(outcome.finish_reason.as_deref(), Some("MAX_TOKENS"));
    assert!(
        outcome.duration_warning,
        "duration_warning must be true for audio > 60 minutes"
    );
    assert!(outcome.txt_path.is_file(), "output .txt file must exist");
}

#[tokio::test]
async fn gemini_poll_failed_and_timeout_trigger_raii_delete_cleanup() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample_poll_fail.mp4");

    // 1. Remote file state == "FAILED" during polling
    let server_failed = MockGeminiServer::start(MockBehavior::PollFailed);
    let out_dir = fix.dir.path().join("out_poll_fail");
    fs::create_dir_all(&out_dir).expect("out dir");
    let temp_dir = fix.dir.path().join("tmp_poll_fail");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    let req = GeminiTranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        input_media: media.clone(),
        temp_dir: temp_dir.clone(),
        output_dir: out_dir,
        base_filename: "sample_poll_fail".to_string(),
        transcripts_subfolder: false,
        api_key: "test-fake-key-poll-fail".to_string(),
        model: "gemini-flash-latest".to_string(),
        target_language: None,
        base_url: Some(server_failed.base_url()),
        poll_interval: Some(Duration::from_millis(20)),
        duration_sec: None,
    };

    let token = CancellationToken::new();
    let res = run_gemini_transcription(req, token.clone(), None).await;
    match res {
        Err(AppError::Process(msg)) => {
            assert!(
                msg.contains("Gemini cloud audio processing failed"),
                "expected FAILED polling error message, got: {msg}"
            );
        }
        other => panic!(
            "expected AppError::Process on FAILED poll state, got: {:?}",
            other
        ),
    }

    assert!(
        server_failed.wait_for_delete(Duration::from_secs(1)).await,
        "DELETE must be called by RAII guard when poll_file_active sees FAILED state"
    );
    let temp_entries: Vec<_> = fs::read_dir(&temp_dir).unwrap().collect();
    assert!(
        temp_entries.is_empty(),
        "temp audio file must be deleted synchronously on FAILED poll state"
    );

    // 2. Polling timeout while state stays "PROCESSING" still triggers RAII guard DELETE
    let server_timeout = MockGeminiServer::start(MockBehavior::PollStuckProcessing);
    let temp_timeout_dir = fix.dir.path().join("tmp_poll_timeout");
    fs::create_dir_all(&temp_timeout_dir).expect("tmp timeout dir");
    let temp_audio = temp_timeout_dir.join("gemini_audio.ogg");
    fs::write(&temp_audio, b"OggS...fake...").expect("write temp audio");

    let client = reqwest::Client::new();
    let base_url = server_timeout.base_url();
    let api_key = "test-fake-key-poll-timeout";
    let poll_res = {
        let mut guard = GeminiFileGuard::new(client.clone(), &base_url, api_key);
        guard.set_temp_audio_file(temp_audio.clone());
        let uploaded = upload_file_resumable(
            &client,
            &base_url,
            api_key,
            &temp_audio,
            "audio/ogg",
            &token,
        )
        .await
        .expect("upload before poll timeout");
        guard.set_remote_file(uploaded.name.clone());

        poll_file_active(
            &client,
            &base_url,
            api_key,
            &uploaded.name,
            Duration::from_millis(25),
            Duration::from_millis(100),
            &token,
        )
        .await
    };

    match poll_res {
        Err(AppError::Process(msg)) => {
            assert!(
                msg.contains("timed out"),
                "expected polling timeout error, got: {msg}"
            );
        }
        other => panic!("expected polling timeout error, got: {:?}", other),
    }

    assert!(
        server_timeout.wait_for_delete(Duration::from_secs(1)).await,
        "DELETE must be called by RAII guard when poll_file_active times out"
    );
    assert!(
        !temp_audio.exists(),
        "temp audio file must be deleted when RAII guard drops on poll timeout"
    );
}

#[tokio::test]
async fn gemini_handles_block_reason_and_max_tokens_truncation_in_queue() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample_blocked.mp4");

    // 1. promptFeedback.blockReason -> AppError::GeminiBlocked("SAFETY") + RAII DELETE
    let server_blocked = MockGeminiServer::start(MockBehavior::BlockedPromptFeedback);
    let out_dir = fix.dir.path().join("out_blocked");
    fs::create_dir_all(&out_dir).expect("out dir");
    let temp_dir = fix.dir.path().join("tmp_blocked");
    fs::create_dir_all(&temp_dir).expect("tmp dir");

    let req = GeminiTranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        input_media: media.clone(),
        temp_dir: temp_dir.clone(),
        output_dir: out_dir.clone(),
        base_filename: "sample_blocked".to_string(),
        transcripts_subfolder: false,
        api_key: "test-fake-key-blocked".to_string(),
        model: "gemini-flash-latest".to_string(),
        target_language: None,
        base_url: Some(server_blocked.base_url()),
        poll_interval: Some(Duration::from_millis(20)),
        duration_sec: None,
    };

    let token = CancellationToken::new();
    let res = run_gemini_transcription(req, token, None).await;
    match res {
        Err(AppError::GeminiBlocked(reason)) => {
            assert_eq!(reason, "SAFETY");
        }
        other => panic!(
            "expected AppError::GeminiBlocked(\"SAFETY\"), got: {:?}",
            other
        ),
    }
    assert!(
        server_blocked.wait_for_delete(Duration::from_secs(3)).await,
        "DELETE must be called by RAII guard when promptFeedback.blockReason blocks generation"
    );

    // 2. MAX_TOKENS truncation surfaces user-visible warning in queue item log
    let server_trunc = MockGeminiServer::start(MockBehavior::TruncatedMaxTokens);
    let config_dir = fix.dir.path().join("config_trunc");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::write(config_dir.join("gemini_api_key"), "test-fake-key-trunc").expect("write key");
    fs::write(config_dir.join("gemini_base_url"), server_trunc.base_url()).expect("write base_url");

    let queue_path = fix.dir.path().join("queue_trunc.json");
    let tools = Arc::new(QueueFakeTools {
        whisper: None,
        ffmpeg: fix.fake_ffmpeg.clone(),
        models_dir: fix.dir.path().join("models_trunc"),
        config_dir: Some(config_dir),
        default_model: "large-v3-turbo-q5_0".to_string(),
        private_temp: None,
    });

    let engine = QueueEngine::open(EngineConfig {
        path: queue_path,
        provider: tools,
        sink: Arc::new(|_update| {}),
        lanes: scaricatore_di_porto_lib::services::queue::model::LaneConfig::default(),
        notify_on_idle: false,
        notifier: Some(Arc::new(DummyNotifier)),
    });
    engine.start();

    let add_req = AddRequest {
        url: format!("local://{}", media.display()),
        title: Some("Truncated Gemini".to_string()),
        site: Some("local".to_string()),
        duration_sec: Some(4000),
        options: ItemOptions {
            kind: Kind::Video,
            quality: QualityChoice::Best,
            output_dir: out_dir.clone(),
            cookies: CookieChoice::None,
            runtime_mode: JsRuntimeMode::QuickJsOnly,
            transcribe: true,
            transcription_engine: Some("gemini_only".to_string()),
            ..ItemOptions::default()
        },
        group_id: None,
        file: Some(media),
    };

    let item_id = engine.add(vec![add_req], true).added[0].clone();
    let mut final_item = None;
    for _ in 0..80 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let snap = engine.snapshot();
        if let Some(it) = snap.items.iter().find(|i| i.id == item_id) {
            if matches!(it.state, ItemState::Done | ItemState::Failed) {
                final_item = Some(it.clone());
                break;
            }
        }
    }

    let item = final_item.expect("queue item should finish");
    assert_eq!(item.state, ItemState::Done, "item failed: {:?}", item.error);
    assert_eq!(item.transcript_origin.as_deref(), Some("gemini"));
    assert!(
        item.log_tail
            .iter()
            .any(|l| l.contains("truncated by the model output limit")),
        "queue item log_tail must contain truncation warning, got: {:?}",
        item.log_tail
    );
}

#[tokio::test]
async fn queue_engine_falls_back_subtitles_to_whisper_to_gemini() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("fallback_video.mp4");
    let server = MockGeminiServer::start(MockBehavior::Normal);

    let bin_dir = fix.dir.path().join("bin");
    let fake_ytdlp = bin_dir.join(if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    });
    fs::copy(fake_tool_source(), &fake_ytdlp).expect("copy fake_tool as yt-dlp");
    let fake_whisper = bin_dir.join(if cfg!(windows) {
        "sdp-whisper.exe"
    } else {
        "sdp-whisper"
    });
    fs::copy(fake_tool_source(), &fake_whisper).expect("copy fake_tool as sdp-whisper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [&fake_ytdlp, &fake_whisper] {
            let mut perms = fs::metadata(p).expect("metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(p, perms).expect("chmod +x");
        }
    }

    // Force Whisper to fail so the engine must fall back from Whisper to Gemini
    fs::write(bin_dir.join("fail_whisper_all.txt"), "1").expect("write fail_whisper_all");

    let models_dir = fix.dir.path().join("models_fb");
    fs::create_dir_all(&models_dir).expect("models dir");
    fs::write(
        models_dir.join("ggml-large-v3-turbo-q5_0.bin"),
        b"fake model",
    )
    .expect("write model");

    let config_dir = fix.dir.path().join("config_fb");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::write(config_dir.join("gemini_api_key"), "test-fake-key-fallback").expect("write key");
    fs::write(config_dir.join("gemini_base_url"), server.base_url()).expect("write base_url");

    let out_dir = fix.dir.path().join("out_fb");
    fs::create_dir_all(&out_dir).expect("out dir");

    struct FallbackFakeTools {
        ytdlp: PathBuf,
        whisper: PathBuf,
        ffmpeg: PathBuf,
        models_dir: PathBuf,
        config_dir: PathBuf,
    }

    impl ToolProvider for FallbackFakeTools {
        fn environment(
            &self,
            options: ItemOptions,
        ) -> BoxFuture<Result<JobEnvironment, ErrorInfo>> {
            fs::create_dir_all(&options.output_dir).ok();
            let env = JobEnvironment {
                ytdlp: self.ytdlp.clone(),
                ffmpeg: Some(self.ffmpeg.clone()),
                ffprobe: scaricatore_di_porto_lib::services::ffmpeg::find_ffprobe(
                    &self.ffmpeg,
                    None,
                ),
                whisper: Some(self.whisper.clone()),
                models_dir: Some(self.models_dir.clone()),
                config_dir: Some(self.config_dir.clone()),
                whisper_threads: 2,
                skip_silence_vad: false,
                default_whisper_model: Some("large-v3-turbo-q5_0".to_string()),
                default_transcript_language: None,
                default_transcription_engine_order: None,
                default_gemini_model: None,
                default_summarize: false,
                default_summary_language: None,
                runtimes: scaricatore_di_porto_lib::services::inspect::Runtimes {
                    mode: JsRuntimeMode::QuickJsOnly,
                    qjs: None,
                    deno: None,
                    ytdlp_version: "2026.01.01".to_string(),
                },
                memory: Arc::new(RuntimeMemoryStore::load(self.models_dir.join("mem.json"))),
                private_temp: None,
                youtube_clients: None,
            };
            Box::pin(std::future::ready(Ok(env)))
        }
    }

    let engine = QueueEngine::open(EngineConfig {
        path: fix.dir.path().join("queue_fb.json"),
        provider: Arc::new(FallbackFakeTools {
            ytdlp: fake_ytdlp,
            whisper: fake_whisper,
            ffmpeg: fix.fake_ffmpeg.clone(),
            models_dir,
            config_dir,
        }),
        sink: Arc::new(|_update| {}),
        lanes: scaricatore_di_porto_lib::services::queue::model::LaneConfig::default(),
        notify_on_idle: false,
        notifier: Some(Arc::new(DummyNotifier)),
    });
    engine.start();

    let add_req = AddRequest {
        url: "https://www.youtube.com/watch?v=fallback123".to_string(),
        title: Some("Fallback Test Video".to_string()),
        site: Some("youtube".to_string()),
        duration_sec: Some(30),
        options: ItemOptions {
            kind: Kind::Video,
            quality: QualityChoice::Best,
            output_dir: out_dir.clone(),
            cookies: CookieChoice::None,
            runtime_mode: JsRuntimeMode::QuickJsOnly,
            transcribe: true,
            transcription_engine: Some("subtitles_whisper_gemini".to_string()),
            ..ItemOptions::default()
        },
        group_id: None,
        file: Some(media),
    };

    let item_id = engine.add(vec![add_req], true).added[0].clone();
    let mut final_item = None;
    for _ in 0..80 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let snap = engine.snapshot();
        if let Some(it) = snap.items.iter().find(|i| i.id == item_id) {
            if matches!(it.state, ItemState::Done | ItemState::Failed) {
                final_item = Some(it.clone());
                break;
            }
        }
    }

    let item = final_item.expect("fallback item should finish");
    assert_eq!(
        item.state,
        ItemState::Done,
        "Subtitles -> Whisper -> Gemini fallback should succeed via Gemini, got error: {:?}",
        item.error
    );

    // 1. Subtitles were attempted first via yt-dlp --skip-download (manual + auto = 2 calls)
    let ytdlp_calls: u32 = fs::read_to_string(bin_dir.join("fake-ytdlp.calls"))
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0);
    assert!(
        ytdlp_calls >= 1,
        "Subtitles step must have invoked yt-dlp before falling back to Whisper"
    );

    // 2. Whisper was attempted second (with CPU retry when first attempt fails)
    let whisper_calls: u32 = fs::read_to_string(bin_dir.join("fake-whisper.calls"))
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0);
    assert!(
        whisper_calls >= 1,
        "Whisper step must have been attempted before falling back to Gemini"
    );
    assert!(
        item.log_tail
            .iter()
            .any(|l| l.contains("Whisper transcription failed")),
        "item log_tail should record Whisper failure before falling back to Gemini, got: {:?}",
        item.log_tail
    );

    // 3. Gemini succeeded as the final fallback
    assert_eq!(item.transcript_origin.as_deref(), Some("gemini"));
    assert_eq!(item.transcript_files.len(), 1);
    assert!(item.transcript_files[0].is_file());
    let txt = fs::read_to_string(&item.transcript_files[0]).expect("read gemini transcript");
    assert!(txt.contains("This is the transcribed text from mock Gemini API."));
}
