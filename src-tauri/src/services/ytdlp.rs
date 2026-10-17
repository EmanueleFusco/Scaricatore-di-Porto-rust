//! Download execution through the yt-dlp CLI (spec §8).
//!
//! Everything in this file is either an arg builder or a parser, and both are
//! pure functions with unit tests — the runner in `services/job.rs` only wires
//! them to [`crate::services::process`] and to the events.
//!
//! ## What was verified, and how
//!
//! All flags below were checked with `--help` of the **pinned** bundled yt-dlp
//! (`2026.08.19`) on 2026-10-14, not against memory or the docs of an older
//! release. Two findings shaped this file:
//!
//! * `download:` progress templates are written to **stdout**, `postprocess:`
//!   ones to **stderr** — measured on a real run, hence the stderr sink of
//!   `process.rs`;
//! * `--playlist-end` no longer exists upstream (spec §6.2 names it): the
//!   equivalent is `-I :<n>`. M1 does not enumerate playlists, so nothing here
//!   uses it; M3 must use `-I`.
//!
//! The forbidden options of spec §8.1 (`--no-check-certificates`,
//! `--prefer-insecure`) appear nowhere outside the tests that assert their
//! absence.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::ErrorInfo;
use crate::services::jsruntime::{self, RuntimeKind};
use crate::services::process::ProcessResult;

// ---------------------------------------------------------------------------
// Constants — the single place where flag values live (spec §5.1, §8.2)
// ---------------------------------------------------------------------------

/// Download progress template (spec §8.3). Verified against the pinned binary.
pub const DOWNLOAD_PROGRESS_ARG: &str = "download:SDP_DL|%(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(info.vcodec)s";

/// Post-processing progress template (spec §8.3). Written to **stderr**.
pub const POSTPROCESS_PROGRESS_ARG: &str =
    "postprocess:SDP_PP|%(progress.status)s|%(progress.postprocessor)s";

/// The final-path print. `after_move:` is a later stage, so it does not turn
/// the download into a simulation; the file path comes from here and is never
/// guessed from the title (spec §8.3, §12.4).
pub const PRINT_FILE_ARG: &str = "after_move:SDP_FILE|%(filepath)s";

/// Prints the format yt-dlp **actually selected**, before downloading.
///
/// Why it exists: `-f "bv[height=1440]+ba/bv[height<=1440]+ba/b[height<=1440]/b"`
/// has a muxed fallback, so a degraded extraction (no separate video/audio
/// formats in that run) does not fail — it quietly picks `b[height<=1440]`,
/// which on YouTube is the 360p muxed format. Measured on the pinned yt-dlp:
/// one line per job, with the *combined* id (`278+251`, height 144) and `NA`
/// for the audio-only path. Comparing this height with what was asked for is
/// what turns a silent surprise into something the app can react to.
pub const PRINT_FORMAT_ARG: &str =
    "before_dl:SDP_FORMAT|%(format_id)s|%(height)s|%(vcodec)s|%(acodec)s";

/// Machine prefixes of the four template outputs.
pub const DOWNLOAD_PREFIX: &str = "SDP_DL|";
pub const POSTPROCESS_PREFIX: &str = "SDP_PP|";
pub const FILE_PREFIX: &str = "SDP_FILE|";
pub const FORMAT_PREFIX: &str = "SDP_FORMAT|";

/// Default file name template (spec §6.3.8: the id is always included because
/// titles elsewhere are often empty or duplicated; the original's ASCII-only
/// sanitising is one of the behaviours of spec §12 and is not ported).
pub const FILENAME_TEMPLATE: &str = "%(title).150B [%(id)s].%(ext)s";

/// File name template when the user disables `[%(id)s]` in Settings (spec §6.3.8, §11.8).
pub const FILENAME_TEMPLATE_NO_ID: &str = "%(title).150B.%(ext)s";

/// Format sort spec for the "Prefer H.264/AAC for compatibility" toggle (spec §6.3.7, §11.8).
pub const H264_SORT_SPEC: &str = "vcodec:h264,res,acodec:m4a";

/// Playlist item count at or above which pacing (`--sleep-interval 1 --max-sleep-interval 4`)
/// is automatically enabled (spec §6.4).
pub const PLAYLIST_PACING_THRESHOLD: usize = 20;

/// Default pacing interval `(min_secs, max_secs)` for large playlists (spec §6.4).
pub const DEFAULT_SLEEP_INTERVAL: (u32, u32) = (1, 4);

/// Escape any literal `%` in a playlist numbering prefix as `%%` so yt-dlp's
/// `-o` template expansion never misinterprets it (spec §6.4).
pub fn escape_template_literal(prefix: &str) -> String {
    prefix.replace('%', "%%")
}

/// Build the `-o` output template from an optional numbering prefix (e.g.
/// `"001 - "`) and the "Include video ID in filename" setting (spec §6.3.8, §6.4).
pub fn build_filename_template(prefix: Option<&str>, include_id: bool) -> String {
    let base = if include_id {
        FILENAME_TEMPLATE
    } else {
        FILENAME_TEMPLATE_NO_ID
    };
    match prefix.filter(|p| !p.is_empty()) {
        Some(raw) => format!("{}{base}", escape_template_literal(raw)),
        None => base.to_owned(),
    }
}

/// Format a 1-based playlist index (`1..=total`) as a zero-padded prefix like
/// `"01 - "` or `"001 - "` (at least 2 digits, or the width of `total`, min 3
/// when `total >= 100`; spec §6.4 uses `"001 - "` as the canonical example).
pub fn format_playlist_prefix(index: usize, total: usize) -> String {
    let width = total.max(1).to_string().len().max(3);
    format!("{index:0width$} - ")
}

/// Preferred merge container, MKV fallback (spec §8.2). Measured: VP9+Opus
/// streams choose MKV, so the final extension really is not always `.mp4`.
pub const MERGE_OUTPUT_FORMAT: &str = "mp4/mkv";

/// How much of the filename a title may take, in bytes. Kept in the template
/// itself; this exists so the number is visible once.
pub const TITLE_MAX_BYTES: usize = 150;

/// Network resilience defaults (spec §8.2).
pub const CONCURRENT_FRAGMENTS: &str = "4";
pub const RETRIES: &str = "10";
pub const FRAGMENT_RETRIES: &str = "10";
pub const SOCKET_TIMEOUT_SECS: &str = "30";

/// The output encoding the tool is forced to write, as an argv pair (spec §4.5).
///
/// Why it is needed, and why only the tool can be asked: our process layer
/// decodes tool output as UTF-8 ([`crate::services::process`], spec §4.5), but
/// a Python tool writes its stdout in the **console code page** when stdout is
/// a pipe — cp1252 on a western Windows. Non-ASCII characters of the final path
/// are then dropped or mangled, and the printed `SDP_FILE` path no longer names
/// the file that really exists: the app reported *"yt-dlp reported … but the
/// file is not there"* while the file sat in the output folder under its real
/// Unicode name, playable. The report is deterministic per title (only titles
/// with characters outside the code page are affected), which is why the same
/// links worked on Linux: there the pipe encoding is UTF-8 already.
///
/// Verified in the pinned tool's own source and `--help`:
/// `YoutubeDL._write_string` → `write_string(encoding=self.params['encoding'])`
/// and, when that is unset, `out.encoding or preferredencoding()` (the pipe's
/// code page). The standalone builds are PyInstaller bundles whose interpreter
/// runs an *isolated* config, so `PYTHONIOENCODING` cannot correct it from
/// outside — measured: the bundled binary ignores it, and a `C` locale is still
/// coerced to UTF-8, which is why this cannot be reproduced on Linux.
pub const ENCODING_ARGS: [&str; 2] = ["--encoding", "utf-8"];

// ---------------------------------------------------------------------------
// Job shape
// ---------------------------------------------------------------------------

/// Video or audio-only (spec §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobKind {
    Video,
    Audio,
}

/// How to choose formats (spec §8.2). Built in one pure function, unit-tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatChoice {
    /// Exact height, the original's behaviour: video at that height if it
    /// exists, otherwise the closest below.
    Exact { height: u32 },
    /// Max height for playlists and sites with unreliable heights; `?` keeps
    /// formats whose height is unknown.
    MaxHeight { height: u32 },
    /// The best muxed or separate pair.
    Best,
    /// Audio only: `-f ba/b -x --audio-format opus` (Ogg Opus).
    AudioOnly,
    /// An explicit format id from the advanced table; `has_audio` decides
    /// whether a separate audio stream has to be added.
    Explicit { format_id: String, has_audio: bool },
}

impl FormatChoice {
    /// The value of `-f`, or `None` for the audio-only path, which passes its
    /// own selector together with the extraction flags.
    pub fn selector(&self) -> Option<String> {
        match self {
            Self::Exact { height } => Some(format!(
                "bv[height={height}]+ba/bv[height<={height}]+ba/b[height<={height}]/b"
            )),
            Self::MaxHeight { height } => {
                Some(format!("bv*[height<=?{height}]+ba/b[height<=?{height}]/b"))
            }
            Self::Best => Some("bv*+ba/b".to_owned()),
            Self::AudioOnly => Some("ba/b".to_owned()),
            Self::Explicit {
                format_id,
                has_audio,
            } => {
                if *has_audio {
                    Some(format_id.clone())
                } else {
                    Some(format!("{format_id}+ba/{format_id}"))
                }
            }
        }
    }

    /// Does the run request audio extraction into Opus (spec §8.2)?
    pub fn extracts_audio(&self) -> bool {
        matches!(self, Self::AudioOnly)
    }
}

/// Where the job writes and where it works (spec §4.9, §8.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDirs {
    /// `--paths home:<out_dir>`
    pub home: PathBuf,
    /// `--paths temp:<out_dir>/.sdp-tmp-<itemid>`
    pub temp: PathBuf,
}

/// SponsorBlock action mode (spec §8.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SponsorBlockMode {
    /// Create chapter markers for sponsored segments (`--sponsorblock-mark`).
    Mark,
    /// Cut out sponsored segments (`--sponsorblock-remove`).
    Remove,
}

