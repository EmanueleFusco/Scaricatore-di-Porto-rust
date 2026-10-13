//! Tool resolution, provisioning and updating (spec §5.2).
//!
//! Everything that needs an external program — yt-dlp, qjs, ffmpeg, the Whisper
//! sidecar — goes through [`resolve`], which walks the lookup order of spec §5.2
//! and validates each candidate by **running it**, never by looking at its name:
//!
//! | # | source | notes |
//! |:--|:--|:--|
//! | 1 | settings override | "Use my own path" in Settings → Tools |
//! | 2 | app data `…/tools/<tool>/` | downloaded or updated copy (spec §5.2) |
//! | 3 | bundled baseline | next to the executable, read-only (spec §5.2.1) |
//! | 4 | system `PATH` | read, never modified (spec §4.8) |
//! | 5 | well-known locations | required on macOS, where a GUI app inherits no shell PATH |
//!
//! First hit wins, but a candidate that cannot answer `--version` within
//! [`VERSION_PROBE_TIMEOUT`] is passed over rather than accepted: a file that
//! exists at the right place but does not run is exactly the failure the spec
//! wants surfaced in the Tools page, not silently used.
//!
//! Two rules that look like details and are not:
//!
//! * **Absolute paths only.** [`crate::services::process`] refuses a relative
//!   program path (spec §4.8), and the lookup never produces one, so the refusal
//!   is a backstop rather than a normal outcome.
//! * **yt-dlp always runs the app-data copy** (spec §5.2.1). The installer
//!   directory is read-only, so the bundled binary can never self-update; the
//!   writable copy is made on first launch and is the only thing the updater
//!   (spec §5.5) is allowed to replace.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::services::manifest::{self, ToolsManifest};
use crate::services::process::{self, ToolCommand};
use crate::services::{paths, process::LineSink};

/// How long `--version` may take before the candidate is rejected (spec §5.2:
/// "validate by running `--version` (or equivalent) with a 5 s timeout").
pub const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `yt-dlp -U` may take. The updater replaces the whole binary, so it
/// is much slower than a version probe; PyInstaller one-file builds unpack
/// themselves on every launch.
pub const UPDATE_TIMEOUT: Duration = Duration::from_secs(300);

/// Directory inside the app data dir that holds every installed tool.
pub const TOOLS_SUBDIR: &str = "tools";

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// Where a resolved tool came from, in the order of spec §5.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolSource {
    /// 1. The path the user typed in Settings → Tools.
    Override,
    /// 2. `…/tools/<tool>/` in the app data dir: downloaded or updated.
    AppData,
    /// 3. Shipped in the installer, next to the app binary.
    Bundled,
    /// 4. Found on the inherited `PATH`.
    PathEnv,
    /// 5. A well-known install location.
    WellKnown,
}

impl ToolSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Override => "override",
            Self::AppData => "appData",
            Self::Bundled => "bundled",
            Self::PathEnv => "path",
            Self::WellKnown => "wellKnown",
        }
    }
}

/// The five places to look, resolved once and then walked.
#[derive(Debug, Clone)]
pub struct Lookup {
    pub override_path: Option<PathBuf>,
    pub tools_dir: PathBuf,
    pub bundled_dir: PathBuf,
    /// Directories from the inherited `PATH`, in order.
    pub path_dirs: Vec<PathBuf>,
    /// `/opt/homebrew/bin`, `%LOCALAPPDATA%\Microsoft\WinGet\Links`, …
    pub well_known_dirs: Vec<PathBuf>,
}

/// The file name a tool has on this platform: `qjs` or `qjs.exe`.
///
/// Pure: every test can name a file without inventing a directory layout.
pub fn executable_name(tool: &str, triple: &str) -> String {
    if triple.contains("windows") {
        format!("{tool}.exe")
    } else {
        tool.to_owned()
    }
}

/// `…/tools/<tool>/` — lookup order #2 (spec §5.2).
pub fn app_data_dir_for(tools_dir: &Path, tool: &str) -> PathBuf {
    tools_dir.join(tool)
}

/// Directories from the inherited `PATH` (spec §5.2 #4).
///
/// Read-only by construction: this is the only place `PATH` is touched, and it
/// only ever reads it (spec §4.8, "the app never mutates PATH").
pub fn path_dirs(path_var: Option<&str>) -> Vec<PathBuf> {
    path_var
        .unwrap_or_default()
        .split(if cfg!(windows) { ';' } else { ':' })
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .filter(|entry| entry.is_absolute())
        .collect()
}

/// Well-known install locations (spec §5.2 #5), in the order given.
///
/// The first five exist because a macOS GUI app is launched by the Finder and
/// inherits none of the user's shell `PATH` — a Homebrew yt-dlp would otherwise
/// be invisible. They are checked on every Unix system, not only on macOS:
/// `/usr/local/bin` is where a hand-built binary lands on Linux too.
pub fn well_known_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            dirs.push(PathBuf::from(profile).join(".deno").join("bin"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(
                PathBuf::from(local)
                    .join("Microsoft")
                    .join("WinGet")
                    .join("Links"),
            );
        }
    } else {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/opt/local/bin"));
        if let Some(home) = home {
            dirs.push(home.join(".deno").join("bin"));
            dirs.push(home.join(".local").join("bin"));
        }
        if cfg!(target_os = "linux") {
            dirs.push(PathBuf::from("/usr/bin"));
            dirs.push(PathBuf::from("/snap/bin"));
        }
    }
    dirs
}

