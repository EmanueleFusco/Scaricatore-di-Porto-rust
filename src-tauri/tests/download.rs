//! Integration tests for the downloader (spec §5.2), against a local HTTP
//! server.
//!
//! No network, no real tool, and identical on Windows, macOS and Linux: the
//! server is a `std::net::TcpListener` on 127.0.0.1 that the test drives byte by
//! byte, which is what makes the interesting cases expressible —
//!
//! * **resume after a killed connection**: the server announces the full length,
//!   sends half the body and then closes. The second call must continue with
//!   `Range: bytes=N-` and produce a file that passes its checksum;
//! * **a server that ignores `Range`** must not make the client append a second
//!   copy of the prefix;
//! * **a wrong checksum** must leave no file behind, so the next attempt cannot
//!   resume from bytes that are known to be wrong.
//!
//! The tests use `Transport::loopback_http()`, which refuses any non-loopback
//! host. That is the only way plain HTTP is ever allowed, in code or here.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use scaricatore_di_porto_lib::services::download::{self, DownloadRequest, Transport};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// What the fake server should do with one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    /// Send the whole body.
    Whole,
    /// Send `cut` bytes, then close the connection without finishing.
    ///
    /// `times` is how many *requests* are truncated; a flaky link that keeps
    /// dropping is the case where the client has to give up, and a link that
    /// recovers once is the case where resume has to work.
    Truncated { cut: usize, times: usize },
    /// Answer 200 with the whole body even when a range was requested.
    IgnoreRange,
    /// Answer 404.
    NotFound,
}

/// A one-file HTTP/1.1 server on loopback, shut down when dropped.
struct Server {
    port: u16,
    requests: Arc<AtomicUsize>,
    ranges_seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(body: Vec<u8>, behaviour: Behaviour) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let body = Arc::new(body);
        let requests = Arc::new(AtomicUsize::new(0));
        let ranges_seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let thread_body = Arc::clone(&body);
        let thread_requests = Arc::clone(&requests);
        let thread_ranges = Arc::clone(&ranges_seen);
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            for stream in listener.incoming() {
                if thread_stop.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                serve_one(
                    stream,
                    &thread_body,
                    behaviour,
                    &thread_requests,
                    &thread_ranges,
                );
            }
        });

        Self {
            port,
            requests,
            ranges_seen,
            stop,
            handle: Some(handle),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }

    /// The `Range` header of the n-th request (1-based), or `""`.
    fn range_of(&self, nth: usize) -> String {
        self.ranges_seen
            .lock()
            .expect("ranges")
            .get(nth - 1)
            .cloned()
            .unwrap_or_default()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock the accept loop with one throwaway connection.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve_one(
    mut stream: TcpStream,
    body: &[u8],
    behaviour: Behaviour,
    requests: &AtomicUsize,
    ranges_seen: &Arc<Mutex<Vec<String>>>,
) {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("read timeout");
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut range = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Err(_) | Ok(0) => break,
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("range:") {
            range = Some(value.trim().to_owned());
        }
    }
    // Drain whatever is still queued in the socket. Closing a socket with unread
    // data sends RST, and an RST makes the client **lose the body it already
    // received** — which is indistinguishable from a bug in the downloader, and
    // cost one debugging round before it was spotted here.
    let mut discard = [0_u8; 1024];
    while matches!(reader.get_mut().read(&mut discard), Ok(read) if read > 0) {}

    let offset: usize = range
        .as_deref()
        .and_then(|value| value.strip_prefix("bytes="))
        .and_then(|value| value.split('-').next())
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);

    // Record what we were asked, for the assertions in the tests.
    ranges_seen
        .lock()
        .expect("ranges")
        .push(range.clone().unwrap_or_default());

    let _ = requests.fetch_add(1, Ordering::Relaxed);
    let ordinal = requests.load(Ordering::Relaxed);

    let (status, slice): (&str, &[u8]) = if behaviour == Behaviour::NotFound {
        ("404 Not Found", &b""[..])
    } else if offset > 0 && behaviour == Behaviour::IgnoreRange {
        ("200 OK", body)
    } else if offset > 0 {
        ("206 Partial Content", &body[offset.min(body.len())..])
    } else {
        ("200 OK", body)
    };

    let header = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Length: {}\r\n\
         Accept-Ranges: bytes\r\n\
         Connection: close\r\n\r\n",
        slice.len()
    );
    if stream.write_all(header.as_bytes()).is_err() {
        return;
    }
    let _ = stream.flush();

    let cut_now = match behaviour {
        // Only the first `times` requests are cut: a link that recovers once is
        // the case where resume has to work inside a single call.
        Behaviour::Truncated { cut, times } => ordinal <= times && slice.len() > cut,
        _ => false,
    };
    if cut_now {
        let cut = match behaviour {
            Behaviour::Truncated { cut, .. } => cut.min(slice.len()),
            _ => 0,
        };
        let _ = stream.write_all(&slice[..cut]);
        let _ = stream.flush();
        // A clean half-close: the client sees EOF *after* the bytes it was sent.
        // Dropping the socket outright would RST and throw those bytes away.
        let _ = stream.shutdown(std::net::Shutdown::Write);
    } else {
        let _ = stream.write_all(slice);
        let _ = stream.flush();
        let _ = stream.shutdown(std::net::Shutdown::Write);
    }
}

