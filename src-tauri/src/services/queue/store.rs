//! `queue.json`: the persisted queue of spec §7.4.
//!
//! * schema version, items, groups — and **no separate order array**: the
//!   order of the `items` vector *is* the queue order. A second copy of the
//!   order is a second thing that can disagree with the first, and nothing
//!   reads it (see D80);
//! * atomic writes (`temp` + rename) through [`paths::atomic_write`];
//! * a corrupt document is moved to `queue.json.bak`, the queue starts empty
//!   and the caller is told, so the UI can show the toast spec §7.4 asks for;
//! * items that were in an active stage when the app stopped become
//!   **Interrupted** (spec §7.4), with a Resume button — their temp directory
//!   is therefore still theirs, and §4.10's orphan sweep must not delete it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::services::paths;
use crate::services::queue::model::{
    ItemState, LaneConfig, QueueGroup, QueueItem, QueueSummary, aggregate, roll_off_history,
};

/// File name inside the app data dir (spec §7.4).
pub const QUEUE_FILE: &str = "queue.json";

/// Bump when the shape of [`QueueFile`] changes; unknown keys are ignored, so
/// an older build reading a newer document still works.
pub const QUEUE_SCHEMA_VERSION: u32 = 1;

/// The whole document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QueueFile {
    pub schema_version: u32,
    pub items: Vec<QueueItem>,
    pub groups: Vec<QueueGroup>,
    /// "Pause all" survives a restart (spec §7.3): a queue the user paused on
    /// purpose must not start moving again on the next launch.
    pub paused_all: bool,
}

impl Default for QueueFile {
    fn default() -> Self {
        Self {
            schema_version: QUEUE_SCHEMA_VERSION,
            items: Vec::new(),
            groups: Vec::new(),
            paused_all: false,
        }
    }
}

impl QueueFile {
    /// What the app found on disk.
    pub fn to_json(&self) -> Result<String, io::Error> {
        let mut json = serde_json::to_string_pretty(self)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        json.push('\n');
        Ok(json)
    }

    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }

    /// Write atomically (spec §7.4 "temp + rename").
    pub fn save(&self, path: &Path) -> Result<(), io::Error> {
        let json = self.to_json()?;
        paths::atomic_write(path, json.as_bytes())
    }

    /// Spec §7.4: an item in an active stage becomes `Interrupted`.
    ///
    /// It keeps its temp directory and its `.part` files, so Resume re-runs the
    /// stage and yt-dlp continues instead of starting over. `paused_from` is
    /// set to the stage it was in, which is what the Resume button goes back to.
    pub fn recover_interrupted(&mut self) -> usize {
        let mut changed = 0;
        for item in &mut self.items {
            if item.state.is_active() {
                item.paused_from = Some(item.state);
                item.state = ItemState::Interrupted;
                item.control = crate::services::queue::model::ControlSignal::None;
                changed += 1;
            }
            // An automatic retry that was waiting for its timer cannot be
            // resumed on a timer that no longer exists: it becomes queued again
            // and gets a fresh budget, because the app was closed, not the
            // network.
            if item.state == ItemState::Queued {
                item.retry_at = None;
                item.auto_retries = 0;
            }
        }
        changed
    }

    /// The counter the footer shows, computed from the document itself.
    pub fn summary(&self, lanes: LaneConfig) -> QueueSummary {
        aggregate(&self.items, self.paused_all, lanes)
    }
}

/// How a load went, including any recovery the UI has to be told about.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadedQueue {
    pub file: QueueFile,
    pub recovery: Option<QueueRecovery>,
}

/// Spec §7.4: "Corrupt file → keep a `.bak`, start empty, show a toast".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRecovery {
    pub backup_path: Option<PathBuf>,
    pub detail: String,
    /// Items that were running when the document was written, and are now
    /// `Interrupted` (0 on a clean load).
    pub interrupted: usize,
}

impl QueueRecovery {
    /// The i18n key of the toast, so no sentence is built in the backend.
    pub const fn message_key(&self) -> &'static str {
        if self.backup_path.is_some() {
            "toast.queueRecovered"
        } else {
            "toast.queueInterrupted"
        }
    }
}