/// Everything the URL is downloaded with (spec §6.3, §6.4, §8.2, §8.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadSpec {
    pub kind: JobKind,
    pub quality: FormatChoice,
    pub cookies: CookieSource,
    /// Optional numbering prefix for playlist items (e.g. `"001 - "`), `%` escaped as `%%` in `-o` (spec §6.4).
    pub filename_prefix: Option<String>,
    /// Include `[%(id)s]` in the filename template (default `true`, spec §6.3.8, §11.8).
    pub include_id_in_filename: bool,
    /// Prefer H.264/AAC over VP9/AV1 (`-S vcodec:h264,res,acodec:m4a`, spec §6.3.7, §11.8).
    pub prefer_h264: bool,
    /// `--download-archive <path>` when `Some` (spec §6.4).
    pub archive_file: Option<PathBuf>,
    /// `(min_secs, max_secs)` → `--sleep-interval <min> --max-sleep-interval <max>` (spec §6.4).
    pub sleep_interval: Option<(u32, u32)>,
    /// `Some(true)` → `--live-from-start`, `Some(false)` → `--no-live-from-start` (spec §6.3.6).
    pub live_from_start: Option<bool>,
    /// `--proxy <url>` (spec §8.2, §11.8).
    pub proxy: Option<String>,
    /// `--limit-rate <rate>` (spec §8.2, §11.8).
    pub limit_rate: Option<String>,
    /// `--referer <url>` (spec §6.3.4, §11.3).
    pub referer: Option<String>,
    /// `--user-agent <ua>` (spec §6.3.4, §11.3).
    pub user_agent: Option<String>,
    /// Repeated `--add-headers FIELD:VALUE` (spec §6.3.4, §11.3).
    pub custom_headers: Vec<String>,
    /// `--impersonate chrome` (set on the one-shot anti-bot retry, spec §6.3.5, §8.5).
    pub impersonate_chrome: bool,
    /// Whether the runner may retry once with `--impersonate chrome` on `ANTIBOT` (spec §6.3.5, §11.8).
    pub impersonate_retry: bool,
    /// `--embed-metadata` (spec §8.4).
    pub embed_metadata: bool,
    /// `--embed-thumbnail` with `--convert-thumbnails jpg` (spec §8.4).
    pub embed_thumbnail: bool,
    /// `--embed-chapters` (spec §8.4).
    pub embed_chapters: bool,
    /// Optional SponsorBlock mode (`mark` or `remove`, spec §8.4).
    pub sponsorblock_mode: Option<SponsorBlockMode>,
    /// User-selectable SponsorBlock categories (default `sponsor,selfpromo,interaction`, spec §8.4).
    pub sponsorblock_categories: Vec<String>,
}

impl Default for DownloadSpec {
    fn default() -> Self {
        Self {
            kind: JobKind::Video,
            quality: FormatChoice::Best,
            cookies: CookieSource::None,
            filename_prefix: None,
            include_id_in_filename: true,
            prefer_h264: false,
            archive_file: None,
            sleep_interval: None,
            live_from_start: None,
            proxy: None,
            limit_rate: None,
            referer: None,
            user_agent: None,
            custom_headers: Vec::new(),
            impersonate_chrome: false,
            impersonate_retry: true,
            embed_metadata: false,
            embed_thumbnail: false,
            embed_chapters: false,
            sponsorblock_mode: None,
            sponsorblock_categories: Vec::new(),
        }
    }
}

impl DownloadSpec {
    pub fn new(kind: JobKind, quality: FormatChoice, cookies: CookieSource) -> Self {
        Self {
            kind,
            quality,
            cookies,
            ..Self::default()
        }
    }

    pub fn video(quality: FormatChoice) -> Self {
        Self::new(JobKind::Video, quality, CookieSource::None)
    }

    /// A selector exists for every video choice, `false` only when the caller
    /// deliberately omitted `-f` (the one-shot retry of spec §8.2).
    pub fn needs_merge(&self) -> bool {
        self.kind == JobKind::Video
    }
}

// ---------------------------------------------------------------------------
// Cookies (spec §8.6)
// ---------------------------------------------------------------------------

/// The browsers `--cookies-from-browser` knows, filtered by OS (spec §8.6:
/// Safari exists only on macOS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfgOs {
    Windows,
    MacOs,
    Linux,
}

impl CfgOs {
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// Stable order, shared by the settings dropdown and the tests.
pub const BROWSER_COOKIE_NAMES: [&str; 8] = [
    "chrome", "edge", "firefox", "brave", "opera", "vivaldi", "chromium", "safari",
];

/// The subset valid on `os`. `safari` is macOS-only; everything else is
/// available on all three.
pub fn browser_cookie_names(os: CfgOs) -> Vec<&'static str> {
    BROWSER_COOKIE_NAMES
        .iter()
        .copied()
        .filter(|name| *name != "safari" || os == CfgOs::MacOs)
        .collect()
}

/// Is `name` a browser the settings may keep on `os`?
pub fn is_known_browser(name: &str, os: CfgOs) -> bool {
    browser_cookie_names(os).contains(&name)
}

/// Where session cookies come from (spec §6.3, §8.6). Always optional.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CookieSource {
    #[default]
    None,
    /// `--cookies-from-browser <name>`
    Browser(String),
    /// `--cookies <file>` in Netscape format; more robust than browser
    /// decryption on Windows/Linux, and the only way on some setups.
    File(PathBuf),
}

impl CookieSource {
    /// The two flags, in argument order. Empty for [`CookieSource::None`].
    pub fn args(&self) -> Vec<OsString> {
        match self {
            Self::None => Vec::new(),
            Self::Browser(name) => vec![
                OsString::from("--cookies-from-browser"),
                OsString::from(name),
            ],
            Self::File(path) => vec![OsString::from("--cookies"), path.clone().into_os_string()],
        }
    }
}

/// Build the source from what the settings hold. A browser name unknown on this
/// OS is treated as "no cookies" instead of being handed to yt-dlp, where it
/// would only produce a confusing error.
pub fn cookie_source(browser: Option<&str>, file: Option<&Path>, os: CfgOs) -> CookieSource {
    if let Some(name) = browser {
        let normalized = name.trim().to_ascii_lowercase();
        if is_known_browser(&normalized, os) {
            return CookieSource::Browser(normalized);
        }
    }
    if let Some(path) = file {
        if !path.as_os_str().is_empty() {
            return CookieSource::File(path.to_path_buf());
        }
    }
    CookieSource::None
}

// ---------------------------------------------------------------------------
// Arg building (spec §8.2, §8.3)
// ---------------------------------------------------------------------------

/// Everything a download command needs beyond the user's options.
#[derive(Debug, Clone)]
pub struct DownloadEnvironment {
    /// `--ffmpeg-location` (spec §4.8: absolute, explicit).
    pub ffmpeg: Option<PathBuf>,
    /// `--no-js-runtimes --js-runtimes …`; `None` would let yt-dlp choose its
    /// own, which the app never wants (spec §5.3.1).
    pub runtime: Option<(RuntimeKind, PathBuf)>,
    /// Extra environment (private temp dir, spec §4.9).
    pub tmp_env: Option<PathBuf>,
    /// The expert override for YouTube's `player_client`; `None` lets yt-dlp
    /// use its own defaults, which is the recommended state (D74).
    pub youtube_clients: Option<String>,
}

