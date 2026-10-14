//! IPC-level tests for the settings commands.
//!
//! These run a real Tauri app on the `MockRuntime`, so they exercise the whole
//! `invoke` path (argument deserialisation, `State` lookup, error
//! serialisation) without opening a window and without touching the user's
//! real config directory.
//!
//! Note: `Builder::setup` does not run until the runtime emits `Ready`, which
//! never happens on `MockRuntime`, so the state is managed on the built app.

use std::path::{Path, PathBuf};

use scaricatore_di_porto_lib::state::{AppState, Settings};
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

/// Builds an app whose settings file lives inside `dir`.
fn app_with_settings_in(dir: &Path) -> tauri::App<MockRuntime> {
    let path = dir.join("settings.json");
    let state = AppState::from_path(path).expect("state loads");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            scaricatore_di_porto_lib::commands::settings::settings_get,
            scaricatore_di_porto_lib::commands::settings::settings_save,
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
fn settings_get_returns_the_defaults_on_a_fresh_install() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_with_settings_in(dir.path());
    let window = webview(&app);

    let value = call(&window, "settings_get", json!({})).expect("settings_get must succeed");
    assert_eq!(value["settings"]["language"], "system");
    assert_eq!(value["settings"]["theme"], "system");
    // Asserted against the constant, not a literal: the schema version changes
    // when the document does, and a hardcoded "1" here would only ever fail.
    assert_eq!(
        value["settings"]["schemaVersion"],
        scaricatore_di_porto_lib::state::SETTINGS_SCHEMA_VERSION
    );
    assert!(value["settings"]["defaultOutputDir"].is_null());
    assert!(value["recovery"].is_null());
    assert!(
        value["settingsPath"]
            .as_str()
            .expect("path")
            .ends_with("settings.json")
    );
}

#[test]
fn tool_settings_travel_through_the_ipc() {
    // The Tools page writes `toolPaths` and `ytdlpChannel`; if they were
    // dropped by serialisation the override would silently stop applying.
    let dir = TempDir::new().expect("temp dir");
    let app = app_with_settings_in(dir.path());
    let window = webview(&app);

    // The override has to be absolute *on this host*: the document is normalised
    // on the way in, and `/opt/mine/yt-dlp` is rooted-but-driveless, so on
    // Windows it is correctly treated as a relative path and dropped. That is
    // the behaviour the second half of this test relies on, arriving by
    // accident through the first half.
    let override_path = if cfg!(windows) {
        r"C:\opt\mine\yt-dlp"
    } else {
        "/opt/mine/yt-dlp"
    };
    let saved = call(
        &window,
        "settings_save",
        json!({
            "settings": {
                "language": "en",
                "theme": "dark",
                "toolPaths": { "yt-dlp": override_path },
                "ytdlpChannel": "nightly",
                "setupWizardCompleted": true
            }
        }),
    )
    .expect("settings_save must succeed");

    assert_eq!(saved["settings"]["toolPaths"]["yt-dlp"], override_path);
    assert_eq!(saved["settings"]["ytdlpChannel"], "nightly");
    assert_eq!(saved["settings"]["setupWizardCompleted"], true);

    // A relative override is dropped by normalisation instead of being stored
    // and refused later by the process layer (spec §4.8).
    let saved = call(
        &window,
        "settings_save",
        json!({
            "settings": {
                "toolPaths": { "ffmpeg": "relative/ffmpeg" }
            }
        }),
    )
    .expect("settings_save must succeed");
    assert!(
        saved["settings"]["toolPaths"]
            .as_object()
            .expect("object")
            .is_empty()
    );
}

#[test]
fn settings_survive_a_restart() {
    let dir = TempDir::new().expect("temp dir");
    let path: PathBuf = dir.path().join("settings.json");

    {
        let app = app_with_settings_in(dir.path());
        let window = webview(&app);
        let saved = call(
            &window,
            "settings_save",
            json!({
                "settings": {
                    "schemaVersion": 1,
                    "language": "it",
                    "theme": "dark",
                    "defaultOutputDir": "/tmp/out"
                }
            }),
        )
        .expect("settings_save must succeed");
        assert_eq!(saved["settings"]["language"], "it");
        assert_eq!(saved["settings"]["theme"], "dark");
    }

    // The document really is on disk…
    let on_disk: Settings =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("settings file"))
            .expect("parse");

    // …and a brand new app instance reads the same values back.
    let app = app_with_settings_in(dir.path());
    let window = webview(&app);
    let value = call(&window, "settings_get", json!({})).expect("settings_get must succeed");

    assert_eq!(value["settings"]["language"], "it");
    assert_eq!(value["settings"]["theme"], "dark");
    assert_eq!(value["settings"]["defaultOutputDir"], "/tmp/out");
    assert_eq!(
        on_disk.language,
        scaricatore_di_porto_lib::state::Language::It
    );
    assert_eq!(on_disk.theme, scaricatore_di_porto_lib::state::Theme::Dark);
}

#[test]
fn a_corrupt_file_is_reported_as_a_recovery_not_an_error() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("settings.json");
    std::fs::write(&path, b"{ broken json").expect("write");

    let app = app_with_settings_in(dir.path());
    let window = webview(&app);

    let value = call(&window, "settings_get", json!({})).expect("must not fail");
    assert_eq!(value["settings"]["theme"], "system");
    assert_eq!(value["recovery"]["code"], "SETTINGS_CORRUPT");
    assert_eq!(value["recovery"]["messageKey"], "error.settingsCorrupt");

    let backup = PathBuf::from(
        value["recovery"]["backupPath"]
            .as_str()
            .expect("backup path"),
    );
    assert!(backup.exists(), "the broken document must be kept");
    assert_eq!(
        std::fs::read_to_string(&backup).expect("read backup"),
        "{ broken json"
    );
}

#[test]
fn an_unknown_enum_value_does_not_break_the_document() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("settings.json");
    std::fs::write(
        &path,
        br#"{"schemaVersion":1,"language":"en","theme":"ultraviolet"}"#,
    )
    .expect("write");

    let app = app_with_settings_in(dir.path());
    let window = webview(&app);
    let value = call(&window, "settings_get", json!({})).expect("must not fail");

    assert_eq!(value["settings"]["theme"], "system", "degrades to default");
    assert_eq!(value["settings"]["language"], "en", "valid value kept");
    assert!(value["recovery"].is_null());
}

#[test]
fn an_unwritable_settings_file_reports_a_localised_error() {
    let dir = TempDir::new().expect("temp dir");

    // Load while the location is still writable…
    let state =
        AppState::from_path(dir.path().join("blocker").join("settings.json")).expect("state loads");

    // …then put a *file* where the config directory should be, so writing
    // fails on every OS (no permission tricks, which do not work the same on
    // Windows).
    std::fs::write(dir.path().join("blocker"), b"not a directory").expect("write");

    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            scaricatore_di_porto_lib::commands::settings::settings_save,
        ])
        .build(mock_context(noop_assets()))
        .expect("mock app must build");
    app.manage(state);
    let window = webview(&app);

    let err = call(
        &window,
        "settings_save",
        json!({
            "settings": {
                "schemaVersion": 1,
                "language": "en",
                "theme": "dark",
                "defaultOutputDir": null
            }
        }),
    )
    .expect_err("saving must fail");

    assert_eq!(err["code"], "IO");
    assert_eq!(err["messageKey"], "error.io");
    assert!(
        err["detail"].as_str().is_some_and(|d| !d.is_empty()),
        "the raw detail must stay available for 'Copy details'"
    );
}
