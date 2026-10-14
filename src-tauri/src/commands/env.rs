//! Tool resolution shared by the inspect, download and self-test commands.
//!
//! The commands themselves stay thin (spec §3); this is the one place that walks
//! the lookup order of spec §5.2 for a whole job and turns the result into the
//! `Runtimes` strategy of spec §5.3.

use std::path::PathBuf;

use crate::error::{AppResult, ErrorCode, ErrorInfo};
use crate::services::inspect::Runtimes;
use crate::services::manifest;
use crate::services::tools::{self, ResolvedTool};
use crate::state::{AppState, Settings};

/// The tools one job needs, already validated by running `--version`.
pub struct JobTools {
    pub ytdlp: ResolvedTool,
    /// `None` only when ffmpeg is not installed anywhere; the job then fails
    /// with `FFMPEG_MISSING` before spawning anything.
    pub ffmpeg: Option<PathBuf>,
    pub runtimes: Runtimes,
    /// Spec §4.9: the private TMP dir handed to children.
    pub private_temp: Option<PathBuf>,
}

/// Resolve yt-dlp, qjs, Deno and ffmpeg for the current settings.
///
/// yt-dlp is mandatory: without it there is no inspection and no download, and
/// the error says exactly that. The other three are optional at this level.
pub async fn resolve_job_tools(state: &AppState, settings: &Settings) -> AppResult<JobTools> {
    let context = state.tools();
    let triple = context.triple();
    let temp = context.temp_dir();

    let lookup = context.lookup(settings, manifest::YTDLP);
    let ytdlp = tools::resolve(manifest::YTDLP, &lookup, triple, temp)
        .await
        .ok_or_else(|| not_found(manifest::YTDLP))?;

    let qjs = tools::resolve(
        manifest::QJS,
        &context.lookup(settings, manifest::QJS),
        triple,
        temp,
    )
    .await;
    let deno = tools::resolve(
        manifest::DENO,
        &context.lookup(settings, manifest::DENO),
        triple,
        temp,
    )
    .await;
    let ffmpeg = tools::resolve(
        tools::FFMPEG,
        &context.lookup(settings, tools::FFMPEG),
        triple,
        temp,
    )
    .await;

    let runtimes = Runtimes {
        mode: settings.js_runtime_mode,
        qjs: qjs.map(|tool| crate::services::inspect::ResolvedRuntime {
            path: tool.path,
            version: tool.version,
        }),
        deno: deno.map(|tool| crate::services::inspect::ResolvedRuntime {
            path: tool.path,
            version: tool.version,
        }),
        ytdlp_version: ytdlp.version.clone().unwrap_or_default(),
    };

    Ok(JobTools {
        ytdlp,
        ffmpeg: ffmpeg.map(|tool| tool.path),
        runtimes,
        private_temp: context.temp_dir().map(std::path::Path::to_path_buf),
    })
}

fn not_found(tool: &str) -> ErrorInfo {
    ErrorInfo::new(
        ErrorCode::Unsupported,
        format!(
            "{tool} was not found anywhere the app looks (spec §5.2): install it, or point \
             the app at your own copy from Settings → Tools"
        ),
    )
}
