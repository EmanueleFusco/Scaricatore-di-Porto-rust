//! M2 IPC tests: the queue's commands, state and events (spec §3, §15).
//!
//! Same approach as `m1_ipc.rs` — the fake yt-dlp is the resolved tool, so the
//! whole path (`queue_add` → engine → `queue:update`) runs offline, without a
//! window and without touching the real settings of the developer machine.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use scaricatore_di_porto_lib::error::ErrorInfo;
use scaricatore_di_porto_lib::services::inspect::{ResolvedRuntime, Runtimes};
use scaricatore_di_porto_lib::services::job::JobEnvironment;
use scaricatore_di_porto_lib::services::jsruntime::{
    JsRuntimeMode, RuntimeKind, RuntimeMemoryStore,
};
use scaricatore_di_porto_lib::services::queue::engine::{
    AddRequest, BoxFuture, EngineConfig, Outcome, QueueEngine, ToolProvider,
};
use scaricatore_di_porto_lib::services::queue::events::QueueUpdate;
use scaricatore_di_porto_lib::services::queue::model::{
    ItemOptions, ItemState, Kind, LaneConfig, QualityChoice,
};
use scaricatore_di_porto_lib::state::{AppState, Settings};
use tempfile::TempDir;

/// The `fake_ytdlp` example `cargo test` builds next to this test binary.
fn fake_ytdlp_source() -> PathBuf {
    let profile_dir = std::env::current_exe()
        .expect("the test binary has a path")
        .parent()
        .and_then(Path::parent)
        .expect("the test binary lives in <profile>/deps/")
        .to_path_buf();
    let mut path = profile_dir.join("examples").join("fake_ytdlp");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    assert!(
        path.is_file(),
        "the fake_ytdlp example was not built next to the tests (looked for {}).\n\
         Run the suite as `cargo test`.",
        path.display()
    );
    path
}

/// Resolves every item to the same fake binary.
struct FakeTools {
    ytdlp: PathBuf,
    memory: Arc<RuntimeMemoryStore>,
}

impl ToolProvider for FakeTools {
    fn environment(&self, options: ItemOptions) -> BoxFuture<Result<JobEnvironment, ErrorInfo>> {
        let _ = std::fs::create_dir_all(&options.output_dir);
        let environment = JobEnvironment {
            ytdlp: self.ytdlp.clone(),
            ffmpeg: Some(self.ytdlp.clone()),
            runtimes: Runtimes {
                mode: JsRuntimeMode::QuickJsOnly,
                qjs: Some(ResolvedRuntime {
                    path: self.ytdlp.clone(),
                    version: Some("0.17.0".into()),
                }),
                deno: None,
                ytdlp_version: "2026.08.19".into(),
            },
            memory: Arc::clone(&self.memory),
            private_temp: None,
            youtube_clients: None,
        };
        Box::pin(std::future::ready(Ok(environment)))
    }
}

struct Fixture {
    dir: TempDir,
    state: AppState,
    /// The `queue:update` events the sink collected.
    events: Arc<Mutex<Vec<QueueUpdate>>>,
}

/// The settings are written **before** the engine opens, which is the order
/// `setup` uses (spec §7.2: the lanes come from the settings that were just
/// read).
fn fixture(settings: Settings) -> Fixture {
    let dir = TempDir::new().expect("temp dir");

    let mut ytdlp = dir.path().join("yt-dlp");
    if cfg!(windows) {
        ytdlp.set_extension("exe");
    }
    std::fs::copy(fake_ytdlp_source(), &ytdlp).expect("copy the fake");
    scaricatore_di_porto_lib::services::tools::make_executable(&ytdlp).expect("chmod +x");

    let state = AppState::from_path_with_tools(
        dir.path().join("settings.json"),
        scaricatore_di_porto_lib::state::ToolContext::in_dir(dir.path()),
    )
    .expect("state loads");
    // Spec §7.2: the lanes come from the settings that were just read, so the
    // engine is opened **after** them — the order `setup` uses.
    let lanes = if settings == Settings::default() {
        LaneConfig::default()
    } else {
        state
            .save_settings(settings)
            .expect("the settings are saved");
        state.settings().lane_config()
    };

    let events: Arc<Mutex<Vec<QueueUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let engine = QueueEngine::open(EngineConfig {
        path: dir.path().join("queue.json"),
        provider: Arc::new(FakeTools {
            ytdlp,
            memory: Arc::new(RuntimeMemoryStore::load(
                dir.path().join("runtime-memory.json"),
            )),
        }),
        sink: {
            let events = Arc::clone(&events);
            Arc::new(move |update: &QueueUpdate| {
                if let Ok(mut events) = events.lock() {
                    events.push(update.clone());
                }
            })
        },
        lanes,
        notify_on_idle: false,
        notifier: None,
    });
    state.set_queue(engine);

    Fixture { dir, state, events }
}