/// The complete argv array, URL included after `--`.
///
/// `option = None` for the one-shot retry without `-f` (spec §8.2: "If yt-dlp
/// reports 'Requested format is not available', retry once without `-f`").
pub fn download_args(
    url: &str,
    spec: &DownloadSpec,
    dirs: &JobDirs,
    env: &DownloadEnvironment,
    omit_format: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();

    // Order matters for at least one pair: `--no-js-runtimes` must precede
    // `--js-runtimes` (spec §5.3.1). Everything else is order-independent.
    for flag in [
        "--ignore-config",
        "--no-playlist",
        "--newline",
        "--no-quiet",
        "--progress",
        "--no-warnings",
        "--color",
        "never",
    ] {
        args.push(flag.into());
    }

    // The tool writes what our reader decodes (spec §4.5, [`ENCODING_ARGS`]).
    args.extend(ENCODING_ARGS.map(OsString::from));

    args.push("--concurrent-fragments".into());
    args.push(CONCURRENT_FRAGMENTS.into());
    args.push("--retries".into());
    args.push(RETRIES.into());
    args.push("--fragment-retries".into());
    args.push(FRAGMENT_RETRIES.into());
    args.push("--socket-timeout".into());
    args.push(SOCKET_TIMEOUT_SECS.into());

    if let Some(ffmpeg) = &env.ffmpeg {
        args.push("--ffmpeg-location".into());
        args.push(ffmpeg.clone().into_os_string());
    }

    if let Some((kind, path)) = &env.runtime {
        args.extend(jsruntime::runtime_args(*kind, path));
    }

    // YouTube-only extractor args (spec §5.3.7): only when the user set an
    // override. By default yt-dlp's own client list is used, because it is the
    // one its maintainers keep working — see D74.
    if let Some(extractor_args) =
        jsruntime::youtube_extractor_args(url, env.youtube_clients.as_deref())
    {
        args.push("--extractor-args".into());
        args.push(extractor_args.into());
    }

    args.extend(spec.cookies.args());

    args.push("--paths".into());
    args.push(paths_arg("home", &dirs.home));
    args.push("--paths".into());
    args.push(paths_arg("temp", &dirs.temp));

    args.push("-o".into());
    args.push(
        build_filename_template(spec.filename_prefix.as_deref(), spec.include_id_in_filename)
            .into(),
    );
    // Safe file names on every OS (spec §8.2); the accented-title bug of the
    // original is spec §12.5.
    args.push("--windows-filenames".into());

    // The final container is never assumed: the real path comes from SDP_FILE.
    if spec.needs_merge() {
        args.push("--merge-output-format".into());
        args.push(MERGE_OUTPUT_FORMAT.into());
        if spec.prefer_h264 {
            args.push("-S".into());
            args.push(H264_SORT_SPEC.into());
        }
    }

    if let Some(archive) = &spec.archive_file
        && !archive.as_os_str().is_empty()
    {
        args.push("--download-archive".into());
        args.push(archive.clone().into_os_string());
    }

    if let Some((min_secs, max_secs)) = spec.sleep_interval
        && min_secs > 0
    {
        args.push("--sleep-interval".into());
        args.push(min_secs.to_string().into());
        if max_secs >= min_secs {
            args.push("--max-sleep-interval".into());
            args.push(max_secs.to_string().into());
        }
    }

    match spec.live_from_start {
        Some(true) => args.push("--live-from-start".into()),
        Some(false) => args.push("--no-live-from-start".into()),
        None => {}
    }

    if let Some(proxy) = spec
        .proxy
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("--proxy".into());
        args.push(proxy.into());
    }

    if let Some(rate) = spec
        .limit_rate
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("--limit-rate".into());
        args.push(rate.into());
    }

    if let Some(referer) = spec
        .referer
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("--referer".into());
        args.push(referer.into());
    }

    if let Some(ua) = spec
        .user_agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("--user-agent".into());
        args.push(ua.into());
    }

    for header in &spec.custom_headers {
        let trimmed = header.trim();
        if !trimmed.is_empty() {
            args.push("--add-headers".into());
            args.push(trimmed.into());
        }
    }

    if spec.impersonate_chrome {
        args.push("--impersonate".into());
        args.push("chrome".into());
    }

    if spec.embed_metadata {
        args.push("--embed-metadata".into());
        args.push("--no-embed-info-json".into());
        if !spec.embed_chapters {
            args.push("--no-embed-chapters".into());
        }
    }
    if spec.embed_thumbnail {
        args.push("--embed-thumbnail".into());
        args.push("--convert-thumbnails".into());
        args.push("jpg".into());
    }
    if spec.embed_chapters {
        args.push("--embed-chapters".into());
    }

    if jsruntime::is_youtube_url(url)
        && let Some(mode) = spec.sponsorblock_mode
    {
        let cats = if spec.sponsorblock_categories.is_empty() {
            "sponsor,selfpromo,interaction".to_owned()
        } else {
            spec.sponsorblock_categories.join(",")
        };
        match mode {
            SponsorBlockMode::Mark => {
                args.push("--sponsorblock-mark".into());
                args.push(cats.into());
            }
            SponsorBlockMode::Remove => {
                args.push("--sponsorblock-remove".into());
                args.push(cats.into());
            }
        }
    }

    args.push("--progress-template".into());
    args.push(DOWNLOAD_PROGRESS_ARG.into());
    args.push("--progress-template".into());
    args.push(POSTPROCESS_PROGRESS_ARG.into());
    args.push("--print".into());
    args.push(PRINT_FORMAT_ARG.into());
    args.push("--print".into());
    args.push(PRINT_FILE_ARG.into());
    // Passing `--print` implies `--quiet` in yt-dlp, which would suppress
    // the archive skip line (`[download] <id>: has already been recorded in the archive`).
    // Explicitly counteract it with `--no-quiet`.
    args.push("--no-quiet".into());

    if spec.quality.extracts_audio() {
        // `-x --audio-format opus` selects `ba/b` and converts to Ogg Opus
        // (spec §8.2); no `-f` on this path.
        args.push("-f".into());
        args.push(
            spec.quality
                .selector()
                .unwrap_or_else(|| "ba/b".to_owned())
                .into(),
        );
        args.push("-x".into());
        args.push("--audio-format".into());
        args.push("opus".into());
    } else if !omit_format && let Some(selector) = spec.quality.selector() {
        args.push("-f".into());
        args.push(selector.into());
    }

    // The URL last, after `--` (spec §4.6), in the same argv array.
    args.push("--".into());
    args.push(url.into());
    args
}

fn paths_arg(kind: &str, dir: &Path) -> OsString {
    let mut value = OsString::from(format!("{kind}:"));
    value.push(dir);
    value
}

/// Does a stdout line announce that yt-dlp skipped the video because its ID was
/// already in `--download-archive` (spec §6.4)?
///
/// Verified in yt-dlp's `YoutubeDL.in_download_archive`:
/// `[download] <title/id>: has already been recorded in the archive`.
pub fn is_archive_skip_line(line: &str) -> bool {
    line.contains("has already been recorded in the archive")
}

/// Parse the output of `yt-dlp --list-impersonate-targets` and return `true`
/// when at least one `chrome` target is available (spec §6.3.5).
pub fn supports_chrome_impersonation(stdout: &str) -> bool {
    stdout.lines().any(|line| {
        let lower = line.trim().to_ascii_lowercase();
        (lower.starts_with("chrome") || lower.contains(" chrome "))
            && !lower.contains("unavailable")
    })
}

/// Probe `yt-dlp --ignore-config --list-impersonate-targets` once per binary
/// and cache whether `--impersonate chrome` is supported (spec §6.3.5).
pub async fn probe_impersonate_chrome(ytdlp: &Path, private_temp: Option<&Path>) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, u64), bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let size = std::fs::metadata(ytdlp).map(|m| m.len()).unwrap_or(0);
    let key = (ytdlp.to_path_buf(), size);

    if let Ok(guard) = cache.lock()
        && let Some(&cached) = guard.get(&key)
    {
        return cached;
    }

    let mut cmd = crate::services::process::ToolCommand::new(ytdlp)
        .args(["--ignore-config", "--list-impersonate-targets"])
        .label("yt-dlp impersonate probe");
    if let Some(temp) = private_temp {
        cmd = cmd.temp_dir(temp);
    }

    let stdout = std::sync::Arc::new(Mutex::new(String::new()));
    let sink = {
        let stdout = std::sync::Arc::clone(&stdout);
        crate::services::process::line_sink(move |line| {
            if let Ok(mut buf) = stdout.lock() {
                buf.push_str(line);
                buf.push('\n');
            }
        })
    };
    let ok =
        crate::services::process::spawn_tool(cmd, tokio_util::sync::CancellationToken::new(), sink)
            .await
            .map(|res| res.success())
            .unwrap_or(false);

    let text = stdout.lock().map(|b| b.clone()).unwrap_or_default();
    let supported = ok && supports_chrome_impersonation(&text);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, supported);
    }
    supported
}

// ---------------------------------------------------------------------------
// Progress parsing (spec §8.3)
// ---------------------------------------------------------------------------

fn number(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()
}

fn float(value: &str) -> Option<f64> {
    value.trim().parse::<f64>().ok()
}

/// One `SDP_DL` line. `total` and `total_estimate` are separate because the UI
/// needs `total.or(total_estimate)`; `NA` is what yt-dlp prints for "unknown".
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadLine {
    pub status: String,
    pub downloaded: Option<u64>,
    pub total: Option<u64>,
    pub total_estimate: Option<u64>,
    pub speed: Option<f64>,
    pub eta: Option<u64>,
    /// `none` (audio-only stream), a codec name (video), or absent.
    pub vcodec: Option<String>,
}

impl DownloadLine {
    /// Is this an audio-only stream? Same rule as the original: `vcodec: none`.
    pub fn is_audio_stream(&self) -> bool {
        self.vcodec.as_deref() == Some("none")
    }

    /// The denominator to use, or `None` for an indeterminate bar.
    pub fn total_or_estimate(&self) -> Option<u64> {
        self.total.or(self.total_estimate)
    }

    /// 0.0–1.0, or `None` when the total is unknown (spec §8.3).
    pub fn fraction(&self) -> Option<f64> {
        let total = self.total_or_estimate()?;
        if total == 0 {
            return None;
        }
        let downloaded = self.downloaded? as f64;
        Some((downloaded / total as f64).clamp(0.0, 1.0))
    }
}

pub fn parse_download_line(line: &str) -> Option<DownloadLine> {
    let rest = line.strip_prefix(DOWNLOAD_PREFIX)?;
    let fields: Vec<&str> = rest.split('|').collect();
    if fields.len() != 7 {
        return None;
    }
    Some(DownloadLine {
        status: fields[0].trim().to_owned(),
        downloaded: number(fields[1]),
        total: number(fields[2]),
        total_estimate: number(fields[3]),
        speed: float(fields[4]),
        eta: number(fields[5]),
        vcodec: match fields[6].trim() {
            "" | "NA" => None,
            other => Some(other.to_owned()),
        },
    })
}

/// `(status, postprocessor)` of an `SDP_PP` line.
pub fn parse_postprocess_line(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix(POSTPROCESS_PREFIX)?;
    let mut fields = rest.splitn(2, '|');
    let status = fields.next()?.trim().to_owned();
    let name = fields.next().unwrap_or_default().trim().to_owned();
    Some((status, name))
}

/// The final path from `SDP_FILE`; the only source of truth for where the file
/// ended up (spec §8.3, §12.4).
pub fn parse_file_line(line: &str) -> Option<PathBuf> {
    let path = line.strip_prefix(FILE_PREFIX)?.trim();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

/// The format yt-dlp says it selected, from `SDP_FORMAT` (`before_dl`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatLine {
    /// `278+251` for a merge, `18` for a muxed format, `251` for audio.
    pub format_id: String,
    /// `None` for the audio-only path (`NA`).
    pub height: Option<u32>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
}

impl FormatLine {
    pub fn has_video(&self) -> bool {
        self.vcodec.as_deref().is_some_and(|codec| codec != "none")
    }
}

pub fn parse_format_line(line: &str) -> Option<FormatLine> {
    let rest = line.strip_prefix(FORMAT_PREFIX)?;
    let fields: Vec<&str> = rest.split('|').collect();
    if fields.len() != 4 {
        return None;
    }
    let text = |value: &str| match value.trim() {
        "" | "NA" => None,
        other => Some(other.to_owned()),
    };
    Some(FormatLine {
        format_id: fields[0].trim().to_owned(),
        height: number(fields[1]).and_then(|value| u32::try_from(value).ok()),
        vcodec: text(fields[2]),
        acodec: text(fields[3]),
    })
}

/// The height the user's choice asked for, when it asked for one.
pub fn requested_height(choice: &FormatChoice) -> Option<u32> {
    match choice {
        FormatChoice::Exact { height } | FormatChoice::MaxHeight { height } => Some(*height),
        FormatChoice::Best | FormatChoice::AudioOnly | FormatChoice::Explicit { .. } => None,
    }
}

