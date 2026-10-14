//! IPC-level tests for the M1 commands and settings fields.
//!
//! Same `MockRuntime` pattern as `settings_ipc.rs`: real argument
//! deserialisation, real `State` lookup, real error serialisation — no window,
//! no network, no real tool. The network behaviour of inspect/download is
//! covered by `ytdlp_job.rs` (fake tool) and `real_selftest.rs` (ignored, real
//! tool).

use std::path::{Path, PathBuf};

use scaricatore_di_porto_lib::state::AppState;
use serde_json::json;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{Manager, WebviewWindow};
use tempfile::TempDir;

fn request(cmd: &str, body: serde_json::Value) -> InvokeRequest {
    InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        }
        .parse()
        .expect("valid ipc url"),
        body: InvokeBody::Json(body),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.to_string(),
    }
}

fn app_with_settings_in(dir: &Path) -> tauri::App<MockRuntime> {
    let state = AppState::from_path(dir.join("settings.json")).expect("state loads");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            scaricatore_di_porto_lib::commands::settings::settings_get,
            scaricatore_di_porto_lib::commands::settings::settings_save,
            scaricatore_di_porto_lib::commands::inspect::inspect_url,
            scaricatore_di_porto_lib::commands::inspect::inspect_cancel,
            scaricatore_di_porto_lib::commands::download::download_start,
            scaricatore_di_porto_lib::commands::download::download_cancel,
            scaricatore_di_porto_lib::commands::download::download_dismiss_deno_offer,
            scaricatore_di_porto_lib::commands::tools::tools_cancel_install,
        ])
        .build(mock_context(noop_assets()))
        .expect("mock app must build");
    app.manage(state);
    app
}

fn webview(app: &tauri::App<MockRuntime>) -> WebviewWindow<MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("mock webview must build")
}

/// The `fake_ytdlp` example, copied under the names the lookup expects so the
/// real `resolve` finds it: `yt-dlp-<triple>`, `qjs-<triple>`, `ffmpeg-<triple>`
/// inside the context's bundled directory (a debug build looks for the triple
/// suffix, D39).
fn fake_tool_environment(root: &Path) -> scaricatore_di_porto_lib::state::ToolContext {
    let profile_dir = std::env::current_exe()
        .expect("test binary path")
        .parent()
        .and_then(Path::parent)
        .expect("<profile>/deps")
        .to_path_buf();
    let mut source = profile_dir.join("examples").join("fake_ytdlp");
    if cfg!(windows) {
        source.set_extension("exe");
    }
    assert!(
        source.is_file(),
        "run the suite as `cargo test`: {}",
        source.display()
    );

    let triple =
        scaricatore_di_porto_lib::services::manifest::current_target_triple().expect("triple");
    let bundled = root.join("bundled");
    std::fs::create_dir_all(&bundled).expect("bundled dir");
    for tool in ["yt-dlp", "qjs", "ffmpeg"] {
        let mut name = format!("{tool}-{triple}");
        if cfg!(windows) {
            name.push_str(".exe");
        }
        std::fs::copy(&source, bundled.join(name)).expect("copy the fake");
    }
    scaricatore_di_porto_lib::state::ToolContext::in_dir(root)
}

fn app_with_fakes_in(dir: &Path) -> tauri::App<MockRuntime> {
    let tools = fake_tool_environment(dir);
    let state =
        AppState::from_path_with_tools(dir.join("settings.json"), tools).expect("state loads");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            scaricatore_di_porto_lib::commands::inspect::inspect_url,
            scaricatore_di_porto_lib::commands::download::download_start,
            scaricatore_di_porto_lib::commands::download::download_cancel,
        ])
        .build(mock_context(noop_assets()))
        .expect("mock app must build");
    app.manage(state);
    app
}

fn call(
    webview: &WebviewWindow<MockRuntime>,
    cmd: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    get_ipc_response(webview, request(cmd, body)).map(|body| {
        body.deserialize::<serde_json::Value>()
            .expect("response must be JSON")
    })
}

#[test]
fn the_m1_settings_fields_travel_through_the_ipc() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_with_settings_in(dir.path());
    let window = webview(&app);

    let saved = call(
        &window,
        "settings_save",
        json!({
            "settings": {
                "jsRuntimeMode": "deno",
                "cookiesFromBrowser": "firefox",
                "cookiesFile": "/home/u/cookies.txt",
                "denoOfferDismissed": true
            }
        }),
    )
    .expect("settings_save must succeed");

    assert_eq!(saved["settings"]["jsRuntimeMode"], "deno");
    assert_eq!(saved["settings"]["cookiesFromBrowser"], "firefox");
    assert_eq!(saved["settings"]["cookiesFile"], "/home/u/cookies.txt");
    assert_eq!(saved["settings"]["denoOfferDismissed"], true);

    // Safari is not a browser on this OS (when this runs on Windows/Linux) and
    // the Rust normalisation drops it; on macOS it is kept. The frontend never
    // offers it elsewhere, so this is the backstop.
    let saved = call(
        &window,
        "settings_save",
        json!({ "settings": { "cookiesFromBrowser": "safari" } }),
    )
    .expect("settings_save must succeed");
    if cfg!(target_os = "macos") {
        assert_eq!(saved["settings"]["cookiesFromBrowser"], "safari");
    } else {
        assert!(saved["settings"]["cookiesFromBrowser"].is_null());
    }
}