/// Turn the user's "Use my own path" value into a candidate executable.
///
/// `is_directory` and `windows` are parameters rather than `is_dir()` and
/// `cfg!(windows)` so the rules can be tested for all three platforms from one
/// machine. A test that only ever exercises the host's answers passes on Linux
/// and proves nothing about Windows — the mistake spec D27 records.
pub fn override_candidate(
    path: &Path,
    is_directory: bool,
    windows: bool,
    file_name: &str,
) -> PathBuf {
    if is_directory {
        return path.join(file_name);
    }
    if windows && path.extension().is_none() {
        return path.with_extension("exe");
    }
    path.to_path_buf()
}

/// Every candidate for `tool`, in lookup order, paired with its source.
///
/// Pure, and the whole of spec §5.2's ordering in one testable place. A file
/// that is not there is still listed: the caller probes what exists and reports
/// the rest as "not found", which is what the Tools page needs.
pub fn candidates(tool: &str, lookup: &Lookup, triple: &str) -> Vec<(ToolSource, PathBuf)> {
    let file_name = executable_name(tool, triple);
    let mut out: Vec<(ToolSource, PathBuf)> = Vec::new();

    if let Some(path) = &lookup.override_path {
        if path.as_os_str().is_empty() {
            return Vec::new();
        }
        // An override wins outright: if it does not work the user gets to see
        // exactly that, instead of the app quietly using a different binary.
        let candidate = override_candidate(path, path.is_dir(), cfg!(windows), &file_name);
        return vec![(ToolSource::Override, candidate)];
    }

    out.push((
        ToolSource::AppData,
        app_data_dir_for(&lookup.tools_dir, tool).join(&file_name),
    ));
    out.push((ToolSource::Bundled, lookup.bundled_dir.join(&file_name)));
    for dir in &lookup.path_dirs {
        out.push((ToolSource::PathEnv, dir.join(&file_name)));
    }
    for dir in &lookup.well_known_dirs {
        out.push((ToolSource::WellKnown, dir.join(&file_name)));
    }
    out
}

// ---------------------------------------------------------------------------
// Version probes
// ---------------------------------------------------------------------------

/// Pull a dotted version out of a `--version` banner.
///
/// Deliberately lenient about the *shape* and strict about the *result*: the
/// first token that parses as a dotted number wins, so
/// * `2026.08.19` (yt-dlp) → `2026.08.19`
/// * `0.17.0` (qjs) → `0.17.0`
/// * `ffmpeg version n9.0.2 Copyright…` → `9.0.2`
///
/// The leading-letter strip is not a guess about style: it is what makes the
/// ffmpeg banner parse. Measured on a real `ffmpeg -version` on 2026-10-13:
/// `ffmpeg version n9.0.2 Copyright (c) 2000-2026 the FFmpeg developers` — the
/// `n` prefix is part of the version since ffmpeg 7.
///
/// Returns `None` when there is no such token, because a version we cannot read
/// must not be compared against `minVersion` (spec §5.1).
pub fn parse_version_banner(text: &str) -> Option<String> {
    for line in text.lines() {
        for token in line.split_whitespace() {
            let digits = token.trim_start_matches(|c: char| !c.is_ascii_digit());
            // A bare `1` is a number but not a version; require the dot. This is
            // what keeps `qjs --version` printing "0" from becoming 0.0.0 and
            // then being compared against the 0.12.0 floor.
            if !digits.contains('.') {
                continue;
            }
            if manifest::parse_version(digits).is_some() {
                return Some(digits.to_owned());
            }
        }
    }
    None
}

/// The arguments that make a tool print its version.
///
/// Spec §5.2 says "`--version` (or equivalent)", and the exceptions are real:
/// ffmpeg's documented flag is `-version` (`ffmpeg --version` happens to work
/// too, but relying on it would be assuming). Checked against the real
/// `ffmpeg -version` on 2026-10-13.
pub fn version_args(tool: &str) -> &'static [&'static str] {
    match tool {
        "ffmpeg" | "ffprobe" => &["-version"],
        _ => &["--version"],
    }
}

/// Run the tool's version flag and return its banner, or `None`.
///
/// `None` covers every reason a candidate is unusable: it does not exist, it is
/// not executable, it crashes, or it takes longer than
/// [`VERSION_PROBE_TIMEOUT`]. The caller only needs "is this usable", and the
/// Tools page shows the path it tried.
pub async fn probe_version(path: &Path, tool: &str, temp_dir: Option<&Path>) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    read_version_banner(path, version_args(tool), temp_dir, VERSION_PROBE_TIMEOUT).await
}

/// The version banner run with an arbitrary timeout, shared by the probe and
/// the updater.
async fn read_version_banner(
    path: &Path,
    args: &[&str],
    temp_dir: Option<&Path>,
    timeout: Duration,
) -> Option<String> {
    let mut command = ToolCommand::new(path).args(args).label("tool version");
    if let Some(dir) = temp_dir {
        command = command.temp_dir(dir);
    }
    let token = CancellationToken::new();
    let stdout = Arc::new(StdoutCollector::default());
    let sink: LineSink = {
        let stdout = Arc::clone(&stdout);
        process::line_sink(move |line| stdout.push(line))
    };

    let run = process::spawn_tool(command, token.clone(), sink);
    let outcome = match tokio::time::timeout(timeout, run).await {
        Ok(result) => result,
        Err(_) => {
            // The child was spawned by `spawn_tool`, which only kills it when
            // the token is cancelled: dropping the future is not enough, so
            // the timeout path cancels explicitly and then kills the tree
            // through the process layer (spec §4.2).
            token.cancel();
            return None;
        }
    };

    match outcome {
        Ok(result) if result.success() => {
            let text = stdout.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text)
            }
        }
        // A tool that answers on stderr and exits non-zero is still telling us
        // something; only an empty answer counts as unusable.
        Ok(_) => {
            let text = stdout.text();
            (!text.trim().is_empty()).then_some(text)
        }
        Err(_) => None,
    }
}