/// Did the run deliver a lower resolution than the choice asked for?
///
/// Only when both sides are known: `MaxHeight`/`Exact` deliberately fall back to
/// a lower height in yt-dlp's own selector, so the answer "lower than asked" is
/// not by itself proof of anything — but on the pinned yt-dlp, with a *degraded*
/// extraction, the fallback is the muxed 360p format, and this is what tells the
/// two apart from the outside.
pub fn is_degraded_resolution(requested: Option<u32>, actual: Option<u32>) -> bool {
    match (requested, actual) {
        (Some(want), Some(got)) => got < want,
        _ => false,
    }
}

/// The *degraded-extraction* signature: a lower height **and** a muxed
/// selection.
///
/// A merged id (`278+251`) proves the separate video and audio streams were
/// found in that run, so a lower height there is a property of the video (the
/// requested one simply does not exist) and re-running would change nothing. A
/// muxed id (`18`) at a lower height is yt-dlp's own fallback chain landing on
/// the 360p format because the separate streams were missing — the case worth
/// one retry with the other runtime (spec §5.3.2).
///
/// `format_id` is `None` when the template did not print one: treat it as the
/// degraded case, because guessing "everything is fine" is the failure this
/// exists to prevent.
pub fn is_degraded_selection(
    requested: Option<u32>,
    actual: Option<u32>,
    format_id: Option<&str>,
) -> bool {
    if !is_degraded_resolution(requested, actual) {
        return false;
    }
    format_id.is_none_or(|id| !id.contains('+'))
}

/// A stable token the frontend translates; no concatenated sentences
/// (spec §11.9). `merging` is the "Merging…" case of spec §8.3.
pub fn postprocessor_label(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.contains("merger") {
        "merging"
    } else if lower.contains("move") {
        "moving"
    } else if lower.contains("extractaudio") || lower.contains("extract_audio") {
        "extractingAudio"
    } else if lower.contains("embed") || lower.contains("metadata") {
        "embedding"
    } else {
        "postProcessing"
    }
}

// ---------------------------------------------------------------------------
// Events (spec §3, §8.3)
// ---------------------------------------------------------------------------

/// The stage of the temporary single-video pipeline (M2 replaces this with the
/// queue state machine of spec §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobStage {
    Preparing,
    Downloading,
    PostProcessing,
    Encoding,
    Done,
    Failed,
    Cancelled,
}

/// The progress payload of spec §3: `{stage, percent|null, speedBps|null,
/// etaSec|null, label, subIndex?, subTotal?}`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub stage: JobStage,
    pub percent: Option<f64>,
    pub speed_bps: Option<f64>,
    pub eta_sec: Option<u64>,
    /// Stable token: `video` | `audio` | `merging` | `moving` |
    /// `extractingAudio` | `embedding` | `postProcessing` | `verifying`.
    pub label: String,
    pub sub_index: Option<u32>,
    pub sub_total: Option<u32>,
}

impl JobProgress {
    pub fn label(stage: JobStage, label: impl Into<String>) -> Self {
        Self {
            stage,
            percent: None,
            speed_bps: None,
            eta_sec: None,
            label: label.into(),
            sub_index: None,
            sub_total: None,
        }
    }
}

/// Tracks which stream is being downloaded, so "Video 1/2" and "Audio 2/2" are
/// reported (spec §8.3) instead of a bare percentage.
#[derive(Debug, Default)]
pub struct ProgressTracker {
    /// Kinds seen so far, in order of first appearance.
    seen: Vec<StreamKind>,
    /// Video means a merge: two streams expected.
    merge_expected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
}

impl ProgressTracker {
    pub fn new(merge_expected: bool) -> Self {
        Self {
            seen: Vec::new(),
            merge_expected,
        }
    }

    /// Fold one download line into a progress payload.
    pub fn on_download(&mut self, line: &DownloadLine) -> JobProgress {
        let kind = if line.is_audio_stream() {
            StreamKind::Audio
        } else {
            StreamKind::Video
        };
        if !self.seen.contains(&kind) {
            self.seen.push(kind);
        }
        let (label, sub_index) = match kind {
            StreamKind::Video => ("video", 1),
            // In a merge job the audio stream is always the second one, whatever
            // order they arrive in; an audio-only job has one stream, 1/1.
            StreamKind::Audio => ("audio", if self.merge_expected { 2 } else { 1 }),
        };
        JobProgress {
            stage: JobStage::Downloading,
            percent: line.fraction().map(|f| f * 100.0),
            speed_bps: line.speed,
            eta_sec: line.eta,
            label: label.to_owned(),
            sub_index: Some(sub_index),
            sub_total: Some(if self.merge_expected { 2 } else { 1 }),
        }
    }

    /// Fold one post-processing line. `None` for postprocessors that are only
    /// noise for the UI (`finished` MoveFiles is identical to `started`, and
    /// repeating it would flicker the label).
    pub fn on_postprocess(&mut self, status: &str, name: &str) -> Option<JobProgress> {
        if status != "started" {
            return None;
        }
        Some(JobProgress {
            stage: JobStage::PostProcessing,
            percent: None,
            speed_bps: None,
            eta_sec: None,
            label: postprocessor_label(name).to_owned(),
            sub_index: None,
            sub_total: None,
        })
    }
}

/// Emits at most one update per `min_interval` (spec §7.6/§11.7: coalesce to
/// ~10/s). Pure clock arithmetic so the tests do not sleep.
#[derive(Debug)]
pub struct ProgressThrottle {
    min_interval: Duration,
    last_emit: Option<Instant>,
}

impl ProgressThrottle {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_emit: None,
        }
    }

    /// `true` when this update should be sent now; records the moment.
    pub fn should_emit(&mut self, now: Instant) -> bool {
        match self.last_emit {
            Some(last) if now.duration_since(last) < self.min_interval => false,
            _ => {
                self.last_emit = Some(now);
                true
            }
        }
    }

    /// Record that an update went out *now*, without consulting the window.
    /// Used after a forced emission: the change itself must pass, but the next
    /// identical update still waits for its window.
    pub fn mark_emitted(&mut self, now: Instant) {
        self.last_emit = Some(now);
    }
}

// ---------------------------------------------------------------------------
// Error classification (spec §8.5)
// ---------------------------------------------------------------------------

/// Stable codes: they travel to the frontend, so renaming one is a breaking
/// change for the i18n dictionaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YtdlpErrorCode {
    FfmpegMissing,
    YtChallenge,
    Auth,
    Unavailable,
    Geo,
    Format,
    Unsupported,
    Drm,
    Antibot,
    Cookies,
    DiskFull,
    Network,
    Extract,
    /// QuickJS failed and the automatic fallback has no Deno to switch to
    /// (spec §5.3.2: "ask once, remember the answer" — the UI offers the
    /// install).
    DenoMissing,
    Unknown,
}

impl YtdlpErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FfmpegMissing => "FFMPEG_MISSING",
            Self::YtChallenge => "YT_CHALLENGE",
            Self::Auth => "AUTH",
            Self::Unavailable => "UNAVAILABLE",
            Self::Geo => "GEO",
            Self::Format => "FORMAT",
            Self::Unsupported => "UNSUPPORTED",
            Self::Drm => "DRM",
            Self::Antibot => "ANTIBOT",
            Self::Cookies => "COOKIES",
            Self::DiskFull => "DISK_FULL",
            Self::Network => "NETWORK",
            Self::Extract => "EXTRACT",
            Self::DenoMissing => "DENO_MISSING",
            Self::Unknown => "UNKNOWN",
        }
    }

    /// The inverse of [`YtdlpErrorCode::as_str`].
    ///
    /// The wire and the persisted queue carry the code as a string (it travels
    /// to the frontend and comes back inside `queue.json`), so reading one back
    /// has to be total. An unknown string is `UNKNOWN`: a code from a newer
    /// build is treated as "nobody recognised this", which is the same thing
    /// the classifier does with a stderr line it cannot place.
    pub fn parse(raw: &str) -> Self {
        match raw {
            "FFMPEG_MISSING" => Self::FfmpegMissing,
            "YT_CHALLENGE" => Self::YtChallenge,
            "AUTH" => Self::Auth,
            "UNAVAILABLE" => Self::Unavailable,
            "GEO" => Self::Geo,
            "FORMAT" => Self::Format,
            "UNSUPPORTED" => Self::Unsupported,
            "DRM" => Self::Drm,
            "ANTIBOT" => Self::Antibot,
            "COOKIES" => Self::Cookies,
            "DISK_FULL" => Self::DiskFull,
            "NETWORK" => Self::Network,
            "EXTRACT" => Self::Extract,
            "DENO_MISSING" => Self::DenoMissing,
            _ => Self::Unknown,
        }
    }

    /// Key into the `error.ytdlp.*` namespace of the i18n dictionaries.
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::FfmpegMissing => "error.ytdlp.ffmpegMissing",
            Self::YtChallenge => "error.ytdlp.ytChallenge",
            Self::Auth => "error.ytdlp.auth",
            Self::Unavailable => "error.ytdlp.unavailable",
            Self::Geo => "error.ytdlp.geo",
            Self::Format => "error.ytdlp.format",
            Self::Unsupported => "error.ytdlp.unsupported",
            Self::Drm => "error.ytdlp.drm",
            Self::Antibot => "error.ytdlp.antibot",
            Self::Cookies => "error.ytdlp.cookies",
            Self::DiskFull => "error.ytdlp.diskFull",
            Self::Network => "error.ytdlp.network",
            Self::Extract => "error.ytdlp.extract",
            Self::DenoMissing => "error.ytdlp.denoMissing",
            Self::Unknown => "error.ytdlp.unknown",
        }
    }

    /// Is a retry worth offering (spec §7.3: transient classes only)?
    pub const fn is_transient(self) -> bool {
        matches!(self, Self::Network | Self::Unknown)
    }
}

/// A classified failure, ready for the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YtdlpError {
    pub code: YtdlpErrorCode,
    /// The last stderr lines, raw: always one click away (spec §8.5).
    pub detail: String,
}