/// Read the queue, recovering from what can be recovered.
///
/// Never returns an error: a queue that cannot be read must not stop the app
/// from starting, which is exactly what "start empty, show a toast" means.
pub fn load(path: &Path) -> LoadedQueue {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return LoadedQueue::default();
        }
        Err(err) => {
            return LoadedQueue {
                file: QueueFile::default(),
                recovery: Some(QueueRecovery {
                    backup_path: None,
                    detail: err.to_string(),
                    interrupted: 0,
                }),
            };
        }
    };

    match QueueFile::from_json(&raw) {
        Ok(mut file) => {
            file.schema_version = QUEUE_SCHEMA_VERSION;
            let interrupted = file.recover_interrupted();
            let recovery = (interrupted > 0).then(|| QueueRecovery {
                backup_path: None,
                detail: format!("{interrupted} items were interrupted by a previous exit"),
                interrupted,
            });
            LoadedQueue { file, recovery }
        }
        Err(err) => {
            // Keep the broken file for the human, start from an empty queue.
            let backup = paths::backup_file(path).ok();
            LoadedQueue {
                file: QueueFile::default(),
                recovery: Some(QueueRecovery {
                    backup_path: backup,
                    detail: err.to_string(),
                    interrupted: 0,
                }),
            }
        }
    }
}

/// The ids of the temp directories the queue still owns (spec §4.10, §7.3).
///
/// Anything the queue can still run, resume or retry keeps its directory —
/// that is where the `.part` files live. A finished, failed or cancelled item
/// does not: its directory is debris and §4.10 asks for it to be removed.
///
/// The ids are the *directory names*, not the item ids, so the caller can match
/// `.sdp-tmp-<id>` directly.
pub fn owned_temp_names(file: &QueueFile) -> Vec<String> {
    file.items
        .iter()
        .filter(|item| !item.state.is_terminal())
        .map(|item| item.id.clone())
        .collect()
}

/// Apply the history cap (spec §7.3) and return the dropped ids.
pub fn trim_history(file: &mut QueueFile) -> Vec<String> {
    roll_off_history(&mut file.items)
}

