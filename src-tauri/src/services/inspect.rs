//! Link inspection: one `yt-dlp -J --flat-playlist` call classifies a URL
//! (spec §6.2).
//!
//! With `--flat-playlist` a single video returns full detail (formats), while a
//! playlist returns a light list. M1 only needs the classification and the
//! video details; the playlist entries are an M3 concern and arrive here as a
//! stub that already carries what the UI needs to say "playlist, N entries,
//! later".
//!
//! Two rules from the spec shape this file:
//!
//! * **never fail just because heights are missing** (spec §6.2, and the
//!   original's assumption is spec §12.15): an audio-only source or a generic
//!   site without heights produces an empty resolution list, which the UI shows
//!   as "Best available + Audio only";
//! * the resolution list excludes storyboards/`mhtml` (spec §6.2), which is
//!   tested against a real 4K capture where those formats are present.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::ErrorInfo;
use crate::services::jsruntime::{self, RuntimeKind, RuntimeMemory, RuntimeMemoryStore};
use crate::services::process::{self, LineSink, ToolCommand};
use crate::services::ytdlp::{self, CookieSource, YtdlpError, YtdlpErrorCode};

// ---------------------------------------------------------------------------
// Timeouts (spec §6.2)
// ---------------------------------------------------------------------------

/// A single video: 60 s.
pub const VIDEO_INSPECT_TIMEOUT: Duration = Duration::from_secs(60);
/// A playlist: 120 s, because enumeration is slower and legitimately so.
pub const PLAYLIST_INSPECT_TIMEOUT: Duration = Duration::from_secs(120);

/// Does this URL look like a playlist/channel, i.e. one that may take longer
/// than a single video to enumerate?
///
/// Deliberately a cheap, pure heuristic: the URL has to be classified *before*
/// the inspection that would tell us, and the cost of a wrong answer is only
/// which timeout applies. A `list=` query on a `watch?v=` URL counts: that is
/// the "video + list" case of spec §6.4, and the inspection has to resolve the
/// video anyway.
pub fn looks_like_playlist(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    [
        "list=",
        "/playlist",
        "/videos",
        "/shorts",
        "/streams",
        "/@",
        "/channel/",
        "/c/",
        "/user/",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub fn inspect_timeout(url: &str) -> Duration {
    if looks_like_playlist(url) {
        PLAYLIST_INSPECT_TIMEOUT
    } else {
        VIDEO_INSPECT_TIMEOUT
    }
}

// ---------------------------------------------------------------------------
// The result model (spec §6.2)
// ---------------------------------------------------------------------------

/// One format, with only the fields spec §6.2 asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FormatLite {
    pub format_id: String,
    pub ext: String,
    pub height: Option<u32>,
    pub width: Option<u32>,
    pub fps: Option<f64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub filesize: Option<u64>,
    pub protocol: Option<String>,
    pub note: Option<String>,
}

impl FormatLite {
    /// Storyboards (`sb0`, `sb3`, …) are images dressed as video. Excluded from
    /// everything the user sees (spec §6.2).
    pub fn is_storyboard(&self) -> bool {
        self.format_id.starts_with("sb")
            || self.ext == "mhtml"
            || self.protocol.as_deref() == Some("mhtml")
    }

    pub fn has_video(&self) -> bool {
        !self.is_storyboard() && self.vcodec.as_deref().is_some_and(|codec| codec != "none")
    }

    pub fn has_audio(&self) -> bool {
        !self.is_storyboard() && self.acodec.as_deref().is_some_and(|codec| codec != "none")
    }
}

/// A video with everything the temporary M1 UI needs (and the M3 UI will keep).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub duration: Option<f64>,
    /// `extractor_key`, shown as the site badge (spec §6.3.2).
    pub site: String,
    pub webpage_url: Option<String>,
    pub is_live: bool,
    pub language: Option<String>,
    pub thumbnail_url: Option<String>,
    pub formats: Vec<FormatLite>,
    /// Distinct heights of real video formats ≥ 144, descending (spec §6.2).
    pub heights: Vec<u32>,
    pub has_video: bool,
    pub has_audio: bool,
    /// Set by the runner, not by the parse: `true` when even after the retries
    /// the extraction still looked incomplete (the JS challenge failed, or no
    /// format declared a height). The UI then says so and offers a retry
    /// instead of presenting a short resolution list as the whole truth.
    pub degraded: bool,
}

/// Does this run say the extraction is incomplete because the JS challenge was
/// not solved?
///
/// The two phrases are yt-dlp's own, per its source at the pinned version
/// (`report_warning(f'n challenge solving failed: Some formats may be
/// missing…')` and the `Signature solving failed` analogue). Matching on
/// "solving failed" plus "may be missing" keeps an age-restriction warning
/// ("some formats may be missing without account age-verification") out: that
/// one needs cookies, not another attempt.
pub fn is_challenge_degraded(stderr_tail: &[String]) -> bool {
    stderr_tail.iter().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.contains("solving failed") && lower.contains("may be missing")
    })
}

/// Does this video look like the resolution list is not the whole story?
///
/// Two signals, both specific:
/// * the warning above (authoritative);
/// * a video that declares video formats but no usable height at all — the
///   condition spec §5.3.2 already names ("zero video formats with a height").
///
/// A muxed-only source without a warning is **not** flagged: plenty of sites
/// legitimately publish one muxed file, and re-running would change nothing.
pub fn looks_degraded(url: &str, video: &VideoInfo, degraded_warning: bool) -> bool {
    if degraded_warning {
        return true;
    }
    video.has_video && video.heights.is_empty() && jsruntime::is_youtube_url(url)
}

/// Should `next` replace `previous` as the inspection result?
///
/// Only for two videos: the longer resolution list wins, then the higher top
/// height. A video is never replaced by a playlist or an error — the video is
/// what the user is looking at. Pure, so the rule is testable.
pub fn choose_next(previous: &Inspected, next: &Inspected) -> bool {
    match (previous, next) {
        (Inspected::Video(before), Inspected::Video(after)) => {
            let before_best = (
                before.heights.len(),
                before.heights.first().copied().unwrap_or(0),
            );
            let after_best = (
                after.heights.len(),
                after.heights.first().copied().unwrap_or(0),
            );
            after_best > before_best
        }
        (Inspected::Video(_), _) => false,
        _ => true,
    }
}
/// What kind of playlist a URL is (spec §6.4). The distinction is made at
/// inspection time because the UI wording differs ("playlist" vs "channel" vs
/// the endless mix warning).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaylistKind {
    Playlist,
    Channel,
    Mix,
    Other,
}

/// One entry of a playlist or multi-video page (spec §6.2, §6.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryLite {
    /// 1-based position in the playlist (`1`, `2`, …).
    pub index: u32,
    pub id: String,
    pub url: String,
    pub title: String,
    pub duration: Option<f64>,
    /// `false` for private, deleted or otherwise unplayable entries (spec §6.4).
    pub available: bool,
}

/// Default cap for standard playlists (spec §6.2, §6.4: `-I :1000`).
pub const PLAYLIST_PAGE_SIZE: u32 = 1000;

/// Default cap for YouTube Mix / Radio lists (spec §6.4: "cap to 50 items by default").
pub const MIX_DEFAULT_CAP: u32 = 50;

/// A playlist or multi-video container with its enumerated entries (spec §6.2, §6.4).
///
/// The data field is `playlist_kind`, not `kind`: `Inspected` is serialised as
/// an internally tagged enum whose tag **is** `kind`, and a second field with
/// the same name would produce a duplicate JSON key — one that serde writes
/// happily and every reader resolves differently.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistStub {
    pub id: String,
    pub title: String,
    pub site: String,
    pub webpage_url: Option<String>,
    pub playlist_kind: PlaylistKind,
    pub count: Option<u32>,
    pub entries: Vec<EntryLite>,
    /// True when the enumeration hit the `-I` cap and more entries exist.
    pub truncated: bool,
}

/// Alias matching the M3 terminology while keeping M1/M2 callers unbroken.
pub type PlaylistInfo = PlaylistStub;

/// The result of one inspection.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Inspected {
    Video(Box<VideoInfo>),
    Playlist(PlaylistStub),
    Error(ErrorInfo),
}

