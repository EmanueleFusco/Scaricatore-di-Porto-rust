//! Safe, cross-platform spawning of external tools (spec §4).
//!
//! Everything that is not this file goes through [`spawn_tool`]: no other module
//! may build a `std::process::Command` or a `tokio::process::Command`. That is
//! the only way to guarantee the rules of spec §4 hold everywhere:
//!
//! * **argv arrays only**, never a shell string (spec §4.6);
//! * **absolute tool paths only** — the app never mutates `PATH` (spec §4.8);
//! * **no console window on Windows** — `CREATE_NO_WINDOW`, *not*
//!   `DETACHED_PROCESS` (spec §4.1);
//! * **tree kill** — a Job Object on Windows, a new process group on Unix
//!   (spec §4.2);
//! * **stdout and stderr drained concurrently** by two separate tasks, with a
//!   ring buffer of the last [`STDERR_TAIL_LINES`] stderr lines (spec §4.3);
//! * **no mutex held across `.await`** — every lock in this file is a plain
//!   synchronous critical section (spec §4.4).
//!
//! ## Lifetime of a child
//!
//! ```text
//! spawn()  ->  RunningTool  ──wait(token)──>  ProcessResult
//!               │  │
//!               │  └── TreeHandle: Job Object (Windows) / pgid (Unix)
//!               └───── Arc<TreeHandle> also registered in the global registry
//! ```
//!
//! The global registry is what `kill_all_running()` walks on app exit
//! (spec §4.2 "On app exit cancel all running items"). The Job Object carries
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so on Windows even a *crash* of the
//! app kills the tree: the OS closes the handle for us.
//!
//! ## Cancellation contract
//!
//! Unix: `killpg(SIGTERM)`, wait up to [`GRACEFUL_KILL_DELAY`], then
//! `killpg(SIGKILL)`. Windows: `TerminateJobObject` — Windows has no
//! equivalent of a signal sent to a process group, so there is nothing to wait
//! for. Either way [`ProcessResult::cancelled`] is `true` and the temp
//! directories of the item are the caller's to delete.

use std::collections::{HashMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;

/// The Unix-only process-group id needs atomic access, because a [`TreeHandle`]
/// is shared between the [`RunningTool`] and the global registry.
#[cfg(unix)]
use std::sync::atomic::AtomicI32;

/// stderr lines kept for the error classifier and "Copy details" (spec §4.3:
/// "the last ~200 stderr lines per item").
pub const STDERR_TAIL_LINES: usize = 200;

/// How long a cancelled Unix process group may take to react to `SIGTERM`
/// before it gets `SIGKILL` (spec §4.2: "wait ~2 s").
pub const GRACEFUL_KILL_DELAY: Duration = Duration::from_secs(2);

/// Poll interval of the grace loop.
const KILL_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// How long to wait for the two reader tasks once the child is gone.
///
/// They only have to flush what is still sitting in the pipe, which is
/// milliseconds. The bound exists for the pathological case: something that
/// inherited the write end (a grandchild that escaped the job / process group,
/// a double-forked daemon) and refuses to die would otherwise block us for ever
/// with no way to report anything. Spec §15 wants a cancel to finish in ~2 s and
/// to never leave the UI stuck.
const READER_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// `CREATE_NO_WINDOW` (spec §4.1). Verified against the Windows SDK
/// documentation of `CreateProcess`; `DETACHED_PROCESS` is deliberately **not**
/// used, exactly as the spec insists.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How long a spawn may be retried when the program is there but is not
/// runnable **yet** (see [`is_text_file_busy`]).
///
/// Unix: "text file busy" is what Linux returns from `execve` when **anything**
/// still holds the binary open for writing. It is transient by nature, and this
/// project creates exactly the situation that provokes it: the first launch
/// copies yt-dlp into the app data dir and runs it a moment later (spec §5.2.1).
/// It is not hypothetical — the test suite hit it while running the tools lookup
/// in parallel, and no `/proc` holder survived long enough to be observed, which
/// is what "transient" looks like from the inside.
///
/// The alternative — reporting "yt-dlp not found" because the filesystem was
/// busy for a few milliseconds — is the failure mode this avoids.
#[cfg(not(windows))]
pub const SPAWN_BUSY_TOTAL: Duration = Duration::from_millis(100);

/// Pause between those retries. Short: the writer closes in microseconds.
#[cfg(not(windows))]
pub const SPAWN_BUSY_DELAY: Duration = Duration::from_millis(20);

/// Windows has no `ETXTBSY`, but it has the same situation under other names
/// ([`is_text_file_busy`]): a **scanner** — antivirus, indexer — holds a freshly
/// created or copied executable for a moment, and a scanner does not finish in
/// 20 ms. The budget is a judgement, not a measurement; see D116.
#[cfg(windows)]
pub const SPAWN_BUSY_TOTAL: Duration = Duration::from_millis(1_000);

/// Windows pause between those retries: long enough to let a scan finish, short
/// enough that a genuinely broken tool still reports within a second.
#[cfg(windows)]
pub const SPAWN_BUSY_DELAY: Duration = Duration::from_millis(50);

/// `ETXTBSY`: the binary is still open for writing (Linux and macOS).
const ETXTBSY: i32 = 26;

/// `ERROR_SHARING_VIOLATION`: another process holds the file open — the Windows
/// spelling of "not runnable yet".
#[cfg(windows)]
const ERROR_SHARING_VIOLATION: i32 = 32;

/// `ERROR_ACCESS_DENIED`: a scanner holds the file with a deny-share mode.
#[cfg(windows)]
const ERROR_ACCESS_DENIED: i32 = 5;

/// Is this the "the file is there and not runnable *yet*" error?
///
/// Unix says `ETXTBSY` when the executable is still open for writing. Windows
/// has no such errno, but it has the same situation under other names: a freshly
/// created or copied executable is read by a scanner and `CreateProcessW`
/// answers `ERROR_SHARING_VIOLATION`, or `ERROR_ACCESS_DENIED`, until it releases
/// the file. The app copies yt-dlp and qjs into the app data dir and runs them a
/// moment later (spec §5.2.1), which is exactly that situation.
///
/// The Windows codes are compared only on Windows: 32 and 5 are `EPIPE` and
/// `EIO` elsewhere, and a real failure retried is a real failure reported later.
pub fn is_text_file_busy(err: &io::Error) -> bool {
    if err.raw_os_error() == Some(ETXTBSY) {
        return true;
    }
    #[cfg(windows)]
    {
        matches!(
            err.raw_os_error(),
            Some(ERROR_SHARING_VIOLATION) | Some(ERROR_ACCESS_DENIED)
        )
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Exit code handed to every process of a terminated Windows job. On Unix the
/// exit status comes from the signal instead and is reported as "no exit code".
#[cfg(windows)]
const TREE_TERMINATE_EXIT_CODE: u32 = 1;

/// Prefix of the per-item temp directory, spec §4.9:
/// `<out>/.sdp-tmp-<itemid>/`.
pub const ITEM_TEMP_PREFIX: &str = ".sdp-tmp-";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Attach a tool name to an OS error without ever panicking.
fn process_error(context: &str, err: impl std::fmt::Display) -> AppError {
    AppError::Process(format!("{context}: {err}"))
}

// ---------------------------------------------------------------------------
// Line plumbing (pure, unit-tested)
// ---------------------------------------------------------------------------

/// Last-N-lines buffer (spec §4.3).
///
/// Bounded on purpose: a failing `yt-dlp` can print megabytes and the UI only
/// ever shows the tail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RingBuffer {
    capacity: usize,
    lines: VecDeque<String>,
}

impl RingBuffer {
    /// `capacity` is clamped to at least 1 so a misconfigured caller cannot
    /// build a buffer that silently drops everything.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            lines: VecDeque::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Append one line, dropping the oldest one when full.
    pub fn push(&mut self, line: String) {
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    /// Oldest line first.
    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }

    /// Oldest line first, without copying the trailing newline handling twice.
    pub fn into_lines(self) -> Vec<String> {
        self.lines.into_iter().collect()
    }
}

impl Default for RingBuffer {
    fn default() -> Self {
        Self::new(STDERR_TAIL_LINES)
    }
}

/// Splits a byte stream into lines without losing partial reads and without
/// ever splitting a multi-byte UTF-8 sequence (spec §4.5: decode with
/// `String::from_utf8_lossy`).
///
/// Consecutive `\r\n` is normalised to `\n`: a tool built on Windows writes CRLF
/// and the ring buffer must not end up with stray `\r` in "Copy details".
#[derive(Debug, Default)]
pub struct LineReader {
    pending: Vec<u8>,
}

impl LineReader {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Feed a chunk, emitting every complete line it contains.
    pub fn push(&mut self, chunk: &[u8], emit: &mut impl FnMut(String)) {
        self.pending.extend_from_slice(chunk);
        while let Some(index) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=index).collect();
            emit(decode_line(&line));
        }
    }

    /// Emit whatever is left at EOF, even without a trailing newline.
    pub fn finish(&mut self, emit: &mut impl FnMut(String)) {
        if self.pending.is_empty() {
            return;
        }
        let rest = std::mem::take(&mut self.pending);
        emit(decode_line(&rest));
    }
}

