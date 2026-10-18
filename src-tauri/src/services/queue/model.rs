//! The queue data model (spec §7.1, §7.5, spec §3 "Item pipeline").
//!
//! Everything here is a plain, serialisable value with pure functions over it:
//! the state machine, the counters the footer shows, duplicate detection, the
//! retry policy and the history roll-off. No processes, no clock (time arrives
//! as `now` parameters), no I/O — that is what makes the queue testable without
//! spawning anything, as spec §15 requires.
//!
//! The pipeline states of spec §3 are declared in full even though M2 only runs
//! the download part of them: `Encoding`, `Transcribing` and `Summarizing` are
//! M4/M5/M6, and having them in the enum now means the persisted documents, the
//! wire values and the i18n keys are already the ones those phases need.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::ErrorInfo;
use crate::services::jsruntime::{self, JsRuntimeMode};
use crate::services::ytdlp::{
    self, CookieSource, FormatChoice, JobDirs, JobKind, JobProgress, JobStage, YtdlpErrorCode,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Spec §7.3: "keep the last 500 finished items as history, then roll off".
pub const HISTORY_LIMIT: usize = 500;

/// Spec §7.3: "up to 2 times with exponential backoff (e.g. 5 s, 20 s)".
pub const MAX_AUTO_RETRIES: u32 = 2;

/// The backoff ladder of spec §7.3. Indexed by the number of retries already
/// made, so `RETRY_BACKOFF_SECS[0]` is the wait before the first retry.
pub const RETRY_BACKOFF_SECS: [u64; 2] = [5, 20];

/// Spec §7.2: the download lane runs 2 items in parallel by default.
pub const DEFAULT_PARALLEL_DOWNLOADS: usize = 2;

/// Spec §7.2: the encode lane is 1 (CPU/GPU heavy).
pub const DEFAULT_ENCODE_LANE: usize = 1;

/// Spec §7.2: the transcribe lane is 1.
pub const DEFAULT_TRANSCRIBE_LANE: usize = 1;

/// Spec §6.4: "max 2 concurrent items per host by default", to reduce bot
/// detection and throttling.
pub const DEFAULT_PER_HOST_CAP: usize = 2;

/// How many stderr lines an item keeps for "Copy details" (spec §4.3, §11.5).
pub const LOG_TAIL_LINES: usize = 200;

// ---------------------------------------------------------------------------
// States
// ---------------------------------------------------------------------------

/// The item state machine of spec §3:
/// `Queued → Inspecting → Downloading → PostProcessing → Encoding →
/// Transcribing → Summarizing → Done | Failed | Cancelled | Paused`, plus
/// `Interrupted`, which spec §7.4 creates on restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemState {
    Queued,
    Inspecting,
    Downloading,
    PostProcessing,
    Encoding,
    Transcribing,
    Summarizing,
    Done,
    Failed,
    Cancelled,
    Paused,
    Interrupted,
}

impl ItemState {
    /// The lanes of spec §7.2. `None` for states that hold no lane: either the
    /// item is not running, or it is waiting for a decision from the user.
    ///
    /// "An item holds only the lane of its current stage" — inspecting,
    /// downloading and post-processing all run inside one yt-dlp process, so
    /// they share the download lane; encoding and transcription get their own.
    pub const fn lane(self) -> Option<Lane> {
        match self {
            Self::Inspecting | Self::Downloading | Self::PostProcessing => Some(Lane::Download),
            Self::Encoding => Some(Lane::Encode),
            Self::Transcribing | Self::Summarizing => Some(Lane::Transcribe),
            Self::Queued
            | Self::Done
            | Self::Failed
            | Self::Cancelled
            | Self::Paused
            | Self::Interrupted => None,
        }
    }

    /// Is a tool running for this item right now?
    pub const fn is_active(self) -> bool {
        self.lane().is_some()
    }

    /// Terminal states: the item will not move again on its own.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    /// Paused and Interrupted are terminal for the scheduler too: they wait for
    /// the user, and must not be picked up again by a later planning pass.
    pub const fn is_parked(self) -> bool {
        self.is_terminal() || matches!(self, Self::Paused | Self::Interrupted)
    }

    /// The stage chip the UI shows (spec §11.5). Serialised as-is, so the wire
    /// value is the state name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Inspecting => "inspecting",
            Self::Downloading => "downloading",
            Self::PostProcessing => "postProcessing",
            Self::Encoding => "encoding",
            Self::Transcribing => "transcribing",
            Self::Summarizing => "summarizing",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Paused => "paused",
            Self::Interrupted => "interrupted",
        }
    }

    /// Tolerant deserialisation.
    ///
    /// An unknown state name is not a reason to drop the item: it comes from a
    /// document written by another build, and `Interrupted` is the honest
    /// landing spot — the item was doing *something*, this app cannot tell
    /// what, and spec §7.4 shows such items with a Resume button rather than
    /// running them blindly.
    pub fn parse(raw: &str) -> Self {
        match raw {
            "queued" => Self::Queued,
            "inspecting" => Self::Inspecting,
            "downloading" => Self::Downloading,
            "postProcessing" => Self::PostProcessing,
            "encoding" => Self::Encoding,
            "transcribing" => Self::Transcribing,
            "summarizing" => Self::Summarizing,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "paused" => Self::Paused,
            _ => Self::Interrupted,
        }
    }
}

/// The three resource lanes of spec §7.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lane {
    Download,
    Encode,
    Transcribe,
}

impl Lane {
    pub const ALL: [Lane; 3] = [Lane::Download, Lane::Encode, Lane::Transcribe];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Encode => "encode",
            Self::Transcribe => "transcribe",
        }
    }
}

/// A state written as a plain string, tolerantly (see [`ItemState::parse`]).
///
/// A missing field never reaches here — `#[serde(default)]` on the struct fills
/// it — and a value that is not a string at all is treated like an unknown one,
/// because a document that odd is not a reason to throw the whole queue away.
fn de_state<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ItemState, D::Error> {
    let raw = String::deserialize(d).unwrap_or_default();
    Ok(ItemState::parse(&raw))
}

// ---------------------------------------------------------------------------
// Options snapshot
// ---------------------------------------------------------------------------

/// The quality a user picked (spec §2.3), serialisable so it can live inside
/// the persisted item.
///
/// The variants mirror [`FormatChoice`] exactly; the conversion is one function
/// so the two cannot drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QualityChoice {
    /// *Best available* — `bv*+ba/b`.
    #[default]
    Best,
    /// Exact height, the original's behaviour (spec §2.9, §8.2).
    Exact { height: u32 },
    /// Max height, for playlists and sites with unreliable heights (§8.2).
    MaxHeight { height: u32 },
    /// Audio only, Opus (§8.2).
    AudioOnly,
    /// An explicit format id from the advanced table (M3 fills the table).
    Explicit { format_id: String, has_audio: bool },
}