impl YtdlpError {
    pub fn new(code: YtdlpErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub fn to_error_info(&self) -> ErrorInfo {
        ErrorInfo {
            code: self.code.as_str().to_owned(),
            message_key: self.code.message_key().to_owned(),
            detail: Some(self.detail.clone()),
        }
    }
}

/// Map the stderr tail to a stable code (spec §8.5).
///
/// `is_youtube` matters for exactly one row: `HTTP Error 403` is a YouTube
/// challenge on YouTube and an anti-bot wall elsewhere. Everything else is
/// text-driven, and the order below is chosen so specific rows win over generic
/// ones (`Requested format is not available` before `Unsupported`, `Sign in…`
/// before the challenge patterns).
pub fn classify_stderr(stderr_tail: &[String], is_youtube: bool) -> YtdlpErrorCode {
    let text = stderr_tail.join("\n").to_ascii_lowercase();
    let has = |pattern: &str| text.contains(pattern);

    // 1. The obvious, actionable ones first.
    if has("ffmpeg not found") || has("ffmpeg is not installed") || has("ffmpeg not installed") {
        return YtdlpErrorCode::FfmpegMissing;
    }
    if has("no space left") || has("enospc") {
        return YtdlpErrorCode::DiskFull;
    }
    if has("failed to decrypt")
        || has("could not decrypt")
        || has("unable to decrypt")
        || has("cookies database")
        || has("cookie database")
        || has("keyring")
    {
        return YtdlpErrorCode::Cookies;
    }
    if has("drm protected") || has("drm-protected") || has("this video is drm") {
        return YtdlpErrorCode::Drm;
    }
    if has("requested format is not available") {
        return YtdlpErrorCode::Format;
    }

    // 2. Access problems. `Sign in to confirm` is AUTH, not YT_CHALLENGE: the
    //    hint the spec gives for it is cookies, and that is the one that works.
    if has("sign in to confirm")
        || has("not a bot")
        || has("age-restricted")
        || has("age restricted")
        || has("this video is private")
        || has("private video")
        || has("login required")
        || has("sign in if you")
        || has("members-only")
    {
        return YtdlpErrorCode::Auth;
    }

    // 3. Geography before the generic "not available": the messages overlap.
    if has("not available in your country")
        || has("geo-restricted")
        || has("geo restricted")
        || has("not available from your location")
    {
        return YtdlpErrorCode::Geo;
    }
    if has("video unavailable")
        || has("this video is unavailable")
        || has("has been removed")
        || has("no longer available")
        || has("removed by the uploader")
        || has("terminated")
    {
        return YtdlpErrorCode::Unavailable;
    }

    // 4. The challenge family. `403` splits by host: YouTube solvable, others
    //    anti-bot (spec §8.5, §6.3.3).
    if has("n challenge")
        || has("nsig")
        || has("signature extraction failed")
        || has("javascript runtime")
        || has("js runtime")
        || has("challenge solving")
        || has("unable to solve challenge")
        || has("failed to solve")
    {
        return YtdlpErrorCode::YtChallenge;
    }
    if has("http error 403") || has("http error: 403") {
        return if is_youtube {
            YtdlpErrorCode::YtChallenge
        } else {
            YtdlpErrorCode::Antibot
        };
    }

    // 5. Extraction vs unsupported.
    if has("unsupported url") || has("no video formats found") || has("no media found") {
        return YtdlpErrorCode::Unsupported;
    }
    if has("unable to extract") || has("failed to extract") || has("extractor error") {
        return YtdlpErrorCode::Extract;
    }

    // 6. Transient, auto-retryable (spec §7.3).
    if has("temporary failure in name resolution")
        || has("name or service not known")
        || has("connection refused")
        || has("connection reset")
        || has("connection aborted")
        || has("connection error")
        || has("timed out")
        || has("timeout")
        || has("network is unreachable")
        || has("unable to download")
        || has("http error 5")
    {
        return YtdlpErrorCode::Network;
    }

    YtdlpErrorCode::Unknown
}

/// Classify a finished run. `None` means "ran to completion, exit 0".
///
/// Deliberately independent of the exit code for the mapping itself: the codes
/// exist for the user, and a tool that exits 1 because ffmpeg is missing must
/// say so. `cancelled` is the caller's concern, not a classification.
pub fn classify_result(result: &ProcessResult, is_youtube: bool) -> Option<YtdlpError> {
    if result.cancelled {
        return None;
    }
    if result.success() {
        return None;
    }
    let code = classify_stderr(&result.stderr_tail, is_youtube);
    // Last 20 lines are plenty for a message and keep "Copy details" readable;
    // the full ring buffer stays available to the caller.
    let detail = tail_text(&result.stderr_tail, 20);
    Some(YtdlpError::new(code, detail))
}

/// The last `count` lines of a tail, joined. Empty input gives a message that
/// names the situation instead of an empty string.
pub fn tail_text(lines: &[String], count: usize) -> String {
    let start = lines.len().saturating_sub(count);
    let text = lines[start..].join("\n");
    if text.trim().is_empty() {
        return "the tool failed without writing anything to stderr".to_owned();
    }
    text
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn args_as_strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn dirs() -> JobDirs {
        JobDirs {
            home: PathBuf::from("/out"),
            temp: PathBuf::from("/out/.sdp-tmp-1"),
        }
    }

    fn plain_env() -> DownloadEnvironment {
        DownloadEnvironment {
            ffmpeg: Some(PathBuf::from("/usr/bin/ffmpeg")),
            runtime: Some((RuntimeKind::QuickJs, PathBuf::from("/opt/qjs"))),
            tmp_env: None,
            youtube_clients: None,
        }
    }

    fn spec(quality: FormatChoice) -> DownloadSpec {
        DownloadSpec::video(quality)
    }

    // -- selectors (spec §8.2, spec §15 "format-selector builder") -----------

    #[test]
    fn exact_height_selector_matches_the_spec() {
        assert_eq!(
            FormatChoice::Exact { height: 1080 }.selector().as_deref(),
            Some("bv[height=1080]+ba/bv[height<=1080]+ba/b[height<=1080]/b")
        );
    }

    #[test]
    fn max_height_selector_keeps_unknown_heights_thanks_to_the_question_mark() {
        assert_eq!(
            FormatChoice::MaxHeight { height: 720 }
                .selector()
                .as_deref(),
            Some("bv*[height<=?720]+ba/b[height<=?720]/b")
        );
    }

    #[test]
    fn best_and_audio_selectors_are_the_documented_ones() {
        assert_eq!(FormatChoice::Best.selector().as_deref(), Some("bv*+ba/b"));
        assert_eq!(FormatChoice::AudioOnly.selector().as_deref(), Some("ba/b"));
        assert!(FormatChoice::AudioOnly.extracts_audio());
        assert!(!FormatChoice::Best.extracts_audio());
    }

    #[test]
    fn explicit_ids_add_an_audio_stream_only_when_needed() {
        let video_only = FormatChoice::Explicit {
            format_id: "137".into(),
            has_audio: false,
        };
        assert_eq!(video_only.selector().as_deref(), Some("137+ba/137"));

        let muxed = FormatChoice::Explicit {
            format_id: "18".into(),
            has_audio: true,
        };
        assert_eq!(muxed.selector().as_deref(), Some("18"));
    }

    // -- the argv array (spec §4.6, §8.1, §8.2, §15) --------------------------

    #[test]
    fn the_base_arguments_are_all_present_and_the_url_comes_last_after_double_dash() {
        let args = download_args(
            "https://youtu.be/x",
            &spec(FormatChoice::Best),
            &dirs(),
            &plain_env(),
            false,
        );
        let rendered = args_as_strings(&args);

        for required in [
            "--ignore-config",
            "--no-playlist",
            "--newline",
            "--no-quiet",
            "--progress",
            "--no-warnings",
            "--color",
            "--concurrent-fragments",
            "--retries",
            "--fragment-retries",
            "--socket-timeout",
            "--ffmpeg-location",
            "--paths",
            "--windows-filenames",
            "--merge-output-format",
            "--progress-template",
            "--print",
            "-f",
        ] {
            assert!(
                rendered.iter().any(|arg| arg == required),
                "missing {required} in {rendered:?}"
            );
        }
        assert_eq!(rendered[rendered.len() - 2], "--");
        assert_eq!(rendered[rendered.len() - 1], "https://youtu.be/x");
        assert_eq!(
            &rendered[rendered.len() - 2..],
            &["--", "https://youtu.be/x"]
        );
        // No extractor args by default: yt-dlp's own client list (D74).
        assert!(
            !rendered.contains(&"--extractor-args".to_owned()),
            "the forced client list must not come back: {rendered:?}"
        );
    }

    /// The final path is the one string the app cannot survive a mojibake in:
    /// it is checked against the filesystem. On Windows a piped stdout makes the
    /// tool write the console code page, which drops every character outside it
    /// — verified in the tool's own source (`write_string` uses the stream's
    /// encoding) — and the check then says "the file is not there" about a file
    /// that is there. The flag has to be in the **base** args, so it is present
    /// on every path: video, audio-only, the retry without `-f`, and the
    /// inspection (D114, spec §4.5).
    #[test]
    fn the_tool_is_forced_to_utf8_on_every_path() {
        let cases = [
            ("video", spec(FormatChoice::Best), false),
            (
                "exact height",
                spec(FormatChoice::Exact { height: 1080 }),
                false,
            ),
            ("audio only", spec(FormatChoice::AudioOnly), false),
            ("retry without -f", spec(FormatChoice::Best), true),
        ];
        for (label, spec, omit_format) in cases {
            let args = download_args(
                "https://youtu.be/x",
                &spec,
                &dirs(),
                &plain_env(),
                omit_format,
            );
            let rendered = args_as_strings(&args);
            assert!(
                rendered.contains(&"--encoding".to_owned())
                    && rendered.contains(&"utf-8".to_owned()),
                "{label}: missing --encoding utf-8 in {rendered:?}"
            );
        }
    }

    #[test]
    fn no_insecure_flag_is_ever_built() {
        // Spec §8.1 forbids both, and spec §12.1 records that the original used
        // them. A test is the only way to keep a deleted line deleted.
        for quality in [
            FormatChoice::Exact { height: 1080 },
            FormatChoice::MaxHeight { height: 720 },
            FormatChoice::Best,
            FormatChoice::AudioOnly,
            FormatChoice::Explicit {
                format_id: "137".into(),
                has_audio: false,
            },
        ] {
            let args = args_as_strings(&download_args(
                "https://youtu.be/x",
                &spec(quality),
                &dirs(),
                &plain_env(),
                false,
            ));
            assert!(
                !args
                    .iter()
                    .any(|arg| arg == "--no-check-certificates" || arg == "--prefer-insecure"),
                "insecure flags leaked into {args:?}"
            );
        }
    }

    #[test]
    fn the_runtime_flags_keep_the_required_order() {
        let args = args_as_strings(&download_args(
            "https://youtu.be/x",
            &spec(FormatChoice::Best),
            &dirs(),
            &plain_env(),
            false,
        ));
        let clear = args.iter().position(|arg| arg == "--no-js-runtimes");
        let enable = args.iter().position(|arg| arg == "--js-runtimes");
        assert!(clear.is_some() && enable.is_some());
        assert!(clear < enable, "--no-js-runtimes must come first: {args:?}");
        assert!(args.contains(&"quickjs:/opt/qjs".to_owned()));
    }

    #[test]
    fn the_extractor_args_are_passed_only_when_overridden_and_only_for_youtube() {
        // Default: nothing, whatever the host (D74).
        let args = args_as_strings(&download_args(
            "https://www.youtube.com/watch?v=x",
            &spec(FormatChoice::Best),
            &dirs(),
            &plain_env(),
            false,
        ));
        assert!(!args.contains(&"--extractor-args".to_owned()));

        // With the expert override, the value reaches argv on YouTube…
        let mut env = plain_env();
        env.youtube_clients = Some("web_embedded,web,tv".to_owned());
        let args = args_as_strings(&download_args(
            "https://www.youtube.com/watch?v=x",
            &spec(FormatChoice::Best),
            &dirs(),
            &env,
            false,
        ));
        assert!(args.windows(2).any(|pair| pair
            == [
                "--extractor-args",
                "youtube:player_client=web_embedded,web,tv"
            ]));

        // …and not on another host.
        let args = args_as_strings(&download_args(
            "https://vimeo.com/1",
            &spec(FormatChoice::Best),
            &dirs(),
            &env,
            false,
        ));
        assert!(!args.contains(&"--extractor-args".to_owned()));
    }

    #[test]
    fn audio_only_asks_for_opus_and_keeps_the_paths() {
        let args = args_as_strings(&download_args(
            "https://youtu.be/x",
            &DownloadSpec::new(JobKind::Audio, FormatChoice::AudioOnly, CookieSource::None),
            &dirs(),
            &plain_env(),
            false,
        ));
        assert!(args.windows(2).any(|pair| pair == ["-x", "--audio-format"]));
        assert!(args.contains(&"opus".to_owned()));
        assert!(!args.contains(&"--merge-output-format".to_owned()));
        assert!(args.contains(&"home:/out".to_owned()));
        assert!(args.contains(&"temp:/out/.sdp-tmp-1".to_owned()));
    }

    #[test]
    fn the_format_retry_omits_the_selector_but_keeps_everything_else() {
        let args = args_as_strings(&download_args(
            "https://youtu.be/x",
            &spec(FormatChoice::MaxHeight { height: 1080 }),
            &dirs(),
            &plain_env(),
            true,
        ));
        assert!(
            !args.contains(&"-f".to_owned()),
            "the retry must not pass -f: {args:?}"
        );
        assert!(args.contains(&"--ignore-config".to_owned()));
        assert!(args.contains(&"--".to_owned()));
    }

    #[test]
    fn cookies_are_passed_as_the_two_different_flags() {
        let spec = DownloadSpec::new(
            JobKind::Video,
            FormatChoice::Best,
            CookieSource::Browser("firefox".into()),
        );
        let args = args_as_strings(&download_args(
            "https://youtu.be/x",
            &spec,
            &dirs(),
            &plain_env(),
            false,
        ));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--cookies-from-browser", "firefox"])
        );

        let spec = DownloadSpec::new(
            JobKind::Video,
            FormatChoice::Best,
            CookieSource::File(PathBuf::from("/home/u/cookies.txt")),
        );
        let args = args_as_strings(&download_args(
            "https://youtu.be/x",
            &spec,
            &dirs(),
            &plain_env(),
            false,
        ));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--cookies", "/home/u/cookies.txt"])
        );