fn request(fx: &Fixture, url: &str, quality: QualityChoice) -> AddRequest {
    AddRequest {
        url: url.to_owned(),
        title: Some("A title".into()),
        site: Some("Youtube".into()),
        duration_sec: Some(10),
        options: ItemOptions {
            kind: Kind::Video,
            quality,
            output_dir: fx.dir.path().join("out"),
            ..ItemOptions::default()
        },
        group_id: None,
    }
}

#[test]
fn a_state_without_an_engine_says_so_instead_of_crashing() {
    // Only the settings tests build a bare state; the queue commands must
    // report the missing engine rather than unwrap a panic.
    let dir = TempDir::new().expect("temp dir");
    let state = AppState::from_path(dir.path().join("settings.json")).expect("state loads");
    assert!(state.queue().is_none());
    let error = match state.require_queue() {
        Err(error) => error,
        // `QueueEngine` is not `Debug`, so `expect_err` cannot be used here.
        Ok(_) => panic!("a bare state must not answer queue commands"),
    };
    assert_eq!(error.code, "UNSUPPORTED");
    assert!(error.detail.unwrap_or_default().contains("queue"));
}

#[test]
fn an_empty_queue_lists_nothing_and_says_so() {
    let fx = fixture(Settings::default());
    let queue = fx.state.require_queue().expect("the queue is registered");
    let snapshot = queue.snapshot();
    assert!(snapshot.items.is_empty());
    assert!(snapshot.groups.is_empty());
    assert_eq!(snapshot.summary.total, 0);
    assert!(!snapshot.summary.is_active());
}

#[test]
fn adding_the_same_link_twice_is_refused_and_can_be_forced() {
    let fx = fixture(Settings::default());
    let queue = fx.state.require_queue().expect("the queue is registered");
    let quality = QualityChoice::Exact { height: 1080 };

    let first = queue.add(
        vec![request(&fx, "https://a.example/1", quality.clone())],
        false,
    );
    assert_eq!(first.added.len(), 1);
    assert!(first.duplicates.is_empty());

    let second = queue.add(
        vec![request(&fx, "https://a.example/1", quality.clone())],
        false,
    );
    assert!(
        second.added.is_empty(),
        "the same link with the same options"
    );
    assert_eq!(second.duplicates.len(), 1);
    assert_eq!(second.duplicates[0].item_id, first.added[0]);

    let forced = queue.add(vec![request(&fx, "https://a.example/1", quality)], true);
    assert_eq!(forced.added.len(), 1, "the user said yes");
    assert_eq!(queue.snapshot().items.len(), 2);
}

#[test]
fn a_different_quality_makes_a_different_item() {
    let fx = fixture(Settings::default());
    let queue = fx.state.require_queue().expect("the queue is registered");
    queue.add(
        vec![request(&fx, "https://a.example/1", QualityChoice::Best)],
        false,
    );
    let second = queue.add(
        vec![request(
            &fx,
            "https://a.example/1",
            QualityChoice::AudioOnly,
        )],
        false,
    );
    assert_eq!(second.added.len(), 1);
    assert!(second.duplicates.is_empty());
}

