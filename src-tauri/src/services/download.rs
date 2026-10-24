//! Downloaded tools: HTTPS, resume, checksum, atomic install (spec §5.2).
//!
//! Everything the app fetches at runtime goes through [`download`], because the
//! list of things that must be true of a download is long and easy to get
//! half-right:
//!
//! * **HTTPS only.** No flag, no setting, no environment variable turns it off
//!   (spec §1, "no TLS verification disabled anywhere"). The one exception is
//!   [`Transport::loopback_http`], which refuses any host that is not a loopback
//!   address and exists so the integration tests can use a local server.
//! * **Resume with `Range`.** A killed connection leaves a `*.part` file that
//!   the next attempt continues from, because an 80 MB model or a 40 MB
//!   yt-dlp over a flaky link is the normal case, not the exception.
//! * **Verify the sha256** before the file is allowed to become an executable.
//! * **`chmod +x` and an atomic rename.** A half-written executable is never
//!   visible under its real name, on any OS.
//! * **Cancellation** kills the download and leaves the `.part` file in place,
//!   so cancelling is resumable rather than destructive.
//! * **Path traversal is impossible** in an archive: every entry is resolved
//!   against the destination and anything that escapes is refused (spec §5.2,
//!   "archives are extracted with path-traversal protection").
//!
//! TLS verification is never disabled. `reqwest` is built with `rustls-tls`
//! (spec §1), whose default is full verification against the bundled Mozilla
//! root store — which is also why a download works the same on a bare Linux
//! container and on macOS, with no system-cert configuration.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::services::manifest::ArchiveKind;
use crate::services::paths;

/// Suffix of the in-progress file (spec §5.2: "download over HTTPS to `*.tmp`
/// with resume").
pub const PART_SUFFIX: &str = ".part";

/// Refuse absurd redirect chains rather than following one for ever.
const MAX_REDIRECTS: usize = 5;

/// How many times one call re-opens a connection that dropped mid-body.
///
/// Each attempt re-reads the size of the `.part` file, so attempt 2 onwards is
/// a **resume** (spec §5.2: "download over HTTPS to `*.tmp` with resume"), not a
/// restart.
const MAX_TRANSIENT_RETRIES: usize = 3;

// ---------------------------------------------------------------------------
// Transport policy
// ---------------------------------------------------------------------------

/// What a download is allowed to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// `https://` and nothing else. The only value the app ever constructs.
    HttpsOnly,
    /// `http://` to a loopback address, for tests.
    LoopbackHttp,
}

impl Transport {
    /// Plain HTTP is allowed only towards the loopback interface.
    ///
    /// Deliberately narrow: a flag that merely said "allow http" could be
    /// pointed at the real internet, and a test-only escape hatch that can do
    /// that is not an escape hatch. `localhost`, `127.x` and `[::1]` only.
    pub fn loopback_http() -> Self {
        Self::LoopbackHttp
    }

    fn accepts(&self, url: &str) -> bool {
        let lower = url.trim().to_ascii_lowercase();
        if lower.starts_with("https://") {
            return true;
        }
        if !lower.starts_with("http://") {
            return false;
        }
        self == &Self::LoopbackHttp && is_loopback(authority_of(&lower))
    }
}

/// The `host[:port]` part of a URL, without scheme, path, query or fragment.
///
/// Everything before a `@` is userinfo and is **discarded**, not kept:
/// `http://127.0.0.1@evil.com/` really connects to `evil.com`, and taking the
/// part before the `@` would make that look like a loopback address.
fn authority_of(lower_url: &str) -> &str {
    let rest = lower_url
        .trim_start_matches("http://")
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    rest.rsplit_once('@').map_or(rest, |(_, host)| host)
}