/// One raw line (terminator included) to one owned `String`.
fn decode_line(raw_line: &[u8]) -> String {
    let without_newline = match raw_line.strip_suffix(b"\n") {
        Some(stripped) => stripped,
        None => raw_line,
    };
    let without_carriage_return = without_newline
        .strip_suffix(b"\r")
        .unwrap_or(without_newline);
    String::from_utf8_lossy(without_carriage_return).into_owned()
}

/// Read `reader` to EOF, handing every complete line to `on_line`. The final
/// line is emitted even when the tool did not end it with a newline.
async fn drain_lines<R>(mut reader: R, mut on_line: impl FnMut(String) + Send) -> io::Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut parser = LineReader::new();
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        parser.push(&buffer[..read], &mut on_line);
    }
    parser.finish(&mut on_line);
    Ok(())
}

// ---------------------------------------------------------------------------
// The command to run
// ---------------------------------------------------------------------------

/// Only `http` and `https` are ever handed to a tool (spec §4.6). Anything else
/// — `file://`, `ftp://`, a Windows UNC path, a shell metacharacter — is refused
/// before a process exists.
pub fn is_allowed_url(url: &str) -> bool {
    let lowered = url.trim().to_ascii_lowercase();
    lowered.starts_with("http://") || lowered.starts_with("https://")
}

/// A tool invocation, built as an argv array (spec §4.6).
///
/// The URL is *not* a plain argument: it is stored separately and appended
/// after `--` when the argv array is assembled, so "URL last" is a property of
/// the type rather than of the caller remembering it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub url: Option<String>,
    pub current_dir: Option<PathBuf>,
    pub env: Vec<(OsString, OsString)>,
    /// Shown in "Copy details" and in error messages; never a secret.
    pub label: String,
}

impl ToolCommand {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            ..Self::default()
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// The URL, appended after `--` (spec §4.6).
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(dir.into());
        self
    }

    /// Extra environment for the child. The child inherits the parent
    /// environment; this only adds or overrides.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Point the child at our private temp dir (spec §4.9). All three names
    /// are set because the three OSes look at different ones.
    pub fn temp_dir(self, dir: impl AsRef<Path>) -> Self {
        temp_env(dir.as_ref())
            .into_iter()
            .fold(self, |command, (key, value)| command.env(key, value))
    }

    /// Human-readable name used in diagnostics.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The complete argv array, in order. Pure, so the arg-building rules of
    /// spec §4.6 can be unit-tested without spawning anything.
    pub fn argv(&self) -> Vec<OsString> {
        let mut argv = self.args.clone();
        if let Some(url) = &self.url {
            argv.push(OsString::from("--"));
            argv.push(OsString::from(url));
        }
        argv
    }

    /// Everything that can be checked without touching the filesystem.
    ///
    /// Returns [`AppError::Process`] with a message that names the problem and
    /// never contains a secret.
    pub fn validate(&self) -> Result<(), AppError> {
        if self.program.as_os_str().is_empty() {
            return Err(AppError::Process(
                "refusing to spawn a command with an empty program path".to_owned(),
            ));
        }
        if !self.program.is_absolute() {
            return Err(AppError::Process(format!(
                "refusing to spawn the relative tool path {} (spec §4.8: absolute paths only)",
                self.program.display()
            )));
        }
        if let Some(url) = &self.url {
            if !is_allowed_url(url) {
                return Err(AppError::Process(format!(
                    "refusing the URL {url:?}: only http:// and https:// are accepted (spec §4.6)"
                )));
            }
        }
        for arg in self.argv() {
            reject_embedded_nul("argument", &arg)?;
        }
        for (key, value) in &self.env {
            reject_embedded_nul("environment name", key)?;
            reject_embedded_nul("environment value", value)?;
        }
        if let Some(dir) = &self.current_dir {
            reject_embedded_nul("working directory", dir.as_os_str())?;
        }
        Ok(())
    }
}

