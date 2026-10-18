//! Integration tests for the Whisper transcription pipeline (spec §10.2, §14 row M5, §15 item 9):
//! - 16 kHz mono WAV preparation with disk space check
//! - Chunking above 2 hours at silence points with timestamp offsets
//! - Progress parsing (-pp) and live segment preview
//! - GPU failure retry with -ng CPU fallback
//! - Hallucination guard (repeated loops / suspicious silence detection)
//! - Tree cancellation and partial file cleanup
//! - Queue engine integration for local file transcription and post-download transcription

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use scaricatore_di_porto_lib::services::job::JobEnvironment;
use scaricatore_di_porto_lib::services::jsruntime::{JsRuntimeMode, RuntimeMemoryStore};
use scaricatore_di_porto_lib::services::queue::engine::{
    AddRequest, BoxFuture, EngineConfig, IdleNotifier, QueueEngine, ToolProvider,
};
use scaricatore_di_porto_lib::services::queue::model::{
    CookieChoice, ItemOptions, ItemState, Kind, QualityChoice, QueueSummary,
};
use scaricatore_di_porto_lib::services::transcribe::whisper::{
    HallucinationWarning, TranscribeProgressSink, TranscribeRequest, detect_audio_silence,
    run_transcribe,
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

struct Fixture {
    dir: TempDir,
    fake_whisper: PathBuf,
    fake_ffmpeg: PathBuf,
    #[allow(dead_code)]
    fake_ffprobe: PathBuf,
    models_dir: PathBuf,
    model_path: PathBuf,
    vad_path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");

        let fake_whisper = bin_dir.join(if cfg!(windows) {
            "sdp-whisper.exe"
        } else {
            "sdp-whisper"
        });
        fs::copy(fake_tool_source(), &fake_whisper).expect("copy fake_tool as sdp-whisper");

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
            for p in [&fake_whisper, &fake_ffmpeg, &fake_ffprobe] {
                let mut perms = fs::metadata(p).expect("metadata").permissions();
                perms.set_mode(0o755);
                fs::set_permissions(p, perms).expect("chmod +x");
            }
        }

        let models_dir = dir.path().join("models");
        fs::create_dir_all(&models_dir).expect("create models dir");
        let model_path = models_dir.join("ggml-large-v3-turbo-q5_0.bin");
        fs::write(&model_path, b"FAKE_WHISPER_MODEL_BIN").expect("write dummy model");

        let vad_path = models_dir.join("ggml-silero-v5.1.2.bin");
        fs::write(&vad_path, b"FAKE_VAD_MODEL_BIN").expect("write dummy vad model");

        Self {
            dir,
            fake_whisper,
            fake_ffmpeg,
            fake_ffprobe,
            models_dir,
            model_path,
            vad_path,
        }
    }

    fn create_dummy_media(&self, filename: &str) -> PathBuf {
        let p = self.dir.path().join(filename);
        fs::write(&p, b"RIFF....WAVEfmt ....data....").expect("write dummy media");
        p
    }

    fn exe_dir(&self) -> PathBuf {
        self.fake_whisper.parent().unwrap().to_path_buf()
    }
}

#[tokio::test]
async fn transcribe_pipeline_progress_and_live_preview_and_outputs() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sample_video.mp4");

    let progress_events = Arc::new(Mutex::new(Vec::<(f64, Option<String>)>::new()));
    let sink: TranscribeProgressSink = {
        let events = Arc::clone(&progress_events);
        Arc::new(move |pct, preview| {
            events
                .lock()
                .unwrap()
                .push((pct, preview.map(str::to_string)));
        })
    };

    let req = TranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        whisper_path: fix.fake_whisper.clone(),
        model_path: fix.model_path.clone(),
        vad_model_path: Some(fix.vad_path.clone()),
        input_media: media.clone(),
        language: Some("it".to_string()),
        translate_to_english: false,
        skip_silence_vad: true,
        whisper_threads: 4,
        transcripts_subfolder: false,
        duration_sec: Some(60.0),
        temp_dir: Some(fix.dir.path().join("tmp")),
    };

    let outcome = run_transcribe(req, CancellationToken::new(), Some(sink))
        .await
        .expect("transcription should succeed");

    assert!(!outcome.used_cpu_fallback);
    assert_eq!(outcome.hallucination_warning, None);

    // Verify transcript files were produced
    assert!(outcome.paths.txt.is_file());
    assert!(outcome.paths.srt.is_file());
    assert!(outcome.paths.vtt.is_file());

    let txt = fs::read_to_string(&outcome.paths.txt).expect("read txt");
    assert!(txt.contains("Hello world"));
    assert!(txt.contains("This is a fake transcription."));

    let srt = fs::read_to_string(&outcome.paths.srt).expect("read srt");
    assert!(srt.contains("00:00:00,000 --> 00:00:02,500"));

    let vtt = fs::read_to_string(&outcome.paths.vtt).expect("read vtt");
    assert!(vtt.starts_with("WEBVTT"));

    // Verify progress events were parsed and live previews captured
    let recorded = progress_events.lock().unwrap().clone();
    assert!(!recorded.is_empty(), "progress events should be recorded");
    let has_100 = recorded.iter().any(|(pct, _)| *pct >= 99.0);
    assert!(has_100, "progress should reach 100%");

    let has_preview = recorded
        .iter()
        .any(|(_, preview)| preview.as_deref().unwrap_or("").contains("Hello world"));
    assert!(has_preview, "live segment preview should be captured");
}