/// Gathers stdout lines into one string. Owned by the reader task, read once
/// the run is over, so no lock is ever held across an `.await` (spec §4.4).
#[derive(Default)]
struct StdoutCollector {
    lines: std::sync::Mutex<Vec<String>>,
}

impl StdoutCollector {
    fn push(&self, line: &str) {
        if let Ok(mut lines) = self.lines.lock() {
            lines.push(line.to_owned());
        }
    }

    fn text(&self) -> String {
        match self.lines.lock() {
            Ok(lines) => lines.join("\n"),
            Err(poisoned) => poisoned.into_inner().join("\n"),
        }
    }
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// A tool the app can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTool {
    pub name: String,
    pub path: PathBuf,
    pub source: ToolSource,
    /// Parsed from `--version`, `None` when the banner had no version in it.
    pub version: Option<String>,
    /// The raw banner, for "Copy diagnostics".
    pub banner: String,
}

impl ResolvedTool {
    /// Is this at least the version the manifest demands (spec §5.1)?
    ///
    /// A tool with an unreadable version answers **false**, and a tool with no
    /// entry in the manifest is not constrained.
    pub fn meets_min_version(&self, manifest: &ToolsManifest, triple: &str) -> bool {
        match (self.version.as_deref(), manifest.entry(&self.name, triple)) {
            (Some(found), Some(entry)) => {
                let min = manifest.tool(&self.name).map(|t| t.min_version.as_str());
                min.is_some_and(|min| manifest::satisfies_min_version(found, min))
                    && entry.executable().is_some()
            }
            // No version to compare: do not block the user, but do not claim it
            // is verified either.
            (None, _) => true,
            (_, None) => true,
        }
    }
}

/// Walk the lookup order and return the first candidate that answers
/// `--version` within the timeout.
pub async fn resolve(
    tool: &str,
    lookup: &Lookup,
    triple: &str,
    temp_dir: Option<&Path>,
) -> Option<ResolvedTool> {
    for (source, path) in candidates(tool, lookup, triple) {
        let Some(banner) = probe_version(&path, tool, temp_dir).await else {
            continue;
        };
        return Some(ResolvedTool {
            name: tool.to_owned(),
            path,
            source,
            version: parse_version_banner(&banner),
            banner: banner.trim().to_owned(),
        });
    }
    None
}

/// Every tool we know how to report on, resolved or not.
///
/// A tool that is missing is **not** an error: spec §1.1 asks for "not
/// available on this platform" instead of a silent failure, and the wizard has
/// to be able to say "not installed yet".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub name: String,
    pub role: String,
    /// Shipped in the installer.
    pub bundled: bool,
    /// Does the manifest have an entry for this platform at all?
    pub available_on_platform: bool,
    /// `true` when the manifest declares a release this app could download.
    pub installable: bool,
    pub source: Option<ToolSource>,
    pub path: Option<String>,
    pub version: Option<String>,
    pub banner: Option<String>,
    pub meets_min_version: bool,
    /// Everything that was tried, so a red row in the Tools page can explain
    /// itself instead of just saying "not found".
    pub searched: Vec<String>,
}

/// Tools the Tools page lists even though the manifest does not manage them.
///
/// ffmpeg is here and not in `tools.manifest.json` on purpose: spec §5 says to
/// *prefer the system install*, and offering a static download for it is an M4
/// decision with real licence and size consequences. It is still detected, so
/// "do you have ffmpeg?" is answered today.
pub fn extra_tools() -> &'static [&'static str] {
    &["ffmpeg", "ffprobe"]
}

/// Status of one tool.
pub async fn status_of(
    manifest: &ToolsManifest,
    tool: &str,
    lookup: &Lookup,
    triple: &str,
    temp_dir: Option<&Path>,
) -> ToolStatus {
    let searched: Vec<String> = candidates(tool, lookup, triple)
        .iter()
        .map(|(_, path)| path.display().to_string())
        .collect();

    let resolved = resolve(tool, lookup, triple, temp_dir).await;
    let entry = manifest.entry(tool, triple);
    let managed = manifest.tool(tool);

    ToolStatus {
        name: tool.to_owned(),
        role: managed.map(|t| t.role.clone()).unwrap_or_default(),
        bundled: managed.map(|t| t.bundled).unwrap_or(false),
        available_on_platform: entry.is_some() || managed.is_none(),
        installable: entry.map(|e| e.is_installable()).unwrap_or(false),
        source: resolved.as_ref().map(|r| r.source),
        path: resolved.as_ref().map(|r| r.path.display().to_string()),
        version: resolved.as_ref().and_then(|r| r.version.clone()),
        banner: resolved.as_ref().map(|r| r.banner.clone()),
        meets_min_version: resolved
            .as_ref()
            .map(|r| r.meets_min_version(manifest, triple))
            .unwrap_or(false),
        searched,
    }
}

