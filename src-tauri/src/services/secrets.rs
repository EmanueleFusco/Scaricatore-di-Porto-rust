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
    #[serde(rename = "environment", alias = "env")]
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

/// Abstraction over the OS keyring (spec §10.5) allowing unit tests to inject
/// an in-memory mock or bypass without mutating the host's real credential store.
pub trait KeyringBackend: Send + Sync {
    fn get_password(&self, service: &str, user: &str) -> Result<String, keyring::Error>;
    fn set_password(&self, service: &str, user: &str, password: &str)
    -> Result<(), keyring::Error>;
    fn delete_credential(&self, service: &str, user: &str) -> Result<(), keyring::Error>;
}

/// Real OS keyring using the `keyring` crate (Credential Manager, Keychain, or Secret Service).
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeyring;

impl KeyringBackend for OsKeyring {
    fn get_password(&self, service: &str, user: &str) -> Result<String, keyring::Error> {
        let entry = keyring::Entry::new(service, user)?;
        entry.get_password()
    }

    fn set_password(
        &self,
        service: &str,
        user: &str,
        password: &str,
    ) -> Result<(), keyring::Error> {
        let entry = keyring::Entry::new(service, user)?;
        entry.set_password(password)
    }

    fn delete_credential(&self, service: &str, user: &str) -> Result<(), keyring::Error> {
        let entry = keyring::Entry::new(service, user)?;
        entry.delete_credential()
    }
}

/// In-memory mock keyring for isolated unit testing.
#[derive(Debug, Default)]
pub struct MockKeyring {
    entries: std::sync::Mutex<std::collections::HashMap<(String, String), String>>,
    available: std::sync::atomic::AtomicBool,
}

impl MockKeyring {
    pub fn new() -> Self {
        Self {
            entries: std::sync::Mutex::new(std::collections::HashMap::new()),
            available: std::sync::atomic::AtomicBool::new(true),
        }
    }

    pub fn unavailable() -> Self {
        Self {
            entries: std::sync::Mutex::new(std::collections::HashMap::new()),
            available: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn set_available(&self, available: bool) {
        self.available
            .store(available, std::sync::atomic::Ordering::SeqCst);
    }
}

impl KeyringBackend for MockKeyring {
    fn get_password(&self, service: &str, user: &str) -> Result<String, keyring::Error> {
        if !self.available.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(keyring::Error::NoStorageAccess(Box::new(
                std::io::Error::new(std::io::ErrorKind::NotFound, "mock keyring unavailable"),
            )));
        }
        let lock = self.entries.lock().unwrap();
        match lock.get(&(service.to_string(), user.to_string())) {
            Some(val) => Ok(val.clone()),
            None => Err(keyring::Error::NoEntry),
        }
    }

    fn set_password(
        &self,
        service: &str,
        user: &str,
        password: &str,
    ) -> Result<(), keyring::Error> {
        if !self.available.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(keyring::Error::NoStorageAccess(Box::new(
                std::io::Error::new(std::io::ErrorKind::NotFound, "mock keyring unavailable"),
            )));
        }
        let mut lock = self.entries.lock().unwrap();
        lock.insert(
            (service.to_string(), user.to_string()),
            password.to_string(),
        );
        Ok(())
    }

    fn delete_credential(&self, service: &str, user: &str) -> Result<(), keyring::Error> {
        if !self.available.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(keyring::Error::NoStorageAccess(Box::new(
                std::io::Error::new(std::io::ErrorKind::NotFound, "mock keyring unavailable"),
            )));
        }
        let mut lock = self.entries.lock().unwrap();
        lock.remove(&(service.to_string(), user.to_string()));
        Ok(())
    }
}

