//! Fake external tool for the process-layer tests (spec §4, spec §15
//! "Integration tests: fake `yt-dlp`, `ffmpeg`, `qjs`, `sdp-whisper` scripts
//! … to test events, cancellation, stderr draining").
//!
//! It is deliberately a **Rust bin of this package**, not a shell/batch script,
//! because a `.sh` cannot be executed on Windows without a POSIX layer (spec
//! §15 wants the same tests on all three OSes) and a `.cmd` cannot be executed
//! on Unix. One cross-platform binary, no interpreter, no network.
//!
//! It is never shipped: `tauri build` bundles only the main binary, and this
//! target has no dependencies at all.
//!
//! Usage (see `tests/process_tree.rs`):
//!
//! | command                     | behaviour                                                   |
//! | :-------------------------- | :---------------------------------------------------------- |
//! | `fake_tool stdout <n> [p]`  | `n` lines on **stdout**, prefixed with `p` (default `out`)   |
//! | `fake_tool stderr <n> [p]`  | `n` lines on **stderr**, prefixed with `p` (default `err`)   |
//! | `fake_tool both <n>`        | `n` lines on both streams, interleaved                       |
//! | `fake_tool stderr-payload <bytes>` | one `x`-filled stderr line of exactly `bytes` bytes (minus the newline) |
//! | `fake_tool sleep <ms>`      | sleep, then exit 0                                          |
//! | `fake_tool exit <code>`     | exit immediately with `code`                                |
//! | `fake_tool grandchild <ms>` | spawn a child of itself that sleeps `ms`, print its pid, then sleep forever |
//! | `fake_tool env <NAME>`      | print the value of `NAME` from the environment              |
//! | `fake_tool args`            | print every remaining argument, one per line, on stdout      |
//! | `fake_tool noop`            | exit 0 without writing anything                              |
//!
//! Unknown commands print their usage on stderr and exit `2`, so a typo in a
//! test is a visible failure instead of a hang.
//!
//! ## Multi-call: impersonating a real tool
//!
//! Copying this binary to `yt-dlp`, `qjs`, `ffmpeg`… makes it behave like that
//! program (`--version`, `yt-dlp -U`, …) instead of like `fake_tool`. That is
//! what `tests/tools.rs` uses to exercise the lookup of spec §5.2 with no real
//! tool on any of the three OSes — and it is why the lookup tests never need a
//! shell script.

use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// How long `grandchild` and `sleep forever` loops run when the test does not
/// say otherwise. Long enough that only a *working* tree-kill can end them.
const DEFAULT_SLEEP_MS: u64 = 120_000;

/// Version a fake `yt-dlp` reports until it is updated.
const DEFAULT_YTDLP_VERSION: &str = "2026.01.01";

/// What a fake `yt-dlp` reports after `-U`.
const UPDATED_YTDLP_VERSION: &str = "2026.12.24";

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    let program = raw.first().cloned().unwrap_or_default();
    let args: Vec<String> = raw.into_iter().skip(1).collect();

    // Multi-call first: argv[0] decides which tool this run is pretending to be.
    let stem = Path::new(&program)
        .file_stem()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let code = match stem.as_str() {
        "yt-dlp" => run_ytdlp(&args),
        "qjs" => run_qjs(&args),
        "ffmpeg" => run_ffmpeg(&args),
        "ffprobe" => run_ffprobe(&args),
        _ => run(&args),
    };
    std::process::exit(code);
}

/// The version file a fake `yt-dlp` keeps next to itself, so `-U` can change
/// what a *later* `--version` reports — which is how `tests/tools.rs` proves the
/// updater really re-probed the binary instead of trusting its own wording.
fn version_file() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|dir| dir.join("fake-ytdlp.version"))
}

fn read_version() -> String {
    version_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| DEFAULT_YTDLP_VERSION.to_owned())
}

