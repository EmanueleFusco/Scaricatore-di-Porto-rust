//! Integration tests for media processing with ffmpeg (spec §9, §14 row M4, §15 item 8):
//! - Video encode with CPU and hardware encoders
//! - Hardware encoder fallback to CPU encoder on failure
//! - Original file kept untouched on encode failure
//! - Original file deleted on success only when keep_original = false
//! - Audio-only encoding (aac -> .m4a, mp3 -> .mp3, opus -> .opus)
//! - Trim parsing, validation, and expected duration calculation
//! - Progress tracking with `-progress pipe:1`

#![allow(clippy::collapsible_if)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use scaricatore_di_porto_lib::commands::system::encoders_probe;
use scaricatore_di_porto_lib::services::encoders::{
    AudioCodec, EncodersState, lookup_encoder, probe_all_encoders, probe_encoder,
};
use scaricatore_di_porto_lib::services::ffmpeg::{
    AudioEncodeOptions, EncodeJobKind, EncodeOutcome, EncodeRequest, FfmpegProgressTracker,
    TrimSpec, VideoEncodeOptions, build_audio_encode_args, build_video_encode_args,
    calculate_expected_duration, derive_encoded_output_path, format_duration_arg, parse_time_spec,
    probe_file_duration, run_encode, validate_trim,
};
use scaricatore_di_porto_lib::state::AppState;
use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};
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

struct Fixture {
    dir: TempDir,
    fake_ffmpeg: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let fake_ffmpeg = dir.path().join(if cfg!(windows) {
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

    fn create_dummy_media(&self, filename: &str, content: &[u8]) -> PathBuf {
        let path = self.dir.path().join(filename);
        fs::write(&path, content).expect("write dummy media");
        path
    }

    fn ensure_fake_ffprobe(&self) -> PathBuf {
        let fake_ffprobe = self.dir.path().join(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
        if !fake_ffprobe.exists() {
            fs::copy(fake_tool_source(), &fake_ffprobe).expect("copy fake_tool as ffprobe");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&fake_ffprobe).expect("metadata").permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&fake_ffprobe, perms).expect("chmod +x");
            }
        }
        fake_ffprobe
    }
}

// ---------------------------------------------------------------------------
// Unit & Helper tests
// ---------------------------------------------------------------------------

#[test]
fn trim_timestamp_parser_accepts_valid_formats_and_rejects_garbage() {
    assert_eq!(parse_time_spec("30").unwrap(), 30.0);
    assert_eq!(parse_time_spec("30.5").unwrap(), 30.5);
    assert_eq!(parse_time_spec("01:30").unwrap(), 90.0);
    assert_eq!(parse_time_spec("01:30.25").unwrap(), 90.25);
    assert_eq!(parse_time_spec("01:02:03").unwrap(), 3723.0);
    assert_eq!(parse_time_spec("01:02:03.500").unwrap(), 3723.5);

    // Garbage rejected
    assert!(parse_time_spec("invalid").is_err());
    assert!(parse_time_spec("12:65").is_err()); // seconds >= 60
    assert!(parse_time_spec("12:75:30").is_err()); // minutes >= 60
    assert!(parse_time_spec("-5").is_err());
}

#[test]
fn trim_validation_and_duration_math() {
    let spec = TrimSpec::new(Some(10.0), Some(40.0));
    assert!(validate_trim(spec.start, spec.end, Some(60.0)).is_ok());
    assert_eq!(
        calculate_expected_duration(Some(&spec), Some(60.0)),
        Some(30.0)
    );

    // start >= end
    let inv = TrimSpec::new(Some(40.0), Some(10.0));
    assert!(validate_trim(inv.start, inv.end, Some(60.0)).is_err());

    // out of media bounds
    let out_of_bounds = TrimSpec::new(Some(10.0), Some(100.0));
    assert!(validate_trim(out_of_bounds.start, out_of_bounds.end, Some(60.0)).is_err());

    // start only
    let start_only = TrimSpec::new(Some(10.0), None);
    assert_eq!(
        calculate_expected_duration(Some(&start_only), Some(60.0)),
        Some(50.0)
    );

    // format duration arg
    assert_eq!(format_duration_arg(45.5), "45.500");
}

#[test]
fn audio_args_builder_matches_spec() {
    let in_file = PathBuf::from("/tmp/input.mp4");
    let out_file = PathBuf::from("/tmp/output.m4a");
    let trim = TrimSpec::new(Some(5.0), Some(15.0));

    let opts = AudioEncodeOptions {
        input: in_file.clone(),
        output: out_file,
        audio_codec: AudioCodec::Aac,
        trim: Some(trim),
        embed_thumbnail: false,
    };
    let args = build_audio_encode_args(&opts);
    assert!(args.iter().any(|a| a == "-nostdin"));
    assert!(args.iter().any(|a| a == "-ss"));
    assert!(args.iter().any(|a| a == "-t"));
    assert!(args.iter().any(|a| a == "aac"));
    assert!(args.iter().any(|a| a == "192k"));

    // mp3
    let mp3_out = PathBuf::from("/tmp/output.mp3");
    let mp3_opts = AudioEncodeOptions {
        input: in_file.clone(),
        output: mp3_out,
        audio_codec: AudioCodec::Mp3,
        trim: None,
        embed_thumbnail: false,
    };
    let mp3_args = build_audio_encode_args(&mp3_opts);
    assert!(mp3_args.iter().any(|a| a == "libmp3lame"));

    // opus
    let opus_out = PathBuf::from("/tmp/output.opus");
    let opus_opts = AudioEncodeOptions {
        input: in_file,
        output: opus_out,
        audio_codec: AudioCodec::Opus,
        trim: None,
        embed_thumbnail: false,
    };
    let opus_args = build_audio_encode_args(&opus_opts);
    assert!(opus_args.iter().any(|a| a == "libopus"));
    assert!(opus_args.iter().any(|a| a == "128k"));
}