#[tokio::test]
async fn transcribe_gpu_failure_retries_with_ng() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("gpu_fail_sample.mp4");

    // Signal fake_whisper to simulate GPU failure on the first run without -ng
    fs::write(fix.exe_dir().join("fail_whisper_gpu.txt"), b"1").expect("write fail gpu flag");

    let req = TranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        whisper_path: fix.fake_whisper.clone(),
        model_path: fix.model_path.clone(),
        vad_model_path: None,
        input_media: media.clone(),
        language: Some("auto".to_string()),
        translate_to_english: false,
        skip_silence_vad: false,
        whisper_threads: 0,
        transcripts_subfolder: false,
        duration_sec: Some(30.0),
        temp_dir: Some(fix.dir.path().join("tmp")),
    };

    let outcome = run_transcribe(req, CancellationToken::new(), None)
        .await
        .expect("transcription should succeed on retry with -ng");

    assert!(
        outcome.used_cpu_fallback,
        "CPU fallback (-ng) should be recorded after GPU failure"
    );
    assert!(outcome.paths.txt.is_file());
}

#[tokio::test]
async fn transcribe_hallucination_guard_detects_loops() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("hallucinating_sample.mp4");

    // Signal fake_whisper to emit repeated segment loops
    fs::write(fix.exe_dir().join("hallucinate.txt"), b"1").expect("write hallucinate flag");

    let req = TranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        whisper_path: fix.fake_whisper.clone(),
        model_path: fix.model_path.clone(),
        vad_model_path: None,
        input_media: media.clone(),
        language: Some("en".to_string()),
        translate_to_english: false,
        skip_silence_vad: false,
        whisper_threads: 0,
        transcripts_subfolder: false,
        duration_sec: Some(30.0),
        temp_dir: Some(fix.dir.path().join("tmp")),
    };

    let outcome = run_transcribe(req, CancellationToken::new(), None)
        .await
        .expect("transcription finishes even with warning");

    match outcome.hallucination_warning {
        Some(HallucinationWarning::RepeatedLoop { count, .. }) => {
            assert!(count >= 5, "repeat count should be at least 5");
        }
        other => panic!("expected RepeatedLoop warning, got: {other:?}"),
    }
}

#[tokio::test]
async fn transcribe_cancellation_kills_process_and_cleans_partials() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("sleep_sample.mp4");

    // Tell fake_whisper to sleep for 10 seconds
    fs::write(fix.exe_dir().join("fake_whisper_sleep.txt"), b"10000").expect("write sleep flag");

    let token = CancellationToken::new();
    let token_clone = token.clone();

    let tmp = fix.dir.path().join("tmp_cancel");
    fs::create_dir_all(&tmp).expect("create cancel tmp");

    let req = TranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        whisper_path: fix.fake_whisper.clone(),
        model_path: fix.model_path.clone(),
        vad_model_path: None,
        input_media: media.clone(),
        language: None,
        translate_to_english: false,
        skip_silence_vad: false,
        whisper_threads: 0,
        transcripts_subfolder: false,
        duration_sec: Some(30.0),
        temp_dir: Some(tmp.clone()),
    };

    let handle = tokio::spawn(async move { run_transcribe(req, token_clone, None).await });

    // Let process start, then cancel
    tokio::time::sleep(Duration::from_millis(80)).await;
    token.cancel();

    let res = handle.await.expect("join handle");
    assert!(res.is_err(), "cancelled transcription should return error");

    // Verify temp directory contains no lingering .wav or partial files
    let remaining: Vec<_> = fs::read_dir(&tmp)
        .expect("read dir")
        .filter_map(Result::ok)
        .collect();
    assert!(
        remaining.is_empty(),
        "all partial/temp files should be cleaned up on cancel: {:?}",
        remaining
    );
}

