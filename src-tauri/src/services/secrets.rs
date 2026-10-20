//! Secure storage for Gemini API key (spec §10.5).
//!
//! Lookup order:
//! 1. `GEMINI_API_KEY` environment variable.
//! 2. OS keyring (`keyring` crate: Credential Manager on Windows, Keychain on macOS, Secret Service on Linux).
//! 3. Linux fallback: private configuration file `<config_dir>/gemini_api_key` with mode `0600`.
//!
//! Security rule: The API key is NEVER logged, and NEVER sent to the frontend.
//! The frontend only receives [`GeminiKeyStatus`] ("set / not set" and storage source).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub const SERVICE_NAME: &str = "scaricatore-di-porto";
pub const KEY_USER: &str = "gemini_api_key";
pub const FALLBACK_FILENAME: &str = "gemini_api_key";
pub const ENV_VAR_NAME: &str = "GEMINI_API_KEY";

/// Where the Gemini API key was loaded from or stored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeySource {
    None,
    Env,
    Keyring,
    File,
}

/// Redacted status exposed to the frontend (spec §10.5: "Never send it back to the frontend").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiKeyStatus {
    pub has_key: bool,
    pub source: KeySource,
}

/// Resolves the fallback file path `<config_dir>/gemini_api_key`.
pub fn fallback_key_path(config_dir: &Path) -> PathBuf {
    config_dir.join(FALLBACK_FILENAME)
}

/// Retrieve the Gemini API key from environment, OS keyring, or fallback file (spec §10.5).
pub fn get_gemini_api_key(config_dir: &Path) -> Option<String> {
    // 1. Environment variable override
    if let Ok(val) = std::env::var(ENV_VAR_NAME) {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }

    // 2. OS Keyring
    if let Ok(entry) = keyring::Entry::new(SERVICE_NAME, KEY_USER) {
        if let Ok(password) = entry.get_password() {
            let trimmed = password.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }

    // 3. Fallback file
    let path = fallback_key_path(config_dir);
    if path.is_file() {
        if let Ok(contents) = fs::read_to_string(&path) {
            let trimmed = contents.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }

    None
}

/// Query whether an API key is available and from which source, without exposing the secret.
pub fn get_gemini_key_status(config_dir: &Path) -> GeminiKeyStatus {
    // 1. Environment variable
    if let Ok(val) = std::env::var(ENV_VAR_NAME) {
        if !val.trim().is_empty() {
            return GeminiKeyStatus {
                has_key: true,
                source: KeySource::Env,
            };
        }
    }

    // 2. OS Keyring
    if let Ok(entry) = keyring::Entry::new(SERVICE_NAME, KEY_USER) {
        if let Ok(password) = entry.get_password() {
            if !password.trim().is_empty() {
                return GeminiKeyStatus {
                    has_key: true,
                    source: KeySource::Keyring,
                };
            }
        }
    }

    // 3. Fallback file
    let path = fallback_key_path(config_dir);
    if path.is_file() {
        if let Ok(contents) = fs::read_to_string(&path) {
            if !contents.trim().is_empty() {
                return GeminiKeyStatus {
                    has_key: true,
                    source: KeySource::File,
                };
            }
        }
    }

    GeminiKeyStatus {
        has_key: false,
        source: KeySource::None,
    }
}

/// Save the Gemini API key. Tries OS Keyring first, then falls back to `<config_dir>/gemini_api_key`
/// with mode `0600` on Unix if the keyring is unavailable.
pub fn set_gemini_api_key(config_dir: &Path, key: &str) -> Result<KeySource, AppError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        delete_gemini_api_key(config_dir)?;
        return Ok(KeySource::None);
    }

    // Try OS keyring first
    let keyring_entry = keyring::Entry::new(SERVICE_NAME, KEY_USER);
    let keyring_result = match keyring_entry {
        Ok(entry) => entry.set_password(trimmed),
        Err(e) => Err(e),
    };

    match keyring_result {
        Ok(()) => {
            // Success in OS keyring: delete any legacy fallback file
            let fallback = fallback_key_path(config_dir);
            if fallback.exists() {
                let _ = fs::remove_file(fallback);
            }
            Ok(KeySource::Keyring)
        }
        Err(_) => {
            // Keyring unavailable (e.g. headless Linux, no Secret Service running):
            // save to fallback file with mode 0600.
            save_key_to_fallback_file(config_dir, trimmed)?;
            Ok(KeySource::File)
        }
    }
}