/// Where the queue lives (spec §7.4: the app data dir).
pub fn queue_file<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> io::Result<PathBuf> {
    Ok(paths::app_data_dir(app)?.join(QUEUE_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::queue::model::{ItemOptions, Kind, QualityChoice};
    use tempfile::TempDir;

    fn item(id: &str, state: ItemState) -> QueueItem {
        QueueItem {
            id: id.to_owned(),
            url: format!("https://example.com/{id}"),
            state,
            options: ItemOptions {
                kind: Kind::Video,
                quality: QualityChoice::Best,
                output_dir: PathBuf::from("/tmp/out"),
                ..ItemOptions::default()
            },
            ..QueueItem::default()
        }
    }

    #[test]
    fn an_absent_file_is_an_empty_queue_with_no_recovery() {
        let dir = TempDir::new().expect("temp dir");
        let loaded = load(&dir.path().join("queue.json"));
        assert!(loaded.file.items.is_empty());
        assert!(loaded.recovery.is_none());
    }

    #[test]
    fn a_round_trip_preserves_the_items_and_their_order() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("queue.json");
        let file = QueueFile {
            items: vec![
                item("q1", ItemState::Done),
                item("q2", ItemState::Queued),
                item("q3", ItemState::Failed),
            ],
            groups: vec![QueueGroup {
                id: "g1".into(),
                title: "A playlist".into(),
                folder: Some(PathBuf::from("/tmp/out/A playlist")),
                index: 0,
            }],
            paused_all: true,
            ..QueueFile::default()
        };
        file.save(&path).expect("save");
        let loaded = load(&path);
        assert!(loaded.recovery.is_none());
        assert_eq!(loaded.file.items.len(), 3);
        assert_eq!(
            loaded
                .file
                .items
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            vec!["q1", "q2", "q3"],
            "the vector order is the queue order"
        );
        assert_eq!(loaded.file.groups[0].title, "A playlist");
        assert!(loaded.file.paused_all);
    }

    #[test]
    fn a_document_missing_every_field_still_loads() {
        let file: QueueFile = serde_json::from_str("{}").expect("parses");
        assert_eq!(file, QueueFile::default());
        assert_eq!(file.schema_version, QUEUE_SCHEMA_VERSION);
    }

    #[test]
    fn a_document_with_unknown_keys_loads_and_ignores_them() {
        let raw = r#"{
            "schemaVersion": 1,
            "somethingFromTheFuture": {"a": [1, 2]},
            "items": [{"id": "q1", "url": "https://a.example/1", "future": true}]
        }"#;
        let file: QueueFile = serde_json::from_str(raw).expect("parses");
        assert_eq!(file.items.len(), 1);
        assert_eq!(file.items[0].id, "q1");
    }

    #[test]
    fn a_corrupt_file_is_kept_as_a_backup_and_the_queue_starts_empty() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("queue.json");
        fs::write(&path, "{ this is not json").expect("write");

        let loaded = load(&path);
        assert!(loaded.file.items.is_empty());
        let recovery = loaded.recovery.expect("a recovery must be reported");
        assert_eq!(recovery.message_key(), "toast.queueRecovered");
        let backup = recovery.backup_path.expect("the file must be kept");
        assert!(backup.is_file(), "the broken document stays for the human");
        assert_eq!(fs::read_to_string(&backup).unwrap(), "{ this is not json");
        assert!(!path.exists(), "the broken file was moved aside");
    }

    #[test]
    fn active_items_come_back_interrupted_and_keep_their_stage() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("queue.json");
        QueueFile {
            items: vec![
                item("running", ItemState::Downloading),
                item("merging", ItemState::PostProcessing),
                item("done", ItemState::Done),
                item("queued", ItemState::Queued),
            ],
            ..QueueFile::default()
        }
        .save(&path)
        .expect("save");

        let loaded = load(&path);
        let by_id: std::collections::HashMap<_, _> = loaded
            .file
            .items
            .iter()
            .map(|i| (i.id.as_str(), i))
            .collect();
        assert_eq!(by_id["running"].state, ItemState::Interrupted);
        assert_eq!(by_id["running"].paused_from, Some(ItemState::Downloading));
        assert_eq!(
            by_id["merging"].paused_from,
            Some(ItemState::PostProcessing)
        );
        assert_eq!(by_id["done"].state, ItemState::Done);
        assert_eq!(by_id["queued"].state, ItemState::Queued);

        let recovery = loaded.recovery.expect("interrupted items are reported");
        assert_eq!(recovery.interrupted, 2);
        assert_eq!(recovery.message_key(), "toast.queueInterrupted");
        assert!(recovery.backup_path.is_none(), "nothing was broken");
    }

    #[test]
    fn a_pending_automatic_retry_is_requeued_with_a_fresh_budget_on_restart() {
        let mut waiting = item("q1", ItemState::Queued);
        waiting.auto_retries = 1;
        waiting.retry_at = Some(4_102_444_800);
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("queue.json");
        QueueFile {
            items: vec![waiting],
            ..QueueFile::default()
        }
        .save(&path)
        .expect("save");

        let loaded = load(&path);
        let item = &loaded.file.items[0];
        assert_eq!(item.state, ItemState::Queued);
        assert_eq!(item.retry_at, None, "the timer died with the process");
        assert_eq!(item.auto_retries, 0);
    }

    #[test]
    fn temp_directories_belong_to_everything_that_can_still_run() {
        let file = QueueFile {
            items: vec![
                item("running", ItemState::Downloading),
                item("paused", ItemState::Paused),
                item("interrupted", ItemState::Interrupted),
                item("queued", ItemState::Queued),
                item("done", ItemState::Done),
                item("failed", ItemState::Failed),
                item("cancelled", ItemState::Cancelled),
            ],
            ..QueueFile::default()
        };
        let owned = owned_temp_names(&file);
        assert_eq!(owned, vec!["running", "paused", "interrupted", "queued"]);
    }

    #[test]
    fn the_history_cap_is_applied_on_the_document() {
        let mut file = QueueFile {
            items: (0..crate::services::queue::model::HISTORY_LIMIT + 3)
                .map(|n| item(&format!("d{n}"), ItemState::Done))
                .collect(),
            ..QueueFile::default()
        };
        let dropped = trim_history(&mut file);
        assert_eq!(dropped.len(), 3);
        assert_eq!(
            file.items.iter().filter(|i| i.state.is_terminal()).count(),
            crate::services::queue::model::HISTORY_LIMIT
        );
    }

    #[test]
    fn the_summary_of_a_document_needs_no_engine() {
        let file = QueueFile {
            items: vec![
                item("q1", ItemState::Downloading),
                item("q2", ItemState::Queued),
            ],
            paused_all: true,
            ..QueueFile::default()
        };
        let summary = file.summary(LaneConfig::default());
        assert_eq!(summary.running, 1);
        assert_eq!(summary.queued, 1);
        assert!(summary.paused_all);
    }
}
