//! The queue IPC commands of spec §3: `queue_add`, `queue_list`, the actions,
//! and the batch inspection the Add bar needs.
//!
//! Like every module in `commands/`, there is no logic here: input is
//! validated, the services decide, and the answers are built. The one piece
//! with substance is [`AppToolProvider`], which is the bridge from "an item's
//! options" to "a resolved yt-dlp, ffmpeg and JS runtimes" — it has to live
//! where [`AppState`] is reachable, and that is here rather than in
//! `services/`.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime, State};

use crate::commands::env::resolve_job_tools;
use crate::error::{AppResult, ErrorCode, ErrorInfo};
use crate::services::job::JobEnvironment;
use crate::services::process;
use crate::services::queue::engine::{
    self, AddOutcome, AddRequest, BoxFuture, Outcome, ToolProvider,
};
use crate::services::queue::events::QueueSnapshot;
use crate::services::queue::model::{CookieChoice, ItemOptions, Kind, QualityChoice};
use crate::services::ytdlp;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// The tool provider
// ---------------------------------------------------------------------------

/// Builds the environment of an item from the tools the app resolved for it.
///
/// Generic over the runtime because it holds an `AppHandle`; the trait object
/// the engine keeps is `Arc<dyn ToolProvider>`, and the engine itself has no
/// idea Tauri exists.
pub struct AppToolProvider<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> AppToolProvider<R> {
    pub fn new(app: &AppHandle<R>) -> Self {
        Self { app: app.clone() }
    }
}