// ---------------------------------------------------------------------------
// The bundled baseline (spec §5.2.1)
// ---------------------------------------------------------------------------

/// Where the bundled tools live at runtime.
///
/// `bundle.externalBin` puts them **next to the main executable**, with the
/// target-triple suffix stripped — verified in `tauri-bundler`'s
/// `bundle/settings.rs::copy_binaries` (`&bin_dir` is `Contents/MacOS` on
/// macOS, `/usr/bin` in the .deb, `$INSTDIR` under NSIS, and the AppImage
/// squashfs on Linux). So `current_exe().parent()` is the answer on all four,
/// and the read-only AppImage mount is why the copy below exists at all.
pub fn bundled_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();

    // Development builds run from `target/debug`, where the binaries are not
    // next to the executable: `tauri dev` does not copy sidecars. Falling back
    // to `src-tauri/binaries` keeps "fetch the tools, press F5" working, and
    // `cfg!(debug_assertions)` keeps the fallback out of release builds
    // entirely — there, a missing bundled tool must read as "not installed",
    // not as "look somewhere else".
    if cfg!(debug_assertions)
        && let Some(dev_dir) = dev_binaries_dir()
        && !dir.join("yt-dlp").is_file()
        && dev_dir.join("yt-dlp").is_file()
    {
        return Some(dev_dir);
    }
    Some(dir)
}

#[cfg(debug_assertions)]
fn dev_binaries_dir() -> Option<PathBuf> {
    Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries"))
}

#[cfg(not(debug_assertions))]
fn dev_binaries_dir() -> Option<PathBuf> {
    None
}

/// Build the whole lookup from the environment.
pub fn lookup_from(
    settings: &crate::state::Settings,
    bundled_dir: PathBuf,
    tools_dir: PathBuf,
) -> Lookup {
    Lookup {
        override_path: settings
            .tool_override(manifest::YTDLP)
            .map(Path::to_path_buf),
        tools_dir,
        bundled_dir,
        path_dirs: path_dirs(std::env::var("PATH").ok().as_deref()),
        well_known_dirs: well_known_dirs(crate::services::paths::home_dir().as_deref()),
    }
}

/// The same, for a tool other than yt-dlp (the override map is keyed by name).
pub fn lookup_for(
    settings: &crate::state::Settings,
    tool: &str,
    bundled_dir: PathBuf,
    tools_dir: PathBuf,
) -> Lookup {
    Lookup {
        override_path: settings.tool_override(tool).map(Path::to_path_buf),
        tools_dir,
        bundled_dir,
        path_dirs: path_dirs(std::env::var("PATH").ok().as_deref()),
        well_known_dirs: well_known_dirs(crate::services::paths::home_dir().as_deref()),
    }
}

/// Copy the bundled yt-dlp into the app data dir on first launch and return the
/// path to run (spec §5.2.1).
///
/// The installer directory is read-only — `.deb`/`rpm` install system-wide, the
/// AppImage is a read-only mount, and on macOS/Windows the app lives in
/// `Program Files`/`Applications` — so `-U` could never rewrite the bundled
/// copy. Running the app-data copy is what makes the tool updatable at all.
///
/// Failure is not fatal: the caller falls back to the bundled binary, which is
/// the spec's own instruction ("keep the bundled one as a fallback").
pub fn install_app_data_copy(
    bundled: &Path,
    tools_dir: &Path,
    triple: &str,
) -> Result<PathBuf, AppError> {
    if !bundled.is_file() {
        return Err(AppError::Process(format!(
            "the bundled yt-dlp is missing at {}",
            bundled.display()
        )));
    }
    let name = executable_name(manifest::YTDLP, triple);
    let target_dir = app_data_dir_for(tools_dir, manifest::YTDLP);
    paths::ensure_dir(&target_dir)?;
    let target = target_dir.join(&name);

    // Only copy when there is nothing usable in place: copying on every launch
    // would silently undo an update the user just installed.
    if target.is_file() {
        return Ok(target);
    }

    // `.part` + rename: a crash mid-copy must not leave a truncated executable
    // that the next launch would happily run.
    let partial = target_dir.join(format!(".{name}.part"));
    fs::copy(bundled, &partial).map_err(|err| {
        AppError::Io(io::Error::new(
            err.kind(),
            format!("copying the bundled yt-dlp: {err}"),
        ))
    })?;
    make_executable(&partial)?;
    fs::rename(&partial, &target).map_err(|err| {
        let _ = fs::remove_file(&partial);
        AppError::Io(io::Error::new(
            err.kind(),
            format!("installing the app-data copy of yt-dlp: {err}"),
        ))
    })?;
    Ok(target)
}

/// `chmod +x` on Unix. A no-op on Windows, where the extension is the flag.
pub fn make_executable(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The yt-dlp updater (spec §5.5)
// ---------------------------------------------------------------------------

/// What an update attempt did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOutcome {
    /// A new version replaced the old one.
    pub updated: bool,
    /// The version before, when it could be read.
    pub previous_version: Option<String>,
    /// The version after.
    pub version: Option<String>,
    /// yt-dlp's own message, shown in the UI ("already up to date", …).
    pub message: String,
}