/// A body big enough that the truncation is unambiguous.
fn payload() -> Vec<u8> {
    // 64 KiB of a repeating pattern: different on every offset, so a doubled or
    // missing prefix cannot pass the checksum by accident.
    (0..64 * 1024)
        .map(|index| ((index * 31 + index / 251) % 251) as u8)
        .collect()
}

fn sha256_of(bytes: &[u8]) -> String {
    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    sha2::Digest::update(&mut hasher, bytes);
    download::hex(&sha2::Digest::finalize(hasher).into())
}

fn request(url: &str, destination: &Path) -> DownloadRequest {
    DownloadRequest::new(url, destination).with_checksum(sha256_of(&payload()))
}

fn temp_paths(dir: &TempDir) -> (PathBuf, PathBuf) {
    (
        dir.path().join("out").join("tool"),
        dir.path().join("out").join("tool.part"),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_complete_download_is_verified_and_installed_under_its_real_name() {
    let dir = TempDir::new().expect("temp dir");
    let server = Server::start(payload(), Behaviour::Whole);
    let (destination, part) = temp_paths(&dir);

    let mut request = request(&server.url("/tool"), &destination);
    request.transport = Transport::loopback_http();

    let mut seen = Vec::new();
    let outcome = download::download(&request, &CancellationToken::new(), |p| seen.push(p))
        .await
        .expect("download");

    assert!(outcome.sha256 == sha256_of(&payload()));
    assert!(destination.is_file());
    assert_eq!(fs::read(&destination).expect("read"), payload());
    assert!(!part.exists(), "the .part file must not survive");
    assert!(!outcome.resumed);
    assert!(!seen.is_empty(), "progress must be reported");
    let last = seen.last().expect("a progress tick");
    assert_eq!(last.received, payload().len() as u64);
    assert_eq!(last.total, Some(payload().len() as u64));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&destination)
            .expect("stat")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "chmod +x is part of installing");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_killed_connection_is_resumed_and_the_result_is_byte_exact() {
    let dir = TempDir::new().expect("temp dir");
    // The link drops the first connection and recovers on the second.
    let cut = 20_000;
    let server = Server::start(payload(), Behaviour::Truncated { cut, times: 1 });
    let (destination, part) = temp_paths(&dir);

    let mut request = request(&server.url("/tool"), &destination);
    request.transport = Transport::loopback_http();

    let outcome = download::download(&request, &CancellationToken::new(), |_| {})
        .await
        .expect("the second attempt must resume and finish");

    assert_eq!(server.requests(), 2, "one attempt was dropped");
    assert!(
        server.range_of(1).is_empty(),
        "the first request cannot ask for a range"
    );
    assert_eq!(
        server.range_of(2),
        format!("bytes={cut}-"),
        "the retry must continue from what is on disk"
    );
    assert!(outcome.resumed);
    assert_eq!(
        fs::read(&destination).expect("read"),
        payload(),
        "the body must appear exactly once, with no gap and no overlap"
    );
    assert_eq!(outcome.sha256, sha256_of(&payload()));
    assert!(!part.exists(), "the .part file must not survive a success");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_that_never_recovers_gives_up_but_still_makes_progress() {
    let dir = TempDir::new().expect("temp dir");
    let cut = 20_000;
    // Every request is cut: the client must stop retrying rather than loop.
    let server = Server::start(payload(), Behaviour::Truncated { cut, times: 99 });
    let (destination, part) = temp_paths(&dir);

    let mut request = request(&server.url("/tool"), &destination);
    request.transport = Transport::loopback_http();

    let err = download::download(&request, &CancellationToken::new(), |_| {})
        .await
        .expect_err("a link that never finishes must fail");
    assert!(err.to_string().contains("connection broke"), "{err}");
    assert_eq!(server.requests(), 3, "a bounded number of attempts");
    assert!(!destination.exists(), "nothing is installed");
    assert!(
        part.is_file(),
        "the partial download is kept so the next attempt can resume"
    );

    // Three attempts of `cut` bytes each, and the file is exactly the first
    // `3 * cut` bytes of the payload: each attempt added new data at the end.
    // A client that restarted, or that appended to a stale offset, would produce
    // a different — and wrong — prefix here.
    let kept = fs::read(&part).expect("read part");
    assert_eq!(kept.len(), 3 * cut);
    assert_eq!(kept, payload()[..kept.len()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_interrupted_before_the_call_resumes_across_calls() {
    let dir = TempDir::new().expect("temp dir");
    let cut = 20_000;
    let broken = Server::start(payload(), Behaviour::Truncated { cut, times: 99 });
    let (destination, part) = temp_paths(&dir);

    // A first call that gives up, as above.
    let mut first = request(&broken.url("/tool"), &destination);
    first.transport = Transport::loopback_http();
    assert!(
        download::download(&first, &CancellationToken::new(), |_| {})
            .await
            .is_err()
    );
    let resumed_from = fs::metadata(&part).expect("stat").len();
    drop(broken);

    // The user presses Retry. A *new* call must continue from the same file.
    let server = Server::start(payload(), Behaviour::Whole);
    let mut second = request(&server.url("/tool"), &destination);
    second.transport = Transport::loopback_http();
    let outcome = download::download(&second, &CancellationToken::new(), |_| {})
        .await
        .expect("resumed download");

    assert!(outcome.resumed);
    assert_eq!(
        server.range_of(1),
        format!("bytes={resumed_from}-"),
        "a fresh call must resume too, not only the in-call retry"
    );
    assert_eq!(fs::read(&destination).expect("read"), payload());
    assert_eq!(outcome.sha256, sha256_of(&payload()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_ignores_the_range_restarts_instead_of_doubling_the_prefix() {
    let dir = TempDir::new().expect("temp dir");
    let server = Server::start(payload(), Behaviour::IgnoreRange);
    let (destination, part) = temp_paths(&dir);

    // Pretend a previous attempt left half a file behind.
    fs::create_dir_all(part.parent().expect("parent")).expect("mkdir");
    fs::write(&part, &payload()[..20_000]).expect("write part");

    let mut request = request(&server.url("/tool"), &destination);
    request.transport = Transport::loopback_http();
    let outcome = download::download(&request, &CancellationToken::new(), |_| {})
        .await
        .expect("download");

    assert!(!outcome.resumed, "a 200 answer means start again");
    assert_eq!(
        fs::read(&destination).expect("read"),
        payload(),
        "the file must be the body exactly once, not the body plus a prefix"
    );
    assert_eq!(outcome.sha256, sha256_of(&payload()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_checksum_leaves_nothing_behind() {
    let dir = TempDir::new().expect("temp dir");
    let server = Server::start(payload(), Behaviour::Whole);
    let (destination, part) = temp_paths(&dir);

    let mut request = DownloadRequest::new(server.url("/tool"), &destination)
        // A checksum that is well-formed but not the file's.
        .with_checksum("0".repeat(64));
    request.transport = Transport::loopback_http();

    let err = download::download(&request, &CancellationToken::new(), |_| {})
        .await
        .expect_err("a checksum mismatch must fail");
    assert!(err.to_string().contains("checksum mismatch"), "{err}");
    assert!(
        !destination.exists(),
        "a rejected file must never be installed"
    );
    assert!(
        !part.exists(),
        "the bad bytes are deleted, so the next attempt cannot resume from them"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_download_never_installs_anything() {
    let dir = TempDir::new().expect("temp dir");
    let server = Server::start(payload(), Behaviour::Whole);
    let (destination, _part) = temp_paths(&dir);

    let token = CancellationToken::new();
    let cancelling = token.clone();
    let mut request = request(&server.url("/tool"), &destination);
    request.transport = Transport::loopback_http();

    let err = download::download(&request, &token, move |progress| {
        if progress.received > 0 {
            cancelling.cancel();
        }
    })
    .await
    .expect_err("cancelling must stop the download");
    assert!(err.to_string().contains("cancelled"), "{err}");
    assert!(!destination.exists(), "nothing is installed");
    // Whether the part file exists depends on where the cancel landed; what
    // must never happen is an installed file.
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_file_answers_404_and_is_not_installed() {
    let dir = TempDir::new().expect("temp dir");
    let server = Server::start(payload(), Behaviour::NotFound);
    let (destination, part) = temp_paths(&dir);

    let mut request = request(&server.url("/gone"), &destination);
    request.transport = Transport::loopback_http();

    let err = download::download(&request, &CancellationToken::new(), |_| {})
        .await
        .expect_err("404 must fail");
    assert!(err.to_string().contains("404"), "{err}");
    assert!(!destination.exists());
    assert!(!part.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_default_transport_refuses_plain_http() {
    let dir = TempDir::new().expect("temp dir");
    let (destination, _) = temp_paths(&dir);
    // No loopback exception: the app's own default must say no, even to
    // 127.0.0.1.
    let request = DownloadRequest::new("http://127.0.0.1:1/tool", &destination);
    let err = download::download(&request, &CancellationToken::new(), |_| {})
        .await
        .expect_err("plain http must be refused");
    assert!(
        err.to_string().contains("only https:// is allowed"),
        "{err}"
    );
    assert!(!destination.exists());
}
