//! Real-tool, real-network checks for M1 — **ignored by default**.
//!
//! These are the checks that `services/inspect::self_test` exists for
//! (spec §5.3.6). They are `#[ignore]`d on purpose: AGENTS.md §6 forbids tests
//! that depend on live YouTube in the normal suite, and a flaky test is worse
//! than no test. Run them deliberately with the bundled binaries fetched:
//!
//! ```text
//! node scripts/fetch_bundled_tools.mjs
//! cd src-tauri
//! cargo test --test real_selftest -- --ignored --nocapture --test-threads=1
//! ```
//!
//! They exercise the real yt-dlp and the real qjs through the same lookup and
//! the same runner the app uses — no fakes, no stubs.

use std::sync::Arc;

use scaricatore_di_porto_lib::services::inspect::{
    self, InspectEnvironment, Inspected, ResolvedRuntime, Runtimes,
};
use scaricatore_di_porto_lib::services::job::{self, JobEnvironment, JobOutcome, JobRequest};
use scaricatore_di_porto_lib::services::jsruntime::{JsRuntimeMode, RuntimeMemoryStore};
use scaricatore_di_porto_lib::services::manifest;
use scaricatore_di_porto_lib::services::tools::{self, Lookup, ToolSource};
use scaricatore_di_porto_lib::services::ytdlp::{
    CookieSource, DownloadSpec, FormatChoice, JobDirs, JobKind,
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn binaries_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries")
}

/// A lookup that can only see the fetched binaries, exactly like the
/// `bundled_tools` test: a system yt-dlp must not be able to satisfy this.
fn lookup() -> Lookup {
    let empty = TempDir::new().expect("temp dir");
    Lookup {
        override_path: None,
        tools_dir: empty.path().join("tools"),
        bundled_dir: binaries_dir(),
        path_dirs: vec![empty.path().join("path")],
        well_known_dirs: Vec::new(),
    }
}

async fn real_ytdlp() -> (std::path::PathBuf, Option<String>) {
    let triple = manifest::current_target_triple().expect("triple");
    let resolved = tools::resolve(manifest::YTDLP, &lookup(), &triple, None)
        .await
        .expect("the bundled yt-dlp must answer --version");
    assert_eq!(resolved.source, ToolSource::Bundled);
    (resolved.path, resolved.version)
}

fn real_qjs() -> Option<ResolvedRuntime> {
    let path = binaries_dir().join(format!(
        "qjs-{}{}",
        manifest::current_target_triple().expect("triple"),
        if cfg!(windows) { ".exe" } else { "" }
    ));
    if !path.is_file() {
        return None;
    }
    // The real qjs prints e.g. "0.17.0" for --version; the lookup would probe
    // it, but this file runs at test start without a probe.
    Some(ResolvedRuntime {
        path,
        version: Some("0.17.0".into()),
    })
}

fn environment(
    dir: &TempDir,
    ytdlp: std::path::PathBuf,
    ytdlp_version: Option<String>,
) -> (InspectEnvironment, JobEnvironment) {
    let qjs = real_qjs();
    let runtimes = Runtimes {
        mode: JsRuntimeMode::Auto,
        qjs,
        deno: None,
        ytdlp_version: ytdlp_version.unwrap_or_default(),
    };
    let memory = Arc::new(RuntimeMemoryStore::load(dir.path().join("memory.json")));
    let inspect_env = InspectEnvironment {
        ytdlp: ytdlp.clone(),
        cookies: CookieSource::None,
        runtimes: runtimes.clone(),
        memory: Arc::clone(&memory),
        private_temp: Some(dir.path().join("tmp")),
    };
    let job_env = JobEnvironment {
        ytdlp,
        ffmpeg: which_ffmpeg(),
        runtimes,
        memory,
        private_temp: Some(dir.path().join("tmp")),
    };
    (inspect_env, job_env)
}