#[tokio::test]
async fn transcribe_chunking_with_silencedetect_and_merges() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("long_podcast.mp4");

    // Duration is > 2 hours (7400 seconds) -> triggers chunking
    let req = TranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        whisper_path: fix.fake_whisper.clone(),
        model_path: fix.model_path.clone(),
        vad_model_path: None,
        input_media: media.clone(),
        language: Some("it".to_string()),
        translate_to_english: false,
        skip_silence_vad: false,
        whisper_threads: 0,
        transcripts_subfolder: true,
        duration_sec: Some(7400.0),
        temp_dir: Some(fix.dir.path().join("tmp_chunk")),
    };

    let outcome = run_transcribe(req, CancellationToken::new(), None)
        .await
        .expect("chunked transcription should succeed");

    assert!(outcome.paths.txt.is_file());
    assert!(outcome.paths.srt.is_file());
    assert!(outcome.paths.vtt.is_file());

    // In a subfolder: transcripts/long_podcast.srt
    assert!(outcome.paths.srt.to_string_lossy().contains("transcripts"));

    let srt = fs::read_to_string(&outcome.paths.srt).expect("read srt");
    assert!(srt.contains("00:00:00,000 -->"));
}

// ---------------------------------------------------------------------------
// Queue Engine Integration
// ---------------------------------------------------------------------------

struct QueueFakeTools {
    whisper: PathBuf,
    ffmpeg: PathBuf,
    models_dir: PathBuf,
    default_model: String,
    private_temp: Option<PathBuf>,
}

impl ToolProvider for QueueFakeTools {
    fn environment(
        &self,
        options: ItemOptions,
    ) -> BoxFuture<Result<JobEnvironment, scaricatore_di_porto_lib::error::ErrorInfo>> {
        fs::create_dir_all(&options.output_dir).ok();
        let env = JobEnvironment {
            ytdlp: self.ffmpeg.clone(),
            ffmpeg: Some(self.ffmpeg.clone()),
            ffprobe: Some(self.ffmpeg.clone()),
            whisper: Some(self.whisper.clone()),
            models_dir: Some(self.models_dir.clone()),
            whisper_threads: 2,
            skip_silence_vad: true,
            default_whisper_model: Some(self.default_model.clone()),
            default_transcript_language: Some("it".to_string()),
            runtimes: scaricatore_di_porto_lib::services::inspect::Runtimes {
                mode: JsRuntimeMode::QuickJsOnly,
                qjs: None,
                deno: None,
                ytdlp_version: "2026.01.01".to_string(),
            },
            private_temp: self.private_temp.clone(),
            memory: Arc::new(RuntimeMemoryStore::load(self.models_dir.join("mem.json"))),
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
async fn queue_engine_transcribes_local_media_end_to_end() {
    let fix = Fixture::new();
    let local_file = fix.create_dummy_media("local_lecture.mp4");

    let queue_path = fix.dir.path().join("queue.json");
    let tools = Arc::new(QueueFakeTools {
        whisper: fix.fake_whisper.clone(),
        ffmpeg: fix.fake_ffmpeg.clone(),
        models_dir: fix.models_dir.clone(),
        default_model: "large-v3-turbo-q5_0".to_string(),
        private_temp: None,
    });

    let config = EngineConfig {
        path: queue_path.clone(),
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
        title: Some("Local Lecture".to_string()),
        site: Some("local".to_string()),
        duration_sec: Some(45),
        options: ItemOptions {
            kind: Kind::Video,
            quality: QualityChoice::Best,
            output_dir: fix.dir.path().join("out"),
            cookies: CookieChoice::None,
            runtime_mode: JsRuntimeMode::QuickJsOnly,
            filename_prefix: None,
            include_id_in_filename: false,
            prefer_h264: false,
            archive_file: None,
            sleep_interval: None,
            live_from_start: None,
            proxy: None,
            limit_rate: None,
            referer: None,
            user_agent: None,
            custom_headers: Vec::new(),
            impersonate_retry: false,
            encode: false,
            video_codec: None,
            audio_codec: None,
            trim_start: None,
            trim_end: None,
            keep_original: true,
            auto_play: false,
            embed_metadata: false,
            embed_thumbnail: false,
            embed_chapters: false,
            sponsorblock_mode: None,
            sponsorblock_categories: Vec::new(),
            transcribe: true,
            whisper_model: Some("large-v3-turbo-q5_0".to_string()),
            transcript_language: Some("it".to_string()),
            translate_to_english: Some(false),
            transcripts_subfolder: Some(false),
        },
        group_id: None,
        file: Some(local_file.clone()),
    };

    let outcome = engine.add(vec![req], true);
    assert_eq!(outcome.added.len(), 1);
    let item_id = outcome.added[0].clone();

    // Wait for queue item to process
    let mut finished = false;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let snapshot = engine.snapshot();
        if let Some(item) = snapshot.items.iter().find(|i| i.id == item_id) {
            if item.state == ItemState::Done {
                finished = true;
                assert_eq!(item.transcript_files.len(), 3);
                for path in &item.transcript_files {
                    assert!(
                        path.is_file(),
                        "transcript file should exist: {}",
                        path.display()
                    );
                }
                break;
            } else if item.state == ItemState::Failed {
                panic!("item failed: {:?}, log: {:?}", item.error, item.log_tail);
            }
        }
    }

    assert!(
        finished,
        "queue item should complete transcription successfully"
    );
}

#[tokio::test]
async fn transcribe_does_not_overwrite_existing_transcript_files() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("lecture.mp4");
    let existing_srt = fix.dir.path().join("lecture.srt");
    fs::write(
        &existing_srt,
        "1\n00:00:00,000 --> 00:00:01,000\nUSER EDITED\n\n",
    )
    .unwrap();

    let req = TranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        whisper_path: fix.fake_whisper.clone(),
        model_path: fix.model_path.clone(),
        vad_model_path: None,
        input_media: media,
        language: Some("en".to_string()),
        translate_to_english: false,
        skip_silence_vad: false,
        whisper_threads: 1,
        transcripts_subfolder: false,
        duration_sec: Some(30.0),
        temp_dir: Some(fix.dir.path().join("tmp")),
    };