impl QualityChoice {
    /// A height nobody should be able to ask for; clamped by the command layer.
    pub fn clamped_height(height: u32) -> u32 {
        height.clamp(144, 4320)
    }

    pub fn to_format_choice(&self) -> FormatChoice {
        match self {
            Self::Best => FormatChoice::Best,
            Self::Exact { height } => FormatChoice::Exact {
                height: Self::clamped_height(*height),
            },
            Self::MaxHeight { height } => FormatChoice::MaxHeight {
                height: Self::clamped_height(*height),
            },
            Self::AudioOnly => FormatChoice::AudioOnly,
            Self::Explicit {
                format_id,
                has_audio,
            } => FormatChoice::Explicit {
                format_id: format_id.clone(),
                has_audio: *has_audio,
            },
        }
    }

    pub fn from_format_choice(choice: &FormatChoice) -> Self {
        match choice {
            FormatChoice::Exact { height } => Self::Exact { height: *height },
            FormatChoice::MaxHeight { height } => Self::MaxHeight { height: *height },
            FormatChoice::Best => Self::Best,
            FormatChoice::AudioOnly => Self::AudioOnly,
            FormatChoice::Explicit {
                format_id,
                has_audio,
            } => Self::Explicit {
                format_id: format_id.clone(),
                has_audio: *has_audio,
            },
        }
    }

    /// Signature used for duplicate detection (spec §7.3): the same URL with a
    /// different quality is *not* a duplicate.
    pub fn signature(&self) -> String {
        match self {
            Self::Best => "best".to_owned(),
            Self::Exact { height } => format!("exact:{height}"),
            Self::MaxHeight { height } => format!("max:{height}"),
            Self::AudioOnly => "audio".to_owned(),
            Self::Explicit {
                format_id,
                has_audio,
            } => format!("id:{format_id}:{has_audio}"),
        }
    }
}

/// Where session cookies come from, as a snapshot value (spec §8.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CookieChoice {
    #[default]
    None,
    /// `--cookies-from-browser <name>`
    Browser(String),
    /// `--cookies <file>`
    File(PathBuf),
}

impl CookieChoice {
    /// Build the snapshot from the global settings, filtering a browser this OS
    /// does not have (spec §8.6).
    pub fn from_settings(browser: Option<&str>, file: Option<&PathBuf>) -> Self {
        cookie_source(browser, file).into()
    }

    pub fn to_source(&self) -> CookieSource {
        match self {
            Self::None => CookieSource::None,
            Self::Browser(name) => CookieSource::Browser(name.clone()),
            Self::File(path) => CookieSource::File(path.clone()),
        }
    }
}

impl From<CookieSource> for CookieChoice {
    fn from(source: CookieSource) -> Self {
        match source {
            CookieSource::None => Self::None,
            CookieSource::Browser(name) => Self::Browser(name),
            CookieSource::File(path) => Self::File(path),
        }
    }
}

fn cookie_source(browser: Option<&str>, file: Option<&PathBuf>) -> CookieSource {
    let os = ytdlp::CfgOs::current();
    let file_path = file.map(std::path::PathBuf::as_path);
    ytdlp::cookie_source(browser, file_path, os)
}

/// Spec §7.1: the immutable snapshot of the options an item was added with, so
/// changing a global setting later never alters queued work.
///
/// `#[serde(default)]` on every field: a document written by an earlier build
/// must load, and a missing field must mean "the default", not "a parse error".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ItemOptions {
    pub kind: Kind,
    pub quality: QualityChoice,
    /// Where the finished file goes. Absolute (spec §4.8).
    pub output_dir: PathBuf,
    /// The cookies in force when the item was added (spec §8.6).
    pub cookies: CookieChoice,
    /// The JS runtime mode in force when the item was added (spec §5.3.3).
    pub runtime_mode: JsRuntimeMode,
    /// Optional literal prefix before `%(title).180B` in `-o`, e.g. `"01 - "`
    /// for numbered playlist entries (spec §6.4, §8.2).
    pub filename_prefix: Option<String>,
    /// Include `[%(id)s]` suffix in the filename (spec §8.2, §11.8).
    pub include_id_in_filename: bool,
    /// Pass `-S vcodec:h264,res,acodec:m4a` (spec §8.2, §11.8).
    pub prefer_h264: bool,
    /// Optional `--download-archive <path>` (spec §6.2.4, §8.2).
    pub archive_file: Option<PathBuf>,
    /// Optional `(min, max)` sleep interval in seconds for playlists > 20 items
    /// (spec §6.4, §8.2).
    pub sleep_interval: Option<(u32, u32)>,
    /// Live stream recording mode (`Some(true)` = `--live-from-start`,
    /// `Some(false)` = `--no-live-from-start`, `None` = normal VOD, spec §6.4).
    pub live_from_start: Option<bool>,
    /// `--proxy <url>` (spec §8.2, §11.8).
    pub proxy: Option<String>,
    /// `--limit-rate <rate>` (spec §8.2, §11.8).
    pub limit_rate: Option<String>,
    /// `--referer <url>` (spec §6.3.4, §8.2).
    pub referer: Option<String>,
    /// `--user-agent <ua>` (spec §6.3.4, §8.2).
    pub user_agent: Option<String>,
    /// Extra `Header: Value` lines passed via `--add-headers` (spec §6.3.4, §8.2).
    pub custom_headers: Vec<String>,
    /// Allow one retry with `--impersonate chrome` on `ANTIBOT` (spec §6.3.5, §8.5).
    pub impersonate_retry: bool,
    /// Encode with ffmpeg after download (spec §9.2).
    pub encode: bool,
    /// Video encoder identifier (e.g. `libx264`, `h264_nvenc`, spec §9.3).
    pub video_codec: Option<String>,
    /// Audio codec identifier (e.g. `aac`, `mp3`, `opus`, spec §9.4).
    pub audio_codec: Option<String>,
    /// Trim start timestamp string (e.g. `"00:01:30"`, spec §9.2).
    pub trim_start: Option<String>,
    /// Trim end timestamp string (spec §9.2).
    pub trim_end: Option<String>,
    /// Keep the original downloaded file after successful encode (default `false`, spec §9.2).
    pub keep_original: bool,
    /// Automatically open media with the system player upon completion (spec §9.5).
    pub auto_play: bool,
    /// `--embed-metadata` (spec §8.4).
    pub embed_metadata: bool,
    /// `--embed-thumbnail` with `--convert-thumbnails jpg` (spec §8.4).
    pub embed_thumbnail: bool,
    /// `--embed-chapters` (spec §8.4).
    pub embed_chapters: bool,
    /// Optional SponsorBlock mode (`mark` or `remove`, spec §8.4).
    pub sponsorblock_mode: Option<ytdlp::SponsorBlockMode>,
    /// User-selectable SponsorBlock categories (spec §8.4).
    pub sponsorblock_categories: Vec<String>,
    /// Transcribe with Whisper after download/encode (spec §10.2).
    pub transcribe: bool,
    /// Model identifier from whisper catalogue (e.g. "large-v3-turbo-q5_0").
    pub whisper_model: Option<String>,
    /// Language code (e.g. "it", "en", "auto").
    pub transcript_language: Option<String>,
    /// Translate to English during transcription (-tr flag).
    pub translate_to_english: Option<bool>,
    /// Place transcripts in a `transcripts/` subfolder.
    pub transcripts_subfolder: Option<bool>,
}

