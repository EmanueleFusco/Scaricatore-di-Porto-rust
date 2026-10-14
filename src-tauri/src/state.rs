//! Shared application state: persisted settings and the handles the rest of the
//! backend will hang off it (spec §3 `state.rs`, spec §2.8).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult, ErrorCode, ErrorInfo};
use crate::services::manifest::{self, ToolsManifest};
use crate::services::paths;
use crate::services::tools::{self, Lookup};

/// Bump when the shape of [`Settings`] changes, and add a migration step here.
pub const SETTINGS_SCHEMA_VERSION: u32 = 2;

/// Which releases the yt-dlp updater may install (spec §5.5, §11.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum YtdlpChannel {
    /// `yt-dlp -U`
    #[default]
    Stable,
    /// `yt-dlp --update-to nightly`
    Nightly,
}

impl YtdlpChannel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
        }
    }
}

fn channel_from_str(raw: &str) -> Option<YtdlpChannel> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "stable" => Some(YtdlpChannel::Stable),
        "nightly" => Some(YtdlpChannel::Nightly),
        _ => None,
    }
}

fn de_channel<'de, D: serde::Deserializer<'de>>(d: D) -> Result<YtdlpChannel, D::Error> {
    let raw = String::deserialize(d)?;
    Ok(channel_from_str(&raw).unwrap_or_default())
}

/// UI theme (spec §11.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow the OS. The effective value comes from the Tauri window theme
    /// API, with `prefers-color-scheme` as the fallback (spec §11.9).
    #[default]
    System,
    Dark,
    Light,
}

/// UI language (spec §11.9). `System` resolves from the OS locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    System,
    It,
    En,
}

impl Language {
    /// `None` when the language must be detected from the OS locale.
    pub const fn explicit(self) -> Option<&'static str> {
        match self {
            Self::It => Some("it"),
            Self::En => Some("en"),
            Self::System => None,
        }
    }
}

fn theme_from_str(raw: &str) -> Option<Theme> {
    match raw {
        "dark" => Some(Theme::Dark),
        "light" => Some(Theme::Light),
        "system" => Some(Theme::System),
        _ => None,
    }
}

fn language_from_str(raw: &str) -> Option<Language> {
    match raw {
        "it" => Some(Language::It),
        "en" => Some(Language::En),
        "system" => Some(Language::System),
        _ => None,
    }
}

fn de_theme<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Theme, D::Error> {
    let raw = String::deserialize(d)?;
    Ok(theme_from_str(raw.trim().to_ascii_lowercase().as_str()).unwrap_or_default())
}

fn de_language<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Language, D::Error> {
    let raw = String::deserialize(d)?;
    Ok(language_from_str(raw.trim().to_ascii_lowercase().as_str()).unwrap_or_default())
}

/// The persisted configuration document (spec §2.6, §2.8).
///
/// Every field has a default, unknown keys are ignored, and the file is written
/// atomically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub schema_version: u32,
    #[serde(deserialize_with = "de_language")]
    pub language: Language,
    #[serde(deserialize_with = "de_theme")]
    pub theme: Theme,
    /// `None` means "use the OS Downloads folder".
    pub default_output_dir: Option<PathBuf>,
    /// Spec §5.2 lookup order #1: "Use my own path" from the Tools page, keyed
    /// by tool name (`yt-dlp`, `qjs`, `ffmpeg`, `ffprobe`, …).
    ///
    /// A `BTreeMap` rather than a `HashMap` so the serialised settings file has
    /// a stable key order: a document that reorders itself on every save makes
    /// every diff noisy and hides real changes.
    pub tool_paths: BTreeMap<String, PathBuf>,
    /// Spec §5.5, §11.8: which channel `-U`/`--update-to` installs.
    #[serde(deserialize_with = "de_channel")]
    pub ytdlp_channel: YtdlpChannel,
    /// The Setup wizard has been completed (spec §11.1: "Shown once,
    /// re-openable from Settings → Tools"). Kept separate from the tools
    /// themselves so the app can show the wizard again on request.
    pub setup_wizard_completed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            language: Language::default(),
            theme: Theme::default(),
            default_output_dir: None,
            tool_paths: BTreeMap::new(),
            ytdlp_channel: YtdlpChannel::default(),
            setup_wizard_completed: false,
        }
    }
}