/// Retrieve the Gemini API key using an injected keyring backend and environment resolver.
pub fn get_gemini_api_key_with<K: KeyringBackend + ?Sized, E: Fn(&str) -> Option<String>>(
    config_dir: &Path,
    keyring: &K,
    env_lookup: E,
) -> Option<String> {
    // 1. Environment variable override
    if let Some(val) = env_lookup(ENV_VAR_NAME) {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }

    // 2. Keyring backend
    if let Ok(password) = keyring.get_password(SERVICE_NAME, KEY_USER) {
        let trimmed = password.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }

    // 3. Fallback file (Linux fallback when no Secret Service is available, spec §10.5;
    // also checked by integration tests across platforms without touching the OS keyring)
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

/// Query key status using an injected keyring backend and environment resolver.
pub fn get_gemini_key_status_with<K: KeyringBackend + ?Sized, E: Fn(&str) -> Option<String>>(
    config_dir: &Path,
    keyring: &K,
    env_lookup: E,
) -> GeminiKeyStatus {
    // 1. Environment variable
    if let Some(val) = env_lookup(ENV_VAR_NAME) {
        if !val.trim().is_empty() {
            return GeminiKeyStatus {
                has_key: true,
                source: KeySource::Env,
            };
        }
    }

    // 2. Keyring backend
    if let Ok(password) = keyring.get_password(SERVICE_NAME, KEY_USER) {
        if !password.trim().is_empty() {
            return GeminiKeyStatus {
                has_key: true,
                source: KeySource::Keyring,
            };
        }
    }

    // 3. Fallback file (Linux fallback when no Secret Service is available, spec §10.5;
    // also checked by integration tests across platforms without touching the OS keyring)
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

/// Save the Gemini API key using an injected keyring backend.
pub fn set_gemini_api_key_with<K: KeyringBackend + ?Sized>(
    config_dir: &Path,
    key: &str,
    keyring: &K,
) -> Result<KeySource, AppError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        delete_gemini_api_key_with(config_dir, keyring)?;
        return Ok(KeySource::None);
    }

    // Try OS keyring first
    let keyring_result = keyring.set_password(SERVICE_NAME, KEY_USER, trimmed);

    match keyring_result {
        Ok(()) => {
            // Success in keyring: delete any legacy fallback file
            let fallback = fallback_key_path(config_dir);
            if fallback.exists() {
                let _ = fs::remove_file(fallback);
            }
            Ok(KeySource::Keyring)
        }
        Err(e) => {
            #[cfg(any(target_os = "linux", test))]
            {
                let _ = e;
                // Keyring unavailable (e.g. headless Linux, no Secret Service running):
                // save to fallback file with mode 0600 (spec §10.5).
                save_key_to_fallback_file(config_dir, trimmed)?;
                Ok(KeySource::File)
            }
            #[cfg(not(any(target_os = "linux", test)))]
            {
                Err(AppError::Io(std::io::Error::other(format!(
                    "Failed to store API key in OS keyring: {e}"
                ))))
            }
        }
    }
}

/// Delete the Gemini API key from both keyring and fallback file using an injected keyring backend.
pub fn delete_gemini_api_key_with<K: KeyringBackend + ?Sized>(
    config_dir: &Path,
    keyring: &K,
) -> Result<(), AppError> {
    let _ = keyring.delete_credential(SERVICE_NAME, KEY_USER);
    let fallback = fallback_key_path(config_dir);
    if fallback.exists() {
        fs::remove_file(fallback).map_err(AppError::Io)?;
    }
    Ok(())
}

/// Retrieve the Gemini API key from environment, OS keyring, or fallback file (spec §10.5).
#[cfg(not(test))]
pub fn get_gemini_api_key(config_dir: &Path) -> Option<String> {
    get_gemini_api_key_with(config_dir, &OsKeyring, |k| std::env::var(k).ok())
}

/// Query whether an API key is available and from which source, without exposing the secret.
#[cfg(not(test))]
pub fn get_gemini_key_status(config_dir: &Path) -> GeminiKeyStatus {
    get_gemini_key_status_with(config_dir, &OsKeyring, |k| std::env::var(k).ok())
}

/// Save the Gemini API key. Tries OS Keyring first, then falls back to `<config_dir>/gemini_api_key`
/// with mode `0600` on Linux if the keyring is unavailable.
#[cfg(not(test))]
pub fn set_gemini_api_key(config_dir: &Path, key: &str) -> Result<KeySource, AppError> {
    set_gemini_api_key_with(config_dir, key, &OsKeyring)
}

/// Delete the Gemini API key from both OS keyring and fallback file.
#[cfg(not(test))]
pub fn delete_gemini_api_key(config_dir: &Path) -> Result<(), AppError> {
    delete_gemini_api_key_with(config_dir, &OsKeyring)
}

