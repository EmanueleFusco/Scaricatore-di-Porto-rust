//! Integration tests for the process layer (spec §4, spec §15).
//!
//! Everything here runs **offline** against the `fake_tool` bin that `cargo
//! test` builds next to these tests — never against a live yt-dlp, never
//! against the network. They are written to be meaningful on Windows, macOS and
//! Linux; the parts that genuinely differ per OS (`process_alive`, how a
//! tree-kill manifests) are the only `cfg`-gated pieces.
//!
//! The four scenarios are the ones `PHASE_PROMPTS.md` M0-B asks for:
//!
//! 1. a normal run: exit code and output captured;
//! 2. a large stderr output that would deadlock a reader that drains one pipe
//!    at a time;
//! 3. cancellation that must kill the child **and** its grandchild;
//! 4. killing a process that already finished.
//!
//! **Run the suite as `cargo test`.** The helper is a Cargo *example*, which
//! Cargo only builds for the default target set; `cargo test --test process_tree`
//! skips it and the suite then fails with an explicit message rather than
//! quietly passing.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use scaricatore_di_porto_lib::error::ErrorCode;
use scaricatore_di_porto_lib::services::process::{self, ToolCommand};
use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};
use tokio_util::sync::CancellationToken;

/// The process registry is global on purpose (the app-exit hook reaches it from
/// a place that has no `AppHandle`), so these tests — which count and kill
/// "everything running" — must not overlap. Every test takes this lock first.
///
/// It is a `tokio` mutex, not `std::sync::Mutex`: the guard is held across
/// `.await` points, which is exactly what `clippy::await_holding_lock` forbids
/// for the blocking one.
static TEST_SERIAL: AsyncMutex<()> = AsyncMutex::const_new(());

async fn serial() -> AsyncMutexGuard<'static, ()> {
    TEST_SERIAL.lock().await
}

/// The `fake_tool` example binary built alongside these tests.
///
/// Cargo exposes `CARGO_BIN_EXE_<name>` only for **bin** targets, and this is
/// an example on purpose (see `Cargo.toml`: a `[[bin]]` would be installed into
/// `/usr/bin` by the Tauri bundler). Examples live in `<profile>/examples/`
/// next to the test binary's `<profile>/deps/`, so the path is derived from
/// `current_exe()`. `SDP_FAKE_TOOL` overrides it for anyone running the tests
/// against a pre-built helper.
fn fake_tool() -> PathBuf {
    if let Some(custom) = std::env::var_os("SDP_FAKE_TOOL") {
        let path = PathBuf::from(custom);
        assert!(
            path.is_file(),
            "SDP_FAKE_TOOL points at {}, which is not a file",
            path.display()
        );
        return path;
    }

    // .../target/<profile>/deps/process_tree-<hash>  ->  .../target/<profile>
    let profile_dir = std::env::current_exe()
        .expect("the test binary has a path")
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the test binary lives in <profile>/deps/")
        .to_path_buf();

    let mut path = profile_dir.join("examples").join("fake_tool");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    assert!(
        path.is_file(),
        "the fake_tool example was not built next to the tests (looked for {}).\n\
         Run the suite as `cargo test` — filtering with `--test process_tree` \
         makes Cargo build only that target and skip the example. \
         `SDP_FAKE_TOOL=<path>` overrides the lookup.",
        path.display()
    );
    path
}

fn command(args: &[&str]) -> ToolCommand {
    ToolCommand::new(fake_tool())
        .args(args.iter().copied())
        .label("fake_tool")
}

/// Collect every stdout line into a vector, so a test can assert on them.
fn collector() -> (process::LineSink, Arc<Mutex<Vec<String>>>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let lines = Arc::clone(&lines);
        process::line_sink(move |line| {
            lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(line.to_owned())
        })
    };
    (sink, lines)
}