fn run_ytdlp(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("{}", read_version());
            0
        }
        Some("-U") | Some("--update") => {
            let next = std::env::var("FAKE_YTDLP_UPDATE_TO")
                .unwrap_or_else(|_| UPDATED_YTDLP_VERSION.to_owned());
            if let Some(path) = version_file() {
                let _ = std::fs::write(&path, &next);
            }
            // yt-dlp prints progress on stderr and finishes with "Done!"; the
            // classifier must work with that, not with an exit code alone.
            eprintln!("[youtube] Downloading simulator: 0% of 1.00MiB at 1.00KiB/s ETA 00:10");
            eprintln!("Done!");
            0
        }
        Some("--update-to") => {
            let channel = args.get(1).map(String::as_str).unwrap_or("stable");
            if let Some(path) = version_file() {
                // A nightly build's version is not a plain dotted number, which
                // is exactly what `parse_version_banner` must refuse to guess.
                let next = if channel == "nightly" {
                    format!("{UPDATED_YTDLP_VERSION}.240101123456")
                } else {
                    UPDATED_YTDLP_VERSION.to_owned()
                };
                let _ = std::fs::write(&path, next);
            }
            eprintln!("Updated to {channel}");
            0
        }
        Some("broken") => {
            eprintln!("ERROR: fake yt-dlp was told to fail");
            1
        }
        Some(other) => {
            eprintln!("fake yt-dlp: unsupported argument {other:?}");
            2
        }
        None => 2,
    }
}

fn run_qjs(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!(
                "{}",
                std::env::var("FAKE_QJS_VERSION").unwrap_or_else(|_| "0.17.0".to_owned())
            );
            0
        }
        Some(other) => {
            eprintln!("fake qjs: unsupported argument {other:?}");
            2
        }
        None => 2,
    }
}

fn run_ffmpeg(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "-version") {
        println!("ffmpeg version n9.0.2 Copyright (c) 2000-2026 the FFmpeg developers");
        return 0;
    }

    if args.iter().any(|a| a == "-encoders") {
        println!("Encoders:");
        println!(" V..... = Video");
        println!(" A..... = Audio");
        println!(" V..... libx264              libx264 H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10");
        println!(" V..... libx265              libx265 H.265 / HEVC");
        println!(" V..... h264_nvenc           NVIDIA NVENC H.264 encoder");
        println!(" V..... hevc_nvenc           NVIDIA NVENC hevc encoder");
        println!(" V..... h264_qsv             Intel Quick Sync Video H.264 encoder");
        println!(" V..... hevc_qsv             Intel Quick Sync Video hevc encoder");
        println!(" V..... h264_amf             AMD AMF H.264 encoder");
        println!(" V..... hevc_amf             AMD AMF hevc encoder");
        println!(" V..... h264_vaapi           VAAPI H.264 encoder");
        println!(" V..... hevc_vaapi           VAAPI hevc encoder");
        println!(" V..... h264_videotoolbox    VideoToolbox H.264 encoder");
        println!(" V..... hevc_videotoolbox    VideoToolbox hevc encoder");
        println!(" A..... aac                  AAC (Advanced Audio Coding)");
        println!(" A..... libmp3lame           libmp3lame MP3 (MPEG audio layer 3)");
        println!(" A..... libopus              libopus Opus");
        return 0;
    }

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let read_flag = |name: &str| -> Option<String> {
        exe_dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join(name)).ok())
    };

    if let Some(record) = read_flag("record_env.txt") {
        let val = std::env::var(record.trim()).unwrap_or_default();
        if let Some(dir) = &exe_dir {
            let _ = std::fs::write(dir.join("recorded_env.txt"), val);
        }
    }

    // Probe mode: ffmpeg ... -c:v <encoder> ... -f null -
    let is_null_sink = args.windows(2).any(|w| w[0] == "-f" && w[1] == "null")
        || args.last().map(String::as_str) == Some("-");
    if is_null_sink {
        let v_codec = args
            .windows(2)
            .find(|w| w[0] == "-c:v")
            .map(|w| w[1].as_str())
            .unwrap_or_default();
        let fail_qv =
            read_flag("fail_qv.txt").is_some() || std::env::var("FAKE_FFMPEG_FAIL_QV").is_ok();
        if fail_qv && args.iter().any(|a| a == "-q:v") {
            eprintln!("fake ffmpeg: simulated -q:v rejection during probe");
            return 1;
        }
        let fail_enc =
            read_flag("fail_probe.txt").or_else(|| std::env::var("FAKE_FFMPEG_FAIL_PROBE").ok());
        if let Some(fail_enc) = fail_enc {
            if fail_enc.split(',').any(|s| s.trim() == v_codec) {
                eprintln!("fake ffmpeg: simulated probe failure for encoder {v_codec}");
                return 1;
            }
        }
        return 0;
    }

    // Encode run:
    let v_codec = args
        .windows(2)
        .find(|w| w[0] == "-c:v")
        .map(|w| w[1].as_str())
        .unwrap_or_default();

    let fail_qv =
        read_flag("fail_qv.txt").is_some() || std::env::var("FAKE_FFMPEG_FAIL_QV").is_ok();
    if fail_qv && args.iter().any(|a| a == "-q:v") {
        eprintln!("fake ffmpeg: simulated -q:v rejection during encode");
        return 1;
    }

    let fail_enc =
        read_flag("fail_encode.txt").or_else(|| std::env::var("FAKE_FFMPEG_FAIL_ENCODE").ok());
    if let Some(fail_enc) = fail_enc {
        if fail_enc.split(',').any(|s| s.trim() == v_codec) {
            eprintln!("fake ffmpeg: simulated encode failure for encoder {v_codec}");
            return 1;
        }
    }

    if read_flag("fail_all.txt").is_some() || std::env::var("FAKE_FFMPEG_FAIL_ALL_ENCODE").is_ok() {
        eprintln!("fake ffmpeg: simulated total encode failure");
        return 1;
    }

    if args.iter().any(|a| a == "-progress") {
        println!("out_time_us=5000000");
        println!("speed=2.0x");
        println!("progress=continue");
        println!("out_time_us=10000000");
        println!("speed=2.0x");
        println!("progress=end");
    }

    // Write dummy output to last argument if it looks like a path
    if let Some(out_path) = args.last() {
        if !out_path.starts_with('-') {
            if let Some(parent) = std::path::Path::new(out_path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(err) = std::fs::write(out_path, b"DUMMY_ENCODED_OUTPUT_DATA") {
                eprintln!("fake ffmpeg: failed to write {out_path}: {err}");
                return 1;
            }
        }
    }

    0
}

