//! Integration tests for the Whisper model catalogue and download manager
//! (spec §10.2, §11.6), against a local HTTP server.
//!
//! Verifies:
//! - Pinned catalogue parsing and model entries
//! - Range-resumed download after a truncated connection
//! - Rejection of corrupted files with sha256 mismatch
//! - Deletion and local file import

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use scaricatore_di_porto_lib::services::download::Transport;
use scaricatore_di_porto_lib::services::models::{
    self, ModelCatalogue, ModelDownloadManager, ModelEntry, VadModelEntry, delete_model,
    import_model, inspect_models,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerMode {
    Whole,
    Truncated { cut: usize, times: usize },
}

struct TestServer {
    port: u16,
    ranges_seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn start(body: Vec<u8>, mode: ServerMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let body = Arc::new(body);
        let requests = Arc::new(AtomicUsize::new(0));
        let ranges_seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let thread_body = Arc::clone(&body);
        let thread_requests = Arc::clone(&requests);
        let thread_ranges = Arc::clone(&ranges_seen);
        let thread_stop = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            for stream in listener.incoming() {
                if thread_stop.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                serve_request(stream, &thread_body, mode, &thread_requests, &thread_ranges);
            }
        });

        Self {
            port,
            ranges_seen,
            stop,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve_request(
    mut stream: TcpStream,
    body: &[u8],
    mode: ServerMode,
    requests: &AtomicUsize,
    ranges: &Mutex<Vec<String>>,
) {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("read timeout");
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut range = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Err(_) | Ok(0) => break,
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("range:") {
            range = Some(value.trim().to_owned());
        }
    }

    let mut discard = [0_u8; 1024];
    while matches!(reader.get_mut().read(&mut discard), Ok(read) if read > 0) {}

    let offset: usize = range
        .as_deref()
        .and_then(|value| value.strip_prefix("bytes="))
        .and_then(|value| value.split('-').next())
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);

    ranges
        .lock()
        .expect("ranges")
        .push(range.clone().unwrap_or_default());

    let _ = requests.fetch_add(1, Ordering::Relaxed);
    let ordinal = requests.load(Ordering::Relaxed);

    let (status, slice): (&str, &[u8]) = if offset > 0 {
        ("206 Partial Content", &body[offset.min(body.len())..])
    } else {
        ("200 OK", body)
    };

    let header = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Length: {}\r\n\
         Accept-Ranges: bytes\r\n\
         Connection: close\r\n\r\n",
        slice.len()
    );
    if stream.write_all(header.as_bytes()).is_err() {
        return;
    }
    let _ = stream.flush();

    let cut_now = match mode {
        ServerMode::Truncated { cut, times } => ordinal <= times && slice.len() > cut,
        _ => false,
    };
    if cut_now {
        let cut = match mode {
            ServerMode::Truncated { cut, .. } => cut.min(slice.len()),
            _ => 0,
        };
        let _ = stream.write_all(&slice[..cut]);
        let _ = stream.flush();
        let _ = stream.shutdown(std::net::Shutdown::Write);
    } else {
        let _ = stream.write_all(slice);
        let _ = stream.flush();
        let _ = stream.shutdown(std::net::Shutdown::Write);
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn embedded_model_catalogue_is_valid() {
    let catalogue = ModelCatalogue::embedded().expect("catalogue must parse");
    assert_eq!(catalogue.schema_version, 1);
    assert_eq!(catalogue.vad.id, "silero-v5.1.2");
    assert_eq!(catalogue.models.len(), 11);

    for m in &catalogue.models {
        assert!(!m.id.is_empty());
        assert!(!m.label.is_empty());
        assert!(m.file.ends_with(".bin"));
        assert_eq!(m.sha256.len(), 64);
        assert!(m.size_bytes > 0);
    }
}

#[tokio::test]
async fn download_manager_resumes_after_dropped_connection() {
    let dir = TempDir::new().expect("temp dir");
    let models_dir = dir.path().join("models");

    // Sample payload representing VAD and Model
    let vad_bytes = vec![0x11; 1024];
    let vad_sha = sha256_hex(&vad_bytes);

    let model_bytes = vec![0x22; 8192];
    let model_sha = sha256_hex(&model_bytes);

    // Create a mock catalogue pointing to local server filenames
    let custom_catalogue = ModelCatalogue {
        schema_version: 1,
        vad: VadModelEntry {
            id: "silero-v5.1.2".to_owned(),
            label: "Silero VAD".to_owned(),
            file: "ggml-silero.bin".to_owned(),
            url: "".to_owned(),
            size_bytes: vad_bytes.len() as u64,
            sha256: vad_sha,
        },
        models: vec![ModelEntry {
            id: "test-model".to_owned(),
            label: "Test Model".to_owned(),
            file: "ggml-test.bin".to_owned(),
            url: "".to_owned(),
            size_bytes: model_bytes.len() as u64,
            sha256: model_sha,
            ram_estimate: "~100 MB".to_owned(),
            tier: "q5_0".to_owned(),
            indicative_wer: Some(3.0),
            multilingual: true,
            notes: None,
            recommended: false,
            advanced: false,
        }],
    };

    // Pre-install VAD so we only test model resumption
    fs::create_dir_all(&models_dir).unwrap();
    fs::write(models_dir.join("ggml-silero.bin"), &vad_bytes).unwrap();

    // Start server that drops connection repeatedly (exhausting in-call retries)
    let cut = 1000;
    let broken_server = TestServer::start(
        model_bytes.clone(),
        ServerMode::Truncated { cut, times: 99 },
    );
    let broken_mirror = broken_server.base_url();

    let manager = ModelDownloadManager::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);

    // First attempt fails after transient retries give up
    let res = manager
        .download_model(
            &models_dir,
            &custom_catalogue,
            "test-model",
            Some(&broken_mirror),
            Transport::loopback_http(),
            move |ev| {
                events_clone.lock().unwrap().push(ev);
            },
        )
        .await;

    assert!(
        res.is_err(),
        "first download attempt must fail on truncated stream"
    );
    // Verify .part file survives so next attempt can resume
    let part_path = models_dir.join("ggml-test.bin.part");
    assert!(
        part_path.is_file(),
        "the .part file must survive a network drop"
    );
    let initial_part_len = fs::metadata(&part_path).unwrap().len();
    assert!(initial_part_len > 0, "part file must contain partial bytes");
    drop(broken_server);

    // Second attempt resumes with a healthy server and completes successfully
    let healthy_server = TestServer::start(model_bytes.clone(), ServerMode::Whole);
    let healthy_mirror = healthy_server.base_url();

    let events_second = Arc::new(Mutex::new(Vec::new()));
    let events_second_clone = Arc::clone(&events_second);

    let final_path = manager
        .download_model(
            &models_dir,
            &custom_catalogue,
            "test-model",
            Some(&healthy_mirror),
            Transport::loopback_http(),
            move |ev| {
                events_second_clone.lock().unwrap().push(ev);
            },
        )
        .await
        .expect("resumed download must succeed");

    assert_eq!(final_path, models_dir.join("ggml-test.bin"));
    assert!(!part_path.exists(), "temporary .part must be removed");
    assert_eq!(fs::read(&final_path).unwrap(), model_bytes);

    // Verify ranges requested: second attempt must have asked for Range: bytes={initial_part_len}-
    let ranges = healthy_server.ranges_seen.lock().unwrap();
    assert!(
        ranges
            .iter()
            .any(|r| r.starts_with(&format!("bytes={initial_part_len}-"))),
        "second attempt must request Range: bytes={initial_part_len}-, seen: {:?}",
        *ranges
    );
}

