//! A minimal, append-only log file (spec §11.8 "Open logs").
//!
//! The app deliberately does not pull in `tauri-plugin-log`: one text file
//! with the last few hundred lines is enough for support, and the plugin would
//! add a dependency tree for a feature that is one `OpenOptions::append` away.
//! Nothing secret is ever written here: callers pass paths, versions and error
//! summaries, never API keys or cookies.

use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager, Runtime};

/// File name inside the platform log directory.
pub const LOG_FILE: &str = "scaricatore-di-porto.log";

/// How many lines "Copy diagnostics" keeps from the tail.
pub const TAIL_LINES: usize = 80;

/// `<app log dir>/scaricatore-di-porto.log`, creating the directory.
pub fn log_path<R: Runtime>(app: &AppHandle<R>) -> io::Result<PathBuf> {
    let dir = app
        .path()
        .app_log_dir()
        .map_err(|err| io::Error::other(err.to_string()))?;
    fs::create_dir_all(&dir)?;
    Ok(dir.join(LOG_FILE))
}

/// Append one line, best effort.
///
/// Logging must never be the reason an operation fails, so callers use this
/// for information only. The timestamp is Unix seconds: no date crate is worth
/// a dependency for a support file.
pub fn append_line<R: Runtime>(app: &AppHandle<R>, level: &str, message: &str) {
    let Ok(path) = log_path(app) else {
        return;
    };
    let _ = append_to(&path, level, message);
}

/// The append primitive, split out so it is testable without an `AppHandle`.
pub fn append_to(path: &Path, level: &str, message: &str) -> io::Result<()> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    // One line per call: an embedded newline would make the tail ambiguous.
    let message = message.replace(['\r', '\n'], " ");
    writeln!(file, "[{stamp}] {level}: {message}")
}

/// The last `max_lines` lines of the log, or an empty string when it does not
/// exist yet. Read errors are reported to the caller, which turns them into a
/// diagnostic line rather than a failure.
pub fn tail_lines(path: &Path, max_lines: usize) -> io::Result<String> {
    if !path.exists() {
        return Ok(String::new());
    }
    let reader = BufReader::new(fs::File::open(path)?);
    let mut lines: Vec<String> = Vec::new();
    for line in reader.lines() {
        lines.push(line?);
        if lines.len() > max_lines {
            lines.remove(0);
        }
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn appends_lines_and_never_writes_a_bare_newline() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(LOG_FILE);
        append_to(&path, "INFO", "first").expect("append");
        append_to(&path, "ERROR", "second\nwith a newline").expect("append");
        let text = fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = text.lines().collect();
        expect_two_lines(&lines);
        assert!(lines[0].ends_with("INFO: first"));
        assert!(lines[1].ends_with("ERROR: second with a newline"));
    }

    fn expect_two_lines(lines: &[&str]) {
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn tail_keeps_only_the_requested_lines() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(LOG_FILE);
        for index in 0..10 {
            append_to(&path, "INFO", &format!("line {index}")).expect("append");
        }
        let tail = tail_lines(&path, 3).expect("tail");
        let lines: Vec<&str> = tail.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].ends_with("line 7"));
        assert!(lines[2].ends_with("line 9"));
    }

    #[test]
    fn a_missing_file_is_an_empty_tail() {
        let dir = TempDir::new().expect("tempdir");
        assert_eq!(
            tail_lines(&dir.path().join("absent.log"), 5).expect("tail"),
            ""
        );
    }
}