#[test]
fn an_unknown_runtime_mode_degrades_to_auto_over_the_ipc() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_with_settings_in(dir.path());
    let window = webview(&app);

    let saved = call(
        &window,
        "settings_save",
        json!({ "settings": { "jsRuntimeMode": "bun" } }),
    )
    .expect("settings_save must succeed");
    assert_eq!(saved["settings"]["jsRuntimeMode"], "auto");
}

#[test]
fn inspect_refuses_a_non_http_url_before_resolving_anything() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_with_settings_in(dir.path());
    let window = webview(&app);

    // No tools are resolvable in this isolated state; the URL must be refused
    // first (spec §4.6), so the error is about the scheme and not about yt-dlp.
    let err = call(
        &window,
        "inspect_url",
        json!({ "requestId": 1, "url": "file:///etc/passwd" }),
    )
    .expect_err("must refuse");
    assert_eq!(err["code"], "UNSUPPORTED");
    assert!(
        err["detail"].as_str().is_some_and(|d| d.contains("http")),
        "{err}"
    );
}

#[test]
fn download_refuses_a_non_http_url_and_an_empty_folder_choice_is_not_a_path() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_with_settings_in(dir.path());
    let window = webview(&app);

    let err = call(
        &window,
        "download_start",
        json!({
            "url": "javascript:alert(1)",
            "quality": { "kind": "best" },
            "outputDir": null
        }),
    )
    .expect_err("must refuse");
    assert_eq!(err["code"], "UNSUPPORTED");
    assert!(err["detail"].as_str().is_some_and(|d| d.contains("http")));
}

#[test]
fn cancelling_an_unknown_job_is_false_not_an_error() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_with_settings_in(dir.path());
    let window = webview(&app);

    let value =
        call(&window, "download_cancel", json!({ "jobId": "job-0-0" })).expect("must not fail");
    assert_eq!(value, json!(false));

    // Same contract for the on-demand install (spec §5.2: progress + Cancel).
    let value = call(&window, "tools_cancel_install", json!({})).expect("must not fail");
    assert_eq!(value, json!(false));
}

#[test]
fn dismissing_the_deno_offer_is_persisted() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_with_settings_in(dir.path());
    let window = webview(&app);

    call(&window, "download_dismiss_deno_offer", json!({})).expect("must succeed");
    let value = call(&window, "settings_get", json!({})).expect("must succeed");
    assert_eq!(value["settings"]["denoOfferDismissed"], true);

    let path: PathBuf = dir.path().join("settings.json");
    let on_disk: scaricatore_di_porto_lib::state::Settings =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("settings file"))
            .expect("parse");
    assert!(on_disk.deno_offer_dismissed);
}

#[test]
fn inspect_and_download_work_through_the_real_commands_with_a_fake_tool() {
    let dir = TempDir::new().expect("temp dir");
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).expect("out dir");
    let app = app_with_fakes_in(dir.path());
    let window = webview(&app);

    // 1. Inspect: the fake answers `-J` with its canned video document.
    let view = call(
        &window,
        "inspect_url",
        json!({ "requestId": 7, "url": "https://youtu.be/good" }),
    )
    .expect("inspect must succeed");
    assert_eq!(view["requestId"], 7);
    assert_eq!(view["inspected"]["kind"], "video");
    assert_eq!(view["inspected"]["title"], "Fake title");
    assert_eq!(view["inspected"]["heights"], json!([1080, 720, 360]));

    // 2. Download: returns a job id immediately; the job then runs in the
    // background and writes into the chosen folder.
    let started = call(
        &window,
        "download_start",
        json!({
            "url": "https://youtu.be/good",
            "quality": { "kind": "exact", "height": 720 },
            "outputDir": out.display().to_string()
        }),
    )
    .expect("download_start must succeed");
    let job_id = started["jobId"].as_str().expect("job id").to_owned();
    assert_eq!(started["outputDir"], out.display().to_string());

    // 3. Wait for the job to finish. `job:done` is emitted to the webview the
    // mock does not have event plumbing for, so the state is the source of
    // truth here; the event payload itself is covered by `ytdlp_job.rs`.
    let state = app.state::<AppState>();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while state.job_running() {
        assert!(
            std::time::Instant::now() < deadline,
            "the job did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let produced: Vec<_> = std::fs::read_dir(&out)
        .expect("out dir")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        produced,
        vec!["Fake title [fakeid].mkv".to_owned()],
        "the file the fake reported via SDP_FILE must be in the folder"
    );

    // A cancelled/stale id is still just `false`.
    let value =
        call(&window, "download_cancel", json!({ "jobId": job_id })).expect("must not fail");
    assert_eq!(value, json!(false));
}