impl<R: Runtime> ToolProvider for AppToolProvider<R> {
    fn environment(&self, options: ItemOptions) -> BoxFuture<Result<JobEnvironment, ErrorInfo>> {
        let app = self.app.clone();
        Box::pin(async move {
            let state = app.try_state::<AppState>().ok_or_else(|| {
                ErrorInfo::new(
                    ErrorCode::Tauri,
                    "the app state is gone: the queue cannot run anything",
                )
            })?;
            let settings = state.settings();
            let tools = resolve_job_tools(&state, &settings).await?;

            // The item's folder has to exist before a process is started: the
            // arg builder passes it to `--paths`, and yt-dlp's failure would be
            // far less clear than this one (spec §13 acceptance item 13).
            crate::services::paths::ensure_dir(&options.output_dir).map_err(|err| {
                ErrorInfo::new(
                    ErrorCode::Io,
                    format!(
                        "the output folder {} cannot be created: {err}",
                        options.output_dir.display()
                    ),
                )
            })?;

            Ok(JobEnvironment {
                ytdlp: tools.ytdlp.path,
                ffmpeg: tools.ffmpeg,
                ffprobe: tools.ffprobe,
                whisper: tools.whisper,
                models_dir: Some(state.models_dir()),
                whisper_threads: settings.whisper_threads,
                skip_silence_vad: settings.skip_silence_vad,
                default_whisper_model: settings.active_whisper_model.clone(),
                default_transcript_language: Some(settings.default_transcript_language.clone()),
                runtimes: tools.runtimes,
                memory: state.runtime_memory().clone(),
                private_temp: tools.private_temp,
                youtube_clients: settings.youtube_player_clients.clone(),
            })
        })
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// What the Add bar sends. `kind` + `quality` is spec §2.3; the rest is what
/// the inspection already knows.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddItemRequest {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub site: Option<String>,
    #[serde(default)]
    pub duration_sec: Option<u64>,
    /// `video` | `audio`; `audio` implies `-x --audio-format opus` (spec §8.2).
    #[serde(default)]
    pub kind: String,
    /// `best` | `exact` | `maxHeight` | `audioOnly` | `explicit`.
    #[serde(default)]
    pub quality: QualityRequest,
    /// Overrides the global default output folder for this item (spec §11.3).
    #[serde(default)]
    pub output_dir: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    /// A sub-folder under the resolved output folder, e.g. the playlist title
    /// (spec §6.4 "folder per playlist"). Sanitised here, never trusted from the
    /// frontend: it may hold path separators or reserved Windows names.
    #[serde(default)]
    pub output_subdir: Option<String>,
    /// Optional literal prefix before `%(title).180B` in `-o`, e.g. `"01 - "`
    /// (spec §6.4, §8.2).
    #[serde(default)]
    pub filename_prefix: Option<String>,
    /// Per-item override for `[%(id)s]` suffix in filenames (spec §8.2, §11.8).
    #[serde(default)]
    pub include_id_in_filename: Option<bool>,
    /// Per-item override for `-S vcodec:h264,res,acodec:m4a` (spec §8.2, §11.8).
    #[serde(default)]
    pub prefer_h264: Option<bool>,
    /// Pass `--download-archive <app-data>/archive.txt` for this item (spec
    /// §6.2.4, §8.2, §11.4).
    #[serde(default)]
    pub use_archive: Option<bool>,
    /// Optional `(min, max)` sleep interval in seconds for playlists > 20 items
    /// (spec §6.4, §8.2).
    #[serde(default)]
    pub sleep_interval: Option<(u32, u32)>,
    /// Live stream recording mode (`Some(true)` = `--live-from-start`,
    /// `Some(false)` = `--no-live-from-start`, `None` = normal VOD, spec §6.4).
    #[serde(default)]
    pub live_from_start: Option<bool>,
    /// Per-link `--referer` override from Add bar "More options" (spec §6.3.4,
    /// §11.3).
    #[serde(default)]
    pub referer: Option<String>,
    /// Per-link `--user-agent` override from Add bar "More options" (spec
    /// §6.3.4, §11.3).
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Extra `Header: Value` lines from Add bar "More options" (spec §6.3.4,
    /// §11.3).
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    pub custom_headers: Vec<String>,
    #[serde(default)]
    pub encode: Option<bool>,
    #[serde(default)]
    pub video_codec: Option<String>,
    #[serde(default)]
    pub audio_codec: Option<String>,
    #[serde(default)]
    pub trim_start: Option<String>,
    #[serde(default)]
    pub trim_end: Option<String>,
    #[serde(default)]
    pub keep_original: Option<bool>,
    #[serde(default)]
    pub auto_play: Option<bool>,
    #[serde(default)]
    pub embed_metadata: Option<bool>,
    #[serde(default)]
    pub embed_thumbnail: Option<bool>,
    #[serde(default)]
    pub embed_chapters: Option<bool>,
    #[serde(default)]
    pub sponsorblock_mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    pub sponsorblock_categories: Vec<String>,
    #[serde(default)]
    pub transcribe: Option<bool>,
    #[serde(default)]
    pub whisper_model: Option<String>,
    #[serde(default)]
    pub transcript_language: Option<String>,
    #[serde(default)]
    pub translate_to_english: Option<bool>,
    #[serde(default)]
    pub transcripts_subfolder: Option<bool>,
}

fn deserialize_null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    let opt = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

/// The quality picker of spec §8.2, as the frontend sends it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QualityRequest {
    #[default]
    Best,
    /// Exact height, the original's behaviour.
    Exact {
        height: u32,
    },
    /// Max height, for playlists and sites with unreliable heights.
    MaxHeight {
        height: u32,
    },
    AudioOnly,
    /// An explicit format id from the advanced table (M3 fills the table).
    Explicit {
        format_id: String,
        has_audio: bool,
    },
}

impl QualityRequest {
    fn into_choice(self) -> QualityChoice {
        match self {
            Self::Best => QualityChoice::Best,
            Self::Exact { height } => QualityChoice::Exact {
                height: QualityChoice::clamped_height(height),
            },
            Self::MaxHeight { height } => QualityChoice::MaxHeight {
                height: QualityChoice::clamped_height(height),
            },
            Self::AudioOnly => QualityChoice::AudioOnly,
            Self::Explicit {
                format_id,
                has_audio,
            } => QualityChoice::Explicit {
                format_id,
                has_audio,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedItem {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateItem {
    pub url: String,
    pub item_id: String,
    pub title: Option<String>,
}

/// What `queue_add` did: the new ids, and the duplicates the UI has to ask about
/// (spec §7.3 "add anyway?").
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddResult {
    pub added: Vec<String>,
    pub duplicates: Vec<DuplicateItem>,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Everything the frontend needs to render the queue (spec §3 `queue_list`).
#[tauri::command]
pub fn queue_list(state: State<'_, AppState>) -> AppResult<QueueSnapshot> {
    Ok(state.require_queue()?.snapshot())
}

/// Add links to the queue (spec §2.2, §7.3).
///
/// `force` is the answer to "add anyway?"; without it the duplicates come back
/// in `duplicates` instead of being added, and nothing is enqueued for them.
#[tauri::command]
pub async fn queue_add<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    items: Vec<AddItemRequest>,
    force: Option<bool>,
) -> AppResult<AddResult> {
    if items.is_empty() {
        return Ok(AddResult::default());
    }
    let queue = state.require_queue()?;
    let settings = state.settings();
    let archive_path = state.tools().archive_path();

    let mut requests = Vec::with_capacity(items.len());
    for item in items {
        let url = item.url.trim().to_owned();
        if !process::is_allowed_url(&url) {
            // Spec §2.2: "ignore non-http(s)". A paste of ten links with one
            // `ftp://` in it must not lose the other nine, and it must not be a
            // hard error either.
            continue;
        }
        let requested_dir = item
            .output_dir
            .filter(|raw| !raw.trim().is_empty())
            .map(PathBuf::from);
        let dir = crate::services::paths::output_dir_for(
            requested_dir,
            settings.default_output_dir.clone(),
            crate::services::paths::default_output_dir(&app),
        )
        .ok_or_else(|| {
            ErrorInfo::new(
                ErrorCode::Unsupported,
                "no output folder: pick one in Settings, or set a default in your OS",
            )
        })?;
        let dir = match item
            .output_subdir
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            Some(name) => dir.join(crate::services::paths::sanitize_component(name, 100)),
            None => dir,
        };
        crate::services::paths::ensure_dir(&dir).map_err(|err| {
            ErrorInfo::new(
                ErrorCode::Io,
                format!(
                    "the output folder {} cannot be created: {err}",
                    dir.display()
                ),
            )
        })?;

        let quality = item.quality.into_choice();
        let kind = if item.kind.eq_ignore_ascii_case("audio") || quality == QualityChoice::AudioOnly
        {
            Kind::Audio
        } else {
            Kind::Video
        };

        let mut custom_headers = settings.custom_headers.clone();
        for header in item.custom_headers {
            let trimmed = header.trim();
            if !trimmed.is_empty() && trimmed.contains(':') && !trimmed.contains(['\r', '\n']) {
                custom_headers.push(trimmed.to_owned());
            }
        }
        // Spec §6.4: the download archive is for playlists ("skip already
        // downloaded (download archive, default on for playlists)"). Single-video
        // requests (`group_id == None`) do not pass `--download-archive` unless
        // `use_archive` is explicitly requested, so a user can always re-download
        // an individual video even if they deleted the file from disk.
        let use_archive = item.use_archive.unwrap_or_else(|| {
            if item.group_id.is_some() {
                settings.use_download_archive
            } else {
                false
            }
        });

        // Spec §9.2/§12.11: reject a malformed or inverted trim range up front.
        crate::services::ffmpeg::parse_and_validate_trim(
            item.trim_start.as_deref().filter(|s| !s.trim().is_empty()),
            item.trim_end.as_deref().filter(|s| !s.trim().is_empty()),
            item.duration_sec.map(|d| d as f64),
        )
        .map_err(|err| ErrorInfo::new(ErrorCode::Unsupported, format!("invalid trim: {err}")))?;

        let (sponsorblock_mode, sponsorblock_categories) = match item.sponsorblock_mode.as_deref() {
            Some("mark") if item.sponsorblock_categories.is_empty() => (None, Vec::new()),
            Some("remove") if item.sponsorblock_categories.is_empty() => (None, Vec::new()),
            Some("mark") => (
                Some(crate::services::ytdlp::SponsorBlockMode::Mark),
                item.sponsorblock_categories,
            ),
            Some("remove") => (
                Some(crate::services::ytdlp::SponsorBlockMode::Remove),
                item.sponsorblock_categories,
            ),
            Some("off") => (None, Vec::new()),
            _ => (
                settings.sponsorblock_mode,
                settings.sponsorblock_categories.clone(),
            ),
        };

        let options = ItemOptions {
            kind,
            // An audio-only item whose quality says otherwise is a bug in the
            // request, and the audio selector is what must win.
            quality: if kind == Kind::Audio {
                QualityChoice::AudioOnly
            } else {
                quality
            },
            output_dir: dir,
            cookies: CookieChoice::from_settings(
                settings.cookies_from_browser.as_deref(),
                settings.cookies_file.as_ref(),
            ),
            runtime_mode: settings.js_runtime_mode,
            filename_prefix: item.filename_prefix,
            include_id_in_filename: item
                .include_id_in_filename
                .unwrap_or(settings.include_id_in_filename),
            prefer_h264: item.prefer_h264.unwrap_or(settings.prefer_h264),
            archive_file: use_archive.then(|| archive_path.clone()),
            sleep_interval: item.sleep_interval,
            live_from_start: item.live_from_start,
            proxy: settings.proxy_url.clone(),
            limit_rate: settings.limit_rate.clone(),
            referer: item
                .referer
                .filter(|s| !s.trim().is_empty() && !s.contains(['\r', '\n']))
                .or_else(|| settings.custom_referer.clone()),
            user_agent: item
                .user_agent
                .filter(|s| !s.trim().is_empty() && !s.contains(['\r', '\n']))
                .or_else(|| settings.custom_user_agent.clone()),
            custom_headers,
            impersonate_retry: settings.impersonate_retry,
            encode: item.encode.unwrap_or(false),
            video_codec: item
                .video_codec
                .or_else(|| settings.default_video_codec.clone()),
            audio_codec: item
                .audio_codec
                .or_else(|| settings.default_audio_codec.clone()),
            trim_start: item.trim_start.filter(|s| !s.trim().is_empty()),
            trim_end: item.trim_end.filter(|s| !s.trim().is_empty()),
            keep_original: item
                .keep_original
                .unwrap_or(settings.keep_original_after_encode),
            auto_play: item.auto_play.unwrap_or(settings.auto_play),
            embed_metadata: item.embed_metadata.unwrap_or(settings.embed_metadata),
            embed_thumbnail: item.embed_thumbnail.unwrap_or(settings.embed_thumbnail),
            embed_chapters: item.embed_chapters.unwrap_or(settings.embed_chapters),
            sponsorblock_mode,
            sponsorblock_categories,
            transcribe: item.transcribe.unwrap_or(false),
            whisper_model: item
                .whisper_model
                .or_else(|| settings.active_whisper_model.clone()),
            transcript_language: item
                .transcript_language
                .or_else(|| Some(settings.default_transcript_language.clone())),
            translate_to_english: item
                .translate_to_english
                .or(Some(settings.whisper_translate_to_english)),
            transcripts_subfolder: item
                .transcripts_subfolder
                .or(Some(settings.transcripts_subfolder)),
        };

        requests.push(AddRequest {
            url,
            title: item.title,
            site: item.site,
            duration_sec: item.duration_sec,
            options,
            group_id: item.group_id,
            file: None,
        });
    }

    let outcome = queue.add(requests, force.unwrap_or(false));
    Ok(to_add_result(outcome))
}

/// Cancel one item: kill the tree, remove the temp dir (spec §7.3).
#[tauri::command]
pub fn queue_cancel(state: State<'_, AppState>, id: String) -> AppResult<bool> {
    Ok(state.require_queue()?.cancel(&id) == Outcome::Ok)
}

/// Pause one item, keeping its partial files.
#[tauri::command]
pub fn queue_pause(state: State<'_, AppState>, id: String) -> AppResult<bool> {
    Ok(state.require_queue()?.pause(&id) == Outcome::Ok)
}

/// Resume one item: yt-dlp continues from the `.part` files.
#[tauri::command]
pub fn queue_resume(state: State<'_, AppState>, id: String) -> AppResult<bool> {
    Ok(state.require_queue()?.resume(&id) == Outcome::Ok)
}

/// Retry one failed item.
#[tauri::command]
pub fn queue_retry(state: State<'_, AppState>, id: String) -> AppResult<bool> {
    Ok(state.require_queue()?.retry(&id) == Outcome::Ok)
}

/// Remove one item from the queue.
#[tauri::command]
pub fn queue_remove(state: State<'_, AppState>, id: String) -> AppResult<bool> {
    Ok(state.require_queue()?.remove(&id) == Outcome::Ok)
}

/// "Pause all" (spec §7.3).
#[tauri::command]
pub fn queue_pause_all(state: State<'_, AppState>) -> AppResult<()> {
    state.require_queue()?.pause_all();
    Ok(())
}

/// "Resume all": everything paused or interrupted goes back in line.
#[tauri::command]
pub fn queue_resume_all(state: State<'_, AppState>) -> AppResult<Vec<String>> {
    Ok(state.require_queue()?.resume_all())
}

/// "Retry failed" (spec §6.4, §7.3).
#[tauri::command]
pub fn queue_retry_failed(state: State<'_, AppState>) -> AppResult<Vec<String>> {
    Ok(state.require_queue()?.retry_failed())
}

/// Reorder, drag and drop style (spec §7.3).
#[tauri::command]
pub fn queue_reorder(state: State<'_, AppState>, ids: Vec<String>) -> AppResult<()> {
    state.require_queue()?.reorder(&ids);
    Ok(())
}

/// "Move to top".
#[tauri::command]
pub fn queue_move_to_top(state: State<'_, AppState>, id: String) -> AppResult<bool> {
    Ok(state.require_queue()?.move_to_top(&id) == Outcome::Ok)
}

/// "Clear finished" (spec §7.3).
#[tauri::command]
pub fn queue_clear_finished(state: State<'_, AppState>) -> AppResult<Vec<String>> {
    Ok(state.require_queue()?.clear_finished())
}

/// Register or update a playlist group (spec §6.4, §7.1, §11.5).
#[tauri::command]
pub fn queue_upsert_group(
    state: State<'_, AppState>,
    group: crate::services::queue::model::QueueGroup,
) -> AppResult<()> {
    state.require_queue()?.upsert_group(group);
    Ok(())
}

/// Pause every item in a playlist group (spec §6.4, §11.5).
#[tauri::command]
pub fn queue_pause_group(state: State<'_, AppState>, group_id: String) -> AppResult<Vec<String>> {
    Ok(state.require_queue()?.pause_group(&group_id))
}

/// Resume every paused/interrupted item in a playlist group (spec §6.4, §11.5).
#[tauri::command]
pub fn queue_resume_group(state: State<'_, AppState>, group_id: String) -> AppResult<Vec<String>> {
    Ok(state.require_queue()?.resume_group(&group_id))
}

/// Retry every failed item in a playlist group (spec §6.4, §11.5).
#[tauri::command]
pub fn queue_retry_group_failed(
    state: State<'_, AppState>,
    group_id: String,
) -> AppResult<Vec<String>> {
    Ok(state.require_queue()?.retry_group_failed(&group_id))
}

/// Remove every item in a playlist group and the group itself (spec §6.4, §11.5).
#[tauri::command]
pub fn queue_remove_group(state: State<'_, AppState>, group_id: String) -> AppResult<Vec<String>> {
    Ok(state.require_queue()?.remove_group(&group_id))
}

/// The lanes in force, so the Add bar can say what the queue will do
/// (spec §7.2).
#[tauri::command]
pub fn queue_lanes(state: State<'_, AppState>) -> AppResult<LaneConfigView> {
    let lanes = state.require_queue()?.lanes();
    Ok(LaneConfigView {
        parallel_downloads: lanes.download,
        per_host_cap: lanes.per_host_cap,
    })
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneConfigView {
    pub parallel_downloads: usize,
    pub per_host_cap: usize,
}

/// Read a dropped `.txt` of links (spec §2.2).
///
/// A cap, because this is a convenience and not a document reader: 256 KiB is
/// far more links than anybody pastes, and an unbounded read of an arbitrary
/// dropped file is a memory problem waiting for the wrong file.
#[tauri::command]
pub fn read_links_file(path: String) -> AppResult<String> {
    read_links_text(PathBuf::from(path.trim()))
}

/// The read itself, separate from the command so it is testable.
fn read_links_text(path: PathBuf) -> AppResult<String> {
    /// Enough for tens of thousands of links.
    const MAX_BYTES: u64 = 256 * 1024;

    let metadata = std::fs::metadata(&path).map_err(|err| {
        ErrorInfo::new(
            ErrorCode::Io,
            format!("{} cannot be read: {err}", path.display()),
        )
    })?;
    if !metadata.is_file() {
        return Err(ErrorInfo::new(
            ErrorCode::Unsupported,
            format!("{} is not a file", path.display()),
        ));
    }
    if metadata.len() > MAX_BYTES {
        return Err(ErrorInfo::new(
            ErrorCode::Unsupported,
            format!(
                "{} is larger than {} KiB: drop a plain list of links",
                path.display(),
                MAX_BYTES / 1024
            ),
        ));
    }
    let raw = std::fs::read(&path).map_err(|err| {
        ErrorInfo::new(
            ErrorCode::Io,
            format!("{} cannot be read: {err}", path.display()),
        )
    })?;
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

/// The language the webview resolved, for the system notification (spec §11.9).
#[tauri::command]
pub fn app_set_language(state: State<'_, AppState>, lang: String) -> AppResult<()> {
    state
        .ui_lang()
        .set(crate::services::notify::Lang::from_tag(&lang));
    Ok(())
}

/// Transcribe an existing queue item with a finished media file (spec §10.2, §11.5).
#[tauri::command]
pub fn queue_transcribe_item(
    state: State<'_, AppState>,
    id: String,
    whisper_model: Option<String>,
    transcript_language: Option<String>,
    translate_to_english: Option<bool>,
    transcripts_subfolder: Option<bool>,
) -> AppResult<bool> {
    let outcome = state.require_queue()?.transcribe_item(
        &id,
        whisper_model,
        transcript_language,
        translate_to_english,
        transcripts_subfolder,
    );
    Ok(outcome == Outcome::Ok)
}

/// Add a local media file to the queue for Whisper transcription (spec §10.2, §11.5).
#[tauri::command]
pub async fn queue_add_local_file(
    state: State<'_, AppState>,
    path: String,
    whisper_model: Option<String>,
    transcript_language: Option<String>,
    translate_to_english: Option<bool>,
    transcripts_subfolder: Option<bool>,
) -> AppResult<String> {
    let file_path = PathBuf::from(&path);
    if !file_path.is_file() {
        return Err(ErrorInfo::new(
            ErrorCode::Io,
            format!("The media file does not exist: {path}"),
        ));
    }

    let settings = state.settings();
    let title = file_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Local Media".to_string());

    let output_dir = file_path
        .parent()
        .map(|p| p.to_path_buf())
        .or_else(|| settings.default_output_dir.clone())
        .unwrap_or_else(|| PathBuf::from("."));

    let ffmpeg =
        crate::commands::env::resolve_cached(&state, &settings, crate::services::tools::FFMPEG)
            .await;
    let ffprobe = crate::commands::env::resolve_cached(&state, &settings, "ffprobe").await;
    let duration_sec = if let Some(ffmpeg) = ffmpeg.as_ref() {
        if let Some(ffprobe_path) = crate::services::ffmpeg::find_ffprobe(
            &ffmpeg.path,
            ffprobe.as_ref().map(|t| t.path.as_path()),
        ) {
            crate::services::ffmpeg::probe_file_duration(
                &ffprobe_path,
                &file_path,
                state.tools().temp_dir(),
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .map(|dur| dur.round() as u64)
        } else {
            None
        }
    } else {
        None
    };

    let item_options = ItemOptions {
        kind: Kind::Video,
        quality: QualityChoice::Best,
        output_dir,
        cookies: CookieChoice::None,
        runtime_mode: settings.js_runtime_mode,
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
        whisper_model: whisper_model.or_else(|| settings.active_whisper_model.clone()),
        transcript_language: transcript_language
            .or_else(|| Some(settings.default_transcript_language.clone())),
        translate_to_english: translate_to_english.or(Some(settings.whisper_translate_to_english)),
        transcripts_subfolder: transcripts_subfolder.or(Some(settings.transcripts_subfolder)),
    };

    let req = AddRequest {
        url: format!("local://{}", file_path.to_string_lossy()),
        title: Some(title),
        site: Some("local".to_string()),
        duration_sec,
        options: item_options,
        group_id: None,
        file: Some(file_path),
    };

    let queue = state.require_queue()?;
    let outcome = queue.add(vec![req], true);
    match outcome.added.into_iter().next() {
        Some(item_id) => Ok(item_id),
        None => Err(ErrorInfo::new(
            ErrorCode::Unsupported,
            "Failed to add local file to queue",
        )),
    }
}

/// A `ToolProvider` over the live app state, for `setup`.
pub fn provider<R: Runtime>(app: &AppHandle<R>) -> Arc<dyn ToolProvider> {
    Arc::new(AppToolProvider::new(app))
}

fn to_add_result(outcome: AddOutcome) -> AddResult {
    AddResult {
        added: outcome.added,
        duplicates: outcome
            .duplicates
            .into_iter()
            .map(|duplicate| DuplicateItem {
                url: duplicate.url,
                item_id: duplicate.item_id,
                title: duplicate.title,
            })
            .collect(),
    }
}

/// Split a paste into links (spec §2.2): whitespace-separated tokens, deduped,
/// non-http(s) dropped.
///
/// A pure function so the rule is testable and identical wherever it is used:
/// the frontend (for drag-and-drop text) and the backend (for a dropped file).
pub fn split_links(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut links = Vec::new();
    for token in text.split_whitespace() {
        let candidate =
            token.trim_matches(|c: char| matches!(c, ',' | ';' | '"' | '\'' | '<' | '>'));
        if !process::is_allowed_url(candidate) {
            continue;
        }
        if seen.insert(candidate.to_owned()) {
            links.push(candidate.to_owned());
        }
    }
    links
}

/// The cookie source the settings ask for (spec §8.6), for tests and for the
/// item snapshot.
pub fn cookies_from_settings(settings: &crate::state::Settings) -> ytdlp::CookieSource {
    CookieChoice::from_settings(
        settings.cookies_from_browser.as_deref(),
        settings.cookies_file.as_ref(),
    )
    .to_source()
}

/// Re-exported so `setup` does not have to reach into the engine module.
pub use engine::EngineConfig as QueueEngineConfig;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::queue::model::QualityChoice;

    #[test]
    fn a_paste_is_split_deduped_and_filtered() {
        let links = split_links(
            "https://a.example/1\n\nhttps://b.example/2, https://a.example/1\n\
             not-a-url\nftp://c.example/3\njavascript:alert(1)\nhttps://d.example/4",
        );
        assert_eq!(
            links,
            vec![
                "https://a.example/1",
                "https://b.example/2",
                "https://d.example/4"
            ]
        );
    }

    #[test]
    fn a_paste_with_nothing_usable_is_empty() {
        assert!(split_links("").is_empty());
        assert!(split_links("hello world\nftp://x/y").is_empty());
    }

    #[test]
    fn the_quality_request_builds_the_documented_choice() {
        assert_eq!(QualityRequest::default().into_choice(), QualityChoice::Best);
        assert_eq!(
            QualityRequest::Exact { height: 1080 }.into_choice(),
            QualityChoice::Exact { height: 1080 }
        );
        assert_eq!(
            QualityRequest::MaxHeight { height: 720 }.into_choice(),
            QualityChoice::MaxHeight { height: 720 }
        );
        assert_eq!(
            QualityRequest::AudioOnly.into_choice(),
            QualityChoice::AudioOnly
        );
        assert_eq!(
            QualityRequest::Explicit {
                format_id: "137".into(),
                has_audio: false
            }
            .into_choice(),
            QualityChoice::Explicit {
                format_id: "137".into(),
                has_audio: false
            }
        );
    }

    #[test]
    fn a_nonsense_height_is_clamped_at_the_command_boundary() {
        assert_eq!(
            QualityRequest::Exact { height: 0 }.into_choice(),
            QualityChoice::Exact { height: 144 }
        );
        assert_eq!(
            QualityRequest::MaxHeight { height: 99_999 }.into_choice(),
            QualityChoice::MaxHeight { height: 4320 }
        );
    }

    #[test]
    fn a_dropped_file_that_is_too_big_is_refused_with_a_clear_message() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let big = dir.path().join("big.txt");
        std::fs::write(&big, vec![b'x'; 300 * 1024]).expect("write");
        let err = read_links_text(big).expect_err("too big");
        assert_eq!(err.code, "UNSUPPORTED");
        assert!(err.detail.unwrap_or_default().contains("256"));
    }

    #[test]
    fn a_dropped_file_is_read_as_links() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let file = dir.path().join("links.txt");
        std::fs::write(&file, "https://a.example/1\nhttps://b.example/2\n").expect("write");
        let raw = read_links_text(file).expect("read");
        assert_eq!(
            split_links(&raw),
            vec!["https://a.example/1", "https://b.example/2"]
        );
    }

    #[test]
    fn a_directory_is_not_a_links_file() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let err = read_links_text(dir.path().to_path_buf()).expect_err("not a file");
        assert_eq!(err.code, "UNSUPPORTED");
    }
}