/// Delete the Gemini API key from both OS keyring and fallback file.
pub fn delete_gemini_api_key(config_dir: &Path) -> Result<(), AppError> {
    if let Ok(entry) = keyring::Entry::new(SERVICE_NAME, KEY_USER) {
        let _ = entry.delete_credential();
    }
    let fallback = fallback_key_path(config_dir);
    if fallback.exists() {
        fs::remove_file(fallback).map_err(AppError::Io)?;
    }
    Ok(())
}

/// Write key to `<config_dir>/gemini_api_key` with strict permissions (mode 0600 on Unix).
pub fn save_key_to_fallback_file(config_dir: &Path, key: &str) -> Result<(), AppError> {
    if !config_dir.exists() {
        fs::create_dir_all(config_dir).map_err(AppError::Io)?;
    }
    let path = fallback_key_path(config_dir);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true).mode(0o600);
        let mut file = options.open(&path).map_err(AppError::Io)?;
        file.write_all(key.as_bytes()).map_err(AppError::Io)?;
        file.flush().map_err(AppError::Io)?;
    }

    #[cfg(not(unix))]
    {
        fs::write(&path, key.as_bytes()).map_err(AppError::Io)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn fallback_key_file_round_trip() {
        let temp = tempdir().unwrap();
        let config_dir = temp.path();

        assert_eq!(get_gemini_api_key(config_dir), None);
        assert_eq!(
            get_gemini_key_status(config_dir),
            GeminiKeyStatus {
                has_key: false,
                source: KeySource::None,
            }
        );

        save_key_to_fallback_file(config_dir, "AIzaSyFakeKeyTest123").unwrap();

        assert_eq!(
            get_gemini_api_key(config_dir),
            Some("AIzaSyFakeKeyTest123".to_string())
        );
        let status = get_gemini_key_status(config_dir);
        assert!(status.has_key);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = fallback_key_path(config_dir);
            let meta = fs::metadata(&path).unwrap();
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(
                mode, 0o600,
                "fallback key file permissions must be 0600 on Unix"
            );
        }

        delete_gemini_api_key(config_dir).unwrap();
        assert_eq!(get_gemini_api_key(config_dir), None);
    }

    #[test]
    fn env_var_overrides_storage() {
        let temp = tempdir().unwrap();
        let config_dir = temp.path();

        save_key_to_fallback_file(config_dir, "file_key_123").unwrap();
        assert_eq!(
            get_gemini_api_key(config_dir),
            Some("file_key_123".to_string())
        );

        // Set env var
        unsafe {
            std::env::set_var(ENV_VAR_NAME, "env_key_456");
        }

        assert_eq!(
            get_gemini_api_key(config_dir),
            Some("env_key_456".to_string())
        );
        assert_eq!(
            get_gemini_key_status(config_dir),
            GeminiKeyStatus {
                has_key: true,
                source: KeySource::Env,
            }
        );

        // Clean up env var
        unsafe {
            std::env::remove_var(ENV_VAR_NAME);
        }

        assert_eq!(
            get_gemini_api_key(config_dir),
            Some("file_key_123".to_string())
        );
    }

    #[test]
    fn set_empty_key_deletes_it() {
        let temp = tempdir().unwrap();
        let config_dir = temp.path();

        save_key_to_fallback_file(config_dir, "to_be_deleted").unwrap();
        assert!(fallback_key_path(config_dir).exists());

        let source = set_gemini_api_key(config_dir, "   ").unwrap();
        assert_eq!(source, KeySource::None);
        assert!(!fallback_key_path(config_dir).exists());
    }
}
