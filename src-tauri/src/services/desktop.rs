//! AppImage-only desktop integration. Native package launchers belong to installers.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Quote one Exec argument, then escape it as a desktop-entry string value.
pub fn desktop_entry(executable: &str, icon: &str, wm_class: &str) -> Option<String> {
    if executable.chars().chain(icon.chars()).any(char::is_control) {
        return None;
    }
    let mut argument = String::from("\"");
    for c in executable.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            argument.push('\\');
        }
        if c == '%' {
            argument.push('%');
        }
        argument.push(c);
    }
    argument.push('"');
    let exec = argument.replace('\\', "\\\\");
    let icon = icon.replace('\\', "\\\\");
    Some(format!(
        "[Desktop Entry]\nType=Application\nName=Scaricatore Di Porto\nComment=Downloader and transcriber\nExec={exec}\nIcon={icon}\nTerminal=false\nCategories=Utility;Network;\nStartupNotify=true\nStartupWMClass={wm_class}\nX-SDP-AppImage=true\n"
    ))
}

/// Recognize only the exact legacy-generated entry pointing into an AppImage mount.
fn is_legacy_entry(text: &str, icon: &str, wm_class: &str) -> bool {
    let Some(exec) = text.lines().find_map(|line| {
        line.strip_prefix("Exec=\"")
            .and_then(|s| s.strip_suffix("\" %U"))
    }) else {
        return false;
    };
    if !exec.starts_with('/') || !exec.split('/').any(|part| part.starts_with(".mount_")) {
        return false;
    }
    text == format!(
        "[Desktop Entry]\nType=Application\nName=Scaricatore Di Porto\nComment=Downloader and transcriber\nExec=\"{exec}\" %U\nIcon={icon}\nTerminal=false\nCategories=Utility;Network;\nStartupNotify=true\nStartupWMClass={wm_class}\n"
    )
}

/// Publish atomically without replacing installer/user entries. Migrate only our
/// exact old transient-mount entries, and update entries carrying our own marker.
pub fn integrate_appimage(appimage: &Path, data_dir: &Path, applications: &Path) -> io::Result<()> {
    if !appimage.is_absolute() || !appimage.is_file() {
        return Ok(());
    }
    let Some(executable) = appimage.to_str() else {
        return Ok(());
    };
    let icon_dir = data_dir.join("icons");
    let icon_path = icon_dir.join("icon.png");
    let Some(icon) = icon_path.to_str() else {
        return Ok(());
    };
    fs::create_dir_all(&icon_dir)?;
    fs::create_dir_all(applications)?;
    fs::write(&icon_path, include_bytes!("../../icons/icon.png"))?;
    for (filename, wm_class) in [
        (
            "com.scaricatorediporto.app.desktop",
            "com.scaricatorediporto.app",
        ),
        ("scaricatore-di-porto.desktop", "scaricatore-di-porto"),
    ] {
        let Some(content) = desktop_entry(executable, icon, wm_class) else {
            continue;
        };
        let path = applications.join(filename);
        let existing = match fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err),
        };
        if let Some(text) = existing.as_deref() {
            if text == content {
                continue;
            }
            if !text.lines().any(|line| line == "X-SDP-AppImage=true")
                && !is_legacy_entry(text, icon, wm_class)
            {
                continue;
            }
        }
        let mut staged = tempfile::NamedTempFile::new_in(applications)?;
        staged.write_all(content.as_bytes())?;
        if let Some(existing) = existing {
            if fs::read_to_string(&path).ok().as_deref() == Some(&existing) {
                staged.persist(&path).map_err(|err| err.error)?;
            }
        } else {
            match staged.persist_noclobber(&path) {
                Ok(_) => {}
                Err(err) if err.error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(err) => return Err(err.error),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quotes_reserved_exec_characters_and_rejects_control_characters() {
        let text = desktop_entry("/tmp/My app % $`\"\\.AppImage", "/tmp/icon\\.png", "sdp")
            .expect("entry");
        assert!(
            text.contains("My app %% \\\\$\\\\`\\\\\"\\\\\\\\.AppImage"),
            "{text}"
        );
        assert!(!text.contains("%U"));
        assert!(desktop_entry("/tmp/app\nExec=bad", "/icon", "sdp").is_none());
    }
    #[test]
    fn preserves_installer_entries_and_updates_only_owned_appimage_entries() {
        let root = tempfile::tempdir().expect("tempdir");
        let appimage = root.path().join("My app.AppImage");
        fs::write(&appimage, b"fixture").expect("appimage");
        let applications = root.path().join("applications");
        fs::create_dir(&applications).expect("apps");
        let managed = applications.join("com.scaricatorediporto.app.desktop");
        fs::write(&managed, "[Desktop Entry]\nExec=installer-managed\n").expect("installer");
        integrate_appimage(&appimage, root.path(), &applications).expect("integrate");
        assert_eq!(
            fs::read_to_string(managed).expect("managed"),
            "[Desktop Entry]\nExec=installer-managed\n"
        );
        let own = applications.join("scaricatore-di-porto.desktop");
        let text = fs::read_to_string(&own).expect("owned");
        assert!(text.contains("My app.AppImage"));
        assert!(!text.contains(".mount_"));
        integrate_appimage(&appimage, root.path(), &applications).expect("repeat");
        assert_eq!(fs::read_to_string(own).expect("owned"), text);
    }
    #[test]
    fn recognizes_only_unmodified_legacy_transient_mount_entries() {
        let old = "[Desktop Entry]\nType=Application\nName=Scaricatore Di Porto\nComment=Downloader and transcriber\nExec=\"/tmp/.mount_sdp/usr/bin/sdp\" %U\nIcon=/icon\nTerminal=false\nCategories=Utility;Network;\nStartupNotify=true\nStartupWMClass=sdp\n";
        assert!(is_legacy_entry(old, "/icon", "sdp"));
        assert!(!is_legacy_entry(
            &old.replace("/tmp/.mount_sdp", "/usr"),
            "/icon",
            "sdp"
        ));
        assert!(!is_legacy_entry(
            &format!("{old}Actions=Custom;\n"),
            "/icon",
            "sdp"
        ));
    }
}
