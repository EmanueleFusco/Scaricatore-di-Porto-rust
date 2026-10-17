//! IPC-level tests for the M2 commands, through the real Tauri handlers.
//!
//! Same `MockRuntime` pattern as `m1_ipc.rs`: real argument deserialisation,
//! real `State` lookup, real error serialisation — no window, no network, no
//! real tool. The behaviour of the engine with a running process is in
//! `queue_engine.rs` (fake tool); this file covers what only exists here: the
//! **wire shapes** of `inspect_urls`, `queue_add` and `read_links_file`, and the
//! refusal paths that must never reach a process.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use scaricatore_di_porto_lib::services::queue::engine::{EngineConfig, QueueEngine};
use scaricatore_di_porto_lib::services::queue::events::QueueUpdate;
use scaricatore_di_porto_lib::services::queue::model::LaneConfig;
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

/// The fake yt-dlp copied under the names the lookup expects (same helper shape
/// as `m1_ipc.rs`, kept here so the two files do not share a module).
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

/// An app with the M1 and M2 commands, the fake tools and a queue that is
/// **not started** (the tests here never need a running download).
fn app_in(dir: &Path) -> tauri::App<MockRuntime> {
    let tools = fake_tool_environment(dir);
    let state =
        AppState::from_path_with_tools(dir.join("settings.json"), tools).expect("state loads");
    let engine = QueueEngine::open(EngineConfig {
        path: dir.join("queue.json"),
        provider: Arc::new(Unresolvable),
        sink: Arc::new(|_: &QueueUpdate| {}),
        lanes: LaneConfig::default(),
        notify_on_idle: false,
        notifier: None,
    });
    state.set_queue(engine);

    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            scaricatore_di_porto_lib::commands::inspect::inspect_urls,
            scaricatore_di_porto_lib::commands::queue::queue_list,
            scaricatore_di_porto_lib::commands::queue::queue_add,
            scaricatore_di_porto_lib::commands::queue::queue_cancel,
            scaricatore_di_porto_lib::commands::queue::queue_pause,
            scaricatore_di_porto_lib::commands::queue::queue_resume,
            scaricatore_di_porto_lib::commands::queue::queue_retry,
            scaricatore_di_porto_lib::commands::queue::queue_remove,
            scaricatore_di_porto_lib::commands::queue::queue_reorder,
            scaricatore_di_porto_lib::commands::queue::queue_move_to_top,
            scaricatore_di_porto_lib::commands::queue::queue_clear_finished,
            scaricatore_di_porto_lib::commands::queue::queue_lanes,
            scaricatore_di_porto_lib::commands::queue::queue_upsert_group,
            scaricatore_di_porto_lib::commands::queue::queue_pause_group,
            scaricatore_di_porto_lib::commands::queue::queue_resume_group,
            scaricatore_di_porto_lib::commands::queue::queue_retry_group_failed,
            scaricatore_di_porto_lib::commands::queue::queue_remove_group,
            scaricatore_di_porto_lib::commands::queue::read_links_file,
            scaricatore_di_porto_lib::commands::queue::app_set_language,
            scaricatore_di_porto_lib::commands::settings::settings_get,
            scaricatore_di_porto_lib::commands::settings::settings_save,
            scaricatore_di_porto_lib::commands::settings::archive_status,
            scaricatore_di_porto_lib::commands::settings::archive_clear,
        ])
        .build(mock_context(noop_assets()))
        .expect("mock app must build");
    app.manage(state);
    app
}

/// A provider that refuses everything: these tests never start a download, and a
/// provider that *could* would hide a mistake in the arguments.
struct Unresolvable;

