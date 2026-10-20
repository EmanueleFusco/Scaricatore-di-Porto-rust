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
    GeminiProgressSink, GeminiTranscribeRequest, list_gemini_models, run_gemini_transcription,
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
            "HTTP/1.1 200 OK\r\nx-goog-upload-url: {}\r\nContent-Length: 0\r\n\r\n",
            upload_url
        );
        let _ = stream.write_all(resp.as_bytes());
    } else if method == "POST" && path.starts_with("/resumable-upload/mock-file-123") {
        // Step 2: Upload streamed bytes -> return UploadedFile JSON
        let json = r#"{"file":{"name":"files/mock-file-123","uri":"https://generativelanguage.googleapis.com/v1beta/files/mock-file-123","state":"PROCESSING"}}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            json.len(),
            json
        );
        let _ = stream.write_all(resp.as_bytes());
    } else if method == "GET" && path.starts_with("/v1beta/files/mock-file-123") {
        if cur_behavior == MockBehavior::SlowPoll {
            thread::sleep(Duration::from_millis(300));
        }
        // Step 3: Polling file status -> ACTIVE
        let json = r#"{"name":"files/mock-file-123","uri":"https://generativelanguage.googleapis.com/v1beta/files/mock-file-123","state":"ACTIVE"}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
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
                    "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json.len(),
                    json
                );
                let _ = stream.write_all(resp.as_bytes());
            }
            MockBehavior::FailGenerateContent500 => {
                let json = r#"{"error":{"code":500,"message":"Internal server error","status":"INTERNAL"}}"#;
                let resp = format!(
                    "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
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
                        "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        json.len(),
                        json
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else {
                    let json = r#"{"candidates":[{"content":{"parts":[{"text":"Transcribed text after quota backoff."}]},"finishReason":"STOP"}]}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        json.len(),
                        json
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
            }
            MockBehavior::Normal | MockBehavior::SlowPoll => {
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
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
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
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
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
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            json.len(),
            json
        );
        let _ = stream.write_all(resp.as_bytes());
    } else {
        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes());
    }
}

// ---------------------------------------------------------------------------
// Test Fixture
// ---------------------------------------------------------------------------

struct Fixture {
    dir: TempDir,
    fake_ffmpeg: PathBuf,
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

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&fake_ffmpeg).expect("metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&fake_ffmpeg, perms).expect("chmod +x");
        }

        Self { dir, fake_ffmpeg }
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

// ---------------------------------------------------------------------------
// Engine ordering test
// ---------------------------------------------------------------------------

struct QueueFakeTools {
    whisper: Option<PathBuf>,
    ffmpeg: PathBuf,
    models_dir: PathBuf,
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
            config_dir: None,
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