impl Settings {
    /// Normalise anything a user (or an older build) may have stored.
    pub fn normalise(&mut self) {
        self.schema_version = SETTINGS_SCHEMA_VERSION;
        if let Some(dir) = &self.default_output_dir {
            // An empty or whitespace-only path is the same as "not set".
            if dir.as_os_str().is_empty() {
                self.default_output_dir = None;
            }
        }
        // An override must be an absolute path (spec §4.8) and must name a
        // tool: a relative or nameless entry is dropped instead of being
        // silently honoured, because the lookup runs before any validation and
        // `spawn` refuses relative paths.
        self.tool_paths
            .retain(|name, path| !name.trim().is_empty() && path.is_absolute());
    }

    /// The user's override for `tool`, if any.
    pub fn tool_override(&self, tool: &str) -> Option<&Path> {
        self.tool_paths.get(tool).map(PathBuf::as_path)
    }

    pub fn to_json(&self) -> Result<String, AppError> {
        let mut json = serde_json::to_string_pretty(self)?;
        json.push('\n');
        Ok(json)
    }

    pub fn load(path: &Path) -> Result<LoadedSettings, AppError> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(LoadedSettings {
                    settings: Settings::default(),
                    recovery: None,
                });
            }
            Err(err) => return Err(AppError::Io(err)),
        };

        match Self::from_json(&raw) {
            Ok(mut settings) => {
                let needs_resave = settings.schema_version != SETTINGS_SCHEMA_VERSION;
                settings.normalise();
                // Older documents are rewritten so the next start is cheap.
                if needs_resave {
                    Self::save(path, &settings)?;
                }
                Ok(LoadedSettings {
                    settings,
                    recovery: None,
                })
            }
            Err(err) => {
                // Keep the broken file for the human, start from defaults.
                let _ = paths::backup_file(path);
                Err(AppError::SettingsCorrupt {
                    path: path.display().to_string(),
                    detail: err.to_string(),
                })
            }
        }
    }

    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        let settings: Settings = serde_json::from_str(raw)?;
        Ok(settings)
    }

    pub fn save(path: &Path, settings: &Settings) -> Result<(), AppError> {
        let json = settings.to_json()?;
        paths::atomic_write(path, json.as_bytes())?;
        Ok(())
    }
}

/// Result of loading the settings file, plus a description of any recovery.
#[derive(Debug)]
pub struct LoadedSettings {
    pub settings: Settings,
    /// `Some` when a broken file was moved aside and defaults were used.
    pub recovery: Option<Recovery>,
}

/// Frontend-facing description of a recovery, so the UI can show a toast.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recovery {
    pub backup_path: Option<String>,
    pub code: String,
    pub message_key: String,
    pub detail: Option<String>,
}

/// Runtime state shared with every command.
pub struct AppState {
    settings: RwLock<Settings>,
    settings_path: PathBuf,
    recovery: Option<Recovery>,
    tools: ToolContext,
}

impl AppState {
    /// Load the settings file, recovering from a corrupt document, and resolve
    /// every directory the tool layer needs (spec §5.2).
    pub fn load(app: &tauri::AppHandle) -> Result<Self, AppError> {
        let context = ToolContext::from_app(app)?;
        Self::from_path_with_tools(paths::settings_file(app)?, context)
    }

    /// Same as [`AppState::load`] but with an explicit settings path and no tool
    /// directories, so tests can stay inside a temp directory.
    pub fn from_path(path: PathBuf) -> Result<Self, AppError> {
        Self::from_path_with_tools(path, ToolContext::isolated())
    }

