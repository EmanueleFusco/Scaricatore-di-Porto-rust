//! Video and audio encoder catalogue, capability probing and hardware-to-CPU
//! fallback (spec §9.3, §9.4).
//!
//! # Specification Rules
//!
//! 1. Candidates per OS (spec §9.3):
//!    - all platforms: `libx264`, `libx265`
//!    - Windows: `h264_nvenc`, `hevc_nvenc`, `h264_amf`, `hevc_amf`, `h264_qsv`, `hevc_qsv`
//!    - Linux: `h264_nvenc`, `hevc_nvenc`, `h264_vaapi`, `hevc_vaapi`, `h264_qsv`, `hevc_qsv`
//!    - macOS: `h264_videotoolbox`, `hevc_videotoolbox`
//! 2. Filter candidate encoders by `ffmpeg -hide_banner -encoders`.
//! 3. Probe survivors with a short 256x256 null encode (10 s timeout). VAAPI adds
//!    `-vaapi_device /dev/dri/renderD128 -vf format=nv12,hwupload`.
//! 4. Cache probe results by `(ffmpeg_path, mtime, version)`.
//! 5. Automatic CPU fallback: if a hardware encoder fails at job time, retry once
//!    with the CPU encoder of the same codec family (`libx264` or `libx265`).

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::services::process::{self, LineSinks, ProcessResult, ToolCommand};
use crate::services::ytdlp::CfgOs;

/// Probe encode timeout per encoder candidate (spec §9.3: "10 s timeout each").
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Supported audio formats for encode (spec §9.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AudioCodec {
    #[default]
    Aac,
    Mp3,
    Opus,
}

impl AudioCodec {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Aac => "aac",
            Self::Mp3 => "mp3",
            Self::Opus => "opus",
        }
    }

    /// File extension produced by this audio codec (spec §9.4: aac -> .m4a, mp3 -> .mp3, opus -> .opus).
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Aac => "m4a",
            Self::Mp3 => "mp3",
            Self::Opus => "opus",
        }
    }

    /// FFmpeg arguments for encoding this audio format.
    pub const fn ffmpeg_args(self) -> &'static [&'static str] {
        match self {
            Self::Aac => &["-c:a", "aac", "-b:a", "192k"],
            Self::Mp3 => &["-c:a", "libmp3lame", "-q:a", "2"],
            Self::Opus => &["-c:a", "libopus", "-b:a", "128k"],
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "aac" | "m4a" => Some(Self::Aac),
            "mp3" => Some(Self::Mp3),
            "opus" => Some(Self::Opus),
            _ => None,
        }
    }
}

/// Known video codec identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderInfo {
    /// Encoder identifier (e.g. `libx264`, `h264_nvenc`).
    pub id: String,
    /// Human-readable label (spec §9.3: `x264`, `Nvidia x264`, etc.).
    pub label: String,
    /// Codec family: `"h264"` or `"hevc"`.
    pub codec: String,
    /// Whether this encoder uses hardware acceleration.
    pub is_hardware: bool,
}

/// Static definition of an encoder candidate.
#[derive(Debug, Clone, Copy)]
pub struct EncoderDefinition {
    pub id: &'static str,
    pub label: &'static str,
    pub codec: &'static str,
    pub is_hardware: bool,
    pub args: &'static [&'static str],
}

impl EncoderDefinition {
    pub const fn to_info(self) -> EncoderInfo {
        EncoderInfo {
            id: String::new(),
            label: String::new(),
            codec: String::new(),
            is_hardware: self.is_hardware,
        }
    }
}