impl Inspected {
    /// The resolution labels of spec §6.2: `"1080p"`, descending.
    pub fn resolution_labels(&self) -> Vec<String> {
        match self {
            Self::Video(video) => video.heights.iter().map(|h| format!("{h}p")).collect(),
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Raw JSON (only the needed fields)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
struct RawFormat {
    format_id: Option<serde_json::Value>,
    ext: Option<String>,
    height: Option<u32>,
    width: Option<u32>,
    fps: Option<f64>,
    vcodec: Option<String>,
    acodec: Option<String>,
    filesize: Option<u64>,
    filesize_approx: Option<u64>,
    protocol: Option<String>,
    format_note: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawEntry {
    id: Option<serde_json::Value>,
    url: Option<String>,
    webpage_url: Option<String>,
    title: Option<String>,
    duration: Option<f64>,
    ie_key: Option<String>,
    extractor_key: Option<String>,
    availability: Option<String>,
    live_status: Option<String>,
    playlist_index: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawInfo {
    #[serde(rename = "_type")]
    info_type: Option<String>,
    id: Option<String>,
    title: Option<String>,
    duration: Option<f64>,
    extractor_key: Option<String>,
    webpage_url: Option<String>,
    is_live: Option<bool>,
    language: Option<String>,
    thumbnail: Option<String>,
    formats: Vec<RawFormat>,
    playlist_count: Option<u32>,
    entries: Option<Vec<serde_json::Value>>,
}

/// `format_id` (and occasionally entry `id`) is usually a string, but some
/// extractors emit numbers; accept both instead of failing the inspection.
fn json_id(value: &Option<serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// URL-case classification & entry reconstruction (spec §6.4)
// ---------------------------------------------------------------------------

/// Does this URL have **both** a single-video target and a `list=` query
/// parameter (e.g. `youtube.com/watch?v=xxx&list=PL…` or `youtu.be/xxx?list=PL…`)?
///
/// Spec §6.4: the Add bar asks inline whether to download *This video only* or
/// the *Whole playlist*.
pub fn is_video_with_list_url(url: &str) -> bool {
    let trimmed = url.trim();
    let Some((_, query_and_frag)) = trimmed.split_once('?') else {
        return false;
    };
    let query = query_and_frag.split('#').next().unwrap_or_default();
    let mut has_v = false;
    let mut has_list = false;
    for pair in query.split('&') {
        let (key, val) = pair.split_once('=').unwrap_or((pair, ""));
        if key.eq_ignore_ascii_case("v") && !val.trim().is_empty() {
            has_v = true;
        } else if key.eq_ignore_ascii_case("list") && !val.trim().is_empty() {
            has_list = true;
        }
    }
    if !has_list {
        return false;
    }
    if has_v {
        return true;
    }
    // `https://youtu.be/<id>?list=PL...` carries the video ID in the path.
    let lower = trimmed.to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("https://youtu.be/")
        .or_else(|| lower.strip_prefix("http://youtu.be/"))
    {
        let path_id = rest.split(['?', '#', '/']).next().unwrap_or_default();
        return !path_id.is_empty();
    }
    false
}

/// Does this URL point to a YouTube Mix / Radio (`list=RD…`, spec §6.4)?
pub fn is_mix_url(url: &str) -> bool {
    let Some((_, query_and_frag)) = url.trim().split_once('?') else {
        return false;
    };
    let query = query_and_frag.split('#').next().unwrap_or_default();
    for pair in query.split('&') {
        if let Some((key, val)) = pair.split_once('=')
            && key.eq_ignore_ascii_case("list")
            && val.trim().to_ascii_uppercase().starts_with("RD")
        {
            return true;
        }
    }
    false
}

/// Is this URL a search page, a feed, or a bare home page (spec §6.4)?
///
/// Such URLs are either unsupported or effectively unbounded feeds, and the
/// spec requires refusing them with a clear message rather than enumerating
/// thousands of unrelated items.
pub fn is_search_or_feed_url(url: &str) -> bool {
    let trimmed = url.trim();
    if !jsruntime::is_youtube_url(trimmed) {
        return false;
    }
    let Some(scheme_end) = trimmed.find("://") else {
        return false;
    };
    let after_scheme = &trimmed[scheme_end + 3..];
    let path_and_query = match after_scheme.find('/') {
        Some(pos) => &after_scheme[pos..],
        None => "",
    };
    let path = path_and_query
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('/');
    let lower_path = path.to_ascii_lowercase();

    if lower_path.is_empty() {
        // Bare `https://www.youtube.com/` without `?v=` or `?list=`.
        let lower_full = path_and_query.to_ascii_lowercase();
        return !lower_full.contains("v=") && !lower_full.contains("list=");
    }
    if lower_path == "/results"
        || lower_path.starts_with("/results/")
        || lower_path == "/feed"
        || lower_path.starts_with("/feed/")
        || lower_path == "/shorts"
    {
        return true;
    }
    false
}

/// Reconstruct a full `http(s)://` URL for one `--flat-playlist` entry
/// (spec §6.4).
///
/// With `--flat-playlist`, `entry.url` may be a full URL or just a video ID
/// depending on the extractor. We prefer `entry_url` when it is already an
/// `http(s)://` URL, then `entry_webpage_url`, and otherwise combine `entry_id`
/// (or a bare `entry_url`) with `ie_key` / `playlist_site`.
pub fn reconstruct_entry_url(
    entry_url: Option<&str>,
    entry_webpage_url: Option<&str>,
    entry_id: Option<&str>,
    ie_key: Option<&str>,
    playlist_site: &str,
) -> Option<String> {
    if let Some(url) = entry_url.map(str::trim).filter(|s| !s.is_empty()) {
        if process::is_allowed_url(url) {
            return Some(url.to_owned());
        }
        if let Some(rest) = url.strip_prefix("//") {
            let candidate = format!("https://{rest}");
            if process::is_allowed_url(&candidate) {
                return Some(candidate);
            }
        }
    }
    if let Some(web) = entry_webpage_url.map(str::trim).filter(|s| !s.is_empty())
        && process::is_allowed_url(web)
    {
        return Some(web.to_owned());
    }

    let id = entry_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| entry_url.map(str::trim).filter(|s| !s.is_empty()))?;

    let extractor = ie_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(playlist_site)
        .to_ascii_lowercase();

    if extractor.starts_with("youtube") {
        Some(format!("https://www.youtube.com/watch?v={id}"))
    } else if extractor == "vimeo" {
        Some(format!("https://vimeo.com/{id}"))
    } else if extractor == "dailymotion" {
        Some(format!("https://www.dailymotion.com/video/{id}"))
    } else if extractor == "twitchvod" {
        Some(format!("https://www.twitch.tv/videos/{id}"))
    } else {
        None
    }
}

/// Is a flat-playlist entry playable/available (spec §6.4)?
///
/// Private, deleted, unavailable and upcoming-premiere placeholders are marked
/// `false` so the Playlist selection panel renders them greyed-out and
/// unchecked by default.
pub fn is_entry_available(
    title: &str,
    availability: Option<&str>,
    live_status: Option<&str>,
    has_valid_url: bool,
) -> bool {
    if !has_valid_url {
        return false;
    }
    let lower_title = title.trim().to_ascii_lowercase();
    if matches!(
        lower_title.as_str(),
        "[private video]"
            | "[deleted video]"
            | "[unavailable video]"
            | "private video"
            | "deleted video"
            | "video unavailable"
    ) {
        return false;
    }
    if let Some(avail) = availability.map(|s| s.trim().to_ascii_lowercase())
        && matches!(
            avail.as_str(),
            "private" | "unavailable" | "needs_auth" | "subscriber_only" | "premium_only"
        )
    {
        return false;
    }
    if let Some(status) = live_status.map(|s| s.trim().to_ascii_lowercase())
        && status == "is_upcoming"
    {
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Pure parsing
// ---------------------------------------------------------------------------

/// Parse the JSON produced by `-J`. Never panics, never returns `Err`: a bad
/// document becomes [`Inspected::Error`] with a readable detail.
pub fn parse_inspect_json(raw: &str) -> Inspected {
    parse_inspect_json_with_range(raw, 1, PLAYLIST_PAGE_SIZE)
}

/// Same as [`parse_inspect_json`], with the 1-based `start_index` and the page
/// `limit` that `-I` asked for so `EntryLite::index` and `PlaylistStub::truncated`
/// reflect the requested slice.
pub fn parse_inspect_json_with_range(raw: &str, start_index: u32, limit: u32) -> Inspected {
    let trimmed = raw.trim();
    let info: RawInfo = match serde_json::from_str(trimmed) {
        Ok(info) => info,
        Err(first_error) => {
            // `null` is what the pinned yt-dlp prints when an extractor failed
            // without a traceback; treat it as "no information", not as JSON
            // garbage.
            if trimmed == "null" || trimmed.is_empty() {
                return Inspected::Error(ErrorInfo {
                    code: YtdlpErrorCode::Extract.as_str().to_owned(),
                    message_key: YtdlpErrorCode::Extract.message_key().to_owned(),
                    detail: Some("yt-dlp returned no information about this URL".to_owned()),
                });
            }
            // A tool whose stdout carries a stray line before the document: find
            // the first byte that can start one and retry from there. The JSON
            // of a large video is many lines long, so "the first line that
            // starts with {" would only ever recover `{` on its own.
            if let Some(start) = trimmed.find('{')
                && let Ok(recovered) = serde_json::from_str::<RawInfo>(&trimmed[start..])
            {
                return classify_info(recovered, start_index, limit);
            }
            return Inspected::Error(ErrorInfo {
                code: YtdlpErrorCode::Extract.as_str().to_owned(),
                message_key: YtdlpErrorCode::Extract.message_key().to_owned(),
                detail: Some(format!(
                    "could not parse the inspection JSON: {first_error}"
                )),
            });
        }
    };
    classify_info(info, start_index, limit)
}

fn classify_info(info: RawInfo, start_index: u32, limit: u32) -> Inspected {
    if matches!(info.info_type.as_deref(), Some("playlist" | "multi_video")) {
        let site = info.extractor_key.clone().unwrap_or_default();
        let url = info.webpage_url.clone().unwrap_or_default();
        let id = info.id.clone().unwrap_or_default();
        let kind = playlist_kind(&site, &id, &url);
        let base_index = start_index.max(1);
        let raw_entries = info.entries.unwrap_or_default();
        let mut entries = Vec::with_capacity(raw_entries.len());

        for (offset, value) in raw_entries.into_iter().enumerate() {
            if value.is_null() {
                continue;
            }
            let raw_entry: RawEntry = serde_json::from_value(value).unwrap_or_default();
            let entry_id = json_id(&raw_entry.id);
            let ie_key = raw_entry
                .ie_key
                .as_deref()
                .or(raw_entry.extractor_key.as_deref());
            let reconstructed = reconstruct_entry_url(
                raw_entry.url.as_deref(),
                raw_entry.webpage_url.as_deref(),
                if entry_id.is_empty() {
                    None
                } else {
                    Some(entry_id.as_str())
                },
                ie_key,
                &site,
            );
            let has_valid_url = reconstructed.is_some();
            let resolved_url = reconstructed.unwrap_or_default();
            let raw_title = raw_entry.title.unwrap_or_default();
            let available = is_entry_available(
                &raw_title,
                raw_entry.availability.as_deref(),
                raw_entry.live_status.as_deref(),
                has_valid_url,
            );
            let title = if raw_title.trim().is_empty() {
                if !entry_id.is_empty() {
                    entry_id.clone()
                } else {
                    resolved_url.clone()
                }
            } else {
                raw_title
            };
            let index = raw_entry
                .playlist_index
                .unwrap_or_else(|| base_index.saturating_add(u32::try_from(offset).unwrap_or(0)));
            entries.push(EntryLite {
                index,
                id: entry_id,
                url: resolved_url,
                title,
                duration: raw_entry.duration,
                available,
            });
        }

        let entries_len = u32::try_from(entries.len()).unwrap_or(u32::MAX);
        let count = info.playlist_count.or(Some(entries_len));
        let effective_limit = if kind == PlaylistKind::Mix && limit == PLAYLIST_PAGE_SIZE {
            MIX_DEFAULT_CAP
        } else {
            limit
        };
        let hit_cap = effective_limit > 0 && entries_len >= effective_limit;
        let more_exist = info.playlist_count.is_none_or(|total| {
            total == entries_len || total > (base_index - 1).saturating_add(entries_len)
        }) || kind == PlaylistKind::Mix;

        return Inspected::Playlist(PlaylistStub {
            playlist_kind: kind,
            id,
            title: info.title.unwrap_or_default(),
            site,
            webpage_url: info.webpage_url,
            count,
            entries,
            truncated: hit_cap && more_exist,
        });
    }

    // Anything that carries formats is a video; `_type` alone is not trusted,
    // because extractors have historically omitted it.
    if info.info_type.as_deref() == Some("video") || !info.formats.is_empty() {
        let formats: Vec<FormatLite> = info
            .formats
            .iter()
            .map(|raw| FormatLite {
                format_id: json_id(&raw.format_id),
                ext: raw.ext.clone().unwrap_or_default(),
                height: raw.height,
                width: raw.width,
                fps: raw.fps,
                vcodec: raw.vcodec.clone(),
                acodec: raw.acodec.clone(),
                filesize: raw.filesize.or(raw.filesize_approx),
                protocol: raw.protocol.clone(),
                note: raw.format_note.clone(),
            })
            .collect();
        let heights = resolution_heights(&formats);
        let has_video = formats.iter().any(FormatLite::has_video);
        let has_audio = formats.iter().any(FormatLite::has_audio);
        return Inspected::Video(Box::new(VideoInfo {
            id: info.id.unwrap_or_default(),
            title: info.title.unwrap_or_default(),
            duration: info.duration,
            site: info.extractor_key.unwrap_or_default(),
            webpage_url: info.webpage_url,
            is_live: info.is_live.unwrap_or(false),
            language: info.language,
            thumbnail_url: info.thumbnail,
            formats,
            heights,
            has_video,
            has_audio,
            // The parser cannot know: only the runner sees yt-dlp's warnings and
            // knows whether a retry already happened.
            degraded: false,
        }));
    }

    Inspected::Error(ErrorInfo {
        code: YtdlpErrorCode::Unsupported.as_str().to_owned(),
        message_key: YtdlpErrorCode::Unsupported.message_key().to_owned(),
        detail: Some("this URL did not resolve to a video or a playlist".to_owned()),
    })
}

/// The resolution list of spec §6.2.
///
/// First pass: distinct heights ≥ 144 among formats that really have a video
/// codec, storyboards excluded, descending. If that is empty — a generic site
/// or an extractor that omits codecs — retry **without the codec condition**,
/// as the original did. If there is still nothing, the answer is an empty list:
/// the UI then offers "Best available" and "Audio only", which is exactly what
/// the spec asks for over failing.
pub fn resolution_heights(formats: &[FormatLite]) -> Vec<u32> {
    let with_codec: Vec<u32> = formats
        .iter()
        .filter(|format| format.has_video())
        .filter_map(|format| format.height)
        .filter(|height| *height >= 144)
        .collect();
    let chosen = if with_codec.is_empty() {
        formats
            .iter()
            .filter(|format| !format.is_storyboard())
            .filter_map(|format| format.height)
            .filter(|height| *height >= 144)
            .collect()
    } else {
        with_codec
    };
    let mut heights: Vec<u32> = chosen;
    heights.sort_unstable();
    heights.dedup();
    heights.reverse();
    heights
}

/// Classify a playlist URL (spec §6.4). Mixes are the `RD…` radio lists: they
/// are dynamic and effectively endless, which the UI warns about.
pub fn playlist_kind(extractor_key: &str, id: &str, url: &str) -> PlaylistKind {
    let lower_url = url.to_ascii_lowercase();
    if id.starts_with("RD") || lower_url.contains("list=rd") {
        return PlaylistKind::Mix;
    }
    if extractor_key.eq_ignore_ascii_case("youtubetab")
        || lower_url.contains("/@")
        || lower_url.contains("/channel/")
        || lower_url.contains("/c/")
        || lower_url.contains("/user/")
    {
        return PlaylistKind::Channel;
    }
    if extractor_key.eq_ignore_ascii_case("youtube") {
        return PlaylistKind::Playlist;
    }
    PlaylistKind::Other
}

// ---------------------------------------------------------------------------
// Running the inspection
// ---------------------------------------------------------------------------

/// What a fallback trigger should do (spec §5.3.2, §5.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackDecision {
    /// Swap to Deno for the next attempt.
    Switch,
    /// The trigger fired but Deno is not installed: the "ask once" case of
    /// spec §5.3.2, which the UI answers with an install offer.
    DenoMissing,
    /// The trigger is not actionable: quickjs-only mode, or the fallback has
    /// already run.
    No,
}

/// A resolved runtime with the version its memory key needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRuntime {
    pub path: PathBuf,
    pub version: Option<String>,
}

/// Everything the inspection needs that is not user options.
#[derive(Debug, Clone)]
pub struct Runtimes {
    pub mode: jsruntime::JsRuntimeMode,
    /// The bundled QuickJS, when it could be resolved.
    pub qjs: Option<ResolvedRuntime>,
    /// Deno, when the user installed it (it is never bundled).
    pub deno: Option<ResolvedRuntime>,
    /// The yt-dlp version the decision is keyed by.
    pub ytdlp_version: String,
}

impl Runtimes {
    /// Which runtime this attempt should use (spec §5.3.3).
    pub fn start(&self, memory: Option<&RuntimeMemory>, now: u64) -> RuntimeKind {
        jsruntime::choose_start_runtime(
            self.mode,
            memory,
            &self.ytdlp_version,
            self.qjs.as_ref().and_then(|r| r.version.as_deref()),
            self.qjs.is_some(),
            now,
        )
    }

    pub fn path_of(&self, kind: RuntimeKind) -> Option<&Path> {
        match kind {
            RuntimeKind::QuickJs => self.qjs.as_ref().map(|r| r.path.as_path()),
            RuntimeKind::Deno => self.deno.as_ref().map(|r| r.path.as_path()),
        }
    }

    /// The automatic fallback of spec §5.3.2 only exists in Auto mode.
    pub fn fallback_allowed(&self) -> bool {
        self.mode == jsruntime::JsRuntimeMode::Auto
    }

    /// May this run switch from the current runtime to Deno right now?
    ///
    /// One shot, Auto mode only, QuickJS → Deno, and only when Deno really is
    /// there (an uninstalled fallback is reported as `DENO_MISSING` so the UI
    /// can offer to install it — never attempted blindly).
    pub fn may_fallback_to_deno(&self, current: RuntimeKind, already_fell_back: bool) -> bool {
        self.fallback_decision(current, already_fell_back) == FallbackDecision::Switch
    }

    /// The three-way decision behind both callers (inspection and download).
    pub fn fallback_decision(
        &self,
        current: RuntimeKind,
        already_fell_back: bool,
    ) -> FallbackDecision {
        if !self.fallback_allowed() || already_fell_back || current != RuntimeKind::QuickJs {
            return FallbackDecision::No;
        }
        if self.deno.is_some() {
            FallbackDecision::Switch
        } else {
            FallbackDecision::DenoMissing
        }
    }

    /// The requested runtime is missing and cannot be substituted.
    pub fn missing_runtime_error(&self, kind: RuntimeKind) -> YtdlpError {
        match kind {
            RuntimeKind::Deno => YtdlpError::new(
                YtdlpErrorCode::DenoMissing,
                "QuickJS could not solve the YouTube challenge and Deno, the automatic \
                 fallback, is not installed (spec §5.3). Install it from Settings → Tools.",
            ),
            RuntimeKind::QuickJs => YtdlpError::new(
                YtdlpErrorCode::YtChallenge,
                "QuickJS is required in this mode but is not available; check Settings → Tools.",
            ),
        }
    }

    pub fn qjs_version(&self) -> Option<&str> {
        self.qjs.as_ref().and_then(|r| r.version.as_deref())
    }
}

/// Extra options for a single inspection call (spec §6.2, §6.3, §6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectOptions {
    /// Explicit 1-based inclusive range `(start, end)` for `-I start:end`.
    /// When `None`, defaults to `:50` for YouTube Mix URLs (`list=RD…`) and
    /// `:1000` for everything else (spec §6.2, §6.4).
    pub item_range: Option<(u32, u32)>,
    /// Pass `--no-playlist` so a `watch?v=…&list=…` URL inspects only the
    /// video (spec §6.4).
    pub no_playlist: bool,
    /// Pass `--impersonate chrome` on the anti-bot retry (spec §6.3.3).
    pub impersonate_chrome: bool,
    /// Allow one retry with `--impersonate chrome` on `ANTIBOT` when the
    /// yt-dlp build supports chrome impersonation (spec §6.3.5, §8.5).
    pub impersonate_retry: bool,
    /// Network options from Settings / Add bar (spec §6.3.3, §8.2).
    pub proxy: Option<String>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub custom_headers: Vec<String>,
}

impl Default for InspectOptions {
    fn default() -> Self {
        Self {
            item_range: None,
            no_playlist: false,
            impersonate_chrome: false,
            impersonate_retry: true,
            proxy: None,
            referer: None,
            user_agent: None,
            custom_headers: Vec::new(),
        }
    }
}

impl InspectOptions {
    /// `(start_index, limit)` used by `parse_inspect_json_with_range`.
    pub fn effective_range(&self, url: &str) -> (u32, u32) {
        if let Some((start, end)) = self.item_range {
            let s = start.max(1);
            let e = end.max(s);
            return (s, e - s + 1);
        }
        if !self.no_playlist && is_mix_url(url) {
            (1, MIX_DEFAULT_CAP)
        } else {
            (1, PLAYLIST_PAGE_SIZE)
        }
    }

    /// Value passed to `-I` (`--playlist-items`).
    pub fn item_spec(&self, url: &str) -> String {
        if let Some((start, end)) = self.item_range {
            let s = start.max(1);
            let e = end.max(s);
            return format!("{s}:{e}");
        }
        if !self.no_playlist && is_mix_url(url) {
            format!(":{MIX_DEFAULT_CAP}")
        } else {
            format!(":{PLAYLIST_PAGE_SIZE}")
        }
    }
}

/// The environment of an inspection: the resolved binary, the runtimes, the
/// cookies and where the memory is written.
#[derive(Clone)]
pub struct InspectEnvironment {
    pub ytdlp: PathBuf,
    pub cookies: CookieSource,
    pub runtimes: Runtimes,
    pub memory: Arc<RuntimeMemoryStore>,
    /// Spec §4.9: the private TMP dir for QuickJS.
    pub private_temp: Option<PathBuf>,
    /// Expert override for YouTube's client list; `None` = yt-dlp's defaults.
    pub youtube_clients: Option<String>,
}

/// The argv array of the inspect command (spec §6.2), URL last after `--`.
///
/// **Deviation from spec §6.2, deliberate:** this command does *not* pass
/// `--no-warnings`. yt-dlp announces a degraded extraction only there —
/// measured on the pinned yt-dlp and confirmed in its source
/// (`yt_dlp/extractor/youtube/_video.py`): `Signature solving failed: Some
/// formats may be missing` and `n challenge solving failed: Some formats may be
/// missing`. Those two lines are the difference between "this video really has
/// only 360p" and "the JS challenge failed, so the DASH streams are gone", and
/// they are on **stderr** while the JSON is on **stdout**, so parsing is not
/// affected. Downloads keep `--no-warnings`; there the detection uses
/// `SDP_FORMAT` instead.
pub fn inspect_args(
    url: &str,
    runtime: Option<(RuntimeKind, &Path)>,
    cookies: &CookieSource,
    youtube_clients: Option<&str>,
) -> Vec<std::ffi::OsString> {
    inspect_args_with_options(
        url,
        runtime,
        cookies,
        youtube_clients,
        &InspectOptions::default(),
    )
}

/// Same as [`inspect_args`], with explicit M3 options (`-I` range,
/// `--no-playlist`, `--impersonate chrome`, proxy and custom headers).
pub fn inspect_args_with_options(
    url: &str,
    runtime: Option<(RuntimeKind, &Path)>,
    cookies: &CookieSource,
    youtube_clients: Option<&str>,
    options: &InspectOptions,
) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = Vec::new();
    for flag in ["--ignore-config", "-J", "--flat-playlist"] {
        args.push(flag.into());
    }
    if options.no_playlist {
        args.push("--no-playlist".into());
    } else {
        args.push("-I".into());
        args.push(options.item_spec(url).into());
    }
    // The tool writes what our reader decodes (spec §4.5): the same code page
    // that mangles a printed path would mangle the warning and error text this
    // inspection turns into user-visible messages. The JSON itself is
    // ASCII-escaped by the tool and would survive either way (D114).
    args.extend(ytdlp::ENCODING_ARGS.map(std::ffi::OsString::from));
    if let Some((kind, path)) = runtime {
        args.extend(jsruntime::runtime_args(kind, path));
    }
    // Only when the expert override is set; by default yt-dlp's own client list
    // is used (D74).
    if let Some(extractor_args) = jsruntime::youtube_extractor_args(url, youtube_clients) {
        args.push("--extractor-args".into());
        args.push(extractor_args.into());
    }
    args.extend(cookies.args());
    if let Some(proxy) = options
        .proxy
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("--proxy".into());
        args.push(proxy.into());
    }
    if let Some(referer) = options
        .referer
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("--referer".into());
        args.push(referer.into());
    }
    if let Some(ua) = options
        .user_agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("--user-agent".into());
        args.push(ua.into());
    }
    for header in &options.custom_headers {
        let trimmed = header.trim();
        if !trimmed.is_empty() && trimmed.contains(':') {
            args.push("--add-headers".into());
            args.push(trimmed.into());
        }
    }
    if options.impersonate_chrome {
        args.push("--impersonate".into());
        args.push("chrome".into());
    }
    args.push("--".into());
    args.push(url.into());
    args
}