impl Default for ItemOptions {
    fn default() -> Self {
        Self {
            kind: Kind::Video,
            quality: QualityChoice::Best,
            output_dir: PathBuf::new(),
            cookies: CookieChoice::None,
            runtime_mode: JsRuntimeMode::default(),
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
            impersonate_retry: true,
            encode: false,
            video_codec: None,
            audio_codec: None,
            trim_start: None,
            trim_end: None,
            keep_original: false,
            auto_play: false,
            embed_metadata: false,
            embed_thumbnail: false,
            embed_chapters: false,
            sponsorblock_mode: None,
            sponsorblock_categories: Vec::new(),
            transcribe: false,
            whisper_model: None,
            transcript_language: None,
            translate_to_english: None,
            transcripts_subfolder: None,
        }
    }
}

impl ItemOptions {
    /// Does this item need an encode/trim stage after download? (spec §9.2)
    ///
    /// "Trim implies Encode (coupling kept from the original)"
    pub fn needs_encode(&self) -> bool {
        self.encode
            || self
                .trim_start
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            || self
                .trim_end
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
    }

    /// The `DownloadSpec` the job runner needs (spec §8.2, §8.4).
    pub fn to_download_spec(&self) -> ytdlp::DownloadSpec {
        ytdlp::DownloadSpec {
            kind: self.kind.to_job_kind(),
            quality: self.quality.to_format_choice(),
            cookies: self.cookies.to_source(),
            filename_prefix: self.filename_prefix.clone(),
            include_id_in_filename: self.include_id_in_filename,
            prefer_h264: self.prefer_h264,
            archive_file: self.archive_file.clone(),
            sleep_interval: self.sleep_interval,
            live_from_start: self.live_from_start,
            proxy: self.proxy.clone(),
            limit_rate: self.limit_rate.clone(),
            referer: self.referer.clone(),
            user_agent: self.user_agent.clone(),
            custom_headers: self.custom_headers.clone(),
            impersonate_chrome: false,
            impersonate_retry: self.impersonate_retry,
            embed_metadata: self.embed_metadata,
            embed_thumbnail: self.embed_thumbnail,
            embed_chapters: self.embed_chapters,
            sponsorblock_mode: self.sponsorblock_mode,
            sponsorblock_categories: self.sponsorblock_categories.clone(),
        }
    }

    /// Signature used for duplicate detection (spec §7.3).
    pub fn signature(&self) -> String {
        let cookies = match &self.cookies {
            CookieChoice::None => "none".to_owned(),
            CookieChoice::Browser(name) => format!("browser:{name}"),
            CookieChoice::File(path) => format!("file:{}", path.display()),
        };
        let sb_mode = match self.sponsorblock_mode {
            None => "none",
            Some(ytdlp::SponsorBlockMode::Mark) => "mark",
            Some(ytdlp::SponsorBlockMode::Remove) => "remove",
        };
        let mut sb_categories = self.sponsorblock_categories.clone();
        sb_categories.sort();
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.kind.as_str(),
            self.quality.signature(),
            self.output_dir.display(),
            cookies,
            self.encode,
            self.video_codec.as_deref().unwrap_or(""),
            self.audio_codec.as_deref().unwrap_or(""),
            self.trim_start.as_deref().unwrap_or(""),
            self.trim_end.as_deref().unwrap_or(""),
            self.keep_original,
            self.embed_metadata,
            self.embed_thumbnail,
            self.embed_chapters,
            sb_mode,
            sb_categories.join(","),
            self.transcribe,
            self.whisper_model.as_deref().unwrap_or(""),
            self.transcript_language.as_deref().unwrap_or(""),
            self.translate_to_english.unwrap_or(false),
            self.transcripts_subfolder.unwrap_or(false),
        )
    }
}

/// Video or audio only (spec §2.3), with the serialisable shape the wire uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    #[default]
    Video,
    Audio,
}

impl Kind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }

    pub const fn to_job_kind(self) -> JobKind {
        match self {
            Self::Video => JobKind::Video,
            Self::Audio => JobKind::Audio,
        }
    }

    /// An audio-only item does not merge anything, so it does not need
    /// `--merge-output-format` (spec §8.2).
    pub const fn needs_merge(self) -> bool {
        matches!(self, Self::Video)
    }
}

// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

/// The progress payload of spec §3, attached to the item instead of to an
/// event of its own: `{stage, percent|null, speedBps|null, etaSec|null, label,
/// subIndex?, subTotal?}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ItemProgress {
    /// 0–100, or `None` for an indeterminate bar (spec §8.3).
    pub percent: Option<f64>,
    pub speed_bps: Option<f64>,
    pub eta_sec: Option<u64>,
    /// Stable token, never a sentence (spec §11.9).
    pub label: String,
    pub sub_index: Option<u32>,
    pub sub_total: Option<u32>,
}

