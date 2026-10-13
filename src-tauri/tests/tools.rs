//! Integration tests for the tools layer (spec §5.2, §5.2.1, §5.5).
//!
//! Offline, on all three OSes, with no real tool involved: `fake_tool` is
//! copied to the file name the lookup looks for (`yt-dlp`, `qjs`, `ffmpeg`,
//! with `.exe` on Windows) and, because it dispatches on `argv[0]`, it behaves
//! like that program. That is what makes "the app-data copy wins over the
//! bundled one" a statement about the lookup rather than about whatever yt-dlp
//! happens to be installed on the machine running the tests.
//!
//! The path layout is always **explicit**: every directory is inside the test's
//! temp dir, and `PATH` is never consulted, because a lookup test that can
//! silently find the developer's own yt-dlp proves nothing.

use std::fs;
use std::path::{Path, PathBuf};

use scaricatore_di_porto_lib::services::manifest::{self, ToolsManifest};
use scaricatore_di_porto_lib::services::tools::{self, Lookup, ToolSource};
use scaricatore_di_porto_lib::state::Settings;
use tempfile::TempDir;

/// The `fake_tool` example binary that `cargo test` builds next to the tests.
///
/// Note for anyone running a single test target: `cargo test --test tools` does
/// **not** build examples, so run plain `cargo test` (what CI does) or
/// `cargo build --examples` first. The panic below says so, because a missing
/// helper otherwise looks like a broken test.
fn fake_tool_binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("current exe");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    let binary = path.join("examples").join(if cfg!(windows) {
        "fake_tool.exe"
    } else {
        "fake_tool"
    });
    assert!(
        binary.is_file(),
        "the fake tool helper is missing at {}.\n\
         Run `cargo test` (which builds examples) or `cargo build --examples` first.",
        binary.display()
    );
    binary
}

fn tool_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_owned()
    }
}

/// Copy the fake tool to `<dir>/<name>` so it impersonates that program.
fn install_fake(dir: &Path, name: &str) -> PathBuf {
    fs::create_dir_all(dir).expect("create dir");
    let target = dir.join(tool_name(name));
    fs::copy(fake_tool_binary(), &target).expect("copy fake tool");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&target).expect("stat").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&target, permissions).expect("chmod");
    }
    target
}

/// A file that exists at the right place but cannot run: the "right path, wrong
/// thing" case the lookup has to skip.
///
/// Bytes that are not an executable on **any** of the three OSes, and no `chmod`:
/// a shell script would happily run on Linux and on macOS, which would make the
/// test pass here and fail on Windows for the wrong reason.
fn install_broken(dir: &Path, name: &str) -> PathBuf {
    fs::create_dir_all(dir).expect("create dir");
    let target = dir.join(tool_name(name));
    fs::write(&target, b"\x00\x7fELF-not-really\x00\x01\x02").expect("write");
    target
}

struct Layout {
    _root: TempDir,
    tools_dir: PathBuf,
    bundled_dir: PathBuf,
    empty_path: PathBuf,
}

impl Layout {
    fn new() -> Self {
        let root = TempDir::new().expect("temp dir");
        let tools_dir = root.path().join("data").join("tools");
        let bundled_dir = root.path().join("app");
        fs::create_dir_all(&tools_dir).expect("tools dir");
        fs::create_dir_all(&bundled_dir).expect("bundled dir");
        let empty_path = root.path().join("empty-path");
        fs::create_dir_all(&empty_path).expect("empty path dir");
        Self {
            _root: root,
            tools_dir,
            bundled_dir,
            empty_path,
        }
    }

    fn app_data_for(&self, tool: &str) -> PathBuf {
        self.tools_dir.join(tool)
    }

    /// A lookup that can only ever find things inside this layout.
    fn lookup(&self, override_path: Option<PathBuf>) -> Lookup {
        Lookup {
            override_path,
            tools_dir: self.tools_dir.clone(),
            bundled_dir: self.bundled_dir.clone(),
            path_dirs: vec![self.empty_path.clone()],
            well_known_dirs: vec![self.empty_path.clone()],
        }
    }
}

fn triple() -> String {
    manifest::current_target_triple().expect("target triple")
}

fn embedded_manifest() -> ToolsManifest {
    ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"))
}

#[tokio::test]
async fn the_app_data_copy_wins_over_the_bundled_one() {
    // Spec §5.2 order #2 before #3, and the reason the copy exists at all: what
    // runs is the one that can be updated.
    let layout = Layout::new();
    install_fake(&layout.app_data_for("yt-dlp"), "yt-dlp");
    install_fake(&layout.bundled_dir, "yt-dlp");

    let resolved = tools::resolve("yt-dlp", &layout.lookup(None), &triple(), None)
        .await
        .expect("the app-data copy must be found");
    assert_eq!(resolved.source, ToolSource::AppData);
    assert_eq!(
        resolved.path,
        layout.app_data_for("yt-dlp").join(tool_name("yt-dlp"))
    );
    assert_eq!(resolved.version.as_deref(), Some("2026.01.01"));
}