/// What an inspection returned.
#[derive(Debug)]
pub enum InspectOutcome {
    Inspected {
        inspected: Box<Inspected>,
        /// Which runtime produced it (spec §15 item 3: the UI must show it).
        runtime: RuntimeKind,
        elapsed_ms: u64,
    },
    Cancelled,
}

/// The one public video the self-test uses (spec §5.3.6).
///
/// "Me at the zoo", the first YouTube video, published 2005: chosen for
/// stability, verified live with the pinned yt-dlp and the bundled qjs on
/// 2026-10-14 (the previous candidate, `BaW_jenozKc`, answers "This video is
/// unavailable" today — a known-good id is not known-good for ever, which is
/// why the check reports elapsed time and formats instead of trusting a name).
pub const SELF_TEST_URL: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";

/// The result of a "Test YouTube extraction" run (spec §5.3.6).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfTestReport {
    pub ok: bool,
    /// `quickjs` | `deno`, when an inspection ran at all.
    pub runtime: Option<String>,
    pub elapsed_ms: u64,
    pub formats: usize,
    pub heights: Vec<u32>,
    pub error: Option<ErrorInfo>,
}

/// Run the self-test: a known public video through `-J`, reporting the runtime
/// and the elapsed time (spec §5.3.6). The fallback applies exactly as in a
/// real inspection, so the report describes the strategy the app would use.
pub async fn self_test(env: &InspectEnvironment, token: CancellationToken) -> SelfTestReport {
    match inspect(env, SELF_TEST_URL, token).await {
        InspectOutcome::Cancelled => SelfTestReport {
            ok: false,
            runtime: None,
            elapsed_ms: 0,
            formats: 0,
            heights: Vec::new(),
            error: Some(ErrorInfo {
                code: "CANCELLED".to_owned(),
                message_key: "error.cancelled".to_owned(),
                detail: None,
            }),
        },
        InspectOutcome::Inspected {
            inspected,
            runtime,
            elapsed_ms,
        } => match &*inspected {
            Inspected::Video(video) => SelfTestReport {
                ok: true,
                runtime: Some(runtime.as_str().to_owned()),
                elapsed_ms,
                formats: video.formats.len(),
                heights: video.heights.clone(),
                error: None,
            },
            Inspected::Playlist(_) => SelfTestReport {
                ok: false,
                runtime: Some(runtime.as_str().to_owned()),
                elapsed_ms,
                formats: 0,
                heights: Vec::new(),
                error: Some(ErrorInfo {
                    code: YtdlpErrorCode::Unsupported.as_str().to_owned(),
                    message_key: YtdlpErrorCode::Unsupported.message_key().to_owned(),
                    detail: Some("the self-test URL resolved to a playlist".to_owned()),
                }),
            },
            Inspected::Error(info) => SelfTestReport {
                ok: false,
                runtime: Some(runtime.as_str().to_owned()),
                elapsed_ms,
                formats: 0,
                heights: Vec::new(),
                error: Some(info.clone()),
            },
        },
    }
}

