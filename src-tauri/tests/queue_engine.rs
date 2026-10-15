//! M2 integration tests: the queue engine driving the **fake** yt-dlp
//! (spec §7, §15).
//!
//! Everything here is offline and runs on the three OSes. The fake binary
//! prints the same `SDP_*` templates the real pinned yt-dlp was measured to
//! print (2026-10-14), takes as long as the test asks (`FAKE_YTDLP_SLEEP_MS`)
//! and records when each download started and ended in its own log, which is
//! what makes "2 in parallel" and "never more than 2 per host" measurable
//! rather than hopeful.
//!
//! Run the suite as `cargo test`: the fake is a Cargo *example*, and
//! `cargo test --test queue_engine` alone would not build it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use scaricatore_di_porto_lib::error::ErrorInfo;
use scaricatore_di_porto_lib::services::inspect::{ResolvedRuntime, Runtimes};
use scaricatore_di_porto_lib::services::job::JobEnvironment;
use scaricatore_di_porto_lib::services::jsruntime::{JsRuntimeMode, RuntimeMemoryStore};
use scaricatore_di_porto_lib::services::queue::engine::{
    AddRequest, BoxFuture, EngineConfig, IdleNotifier, QueueEngine, ToolProvider,
};
use scaricatore_di_porto_lib::services::queue::events::{QueueSnapshot, QueueUpdate};
use scaricatore_di_porto_lib::services::queue::model::{
    CookieChoice, ItemOptions, ItemState, Kind, LaneConfig, QualityChoice, QueueSummary,
};
use scaricatore_di_porto_lib::state::Settings;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

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

/// One test's world: a private copy of the fake, an output folder and a
/// `queue.json`.
struct Fixture {
    dir: TempDir,
    ytdlp: PathBuf,
    out: PathBuf,
    queue_path: PathBuf,
    memory: Arc<RuntimeMemoryStore>,
    updates: Arc<Mutex<Vec<QueueUpdate>>>,
    notifications: Arc<Mutex<Vec<QueueSummary>>>,
}

fn fixture() -> Fixture {
    let dir = TempDir::new().expect("temp dir");
    let mut ytdlp = dir.path().join("yt-dlp");
    if cfg!(windows) {
        ytdlp.set_extension("exe");
    }
    fs::copy(fake_ytdlp_source(), &ytdlp).expect("copy the fake");
    scaricatore_di_porto_lib::services::tools::make_executable(&ytdlp).expect("chmod +x");

    let out = dir.path().join("out");
    fs::create_dir_all(&out).expect("mkdir out");

    Fixture {
        queue_path: dir.path().join("queue.json"),
        memory: Arc::new(RuntimeMemoryStore::load(
            dir.path().join("runtime-memory.json"),
        )),
        updates: Arc::new(Mutex::new(Vec::new())),
        notifications: Arc::new(Mutex::new(Vec::new())),
        ytdlp,
        out,
        dir,
    }
}

/// A provider that hands every item the fake binary.
struct FakeTools {
    ytdlp: PathBuf,
    memory: Arc<RuntimeMemoryStore>,
}

