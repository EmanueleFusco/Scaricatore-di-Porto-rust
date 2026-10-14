//! Thin IPC handlers for the tools layer (spec §3: `commands/` has no logic —
//! validate, call the service, return).

use std::path::PathBuf;

use serde::Serialize;
use tauri::{Emitter, State};

use crate::error::{AppResult, ErrorCode, ErrorInfo};
use crate::services::inspect::SelfTestReport;
use crate::services::manifest;
use crate::services::tools::{self, ToolSource, ToolStatus, UpdateOutcome};
use crate::state::{AppState, IntoAppResult, YtdlpChannel};

/// Everything the Tools page and the wizard need in one round-trip.
///
/// One command instead of five: the page shows six rows and each row needs the
/// status *and* the paths for "Copy details", and six IPC calls per render is
/// six chances to show a half-updated table.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsView {
    pub tools: Vec<ToolStatus>,
    /// The Rust target triple of this build, which is also the key used in
    /// `tools.manifest.json`.
    pub triple: String,
    /// Where downloaded and updated tools live.
    pub tools_dir: String,
    /// Where the bundled baseline was found.
    pub bundled_dir: String,
    /// The private temp dir handed to every tool (spec §4.9).
    pub temp_dir: Option<String>,
    /// `true` once the Setup wizard has been completed (spec §11.1).
    pub setup_wizard_completed: bool,
}

/// Every tool the app knows about: the ones in the manifest, plus the ones it
/// only detects (`ffmpeg`, `ffprobe` — spec §5 prefers the system install).
fn tool_names(context: &crate::state::ToolContext) -> Vec<String> {
    let mut names: Vec<String> = context
        .manifest()
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    for extra in tools::extra_tools() {
        names.push((*extra).to_owned());
    }
    names
}

/// Resolve every tool and report on it.
#[tauri::command]
pub async fn tools_status(state: State<'_, AppState>) -> AppResult<ToolsView> {
    let context = state.tools();
    let settings = state.settings();
    let mut rows = Vec::new();
    for name in tool_names(context) {
        let lookup = context.lookup(&settings, &name);
        rows.push(
            tools::status_of(
                context.manifest(),
                &name,
                &lookup,
                context.triple(),
                context.temp_dir(),
            )
            .await,
        );
    }
    Ok(ToolsView {
        tools: rows,
        triple: context.triple().to_owned(),
        tools_dir: context.tools_dir().display().to_string(),
        bundled_dir: context.bundled_dir().display().to_string(),
        temp_dir: context.temp_dir().map(|dir| dir.display().to_string()),
        setup_wizard_completed: settings.setup_wizard_completed,
    })
}

/// Status of one tool, for the "Test" button (spec §11.8).
#[tauri::command]
pub async fn tools_test(state: State<'_, AppState>, tool: String) -> AppResult<ToolStatus> {
    let context = state.tools();
    let settings = state.settings();
    let lookup = context.lookup(&settings, &tool);
    Ok(tools::status_of(
        context.manifest(),
        &tool,
        &lookup,
        context.triple(),
        context.temp_dir(),
    )
    .await)
}

/// Point a tool at a path the user chose, or clear the override with `null`.
///
/// Refuses an empty tool name and a relative path before touching the document:
/// a relative path is rejected by the process layer (spec §4.8) and would only
/// turn into a confusing "cannot find" later.
#[tauri::command]
pub fn tools_set_path(
    state: State<'_, AppState>,
    tool: String,
    path: Option<String>,
) -> AppResult<()> {
    let name = tool.trim();
    if name.is_empty() {
        return Err(refuse("tools_set_path: the tool name is empty"));
    }
    let mut settings = state.settings();
    match path {
        Some(raw) if !raw.trim().is_empty() => {
            let chosen = PathBuf::from(&raw);
            if !chosen.is_absolute() {
                return Err(refuse(format!(
                    "tools_set_path: {raw:?} is not an absolute path (spec §4.8)"
                )));
            }
            settings.tool_paths.insert(name.to_owned(), chosen);
        }
        _ => {
            settings.tool_paths.remove(name);
        }
    }
    state.save_settings(settings).app_result()?;
    Ok(())
}