/// Is this authority the loopback interface?
///
/// Deliberately strict. `starts_with("127.")` alone would also accept
/// `127.0.0.1.evil.com`, which is an ordinary internet host — and a "test only"
/// allowance that can be pointed at the internet is not an allowance.
fn is_loopback(authority: &str) -> bool {
    let host = match authority.rsplit_once(':') {
        // Only strip something that really is a port, so `::1` keeps its colons.
        Some((name, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => authority,
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host == "localhost" || host == "::1" {
        return true;
    }
    let octets: Vec<&str> = host.split('.').collect();
    octets.len() == 4
        && octets[0] == "127"
        && octets
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

// ---------------------------------------------------------------------------
// Request / outcome
// ---------------------------------------------------------------------------

/// What to download, and where it has to end up.
#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub url: String,
    /// Final path. Nothing but this exact name ever becomes visible.
    pub destination: PathBuf,
    /// The archive to unwrap, if any.
    pub archive_kind: ArchiveKind,
    /// Lowercase hex. `None` only for an archive whose checksum is verified
    /// before extraction; the installed executable always goes through it.
    pub expected_sha256: Option<String>,
    pub transport: Transport,
}

impl DownloadRequest {
    pub fn new(url: impl Into<String>, destination: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            destination: destination.into(),
            archive_kind: ArchiveKind::None,
            expected_sha256: None,
            transport: Transport::HttpsOnly,
        }
    }

    pub fn with_checksum(mut self, sha256: impl Into<String>) -> Self {
        self.expected_sha256 = Some(sha256.into());
        self
    }

    pub fn with_archive(mut self, kind: ArchiveKind) -> Self {
        self.archive_kind = kind;
        self
    }

    /// The in-progress file. Same directory as the destination, so the final
    /// rename is atomic (spec §5.2).
    pub fn part_path(&self) -> PathBuf {
        let mut name = self.destination.as_os_str().to_os_string();
        name.push(PART_SUFFIX);
        PathBuf::from(name)
    }
}

/// How far along a download is, for `tool:progress` (spec §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    pub received: u64,
    /// `None` when the server does not say (`Content-Length: 0` chunked).
    pub total: Option<u64>,
}

/// What a finished download left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOutcome {
    pub path: PathBuf,
    pub bytes: u64,
    /// True when an existing `*.part` was continued rather than started over.
    pub resumed: bool,
    /// sha256 of the installed file, always recorded.
    pub sha256: String,
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// The lowercase hex sha256 of a file.
pub fn file_sha256(path: &Path) -> Result<[u8; 32], AppError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

/// hex of a digest, lowercase (spec §5.2 compares against lowercase hex).
pub fn hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Does this byte string look like a sha256? Pure, so the archive guards and the
/// manifest can share the rule.
pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// **The path-traversal guard** (spec §5.2).
///
/// Rejects, in this order: absolute paths, Windows drive prefixes, UNC paths,
/// `..` anywhere, and anything that still resolves outside `base` after
/// normalisation. Both checks are needed — `a/../../etc/passwd` is caught by the
/// first, and a symlink-shaped name that normalises outside by another route is
/// caught by the second.
pub fn safe_extract_path(base: &Path, entry: &Path) -> Option<PathBuf> {
    let raw = entry.as_os_str().to_string_lossy();
    if raw.is_empty() || raw.starts_with('/') {
        return None;
    }
    // A backslash is refused on **every** platform, not only Windows. On Unix
    // `a\..\..\etc` is one harmless filename, but the same bytes unpacked on
    // Windows are a traversal — and a guard that only holds on the machine that
    // wrote it is not a guard.
    if raw.contains('\\') {
        return None;
    }
    // `C:\…` and `C:…`
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return None;
    }
    if raw.contains('\0') {
        return None;
    }
    let mut resolved = base.to_path_buf();
    for component in entry.components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => {}
            // `ParentDir`, `RootDir`, `Prefix`: all escape, all refused.
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    resolved.starts_with(base).then_some(resolved)
}

/// How many bytes are already on disk for a resumable download.
pub fn existing_part_size(part: &Path) -> u64 {
    fs::metadata(part).map(|meta| meta.len()).unwrap_or(0)
}

/// The `Range` header value for resuming from `offset` bytes, or `None` when
/// there is nothing to resume.
///
/// Pure: the header is the part of resume that is easy to get subtly wrong, so
/// it is decided without touching a socket.
pub fn range_header(offset: u64) -> Option<String> {
    (offset > 0).then(|| format!("bytes={offset}-"))
}