impl scaricatore_di_porto_lib::services::queue::engine::ToolProvider for Unresolvable {
    fn environment(
        &self,
        _options: scaricatore_di_porto_lib::services::queue::model::ItemOptions,
    ) -> scaricatore_di_porto_lib::services::queue::engine::BoxFuture<
        Result<
            scaricatore_di_porto_lib::services::job::JobEnvironment,
            scaricatore_di_porto_lib::error::ErrorInfo,
        >,
    > {
        Box::pin(std::future::ready(Err(
            scaricatore_di_porto_lib::error::ErrorInfo::new(
                scaricatore_di_porto_lib::error::ErrorCode::Unsupported,
                "this test never runs a download",
            ),
        )))
    }
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

fn settings_file(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

// ---------------------------------------------------------------------------
// inspect_urls (spec §6.2: three in parallel, answers in order)
// ---------------------------------------------------------------------------

#[test]
fn inspect_urls_answers_one_row_per_url_in_the_order_it_was_sent() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    let answers = tauri::async_runtime::block_on(async {
        call(
            &window,
            "inspect_urls",
            json!({
                "requestId": 3,
                "urls": [
                    "https://youtu.be/first",
                    "https://youtu.be/second",
                    "file:///etc/passwd",
                    "https://youtu.be/third"
                ]
            }),
        )
    })
    .expect("inspect_urls must succeed");

    let rows = answers.as_array().expect("an array of rows");
    assert_eq!(rows.len(), 4, "one answer per URL, refusals included");
    // Order is the request order, whatever finished first.
    let urls: Vec<&str> = rows
        .iter()
        .map(|row| row["url"].as_str().expect("a url"))
        .collect();
    assert_eq!(
        urls,
        vec![
            "https://youtu.be/first",
            "https://youtu.be/second",
            "file:///etc/passwd",
            "https://youtu.be/third"
        ]
    );
    // Three of them are the canned video document.
    for row in rows
        .iter()
        .filter(|row| row["url"].as_str() != Some("file:///etc/passwd"))
    {
        assert_eq!(row["inspected"]["kind"], "video");
        assert!(row["error"].is_null());
    }
    // The refused one is a row with an error, not a failed batch: a paste of ten
    // links with one bad one must not lose the other nine (spec §2.2).
    let refused = &rows[2];
    assert!(refused["inspected"].is_null());
    assert_eq!(refused["error"]["code"], "UNSUPPORTED");
}

#[test]
fn inspect_urls_of_nothing_is_an_empty_list_not_an_error() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    let answers = tauri::async_runtime::block_on(async {
        call(
            &window,
            "inspect_urls",
            json!({ "requestId": 1, "urls": [] }),
        )
    })
    .expect("must succeed");
    assert_eq!(answers.as_array().map(Vec::len), Some(0));
}

// ---------------------------------------------------------------------------
// queue_add / queue_list (spec §7.3)
// ---------------------------------------------------------------------------

#[test]
fn queue_add_refuses_a_non_http_url_and_keeps_the_others() {
    let dir = TempDir::new().expect("temp dir");
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).expect("out dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    let result = tauri::async_runtime::block_on(async {
        call(
            &window,
            "queue_add",
            json!({
                "force": false,
                "items": [
                    { "url": "javascript:alert(1)", "kind": "video", "quality": { "kind": "best" } },
                    {
                        "url": "https://a.example/1",
                        "title": "One",
                        "site": "Example",
                        "durationSec": 12,
                        "kind": "video",
                        "quality": { "kind": "maxHeight", "height": 720 },
                        "outputDir": out.display().to_string(),
                        "customHeaders": null
                    }
                ]
            }),
        )
    })
    .expect("queue_add must succeed");

    assert_eq!(
        result["added"].as_array().map(Vec::len),
        Some(1),
        "the non-http(s) link is ignored, not fatal (spec §2.2)"
    );
    assert!(result["duplicates"].as_array().expect("array").is_empty());

    // And the item really landed, with the options the request carried.
    let snapshot = call(&window, "queue_list", json!({})).expect("queue_list must succeed");
    let item = &snapshot["items"][0];
    assert_eq!(item["url"], "https://a.example/1");
    assert_eq!(item["title"], "One");
    assert_eq!(item["site"], "Example");
    assert_eq!(item["durationSec"], 12);
    assert_eq!(item["state"], "queued");
    assert_eq!(item["options"]["quality"]["kind"], "maxHeight");
    assert_eq!(item["options"]["quality"]["height"], 720);
    assert_eq!(item["options"]["kind"], "video");
    assert_eq!(item["options"]["outputDir"], out.display().to_string());
    assert_eq!(snapshot["summary"]["queued"], 1);
}