impl ItemProgress {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn with_label(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    /// Fold one `JobProgress` of spec §3 into the item.
    pub fn apply(&mut self, progress: &JobProgress) {
        self.percent = progress.percent;
        self.speed_bps = progress.speed_bps;
        self.eta_sec = progress.eta_sec;
        self.label = progress.label.clone();
        self.sub_index = progress.sub_index;
        self.sub_total = progress.sub_total;
    }

    /// The state a stage name maps to, so the runner and the store cannot
    /// disagree about what "postProcessing" means.
    pub const fn state_of(stage: JobStage) -> ItemState {
        match stage {
            JobStage::Preparing => ItemState::Queued,
            JobStage::Downloading => ItemState::Downloading,
            JobStage::PostProcessing => ItemState::PostProcessing,
            JobStage::Encoding => ItemState::Encoding,
            JobStage::Done => ItemState::Done,
            JobStage::Failed => ItemState::Failed,
            JobStage::Cancelled => ItemState::Cancelled,
        }
    }
}

// ---------------------------------------------------------------------------
// Item
// ---------------------------------------------------------------------------

/// One queue item (spec §7.1).
///
/// The order of the fields is the order of the story: what was asked for, what
/// the options were frozen to, where it is, what it produced, and what to show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QueueItem {
    pub id: String,
    pub url: String,
    pub title: Option<String>,
    /// The site badge (spec §6.3.2): the extractor's key, e.g. `Youtube`.
    pub site: Option<String>,
    pub duration_sec: Option<u64>,
    #[serde(deserialize_with = "de_state")]
    pub state: ItemState,
    pub options: ItemOptions,
    /// Group of spec §7.1 (a playlist, in M3).
    pub group_id: Option<String>,
    pub progress: ItemProgress,
    /// The real output path, from `SDP_FILE` (spec §8.3). Never guessed.
    pub file: Option<PathBuf>,
    /// The size of that file, read from it once the download is done
    /// (spec §11.5). `None` while it runs, and for anything that produced
    /// nothing.
    pub size_bytes: Option<u64>,
    /// `quickjs` | `deno` (spec §15 item 3).
    pub runtime: Option<String>,
    pub requested_height: Option<u32>,
    pub actual_height: Option<u32>,
    pub error: Option<ErrorInfo>,
    /// Last stderr lines, one click away (spec §8.5).
    pub log_tail: Vec<String>,
    /// Automatic retries already spent (spec §7.3).
    pub auto_retries: u32,
    /// Unix seconds; the scheduler leaves the item alone until then.
    pub retry_at: Option<u64>,
    /// The stage to resume, kept so a paused item knows where it was.
    pub paused_from: Option<ItemState>,
    /// Set while a pause/cancel/remove is in flight: the task that owns the
    /// token reads it when the process ends and records the right state
    /// instead of a plain failure.
    pub control: ControlSignal,
    pub added_at: u64,
    pub updated_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    /// Transcript output files (.txt, .srt, .vtt) (spec §10.2).
    pub transcript_files: Vec<PathBuf>,
    /// Live segment preview or last preview text (spec §11.5).
    pub transcript_preview: Option<String>,
}

impl Default for QueueItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            url: String::new(),
            title: None,
            site: None,
            duration_sec: None,
            state: ItemState::Queued,
            options: ItemOptions::default(),
            group_id: None,
            progress: ItemProgress::empty(),
            file: None,
            size_bytes: None,
            runtime: None,
            requested_height: None,
            actual_height: None,
            error: None,
            log_tail: Vec::new(),
            auto_retries: 0,
            retry_at: None,
            paused_from: None,
            control: ControlSignal::None,
            added_at: 0,
            updated_at: 0,
            started_at: None,
            finished_at: None,
            transcript_files: Vec::new(),
            transcript_preview: None,
        }
    }
}

/// What the user asked for while a stage was running.
///
/// It lives on the item because the decision and the outcome arrive in
/// different places: the command sets the signal and cancels the token, the
/// running task sees the signal when the process ends. A paused item must not
/// come out as "cancelled", and a cancelled one must not come out as "paused",
/// which is the whole reason this exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ControlSignal {
    #[default]
    None,
    Pause,
    Cancel,
    /// Cancel plus "and take the item out of the queue" (spec §7.3 remove).
    Remove,
}

impl QueueItem {
    /// Host of the URL, for the per-host cap of spec §6.4. Lowercased, without
    /// `www.`, so `www.youtube.com` and `youtube.com` share the budget.
    pub fn host(&self) -> String {
        host_key(&self.url)
    }

    /// The full item plus what the item owns on disk.
    pub fn temp_dir(&self) -> PathBuf {
        crate::services::process::item_temp_dir(&self.options.output_dir, &self.id)
    }

    /// Is a retry sensible right now, ignoring the timer? (spec §7.3)
    pub fn can_retry(&self) -> bool {
        matches!(
            self.state,
            ItemState::Failed | ItemState::Cancelled | ItemState::Paused | ItemState::Interrupted
        )
    }

    /// Same URL **and** same options, which is what makes two items duplicates
    /// (spec §7.3: "the same normalized URL + same options already queued").
    pub fn duplicate_key(&self) -> String {
        format!("{}|{}", normalised_url(&self.url), self.options.signature())
    }

    /// Normalised URL for the "already queued?" question. A finished item does
    /// not count: re-downloading a file the user has is a legitimate choice.
    pub fn counts_as_duplicate(&self) -> bool {
        !self.state.is_terminal()
    }

    pub fn is_running(&self) -> bool {
        self.state.is_active()
    }

    /// `true` when the automatic retry policy applies to this failure.
    pub fn auto_retryable(&self) -> bool {
        self.error
            .as_ref()
            .is_some_and(|error| auto_retryable(YtdlpErrorCode::parse(&error.code)))
    }

    /// How long to wait before the next automatic retry, `None` when the policy
    /// says stop (spec §7.3: 2 attempts, 5 s then 20 s).
    pub fn backoff_for_next_retry(&self) -> Option<Duration> {
        backoff(self.auto_retries)
    }

    /// Append one stderr line to the tail, keeping the last
    /// [`LOG_TAIL_LINES`] (spec §4.3).
    pub fn push_log(&mut self, line: &str) {
        self.log_tail.push(line.to_owned());
        if self.log_tail.len() > LOG_TAIL_LINES {
            let excess = self.log_tail.len() - LOG_TAIL_LINES;
            self.log_tail.drain(..excess);
        }
    }
}

/// The host key used by the per-host cap.
pub fn host_key(url: &str) -> String {
    jsruntime::url_host(url)
        .map(|host| host.trim_start_matches("www.").to_owned())
        .unwrap_or_default()
}

/// Spec §7.3 duplicate detection.
///
/// Normalises what does not change which media a URL points at and drops what
/// has no bearing on it at all:
///
/// * scheme and host lowercased, `www.` dropped;
/// * the fragment dropped (`#t=30` is the same video);
/// * an empty `?` dropped, so `…?` and `…` match;
/// * **the path and the query are kept byte for byte.** Stripping a trailing
///   slash or reordering query parameters would be a guess about what the site
///   considers the same resource, and two "duplicates" that are really two
///   videos (or one video and one 404) are worse than an extra question.
pub fn normalised_url(url: &str) -> String {
    let trimmed = url.trim();
    let lowered = trimmed.to_ascii_lowercase();
    let Some(scheme_end) = lowered.find("://") else {
        return lowered.split('#').next().unwrap_or_default().to_owned();
    };
    let scheme = &lowered[..scheme_end];
    let rest = &lowered[scheme_end + 3..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let host = match authority.rsplit_once('@') {
        Some((_, host)) => host,
        None => authority,
    };
    let host = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    };
    let host = host.trim_start_matches("www.");
    let remainder = &rest[authority_end..];
    let path = remainder.split(['?', '#']).next().unwrap_or_default();
    let query = match remainder.split_once('?') {
        Some((_, after)) => after.split('#').next().unwrap_or_default(),
        None => "",
    };
    if query.is_empty() {
        format!("{scheme}://{host}{path}")
    } else {
        format!("{scheme}://{host}{path}?{query}")
    }
}

