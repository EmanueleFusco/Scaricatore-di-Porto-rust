//! Whisper model catalogue and model download manager (spec §10.2, §11.6).
//!
//! Provides the pinned model catalogue from `whisper.models.json`, disk-space checks,
//! Range-resumed downloads into `.part` files, sha256 checksum verification,
//! VAD model co-downloading, model importing from local files, and deletion.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::services::download::{self, DownloadProgress, DownloadRequest, Transport};
use crate::services::ytdlp::ProgressThrottle;

/// Pinned Whisper model catalogue JSON embedded into the binary.
pub const MODELS_CATALOGUE_JSON: &str = include_str!("../../../whisper.models.json");

/// Subdirectory inside app data directory where models live.
pub const MODELS_SUBDIR: &str = "models";

// ---------------------------------------------------------------------------
// Catalogue Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogue {
    pub schema_version: u32,
    pub vad: VadModelEntry,
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VadModelEntry {
    pub id: String,
    pub label: String,
    pub file: String,
    pub url: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    pub id: String,
    pub label: String,
    pub file: String,
    pub url: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub ram_estimate: String,
    pub tier: String,
    pub indicative_wer: Option<f32>,
    pub multilingual: bool,
    pub recommended: bool,
    pub advanced: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

impl ModelCatalogue {
    /// Parse and validate the embedded model catalogue.
    pub fn embedded() -> Result<Self, AppError> {
        Self::parse(MODELS_CATALOGUE_JSON)
    }

    /// Parse catalogue from JSON string.
    pub fn parse(raw: &str) -> Result<Self, AppError> {
        let catalogue: Self = serde_json::from_str(raw)?;
        catalogue.validate()?;
        Ok(catalogue)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.schema_version != 1 {
            return Err(AppError::Unsupported(format!(
                "Unsupported schemaVersion: {}",
                self.schema_version
            )));
        }
        if self.vad.id.is_empty()
            || self.vad.file.is_empty()
            || !crate::services::manifest::is_safe_asset_name(&self.vad.file)
            || !self.vad.file.ends_with(".bin")
            || self.vad.sha256.len() != 64
        {
            return Err(AppError::Unsupported(
                "Invalid VAD model entry in catalogue".to_owned(),
            ));
        }
        if self.models.is_empty() {
            return Err(AppError::Unsupported(
                "Model catalogue has no models".to_owned(),
            ));
        }
        for model in &self.models {
            if model.id.is_empty()
                || model.file.is_empty()
                || !crate::services::manifest::is_safe_asset_name(&model.file)
                || !model.file.ends_with(".bin")
                || model.sha256.len() != 64
            {
                return Err(AppError::Unsupported(format!(
                    "Invalid model entry in catalogue: {}",
                    model.id
                )));
            }
        }
        Ok(())
    }

    pub fn find(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == id)
    }

    pub fn find_by_file(&self, file: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.file == file)
    }

    pub fn find_by_sha256(&self, sha: &str) -> Option<&ModelEntry> {
        let sha_lower = sha.to_ascii_lowercase();
        self.models
            .iter()
            .find(|m| m.sha256.eq_ignore_ascii_case(&sha_lower))
    }

    /// Resolves the download URL, applying any mirror/base URL override.
    pub fn effective_url(&self, model: &ModelEntry, mirror_base: Option<&str>) -> String {
        assert!(
            crate::services::manifest::is_safe_asset_name(&model.file),
            "model file name must be safe: {}",
            model.file
        );
        match mirror_base.map(str::trim).filter(|s| !s.is_empty()) {
            Some(base) => {
                let clean_base = base.trim_end_matches('/');
                format!("{}/{}", clean_base, model.file)
            }
            None => model.url.clone(),
        }
    }

    /// Resolves the VAD model download URL, applying any mirror/base URL override.
    pub fn effective_vad_url(&self, mirror_base: Option<&str>) -> String {
        assert!(
            crate::services::manifest::is_safe_asset_name(&self.vad.file),
            "VAD file name must be safe: {}",
            self.vad.file
        );
        match mirror_base.map(str::trim).filter(|s| !s.is_empty()) {
            Some(base) => {
                let clean_base = base.trim_end_matches('/');
                format!("{}/{}", clean_base, self.vad.file)
            }
            None => self.vad.url.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Disk Space Helper
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn platform_available_space_bytes(path: &Path) -> io::Result<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let res = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if res != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
}

#[cfg(windows)]
fn platform_available_space_bytes(path: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut free_bytes_available: u64 = 0;
    let res = unsafe {
        windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            windows::core::PCWSTR(wide.as_ptr()),
            Some(&mut free_bytes_available),
            None,
            None,
        )
    };
    if res.is_ok() {
        Ok(free_bytes_available)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Available free disk space in bytes for the volume holding `target`.
/// Walks up ancestors if `target` does not exist yet.
pub fn free_disk_space(target: &Path) -> io::Result<u64> {
    let existing = target.ancestors().find(|p| p.exists()).unwrap_or(target);
    platform_available_space_bytes(existing)
}

// ---------------------------------------------------------------------------
// Model Status & View Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ModelInstallState {
    NotInstalled,
    Downloading {
        received_bytes: u64,
        total_bytes: Option<u64>,
    },
    Paused {
        received_bytes: u64,
        total_bytes: Option<u64>,
    },
    Installed {
        size_bytes: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelViewItem {
    pub id: String,
    pub label: String,
    pub file: String,
    pub size_bytes: u64,
    pub ram_estimate: String,
    pub tier: String,
    pub indicative_wer: Option<f32>,
    pub multilingual: bool,
    pub recommended: bool,
    pub advanced: bool,
    pub notes: Option<String>,
    pub state: ModelInstallState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VadStatusView {
    pub id: String,
    pub installed: bool,
    pub file: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsView {
    pub models: Vec<ModelViewItem>,
    pub active_model: Option<String>,
    pub recommended_model: String,
    pub low_core_system: bool,
    pub free_space_bytes: u64,
    pub vad: VadStatusView,
    pub models_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProgressEvent {
    pub id: String,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub percent: Option<f32>,
    pub phase: String, // "vad" or "model"
}

/// Throttle for model download progress events (spec §7.6, §10.2).
///
/// Coalesces rapid TCP chunk events to ~10/s (100ms window), but always emits
/// 0%, 100%, and phase changes ("vad" -> "model").
#[derive(Debug)]
pub struct ModelProgressThrottle {
    throttle: ProgressThrottle,
    last_phase: Option<String>,
    has_emitted_zero: bool,
    has_emitted_hundred: bool,
}

impl ModelProgressThrottle {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            throttle: ProgressThrottle::new(min_interval),
            last_phase: None,
            has_emitted_zero: false,
            has_emitted_hundred: false,
        }
    }

    pub fn should_emit(&mut self, event: &ModelProgressEvent, now: Instant) -> bool {
        let phase_changed = self.last_phase.as_deref() != Some(&event.phase);
        if phase_changed {
            self.last_phase = Some(event.phase.clone());
            let is_zero = event.percent.is_some_and(|p| p <= 0.0) || event.received_bytes == 0;
            let is_hundred = event.percent.is_some_and(|p| p >= 100.0)
                || event
                    .total_bytes
                    .is_some_and(|tot| tot > 0 && event.received_bytes >= tot);
            self.has_emitted_zero = is_zero;
            self.has_emitted_hundred = is_hundred;
            self.throttle.mark_emitted(now);
            return true;
        }

        let is_zero = event.percent.is_some_and(|p| p <= 0.0) || event.received_bytes == 0;
        if is_zero && !self.has_emitted_zero {
            self.has_emitted_zero = true;
            self.throttle.mark_emitted(now);
            return true;
        }

        let is_hundred = event.percent.is_some_and(|p| p >= 100.0)
            || event
                .total_bytes
                .is_some_and(|tot| tot > 0 && event.received_bytes >= tot);
        if is_hundred && !self.has_emitted_hundred {
            self.has_emitted_hundred = true;
            self.throttle.mark_emitted(now);
            return true;
        }

        self.throttle.should_emit(now)
    }
}

/// Wraps a model progress sink with a throttle (e.g. 100ms interval).
pub fn throttled_model_progress_sink<F>(
    min_interval: Duration,
    sink: F,
) -> impl FnMut(ModelProgressEvent) + Send + Sync + 'static
where
    F: Fn(ModelProgressEvent) + Send + Sync + 'static,
{
    let throttle = Mutex::new(ModelProgressThrottle::new(min_interval));
    move |event| {
        let should_emit = match throttle.lock() {
            Ok(mut g) => g.should_emit(&event, Instant::now()),
            Err(p) => p.into_inner().should_emit(&event, Instant::now()),
        };
        if should_emit {
            sink(event);
        }
    }
}

// ---------------------------------------------------------------------------
// Recommendation Logic
// ---------------------------------------------------------------------------

/// Recommends default model ID and returns whether the machine is a low-core system
/// (spec §10.2: default Turbo q5_0, or Small q5_1 if cores <= 4).
pub fn recommend_model() -> (&'static str, bool) {
    let cores = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);
    if cores <= 4 {
        ("small-q5_1", true)
    } else {
        ("large-v3-turbo-q5_0", false)
    }
}

// ---------------------------------------------------------------------------
// Model Status Inspection
// ---------------------------------------------------------------------------

pub fn is_vad_installed(models_dir: &Path, vad: &VadModelEntry) -> bool {
    if !crate::services::manifest::is_safe_asset_name(&vad.file) {
        return false;
    }
    assert!(
        crate::services::manifest::is_safe_asset_name(&vad.file),
        "VAD file name must be safe: {}",
        vad.file
    );
    let path = models_dir.join(&vad.file);
    path.is_file() && fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false)
}

pub fn inspect_models(
    models_dir: &Path,
    catalogue: &ModelCatalogue,
    active_model: Option<&str>,
    downloading_models: &HashMap<String, DownloadProgress>,
) -> ModelsView {
    let (rec_model, low_core) = recommend_model();
    let free_space = free_disk_space(models_dir).unwrap_or(0);
    let vad_installed = is_vad_installed(models_dir, &catalogue.vad);

    let vad_status = VadStatusView {
        id: catalogue.vad.id.clone(),
        installed: vad_installed,
        file: catalogue.vad.file.clone(),
        size_bytes: catalogue.vad.size_bytes,
    };

    let items = catalogue
        .models
        .iter()
        .map(|model| {
            if !crate::services::manifest::is_safe_asset_name(&model.file) {
                return ModelViewItem {
                    id: model.id.clone(),
                    label: model.label.clone(),
                    file: model.file.clone(),
                    ram_estimate: model.ram_estimate.clone(),
                    tier: model.tier.clone(),
                    indicative_wer: model.indicative_wer,
                    multilingual: model.multilingual,
                    notes: model.notes.clone(),
                    recommended: model.recommended,
                    advanced: model.advanced,
                    size_bytes: model.size_bytes,
                    state: ModelInstallState::NotInstalled,
                };
            }
            assert!(
                crate::services::manifest::is_safe_asset_name(&model.file),
                "model file name must be safe: {}",
                model.file
            );
            let final_path = models_dir.join(&model.file);
            let part_path = models_dir.join(format!("{}.part", model.file));

            let state = if let Some(prog) = downloading_models.get(&model.id) {
                ModelInstallState::Downloading {
                    received_bytes: prog.received,
                    total_bytes: prog.total.or(Some(model.size_bytes)),
                }
            } else if final_path.is_file() {
                let size = fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
                ModelInstallState::Installed { size_bytes: size }
            } else if part_path.is_file() {
                let part_size = fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);
                ModelInstallState::Paused {
                    received_bytes: part_size,
                    total_bytes: Some(model.size_bytes),
                }
            } else {
                ModelInstallState::NotInstalled
            };

            ModelViewItem {
                id: model.id.clone(),
                label: model.label.clone(),
                file: model.file.clone(),
                size_bytes: model.size_bytes,
                ram_estimate: model.ram_estimate.clone(),
                tier: model.tier.clone(),
                indicative_wer: model.indicative_wer,
                multilingual: model.multilingual,
                recommended: model.id == rec_model,
                advanced: model.advanced,
                notes: model.notes.clone(),
                state,
            }
        })
        .collect();

    ModelsView {
        models: items,
        active_model: active_model.map(str::to_owned),
        recommended_model: rec_model.to_owned(),
        low_core_system: low_core,
        free_space_bytes: free_space,
        vad: vad_status,
        models_dir: models_dir.to_path_buf(),
    }
}

// ---------------------------------------------------------------------------
// Model Download Manager
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct ModelDownloadManager {
    tokens: Arc<Mutex<HashMap<String, CancellationToken>>>,
    in_flight_progress: Arc<Mutex<HashMap<String, DownloadProgress>>>,
}

impl ModelDownloadManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn in_flight_progress(&self) -> HashMap<String, DownloadProgress> {
        self.in_flight_progress
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn is_downloading(&self, id: &str) -> bool {
        self.tokens
            .lock()
            .map(|g| g.contains_key(id))
            .unwrap_or(false)
    }

    pub fn pause(&self, id: &str) -> bool {
        let mut tokens = match self.tokens.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if let Some(token) = tokens.remove(id) {
            token.cancel();
            if let Ok(mut prog) = self.in_flight_progress.lock() {
                prog.remove(id);
            }
            true
        } else {
            false
        }
    }

    pub fn cancel(&self, id: &str) -> bool {
        self.pause(id)
    }

    /// Download model (and VAD model if missing) with Range resume, sha256 check, and disk space guard.
    pub async fn download_model<F>(
        &self,
        models_dir: &Path,
        catalogue: &ModelCatalogue,
        model_id: &str,
        mirror_base: Option<&str>,
        transport: Transport,
        on_progress: F,
    ) -> Result<PathBuf, AppError>
    where
        F: FnMut(ModelProgressEvent) + Send + Sync + 'static,
    {
        let model = catalogue
            .find(model_id)
            .ok_or_else(|| AppError::ModelNotFound(model_id.to_owned()))?;

        // Ensure models directory exists
        fs::create_dir_all(models_dir)?;

        // Pre-flight: Check free disk space
        let free_space = free_disk_space(models_dir).unwrap_or(0);
        let vad_needed = if is_vad_installed(models_dir, &catalogue.vad) {
            0
        } else {
            let vad_part = models_dir.join(format!("{}.part", catalogue.vad.file));
            let vad_existing = fs::metadata(&vad_part).map(|m| m.len()).unwrap_or(0);
            catalogue.vad.size_bytes.saturating_sub(vad_existing)
        };
        let model_part = models_dir.join(format!("{}.part", model.file));
        let model_existing = fs::metadata(&model_part).map(|m| m.len()).unwrap_or(0);
        let model_needed = model.size_bytes.saturating_sub(model_existing);
        let total_needed = model_needed + vad_needed + 10 * 1024 * 1024; // 10 MB safety margin

        if free_space > 0 && free_space < total_needed {
            return Err(AppError::DiskFull {
                needed: total_needed,
                available: free_space,
            });
        }

        let token = CancellationToken::new();
        {
            let mut tokens = match self.tokens.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            tokens.insert(model_id.to_owned(), token.clone());
        }

        let initial_received = model_existing;
        if let Ok(mut prog) = self.in_flight_progress.lock() {
            prog.insert(
                model_id.to_owned(),
                DownloadProgress {
                    received: initial_received,
                    total: Some(model.size_bytes),
                },
            );
        }

        let progress_map = Arc::clone(&self.in_flight_progress);

        let progress_cb = Arc::new(Mutex::new(on_progress));

        // 1. Download VAD model first if not installed (spec §10.2: "fetched together with first model")
        if !is_vad_installed(models_dir, &catalogue.vad) {
            if !crate::services::manifest::is_safe_asset_name(&catalogue.vad.file) {
                return Err(AppError::Unsupported(format!(
                    "Unsafe VAD model file name: {}",
                    catalogue.vad.file
                )));
            }
            assert!(
                crate::services::manifest::is_safe_asset_name(&catalogue.vad.file),
                "VAD file name must be safe: {}",
                catalogue.vad.file
            );
            let vad_dest = models_dir.join(&catalogue.vad.file);
            let vad_url = catalogue.effective_vad_url(mirror_base);
            let mut vad_req =
                DownloadRequest::new(vad_url, &vad_dest).with_checksum(&catalogue.vad.sha256);
            vad_req.transport = transport;

            let mut vad_sink = {
                let id = catalogue.vad.id.clone();
                let p_map = Arc::clone(&progress_map);
                let m_id = model_id.to_owned();
                let cb = Arc::clone(&progress_cb);
                move |p: DownloadProgress| {
                    if let Ok(mut map) = p_map.lock() {
                        map.insert(m_id.clone(), p);
                    }
                    let pct = p.total.map(|t| (p.received as f32 / t as f32) * 100.0);
                    if let Ok(mut f) = cb.lock() {
                        f(ModelProgressEvent {
                            id: id.clone(),
                            received_bytes: p.received,
                            total_bytes: p.total,
                            percent: pct,
                            phase: "vad".to_owned(),
                        });
                    }
                }
            };

            let outcome = download::download(&vad_req, &token, &mut vad_sink).await;
            if let Err(err) = outcome {
                if let Ok(mut tokens) = self.tokens.lock() {
                    tokens.remove(model_id);
                }
                if let Ok(mut prog) = self.in_flight_progress.lock() {
                    prog.remove(model_id);
                }
                return Err(err);
            }
        }

        // 2. Download the requested Whisper model
        if !crate::services::manifest::is_safe_asset_name(&model.file) {
            return Err(AppError::Unsupported(format!(
                "Unsafe model file name: {}",
                model.file
            )));
        }
        assert!(
            crate::services::manifest::is_safe_asset_name(&model.file),
            "model file name must be safe: {}",
            model.file
        );
        let model_dest = models_dir.join(&model.file);
        let model_url = catalogue.effective_url(model, mirror_base);
        let mut model_req =
            DownloadRequest::new(model_url, &model_dest).with_checksum(&model.sha256);
        model_req.transport = transport;

        let model_id_owned = model_id.to_owned();
        let outcome = {
            let p_map = Arc::clone(&progress_map);
            let m_id = model_id_owned.clone();
            let cb = Arc::clone(&progress_cb);
            let mut sink = move |p: DownloadProgress| {
                if let Ok(mut map) = p_map.lock() {
                    map.insert(m_id.clone(), p);
                }
                let pct = p.total.map(|t| (p.received as f32 / t as f32) * 100.0);
                if let Ok(mut f) = cb.lock() {
                    f(ModelProgressEvent {
                        id: model_id_owned.clone(),
                        received_bytes: p.received,
                        total_bytes: p.total,
                        percent: pct,
                        phase: "model".to_owned(),
                    });
                }
            };

            download::download(&model_req, &token, &mut sink).await
        };

        // Cleanup token and progress tracking
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.remove(model_id);
        }
        if let Ok(mut prog) = self.in_flight_progress.lock() {
            prog.remove(model_id);
        }

        let outcome = outcome?;
        Ok(outcome.path)
    }
}