fn collected(lines: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    lines
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Is a pid still a live process?
///
/// Unix: `kill(pid, 0)` answers "exists" for a zombie too, so a grandchild that
/// has been killed but not yet reaped still counts as alive for a moment —
/// [`wait_until_gone`] tolerates that.
#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // SAFETY: signal 0 runs the existence and permission checks and delivers
    // nothing to the target.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }
    // SAFETY: `kill` sets errno on failure.
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_ACCESS_RIGHTS};

    /// The least privilege that still lets `OpenProcess` succeed on a running
    /// process of this same user (verified value of `PROCESS_QUERY_LIMITED_INFORMATION`).
    const PROCESS_QUERY_LIMITED_INFORMATION: PROCESS_ACCESS_RIGHTS = PROCESS_ACCESS_RIGHTS(0x1000);

    // A Windows process object lives until its last handle is closed, so an
    // exited grandchild (whose only handle belonged to `fake_tool`, which is
    // dead too) makes `OpenProcess` fail. We hold no handle on it.
    // SAFETY: opening a pid of this same user for query rights only; the
    // handle is closed by `OwnedHandle` at the end of the expression.
    match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(handle) => {
            drop(unsafe { OwnedHandle::from_raw_handle(handle.0) });
            true
        }
        Err(_) => false,
    }
}