#[tokio::test]
async fn download_manager_rejects_corrupted_model_checksum() {
    let dir = TempDir::new().expect("temp dir");
    let models_dir = dir.path().join("models");

    let bytes = vec![0x33; 2048];
    // Intentionally wrong expected sha
    let wrong_sha = "0000000000000000000000000000000000000000000000000000000000000000";

    let custom_catalogue = ModelCatalogue {
        schema_version: 1,
        vad: VadModelEntry {
            id: "silero-v5.1.2".to_owned(),
            label: "Silero VAD".to_owned(),
            file: "ggml-silero.bin".to_owned(),
            url: "".to_owned(),
            size_bytes: 10,
            sha256: "dummy".to_owned(),
        },
        models: vec![ModelEntry {
            id: "corrupt-model".to_owned(),
            label: "Corrupt Model".to_owned(),
            file: "ggml-corrupt.bin".to_owned(),
            url: "".to_owned(),
            size_bytes: bytes.len() as u64,
            sha256: wrong_sha.to_owned(),
            ram_estimate: "~100 MB".to_owned(),
            tier: "q5_0".to_owned(),
            indicative_wer: None,
            multilingual: true,
            notes: None,
            recommended: false,
            advanced: false,
        }],
    };

    // Pre-install VAD to isolate test to model download
    fs::create_dir_all(&models_dir).unwrap();
    fs::write(models_dir.join("ggml-silero.bin"), b"vad-content").unwrap();

    let server = TestServer::start(bytes, ServerMode::Whole);
    let mirror_url = server.base_url();

    let manager = ModelDownloadManager::new();
    let res = manager
        .download_model(
            &models_dir,
            &custom_catalogue,
            "corrupt-model",
            Some(&mirror_url),
            Transport::loopback_http(),
            |_| {},
        )
        .await;

    assert!(res.is_err());
    // Dest file and .part file must NOT remain on disk when checksum fails
    assert!(!models_dir.join("ggml-corrupt.bin").exists());
    assert!(!models_dir.join("ggml-corrupt.bin.part").exists());
}