fn reject_embedded_nul(what: &str, value: &OsStr) -> Result<(), AppError> {
    if value.to_string_lossy().contains('\0') {
        return Err(AppError::Process(format!(
            "refusing to spawn: {what} contains a NUL byte"
        )));
    }
    Ok(())
}

/// `TMPDIR`, `TEMP` and `TMP` pointing at our private directory (spec §4.9).
///
/// QuickJS writes its EJS script there, which is why the directory is `0700`.
pub fn temp_env(dir: &Path) -> Vec<(OsString, OsString)> {
    let value = OsString::from(dir);
    ["TMPDIR", "TEMP", "TMP"]
        .into_iter()
        .map(|name| (OsString::from(name), value.clone()))
        .collect()
}

/// `<out>/.sdp-tmp-<itemid>` (spec §4.9): same filesystem as the output, so
/// yt-dlp's `.part` files can be renamed atomically.
pub fn item_temp_dir(output_dir: &Path, item_id: &str) -> PathBuf {
    output_dir.join(format!("{ITEM_TEMP_PREFIX}{item_id}"))
}

// ---------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------

/// Receives every stdout line as it arrives. `Arc` because the queue hands the
/// same sink to the parser and to the progress emitter.
pub type LineSink = Arc<dyn Fn(&str) + Send + Sync>;

/// A sink that throws the lines away, for the calls that only want the result.
pub fn discard_lines() -> LineSink {
    Arc::new(|_line| {})
}

/// Wrap a closure as a [`LineSink`].
pub fn line_sink<F>(sink: F) -> LineSink
where
    F: Fn(&str) + Send + Sync + 'static,
{
    Arc::new(sink)
}

/// The two line callbacks of one run.
///
/// stderr has a callback **as well as** the ring buffer, and that is not
/// symmetric decoration: measured with the pinned yt-dlp (2026.08.19), the
/// `download:` progress template goes to stdout while `postprocess:` goes to
/// stderr, so "Merging…" and the audio extraction can only be followed live by
/// reading stderr. The ring buffer keeps being filled either way, because the
/// error classifier of spec §8.5 needs the tail.
#[derive(Clone)]
pub struct LineSinks {
    pub stdout: LineSink,
    pub stderr: LineSink,
}

impl LineSinks {
    pub fn new(stdout: LineSink, stderr: LineSink) -> Self {
        Self { stdout, stderr }
    }

    /// Only stdout is interesting; stderr still lands in the ring buffer.
    pub fn stdout_only(stdout: LineSink) -> Self {
        Self {
            stdout,
            stderr: discard_lines(),
        }
    }
}

// ---------------------------------------------------------------------------
// The tree handle
// ---------------------------------------------------------------------------

/// Handle on "the child and everything it spawns".
///
/// * Windows: the Job Object, with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` set, so
///   the tree dies when we terminate the job **or** when the last handle to it
///   is closed, which the OS does even if the app crashes.
/// * Unix: the child's pid, which is also its process-group id because the child
///   is spawned with `process_group(0)`.
pub struct TreeHandle {
    pid: u32,
    /// 0 means "forgotten": the group was reaped, so its id must never be
    /// signalled again. See [`TreeHandle::forget_group`].
    #[cfg(unix)]
    pgid: AtomicI32,
    #[cfg(windows)]
    job: windows::Win32::Foundation::HANDLE,
}

// `HANDLE` is a raw pointer and therefore neither `Send` nor `Sync` (verified:
// windows 0.62.2 `Win32::Foundation::HANDLE` has no `Send`/`Sync` impl). It is
// safe to declare them here: every method takes `&self` and only *uses* the
// handle, never replaces or moves it, and the handle is closed exactly once, in
// `Drop`, which needs `&mut self` and therefore cannot race with another
// `&self` call.
#[cfg(windows)]
unsafe impl Send for TreeHandle {}
#[cfg(windows)]
unsafe impl Sync for TreeHandle {}

impl std::fmt::Debug for TreeHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TreeHandle")
            .field("pid", &self.pid)
            .finish()
    }
}

impl TreeHandle {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Current process-group id, or 0 once it has been forgotten.
    #[cfg(unix)]
    fn pgid(&self) -> i32 {
        self.pgid.load(Ordering::Relaxed)
    }

    /// Kill the whole tree, immediately and without asking.
    ///
    /// Idempotent, and safe to call on a process that already exited. On Unix
    /// an empty process group answers `ESRCH`, which is reported as success
    /// because "nothing left to kill" is exactly the wanted outcome; on Windows
    /// the job's active-process count is read first and an empty job returns
    /// without calling `TerminateJobObject`, whose behaviour on an empty job is
    /// not documented.
    pub fn kill_tree(&self) -> io::Result<()> {
        #[cfg(windows)]
        {
            self.terminate_windows_job()
        }

        #[cfg(unix)]
        {
            match signal_group(self.pgid(), libc::SIGKILL) {
                Ok(()) => Ok(()),
                Err(err) if err.raw_os_error() == Some(libc::ESRCH) => Ok(()),
                Err(err) => Err(err),
            }
        }
    }

    /// Windows half of [`TreeHandle::kill_tree`].
    #[cfg(windows)]
    fn terminate_windows_job(&self) -> io::Result<()> {
        use windows::Win32::System::JobObjects::{
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
            QueryInformationJobObject, TerminateJobObject,
        };

        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: `accounting` outlives the call and its size is passed with
        // it; `self.job` is a live handle owned by this struct.
        let queried = unsafe {
            QueryInformationJobObject(
                Some(self.job),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            )
        };
        if queried.is_ok() && accounting.ActiveProcesses == 0 {
            // Already finished: this is the "kill a dead process is fine" case.
            return Ok(());
        }

        // SAFETY: `self.job` is a live handle created by `CreateJobObjectW`,
        // which grants JOB_OBJECT_TERMINATE, and it is closed once, in `Drop`.
        unsafe { TerminateJobObject(self.job, TREE_TERMINATE_EXIT_CODE) }
            .map_err(|err| io::Error::other(format!("TerminateJobObject failed: {err}")))
    }

