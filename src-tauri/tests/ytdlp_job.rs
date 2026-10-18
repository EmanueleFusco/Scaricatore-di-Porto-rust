//! M1 integration tests: inspection and the single-video job, driven by the
//! fake yt-dlp (spec §15).
//!
//! Everything here is **offline** and runs on the three OSes: the fake binary
//! answers `-J` with canned JSON and downloads by printing the same
//! `SDP_DL`/`SDP_PP`/`SDP_FILE` templates the real pinned yt-dlp was measured
//! to print (2026-10-14). The scenarios with real network access live in the
//! self-test command (spec §5.3.6) and are run by hand or in CI, never here.
//!
//! Run the suite as `cargo test`: the fake is a Cargo *example*, and
//! `cargo test --test ytdlp_job` alone would not build it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use scaricatore_di_porto_lib::services::inspect::{
    self, InspectEnvironment, Inspected, ResolvedRuntime, Runtimes,
};
use scaricatore_di_porto_lib::services::job::{self, JobEnvironment, JobOutcome, JobRequest};
use scaricatore_di_porto_lib::services::jsruntime::{
    JsRuntimeMode, RuntimeKind, RuntimeMemoryStore,
};
use scaricatore_di_porto_lib::services::ytdlp::{
    CookieSource, DownloadSpec, FormatChoice, JobDirs,
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// The fake binary, isolated per test
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

struct Fixture {
    _dir: TempDir,
    ytdlp: PathBuf,
    home: PathBuf,
    temp: PathBuf,
    log: PathBuf,
    pid_file: PathBuf,
    memory_path: PathBuf,
}

/// Copy the fake into a directory of its own, so its log and pid file belong to
/// one test only.
fn fixture() -> Fixture {
    let dir = TempDir::new().expect("temp dir");
    let mut ytdlp = dir.path().join("yt-dlp");
    if cfg!(windows) {
        ytdlp.set_extension("exe");
    }
    fs::copy(fake_ytdlp_source(), &ytdlp).expect("copy the fake");
    scaricatore_di_porto_lib::services::tools::make_executable(&ytdlp).expect("chmod +x");

    let home = dir.path().join("out");
    let temp = home.join(".sdp-tmp-job");
    fs::create_dir_all(&home).expect("mkdir out");

    Fixture {
        log: dir.path().join("fake-ytdlp.log"),
        pid_file: dir.path().join("fake-ytdlp.pid"),
        memory_path: dir.path().join("runtime-memory.json"),
        _dir: dir,
        ytdlp,
        home,
        temp,
    }
}

impl Fixture {
    fn runtimes(&self, mode: JsRuntimeMode, with_deno: bool) -> Runtimes {
        Runtimes {
            mode,
            qjs: Some(ResolvedRuntime {
                path: self.ytdlp.clone(),
                version: Some("0.17.0".into()),
            }),
            deno: with_deno.then(|| ResolvedRuntime {
                path: self.ytdlp.clone(),
                version: Some("2.9.7".into()),
            }),
            ytdlp_version: "2026.08.19".into(),
        }
    }

    fn memory(&self) -> Arc<RuntimeMemoryStore> {
        Arc::new(RuntimeMemoryStore::load(self.memory_path.clone()))
    }

    fn env(
        &self,
        mode: JsRuntimeMode,
        with_deno: bool,
        memory: &Arc<RuntimeMemoryStore>,
    ) -> JobEnvironment {
        JobEnvironment {
            ytdlp: self.ytdlp.clone(),
            ffmpeg: Some(self.ytdlp.clone()), // presence is all the runner checks
            ffprobe: None,
            runtimes: self.runtimes(mode, with_deno),
            memory: Arc::clone(memory),
            private_temp: None,
            youtube_clients: None,
        }
    }

    fn request(&self, url: &str) -> JobRequest {
        self.request_with_stall(url, job::STALL_TIMEOUT)
    }

    fn request_with_stall(&self, url: &str, stall: Duration) -> JobRequest {
        JobRequest {
            url: url.to_owned(),
            spec: DownloadSpec::new(
                scaricatore_di_porto_lib::services::ytdlp::JobKind::Video,
                FormatChoice::MaxHeight { height: 1080 },
                CookieSource::None,
            ),
            dirs: JobDirs {
                home: self.home.clone(),
                temp: self.temp.clone(),
            },
            stall_timeout: stall,
        }
    }

    fn log_lines(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn collecting_sink() -> (job::ProgressSink, Arc<Mutex<Vec<String>>>) {
    let labels = Arc::new(Mutex::new(Vec::new()));
    let sink: job::ProgressSink = {
        let labels = Arc::clone(&labels);
        Arc::new(
            move |progress: &scaricatore_di_porto_lib::services::ytdlp::JobProgress| {
                if let Ok(mut list) = labels.lock() {
                    list.push(progress.label.clone());
                }
            },
        )
    };
    (sink, labels)
}

/// Unwrap an inspection that was expected to finish.
fn inspected(outcome: inspect::InspectOutcome) -> Box<Inspected> {
    match outcome {
        inspect::InspectOutcome::Inspected { inspected, .. } => inspected,
        inspect::InspectOutcome::Cancelled => panic!("the inspection was cancelled"),
    }
}

// ---------------------------------------------------------------------------
// Process liveness (same shape as process_tree.rs: only this part is per-OS)
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // SAFETY: signal 0 checks existence and permission, it delivers nothing.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_ACCESS_RIGHTS};
    const PROCESS_QUERY_LIMITED_INFORMATION: PROCESS_ACCESS_RIGHTS = PROCESS_ACCESS_RIGHTS(0x1000);
    // SAFETY: opening for query only; the handle is closed by OwnedHandle.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) };
    match handle {
        Ok(handle) => {
            let _owned = unsafe { OwnedHandle::from_raw_handle(handle.0) };
            true
        }
        Err(_) => false,
    }
}

fn wait_until_gone(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !process_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !process_alive(pid)
}

// ---------------------------------------------------------------------------
// Download jobs
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_successful_job_reports_the_sdp_file_and_the_progress_labels() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = fixture.env(JsRuntimeMode::Auto, false, &memory);
    let (sink, labels) = collecting_sink();

    let outcome = job::run_job(
        &env,
        &fixture.request("https://youtu.be/good"),
        CancellationToken::new(),
        sink,
    )
    .await;

    let JobOutcome::Done { file, runtime, .. } = outcome else {
        panic!("expected success, got {outcome:?}");
    };
    assert_eq!(runtime, RuntimeKind::QuickJs);
    assert_eq!(file, fixture.home.join("Fake title [fakeid].mkv"));
    assert!(file.is_file(), "the reported path must exist");

    let labels = labels.lock().expect("labels").clone();
    for expected in ["preparing", "video", "audio", "merging"] {
        assert!(
            labels.contains(&expected.to_owned()),
            "missing {expected} in {labels:?}"
        );
    }
    // Two streams were reported (Video 1/2, Audio 2/2).
    let log = fixture.log_lines();
    assert_eq!(log.len(), 1, "one attempt, one invocation: {log:?}");
    assert!(log[0].contains("runtime=quickjs"));
    assert!(log[0].contains("format=yes"));
}

/// The D114 report, reproduced on every OS by the fake (see
/// `print_file_line` in the fake): with stdout piped, the tool writes the
/// *console* code page, so a path with accented letters arrives without them
/// and the app concludes "yt-dlp reported … but the file is not there" about a
/// file that is sitting in the output folder, playable. It never happened on
/// Linux, where the pipe is UTF-8, and it always happened on Windows for the
/// same links — the titles decide, not the site.
///
/// The tool is told to write UTF-8 (spec §4.5), so the path it prints really
/// names the file it wrote.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_accented_final_path_survives_the_tools_console_code_page() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = fixture.env(JsRuntimeMode::Auto, false, &memory);
    let (sink, _labels) = collecting_sink();

    let outcome = job::run_job(
        &env,
        &fixture.request("https://a.example/accent-città-perché"),
        CancellationToken::new(),
        sink,
    )
    .await;

    let JobOutcome::Done { file, .. } = outcome else {
        panic!("the accented path must not be reported as missing: {outcome:?}");
    };
    assert!(
        file.to_string_lossy().contains('à'),
        "the accent has to survive the trip: {file:?}"
    );
    assert!(file.is_file(), "the reported path must exist");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_job_kills_the_tree_and_removes_nothing_else() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = fixture.env(JsRuntimeMode::Auto, false, &memory);
    let (sink, labels) = collecting_sink();
    let token = CancellationToken::new();

    let request = fixture.request("https://youtu.be/cancel-me");
    let run = tokio::spawn({
        let token = token.clone();
        async move { job::run_job(&env, &request, token, sink).await }
    });

    // Wait until the fake is really running (it printed its pid).
    let pid_deadline = Instant::now() + Duration::from_secs(5);
    let pid = loop {
        if let Ok(raw) = fs::read_to_string(&fixture.pid_file)
            && let Ok(pid) = raw.trim().parse::<u32>()
        {
            break pid;
        }
        assert!(Instant::now() < pid_deadline, "the fake never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    // And until it produced the pre-cancel progress lines.
    let progress_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if labels.lock().expect("labels").contains(&"video".to_owned()) {
            break;
        }
        assert!(Instant::now() < progress_deadline, "no progress arrived");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    token.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("the job must end promptly")
        .expect("the task must not panic");
    assert!(matches!(outcome, JobOutcome::Cancelled), "got {outcome:?}");

    assert!(
        wait_until_gone(pid),
        "the yt-dlp process survived the cancel (pid {pid})"
    );
    assert!(
        !fixture.home.join("Fake title [fakeid].mkv").exists(),
        "a cancelled job must not leave an output"
    );

    // The caller removes the job temp dir (spec §7.3); the output folder and any
    // finished file stay.
    fs::create_dir_all(&fixture.temp).expect("mkdir temp");
    job::cleanup_job_dir(&fixture.temp);
    assert!(!fixture.temp.exists());
    assert!(fixture.home.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_degraded_download_retries_with_deno_instead_of_silently_keeping_360p() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = fixture.env(JsRuntimeMode::Auto, true, &memory);
    let (sink, _labels) = collecting_sink();

    // The fake "downloads" the muxed 360p slowly on the first call; the app
    // must cancel it at the SDP_FORMAT line and retry with Deno.
    let outcome = job::run_job(
        &env,
        &fixture.request("https://youtu.be/degraded-download"),
        CancellationToken::new(),
        sink,
    )
    .await;

    let JobOutcome::Done {
        runtime,
        requested_height,
        actual_height,
        ..
    } = outcome
    else {
        panic!("expected success, got {outcome:?}");
    };
    assert_eq!(
        runtime,
        RuntimeKind::Deno,
        "the runtime used is the second one"
    );
    assert_eq!(requested_height, Some(1080));
    assert_eq!(actual_height, Some(1080), "not the degraded 360p");
    let log = fixture.log_lines();
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[0].contains("runtime=quickjs"));
    assert!(log[1].contains("runtime=deno"));
    assert_eq!(
        memory.get().expect("remembered").preferred,
        RuntimeKind::Deno
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_degraded_download_without_deno_retries_the_same_runtime() {
    let fixture = fixture();
    let memory = fixture.memory();
    // No Deno: the owner's setup. The transient challenge is retried with
    // QuickJS, which is what a manual re-paste used to fix.
    let env = fixture.env(JsRuntimeMode::Auto, false, &memory);
    let (sink, _labels) = collecting_sink();

    let outcome = job::run_job(
        &env,
        &fixture.request("https://youtu.be/degraded-download"),
        CancellationToken::new(),
        sink,
    )
    .await;

    let JobOutcome::Done {
        runtime,
        actual_height,
        ..
    } = outcome
    else {
        panic!("expected success, got {outcome:?}");
    };
    assert_eq!(runtime, RuntimeKind::QuickJs);
    assert_eq!(actual_height, Some(1080));
    let log = fixture.log_lines();
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[0].contains("runtime=quickjs"));
    assert!(log[1].contains("runtime=quickjs"));
    assert!(memory.get().is_none(), "nothing to remember");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_degraded_download_that_never_recovers_fails_without_saving_anything() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = fixture.env(JsRuntimeMode::Auto, false, &memory);
    let (sink, _labels) = collecting_sink();

    let outcome = job::run_job(
        &env,
        &fixture.request("https://youtu.be/degraded-always"),
        CancellationToken::new(),
        sink,
    )
    .await;

    let JobOutcome::Failed(error) = outcome else {
        panic!("expected a failure, got {outcome:?}");
    };
    assert_eq!(error.code.as_str(), "YT_CHALLENGE");
    assert_eq!(fixture.log_lines().len(), 2, "one retry, then give up");
    let leftovers: Vec<String> = std::fs::read_dir(&fixture.home)
        .expect("out dir")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_merged_result_at_a_lower_height_is_not_retried() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = fixture.env(JsRuntimeMode::Auto, true, &memory);
    let (sink, _labels) = collecting_sink();

    // The fake merges two streams (id `137+140`) but only at 720p: the streams
    // were found, so the video simply has no 1080p. Retrying would download the
    // same thing again; the honest answer is "done, 720p".
    let outcome = job::run_job(
        &env,
        &fixture.request("https://youtu.be/merged-lower"),
        CancellationToken::new(),
        sink,
    )
    .await;

    let JobOutcome::Done {
        runtime,
        requested_height,
        actual_height,
        ..
    } = outcome
    else {
        panic!("expected success, got {outcome:?}");
    };
    assert_eq!(
        runtime,
        RuntimeKind::QuickJs,
        "no runtime swap was warranted"
    );
    assert_eq!(requested_height, Some(1080));
    assert_eq!(actual_height, Some(720));
    assert_eq!(fixture.log_lines().len(), 1, "one attempt only");
    assert!(memory.get().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_format_failure_retries_once_without_the_selector() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = fixture.env(JsRuntimeMode::Auto, false, &memory);
    let (sink, _labels) = collecting_sink();

    let outcome = job::run_job(
        &env,
        &fixture.request("https://youtu.be/format-fail"),
        CancellationToken::new(),
        sink,
    )
    .await;

    assert!(
        matches!(outcome, JobOutcome::Done { .. }),
        "got {outcome:?}"
    );
    let log = fixture.log_lines();
    assert_eq!(log.len(), 2, "one failure, one retry: {log:?}");
    assert!(log[0].contains("format=yes"));
    assert!(
        log[1].contains("format=no"),
        "the retry must omit -f (spec §8.2): {log:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_challenge_falls_back_to_deno_once_and_the_outcome_is_remembered() {
    let fixture = fixture();
    let memory = fixture.memory();
    let (sink, _labels) = collecting_sink();

    let env = fixture.env(JsRuntimeMode::Auto, true, &memory);
    let outcome = job::run_job(
        &env,
        &fixture.request("https://youtu.be/challenge"),
        CancellationToken::new(),
        sink,
    )
    .await;
    let JobOutcome::Done { runtime, .. } = outcome else {
        panic!("expected the Deno attempt to succeed, got {outcome:?}");
    };
    assert_eq!(
        runtime,
        RuntimeKind::Deno,
        "the UI must show the runtime used"
    );

    let log = fixture.log_lines();
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[0].contains("runtime=quickjs"));
    assert!(log[1].contains("runtime=deno"));

    // spec §5.3.3: remembered for the same pair of versions.
    let reloaded = Arc::new(RuntimeMemoryStore::load(fixture.memory_path.clone()));
    let remembered = reloaded.get().expect("a decision was written");
    assert_eq!(remembered.preferred, RuntimeKind::Deno);
    assert_eq!(remembered.ytdlp_version, "2026.08.19");
    assert_eq!(remembered.qjs_version.as_deref(), Some("0.17.0"));

    // The next job starts with Deno directly: one invocation, no QuickJS.
    let (sink, _labels) = collecting_sink();
    let env = fixture.env(JsRuntimeMode::Auto, true, &reloaded);
    let outcome = job::run_job(
        &env,
        &fixture.request("https://youtu.be/challenge"),
        CancellationToken::new(),
        sink,
    )
    .await;
    assert!(matches!(outcome, JobOutcome::Done { .. }));
    let log = fixture.log_lines();
    assert_eq!(log.len(), 3, "{log:?}");
    assert!(
        log[2].contains("runtime=deno"),
        "the remembered decision must be used: {log:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_challenge_switches_runtime_instead_of_failing() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = fixture.env(JsRuntimeMode::Auto, true, &memory);
    let (sink, _labels) = collecting_sink();

    // The fake produces nothing at all for QuickJS; the watchdog has to end the
    // attempt (spec §5.3.2 "no progress for 90 s", shortened here).
    //
    // 5 s, not the 300 ms this test started with: the watchdog has to outlast
    // the **launch** of the copied fake, not only its stall. On the Windows
    // runner the first execution of that freshly written file is delayed by the
    // loader and by Defender's real-time scan, and a 300 ms budget killed the
    // process before it could write its first log line — the behaviour was right
    // (it fell back to Deno, which the outcome asserts), the log just had one
    // line instead of two. Production waits 90 s, so this is a margin for the
    // test, never a behaviour.
    let stall = Duration::from_secs(5);
    let outcome = job::run_job(
        &env,
        &fixture.request_with_stall("https://youtu.be/stall", stall),
        CancellationToken::new(),
        sink,
    )
    .await;

    let JobOutcome::Done { runtime, .. } = outcome else {
        panic!("expected the Deno attempt to succeed, got {outcome:?}");
    };
    assert_eq!(runtime, RuntimeKind::Deno);
    let log = fixture.log_lines();
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[0].contains("runtime=quickjs"));
    assert!(log[1].contains("runtime=deno"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_deno_is_reported_instead_of_being_attempted() {
    let fixture = fixture();
    let memory = fixture.memory();
    // Auto mode, but Deno is not installed: the "ask once" case.
    let env = fixture.env(JsRuntimeMode::Auto, false, &memory);
    let (sink, _labels) = collecting_sink();

    let outcome = job::run_job(
        &env,
        &fixture.request("https://youtu.be/challenge"),
        CancellationToken::new(),
        sink,
    )
    .await;

    let JobOutcome::Failed(error) = outcome else {
        panic!("expected a failure, got {outcome:?}");
    };
    assert_eq!(error.code.as_str(), "DENO_MISSING");
    let log = fixture.log_lines();
    assert_eq!(log.len(), 1, "Deno was never attempted: {log:?}");
    assert!(log[0].contains("runtime=quickjs"));
    assert!(memory.get().is_none(), "nothing to remember");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quickjs_only_never_falls_back() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = fixture.env(JsRuntimeMode::QuickJsOnly, true, &memory);
    let (sink, _labels) = collecting_sink();

    let outcome = job::run_job(
        &env,
        &fixture.request("https://youtu.be/challenge"),
        CancellationToken::new(),
        sink,
    )
    .await;

    let JobOutcome::Failed(error) = outcome else {
        panic!("expected a failure, got {outcome:?}");
    };
    assert_eq!(error.code.as_str(), "YT_CHALLENGE");
    assert_eq!(fixture.log_lines().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_ffmpeg_fails_before_anything_is_spawned() {
    let fixture = fixture();
    let memory = fixture.memory();
    let mut env = fixture.env(JsRuntimeMode::Auto, false, &memory);
    env.ffmpeg = None;
    let (sink, _labels) = collecting_sink();

    let outcome = job::run_job(
        &env,
        &fixture.request("https://youtu.be/good"),
        CancellationToken::new(),
        sink,
    )
    .await;

    let JobOutcome::Failed(error) = outcome else {
        panic!("expected a failure, got {outcome:?}");
    };
    assert_eq!(error.code.as_str(), "FFMPEG_MISSING");
    assert!(
        !fixture.log.exists(),
        "no process may be started without ffmpeg"
    );
}

// ---------------------------------------------------------------------------
// Inspection
// ---------------------------------------------------------------------------

fn inspect_env(
    fixture: &Fixture,
    mode: JsRuntimeMode,
    with_deno: bool,
    memory: &Arc<RuntimeMemoryStore>,
) -> InspectEnvironment {
    InspectEnvironment {
        ytdlp: fixture.ytdlp.clone(),
        cookies: CookieSource::None,
        runtimes: fixture.runtimes(mode, with_deno),
        memory: Arc::clone(memory),
        private_temp: None,
        youtube_clients: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_inspection_parses_a_video_and_filters_storyboards() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = inspect_env(&fixture, JsRuntimeMode::Auto, false, &memory);

    let outcome = inspect::inspect(&env, "https://youtu.be/good", CancellationToken::new()).await;
    let inspected = inspected(outcome);
    let Inspected::Video(video) = &*inspected else {
        panic!("expected a video, got {inspected:?}");
    };
    assert_eq!(video.id, "fakeid");
    assert_eq!(video.title, "Fake title");
    assert_eq!(video.heights, vec![1080, 720, 360]);
    assert!(video.has_video && video.has_audio);
    assert_eq!(inspected.resolution_labels(), vec!["1080p", "720p", "360p"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_degraded_inspection_retries_itself_and_returns_the_full_list() {
    let fixture = fixture();
    let memory = fixture.memory();
    // No Deno: the extra attempt has to use the same runtime, because a failed
    // challenge solve is transient (it is what a re-paste fixes by hand).
    let env = inspect_env(&fixture, JsRuntimeMode::Auto, false, &memory);

    let outcome = inspect::inspect(
        &env,
        "https://youtu.be/degraded-once",
        CancellationToken::new(),
    )
    .await;
    let inspected = inspected(outcome);
    let Inspected::Video(video) = &*inspected else {
        panic!("expected a video, got {inspected:?}");
    };
    assert_eq!(
        video.heights,
        vec![1080, 720, 360],
        "the retry fixed the list"
    );
    assert!(!video.degraded, "and the UI is told the list is complete");
    let log = fixture.log_lines();
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[0].contains("runtime=quickjs"));
    assert!(
        log[1].contains("runtime=quickjs"),
        "same runtime when no Deno"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_degraded_inspection_with_deno_switches_runtime() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = inspect_env(&fixture, JsRuntimeMode::Auto, true, &memory);

    let outcome = inspect::inspect(
        &env,
        "https://youtu.be/degraded-once",
        CancellationToken::new(),
    )
    .await;
    let inspect::InspectOutcome::Inspected {
        inspected, runtime, ..
    } = outcome
    else {
        panic!("cancelled");
    };
    assert_eq!(
        runtime,
        RuntimeKind::Deno,
        "the fallback did the second try"
    );
    let Inspected::Video(video) = &*inspected else {
        panic!("expected a video");
    };
    assert_eq!(video.heights, vec![1080, 720, 360]);
    assert!(!video.degraded);
    let log = fixture.log_lines();
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[0].contains("runtime=quickjs"));
    assert!(log[1].contains("runtime=deno"));
    assert_eq!(
        memory.get().expect("remembered").preferred,
        RuntimeKind::Deno
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_degraded_inspection_that_never_recovers_is_flagged() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = inspect_env(&fixture, JsRuntimeMode::Auto, false, &memory);

    let outcome = inspect::inspect(
        &env,
        "https://youtu.be/degraded-always",
        CancellationToken::new(),
    )
    .await;
    let inspected = inspected(outcome);
    let Inspected::Video(video) = &*inspected else {
        panic!("expected a video, got {inspected:?}");
    };
    // The list is short, but it is *there*, and the flag says it may be
    // incomplete instead of pretending it is the whole truth.
    assert_eq!(video.heights, vec![360]);
    assert!(video.degraded);
    assert_eq!(
        fixture.log_lines().len(),
        2,
        "one extra attempt, not a loop"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_playlist_url_returns_the_stub() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = inspect_env(&fixture, JsRuntimeMode::Auto, false, &memory);

    let outcome = inspect::inspect(
        &env,
        "https://www.youtube.com/playlist?list=PL1",
        CancellationToken::new(),
    )
    .await;
    let inspected = inspected(outcome);
    let Inspected::Playlist(playlist) = &*inspected else {
        panic!("expected a playlist, got {inspected:?}");
    };
    assert_eq!(playlist.title, "A playlist");
    assert_eq!(playlist.count, Some(3));
    assert_eq!(playlist.entries.len(), 3);
    assert_eq!(playlist.entries[0].url, "https://www.youtube.com/watch?v=a");
    assert_eq!(playlist.entries[0].title, "Video A");
    assert!(playlist.entries[0].available);
    assert!(
        !playlist.entries[1].available,
        "private entries must be marked unavailable (spec §6.4)"
    );
    assert!(playlist.entries[2].available);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_video_with_list_url_respects_no_playlist_option() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = inspect_env(&fixture, JsRuntimeMode::Auto, false, &memory);

    let opts = inspect::InspectOptions {
        no_playlist: true,
        ..Default::default()
    };
    let outcome = inspect::inspect_with_options(
        &env,
        "https://www.youtube.com/watch?v=fakeid&list=PL1",
        &opts,
        CancellationToken::new(),
    )
    .await;
    let inspected = inspected(outcome);
    let Inspected::Video(video) = &*inspected else {
        panic!("expected single video when no_playlist=true, got {inspected:?}");
    };
    assert_eq!(video.id, "fakeid");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_search_or_feed_url_is_refused_without_spawning_ytdlp() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = inspect_env(&fixture, JsRuntimeMode::Auto, false, &memory);

    let outcome = inspect::inspect(
        &env,
        "https://www.youtube.com/results?search_query=rust",
        CancellationToken::new(),
    )
    .await;
    let inspected = inspected(outcome);
    let Inspected::Error(info) = &*inspected else {
        panic!("expected unsupported error for search page, got {inspected:?}");
    };
    assert_eq!(info.code, "UNSUPPORTED");
    assert!(
        !fixture.log.exists(),
        "search/feed pages must be refused before spawning yt-dlp (spec §6.4)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_auth_failure_is_classified_with_the_cookies_hint() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = inspect_env(&fixture, JsRuntimeMode::Auto, false, &memory);

    let outcome =
        inspect::inspect(&env, "https://youtu.be/auth-fail", CancellationToken::new()).await;
    let inspected = inspected(outcome);
    let Inspected::Error(info) = &*inspected else {
        panic!("expected an error, got {inspected:?}");
    };
    assert_eq!(info.code, "AUTH");
    assert_eq!(info.message_key, "error.ytdlp.auth");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_degraded_extraction_falls_back_to_deno_and_remembers_it() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = inspect_env(&fixture, JsRuntimeMode::Auto, true, &memory);

    let outcome =
        inspect::inspect(&env, "https://youtu.be/degraded", CancellationToken::new()).await;
    let inspected = inspected(outcome);
    let Inspected::Video(video) = &*inspected else {
        panic!("expected a video, got {inspected:?}");
    };
    assert_eq!(
        video.heights,
        vec![1080, 720, 360],
        "the Deno attempt must be the one that produced the result"
    );
    let log = fixture.log_lines();
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[0].contains("runtime=quickjs"));
    assert!(log[1].contains("runtime=deno"));
    assert_eq!(
        memory.get().expect("remembered").preferred,
        RuntimeKind::Deno
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_challenge_failure_in_inspection_falls_back_to_deno() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = inspect_env(&fixture, JsRuntimeMode::Auto, true, &memory);

    let outcome =
        inspect::inspect(&env, "https://youtu.be/challenge", CancellationToken::new()).await;
    let inspected = inspected(outcome);
    assert!(
        matches!(&*inspected, Inspected::Video(_)),
        "got {inspected:?}"
    );
    assert_eq!(fixture.log_lines().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_inspection_is_cancellable() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = inspect_env(&fixture, JsRuntimeMode::Auto, false, &memory);
    let token = CancellationToken::new();

    let run = tokio::spawn({
        let token = token.clone();
        async move { inspect::inspect(&env, "https://youtu.be/slow-inspect", token).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    token.cancel();

    let outcome = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("cancel must end the inspection")
        .expect("no panic");
    assert!(matches!(outcome, inspect::InspectOutcome::Cancelled));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn garbage_from_the_tool_is_an_error_not_a_panic() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = inspect_env(&fixture, JsRuntimeMode::Auto, false, &memory);

    let outcome =
        inspect::inspect(&env, "https://youtu.be/garbage", CancellationToken::new()).await;
    let inspected = inspected(outcome);
    assert!(
        matches!(&*inspected, Inspected::Error(_)),
        "got {inspected:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_antibot_failure_retries_once_with_chrome_impersonation_in_inspect_and_download() {
    let fixture = fixture();
    let memory = fixture.memory();
    let i_env = inspect_env(&fixture, JsRuntimeMode::Auto, false, &memory);

    let outcome = inspect::inspect(
        &i_env,
        "https://site.example/antibot-retry",
        CancellationToken::new(),
    )
    .await;
    let inspected = inspected(outcome);
    assert!(
        matches!(&*inspected, Inspected::Video(_)),
        "expected inspect to succeed on impersonate retry, got {inspected:?}"
    );
    assert_eq!(fixture.log_lines().len(), 2, "two inspect attempts");

    let j_env = fixture.env(JsRuntimeMode::Auto, false, &memory);
    let (sink, _labels) = collecting_sink();
    let j_outcome = job::run_job(
        &j_env,
        &fixture.request("https://site.example/antibot-retry"),
        CancellationToken::new(),
        sink,
    )
    .await;
    assert!(
        matches!(j_outcome, JobOutcome::Done { .. }),
        "expected download to succeed on impersonate retry, got {j_outcome:?}"
    );
    assert_eq!(fixture.log_lines().len(), 4, "two more download attempts");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_archive_skip_reports_skipped_archive_without_failing() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = fixture.env(JsRuntimeMode::Auto, false, &memory);
    let (sink, _labels) = collecting_sink();

    let mut req = fixture.request("https://youtu.be/archive-skip");
    req.spec.archive_file = Some(fixture.home.join("archive.txt"));

    let outcome = job::run_job(&env, &req, CancellationToken::new(), sink).await;
    assert!(
        matches!(outcome, JobOutcome::SkippedArchive { .. }),
        "expected SkippedArchive, got {outcome:?}"
    );
    assert_eq!(fixture.log_lines().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_embedding_failure_is_treated_as_a_warning_when_output_file_exists() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = fixture.env(JsRuntimeMode::Auto, false, &memory);
    let (sink, _labels) = collecting_sink();

    let mut req = fixture.request("https://youtu.be/embed-fail");
    req.spec.embed_thumbnail = true;

    let outcome = job::run_job(&env, &req, CancellationToken::new(), sink).await;
    match outcome {
        JobOutcome::Done { file, .. } => {
            assert!(
                file.is_file(),
                "expected output file to exist: {}",
                file.display()
            );
        }
        other => panic!("expected JobOutcome::Done despite embedding failure, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_job_fails_when_tool_fails_and_no_embed_option_was_requested() {
    let fixture = fixture();
    let memory = fixture.memory();
    let env = fixture.env(JsRuntimeMode::Auto, false, &memory);
    let (sink, _labels) = collecting_sink();

    let req = fixture.request("https://youtu.be/embed-fail");
    let outcome = job::run_job(&env, &req, CancellationToken::new(), sink).await;
    assert!(
        matches!(outcome, JobOutcome::Failed(_)),
        "expected JobOutcome::Failed when no embed option was requested, got {outcome:?}"
    );
}