/// Inspect `url` with default [`InspectOptions`].
pub async fn inspect(
    env: &InspectEnvironment,
    url: &str,
    token: CancellationToken,
) -> InspectOutcome {
    inspect_with_options(env, url, &InspectOptions::default(), token).await
}

/// Inspect `url` with explicit [`InspectOptions`], including the retries that
/// keep the resolution list honest.
///
/// The rule, in order:
/// 1. refuse search/feed/home URLs upfront (spec §6.4);
/// 2. one attempt with the chosen runtime;
/// 3. if the result looks degraded — yt-dlp's own "solving failed: some formats
///    may be missing" warning, or a video with no usable height (spec §5.3.2) —
///    make **one** more attempt: with Deno when the fallback is available
///    (spec §5.3.2/§5.3.3, remembered for 24 h), otherwise with the same
///    runtime, because a failed challenge solve is transient (that is exactly
///    what a user's "paste it again" fixes);
/// 4. return the best of what was seen; if it is still degraded, the flag says
///    so and the UI can offer a retry instead of pretending the list is
///    complete.
///
/// A `YT_CHALLENGE` **error** also triggers the runtime fallback, as before.
/// An `ANTIBOT` error triggers a one-shot retry with `--impersonate chrome`
/// when `options.impersonate_retry` is enabled and the yt-dlp binary supports
/// chrome impersonation (spec §6.3.5, §8.5).
pub async fn inspect_with_options(
    env: &InspectEnvironment,
    url: &str,
    options: &InspectOptions,
    token: CancellationToken,
) -> InspectOutcome {
    let now = jsruntime::now_unix();
    let mut runtime = env.runtimes.start(env.memory.get().as_ref(), now);

    if is_search_or_feed_url(url) {
        return InspectOutcome::Inspected {
            inspected: Box::new(Inspected::Error(ErrorInfo {
                code: YtdlpErrorCode::Unsupported.as_str().to_owned(),
                message_key: YtdlpErrorCode::Unsupported.message_key().to_owned(),
                detail: Some(
                    "search results, feeds and home pages are not downloadable playlists or videos (spec §6.4)"
                        .to_owned(),
                ),
            })),
            runtime,
            elapsed_ms: 0,
        };
    }

    let mut current_options = options.clone();
    let mut fell_back = false;
    let mut extra_attempt_made = false;
    let mut impersonate_retried = false;
    let mut elapsed_ms = 0_u64;
    // The best result seen so far, with whether *that* attempt was degraded.
    let mut best: Option<(Box<Inspected>, bool)> = None;

    loop {
        let started = std::time::Instant::now();
        let outcome = run_once(env, url, runtime, &current_options, token.clone()).await;
        elapsed_ms += u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match outcome {
            RunOutcome::Cancelled => return InspectOutcome::Cancelled,
            RunOutcome::Inspected {
                inspected,
                degraded_warning,
            } => {
                let degraded = match &*inspected {
                    Inspected::Video(video) => looks_degraded(url, video, degraded_warning),
                    _ => false,
                };
                let (chosen, chosen_degraded) = match best.take() {
                    Some((previous, previous_degraded)) => {
                        if choose_next(&previous, &inspected) {
                            (inspected, degraded)
                        } else {
                            (previous, previous_degraded)
                        }
                    }
                    None => (inspected, degraded),
                };

                if degraded && !extra_attempt_made {
                    // One more attempt: Deno when the fallback is available
                    // (spec §5.3.2/§5.3.3), otherwise the same runtime, because
                    // a failed challenge solve is transient — that is exactly
                    // what a user's "paste it again" fixes.
                    extra_attempt_made = true;
                    best = Some((chosen, chosen_degraded));
                    if env.runtimes.may_fallback_to_deno(runtime, fell_back) {
                        runtime = RuntimeKind::Deno;
                        fell_back = true;
                    }
                    continue;
                }

                if fell_back {
                    remember_deno(env, now);
                }
                return InspectOutcome::Inspected {
                    inspected: mark_degraded(*chosen, chosen_degraded),
                    runtime,
                    elapsed_ms,
                };
            }
            RunOutcome::Failed(error) => {
                if error.code == YtdlpErrorCode::YtChallenge
                    && env.runtimes.may_fallback_to_deno(runtime, fell_back)
                {
                    runtime = RuntimeKind::Deno;
                    fell_back = true;
                    continue;
                }
                if error.code == YtdlpErrorCode::Antibot
                    && current_options.impersonate_retry
                    && !impersonate_retried
                    && !current_options.impersonate_chrome
                {
                    impersonate_retried = true;
                    if ytdlp::probe_impersonate_chrome(&env.ytdlp, env.private_temp.as_deref())
                        .await
                    {
                        current_options.impersonate_chrome = true;
                        continue;
                    }
                }
                // An attempt that failed after a usable degraded one: keep the
                // usable one rather than losing the whole inspection.
                if let Some((best, best_degraded)) = best.take() {
                    if fell_back {
                        remember_deno(env, now);
                    }
                    return InspectOutcome::Inspected {
                        inspected: mark_degraded(*best, best_degraded),
                        runtime,
                        elapsed_ms,
                    };
                }
                return InspectOutcome::Inspected {
                    inspected: Box::new(Inspected::Error(error.to_error_info())),
                    runtime,
                    elapsed_ms,
                };
            }
        }
    }
}