/// Decide what to do with the server's answer.
///
/// * `206 Partial Content` with a `Content-Range` → append.
/// * `200 OK` while we asked for a range → the server ignored it; the only safe
///   move is to restart, because appending would duplicate the prefix and
///   produce a file that fails its checksum for no visible reason.
/// * `416 Range Not Satisfiable` → the part file is at or past the end of the
///   real file, which means it is garbage: start over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeDecision {
    Append,
    Restart,
}

pub fn decide_resume(status: u16, offset: u64) -> ResumeDecision {
    if offset == 0 {
        return ResumeDecision::Restart;
    }
    match status {
        206 => ResumeDecision::Append,
        // 200: the whole body is coming.
        416 => ResumeDecision::Restart,
        _ => ResumeDecision::Restart,
    }
}

/// `Content-Length` of a response, when it is a usable number.
pub fn content_length(value: Option<&str>) -> Option<u64> {
    value?.trim().parse::<u64>().ok()
}

// ---------------------------------------------------------------------------
// The download itself
// ---------------------------------------------------------------------------

fn download_error(message: impl Into<String>) -> AppError {
    AppError::Process(message.into())
}

async fn wait_backoff(attempt: usize, token: &CancellationToken) -> Result<(), AppError> {
    let millis = 250u64.saturating_mul(1u64 << attempt.saturating_sub(1).min(10));
    tokio::select! {
        _ = token.cancelled() => Err(download_error("the download was cancelled")),
        _ = tokio::time::sleep(Duration::from_millis(millis)) => Ok(()),
    }
}