#[test]
fn queue_add_reports_duplicates_and_the_second_call_forces_them() {
    let dir = TempDir::new().expect("temp dir");
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).expect("out dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    let item = json!({
        "url": "https://www.a.example/1",
        "kind": "video",
        "quality": { "kind": "best" },
        "outputDir": out.display().to_string()
    });
    let first = tauri::async_runtime::block_on(async {
        call(
            &window,
            "queue_add",
            json!({ "force": false, "items": [item] }),
        )
    })
    .expect("must succeed");
    assert_eq!(first["added"].as_array().map(Vec::len), Some(1));
    let first_id = first["added"][0].as_str().expect("an id").to_owned();

    // Same resource, different spelling: a duplicate, with the id that holds it.
    let again = tauri::async_runtime::block_on(async {
        call(
            &window,
            "queue_add",
            json!({
                "force": false,
                "items": [{
                    "url": "https://a.example/1",
                    "kind": "video",
                    "quality": { "kind": "best" },
                    "outputDir": out.display().to_string()
                }]
            }),
        )
    })
    .expect("must succeed");
    assert!(again["added"].as_array().expect("array").is_empty());
    assert_eq!(again["duplicates"][0]["itemId"], first_id);
    assert_eq!(again["duplicates"][0]["url"], "https://a.example/1");

    // "Add anyway" is the same call with `force`.
    let forced = tauri::async_runtime::block_on(async {
        call(
            &window,
            "queue_add",
            json!({
                "force": true,
                "items": [{
                    "url": "https://a.example/1",
                    "kind": "video",
                    "quality": { "kind": "best" },
                    "outputDir": out.display().to_string()
                }]
            }),
        )
    })
    .expect("must succeed");
    assert_eq!(forced["added"].as_array().map(Vec::len), Some(1));
    let snapshot = call(&window, "queue_list", json!({})).expect("queue_list");
    assert_eq!(snapshot["items"].as_array().map(Vec::len), Some(2));
}

#[test]
fn an_audio_only_quality_wins_over_the_kind_the_frontend_sent() {
    let dir = TempDir::new().expect("temp dir");
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).expect("out dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    // A row that says `kind: video` with an audio quality is a frontend bug; the
    // audio selector is what must win, or yt-dlp would be asked for a video
    // selector on an audio-only request (spec §8.2).
    let result = tauri::async_runtime::block_on(async {
        call(
            &window,
            "queue_add",
            json!({
                "force": false,
                "items": [{
                    "url": "https://a.example/song",
                    "kind": "video",
                    "quality": { "kind": "audioOnly" },
                    "outputDir": out.display().to_string()
                }]
            }),
        )
    })
    .expect("must succeed");
    assert_eq!(result["added"].as_array().map(Vec::len), Some(1));

    let snapshot = call(&window, "queue_list", json!({})).expect("queue_list");
    assert_eq!(snapshot["items"][0]["options"]["kind"], "audio");
    assert_eq!(
        snapshot["items"][0]["options"]["quality"]["kind"],
        "audioOnly"
    );
    assert_eq!(snapshot["items"][0]["options"]["audioCodec"], "aac");
}

#[test]
fn the_actions_of_an_unknown_id_are_false_not_an_error() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    for cmd in [
        "queue_cancel",
        "queue_pause",
        "queue_resume",
        "queue_retry",
        "queue_remove",
        "queue_move_to_top",
    ] {
        let value = call(&window, cmd, json!({ "id": "nope" })).expect("must not fail");
        assert_eq!(value, json!(false), "{cmd} on an unknown id");
    }
    // The bulk ones answer with what they did.
    assert_eq!(
        call(&window, "queue_clear_finished", json!({})).expect("must not fail"),
        json!([])
    );
}

#[test]
fn reorder_and_move_to_top_change_the_order_over_the_wire() {
    let dir = TempDir::new().expect("temp dir");
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).expect("out dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    let added = tauri::async_runtime::block_on(async {
        call(
            &window,
            "queue_add",
            json!({
                "force": false,
                "items": [
                    { "url": "https://a.example/1", "quality": { "kind": "best" }, "outputDir": out.display().to_string() },
                    { "url": "https://a.example/2", "quality": { "kind": "best" }, "outputDir": out.display().to_string() },
                    { "url": "https://a.example/3", "quality": { "kind": "best" }, "outputDir": out.display().to_string() }
                ]
            }),
        )
    })
    .expect("must succeed");
    let ids: Vec<String> = added["added"]
        .as_array()
        .expect("array")
        .iter()
        .map(|id| id.as_str().expect("an id").to_owned())
        .collect();
    assert_eq!(ids.len(), 3);

    call(
        &window,
        "queue_reorder",
        json!({ "ids": [ids[2], ids[1], ids[0]] }),
    )
    .expect("queue_reorder must succeed");
    let snapshot = call(&window, "queue_list", json!({})).expect("queue_list");
    let order: Vec<&str> = snapshot["items"]
        .as_array()
        .expect("array")
        .iter()
        .map(|item| item["id"].as_str().expect("an id"))
        .collect();
    assert_eq!(
        order,
        vec![ids[2].as_str(), ids[1].as_str(), ids[0].as_str()]
    );

    assert_eq!(
        call(&window, "queue_move_to_top", json!({ "id": ids[0] })).expect("must not fail"),
        json!(true)
    );
    let snapshot = call(&window, "queue_list", json!({})).expect("queue_list");
    assert_eq!(snapshot["items"][0]["id"], ids[0].as_str());
}

