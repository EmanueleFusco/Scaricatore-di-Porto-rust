//! Typed errors for the whole backend (spec §3 `error.rs`).
//!
//! Every error that can reach the frontend is turned into a serialisable
//! [`ErrorInfo`], so the UI can localise it with `message_key` and still show
//! the raw `detail` in "Copy details" (spec §8.5).

use serde::{Deserialize, Serialize};

/// Machine-readable, stable error codes. These end up in the frontend, so
/// never rename one without also updating the i18n dictionaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Io,
    Json,
    Tauri,
    SettingsCorrupt,
    Unsupported,
    /// Spawning an external tool, or controlling its process tree, failed
    /// (spec §4).
    Process,
    /// Whisper model catalogue id was not found (spec §10.2).
    ModelNotFound,
    /// Downloaded or imported model checksum does not match expected sha256.
    ChecksumMismatch,
    /// Target disk partition has insufficient free space (spec §10.2).
    DiskFull,
    /// Source file path for model import does not exist.
    FileNotFound,
    /// Local file imported does not match any known model in catalogue.
    UnknownModel,
}

impl ErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Io => "IO",
            Self::Json => "JSON",
            Self::Tauri => "TAURI",
            Self::SettingsCorrupt => "SETTINGS_CORRUPT",
            Self::Unsupported => "UNSUPPORTED",
            Self::Process => "PROCESS",
            Self::ModelNotFound => "MODEL_NOT_FOUND",
            Self::ChecksumMismatch => "CHECKSUM_MISMATCH",
            Self::DiskFull => "DISK_FULL",
            Self::FileNotFound => "FILE_NOT_FOUND",
            Self::UnknownModel => "UNKNOWN_MODEL",
        }
    }

    /// Key into the `error.*` namespace of the i18n dictionaries.
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Io => "error.io",
            Self::Json => "error.json",
            Self::Tauri => "error.tauri",
            Self::SettingsCorrupt => "error.settingsCorrupt",
            Self::Unsupported => "error.unsupported",
            Self::Process => "error.process",
            Self::ModelNotFound => "error.modelNotFound",
            Self::ChecksumMismatch => "error.checksumMismatch",
            Self::DiskFull => "error.diskFull",
            Self::FileNotFound => "error.fileNotFound",
            Self::UnknownModel => "error.unknownModel",
        }
    }
}

/// Wire format for every error returned to the frontend (spec §3).
///
/// `Deserialize` as well as `Serialize`: a queue item keeps its last error in
/// `queue.json` (spec §7.4), so the shape has to survive a round trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorInfo {
    pub code: String,
    pub message_key: String,
    pub detail: Option<String>,
}