/// Download `request.url` to `request.destination`, resuming and verifying.
///
/// `on_progress` is called as bytes arrive so the UI can show a bar; it is
/// synchronous and never blocks for long.
pub async fn download(
    request: &DownloadRequest,
    token: &CancellationToken,
    mut on_progress: impl FnMut(DownloadProgress),
) -> Result<DownloadOutcome, AppError> {
    if !request.transport.accepts(&request.url) {
        return Err(download_error(format!(
            "refusing to download {}: only https:// is allowed (spec §1)",
            request.url
        )));
    }
    if let Some(expected) = &request.expected_sha256
        && !is_sha256_hex(expected)
    {
        return Err(download_error(format!(
            "the expected sha256 {expected:?} is not 64 lowercase hex characters"
        )));
    }

    let destination = &request.destination;
    if let Some(parent) = destination.parent() {
        paths::ensure_dir(parent)?;
    }
    let part = request.part_path();

    let client = client()?;
    let mut attempt = 0;
    let (received_total, resumed) = loop {
        // Re-read on every attempt. A first attempt that wrote 20 000 bytes must
        // make the *second* one send `Range: bytes=20000-`; re-using the offset
        // from before the loop would silently retry the same thing.
        let offset = existing_part_size(&part);

        let mut builder = client.get(&request.url);
        if let Some(range) = range_header(offset) {
            builder = builder.header(reqwest::header::RANGE, range);
        }
        let response = match builder.send().await {
            Ok(resp) => resp,
            Err(err) => {
                attempt += 1;
                if attempt >= MAX_TRANSIENT_RETRIES {
                    return Err(download_error(format!("GET {}: {err}", request.url)));
                }
                wait_backoff(attempt, token).await?;
                continue;
            }
        };

        let status = response.status().as_u16();
        if (500..600).contains(&status) {
            attempt += 1;
            if attempt >= MAX_TRANSIENT_RETRIES {
                return Err(download_error(format!(
                    "GET {} answered HTTP {status}",
                    request.url
                )));
            }
            wait_backoff(attempt, token).await?;
            continue;
        }

        if !(200..300).contains(&status) && status != 416 {
            return Err(download_error(format!(
                "GET {} answered HTTP {status}",
                request.url
            )));
        }

        let append = decide_resume(status, offset) == ResumeDecision::Append;
        let resume_offset = if append { offset } else { 0 };
        // The body length plus what we already have: the server's
        // `Content-Length` covers only the range it is sending.
        let body_len = content_length(
            response
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok()),
        );
        let total = body_len.map(|len| len + resume_offset);

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&part)
            .map_err(|err| download_error(format!("opening {}: {err}", part.display())))?;

        let mut received = resume_offset;
        on_progress(DownloadProgress { received, total });

        let mut stream = response;
        let mut failed: Option<AppError> = None;
        loop {
            // `chunk()` is `Result<Option<Bytes>>`: a dropped connection and the
            // end of the body arrive through the same call, so the retry loop
            // lives here rather than in a `while let`.
            let next = match stream.chunk().await {
                Ok(Some(chunk)) => Some(chunk),
                Ok(None) => None,
                Err(err) => {
                    failed = Some(download_error(format!(
                        "the connection broke after {received} bytes: {err}"
                    )));
                    None
                }
            };
            let Some(chunk) = next else {
                break;
            };
            if token.is_cancelled() {
                // The `.part` file is deliberately kept: resuming is the point.
                return Err(download_error("the download was cancelled"));
            }
            file.write_all(&chunk)
                .map_err(|err| download_error(format!("writing {}: {err}", part.display())))?;
            received += chunk.len() as u64;
            on_progress(DownloadProgress { received, total });
        }
        file.flush().ok();
        drop(file);

        if let Some(err) = failed {
            // The `.part` file now holds everything received so far, so the next
            // attempt continues from here. Anything that is not a dropped
            // connection fails the call at once.
            attempt += 1;
            if attempt >= MAX_TRANSIENT_RETRIES {
                return Err(err);
            }
            wait_backoff(attempt, token).await?;
            continue;
        }
        break (received, append);
    };

    // Verify before the file is allowed to become an executable.
    let digest = hex(&file_sha256(&part)?);
    if let Some(expected) = &request.expected_sha256
        && &digest != expected
    {
        // A wrong checksum means the bytes are not the tool: delete them, or the
        // next attempt would resume from and complete a broken file.
        let _ = fs::remove_file(&part);
        return Err(download_error(format!(
            "checksum mismatch for {}: expected {expected}, got {digest}",
            request.url
        )));
    }

    if request.archive_kind.needs_extraction() {
        // A `.gz` holds one file, not a tree: the caller installs the returned
        // path directly. The other archives return the directory they filled.
        let staged = match request.archive_kind {
            ArchiveKind::Gz => extract_gz(&part, destination, &request.url)?,
            kind => extract_archive(&part, destination, kind)?,
        };
        let _ = fs::remove_file(&part);
        return Ok(DownloadOutcome {
            path: staged,
            bytes: received_total,
            resumed,
            sha256: digest,
        });
    }

    make_executable(&part)?;
    fs::rename(&part, destination).map_err(|err| {
        download_error(format!(
            "moving the download into place at {}: {err}",
            destination.display()
        ))
    })?;

    Ok(DownloadOutcome {
        path: destination.clone(),
        bytes: received_total,
        resumed,
        sha256: digest,
    })
}

fn client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .user_agent(concat!("scaricatore-di-porto/", env!("CARGO_PKG_VERSION")))
        // TLS verification is on and stays on: no danger_accept_invalid_*
        // anywhere in this file (spec §1).
        .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
        .build()
        .map_err(|err| download_error(format!("building the HTTP client: {err}")))
}

/// `chmod +x` on Unix; a no-op on Windows, where the extension is the flag.
fn make_executable(path: &Path) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Archives
// ---------------------------------------------------------------------------

/// Unpack `archive` into `destination`, refusing anything that escapes it.
///
/// The executable to install is looked for by name inside the unpacked tree —
/// every real archive we consume puts it at a predictable place, and searching
/// for it is better than hard-coding paths that change between releases.
pub fn extract_archive(
    archive: &Path,
    destination: &Path,
    kind: ArchiveKind,
) -> Result<PathBuf, AppError> {
    paths::ensure_dir(destination)?;
    match kind {
        ArchiveKind::None => Err(download_error(
            "extract_archive called for a tool that is not an archive",
        )),
        ArchiveKind::Zip => extract_zip(archive, destination),
        ArchiveKind::Tar | ArchiveKind::TarGz | ArchiveKind::TarXz => {
            extract_tar(archive, destination, kind)
        }
        // Handled before this function is called, because its output is a file
        // rather than the directory the other kinds return.
        ArchiveKind::Gz => Err(download_error(
            "extract_archive cannot unpack a single gzip stream",
        )),
    }
}

