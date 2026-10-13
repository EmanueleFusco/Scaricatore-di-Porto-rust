//! The M0-C exit criterion, run for real: "with no internet, the bundled
//! yt-dlp and qjs run (`--version`)".
//!
//! This test uses the **actual** binaries that
//! `scripts/fetch_bundled_tools.mjs` downloaded, through the app's own lookup —
//! not through a fake. It is the only test in the suite that touches a real
//! tool, and it is skipped, loudly, when the binaries are not there: a developer
//! who has not run the fetch script, or a CI job that has not run it either,
//! must get a skip and not a false green or a false red.
//!
//! No network is involved: `--version` is purely local.
//!
//! Run it deliberately with:
//! ```text
//! node scripts/fetch_bundled_tools.mjs
//! cargo test --test bundled_tools -- --nocapture
//! ```

use std::fs;
use std::path::PathBuf;

use scaricatore_di_porto_lib::services::manifest::{self, ToolsManifest};
use scaricatore_di_porto_lib::services::tools::{self, Lookup, ToolSource};
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is `<repo>/src-tauri`.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the repository root")
        .to_path_buf()
}

fn binaries_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries")
}

fn bundled_source_dir() -> PathBuf {
    repo_root().join("src-tauri").join("binaries")
}

fn triple() -> String {
    manifest::current_target_triple().expect("target triple")
}

/// A lookup that can only see one directory, so a pass really is about the
/// bundled files and not about a yt-dlp installed on the machine.
fn lookup_only(bundled: PathBuf) -> Lookup {
    let empty = TempDir::new().expect("temp dir");
    Lookup {
        override_path: None,
        tools_dir: empty.path().join("tools"),
        bundled_dir: bundled,
        path_dirs: vec![empty.path().join("path")],
        well_known_dirs: vec![empty.path().join("well-known")],
    }
}

fn skip(tool: &str, triple: &str) {
    eprintln!(
        "\n=== SKIPPED: the real {tool} is not in src-tauri/binaries.\n\
         Run:  node scripts/fetch_bundled_tools.mjs\n\
         Target: {triple}\n\
         Expected: {}\n===",
        bundled_source_dir()
            .join(format!("{tool}-{triple}"))
            .display()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_bundled_ytdlp_and_qjs_run_without_a_network() {
    let triple = triple();
    let manifest = ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}"));

    for tool in [manifest::YTDLP, manifest::QJS] {
        let expected = binaries_dir().join(format!(
            "{tool}-{triple}{}",
            if triple.contains("windows") {
                ".exe"
            } else {
                ""
            }
        ));
        if !expected.is_file() {
            skip(tool, &triple);
            continue;
        }

        let resolved = tools::resolve(tool, &lookup_only(bundled_source_dir()), &triple, None)
            .await
            .unwrap_or_else(|| panic!("{tool}: the bundled binary did not answer --version"));

        assert_eq!(
            resolved.source,
            ToolSource::Bundled,
            "{tool} must come from the bundled baseline"
        );
        assert!(resolved.path.starts_with(bundled_source_dir()));

        let version = resolved
            .version
            .as_deref()
            .unwrap_or_else(|| panic!("{tool}: --version printed {:?}", resolved.banner));
        assert!(
            manifest::parse_version(version).is_some(),
            "{tool}: {version:?} is not a dotted numeric version"
        );
        assert!(
            resolved.meets_min_version(&manifest, &triple),
            "{tool}: {version} is below the manifest's minVersion"
        );
        eprintln!("{tool}: {version} (from {})", resolved.path.display());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_bundled_ytdlp_is_a_real_yt_dlp_and_not_a_stub() {
    let triple = triple();
    let path = binaries_dir().join(format!(
        "yt-dlp-{triple}{}",
        if triple.contains("windows") {
            ".exe"
        } else {
            ""
        }
    ));
    if !path.is_file() {
        skip("yt-dlp", &triple);
        return;
    }

    // The file is tens of megabytes, not a placeholder. This is a crude but
    // useful guard against shipping a stub that answers `--version` and then
    // does nothing.
    let size = fs::metadata(&path).expect("stat").len();
    assert!(
        size > 5_000_000,
        "the bundled yt-dlp is only {size} bytes; the official standalone build is much larger"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_bundled_qjs_is_a_real_interpreter_and_can_execute_a_script() {
    let triple = triple();
    let path = binaries_dir().join(format!(
        "qjs-{triple}{}",
        if triple.contains("windows") {
            ".exe"
        } else {
            ""
        }
    ));
    if !path.is_file() {
        skip("qjs", &triple);
        return;
    }

    // `--version` is not enough: yt-dlp actually *runs* qjs to solve the EJS
    // challenge (spec §5.3), so an interpreter that only prints its version
    // would pass every other test and fail on the first YouTube link.
    let script = TempDir::new().expect("temp dir");
    let script_path = script.path().join("probe.js");
    fs::write(&script_path, b"console.log('sdp-qjs-ok');\n").expect("write");

    let command = scaricatore_di_porto_lib::services::process::ToolCommand::new(&path)
        .arg(&script_path)
        .label("bundled qjs probe");

    // The sink must be `'static`, so the lines live behind an `Arc` that is
    // read back once the run is over — the same shape the tools service uses.
    let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = {
        let collected = std::sync::Arc::clone(&collected);
        scaricatore_di_porto_lib::services::process::line_sink(move |line| {
            if let Ok(mut lines) = collected.lock() {
                lines.push(line.to_owned());
            }
        })
    };
    let outcome = scaricatore_di_porto_lib::services::process::spawn_tool(
        command,
        tokio_util::sync::CancellationToken::new(),
        sink,
    )
    .await
    .expect("qjs must run");

    assert!(
        outcome.success(),
        "qjs exited {:?}: {:?}",
        outcome.exit_code,
        outcome.stderr_tail
    );
    let lines = collected.lock().expect("lines").clone();
    assert!(
        lines.iter().any(|line| line.contains("sdp-qjs-ok")),
        "qjs did not run the script; it printed {lines:?}"
    );
}