/// The catalogue of supported encoders and their default encoding flags (spec §9.3).
pub static ENCODER_CATALOGUE: &[EncoderDefinition] = &[
    EncoderDefinition {
        id: "libx264",
        label: "x264",
        codec: "h264",
        is_hardware: false,
        args: &["-preset", "medium", "-crf", "20", "-pix_fmt", "yuv420p"],
    },
    EncoderDefinition {
        id: "libx265",
        label: "x265",
        codec: "hevc",
        is_hardware: false,
        args: &[
            "-preset", "medium", "-crf", "24", "-pix_fmt", "yuv420p", "-tag:v", "hvc1",
        ],
    },
    EncoderDefinition {
        id: "h264_nvenc",
        label: "Nvidia x264",
        codec: "h264",
        is_hardware: true,
        args: &["-preset", "p5", "-rc", "vbr", "-cq", "23", "-b:v", "0"],
    },
    EncoderDefinition {
        id: "hevc_nvenc",
        label: "Nvidia x265",
        codec: "hevc",
        is_hardware: true,
        args: &[
            "-preset", "p5", "-rc", "vbr", "-cq", "23", "-b:v", "0", "-tag:v", "hvc1",
        ],
    },
    EncoderDefinition {
        id: "h264_amf",
        label: "AMD x264",
        codec: "h264",
        is_hardware: true,
        args: &[
            "-quality", "quality", "-rc", "cqp", "-qp_i", "22", "-qp_p", "24",
        ],
    },
    EncoderDefinition {
        id: "hevc_amf",
        label: "AMD x265",
        codec: "hevc",
        is_hardware: true,
        args: &[
            "-quality", "quality", "-rc", "cqp", "-qp_i", "22", "-qp_p", "24", "-tag:v", "hvc1",
        ],
    },
    EncoderDefinition {
        id: "h264_qsv",
        label: "Intel x264",
        codec: "h264",
        is_hardware: true,
        args: &["-global_quality", "23", "-preset", "medium"],
    },
    EncoderDefinition {
        id: "hevc_qsv",
        label: "Intel x265",
        codec: "hevc",
        is_hardware: true,
        args: &[
            "-global_quality",
            "23",
            "-preset",
            "medium",
            "-tag:v",
            "hvc1",
        ],
    },
    EncoderDefinition {
        id: "h264_videotoolbox",
        label: "VideoToolbox x264",
        codec: "h264",
        is_hardware: true,
        args: &["-q:v", "60"],
    },
    EncoderDefinition {
        id: "hevc_videotoolbox",
        label: "VideoToolbox x265",
        codec: "hevc",
        is_hardware: true,
        args: &["-q:v", "60", "-tag:v", "hvc1"],
    },
    EncoderDefinition {
        id: "h264_vaapi",
        label: "VAAPI x264",
        codec: "h264",
        is_hardware: true,
        args: &[
            "-vaapi_device",
            "/dev/dri/renderD128",
            "-vf",
            "format=nv12,hwupload",
            "-qp",
            "23",
        ],
    },
    EncoderDefinition {
        id: "hevc_vaapi",
        label: "VAAPI x265",
        codec: "hevc",
        is_hardware: true,
        args: &[
            "-vaapi_device",
            "/dev/dri/renderD128",
            "-vf",
            "format=nv12,hwupload",
            "-qp",
            "23",
        ],
    },
];

/// Find an encoder definition by its identifier.
pub fn lookup_encoder(id: &str) -> Option<&'static EncoderDefinition> {
    ENCODER_CATALOGUE.iter().find(|def| def.id == id)
}

/// Returns the CPU fallback encoder id for a given encoder (spec §9.3).
///
/// If a hardware encoder fails at job time, retry once with the CPU encoder of
/// the same codec family (`libx264` or `libx265`).
pub fn cpu_fallback_encoder(encoder_id: &str) -> Option<&'static str> {
    let def = lookup_encoder(encoder_id)?;
    if !def.is_hardware {
        return None;
    }
    match def.codec {
        "h264" => Some("libx264"),
        "hevc" => Some("libx265"),
        _ => None,
    }
}

/// Candidate encoder IDs per operating system (spec §9.3).
pub fn candidates_for_os(os: CfgOs) -> &'static [&'static str] {
    match os {
        CfgOs::Linux => &[
            "libx264",
            "libx265",
            "h264_nvenc",
            "hevc_nvenc",
            "h264_vaapi",
            "hevc_vaapi",
            "h264_qsv",
            "hevc_qsv",
        ],
        CfgOs::Windows => &[
            "libx264",
            "libx265",
            "h264_nvenc",
            "hevc_nvenc",
            "h264_amf",
            "hevc_amf",
            "h264_qsv",
            "hevc_qsv",
        ],
        CfgOs::MacOs => &[
            "libx264",
            "libx265",
            "h264_videotoolbox",
            "hevc_videotoolbox",
        ],
    }
}