/// Set the flag on a video result (no-op for anything else).
fn mark_degraded(inspected: Inspected, degraded: bool) -> Box<Inspected> {
    match inspected {
        Inspected::Video(mut video) => {
            video.degraded = degraded;
            Box::new(Inspected::Video(video))
        }
        other @ (Inspected::Playlist(_) | Inspected::Error(_)) => Box::new(other),
    }
}

fn remember_deno(env: &InspectEnvironment, now: u64) {
    env.memory.remember(
        &env.runtimes.ytdlp_version,
        env.runtimes.qjs_version(),
        RuntimeKind::Deno,
        now,
    );
}

enum RunOutcome {
    Inspected {
        inspected: Box<Inspected>,
        /// yt-dlp said the JS challenge failed (its "solving failed … some
        /// formats may be missing" warning).
        degraded_warning: bool,
    },
    Failed(YtdlpError),
    Cancelled,
}

async fn run_once(
    env: &InspectEnvironment,
    url: &str,
    runtime: RuntimeKind,
    options: &InspectOptions,
    token: CancellationToken,
) -> RunOutcome {
    let Some(runtime_path) = env.runtimes.path_of(runtime) else {
        // The chosen runtime is not installed at all (Auto never chooses a
        // missing Deno; explicit modes and Auto-without-qjs can).
        let error = YtdlpError::new(
            YtdlpErrorCode::YtChallenge,
            format!(
                "the {} runtime is not available; install it from Settings → Tools",
                runtime.as_str()
            ),
        );
        return RunOutcome::Failed(error);
    };

    let args = inspect_args_with_options(
        url,
        Some((runtime, runtime_path)),
        &env.cookies,
        env.youtube_clients.as_deref(),
        options,
    );
    let mut command = ToolCommand::new(&env.ytdlp)
        .args(args)
        .label("yt-dlp inspect");
    if let Some(temp) = &env.private_temp {
        command = command.temp_dir(temp);
    }

    // `-J` writes one big JSON document; collect it line by line.
    let buffer = Arc::new(Mutex::new(String::new()));
    let sink: LineSink = {
        let buffer = Arc::clone(&buffer);
        process::line_sink(move |line| {
            if let Ok(mut text) = buffer.lock() {
                text.push_str(line);
                text.push('\n');
            }
        })
    };

    let deadline = if options.no_playlist {
        VIDEO_INSPECT_TIMEOUT
    } else {
        inspect_timeout(url)
    };
    let result =
        tokio::time::timeout(deadline, process::spawn_tool(command, token.clone(), sink)).await;

    let process_result = match result {
        Ok(Ok(result)) => result,
        // Timeout: the future was dropped, and dropping a `RunningTool` kills
        // the tree (spec §4.2), so nothing is left behind.
        Err(_elapsed) => {
            return RunOutcome::Failed(YtdlpError::new(
                YtdlpErrorCode::Network,
                format!(
                    "the inspection did not finish within {} s (spec §6.2)",
                    deadline.as_secs()
                ),
            ));
        }
        Ok(Err(error)) => {
            return RunOutcome::Failed(YtdlpError::new(YtdlpErrorCode::Extract, error.to_string()));
        }
    };

    if process_result.cancelled {
        // Cancelled by the user (the caller's token) or the timeout path above.
        return RunOutcome::Cancelled;
    }

    if let Some(error) = ytdlp::classify_result(&process_result, jsruntime::is_youtube_url(url)) {
        return RunOutcome::Failed(error);
    }

    let raw = match buffer.lock() {
        Ok(text) => text.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    let (start_index, limit) = options.effective_range(url);
    RunOutcome::Inspected {
        inspected: Box::new(parse_inspect_json_with_range(&raw, start_index, limit)),
        degraded_warning: is_challenge_degraded(&process_result.stderr_tail),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4K capture from the pinned yt-dlp, trimmed to the fields this module
    /// reads. Storyboards are really in it (the real capture had `sb0`…`sb3`
    /// with `mhtml`), as is a vertical format and audio-only formats.
    const YOUTUBE_4K: &str = r#"{
      "_type": "video",
      "id": "dQw4w9WgXcQ",
      "title": "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)",
      "duration": 213.0,
      "extractor_key": "Youtube",
      "webpage_url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
      "is_live": false,
      "language": null,
      "thumbnail": "https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg",
      "formats": [
        {"format_id": "233", "ext": "mp4", "vcodec": "none", "acodec": null, "protocol": "m3u8_native"},
        {"format_id": "140", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "filesize": 309288, "protocol": "https"},
        {"format_id": "251", "ext": "webm", "vcodec": "none", "acodec": "opus", "filesize": 252182, "protocol": "https"},
        {"format_id": "160", "ext": "mp4", "height": 144, "width": 256, "fps": 15, "vcodec": "avc1.4D400B", "acodec": "none", "filesize": 195278, "protocol": "https"},
        {"format_id": "278", "ext": "webm", "height": 144, "width": 256, "fps": 15, "vcodec": "vp9", "acodec": "none", "protocol": "https"},
        {"format_id": "137", "ext": "mp4", "height": 1080, "width": 1920, "fps": 30, "vcodec": "avc1.640028", "acodec": "none", "filesize": 20573888, "protocol": "https"},
        {"format_id": "248", "ext": "webm", "height": 1080, "width": 1920, "fps": 30, "vcodec": "vp9", "acodec": "none", "protocol": "https"},
        {"format_id": "271", "ext": "webm", "height": 1440, "width": 2560, "fps": 30, "vcodec": "vp9", "acodec": "none", "protocol": "https"},
        {"format_id": "313", "ext": "webm", "height": 2160, "width": 3840, "fps": 30, "vcodec": "vp9", "acodec": "none", "protocol": "https"},
        {"format_id": "598", "ext": "mp4", "height": 144, "width": 256, "fps": 15.0, "vcodec": "av01.0.00M.08", "acodec": "none", "protocol": "m3u8_native"},
        {"format_id": "18", "ext": "mp4", "height": 360, "width": 640, "fps": 30, "vcodec": "avc1.42001E", "acodec": "mp4a.40.2", "filesize_approx": 7345000, "protocol": "https"},
        {"format_id": "sb0", "ext": "mhtml", "vcodec": "none", "acodec": "none", "protocol": "mhtml"},
        {"format_id": "sb3", "ext": "mhtml", "vcodec": "none", "acodec": "none", "protocol": "mhtml"}
      ]
    }"#;

    #[test]
    fn a_video_json_becomes_the_video_model_with_the_sorted_ladder() {
        let parsed = parse_inspect_json(YOUTUBE_4K);
        let Inspected::Video(video) = &parsed else {
            panic!("expected a video, got {parsed:?}");
        };
        assert_eq!(video.id, "dQw4w9WgXcQ");
        assert_eq!(video.site, "Youtube");
        assert_eq!(video.duration, Some(213.0));
        assert!(!video.is_live);
        assert!(video.has_video);
        assert!(video.has_audio);
        assert_eq!(
            video.heights,
            vec![2160, 1440, 1080, 360, 144],
            "distinct heights, descending, 144 floor"
        );
        assert_eq!(
            parsed.resolution_labels(),
            vec!["2160p", "1440p", "1080p", "360p", "144p"]
        );
        // Storyboards are in the capture and must be filtered out silently.
        assert!(video.formats.iter().any(FormatLite::is_storyboard));
        // filesize falls back to filesize_approx (format 18 has only that).
        let f18 = video
            .formats
            .iter()
            .find(|format| format.format_id == "18")
            .expect("format 18");
        assert_eq!(f18.filesize, Some(7_345_000));
        // Both a video-only and a muxed format are exposed to the format table.
        assert!(!f18.vcodec.as_deref().unwrap_or_default().is_empty());
        assert_ne!(f18.acodec.as_deref(), Some("none"));
    }

    #[test]
    fn a_vertical_video_lists_the_right_heights_from_height_not_width() {
        // A Short: 1080 wide, 1920 tall. The resolution list is heights, and
        // confusing width for height is exactly the vertical-video bug of
        // spec §2.9.
        let raw = r#"{
          "_type": "video", "id": "short", "title": "vertical", "extractor_key": "Youtube",
          "formats": [
            {"format_id": "137", "ext": "mp4", "height": 1920, "width": 1080, "vcodec": "avc1", "acodec": "none"},
            {"format_id": "248", "ext": "webm", "height": 1080, "width": 608, "vcodec": "vp9", "acodec": "none"},
            {"format_id": "140", "ext": "m4a", "height": null, "width": null, "vcodec": "none", "acodec": "mp4a"}
          ]
        }"#;
        let Inspected::Video(video) = parse_inspect_json(raw) else {
            panic!("expected a video");
        };
        assert_eq!(video.heights, vec![1920, 1080]);
        assert!(video.has_video);
        assert!(video.has_audio);
    }

    #[test]
    fn an_ultrawide_2to1_height_is_kept_as_is() {
        let raw = r#"{
          "_type": "video", "id": "uw", "title": "ultrawide", "extractor_key": "Youtube",
          "formats": [
            {"format_id": "a", "ext": "webm", "height": 1440, "width": 2880, "vcodec": "vp9", "acodec": "none"},
            {"format_id": "b", "ext": "mp4", "height": 1080, "width": 2160, "vcodec": "avc1", "acodec": "none"}
          ]
        }"#;
        let Inspected::Video(video) = parse_inspect_json(raw) else {
            panic!("expected a video");
        };
        assert_eq!(video.heights, vec![1440, 1080]);
    }

    #[test]
    fn an_audio_only_source_has_an_empty_resolution_list_and_does_not_fail() {
        let raw = r#"{
          "_type": "video", "id": "podcast", "title": "an episode",
          "extractor_key": "Soundcloud", "duration": 3600,
          "formats": [
            {"format_id": "a1", "ext": "mp3", "vcodec": "none", "acodec": "mp3", "protocol": "https"}
          ]
        }"#;
        let parsed = parse_inspect_json(raw);
        let Inspected::Video(video) = &parsed else {
            panic!("expected a video");
        };
        assert!(video.heights.is_empty(), "no heights is not an error");
        assert!(video.has_audio);
        assert!(!video.has_video);
        assert!(parsed.resolution_labels().is_empty());
    }

    #[test]
    fn a_generic_site_without_video_codecs_falls_back_to_heights_alone() {
        // The original's fallback (spec §6.2): no format declares a codec, but
        // a height exists.
        let raw = r#"{
          "_type": "video", "id": "g", "title": "generic", "extractor_key": "Generic",
          "formats": [
            {"format_id": "1", "ext": "mp4", "height": 720},
            {"format_id": "2", "ext": "mp4", "height": 480}
          ]
        }"#;
        let Inspected::Video(video) = parse_inspect_json(raw) else {
            panic!("expected a video");
        };
        assert_eq!(video.heights, vec![720, 480]);
    }

    #[test]
    fn a_format_with_an_unknown_height_never_hides_the_known_ones() {
        let raw = r#"{
          "_type": "video", "id": "x", "title": "t", "extractor_key": "Generic",
          "formats": [
            {"format_id": "1", "ext": "mp4", "height": null, "vcodec": "h264", "acodec": "aac"},
            {"format_id": "2", "ext": "mp4", "height": 480, "vcodec": "h264", "acodec": "none"}
          ]
        }"#;
        let Inspected::Video(video) = parse_inspect_json(raw) else {
            panic!("expected a video");
        };
        assert_eq!(video.heights, vec![480]);
    }

    #[test]
    fn numeric_format_ids_do_not_break_the_document() {
        let raw = r#"{
          "_type": "video", "id": "n", "title": "t", "extractor_key": "Generic",
          "formats": [{"format_id": 22, "ext": "mp4", "height": 720, "vcodec": "h264", "acodec": "aac"}]
        }"#;
        let Inspected::Video(video) = parse_inspect_json(raw) else {
            panic!("expected a video");
        };
        assert_eq!(video.formats[0].format_id, "22");
    }

    #[test]
    fn a_playlist_becomes_the_stub_with_its_kind_and_count() {
        let raw = r#"{
          "_type": "playlist",
          "id": "PLabcdefghijklmnop",
          "title": "A playlist",
          "extractor_key": "Youtube",
          "webpage_url": "https://www.youtube.com/playlist?list=PLabcdefghijklmnop",
          "playlist_count": 42,
          "entries": [
            {"id": "a", "title": "First video", "duration": 120.0},
            {"id": "b", "title": "[Private video]", "availability": "private"}
          ]
        }"#;
        let Inspected::Playlist(playlist) = parse_inspect_json(raw) else {
            panic!("expected a playlist");
        };
        assert_eq!(playlist.playlist_kind, PlaylistKind::Playlist);
        assert_eq!(playlist.count, Some(42));
        assert_eq!(playlist.title, "A playlist");
        assert!(!playlist.truncated);
        assert_eq!(playlist.entries.len(), 2);
        assert_eq!(
            playlist.entries[0],
            EntryLite {
                index: 1,
                id: "a".into(),
                url: "https://www.youtube.com/watch?v=a".into(),
                title: "First video".into(),
                duration: Some(120.0),
                available: true,
            }
        );
        assert_eq!(
            playlist.entries[1],
            EntryLite {
                index: 2,
                id: "b".into(),
                url: "https://www.youtube.com/watch?v=b".into(),
                title: "[Private video]".into(),
                duration: None,
                available: false,
            }
        );
    }

    #[test]
    fn the_playlist_stub_serialises_without_duplicate_keys() {
        // `Inspected` is internally tagged with `kind`; the stub's own field is
        // `playlistKind`. A field named `kind` would emit the key twice, and
        // nothing on the Rust side would complain.
        let playlist = Inspected::Playlist(PlaylistStub {
            id: "PL1".into(),
            title: "t".into(),
            site: "Youtube".into(),
            webpage_url: Some("https://www.youtube.com/playlist?list=PL1".into()),
            playlist_kind: PlaylistKind::Channel,
            count: Some(3),
            entries: vec![EntryLite {
                index: 1,
                id: "v1".into(),
                url: "https://www.youtube.com/watch?v=v1".into(),
                title: "Video 1".into(),
                duration: Some(60.0),
                available: true,
            }],
            truncated: false,
        });
        let json = serde_json::to_string(&playlist).expect("serialise");
        assert_eq!(json.matches("\"kind\"").count(), 1, "{json}");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(value["kind"], "playlist");
        assert_eq!(value["playlistKind"], "channel");
        assert_eq!(value["count"], 3);
        assert_eq!(
            value["entries"][0]["url"],
            "https://www.youtube.com/watch?v=v1"
        );
    }

    #[test]
    fn playlist_kinds_are_classified() {
        assert_eq!(
            playlist_kind(
                "Youtube",
                "PL123",
                "https://www.youtube.com/playlist?list=PL123"
            ),
            PlaylistKind::Playlist
        );
        assert_eq!(
            playlist_kind(
                "Youtube",
                "RDabc",
                "https://www.youtube.com/watch?v=x&list=RDabc"
            ),
            PlaylistKind::Mix
        );
        assert_eq!(
            playlist_kind(
                "YoutubeTab",
                "UC123",
                "https://www.youtube.com/@name/videos"
            ),
            PlaylistKind::Channel
        );
        assert_eq!(
            playlist_kind("Vimeo", "album", "https://vimeo.com/album/1"),
            PlaylistKind::Other
        );
    }

    #[test]
    fn entry_url_reconstruction_handles_full_urls_bare_ids_and_extractors() {
        // 1. `entry.url` is already an http(s) URL.
        assert_eq!(
            reconstruct_entry_url(
                Some("https://vimeo.com/12345"),
                None,
                Some("12345"),
                Some("Vimeo"),
                "Vimeo"
            ),
            Some("https://vimeo.com/12345".into())
        );
        // 2. `entry.url` is a bare ID on YouTube.
        assert_eq!(
            reconstruct_entry_url(
                Some("dQw4w9WgXcQ"),
                None,
                Some("dQw4w9WgXcQ"),
                Some("Youtube"),
                "YoutubeTab"
            ),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into())
        );
        // 3. `entry.url` is missing, falls back to `webpage_url`.
        assert_eq!(
            reconstruct_entry_url(
                None,
                Some("https://soundcloud.com/artist/track"),
                Some("99"),
                None,
                "SoundcloudSet"
            ),
            Some("https://soundcloud.com/artist/track".into())
        );
        // 4. Protocol-relative URL.
        assert_eq!(
            reconstruct_entry_url(Some("//example.com/video/1"), None, None, None, "Generic"),
            Some("https://example.com/video/1".into())
        );
        // 5. Bare ID on Vimeo and TwitchVod.
        assert_eq!(
            reconstruct_entry_url(None, None, Some("777"), Some("Vimeo"), "Vimeo"),
            Some("https://vimeo.com/777".into())
        );
        assert_eq!(
            reconstruct_entry_url(None, None, Some("888"), Some("TwitchVod"), "Twitch"),
            Some("https://www.twitch.tv/videos/888".into())
        );
        // 6. Unknown site with bare ID only cannot be guessed safely.
        assert_eq!(
            reconstruct_entry_url(None, None, Some("id1"), None, "CustomExtractor"),
            None
        );
    }

    #[test]
    fn multi_video_and_truncated_pages_are_parsed_with_indices() {
        let raw = r#"{
          "_type": "multi_video",
          "id": "course-1",
          "title": "Course Module",
          "extractor_key": "Generic",
          "webpage_url": "https://example.com/course",
          "playlist_count": 105,
          "entries": [
            {"id": "1", "url": "https://example.com/v1", "title": "Part 1", "duration": 60.0},
            {"id": "2", "url": "https://example.com/v2", "title": "[Deleted video]"},
            {"id": "3", "url": "https://example.com/v3", "title": "Part 3", "live_status": "is_upcoming"}
          ]
        }"#;
        let Inspected::Playlist(playlist) = parse_inspect_json_with_range(raw, 101, 3) else {
            panic!("expected a playlist");
        };
        assert_eq!(playlist.playlist_kind, PlaylistKind::Other);
        assert_eq!(playlist.count, Some(105));
        assert!(
            playlist.truncated,
            "3 entries hit the page limit of 3 with 105 total"
        );
        assert_eq!(playlist.entries[0].index, 101);
        assert!(playlist.entries[0].available);
        assert_eq!(playlist.entries[1].index, 102);
        assert!(!playlist.entries[1].available, "deleted video is disabled");
        assert_eq!(playlist.entries[2].index, 103);
        assert!(
            !playlist.entries[2].available,
            "upcoming premiere is disabled"
        );
    }

    #[test]
    fn url_cases_detect_video_with_list_mix_and_search_or_feed() {
        assert!(is_video_with_list_url(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL12345"
        ));
        assert!(is_video_with_list_url(
            "https://youtu.be/dQw4w9WgXcQ?list=RDdQw4w9WgXcQ"
        ));
        assert!(!is_video_with_list_url(
            "https://www.youtube.com/playlist?list=PL12345"
        ));
        assert!(!is_video_with_list_url(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        ));

        assert!(is_mix_url(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=RDdQw4w9WgXcQ"
        ));
        assert!(is_mix_url("https://www.youtube.com/playlist?list=RDMM"));
        assert!(!is_mix_url("https://www.youtube.com/playlist?list=PL12345"));

        assert!(is_search_or_feed_url("https://www.youtube.com/"));
        assert!(is_search_or_feed_url(
            "https://www.youtube.com/results?search_query=rust"
        ));
        assert!(is_search_or_feed_url(
            "https://www.youtube.com/feed/subscriptions"
        ));
        assert!(is_search_or_feed_url("https://www.youtube.com/shorts"));
        assert!(!is_search_or_feed_url(
            "https://www.youtube.com/shorts/abc123xyz"
        ));
        assert!(!is_search_or_feed_url(
            "https://www.youtube.com/@channel/shorts"
        ));
        assert!(!is_search_or_feed_url(
            "https://www.youtube.com/playlist?list=WL"
        ));
    }

    #[test]
    fn garbage_and_null_produce_a_readable_error_instead_of_a_panic() {
        for raw in ["", "null", "{ this is not json", "[]"] {
            match parse_inspect_json(raw) {
                Inspected::Error(info) => {
                    assert!(info.detail.is_some(), "a detail is always present");
                }
                other => panic!("{raw:?} must be an error, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_document_with_a_stray_line_before_it_is_recovered() {
        let raw = format!("some warning line\n{YOUTUBE_4K}");
        let Inspected::Video(video) = parse_inspect_json(&raw) else {
            panic!("expected a video");
        };
        assert_eq!(video.id, "dQw4w9WgXcQ");
    }

    #[test]
    fn only_the_challenge_warnings_count_as_a_degraded_extraction() {
        // The two sentences yt-dlp prints in the pinned version.
        assert!(is_challenge_degraded(&[
            "WARNING: [youtube] n challenge solving failed: Some formats may be missing. Make sure you have a JavaScript runtime".to_owned()
        ]));
        assert!(is_challenge_degraded(&[
            "WARNING: [youtube] Signature solving failed: Some formats may be missing.".to_owned()
        ]));
        // The age-restriction warning contains the same tail but is not this
        // problem: it needs cookies, not another attempt.
        assert!(!is_challenge_degraded(&[
            "WARNING: [youtube] x: This video is age-restricted; some formats may be missing without account age-verification".to_owned()
        ]));
        assert!(!is_challenge_degraded(&[]));
        assert!(!is_challenge_degraded(&[
            "[download] Destination: x".to_owned()
        ]));
    }

    #[test]
    fn the_degraded_flag_needs_the_warning_or_a_video_without_heights() {
        let mut video = match parse_inspect_json(YOUTUBE_4K) {
            Inspected::Video(video) => video,
            other => panic!("expected a video: {other:?}"),
        };
        // A healthy YouTube video with the full ladder.
        assert!(!looks_degraded(
            "https://www.youtube.com/watch?v=x",
            &video,
            false
        ));
        // The warning alone is enough, whatever the heights say.
        assert!(looks_degraded(
            "https://www.youtube.com/watch?v=x",
            &video,
            true
        ));
        // No heights at all on a YouTube URL: the spec §5.3.2 condition.
        video.heights.clear();
        assert!(looks_degraded(
            "https://www.youtube.com/watch?v=x",
            &video,
            false
        ));
        // …but not on a generic site: a muxed-only source is a normal source.
        assert!(!looks_degraded("https://vimeo.com/1", &video, false));
    }

    #[test]
    fn the_better_result_wins_and_a_video_is_never_replaced_by_an_error() {
        let full = parse_inspect_json(YOUTUBE_4K); // 5 heights
        let degraded = parse_inspect_json(
            r#"{"_type":"video","id":"x","title":"t","extractor_key":"Youtube",
                 "formats":[{"format_id":"18","ext":"mp4","height":360,"vcodec":"avc1","acodec":"mp4a"}]}"#,
        );
        // The full list replaces the degraded one…
        assert!(choose_next(&degraded, &full));
        // …and never the other way round.
        assert!(!choose_next(&full, &degraded));

        let error = parse_inspect_json("null");
        assert!(
            !choose_next(&full, &error),
            "an error never replaces a video"
        );
        assert!(choose_next(&error, &full), "a video replaces an error");
        // Same shape: keep the first (no needless churn).
        assert!(!choose_next(&full, &full));
    }

    #[test]
    fn the_timeout_follows_the_url_shape() {
        assert_eq!(
            inspect_timeout("https://www.youtube.com/watch?v=x"),
            VIDEO_INSPECT_TIMEOUT
        );
        assert_eq!(
            inspect_timeout("https://www.youtube.com/playlist?list=PL1"),
            PLAYLIST_INSPECT_TIMEOUT
        );
        assert_eq!(
            inspect_timeout("https://www.youtube.com/@name/videos"),
            PLAYLIST_INSPECT_TIMEOUT
        );
        assert_eq!(
            inspect_timeout("https://www.youtube.com/watch?v=x&list=PL1"),
            PLAYLIST_INSPECT_TIMEOUT
        );
        assert!(!looks_like_playlist("https://vimeo.com/12345"));
    }

    #[test]
    fn inspect_args_put_the_url_last_and_respect_the_runtime_order() {
        let args = inspect_args(
            "https://youtu.be/x",
            Some((RuntimeKind::QuickJs, Path::new("/opt/qjs"))),
            &CookieSource::None,
            Some("web_embedded,web,tv"),
        );
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        for required in [
            "--ignore-config",
            "-J",
            "--flat-playlist",
            "-I",
            ":1000",
            "--encoding",
        ] {
            assert!(rendered.contains(&required.to_owned()), "{rendered:?}");
        }
        // The flag belongs to how we call the tool, not to one stage (D114);
        // here it is the warning and error text that would otherwise arrive
        // mangled, since the JSON itself is ASCII-escaped by the tool.
        assert!(rendered.contains(&"utf-8".to_owned()), "{rendered:?}");
        // Deliberate deviation from spec §6.2: warnings stay ON, because the
        // "solving failed: some formats may be missing" line is the only honest
        // signal of a degraded extraction (they go to stderr, the JSON to
        // stdout).
        assert!(
            !rendered.contains(&"--no-warnings".to_owned()),
            "the inspection must keep warnings: {rendered:?}"
        );
        let clear = rendered.iter().position(|a| a == "--no-js-runtimes");
        let enable = rendered.iter().position(|a| a == "--js-runtimes");
        assert!(clear < enable);
        assert_eq!(rendered[rendered.len() - 2], "--");
        assert_eq!(rendered[rendered.len() - 1], "https://youtu.be/x");
        assert!(rendered.contains(&jsruntime::YOUTUBE_PLAYER_CLIENTS.to_owned()));

        // A non-YouTube link gets no YouTube extractor args.
        let args = inspect_args(
            "https://vimeo.com/1",
            None,
            &CookieSource::None,
            Some("web"),
        );
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(!rendered.contains(&jsruntime::YOUTUBE_PLAYER_CLIENTS.to_owned()));
        assert!(!rendered.contains(&"--js-runtimes".to_owned()));
    }

    #[test]
    fn inspect_args_with_options_supports_no_playlist_mix_cap_and_network_flags() {
        let opts = InspectOptions {
            item_range: Some((1001, 2000)),
            no_playlist: false,
            impersonate_chrome: true,
            impersonate_retry: true,
            proxy: Some("http://127.0.0.1:8080".into()),
            referer: Some("https://site.example/page".into()),
            user_agent: Some("CustomUA/1.0".into()),
            custom_headers: vec!["X-Test: 1".into()],
        };
        let args = inspect_args_with_options(
            "https://site.example/playlist",
            None,
            &CookieSource::None,
            None,
            &opts,
        );
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(rendered.windows(2).any(|w| w == ["-I", "1001:2000"]));
        assert!(
            rendered
                .windows(2)
                .any(|w| w == ["--proxy", "http://127.0.0.1:8080"])
        );
        assert!(
            rendered
                .windows(2)
                .any(|w| w == ["--referer", "https://site.example/page"])
        );
        assert!(
            rendered
                .windows(2)
                .any(|w| w == ["--user-agent", "CustomUA/1.0"])
        );
        assert!(
            rendered
                .windows(2)
                .any(|w| w == ["--add-headers", "X-Test: 1"])
        );
        assert!(
            rendered
                .windows(2)
                .any(|w| w == ["--impersonate", "chrome"])
        );
        assert_eq!(
            &rendered[rendered.len() - 2..],
            &["--", "https://site.example/playlist"]
        );

        // Mix default cap (`:50`) when no explicit range is passed.
        let mix_args = inspect_args(
            "https://www.youtube.com/watch?v=x&list=RDx",
            None,
            &CookieSource::None,
            None,
        );
        let mix_rendered: Vec<String> = mix_args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(mix_rendered.windows(2).any(|w| w == ["-I", ":50"]));

        // `no_playlist: true` passes `--no-playlist` instead of `-I`.
        let single_opts = InspectOptions {
            no_playlist: true,
            ..InspectOptions::default()
        };
        let single_args = inspect_args_with_options(
            "https://www.youtube.com/watch?v=x&list=PL1",
            None,
            &CookieSource::None,
            None,
            &single_opts,
        );
        let single_rendered: Vec<String> = single_args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(single_rendered.contains(&"--no-playlist".to_owned()));
        assert!(!single_rendered.contains(&"-I".to_owned()));
    }

    #[test]
    fn the_inspection_carries_the_cookies_it_was_given() {
        let args = inspect_args(
            "https://youtu.be/x",
            None,
            &CookieSource::Browser("firefox".into()),
            None,
        );
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(
            rendered
                .windows(2)
                .any(|pair| pair == ["--cookies-from-browser", "firefox"])
        );
    }
}