#[test]
fn the_lane_settings_travel_to_the_frontend() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    let lanes = call(&window, "queue_lanes", json!({})).expect("must succeed");
    assert_eq!(lanes["parallelDownloads"], 2, "the spec §7.2 default");
    assert_eq!(lanes["perHostCap"], 2, "the spec §6.4 default");
}

// ---------------------------------------------------------------------------
// read_links_file (spec §2.2: drop a .txt of links)
// ---------------------------------------------------------------------------

#[test]
fn a_dropped_text_file_is_read_back_as_text() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    let links = dir.path().join("links.txt");
    std::fs::write(&links, "https://a.example/1\nhttps://b.example/2\n").expect("write");
    let raw = call(
        &window,
        "read_links_file",
        json!({ "path": links.display().to_string() }),
    )
    .expect("must succeed");
    assert_eq!(raw, json!("https://a.example/1\nhttps://b.example/2\n"));
}

#[test]
fn a_dropped_file_that_cannot_be_read_says_why() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    let err = call(
        &window,
        "read_links_file",
        json!({ "path": dir.path().join("nope.txt").display().to_string() }),
    )
    .expect_err("must fail");
    assert_eq!(err["code"], "IO");
    assert!(err["messageKey"].is_string());
}

// ---------------------------------------------------------------------------
// app_set_language (spec §11.9 → §7.7)
// ---------------------------------------------------------------------------

#[test]
fn the_webview_can_tell_the_backend_which_language_is_in_effect() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    call(&window, "app_set_language", json!({ "lang": "en" })).expect("must succeed");
    assert_eq!(app.state::<AppState>().ui_lang().get().as_str(), "en");
    call(&window, "app_set_language", json!({ "lang": "it" })).expect("must succeed");
    assert_eq!(app.state::<AppState>().ui_lang().get().as_str(), "it");
    // Nonsense falls back to Italian rather than to an empty notification.
    call(&window, "app_set_language", json!({ "lang": "klingon" })).expect("must succeed");
    assert_eq!(app.state::<AppState>().ui_lang().get().as_str(), "it");
}

// ---------------------------------------------------------------------------
// The settings that feed the queue (spec §7.2, §6.4, §7.7, §11.3)
// ---------------------------------------------------------------------------

#[test]
fn the_queue_settings_travel_through_the_settings_ipc_and_are_clamped() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    let saved = call(&window, "queue_lanes", json!({})).expect("must succeed");
    assert_eq!(saved["parallelDownloads"], 2);

    // The four fields of M2, the way the frontend sends them: the whole
    // document, read from `settings_get`, changed, saved with `settings_save`.
    let view = call(&window, "settings_get", json!({})).expect("must succeed");
    let mut document = view["settings"].clone();
    assert_eq!(document["parallelDownloads"], 2, "the spec §7.2 default");

    document["parallelDownloads"] = json!(3);
    document["perHostCap"] = json!(4);
    document["notifyOnIdle"] = json!(true);
    document["pasteLinksAnywhere"] = json!(true);
    let answer =
        call(&window, "settings_save", json!({ "settings": document })).expect("must succeed");
    assert_eq!(answer["settings"]["parallelDownloads"], 3);
    assert_eq!(answer["settings"]["perHostCap"], 4);
    assert_eq!(answer["settings"]["notifyOnIdle"], true);
    assert_eq!(answer["settings"]["pasteLinksAnywhere"], true);
    assert_eq!(
        answer["settings"]["schemaVersion"],
        scaricatore_di_porto_lib::state::SETTINGS_SCHEMA_VERSION,
        "current schema"
    );

    // And the engine was told, without a restart.
    let lanes = call(&window, "queue_lanes", json!({})).expect("must succeed");
    assert_eq!(lanes["parallelDownloads"], 3);
    assert_eq!(lanes["perHostCap"], 4);

    // Out of range is clamped on the way in, so the document can never hold a
    // value the scheduler would refuse (spec §7.2) — and what comes back is
    // what was written, so the UI shows the effective value.
    let mut document =
        call(&window, "settings_get", json!({})).expect("must succeed")["settings"].clone();
    document["parallelDownloads"] = json!(99);
    document["perHostCap"] = json!(0);
    let answer =
        call(&window, "settings_save", json!({ "settings": document })).expect("must succeed");
    assert_eq!(answer["settings"]["parallelDownloads"], 4);
    assert_eq!(answer["settings"]["perHostCap"], 1);

    // The same document is on disk: a restart reads what the engine is using.
    let on_disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(settings_file(dir.path())).expect("read"))
            .expect("settings.json is JSON");
    assert_eq!(on_disk["parallelDownloads"], 4);
    assert_eq!(on_disk["perHostCap"], 1);
    assert_eq!(
        on_disk["schemaVersion"],
        scaricatore_di_porto_lib::state::SETTINGS_SCHEMA_VERSION
    );
}