#[cfg(test)]
pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
thread_local! {
    static TEST_KEYRING: std::cell::RefCell<Option<std::sync::Arc<dyn KeyringBackend>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub fn current_test_keyring() -> std::sync::Arc<dyn KeyringBackend> {
    TEST_KEYRING.with(|cell| {
        let mut b = cell.borrow_mut();
        if let Some(ref backend) = *b {
            backend.clone()
        } else {
            let mock = std::sync::Arc::new(MockKeyring::new());
            *b = Some(mock.clone());
            mock
        }
    })
}

#[cfg(test)]
pub fn set_test_keyring(backend: Option<std::sync::Arc<dyn KeyringBackend>>) {
    TEST_KEYRING.with(|cell| {
        *cell.borrow_mut() = backend;
    });
}

#[cfg(test)]
pub fn get_gemini_api_key(config_dir: &Path) -> Option<String> {
    let backend = current_test_keyring();
    get_gemini_api_key_with(config_dir, backend.as_ref(), |k| std::env::var(k).ok())
}

#[cfg(test)]
pub fn get_gemini_key_status(config_dir: &Path) -> GeminiKeyStatus {
    let backend = current_test_keyring();
    get_gemini_key_status_with(config_dir, backend.as_ref(), |k| std::env::var(k).ok())
}

#[cfg(test)]
pub fn set_gemini_api_key(config_dir: &Path, key: &str) -> Result<KeySource, AppError> {
    let backend = current_test_keyring();
    set_gemini_api_key_with(config_dir, key, backend.as_ref())
}

#[cfg(test)]
pub fn delete_gemini_api_key(config_dir: &Path) -> Result<(), AppError> {
    let backend = current_test_keyring();
    delete_gemini_api_key_with(config_dir, backend.as_ref())
}

/// Write key to `<config_dir>/gemini_api_key` with strict permissions (mode 0600 on Unix)
/// using an atomic temp-file + rename write.
pub fn save_key_to_fallback_file(config_dir: &Path, key: &str) -> Result<(), AppError> {
    if !config_dir.exists() {
        fs::create_dir_all(config_dir).map_err(AppError::Io)?;
    }
    let path = fallback_key_path(config_dir);

    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

    let file_name = path.file_name().ok_or_else(|| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path has no file name",
        ))
    })?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = config_dir.join(tmp_name);

    let write_result = (|| -> Result<(), std::io::Error> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true).mode(0o600);
            let mut file = options.open(&tmp)?;
            file.write_all(key.as_bytes())?;
            file.flush()?;
            file.sync_all()?;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(not(unix))]
        {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(key.as_bytes())?;
            file.flush()?;
            file.sync_all()?;
        }
        fs::rename(&tmp, &path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    })();

    if let Err(err) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(AppError::Io(err));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    /// RAII guard to serialize environment variable mutations across test threads
    /// and reset the thread-local mock keyring.
    pub struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        orig: Option<String>,
    }

    impl EnvGuard {
        pub fn new() -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            set_test_keyring(Some(Arc::new(MockKeyring::new())));
            let orig = std::env::var(ENV_VAR_NAME).ok();
            unsafe {
                std::env::remove_var(ENV_VAR_NAME);
            }
            Self { _lock: lock, orig }
        }

        pub fn set(&mut self, val: &str) {
            unsafe {
                std::env::set_var(ENV_VAR_NAME, val);
            }
        }

        pub fn clear(&mut self) {
            unsafe {
                std::env::remove_var(ENV_VAR_NAME);
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            set_test_keyring(None);
            match &self.orig {
                Some(val) => unsafe { std::env::set_var(ENV_VAR_NAME, val) },
                None => unsafe { std::env::remove_var(ENV_VAR_NAME) },
            }
        }
    }

    #[test]
    fn fallback_key_file_round_trip() {
        let _guard = EnvGuard::new();
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
        let mut guard = EnvGuard::new();
        let temp = tempdir().unwrap();
        let config_dir = temp.path();

        save_key_to_fallback_file(config_dir, "file_key_123").unwrap();
        assert_eq!(
            get_gemini_api_key(config_dir),
            Some("file_key_123".to_string())
        );

        // Set env var via guard
        guard.set("env_key_456");

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

        // Clean up env var via guard
        guard.clear();

        assert_eq!(
            get_gemini_api_key(config_dir),
            Some("file_key_123".to_string())
        );
    }

    #[test]
    fn set_empty_key_deletes_it() {
        let _guard = EnvGuard::new();
        let temp = tempdir().unwrap();
        let config_dir = temp.path();

        save_key_to_fallback_file(config_dir, "to_be_deleted").unwrap();
        assert!(fallback_key_path(config_dir).exists());

        let source = set_gemini_api_key(config_dir, "   ").unwrap();
        assert_eq!(source, KeySource::None);
        assert!(!fallback_key_path(config_dir).exists());
    }

    #[test]
    fn concurrent_env_var_mutation_races_with_key_lookup() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::thread;

        let running = Arc::new(AtomicBool::new(true));
        let r1 = Arc::clone(&running);

        let t1 = thread::spawn(move || {
            while r1.load(Ordering::Relaxed) {
                {
                    let mut guard = EnvGuard::new();
                    guard.set("polluting_env_key");
                    thread::yield_now();
                    guard.clear();
                }
                thread::yield_now();
            }
        });

        let mut failed = false;
        for _ in 0..100 {
            let _guard = EnvGuard::new();
            let temp = tempdir().unwrap();
            let config_dir = temp.path();
            save_key_to_fallback_file(config_dir, "file_key_123").unwrap();
            let found = get_gemini_api_key(config_dir);
            if found != Some("file_key_123".to_string()) {
                failed = true;
                break;
            }
        }

        running.store(false, Ordering::Relaxed);
        let _ = t1.join();

        assert!(
            !failed,
            "get_gemini_api_key observed race on process-wide GEMINI_API_KEY"
        );
    }

    #[test]
    fn mock_keyring_backend_round_trip() {
        let temp = tempdir().unwrap();
        let config_dir = temp.path();
        let mock = MockKeyring::new();

        let source = set_gemini_api_key_with(config_dir, "mock_key_value", &mock).unwrap();
        assert_eq!(source, KeySource::Keyring);
        assert_eq!(
            mock.get_password(SERVICE_NAME, KEY_USER).unwrap(),
            "mock_key_value"
        );
        assert!(!fallback_key_path(config_dir).exists());

        assert_eq!(
            get_gemini_api_key_with(config_dir, &mock, |_| None),
            Some("mock_key_value".to_string())
        );
        assert_eq!(
            get_gemini_key_status_with(config_dir, &mock, |_| None),
            GeminiKeyStatus {
                has_key: true,
                source: KeySource::Keyring,
            }
        );

        delete_gemini_api_key_with(config_dir, &mock).unwrap();
        assert!(mock.get_password(SERVICE_NAME, KEY_USER).is_err());
        assert_eq!(get_gemini_api_key_with(config_dir, &mock, |_| None), None);
    }

    #[test]
    fn mock_keyring_unavailable_falls_back_to_file() {
        let temp = tempdir().unwrap();
        let config_dir = temp.path();
        let mock = MockKeyring::unavailable();

        let source = set_gemini_api_key_with(config_dir, "mock_fallback_key", &mock).unwrap();
        assert_eq!(source, KeySource::File);
        assert!(fallback_key_path(config_dir).exists());

        assert_eq!(
            get_gemini_api_key_with(config_dir, &mock, |_| None),
            Some("mock_fallback_key".to_string())
        );
        assert_eq!(
            get_gemini_key_status_with(config_dir, &mock, |_| None),
            GeminiKeyStatus {
                has_key: true,
                source: KeySource::File,
            }
        );

        delete_gemini_api_key_with(config_dir, &mock).unwrap();
        assert!(!fallback_key_path(config_dir).exists());
        assert_eq!(get_gemini_api_key_with(config_dir, &mock, |_| None), None);
    }

    #[test]
    fn default_functions_use_mock_keyring_in_test() {
        let _guard = EnvGuard::new();
        let temp = tempdir().unwrap();
        let config_dir = temp.path();

        let source = set_gemini_api_key(config_dir, "test_in_memory_key").unwrap();
        assert_eq!(source, KeySource::Keyring);
        assert_eq!(
            get_gemini_api_key(config_dir),
            Some("test_in_memory_key".to_string())
        );
        assert_eq!(
            get_gemini_key_status(config_dir),
            GeminiKeyStatus {
                has_key: true,
                source: KeySource::Keyring,
            }
        );

        delete_gemini_api_key(config_dir).unwrap();
        assert_eq!(get_gemini_api_key(config_dir), None);
        assert_eq!(
            get_gemini_key_status(config_dir),
            GeminiKeyStatus {
                has_key: false,
                source: KeySource::None,
            }
        );
    }

    #[test]
    fn key_source_env_serializes_as_environment() {
        assert_eq!(
            serde_json::to_string(&KeySource::Env).unwrap(),
            "\"environment\""
        );
        let status = GeminiKeyStatus {
            has_key: true,
            source: KeySource::Env,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(
            json.contains(r#""source":"environment""#),
            "expected 'environment', got: {json}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_key_to_fallback_file_enforces_0600_on_existing_file() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempdir().unwrap();
        let config_dir = temp.path();
        let path = fallback_key_path(config_dir);

        // Pre-create fallback file with loose permissions (0644)
        fs::write(&path, "old_key").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let initial_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(initial_mode, 0o644);

        // Save new key: must enforce 0600 on existing file
        save_key_to_fallback_file(config_dir, "new_key").unwrap();

        let final_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            final_mode, 0o600,
            "mode must be updated to 0600 even if file previously existed with 0644"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "new_key");
    }
}