/// Update yt-dlp (spec §5.5).
///
/// Only ever touches the **app-data copy**. Running `-U` on a binary the user
/// installed themselves — a Homebrew bottle, a package manager's file — would
/// be something we have no business doing, so those sources report back that
/// there is nothing to update instead.
#[tauri::command]
pub async fn tools_update_ytdlp(state: State<'_, AppState>) -> AppResult<UpdateOutcome> {
    let context = state.tools();
    let settings = state.settings();
    let lookup = context.lookup(&settings, manifest::YTDLP);
    let temp_dir = context.temp_dir();

    let resolved = tools::resolve(manifest::YTDLP, &lookup, context.triple(), temp_dir)
        .await
        .ok_or_else(|| {
            refuse(format!(
                "yt-dlp was not found in any of the {} locations of spec §5.2",
                lookup_candidates(&lookup, context.triple())
            ))
        })?;

    let target = match resolved.source {
        // Exactly what the updater is for.
        ToolSource::AppData => resolved.path,
        ToolSource::Bundled => {
            // The installer directory is read-only, so the update has to happen
            // on the writable copy (spec §5.2.1). Make it first if this is the
            // very first launch.
            tools::install_app_data_copy(&resolved.path, context.tools_dir(), context.triple())
                .app_result()?
        }
        other => {
            return Ok(UpdateOutcome {
                updated: false,
                previous_version: resolved.version.clone(),
                version: resolved.version,
                message: format!(
                    "yt-dlp comes from {}; update it with whatever installed it there",
                    other.as_str()
                ),
            });
        }
    };

    tools::update_ytdlp(&target, settings.ytdlp_channel, temp_dir)
        .await
        .app_result()
}

/// Choose the yt-dlp release channel (spec §5.5, §11.8).
#[tauri::command]
pub fn tools_set_ytdlp_channel(state: State<'_, AppState>, channel: String) -> AppResult<()> {
    let parsed = match channel.trim().to_ascii_lowercase().as_str() {
        "stable" => YtdlpChannel::Stable,
        "nightly" => YtdlpChannel::Nightly,
        other => {
            return Err(refuse(format!(
                "tools_set_ytdlp_channel: unknown channel {other:?}"
            )));
        }
    };
    let mut settings = state.settings();
    settings.ytdlp_channel = parsed;
    state.save_settings(settings).app_result()?;
    Ok(())
}

/// Mark the Setup wizard as done (spec §11.1: "Shown once, re-openable from
/// Settings → Tools").
#[tauri::command]
pub fn tools_complete_setup(state: State<'_, AppState>) -> AppResult<()> {
    let mut settings = state.settings();
    settings.setup_wizard_completed = true;
    state.save_settings(settings).app_result()?;
    Ok(())
}

/// Install a tool on demand (spec §5.2, §5.2.1, §5.3.2).
///
/// Deno is the one installable tool of M1: it is never bundled, and it is
/// downloaded only when the user presses the button after a fallback needed it.
/// ffmpeg stays "later" for the same reason as in M0-C: it is an M4 decision
/// with licence and size consequences, and a button that silently does nothing
/// would be worse than the words.
#[tauri::command]
pub async fn tools_install<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    tool: String,
) -> AppResult<ToolStatus> {
    let name = tool.trim().to_ascii_lowercase();
    if name != manifest::DENO {
        return Err(refuse(format!(
            "{name} cannot be installed by the app yet: Deno is the only on-demand \
             install of M1; ffmpeg, the Whisper sidecar and the models come with \
             later phases."
        )));
    }

    let context = state.tools();
    let token = state.start_install();
    let events = app.clone();
    let tool_name = name.clone();
    // `tool:progress` (spec §3): the download callback is synchronous, so the
    // event is emitted from the same thread that is reading the body.
    let on_progress = move |progress: crate::services::download::DownloadProgress| {
        let _ = events.emit(
            "tool:progress",
            ToolProgressEvent {
                tool: tool_name.clone(),
                received: progress.received,
                total: progress.total,
            },
        );
    };

    let installed = tools::install_deno(
        context.manifest(),
        context.triple(),
        context.tools_dir(),
        &token,
        on_progress,
    )
    .await;
    state.finish_install();
    installed.map_err(|err| err.to_error_info())?;

    // Report the row as the Tools page would show it after a refresh.
    let settings = state.settings();
    let lookup = context.lookup(&settings, &name);
    Ok(tools::status_of(
        context.manifest(),
        &name,
        &lookup,
        context.triple(),
        context.temp_dir(),
    )
    .await)
}