/// Parse the output of `ffmpeg -hide_banner -encoders` and return the set of
/// video encoder names present in the binary.
pub fn parse_supported_encoders(stdout: &str) -> HashSet<String> {
    let mut encoders = HashSet::new();
    for line in stdout.lines() {
        let trimmed = line.trim_start();
        // Encoder lines in ffmpeg look like:
        //  V..... libx264              libx264 H.264 / AVC...
        //  V....D h264_nvenc           NVIDIA NVENC...
        // The first 6 characters describe capabilities (position 0 is video/audio/sub).
        // Video encoders have 'V' in the first position of the flags.
        if !trimmed.starts_with('V') {
            continue;
        }
        let mut tokens = trimmed.split_whitespace();
        // Skip the flag token (e.g. "V....." or "V....D")
        let Some(_flags) = tokens.next() else {
            continue;
        };
        if let Some(name) = tokens.next() {
            encoders.insert(name.to_owned());
        }
    }
    encoders
}

/// Arguments to probe an encoder with a short 256x256 test pattern (spec §9.3).
pub fn build_probe_args(def: &EncoderDefinition) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "color=c=black:s=256x256:d=0.2".into(),
        "-c:v".into(),
        def.id.into(),
    ];
    for extra in def.args {
        args.push((*extra).into());
    }
    args.push("-f".into());
    args.push("null".into());
    args.push("-".into());
    args
}

/// Cached probing result (spec §9.3: "cache by (ffmpeg path, mtime, version)").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderCache {
    pub ffmpeg_path: PathBuf,
    pub mtime: Option<SystemTime>,
    pub version: Option<String>,
    pub probed: Vec<EncoderInfo>,
}

impl EncoderCache {
    pub fn is_fresh(
        &self,
        current_path: &Path,
        current_mtime: Option<SystemTime>,
        current_version: Option<&str>,
    ) -> bool {
        self.ffmpeg_path == current_path
            && self.mtime == current_mtime
            && self.version.as_deref() == current_version
    }
}

/// Global cache holder for probed encoders.
#[derive(Default)]
pub struct EncodersState {
    cache: Mutex<Option<EncoderCache>>,
}

impl EncodersState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_probed(
        &self,
        path: &Path,
        mtime: Option<SystemTime>,
        version: Option<&str>,
    ) -> Option<Vec<EncoderInfo>> {
        let lock = self.cache.lock().ok()?;
        lock.as_ref().and_then(|c| {
            if c.is_fresh(path, mtime, version) {
                Some(c.probed.clone())
            } else {
                None
            }
        })
    }

    pub fn set_probed(&self, cache: EncoderCache) {
        if let Ok(mut lock) = self.cache.lock() {
            *lock = Some(cache);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut lock) = self.cache.lock() {
            *lock = None;
        }
    }
}

/// Probe a single encoder candidate using `ffmpeg`.
pub async fn probe_encoder(
    ffmpeg_path: &Path,
    def: &EncoderDefinition,
    token: CancellationToken,
) -> bool {
    let args = build_probe_args(def);
    let cmd = ToolCommand::new(ffmpeg_path)
        .args(args)
        .label(format!("ffmpeg-probe-{}", def.id));

    let probe_token = CancellationToken::new();
    let child_token = probe_token.clone();

    // Enforce 10s probe timeout per spec §9.3.
    let timeout_future = tokio::time::sleep(PROBE_TIMEOUT);
    tokio::pin!(timeout_future);

    let sinks = LineSinks::new(process::discard_lines(), process::discard_lines());
    let run_future = process::spawn_tool_with_sinks(cmd, child_token, sinks);

    tokio::select! {
        _ = token.cancelled() => {
            probe_token.cancel();
            false
        }
        _ = &mut timeout_future => {
            probe_token.cancel();
            false
        }
        res = run_future => {
            matches!(res, Ok(ProcessResult { exit_code: Some(0), .. }))
        }
    }
}