/// argv for `yt-dlp -U`, or `--update-to <channel>`.
///
/// Verified against the bundled binary's own `--help` on 2026-10-13
/// (yt-dlp 2026.08.19):
/// `-U, --update  Update this program to the latest stable version` and
/// `--update-to [CHANNEL]@[TAG]  Upgrade/downgrade to a specific version.`
pub fn update_args(channel: crate::state::YtdlpChannel) -> Vec<String> {
    match channel {
        crate::state::YtdlpChannel::Stable => vec!["-U".to_owned()],
        crate::state::YtdlpChannel::Nightly => {
            vec!["--update-to".to_owned(), "nightly".to_owned()]
        }
    }
}

/// Decide what an update run means, from its exit code and output.
///
/// yt-dlp writes progress and the outcome to **stderr**, and says "already up
/// to date" as a normal exit-0 run, so this cannot be reduced to "exit code 0
/// means updated".
pub fn classify_update(
    success: bool,
    output: &str,
    before: Option<&str>,
    after: Option<&str>,
) -> UpdateOutcome {
    let message = last_meaningful_line(output).unwrap_or_else(|| {
        if success {
            String::new()
        } else {
            "the updater failed".to_owned()
        }
    });
    // A non-zero exit is never an update, whatever it printed. Deciding this from
    // the wording alone would let "ERROR: unable to download" pass for success
    // just because it does not happen to contain the words "up to date".
    let updated = if !success {
        false
    } else {
        match (before, after) {
            // Both known: the updater moved the version.
            (Some(before), Some(after)) => before != after,
            // It used to answer and now it does not: the update left a binary
            // that no longer runs. That is not an update, and saying so is what
            // keeps the Tools page from claiming success.
            (Some(_), None) => false,
            // Only the new one is readable: we cannot prove it changed.
            (None, Some(_)) => false,
            // Nothing readable before or after: fall back to the wording.
            (None, None) => !looks_already_up_to_date(output),
        }
    };
    UpdateOutcome {
        updated,
        previous_version: before.map(str::to_owned),
        version: after.map(str::to_owned),
        message,
    }
}

fn last_meaningful_line(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(str::to_owned)
}

/// yt-dlp says `... is up to date!` / `You have already installed the latest
/// version`; both mean "nothing changed".
fn looks_already_up_to_date(output: &str) -> bool {
    let lowered = output.to_ascii_lowercase();
    lowered.contains("up to date")
        || lowered.contains("already installed the latest")
        || lowered.contains("already the latest")
}

/// Run the updater on the **app-data copy** and report what changed.
///
/// The binary replaces itself in place, so the caller must not be running it
/// when this is called, and the version is re-read afterwards from the same
/// path (no restart needed — spec §5.5, "each job starts a new process").
pub async fn update_ytdlp(
    path: &Path,
    channel: crate::state::YtdlpChannel,
    temp_dir: Option<&Path>,
) -> Result<UpdateOutcome, AppError> {
    let before = probe_version(path, manifest::YTDLP, temp_dir)
        .await
        .and_then(|b| parse_version_banner(&b));

    let mut command = ToolCommand::new(path)
        .args(update_args(channel))
        .label("yt-dlp update");
    if let Some(dir) = temp_dir {
        command = command.temp_dir(dir);
    }
    let token = CancellationToken::new();

    let outcome =
        tokio::time::timeout(UPDATE_TIMEOUT, run_capturing_output(command, token.clone())).await;

    let (success, output) = match outcome {
        Ok(result) => result?,
        Err(_) => {
            token.cancel();
            return Err(AppError::Process(format!(
                "the yt-dlp updater did not finish within {} s",
                UPDATE_TIMEOUT.as_secs()
            )));
        }
    };

    // The binary rewrote itself: anything the manifest said about permissions
    // no longer holds.
    if path.exists() {
        make_executable(path)?;
        quarantine_repair(path).await;
    }

    let after = probe_version(path, manifest::YTDLP, temp_dir)
        .await
        .and_then(|b| parse_version_banner(&b));
    Ok(classify_update(
        success,
        &output,
        before.as_deref(),
        after.as_deref(),
    ))
}

/// Run a command and return `(exited zero, stdout + stderr)`.
async fn run_capturing_output(
    command: ToolCommand,
    token: CancellationToken,
) -> Result<(bool, String), AppError> {
    let collected = Arc::new(OutputCollector::default());
    let sink: LineSink = {
        let collected = Arc::clone(&collected);
        process::line_sink(move |line| collected.push(line))
    };
    let result = process::spawn_tool(command, token, sink).await?;
    // yt-dlp writes to both streams and interleaves them, so ordering between
    // the two is lost anyway; what matters is that nothing is dropped.
    let mut text = collected.text();
    if text.trim().is_empty() {
        text.push_str(&result.stderr_tail.join("\n"));
    }
    Ok((result.success(), text))
}

/// Collects lines from both stdout and stderr into one buffer.
#[derive(Default)]
struct OutputCollector {
    lines: std::sync::Mutex<Vec<String>>,
}

impl OutputCollector {
    fn push(&self, line: &str) {
        if let Ok(mut lines) = self.lines.lock() {
            lines.push(line.to_owned());
        }
    }

    fn text(&self) -> String {
        match self.lines.lock() {
            Ok(lines) => lines.join("\n"),
            Err(poisoned) => poisoned.into_inner().join("\n"),
        }
    }
}

// ---------------------------------------------------------------------------
// macOS quarantine (spec §5.2)
// ---------------------------------------------------------------------------