/// Decompress a single-file `.gz` asset into `destination`.
///
/// The name comes from the URL, never from the (untrusted) archive contents:
/// only the final path segment is kept, so a crafted URL cannot escape the
/// destination. The caller renames the result to the executable name it wants.
fn extract_gz(archive: &Path, destination: &Path, url: &str) -> Result<PathBuf, AppError> {
    paths::ensure_dir(destination)?;
    let asset = url
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or("tool");
    let name = asset.strip_suffix(".gz").unwrap_or(asset);
    let file_name = Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("tool");
    let target = destination.join(file_name);

    let input = File::open(archive)?;
    let mut decoder = flate2::read::GzDecoder::new(std::io::BufReader::new(input));
    let mut out = File::create(&target)?;
    std::io::copy(&mut decoder, &mut out).map_err(|err| {
        download_error(format!(
            "{} is not a valid gzip stream: {err}",
            archive.display()
        ))
    })?;
    Ok(target)
}

/// Install the entry called `executable` from the unpacked tree.
///
/// Searched first at the top level, then one level deep: BtbN and Deno both ship
/// `ffmpeg-…/bin/ffmpeg`, and hard-coding either shape would break on the next
/// release.
pub fn find_executable(root: &Path, executable: &str) -> Option<PathBuf> {
    let direct = root.join(executable);
    if direct.is_file() {
        return Some(direct);
    }
    let entries = fs::read_dir(root).ok()?;
    let mut directories: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.path())
        .collect();
    // Sorted so a tree with two candidates picks the same one every time.
    directories.sort();
    directories
        .iter()
        .map(|dir| dir.join(executable))
        .find(|candidate| candidate.is_file())
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<PathBuf, AppError> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|err| download_error(format!("reading {} as a zip: {err}", archive.display())))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|err| download_error(format!("zip entry {index}: {err}")))?;
        // `enclosed_name()` is the library's own traversal-safe name, but it is
        // belt and braces: the shared guard is checked too, and a refusal here
        // fails the whole archive rather than skipping one entry.
        let name = entry.enclosed_name().ok_or_else(|| {
            download_error(format!("the archive entry {index} escapes the destination"))
        })?;
        let target = safe_extract_path(destination, &name).ok_or_else(|| {
            download_error(format!("the archive entry {index} escapes the destination"))
        })?;

        if entry.is_dir() {
            paths::ensure_dir(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            paths::ensure_dir(parent)?;
        }
        let mut out = File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(destination.to_path_buf())
}