fn run_ffprobe(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "-version") {
        println!("ffprobe version n9.0.2 Copyright (c) 2000-2026 the FFmpeg developers");
        return 0;
    }

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let read_flag = |name: &str| -> Option<String> {
        exe_dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join(name)).ok())
    };

    if let Some(record) = read_flag("record_env.txt") {
        let val = std::env::var(record.trim()).unwrap_or_default();
        if let Some(dir) = &exe_dir {
            let _ = std::fs::write(dir.join("recorded_env.txt"), val);
        }
    }

    if let Some(dur) = read_flag("fake_ffprobe_duration.txt") {
        println!("{}", dur.trim());
        return 0;
    }

    // Default duration for probe
    println!("120.000000");
    0
}

fn usage() -> String {
    [
        "usage: fake_tool <command> [args]",
        "  stdout <n> [prefix]        n lines on stdout",
        "  stderr <n> [prefix]        n lines on stderr",
        "  both <n>                   n lines on stdout and stderr",
        "  stderr-payload <bytes>     one stderr line of exactly <bytes> bytes",
        "  sleep <ms>                 sleep then exit 0",
        "  exit <code>                exit immediately with <code>",
        "  grandchild <ms>            spawn a sleeping child, print its pid, then sleep",
        "  env <NAME>                 print an environment variable",
        "  args                       print every remaining argument, one per line",
        "  noop                       exit 0",
    ]
    .join("\n")
}