// ---------------------------------------------------------------------------
// Retry policy (spec §7.3)
// ---------------------------------------------------------------------------

/// The only class the queue retries on its own.
///
/// Spec §7.3 lists "transient classes (`NETWORK`, HTTP 5xx, timeouts)" and
/// says permanent ones (private, unavailable, DRM, unsupported, disk full) are
/// never retried. The classifier of spec §8.5 already folds "timed out" and
/// "HTTP Error 5xx" into `NETWORK` (verified in `services/ytdlp.rs`, whose
/// tests assert exactly those two lines), so one class covers the whole family.
///
/// `UNKNOWN` is deliberately **not** on the list: it is the catch-all for
/// anything the classifier did not recognise, and retrying a real bug twice with
/// a 20 s wait hides it instead of showing it.
pub const fn auto_retryable(code: YtdlpErrorCode) -> bool {
    matches!(code, YtdlpErrorCode::Network)
}

/// The backoff for the retry number `attempt` (0 = the first retry).
pub fn backoff(attempt: u32) -> Option<Duration> {
    RETRY_BACKOFF_SECS
        .get(attempt as usize)
        .map(|secs| Duration::from_secs(*secs))
}

/// Should this failure start the automatic retry loop, given the retries
/// already spent? (spec §7.3)
pub const fn should_auto_retry(code: YtdlpErrorCode, already_retried: u32) -> bool {
    already_retried < MAX_AUTO_RETRIES && auto_retryable(code)
}

// ---------------------------------------------------------------------------
// Groups, summary, aggregation (spec §7.1, §7.5)
// ---------------------------------------------------------------------------

/// A group of items: a playlist, from M3 (spec §7.1). M2 creates none but the
/// model, the progress computation and the UI grouping are already here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QueueGroup {
    pub id: String,
    pub title: String,
    /// The folder the group's items go to, sanitised (spec §6.4).
    pub folder: Option<PathBuf>,
    /// Position in the queue list.
    pub index: usize,
}

/// "7/42" for a group header (spec §7.5, §11.5).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupProgress {
    pub group_id: String,
    pub done: u32,
    pub total: u32,
    /// 0–100, or `None` while nothing is done and the total is unknown.
    pub percent: Option<f64>,
}

impl GroupProgress {
    pub fn of(items: &[QueueItem], group_id: &str) -> Self {
        let mut done = 0;
        let mut total = 0;
        for item in items
            .iter()
            .filter(|item| item.group_id.as_deref() == Some(group_id))
        {
            total += 1;
            if item.state == ItemState::Done {
                done += 1;
            }
        }
        Self {
            group_id: group_id.to_owned(),
            done,
            total,
            percent: (total > 0).then(|| done as f64 * 100.0 / total as f64),
        }
    }
}

/// What the footer shows (spec §7.5): "N running · M queued · speed".
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSummary {
    pub total: u32,
    pub queued: u32,
    pub running: u32,
    pub paused: u32,
    pub interrupted: u32,
    pub done: u32,
    pub failed: u32,
    pub cancelled: u32,
    /// Sum of the running items' speeds; `None` when nothing runs.
    pub speed_bps: Option<f64>,
    /// "Pause all" is on (spec §7.3).
    pub paused_all: bool,
    /// Parallel downloads in effect (spec §7.2), for the footer hint.
    pub parallel_downloads: usize,
    pub per_host_cap: usize,
}

impl QueueSummary {
    /// `true` while something can still happen on its own.
    pub fn is_active(&self) -> bool {
        self.running > 0 || self.queued > 0
    }

    /// The queue just went idle: a notification candidate (spec §7.7). It must
    /// be a *transition*, which is why the caller remembers the previous value
    /// instead of asking this question every time.
    pub fn became_idle(&self, previous: &QueueSummary) -> bool {
        previous.is_active() && !self.is_active()
    }
}

/// Count the items. Pure, so the footer numbers are testable without a queue.
pub fn aggregate(items: &[QueueItem], paused_all: bool, lanes: LaneConfig) -> QueueSummary {
    let mut summary = QueueSummary {
        paused_all,
        parallel_downloads: lanes.download,
        per_host_cap: lanes.per_host_cap,
        ..QueueSummary::default()
    };
    let mut speed = 0.0;
    let mut any_speed = false;
    for item in items {
        summary.total += 1;
        match item.state {
            ItemState::Done => summary.done += 1,
            ItemState::Failed => summary.failed += 1,
            ItemState::Cancelled => summary.cancelled += 1,
            ItemState::Paused => summary.paused += 1,
            ItemState::Interrupted => summary.interrupted += 1,
            ItemState::Queued => summary.queued += 1,
            ItemState::Inspecting
            | ItemState::Downloading
            | ItemState::PostProcessing
            | ItemState::Encoding
            | ItemState::Transcribing
            | ItemState::Summarizing => {
                summary.running += 1;
                if let Some(value) = item.progress.speed_bps
                    && value.is_finite()
                    && value > 0.0
                {
                    speed += value;
                    any_speed = true;
                }
            }
        }
    }
    summary.speed_bps = any_speed.then_some(speed);
    summary
}

/// The lane capacities in force (spec §7.2, §11.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneConfig {
    /// 1–4, default 2.
    pub download: usize,
    /// 1, CPU/GPU heavy.
    pub encode: usize,
    /// 1.
    pub transcribe: usize,
    /// Max concurrent items per host (spec §6.4), default 2.
    pub per_host_cap: usize,
}

impl Default for LaneConfig {
    fn default() -> Self {
        Self {
            download: DEFAULT_PARALLEL_DOWNLOADS,
            encode: DEFAULT_ENCODE_LANE,
            transcribe: DEFAULT_TRANSCRIBE_LANE,
            per_host_cap: DEFAULT_PER_HOST_CAP,
        }
    }
}

impl LaneConfig {
    /// Clamp whatever the settings file or a command produced. A zero would
    /// deadlock the queue (nothing can ever run) and a huge number would defeat
    /// the lane the spec exists for.
    pub fn clamped(mut self) -> Self {
        self.download = self.download.clamp(1, 4);
        self.encode = self.encode.clamp(1, 4);
        self.transcribe = self.transcribe.clamp(1, 4);
        self.per_host_cap = self.per_host_cap.clamp(1, 8);
        self
    }

    pub fn capacity(self, lane: Lane) -> usize {
        match lane {
            Lane::Download => self.download,
            Lane::Encode => self.encode,
            Lane::Transcribe => self.transcribe,
        }
    }
}

// ---------------------------------------------------------------------------
// History (spec §7.3)
// ---------------------------------------------------------------------------