#[tokio::test]
async fn the_bundled_baseline_is_used_when_there_is_no_app_data_copy() {
    // Spec §5.2.1: with a fresh install and no writable copy yet, the bundled
    // binary must already work — this is the "offline right after install"
    // requirement.
    let layout = Layout::new();
    install_fake(&layout.bundled_dir, "yt-dlp");

    let resolved = tools::resolve("yt-dlp", &layout.lookup(None), &triple(), None)
        .await
        .expect("the bundled copy must be found");
    assert_eq!(resolved.source, ToolSource::Bundled);
    assert_eq!(resolved.version.as_deref(), Some("2026.01.01"));
}

#[tokio::test]
async fn a_candidate_that_does_not_run_is_skipped_not_used() {
    // A file at the right path that cannot answer `--version` must not be
    // accepted: that is the failure the Tools page has to surface.
    let layout = Layout::new();
    install_broken(&layout.app_data_for("yt-dlp"), "yt-dlp");
    install_fake(&layout.bundled_dir, "yt-dlp");

    let resolved = tools::resolve("yt-dlp", &layout.lookup(None), &triple(), None)
        .await
        .expect("the bundled copy must be found");
    assert_eq!(
        resolved.source,
        ToolSource::Bundled,
        "the broken app-data copy must have been passed over"
    );
}

#[tokio::test]
async fn an_override_beats_every_other_source() {
    let layout = Layout::new();
    install_fake(&layout.app_data_for("yt-dlp"), "yt-dlp");
    install_fake(&layout.bundled_dir, "yt-dlp");
    let mine = install_fake(&layout._root.path().join("mine"), "yt-dlp");

    let resolved = tools::resolve(
        "yt-dlp",
        &layout.lookup(Some(mine.clone())),
        &triple(),
        None,
    )
    .await
    .expect("the override must be found");
    assert_eq!(resolved.source, ToolSource::Override);
    assert_eq!(resolved.path, mine);
}

#[tokio::test]
async fn a_missing_tool_is_not_an_error() {
    // Spec §1.1: show "not available", never fail silently and never panic.
    let layout = Layout::new();
    assert!(
        tools::resolve("yt-dlp", &layout.lookup(None), &triple(), None)
            .await
            .is_none()
    );

    let status = tools::status_of(
        &embedded_manifest(),
        "yt-dlp",
        &layout.lookup(None),
        &triple(),
        None,
    )
    .await;
    assert!(status.source.is_none());
    assert!(status.path.is_none());
    assert_eq!(status.version, None);
    assert!(
        !status.searched.is_empty(),
        "the UI needs to show what was tried"
    );
    assert!(status.available_on_platform);
}

#[tokio::test]
async fn qjs_and_ffmpeg_report_the_versions_their_real_banners_carry() {
    // The three banner shapes of spec §5, as produced by the real tools:
    // yt-dlp `2026.08.19`, qjs `0.17.0`, and ffmpeg's
    // `ffmpeg version n9.0.2 Copyright (c) 2000-2026 …` with its `n` prefix.
    let layout = Layout::new();
    install_fake(&layout.bundled_dir, "qjs");
    install_fake(&layout.bundled_dir, "ffmpeg");

    let qjs = tools::resolve("qjs", &layout.lookup(None), &triple(), None)
        .await
        .expect("qjs");
    assert_eq!(qjs.version.as_deref(), Some("0.17.0"));

    let ffmpeg = tools::resolve("ffmpeg", &layout.lookup(None), &triple(), None)
        .await
        .expect("ffmpeg");
    assert_eq!(ffmpeg.version.as_deref(), Some("9.0.2"));
}

#[tokio::test]
async fn an_old_tool_is_reported_as_below_the_manifest_minimum() {
    // Spec §5.1: `minVersion` is enforced, and the flag we need (`--js-runtimes`,
    // §5.3) did not exist before the floor.
    let layout = Layout::new();
    let version_file = layout.app_data_for("yt-dlp").join("fake-ytdlp.version");
    fs::create_dir_all(version_file.parent().expect("parent")).expect("create");
    install_fake(&layout.app_data_for("yt-dlp"), "yt-dlp");
    fs::write(&version_file, "2020.01.01").expect("write version");

    let manifest = embedded_manifest();
    let resolved = tools::resolve("yt-dlp", &layout.lookup(None), &triple(), None)
        .await
        .expect("yt-dlp");
    assert!(!resolved.meets_min_version(&manifest, &triple()));

    let status = tools::status_of(&manifest, "yt-dlp", &layout.lookup(None), &triple(), None).await;
    assert!(!status.meets_min_version);
    assert!(status.version.is_some(), "the version is still reported");
}