#[test]
fn model_deletion_and_local_import_flow() {
    let dir = TempDir::new().expect("temp dir");
    let models_dir = dir.path().join("models");
    let source_dir = dir.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();

    let file_content = b"whisper-local-import-content";
    let sha = sha256_hex(file_content);

    let local_file = source_dir.join("downloaded-manually.bin");
    fs::write(&local_file, file_content).unwrap();

    let catalogue = ModelCatalogue {
        schema_version: 1,
        vad: VadModelEntry {
            id: "silero-v5.1.2".to_owned(),
            label: "Silero".to_owned(),
            file: "ggml-silero.bin".to_owned(),
            url: "".to_owned(),
            size_bytes: 10,
            sha256: "dummy".to_owned(),
        },
        models: vec![ModelEntry {
            id: "imported-model".to_owned(),
            label: "Imported Model".to_owned(),
            file: "ggml-imported.bin".to_owned(),
            url: "".to_owned(),
            size_bytes: file_content.len() as u64,
            sha256: sha,
            ram_estimate: "~100 MB".to_owned(),
            tier: "q5_0".to_owned(),
            indicative_wer: None,
            multilingual: true,
            notes: None,
            recommended: false,
            advanced: false,
        }],
    };

    // Import matching by sha256
    let imported =
        import_model(&models_dir, &catalogue, &local_file, None).expect("import must succeed");
    assert_eq!(imported.id, "imported-model");
    let installed_file = models_dir.join("ggml-imported.bin");
    assert!(installed_file.is_file());

    // Inspect models view reflects installed status
    let view = inspect_models(
        &models_dir,
        &catalogue,
        Some("imported-model"),
        &std::collections::HashMap::new(),
    );
    assert_eq!(view.active_model.as_deref(), Some("imported-model"));
    let item = view
        .models
        .iter()
        .find(|m| m.id == "imported-model")
        .unwrap();
    match item.state {
        models::ModelInstallState::Installed { size_bytes } => {
            assert_eq!(size_bytes, file_content.len() as u64);
        }
        _ => panic!("model must be marked as installed"),
    }

    // Delete model
    delete_model(&models_dir, &catalogue, "imported-model").expect("delete must succeed");
    assert!(!installed_file.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn models_delete_cancels_in_flight_download_and_prevents_recreating_file() {
    let dir = TempDir::new().expect("temp dir");
    let state =
        scaricatore_di_porto_lib::state::AppState::from_path(dir.path().join("settings.json"))
            .expect("state");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            scaricatore_di_porto_lib::commands::models::models_delete,
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app must build");
    tauri::Manager::manage(&app, state);

    let models_dir =
        tauri::Manager::state::<scaricatore_di_porto_lib::state::AppState>(&app).models_dir();
    let manager = Arc::clone(
        tauri::Manager::state::<scaricatore_di_porto_lib::state::AppState>(&app).models_manager(),
    );
    let catalogue = ModelCatalogue::embedded().expect("embedded catalogue");

    // Pre-create VAD model so download_model targets the Whisper model directly
    fs::create_dir_all(&models_dir).unwrap();
    fs::write(models_dir.join(&catalogue.vad.file), b"dummy-vad").unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("port").port();
    let (release_tx, release_rx) = std::sync::mpsc::channel();

    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            stream
                .set_read_timeout(Some(Duration::from_millis(200)))
                .expect("read timeout");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Err(_) | Ok(0) => break,
                    Ok(_) => {}
                }
                if line.trim().is_empty() {
                    break;
                }
            }
            let response = "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n";
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&[0x42; 100]).unwrap();
            stream.flush().unwrap();

            // Wait until test signals
            let _ = release_rx.recv();
            let _ = stream.write_all(&[0x42; 900]);
            let _ = stream.flush();
        }
    });

    let (chunk_tx, chunk_rx) = std::sync::mpsc::channel();
    let on_progress = move |p: models::ModelProgressEvent| {
        if p.received_bytes > 0 {
            let _ = chunk_tx.send(());
        }
    };

    let mirror_url = format!("http://127.0.0.1:{port}");
    let manager_clone = Arc::clone(&manager);
    let models_dir_clone = models_dir.clone();
    let catalogue_clone = catalogue.clone();

    let download_handle = tokio::spawn(async move {
        manager_clone
            .download_model(
                &models_dir_clone,
                &catalogue_clone,
                "tiny-q5_1",
                Some(&mirror_url),
                Transport::loopback_http(),
                on_progress,
            )
            .await
    });

    // Wait until download has received the first chunk
    if let Err(e) = chunk_rx.recv_timeout(Duration::from_secs(5)) {
        if download_handle.is_finished() {
            let res = tokio::time::timeout(Duration::from_millis(100), download_handle).await;
            panic!("download_handle finished early with: {:?}", res);
        }
        panic!("download must receive first chunk: {:?}", e);
    }

    assert!(manager.is_downloading("tiny-q5_1"));

    // Invoke models_delete while download is in flight
    let res = scaricatore_di_porto_lib::commands::models::models_delete(
        tauri::Manager::state::<scaricatore_di_porto_lib::state::AppState>(&app),
        "tiny-q5_1".to_string(),
    );
    assert!(res.is_ok(), "models_delete should succeed: {:?}", res.err());

    // models_delete must cancel in-flight download so it is no longer downloading
    assert!(
        !manager.is_downloading("tiny-q5_1"),
        "models_manager must not report model as downloading after models_delete"
    );

    // Unblock server
    let _ = release_tx.send(());
    let _ = server_handle.join();

    // The download task must abort due to cancellation
    let download_res = download_handle.await.expect("task join");
    assert!(
        download_res.is_err(),
        "download task must abort on cancellation, but got: {:?}",
        download_res
    );
    let err_str = download_res.unwrap_err().to_string();
    assert!(
        err_str.contains("the download was cancelled"),
        "expected cancellation error, got: {err_str}"
    );

    let model_file = models_dir.join("ggml-tiny-q5_1.bin");
    assert!(
        !model_file.exists(),
        "model file must not exist after cancellation and delete"
    );
}