/// Wait until `pid` is gone, or fail the test. Reaping is asynchronous, so this
/// polls instead of checking once.
async fn wait_until_gone(pid: u32, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while process_alive(pid) {
        assert!(
            Instant::now() < deadline,
            "{what} (pid {pid}) was still alive 15 s after the cancellation"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Poll the stdout collector until the fake tool has printed the pid of the
/// grandchild it spawned.
async fn wait_for_grandchild_pid(lines: &Arc<Mutex<Vec<String>>>) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(pid) = collected(lines)
            .first()
            .and_then(|line| line.trim().parse().ok())
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "the fake tool never reported its grandchild pid; stdout was {:?}",
            collected(lines)
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// ---------------------------------------------------------------------------
// Test 1 — a normal run
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_normal_run_reports_the_exit_code_and_both_streams() {
    let _serial = serial().await;

    // `both` writes to the two streams at the same time, so this one invocation
    // covers stdout delivery and stderr capture together.
    let (sink, lines) = collector();
    let result = process::spawn_tool(command(&["both", "3"]), CancellationToken::new(), sink)
        .await
        .expect("the tool must run");

    assert!(result.success(), "exit code was {:?}", result.exit_code);
    assert_eq!(result.exit_code, Some(0));
    assert!(!result.cancelled);
    assert!(result.pid.is_some(), "the pid is needed for diagnostics");
    assert_eq!(
        collected(&lines),
        vec!["out-0", "out-1", "out-2"],
        "every stdout line must reach the callback"
    );
    assert_eq!(result.stderr_tail, vec!["err-0", "err-1", "err-2"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_zero_exit_code_is_reported_as_a_failure() {
    let _serial = serial().await;

    let result = process::spawn_tool(
        command(&["exit", "3"]),
        CancellationToken::new(),
        process::discard_lines(),
    )
    .await
    .expect("the tool must run");

    assert_eq!(result.exit_code, Some(3));
    assert!(!result.success());
    assert!(!result.cancelled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tool_that_does_not_exist_is_an_error_not_a_panic() {
    let _serial = serial().await;

    let missing = fake_tool().with_file_name("definitely-not-here");
    let error = process::spawn_tool(
        ToolCommand::new(missing).label("missing"),
        CancellationToken::new(),
        process::discard_lines(),
    )
    .await
    .expect_err("must fail");

    assert_eq!(error.code(), ErrorCode::Process);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_child_sees_the_private_temp_directory() {
    let _serial = serial().await;

    // spec §4.9: TMPDIR/TEMP/TMP point at our private per-app directory.
    let (sink, lines) = collector();
    let result = process::spawn_tool(
        command(&["env", "TMPDIR"]).temp_dir("/private/sdp/tmp"),
        CancellationToken::new(),
        sink,
    )
    .await
    .expect("the tool must run");

    assert!(result.success());
    assert_eq!(collected(&lines), vec!["/private/sdp/tmp"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_url_really_lands_after_the_double_dash() {
    let _serial = serial().await;

    // spec §4.6. The pure `ToolCommand::argv()` rules are unit-tested next to
    // the code; this checks the same thing on a real command line, where a
    // shell would have had its chance to mangle it.
    let (sink, lines) = collector();
    let result = process::spawn_tool(
        command(&["args"])
            .arg("--ignore-config")
            .arg("-f")
            .arg("bestvideo+bestaudio")
            .url("https://youtu.be/dQw4w9WgXcQ"),
        CancellationToken::new(),
        sink,
    )
    .await
    .expect("the tool must run");

    assert!(result.success());
    assert_eq!(
        collected(&lines),
        vec![
            "--ignore-config",
            "-f",
            "bestvideo+bestaudio",
            "--",
            "https://youtu.be/dQw4w9WgXcQ",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_url_that_is_not_http_is_refused_before_anything_is_spawned() {
    let _serial = serial().await;

    let error = process::spawn_tool(
        command(&["noop"]).url("file:///etc/passwd"),
        CancellationToken::new(),
        process::discard_lines(),
    )
    .await
    .expect_err("must fail");

    assert_eq!(error.code(), ErrorCode::Process);
    assert!(
        error.to_string().contains("http"),
        "the message must say what is accepted: {error}"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — a large stderr output must not deadlock
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_large_stderr_output_does_not_deadlock() {
    let _serial = serial().await;

    // 40 000 lines is roughly 500 KB per stream, far more than a pipe buffer: a
    // reader that did not drain stdout and stderr concurrently would block here
    // for ever. The `timeout` turns that hang into a failure.
    const LINES: u64 = 40_000;

    let run = process::spawn_tool(
        command(&["both", &LINES.to_string()]),
        CancellationToken::new(),
        process::discard_lines(),
    );

    let result = tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .expect("the tool deadlocked: a pipe filled up and nobody drained it")
        .expect("the tool must run");

    assert!(result.success(), "exit code was {:?}", result.exit_code);
    assert_eq!(
        result.stderr_tail.len(),
        process::STDERR_TAIL_LINES,
        "the stderr tail must be capped at the ring buffer size"
    );
    assert_eq!(
        result.stderr_tail.last().map(String::as_str),
        Some(format!("err-{}", LINES - 1).as_str()),
        "the tail must end with the last line the tool wrote"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_single_huge_stderr_line_survives_intact() {
    let _serial = serial().await;

    // The line reader buffers until a newline: a 1 MiB line must come back whole
    // and must not be chopped at a pipe-buffer boundary.
    const BYTES: usize = 1024 * 1024;

    let result = process::spawn_tool(
        command(&["stderr-payload", &BYTES.to_string()]),
        CancellationToken::new(),
        process::discard_lines(),
    )
    .await
    .expect("the tool must run");

    assert!(result.success());
    assert_eq!(result.stderr_tail.len(), 1);
    assert_eq!(result.stderr_tail[0].len(), BYTES);
}

// ---------------------------------------------------------------------------
// Test 3 — cancellation must kill the child AND its grandchild
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_kills_the_child_and_the_grandchild() {
    let _serial = serial().await;

    let (sink, lines) = collector();
    let token = CancellationToken::new();

    let mut tool = process::spawn(command(&["grandchild", "120000"]), sink)
        .await
        .expect("the tool must start");

    let grandchild = wait_for_grandchild_pid(&lines).await;
    assert!(
        process_alive(grandchild),
        "the grandchild must be running before we cancel"
    );

    let started = Instant::now();
    token.cancel();
    let result = tokio::time::timeout(Duration::from_secs(30), tool.wait(token))
        .await
        .expect("cancellation never completed")
        .expect("the cancelled run must report a result");

    assert!(result.cancelled, "the result must be flagged as cancelled");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "cancellation took {:?}, spec §4.2 allows about 2 s",
        started.elapsed()
    );

    // The whole point of the Job Object / process group: `child.kill()` alone
    // would have left this grandchild running (spec §4.2).
    wait_until_gone(grandchild, "the grandchild").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_a_tool_without_children_is_still_prompt() {
    let _serial = serial().await;

    let token = CancellationToken::new();
    let mut tool = process::spawn(command(&["sleep", "120000"]), process::discard_lines())
        .await
        .expect("the tool must start");

    let started = Instant::now();
    token.cancel();
    let result = tokio::time::timeout(Duration::from_secs(30), tool.wait(token))
        .await
        .expect("cancellation never completed")
        .expect("the cancelled run must report a result");

    assert!(result.cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "cancellation took {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropping_a_running_tool_kills_the_tree() {
    let _serial = serial().await;

    // The RAII guarantee: a `RunningTool` that goes out of scope must not leave
    // the process behind. On Windows this is also what keeps the Job Object's
    // KILL_ON_JOB_CLOSE promise meaningful.
    let (sink, lines) = collector();
    let (child, grandchild) = {
        let tool = process::spawn(command(&["grandchild", "120000"]), sink)
            .await
            .expect("the tool must start");
        let grandchild = wait_for_grandchild_pid(&lines).await;
        (
            tool.pid().expect("a spawned child always has a pid"),
            grandchild,
        )
    };

    wait_until_gone(child, "the child").await;
    wait_until_gone(grandchild, "the grandchild").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_app_exit_hook_kills_every_running_tool() {
    let _serial = serial().await;

    // spec §4.2 "On app exit cancel all running items": `kill_all_running` is
    // what `lib.rs` calls from the Tauri exit hook.
    let mut first = process::spawn(command(&["sleep", "120000"]), process::discard_lines())
        .await
        .expect("the tool must start");
    let mut second = process::spawn(command(&["sleep", "120000"]), process::discard_lines())
        .await
        .expect("the tool must start");
    let first_pid = first.pid().expect("pid");
    let second_pid = second.pid().expect("pid");

    assert_eq!(
        process::kill_all_running(),
        2,
        "both registered trees must be signalled"
    );
    assert_eq!(
        process::kill_all_running(),
        0,
        "the hook must be idempotent"
    );
    assert_eq!(process::running_count(), 0);

    // The two owning tasks then notice their child is gone and reap it, which
    // is what frees the pid for the check below. A tool killed from outside is
    // *not* "cancelled": nothing cancelled it, it was terminated.
    //
    // The exit code is the one place where the two platforms legitimately
    // differ, and the difference is ours by design, not a bug:
    //
    //   Unix    -> `killpg(SIGKILL)`, so the child was killed by a signal and
    //               `ExitStatus::code()` is `None`.
    //   Windows -> `TerminateJobObject(job, TREE_TERMINATE_EXIT_CODE)`, and the
    //               Microsoft documentation of that second parameter says it is
    //               "the exit code to be used by all processes and threads in
    //               the job object", so it is `Some(1)`.
    //
    // What the app actually relies on is `success() == false`, which holds on
    // both: an exit code must never be read as the sign of a kill.
    let first_result = first
        .wait(CancellationToken::new())
        .await
        .expect("the killed tool must report a result");
    assert!(
        !first_result.cancelled,
        "nobody cancelled it, it was terminated"
    );
    assert_eq!(
        first_result.exit_code,
        if cfg!(windows) { Some(1) } else { None },
        "a terminated tree has no exit code on Unix (signal) and the terminate \
         code on Windows"
    );
    assert!(!first_result.success());

    let _ = second
        .wait(CancellationToken::new())
        .await
        .expect("must report");

    wait_until_gone(first_pid, "the first child").await;
    wait_until_gone(second_pid, "the second child").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_finished_tool_is_no_longer_registered() {
    let _serial = serial().await;

    let mut tool = process::spawn(command(&["exit", "0"]), process::discard_lines())
        .await
        .expect("the tool must start");
    assert_eq!(process::running_count(), 1, "a live tool is registered");

    let result = tool.wait(CancellationToken::new()).await.expect("must run");
    assert!(result.success());
    assert_eq!(
        process::running_count(),
        0,
        "a finished tool must not stay in the registry"
    );
}

// ---------------------------------------------------------------------------
// Test 4 — killing something that already finished is fine
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn killing_a_finished_tool_is_not_an_error() {
    let _serial = serial().await;

    let mut tool = process::spawn(command(&["exit", "0"]), process::discard_lines())
        .await
        .expect("the tool must start");
    let result = tool.wait(CancellationToken::new()).await.expect("must run");
    assert!(result.success());

    // "A `kill_tree` that also works if the child has already exited (no panic,
    // no error)."
    assert!(
        tool.kill_tree().is_ok(),
        "killing a finished tool must succeed"
    );
    // ... and it must stay idempotent.
    assert!(tool.kill_tree().is_ok());
    assert!(tool.kill_tree().is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_token_cancelled_before_the_run_is_a_cancellation() {
    let _serial = serial().await;

    let token = CancellationToken::new();
    token.cancel();

    // `sleep`, not `exit 0`: a tool that could finish before the first poll
    // would make this test a coin toss.
    let result = process::spawn_tool(command(&["sleep", "5000"]), token, process::discard_lines())
        .await
        .expect("the tool must run");

    assert!(
        result.cancelled,
        "a token already cancelled must win over a running tool"
    );
    assert!(!result.success());
}