impl ErrorInfo {
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code: code.as_str().to_owned(),
            message_key: code.message_key().to_owned(),
            detail: Some(detail.into()),
        }
    }

    pub fn bare(code: ErrorCode) -> Self {
        Self {
            code: code.as_str().to_owned(),
            message_key: code.message_key().to_owned(),
            detail: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("malformed JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("tauri error: {0}")]
    Tauri(#[from] tauri::Error),

    #[error("{path} could not be read or is corrupt: {detail}")]
    SettingsCorrupt { path: String, detail: String },

    #[error("unsupported: {0}")]
    Unsupported(String),

    /// Spawning a tool or controlling its process tree failed. `detail` always
    /// names the tool and the OS error, never a secret.
    #[error("{0}")]
    Process(String),

    #[error("model not found: {0}")]
    ModelNotFound(String),

    #[error("checksum mismatch: expected {expected}, actual {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("insufficient disk space: needed {needed} bytes, available {available} bytes")]
    DiskFull { needed: u64, available: u64 },

    #[error("file not found: {0}")]
    FileNotFound(String),

    #[error("unknown model: {0}")]
    UnknownModel(String),
}

impl AppError {
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::Io(_) => ErrorCode::Io,
            Self::Json(_) => ErrorCode::Json,
            Self::Tauri(_) => ErrorCode::Tauri,
            Self::SettingsCorrupt { .. } => ErrorCode::SettingsCorrupt,
            Self::Unsupported(_) => ErrorCode::Unsupported,
            Self::Process(_) => ErrorCode::Process,
            Self::ModelNotFound(_) => ErrorCode::ModelNotFound,
            Self::ChecksumMismatch { .. } => ErrorCode::ChecksumMismatch,
            Self::DiskFull { .. } => ErrorCode::DiskFull,
            Self::FileNotFound(_) => ErrorCode::FileNotFound,
            Self::UnknownModel(_) => ErrorCode::UnknownModel,
        }
    }

    /// Never leaks secrets: only file paths and error strings, never values.
    pub fn to_error_info(&self) -> ErrorInfo {
        ErrorInfo::new(self.code(), self.to_string())
    }
}

// A `Result` alias used by every command, so the error type is always the
// frontend-facing [`ErrorInfo`].
pub type AppResult<T> = std::result::Result<T, ErrorInfo>;

/// Is `key` a label in both frontend dictionaries?
///
/// A `message_key` the dictionaries do not know is printed as the key itself
/// (the frontend's `t()` falls back to it), which is how raw keys reached a
/// screenshot (D98). The backend cannot see the UI; it can check this.
#[cfg(test)]
pub(crate) fn message_key_in_dictionaries(key: &str) -> bool {
    let it: serde_json::Value = serde_json::from_str(include_str!("../../src/lib/i18n/it.json"))
        .expect("the Italian dictionary parses");
    let en: serde_json::Value = serde_json::from_str(include_str!("../../src/lib/i18n/en.json"))
        .expect("the English dictionary parses");
    [&it, &en].iter().all(|dictionary| {
        dictionary
            .get(key)
            .and_then(|value| value.as_str())
            .is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every code, so a new one has to be added here and looked at.
    const CODES: [ErrorCode; 11] = [
        ErrorCode::Io,
        ErrorCode::Json,
        ErrorCode::Tauri,
        ErrorCode::SettingsCorrupt,
        ErrorCode::Unsupported,
        ErrorCode::Process,
        ErrorCode::ModelNotFound,
        ErrorCode::ChecksumMismatch,
        ErrorCode::DiskFull,
        ErrorCode::FileNotFound,
        ErrorCode::UnknownModel,
    ];

    #[test]
    fn error_info_carries_code_key_and_detail() {
        let err = AppError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "nope",
        ));
        let info = err.to_error_info();
        assert_eq!(info.code, "IO");
        assert_eq!(info.message_key, "error.io");
        assert!(info.detail.unwrap().contains("nope"));
    }

    #[test]
    fn error_info_serialises_to_the_documented_shape() {
        let info = ErrorInfo::new(ErrorCode::Unsupported, "because");
        let json = serde_json::to_value(&info).expect("serialisable");
        assert_eq!(json["code"], "UNSUPPORTED");
        assert_eq!(json["messageKey"], "error.unsupported");
        assert_eq!(json["detail"], "because");
    }

    #[test]
    fn bare_error_has_no_detail() {
        let json = serde_json::to_value(ErrorInfo::bare(ErrorCode::Json)).expect("serialisable");
        assert!(json["detail"].is_null());
    }

    #[test]
    fn every_code_has_a_distinct_message_key() {
        let mut keys: Vec<_> = CODES.iter().map(|c| c.message_key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), CODES.len());
    }

    #[test]
    fn every_code_has_a_dictionary_entry() {
        let missing: Vec<_> = CODES
            .iter()
            .map(|c| c.message_key())
            .filter(|key| !message_key_in_dictionaries(key))
            .collect();
        assert!(
            missing.is_empty(),
            "message keys missing from the i18n dictionaries: {missing:?}"
        );
    }
}