#[tokio::test]
async fn download_model_accounts_for_existing_part_file_in_disk_space_check() {
    let dir = TempDir::new().expect("temp dir");
    let models_dir = dir.path().join("models");
    fs::create_dir_all(&models_dir).unwrap();

    let safety_margin = 10 * 1024 * 1024;
    let part_bytes = 200 * 1024 * 1024; // 200 MiB

    // Create and size the .part file FIRST so Windows NTFS deducts its space before we measure free_space!
    let part_path = models_dir.join("ggml-resumed.bin.part");
    let file = fs::File::create(&part_path).unwrap();
    file.set_len(part_bytes).unwrap();
    drop(file);

    let free_space = models::free_disk_space(&models_dir).unwrap_or(0);
    assert!(
        free_space > 250 * 1024 * 1024,
        "need at least 250 MB free space on test volume"
    );

    // Without subtracting .part:
    //   total_needed = model_size + safety_margin = free_space + 60 MiB > free_space (DiskFull).
    // With subtracting .part:
    //   total_needed = (model_size - 200 MiB) + 10 MiB = free_space - 140 MiB < free_space (140 MiB headroom for OS jitter).
    let model_size = free_space + safety_margin + 40 * 1024 * 1024;

    let custom_catalogue = ModelCatalogue {
        schema_version: 1,
        vad: VadModelEntry {
            id: "silero-v5.1.2".to_owned(),
            label: "Silero VAD".to_owned(),
            file: "ggml-silero.bin".to_owned(),
            url: "".to_owned(),
            size_bytes: 10,
            sha256: "dummy".to_owned(),
        },
        models: vec![ModelEntry {
            id: "resumed-model".to_owned(),
            label: "Resumed Model".to_owned(),
            file: "ggml-resumed.bin".to_owned(),
            url: "".to_owned(),
            size_bytes: model_size,
            sha256: "0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
            ram_estimate: "~100 MB".to_owned(),
            tier: "q5_0".to_owned(),
            indicative_wer: None,
            multilingual: true,
            notes: None,
            recommended: false,
            advanced: false,
        }],
    };

    // Mark VAD as already installed so vad_needed is 0
    fs::write(models_dir.join(&custom_catalogue.vad.file), b"vad").unwrap();

    let manager = ModelDownloadManager::new();
    let res = manager
        .download_model(
            &models_dir,
            &custom_catalogue,
            "resumed-model",
            Some("http://127.0.0.1:9"), // dummy mirror
            Transport::loopback_http(),
            |_| {},
        )
        .await;

    if let Err(scaricatore_di_porto_lib::error::AppError::DiskFull { needed, available }) = res {
        panic!(
            "disk check demanded full {needed} bytes despite {part_bytes} bytes in .part (available: {available})"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn models_download_throttles_rapid_chunk_progress_events() {
    let dir = TempDir::new().expect("temp dir");
    let state =
        scaricatore_di_porto_lib::state::AppState::from_path(dir.path().join("settings.json"))
            .expect("state");
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app");
    tauri::Manager::manage(&app, state);

    // Pre-create VAD file so models_download proceeds directly to downloading the model
    let catalogue = ModelCatalogue::embedded().unwrap();
    let models_dir =
        tauri::Manager::state::<scaricatore_di_porto_lib::state::AppState>(&app).models_dir();
    fs::create_dir_all(&models_dir).unwrap();
    fs::write(models_dir.join(&catalogue.vad.file), b"vad-dummy").unwrap();

    // Start a mock TCP server sending 25 rapid chunks of 100 bytes each
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    let server_handle = thread::spawn(move || {
        for stream in listener.incoming() {
            if stop_clone.load(Ordering::Relaxed) {
                break;
            }
            let Ok(mut stream) = stream else { break };
            let _ = stream.set_nodelay(true);
            let mut reader = BufReader::new(match stream.try_clone() {
                Ok(c) => c,
                Err(_) => break,
            });
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Err(_) | Ok(0) => break,
                    Ok(_) => {}
                }
                if line.trim().is_empty() {
                    break;
                }
            }
            let resp = "HTTP/1.1 200 OK\r\nContent-Length: 2500\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();

            // Send 25 small chunks rapidly with TCP_NODELAY enabled; sleep 1ms on the
            // first 15 chunks so the client reads multiple discrete TCP chunks without
            // accumulating 25 * macOS timer-coalescing slack (>200ms on VM runners).
            for i in 0..25 {
                let _ = stream.write_all(&[0x42; 100]);
                let _ = stream.flush();
                if i < 15 {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            let _ = stream.shutdown(std::net::Shutdown::Write);
            drop(stream);
        }
    });

    // Set mirror url to local test server
    let mut settings =
        tauri::Manager::state::<scaricatore_di_porto_lib::state::AppState>(&app).settings();
    settings.whisper_models_mirror_url = Some(format!("http://127.0.0.1:{port}"));
    tauri::Manager::state::<scaricatore_di_porto_lib::state::AppState>(&app)
        .save_settings(settings)
        .unwrap();

    let progress_events = Arc::new(Mutex::new(Vec::new()));
    let p_events = Arc::clone(&progress_events);
    tauri::Listener::listen(&app, "models:progress", move |event| {
        p_events.lock().unwrap().push(event.payload().to_string());
    });

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let d_tx1 = done_tx.clone();
    tauri::Listener::listen(&app, "models:completed", move |_| {
        let _ = d_tx1.send(());
    });
    let d_tx2 = done_tx.clone();
    tauri::Listener::listen(&app, "models:error", move |_| {
        let _ = d_tx2.send(());
    });

    scaricatore_di_porto_lib::commands::models::models_download(
        tauri::Manager::app_handle(&app).clone(),
        tauri::Manager::state::<scaricatore_di_porto_lib::state::AppState>(&app),
        "tiny-q5_1".to_string(),
    )
    .await
    .unwrap();

    done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("download task must finish or error");

    stop.store(true, Ordering::Relaxed);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_handle.join();

    let count = progress_events.lock().unwrap().len();
    // Unthrottled code emits an IPC event on every single read chunk (>= 15 events).
    // Throttled code coalesces intermediate chunk events to 100ms intervals (<= 6 events,
    // accommodating macOS VM timer coalescing slack while proving throttling is active).
    assert!(
        (2..=6).contains(&count),
        "models:progress emitted unthrottled on every TCP read chunk: got {count} events for 25 rapid chunks (expected 2..=6)"
    );
}

#[test]
fn src_unit_tests_do_not_call_tauri_mock_builder() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");
    let mut offenders = Vec::new();

    fn visit_dir(dir: &std::path::Path, offenders: &mut Vec<String>) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit_dir(&path, offenders);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let content = fs::read_to_string(&path).unwrap_or_default();
                if content.contains("mock_builder(") {
                    offenders.push(path.display().to_string());
                }
            }
        }
    }

    visit_dir(&src_dir, &mut offenders);
    assert!(
        offenders.is_empty(),
        "mock_builder() must NOT be called in unit tests under src/ because \
         cargo:rustc-link-arg-tests (/MANIFEST:EMBED) only applies to integration \
         tests in tests/*. Calling mock_builder() in src/ causes scaricatore_di_porto_lib-*.exe \
         to crash with 0xc0000139 (STATUS_ENTRYPOINT_NOT_FOUND) on Windows. Offenders: {offenders:?}"
    );
}