impl ToolProvider for FakeTools {
    fn environment(&self, options: ItemOptions) -> BoxFuture<Result<JobEnvironment, ErrorInfo>> {
        fs::create_dir_all(&options.output_dir).ok();
        let environment = JobEnvironment {
            ytdlp: self.ytdlp.clone(),
            // Presence is all the pipeline checks.
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

struct RecordingNotifier {
    seen: Arc<Mutex<Vec<QueueSummary>>>,
}

impl IdleNotifier for RecordingNotifier {
    fn notify(&self, summary: &QueueSummary) {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(summary.clone());
        }
    }
}

impl Fixture {
    fn engine_with(&self, lanes: LaneConfig, notify_on_idle: bool) -> QueueEngine {
        let seen = Arc::clone(&self.updates);
        let notifier = Arc::new(RecordingNotifier {
            seen: Arc::clone(&self.notifications),
        });
        let engine = QueueEngine::open(EngineConfig {
            path: self.queue_path.clone(),
            provider: Arc::new(FakeTools {
                ytdlp: self.ytdlp.clone(),
                memory: Arc::clone(&self.memory),
            }),
            sink: Arc::new(move |update: &QueueUpdate| {
                if let Ok(mut seen) = seen.lock() {
                    seen.push(update.clone());
                }
            }),
            lanes,
            notify_on_idle,
            notifier: Some(notifier),
        });
        engine.start();
        engine
    }

    fn engine(&self) -> QueueEngine {
        self.engine_with(LaneConfig::default(), false)
    }

    /// An engine whose tasks were never started: nothing runs, so what it says
    /// about an item is only what `add()` put there.
    fn idle_engine(&self) -> QueueEngine {
        QueueEngine::open(EngineConfig {
            path: self.queue_path.clone(),
            provider: Arc::new(FakeTools {
                ytdlp: self.ytdlp.clone(),
                memory: Arc::clone(&self.memory),
            }),
            sink: Arc::new(|_: &QueueUpdate| {}),
            lanes: LaneConfig::default(),
            notify_on_idle: false,
            notifier: None,
        })
    }

    fn request(&self, url: &str) -> AddRequest {
        AddRequest {
            url: url.to_owned(),
            title: Some(format!("title of {url}")),
            site: Some("Youtube".into()),
            duration_sec: Some(10),
            options: ItemOptions {
                kind: Kind::Video,
                quality: QualityChoice::MaxHeight { height: 1080 },
                output_dir: self.out.clone(),
                cookies: CookieChoice::None,
                runtime_mode: JsRuntimeMode::Auto,
            },
            group_id: None,
        }
    }

    /// `start <millis> <url>` / `end <millis> <url>` records of the fake, as
    /// `(url, started_at, ended_at)` intervals.
    fn runs(&self) -> Vec<(String, Option<u128>, Option<u128>)> {
        let raw =
            fs::read_to_string(self.dir.path().join("fake-ytdlp-runs.log")).unwrap_or_default();
        let mut order: Vec<String> = Vec::new();
        let mut starts: std::collections::HashMap<String, u128> = std::collections::HashMap::new();
        let mut ends: std::collections::HashMap<String, u128> = std::collections::HashMap::new();
        for line in raw.lines() {
            let mut parts = line.split_whitespace();
            let kind = parts.next().unwrap_or_default();
            let Some(at) = parts.next().and_then(|raw| raw.parse::<u128>().ok()) else {
                continue;
            };
            let url = parts.next().unwrap_or_default().to_owned();
            if url.is_empty() || kind != "start" && kind != "end" {
                // A garbled record (two processes, one buffer) must not be
                // mistaken for a real start or end.
                continue;
            }
            if !starts.contains_key(&url) {
                order.push(url.clone());
            }
            if kind == "start" {
                starts.insert(url, at);
            } else if let Some(start) = starts.get(&url) {
                ends.insert(url.clone(), (*start).max(at));
            }
        }
        order
            .into_iter()
            .map(|url| {
                let started = starts.get(&url).copied();
                let ended = ends.get(&url).copied();
                (url, started, ended)
            })
            .collect()
    }

    /// The largest number of downloads that were running at the same instant,
    /// counting only the ones whose URL contains `needle`.
    ///
    /// Counted by **interval overlap with a common window**, not by walking the
    /// events: two starts and ends can share a millisecond, and an event-ordered
    /// walk would then count a start after an end that happened first.
    fn max_concurrency(&self, needle: Option<&str>) -> usize {
        let intervals: Vec<(u128, u128)> = self
            .runs()
            .into_iter()
            .filter(|(url, _, _)| needle.is_none_or(|needle| url.contains(needle)))
            // A start with no end is a killed download: it cannot be placed in
            // time, and counting it as "still running" would inflate the peak.
            .filter_map(|(_, started, ended)| Some((started?, ended?)))
            .collect();
        intervals
            .iter()
            .map(|(start, _)| {
                // The instant `COMMON_MS` after this download started: every
                // interval alive there shares at least that much life.
                let at = start + COMMON_MS;
                intervals
                    .iter()
                    .filter(|(other_start, other_end)| *other_start <= at && at <= *other_end)
                    .count()
            })
            .max()
            .unwrap_or(0)
    }
}

/// The smallest stretch of time a group of downloads must share to be counted
/// as concurrent.
///
/// The fake writes its `end` record and **then** exits, so the real end of a
/// process is a few milliseconds after the record (measured here: 1–9 ms).
/// Counting that sliver as an overlap would let a correct lane of 2 look like 3
/// on a fast machine and pass on a slow one. Asking for 10 ms of **common** life
/// measures what the record can actually prove: N downloads were all running at
/// the same instant, for long enough to notice.
const COMMON_MS: u128 = 10;

async fn wait_for(
    engine: &QueueEngine,
    what: &str,
    predicate: impl Fn(&QueueSnapshot) -> bool,
    within: Duration,
) -> QueueSnapshot {
    let deadline = Instant::now() + within;
    loop {
        let snapshot = engine.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out after {within:?} waiting for {what}; the queue is {states:?}",
            states = snapshot
                .items
                .iter()
                .map(|item| (item.id.as_str(), item.state, item.progress.label.clone()))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn all_settled(snapshot: &QueueSnapshot) -> bool {
    snapshot.items.iter().all(|item| item.state.is_parked())
}

fn state_of(snapshot: &QueueSnapshot, id: &str) -> Option<ItemState> {
    snapshot
        .items
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.state)
}

/// A `serde_json::Value` of an id, to compare an item read off the file.
fn json_id(id: &str) -> serde_json::Value {
    serde_json::Value::String(id.to_owned())
}

/// The state **`queue.json`** gives an item, read from the disk and not from the
/// engine, waiting for the engine to catch up.
///
/// The wait is not a fudge: an in-memory state change becomes visible to
/// `snapshot()` a moment before the write that records it (the state lock is
/// released before the file is touched, and never held across a write), and a
/// polling test that woke up in that microsecond would read the *previous*
/// document. So this waits for the file to leave `queued`, and the caller
/// asserts the exact value it finds — which is only meaningful because the
/// item's download is long enough for the read to land inside it.
async fn state_after_the_start(fx: &Fixture, id: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let last = read_state_on_disk(fx, id);
        if last.as_deref().is_some_and(|state| state != "queued") {
            return last.expect("just checked");
        }
        assert!(
            Instant::now() < deadline,
            "the file never recorded {id} as running; it says {last:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Wait for something the engine's tasks deliver to the sink — an event, a
/// notification, a toast — the same way [`wait_for`] waits for a state: by
/// polling, with a deadline that fails the test with the reason.
async fn wait_until(
    fx: &Fixture,
    what: &str,
    within: Duration,
    ready: impl Fn(&[QueueUpdate]) -> bool,
) {
    let deadline = Instant::now() + within;
    loop {
        let seen = fx.updates.lock().expect("updates").clone();
        if ready(&seen) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out after {within:?} waiting for {what}; {} events so far",
            seen.len()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// The recovery toast of spec §7.4 reaching the sink: the flusher is a task, so
/// "it arrived" is a wait, not an instant.
async fn wait_for_recovery_toast(fx: &Fixture) {
    wait_until(
        fx,
        "the recovery toast to reach the UI",
        Duration::from_secs(10),
        |seen| {
            seen.iter().any(|update| {
                update
                    .toast
                    .as_ref()
                    .is_some_and(|toast| toast.message_key == "toast.queueRecovered")
            })
        },
    )
    .await;
}

/// The first event that carries a given item, the way the frontend's store
/// patches one row.
async fn wait_for_item_event(fx: &Fixture, id: &str) {
    let wanted = id.to_owned();
    wait_until(
        fx,
        "the item to arrive over the event channel",
        Duration::from_secs(10),
        move |seen| {
            seen.iter().any(|update| {
                update.removed.contains(&wanted)
                    || update.items.iter().any(|item| item.id == wanted)
            })
        },
    )
    .await;
}

/// Wait until an engine is really quiet after `shutdown()`: the item is either
/// parked (it was running and has now stopped) or still `Queued`, which is not
/// parked but can no longer move — the scheduler that would have started it is
/// gone. A test that returns before this point leaves a task writing into a
/// `TempDir` that is about to be deleted, which is how a whole suite ends up
/// with a flaky neighbour.
async fn wait_until_quiet(engine: &QueueEngine, id: &str) {
    wait_for(
        engine,
        "the item to settle",
        |snapshot| {
            state_of(snapshot, id)
                .is_some_and(|state| state.is_parked() || state == ItemState::Queued)
        },
        Duration::from_secs(20),
    )
    .await;
}

fn read_state_on_disk(fx: &Fixture, id: &str) -> Option<String> {
    let raw = fs::read_to_string(&fx.queue_path).ok()?;
    let document: serde_json::Value = serde_json::from_str(&raw).ok()?;
    document["items"]
        .as_array()?
        .iter()
        .find(|item| item["id"] == json_id(id))?
        .get("state")?
        .as_str()
        .map(str::to_owned)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ten_links_run_two_at_a_time_and_all_finish() {
    let fx = fixture();
    let engine = fx.engine_with(
        LaneConfig {
            download: 2,
            ..LaneConfig::default()
        },
        false,
    );

    let urls: Vec<String> = (0..10)
        .map(|n| format!("https://site{n}.example/slow-{n}"))
        .collect();
    let outcome = engine.add(urls.iter().map(|url| fx.request(url)).collect(), false);
    assert_eq!(outcome.added.len(), 10);

    let snapshot = wait_for(
        &engine,
        "all ten items to settle",
        |snapshot| snapshot.items.len() == 10 && all_settled(snapshot),
        Duration::from_secs(60),
    )
    .await;

    assert_eq!(snapshot.summary.done, 10, "every item finished");
    assert_eq!(snapshot.summary.failed, 0);
    assert!(
        fx.max_concurrency(None) <= 2,
        "the download lane must cap concurrency at 2, saw {} in {:?}",
        fx.max_concurrency(None),
        fx.runs()
    );
    assert!(
        fx.max_concurrency(None) >= 2,
        "two must actually have overlapped, saw {} in {:?}",
        fx.max_concurrency(None),
        fx.runs()
    );

    // The output of each item is its own file, from SDP_FILE.
    for item in &snapshot.items {
        let file = item.file.as_ref().expect("a finished item has a file");
        assert!(Path::new(file).is_file(), "{} is missing", file.display());
        assert!(
            file.to_string_lossy().contains("slow-"),
            "{} should carry the URL it came from",
            file.display()
        );
    }

    // And the temp directories of finished items are gone (spec §7.3).
    let leftovers: Vec<PathBuf> = fs::read_dir(&fx.out)
        .expect("read out")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".sdp-tmp-"))
        })
        .collect();
    assert!(leftovers.is_empty(), "temp dirs left behind: {leftovers:?}");

    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_parallel_download_means_one_at_a_time() {
    let fx = fixture();
    let engine = fx.engine_with(
        LaneConfig {
            download: 1,
            ..LaneConfig::default()
        },
        false,
    );
    let urls: Vec<String> = (0..3)
        .map(|n| format!("https://site{n}.example/slow-{n}"))
        .collect();
    engine.add(urls.iter().map(|url| fx.request(url)).collect(), false);

    wait_for(
        &engine,
        "all three items to settle",
        |snapshot| snapshot.items.len() == 3 && all_settled(snapshot),
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(fx.max_concurrency(None), 1, "the lane was asked for 1");
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_per_host_cap_stops_the_third_download_of_one_site() {
    let fx = fixture();
    // A wide lane, so the host cap is what decides (spec §6.4).
    let engine = fx.engine_with(
        LaneConfig {
            download: 4,
            per_host_cap: 2,
            ..LaneConfig::default()
        },
        false,
    );

    // Three from one host and one from another: the other one is not held back.
    let urls = [
        "https://www.youtube.com/slow-a",
        "https://youtube.com/slow-b",
        "https://youtube.com/slow-c",
        "https://vimeo.com/slow-d",
    ];
    engine.add(urls.iter().map(|url| fx.request(url)).collect(), false);

    wait_for(
        &engine,
        "all four items to settle",
        |snapshot| snapshot.items.len() == 4 && all_settled(snapshot),
        Duration::from_secs(60),
    )
    .await;

    assert_eq!(
        fx.max_concurrency(Some("youtube.com")),
        2,
        "the per-host cap is about the host: never three from YouTube at once \
         (spec §6.4); the runs were {:?}",
        fx.runs()
    );
    assert!(
        fx.max_concurrency(None) >= 3,
        "and the other host is not held back by it: the lane allowed a third \
         download to run, saw {} in {:?}",
        fx.max_concurrency(None),
        fx.runs()
    );
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_keeps_the_item_and_resume_finishes_it() {
    let fx = fixture();
    let engine = fx.engine();
    let id = engine
        .add(vec![fx.request("https://a.example/slow-one")], false)
        .added[0]
        .clone();

    // Wait until it is really running, then pause.
    wait_for(
        &engine,
        "the item to start",
        |snapshot| state_of(snapshot, &id) == Some(ItemState::Downloading),
        Duration::from_secs(20),
    )
    .await;
    assert_eq!(
        engine.pause(&id),
        scaricatore_di_porto_lib::services::queue::Outcome::Ok
    );

    let snapshot = wait_for(
        &engine,
        "the item to be paused",
        |snapshot| state_of(snapshot, &id) == Some(ItemState::Paused),
        Duration::from_secs(20),
    )
    .await;
    let paused = snapshot
        .items
        .iter()
        .find(|item| item.id == id)
        .expect("the item is still there");
    assert!(
        paused.error.is_none(),
        "a pause is not a failure (spec §7.3)"
    );
    assert_eq!(paused.paused_from, Some(ItemState::Downloading));

    // Nothing else starts while the queue is paused.
    engine.pause_all();
    assert!(engine.snapshot().summary.paused_all);

    let resumed = engine.resume_all();
    assert_eq!(resumed, vec![id.clone()]);
    let snapshot = wait_for(
        &engine,
        "the resumed item to finish",
        |snapshot| state_of(snapshot, &id) == Some(ItemState::Done),
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(snapshot.summary.failed, 0);
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_stops_the_item_and_leaves_no_temp_directory() {
    let fx = fixture();
    let engine = fx.engine();
    let id = engine
        .add(vec![fx.request("https://a.example/slow-cancel")], false)
        .added[0]
        .clone();

    wait_for(
        &engine,
        "the item to start",
        |snapshot| state_of(snapshot, &id) == Some(ItemState::Downloading),
        Duration::from_secs(20),
    )
    .await;
    engine.cancel(&id);

    wait_for(
        &engine,
        "the item to be cancelled",
        |snapshot| state_of(snapshot, &id) == Some(ItemState::Cancelled),
        Duration::from_secs(20),
    )
    .await;

    // Its temp directory goes; the finished outputs of other items would not.
    let temp = fx.out.join(format!(".sdp-tmp-{id}"));
    assert!(!temp.exists(), "the temp dir must be removed (spec §7.3)");
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_transient_network_failure_is_retried_by_the_queue() {
    let fx = fixture();
    let engine = fx.engine();
    let id = engine
        .add(vec![fx.request("https://a.example/netfail-once-1")], false)
        .added[0]
        .clone();

    let snapshot = wait_for(
        &engine,
        "the item to recover on its own",
        |snapshot| state_of(snapshot, &id) == Some(ItemState::Done),
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(snapshot.summary.failed, 0);
    let item = snapshot
        .items
        .iter()
        .find(|item| item.id == id)
        .expect("there");
    assert_eq!(
        item.auto_retries, 1,
        "one automatic retry was spent (spec §7.3)"
    );
    assert!(item.retry_at.is_none());
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_permanent_failure_is_never_retried_by_itself() {
    let fx = fixture();
    let engine = fx.engine();
    let id = engine
        .add(
            vec![fx.request("https://a.example/netfail-always-1")],
            false,
        )
        .added[0]
        .clone();

    // A network class that keeps failing: two automatic retries, then it stops
    // and waits for the user.
    let snapshot = wait_for(
        &engine,
        "the item to give up",
        |snapshot| state_of(snapshot, &id) == Some(ItemState::Failed),
        Duration::from_secs(90),
    )
    .await;
    let item = snapshot
        .items
        .iter()
        .find(|item| item.id == id)
        .expect("there");
    assert_eq!(
        item.auto_retries,
        scaricatore_di_porto_lib::services::queue::model::MAX_AUTO_RETRIES
    );

    // A manual retry works and is the user's to make.
    assert_eq!(
        engine.retry(&id),
        scaricatore_di_porto_lib::services::queue::Outcome::Ok
    );
    assert_eq!(
        engine
            .snapshot()
            .items
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.auto_retries),
        Some(0),
        "a manual retry gives the budget back"
    );
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_private_video_fails_once_and_is_not_retried() {
    let fx = fixture();
    let engine = fx.engine();
    let id = engine
        .add(vec![fx.request("https://a.example/error-auth-1")], false)
        .added[0]
        .clone();

    let snapshot = wait_for(
        &engine,
        "the item to fail",
        |snapshot| state_of(snapshot, &id) == Some(ItemState::Failed),
        Duration::from_secs(30),
    )
    .await;
    let item = snapshot
        .items
        .iter()
        .find(|item| item.id == id)
        .expect("there");
    assert_eq!(item.auto_retries, 0, "AUTH is a permanent class");
    let error = item.error.as_ref().expect("a classified error");
    assert_eq!(error.code, "AUTH");
    assert_eq!(error.message_key, "error.ytdlp.auth");
    assert!(
        !error.detail.clone().unwrap_or_default().is_empty(),
        "the raw detail stays one click away (spec §8.5)"
    );
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_queue_survives_a_restart_and_the_interrupted_item_can_resume() {
    let fx = fixture();

    // First "launch": two items, one of them stopped mid-flight.
    let (first_id, engine) = {
        let engine = fx.engine_with(
            LaneConfig {
                download: 1,
                ..LaneConfig::default()
            },
            false,
        );
        let ids = engine
            .add(
                vec![
                    // A download with a **clock** the test sets, so the
                    // assertions below are never racing a 400 ms fake that can
                    // finish underneath them. Eight seconds is the margin: it
                    // has to survive a loaded machine that gets no CPU for a
                    // while, and the resume at the end waits it out.
                    fx.request("https://a.example/veryslow-8000"),
                    fx.request("https://b.example/veryslow-8000"),
                ],
                false,
            )
            .added;
        wait_for(
            &engine,
            "the first item to start",
            |snapshot| state_of(snapshot, &ids[0]) == Some(ItemState::Downloading),
            Duration::from_secs(20),
        )
        .await;
        // The file, read **while the tool runs**: this is what a crash (`kill
        // -9`, a power cut) would leave behind, and it is the reason the state
        // is written before the process is spawned (D91). A file that still
        // said "queued" here would come back from the next launch as an item
        // nobody had started, and the queue would download it again by itself.
        assert_eq!(
            state_after_the_start(&fx, &ids[0]).await,
            "downloading",
            "a running item is on disk as running, never as queued"
        );
        // This is what closing the app mid-download looks like: cancel the
        // token of everything still running and stop.
        engine.shutdown();
        (ids[0].clone(), engine)
    };

    // Closing the app takes a moment: the kill is `SIGTERM` first, and the item
    // records itself as cancelled only once the process is really gone. Waiting
    // for that is also what a real exit hook would have to do.
    wait_for(
        &engine,
        "the killed item to settle",
        |snapshot| state_of(snapshot, &first_id).is_some_and(ItemState::is_parked),
        Duration::from_secs(20),
    )
    .await;
    drop(engine);

    // Second "launch" over the same file.
    let engine = fx.engine();
    let snapshot = engine.snapshot();
    assert_eq!(snapshot.items.len(), 2, "nothing was lost on the way out");
    let recovered = state_of(&snapshot, &first_id).expect("the item is there");
    assert!(
        matches!(
            recovered,
            ItemState::Cancelled | ItemState::Interrupted | ItemState::Paused
        ),
        "an item that was running comes back parked, never running again on its \\
         own: {recovered:?}"
    );

    // Whatever it came back as, the user decides: Resume for interrupted and
    // paused, Retry for cancelled (spec §7.3).
    engine.resume_all();
    if matches!(recovered, ItemState::Cancelled) {
        engine.retry(&first_id);
    }
    let snapshot = wait_for(
        &engine,
        "the recovered item to finish after Resume",
        |snapshot| {
            matches!(
                state_of(snapshot, &first_id),
                Some(ItemState::Done | ItemState::Failed)
            )
        },
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(
        state_of(&snapshot, &first_id),
        Some(ItemState::Done),
        "the file is on disk: {:?}",
        snapshot
            .items
            .iter()
            .map(|item| (item.id.as_str(), item.state, item.error.clone()))
            .collect::<Vec<_>>()
    );
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_item_that_was_running_when_the_app_died_comes_back_interrupted() {
    let fx = fixture();

    // What a crash leaves behind: the file still says "downloading", because
    // nothing ran to record the opposite. Spec §7.4 is about this exact case,
    // and a graceful shutdown never produces it, so it is written here.
    let item = scaricatore_di_porto_lib::services::queue::model::QueueItem {
        id: "q-crashed".into(),
        url: "https://a.example/slow-one".into(),
        state: ItemState::Downloading,
        options: ItemOptions {
            kind: Kind::Video,
            quality: QualityChoice::MaxHeight { height: 1080 },
            output_dir: fx.out.clone(),
            ..ItemOptions::default()
        },
        ..Default::default()
    };
    let file = scaricatore_di_porto_lib::services::queue::store::QueueFile {
        items: vec![item],
        ..Default::default()
    };
    file.save(&fx.queue_path).expect("write the queue file");

    let engine = fx.engine();
    let snapshot = engine.snapshot();
    assert_eq!(
        state_of(&snapshot, "q-crashed"),
        Some(ItemState::Interrupted),
        "an item that was running is interrupted, not lost and not re-run blindly"
    );
    assert_eq!(
        engine.owned_temp_names(),
        vec!["q-crashed".to_owned()],
        "its temp directory is kept so Resume can continue (spec §4.10)"
    );

    // Spec §7.4: it waits for the user. It must not start by itself while the
    // app is still deciding what to show.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        state_of(&engine.snapshot(), "q-crashed"),
        Some(ItemState::Interrupted),
        "an interrupted item is not re-run behind the user's back"
    );

    // And resuming really finishes it.
    assert_eq!(engine.resume_all(), vec!["q-crashed".to_owned()]);
    let snapshot = wait_for(
        &engine,
        "the interrupted item to finish after Resume",
        |snapshot| state_of(snapshot, "q-crashed") == Some(ItemState::Done),
        Duration::from_secs(60),
    )
    .await;
    assert!(snapshot.items.iter().any(|item| {
        item.file
            .as_deref()
            .is_some_and(|file| Path::new(file).is_file())
    }));
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_queue_file_survives_a_corruption_with_a_backup() {
    let fx = fixture();
    {
        let engine = fx.engine();
        let id = engine
            .add(vec![fx.request("https://a.example/slow-one")], false)
            .added[0]
            .clone();
        // The item has to be finished before the file is corrupted. `shutdown`
        // asks the running task to stop, and that task still writes the file
        // once the process is really gone: a real app exits the process and
        // nothing writes afterwards, a test cannot. Corrupting the document
        // while a write is still in flight would be testing a race instead of
        // the recovery.
        engine.shutdown();
        wait_until_quiet(&engine, &id).await;
    }
    fs::write(&fx.queue_path, "{ not json at all").expect("corrupt it");

    let engine = fx.engine();
    let snapshot = engine.snapshot();
    assert!(snapshot.items.is_empty(), "the queue starts empty");
    assert!(
        fx.dir.path().join("queue.json.bak").is_file(),
        "the broken document is kept for the human (spec §7.4)"
    );

    // And the toast that says so reached the sink: the UI has to be told, not
    // left to find out.
    wait_for_recovery_toast(&fx).await;
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_footer_counters_and_the_idle_notification_follow_the_queue() {
    let fx = fixture();
    let engine = fx.engine_with(LaneConfig::default(), true);
    let urls: Vec<String> = (0..3)
        .map(|n| format!("https://site{n}.example/slow-{n}"))
        .collect();
    engine.add(urls.iter().map(|url| fx.request(url)).collect(), false);

    let snapshot = wait_for(
        &engine,
        "the queue to go idle",
        |snapshot| snapshot.summary.done == 3 && !snapshot.summary.is_active(),
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(snapshot.summary.running, 0);
    assert_eq!(snapshot.summary.queued, 0);
    assert_eq!(snapshot.summary.total, 3);

    // The counters are visible a moment before the notification is sent: the
    // state is updated under the lock and the notifier is called after it, so
    // a test that read the notifications as soon as the queue looked idle would
    // sometimes see none. Wait for the one, then give the others a chance to
    // show up — "exactly one" is the claim, and a claim about absence has to
    // wait too.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if !fx.notifications.lock().expect("notifications").is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the queue went idle and said nothing (spec §7.7)"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    // The flusher's own task is what would send a second one, and it has a
    // coalescing delay; a queue that is already idle cannot re-trigger the
    // transition, so this only has to outlast the in-flight batch.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let notifications = fx.notifications.lock().expect("notifications").clone();
    assert_eq!(
        notifications.len(),
        1,
        "one notification for the transition to idle (spec §7.7), not one per item"
    );
    assert_eq!(notifications[0].done, 3);
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clear_finished_empties_the_queue_and_removes_are_idempotent() {
    let fx = fixture();
    let engine = fx.engine();
    let urls: Vec<String> = (0..2)
        .map(|n| format!("https://site{n}.example/slow-{n}"))
        .collect();
    let ids = engine
        .add(urls.iter().map(|url| fx.request(url)).collect(), false)
        .added;
    wait_for(
        &engine,
        "both items to finish",
        |snapshot| snapshot.items.len() == 2 && all_settled(snapshot),
        Duration::from_secs(60),
    )
    .await;

    let cleared = engine.clear_finished();
    assert_eq!(cleared.len(), 2);
    assert!(engine.snapshot().items.is_empty());
    assert_eq!(engine.clear_finished().len(), 0, "nothing left to clear");

    // Removing an id that is gone is not an error (spec §15: a stale click).
    assert_eq!(
        engine.remove(&ids[0]),
        scaricatore_di_porto_lib::services::queue::Outcome::NotFound
    );
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reorder_decides_who_runs_first() {
    let fx = fixture();
    let engine = fx.engine_with(
        LaneConfig {
            download: 1,
            ..LaneConfig::default()
        },
        false,
    );
    let urls: Vec<String> = (0..3)
        .map(|n| format!("https://site{n}.example/slow-{n}"))
        .collect();
    let ids = engine
        .add(urls.iter().map(|url| fx.request(url)).collect(), false)
        .added;

    // The queue is already busy with the first item; the rest may start in any
    // order, so the reorder is asserted on the order itself.
    engine.reorder(&[ids[2].clone(), ids[1].clone(), ids[0].clone()]);
    let order: Vec<String> = engine
        .snapshot()
        .items
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(order, vec![ids[2].clone(), ids[1].clone(), ids[0].clone()]);

    engine.move_to_top(&ids[1]);
    let order: Vec<String> = engine
        .snapshot()
        .items
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(order[0], ids[1]);
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_events_carry_the_items_and_the_summary_and_are_coalesced() {
    let fx = fixture();
    let engine = fx.engine_with(
        LaneConfig {
            download: 2,
            ..LaneConfig::default()
        },
        false,
    );
    let urls: Vec<String> = (0..4)
        .map(|n| format!("https://site{n}.example/slow-{n}"))
        .collect();
    engine.add(urls.iter().map(|url| fx.request(url)).collect(), false);
    wait_for(
        &engine,
        "the four items to settle",
        |snapshot| snapshot.items.len() == 4 && all_settled(snapshot),
        Duration::from_secs(60),
    )
    .await;
    // The flusher is a task: "the last batch has been sent" is a wait, not an
    // instant, and a fixed sleep would be a flake on a loaded machine.
    wait_until(
        &fx,
        "the last event to report a settled queue",
        Duration::from_secs(10),
        |seen| {
            seen.last()
                .is_some_and(|update| update.summary.done == 4 && !update.summary.is_active())
        },
    )
    .await;

    let seen = fx.updates.lock().expect("updates").clone();
    assert!(!seen.is_empty(), "something was emitted");
    assert!(
        seen.iter().all(|update| update.seq > 0),
        "every event carries its sequence"
    );
    // The summary travels with the events (spec §7.5).
    let last = seen.last().expect("the last event");
    assert!(last.summary.total >= 4 || last.summary.total == 0);

    // The per-item rate: an item that ran for ~1.6 s cannot have produced more
    // than a few dozen events, and the ones it produced are all its own.
    let per_item = seen
        .iter()
        .flat_map(|update| update.items.iter().map(|item| item.id.as_str()))
        .count();
    assert!(per_item < 400, "{per_item} item updates is not coalesced");
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_settings_of_the_lanes_reach_the_engine() {
    let fx = fixture();
    // 4 parallel downloads from the settings (spec §7.2, §11.8).
    let settings = Settings {
        parallel_downloads: 4,
        per_host_cap: 4,
        ..Settings::default()
    };
    let engine = fx.engine_with(settings.lane_config(), settings.notify_on_idle);
    assert_eq!(engine.lanes().download, 4);
    assert_eq!(engine.lanes().per_host_cap, 4);

    let urls: Vec<String> = (0..4)
        .map(|n| format!("https://site{n}.example/slow-{n}"))
        .collect();
    engine.add(urls.iter().map(|url| fx.request(url)).collect(), false);
    wait_for(
        &engine,
        "the four items to settle",
        |snapshot| snapshot.items.len() == 4 && all_settled(snapshot),
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(fx.max_concurrency(None), 4, "four in parallel, as asked");

    // Changing it mid-flight takes effect on the next planning pass.
    engine.configure(
        LaneConfig {
            download: 1,
            per_host_cap: 4,
            ..LaneConfig::default()
        },
        false,
    );
    assert_eq!(engine.lanes().download, 1);
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn out_of_range_lane_settings_are_clamped_rather_than_deadlocking() {
    let fx = fixture();
    let engine = fx.engine_with(
        LaneConfig {
            download: 0,
            encode: 0,
            transcribe: 0,
            per_host_cap: 0,
        },
        false,
    );
    let lanes = engine.lanes();
    assert_eq!(lanes.download, 1, "zero would deadlock the queue");
    assert!(lanes.per_host_cap >= 1);

    let id = engine
        .add(vec![fx.request("https://a.example/slow-one")], false)
        .added[0]
        .clone();
    let snapshot = wait_for(
        &engine,
        "the item to finish anyway",
        |snapshot| state_of(snapshot, &id) == Some(ItemState::Done),
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(snapshot.summary.done, 1);
    engine.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_added_item_is_persisted_patched_and_gone_from_a_restart() {
    let fx = fixture();

    // The frontend's three sources, in the order it meets them: the initial
    // `queue_list`, the events, and the file after a restart.
    let queue = fx.engine();
    let added = queue.add(vec![fx.request("https://a.example/slow-one")], false);
    let id = added.added[0].clone();
    assert!(fx.dir.path().join("queue.json").is_file());

    // 1. `queue_list` right after the add (what the store calls to fill the list).
    let snapshot = queue.snapshot();
    assert_eq!(snapshot.items.len(), 1);
    assert_eq!(snapshot.items[0].id, id);
    assert_eq!(snapshot.summary.total, 1);
    // The scheduler is already free to pick the item up, so "queued" is only
    // true for as long as it has not: the invariant is that the footer counts
    // it exactly once, in whichever bucket it happens to be.
    assert_eq!(
        snapshot.summary.queued + snapshot.summary.running,
        1,
        "the new item must be counted: {snapshot:?}"
    );

    // 2. The event that patches the row.
    wait_for_item_event(&fx, &id).await;

    // 3. The file: a fresh engine over it finds the same item.
    let reopened = fx.idle_engine();
    assert_eq!(reopened.snapshot().items.len(), 1);
    assert_eq!(reopened.snapshot().items[0].id, id);

    // Let the running item really stop before the fixture's directory goes:
    // a task that writes into a deleted temp dir is noise in the log of the
    // next run, and "the test ended" is not the same moment as "the process is
    // gone".
    queue.shutdown();
    wait_until_quiet(&queue, &id).await;
}

#[test]
fn the_snapshot_is_the_shape_the_frontend_expects() {
    // No scheduler, no flusher: this is about the *shape* of the wire, and a
    // running engine would be racing the very fields under test.
    let fx = fixture();
    let queue = fx.idle_engine();
    queue.add(vec![fx.request("https://a.example/slow-one")], false);
    let json = serde_json::to_value(queue.snapshot()).expect("the snapshot is serialisable");
    // `seq` is 0 because nothing has been emitted yet; the frontend only needs
    // it to be there and to grow.
    assert_eq!(json["seq"], 0);
    assert_eq!(json["items"][0]["state"], "queued");
    assert_eq!(json["items"][0]["url"], "https://a.example/slow-one");
    assert_eq!(json["items"][0]["options"]["kind"], "video");
    assert_eq!(json["items"][0]["options"]["quality"]["kind"], "maxHeight");
    // Every optional is an explicit null, never a missing key: the frontend's
    // normaliser would default it, but the wire has one shape.
    for key in [
        "file",
        "runtime",
        "requestedHeight",
        "actualHeight",
        "error",
        "retryAt",
    ] {
        assert!(json["items"][0][key].is_null(), "{key} must be null");
    }
    assert!(
        json["items"][0]["logTail"]
            .as_array()
            .expect("an array")
            .is_empty(),
        "a new item has no log yet, but the key is there"
    );
    assert_eq!(json["items"][0]["autoRetries"], 0);
    assert!(json["summary"]["parallelDownloads"].is_number());
    assert_eq!(json["summary"]["perHostCap"], 2);
    assert!(json["groups"].as_array().expect("an array").is_empty());
}

#[test]
fn the_options_are_frozen_into_the_item_so_later_settings_cannot_change_them() {
    use scaricatore_di_porto_lib::services::jsruntime::JsRuntimeMode;

    let fx = fixture();
    let queue = fx.idle_engine();
    let mut options = ItemOptions {
        kind: scaricatore_di_porto_lib::services::queue::model::Kind::Audio,
        quality: QualityChoice::AudioOnly,
        output_dir: fx.out.clone(),
        ..ItemOptions::default()
    };
    options.runtime_mode = JsRuntimeMode::DenoOnly;
    let added = queue.add(
        vec![AddRequest {
            url: "https://a.example/song".into(),
            title: None,
            site: None,
            duration_sec: None,
            options,
            group_id: None,
        }],
        false,
    );
    let id = added.added[0].clone();

    // Whatever happens to the settings afterwards (this test cannot change
    // them: it has no settings store, and that is the point — the item carries
    // its own snapshot), the item is what the runner will use.
    let item = queue
        .snapshot()
        .items
        .into_iter()
        .find(|item| item.id == id)
        .expect("the item");
    assert_eq!(item.options.runtime_mode, JsRuntimeMode::DenoOnly);
    assert_eq!(item.options.quality, QualityChoice::AudioOnly);
    let spec = item.options.to_download_spec();
    assert!(spec.quality.extracts_audio());
    assert!(!spec.needs_merge(), "an audio-only item does not merge");
}

#[test]
fn a_group_is_upserted_and_survives_the_file() {
    use scaricatore_di_porto_lib::services::queue::model::QueueGroup;

    let fx = fixture();
    let queue = fx.idle_engine();
    let group = QueueGroup {
        id: "g1".into(),
        title: "A playlist".into(),
        folder: Some(fx.out.join("A playlist")),
        index: 0,
    };
    queue.upsert_group(group.clone());
    queue.upsert_group(QueueGroup {
        title: "A playlist, renamed".into(),
        ..group
    });

    let reopened = fx.idle_engine();
    let groups = reopened.snapshot().groups;
    assert_eq!(groups.len(), 1, "an upsert replaces, it does not append");
    assert_eq!(groups[0].title, "A playlist, renamed");
}