fn run(args: &[String]) -> i32 {
    let Some(command) = args.first() else {
        eprintln!("{}", usage());
        return 2;
    };
    let rest = &args[1..];

    match command.as_str() {
        "stdout" => match number(rest, 0) {
            Ok(n) => {
                write_lines(io::stdout(), n, prefix(rest, 1, "out"));
                0
            }
            Err(message) => fail(&message),
        },
        "stderr" => match number(rest, 0) {
            Ok(n) => {
                write_lines(io::stderr(), n, prefix(rest, 1, "err"));
                0
            }
            Err(message) => fail(&message),
        },
        "both" => match number(rest, 0) {
            Ok(n) => {
                // Interleaved on purpose: both pipes fill at the same rate,
                // which is exactly the deadlock this exercises (spec §4.3).
                for index in 0..n {
                    let _ = writeln!(io::stdout(), "out-{index}");
                    let _ = writeln!(io::stderr(), "err-{index}");
                }
                0
            }
            Err(message) => fail(&message),
        },
        "stderr-payload" => match number(rest, 0) {
            Ok(bytes) => {
                let width = usize::try_from(bytes).unwrap_or(0);
                let mut line: Vec<u8> = std::iter::repeat_n(b'x', width).collect();
                line.push(b'\n');
                let mut out = BufWriter::new(io::stderr().lock());
                let _ = out.write_all(&line);
                let _ = out.flush();
                0
            }
            Err(message) => fail(&message),
        },
        "sleep" => match number(rest, 0) {
            Ok(ms) => {
                std::thread::sleep(Duration::from_millis(ms));
                0
            }
            Err(message) => fail(&message),
        },
        "exit" => match number(rest, 0) {
            Ok(code) => i32::try_from(code).unwrap_or(1),
            Err(message) => fail(&message),
        },
        "grandchild" => match spawn_grandchild(rest) {
            Ok(()) => loop_sleep(),
            Err(message) => fail(&message),
        },
        "forever" => loop_sleep(),
        "args" => {
            // Prints what the OS actually handed us, which is how the
            // integration tests check that the URL really lands after `--`.
            for arg in rest {
                println!("{arg}");
            }
            0
        }
        "env" => match rest.first() {
            Some(name) => {
                println!("{}", std::env::var(name).unwrap_or_default());
                0
            }
            None => fail("env needs a variable name"),
        },
        "noop" => 0,
        other => {
            eprintln!("fake_tool: unknown command {other:?}\n{}", usage());
            2
        }
    }
}

fn fail(message: &str) -> i32 {
    eprintln!("fake_tool: {message}\n{}", usage());
    2
}

fn prefix(args: &[String], index: usize, fallback: &str) -> String {
    args.get(index)
        .cloned()
        .unwrap_or_else(|| fallback.to_owned())
}

fn number(args: &[String], index: usize) -> Result<u64, String> {
    let raw = args
        .get(index)
        .ok_or_else(|| format!("missing a number at position {index}"))?;
    raw.parse::<u64>()
        .map_err(|_| format!("{raw:?} is not a number"))
}

/// Write `count` numbered lines. `BufWriter` + an explicit flush: `println!`
/// alone would leave the tail of a 20 000-line run sitting in the buffer while
/// the test waits for it.
fn write_lines<W: Write>(writer: W, count: u64, prefix: String) {
    let mut writer = BufWriter::new(writer);
    for index in 0..count {
        let _ = writeln!(writer, "{prefix}-{index}");
    }
    let _ = writer.flush();
}

/// Spawn a copy of ourselves that just sleeps, then report its pid on stdout
/// and sleep in the foreground.
///
/// stdio is inherited on purpose: the grandchild therefore holds the write end
/// of our stdout/stderr pipes, so if the tree-kill does not reach it the
/// parent's readers never see EOF and the test times out instead of silently
/// passing.
fn spawn_grandchild(args: &[String]) -> Result<(), String> {
    let ms = number(args, 0)?;

    let exe = std::env::current_exe().map_err(|e| format!("cannot find own path: {e}"))?;
    let child = Command::new(exe)
        .arg("sleep")
        .arg(ms.to_string())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot spawn the grandchild: {e}"))?;

    println!("{}", child.id());
    let _ = io::stdout().flush();
    Ok(())
}

/// Sleep effectively forever. Only a working tree-kill ends this process, and
/// only a *broken* one makes the test run into the timeout.
fn loop_sleep() -> i32 {
    loop {
        std::thread::sleep(Duration::from_millis(DEFAULT_SLEEP_MS));
    }
}