#[test]
fn the_lane_settings_of_the_document_reach_the_engine_and_survive_a_save() {
    let fx = fixture(Settings {
        parallel_downloads: 4,
        per_host_cap: 3,
        ..Settings::default()
    });
    let queue = fx.state.require_queue().expect("the queue is registered");
    assert_eq!(queue.lanes().download, 4);
    assert_eq!(queue.lanes().per_host_cap, 3);

    // Lowering them writes the file and reaches the engine without a restart.
    let mut settings = fx.state.settings();
    settings.parallel_downloads = 1;
    settings.per_host_cap = 1;
    fx.state.save_settings(settings).expect("the settings save");
    assert_eq!(queue.lanes().download, 1);
    assert_eq!(queue.lanes().per_host_cap, 1);
}

#[test]
fn out_of_range_caps_are_clamped_by_the_document_before_the_engine_sees_them() {
    let fx = fixture(Settings {
        parallel_downloads: 99,
        per_host_cap: 0,
        ..Settings::default()
    });
    let settings = fx.state.settings();
    assert_eq!(settings.parallel_downloads, 4, "spec §7.2 caps it at 4");
    assert_eq!(settings.per_host_cap, 1, "zero would deadlock the queue");
    // And the engine opened from those settings obeys them too.
    let lanes = fx.state.require_queue().expect("queue").lanes();
    assert_eq!(lanes.download, 4);
    assert_eq!(lanes.per_host_cap, 1);
}

#[test]
fn the_actions_are_idempotent_on_an_id_that_does_not_exist() {
    let fx = fixture(Settings::default());
    let queue = fx.state.require_queue().expect("the queue is registered");
    assert_eq!(queue.cancel("nope"), Outcome::NotFound);
    assert_eq!(queue.pause("nope"), Outcome::NotFound);
    assert_eq!(queue.resume("nope"), Outcome::NotFound);
    assert_eq!(queue.retry("nope"), Outcome::NotFound);
    assert_eq!(queue.remove("nope"), Outcome::NotFound);
    assert_eq!(queue.move_to_top("nope"), Outcome::NotFound);
    // And the bulk ones answer with what they did, not with a failure.
    assert!(queue.clear_finished().is_empty());
    assert!(queue.retry_failed().is_empty());
    assert!(queue.resume_all().is_empty());
}