    let outcome = run_transcribe(req, CancellationToken::new(), None)
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(&existing_srt).unwrap(),
        "1\n00:00:00,000 --> 00:00:01,000\nUSER EDITED\n\n",
        "existing .srt must never be silently overwritten"
    );
    assert_ne!(outcome.paths.srt, existing_srt);
    assert!(outcome.paths.srt.is_file());
    assert!(outcome.paths.txt.is_file());
    assert!(outcome.paths.vtt.is_file());
}

#[tokio::test]
async fn transcribe_cleans_temporary_wav_when_whisper_fails_to_spawn() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("error_sample.mp4");
    let tmp = fix.dir.path().join("tmp_error");
    fs::create_dir_all(&tmp).expect("create error tmp");

    let req = TranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        whisper_path: fix.dir.path().join("nonexistent_whisper_binary"),
        model_path: fix.model_path.clone(),
        vad_model_path: None,
        input_media: media,
        language: None,
        translate_to_english: false,
        skip_silence_vad: false,
        whisper_threads: 0,
        transcripts_subfolder: false,
        duration_sec: Some(30.0),
        temp_dir: Some(tmp.clone()),
    };

    let res = run_transcribe(req, CancellationToken::new(), None).await;
    assert!(
        res.is_err(),
        "should fail when whisper executable is nonexistent"
    );

    let remaining: Vec<_> = fs::read_dir(&tmp)
        .expect("read dir")
        .filter_map(Result::ok)
        .collect();
    assert!(
        remaining.is_empty(),
        "temporary prep wav must not leak on whisper spawn error, found: {:?}",
        remaining
    );
}

