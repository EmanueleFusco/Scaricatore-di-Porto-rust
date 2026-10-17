//! Fake yt-dlp for the M1 integration tests (spec §15: "fake yt-dlp … scripts
//! that emit canned output, to test events, cancellation, stderr draining").
//!
//! Same reasoning as `fake_tool`: a Rust program, not a script, so the same
//! tests run on Windows, macOS and Linux. It answers `--version`, prints canned
//! `-J` JSON for inspections and canned `SDP_DL`/`SDP_PP`/`SDP_FILE` lines for
//! downloads, and fails on purpose for the URLs whose names say so.
//!
//! Behaviour is selected by the **URL** (the last argument, after `--`) and by
//! the runtime named in `--js-runtimes`, so a test can make QuickJS "fail" while
//! Deno "works" without two different binaries.
//!
//! Every invocation appends one line to `fake-ytdlp.log` next to this binary,
//! and writes its pid to `fake-ytdlp.pid`: the tests copy the binary into a temp
//! dir, so each test gets its own log and process id.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

const VERSION: &str = "2026.08.19";

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    let args: Vec<String> = raw.iter().skip(1).cloned().collect();

    write_pid();

    // The fixtures copy this one binary under the `yt-dlp`, `qjs` **and**
    // `ffmpeg` names, and the probe asks each tool with its own documented
    // flag: `--version` for yt-dlp and qjs, `-version` for ffmpeg. An
    // unrecognised probe must never fall through to the download path.
    if args.first().map(String::as_str) == Some("--version") {
        println!("{VERSION}");
        return;
    }
    if args.first().map(String::as_str) == Some("-version") {
        println!("ffmpeg version 9.9.9-fake");
        return;
    }
    if args.iter().any(|arg| arg == "--list-impersonate-targets") {
        println!(
            "[info] Available impersonate targets\n\
             Client  OS        Source\n\
             ---------------------------------\n\
             Chrome  macos-14  curl_cffi"
        );
        return;
    }

    let url = url_of(&args).unwrap_or_default();
    let runtime = runtime_of(&args);
    let inspect = args.iter().any(|arg| arg == "-J");
    log(&format!(
        "invoke {} runtime={} format={} url={}",
        if inspect { "inspect" } else { "download" },
        runtime,
        if args.iter().any(|arg| arg == "-f") {
            "yes"
        } else {
            "no"
        },
        url
    ));

    let code = if inspect {
        inspect_url(&args, &url, &runtime)
    } else {
        // A download has a URL after `--` and a folder to write into, and both
        // come from the app. Neither is guessed: refusing here is what keeps a
        // probe (or a mistake) from writing a fake media file into the
        // process's working directory — which is how one of those files once
        // landed in the repository root and was committed.
        if url.is_empty() {
            eprintln!("ERROR: not a download: no URL after `--`");
            std::process::exit(2);
        }
        let home = paths_value(&args, "home:");
        if home.as_ref().is_none_or(|path| path.as_os_str().is_empty()) {
            eprintln!("ERROR: not a download: no `--paths home:` folder");
            std::process::exit(2);
        }
        download_url(&args, &url, &runtime)
    };
    std::process::exit(code);
}

/// The URL is always the last argument, after `--` (spec §4.6).
fn url_of(args: &[String]) -> Option<String> {
    args.iter()
        .rposition(|arg| arg == "--")
        .and_then(|index| args.get(index + 1))
        .cloned()
}

/// `quickjs`, `deno` or `none`, from the `--js-runtimes` value.
fn runtime_of(args: &[String]) -> String {
    args.windows(2)
        .find(|pair| pair[0] == "--js-runtimes")
        .map(|pair| {
            if pair[1].starts_with("quickjs:") {
                "quickjs".to_owned()
            } else if pair[1].starts_with("deno:") {
                "deno".to_owned()
            } else {
                "other".to_owned()
            }
        })
        .unwrap_or_else(|| "none".to_owned())
}