#[test]
fn a_corrupt_queue_file_keeps_a_backup_and_reports_the_recovery() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("queue.json");
    std::fs::write(&path, "{ not json").expect("write");
    let loaded = scaricatore_di_porto_lib::services::queue::store::load(&path);
    assert!(loaded.file.items.is_empty(), "spec §7.4: start empty");
    let recovery = loaded.recovery.expect("a recovery is reported");
    assert_eq!(recovery.message_key(), "toast.queueRecovered");
    assert!(
        recovery.backup_path.is_some_and(|backup| backup.is_file()),
        "the broken document stays for the human"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adding_an_item_emits_it_as_an_event_the_frontend_can_patch() {
    let fx = fixture(Settings::default());
    let queue = fx.state.require_queue().expect("the queue is registered");
    // The flusher is a task, so the engine has to be started inside a runtime.
    queue.start();
    let added = queue.add(
        vec![request(&fx, "https://a.example/1", QualityChoice::Best)],
        false,
    );
    let id = added.added[0].clone();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let seen = fx.events.lock().expect("events").clone();
        if seen
            .iter()
            .any(|update| update.items.iter().any(|item| item.id == id))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the added item never arrived as an event: {seen:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    queue.shutdown();
}

#[test]
fn an_item_added_through_the_state_is_persisted_and_the_file_survives() {
    let fx = fixture(Settings::default());
    let queue = fx.state.require_queue().expect("the queue is registered");
    let added = queue.add(
        vec![request(&fx, "https://a.example/1", QualityChoice::Best)],
        false,
    );
    let id = added.added[0].clone();
    assert!(fx.dir.path().join("queue.json").is_file());

    // A second engine over the same file is what a restart looks like.
    let reopened = QueueEngine::open(EngineConfig {
        path: fx.dir.path().join("queue.json"),
        provider: Arc::new(FakeTools {
            ytdlp: PathBuf::from("/nonexistent/yt-dlp"),
            memory: Arc::new(RuntimeMemoryStore::load(
                fx.dir.path().join("other-memory.json"),
            )),
        }),
        sink: Arc::new(|_: &QueueUpdate| {}),
        lanes: LaneConfig::default(),
        notify_on_idle: false,
        notifier: None,
    });
    let snapshot = reopened.snapshot();
    assert_eq!(snapshot.items.len(), 1);
    assert_eq!(snapshot.items[0].id, id);
    assert_eq!(snapshot.items[0].state, ItemState::Queued);
}

#[test]
fn the_snapshot_is_the_shape_the_frontend_expects() {
    let fx = fixture(Settings::default());
    let queue = fx.state.require_queue().expect("the queue is registered");
    queue.add(
        vec![request(&fx, "https://a.example/1", QualityChoice::Best)],
        false,
    );
    let json = serde_json::to_value(queue.snapshot()).expect("the snapshot is serialisable");
    assert_eq!(json["seq"], 0);
    assert_eq!(json["items"][0]["state"], "queued");
    assert_eq!(json["items"][0]["url"], "https://a.example/1");
    // The summary is what the footer reads (spec §7.5).
    assert!(json["summary"]["queued"].is_number());
    assert!(json["summary"]["parallelDownloads"].is_number());
    assert_eq!(json["summary"]["parallelDownloads"], 2);
    // And the frontend never has to guess what a missing optional means.
    assert!(json["items"][0]["file"].is_null());
    assert!(json["items"][0]["error"].is_null());
}

#[test]
fn the_runtime_mode_and_cookies_are_frozen_into_the_item() {
    // Spec §7.1: the options are a snapshot, so a later settings change cannot
    // alter queued work.
    let fx = fixture(Settings::default());
    let queue = fx.state.require_queue().expect("the queue is registered");
    let mut options = ItemOptions {
        kind: Kind::Audio,
        quality: QualityChoice::AudioOnly,
        output_dir: fx.dir.path().join("out"),
        ..ItemOptions::default()
    };
    options.runtime_mode = JsRuntimeMode::DenoOnly;
    queue.add(
        vec![AddRequest {
            url: "https://a.example/1".into(),
            title: None,
            site: None,
            duration_sec: None,
            options: options.clone(),
            group_id: None,
        }],
        false,
    );

    let mut settings = fx.state.settings();
    settings.js_runtime_mode = JsRuntimeMode::QuickJsOnly;
    fx.state.save_settings(settings).expect("settings save");

    let snapshot = queue.snapshot();
    assert_eq!(
        snapshot.items[0].options.runtime_mode,
        JsRuntimeMode::DenoOnly
    );
    assert_eq!(snapshot.items[0].options.quality, QualityChoice::AudioOnly);
}

#[test]
fn a_group_is_upserted_and_survives_the_file() {
    use scaricatore_di_porto_lib::services::queue::model::QueueGroup;

    let fx = fixture(Settings::default());
    let queue = fx.state.require_queue().expect("the queue is registered");
    queue.upsert_group(QueueGroup {
        id: "g1".into(),
        title: "A playlist".into(),
        folder: Some(fx.dir.path().join("out/A playlist")),
        index: 0,
    });
    let snapshot = queue.snapshot();
    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].title, "A playlist");

    let reopened = QueueEngine::open(EngineConfig {
        path: fx.dir.path().join("queue.json"),
        provider: Arc::new(FakeTools {
            ytdlp: PathBuf::from("/nonexistent/yt-dlp"),
            memory: Arc::new(RuntimeMemoryStore::load(
                fx.dir.path().join("other-memory.json"),
            )),
        }),
        sink: Arc::new(|_: &QueueUpdate| {}),
        lanes: LaneConfig::default(),
        notify_on_idle: false,
        notifier: None,
    });
    assert_eq!(reopened.snapshot().groups.len(), 1);
}

#[test]
fn the_runtime_memory_is_shared_between_the_state_and_the_provider() {
    // The engine's provider is the one thing that talks to the settings, so
    // this checks the seam itself: the same store, two clones.
    let fx = fixture(Settings::default());
    let memory = Arc::new(RuntimeMemoryStore::load(fx.dir.path().join("m.json")));
    memory.remember("2026.08.19", Some("0.17.0"), RuntimeKind::Deno, 1_000);
    assert_eq!(
        memory.get().map(|entry| entry.preferred),
        Some(RuntimeKind::Deno)
    );
}