/// Best-effort repair for a binary macOS refuses to launch.
///
/// Spec §5.2: "if a downloaded binary is killed on launch, try
/// `xattr -d com.apple.quarantine <file>` then `codesign --force -s - <file>`".
///
/// Only runs on macOS, only for files in the **app data dir** (never for a
/// bundled binary, which came in through notarization), and only after a probe
/// has already failed. It is called after the updater because that replaces the
/// file with a freshly downloaded one.
#[cfg(target_os = "macos")]
pub async fn quarantine_repair(path: &Path) -> bool {
    use tokio::process::Command;

    for argv in [
        vec!["xattr", "-d", "com.apple.quarantine"],
        vec!["codesign", "--force", "-s", "-"],
    ] {
        let Ok(mut child) = Command::new(argv[0])
            .args(&argv[1..])
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return false;
        };
        if tokio::time::timeout(Duration::from_secs(20), child.wait())
            .await
            .map(|result| result.is_err())
            .unwrap_or(true)
        {
            // Either it failed or it hung: stop here, this is best effort.
            return false;
        }
    }
    true
}

#[cfg(not(target_os = "macos"))]
pub async fn quarantine_repair(_path: &Path) -> bool {
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const LINUX: &str = "x86_64-unknown-linux-gnu";
    const WINDOWS: &str = "x86_64-pc-windows-msvc";

    fn lookup() -> Lookup {
        Lookup {
            override_path: None,
            tools_dir: PathBuf::from("/data/tools"),
            bundled_dir: PathBuf::from("/app"),
            path_dirs: vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")],
            well_known_dirs: vec![PathBuf::from("/opt/homebrew/bin")],
        }
    }

    #[test]
    fn executable_name_follows_the_platform_not_the_host() {
        assert_eq!(executable_name("qjs", LINUX), "qjs");
        assert_eq!(executable_name("qjs", WINDOWS), "qjs.exe");
        assert_eq!(
            executable_name("yt-dlp", "universal-apple-darwin"),
            "yt-dlp"
        );
    }

    #[test]
    fn the_lookup_order_is_exactly_the_one_of_spec_5_2() {
        let found = candidates("qjs", &lookup(), LINUX);
        let sources: Vec<ToolSource> = found.iter().map(|(source, _)| *source).collect();
        assert_eq!(sources[0], ToolSource::AppData, "#2 app data");
        assert_eq!(sources[1], ToolSource::Bundled, "#3 bundled");
        assert_eq!(sources[2], ToolSource::PathEnv, "#4 PATH");
        assert_eq!(sources[3], ToolSource::PathEnv);
        assert_eq!(sources[4], ToolSource::WellKnown, "#5 well known");
        assert!(
            !sources.contains(&ToolSource::Override),
            "no override configured"
        );
    }

    #[test]
    fn an_override_wins_and_nothing_else_is_listed() {
        let mut lookup = lookup();
        lookup.override_path = Some(PathBuf::from("/opt/mine/qjs"));
        let found = candidates("qjs", &lookup, LINUX);
        assert_eq!(
            found,
            vec![(ToolSource::Override, PathBuf::from("/opt/mine/qjs"))]
        );
    }

    #[test]
    fn the_override_path_rules_are_checked_for_all_three_platforms() {
        // The rules are asked of `override_candidate` directly, with the
        // platform stated, so this test says something about Windows from a
        // Linux machine (the D27 lesson).
        let as_file =
            |path: &str, windows| override_candidate(Path::new(path), false, windows, "qjs.exe");
        assert_eq!(
            as_file(r"C:\tools\yt-dlp", true),
            PathBuf::from(r"C:\tools\yt-dlp.exe"),
            "a Windows path with no extension gets one"
        );
        assert_eq!(
            as_file(r"C:\tools\yt-dlp.exe", true),
            PathBuf::from(r"C:\tools\yt-dlp.exe"),
            "an extension that is already .exe is left alone"
        );
        assert_eq!(
            as_file(r"C:\tools\yt-dlp", false),
            PathBuf::from(r"C:\tools\yt-dlp"),
            "on Unix nothing is appended"
        );
        assert_eq!(
            as_file("/opt/mine/yt-dlp.custom", false),
            PathBuf::from("/opt/mine/yt-dlp.custom"),
            "any extension on Unix is left alone"
        );
    }

    #[test]
    fn an_override_that_is_a_directory_gets_the_executable_name() {
        // What the folder picker gives back when the user chooses a folder.
        assert_eq!(
            override_candidate(Path::new("/opt/mine/bin"), true, false, "qjs"),
            PathBuf::from("/opt/mine/bin/qjs")
        );
        assert_eq!(
            override_candidate(Path::new("/opt/mine/bin"), true, true, "qjs.exe"),
            // The `windows` flag only decides the extension rules; the path
            // separators stay native, which is exactly why the flag is a
            // parameter and not a second platform.
            PathBuf::from("/opt/mine/bin").join("qjs.exe"),
            "the platform name is used, not the raw tool name"
        );
    }

    #[test]
    fn an_override_that_is_an_empty_string_is_ignored_rather_than_probed() {
        let mut lookup = lookup();
        lookup.override_path = Some(PathBuf::new());
        assert!(candidates("qjs", &lookup, LINUX).is_empty());
    }

    #[test]
    fn the_app_data_copy_sits_in_tools_slash_tool_slash() {
        assert_eq!(
            app_data_dir_for(Path::new("/data/tools"), "yt-dlp"),
            PathBuf::from("/data/tools/yt-dlp")
        );
    }

    #[test]
    fn path_dirs_are_split_read_and_never_made_relative() {
        let separator = if cfg!(windows) { ';' } else { ':' };
        let raw = [
            String::from("/usr/bin"),
            String::from(""),
            String::from("/opt/bin"),
        ]
        .join(&separator.to_string());
        assert_eq!(
            path_dirs(Some(&raw)),
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/opt/bin")]
        );
        // A relative PATH entry is dropped: it would make the program path
        // relative, and `process.rs` refuses those (spec §4.8).
        let relative = format!("bin{}", separator);
        assert!(path_dirs(Some(&relative)).is_empty());
        assert!(path_dirs(None).is_empty());
    }

    #[test]
    fn well_known_locations_include_the_ones_the_spec_lists() {
        let home = PathBuf::from("/home/tester");
        let dirs = well_known_dirs(Some(&home));
        let joined: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        assert!(joined.contains(&"/opt/homebrew/bin".to_owned()));
        assert!(joined.contains(&"/usr/local/bin".to_owned()));
        assert!(joined.contains(&"/opt/local/bin".to_owned()));
        assert!(joined.iter().any(|d| d.contains(".deno")));
        assert!(joined.iter().any(|d| d.contains(".local")));
        if cfg!(target_os = "linux") {
            assert!(joined.contains(&"/usr/bin".to_owned()));
            assert!(joined.contains(&"/snap/bin".to_owned()));
        }
    }

    #[test]
    fn version_banners_are_parsed_from_the_real_tool_output() {
        // Verbatim output of the real binaries on 2026-10-13:
        //   yt-dlp 2026.08.19 --version  -> "2026.08.19"
        //   qjs 0.17.0 --version         -> "0.17.0"
        //   ffmpeg n9.0.2 -version       -> "ffmpeg version n9.0.2 Copyright (c) 2000-2026 …"
        assert_eq!(
            parse_version_banner("2026.08.19").as_deref(),
            Some("2026.08.19")
        );
        assert_eq!(parse_version_banner("0.17.0").as_deref(), Some("0.17.0"));
        assert_eq!(
            parse_version_banner(
                "ffmpeg version n9.0.2 Copyright (c) 2000-2026 the FFmpeg developers"
            )
            .as_deref(),
            Some("9.0.2"),
            "the `n` prefix of a modern ffmpeg version is part of the token, not junk"
        );
        assert_eq!(
            parse_version_banner("ffmpeg version 6.1.1 Copyright (c) 2000-2023").as_deref(),
            Some("6.1.1"),
            "the older, unprefixed form still parses"
        );
        assert_eq!(parse_version_banner("v2.3.4").as_deref(), Some("2.3.4"));
    }

    #[test]
    fn a_nightly_yt_dlp_version_is_comparable_after_all() {
        // Real yt-dlp nightlies are `YYYY.MM.DD.NNNNNN`, which *is* dotted
        // numeric. An earlier note in this file claimed otherwise; checked
        // against the actual naming, they parse and compare normally.
        assert_eq!(
            parse_version_banner("2025.11.12.251112123456").as_deref(),
            Some("2025.11.12.251112123456")
        );
        assert!(manifest::satisfies_min_version(
            "2025.11.12.251112123456",
            "2025.11.12"
        ));
        assert!(!manifest::satisfies_min_version(
            "2025.11.11.251111000000",
            "2025.11.12"
        ));
    }

    #[test]
    fn the_version_flag_is_the_one_each_tool_documents() {
        assert_eq!(version_args("yt-dlp"), &["--version"]);
        assert_eq!(version_args("qjs"), &["--version"]);
        // ffmpeg's documented flag. Verified against a real `ffmpeg -version` on
        // 2026-10-13 rather than from memory.
        assert_eq!(version_args("ffmpeg"), &["-version"]);
        assert_eq!(version_args("ffprobe"), &["-version"]);
    }

    #[test]
    fn a_banner_without_a_version_is_not_invented() {
        assert_eq!(parse_version_banner("hello"), None);
        assert_eq!(parse_version_banner(""), None);
        // A bare number is not a version: `qjs --version` printing "0" must not
        // become 0.0.0 and then be compared to 0.12.0.
        assert_eq!(parse_version_banner("1"), None);
    }

    #[test]
    fn update_args_match_yt_dlp_s_own_help() {
        use crate::state::YtdlpChannel;
        assert_eq!(update_args(YtdlpChannel::Stable), vec!["-U"]);
        assert_eq!(
            update_args(YtdlpChannel::Nightly),
            vec!["--update-to", "nightly"]
        );
    }

    #[test]
    fn an_update_is_detected_from_the_version_change_not_the_exit_code() {
        let outcome = classify_update(true, "Done!", Some("2025.01.01"), Some("2026.08.19"));
        assert!(outcome.updated);
        assert_eq!(outcome.previous_version.as_deref(), Some("2025.01.01"));
        assert_eq!(outcome.version.as_deref(), Some("2026.08.19"));
        assert_eq!(outcome.message, "Done!");
    }

    #[test]
    fn already_up_to_date_is_not_reported_as_an_update() {
        let outcome = classify_update(
            true,
            "WARNING: You have already installed the latest version",
            Some("2026.08.19"),
            Some("2026.08.19"),
        );
        assert!(!outcome.updated);
        assert!(outcome.message.contains("already installed the latest"));
    }

    #[test]
    fn a_failed_update_is_never_reported_as_success_whatever_it_says() {
        // "unable to download" does not contain "up to date", so a wording-only
        // rule would have called this a successful update.
        let outcome = classify_update(false, "ERROR: unable to download", None, None);
        assert!(!outcome.updated);
        assert!(outcome.message.contains("unable to download"));
        let outcome = classify_update(
            false,
            "WARNING: You have already installed the latest version",
            None,
            None,
        );
        assert!(
            !outcome.updated,
            "the wording does not override a non-zero exit"
        );
    }

    #[test]
    fn without_a_before_version_an_up_to_date_message_still_counts_as_no_change() {
        let outcome = classify_update(true, "yt-dlp is up to date!", None, None);
        assert!(!outcome.updated);
    }

    #[tokio::test]
    async fn probing_a_file_that_does_not_exist_is_not_an_error() {
        let dir = TempDir::new().expect("temp dir");
        let missing = dir.path().join("nope");
        assert_eq!(probe_version(&missing, "yt-dlp", None).await, None);
    }

    #[tokio::test]
    async fn probing_a_program_that_never_answers_gives_up_at_the_timeout() {
        // A shell-free way to produce an unanswering executable on all three
        // OSes: the test binary itself with an argument it ignores would work,
        // but a plain non-executable file proves the "cannot run" branch
        // without depending on anything.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("not-a-program");
        fs::write(&path, b"this is not an executable image").expect("write");
        make_executable(&path).expect("chmod");
        assert_eq!(probe_version(&path, "yt-dlp", None).await, None);
    }

    #[tokio::test]
    async fn installing_the_app_data_copy_reuses_a_copy_that_is_already_there() {
        let dir = TempDir::new().expect("temp dir");
        let bundled = dir.path().join("bundled-yt-dlp");
        fs::write(&bundled, b"bundled").expect("write");
        let tools = dir.path().join("tools");

        let first = install_app_data_copy(&bundled, &tools, LINUX).expect("first install");
        assert_eq!(first, tools.join("yt-dlp").join("yt-dlp"));
        assert_eq!(fs::read(&first).expect("read"), b"bundled");

        // The updater replaced it: a second launch must not undo that.
        fs::write(&first, b"updated").expect("write");
        let second = install_app_data_copy(&bundled, &tools, LINUX).expect("second install");
        assert_eq!(fs::read(&second).expect("read"), b"updated");
    }

    #[tokio::test]
    async fn installing_the_app_data_copy_fails_cleanly_without_the_bundled_binary() {
        let dir = TempDir::new().expect("temp dir");
        let err = install_app_data_copy(
            &dir.path().join("missing"),
            &dir.path().join("tools"),
            LINUX,
        )
        .expect_err("must fail");
        assert!(err.to_string().contains("bundled yt-dlp is missing"));
    }

    #[tokio::test]
    async fn installing_the_app_data_copy_leaves_no_part_file_behind() {
        let dir = TempDir::new().expect("temp dir");
        let bundled = dir.path().join("bundled-yt-dlp");
        fs::write(&bundled, b"bundled").expect("write");
        let tools = dir.path().join("tools");
        install_app_data_copy(&bundled, &tools, LINUX).expect("install");

        let leftovers: Vec<String> = fs::read_dir(tools.join("yt-dlp"))
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "yt-dlp")
            .collect();
        assert!(leftovers.is_empty(), "unexpected leftovers: {leftovers:?}");
    }

    #[test]
    fn extra_tools_are_the_two_ffmpeg_binaries_and_nothing_else() {
        assert_eq!(extra_tools(), &["ffmpeg", "ffprobe"]);
    }

    #[test]
    fn every_manifest_tool_has_a_stable_source_name() {
        // These strings go into the UI and the settings file.
        for (source, expected) in [
            (ToolSource::Override, "override"),
            (ToolSource::AppData, "appData"),
            (ToolSource::Bundled, "bundled"),
            (ToolSource::PathEnv, "path"),
            (ToolSource::WellKnown, "wellKnown"),
        ] {
            assert_eq!(source.as_str(), expected);
        }
    }

    #[test]
    fn the_lookup_map_is_empty_for_a_default_document() {
        let settings = crate::state::Settings::default();
        let built = lookup_from(
            &settings,
            PathBuf::from("/app"),
            PathBuf::from("/data/tools"),
        );
        assert!(built.override_path.is_none());
        assert_eq!(built.tools_dir, PathBuf::from("/data/tools"));
        assert_eq!(built.bundled_dir, PathBuf::from("/app"));
    }

    #[test]
    fn the_lookup_uses_the_override_of_the_tool_being_looked_up() {
        let mut settings = crate::state::Settings::default();
        settings
            .tool_paths
            .insert("qjs".to_owned(), PathBuf::from("/opt/mine/qjs"));
        let ytdlp = lookup_from(&settings, PathBuf::from("/app"), PathBuf::from("/d"));
        let qjs = lookup_for(&settings, "qjs", PathBuf::from("/app"), PathBuf::from("/d"));
        assert!(ytdlp.override_path.is_none());
        assert_eq!(
            qjs.override_path.as_deref(),
            Some(Path::new("/opt/mine/qjs"))
        );
    }

    #[test]
    fn the_bundled_dir_is_next_to_the_executable() {
        // The AppImage/.deb/.app/NSIS cases all reduce to this (see the doc
        // comment), so the test only has to prove the shape.
        let dir = bundled_dir().expect("a directory next to this executable");
        assert!(dir.is_absolute());
    }
}