/// Drop the oldest finished items once there are more than [`HISTORY_LIMIT`].
///
/// Returns the ids that were dropped, so the caller can tell the frontend to
/// forget them. The newest are kept, and nothing that can still run is ever
/// touched: a queued, running, paused or interrupted item is not history.
pub fn roll_off_history(items: &mut Vec<QueueItem>) -> Vec<String> {
    let finished: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.state.is_terminal())
        .map(|(index, _)| index)
        .collect();
    if finished.len() <= HISTORY_LIMIT {
        return Vec::new();
    }
    // `finished` is in queue order, and items are appended at the end, so the
    // first ones are the oldest.
    let excess = finished.len() - HISTORY_LIMIT;
    let drop: BTreeSet<usize> = finished.into_iter().take(excess).collect();
    let mut removed = Vec::with_capacity(excess);
    let mut index = 0;
    items.retain(|item| {
        let keep = !drop.contains(&index);
        if !keep {
            removed.push(item.id.clone());
        }
        index += 1;
        keep
    });
    removed
}

// ---------------------------------------------------------------------------
// Helpers shared with the engine
// ---------------------------------------------------------------------------

/// The `JobDirs` of an item: `home` is the output folder, `temp` is the
/// per-item private directory on the same filesystem (spec §4.9).
pub fn job_dirs(item: &QueueItem) -> JobDirs {
    JobDirs {
        home: item.options.output_dir.clone(),
        temp: item.temp_dir(),
    }
}