/// Cancel the install in flight (spec §5.2: progress **and** Cancel).
#[tauri::command]
pub fn tools_cancel_install(state: State<'_, AppState>) -> bool {
    state.cancel_install()
}

/// The `tool:progress` payload (spec §3).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolProgressEvent {
    pub tool: String,
    pub received: u64,
    pub total: Option<u64>,
}

/// "Test YouTube extraction" (spec §5.3.6): a known public video through
/// `-J`, reporting the runtime used and the elapsed time.
#[tauri::command]
pub async fn tools_self_test(state: State<'_, AppState>) -> AppResult<SelfTestReport> {
    let settings = state.settings();
    let tools = crate::commands::env::resolve_job_tools(&state, &settings).await?;
    let environment = crate::services::inspect::InspectEnvironment {
        ytdlp: tools.ytdlp.path,
        cookies: crate::services::ytdlp::cookie_source(
            settings.cookies_from_browser.as_deref(),
            settings.cookies_file.as_deref(),
            crate::services::ytdlp::CfgOs::current(),
        ),
        runtimes: tools.runtimes,
        memory: state.runtime_memory().clone(),
        private_temp: tools.private_temp,
        youtube_clients: settings.youtube_player_clients.clone(),
    };
    Ok(crate::services::inspect::self_test(
        &environment,
        tokio_util::sync::CancellationToken::new(),
    )
    .await)
}

fn lookup_candidates(lookup: &tools::Lookup, triple: &str) -> usize {
    tools::candidates(manifest::YTDLP, lookup, triple).len()
}

/// First-launch copy of the bundled yt-dlp (spec §5.2.1), run at startup.
///
/// Never fatal: if the copy cannot be made the app still runs on the bundled
/// binary, which is exactly the fallback the spec asks for. Errors are logged
/// with no secrets — paths and OS error strings only.
pub fn bootstrap_first_run(context: &crate::state::ToolContext) -> Option<PathBuf> {
    if context.triple().is_empty() {
        return None;
    }
    let bundled = context
        .bundled_dir()
        .join(tools::executable_name(manifest::YTDLP, context.triple()));
    if !bundled.is_file() {
        // Development build without a fetch, or a store build: not an error.
        // Logged, because "the Tools page says yt-dlp is missing" is otherwise
        // impossible to explain from the outside.
        eprintln!(
            "scaricatore-di-porto: no bundled yt-dlp at {} (target {})",
            bundled.display(),
            context.triple()
        );
        return None;
    }
    match tools::install_app_data_copy(&bundled, context.tools_dir(), context.triple()) {
        Ok(path) => {
            eprintln!(
                "scaricatore-di-porto: yt-dlp installed in the app data dir at {}",
                path.display()
            );
            Some(path)
        }
        Err(err) => {
            eprintln!(
                "scaricatore-di-porto: the app-data copy of yt-dlp could not be created \
                 ({}); the bundled binary will be used",
                err
            );
            None
        }
    }
}

/// Turn a service error into the wire shape every command returns.
fn refuse(detail: impl Into<String>) -> ErrorInfo {
    ErrorInfo::new(ErrorCode::Unsupported, detail)
}