        assert!(CookieSource::None.args().is_empty());
    }

    #[test]
    fn filename_templates_escape_percent_and_respect_id_toggle() {
        assert_eq!(
            build_filename_template(None, true),
            "%(title).150B [%(id)s].%(ext)s"
        );
        assert_eq!(
            build_filename_template(None, false),
            "%(title).150B.%(ext)s"
        );
        assert_eq!(
            build_filename_template(Some("001 - "), true),
            "001 - %(title).150B [%(id)s].%(ext)s"
        );
        assert_eq!(
            build_filename_template(Some("100% Live - "), false),
            "100%% Live - %(title).150B.%(ext)s"
        );
        assert_eq!(format_playlist_prefix(1, 12), "001 - ");
        assert_eq!(format_playlist_prefix(42, 1500), "0042 - ");
    }

    #[test]
    fn m3_flags_reach_argv_and_keep_url_after_double_dash() {
        let mut s = spec(FormatChoice::MaxHeight { height: 1080 });
        s.filename_prefix = Some("007 - ".into());
        s.include_id_in_filename = false;
        s.prefer_h264 = true;
        s.archive_file = Some(PathBuf::from("/data/archive.txt"));
        s.sleep_interval = Some((1, 4));
        s.live_from_start = Some(true);
        s.proxy = Some("socks5://127.0.0.1:1080".into());
        s.limit_rate = Some("5M".into());
        s.referer = Some("https://site.example/page".into());
        s.user_agent = Some("CustomUA/2.0".into());
        s.custom_headers = vec!["Origin: https://site.example".into()];
        s.impersonate_chrome = true;

        let args = args_as_strings(&download_args(
            "https://site.example/v/1",
            &s,
            &dirs(),
            &plain_env(),
            false,
        ));
        assert!(
            args.windows(2)
                .any(|p| p == ["-o", "007 - %(title).150B.%(ext)s"])
        );
        assert!(
            args.windows(2)
                .any(|p| p == ["-S", "vcodec:h264,res,acodec:m4a"])
        );
        assert!(
            args.windows(2)
                .any(|p| p == ["--download-archive", "/data/archive.txt"])
        );
        assert!(args.windows(2).any(|p| p == ["--sleep-interval", "1"]));
        assert!(args.windows(2).any(|p| p == ["--max-sleep-interval", "4"]));
        assert!(args.contains(&"--live-from-start".to_owned()));
        assert!(
            args.windows(2)
                .any(|p| p == ["--proxy", "socks5://127.0.0.1:1080"])
        );
        assert!(args.windows(2).any(|p| p == ["--limit-rate", "5M"]));
        assert!(
            args.windows(2)
                .any(|p| p == ["--referer", "https://site.example/page"])
        );
        assert!(
            args.windows(2)
                .any(|p| p == ["--user-agent", "CustomUA/2.0"])
        );
        assert!(
            args.windows(2)
                .any(|p| p == ["--add-headers", "Origin: https://site.example"])
        );
        assert!(args.windows(2).any(|p| p == ["--impersonate", "chrome"]));
        assert_eq!(args[args.len() - 2], "--");
        assert_eq!(args[args.len() - 1], "https://site.example/v/1");
    }

    #[test]
    fn archive_skip_and_impersonation_target_parsers_recognise_real_output() {
        assert!(is_archive_skip_line(
            "[download] dQw4w9WgXcQ: has already been recorded in the archive"
        ));
        assert!(!is_archive_skip_line("[download] 100% of 10.00MiB"));

        let available = "[info] Available impersonate targets\nClient     OS       Source\nChrome-136 Windows-10 curl_cffi\n";
        assert!(supports_chrome_impersonation(available));

        let unavailable =
            "[info] Available impersonate targets\nChrome-136 Windows-10 curl_cffi (unavailable)\n";
        assert!(!supports_chrome_impersonation(unavailable));
    }

    // -- cookies per OS (spec §8.6) -----------------------------------------

    #[test]
    fn safari_is_macos_only() {
        assert!(browser_cookie_names(CfgOs::MacOs).contains(&"safari"));
        assert!(!browser_cookie_names(CfgOs::Windows).contains(&"safari"));
        assert!(!browser_cookie_names(CfgOs::Linux).contains(&"safari"));
        for os in [CfgOs::Windows, CfgOs::MacOs, CfgOs::Linux] {
            let names = browser_cookie_names(os);
            for expected in [
                "chrome", "edge", "firefox", "brave", "opera", "vivaldi", "chromium",
            ] {
                assert!(names.contains(&expected), "{os:?} misses {expected}");
            }
        }
    }

    #[test]
    fn a_browser_unknown_on_this_os_is_replaced_by_the_cookie_file_then_by_nothing() {
        let file = Path::new("/home/u/cookies.txt");
        // Safari on Windows: invalid, so the file wins.
        assert_eq!(
            cookie_source(Some("Safari"), Some(file), CfgOs::Windows),
            CookieSource::File(file.to_path_buf())
        );
        // Nothing valid at all.
        assert_eq!(
            cookie_source(Some("netscape"), None, CfgOs::Linux),
            CookieSource::None
        );
        // Valid browser wins over the file (spec §8.6: either one).
        assert_eq!(
            cookie_source(Some("FIREFOX"), Some(file), CfgOs::Linux),
            CookieSource::Browser("firefox".into())
        );
    }

    // -- parsing (spec §8.3, spec §15 "progress/print parser") ----------------

    #[test]
    fn a_download_line_parses_with_and_without_a_total() {
        let line = "SDP_DL|downloading|12345|1000000|NA|345678.9|42|vp9";
        let parsed = parse_download_line(line).expect("parses");
        assert_eq!(parsed.status, "downloading");
        assert_eq!(parsed.downloaded, Some(12345));
        assert_eq!(parsed.total, Some(1_000_000));
        assert_eq!(parsed.total_estimate, None);
        assert_eq!(parsed.speed, Some(345_678.9));
        assert_eq!(parsed.eta, Some(42));
        assert_eq!(parsed.vcodec.as_deref(), Some("vp9"));
        assert!(!parsed.is_audio_stream());
        assert_eq!(parsed.fraction(), Some(0.012345));
    }

    #[test]
    fn an_unknown_total_is_an_indeterminate_bar_not_zero_percent() {
        let line = "SDP_DL|downloading|1000|NA|NA|1.0|NA|none";
        let parsed = parse_download_line(line).expect("parses");
        assert_eq!(parsed.total_or_estimate(), None);
        assert_eq!(parsed.fraction(), None);
        assert!(parsed.is_audio_stream());
    }

    #[test]
    fn the_estimate_is_used_when_the_real_total_is_missing() {
        let line = "SDP_DL|downloading|500|NA|1000|1.0|1|none";
        let parsed = parse_download_line(line).expect("parses");
        assert_eq!(parsed.total_or_estimate(), Some(1000));
        assert_eq!(parsed.fraction(), Some(0.5));
    }

    #[test]
    fn the_finished_status_parses_too() {
        let line = "SDP_DL|finished|252182|252182|NA|1149463.1|NA|none";
        let parsed = parse_download_line(line).expect("parses");
        assert_eq!(parsed.status, "finished");
        assert_eq!(parsed.eta, None);
    }

    #[test]
    fn lines_from_other_templates_are_ignored() {
        assert!(parse_download_line("[download]  12.3% of 1.00MiB").is_none());
        assert!(parse_download_line("SDP_DL|too|few|fields").is_none());
        assert!(parse_postprocess_line("SDP_DL|started|Merger").is_none());
        assert!(parse_file_line("SDP_DL|started|Merger").is_none());
    }

    #[test]
    fn the_postprocess_line_and_the_final_path_parse() {
        let (status, name) = parse_postprocess_line("SDP_PP|started|Merger").expect("parses");
        assert_eq!(status, "started");
        assert_eq!(name, "Merger");

        assert_eq!(
            parse_file_line("SDP_FILE|/out/Title [id].mkv"),
            Some(PathBuf::from("/out/Title [id].mkv"))
        );
        assert_eq!(parse_file_line("SDP_FILE|  "), None);
    }

    #[test]
    fn the_selected_format_line_parses_the_measured_shapes() {
        // Measured on the pinned yt-dlp (2026-10-14): a merge prints the
        // combined id and the video height.
        let merged = parse_format_line("SDP_FORMAT|278+251|144|vp9|opus").expect("parses");
        assert_eq!(merged.format_id, "278+251");
        assert_eq!(merged.height, Some(144));
        assert!(merged.has_video());
        assert_eq!(merged.acodec.as_deref(), Some("opus"));

        // The audio-only path prints `NA` for the height.
        let audio = parse_format_line("SDP_FORMAT|251|NA|none|opus").expect("parses");
        assert_eq!(audio.height, None);
        assert!(!audio.has_video());

        // A muxed format has both codecs.
        let muxed = parse_format_line("SDP_FORMAT|18|360|avc1.42001E|mp4a.40.2").expect("parses");
        assert_eq!(muxed.height, Some(360));
        assert!(muxed.has_video());

        assert!(parse_format_line("SDP_FORMAT|too|few|fields").is_none());
        assert!(parse_format_line("SDP_DL|x|y|z|w|v|u|t").is_none());
    }

    #[test]
    fn only_height_choices_carry_a_requested_height() {
        assert_eq!(
            requested_height(&FormatChoice::Exact { height: 1440 }),
            Some(1440)
        );
        assert_eq!(
            requested_height(&FormatChoice::MaxHeight { height: 720 }),
            Some(720)
        );
        assert_eq!(requested_height(&FormatChoice::Best), None);
        assert_eq!(requested_height(&FormatChoice::AudioOnly), None);
        assert_eq!(
            requested_height(&FormatChoice::Explicit {
                format_id: "137".into(),
                has_audio: false,
            }),
            None
        );
    }

    #[test]
    fn a_lower_than_asked_height_is_flagged_only_when_both_are_known() {
        assert!(is_degraded_resolution(Some(1440), Some(360)));
        assert!(!is_degraded_resolution(Some(1440), Some(1440)));
        // A higher result is not a problem to invent a warning about.
        assert!(!is_degraded_resolution(Some(720), Some(1080)));
        // Unknown on either side: no claim.
        assert!(!is_degraded_resolution(None, Some(360)));
        assert!(!is_degraded_resolution(Some(1440), None));
        assert!(!is_degraded_resolution(None, None));
    }

    #[test]
    fn the_degraded_signature_needs_a_muxed_selection() {
        // The user's case: muxed 360p when 1440p was asked for.
        assert!(is_degraded_selection(Some(1440), Some(360), Some("18")));
        // A real merge at a lower height: the video has no higher resolution,
        // so a retry would download the same file again.
        assert!(!is_degraded_selection(
            Some(1440),
            Some(720),
            Some("137+140")
        ));
        // Same as above, and the heights match: nothing at all.
        assert!(!is_degraded_selection(
            Some(720),
            Some(720),
            Some("137+140")
        ));
        assert!(!is_degraded_selection(Some(720), Some(720), Some("18")));
        // No format line: unknown, but the mismatch is real, so be cautious.
        assert!(is_degraded_selection(Some(1440), Some(360), None));
        // No requested height (Best, audio): never a degradation.
        assert!(!is_degraded_selection(None, Some(360), Some("18")));
    }

    #[test]
    fn postprocessor_names_become_stable_tokens() {
        assert_eq!(postprocessor_label("Merger"), "merging");
        assert_eq!(postprocessor_label("MoveFiles"), "moving");
        assert_eq!(postprocessor_label("ExtractAudio"), "extractingAudio");
        assert_eq!(postprocessor_label("FFmpegExtractAudio"), "extractingAudio");
        assert_eq!(postprocessor_label("EmbedThumbnail"), "embedding");
        assert_eq!(postprocessor_label("SomethingNew"), "postProcessing");
    }

    #[test]
    fn the_tracker_labels_the_two_streams_and_the_merge() {
        let mut tracker = ProgressTracker::new(true);
        let video = tracker.on_download(
            &parse_download_line("SDP_DL|downloading|1|2|NA|1.0|1|vp9").expect("line"),
        );
        assert_eq!(video.label, "video");
        assert_eq!(video.sub_index, Some(1));
        assert_eq!(video.sub_total, Some(2));

        let audio = tracker.on_download(
            &parse_download_line("SDP_DL|downloading|1|2|NA|1.0|1|none").expect("line"),
        );
        assert_eq!(audio.label, "audio");
        assert_eq!(audio.sub_index, Some(2));
        assert_eq!(audio.sub_total, Some(2));

        let merging = tracker
            .on_postprocess("started", "Merger")
            .expect("started is shown");
        assert_eq!(merging.stage, JobStage::PostProcessing);
        assert_eq!(merging.label, "merging");
        assert!(
            merging.percent.is_none(),
            "merging has no progress of its own"
        );

        assert!(tracker.on_postprocess("finished", "Merger").is_none());
    }

    #[test]
    fn an_audio_only_job_reports_one_stream() {
        let mut tracker = ProgressTracker::new(false);
        let audio = tracker.on_download(
            &parse_download_line("SDP_DL|downloading|1|2|NA|NA|NA|none").expect("line"),
        );
        assert_eq!(audio.sub_total, Some(1));
        assert_eq!(audio.sub_index, Some(1));
    }

    #[test]
    fn the_throttle_lets_ten_updates_a_second_through() {
        let mut throttle = ProgressThrottle::new(Duration::from_millis(100));
        let start = Instant::now();
        assert!(throttle.should_emit(start));
        assert!(!throttle.should_emit(start + Duration::from_millis(50)));
        assert!(throttle.should_emit(start + Duration::from_millis(100)));
        assert!(!throttle.should_emit(start + Duration::from_millis(150)));
        // A forced emission (a stage change that must not be coalesced away)
        // still counts as this window's update.
        throttle.mark_emitted(start + Duration::from_millis(150));
        assert!(!throttle.should_emit(start + Duration::from_millis(200)));
        assert!(throttle.should_emit(start + Duration::from_millis(250)));
    }

    // -- classifier (spec §8.5, spec §15 "error classifier") -----------------

    fn classify(lines: &[&str], youtube: bool) -> YtdlpErrorCode {
        let owned: Vec<String> = lines.iter().map(|line| (*line).to_owned()).collect();
        classify_stderr(&owned, youtube)
    }

    #[test]
    fn the_classifier_covers_every_row_of_the_spec_table() {
        // ffmpeg
        assert_eq!(
            classify(&["ERROR: Postprocessing: ffmpeg not found"], false),
            YtdlpErrorCode::FfmpegMissing
        );
        // A real message from the pinned binary for the audio path.
        assert_eq!(
            classify(
                &["ERROR: You have requested merging but ffmpeg is not installed"],
                false
            ),
            YtdlpErrorCode::FfmpegMissing
        );
        // YouTube challenge, both the 403 and the signature family.
        assert_eq!(
            classify(
                &["ERROR: unable to download video data: HTTP Error 403: Forbidden"],
                true
            ),
            YtdlpErrorCode::YtChallenge
        );
        assert_eq!(
            classify(&["WARNING: n challenge solving failed"], true),
            YtdlpErrorCode::YtChallenge
        );
        assert_eq!(
            classify(&["ERROR: Signature extraction failed"], true),
            YtdlpErrorCode::YtChallenge
        );
        assert_eq!(
            classify(
                &["ERROR: No supported JavaScript runtime could be found"],
                true
            ),
            YtdlpErrorCode::YtChallenge
        );
        // Auth.
        assert_eq!(
            classify(
                &[
                    "ERROR: [youtube] Sign in to confirm you're not a bot. Use --cookies-from-browser or --cookies"
                ],
                true
            ),
            YtdlpErrorCode::Auth
        );
        assert_eq!(
            classify(
                &["ERROR: [youtube] Private video. Sign in if you've been granted access"],
                true
            ),
            YtdlpErrorCode::Auth
        );
        assert_eq!(
            classify(&["ERROR: [youtube] This video is age-restricted"], true),
            YtdlpErrorCode::Auth
        );
        // Geo, then the generic unavailable.
        assert_eq!(
            classify(
                &["ERROR: [youtube] This video is not available in your country"],
                true
            ),
            YtdlpErrorCode::Geo
        );
        assert_eq!(
            classify(&["ERROR: [youtube] aaa: This video is unavailable"], true),
            YtdlpErrorCode::Unavailable
        );
        assert_eq!(
            classify(&["ERROR: Video has been removed by the uploader"], true),
            YtdlpErrorCode::Unavailable
        );
        // Format: the exact wording measured on 2026-10-14.
        assert_eq!(
            classify(
                &[
                    "ERROR: [youtube] x: Requested format is not available. Use --list-formats for a list of available formats"
                ],
                true
            ),
            YtdlpErrorCode::Format
        );
        // Unsupported.
        assert_eq!(
            classify(&["ERROR: Unsupported URL: https://example.com/"], false),
            YtdlpErrorCode::Unsupported
        );
        assert_eq!(
            classify(&["ERROR: [generic] No video formats found!"], false),
            YtdlpErrorCode::Unsupported
        );
        // DRM.
        assert_eq!(
            classify(&["ERROR: [generic] This video is DRM protected"], false),
            YtdlpErrorCode::Drm
        );
        // Anti-bot: 403 on a non-YouTube host.
        assert_eq!(
            classify(
                &["ERROR: unable to download webpage: HTTP Error 403: Forbidden"],
                false
            ),
            YtdlpErrorCode::Antibot
        );
        // Cookies.
        assert_eq!(
            classify(&["ERROR: could not decrypt cookies from Chrome"], false),
            YtdlpErrorCode::Cookies
        );
        assert_eq!(
            classify(&["ERROR: ERROR: Failed to decrypt with DPAPI"], false),
            YtdlpErrorCode::Cookies
        );
        // Disk full.
        assert_eq!(
            classify(&["OSError: [Errno 28] No space left on device"], false),
            YtdlpErrorCode::DiskFull
        );
        // Network family.
        assert_eq!(
            classify(
                &[
                    "ERROR: [youtube] unable to download API page: <urlopen error [Errno -3] Temporary failure in name resolution>"
                ],
                true
            ),
            YtdlpErrorCode::Network
        );
        assert_eq!(
            classify(&["ERROR: The read operation timed out"], false),
            YtdlpErrorCode::Network
        );
        assert_eq!(
            classify(
                &["ERROR: unable to download webpage: HTTP Error 503: Service Unavailable"],
                false
            ),
            YtdlpErrorCode::Network
        );
        // Extractor.
        assert_eq!(
            classify(&["ERROR: [youtube] Unable to extract player URL"], true),
            YtdlpErrorCode::Extract
        );
        // Anything else.
        assert_eq!(
            classify(&["ERROR: something nobody planned for"], false),
            YtdlpErrorCode::Unknown
        );
        assert_eq!(classify(&[], false), YtdlpErrorCode::Unknown);
    }

    #[test]
    fn classification_only_happens_for_a_failed_run() {
        let success = ProcessResult {
            label: "yt-dlp".into(),
            pid: Some(1),
            exit_code: Some(0),
            cancelled: false,
            stderr_tail: vec!["WARNING: ffmpeg not found".into()],
            duration_ms: 1,
        };
        assert!(classify_result(&success, true).is_none());

        let cancelled = ProcessResult {
            cancelled: true,
            ..success.clone()
        };
        assert!(classify_result(&cancelled, true).is_none());

        let failed = ProcessResult {
            exit_code: Some(1),
            stderr_tail: vec!["ERROR: Sign in to confirm you're not a bot".into()],
            ..success
        };
        let error = classify_result(&failed, true).expect("classified");
        assert_eq!(error.code, YtdlpErrorCode::Auth);
        assert!(error.detail.contains("Sign in"));
    }

    #[test]
    fn only_the_transient_classes_are_retryable() {
        assert!(YtdlpErrorCode::Network.is_transient());
        assert!(!YtdlpErrorCode::Auth.is_transient());
        assert!(!YtdlpErrorCode::Format.is_transient());
    }

    /// Every error code, so a new one has to be added here and looked at.
    const CODES: [YtdlpErrorCode; 15] = [
        YtdlpErrorCode::FfmpegMissing,
        YtdlpErrorCode::YtChallenge,
        YtdlpErrorCode::Auth,
        YtdlpErrorCode::Unavailable,
        YtdlpErrorCode::Geo,
        YtdlpErrorCode::Format,
        YtdlpErrorCode::Unsupported,
        YtdlpErrorCode::Drm,
        YtdlpErrorCode::Antibot,
        YtdlpErrorCode::Cookies,
        YtdlpErrorCode::DiskFull,
        YtdlpErrorCode::Network,
        YtdlpErrorCode::Extract,
        YtdlpErrorCode::DenoMissing,
        YtdlpErrorCode::Unknown,
    ];

    #[test]
    fn error_codes_have_distinct_stable_strings_and_keys() {
        let mut strings: Vec<&str> = CODES.iter().map(|c| c.as_str()).collect();
        let mut keys: Vec<&str> = CODES.iter().map(|c| c.message_key()).collect();
        let count = CODES.len();
        strings.sort_unstable();
        strings.dedup();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(strings.len(), count);
        assert_eq!(keys.len(), count);
    }

    #[test]
    fn every_error_code_has_a_dictionary_entry() {
        let missing: Vec<_> = CODES
            .iter()
            .map(|c| c.message_key())
            .filter(|key| !crate::error::message_key_in_dictionaries(key))
            .collect();
        assert!(
            missing.is_empty(),
            "message keys missing from the i18n dictionaries: {missing:?}"
        );
    }

    #[test]
    fn the_classified_error_serialises_with_the_frontend_shape() {
        let error = YtdlpError::new(YtdlpErrorCode::YtChallenge, "detail");
        let info: ErrorInfo = error.to_error_info();
        assert_eq!(info.code, "YT_CHALLENGE");
        assert_eq!(info.message_key, "error.ytdlp.ytChallenge");
        assert_eq!(info.detail.as_deref(), Some("detail"));
        // The code is a `String` on purpose: the frontend keys its dictionary by
        // the same value, and the test above pins the strings.
    }

    #[test]
    fn a_failure_without_stderr_still_has_a_readable_detail() {
        assert!(tail_text(&[], 20).contains("stderr"));
        let lines: Vec<String> = (0..30).map(|i| format!("line-{i}")).collect();
        let tail = tail_text(&lines, 20);
        assert!(!tail.contains("line-9\n"));
        assert!(tail.contains("line-29"));
    }

    #[test]
    fn download_args_include_embeds_and_sponsorblock() {
        let env = plain_env();
        let dirs = JobDirs {
            home: PathBuf::from("/home/user/downloads"),
            temp: PathBuf::from("/home/user/downloads/.tmp-123"),
        };
        let mut spec = DownloadSpec::video(FormatChoice::Best);
        spec.embed_metadata = true;
        spec.embed_thumbnail = true;
        spec.embed_chapters = true;
        spec.sponsorblock_mode = Some(SponsorBlockMode::Mark);
        spec.sponsorblock_categories = vec!["sponsor".to_string(), "interaction".to_string()];

        let args = download_args("https://youtube.com/watch?v=abc", &spec, &dirs, &env, false);
        let strings = args_as_strings(&args);

        assert!(strings.contains(&"--embed-metadata".to_string()));
        assert!(strings.contains(&"--no-embed-info-json".to_string()));
        assert!(strings.contains(&"--embed-thumbnail".to_string()));
        assert!(strings.contains(&"--convert-thumbnails".to_string()));
        assert!(strings.contains(&"jpg".to_string()));
        assert!(strings.contains(&"--embed-chapters".to_string()));
        assert!(!strings.contains(&"--no-embed-chapters".to_string()));
        assert!(strings.contains(&"--sponsorblock-mark".to_string()));
        assert!(strings.contains(&"sponsor,interaction".to_string()));

        // Test SponsorBlock remove mode with default categories
        let mut spec_remove = DownloadSpec::video(FormatChoice::Best);
        spec_remove.sponsorblock_mode = Some(SponsorBlockMode::Remove);
        let args_remove = download_args(
            "https://youtube.com/watch?v=abc",
            &spec_remove,
            &dirs,
            &env,
            false,
        );
        let strings_remove = args_as_strings(&args_remove);

        assert!(strings_remove.contains(&"--sponsorblock-remove".to_string()));
        assert!(strings_remove.contains(&"sponsor,selfpromo,interaction".to_string()));

        // Spec §8.4: SponsorBlock is YouTube-only; non-YouTube URLs must never get SponsorBlock flags
        let args_non_yt = download_args("https://vimeo.com/12345", &spec, &dirs, &env, false);
        let strings_non_yt = args_as_strings(&args_non_yt);
        assert!(!strings_non_yt.contains(&"--sponsorblock-mark".to_string()));
        assert!(!strings_non_yt.contains(&"--sponsorblock-remove".to_string()));
    }

    #[test]
    fn embed_metadata_without_chapters_passes_no_embed_chapters() {
        let env = plain_env();
        let dirs = JobDirs {
            home: PathBuf::from("/home/user/downloads"),
            temp: PathBuf::from("/home/user/downloads/.tmp-123"),
        };
        let mut spec = DownloadSpec::video(FormatChoice::Best);
        spec.embed_metadata = true;
        spec.embed_chapters = false;

        let args = download_args("https://youtube.com/watch?v=abc", &spec, &dirs, &env, false);
        let strings = args_as_strings(&args);

        assert!(strings.contains(&"--embed-metadata".to_string()));
        assert!(strings.contains(&"--no-embed-info-json".to_string()));
        assert!(strings.contains(&"--no-embed-chapters".to_string()));
        assert!(!strings.contains(&"--embed-chapters".to_string()));

        let spec_none = DownloadSpec::video(FormatChoice::Best);
        let args_none = download_args(
            "https://youtube.com/watch?v=abc",
            &spec_none,
            &dirs,
            &env,
            false,
        );
        let strings_none = args_as_strings(&args_none);

        assert!(!strings_none.contains(&"--embed-metadata".to_string()));
        assert!(!strings_none.contains(&"--no-embed-info-json".to_string()));
        assert!(!strings_none.contains(&"--no-embed-chapters".to_string()));
        assert!(!strings_none.contains(&"--embed-chapters".to_string()));
    }
}
