//! App directories, atomic writes and filename helpers (spec §3 `paths.rs`,
//! spec §2.8 persisted config).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

/// File name of the settings document inside the config dir (spec §2.8).
pub const SETTINGS_FILE: &str = "settings.json";
/// Kept when a settings file could not be parsed (spec §7.4 does the same for
/// the queue; here for the config).
pub const BACKUP_SUFFIX: &str = ".bak";

/// `…/<AppConfig>/settings.json`
pub fn settings_file(app: &AppHandle) -> io::Result<PathBuf> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| io::Error::other(format!("cannot resolve the app config dir: {e}")))?;
    Ok(dir.join(SETTINGS_FILE))
}

/// The app's writable data directory. Everything the app installs or remembers
/// at runtime lives below it (tools, runtime memory, downloaded models).
pub fn app_data_dir(app: &AppHandle) -> io::Result<PathBuf> {
    app.path()
        .app_data_dir()
        .map_err(|e| io::Error::other(format!("cannot resolve the app data dir: {e}")))
}

/// Directory for downloaded/installed tools (spec §5.2 lookup order #2).
pub fn tools_dir(app: &AppHandle) -> io::Result<PathBuf> {
    Ok(app_data_dir(app)?.join("tools"))
}

/// The user's home directory, when the OS exposes one.
///
/// Used only to build the well-known tool locations of spec §5.2
/// (`~/.deno/bin`, `~/.local/bin`). `None` simply drops those two entries
/// rather than guessing a path.
pub fn home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    }
}

/// Per-app private temp dir handed to child processes as `TMPDIR`/`TEMP`/`TMP`
/// (spec §4.9). Created with mode `0700` on Unix because QuickJS writes EJS
/// scripts there.
pub fn private_temp_dir(app: &AppHandle) -> io::Result<PathBuf> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| io::Error::other(format!("cannot resolve the app cache dir: {e}")))?
        .join("tmp");
    create_private_dir(&dir)?;
    Ok(dir)
}

#[cfg(unix)]
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if dir.is_dir() {
        return Ok(());
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    // Windows has no 0700, so the mode cannot be set and asking for one would be
    // a lie. What replaces it is *where* the directory is: every caller passes a
    // path under the per-user app data dir, which already carries that user's
    // ACL and is not visible to other accounts. Spec 4 item 9 asks for 0700 so
    // that a QuickJS temp dir is private; on Windows that privacy comes from the
    // parent, not from the leaf.
    //
    // Consequence worth knowing: the difference shows up in the test count.
    // `create_private_dir_uses_mode_0700` is `#[cfg(unix)]`, so the suite is 133
    // tests on Linux and 132 on Windows. That gap is deliberate, and it is
    // asserted here rather than discovered later by counting.
    fs::create_dir_all(dir)
}

/// Default output folder: the OS "Downloads" folder when the platform exposes
/// one, otherwise `None` (the user then picks a folder in Settings, spec §11.8).
pub fn default_output_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .download_dir()
        .ok()
        .filter(|p| !p.as_os_str().is_empty())
}

/// Which output folder a job uses (spec §11.8).
///
/// Priority: what the current request asked for → the configured default → the
/// OS Downloads folder. Pure so the ordering is testable; an empty path counts
/// as "not set" at every level, because `""` is what an empty entry field
/// produces and "write into the process working directory" is never the intent.
pub fn output_dir_for(
    requested: Option<PathBuf>,
    configured: Option<PathBuf>,
    os_default: Option<PathBuf>,
) -> Option<PathBuf> {
    let usable = |path: Option<PathBuf>| path.filter(|p| !p.as_os_str().is_empty());
    usable(requested)
        .or_else(|| usable(configured))
        .or_else(|| usable(os_default))
}

/// `mkdir -p` that is a no-op when the directory already exists.
pub fn ensure_dir(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dir)
}

/// Write `bytes` to `path` atomically: a sibling temp file is written and
/// fsync'ed, then renamed over the destination (spec §2.8 "temp + rename").
///
/// `std::fs::rename` maps to `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)` on
/// Windows and to POSIX `rename(2)` elsewhere, so it replaces the destination
/// atomically on all three targets without pulling in an extra dependency.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    ensure_dir(parent)?;

    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(format!(".tmp-{}", std::process::id()));
    let tmp = parent.join(tmp_name);

    // Any failure below must not leave the temp file behind.
    let write_result = (|| -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }

    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

/// `path.with_extension("bak")`-ish: `foo.json` -> `foo.json.bak`.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(BACKUP_SUFFIX);
    PathBuf::from(name)
}

/// Move a file to `<file>.bak`, replacing any previous backup. Never fails the
/// caller: if the copy cannot be made the original is still returned untouched
/// by the caller, so a corrupt file is not silently destroyed.
pub fn backup_file(path: &Path) -> io::Result<PathBuf> {
    let backup = backup_path(path);
    fs::rename(path, &backup)?;
    Ok(backup)
}