#[tokio::test]
async fn the_first_launch_copy_is_made_and_then_left_alone() {
    // Spec §5.2.1, end to end: bundled -> app data on first launch, and a second
    // call must not undo an update.
    let layout = Layout::new();
    let bundled = install_fake(&layout.bundled_dir, "yt-dlp");

    let installed = tools::install_app_data_copy(&bundled, &layout.tools_dir, &triple())
        .expect("first launch copy");
    assert_eq!(
        installed,
        layout.app_data_for("yt-dlp").join(tool_name("yt-dlp"))
    );
    assert!(installed.is_file());

    let resolved = tools::resolve("yt-dlp", &layout.lookup(None), &triple(), None)
        .await
        .expect("the copy must now win");
    assert_eq!(resolved.source, ToolSource::AppData);

    // An update writes a new version next to the copy…
    let outcome = tools::update_ytdlp(
        &installed,
        scaricatore_di_porto_lib::state::YtdlpChannel::Stable,
        None,
    )
    .await
    .expect("update");
    assert!(outcome.updated, "{outcome:?}");
    assert_eq!(outcome.previous_version.as_deref(), Some("2026.01.01"));
    assert_eq!(outcome.version.as_deref(), Some("2026.12.24"));

    // …and the next launch must not put the old bundled binary back.
    let again = tools::install_app_data_copy(&bundled, &layout.tools_dir, &triple())
        .expect("second launch copy");
    let after = tools::resolve("yt-dlp", &layout.lookup(None), &triple(), None)
        .await
        .expect("yt-dlp");
    assert_eq!(after.path, again);
    assert_eq!(after.version.as_deref(), Some("2026.12.24"));
}

#[tokio::test]
async fn updating_twice_reports_that_nothing_changed() {
    let layout = Layout::new();
    let bundled = install_fake(&layout.bundled_dir, "yt-dlp");
    let installed =
        tools::install_app_data_copy(&bundled, &layout.tools_dir, &triple()).expect("copy");

    let first = tools::update_ytdlp(
        &installed,
        scaricatore_di_porto_lib::state::YtdlpChannel::Stable,
        None,
    )
    .await
    .expect("first update");
    assert!(first.updated);

    let second = tools::update_ytdlp(
        &installed,
        scaricatore_di_porto_lib::state::YtdlpChannel::Stable,
        None,
    )
    .await
    .expect("second update");
    assert!(!second.updated, "the version did not move: {second:?}");
    assert_eq!(second.message, "Done!");
}

#[tokio::test]
async fn the_nightly_channel_installs_a_nightly_named_version() {
    // Real yt-dlp nightlies are named `YYYY.MM.DD.NNNNNN` (e.g.
    // 2025.11.12.251112123456): dotted numeric, and comparable. The fake uses
    // the same shape so this test would notice if the naming changed.
    let layout = Layout::new();
    let bundled = install_fake(&layout.bundled_dir, "yt-dlp");
    let installed =
        tools::install_app_data_copy(&bundled, &layout.tools_dir, &triple()).expect("copy");

    let outcome = tools::update_ytdlp(
        &installed,
        scaricatore_di_porto_lib::state::YtdlpChannel::Nightly,
        None,
    )
    .await
    .expect("nightly update");
    assert!(outcome.updated, "{outcome:?}");
    assert_eq!(outcome.version.as_deref(), Some("2026.12.24.240101123456"));
    assert_eq!(outcome.message, "Updated to nightly");

    // And the next lookup sees the nightly version, not a guess.
    let after = tools::resolve("yt-dlp", &layout.lookup(None), &triple(), None)
        .await
        .expect("yt-dlp");
    assert_eq!(after.version.as_deref(), Some("2026.12.24.240101123456"));
}

#[tokio::test]
async fn a_failing_updater_is_reported_as_a_failure() {
    let layout = Layout::new();
    let broken = install_broken(&layout.bundled_dir, "yt-dlp");

    let err = tools::update_ytdlp(
        &broken,
        scaricatore_di_porto_lib::state::YtdlpChannel::Stable,
        None,
    )
    .await
    .expect_err("a binary that cannot run cannot update");
    assert!(
        !err.to_string().is_empty(),
        "the detail is what the UI shows"
    );
}

#[tokio::test]
async fn the_lookup_never_produces_a_relative_program_path() {
    // Spec §4.8, checked on the real layout rather than on a mock: `process.rs`
    // refuses a relative program, and the lookup must never hand it one.
    let layout = Layout::new();
    for tool in ["yt-dlp", "qjs", "ffmpeg"] {
        for (_, path) in tools::candidates(tool, &layout.lookup(None), &triple()) {
            assert!(path.is_absolute(), "{tool}: {} is relative", path.display());
        }
    }
}

#[tokio::test]
async fn the_settings_override_is_what_the_lookup_actually_reads() {
    // The Tools page writes `tool_paths`; this closes the loop from the document
    // to the resolved path, which is the part a user can get wrong.
    let layout = Layout::new();
    install_fake(&layout._root.path().join("mine"), "yt-dlp");
    let mine = layout._root.path().join("mine").join(tool_name("yt-dlp"));

    let mut settings = Settings::default();
    settings
        .tool_paths
        .insert("yt-dlp".to_owned(), mine.clone());
    settings.normalise();

    let lookup = Lookup {
        override_path: settings.tool_override("yt-dlp").map(Path::to_path_buf),
        ..layout.lookup(None)
    };
    let resolved = tools::resolve("yt-dlp", &lookup, &triple(), None)
        .await
        .expect("yt-dlp");
    assert_eq!(resolved.source, ToolSource::Override);
    assert_eq!(resolved.path, mine);
}