/// A system ffmpeg, because merging and opus need one (spec §5.2.1).
fn which_ffmpeg() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads ~42 MB from GitHub; run it by hand"]
async fn the_deno_install_downloads_verifies_and_runs() {
    let dir = TempDir::new().expect("temp dir");
    let triple = manifest::current_target_triple().expect("triple");
    let manifest =
        scaricatore_di_porto_lib::services::manifest::ToolsManifest::embedded().expect("manifest");
    let tools_dir = dir.path().join("tools");

    let token = CancellationToken::new();
    let path = tools::install_deno(&manifest, &triple, &tools_dir, &token, |progress| {
        eprintln!(
            "deno: {}/{} bytes",
            progress.received,
            progress
                .total
                .map(|t| t.to_string())
                .unwrap_or_else(|| "?".to_owned())
        );
    })
    .await
    .expect("the deno install must succeed");
    eprintln!("installed deno at {}", path.display());
    assert!(path.is_file());

    // The installed binary really runs and reports the pinned version.
    let output = std::process::Command::new(&path)
        .arg("--version")
        .output()
        .expect("deno --version");
    let text = String::from_utf8_lossy(&output.stdout);
    eprintln!("{text}");
    assert!(output.status.success());
    assert!(
        text.contains("2.9.7"),
        "expected the pinned 2.9.7, got: {text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the network and live YouTube; run it by hand"]
async fn the_self_test_passes_with_the_bundled_tools() {
    let dir = TempDir::new().expect("temp dir");
    let (path, version) = real_ytdlp().await;
    eprintln!(
        "real yt-dlp: {} ({})",
        path.display(),
        version.as_deref().unwrap_or("?")
    );
    let (inspect_env, _job_env) = environment(&dir, path, version);

    let report = inspect::self_test(&inspect_env, CancellationToken::new()).await;
    eprintln!("self-test: {report:?}");
    assert!(report.ok, "self-test failed: {report:?}");
    assert_eq!(report.runtime.as_deref(), Some("quickjs"));
    assert!(report.formats > 0);
    assert!(!report.heights.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the network and live YouTube; run it by hand"]
async fn a_real_inspection_and_download_end_to_end() {
    if which_ffmpeg().is_none() {
        eprintln!("no system ffmpeg: skipping");
        return;
    }
    let dir = TempDir::new().expect("temp dir");
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).expect("out dir");
    let (path, version) = real_ytdlp().await;
    let (inspect_env, job_env) = environment(&dir, path, version);

    // 1. Inspect the known public video.
    let outcome = inspect::inspect(
        &inspect_env,
        inspect::SELF_TEST_URL,
        CancellationToken::new(),
    )
    .await;
    let inspect::InspectOutcome::Inspected {
        inspected,
        runtime,
        elapsed_ms,
    } = outcome
    else {
        panic!("cancelled");
    };
    let Inspected::Video(video) = &*inspected else {
        panic!("expected a video: {inspected:?}");
    };
    eprintln!(
        "inspect: {} | {} | {:?} | runtime {} in {} ms",
        video.title,
        video.site,
        video.heights,
        runtime.as_str(),
        elapsed_ms
    );
    assert!(video.has_video, "this video has video formats");
    assert!(!video.heights.is_empty());

    // 2. Download it at the smallest available height (the zoo is 144p).
    let request = JobRequest {
        url: inspect::SELF_TEST_URL.to_owned(),
        spec: DownloadSpec {
            kind: JobKind::Video,
            quality: FormatChoice::MaxHeight { height: 144 },
            cookies: CookieSource::None,
        },
        dirs: JobDirs {
            home: out.clone(),
            temp: out.join(".sdp-tmp-real"),
        },
        stall_timeout: job::STALL_TIMEOUT,
    };
    let sink = Arc::new(|_progress: &scaricatore_di_porto_lib::services::ytdlp::JobProgress| {});
    let outcome = job::run_job(&job_env, &request, CancellationToken::new(), sink).await;
    let JobOutcome::Done {
        file,
        runtime,
        elapsed_ms,
    } = outcome
    else {
        panic!("download failed: {outcome:?}");
    };
    eprintln!(
        "download: {} | runtime {} | {} ms",
        file.display(),
        runtime.as_str(),
        elapsed_ms
    );
    assert!(file.is_file(), "the file must exist: {}", file.display());
    assert!(
        file.extension()
            .is_some_and(|ext| ext == "mp4" || ext == "mkv"),
        "merged output: {}",
        file.display()
    );
    assert!(std::fs::metadata(&file).expect("stat").len() > 10_000);

    // 3. The audio-only path produces the documented .opus.
    let audio = JobRequest {
        url: inspect::SELF_TEST_URL.to_owned(),
        spec: DownloadSpec {
            kind: JobKind::Audio,
            quality: FormatChoice::AudioOnly,
            cookies: CookieSource::None,
        },
        dirs: JobDirs {
            home: out.clone(),
            temp: out.join(".sdp-tmp-real-audio"),
        },
        stall_timeout: job::STALL_TIMEOUT,
    };
    let sink = Arc::new(|_progress: &scaricatore_di_porto_lib::services::ytdlp::JobProgress| {});
    let outcome = job::run_job(&job_env, &audio, CancellationToken::new(), sink).await;
    let JobOutcome::Done { file, .. } = outcome else {
        panic!("audio download failed: {outcome:?}");
    };
    eprintln!("audio: {}", file.display());
    assert_eq!(file.extension().and_then(|ext| ext.to_str()), Some("opus"));

    // Clean-up is the caller's job; do what the command does.
    job::cleanup_job_dir(&request.dirs.temp);
    job::cleanup_job_dir(&audio.dirs.temp);
    assert!(!request.dirs.temp.exists());
    assert!(out.exists(), "finished outputs stay");
}