#[tokio::test]
async fn transcribe_cleans_temporary_wav_and_chunks_when_whisper_fails_to_spawn_in_chunked_mode() {
    let fix = Fixture::new();
    let media = fix.create_dummy_media("error_long_podcast.mp4");
    let tmp = fix.dir.path().join("tmp_chunk_error");
    fs::create_dir_all(&tmp).expect("create chunk error tmp");

    let req = TranscribeRequest {
        ffmpeg_path: fix.fake_ffmpeg.clone(),
        whisper_path: fix.dir.path().join("nonexistent_whisper_binary"),
        model_path: fix.model_path.clone(),
        vad_model_path: None,
        input_media: media,
        language: None,
        translate_to_english: false,
        skip_silence_vad: false,
        whisper_threads: 0,
        transcripts_subfolder: false,
        duration_sec: Some(7400.0),
        temp_dir: Some(tmp.clone()),
    };

    let res = run_transcribe(req, CancellationToken::new(), None).await;
    assert!(
        res.is_err(),
        "should fail when whisper executable is nonexistent"
    );

    let remaining: Vec<_> = fs::read_dir(&tmp)
        .expect("read dir")
        .filter_map(Result::ok)
        .collect();
    assert!(
        remaining.is_empty(),
        "temporary wav and chunk files must not leak on whisper spawn error in chunked mode, found: {:?}",
        remaining
    );
}

#[tokio::test]
async fn queue_engine_transcribe_uses_job_temp_dir_not_shared_private_temp() {
    let fix = Fixture::new();
    let local_file = fix.create_dummy_media("local_lecture_job_temp.mp4");
    let shared_cache_tmp = fix.dir.path().join("shared_app_cache_tmp");
    fs::create_dir_all(&shared_cache_tmp).expect("create shared cache tmp");

    let queue_path = fix.dir.path().join("queue_job_temp.json");
    let tools = Arc::new(QueueFakeTools {
        whisper: fix.fake_whisper.clone(),
        ffmpeg: fix.fake_ffmpeg.clone(),
        models_dir: fix.models_dir.clone(),
        default_model: "large-v3-turbo-q5_0".to_string(),
        private_temp: Some(shared_cache_tmp.clone()),
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

    let out_dir = fix.dir.path().join("out_job_temp");
    let req = AddRequest {
        url: format!("local://{}", local_file.display()),
        title: Some("Local Lecture Job Temp".to_string()),
        site: Some("local".to_string()),
        duration_sec: Some(30),
        options: ItemOptions {
            kind: Kind::Video,
            quality: QualityChoice::Best,
            output_dir: out_dir.clone(),
            cookies: CookieChoice::None,
            runtime_mode: JsRuntimeMode::QuickJsOnly,
            filename_prefix: None,
            include_id_in_filename: false,
            prefer_h264: false,
            archive_file: None,
            sleep_interval: None,
            live_from_start: None,
            proxy: None,
            limit_rate: None,
            referer: None,
            user_agent: None,
            custom_headers: Vec::new(),
            impersonate_retry: false,
            encode: false,
            video_codec: None,
            audio_codec: None,
            trim_start: None,
            trim_end: None,
            keep_original: true,
            auto_play: false,
            embed_metadata: false,
            embed_thumbnail: false,
            embed_chapters: false,
            sponsorblock_mode: None,
            sponsorblock_categories: Vec::new(),
            transcribe: true,
            whisper_model: Some("large-v3-turbo-q5_0".to_string()),
            transcript_language: Some("it".to_string()),
            translate_to_english: Some(false),
            transcripts_subfolder: Some(false),
        },
        group_id: None,
        file: Some(local_file.clone()),
    };

    let outcome = engine.add(vec![req], true);
    assert_eq!(outcome.added.len(), 1);
    let item_id = outcome.added[0].clone();

    let mut finished = false;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let snapshot = engine.snapshot();
        if let Some(item) = snapshot.items.iter().find(|i| i.id == item_id) {
            if item.state == ItemState::Done {
                finished = true;
                break;
            } else if item.state == ItemState::Failed {
                panic!("item failed: {:?}, log: {:?}", item.error, item.log_tail);
            }
        }
    }
    assert!(
        finished,
        "queue item should complete transcription successfully"
    );

    // Shared app cache tmp should not have received transcription temp files
    let remaining_shared: Vec<_> = fs::read_dir(&shared_cache_tmp)
        .expect("read shared tmp dir")
        .filter_map(Result::ok)
        .collect();
    assert!(
        remaining_shared.is_empty(),
        "shared private_temp must not be used for transcription files: {:?}",
        remaining_shared
    );
}

#[test]
fn docs_progress_and_decisions_reflect_whisper_vad_and_hallucination_reality() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let progress =
        fs::read_to_string(repo_root.join("docs/PROGRESS.md")).expect("read PROGRESS.md");
    let decisions =
        fs::read_to_string(repo_root.join("docs/DECISIONS.md")).expect("read DECISIONS.md");

    // 1. Hallucination guard: must not falsely claim trimming of repetitions (spec §10.2 item 7 is detection & toast warning only)
    assert!(
        !progress.contains("segments), trims repetitions"),
        "PROGRESS.md falsely claims that hallucination guard trims repetitions"
    );
    assert!(
        !decisions.contains("Trims repeating tails"),
        "DECISIONS.md falsely claims that hallucination guard trims repeating tails"
    );

    // 2. GPU error fallback: must not falsely claim GPU error detection
    assert!(
        !progress.contains("detects GPU errors on non-`-ng` runs"),
        "PROGRESS.md falsely claims automatic GPU error detection"
    );

    // 3. Silero VAD metadata: must not cite snakers4/silero-vad or wrong byte size / sha256 in D119
    assert!(
        !progress.contains("and snakers4/silero-vad models"),
        "PROGRESS.md falsely claims VAD model comes from snakers4/silero-vad"
    );
    assert!(
        !decisions.contains("and Silero VAD (`silero-v5.1.2`, 5,463,552 bytes"),
        "DECISIONS.md falsely claims Silero VAD size is 5,463,552 bytes"
    );
    assert!(
        !decisions.contains("6e033f947704df34baabfb3a1f879fa1e2b6186b5936aa7d6206d2146e5bf13b"),
        "DECISIONS.md falsely claims Silero VAD sha256 is 6e033f94..."
    );
    assert!(
        !decisions.contains("checks if ggml-silero.bin exists"),
        "DECISIONS.md D119 falsely claims checking for ggml-silero.bin"
    );

    // Positive checks: DECISIONS.md and whisper.models.json agree
    let models_json: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(repo_root.join("whisper.models.json"))
            .expect("read whisper.models.json"),
    )
    .expect("parse whisper.models.json");
    let vad_size = models_json["vad"]["sizeBytes"].as_u64().unwrap();
    let vad_sha = models_json["vad"]["sha256"].as_str().unwrap();
    assert_eq!(vad_size, 885098);
    assert_eq!(
        vad_sha,
        "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf"
    );
    assert!(decisions.contains("885,098 bytes"));
    assert!(decisions.contains("29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf"));
    assert!(decisions.contains("ggml-org/whisper-vad"));
}