    /// Unix only: `SIGTERM` the process group, the polite half of spec §4.2.
    /// A no-op on Windows, which has no group signal.
    pub fn request_terminate(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            match signal_group(self.pgid(), libc::SIGTERM) {
                Ok(()) => Ok(()),
                // Already gone: that is the outcome we wanted anyway.
                Err(err) if err.raw_os_error() == Some(libc::ESRCH) => Ok(()),
                Err(err) => Err(err),
            }
        }

        #[cfg(windows)]
        {
            Ok(())
        }
    }

    /// Does the tree still have members? Used to stop waiting as soon as the
    /// cancellation has taken effect.
    ///
    /// `EPERM` counts as "alive": the group exists, we just may not signal it.
    #[cfg(unix)]
    fn has_members(&self) -> bool {
        // SAFETY: signal 0 performs the permission and existence checks and
        // sends nothing. A pgid of 0 would mean "my own process group", which
        // is never what we want, so `forget_group`'s 0 is filtered out first.
        let pgid = self.pgid();
        if pgid == 0 {
            return false;
        }
        // SAFETY: see above.
        let result = unsafe { libc::killpg(pgid, 0) };
        if result == 0 {
            return true;
        }
        // SAFETY: `killpg` sets errno on failure.
        let error = io::Error::last_os_error();
        // EPERM counts as alive: the group is there, we just may not signal it.
        error.raw_os_error() != Some(libc::ESRCH)
    }

    /// Windows has no graceful group signal, so there is never a wait worth
    /// doing: the answer is "no", and `cancel` goes straight to `kill_tree`.
    #[cfg(windows)]
    fn has_members(&self) -> bool {
        false
    }

    /// Stop using the process-group id.
    ///
    /// Called right after the child has been reaped. From that moment the pid
    /// belongs to the OS again and could in principle be handed to another
    /// process, so the only correct thing to do with the old number is to
    /// forget it. `kill_tree()` becomes a no-op instead of a signal sent to a
    /// stranger.
    #[cfg(unix)]
    fn forget_group(&self) {
        self.pgid.store(0, Ordering::Relaxed);
    }

    #[cfg(windows)]
    fn forget_group(&self) {}
}