    /// Full constructor: the settings path **and** where the tools live.
    pub fn from_path_with_tools(path: PathBuf, tools: ToolContext) -> Result<Self, AppError> {
        let outcome = match Settings::load(&path) {
            Ok(loaded) => loaded,
            Err(AppError::SettingsCorrupt { path, detail }) => LoadedSettings {
                settings: Settings::default(),
                recovery: Some(Recovery {
                    backup_path: Some(
                        paths::backup_path(Path::new(&path))
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    code: ErrorCode::SettingsCorrupt.as_str().to_owned(),
                    message_key: ErrorCode::SettingsCorrupt.message_key().to_owned(),
                    detail: Some(detail),
                }),
            },
            Err(other) => return Err(other),
        };

        Ok(Self {
            settings: RwLock::new(outcome.settings),
            settings_path: path,
            recovery: outcome.recovery,
            tools,
        })
    }

    /// Where the tool layer reads its directories from.
    pub fn tools(&self) -> &ToolContext {
        &self.tools
    }

    /// In-memory snapshot; never blocks on `.await` because it is a sync call.
    pub fn settings(&self) -> Settings {
        match self.settings.read() {
            Ok(guard) => guard.clone(),
            // A poisoned lock only means a previous writer panicked; the data
            // is still structurally valid, so surface the last known state.
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn save_settings(&self, settings: Settings) -> Result<Settings, AppError> {
        let mut settings = settings;
        settings.normalise();
        Settings::save(&self.settings_path, &settings)?;

        match self.settings.write() {
            Ok(mut guard) => {
                *guard = settings.clone();
                Ok(settings)
            }
            Err(poisoned) => {
                *poisoned.into_inner() = settings.clone();
                Ok(settings)
            }
        }
    }

    pub fn recovery(&self) -> Option<&Recovery> {
        self.recovery.as_ref()
    }

    pub fn settings_path(&self) -> &Path {
        &self.settings_path
    }
}

/// Everything the tool layer needs that does not change while the app runs
/// (spec §5.2).
///
/// Resolved once at startup: the target triple, where downloaded tools go, where
/// the bundled ones are, and the private temp dir handed to children (spec
/// §4.9). Doing it per call would mean re-deriving the same answers from the
/// filesystem on every probe, and a lookup that changes its mind between two
/// calls in the same operation is a bug that only shows up on someone else's
/// machine.
#[derive(Debug, Clone)]
pub struct ToolContext {
    manifest: ToolsManifest,
    triple: String,
    tools_dir: PathBuf,
    bundled_dir: PathBuf,
    temp_dir: Option<PathBuf>,
}

impl ToolContext {
    /// Resolve from the running app.
    pub fn from_app(app: &tauri::AppHandle) -> Result<Self, AppError> {
        let manifest =
            ToolsManifest::embedded().map_err(|err| AppError::Unsupported(err.to_string()))?;
        let triple = manifest::current_target_triple().map_err(AppError::Unsupported)?;
        let tools_dir = paths::tools_dir(app)?;
        // The bundled directory is where the executable lives; if that cannot be
        // determined the app still works, it simply has no bundled baseline and
        // the Tools page says so.
        let bundled_dir = tools::bundled_dir().unwrap_or_else(|| tools_dir.clone());
        let temp_dir = paths::private_temp_dir(app).ok();
        Ok(Self {
            manifest,
            triple,
            tools_dir,
            bundled_dir,
            temp_dir,
        })
    }

    /// Everything empty, for tests that only exercise the settings commands.
    pub fn isolated() -> Self {
        let root = std::env::temp_dir().join("scaricatore-di-porto-isolated");
        Self {
            manifest: ToolsManifest {
                schema_version: manifest::SCHEMA_VERSION,
                tools: Vec::new(),
            },
            triple: String::new(),
            tools_dir: root.join("tools"),
            bundled_dir: root.join("bundled"),
            temp_dir: None,
        }
    }

    /// A context pointed at directories the caller controls.
    pub fn in_dir(root: &Path) -> Self {
        Self {
            manifest: ToolsManifest::embedded().unwrap_or_else(|err| panic!("{err}")),
            triple: manifest::current_target_triple().unwrap_or_default(),
            tools_dir: root.join("tools"),
            bundled_dir: root.join("bundled"),
            temp_dir: Some(root.join("tmp")),
        }
    }

    pub fn manifest(&self) -> &ToolsManifest {
        &self.manifest
    }

    pub fn triple(&self) -> &str {
        &self.triple
    }

    pub fn tools_dir(&self) -> &Path {
        &self.tools_dir
    }

    pub fn bundled_dir(&self) -> &Path {
        &self.bundled_dir
    }

    pub fn temp_dir(&self) -> Option<&Path> {
        self.temp_dir.as_deref()
    }

    /// Build the spec §5.2 lookup for `tool`, honouring the user's override.
    pub fn lookup(&self, settings: &Settings, tool: &str) -> Lookup {
        Lookup {
            override_path: settings.tool_override(tool).map(Path::to_path_buf),
            tools_dir: self.tools_dir.clone(),
            bundled_dir: self.bundled_dir.clone(),
            path_dirs: tools::path_dirs(std::env::var("PATH").ok().as_deref()),
            well_known_dirs: tools::well_known_dirs(paths::home_dir().as_deref()),
        }
    }
}

/// Payload of the `settings_get` command.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub settings: Settings,
    pub recovery: Option<Recovery>,
    pub settings_path: String,
}

/// Build the view returned to the frontend, mapping IO failures to
/// [`ErrorInfo`] as every command must.
pub fn settings_view(state: &AppState) -> AppResult<SettingsView> {
    Ok(SettingsView {
        settings: state.settings(),
        recovery: state.recovery().cloned(),
        settings_path: state.settings_path().display().to_string(),
    })
}

/// Helper so a failed command returns `ErrorInfo` without an explicit map at
/// every call site.
pub trait IntoAppResult<T> {
    fn app_result(self) -> AppResult<T>;
}

impl<T, E: Into<AppError>> IntoAppResult<T> for std::result::Result<T, E> {
    fn app_result(self) -> AppResult<T> {
        self.map_err(|e| {
            let e: AppError = e.into();
            ErrorInfo::new(e.code(), e.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn settings_path(dir: &TempDir) -> PathBuf {
        dir.path().join("settings.json")
    }

    #[test]
    fn missing_fields_get_their_defaults() {
        let s: Settings = serde_json::from_str("{}").expect("parse");
        assert_eq!(s, Settings::default());
        assert_eq!(s.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(s.language, Language::System);
        assert_eq!(s.theme, Theme::System);
        assert!(s.default_output_dir.is_none());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let raw = r#"{"theme":"dark","somethingFromTheFuture":{"a":1},"n":7}"#;
        let s: Settings = serde_json::from_str(raw).expect("parse");
        assert_eq!(s.theme, Theme::Dark);
    }

    #[test]
    fn a_partially_written_document_still_loads() {
        let raw = r#"{"theme":"light","defaultOutputDir":"/tmp/out"}"#;
        let s: Settings = serde_json::from_str(raw).expect("parse");
        assert_eq!(s.theme, Theme::Light);
        assert_eq!(s.default_output_dir.as_deref(), Some(Path::new("/tmp/out")));
        assert_eq!(s.language, Language::System, "missing field falls back");
    }

    #[test]
    fn values_serialise_in_the_frontend_shape() {
        let s = Settings {
            language: Language::En,
            theme: Theme::Dark,
            default_output_dir: None,
            ..Settings::default()
        };
        let json = serde_json::to_value(&s).expect("serialise");
        assert_eq!(json["language"], "en");
        assert_eq!(json["theme"], "dark");
        assert_eq!(json["schemaVersion"], SETTINGS_SCHEMA_VERSION);
        assert!(json["defaultOutputDir"].is_null());
        assert_eq!(json["ytdlpChannel"], "stable");
        assert!(json["setupWizardCompleted"].is_boolean());
        assert!(json["toolPaths"].is_object());
    }

    #[test]
    fn a_tool_override_survives_a_round_trip_and_a_relative_one_is_dropped() {
        let dir = TempDir::new().expect("temp dir");
        let path = settings_path(&dir);
        let mut s = Settings::default();
        // Spelled the way this host spells an absolute path. `/opt/mine/qjs` is rooted
        // but driveless, so `is_absolute()` is false for it on Windows and
        // `normalise` correctly drops it as relative — the same code, a
        // different answer, and a test written only on Linux cannot see it.
        let absolute = if cfg!(windows) {
            r"C:\opt\mine\qjs"
        } else {
            "/opt/mine/qjs"
        };
        s.tool_paths
            .insert("qjs".to_owned(), PathBuf::from(absolute));
        s.tool_paths
            .insert("ffmpeg".to_owned(), PathBuf::from("relative/ffmpeg"));
        s.tool_paths.insert(
            String::new(),
            PathBuf::from(if cfg!(windows) { r"C:\x" } else { "/x" }),
        );

        Settings::save(&path, &s).expect("save");
        let loaded = Settings::load(&path).expect("load");
        assert_eq!(
            loaded.settings.tool_override("qjs"),
            Some(Path::new(absolute))
        );
        // A relative path would be refused by `process.rs` (spec §4.8); it is
        // dropped at normalisation rather than failing every later lookup.
        assert_eq!(loaded.settings.tool_override("ffmpeg"), None);
        assert_eq!(loaded.settings.tool_paths.len(), 1);
    }

    #[test]
    fn an_unknown_ytdlp_channel_degrades_to_stable() {
        let parsed: Settings = serde_json::from_str(r#"{"ytdlpChannel":"weekly"}"#).expect("parse");
        assert_eq!(parsed.ytdlp_channel, YtdlpChannel::Stable);
        let parsed: Settings =
            serde_json::from_str(r#"{"ytdlpChannel":"NIGHTLY"}"#).expect("parse");
        assert_eq!(parsed.ytdlp_channel, YtdlpChannel::Nightly);
    }

    #[test]
    fn a_document_without_the_m0c_fields_still_loads_and_is_upgraded() {
        // This is what a real M0-A settings file looks like.
        let dir = TempDir::new().expect("temp dir");
        let path = settings_path(&dir);
        fs::write(&path, br#"{"schemaVersion":1,"theme":"dark"}"#).expect("write");

        let loaded = Settings::load(&path).expect("load");
        assert_eq!(loaded.settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(loaded.settings.theme, Theme::Dark);
        assert_eq!(loaded.settings.ytdlp_channel, YtdlpChannel::Stable);
        assert!(!loaded.settings.setup_wizard_completed);
        assert!(loaded.settings.tool_paths.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TempDir::new().expect("temp dir");
        let path = settings_path(&dir);
        let s = Settings {
            language: Language::It,
            theme: Theme::Light,
            default_output_dir: Some(PathBuf::from("/downloads")),
            ..Settings::default()
        };

        Settings::save(&path, &s).expect("save");
        let loaded = Settings::load(&path).expect("load");
        assert!(loaded.recovery.is_none());
        assert_eq!(loaded.settings, s);
    }

    #[test]
    fn load_on_a_missing_file_returns_defaults() {
        let dir = TempDir::new().expect("temp dir");
        let loaded = Settings::load(&settings_path(&dir)).expect("load");
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.recovery.is_none());
    }

    #[test]
    fn a_corrupt_file_is_backed_up_and_defaults_are_used() {
        let dir = TempDir::new().expect("temp dir");
        let path = settings_path(&dir);
        fs::write(&path, b"{ this is not json").expect("write");

        let err = Settings::load(&path).expect_err("must fail");
        assert!(matches!(err, AppError::SettingsCorrupt { .. }));

        let backup = paths::backup_path(&path);
        assert!(backup.exists(), "the broken file must be kept");
        assert_eq!(
            fs::read_to_string(&backup).expect("read"),
            "{ this is not json"
        );

        // A second start after the recovery is clean again.
        let loaded = Settings::load(&path).expect("load");
        assert_eq!(loaded.settings, Settings::default());
    }

    #[test]
    fn a_truncated_document_is_treated_as_corrupt() {
        let dir = TempDir::new().expect("temp dir");
        let path = settings_path(&dir);
        fs::write(&path, br#"{"theme":"da"#).expect("write");
        assert!(Settings::load(&path).is_err());
        assert!(paths::backup_path(&path).exists());
    }

    #[test]
    fn an_unknown_enum_value_degrades_to_the_default_and_keeps_the_rest() {
        let raw = r#"{"theme":"ultraviolet","language":"en"}"#;
        let parsed: Settings = serde_json::from_str(raw).expect("must not fail the document");
        assert_eq!(parsed.theme, Theme::System);
        assert_eq!(parsed.language, Language::En, "valid values are preserved");
    }

    #[test]
    fn enum_values_are_case_and_whitespace_insensitive() {
        let parsed: Settings = serde_json::from_str(r#"{"theme":" DARK "}"#).expect("parse");
        assert_eq!(parsed.theme, Theme::Dark);
    }

    #[test]
    fn an_older_schema_version_is_upgraded_and_resaved() {
        let dir = TempDir::new().expect("temp dir");
        let path = settings_path(&dir);
        fs::write(&path, br#"{"schemaVersion":0,"theme":"dark"}"#).expect("write");

        let loaded = Settings::load(&path).expect("load");
        assert_eq!(loaded.settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(loaded.settings.theme, Theme::Dark);

        let on_disk: Settings =
            serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("parse");
        assert_eq!(on_disk.schema_version, SETTINGS_SCHEMA_VERSION);
    }

    #[test]
    fn a_newer_schema_version_is_normalised_down_to_the_known_one() {
        let dir = TempDir::new().expect("temp dir");
        let path = settings_path(&dir);
        fs::write(
            &path,
            format!(r#"{{"schemaVersion":{}}}"#, SETTINGS_SCHEMA_VERSION + 1).as_bytes(),
        )
        .expect("write");
        // No migration exists yet, so the document is downgraded and resaved.
        // TODO(spec §2.8): add a real migration when a version 2 schema lands.
        let loaded = Settings::load(&path).expect("load");
        assert_eq!(loaded.settings.schema_version, SETTINGS_SCHEMA_VERSION);
    }

    #[test]
    fn normalise_clears_an_empty_output_dir() {
        let mut s = Settings {
            default_output_dir: Some(PathBuf::new()),
            ..Settings::default()
        };
        s.normalise();
        assert!(s.default_output_dir.is_none());
    }

    #[test]
    fn state_saves_and_reloads_through_the_same_path() {
        let dir = TempDir::new().expect("temp dir");
        let path = settings_path(&dir);
        let state = AppState::from_path(path.clone()).expect("state loads");

        let mut updated = state.settings();
        updated.theme = Theme::Dark;
        updated.language = Language::It;
        let saved = state.save_settings(updated).expect("save");
        assert_eq!(saved.theme, Theme::Dark);

        let reloaded = Settings::load(&path).expect("load");
        assert_eq!(reloaded.settings, saved);
        assert_eq!(state.settings(), saved);
    }

    #[test]
    fn language_explicit_returns_none_for_system() {
        assert_eq!(Language::System.explicit(), None);
        assert_eq!(Language::It.explicit(), Some("it"));
        assert_eq!(Language::En.explicit(), Some("en"));
    }
}
