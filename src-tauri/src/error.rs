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
    /// sdp-whisper sidecar not found on the system (spec §5.4, §10.2).
    WhisperMissing,
    /// Whisper model file not found on disk (spec §10.2).
    WhisperModelMissing,
    /// Gemini API key is missing (spec §10.3, §10.5).
    GeminiKeyMissing,
    /// Gemini API key is invalid or unauthorized (HTTP 400/403).
    GeminiKeyInvalid,
    /// Gemini quota or rate limit exceeded (HTTP 429).
    GeminiQuotaExceeded,
    /// Gemini model not found or retired (HTTP 404).
    GeminiModelNotFound,
    /// Gemini blocked content generation (prompt feedback / block reason).
    GeminiBlocked,
    /// No subtitles found for the item (spec §10.1).
    NoSubtitles,
    /// In-app update check or install failed (spec §13.4).
    Update,
    /// Requested tool was not found on the system PATH or well-known dirs.
    ToolNotInPath,
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
            Self::WhisperMissing => "WHISPER_MISSING",
            Self::WhisperModelMissing => "WHISPER_MODEL_MISSING",
            Self::GeminiKeyMissing => "GEMINI_KEY_MISSING",
            Self::GeminiKeyInvalid => "GEMINI_KEY_INVALID",
            Self::GeminiQuotaExceeded => "GEMINI_QUOTA_EXCEEDED",
            Self::GeminiModelNotFound => "GEMINI_MODEL_NOT_FOUND",
            Self::GeminiBlocked => "GEMINI_BLOCKED",
            Self::NoSubtitles => "NO_SUBTITLES",
            Self::Update => "UPDATE",
            Self::ToolNotInPath => "TOOL_NOT_IN_PATH",
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
            Self::WhisperMissing => "error.whisperMissing",
            Self::WhisperModelMissing => "error.whisperModelMissing",
            Self::GeminiKeyMissing => "error.geminiKeyMissing",
            Self::GeminiKeyInvalid => "error.geminiKeyInvalid",
            Self::GeminiQuotaExceeded => "error.geminiQuotaExceeded",
            Self::GeminiModelNotFound => "error.geminiModelNotFound",
            Self::GeminiBlocked => "error.geminiBlocked",
            Self::NoSubtitles => "error.noSubtitles",
            Self::Update => "error.update",
            Self::ToolNotInPath => "error.tools.notInSystemPath",
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

    #[error("sdp-whisper sidecar not found (spec §5.4). Please install or provision it.")]
    WhisperMissing,

    #[error("Whisper model '{0}' is not installed.")]
    WhisperModelMissing(String),

    #[error("Gemini API key is not configured.")]
    GeminiKeyMissing,

    #[error("Gemini API key is invalid or unauthorized: {0}")]
    GeminiKeyInvalid(String),

    #[error("Gemini API quota or rate limit exceeded: {0}")]
    GeminiQuotaExceeded(String),

    #[error(
        "Model '{0}' is no longer available on Gemini. Select another model (for example gemini-flash-latest) in Settings → Transcription."
    )]
    GeminiModelNotFound(String),

    #[error("Gemini generation blocked: {0}")]
    GeminiBlocked(String),

    #[error("No subtitles found for requested languages.")]
    NoSubtitles,

    #[error("update failed: {0}")]
    Update(String),
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
            Self::WhisperMissing => ErrorCode::WhisperMissing,
            Self::WhisperModelMissing(_) => ErrorCode::WhisperModelMissing,
            Self::GeminiKeyMissing => ErrorCode::GeminiKeyMissing,
            Self::GeminiKeyInvalid(_) => ErrorCode::GeminiKeyInvalid,
            Self::GeminiQuotaExceeded(_) => ErrorCode::GeminiQuotaExceeded,
            Self::GeminiModelNotFound(_) => ErrorCode::GeminiModelNotFound,
            Self::GeminiBlocked(_) => ErrorCode::GeminiBlocked,
            Self::NoSubtitles => ErrorCode::NoSubtitles,
            Self::Update(_) => ErrorCode::Update,
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
    const CODES: [ErrorCode; 21] = [
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
        ErrorCode::WhisperMissing,
        ErrorCode::WhisperModelMissing,
        ErrorCode::GeminiKeyMissing,
        ErrorCode::GeminiKeyInvalid,
        ErrorCode::GeminiQuotaExceeded,
        ErrorCode::GeminiModelNotFound,
        ErrorCode::GeminiBlocked,
        ErrorCode::NoSubtitles,
        ErrorCode::Update,
        ErrorCode::ToolNotInPath,
    ];

    #[test]
    fn gemini_model_not_found_message_format() {
        let err = AppError::GeminiModelNotFound("gemini-2.5-flash".to_string());
        assert_eq!(
            err.to_string(),
            "Model 'gemini-2.5-flash' is no longer available on Gemini. Select another model (for example gemini-flash-latest) in Settings → Transcription."
        );
    }

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
