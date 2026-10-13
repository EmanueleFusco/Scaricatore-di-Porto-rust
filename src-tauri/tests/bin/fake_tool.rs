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

use std::io::{self, BufWriter, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

/// How long `grandchild` and `sleep forever` loops run when the test does not
/// say otherwise. Long enough that only a *working* tree-kill can end them.
const DEFAULT_SLEEP_MS: u64 = 120_000;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    std::process::exit(code);
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