#[test]
fn progress_tracker_parses_pipe1_output() {
    let mut tracker = FfmpegProgressTracker::new(Some(100.0));

    let p1 = tracker.handle_line("out_time_us=25000000");
    assert!(p1.is_some());
    let prog = p1.unwrap();
    assert_eq!(prog.out_time_sec, 25.0);
    assert_eq!(prog.percent, Some(25.0));

    let p2 = tracker.handle_line("speed=2.0x");
    assert!(p2.is_none()); // speed alone doesn't trigger progress emission

    let p3 = tracker.handle_line("progress=continue");
    assert!(p3.is_none());

    let p4 = tracker.handle_line("progress=end");
    assert!(p4.is_some());
    let end_prog = p4.unwrap();
    assert!(end_prog.is_end);
    assert_eq!(end_prog.speed, Some(2.0));
}

// ---------------------------------------------------------------------------
// Integration tests with fake ffmpeg
// ---------------------------------------------------------------------------

#[tokio::test]
async fn probe_encoders_detects_candidates_and_filters_failures() {
    let fixture = Fixture::new();

    // Configure fake ffmpeg to fail probe for h264_vaapi
    fs::write(fixture.dir.path().join("fail_probe.txt"), "h264_vaapi")
        .expect("write fail_probe.txt");

    let cancel = CancellationToken::new();
    let state = EncodersState::new();
    let probed = probe_all_encoders(&fixture.fake_ffmpeg, None, None, &state, cancel, true, None)
        .await
        .expect("probe should succeed");

    assert!(!probed.is_empty(), "must detect at least CPU encoders");
    assert!(
        probed.iter().any(|e| e.id == "libx264"),
        "libx264 should be detected"
    );
    assert!(
        !probed.iter().any(|e| e.id == "h264_vaapi"),
        "h264_vaapi probe was simulated to fail and must be excluded"
    );
}

#[tokio::test]
async fn video_encode_succeeds_and_deletes_original_when_keep_original_false() {
    let fixture = Fixture::new();
    let original = fixture.create_dummy_media("sample.mp4", b"ORIGINAL_VIDEO_CONTENT");
    assert!(original.exists());
    let output = derive_encoded_output_path(&original, "mp4");

    let cancel = CancellationToken::new();
    let sink = Arc::new(|_: &_| {});

    let request = EncodeRequest {
        ffmpeg_path: fixture.fake_ffmpeg.clone(),
        kind: EncodeJobKind::Video(VideoEncodeOptions {
            input: original.clone(),
            output: output.clone(),
            video_encoder: "libx264".to_owned(),
            audio_codec: Some(AudioCodec::Aac),
            trim: None,
            embed_thumbnail: false,
        }),
        keep_original: false,
        total_duration: Some(10.0),
        temp_dir: None,
    };

    let outcome = run_encode(request, cancel, sink).await;

    match outcome {
        EncodeOutcome::Success {
            output_file,
            used_cpu_fallback,
        } => {
            assert!(!used_cpu_fallback);
            assert!(output_file.exists());
            assert_eq!(
                output_file.file_name().unwrap().to_str().unwrap(),
                "sample-encoded.mp4"
            );
        }
        other => panic!("expected success, got {other:?}"),
    }

    // Original deleted
    assert!(
        !original.exists(),
        "original file must be deleted when keep_original is false and exit code is 0"
    );
}

#[tokio::test]
async fn video_encode_keeps_original_when_keep_original_true() {
    let fixture = Fixture::new();
    let original = fixture.create_dummy_media("sample2.mp4", b"ORIGINAL_VIDEO_CONTENT");
    assert!(original.exists());
    let output = derive_encoded_output_path(&original, "mp4");

    let cancel = CancellationToken::new();
    let sink = Arc::new(|_: &_| {});

    let request = EncodeRequest {
        ffmpeg_path: fixture.fake_ffmpeg.clone(),
        kind: EncodeJobKind::Video(VideoEncodeOptions {
            input: original.clone(),
            output: output.clone(),
            video_encoder: "libx264".to_owned(),
            audio_codec: Some(AudioCodec::Aac),
            trim: None,
            embed_thumbnail: false,
        }),
        keep_original: true,
        total_duration: Some(10.0),
        temp_dir: None,
    };

    let outcome = run_encode(request, cancel, sink).await;

    match outcome {
        EncodeOutcome::Success { output_file, .. } => {
            assert!(output_file.exists());
        }
        other => panic!("expected success, got {other:?}"),
    }

    // Original preserved
    assert!(
        original.exists(),
        "original file must be preserved when keep_original is true"
    );
}