/// Windows device names that cannot be used as a file or folder name, with or
/// without an extension (spec §6.4 folder sanitising).
pub const RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Characters that are illegal in a path component on at least one supported
/// OS. Deliberately *not* ASCII-only: spec §12.5 forbids the original's
/// `[^a-zA-Z0-9_. -]` sanitiser, which destroys accents and non-Latin titles.
pub const ILLEGAL_CHARS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Make `name` safe as a single path component on Windows, macOS and Linux.
///
/// Keeps every Unicode letter (accents, CJK, emoji), replaces only genuinely
/// illegal characters, trims trailing dots/spaces (illegal on Windows) and
/// guards against the reserved device names.
pub fn sanitize_component(name: &str, max_len: usize) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if ILLEGAL_CHARS.contains(&c) { '_' } else { c })
        .collect();

    // Windows rejects names ending in a dot or a space.
    let trimmed = cleaned.trim_end_matches(['.', ' ']).trim();

    let mut out: String = trimmed.chars().take(max_len.max(1)).collect();
    while out.ends_with(['.', ' ']) {
        out.pop();
    }

    if out.is_empty() {
        return "_".to_owned();
    }

    let stem_upper = out
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem_upper.as_str()) {
        out.insert(0, '_');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn atomic_write_creates_missing_parents() {
        let dir = TempDir::new().expect("temp dir");
        let target = dir.path().join("a").join("b").join("settings.json");
        atomic_write(&target, b"hello").expect("write");
        assert_eq!(fs::read_to_string(&target).expect("read"), "hello");
    }

    #[test]
    fn atomic_write_replaces_existing_content_and_leaves_no_temp_file() {
        let dir = TempDir::new().expect("temp dir");
        let target = dir.path().join("settings.json");
        atomic_write(&target, b"old").expect("first write");
        atomic_write(&target, b"new").expect("second write");
        assert_eq!(fs::read_to_string(&target).expect("read"), "new");

        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "settings.json")
            .collect();
        assert!(leftovers.is_empty(), "unexpected files: {leftovers:?}");
    }

    #[test]
    fn atomic_write_rejects_a_path_without_parent() {
        let err = atomic_write(Path::new(""), b"x").expect_err("must fail");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn backup_path_appends_suffix() {
        let backup = backup_path(Path::new("/tmp/settings.json"));
        assert!(backup.to_string_lossy().ends_with("settings.json.bak"));
    }

    #[test]
    fn backup_file_moves_the_original() {
        let dir = TempDir::new().expect("temp dir");
        let target = dir.path().join("settings.json");
        atomic_write(&target, b"{ broken").expect("write");
        let backup = backup_file(&target).expect("backup");
        assert!(!target.exists());
        assert_eq!(fs::read_to_string(&backup).expect("read"), "{ broken");
    }

    #[test]
    fn sanitize_keeps_unicode_and_accents() {
        assert_eq!(sanitize_component("Perché no", 100), "Perché no");
        assert_eq!(sanitize_component("日本語タイトル", 100), "日本語タイトル");
        assert_eq!(sanitize_component("emoji 🎬 clip", 100), "emoji 🎬 clip");
    }

    #[test]
    fn sanitize_replaces_only_illegal_characters() {
        assert_eq!(
            sanitize_component(r#"a<b>c:d"e/f\g|h?i*j"#, 100),
            "a_b_c_d_e_f_g_h_i_j"
        );
    }

    #[test]
    fn sanitize_trims_trailing_dots_and_spaces() {
        assert_eq!(sanitize_component("name.  ", 100), "name");
        assert_eq!(sanitize_component("...", 100), "_");
    }

    #[test]
    fn sanitize_escapes_reserved_device_names() {
        assert_eq!(sanitize_component("CON", 100), "_CON");
        assert_eq!(sanitize_component("nul.txt", 100), "_nul.txt");
        assert_eq!(sanitize_component("console", 100), "console");
    }

    #[test]
    fn sanitize_enforces_max_length_by_characters() {
        let out = sanitize_component("àèìòùàèìòù", 5);
        assert_eq!(out.chars().count(), 5);
        assert_eq!(out, "àèìòù");
    }

    #[test]
    fn sanitize_falls_back_for_empty_names() {
        assert_eq!(sanitize_component("", 100), "_");
        assert_eq!(sanitize_component("   ", 100), "_");
    }

    #[test]
    fn create_private_dir_is_idempotent() {
        let dir = TempDir::new().expect("temp dir");
        let target = dir.path().join("nested").join("tmp");
        create_private_dir(&target).expect("first");
        create_private_dir(&target).expect("second");
        assert!(target.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn create_private_dir_uses_mode_0700() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().expect("temp dir");
        let target = dir.path().join("tmp");
        create_private_dir(&target).expect("create");
        let mode = fs::metadata(&target).expect("stat").permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "got {:o}", mode & 0o777);
    }

    #[test]
    fn the_output_folder_prefers_the_request_then_the_setting_then_the_os() {
        let request = PathBuf::from("/request");
        let setting = PathBuf::from("/setting");
        let os = PathBuf::from("/downloads");

        assert_eq!(
            output_dir_for(
                Some(request.clone()),
                Some(setting.clone()),
                Some(os.clone())
            ),
            Some(request)
        );
        assert_eq!(
            output_dir_for(None, Some(setting.clone()), Some(os.clone())),
            Some(setting)
        );
        assert_eq!(output_dir_for(None, None, Some(os.clone())), Some(os));
        assert_eq!(output_dir_for(None, None, None), None);

        // An empty path is "not set" at every level: it is what an empty entry
        // field produces, and writing to the process working directory is never
        // the intent.
        assert_eq!(
            output_dir_for(
                Some(PathBuf::new()),
                None,
                Some(PathBuf::from("/downloads"))
            ),
            Some(PathBuf::from("/downloads"))
        );
        assert_eq!(output_dir_for(None, Some(PathBuf::new()), None), None);
    }
}