fn own_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Append one line, in **one** write.
///
/// The queue tests run several of these processes at the same time, all into
/// the same file: `writeln!` can split a line into several `write` calls, and
/// two processes then interleave their bytes into records like
/// `…slow-aend 1792053948733`. A single `write_all` of a buffer ending in a
/// newline is one `write(2)` on an `O_APPEND` descriptor, which the kernel
/// applies atomically — the log stays parseable when four downloads overlap.
fn append_line(name: &str, line: &str) {
    let path = own_dir().join(name);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let mut buffer = String::with_capacity(line.len() + 1);
        buffer.push_str(line);
        buffer.push('\n');
        let _ = file.write_all(buffer.as_bytes());
    }
}

fn log(line: &str) {
    append_line("fake-ytdlp.log", line);
}

/// The M2 queue tests need to know **when** a download started and ended, to
/// measure how many really ran at the same time. It goes in its own file: the
/// M1 tests assert on the line count and the order of `fake-ytdlp.log`, and
/// they must not see extra lines.
fn runs_log(line: &str) {
    append_line("fake-ytdlp-runs.log", line);
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// How long a "slow" download pretends to take, overridable so a test can
/// trade duration for precision.
fn sleep_millis() -> u64 {
    std::env::var("FAKE_YTDLP_SLEEP_MS")
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or(400)
}

/// A counter of its own, so the M1 scenarios that count invocations are not
/// disturbed by the M2 ones.
fn counter(name: &str, bump: bool) -> u32 {
    let path = own_dir().join(format!("fake-ytdlp.{name}"));
    let previous = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let next = if bump { previous + 1 } else { previous };
    if bump {
        let _ = std::fs::write(path, next.to_string());
    }
    next
}

fn write_pid() {
    let path = own_dir().join("fake-ytdlp.pid");
    let _ = std::fs::write(path, std::process::id().to_string());
}

/// How many inspect/download invocations this copy has seen (the `--version`
/// probe does not count). Used by the degraded-extraction scenarios, where the
/// first call is intentionally worse than the second.
fn invocation_count() -> u32 {
    let path = own_dir().join("fake-ytdlp.calls");
    let previous = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let next = previous + 1;
    let _ = std::fs::write(path, next.to_string());
    next
}

/// yt-dlp's own wording for a failed JS challenge, verbatim from its source at
/// the pinned version. The app must recognise exactly this.
fn challenge_warning() {
    eprintln!(
        "WARNING: [youtube] n challenge solving failed: Some formats may be missing. \
         Ensure you have a supported JavaScript runtime installed"
    );
}

fn inspect_url(args: &[String], url: &str, runtime: &str) -> i32 {
    if url.contains("antibot-retry") {
        let impersonated = args
            .windows(2)
            .any(|pair| pair[0] == "--impersonate" && pair[1] == "chrome");
        if !impersonated {
            eprintln!("ERROR: [generic] video: HTTP Error 403: Forbidden (Cloudflare)");
            return 1;
        }
    }
    if url.contains("auth-fail") {
        eprintln!(
            "ERROR: [youtube] x: Sign in to confirm you're not a bot. Use --cookies-from-browser or --cookies for the authentication."
        );
        return 1;
    }
    if url.contains("challenge") && runtime == "quickjs" {
        // A challenge failure of the family the classifier maps to YT_CHALLENGE.
        eprintln!("ERROR: [youtube] x: Unable to solve the JS challenge (nsig)");
        return 1;
    }
    // The degraded extraction of the real report: the JS challenge fails, yt-dlp
    // warns, and the only format left is the muxed 360p. "degraded-once" is
    // fixed by the next call (any runtime); "degraded-always" never recovers.
    if url.contains("degraded-once") {
        if invocation_count() == 1 {
            challenge_warning();
            println!(
                r#"{{"_type":"video","id":"deg","title":"degraded","extractor_key":"Youtube","formats":[{{"format_id":"18","ext":"mp4","height":360,"vcodec":"avc1","acodec":"mp4a"}},{{"format_id":"sb0","ext":"mhtml","vcodec":"none","acodec":"none","protocol":"mhtml"}}]}}"#
            );
            return 0;
        }
        print!("{VIDEO_JSON}");
        return 0;
    }
    if url.contains("degraded-always") {
        challenge_warning();
        println!(
            r#"{{"_type":"video","id":"deg","title":"degraded","extractor_key":"Youtube","formats":[{{"format_id":"18","ext":"mp4","height":360,"vcodec":"avc1","acodec":"mp4a"}}]}}"#
        );
        return 0;
    }
    if url.contains("noheights") || (url.contains("degraded") && runtime == "quickjs") {
        // Video formats exist but none has a height: the fallback condition of
        // spec §5.3.2. With Deno the same URL comes back complete.
        if url.contains("degraded") && runtime == "deno" {
            print!("{VIDEO_JSON}");
            return 0;
        }
        println!(
            r#"{{"_type":"video","id":"nh","title":"no heights","extractor_key":"Youtube","formats":[{{"format_id":"v","ext":"mp4","vcodec":"h264","acodec":"none"}},{{"format_id":"a","ext":"m4a","vcodec":"none","acodec":"mp4a"}}]}}"#
        );
        return 0;
    }
    let no_playlist = args.iter().any(|arg| arg == "--no-playlist");
    if (url.contains("playlist") || url.contains("list=")) && !no_playlist {
        println!(
            r#"{{"_type":"playlist","id":"PL1","title":"A playlist","extractor_key":"Youtube","webpage_url":"https://www.youtube.com/playlist?list=PL1","playlist_count":3,"entries":[{{"id":"a","title":"Video A","duration":60.0}},{{"id":"b","title":"[Private video]"}},{{"id":"c","title":"Video C","duration":180.0}}]}}"#
        );
        return 0;
    }
    if url.contains("slow-inspect") {
        sleep_forever();
    }
    if url.contains("garbage") {
        println!("not json at all");
        return 0;
    }
    print!("{VIDEO_JSON}");
    0
}

fn download_url(args: &[String], url: &str, runtime: &str) -> i32 {
    // Every download announces its start and its end, so a test can measure
    // how many really overlapped (spec §7.2). "slow" and the network scenarios
    // come first: they are the ones the queue tests drive.
    runs_log(&format!("start {} {}", now_millis(), url));
    let code = download_body(args, url, runtime);
    runs_log(&format!("end {} {}", now_millis(), url));
    code
}

fn download_body(args: &[String], url: &str, runtime: &str) -> i32 {
    if url.contains("antibot-retry") {
        let impersonated = args
            .windows(2)
            .any(|pair| pair[0] == "--impersonate" && pair[1] == "chrome");
        if !impersonated {
            eprintln!("ERROR: [generic] video: HTTP Error 403: Forbidden (Cloudflare)");
            return 1;
        }
    }
    if url.contains("archive-skip") && args.iter().any(|arg| arg == "--download-archive") {
        // Real yt-dlp implies `--quiet` when `--print` is present; it only
        // outputs the skip message if `--no-quiet` is also passed.
        let quiet =
            args.iter().any(|arg| arg == "--print") && !args.iter().any(|arg| arg == "--no-quiet");
        if !quiet {
            println!("[download] fakeid: has already been recorded in the archive");
        }
        return 0;
    }
    // A transient network failure that heals on the second attempt: the queue's
    // automatic retry (spec §7.3) must notice, wait and try again.
    if url.contains("netfail-once") && counter("netfail-once", true) <= 1 {
        eprintln!(
            "ERROR: unable to download webpage: <urlopen error [Errno -3] Temporary failure in \
             name resolution>"
        );
        return 1;
    }
    if url.contains("netfail-always") {
        eprintln!("ERROR: The read operation timed out");
        return 1;
    }
    if url.contains("error-auth") {
        eprintln!(
            "ERROR: [youtube] x: Private video. Sign in if you've been granted access to this video"
        );
        return 1;
    }
    if url.contains("format-fail") && args.iter().any(|arg| arg == "-f") {
        eprintln!(
            "ERROR: [youtube] x: Requested format is not available. Use --list-formats for a list of available formats"
        );
        return 1;
    }
    if url.contains("challenge") && runtime == "quickjs" {
        eprintln!("ERROR: [youtube] x: Unable to solve the JS challenge (nsig)");
        return 1;
    }
    if url.contains("stall") && runtime == "quickjs" {
        // Nothing on either stream: the watchdog of spec §5.3.2 is the only
        // thing that can end this.
        sleep_forever();
    }
    if url.contains("cancel") {
        emit_download_line("downloading", 512, 1024, "vp9");
        emit_download_line("downloading", 1024, 1024, "vp9");
        sleep_forever();
    }
    // A download that is slow on a **clock** rather than on progress, for the
    // test that has to close the app while a tool is really running.
    //
    // The tool announces itself with a marker in the output folder — named
    // after the URL it is downloading — and then sleeps: the test waits for the
    // marker instead of hoping the tool is still alive when it gets around to
    // shutting the app down. A margin alone does not work: the CI starved a
    // poller past an 8 s one and read the queue file after the run had
    // finished. `veryslow-<ms>` is how long it then sleeps, which is only how
    // long the window between the marker and the shutdown stays open.
    if url.contains("veryslow") {
        let millis = url
            .split("veryslow-")
            .nth(1)
            .and_then(|raw| {
                raw.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse::<u64>()
                    .ok()
            })
            .unwrap_or(5_000);
        println!("SDP_FORMAT|137+140|1080|vp9|opus");
        let home = home_dir(args);
        let _ = std::fs::create_dir_all(&home);
        // The marker is named after the **host** the tool is downloading, not
        // after the title: two items of one test differ by host, and the test
        // must be able to tell which of them the lane actually picked — a marker
        // built from the URL tail cannot (two URLs of one test share a tail).
        let host = url
            .split('/')
            .nth(2)
            .and_then(|authority| authority.split(':').next())
            .filter(|host| !host.is_empty())
            .unwrap_or("unknown-host");
        let _ = std::fs::write(home.join(format!(".fake-started-{host}")), b"");
        // Twenty steps, so the progress line grows like a real download and
        // the watchdog of spec §5.3.2 has something to watch.
        let steps: u64 = 20;
        for step in 1..=steps {
            emit_download_line("downloading", step * (millis / steps), millis, "vp9");
            thread::sleep(Duration::from_millis(millis / steps));
        }
        emit_download_line("downloading", millis, millis, "vp9");
        eprintln!("SDP_PP|started|Merger");
        let file = home.join(safe_name(url));
        let _ = std::fs::write(&file, b"fake media bytes");
        print_file_line(args, &file);
        return 0;
    }

    // A degraded extraction: the separate streams are missing, so yt-dlp's own
    // fallback chain lands on the **muxed** 360p format (no `+` in the id).
    // The first invocation then "downloads" slowly — the app must cancel it at
    // the SDP_FORMAT line instead of saving the wrong file. The next
    // invocation (Deno, or the same runtime when Deno is absent) is healthy.
    if url.contains("degraded-download") {
        if invocation_count() == 1 {
            println!("SDP_FORMAT|18|360|avc1|mp4a.40.2");
            emit_download_line("downloading", 2048, 40960, "avc1");
            sleep_forever();
        }
        println!("SDP_FORMAT|137+140|1080|vp9|mp4a.40.2");
        emit_download_line("downloading", 2048, 4096, "vp9");
        emit_download_line("finished", 4096, 4096, "vp9");
        let home = home_dir(args);
        let _ = std::fs::create_dir_all(&home);
        let file = home.join("Fake title [fakeid].mkv");
        let _ = std::fs::write(&file, b"fake media bytes");
        print_file_line(args, &file);
        return 0;
    }

    // A degraded extraction that never recovers: every attempt picks the muxed
    // 360p. The app must give up with an error, not save it.
    if url.contains("degraded-always") {
        println!("SDP_FORMAT|18|360|avc1|mp4a.40.2");
        emit_download_line("downloading", 2048, 40960, "avc1");
        sleep_forever();
    }

    // A **merge** at a lower height than requested: the separate streams were
    // found, so the video simply has no higher resolution. Not a degraded
    // extraction, and no retry is worth making.
    if url.contains("merged-lower") {
        println!("SDP_FORMAT|137+140|720|avc1|mp4a.40.2");
        emit_download_line("downloading", 2048, 4096, "avc1");
        emit_download_line("finished", 4096, 4096, "avc1");
        let home = home_dir(args);
        let _ = std::fs::create_dir_all(&home);
        let file = home.join("Fake title [fakeid].mkv");
        let _ = std::fs::write(&file, b"fake media bytes");
        print_file_line(args, &file);
        return 0;
    }

    // A download that takes measurable time, for the queue's parallelism tests:
    // it emits progress, waits, and finishes. The wait is what makes two
    // downloads overlap for long enough to be observed.
    if url.contains("slow") {
        let total = sleep_millis() * 4;
        println!("SDP_FORMAT|137+140|1080|vp9|opus");
        for step in 1..4 {
            emit_download_line("downloading", step * (total / 4), total, "vp9");
            thread::sleep(Duration::from_millis(sleep_millis()));
        }
        emit_download_line("downloading", total, total, "vp9");
        eprintln!("SDP_PP|started|Merger");
        let home = home_dir(args);
        let _ = std::fs::create_dir_all(&home);
        let file = home.join(safe_name(url));
        let _ = std::fs::write(&file, b"fake media bytes");
        print_file_line(args, &file);
        return 0;
    }

    // A download whose real file name has characters outside the console code
    // page, for the encoding scenario of D114. The file is written under its
    // real Unicode name — exactly like the real tool, which creates it with
    // wide-char APIs — and the printed path goes through `print_file_line`,
    // which writes the code page unless `--encoding utf-8` was passed.
    if url.contains("accent") {
        println!("SDP_FORMAT|137+140|1080|vp9|opus");
        emit_download_line("downloading", 2048, 4096, "vp9");
        emit_download_line("finished", 4096, 4096, "vp9");
        eprintln!("SDP_PP|started|Merger");
        let home = home_dir(args);
        let _ = std::fs::create_dir_all(&home);
        let file = home.join(safe_name(url));
        let _ = std::fs::write(&file, b"fake media bytes");
        print_file_line(args, &file);
        return 0;
    }

    // A normal two-stream download with a merge.
    println!("SDP_FORMAT|137+140|1080|avc1|mp4a.40.2");
    emit_download_line("downloading", 2048, 4096, "vp9");
    emit_download_line("finished", 4096, 4096, "vp9");
    emit_download_line("downloading", 1024, 2048, "none");
    emit_download_line("finished", 2048, 2048, "none");
    eprintln!("SDP_PP|started|Merger");
    eprintln!("SDP_PP|finished|Merger");
    eprintln!("SDP_PP|started|MoveFiles");

    // Write the output where `--paths home:` says, with a name that is never
    // guessed by the app (it must read SDP_FILE).
    let home = home_dir(args);
    let _ = std::fs::create_dir_all(&home);
    let opus = args.iter().any(|arg| arg == "-x");
    if opus {
        println!("SDP_FORMAT|251|NA|none|opus");
    }
    let name = if opus {
        "Fake song [fakeid].opus"
    } else {
        "Fake title [fakeid].mkv"
    };
    let file = home.join(name);
    let _ = std::fs::write(&file, b"fake media bytes");
    print_file_line(args, &file);
    if url.contains("embed-fail") {
        eprintln!("ERROR: [EmbedThumbnail] Only mp4, mkv and ogg support embedding thumbnails");
        return 1;
    }
    0
}

fn emit_download_line(status: &str, downloaded: u64, total: u64, vcodec: &str) {
    println!("SDP_DL|{status}|{downloaded}|{total}|NA|1048576.0|3|{vcodec}");
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
}

/// A file name from the URL's last path segment, so ten items of one test do
/// not write to the same output.
///
/// Anything the OS can put in a file name is kept, accents included: the app
/// must not assume an ASCII title (spec §12.5 is the original's mistake), and
/// the encoding test needs a name the *console* code page would not survive.
fn safe_name(url: &str) -> String {
    let tail = url
        .split(['/', '?', '#'])
        .rfind(|part| !part.is_empty())
        .unwrap_or("fake");
    let cleaned: String = tail
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("Fake {cleaned} [fakeid].mkv")
}

/// The final-path line, written the way the real tool would write it.
///
/// Measured on Windows: when stdout is a **pipe**, yt-dlp writes the *console*
/// code page (cp1252 in the west, via `write_string`'s `out.encoding`), so
/// every character outside it is dropped and the path it prints stops naming
/// the file it just wrote — which is the D114 report: "yt-dlp reported … but
/// the file is not there", about a file sitting in the output folder, playable,
/// and never reproduced on Linux, where the pipe is UTF-8. `--encoding utf-8`
/// (spec §4.5) is what makes the tool write what our reader decodes.
///
/// The fake reproduces that on every OS — ASCII stands in for the code page:
/// dropping the character is the failure, whichever code page drops it — so a
/// missing flag is caught by the suite instead of by a user on Windows.
fn print_file_line(args: &[String], file: &Path) {
    let text = file.display().to_string();
    let forced_utf8 = args
        .windows(2)
        .any(|pair| pair[0] == "--encoding" && pair[1] == "utf-8");
    if forced_utf8 {
        println!("SDP_FILE|{text}");
        return;
    }
    let console_code_page: String = text.chars().filter(char::is_ascii).collect();
    println!("SDP_FILE|{console_code_page}");
}

fn paths_value(args: &[String], prefix: &str) -> Option<PathBuf> {
    args.windows(2)
        .filter(|pair| pair[0] == "--paths")
        .map(|pair| pair[1].clone())
        .find_map(|value| value.strip_prefix(prefix).map(PathBuf::from))
}

/// Where a download writes, from `--paths home:`.
///
/// `main` refuses a download invocation without it, so the `expect` documents
/// an invariant rather than a guess: falling back to the process's working
/// directory here is exactly the accident this file must not repeat.
fn home_dir(args: &[String]) -> PathBuf {
    paths_value(args, "home:")
        .filter(|path| !path.as_os_str().is_empty())
        .expect("main rejects a download without --paths home:")
}

fn sleep_forever() -> ! {
    loop {
        thread::sleep(Duration::from_millis(200));
    }
}

/// A small but complete video document: storyboard, audio-only format, two
/// video heights.
const VIDEO_JSON: &str = r#"{
  "_type": "video",
  "id": "fakeid",
  "title": "Fake title",
  "duration": 10.0,
  "extractor_key": "Youtube",
  "webpage_url": "https://www.youtube.com/watch?v=fakeid",
  "is_live": false,
  "formats": [
    {"format_id": "sb0", "ext": "mhtml", "protocol": "mhtml", "vcodec": "none", "acodec": "none"},
    {"format_id": "251", "ext": "webm", "vcodec": "none", "acodec": "opus", "protocol": "https"},
    {"format_id": "137", "ext": "mp4", "height": 1080, "width": 1920, "vcodec": "avc1", "acodec": "none", "protocol": "https"},
    {"format_id": "136", "ext": "mp4", "height": 720, "width": 1280, "vcodec": "avc1", "acodec": "none", "protocol": "https"},
    {"format_id": "18", "ext": "mp4", "height": 360, "width": 640, "vcodec": "avc1", "acodec": "mp4a", "protocol": "https"}
  ]
}
"#;