#[tokio::test]
async fn hardware_encoder_failure_falls_back_to_cpu_encoder() {
    let fixture = Fixture::new();
    let original = fixture.create_dummy_media("hw_test.mp4", b"ORIGINAL_VIDEO_CONTENT");
    let output = derive_encoded_output_path(&original, "mp4");

    // Tell fake ffmpeg to fail encode when -c:v h264_nvenc is used
    fs::write(fixture.dir.path().join("fail_encode.txt"), "h264_nvenc")
        .expect("write fail_encode.txt");

    let cancel = CancellationToken::new();
    let sink = Arc::new(|_: &_| {});

    let request = EncodeRequest {
        ffmpeg_path: fixture.fake_ffmpeg.clone(),
        kind: EncodeJobKind::Video(VideoEncodeOptions {
            input: original.clone(),
            output: output.clone(),
            video_encoder: "h264_nvenc".to_owned(),
            audio_codec: Some(AudioCodec::Aac),
            trim: None,
            embed_thumbnail: false,
        }),
        keep_original: true,
        total_duration: Some(10.0),
        temp_dir: None,
    };

    let outcome = run_encode(request, cancel, sink).await;

    match outcome {
        EncodeOutcome::Success {
            output_file,
            used_cpu_fallback,
        } => {
            assert!(
                used_cpu_fallback,
                "used_cpu_fallback must be true when hardware encoder failed"
            );
            assert!(output_file.exists());
        }
        other => panic!("expected success via CPU fallback, got {other:?}"),
    }
}

#[tokio::test]
async fn encode_failure_preserves_original_file() {
    let fixture = Fixture::new();
    let original = fixture.create_dummy_media("fail_test.mp4", b"ORIGINAL_VIDEO_CONTENT");
    let output = derive_encoded_output_path(&original, "mp4");

    // Tell fake ffmpeg to fail all encodes
    fs::write(fixture.dir.path().join("fail_all.txt"), "1").expect("write fail_all.txt");

    let cancel = CancellationToken::new();
    let sink = Arc::new(|_: &_| {});

    let request = EncodeRequest {
        ffmpeg_path: fixture.fake_ffmpeg.clone(),
        kind: EncodeJobKind::Video(VideoEncodeOptions {
            input: original.clone(),
            output: output.clone(),
            video_encoder: "libx264".to_owned(),
            audio_codec: Some(AudioCodec::Aac),
            trim: None,
            embed_thumbnail: false,
        }),
        keep_original: false, // even with keep_original = false, original MUST NOT be deleted on failure
        total_duration: Some(10.0),
        temp_dir: None,
    };

    let outcome = run_encode(request, cancel, sink).await;

    assert!(
        matches!(outcome, EncodeOutcome::Failed { .. }),
        "must report failure when encode fails"
    );
    assert!(
        original.exists(),
        "spec §9.2: original file MUST be kept untouched when ffmpeg fails"
    );
}

#[tokio::test]
async fn encoders_probe_succeeds_without_ytdlp_and_caches_ffmpeg_version() {
    let fixture = Fixture::new();
    let dir = TempDir::new().expect("temp dir");
    let triple =
        scaricatore_di_porto_lib::services::manifest::current_target_triple().expect("triple");
    let bundled = dir.path().join("bundled");
    fs::create_dir_all(&bundled).expect("bundled dir");

    // Only bundle fake ffmpeg, explicitly NO yt-dlp
    let mut ffmpeg_name = format!("ffmpeg-{triple}");
    if cfg!(windows) {
        ffmpeg_name.push_str(".exe");
    }
    let target_suffixed = bundled.join(&ffmpeg_name);
    fs::copy(&fixture.fake_ffmpeg, &target_suffixed).expect("copy fake ffmpeg");
    let target_plain = bundled.join(if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    });
    fs::copy(&fixture.fake_ffmpeg, &target_plain).expect("copy fake ffmpeg plain");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&target_suffixed)
            .expect("metadata")
            .permissions();
        perms.set_mode(0o755);
        let _ = fs::set_permissions(&target_suffixed, perms);
        let mut perms2 = fs::metadata(&target_plain).expect("metadata").permissions();
        perms2.set_mode(0o755);
        let _ = fs::set_permissions(&target_plain, perms2);
    }

    let tools_ctx = scaricatore_di_porto_lib::state::ToolContext::in_dir(dir.path());
    let state =
        AppState::from_path_with_tools(dir.path().join("settings.json"), tools_ctx).expect("state");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("mock app");
    app.manage(state);

    let result = encoders_probe(app.state::<AppState>(), None).await;
    assert!(
        result.is_ok(),
        "encoders_probe must succeed even when yt-dlp is not installed"
    );
    let probed = result.unwrap();
    assert!(!probed.is_empty(), "probed encoders must not be empty");

    // Verify cache is keyed by ffmpeg version ("9.0.2") rather than yt-dlp version ("2026.01.01")
    let app_state = app.state::<AppState>();
    let settings = app_state.settings();
    let resolved_ffmpeg =
        scaricatore_di_porto_lib::commands::env::resolve_cached(&app_state, &settings, "ffmpeg")
            .await
            .expect("ffmpeg must resolve");
    assert_eq!(
        resolved_ffmpeg.source,
        scaricatore_di_porto_lib::services::tools::ToolSource::Bundled,
        "ffmpeg must be resolved from Bundled source"
    );
    let mtime = fs::metadata(&resolved_ffmpeg.path)
        .and_then(|m| m.modified())
        .ok();

    let cached_by_ffmpeg_ver =
        app_state
            .encoders()
            .get_probed(&resolved_ffmpeg.path, mtime, Some("9.0.2"));
    assert!(
        cached_by_ffmpeg_ver.is_some(),
        "probe cache must be keyed by ffmpeg version"
    );

    let cached_by_ytdlp_ver =
        app_state
            .encoders()
            .get_probed(&resolved_ffmpeg.path, mtime, Some("2026.01.01"));
    assert!(
        cached_by_ytdlp_ver.is_none(),
        "probe cache must NOT be keyed by yt-dlp version"
    );
}