// ---------------------------------------------------------------------------
// Model Operations: Delete & Import
// ---------------------------------------------------------------------------

/// Delete a model from disk, freeing storage (spec §10.2).
pub fn delete_model(
    models_dir: &Path,
    catalogue: &ModelCatalogue,
    model_id: &str,
) -> Result<(), AppError> {
    let model = catalogue
        .find(model_id)
        .ok_or_else(|| AppError::ModelNotFound(model_id.to_owned()))?;

    if !crate::services::manifest::is_safe_asset_name(&model.file) {
        return Err(AppError::Unsupported(format!(
            "Unsafe model file name: {}",
            model.file
        )));
    }
    assert!(
        crate::services::manifest::is_safe_asset_name(&model.file),
        "model file name must be safe: {}",
        model.file
    );
    let dest = models_dir.join(&model.file);
    let part = models_dir.join(format!("{}.part", model.file));

    let _ = fs::remove_file(part);
    if dest.is_file() {
        fs::remove_file(dest)?;
    }
    Ok(())
}

/// Cancel download and delete any partial `.part` file from disk (spec §10.2, §11.6).
pub fn cancel_model_download(
    models_dir: &Path,
    catalogue: &ModelCatalogue,
    model_id: &str,
) -> Result<bool, AppError> {
    let model = catalogue
        .find(model_id)
        .ok_or_else(|| AppError::ModelNotFound(model_id.to_owned()))?;

    if !crate::services::manifest::is_safe_asset_name(&model.file) {
        return Err(AppError::Unsupported(format!(
            "Unsafe model file name: {}",
            model.file
        )));
    }
    assert!(
        crate::services::manifest::is_safe_asset_name(&model.file),
        "model file name must be safe: {}",
        model.file
    );
    let part = models_dir.join(format!("{}.part", model.file));
    if part.is_file() {
        fs::remove_file(part)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Computes SHA-256 of any local file.
pub fn compute_file_sha256(path: &Path) -> Result<String, AppError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// Import a local model file into the models directory (spec §10.2: "Import model file… for offline machines").
pub fn import_model(
    models_dir: &Path,
    catalogue: &ModelCatalogue,
    source_path: &Path,
    target_model_id: Option<&str>,
) -> Result<ModelEntry, AppError> {
    if !source_path.is_file() {
        return Err(AppError::FileNotFound(source_path.display().to_string()));
    }

    let actual_sha = compute_file_sha256(source_path)?;

    let model = if let Some(id) = target_model_id {
        let m = catalogue
            .find(id)
            .ok_or_else(|| AppError::ModelNotFound(id.to_string()))?;
        if !m.sha256.eq_ignore_ascii_case(&actual_sha) {
            return Err(AppError::ChecksumMismatch {
                expected: m.sha256.clone(),
                actual: actual_sha,
            });
        }
        m
    } else if let Some(matched) = catalogue.find_by_sha256(&actual_sha) {
        matched
    } else {
        return Err(AppError::UnknownModel(actual_sha));
    };

    if !crate::services::manifest::is_safe_asset_name(&model.file) {
        return Err(AppError::Unsupported(format!(
            "Unsafe model file name: {}",
            model.file
        )));
    }

    fs::create_dir_all(models_dir)?;
    assert!(
        crate::services::manifest::is_safe_asset_name(&model.file),
        "model file name must be safe: {}",
        model.file
    );
    let dest = models_dir.join(&model.file);
    let tmp = models_dir.join(format!("{}.import.tmp", model.file));

    fs::copy(source_path, &tmp)?;
    fs::rename(&tmp, &dest)?;

    Ok(model.clone())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn embedded_catalogue_parses_and_validates() {
        let catalogue = ModelCatalogue::embedded().expect("embedded catalogue must be valid");
        assert_eq!(catalogue.schema_version, 1);
        assert_eq!(catalogue.vad.id, "silero-v5.1.2");
        assert_eq!(catalogue.models.len(), 11);

        let turbo = catalogue
            .find("large-v3-turbo-q5_0")
            .expect("turbo must exist");
        assert_eq!(turbo.label, "Turbo");
        assert_eq!(turbo.file, "ggml-large-v3-turbo-q5_0.bin");
        assert_eq!(turbo.size_bytes, 574041195);
        assert_eq!(
            turbo.sha256,
            "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"
        );
        assert_eq!(turbo.indicative_wer, Some(2.8));
        assert!(turbo.recommended);
    }

    #[test]
    fn model_finder_by_file_and_sha256() {
        let catalogue = ModelCatalogue::embedded().unwrap();
        assert_eq!(
            catalogue
                .find_by_file("ggml-tiny-q5_1.bin")
                .map(|m| m.id.as_str()),
            Some("tiny-q5_1")
        );
        assert_eq!(
            catalogue
                .find_by_sha256("818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7")
                .map(|m| m.id.as_str()),
            Some("tiny-q5_1")
        );
        assert_eq!(catalogue.find_by_file("nonexistent.bin"), None);
    }

    #[test]
    fn effective_urls_handle_custom_mirrors() {
        let catalogue = ModelCatalogue::embedded().unwrap();
        let tiny = catalogue.find("tiny-q5_1").unwrap();

        // Default URL
        assert_eq!(
            catalogue.effective_url(tiny, None),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin"
        );

        // Mirror base URL
        assert_eq!(
            catalogue.effective_url(tiny, Some("https://hf-mirror.com/models")),
            "https://hf-mirror.com/models/ggml-tiny-q5_1.bin"
        );
        assert_eq!(
            catalogue.effective_url(tiny, Some("https://hf-mirror.com/models/")),
            "https://hf-mirror.com/models/ggml-tiny-q5_1.bin"
        );

        // VAD mirror
        assert_eq!(
            catalogue.effective_vad_url(Some("https://hf-mirror.com/vad/")),
            "https://hf-mirror.com/vad/ggml-silero-v5.1.2.bin"
        );
    }

    #[test]
    fn recommend_model_selects_turbo_or_small() {
        let (model, _is_low) = recommend_model();
        assert!(model == "large-v3-turbo-q5_0" || model == "small-q5_1");
    }

    #[test]
    fn free_disk_space_returns_positive_bytes_for_tempdir() {
        let dir = tempdir().unwrap();
        let space = free_disk_space(dir.path()).expect("disk space check must succeed");
        assert!(space > 0);
    }

    #[test]
    fn delete_and_import_model_round_trip() {
        let dir = tempdir().unwrap();
        let models_dir = dir.path().join("models");
        let catalogue = ModelCatalogue::embedded().unwrap();

        // Create a mock model file matching tiny-q5_1 sha256
        let tiny = catalogue.find("tiny-q5_1").unwrap();
        let source_dir = dir.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();
        let source_file = source_dir.join("local-tiny.bin");

        // Write content and compute sha256
        fs::write(&source_file, b"test-model-content").unwrap();
        let actual_sha = compute_file_sha256(&source_file).unwrap();

        // Custom catalogue with this sha
        let mut custom_catalogue = catalogue.clone();
        custom_catalogue.models[0].sha256 = actual_sha.clone();

        // Import matching by sha256
        let imported = import_model(&models_dir, &custom_catalogue, &source_file, None).unwrap();
        assert_eq!(imported.id, "tiny-q5_1");
        assert!(models_dir.join(&tiny.file).is_file());

        // Delete model
        delete_model(&models_dir, &custom_catalogue, "tiny-q5_1").unwrap();
        assert!(!models_dir.join(&tiny.file).exists());
    }

    #[test]
    fn import_fails_when_checksum_mismatches() {
        let dir = tempdir().unwrap();
        let models_dir = dir.path().join("models");
        let catalogue = ModelCatalogue::embedded().unwrap();

        let source_file = dir.path().join("wrong.bin");
        fs::write(&source_file, b"wrong-content").unwrap();

        let err =
            import_model(&models_dir, &catalogue, &source_file, Some("tiny-q5_1")).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("checksum mismatch"));
    }

    #[test]
    fn catalogue_validate_rejects_path_traversal_and_unsafe_filenames() {
        let mut catalogue = ModelCatalogue::embedded().unwrap();

        // Traversal in model.file
        catalogue.models[0].file = "../escaped.bin".to_owned();
        assert!(catalogue.validate().is_err());

        // Absolute path in model.file
        catalogue.models[0].file = "/root/escaped.bin".to_owned();
        assert!(catalogue.validate().is_err());

        // Backslash traversal
        catalogue.models[0].file = r"..\escaped.bin".to_owned();
        assert!(catalogue.validate().is_err());

        // Missing .bin extension
        catalogue.models[0].file = "escaped.txt".to_owned();
        assert!(catalogue.validate().is_err());

        // Hidden file
        catalogue.models[0].file = ".hidden.bin".to_owned();
        assert!(catalogue.validate().is_err());

        // Reset model file and test VAD file
        catalogue.models[0].file = "ggml-tiny-q5_1.bin".to_owned();
        assert!(catalogue.validate().is_ok());

        catalogue.vad.file = "../escaped_vad.bin".to_owned();
        assert!(catalogue.validate().is_err());

        catalogue.vad.file = "escaped_vad.txt".to_owned();
        assert!(catalogue.validate().is_err());
    }

    #[tokio::test]
    async fn model_operations_reject_path_traversal_filenames() {
        let dir = tempdir().unwrap();
        let models_dir = dir.path().join("models");
        fs::create_dir_all(&models_dir).unwrap();

        // Target outside models_dir
        let outside_file = dir.path().join("outside.bin");
        fs::write(&outside_file, b"secret-data").unwrap();

        let mut catalogue = ModelCatalogue::embedded().unwrap();
        catalogue.models[0].file = "../outside.bin".to_owned();

        // delete_model must not delete outside_file and must return error
        let res = delete_model(&models_dir, &catalogue, &catalogue.models[0].id.clone());
        assert!(res.is_err(), "delete_model must reject path traversal");
        assert!(
            outside_file.exists(),
            "outside file must not have been deleted"
        );

        // import_model must not write to outside_file and must return error
        let source_file = dir.path().join("source.bin");
        fs::write(&source_file, b"new-data").unwrap();
        let actual_sha = compute_file_sha256(&source_file).unwrap();
        catalogue.models[0].sha256 = actual_sha;

        let res = import_model(
            &models_dir,
            &catalogue,
            &source_file,
            Some(&catalogue.models[0].id.clone()),
        );
        assert!(res.is_err(), "import_model must reject path traversal");
        assert_eq!(fs::read(&outside_file).unwrap(), b"secret-data");

        // download_model must reject path traversal and not write to outside_file
        let manager = ModelDownloadManager::new();
        let res = manager
            .download_model(
                &models_dir,
                &catalogue,
                &catalogue.models[0].id.clone(),
                None,
                crate::services::download::Transport::loopback_http(),
                |_| {},
            )
            .await;
        assert!(res.is_err(), "download_model must reject path traversal");
        assert_eq!(fs::read(&outside_file).unwrap(), b"secret-data");

        // inspect_models safely handles unsafe model file without path traversal
        let view = inspect_models(
            &models_dir,
            &catalogue,
            None,
            &std::collections::HashMap::new(),
        );
        assert_eq!(view.models[0].state, ModelInstallState::NotInstalled);

        // is_vad_installed safely returns false on unsafe vad file
        let mut vad_bad = catalogue.vad.clone();
        vad_bad.file = "../outside.bin".to_owned();
        assert!(!is_vad_installed(&models_dir, &vad_bad));
    }

    #[test]
    fn inspect_models_does_not_report_idle_part_file_as_downloading() {
        let dir = tempdir().unwrap();
        let models_dir = dir.path().join("models");
        fs::create_dir_all(&models_dir).unwrap();
        let catalogue = ModelCatalogue::embedded().unwrap();
        let tiny = &catalogue.models[0];
        let part_path = models_dir.join(format!("{}.part", tiny.file));
        fs::write(&part_path, vec![0u8; 1024]).unwrap();

        let in_flight = std::collections::HashMap::new();
        let view = inspect_models(&models_dir, &catalogue, None, &in_flight);
        let item = view.models.iter().find(|m| m.id == tiny.id).unwrap();

        assert_eq!(
            item.state,
            ModelInstallState::Paused {
                received_bytes: 1024,
                total_bytes: Some(tiny.size_bytes),
            }
        );
    }

    #[test]
    fn cancel_model_download_removes_part_file() {
        let dir = tempdir().unwrap();
        let models_dir = dir.path().join("models");
        fs::create_dir_all(&models_dir).unwrap();
        let catalogue = ModelCatalogue::embedded().unwrap();
        let tiny = &catalogue.models[0];
        let part_path = models_dir.join(format!("{}.part", tiny.file));
        fs::write(&part_path, vec![0u8; 1024]).unwrap();
        assert!(part_path.is_file());

        let removed = cancel_model_download(&models_dir, &catalogue, &tiny.id).unwrap();
        assert!(removed);
        assert!(!part_path.exists());

        let view = inspect_models(
            &models_dir,
            &catalogue,
            None,
            &std::collections::HashMap::new(),
        );
        let item = view.models.iter().find(|m| m.id == tiny.id).unwrap();
        assert_eq!(item.state, ModelInstallState::NotInstalled);
    }

    #[test]
    fn model_progress_throttle_coalesces_and_preserves_boundaries() {
        let mut throttle = ModelProgressThrottle::new(Duration::from_millis(100));
        let t0 = Instant::now();

        // 1. Initial 0% event for VAD phase: always emitted
        let ev0 = ModelProgressEvent {
            id: "vad-model".to_string(),
            received_bytes: 0,
            total_bytes: Some(1000),
            percent: Some(0.0),
            phase: "vad".to_string(),
        };
        assert!(
            throttle.should_emit(&ev0, t0),
            "initial phase change + 0% must emit"
        );

        // 2. Intermediate update 30ms later: suppressed
        let ev1 = ModelProgressEvent {
            id: "vad-model".to_string(),
            received_bytes: 300,
            total_bytes: Some(1000),
            percent: Some(30.0),
            phase: "vad".to_string(),
        };
        assert!(
            !throttle.should_emit(&ev1, t0 + Duration::from_millis(30)),
            "rapid chunk must be throttled"
        );

        // 3. Intermediate update 100ms later: emitted
        let ev2 = ModelProgressEvent {
            id: "vad-model".to_string(),
            received_bytes: 600,
            total_bytes: Some(1000),
            percent: Some(60.0),
            phase: "vad".to_string(),
        };
        assert!(
            throttle.should_emit(&ev2, t0 + Duration::from_millis(100)),
            "update at throttle interval must emit"
        );

        // 4. Final 100% event 105ms later (only 5ms after ev2): always emitted
        let ev3 = ModelProgressEvent {
            id: "vad-model".to_string(),
            received_bytes: 1000,
            total_bytes: Some(1000),
            percent: Some(100.0),
            phase: "vad".to_string(),
        };
        assert!(
            throttle.should_emit(&ev3, t0 + Duration::from_millis(105)),
            "100% completion must always emit"
        );

        // 5. Subsequent duplicate 100% 110ms later: not spammed
        assert!(
            !throttle.should_emit(&ev3, t0 + Duration::from_millis(110)),
            "duplicate 100% must not spam"
        );

        // 6. Phase change from "vad" to "model" 112ms later: always emitted
        let ev_m0 = ModelProgressEvent {
            id: "tiny-q5_1".to_string(),
            received_bytes: 0,
            total_bytes: Some(10000),
            percent: Some(0.0),
            phase: "model".to_string(),
        };
        assert!(
            throttle.should_emit(&ev_m0, t0 + Duration::from_millis(112)),
            "phase change to model must always emit"
        );

        // 7. Intermediate chunk 115ms later: throttled
        let ev_m1 = ModelProgressEvent {
            id: "tiny-q5_1".to_string(),
            received_bytes: 100,
            total_bytes: Some(10000),
            percent: Some(1.0),
            phase: "model".to_string(),
        };
        assert!(
            !throttle.should_emit(&ev_m1, t0 + Duration::from_millis(115)),
            "rapid model chunk must be throttled"
        );

        // 8. Model 100% completion 120ms later: always emitted
        let ev_m_done = ModelProgressEvent {
            id: "tiny-q5_1".to_string(),
            received_bytes: 10000,
            total_bytes: Some(10000),
            percent: Some(100.0),
            phase: "model".to_string(),
        };
        assert!(
            throttle.should_emit(&ev_m_done, t0 + Duration::from_millis(120)),
            "model 100% completion must always emit"
        );
    }
}