#[tokio::test]
async fn detect_audio_silence_preserves_all_intervals_beyond_stderr_ring_buffer() {
    let fix = Fixture::new();
    let bin_dir = fix.fake_ffmpeg.parent().expect("fake_ffmpeg dir");

    // Generate 250 silence intervals (500 lines on stderr)
    // with intervals spanning from t = 10.0s to beyond t = 4900.0s.
    let mut fake_silence = String::new();
    for i in 0..250 {
        let start = 10.0 + (i as f64) * 20.0;
        let end = start + 2.5;
        fake_silence.push_str(&format!(
            "[silencedetect @ 0x5555] silence_start: {start:.1}\n\
             [silencedetect @ 0x5555] silence_end: {end:.1} | silence_duration: 2.5\n"
        ));
    }
    fs::write(bin_dir.join("fake_silence.txt"), fake_silence).expect("write fake_silence.txt");

    let dummy_wav = fix.dir.path().join("dummy.wav");
    fs::write(&dummy_wav, b"RIFF....WAVEfmt ").expect("write dummy wav");

    let intervals = detect_audio_silence(
        &fix.fake_ffmpeg,
        &dummy_wav,
        Some(fix.dir.path()),
        &CancellationToken::new(),
    )
    .await;

    // With 250 intervals (500 lines of stderr), a 200-line ring buffer only retains the last ~100 intervals.
    // Full streaming via LineSinks must retain all 250 intervals including early intervals like t = 10.0 and t = 1770.0.
    assert_eq!(
        intervals.len(),
        250,
        "all 250 silence intervals must be preserved, got {}",
        intervals.len()
    );
    assert!(
        intervals.iter().any(|i| (i.start_sec - 10.0).abs() < 0.1),
        "early silence interval at t=10.0s was lost"
    );
    assert!(
        intervals.iter().any(|i| (i.start_sec - 1770.0).abs() < 0.1),
        "silence interval around 30m boundary at t=1770.0s was lost"
    );
}