/// The queued state an item shows while its automatic retry is pending.
pub fn retrying_state(item: &QueueItem) -> ItemState {
    if item.retry_at.is_some() {
        ItemState::Queued
    } else {
        item.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, url: &str, state: ItemState) -> QueueItem {
        QueueItem {
            id: id.to_owned(),
            url: url.to_owned(),
            state,
            ..QueueItem::default()
        }
    }

    // -- states ------------------------------------------------------------

    #[test]
    fn every_active_state_holds_exactly_one_lane() {
        for state in [
            ItemState::Inspecting,
            ItemState::Downloading,
            ItemState::PostProcessing,
            ItemState::Encoding,
            ItemState::Transcribing,
            ItemState::Summarizing,
        ] {
            assert!(state.is_active(), "{state:?} must hold a lane");
            assert!(state.lane().is_some(), "{state:?} must name its lane");
        }
        for state in [
            ItemState::Queued,
            ItemState::Done,
            ItemState::Failed,
            ItemState::Cancelled,
            ItemState::Paused,
            ItemState::Interrupted,
        ] {
            assert_eq!(state.lane(), None, "{state:?} must hold no lane");
            assert!(!state.is_active());
        }
    }

    #[test]
    fn terminal_and_parked_are_not_the_same_set() {
        assert!(ItemState::Done.is_terminal());
        assert!(ItemState::Failed.is_terminal());
        assert!(ItemState::Cancelled.is_terminal());
        assert!(!ItemState::Paused.is_terminal(), "paused can resume");
        assert!(
            !ItemState::Interrupted.is_terminal(),
            "interrupted can resume"
        );
        assert!(ItemState::Paused.is_parked());
        assert!(ItemState::Interrupted.is_parked());
        assert!(!ItemState::Queued.is_parked());
    }

    #[test]
    fn the_wire_name_of_a_state_is_camel_case_and_stable() {
        assert_eq!(ItemState::PostProcessing.as_str(), "postProcessing");
        assert_eq!(ItemState::Done.as_str(), "done");
        assert_eq!(ItemState::Interrupted.as_str(), "interrupted");
    }

    #[test]
    fn an_unknown_state_name_lands_on_interrupted_not_on_done() {
        assert_eq!(ItemState::parse("downloading"), ItemState::Downloading);
        assert_eq!(ItemState::parse("nonsense"), ItemState::Interrupted);
        assert_eq!(ItemState::parse("Done"), ItemState::Interrupted);
    }

    #[test]
    fn a_document_with_a_broken_state_still_loads_every_other_field() {
        let raw = r#"{
            "id": "q1",
            "url": "https://example.com/a",
            "state": "written-by-a-newer-build",
            "title": "kept"
        }"#;
        let item: QueueItem = serde_json::from_str(raw).expect("parses");
        assert_eq!(item.state, ItemState::Interrupted);
        assert_eq!(item.title.as_deref(), Some("kept"));
    }

    // -- options snapshot --------------------------------------------------

    #[test]
    fn a_snapshot_without_fields_gets_the_documented_defaults() {
        let options: ItemOptions = serde_json::from_str("{}").expect("parses");
        assert_eq!(options, ItemOptions::default());
        assert_eq!(options.kind, Kind::Video);
        assert_eq!(options.quality, QualityChoice::Best);
        assert_eq!(options.cookies, CookieChoice::None);
    }

    #[test]
    fn quality_survives_a_round_trip_and_builds_the_same_selector() {
        for choice in [
            QualityChoice::Best,
            QualityChoice::Exact { height: 1080 },
            QualityChoice::MaxHeight { height: 720 },
            QualityChoice::AudioOnly,
            QualityChoice::Explicit {
                format_id: "137".into(),
                has_audio: false,
            },
        ] {
            let json = serde_json::to_string(&choice).expect("serialise");
            let back: QualityChoice = serde_json::from_str(&json).expect("parse");
            assert_eq!(back, choice);
            assert_eq!(
                back.to_format_choice().selector(),
                choice.to_format_choice().selector()
            );
        }
        assert_eq!(
            QualityChoice::Exact { height: 1080 }
                .to_format_choice()
                .selector()
                .unwrap(),
            "bv[height=1080]+ba/bv[height<=1080]+ba/b[height<=1080]/b"
        );
    }

    #[test]
    fn a_nonsense_height_is_clamped_instead_of_building_a_useless_selector() {
        assert_eq!(QualityChoice::clamped_height(0), 144);
        assert_eq!(QualityChoice::clamped_height(100_000), 4320);
    }

    #[test]
    fn the_options_signature_separates_quality_cookies_and_folder() {
        let base = ItemOptions::default();
        let other_quality = ItemOptions {
            quality: QualityChoice::Exact { height: 720 },
            ..base.clone()
        };
        let other_folder = ItemOptions {
            output_dir: PathBuf::from("/tmp/elsewhere"),
            ..base.clone()
        };
        let other_cookies = ItemOptions {
            cookies: CookieChoice::Browser("firefox".into()),
            ..base.clone()
        };
        assert_ne!(base.signature(), other_quality.signature());
        assert_ne!(base.signature(), other_folder.signature());
        assert_ne!(base.signature(), other_cookies.signature());
        assert_eq!(base.signature(), ItemOptions::default().signature());
    }

    #[test]
    fn the_options_signature_separates_m4_options() {
        let base = ItemOptions::default();
        let other_encode = ItemOptions {
            encode: true,
            ..base.clone()
        };
        let other_vcodec = ItemOptions {
            video_codec: Some("libx264".into()),
            ..base.clone()
        };
        let other_acodec = ItemOptions {
            audio_codec: Some("opus".into()),
            ..base.clone()
        };
        let other_trim_start = ItemOptions {
            trim_start: Some("00:10".into()),
            ..base.clone()
        };
        let other_trim_end = ItemOptions {
            trim_end: Some("00:20".into()),
            ..base.clone()
        };
        let other_keep_original = ItemOptions {
            keep_original: true,
            ..base.clone()
        };
        let other_embed_metadata = ItemOptions {
            embed_metadata: true,
            ..base.clone()
        };
        let other_embed_thumbnail = ItemOptions {
            embed_thumbnail: true,
            ..base.clone()
        };
        let other_embed_chapters = ItemOptions {
            embed_chapters: true,
            ..base.clone()
        };
        let other_sb_mode = ItemOptions {
            sponsorblock_mode: Some(ytdlp::SponsorBlockMode::Mark),
            ..base.clone()
        };
        let other_sb_cats = ItemOptions {
            sponsorblock_categories: vec!["sponsor".into()],
            ..base.clone()
        };

        assert_ne!(base.signature(), other_encode.signature());
        assert_ne!(base.signature(), other_vcodec.signature());
        assert_ne!(base.signature(), other_acodec.signature());
        assert_ne!(base.signature(), other_trim_start.signature());
        assert_ne!(base.signature(), other_trim_end.signature());
        assert_ne!(base.signature(), other_keep_original.signature());
        assert_ne!(base.signature(), other_embed_metadata.signature());
        assert_ne!(base.signature(), other_embed_thumbnail.signature());
        assert_ne!(base.signature(), other_embed_chapters.signature());
        assert_ne!(base.signature(), other_sb_mode.signature());
        assert_ne!(base.signature(), other_sb_cats.signature());

        let clip_a = ItemOptions {
            trim_start: Some("00:00".into()),
            trim_end: Some("00:10".into()),
            ..base.clone()
        };
        let clip_b = ItemOptions {
            trim_start: Some("00:20".into()),
            trim_end: Some("00:30".into()),
            ..base
        };
        assert_ne!(clip_a.signature(), clip_b.signature());
    }

    #[test]
    fn kind_decides_whether_a_merge_is_expected() {
        assert!(Kind::Video.needs_merge());
        assert!(!Kind::Audio.needs_merge());
        let audio = ItemOptions {
            kind: Kind::Audio,
            quality: QualityChoice::AudioOnly,
            ..ItemOptions::default()
        };
        assert!(!audio.to_download_spec().needs_merge());
    }

    // -- duplicates --------------------------------------------------------

    #[test]
    fn url_normalisation_ignores_case_www_and_the_fragment() {
        let canonical = normalised_url("https://www.youtube.com/watch?v=abc123");
        assert_eq!(
            normalised_url("HTTPS://YouTube.com/watch?v=abc123"),
            canonical
        );
        assert_eq!(
            normalised_url("https://www.youtube.com/watch?v=abc123#t=30"),
            canonical
        );
        assert_eq!(
            normalised_url("https://youtube.com:443/watch?v=abc123"),
            canonical,
            "the default port does not change the resource"
        );
        assert_eq!(
            normalised_url("  https://youtube.com/watch?v=abc123  "),
            canonical
        );
    }

    #[test]
    fn url_normalisation_keeps_what_makes_a_resource_different() {
        let a = normalised_url("https://example.com/watch/1");
        assert_ne!(a, normalised_url("https://example.com/watch/2"));
        assert_ne!(a, normalised_url("https://example.com/watch/1/"));
        assert_ne!(
            normalised_url("https://example.com/w?v=1"),
            normalised_url("https://example.com/w?v=2")
        );
    }

    #[test]
    fn userinfo_cannot_smuggle_a_host() {
        // The hostile shape `youtube.com@evil.example`: the host is what comes
        // after the `@`, exactly as `services::jsruntime::url_host` decides, so
        // it normalises to the **evil** host and can never share a duplicate key
        // (or a per-host budget) with the YouTube one.
        assert_eq!(
            normalised_url("https://youtube.com@evil.example/a"),
            normalised_url("https://evil.example/a")
        );
        assert_ne!(
            normalised_url("https://youtube.com@evil.example/a"),
            normalised_url("https://youtube.com/a")
        );
    }

    #[test]
    fn duplicates_need_the_same_url_and_the_same_options() {
        let first = item("q1", "https://www.youtube.com/watch?v=x", ItemState::Queued);
        let mut same_options = item("q2", "https://youtube.com/watch?v=x", ItemState::Queued);
        assert_eq!(first.duplicate_key(), same_options.duplicate_key());
        same_options.options.quality = QualityChoice::AudioOnly;
        assert_ne!(first.duplicate_key(), same_options.duplicate_key());
    }

    #[test]
    fn a_finished_item_is_never_a_duplicate_of_a_new_one() {
        let done = item("q1", "https://example.com/a", ItemState::Done);
        let running = item("q2", "https://example.com/a", ItemState::Downloading);
        assert_eq!(done.duplicate_key(), running.duplicate_key());
        assert!(!done.counts_as_duplicate(), "re-downloading is a choice");
        assert!(running.counts_as_duplicate());
        assert!(
            !item("q3", "https://example.com/a", ItemState::Failed).counts_as_duplicate(),
            "a failed item is not queued: the answer to that is Retry, not a duplicate question"
        );
        assert!(!item("q4", "https://example.com/a", ItemState::Paused).is_running());
    }

    // -- retry policy ------------------------------------------------------

    #[test]
    fn only_transient_classes_are_retried_automatically() {
        assert!(
            auto_retryable(YtdlpErrorCode::Network),
            "NETWORK covers timeouts and 5xx, which the classifier folds into it"
        );
        for code in [
            YtdlpErrorCode::Auth,
            YtdlpErrorCode::Unavailable,
            YtdlpErrorCode::Geo,
            YtdlpErrorCode::Format,
            YtdlpErrorCode::Unsupported,
            YtdlpErrorCode::Drm,
            YtdlpErrorCode::Antibot,
            YtdlpErrorCode::Cookies,
            YtdlpErrorCode::DiskFull,
            YtdlpErrorCode::Extract,
            YtdlpErrorCode::FfmpegMissing,
            YtdlpErrorCode::YtChallenge,
            YtdlpErrorCode::DenoMissing,
            YtdlpErrorCode::WhisperMissing,
            YtdlpErrorCode::WhisperModelMissing,
            YtdlpErrorCode::Unknown,
        ] {
            assert!(
                !auto_retryable(code),
                "{code:?} must not be retried silently"
            );
        }
    }

    #[test]
    fn the_backoff_is_five_then_twenty_then_nothing() {
        assert_eq!(backoff(0), Some(Duration::from_secs(5)));
        assert_eq!(backoff(1), Some(Duration::from_secs(20)));
        assert_eq!(backoff(2), None, "two automatic retries, then the user");
    }

    #[test]
    fn the_retry_budget_is_two_and_never_more() {
        assert!(should_auto_retry(YtdlpErrorCode::Network, 0));
        assert!(should_auto_retry(YtdlpErrorCode::Network, 1));
        assert!(!should_auto_retry(
            YtdlpErrorCode::Network,
            MAX_AUTO_RETRIES
        ));
        assert!(!should_auto_retry(YtdlpErrorCode::Auth, 0));
    }

    // -- aggregation -------------------------------------------------------

    #[test]
    fn the_summary_counts_every_state_once_and_sums_running_speed() {
        let mut a = item("q1", "https://a.example/1", ItemState::Downloading);
        a.progress.speed_bps = Some(3.0e6);
        let mut b = item("q2", "https://a.example/2", ItemState::Downloading);
        b.progress.speed_bps = Some(2.0e6);
        let items = vec![
            a,
            b,
            item("q3", "https://a.example/3", ItemState::Queued),
            item("q4", "https://a.example/4", ItemState::Done),
            item("q5", "https://a.example/5", ItemState::Failed),
            item("q6", "https://a.example/6", ItemState::Cancelled),
            item("q7", "https://a.example/7", ItemState::Paused),
            item("q8", "https://a.example/8", ItemState::Interrupted),
        ];
        let summary = aggregate(&items, false, LaneConfig::default());
        assert_eq!(summary.total, 8);
        assert_eq!(summary.running, 2);
        assert_eq!(summary.queued, 1);
        assert_eq!(summary.done, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.cancelled, 1);
        assert_eq!(summary.paused, 1);
        assert_eq!(summary.interrupted, 1);
        assert_eq!(summary.speed_bps, Some(5.0e6));
        assert!(summary.is_active());
        assert_eq!(summary.parallel_downloads, DEFAULT_PARALLEL_DOWNLOADS);
        assert_eq!(summary.per_host_cap, DEFAULT_PER_HOST_CAP);
    }

    #[test]
    fn an_idle_queue_has_no_speed_and_is_the_only_notification_candidate() {
        let idle = aggregate(&[], true, LaneConfig::default());
        assert_eq!(idle.speed_bps, None);
        assert!(!idle.is_active());
        assert!(idle.paused_all);

        let mut running = item("q1", "https://a.example/1", ItemState::Downloading);
        running.progress.speed_bps = Some(0.0);
        let busy = aggregate(std::slice::from_ref(&running), false, LaneConfig::default());
        // A speed of zero is not an aggregate: "no data" and "0 Mbit/s" would
        // look the same, and the first is honest.
        assert_eq!(busy.speed_bps, None);

        assert!(idle.became_idle(&busy));
        assert!(!idle.became_idle(&idle));
    }

    #[test]
    fn group_progress_counts_only_that_group() {
        let items = [
            item("q1", "https://a.example/1", ItemState::Done),
            item("q2", "https://a.example/2", ItemState::Downloading),
            {
                let mut other = item("q3", "https://b.example/1", ItemState::Done);
                other.group_id = Some("other".into());
                other
            },
        ];
        let with_group: Vec<QueueItem> = items
            .iter()
            .map(|i| {
                let mut clone = i.clone();
                if clone.group_id.is_none() {
                    clone.group_id = Some("g1".into());
                }
                clone
            })
            .collect();
        let progress = GroupProgress::of(&with_group, "g1");
        assert_eq!(progress.total, 2);
        assert_eq!(progress.done, 1);
        assert_eq!(progress.percent, Some(50.0));
    }

    // -- history -----------------------------------------------------------

    #[test]
    fn history_is_rolled_off_oldest_first_and_never_touches_live_items() {
        let mut items: Vec<QueueItem> = (0..HISTORY_LIMIT + 5)
            .map(|n| item(&format!("done-{n}"), "https://a.example/d", ItemState::Done))
            .collect();
        items.push(item("live", "https://a.example/live", ItemState::Queued));
        items.push(item(
            "paused",
            "https://a.example/paused",
            ItemState::Paused,
        ));

        let removed = roll_off_history(&mut items);
        assert_eq!(removed.len(), 5);
        assert_eq!(removed.first().map(String::as_str), Some("done-0"));
        let finished_left = items.iter().filter(|i| i.state.is_terminal()).count();
        assert_eq!(finished_left, HISTORY_LIMIT);
        assert!(items.iter().any(|i| i.id == "live"));
        assert!(items.iter().any(|i| i.id == "paused"));
    }

    #[test]
    fn a_queue_under_the_limit_keeps_everything() {
        let mut items: Vec<QueueItem> = (0..10)
            .map(|n| item(&format!("d{n}"), "https://a.example/d", ItemState::Done))
            .collect();
        assert!(roll_off_history(&mut items).is_empty());
        assert_eq!(items.len(), 10);
    }

    // -- misc --------------------------------------------------------------

    #[test]
    fn the_host_key_is_www_insensitive_and_lowercase() {
        assert_eq!(host_key("https://WWW.YouTube.com/watch?v=x"), "youtube.com");
        assert_eq!(host_key("https://youtu.be/x"), "youtu.be");
        assert_eq!(host_key("not a url"), "");
    }

    #[test]
    fn lane_config_is_clamped_to_something_that_can_actually_run() {
        let stuck = LaneConfig {
            download: 0,
            encode: 0,
            transcribe: 0,
            per_host_cap: 0,
        }
        .clamped();
        assert_eq!(stuck.download, 1, "zero would deadlock the queue");
        assert_eq!(stuck.encode, 1);
        assert_eq!(stuck.transcribe, 1);
        assert_eq!(stuck.per_host_cap, 1);
        let huge = LaneConfig {
            download: 99,
            encode: 99,
            transcribe: 99,
            per_host_cap: 99,
        }
        .clamped();
        assert_eq!(huge.download, 4);
        assert_eq!(huge.per_host_cap, 8);
    }

    #[test]
    fn a_log_tail_never_grows_past_the_cap() {
        let mut queue_item = QueueItem::default();
        for n in 0..(LOG_TAIL_LINES + 50) {
            queue_item.push_log(&format!("line {n}"));
        }
        assert_eq!(queue_item.log_tail.len(), LOG_TAIL_LINES);
        let expected = format!("line {}", LOG_TAIL_LINES + 49);
        assert_eq!(queue_item.log_tail.last(), Some(&expected));
    }

    #[test]
    fn a_paused_item_remembers_the_stage_it_was_paused_in() {
        let mut paused = item("q1", "https://a.example/1", ItemState::Paused);
        assert_eq!(paused.paused_from, None, "nothing was running yet");
        paused.paused_from = Some(ItemState::PostProcessing);
        assert_eq!(paused.paused_from, Some(ItemState::PostProcessing));
    }
}