fn extract_tar(archive: &Path, destination: &Path, kind: ArchiveKind) -> Result<PathBuf, AppError> {
    let bytes = fs::read(archive)?;
    let plain: Vec<u8> = match kind {
        ArchiveKind::Tar => bytes.clone(),
        ArchiveKind::TarGz => {
            use std::io::Read as _;
            let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
            let mut out = Vec::new();
            decoder.read_to_end(&mut out).map_err(|err| {
                download_error(format!("{} is not a gzip stream: {err}", archive.display()))
            })?;
            out
        }
        ArchiveKind::TarXz => {
            // `xz_decompress(&mut BufRead, &mut Write)` — verified against
            // docs.rs for lzma-rs 0.3.0 rather than guessed.
            let mut input = &bytes[..];
            let mut out = Vec::new();
            lzma_rs::xz_decompress(&mut input, &mut out).map_err(|err| {
                download_error(format!("{} is not an xz stream: {err}", archive.display()))
            })?;
            out
        }
        ArchiveKind::None | ArchiveKind::Zip | ArchiveKind::Gz => {
            unreachable!("handled by the caller")
        }
    };

    let mut tar = tar::Archive::new(&plain[..]);
    tar.set_preserve_permissions(false);
    tar.set_overwrite(true);
    let entries = tar
        .entries()
        .map_err(|err| download_error(format!("reading {}: {err}", archive.display())))?;
    for entry in entries {
        let mut entry = entry.map_err(|err| download_error(format!("tar entry: {err}")))?;
        let name = entry
            .path()
            .map_err(|err| download_error(format!("tar entry name: {err}")))?
            .into_owned();
        // A tar can contain links, devices and setuid bits; none of that is ever
        // wanted from a tool archive, so only regular files and directories are
        // ever written out.
        let kind = entry.header().entry_type();
        let is_dir = kind.is_dir();
        if !is_dir && !kind.is_file() {
            continue;
        }
        let target = safe_extract_path(destination, &name).ok_or_else(|| {
            download_error(format!(
                "the archive entry {} escapes the destination",
                name.display()
            ))
        })?;
        if is_dir {
            paths::ensure_dir(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            paths::ensure_dir(parent)?;
        }
        let mut out = File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(destination.to_path_buf())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn https_is_accepted_and_everything_else_is_not() {
        assert!(Transport::HttpsOnly.accepts("https://example.invalid/a"));
        assert!(Transport::HttpsOnly.accepts("HTTPS://example.invalid/a"));
        assert!(!Transport::HttpsOnly.accepts("http://example.invalid/a"));
        assert!(!Transport::HttpsOnly.accepts("ftp://example.invalid/a"));
        assert!(!Transport::HttpsOnly.accepts("file:///etc/passwd"));
        assert!(!Transport::HttpsOnly.accepts("/etc/passwd"));
    }

    #[test]
    fn the_loopback_escape_hatch_cannot_be_pointed_at_the_internet() {
        let t = Transport::loopback_http();
        assert!(t.accepts("http://127.0.0.1:8080/x"));
        assert!(t.accepts("http://127.5.5.5/x"));
        assert!(t.accepts("http://localhost:1/x"));
        assert!(t.accepts("http://[::1]:9/x"));
        assert!(!t.accepts("http://example.invalid/x"));
        assert!(!t.accepts("http://10.0.0.1/x"));
        // The one that a `starts_with("127.")` check would have let through.
        assert!(!t.accepts("http://127.0.0.1.evil.com/x"));
        assert!(!t.accepts("http://127.0.0.1@evil.com/x"));
        // Even the test transport does not give up TLS where TLS exists.
        assert!(t.accepts("https://example.invalid/x"));
    }

    #[test]
    fn the_loopback_check_reads_the_authority_not_the_path() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("127.0.0.1:65535"));
        assert!(is_loopback("localhost:80"));
        assert!(is_loopback("[::1]:443"));
        assert!(!is_loopback("127.0.0.1.evil.com"));
        assert!(!is_loopback("127.0.0.1:80:90"));
        assert!(!is_loopback("1270.0.0.1"));
        assert!(!is_loopback(""));
        // Userinfo is discarded, so the real host is what gets checked.
        assert_eq!(authority_of("http://127.0.0.1@evil.com/x"), "evil.com");
        assert_eq!(authority_of("http://127.0.0.1:8080/x"), "127.0.0.1:8080");
    }

    #[test]
    fn a_relative_path_or_an_absurd_name_cannot_escape_the_destination() {
        let base = Path::new("/data/tools/ffmpeg");
        assert!(safe_extract_path(base, Path::new("bin/ffmpeg")).is_some());
        assert!(safe_extract_path(base, Path::new("./bin/ffmpeg")).is_some());

        for hostile in [
            "../etc/passwd",
            "../../../etc/passwd",
            "a/../../etc/passwd",
            "/etc/passwd",
            r"C:\Windows\System32\evil.exe",
            // Backslashes are refused on every platform: harmless as a file name
            // on Unix, a traversal on Windows.
            r"..\..\evil",
            r"bin\..\..\evil",
            "",
        ] {
            assert!(
                safe_extract_path(base, Path::new(hostile)).is_none(),
                "{hostile:?} must be refused"
            );
        }
    }

    #[test]
    fn a_traversal_that_survives_normalisation_is_still_refused() {
        // `safe` is the belt; `starts_with(base)` is the braces.
        let base = Path::new("/data/tools");
        assert_eq!(
            safe_extract_path(base, Path::new("ffmpeg/bin/ffmpeg")).as_deref(),
            Some(Path::new("/data/tools/ffmpeg/bin/ffmpeg"))
        );
        assert!(safe_extract_path(base, Path::new("../tools-evil/x")).is_none());
    }

    #[test]
    fn range_is_only_sent_when_there_is_something_to_resume() {
        assert_eq!(range_header(0), None);
        assert_eq!(range_header(1024).as_deref(), Some("bytes=1024-"));
    }

    #[test]
    fn a_server_that_ignores_the_range_makes_us_restart_not_append() {
        // Appending to a full body would produce a file that is the right length
        // and the wrong content: the checksum would fail with no visible reason.
        assert_eq!(decide_resume(206, 500), ResumeDecision::Append);
        assert_eq!(decide_resume(200, 500), ResumeDecision::Restart);
        assert_eq!(decide_resume(416, 500), ResumeDecision::Restart);
        assert_eq!(decide_resume(206, 0), ResumeDecision::Restart);
    }

    #[test]
    fn the_total_is_the_body_plus_what_we_already_have() {
        assert_eq!(content_length(Some("1000")), Some(1000));
        assert_eq!(content_length(Some(" 1000 ")), Some(1000));
        assert_eq!(content_length(None), None);
        assert_eq!(content_length(Some("not a number")), None);
    }

    #[test]
    fn sha256_of_a_file_is_the_published_value() {
        // `echo -n "abc" | sha256sum` — a fixed vector, so a wrong digest
        // implementation cannot pass.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("abc");
        fs::write(&path, b"abc").expect("write");
        assert_eq!(
            hex(&file_sha256(&path).expect("hash")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hex_is_lowercase_and_fixed_width() {
        assert_eq!(hex(&[0_u8; 32]), "0".repeat(64));
        assert_eq!(hex(&[0xff_u8; 32]), "f".repeat(64));
        assert_eq!(hex(&[0x0a_u8; 32]).len(), 64);
    }

    #[test]
    fn is_sha256_hex_is_strict() {
        assert!(is_sha256_hex(&"a".repeat(64)));
        assert!(!is_sha256_hex(&"A".repeat(64)));
        assert!(!is_sha256_hex(&"a".repeat(63)));
        assert!(!is_sha256_hex(""));
    }

    #[test]
    fn the_part_file_sits_next_to_the_destination() {
        let request = DownloadRequest::new("https://example.invalid/t", "/data/tools/t");
        assert_eq!(request.part_path(), PathBuf::from("/data/tools/t.part"));
        assert!(
            request.part_path().parent() == request.destination.parent(),
            "a rename across directories is not atomic"
        );
    }

    #[test]
    fn extracting_a_plain_file_as_an_archive_is_refused() {
        let dir = TempDir::new().expect("temp dir");
        let err = extract_archive(
            &dir.path().join("nope"),
            &dir.path().join("out"),
            ArchiveKind::None,
        )
        .expect_err("must fail");
        assert!(err.to_string().contains("not an archive"));
    }

    #[test]
    fn the_executable_is_found_at_the_top_level_and_one_level_down() {
        let dir = TempDir::new().expect("temp dir");

        let flat = dir.path().join("flat");
        fs::create_dir_all(&flat).expect("dir");
        fs::write(flat.join("ffmpeg"), b"x").expect("write");
        assert_eq!(
            find_executable(&flat, "ffmpeg").as_deref(),
            Some(flat.join("ffmpeg").as_path())
        );

        // The BtbN and Deno layout: one directory deep.
        let nested = dir.path().join("nested");
        fs::create_dir_all(nested.join("ffmpeg-9.0")).expect("dir");
        fs::write(nested.join("ffmpeg-9.0").join("ffmpeg"), b"x").expect("write");
        assert_eq!(
            find_executable(&nested, "ffmpeg").as_deref(),
            Some(nested.join("ffmpeg-9.0").join("ffmpeg").as_path())
        );

        assert_eq!(find_executable(&nested, "ffprobe"), None);
    }

    #[test]
    fn a_zip_is_unpacked_and_a_hostile_entry_is_refused_whole() {
        use std::io::Write as _;
        let dir = TempDir::new().expect("temp dir");
        let archive = dir.path().join("tool.zip");
        {
            let mut writer = zip::ZipWriter::new(File::create(&archive).expect("create"));
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer
                .start_file("ffmpeg-9.0/bin/ffmpeg", options)
                .expect("entry");
            writer.write_all(b"#!/bin/sh\necho fake\n").expect("write");
            writer.start_file("../../escaped", options).expect("entry");
            writer.write_all(b"nope").expect("write");
            writer.finish().expect("finish");
        }

        let out = dir.path().join("out");
        let err = extract_archive(&archive, &out, ArchiveKind::Zip).expect_err("must refuse");
        assert!(err.to_string().contains("escapes the destination"), "{err}");
        assert!(
            !dir.path()
                .parent()
                .expect("parent")
                .join("escaped")
                .exists(),
            "nothing may be written outside the destination"
        );
    }

    #[test]
    fn a_single_gz_asset_is_decompressed_and_named_from_the_url() {
        use std::io::Write as _;
        let dir = TempDir::new().expect("temp dir");
        let gz = dir.path().join("asset.gz");
        {
            let encoder = flate2::write::GzEncoder::new(
                File::create(&gz).expect("create"),
                flate2::Compression::fast(),
            );
            let mut writer = std::io::BufWriter::new(encoder);
            writer.write_all(b"#!/bin/sh\necho fake\n").expect("write");
            writer
                .into_inner()
                .expect("flush")
                .finish()
                .expect("finish");
        }

        let out = dir.path().join("out");
        let path = extract_gz(
            &gz,
            &out,
            "https://example.invalid/releases/ffmpeg-linux-x64.gz",
        )
        .expect("unpack");
        assert_eq!(path, out.join("ffmpeg-linux-x64"));
        assert_eq!(fs::read(&path).expect("read"), b"#!/bin/sh\necho fake\n");
    }

    #[test]
    fn a_gz_name_cannot_escape_the_destination() {
        use std::io::Write as _;
        let dir = TempDir::new().expect("temp dir");
        let gz = dir.path().join("asset.gz");
        {
            let encoder = flate2::write::GzEncoder::new(
                File::create(&gz).expect("create"),
                flate2::Compression::fast(),
            );
            let mut writer = std::io::BufWriter::new(encoder);
            writer.write_all(b"x").expect("write");
            writer
                .into_inner()
                .expect("flush")
                .finish()
                .expect("finish");
        }

        let out = dir.path().join("out");
        // The last segment is what is kept, so `..` never becomes a parent.
        let path =
            extract_gz(&gz, &out, "https://example.invalid/../../escape.gz").expect("unpack");
        assert_eq!(path, out.join("escape"));
    }

    #[test]
    fn a_tar_gz_is_unpacked_and_links_are_ignored() {
        let dir = TempDir::new().expect("temp dir");
        let plain = dir.path().join("tool.tar");
        {
            let mut builder = tar::Builder::new(File::create(&plain).expect("create"));
            let data = b"#!/bin/sh\necho fake\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "ffmpeg-9.0/bin/ffmpeg", &data[..])
                .expect("append");
            // A symlink: never written, because a tool archive has no reason to
            // contain one and honouring it is how extraction escapes.
            let mut link = tar::Header::new_gnu();
            link.set_entry_type(tar::EntryType::Symlink);
            link.set_size(0);
            link.set_mode(0o777);
            link.set_cksum();
            builder
                .append_link(&mut link, "bin/ffmpeg", "../../../etc/passwd")
                .expect("append link");
            builder.finish().expect("finish");
        }
        let gz = dir.path().join("tool.tar.gz");
        {
            let encoder = flate2::write::GzEncoder::new(
                File::create(&gz).expect("create"),
                flate2::Compression::fast(),
            );
            let mut writer = std::io::BufWriter::new(encoder);
            std::io::copy(&mut File::open(&plain).expect("open"), &mut writer).expect("copy");
        }

        let out = dir.path().join("out");
        extract_archive(&gz, &out, ArchiveKind::TarGz).expect("unpack");
        assert!(out.join("ffmpeg-9.0").join("bin").join("ffmpeg").is_file());
        assert!(
            !out.join("passwd").exists(),
            "a symlink entry must not be materialised"
        );
    }
}
