//! JavaScript runtime strategy: QuickJS first, Deno as the automatic fallback
//! (spec §5.3).
//!
//! Why this file exists: yt-dlp enables Deno by default and prefers higher
//! priority runtimes (deno, node, quickjs, bun), so "use our bundled qjs" is not
//! a matter of *adding* a flag — every attempt must first clear the defaults with
//! `--no-js-runtimes`, and only then name the runtime it may use. Getting that
//! order wrong silently runs Deno (or nothing) while looking correct in the arg
//! list.
//!
//! Measured against the pinned yt-dlp (2026.08.19): `--no-js-runtimes` and
//! `--js-runtimes quickjs:<path>` exist, and the option's own help says
//! `--no-js-runtimes needs to be passed before enabling other runtimes`.
//!
//! The strategy is split in two halves on purpose:
//!
//! * pure decisions — which runtime to start with, whether a remembered outcome
//!   still applies ([`choose_start_runtime`], [`memory_applies`]) — unit-tested
//!   without running anything;
//! * [`RuntimeMemoryStore`], which persists "QuickJS failed and Deno worked"
//!   per `(yt-dlp version, qjs version)` for [`RUNTIME_MEMORY_TTL_SECS`]
//!   (spec §5.3.3), so the next start begins with Deno instead of paying the
//!   QuickJS timeout again.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::services::paths;

/// File name of the runtime memory inside the app data dir.
pub const RUNTIME_MEMORY_FILE: &str = "runtime-memory.json";

/// How long a remembered "Deno worked, QuickJS did not" lasts (spec §5.3.3:
/// "start with B for 24 h or until yt-dlp/qjs changes").
pub const RUNTIME_MEMORY_TTL_SECS: u64 = 24 * 60 * 60;

/// The YouTube client list from the original app, in one place (spec §5.3.7).
///
/// Passed **only for YouTube hosts**. Measured on 2026-10-14 with the pinned
/// yt-dlp: forcing this list takes the 4K sample from 49 to 31 formats — both
/// runs keep the full 2160…144 ladder, so the resolution list is unaffected, but
/// the reduction is real and the constant is isolated so it can be dropped or
/// overridden without touching the arg builders (the expert override field
/// belongs to the Settings work, spec §11.8).
pub const YOUTUBE_PLAYER_CLIENTS: &str = "youtube:player_client=web_embedded,web,tv";

// ---------------------------------------------------------------------------
// The runtimes
// ---------------------------------------------------------------------------

/// A JavaScript runtime the app knows how to point yt-dlp at (spec §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeKind {
    /// The bundled QuickJS-NG, named `quickjs` in yt-dlp's option.
    QuickJs,
    /// The fallback, enabled by default upstream but always named explicitly
    /// here so an attempt never depends on the user's own installations.
    Deno,
}

impl RuntimeKind {
    /// The name yt-dlp uses in `--js-runtimes <name>:<path>`.
    pub const fn ytdlp_name(self) -> &'static str {
        match self {
            Self::QuickJs => "quickjs",
            Self::Deno => "deno",
        }
    }

    /// The other runtime, for the one-shot fallback.
    pub const fn other(self) -> Self {
        match self {
            Self::QuickJs => Self::Deno,
            Self::Deno => Self::QuickJs,
        }
    }

    pub const fn as_str(self) -> &'static str {
        self.ytdlp_name()
    }
}

/// `Settings.jsRuntimeMode` (spec §5.3.3: *Auto (QuickJS→Deno)*, *QuickJS only*,
/// *Deno only*).
///
/// The explicit `rename`s keep the wire values (`auto`, `quickjs`, `deno`)
/// independent of the Rust variant names: with `rename_all` alone, `DenoOnly`
/// serialised as `"denoonly"`, the frontend fell back to `auto`, and a mode the
/// user picked was silently dropped on the next save. Found by the IPC
/// round-trip test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JsRuntimeMode {
    /// Try QuickJS, fall back to Deno once when the trigger conditions of
    /// spec §5.3.2 hold.
    #[default]
    Auto,
    /// Never fall back; a QuickJS failure is the user's answer.
    #[serde(rename = "quickjs")]
    QuickJsOnly,
    /// Start with Deno directly.
    #[serde(rename = "deno")]
    DenoOnly,
}

impl JsRuntimeMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::QuickJsOnly => "quickjs",
            Self::DenoOnly => "deno",
        }
    }
}