/// Query `ffmpeg -hide_banner -encoders` using `process::spawn_tool`.
pub async fn query_ffmpeg_encoders(
    ffmpeg_path: &Path,
    token: CancellationToken,
) -> Result<HashSet<String>, AppError> {
    let cmd = ToolCommand::new(ffmpeg_path)
        .args(["-hide_banner", "-nostdin", "-encoders"])
        .label("ffmpeg-list-encoders");

    let stdout_lines = Arc::new(Mutex::new(Vec::new()));
    let sink_lines = Arc::clone(&stdout_lines);
    let on_stdout = Arc::new(move |line: &str| {
        if let Ok(mut v) = sink_lines.lock() {
            v.push(line.to_owned());
        }
    });

    let result = process::spawn_tool(cmd, token, on_stdout).await?;
    if result.exit_code != Some(0) {
        return Err(AppError::Process(format!(
            "ffmpeg -encoders failed: {}",
            result.stderr_tail.join("\n")
        )));
    }

    let text = stdout_lines
        .lock()
        .map(|v| v.join("\n"))
        .unwrap_or_default();
    Ok(parse_supported_encoders(&text))
}

/// Probe all available encoders for the current OS and binary, respecting the cache.
pub async fn probe_all_encoders(
    ffmpeg_path: &Path,
    mtime: Option<SystemTime>,
    version: Option<&str>,
    state: &EncodersState,
    token: CancellationToken,
    force_refresh: bool,
) -> Result<Vec<EncoderInfo>, AppError> {
    if !force_refresh {
        if let Some(cached) = state.get_probed(ffmpeg_path, mtime, version) {
            return Ok(cached);
        }
    }

    let supported_names = query_ffmpeg_encoders(ffmpeg_path, token.clone()).await?;
    let os_candidates = candidates_for_os(CfgOs::current());

    let mut probed: Vec<EncoderInfo> = Vec::new();

    for id in os_candidates {
        if token.is_cancelled() {
            break;
        }
        if !supported_names.contains(*id) {
            continue;
        }
        let Some(def) = lookup_encoder(id) else {
            continue;
        };

        let ok = probe_encoder(ffmpeg_path, def, token.clone()).await;
        if ok {
            probed.push(EncoderInfo {
                id: def.id.to_owned(),
                label: def.label.to_owned(),
                codec: def.codec.to_owned(),
                is_hardware: def.is_hardware,
            });
        }
    }

    state.set_probed(EncoderCache {
        ffmpeg_path: ffmpeg_path.to_path_buf(),
        mtime,
        version: version.map(str::to_owned),
        probed: probed.clone(),
    });

    Ok(probed)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_codec_properties_match_spec() {
        assert_eq!(AudioCodec::Aac.extension(), "m4a");
        assert_eq!(AudioCodec::Mp3.extension(), "mp3");
        assert_eq!(AudioCodec::Opus.extension(), "opus");

        assert_eq!(AudioCodec::parse("AAC"), Some(AudioCodec::Aac));
        assert_eq!(AudioCodec::parse("m4a"), Some(AudioCodec::Aac));
        assert_eq!(AudioCodec::parse("mp3"), Some(AudioCodec::Mp3));
        assert_eq!(AudioCodec::parse("OPUS"), Some(AudioCodec::Opus));
        assert_eq!(AudioCodec::parse("flac"), None);

        assert_eq!(
            AudioCodec::Aac.ffmpeg_args(),
            &["-c:a", "aac", "-b:a", "192k"]
        );
        assert_eq!(
            AudioCodec::Mp3.ffmpeg_args(),
            &["-c:a", "libmp3lame", "-q:a", "2"]
        );
        assert_eq!(
            AudioCodec::Opus.ffmpeg_args(),
            &["-c:a", "libopus", "-b:a", "128k"]
        );
    }

    #[test]
    fn cpu_fallback_maps_to_libx264_and_libx265() {
        assert_eq!(cpu_fallback_encoder("h264_nvenc"), Some("libx264"));
        assert_eq!(cpu_fallback_encoder("h264_amf"), Some("libx264"));
        assert_eq!(cpu_fallback_encoder("h264_qsv"), Some("libx264"));
        assert_eq!(cpu_fallback_encoder("h264_vaapi"), Some("libx264"));
        assert_eq!(cpu_fallback_encoder("h264_videotoolbox"), Some("libx264"));

        assert_eq!(cpu_fallback_encoder("hevc_nvenc"), Some("libx265"));
        assert_eq!(cpu_fallback_encoder("hevc_amf"), Some("libx265"));
        assert_eq!(cpu_fallback_encoder("hevc_qsv"), Some("libx265"));
        assert_eq!(cpu_fallback_encoder("hevc_vaapi"), Some("libx265"));
        assert_eq!(cpu_fallback_encoder("hevc_videotoolbox"), Some("libx265"));

        // CPU encoders have no fallback
        assert_eq!(cpu_fallback_encoder("libx264"), None);
        assert_eq!(cpu_fallback_encoder("libx265"), None);
        assert_eq!(cpu_fallback_encoder("unknown_encoder"), None);
    }

    #[test]
    fn candidates_per_os_match_spec() {
        let linux = candidates_for_os(CfgOs::Linux);
        assert!(linux.contains(&"libx264"));
        assert!(linux.contains(&"libx265"));
        assert!(linux.contains(&"h264_nvenc"));
        assert!(linux.contains(&"h264_vaapi"));
        assert!(linux.contains(&"h264_qsv"));
        assert!(!linux.contains(&"h264_videotoolbox"));
        assert!(!linux.contains(&"h264_amf"));

        let windows = candidates_for_os(CfgOs::Windows);
        assert!(windows.contains(&"libx264"));
        assert!(windows.contains(&"h264_amf"));
        assert!(!windows.contains(&"h264_vaapi"));
        assert!(!windows.contains(&"h264_videotoolbox"));

        let macos = candidates_for_os(CfgOs::MacOs);
        assert!(macos.contains(&"libx264"));
        assert!(macos.contains(&"h264_videotoolbox"));
        assert!(!macos.contains(&"h264_nvenc"));
        assert!(!macos.contains(&"h264_vaapi"));
    }

    #[test]
    fn parses_ffmpeg_encoders_output() {
        let sample = "\
Encoders:
 V..... = Video
 A..... = Audio
 S..... = Subtitle
 ------
 V..... libx264              libx264 H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10 (codec h264)
 V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)
 V....D h264_vaapi           H.264/AVC (VAAPI) (codec h264)
 A....D aac                  AAC (Advanced Audio Coding)
 A....D libmp3lame           libmp3lame MP3 (MPEG audio layer 3) (codec mp3)
";
        let parsed = parse_supported_encoders(sample);
        assert!(parsed.contains("libx264"));
        assert!(parsed.contains("h264_nvenc"));
        assert!(parsed.contains("h264_vaapi"));
        // Audio encoders should not be in video encoder list
        assert!(!parsed.contains("aac"));
        assert!(!parsed.contains("libmp3lame"));
    }

    #[test]
    fn builds_probe_args_for_encoders() {
        let def = lookup_encoder("libx264").expect("libx264 exists");
        let args = build_probe_args(def);
        let args_str: Vec<String> = args
            .into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();

        assert_eq!(args_str[0], "-hide_banner");
        assert_eq!(args_str[1], "-nostdin");
        assert_eq!(args_str[2], "-f");
        assert_eq!(args_str[3], "lavfi");
        assert_eq!(args_str[4], "-i");
        assert_eq!(args_str[5], "color=c=black:s=256x256:d=0.2");
        assert_eq!(args_str[6], "-c:v");
        assert_eq!(args_str[7], "libx264");
        assert!(args_str.contains(&"-pix_fmt".to_string()));
        assert_eq!(args_str.last().map(|s| s.as_str()), Some("-"));
    }

    #[test]
    fn encoder_cache_freshness() {
        let path = PathBuf::from("/usr/bin/ffmpeg");
        let now = SystemTime::now();
        let cache = EncoderCache {
            ffmpeg_path: path.clone(),
            mtime: Some(now),
            version: Some("9.0.2".to_string()),
            probed: vec![],
        };

        assert!(cache.is_fresh(&path, Some(now), Some("9.0.2")));
        assert!(!cache.is_fresh(&PathBuf::from("/opt/ffmpeg"), Some(now), Some("9.0.2")));
        assert!(!cache.is_fresh(&path, None, Some("9.0.2")));
        assert!(!cache.is_fresh(&path, Some(now), Some("8.0.0")));
    }
}