#[tokio::test]
async fn hardware_encoder_fallback_informs_user_via_toast_and_log_tail() {
    use scaricatore_di_porto_lib::error::ErrorInfo;
    use scaricatore_di_porto_lib::services::inspect::Runtimes;
    use scaricatore_di_porto_lib::services::job::JobEnvironment;
    use scaricatore_di_porto_lib::services::jsruntime::{JsRuntimeMode, RuntimeMemoryStore};
    use scaricatore_di_porto_lib::services::queue::engine::{
        BoxFuture, EngineConfig, QueueEngine, ToolProvider,
    };
    use scaricatore_di_porto_lib::services::queue::events::QueueUpdate;
    use scaricatore_di_porto_lib::services::queue::model::{
        ItemOptions, ItemState, LaneConfig, QueueItem,
    };
    use scaricatore_di_porto_lib::services::queue::store::{QUEUE_SCHEMA_VERSION, QueueFile};
    use std::sync::Mutex;

    let fixture = Fixture::new();
    let original = fixture.create_dummy_media("hw_fallback.mp4", b"ORIGINAL_VIDEO_CONTENT");

    // Tell fake ffmpeg to fail encode when -c:v h264_nvenc is used
    fs::write(fixture.dir.path().join("fail_encode.txt"), "h264_nvenc")
        .expect("write fail_encode.txt");

    struct FakeProvider {
        ffmpeg: PathBuf,
        memory: Arc<RuntimeMemoryStore>,
    }

    impl ToolProvider for FakeProvider {
        fn environment(
            &self,
            _options: ItemOptions,
        ) -> BoxFuture<Result<JobEnvironment, ErrorInfo>> {
            let env = JobEnvironment {
                ytdlp: self.ffmpeg.clone(),
                ffmpeg: Some(self.ffmpeg.clone()),
                ffprobe: None,
                whisper: None,
                models_dir: None,
                config_dir: None,
                whisper_threads: 0,
                skip_silence_vad: true,
                default_whisper_model: None,
                default_transcript_language: None,
                default_transcription_engine_order: None,
                default_gemini_model: None,
                default_summarize: false,
                default_summary_language: None,
                runtimes: Runtimes {
                    mode: JsRuntimeMode::QuickJsOnly,
                    qjs: None,
                    deno: None,
                    ytdlp_version: "2026.08.19".into(),
                },
                memory: Arc::clone(&self.memory),
                private_temp: None,
                youtube_clients: None,
            };
            Box::pin(std::future::ready(Ok(env)))
        }
    }

    let item_id = "test-hw-fallback-item".to_string();
    let initial_item = QueueItem {
        id: item_id.clone(),
        url: "https://example.com/test".into(),
        title: Some("Hardware encode fallback test".into()),
        file: Some(original.clone()),
        state: ItemState::Queued,
        options: ItemOptions {
            encode: true,
            video_codec: Some("h264_nvenc".into()),
            output_dir: fixture.dir.path().to_path_buf(),
            ..ItemOptions::default()
        },
        ..QueueItem::default()
    };

    let queue_path = fixture.dir.path().join("queue.json");
    let queue_file = QueueFile {
        schema_version: QUEUE_SCHEMA_VERSION,
        items: vec![initial_item],
        groups: Vec::new(),
        paused_all: false,
    };
    queue_file.save(&queue_path).expect("save queue file");

    let updates = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&updates);

    let engine = QueueEngine::open(EngineConfig {
        path: queue_path,
        provider: Arc::new(FakeProvider {
            ffmpeg: fixture.fake_ffmpeg.clone(),
            memory: Arc::new(RuntimeMemoryStore::load(
                fixture.dir.path().join("runtime-memory.json"),
            )),
        }),
        sink: Arc::new(move |update: &QueueUpdate| {
            if let Ok(mut seen) = seen.lock() {
                seen.push(update.clone());
            }
        }),
        lanes: LaneConfig::default(),
        notify_on_idle: false,
        notifier: None,
    });
    engine.start();

    // Poll until the item is Done and the fallback toast has reached the sink
    let start = std::time::Instant::now();
    let mut finished = false;
    let mut toast_emitted = false;
    while start.elapsed() < Duration::from_secs(5) {
        if !finished {
            let snap = engine.snapshot();
            if let Some(item) = snap.items.iter().find(|i| i.id == item_id) {
                if item.state == ItemState::Done {
                    assert!(
                        item.log_tail
                            .iter()
                            .any(|l| l.contains("Hardware encoder failed")),
                        "log_tail must record hardware encoder fallback notice"
                    );
                    finished = true;
                }
            }
        }
        if !toast_emitted {
            let seen_updates = updates.lock().unwrap();
            toast_emitted = seen_updates.iter().any(|u| {
                u.toast
                    .as_ref()
                    .is_some_and(|t| t.message_key == "toast.hardwareEncoderFallback")
            });
        }
        if finished && toast_emitted {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    engine.shutdown();

    assert!(finished, "item must finish with Done state");
    assert!(
        toast_emitted,
        "a toast with messageKey 'toast.hardwareEncoderFallback' must be emitted"
    );
}

#[tokio::test]
async fn probe_videotoolbox_falls_back_to_bitrate_when_qv_rejected() {
    let fixture = Fixture::new();

    // Configure fake ffmpeg to reject -q:v
    fs::write(fixture.dir.path().join("fail_qv.txt"), "1").expect("write fail_qv.txt");

    let cancel = CancellationToken::new();
    let def = lookup_encoder("h264_videotoolbox").expect("h264_videotoolbox exists");

    let ok = probe_encoder(&fixture.fake_ffmpeg, def, cancel, None).await;
    assert!(
        ok,
        "probe_encoder must succeed by falling back to -b:v when -q:v fails"
    );
}

#[tokio::test]
async fn encode_videotoolbox_retries_with_bitrate_when_qv_rejected() {
    let fixture = Fixture::new();
    let original = fixture.create_dummy_media("sample.mp4", b"ORIGINAL_VIDEO_CONTENT");
    let output = derive_encoded_output_path(&original, "mp4");

    // Configure fake ffmpeg to reject -q:v
    fs::write(fixture.dir.path().join("fail_qv.txt"), "1").expect("write fail_qv.txt");

    let cancel = CancellationToken::new();
    let sink = Arc::new(|_: &_| {});

    let request = EncodeRequest {
        ffmpeg_path: fixture.fake_ffmpeg.clone(),
        kind: EncodeJobKind::Video(VideoEncodeOptions {
            input: original.clone(),
            output: output.clone(),
            video_encoder: "h264_videotoolbox".to_owned(),
            audio_codec: Some(AudioCodec::Aac),
            trim: None,
            embed_thumbnail: false,
        }),
        keep_original: true,
        total_duration: Some(10.0),
        temp_dir: None,
    };

    let outcome = run_encode(request, cancel, sink).await;

    match outcome {
        EncodeOutcome::Success {
            output_file,
            used_cpu_fallback,
        } => {
            assert_eq!(output_file, output);
            assert!(
                !used_cpu_fallback,
                "used_cpu_fallback must be false because videotoolbox succeeded via -b:v fallback"
            );
            assert!(output_file.exists());
        }
        other => panic!("expected EncodeOutcome::Success, got {:?}", other),
    }
}

#[test]
fn encode_args_hevc_vaapi_includes_hvc1_tag() {
    let opts = VideoEncodeOptions {
        input: PathBuf::from("in.mp4"),
        output: PathBuf::from("out.mp4"),
        video_encoder: "hevc_vaapi".to_string(),
        audio_codec: None,
        trim: None,
        embed_thumbnail: false,
    };
    let args = build_video_encode_args(&opts);
    let args_str: Vec<String> = args
        .into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(
        args_str
            .windows(2)
            .any(|w| w[0] == "-tag:v" && w[1] == "hvc1"),
        "hevc_vaapi must include -tag:v hvc1"
    );
}

#[tokio::test]
async fn ffmpeg_and_encoders_pass_private_temp_dir_to_child_processes() {
    let fixture = Fixture::new();
    let original = fixture.create_dummy_media("temp_test.mp4", b"ORIGINAL_VIDEO_CONTENT");
    let output = derive_encoded_output_path(&original, "mp4");
    let expected_temp = fixture.dir.path().join("my_private_temp");
    fs::create_dir_all(&expected_temp).expect("create temp dir");

    let env_var_to_record = if cfg!(windows) { "TEMP" } else { "TMPDIR" };
    fs::write(fixture.dir.path().join("record_env.txt"), env_var_to_record)
        .expect("write record_env.txt");

    // 1. Test execute_single_encode / run_encode passes temp_dir
    let cancel = CancellationToken::new();
    let sink = Arc::new(|_: &_| {});
    let request = EncodeRequest {
        ffmpeg_path: fixture.fake_ffmpeg.clone(),
        kind: EncodeJobKind::Video(VideoEncodeOptions {
            input: original.clone(),
            output: output.clone(),
            video_encoder: "libx264".to_owned(),
            audio_codec: Some(AudioCodec::Aac),
            trim: None,
            embed_thumbnail: false,
        }),
        keep_original: true,
        total_duration: Some(10.0),
        temp_dir: Some(expected_temp.clone()),
    };

    let outcome = run_encode(request, cancel.clone(), sink).await;
    assert!(matches!(outcome, EncodeOutcome::Success { .. }));

    let recorded = fs::read_to_string(fixture.dir.path().join("recorded_env.txt"))
        .expect("read recorded_env.txt");
    assert_eq!(
        recorded.trim(),
        expected_temp.to_string_lossy(),
        "run_encode must set private temp_dir on child process"
    );

    // 2. Test probe_encoder passes temp_dir
    let _ = fs::remove_file(fixture.dir.path().join("recorded_env.txt"));
    let def = lookup_encoder("libx264").expect("libx264 exists");
    let ok = probe_encoder(
        &fixture.fake_ffmpeg,
        def,
        cancel.clone(),
        Some(&expected_temp),
    )
    .await;
    assert!(ok);
    let recorded_probe = fs::read_to_string(fixture.dir.path().join("recorded_env.txt"))
        .expect("read recorded_env.txt");
    assert_eq!(
        recorded_probe.trim(),
        expected_temp.to_string_lossy(),
        "probe_encoder must set private temp_dir on child process"
    );

    // 3. Test probe_file_duration passes temp_dir
    let _ = fs::remove_file(fixture.dir.path().join("recorded_env.txt"));
    let fake_ffprobe = fixture.ensure_fake_ffprobe();
    let dur = probe_file_duration(&fake_ffprobe, &original, Some(&expected_temp), cancel).await;
    assert_eq!(dur, Some(120.0));
    let recorded_ffprobe = fs::read_to_string(fixture.dir.path().join("recorded_env.txt"))
        .expect("read recorded_env.txt");
    assert_eq!(
        recorded_ffprobe.trim(),
        expected_temp.to_string_lossy(),
        "probe_file_duration must set private temp_dir on child process"
    );
}

#[tokio::test]
async fn encode_falls_back_to_ffprobe_duration_when_total_duration_is_none() {
    let fixture = Fixture::new();
    let original = fixture.create_dummy_media("dur_test.mp4", b"ORIGINAL_VIDEO_CONTENT");
    let output = derive_encoded_output_path(&original, "mp4");
    fixture.ensure_fake_ffprobe();

    // Specify fake ffprobe duration
    fs::write(
        fixture.dir.path().join("fake_ffprobe_duration.txt"),
        "200.0",
    )
    .expect("write fake_ffprobe_duration.txt");

    let cancel = CancellationToken::new();
    let recorded_percent = Arc::new(std::sync::Mutex::new(Vec::new()));
    let rec = Arc::clone(&recorded_percent);
    let sink = Arc::new(
        move |prog: &scaricatore_di_porto_lib::services::ffmpeg::FfmpegProgress| {
            if let Some(p) = prog.percent {
                rec.lock().unwrap().push(p);
            }
        },
    );

    let request = EncodeRequest {
        ffmpeg_path: fixture.fake_ffmpeg.clone(),
        kind: EncodeJobKind::Video(VideoEncodeOptions {
            input: original.clone(),
            output: output.clone(),
            video_encoder: "libx264".to_owned(),
            audio_codec: Some(AudioCodec::Aac),
            trim: None,
            embed_thumbnail: false,
        }),
        keep_original: true,
        total_duration: None, // Missing! Must be queried from ffprobe
        temp_dir: None,
    };

    let outcome = run_encode(request, cancel, sink).await;
    assert!(matches!(outcome, EncodeOutcome::Success { .. }));

    let percentages = recorded_percent.lock().unwrap().clone();
    assert!(
        !percentages.is_empty(),
        "percent progress must be calculated using ffprobe duration"
    );
    // fake_ffmpeg outputs out_time_us=5000000 (5s) -> 5/200 = 2.5%, and progress=end -> 100%
    assert!(percentages.iter().any(|&p| (p - 2.5).abs() < 0.1));
    assert!(percentages.iter().any(|&p| (p - 100.0).abs() < 0.1));
}

use scaricatore_di_porto_lib::error::ErrorInfo;
use scaricatore_di_porto_lib::services::inspect::Runtimes;
use scaricatore_di_porto_lib::services::job::JobEnvironment;
use scaricatore_di_porto_lib::services::jsruntime::{JsRuntimeMode, RuntimeMemoryStore};
use scaricatore_di_porto_lib::services::queue::engine::{
    BoxFuture, EngineConfig, QueueEngine, ToolProvider,
};
use scaricatore_di_porto_lib::services::queue::model::{
    ItemOptions, ItemState, LaneConfig, QueueItem,
};
use scaricatore_di_porto_lib::services::queue::store::{QUEUE_SCHEMA_VERSION, QueueFile};

struct TestProvider {
    ffmpeg: PathBuf,
    memory: Arc<RuntimeMemoryStore>,
}

impl ToolProvider for TestProvider {
    fn environment(&self, _options: ItemOptions) -> BoxFuture<Result<JobEnvironment, ErrorInfo>> {
        let env = JobEnvironment {
            ytdlp: self.ffmpeg.clone(),
            ffmpeg: Some(self.ffmpeg.clone()),
            ffprobe: None,
            whisper: None,
            models_dir: None,
            config_dir: None,
            whisper_threads: 0,
            skip_silence_vad: true,
            default_whisper_model: None,
            default_transcript_language: None,
            default_transcription_engine_order: None,
            default_gemini_model: None,
            default_summarize: false,
            default_summary_language: None,
            runtimes: Runtimes {
                mode: JsRuntimeMode::QuickJsOnly,
                qjs: None,
                deno: None,
                ytdlp_version: "2026.08.19".into(),
            },
            memory: Arc::clone(&self.memory),
            private_temp: None,
            youtube_clients: None,
        };
        Box::pin(std::future::ready(Ok(env)))
    }
}

#[tokio::test]
async fn queue_engine_probes_duration_with_ffprobe_and_validates_trim() {
    let fixture = Fixture::new();
    let original = fixture.create_dummy_media("sample_queue.mp4", b"ORIGINAL_VIDEO_CONTENT");
    fixture.ensure_fake_ffprobe();

    fs::write(
        fixture.dir.path().join("fake_ffprobe_duration.txt"),
        "100.0",
    )
    .expect("write fake_ffprobe_duration.txt");

    // 1. Success case: item with duration_sec = None gets populated from ffprobe
    let item_id = "test-ffprobe-item".to_string();
    let initial_item = QueueItem {
        id: item_id.clone(),
        url: "https://example.com/test".into(),
        file: Some(original.clone()),
        state: ItemState::Queued,
        duration_sec: None, // No duration from metadata!
        options: ItemOptions {
            encode: true,
            output_dir: fixture.dir.path().to_path_buf(),
            trim_end: Some("01:00".into()), // 60s <= 100s, valid
            ..ItemOptions::default()
        },
        ..QueueItem::default()
    };

    let queue_path = fixture.dir.path().join("queue_ffprobe.json");
    let queue_file = QueueFile {
        schema_version: QUEUE_SCHEMA_VERSION,
        items: vec![initial_item],
        groups: Vec::new(),
        paused_all: false,
    };
    queue_file.save(&queue_path).expect("save queue file");

    let engine = QueueEngine::open(EngineConfig {
        path: queue_path,
        provider: Arc::new(TestProvider {
            ffmpeg: fixture.fake_ffmpeg.clone(),
            memory: Arc::new(RuntimeMemoryStore::load(
                fixture.dir.path().join("runtime-memory.json"),
            )),
        }),
        sink: Arc::new(|_| {}),
        lanes: LaneConfig::default(),
        notify_on_idle: false,
        notifier: None,
    });
    engine.start();

    let start = std::time::Instant::now();
    let mut item_updated_duration = false;
    while start.elapsed() < Duration::from_secs(5) {
        let snap = engine.snapshot();
        if let Some(item) = snap.items.iter().find(|i| i.id == item_id) {
            if item.state == ItemState::Done {
                assert_eq!(
                    item.duration_sec,
                    Some(100),
                    "item.duration_sec should be populated from ffprobe"
                );
                item_updated_duration = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        item_updated_duration,
        "item should complete successfully with probed duration"
    );

    // 2. Reject trim exceeding duration probed by ffprobe (Spec §9.1, §9.2, ffmpeg.rs:183-190)
    let original2 = fixture.create_dummy_media("sample_queue2.mp4", b"ORIGINAL_VIDEO_CONTENT");
    let invalid_trim_id = "test-trim-exceed-item".to_string();
    let invalid_item = QueueItem {
        id: invalid_trim_id.clone(),
        url: "https://example.com/test2".into(),
        file: Some(original2.clone()),
        state: ItemState::Queued,
        duration_sec: None,
        options: ItemOptions {
            encode: true,
            output_dir: fixture.dir.path().to_path_buf(),
            trim_end: Some("02:00".into()), // 120s > 100s, must be rejected
            ..ItemOptions::default()
        },
        ..QueueItem::default()
    };
    let queue_path2 = fixture.dir.path().join("queue_ffprobe_exceed.json");
    let queue_file2 = QueueFile {
        schema_version: QUEUE_SCHEMA_VERSION,
        items: vec![invalid_item],
        groups: Vec::new(),
        paused_all: false,
    };
    queue_file2.save(&queue_path2).expect("save queue file");

    let engine2 = QueueEngine::open(EngineConfig {
        path: queue_path2,
        provider: Arc::new(TestProvider {
            ffmpeg: fixture.fake_ffmpeg.clone(),
            memory: Arc::new(RuntimeMemoryStore::load(
                fixture.dir.path().join("runtime-memory2.json"),
            )),
        }),
        sink: Arc::new(|_| {}),
        lanes: LaneConfig::default(),
        notify_on_idle: false,
        notifier: None,
    });
    engine2.start();

    let start = std::time::Instant::now();
    let mut trim_rejected = false;
    while start.elapsed() < Duration::from_secs(5) {
        let snap = engine2.snapshot();
        if let Some(item) = snap.items.iter().find(|i| i.id == invalid_trim_id) {
            if item.state == ItemState::Failed {
                assert!(
                    item.error
                        .as_ref()
                        .and_then(|e| e.detail.as_deref())
                        .unwrap_or("")
                        .contains("invalid trim"),
                    "should fail with invalid trim error when trim exceeds ffprobe duration"
                );
                trim_rejected = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        trim_rejected,
        "item with trim exceeding ffprobe duration must fail"
    );
}

#[tokio::test]
async fn queue_engine_pushes_ffmpeg_stderr_to_log_tail_on_failure() {
    let fixture = Fixture::new();
    let original = fixture.create_dummy_media("fail_log_tail.mp4", b"ORIGINAL_VIDEO_CONTENT");
    let item_id = "test-fail-log-tail-item".to_string();

    // Tell fake ffmpeg to fail all encodes
    fs::write(fixture.dir.path().join("fail_all.txt"), "1").expect("write fail_all.txt");

    let item = QueueItem {
        id: item_id.clone(),
        url: "https://example.com/test-fail".into(),
        file: Some(original.clone()),
        state: ItemState::Queued,
        duration_sec: Some(10),
        options: ItemOptions {
            encode: true,
            output_dir: fixture.dir.path().to_path_buf(),
            ..ItemOptions::default()
        },
        ..QueueItem::default()
    };
    let queue_path = fixture.dir.path().join("queue_fail_log_tail.json");
    let queue_file = QueueFile {
        schema_version: QUEUE_SCHEMA_VERSION,
        items: vec![item],
        groups: Vec::new(),
        paused_all: false,
    };
    queue_file.save(&queue_path).expect("save queue file");

    let engine = QueueEngine::open(EngineConfig {
        path: queue_path,
        provider: Arc::new(TestProvider {
            ffmpeg: fixture.fake_ffmpeg.clone(),
            memory: Arc::new(RuntimeMemoryStore::load(
                fixture.dir.path().join("runtime-memory-fail.json"),
            )),
        }),
        sink: Arc::new(|_| {}),
        lanes: LaneConfig::default(),
        notify_on_idle: false,
        notifier: None,
    });
    engine.start();

    let start = std::time::Instant::now();
    let mut failure_recorded_with_log = false;
    while start.elapsed() < Duration::from_secs(5) {
        let snap = engine.snapshot();
        if let Some(item) = snap.items.iter().find(|i| i.id == item_id) {
            if item.state == ItemState::Failed {
                assert!(
                    item.log_tail
                        .iter()
                        .any(|line| line.contains("simulated total encode failure")),
                    "item.log_tail should contain stderr lines from failed ffmpeg process"
                );
                failure_recorded_with_log = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        failure_recorded_with_log,
        "item should fail and have stderr in log_tail"
    );
}

#[test]
fn embed_thumbnail_preserves_attached_pic_in_ffmpeg_args() {
    // 1. Video encode with embed_thumbnail
    let v_opts = VideoEncodeOptions {
        input: PathBuf::from("video.mkv"),
        output: PathBuf::from("video-encoded.mp4"),
        video_encoder: "libx264".to_string(),
        audio_codec: Some(AudioCodec::Aac),
        trim: None,
        embed_thumbnail: true,
    };
    let v_args: Vec<String> = build_video_encode_args(&v_opts)
        .into_iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    assert!(v_args.contains(&"0:V:0?".to_string()));
    assert!(v_args.contains(&"0:v:disp:attached_pic?".to_string()));
    assert!(v_args.contains(&"-c:v:disp:attached_pic".to_string()));
    assert!(v_args.contains(&"-disposition:v:disp:attached_pic".to_string()));

    // 2. Audio encode AAC with embed_thumbnail
    let a_opts_aac = AudioEncodeOptions {
        input: PathBuf::from("audio.webm"),
        output: PathBuf::from("audio-encoded.m4a"),
        audio_codec: AudioCodec::Aac,
        trim: None,
        embed_thumbnail: true,
    };
    let a_args_aac: Vec<String> = build_audio_encode_args(&a_opts_aac)
        .into_iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    assert!(!a_args_aac.contains(&"-vn".to_string()));
    assert!(a_args_aac.contains(&"0:a:0?".to_string()));
    assert!(a_args_aac.contains(&"0:v:disp:attached_pic?".to_string()));

    // 3. Audio encode MP3 with embed_thumbnail
    let a_opts_mp3 = AudioEncodeOptions {
        input: PathBuf::from("audio.webm"),
        output: PathBuf::from("audio-encoded.mp3"),
        audio_codec: AudioCodec::Mp3,
        trim: None,
        embed_thumbnail: true,
    };
    let a_args_mp3: Vec<String> = build_audio_encode_args(&a_opts_mp3)
        .into_iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    assert!(!a_args_mp3.contains(&"-vn".to_string()));
    assert!(a_args_mp3.contains(&"0:a:0?".to_string()));
    assert!(a_args_mp3.contains(&"0:v:disp:attached_pic?".to_string()));

    // 4. Audio encode Opus with embed_thumbnail strips video (-vn) due to lack of attached_pic stream support
    let a_opts_opus = AudioEncodeOptions {
        input: PathBuf::from("audio.webm"),
        output: PathBuf::from("audio-encoded.opus"),
        audio_codec: AudioCodec::Opus,
        trim: None,
        embed_thumbnail: true,
    };
    let a_args_opus: Vec<String> = build_audio_encode_args(&a_opts_opus)
        .into_iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    assert!(a_args_opus.contains(&"-vn".to_string()));
}