/// Lenient parse: an unknown mode is `Auto`, not a broken settings file.
pub fn runtime_mode_from_str(raw: &str) -> Option<JsRuntimeMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(JsRuntimeMode::Auto),
        "quickjs" | "quickjs-only" | "quickjsonly" => Some(JsRuntimeMode::QuickJsOnly),
        "deno" | "deno-only" | "denoonly" => Some(JsRuntimeMode::DenoOnly),
        _ => None,
    }
}

pub fn de_runtime_mode<'de, D: serde::Deserializer<'de>>(d: D) -> Result<JsRuntimeMode, D::Error> {
    let raw = String::deserialize(d)?;
    Ok(runtime_mode_from_str(&raw).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Flags (spec §5.3.1)
// ---------------------------------------------------------------------------

/// The two flags that force exactly one runtime, in the order that works.
///
/// `--no-js-runtimes` **before** `--js-runtimes`: the option's own help text in
/// the pinned yt-dlp says so, and the reverse order leaves the defaults enabled.
pub fn runtime_args(kind: RuntimeKind, path: &Path) -> Vec<std::ffi::OsString> {
    vec![
        "--no-js-runtimes".into(),
        "--js-runtimes".into(),
        runtime_flag(kind, path),
    ]
}

/// `"<name>:<absolute path>"`, the value yt-dlp's option expects.
pub fn runtime_flag(kind: RuntimeKind, path: &Path) -> std::ffi::OsString {
    let mut flag = std::ffi::OsString::from(format!("{}:", kind.ytdlp_name()));
    flag.push(path);
    flag
}

// ---------------------------------------------------------------------------
// YouTube hosts (spec §5.3.7, §6.3.1)
// ---------------------------------------------------------------------------

/// The host of an http(s) URL, lowercased, without userinfo or port.
///
/// Deliberately a small parser, not a URL crate: it only has to answer "is this
/// YouTube?" well enough to decide whether to pass extractor args, and the one
/// hostile case that matters — `youtube.com@evil.example` — is handled by
/// discarding everything before the last `@`, exactly as `download.rs` does for
/// its loopback check.
pub fn url_host(url: &str) -> Option<String> {
    let lowered = url.trim().to_ascii_lowercase();
    let rest = lowered
        .strip_prefix("https://")
        .or_else(|| lowered.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// `youtube.com`, any `*.youtube.com`, and `youtu.be`.
pub fn is_youtube_host(host: &str) -> bool {
    host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtu.be"
}

pub fn is_youtube_url(url: &str) -> bool {
    url_host(url).is_some_and(|host| is_youtube_host(&host))
}

/// The extractor args for a URL, when it is a YouTube one (spec §5.3.7).
pub fn youtube_extractor_args(url: &str) -> Option<&'static str> {
    is_youtube_url(url).then_some(YOUTUBE_PLAYER_CLIENTS)
}

// ---------------------------------------------------------------------------
// Remembered outcome (spec §5.3.3)
// ---------------------------------------------------------------------------

/// "QuickJS failed and Deno worked", keyed by the two versions it was decided
/// with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeMemory {
    /// The yt-dlp version the decision was made with.
    pub ytdlp_version: String,
    /// The qjs version at that moment; `None` when qjs could not be read.
    pub qjs_version: Option<String>,
    /// Which runtime to start with next time.
    pub preferred: RuntimeKind,
    /// Unix seconds. Part of the decision, not decoration: spec §5.3.3 gives the
    /// memory a 24 h life even when nothing changed.
    pub decided_at: u64,
}

impl Default for RuntimeMemory {
    fn default() -> Self {
        Self {
            ytdlp_version: String::new(),
            qjs_version: None,
            preferred: RuntimeKind::QuickJs,
            decided_at: 0,
        }
    }
}

/// Does a remembered decision still describe *this* pair of binaries, within
/// the TTL? (spec §5.3.3: "per (yt-dlp version, qjs version) … for 24 h or
/// until yt-dlp/qjs changes".)
pub fn memory_applies(
    memory: &RuntimeMemory,
    ytdlp_version: &str,
    qjs_version: Option<&str>,
    now: u64,
) -> bool {
    if memory.ytdlp_version != ytdlp_version {
        return false;
    }
    if memory.qjs_version.as_deref() != qjs_version {
        return false;
    }
    // A clock that jumped backwards must not make a stale memory eternal.
    now >= memory.decided_at && now - memory.decided_at < RUNTIME_MEMORY_TTL_SECS
}

/// Which runtime an attempt starts with (spec §5.3.3).
///
/// `quickjs_available` is passed in because "Auto + no QuickJS binary at all"
/// must still start with Deno instead of failing on a runtime the app does not
/// have: qjs ships in the installer, but "the bundled file is missing" is a
/// state the Tools page already knows how to report.
pub fn choose_start_runtime(
    mode: JsRuntimeMode,
    memory: Option<&RuntimeMemory>,
    ytdlp_version: &str,
    qjs_version: Option<&str>,
    quickjs_available: bool,
    now: u64,
) -> RuntimeKind {
    match mode {
        JsRuntimeMode::QuickJsOnly => RuntimeKind::QuickJs,
        JsRuntimeMode::DenoOnly => RuntimeKind::Deno,
        JsRuntimeMode::Auto => {
            if let Some(memory) = memory
                && memory_applies(memory, ytdlp_version, qjs_version, now)
                && memory.preferred == RuntimeKind::Deno
            {
                return RuntimeKind::Deno;
            }
            if quickjs_available {
                RuntimeKind::QuickJs
            } else {
                RuntimeKind::Deno
            }
        }
    }
}

/// The current time in Unix seconds; `0` is a safe fallback that simply means
/// "no memory applies".
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

/// The runtime memory as it lives on disk and in memory.
///
/// Writes are best effort: failing to remember a performance hint must never
/// fail the download that discovered it. Reads are lenient (a corrupt file
/// behaves like no file), which is why the file is *not* the settings document —
/// losing it costs one slow attempt, and a settings file that refuses to load is
/// a much bigger problem.
#[derive(Debug)]
pub struct RuntimeMemoryStore {
    path: PathBuf,
    memory: Mutex<Option<RuntimeMemory>>,
}

impl RuntimeMemoryStore {
    /// Load from `path`; a missing or unreadable file is an empty memory.
    pub fn load(path: PathBuf) -> Self {
        let memory = Self::read(&path);
        Self {
            path,
            memory: Mutex::new(memory),
        }
    }

    fn read(path: &Path) -> Option<RuntimeMemory> {
        let raw = fs::read_to_string(path).ok()?;
        match serde_json::from_str(&raw) {
            Ok(memory) => Some(memory),
            Err(err) => {
                // No secrets, no user content: a path and a parser message.
                eprintln!(
                    "scaricatore-di-porto: ignoring the runtime memory at {} ({err})",
                    path.display()
                );
                None
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self) -> Option<RuntimeMemory> {
        match self.memory.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Remember that `preferred` is the runtime to start with, for this pair of
    /// versions. Failures are logged, not propagated.
    pub fn remember(
        &self,
        ytdlp_version: &str,
        qjs_version: Option<&str>,
        preferred: RuntimeKind,
        decided_at: u64,
    ) {
        let memory = RuntimeMemory {
            ytdlp_version: ytdlp_version.to_owned(),
            qjs_version: qjs_version.map(str::to_owned),
            preferred,
            decided_at,
        };
        {
            let mut slot = match self.memory.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            *slot = Some(memory.clone());
        }
        if let Err(err) = self.persist(&memory) {
            eprintln!(
                "scaricatore-di-porto: could not save the runtime memory to {} ({err})",
                self.path.display()
            );
        }
    }

    /// Forget the decision (used when the pair of versions changed beyond the
    /// TTL is not enough: a manual "test again" resets it).
    pub fn clear(&self) {
        {
            let mut slot = match self.memory.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            *slot = None;
        }
        let _ = fs::remove_file(&self.path);
    }

    fn persist(&self, memory: &RuntimeMemory) -> io::Result<()> {
        let json = serde_json::to_string_pretty(memory)
            .map_err(|err| io::Error::other(format!("serialising the runtime memory: {err}")))?;
        if let Some(parent) = self.path.parent() {
            paths::ensure_dir(parent)?;
        }
        paths::atomic_write(&self.path, json.as_bytes())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn runtime_args_clear_defaults_before_enabling_the_runtime() {
        // The whole point of the module: the reverse order leaves yt-dlp's
        // default runtimes enabled (spec §5.3.1).
        let args = runtime_args(RuntimeKind::QuickJs, Path::new("/opt/qjs"));
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rendered,
            vec!["--no-js-runtimes", "--js-runtimes", "quickjs:/opt/qjs"]
        );

        let args = runtime_args(RuntimeKind::Deno, Path::new("C:\\tools\\deno.exe"));
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rendered,
            vec![
                "--no-js-runtimes",
                "--js-runtimes",
                "deno:C:\\tools\\deno.exe"
            ]
        );
    }

    #[test]
    fn the_runtime_flag_is_name_colon_path() {
        assert_eq!(
            runtime_flag(RuntimeKind::QuickJs, Path::new("/usr/bin/qjs")),
            std::ffi::OsString::from("quickjs:/usr/bin/qjs")
        );
        assert_eq!(RuntimeKind::Deno.ytdlp_name(), "deno");
        assert_eq!(RuntimeKind::QuickJs.other(), RuntimeKind::Deno);
    }

    #[test]
    fn url_host_strips_scheme_port_userinfo_and_case() {
        assert_eq!(
            url_host("https://www.youtube.com/watch?v=x").as_deref(),
            Some("www.youtube.com")
        );
        assert_eq!(
            url_host("HTTP://Example.ORG:8080/a").as_deref(),
            Some("example.org")
        );
        assert_eq!(
            url_host("https://user:pw@host.example/x").as_deref(),
            Some("host.example")
        );
        // The hostile one: anything before the last @ is userinfo.
        assert_eq!(
            url_host("https://youtube.com@evil.example/x").as_deref(),
            Some("evil.example")
        );
        assert_eq!(url_host("not a url"), None);
        assert_eq!(url_host("file:///etc/passwd"), None);
    }

    #[test]
    fn youtube_hosts_are_recognised_and_nothing_else_is() {
        assert!(is_youtube_url(
            "https://www.youtube.com/watch?v=jNQXAC9IVRw"
        ));
        assert!(is_youtube_url("https://youtube.com/playlist?list=PL1"));
        assert!(is_youtube_url("https://m.youtube.com/watch?v=x"));
        assert!(is_youtube_url("https://music.youtube.com/watch?v=x"));
        assert!(is_youtube_url("https://youtu.be/jNQXAC9IVRw"));

        assert!(!is_youtube_url("https://notyoutube.com/watch?v=x"));
        assert!(!is_youtube_url("https://youtube.com.evil.example/watch"));
        assert!(!is_youtube_url("https://vimeo.com/12345"));
        assert!(!is_youtube_url("https://example.org/youtube.com"));
    }

    #[test]
    fn extractor_args_are_only_for_youtube_hosts() {
        assert_eq!(
            youtube_extractor_args("https://www.youtube.com/watch?v=x"),
            Some(YOUTUBE_PLAYER_CLIENTS)
        );
        assert_eq!(youtube_extractor_args("https://vimeo.com/1"), None);
    }

    #[test]
    fn an_unknown_runtime_mode_degrades_to_auto() {
        assert_eq!(runtime_mode_from_str("AUTO"), Some(JsRuntimeMode::Auto));
        assert_eq!(
            runtime_mode_from_str(" QuickJS "),
            Some(JsRuntimeMode::QuickJsOnly)
        );
        assert_eq!(runtime_mode_from_str("deno"), Some(JsRuntimeMode::DenoOnly));
        assert_eq!(runtime_mode_from_str("bun"), None);

        #[derive(serde::Deserialize)]
        struct Holder {
            #[serde(deserialize_with = "de_runtime_mode")]
            mode: JsRuntimeMode,
        }
        let holder: Holder = serde_json::from_str(r#"{"mode":"BUN"}"#).expect("lenient");
        assert_eq!(holder.mode, JsRuntimeMode::Auto);
        let holder: Holder = serde_json::from_str(r#"{"mode":"deno"}"#).expect("parse");
        assert_eq!(holder.mode, JsRuntimeMode::DenoOnly);
    }

    #[test]
    fn the_wire_values_of_the_runtime_mode_are_auto_quickjs_deno() {
        // The frontend keys its dictionary by these exact strings; the derive's
        // default would have produced "quickjsonly"/"denoonly".
        assert_eq!(
            serde_json::to_value(JsRuntimeMode::Auto).expect("serialise"),
            serde_json::json!("auto")
        );
        assert_eq!(
            serde_json::to_value(JsRuntimeMode::QuickJsOnly).expect("serialise"),
            serde_json::json!("quickjs")
        );
        assert_eq!(
            serde_json::to_value(JsRuntimeMode::DenoOnly).expect("serialise"),
            serde_json::json!("deno")
        );
        // And they round-trip through the same names.
        for mode in [
            JsRuntimeMode::Auto,
            JsRuntimeMode::QuickJsOnly,
            JsRuntimeMode::DenoOnly,
        ] {
            let json = serde_json::to_string(&mode).expect("serialise");
            let back: JsRuntimeMode = serde_json::from_str(&json).expect("parse");
            assert_eq!(back, mode);
        }
    }

    #[test]
    fn memory_only_applies_to_the_same_pair_within_the_ttl() {
        let memory = RuntimeMemory {
            ytdlp_version: "2026.08.19".into(),
            qjs_version: Some("0.17.0".into()),
            preferred: RuntimeKind::Deno,
            decided_at: 1_000,
        };
        assert!(memory_applies(&memory, "2026.08.19", Some("0.17.0"), 1_000));
        assert!(memory_applies(
            &memory,
            "2026.08.19",
            Some("0.17.0"),
            1_000 + RUNTIME_MEMORY_TTL_SECS - 1
        ));
        // Expired.
        assert!(!memory_applies(
            &memory,
            "2026.08.19",
            Some("0.17.0"),
            1_000 + RUNTIME_MEMORY_TTL_SECS
        ));
        // yt-dlp changed.
        assert!(!memory_applies(
            &memory,
            "2026.09.01",
            Some("0.17.0"),
            1_001
        ));
        // qjs changed.
        assert!(!memory_applies(
            &memory,
            "2026.08.19",
            Some("0.18.0"),
            1_001
        ));
        // qjs disappeared or could not be read.
        assert!(!memory_applies(&memory, "2026.08.19", None, 1_001));
        // Clock jumped backwards: stale, not eternal.
        assert!(!memory_applies(&memory, "2026.08.19", Some("0.17.0"), 999));
    }

    #[test]
    fn auto_starts_with_quickjs_unless_memory_says_deno() {
        let now = 10_000;
        assert_eq!(
            choose_start_runtime(
                JsRuntimeMode::Auto,
                None,
                "2026.08.19",
                Some("0.17.0"),
                true,
                now
            ),
            RuntimeKind::QuickJs
        );

        let memory = RuntimeMemory {
            ytdlp_version: "2026.08.19".into(),
            qjs_version: Some("0.17.0".into()),
            preferred: RuntimeKind::Deno,
            decided_at: now - 10,
        };
        assert_eq!(
            choose_start_runtime(
                JsRuntimeMode::Auto,
                Some(&memory),
                "2026.08.19",
                Some("0.17.0"),
                true,
                now
            ),
            RuntimeKind::Deno
        );
    }

    #[test]
    fn auto_without_a_quickjs_binary_starts_with_deno_instead_of_failing() {
        assert_eq!(
            choose_start_runtime(JsRuntimeMode::Auto, None, "2026.08.19", None, false, 0),
            RuntimeKind::Deno
        );
    }

    #[test]
    fn explicit_modes_ignore_memory() {
        let memory = RuntimeMemory {
            ytdlp_version: "2026.08.19".into(),
            qjs_version: Some("0.17.0".into()),
            preferred: RuntimeKind::Deno,
            decided_at: 100,
        };
        assert_eq!(
            choose_start_runtime(
                JsRuntimeMode::QuickJsOnly,
                Some(&memory),
                "2026.08.19",
                Some("0.17.0"),
                true,
                101
            ),
            RuntimeKind::QuickJs
        );
        assert_eq!(
            choose_start_runtime(JsRuntimeMode::DenoOnly, None, "x", None, true, 0),
            RuntimeKind::Deno
        );
    }

    #[test]
    fn the_store_round_trips_and_survives_a_corrupt_file() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(RUNTIME_MEMORY_FILE);

        let store = RuntimeMemoryStore::load(path.clone());
        assert!(store.get().is_none());

        store.remember("2026.08.19", Some("0.17.0"), RuntimeKind::Deno, 42);
        let reopened = RuntimeMemoryStore::load(path.clone());
        let memory = reopened.get().expect("memory survives a reload");
        assert_eq!(memory.preferred, RuntimeKind::Deno);
        assert_eq!(memory.ytdlp_version, "2026.08.19");
        assert_eq!(memory.qjs_version.as_deref(), Some("0.17.0"));
        assert_eq!(memory.decided_at, 42);

        // A corrupt file is "no memory", not an error.
        fs::write(&path, b"{not json").expect("write");
        let broken = RuntimeMemoryStore::load(path.clone());
        assert!(broken.get().is_none());

        reopened.clear();
        assert!(!path.exists());
        assert!(reopened.get().is_none());
    }
}