#[test]
fn archive_and_group_commands_work_over_the_ipc() {
    let dir = TempDir::new().expect("temp dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    let status = call(&window, "archive_status", json!({})).expect("archive_status");
    assert_eq!(status["entries"], 0);
    let archive_path = status["path"].as_str().expect("archive path");
    std::fs::create_dir_all(std::path::Path::new(archive_path).parent().unwrap()).expect("mkdir");
    std::fs::write(archive_path, "youtube a\nyoutube b\n").expect("write archive");

    let status = call(&window, "archive_status", json!({})).expect("archive_status");
    assert_eq!(status["entries"], 2);

    let cleared = call(&window, "archive_clear", json!({})).expect("archive_clear");
    assert_eq!(cleared["entries"], 0);

    // Upsert a group and verify it appears in queue_list.
    call(
        &window,
        "queue_upsert_group",
        json!({
            "group": {
                "id": "g-pl",
                "title": "My Playlist",
                "folder": "/tmp/My Playlist",
                "index": 0
            }
        }),
    )
    .expect("queue_upsert_group");
    let list = call(&window, "queue_list", json!({})).expect("queue_list");
    assert_eq!(list["groups"].as_array().expect("groups").len(), 1);
    assert_eq!(list["groups"][0]["title"], "My Playlist");

    let removed = call(&window, "queue_remove_group", json!({ "groupId": "g-pl" }))
        .expect("queue_remove_group");
    assert_eq!(removed, json!([]));
    let list = call(&window, "queue_list", json!({})).expect("queue_list");
    assert_eq!(list["groups"].as_array().expect("groups").len(), 0);
}

#[test]
fn queue_add_treats_empty_sponsorblock_categories_or_off_as_disabled() {
    let dir = TempDir::new().expect("temp dir");
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).expect("out dir");
    let app = app_in(dir.path());
    let window = webview(&app);

    let _ = call(
        &window,
        "queue_add",
        json!({
            "force": false,
            "items": [
                {
                    "url": "https://a.example/empty-cats",
                    "kind": "video",
                    "quality": { "kind": "best" },
                    "outputDir": out.display().to_string(),
                    "sponsorblockMode": "remove",
                    "sponsorblockCategories": []
                },
                {
                    "url": "https://a.example/mode-off",
                    "kind": "video",
                    "quality": { "kind": "best" },
                    "outputDir": out.display().to_string(),
                    "sponsorblockMode": "off",
                    "sponsorblockCategories": ["sponsor"]
                },
                {
                    "url": "https://a.example/mode-mark",
                    "kind": "video",
                    "quality": { "kind": "best" },
                    "outputDir": out.display().to_string(),
                    "sponsorblockMode": "mark",
                    "sponsorblockCategories": ["sponsor", "intro"]
                }
            ]
        }),
    )
    .expect("queue_add must succeed");

    let list = call(&window, "queue_list", json!({})).expect("queue_list");
    let items = list["items"].as_array().expect("items array");

    let item0 = items
        .iter()
        .find(|i| i["url"] == "https://a.example/empty-cats")
        .unwrap();
    assert_eq!(
        item0["options"]["sponsorblockMode"],
        serde_json::Value::Null
    );

    let item1 = items
        .iter()
        .find(|i| i["url"] == "https://a.example/mode-off")
        .unwrap();
    assert_eq!(
        item1["options"]["sponsorblockMode"],
        serde_json::Value::Null
    );

    let item2 = items
        .iter()
        .find(|i| i["url"] == "https://a.example/mode-mark")
        .unwrap();
    assert_eq!(item2["options"]["sponsorblockMode"], "mark");
    assert_eq!(
        item2["options"]["sponsorblockCategories"],
        json!(["sponsor", "intro"])
    );
}