/// `killpg(pgid, signal)`, mapping an empty group to "already done".
#[cfg(unix)]
fn signal_group(pgid: i32, signal: i32) -> io::Result<()> {
    // `killpg(0, …)` would signal *our own* process group, which is exactly the
    // disaster `forget_group` exists to prevent. Never let 0 through.
    if pgid == 0 {
        return Ok(());
    }
    // SAFETY: a plain `killpg` with a pgid this process created and still owns.
    let result = unsafe { libc::killpg(pgid, signal) };
    if result == 0 {
        Ok(())
    } else {
        // SAFETY: `killpg` sets errno on failure.
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
impl Drop for TreeHandle {
    fn drop(&mut self) {
        // Closing the last handle to a job that carries
        // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE terminates whatever is still in it.
        // SAFETY: `job` is owned by this value and closed exactly once.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.job) };
    }
}

// ---------------------------------------------------------------------------
// Platform set-up
// ---------------------------------------------------------------------------

/// Put a process group around the child: the child's pid becomes the group id
/// (spec §4.2). Must run *before* the spawn.
#[cfg(unix)]
fn configure(command: &mut Command) {
    // SAFETY-adjacent detail: `process_group` is a `std` method that makes the
    // forked child call `setpgid(0, 0)` before `exec`, so the group is in place
    // before any of the tool's own code can spawn anything.
    command.process_group(0);
}

/// Windows: no console window (spec §4.1). `NOT_DETACHED_PROCESS`, on purpose:
/// the Python original used `DETACHED_PROCESS` and the spec forbids it.
#[cfg(windows)]
fn configure(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

/// Unix half of [`attach`]: `configure` already made the child a process-group
/// leader, so the group id is simply its pid.
#[cfg(unix)]
fn attach(child: &Child) -> io::Result<TreeHandle> {
    let pid = child
        .id()
        .ok_or_else(|| io::Error::other("the spawned process has no id"))?;
    let pgid = i32::try_from(pid)
        .map_err(|_| io::Error::other(format!("pid {pid} does not fit in a process-group id")))?;
    Ok(TreeHandle {
        pid,
        pgid: AtomicI32::new(pgid),
    })
}

/// Windows half of [`attach`]: create a Job Object per invocation, ask it to
/// kill its members when the last handle closes, and put the child in it.
#[cfg(windows)]
fn attach(child: &Child) -> io::Result<TreeHandle> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows::core::PCWSTR;

    let pid = child
        .id()
        .ok_or_else(|| io::Error::other("the spawned process has no id"))?;
    // `raw_handle()` returns `None` only once the child has been reaped, which
    // cannot have happened yet: this runs straight after `spawn`.
    let process = HANDLE(
        child
            .raw_handle()
            .ok_or_else(|| io::Error::other("the spawned process has no handle"))?,
    );

    // SAFETY: a null name gives an unnamed job that only this handle refers to,
    // which is what we want: one job per tool invocation, never shared.
    let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
        .map_err(|err| io::Error::other(format!("CreateJobObjectW failed: {err}")))?;

    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `limits` outlives the call and its size is passed alongside it,
    // which is what SetInformationJobObject requires.
    let configured = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if let Err(err) = configured {
        // SAFETY: `job` was just created here and is closed exactly once.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(job) };
        return Err(io::Error::other(format!(
            "SetInformationJobObject(KILL_ON_JOB_CLOSE) failed: {err}"
        )));
    }

    // SAFETY: `process` is the live handle borrowed from `child` and `job` was
    // created above.
    if let Err(err) = unsafe { AssignProcessToJobObject(job, process) } {
        // SAFETY: `job` was just created here and is closed exactly once.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(job) };
        return Err(io::Error::other(format!(
            "AssignProcessToJobObject failed: {err}"
        )));
    }

    Ok(TreeHandle { pid, job })
}

// ---------------------------------------------------------------------------
// Registry of live trees (app-exit hook)
// ---------------------------------------------------------------------------

struct Registry {
    next_id: AtomicU64,
    live: Mutex<HashMap<u64, Arc<TreeHandle>>>,
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry {
        // 1 keeps "no registration" (0) distinguishable from a real id.
        next_id: AtomicU64::new(1),
        live: Mutex::new(HashMap::new()),
    })
}

/// Register a tree and stamp the handle with its id.
fn register(tree: &Arc<TreeHandle>) -> u64 {
    let id = registry().next_id.fetch_add(1, Ordering::Relaxed);
    let mut slot = match registry().live.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    slot.insert(id, Arc::clone(tree));
    id
}

fn unregister(id: u64) {
    let mut slot = match registry().live.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    slot.remove(&id);
}

/// Kill every tool still running. Called from the Tauri exit hook
/// (spec §4.2 "On app exit cancel all running items") and by tests.
///
/// Returns how many trees were signalled. Synchronous on purpose: the exit hook
/// has no time to await, and every lock taken here is released before the next
/// statement, so no `.await` can ever happen while a lock is held.
pub fn kill_all_running() -> usize {
    let taken = {
        let mut slot = match registry().live.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        std::mem::take(&mut *slot)
    };
    let count = taken.len();
    for tree in taken.values() {
        // A failure here means the process is already gone; nothing to do.
        let _ = tree.kill_tree();
    }
    count
}

/// How many tools are currently registered as running (tests and diagnostics).
pub fn running_count() -> usize {
    match registry().live.lock() {
        Ok(guard) => guard.len(),
        Err(poisoned) => poisoned.into_inner().len(),
    }
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

/// What a finished tool run left behind.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResult {
    /// The `label` of the command, for "Copy details".
    pub label: String,
    pub pid: Option<u32>,
    /// `None` when the process was killed by a signal **on Unix**, or when the
    /// status is unavailable.
    ///
    /// **Do not read this as "was it killed?".** A tree we terminate on
    /// Windows exits with `Some(1)`, because `TerminateJobObject` *sets* the
    /// exit code of every member of the job; the same tree killed with
    /// `SIGKILL` on Unix reports `None`. The portable signals are
    /// [`ProcessResult::cancelled`] and [`ProcessResult::success`], and the
    /// error classifier of spec §8.5 works on the stderr text, not on this
    /// number.
    pub exit_code: Option<i32>,
    pub cancelled: bool,
    /// The last [`STDERR_TAIL_LINES`] stderr lines, oldest first.
    pub stderr_tail: Vec<String>,
    pub duration_ms: u64,
}

impl ProcessResult {
    /// Ran to completion *and* exited 0.
    pub fn success(&self) -> bool {
        !self.cancelled && self.exit_code == Some(0)
    }
}

// ---------------------------------------------------------------------------
// Running tool
// ---------------------------------------------------------------------------

/// A tool that has been started and is being observed.
pub struct RunningTool {
    label: String,
    pid: Option<u32>,
    child: Child,
    tree: Arc<TreeHandle>,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<RingBuffer>,
    /// `Some(id)` until the tool is finished; `Drop` uses it to de-register.
    registration: Option<u64>,
}

impl RunningTool {
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Kill the child and everything it spawned, right now, without waiting.
    /// Idempotent; calling it on a finished tool is a no-op.
    pub fn kill_tree(&self) -> io::Result<()> {
        self.tree.kill_tree()
    }

    /// Wait for the tool to finish, or for `token` to be cancelled.
    ///
    /// Cancellation (spec §4.4) is `tokio::select!` on the child and the token.
    /// No lock is held across either `.await`: the only mutex in this module is
    /// the registry, and every critical section in it is a single statement.
    pub async fn wait(&mut self, token: CancellationToken) -> Result<ProcessResult, AppError> {
        let started = std::time::Instant::now();

        // `select!` on the child and the token (spec §4.4). The handlers touch
        // nothing, so no borrow of `self` is alive while both futures are.
        let exited = tokio::select! {
            // `biased` makes the order deterministic: when the tool has already
            // finished, that is what we report, and a token cancelled in the
            // same instant does not turn a finished download into a cancelled
            // one.
            biased;
            status = self.child.wait() => Some(status),
            () = token.cancelled() => None,
        };

        let (cancelled, status) = match exited {
            Some(status) => (false, status),
            None => {
                self.cancel_gracefully().await;
                // The tree is gone, so both pipes are at EOF and this cannot
                // hang for long.
                (true, self.child.wait().await)
            }
        };

        self.finish(started, status, cancelled).await
    }

    /// spec §4.2: `SIGTERM` the group, give it up to [`GRACEFUL_KILL_DELAY`],
    /// then `SIGKILL`. On Windows the grace loop is skipped and
    /// [`TreeHandle::kill_tree`] terminates the job directly.
    async fn cancel_gracefully(&mut self) {
        let _ = self.tree.request_terminate();

        let deadline = Instant::now() + GRACEFUL_KILL_DELAY;
        while self.tree.has_members() && Instant::now() < deadline {
            // `try_wait` also stops the loop the moment the direct child is
            // gone, so a well-behaved tool is not made to wait out the full
            // grace period just because its zombie still answers `killpg`.
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            tokio::time::sleep(KILL_POLL_INTERVAL).await;
        }

        let _ = self.tree.kill_tree();
    }

    async fn finish(
        &mut self,
        started: std::time::Instant,
        status: io::Result<std::process::ExitStatus>,
        cancelled: bool,
    ) -> Result<ProcessResult, AppError> {
        // Both readers are at EOF now; joining them also collects the ring
        // buffer. A panicking reader is not worth failing the whole run over:
        // whatever it managed to read is still returned.
        let drain = async {
            let _ = (&mut self.stdout_task).await;
            (&mut self.stderr_task).await.ok()
        };
        let stderr_tail = match tokio::time::timeout(READER_DRAIN_TIMEOUT, drain).await {
            Ok(Some(buffer)) => buffer.into_lines(),
            // Timed out (see `READER_DRAIN_TIMEOUT`) or the reader task died:
            // report what we have instead of hanging.
            Ok(None) | Err(_) => Vec::new(),
        };

        // From here the pid belongs to the OS again: never signal it.
        self.tree.forget_group();
        self.de_register();

        let status = status.map_err(|err| {
            process_error(
                &format!("waiting for {} (pid {})", self.label, self.pid.unwrap_or(0)),
                err,
            )
        })?;

        Ok(ProcessResult {
            label: self.label.clone(),
            pid: self.pid,
            exit_code: status.code(),
            cancelled,
            stderr_tail,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }

    fn de_register(&mut self) {
        if let Some(id) = self.registration.take() {
            unregister(id);
        }
    }
}

impl Drop for RunningTool {
    fn drop(&mut self) {
        self.de_register();
        // A `RunningTool` that is dropped while its child still runs would
        // otherwise leave an orphan (and on Windows would hold the job handle
        // open, cancelling the KILL_ON_JOB_CLOSE guarantee). Terminating here
        // makes "dropping means stopped" true on every OS.
        let _ = self.tree.kill_tree();
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Start a tool. Both pipes are already being drained when this returns.
pub async fn spawn(cmd: ToolCommand, on_stdout_line: LineSink) -> Result<RunningTool, AppError> {
    spawn_with_sinks(cmd, LineSinks::stdout_only(on_stdout_line)).await
}

/// Start a tool, handing every stdout *and* stderr line to its callback.
///
/// See [`LineSinks`] for why stderr can have one: the pinned yt-dlp writes
/// post-processing progress there, not on stdout.
pub async fn spawn_with_sinks(cmd: ToolCommand, sinks: LineSinks) -> Result<RunningTool, AppError> {
    cmd.validate()?;

    let label = if cmd.label.is_empty() {
        cmd.program
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| cmd.program.display().to_string())
    } else {
        cmd.label.clone()
    };

    let program = cmd.program.clone();

    // Build the argv array once and re-create the `Command` per attempt: a
    // `Command` cannot be spawned twice, and the ETXTBSY retry needs a fresh one.
    let argv = cmd.argv();
    // A budget, not a count: the same "not runnable yet" window has different
    // lengths on the two OS families (`SPAWN_BUSY_TOTAL`).
    let busy_until = Instant::now() + SPAWN_BUSY_TOTAL;
    let mut child = loop {
        let mut command = Command::new(&program);
        command
            .args(&argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &cmd.current_dir {
            command.current_dir(dir);
        }
        for (key, value) in &cmd.env {
            command.env(key, value);
        }
        configure(&mut command);

        match command.spawn() {
            Ok(child) => break child,
            Err(err) if is_text_file_busy(&err) && Instant::now() < busy_until => {
                // The program is there and something else holds it: the writer
                // closes, or the scanner finishes, within the budget.
                tokio::time::sleep(SPAWN_BUSY_DELAY).await;
            }
            Err(err) => {
                return Err(process_error(&format!("starting {program:?}"), err));
            }
        }
    };

    // From here on the child exists. If the tree handle cannot be built we own
    // the responsibility of not leaving it behind.
    let tree = match attach(&child) {
        Ok(tree) => Arc::new(tree),
        Err(err) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(process_error(
                &format!("attaching {program:?} to a job"),
                err,
            ));
        }
    };
    let pid = child.id();
    let registration = register(&tree);

    let stdout_task = match child.stdout.take() {
        Some(stdout) => {
            let sink = Arc::clone(&sinks.stdout);
            tokio::spawn(async move {
                let _ = drain_lines(stdout, move |line| sink(&line)).await;
            })
        }
        None => tokio::spawn(async {}),
    };

    let stderr_task = match child.stderr.take() {
        Some(stderr) => {
            let sink = Arc::clone(&sinks.stderr);
            tokio::spawn(async move {
                // The buffer lives inside the task, so the parent never shares a lock
                // with a running child: it just receives it back when the task ends.
                let mut ring = RingBuffer::new(STDERR_TAIL_LINES);
                let _ = drain_lines(stderr, |line| {
                    sink(&line);
                    ring.push(line);
                })
                .await;
                ring
            })
        }
        None => tokio::spawn(async { RingBuffer::new(STDERR_TAIL_LINES) }),
    };

    Ok(RunningTool {
        label,
        pid,
        child,
        tree,
        stdout_task,
        stderr_task,
        registration: Some(registration),
    })
}

/// Spawn a tool and wait for it, or for `token` to be cancelled.
///
/// This is the call every other service should make: it validates the argv
/// array, sets the no-console flag or the process group, drains both pipes
/// concurrently, applies the cancellation policy of spec §4.2 and returns the
/// exit code with the stderr tail.
pub async fn spawn_tool(
    cmd: ToolCommand,
    token: CancellationToken,
    on_stdout_line: LineSink,
) -> Result<ProcessResult, AppError> {
    spawn_tool_with_sinks(cmd, token, LineSinks::stdout_only(on_stdout_line)).await
}

/// [`spawn_tool`] with a callback on stderr too, for tools that report progress
/// there (yt-dlp's post-processing templates).
pub async fn spawn_tool_with_sinks(
    cmd: ToolCommand,
    token: CancellationToken,
    sinks: LineSinks,
) -> Result<ProcessResult, AppError> {
    let mut tool = spawn_with_sinks(cmd, sinks).await?;
    tool.wait(token).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn os(value: &str) -> OsString {
        OsString::from(value)
    }

    /// A path that is **absolute on the platform the test is running on**.
    ///
    /// Not a detail: on Windows `Path::new("/usr/bin/yt-dlp").is_absolute()` is
    /// `false`, because there is no drive prefix and no UNC root. A hardcoded
    /// POSIX path would make `validate()` reject it (correctly!) and the tests
    /// below would either fail for the wrong reason or — worse — pass while
    /// never reaching the check they exist to cover. That is exactly what
    /// happened on the first Windows CI run.
    fn absolute_path(leaf: &str) -> PathBuf {
        let mut path = if cfg!(windows) {
            PathBuf::from(r"C:\tools")
        } else {
            PathBuf::from("/usr/tools")
        };
        path.push(leaf);
        assert!(
            path.is_absolute(),
            "the sample path {} must be absolute on this platform",
            path.display()
        );
        path
    }

    /// An absolute path to a tool, valid everywhere.
    fn sample_tool() -> PathBuf {
        absolute_path(if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        })
    }

    // -- RingBuffer ---------------------------------------------------------

    #[test]
    fn ring_buffer_keeps_everything_until_it_is_full() {
        let mut ring = RingBuffer::new(3);
        ring.push("a".to_owned());
        ring.push("b".to_owned());
        assert_eq!(ring.lines(), vec!["a", "b"]);
        assert_eq!(ring.len(), 2);
        assert!(!ring.is_empty());
    }

    #[test]
    fn ring_buffer_drops_the_oldest_line_when_full() {
        let mut ring = RingBuffer::new(3);
        for line in ["a", "b", "c", "d", "e"] {
            ring.push(line.to_owned());
        }
        assert_eq!(ring.lines(), vec!["c", "d", "e"]);
        assert_eq!(ring.len(), 3);
    }

    #[test]
    fn ring_buffer_clamps_a_zero_capacity() {
        let ring = RingBuffer::new(0);
        assert_eq!(ring.capacity(), 1);
    }

    #[test]
    fn ring_buffer_default_capacity_is_the_specified_tail() {
        assert_eq!(RingBuffer::default().capacity(), STDERR_TAIL_LINES);
    }

    // -- LineReader ---------------------------------------------------------

    #[test]
    fn line_reader_splits_complete_lines_only() {
        let mut reader = LineReader::new();
        let mut seen = Vec::new();
        reader.push(b"one\ntwo\n", &mut |line| seen.push(line));
        assert_eq!(seen, vec!["one", "two"]);
    }

    #[test]
    fn line_reader_keeps_a_partial_line_until_it_is_complete() {
        let mut reader = LineReader::new();
        let mut seen = Vec::new();
        reader.push(b"part", &mut |line| seen.push(line));
        assert!(seen.is_empty(), "an incomplete line must not be emitted");

        reader.push(b"ial\n", &mut |line| seen.push(line));
        assert_eq!(seen, vec!["partial"]);
    }

    #[test]
    fn line_reader_finishes_with_an_unterminated_last_line() {
        let mut reader = LineReader::new();
        let mut seen = Vec::new();
        reader.push(b"no newline", &mut |line| seen.push(line));
        reader.finish(&mut |line| seen.push(line));
        assert_eq!(seen, vec!["no newline"]);
    }

    #[test]
    fn line_reader_finish_on_an_empty_stream_emits_nothing() {
        let mut reader = LineReader::new();
        let mut seen = Vec::new();
        reader.push(b"", &mut |line| seen.push(line));
        reader.finish(&mut |line| seen.push(line));
        assert!(seen.is_empty());
    }

    #[test]
    fn line_reader_normalises_windows_line_endings() {
        let mut reader = LineReader::new();
        let mut seen = Vec::new();
        reader.push(b"alpha\r\nbeta\r\n", &mut |line| seen.push(line));
        assert_eq!(seen, vec!["alpha", "beta"]);
    }

    #[test]
    fn line_reader_keeps_a_lone_carriage_return_inside_a_line() {
        let mut reader = LineReader::new();
        let mut seen = Vec::new();
        reader.push(b"a\rb\n", &mut |line| seen.push(line));
        assert_eq!(seen, vec!["a\rb"]);
    }

    #[test]
    fn line_reader_never_splits_a_multibyte_character() {
        // "à" is 0xC3 0xA0: feeding the two halves separately must still
        // produce one correct line.
        let mut reader = LineReader::new();
        let mut seen = Vec::new();
        reader.push(&[0xC3], &mut |line| seen.push(line));
        reader.push(&[0xA0, b'\n'], &mut |line| seen.push(line));
        assert_eq!(seen, vec!["à"]);
    }

    #[test]
    fn line_reader_replaces_invalid_utf8_instead_of_failing() {
        let mut reader = LineReader::new();
        let mut seen = Vec::new();
        reader.push(&[0xFF, 0xFE, b'\n'], &mut |line| seen.push(line));
        assert_eq!(seen.len(), 1);
        assert!(!seen[0].is_empty());
    }

    // -- URLs ---------------------------------------------------------------

    #[test]
    fn only_http_and_https_urls_are_accepted() {
        assert!(is_allowed_url("https://youtu.be/abc"));
        assert!(is_allowed_url("http://example.org/x"));
        assert!(is_allowed_url("HTTPS://EXAMPLE.ORG/x"), "case-insensitive");
        assert!(is_allowed_url("  https://example.org/x  "), "trimmed");
    }

    #[test]
    fn every_other_scheme_is_refused() {
        for url in [
            "file:///etc/passwd",
            "ftp://example.org/x",
            "\\\\server\\share\\x",
            "javascript:alert(1)",
            "data:text/html,x",
            "/etc/passwd",
            "",
        ] {
            assert!(!is_allowed_url(url), "{url:?} must be refused");
        }
    }

    // -- ToolCommand --------------------------------------------------------

    #[test]
    fn argv_puts_the_url_last_after_a_double_dash() {
        let command = ToolCommand::new("/opt/ytdlp")
            .arg("--ignore-config")
            .arg("-f")
            .arg("bestvideo")
            .url("https://youtu.be/abc");
        assert_eq!(
            command.argv(),
            vec![
                os("--ignore-config"),
                os("-f"),
                os("bestvideo"),
                os("--"),
                os("https://youtu.be/abc")
            ]
        );
    }

    #[test]
    fn argv_without_a_url_has_no_separator() {
        let command = ToolCommand::new("/opt/ffmpeg").arg("-version");
        assert_eq!(command.argv(), vec![os("-version")]);
    }

    #[test]
    fn argv_keeps_argument_order_and_duplicates() {
        let command = ToolCommand::new("/opt/ytdlp")
            .args(["--a", "--b"])
            .arg("--a");
        assert_eq!(command.argv(), vec![os("--a"), os("--b"), os("--a")]);
    }

    #[test]
    fn validate_accepts_an_absolute_program_and_an_https_url() {
        let command = ToolCommand::new(sample_tool()).url("https://example.org/v");
        assert!(command.validate().is_ok(), "must accept");
    }

    #[test]
    fn validate_refuses_a_relative_program_path() {
        let command = ToolCommand::new("yt-dlp");
        let error = command.validate().expect_err("must refuse");
        assert!(
            error.to_string().contains("absolute"),
            "unhelpful message: {error}"
        );
    }

    #[test]
    fn validate_refuses_an_empty_program_path() {
        let command = ToolCommand::new("");
        assert!(command.validate().is_err());
    }

    #[test]
    fn validate_refuses_a_non_http_url() {
        // `sample_tool()`, not a hardcoded path: with a relative path the test
        // would pass on the "absolute" rule and never reach the URL check.
        let command = ToolCommand::new(sample_tool()).url("file:///etc/passwd");
        let error = command.validate().expect_err("must refuse");
        assert!(
            error.to_string().contains("http"),
            "must fail on the scheme, not on something else: {error}"
        );
    }

    #[test]
    fn validate_refuses_a_nul_byte_anywhere() {
        // Same reasoning as above: each case below has to reach the NUL check.
        let command = ToolCommand::new(sample_tool()).arg("--a\0b");
        assert!(command.validate().is_err());

        let command = ToolCommand::new(sample_tool()).env("A\0B", "1");
        assert!(command.validate().is_err());

        let command = ToolCommand::new(sample_tool()).env("A", "1\0");
        assert!(command.validate().is_err());
    }

    #[test]
    fn validate_does_not_require_the_program_to_exist() {
        // Existence is a tool-resolution concern (spec §5.2), not an argv one:
        // this function must stay a pure check so it can be unit-tested.
        let command = ToolCommand::new(absolute_path("definitely-not-here/yt-dlp"));
        assert!(command.validate().is_ok());
    }

    #[test]
    fn a_backslash_path_is_relative_on_unix_and_absolute_on_windows() {
        // Documents the platform difference that caused the first Windows CI
        // failure: the separator is not the issue, the drive prefix is.
        let windows_style = PathBuf::from(r"C:\tools\yt-dlp.exe");
        assert_eq!(
            windows_style.is_absolute(),
            cfg!(windows),
            "is_absolute() of a drive-prefixed path must follow the target OS"
        );
        let posix_style = PathBuf::from("/usr/tools/yt-dlp");
        assert_eq!(
            posix_style.is_absolute(),
            !cfg!(windows),
            "a path without a drive prefix is not absolute on Windows"
        );
    }

    // -- temp dir -----------------------------------------------------------

    #[test]
    fn temp_env_sets_all_three_names() {
        let env = temp_env(Path::new("/cache/tmp"));
        let names: Vec<String> = env
            .iter()
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["TMPDIR", "TEMP", "TMP"]);
        for (_, value) in env {
            assert_eq!(value, os("/cache/tmp"));
        }
    }

    #[test]
    fn temp_dir_builder_adds_the_three_variables() {
        let command = ToolCommand::new(sample_tool()).temp_dir("/cache/tmp");
        assert_eq!(command.env, temp_env(Path::new("/cache/tmp")));
    }

    #[test]
    fn item_temp_dir_is_under_the_output_dir_with_the_spec_prefix() {
        let dir = item_temp_dir(Path::new("/home/u/Videos"), "item-7");
        assert_eq!(dir, Path::new("/home/u/Videos").join(".sdp-tmp-item-7"));
        assert_eq!(
            dir.parent(),
            Some(Path::new("/home/u/Videos")),
            "must stay on the output filesystem"
        );
    }

    // -- ProcessResult ------------------------------------------------------

    #[test]
    fn success_needs_both_a_zero_exit_and_no_cancellation() {
        let base = ProcessResult {
            label: "t".to_owned(),
            pid: Some(1),
            exit_code: Some(0),
            cancelled: false,
            stderr_tail: Vec::new(),
            duration_ms: 0,
        };
        assert!(base.success());

        let cancelled = ProcessResult {
            cancelled: true,
            ..base.clone()
        };
        assert!(!cancelled.success());

        let failed = ProcessResult {
            exit_code: Some(1),
            ..base.clone()
        };
        assert!(!failed.success());

        let signalled = ProcessResult {
            exit_code: None,
            ..base
        };
        assert!(!signalled.success());
    }

    #[test]
    fn process_result_serialises_in_the_frontend_shape() {
        let result = ProcessResult {
            label: "yt-dlp".to_owned(),
            pid: Some(4242),
            exit_code: Some(0),
            cancelled: false,
            stderr_tail: vec!["WARNING: x".to_owned()],
            duration_ms: 12,
        };
        let json = serde_json::to_value(&result).expect("serialise");
        assert_eq!(json["label"], "yt-dlp");
        assert_eq!(json["pid"], 4242);
        assert_eq!(json["exitCode"], 0);
        assert_eq!(json["cancelled"], false);
        assert_eq!(json["stderrTail"][0], "WARNING: x");
        assert_eq!(json["durationMs"], 12);
    }

    #[test]
    fn text_file_busy_is_recognised_and_nothing_else_is() {
        // errno 26 on Linux and macOS, and nothing else: mistaking another error
        // for it would turn a real failure into a slower real failure. 32 and 5
        // are the *Windows* names of the same situation; here they are EPIPE and
        // EIO, which must not be retried (the `#[cfg(windows)]` test below is
        // the other half of this one).
        let busy = io::Error::from_raw_os_error(26);
        assert!(is_text_file_busy(&busy));
        for other in [2, 5, 13, 22, 27, 32] {
            assert!(!is_text_file_busy(&io::Error::from_raw_os_error(other)));
        }
        assert!(!is_text_file_busy(&io::Error::other("not an os error")));
    }

    /// The Windows half: a scanner reading a freshly created executable makes
    /// `CreateProcessW` answer this way, and it is the same "not runnable yet" as
    /// `ETXTBSY` (D116).
    #[cfg(windows)]
    #[test]
    fn windows_sharing_violations_are_the_same_not_runnable_yet() {
        for code in [32, 5] {
            assert!(
                is_text_file_busy(&io::Error::from_raw_os_error(code)),
                "windows error {code} must be retried"
            );
        }
        // And a file that is simply not there is not retried: it is not a scan.
        for code in [2, 3] {
            assert!(!is_text_file_busy(&io::Error::from_raw_os_error(code)));
        }
    }

    #[test]
    fn the_spawn_retry_stays_shorter_than_the_user_notices() {
        // A *real* failure must not hide behind the retry: the budget has to stay
        // well inside the shortest user-visible timeout in the app, or waiting
        // would buy nothing. One second is the ceiling, and the Windows budget is
        // the one that comes close to it.
        assert!(
            SPAWN_BUSY_DELAY < SPAWN_BUSY_TOTAL,
            "the delay must fit the budget"
        );
        assert!(
            SPAWN_BUSY_TOTAL <= Duration::from_secs(1),
            "{:?} is too long",
            SPAWN_BUSY_TOTAL
        );
    }
}
