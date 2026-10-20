//! The queue engine of spec §7: scheduling, the per-item pipeline, the actions
//! and the persistence triggers.
//!
//! ## Shape of the thing
//!
//! One [`QueueEngine`] is a handle over an `Arc<Inner>`. The inner state is a
//! mutex-guarded [`State`] (items, groups, the pause flag); everything else is
//! machinery around it:
//!
//! * a **scheduler task** wakes on `notify` (or on the next retry timer) and
//!   asks [`scheduler::plan`] what to start;
//! * one **task per running item**, which resolves the tools and calls
//!   [`job::run_job`] — the M1 pipeline, unchanged, which is how "keep M1's
//!   behaviour intact" is guaranteed rather than hoped for;
//! * a **flusher task** that turns dirty items into `queue:update` events at
//!   ≤ 10/s per item (spec §7.6).
//!
//! ## Locks
//!
//! Spec §4.4 says never hold a mutex across an `.await` on a child process. The
//! rule is honoured here literally and more widely: **no mutex is held across
//! any `.await`**. State is locked, copied out or mutated in place, and unlocked
//! again; the sink (which emits a Tauri event), the persistence write and the
//! notification all run with no lock held. The only nesting is
//! `state → lanes`, always in that order, and never the other way round.
//!
//! ## Lanes are counters, not semaphores
//!
//! Spec §7.2 says "separate semaphores". This engine keeps the equivalent as
//! counters inside the scheduler, because the scheduler is the only thing that
//! ever starts an item: a permit could not be acquired anywhere else, so a
//! semaphore would only duplicate a count that already exists — and a duplicated
//! count is the kind of thing that disagrees with reality when the user changes
//! the parallelism in Settings mid-download. The behaviour spec §15 item 7 asks
//! for (parallel downloads, per-host cap) is unchanged, and it is testable as a
//! pure function. See D82.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::error::ErrorInfo;
use crate::services::job::{self, JobEnvironment, JobOutcome, JobRequest};
use crate::services::process;
use crate::services::queue::events::{
    Coalescer, EventSink, QueueSnapshot, QueueToast, QueueUpdate,
};
use crate::services::queue::model::{
    ControlSignal, ItemOptions, ItemProgress, ItemState, Kind, Lane, LaneConfig, QueueGroup,
    QueueItem, QueueSummary, aggregate, job_dirs, roll_off_history, should_auto_retry,
};
use crate::services::queue::scheduler::{self, Candidate, Capacity};
use crate::services::queue::store::{self, LoadedQueue, QueueFile};
use crate::services::ytdlp::{JobDirs, JobProgress, YtdlpError, YtdlpErrorCode};

/// A boxed future, so a [`ToolProvider`] can be implemented with any async body
/// without pulling in a futures crate.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Spec §7.2, §5.2: whoever owns the app state builds the environment for an
/// item's options. The engine knows nothing about where tools come from, which
/// is what lets the tests point it at a fake yt-dlp.
pub trait ToolProvider: Send + Sync + 'static {
    /// `Err` means the item cannot run at all (no yt-dlp, no ffmpeg); the queue
    /// records it as an ordinary failure carrying that error.
    fn environment(&self, options: ItemOptions) -> BoxFuture<Result<JobEnvironment, ErrorInfo>>;
}

/// Spec §7.7: the notification when the queue goes idle.
pub trait IdleNotifier: Send + Sync + 'static {
    fn notify(&self, summary: &QueueSummary);
}

/// What the engine is configured with at startup.
pub struct EngineConfig {
    /// `queue.json` (spec §7.4).
    pub path: PathBuf,
    pub provider: Arc<dyn ToolProvider>,
    pub sink: EventSink,
    pub lanes: LaneConfig,
    /// Spec §7.3: "Pause all" survives a restart, so it is read from the file.
    pub notify_on_idle: bool,
    pub notifier: Option<Arc<dyn IdleNotifier>>,
}

/// The result of one action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Applied.
    Ok,
    /// The id belongs to nothing: a stale click in the UI, not an error.
    NotFound,
}

/// An item the queue already holds, so the UI can ask "add anyway?"
/// (spec §7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateRef {
    pub url: String,
    pub item_id: String,
    pub title: Option<String>,
}

/// What `add` did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddOutcome {
    pub added: Vec<String>,
    pub duplicates: Vec<DuplicateRef>,
}

/// The mutable state of the queue.
#[derive(Default)]
struct State {
    items: Vec<QueueItem>,
    groups: Vec<QueueGroup>,
    paused_all: bool,
    /// Set when the order changed since the last event was built.
    order_dirty: bool,
    /// Set when the group list changed since the last event was built.
    groups_dirty: bool,
    /// The counters as of the last time anything finished. The idle
    /// **transition** is what deserves a notification (spec §7.7), not the state
    /// of being idle, and the transition is a comparison between two summaries.
    previous: QueueSummary,
}

impl State {
    fn index_of(&self, id: &str) -> Option<usize> {
        self.items.iter().position(|item| item.id == id)
    }

    fn item(&self, id: &str) -> Option<&QueueItem> {
        let index = self.index_of(id)?;
        self.items.get(index)
    }

    fn item_mut(&mut self, id: &str) -> Option<&mut QueueItem> {
        let index = self.index_of(id)?;
        self.items.get_mut(index)
    }

    fn summary(&self, lanes: LaneConfig) -> QueueSummary {
        aggregate(&self.items, self.paused_all, lanes)
    }
}

/// A running item, so its cancellation token can be reached from a command.
struct Running {
    token: CancellationToken,
}

/// The shared machinery.
struct Inner {
    path: PathBuf,
    state: Mutex<State>,
    running: Mutex<BTreeMap<String, Running>>,
    lanes: Mutex<LaneConfig>,
    notify_on_idle: Mutex<bool>,
    provider: Arc<dyn ToolProvider>,
    notifier: Option<Arc<dyn IdleNotifier>>,
    sink: EventSink,
    coalescer: Mutex<Coalescer>,
    seq: AtomicU64,
    /// Wake the scheduler: something an item could start.
    wake: Notify,
    /// Wake the flusher: something changed.
    dirty: Notify,
    stopped: AtomicBool,
    /// Ids the frontend must forget (removed, or rolled off by the history cap).
    pending_removed: Mutex<Vec<String>>,
    /// Toasts waiting to travel with the next event.
    pending_toasts: Mutex<Vec<QueueToast>>,
    /// Limits concurrent encode jobs (spec §7.2: "The encode lane is 1 by default").
    /// Counted against the live lane config, so a change in Settings also
    /// reaches items already waiting.
    encodes_running: Mutex<usize>,
    /// Woken when an encode slot frees up or the lane capacity changes.
    encode_slot: Notify,
    /// Limits concurrent transcribe jobs (spec §7.2: "The transcribe lane is 1 by default").
    transcribes_running: Mutex<usize>,
    /// Woken when a transcribe slot frees up or the lane capacity changes.
    transcribe_slot: Notify,
}

/// The handle. Cloneable, cheap, and the only thing the commands see.
#[derive(Clone)]
pub struct QueueEngine {
    inner: Arc<Inner>,
}

impl QueueEngine {
    /// Open the queue file, recover what can be recovered, and return an engine
    /// that is not running yet ([`QueueEngine::start`]).
    pub fn open(config: EngineConfig) -> Self {
        let LoadedQueue { file, recovery } = store::load(&config.path);
        let lanes = config.lanes.clamped();
        // Anything still queued makes the queue active at startup, so the first
        // terminal transition is not mistaken for "the queue became idle".
        let previous = file.summary(lanes);
        let engine = Self {
            inner: Arc::new(Inner {
                path: config.path,
                state: Mutex::new(State {
                    items: file.items,
                    groups: file.groups,
                    paused_all: file.paused_all,
                    order_dirty: false,
                    groups_dirty: false,
                    previous,
                }),
                running: Mutex::new(BTreeMap::new()),
                lanes: Mutex::new(lanes),
                notify_on_idle: Mutex::new(config.notify_on_idle),
                provider: config.provider,
                notifier: config.notifier,
                sink: config.sink,
                coalescer: Mutex::new(Coalescer::new(
                    crate::services::queue::events::UPDATE_INTERVAL,
                )),
                seq: AtomicU64::new(0),
                wake: Notify::new(),
                dirty: Notify::new(),
                stopped: AtomicBool::new(false),
                pending_removed: Mutex::new(Vec::new()),
                pending_toasts: Mutex::new(Vec::new()),
                encodes_running: Mutex::new(0),
                encode_slot: Notify::new(),
                transcribes_running: Mutex::new(0),
                transcribe_slot: Notify::new(),
            }),
        };
        if let Some(recovery) = recovery {
            push_toast(
                &engine.inner,
                QueueToast::new(recovery.message_key()).with("count", recovery.interrupted),
            );
        }
        engine
    }

    /// What the frontend needs to render the queue (spec §3 `queue_list`).
    pub fn snapshot(&self) -> QueueSnapshot {
        let lanes = self.lanes();
        let state = lock(&self.inner.state);
        QueueSnapshot {
            seq: self.inner.seq.load(Ordering::Relaxed),
            items: state.items.clone(),
            groups: state.groups.clone(),
            summary: state.summary(lanes),
            // Toasts travel as events; the flusher is holding this one.
            toast: None,
        }
    }

    /// Temp directory names the queue still owns (spec §4.10, §7.3).
    pub fn owned_temp_names(&self) -> Vec<String> {
        let state = lock(&self.inner.state);
        state
            .items
            .iter()
            .filter(|item| !item.state.is_terminal())
            .map(|item| item.id.clone())
            .collect()
    }

    /// Every output folder a live item writes to, so the startup sweep looks in
    /// all of them and not only in the default one.
    pub fn live_output_dirs(&self) -> Vec<PathBuf> {
        let state = lock(&self.inner.state);
        let mut dirs: Vec<PathBuf> = state
            .items
            .iter()
            .filter(|item| !item.state.is_terminal())
            .map(|item| item.options.output_dir.clone())
            .filter(|dir| !dir.as_os_str().is_empty())
            .collect();
        dirs.sort();
        dirs.dedup();
        dirs
    }

    /// Start the scheduler and the flusher.
    ///
    /// The spawns go through **Tauri's** runtime, not a bare `tokio::spawn`:
    /// this is called from `setup`, on the main thread, where no Tokio context
    /// is entered — and a bare `tokio::spawn` there panics with "there is no
    /// reactor running". Found by actually starting the app, which no test
    /// would have: the tests call this from inside a `#[tokio::test]`.
    pub fn start(&self) {
        let scheduler = Arc::clone(&self.inner);
        tauri::async_runtime::spawn(async move { scheduler_loop(scheduler).await });
        let flusher = Arc::clone(&self.inner);
        tauri::async_runtime::spawn(async move { flusher_loop(flusher).await });
    }

    /// Stop both tasks and cancel everything running (spec §4.2: "On app exit
    /// cancel all running items").
    pub fn shutdown(&self) {
        self.inner.stopped.store(true, Ordering::SeqCst);
        {
            let running = lock(&self.inner.running);
            for entry in running.values() {
                entry.token.cancel();
            }
        }
        self.inner.wake.notify_waiters();
        self.inner.dirty.notify_waiters();
    }

    /// The lane configuration in force (spec §7.2, §11.8).
    pub fn lanes(&self) -> LaneConfig {
        *lock(&self.inner.lanes)
    }

    /// Adopt new lane capacities and the notification setting. Takes effect on
    /// the next planning pass, which happens as soon as this returns.
    pub fn configure(&self, lanes: LaneConfig, notify_on_idle: bool) {
        let lanes = lanes.clamped();
        *lock(&self.inner.lanes) = lanes;
        // Items waiting for an encode slot re-check against the new capacity.
        self.inner.encode_slot.notify_waiters();
        // Items waiting for a transcribe slot re-check against the new capacity.
        self.inner.transcribe_slot.notify_waiters();
        *lock(&self.inner.notify_on_idle) = notify_on_idle;
        self.inner.wake.notify_waiters();
    }

    // -----------------------------------------------------------------
    // Adding (spec §2.2, §7.3)
    // -----------------------------------------------------------------

    /// Add items, refusing duplicates unless `force`.
    ///
    /// A duplicate is "the same normalised URL **and** the same options, already
    /// queued" (spec §7.3). A terminal item never counts: downloading a file
    /// the user already has is a legitimate thing to ask for.
    pub fn add(&self, requests: Vec<AddRequest>, force: bool) -> AddOutcome {
        if requests.is_empty() {
            return AddOutcome::default();
        }
        let now = now_unix();
        let mut outcome = AddOutcome::default();
        {
            let mut state = lock(&self.inner.state);
            // What the queue already holds, in one pass: key → item id.
            let mut held: Vec<(String, String)> = state
                .items
                .iter()
                .filter(|item| item.counts_as_duplicate())
                .map(|item| (item.duplicate_key(), item.id.clone()))
                .collect();

            for request in requests {
                let item = request.into_item(next_id(), now);
                let key = item.duplicate_key();
                let duplicate = held
                    .iter()
                    .find(|(other, _)| *other == key)
                    .map(|(_, id)| id.clone());
                match (duplicate, force) {
                    (Some(item_id), false) => {
                        outcome.duplicates.push(DuplicateRef {
                            url: item.url,
                            item_id,
                            title: item.title,
                        });
                    }
                    _ => {
                        held.push((key, item.id.clone()));
                        outcome.added.push(item.id.clone());
                        state.items.push(item);
                    }
                }
            }
            state.order_dirty = true;
        }
        self.after_mutation();
        outcome
    }

    // -----------------------------------------------------------------
    // Actions (spec §7.3)
    // -----------------------------------------------------------------

    /// Pause one item: kill the process, keep the partial files.
    pub fn pause(&self, id: &str) -> Outcome {
        self.signal(id, ControlSignal::Pause)
    }

    /// "Pause all" (spec §7.3).
    pub fn pause_all(&self) {
        {
            let mut state = lock(&self.inner.state);
            state.paused_all = true;
        }
        let ids: Vec<String> = {
            let state = lock(&self.inner.state);
            state
                .items
                .iter()
                .filter(|item| item.state.is_active())
                .map(|item| item.id.clone())
                .collect()
        };
        for id in &ids {
            let _ = self.pause(id);
        }
        self.after_mutation();
    }

    /// Resume one item: re-queue it and let yt-dlp continue from the `.part`
    /// files (spec §7.3).
    pub fn resume(&self, id: &str) -> Outcome {
        self.requeue(id, false)
    }

    /// Manual retry: like resume, but it also clears the error and gives the
    /// automatic-retry budget back.
    pub fn retry(&self, id: &str) -> Outcome {
        self.requeue(id, true)
    }

    /// "Retry failed" (spec §6.4, §7.3).
    pub fn retry_failed(&self) -> Vec<String> {
        let ids = self.items_in_state(&[ItemState::Failed]);
        for id in &ids {
            let _ = self.retry(id);
        }
        ids
    }

    /// Resume everything that can resume: after "pause all", or after a restart
    /// where the items came back interrupted.
    pub fn resume_all(&self) -> Vec<String> {
        {
            let mut state = lock(&self.inner.state);
            state.paused_all = false;
        }
        let ids = self.items_in_state(&[ItemState::Paused, ItemState::Interrupted]);
        for id in &ids {
            let _ = self.resume(id);
        }
        self.after_mutation();
        ids
    }

    fn requeue(&self, id: &str, reset_budget: bool) -> Outcome {
        {
            let mut state = lock(&self.inner.state);
            let Some(item) = state.item_mut(id) else {
                return Outcome::NotFound;
            };
            if item.state.is_active() {
                return Outcome::Ok;
            }
            if !matches!(
                item.state,
                ItemState::Paused
                    | ItemState::Interrupted
                    | ItemState::Failed
                    | ItemState::Cancelled
            ) {
                return Outcome::Ok;
            }
            // **Queued**, not the stage it was paused in: only a queued item is
            // eligible for the scheduler to start again, and the stage is kept
            // in `paused_from` for the lane and for the UI.
            item.state = ItemState::Queued;
            item.control = ControlSignal::None;
            item.retry_at = None;
            item.error = None;
            item.finished_at = None;
            item.transcript_preview = None;
            item.updated_at = now_unix();
            if reset_budget {
                item.auto_retries = 0;
            }
            state.order_dirty = true;
        }
        self.after_mutation();
        Outcome::Ok
    }

    /// Cancel: kill the tree, delete the item's temp dir, never touch finished
    /// outputs (spec §7.3, §4.2).
    pub fn cancel(&self, id: &str) -> Outcome {
        self.signal(id, ControlSignal::Cancel)
    }

    /// Remove: cancel, then take the item out of the queue.
    pub fn remove(&self, id: &str) -> Outcome {
        self.signal(id, ControlSignal::Remove)
    }

    /// Ask a running item to stop, or apply the signal right away when nothing
    /// is running.
    fn signal(&self, id: &str, signal: ControlSignal) -> Outcome {
        let (token, temp) = {
            let mut state = lock(&self.inner.state);
            let Some(index) = state.index_of(id) else {
                return Outcome::NotFound;
            };
            let running_token = lock(&self.inner.running)
                .get(id)
                .filter(|_| state.items[index].state.is_active())
                .map(|entry| entry.token.clone());
            if let Some(token) = running_token {
                state.items[index].control = signal;
                (Some(token), None)
            } else {
                match signal {
                    ControlSignal::Remove => {
                        let temp = state.items[index].temp_dir();
                        state.items.remove(index);
                        state.order_dirty = true;
                        note_removed(&self.inner, id);
                        (None, Some(temp))
                    }
                    ControlSignal::Cancel => {
                        let temp = state.items[index].temp_dir();
                        let item = &mut state.items[index];
                        item.state = ItemState::Cancelled;
                        item.control = ControlSignal::None;
                        item.error = None;
                        item.transcript_preview = None;
                        item.progress = ItemProgress::empty();
                        item.finished_at = Some(now_unix());
                        item.updated_at = now_unix();
                        (None, Some(temp))
                    }
                    // Pausing something that is not running can only come from a
                    // stale view of the row — the engine pauses running items
                    // through the token above. It parks the item rather than
                    // pretending it was running: a queued item the user paused
                    // must come back with Resume, not with Retry.
                    ControlSignal::Pause => {
                        let item = &mut state.items[index];
                        if !item.state.is_parked() {
                            item.paused_from = Some(item.state);
                            item.state = ItemState::Paused;
                            item.control = ControlSignal::None;
                            item.transcript_preview = None;
                            item.progress.speed_bps = None;
                            item.progress.eta_sec = None;
                            item.updated_at = now_unix();
                        }
                        (None, None)
                    }
                    ControlSignal::None => (None, None),
                }
            }
        };
        // Spec §7.3: a cancelled item's temp dir goes, its finished outputs do
        // not — and it goes with the lock released, because deleting a folder
        // is not something to hold the queue still for.
        if let Some(temp) = &temp {
            job::cleanup_job_dir(temp);
        }
        if let Some(token) = token {
            token.cancel();
        }
        self.after_mutation();
        Outcome::Ok
    }

    /// "Clear finished" (spec §7.3): every terminal item goes.
    pub fn clear_finished(&self) -> Vec<String> {
        let ids = self.items_in_state(&[ItemState::Done, ItemState::Failed, ItemState::Cancelled]);
        for id in &ids {
            let _ = self.remove(id);
        }
        ids
    }

    /// Reorder (spec §7.3: drag and drop). The given ids come first, in the
    /// order given; the rest keep their relative order after them.
    pub fn reorder(&self, ids: &[String]) -> Outcome {
        {
            let mut state = lock(&self.inner.state);
            let mut moved: Vec<QueueItem> = Vec::with_capacity(state.items.len());
            for id in ids {
                if let Some(index) = state.items.iter().position(|item| &item.id == id) {
                    moved.push(state.items.remove(index));
                }
            }
            moved.append(&mut state.items);
            state.items = moved;
            state.order_dirty = true;
        }
        self.after_mutation();
        Outcome::Ok
    }

    /// Move one item to the top (spec §7.3).
    pub fn move_to_top(&self, id: &str) -> Outcome {
        {
            let mut state = lock(&self.inner.state);
            let Some(index) = state.items.iter().position(|item| item.id == id) else {
                return Outcome::NotFound;
            };
            let item = state.items.remove(index);
            state.items.insert(0, item);
            state.order_dirty = true;
        }
        self.after_mutation();
        Outcome::Ok
    }

    /// Add or replace a group (spec §7.1; M3 creates them from playlists).
    pub fn upsert_group(&self, group: QueueGroup) {
        {
            let mut state = lock(&self.inner.state);
            match state.groups.iter_mut().find(|other| other.id == group.id) {
                Some(existing) => *existing = group,
                None => state.groups.push(group),
            }
            state.groups_dirty = true;
        }
        lock(&self.inner.coalescer).mark_summary();
        self.after_mutation();
    }

    /// Pause every active or queued item in `group_id` (spec §6.4, §11.5).
    pub fn pause_group(&self, group_id: &str) -> Vec<String> {
        let ids: Vec<String> = {
            let state = lock(&self.inner.state);
            state
                .items
                .iter()
                .filter(|item| {
                    item.group_id.as_deref() == Some(group_id) && !item.state.is_parked()
                })
                .map(|item| item.id.clone())
                .collect()
        };
        for id in &ids {
            let _ = self.pause(id);
            mark_dirty(&self.inner, id, true);
        }
        ids
    }

    /// Resume every paused or interrupted item in `group_id` (spec §6.4, §11.5).
    pub fn resume_group(&self, group_id: &str) -> Vec<String> {
        let ids = self.group_items_in_state(group_id, &[ItemState::Paused, ItemState::Interrupted]);
        for id in &ids {
            let _ = self.resume(id);
            mark_dirty(&self.inner, id, true);
        }
        ids
    }

    /// Retry every failed item in `group_id` (spec §6.4, §11.5).
    pub fn retry_group_failed(&self, group_id: &str) -> Vec<String> {
        let ids = self.group_items_in_state(group_id, &[ItemState::Failed]);
        for id in &ids {
            let _ = self.retry(id);
            mark_dirty(&self.inner, id, true);
        }
        ids
    }

    /// Remove every item in `group_id` and the group record itself (spec §6.4,
    /// §11.5).
    pub fn remove_group(&self, group_id: &str) -> Vec<String> {
        let ids: Vec<String> = {
            let state = lock(&self.inner.state);
            state
                .items
                .iter()
                .filter(|item| item.group_id.as_deref() == Some(group_id))
                .map(|item| item.id.clone())
                .collect()
        };
        for id in &ids {
            let _ = self.remove(id);
        }
        {
            let mut state = lock(&self.inner.state);
            let before = state.groups.len();
            state.groups.retain(|group| group.id != group_id);
            if state.groups.len() != before {
                state.groups_dirty = true;
            }
        }
        lock(&self.inner.coalescer).mark_summary();
        self.after_mutation();
        ids
    }

    /// Update one item from outside the engine, for instance with the
    /// inspection result that lands after the item was created.
    pub fn patch_item(&self, id: &str, patch: ItemPatch) -> Outcome {
        {
            let mut state = lock(&self.inner.state);
            let Some(item) = state.item_mut(id) else {
                return Outcome::NotFound;
            };
            if let Some(title) = patch.title {
                item.title = Some(title);
            }
            if let Some(site) = patch.site {
                item.site = Some(site);
            }
            if let Some(duration) = patch.duration_sec {
                item.duration_sec = Some(duration);
            }
            if let Some(options) = patch.options {
                item.options = options;
            }
            item.updated_at = now_unix();
        }
        self.after_mutation();
        Outcome::Ok
    }

    /// Attach the tail of an item's log, for "Copy details" (spec §8.5).
    pub fn set_log_tail(&self, id: &str, tail: Vec<String>) -> Outcome {
        {
            let mut state = lock(&self.inner.state);
            let Some(item) = state.item_mut(id) else {
                return Outcome::NotFound;
            };
            item.log_tail = tail;
            item.updated_at = now_unix();
        }
        self.after_mutation();
        Outcome::Ok
    }

    /// Mark an existing item with finished media for transcription (spec §10.2, §11.5).
    #[allow(clippy::too_many_arguments)]
    pub fn transcribe_item(
        &self,
        id: &str,
        whisper_model: Option<String>,
        transcript_language: Option<String>,
        translate_to_english: Option<bool>,
        transcripts_subfolder: Option<bool>,
        transcription_engine: Option<String>,
        gemini_model: Option<String>,
        summarize: Option<bool>,
        summary_language: Option<String>,
    ) -> Outcome {
        let ok = {
            let mut state = lock(&self.inner.state);
            let Some(item) = state.item_mut(id) else {
                return Outcome::NotFound;
            };
            if item.file.as_ref().map(|f| f.is_file()) != Some(true) {
                return Outcome::NotFound;
            }
            item.options.transcribe = true;
            if whisper_model.is_some() {
                item.options.whisper_model = whisper_model;
            }
            if transcript_language.is_some() {
                item.options.transcript_language = transcript_language;
            }
            if translate_to_english.is_some() {
                item.options.translate_to_english = translate_to_english;
            }
            if transcripts_subfolder.is_some() {
                item.options.transcripts_subfolder = transcripts_subfolder;
            }
            if transcription_engine.is_some() {
                item.options.transcription_engine = transcription_engine;
            }
            if gemini_model.is_some() {
                item.options.gemini_model = gemini_model;
            }
            if summarize.is_some() {
                item.options.summarize = summarize;
            }
            if summary_language.is_some() {
                item.options.summary_language = summary_language;
            }
            item.state = ItemState::Queued;
            item.progress = ItemProgress::with_label("waitingTranscribe");
            item.error = None;
            item.updated_at = now_unix();
            true
        };
        if ok {
            mark_dirty(&self.inner, id, true);
            self.after_mutation();
        }
        Outcome::Ok
    }

    fn items_in_state(&self, states: &[ItemState]) -> Vec<String> {
        let state = lock(&self.inner.state);
        state
            .items
            .iter()
            .filter(|item| states.contains(&item.state))
            .map(|item| item.id.clone())
            .collect()
    }

    fn group_items_in_state(&self, group_id: &str, states: &[ItemState]) -> Vec<String> {
        let state = lock(&self.inner.state);
        state
            .items
            .iter()
            .filter(|item| {
                item.group_id.as_deref() == Some(group_id) && states.contains(&item.state)
            })
            .map(|item| item.id.clone())
            .collect()
    }

    // -----------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------

    /// Persist, tell the flusher, wake the scheduler.
    ///
    /// Every action that changes the queue goes through here, which is why a
    /// write happens exactly on the state changes spec §7.4 lists (add, remove,
    /// finish) and **not** on progress: progress never reaches the file, so the
    /// "≤ 1 write/s while running" bound is structural rather than timed.
    fn after_mutation(&self) {
        self.persist();
        self.inner.dirty.notify_waiters();
        self.inner.wake.notify_waiters();
    }

    fn persist(&self) {
        self_persist(&self.inner);
    }
}

/// An item's temp dir, given a snapshot of the items. `None` when the id is not
/// there (it was just removed, so there is nothing to clean).
fn item_temp_for(items: &[QueueItem], id: &str) -> Option<PathBuf> {
    items
        .iter()
        .find(|item| item.id == id)
        .map(QueueItem::temp_dir)
}

/// Write `queue.json` from the current state. Best effort: a queue that cannot
/// be written is still a working queue, and the failure goes to stderr for the
/// next bug report rather than into the middle of a download.
fn self_persist(inner: &Arc<Inner>) {
    let file = {
        let state = lock(&inner.state);
        QueueFile {
            schema_version: store::QUEUE_SCHEMA_VERSION,
            items: state.items.clone(),
            groups: state.groups.clone(),
            paused_all: state.paused_all,
        }
    };
    if let Err(err) = file.save(&inner.path) {
        eprintln!("scaricatore-di-porto: cannot write the queue file: {err}");
    }
}

fn push_toast(inner: &Arc<Inner>, toast: QueueToast) {
    lock(&inner.pending_toasts).push(toast);
    lock(&inner.coalescer).mark_summary();
    inner.dirty.notify_waiters();
}

fn transcription_error_outcome(err: crate::error::AppError) -> JobOutcome {
    let err_msg = err.to_string();
    match err {
        crate::error::AppError::WhisperMissing => {
            JobOutcome::Failed(YtdlpError::new(YtdlpErrorCode::WhisperMissing, err_msg))
        }
        crate::error::AppError::WhisperModelMissing(model) => {
            JobOutcome::Failed(YtdlpError::new(YtdlpErrorCode::WhisperModelMissing, model))
        }
        crate::error::AppError::GeminiKeyMissing => {
            JobOutcome::Failed(YtdlpError::new(YtdlpErrorCode::GeminiKeyMissing, err_msg))
        }
        crate::error::AppError::GeminiKeyInvalid(detail) => {
            JobOutcome::Failed(YtdlpError::new(YtdlpErrorCode::GeminiKeyInvalid, detail))
        }
        crate::error::AppError::GeminiQuotaExceeded(detail) => {
            JobOutcome::Failed(YtdlpError::new(YtdlpErrorCode::GeminiQuotaExceeded, detail))
        }
        crate::error::AppError::GeminiModelNotFound(model) => {
            JobOutcome::Failed(YtdlpError::new(YtdlpErrorCode::GeminiModelNotFound, model))
        }
        crate::error::AppError::GeminiBlocked(reason) => {
            JobOutcome::Failed(YtdlpError::new(YtdlpErrorCode::GeminiBlocked, reason))
        }
        crate::error::AppError::NoSubtitles => {
            JobOutcome::Failed(YtdlpError::new(YtdlpErrorCode::NoSubtitles, err_msg))
        }
        _ => JobOutcome::Failed(YtdlpError::new(
            YtdlpErrorCode::Unknown,
            format!("Transcription failed: {err_msg}"),
        )),
    }
}

fn note_removed(inner: &Arc<Inner>, id: &str) {
    {
        let mut pending = lock(&inner.pending_removed);
        if !pending.iter().any(|other| other == id) {
            pending.push(id.to_owned());
        }
    }
    lock(&inner.coalescer).forget(id);
    inner.dirty.notify_waiters();
}

/// What the caller wants to add (spec §2.3, with the inspection result the Add
/// bar already has).
#[derive(Debug, Clone, PartialEq)]
pub struct AddRequest {
    pub url: String,
    pub title: Option<String>,
    pub site: Option<String>,
    pub duration_sec: Option<u64>,
    pub options: ItemOptions,
    pub group_id: Option<String>,
    pub file: Option<PathBuf>,
}

impl AddRequest {
    fn into_item(self, id: String, now: u64) -> QueueItem {
        QueueItem {
            id,
            url: self.url,
            title: self.title,
            site: self.site,
            duration_sec: self.duration_sec,
            group_id: self.group_id,
            file: self.file,
            state: ItemState::Queued,
            options: self.options,
            added_at: now,
            updated_at: now,
            ..QueueItem::default()
        }
    }
}

/// Fields the engine lets a caller change on an item it just created.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemPatch {
    pub title: Option<String>,
    pub site: Option<String>,
    pub duration_sec: Option<u64>,
    pub options: Option<ItemOptions>,
}

/// Unix seconds, from the same clock the rest of the app uses.
fn now_unix() -> u64 {
    crate::services::jsruntime::now_unix()
}

fn next_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    format!("q{millis}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ---------------------------------------------------------------------------
// The tasks
// ---------------------------------------------------------------------------

/// Plan, start, and work out when to wake up next.
async fn scheduler_loop(inner: Arc<Inner>) {
    loop {
        if inner.stopped.load(Ordering::SeqCst) {
            return;
        }
        // Registered *before* the state is read, so a change made while we are
        // planning cannot be missed (the classic lost-wakeup).
        let wake = inner.wake.notified();
        tokio::pin!(wake);
        wake.as_mut().enable();

        let next = tick(&inner).await;
        match next {
            None => wake.await,
            Some(delay) => {
                tokio::select! {
                    () = &mut wake => {}
                    () = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
}

/// One scheduling pass. Returns how long to sleep when nothing can start now.
async fn tick(inner: &Arc<Inner>) -> Option<Duration> {
    let now = now_unix();
    let lanes = *lock(&inner.lanes);
    let (starts, wake_at, paused_all) = {
        let state = lock(&inner.state);
        let candidates = candidates_of(&state.items);
        let mut free = BTreeMap::new();
        for lane in Lane::ALL {
            let busy = candidates
                .iter()
                .filter(|candidate| candidate.state.lane() == Some(lane))
                .count();
            free.insert(lane, lanes.capacity(lane).saturating_sub(busy));
        }
        let mut capacity = Capacity::new(
            free,
            scheduler::running_by_host(&candidates),
            lanes.per_host_cap,
        );
        let wake_at = scheduler::next_wakeup(&candidates, now);
        let paused_all = state.paused_all;
        let starts = if paused_all {
            Vec::new()
        } else {
            scheduler::plan(&candidates, &mut capacity, now)
                .into_iter()
                .map(|(id, _lane)| id)
                .collect::<Vec<String>>()
        };
        (starts, wake_at, paused_all)
    };

    // The lock is released before anything starts: `start_item` takes it again.
    for id in starts {
        start_item(inner, id);
    }
    if paused_all {
        return wake_at.map(|at| until(at, now));
    }
    wake_at.map(|at| until(at, now))
}

fn until(at: u64, now: u64) -> Duration {
    Duration::from_secs(at.saturating_sub(now)).max(Duration::from_millis(50))
}

/// The lane a queued item will enter when [`start_item`] runs.
fn initial_lane_of(item: &QueueItem) -> Lane {
    let is_existing_media = item.file.as_ref().map(|f| f.is_file()).unwrap_or(false);
    let already_encoded =
        is_existing_media && item.options.needs_encode() && item.size_bytes.is_some();
    if is_existing_media
        && item.options.transcribe
        && (!item.options.needs_encode() || already_encoded)
    {
        Lane::Transcribe
    } else if is_existing_media && item.options.needs_encode() && !already_encoded {
        Lane::Encode
    } else {
        Lane::Download
    }
}

/// The snapshot the planner works on.
fn candidates_of(items: &[QueueItem]) -> Vec<Candidate<'_>> {
    items
        .iter()
        .map(|item| Candidate {
            id: item.id.as_str(),
            url: item.url.as_str(),
            state: item.state,
            initial_lane: initial_lane_of(item),
            retry_at: item.retry_at,
        })
        .collect()
}

/// Mark one item running and spawn its task.
fn start_item(inner: &Arc<Inner>, id: String) {
    let now = now_unix();
    let token = CancellationToken::new();
    {
        // Spec §4.2: "On app exit cancel all running items". An item the
        // scheduler planned a moment before the exit must not start **after**
        // it either: spawning yt-dlp while the app is tearing down is a
        // process the exit hook cannot see, and it rewrites the queue file
        // underneath whatever the next launch reads. Checked before the state
        // lock, so nothing is left holding it if we bail.
        if inner.stopped.load(Ordering::SeqCst) {
            return;
        }
        let mut state = lock(&inner.state);
        let Some(item) = state.item_mut(&id) else {
            return;
        };
        // The scheduler is the only writer of this transition, but the state
        // can have moved between the plan and now (a cancel, a retry, the
        // shutdown above): a second look is cheaper than starting an item
        // twice.
        if !matches!(item.state, ItemState::Queued | ItemState::Interrupted) {
            return;
        }
        match initial_lane_of(item) {
            Lane::Transcribe => {
                item.state = ItemState::Transcribing;
                item.progress = ItemProgress::with_label("waitingTranscribe");
            }
            Lane::Encode => {
                item.state = ItemState::Encoding;
                item.progress = ItemProgress::with_label("waitingEncode");
            }
            Lane::Download => {
                item.state = ItemState::Downloading;
                item.progress = ItemProgress::with_label("preparing");
            }
        }
        item.control = ControlSignal::None;
        item.error = None;
        item.retry_at = None;
        item.started_at = Some(now);
        item.updated_at = now;
        item.finished_at = None;
        lock(&inner.running).insert(
            id.clone(),
            Running {
                token: token.clone(),
            },
        );
    }
    // **The file is written before the tool is spawned**, and this is the whole
    // point: a `queue.json` that still says `Queued` while yt-dlp runs comes
    // back from a crash as "not started yet", and spec §7.4's recovery only
    // knows how to park items it can see were in an active stage. The queue
    // would then quietly download the item again by itself, with no Resume and
    // no question asked (D91).
    //
    // The one thing this can get wrong is dying between here and the spawn: the
    // file says `Downloading` while nothing ran, so the item comes back
    // `Interrupted` and waits for a Resume it did not need. That is the cheap
    // direction to be wrong in.
    self_persist(inner);
    mark_dirty(inner, &id, true);
    tokio::spawn(run_item(Arc::clone(inner), id, token));
}

/// One item's whole pipeline: resolve the tools, download via yt-dlp, and
/// if requested, encode/trim via ffmpeg and auto-play via opener.
async fn run_item(inner: Arc<Inner>, id: String, token: CancellationToken) {
    let Some((environment, request)) = prepare(&inner, &id).await else {
        return;
    };

    let sink_id = id.clone();
    let progress_inner = Arc::clone(&inner);
    let sink: job::ProgressSink = Arc::new(move |progress: &JobProgress| {
        on_progress(&progress_inner, &sink_id, progress);
    });

    // Spec §7.3/§9.1: a download already finished by an earlier attempt that
    // stopped in Encoding (pause, crash, ffmpeg failure) goes straight to the
    // encode stage. Re-running yt-dlp would re-download it, or with the
    // archive on, report it as skipped and never encode it.
    let downloaded = {
        let state = lock(&inner.state);
        state.item(&id).and_then(|item| {
            let file = item.file.as_ref()?;
            let is_existing_file = file.is_file()
                && ((item.options.needs_encode() && item.size_bytes.is_none())
                    || item.options.transcribe
                    || item.url.starts_with("local://"));
            is_existing_file.then(|| JobOutcome::Done {
                file: file.clone(),
                runtime: if item.runtime.as_deref() == Some("deno") {
                    crate::services::jsruntime::RuntimeKind::Deno
                } else {
                    crate::services::jsruntime::RuntimeKind::QuickJs
                },
                elapsed_ms: 0,
                requested_height: item.requested_height,
                actual_height: item.actual_height,
            })
        })
    };
    let outcome = match downloaded {
        Some(outcome) => outcome,
        None => job::run_job(&environment, &request, token.clone(), sink).await,
    };

    match outcome {
        JobOutcome::Done {
            file,
            runtime,
            elapsed_ms,
            requested_height,
            actual_height,
        } => {
            let (options, mut duration_sec) = {
                let state = lock(&inner.state);
                let Some(item) = state.item(&id) else {
                    return;
                };
                (item.options.clone(), item.duration_sec)
            };

            let already_encoded = {
                let state = lock(&inner.state);
                state
                    .item(&id)
                    .is_some_and(|item| item.options.needs_encode() && item.size_bytes.is_some())
            };

            if options.needs_encode() && !already_encoded {
                let Some(ffmpeg_path) = environment.ffmpeg.as_ref() else {
                    finish_item(
                        &inner,
                        &id,
                        JobOutcome::Failed(YtdlpError::new(
                            YtdlpErrorCode::FfmpegMissing,
                            "ffmpeg is required for encoding/trimming but was not found".to_owned(),
                        )),
                    );
                    return;
                };

                // Move from Downloading to Encoding right away: that frees the
                // download lane (spec §7.1). Until an encode slot is free the
                // item says "waiting", not "Encoding 0%" (spec §7.2). The file
                // is remembered so a pause/crash resumes from it (spec §7.3).
                let cancelled = {
                    let mut state = lock(&inner.state);
                    let Some(item) = state.item_mut(&id) else {
                        return;
                    };
                    item.file = Some(file.clone());
                    item.size_bytes = None;
                    item.runtime = Some(runtime.as_str().to_owned());
                    item.requested_height = requested_height;
                    item.actual_height = actual_height;
                    if item.control == ControlSignal::None {
                        item.state = ItemState::Encoding;
                        item.progress.label = "waitingEncode".to_owned();
                        item.progress.percent = None;
                        item.progress.speed_bps = None;
                        item.progress.eta_sec = None;
                        item.updated_at = now_unix();
                        false
                    } else {
                        true
                    }
                };
                if cancelled {
                    finish_item(&inner, &id, JobOutcome::Cancelled);
                    return;
                }
                self_persist(&inner);
                mark_dirty(&inner, &id, true);
                // Wake scheduler: the download lane has been freed!
                inner.wake.notify_waiters();

                let Some(_slot) = acquire_encode_slot(&inner, &token).await else {
                    finish_item(&inner, &id, JobOutcome::Cancelled);
                    return;
                };

                {
                    let mut state = lock(&inner.state);
                    if let Some(item) = state.item_mut(&id) {
                        item.progress.label = "encoding".to_owned();
                        item.progress.percent = Some(0.0);
                        item.updated_at = now_unix();
                    }
                }
                mark_dirty(&inner, &id, true);

                // Re-check control signal after waiting for encode slot
                let stopped = lock(&inner.state)
                    .item(&id)
                    .is_some_and(|item| item.control != ControlSignal::None);
                if stopped {
                    finish_item(&inner, &id, JobOutcome::Cancelled);
                    return;
                }

                let mut total_duration = duration_sec.map(|s| s as f64);
                if total_duration.is_none() {
                    if let Some(ffprobe) = crate::services::ffmpeg::find_ffprobe(
                        ffmpeg_path,
                        environment.ffprobe.as_deref(),
                    ) {
                        if let Some(dur) = crate::services::ffmpeg::probe_file_duration(
                            &ffprobe,
                            &file,
                            environment.private_temp.as_deref(),
                            token.clone(),
                        )
                        .await
                        {
                            total_duration = Some(dur);
                            duration_sec = Some(dur.round() as u64);
                            let mut state = lock(&inner.state);
                            if let Some(item) = state.item_mut(&id) {
                                item.duration_sec = duration_sec;
                            }
                        }
                    }
                }

                let (trim_start_sec, trim_end_sec) =
                    match crate::services::ffmpeg::parse_and_validate_trim(
                        options.trim_start.as_deref(),
                        options.trim_end.as_deref(),
                        total_duration,
                    ) {
                        Ok(range) => range,
                        Err(err) => {
                            // Spec §9.2/§12.11: never encode a bogus range; keep the original.
                            finish_item(
                                &inner,
                                &id,
                                JobOutcome::Failed(YtdlpError::new(
                                    YtdlpErrorCode::Unknown,
                                    format!("Encode failed: invalid trim: {err}"),
                                )),
                            );
                            return;
                        }
                    };
                let trim = if trim_start_sec.is_some() || trim_end_sec.is_some() {
                    Some(crate::services::ffmpeg::TrimSpec::new(
                        trim_start_sec,
                        trim_end_sec,
                    ))
                } else {
                    None
                };

                let kind = if options.kind == Kind::Audio {
                    let audio_codec = options
                        .audio_codec
                        .as_deref()
                        .and_then(crate::services::encoders::AudioCodec::parse)
                        .unwrap_or(crate::services::encoders::AudioCodec::Aac);
                    let output = crate::services::ffmpeg::derive_encoded_output_path(
                        &file,
                        audio_codec.extension(),
                    );
                    crate::services::ffmpeg::EncodeJobKind::Audio(
                        crate::services::ffmpeg::AudioEncodeOptions {
                            input: file.clone(),
                            output,
                            audio_codec,
                            trim,
                            embed_thumbnail: options.embed_thumbnail,
                        },
                    )
                } else {
                    let video_encoder = options
                        .video_codec
                        .clone()
                        .unwrap_or_else(|| "libx264".to_string());
                    let audio_codec = options
                        .audio_codec
                        .as_deref()
                        .and_then(crate::services::encoders::AudioCodec::parse);
                    let output = crate::services::ffmpeg::derive_encoded_output_path(&file, "mp4");
                    crate::services::ffmpeg::EncodeJobKind::Video(
                        crate::services::ffmpeg::VideoEncodeOptions {
                            input: file.clone(),
                            output,
                            video_encoder,
                            audio_codec,
                            trim,
                            embed_thumbnail: options.embed_thumbnail,
                        },
                    )
                };

                let encode_req = crate::services::ffmpeg::EncodeRequest {
                    ffmpeg_path: ffmpeg_path.clone(),
                    kind,
                    keep_original: options.keep_original,
                    total_duration,
                    temp_dir: environment.private_temp.clone(),
                };

                let encode_sink_id = id.clone();
                let encode_inner = Arc::clone(&inner);
                let encode_sink: crate::services::ffmpeg::EncodeProgressSink =
                    Arc::new(move |prog: &crate::services::ffmpeg::FfmpegProgress| {
                        on_encode_progress(&encode_inner, &encode_sink_id, prog);
                    });

                let encode_outcome =
                    crate::services::ffmpeg::run_encode(encode_req, token.clone(), encode_sink)
                        .await;
                drop(_slot);
                inner.wake.notify_waiters();

                match encode_outcome {
                    crate::services::ffmpeg::EncodeOutcome::Success {
                        output_file,
                        used_cpu_fallback,
                    } => {
                        if used_cpu_fallback {
                            // Spec §9.3: tell the user when hardware encoder failed and retried with CPU encoder
                            {
                                let mut state = lock(&inner.state);
                                if let Some(item) = state.item_mut(&id) {
                                    item.push_log(
                                        "Hardware encoder failed; fell back to CPU encoder.",
                                    );
                                }
                            }
                            push_toast(&inner, QueueToast::new("toast.hardwareEncoderFallback"));
                        }
                        let media_file = output_file;
                        let size_bytes = std::fs::metadata(&media_file).ok().map(|m| m.len());
                        {
                            let mut state = lock(&inner.state);
                            if let Some(item) = state.item_mut(&id) {
                                item.file = Some(media_file.clone());
                                item.size_bytes = size_bytes;
                                item.updated_at = now_unix();
                            }
                        }
                        self_persist(&inner);
                        mark_dirty(&inner, &id, false);

                        if options.transcribe {
                            match run_transcribe_stage(
                                &inner,
                                &id,
                                media_file.clone(),
                                duration_sec,
                                &environment,
                                &options,
                                &token,
                            )
                            .await
                            {
                                Ok(res) => {
                                    {
                                        let mut state = lock(&inner.state);
                                        if let Some(item) = state.item_mut(&id) {
                                            item.transcript_files = res.transcript_files;
                                            item.transcript_origin = Some(res.origin);
                                            item.transcript_preview = None;
                                            if res.used_cpu_fallback {
                                                item.push_log("Whisper GPU execution failed; fell back to CPU (-ng).");
                                            }
                                            if res.hallucination_warning.is_some() {
                                                item.push_log("Warning: possible hallucination detected (repeated loops or long silence). Consider enabling VAD, using a larger model, or setting the language explicitly.");
                                            }
                                        }
                                    }
                                    if res.used_cpu_fallback {
                                        push_toast(
                                            &inner,
                                            QueueToast::new("toast.whisperCpuFallback"),
                                        );
                                    }
                                    if res.hallucination_warning.is_some() {
                                        push_toast(
                                            &inner,
                                            QueueToast::new("toast.hallucinationWarning"),
                                        );
                                    }
                                }
                                Err(err) => {
                                    if token.is_cancelled() {
                                        finish_item(&inner, &id, JobOutcome::Cancelled);
                                        return;
                                    }
                                    let err_msg = err.to_string();
                                    {
                                        let mut state = lock(&inner.state);
                                        if let Some(item) = state.item_mut(&id) {
                                            item.push_log(&err_msg);
                                        }
                                    }
                                    finish_item(&inner, &id, transcription_error_outcome(err));
                                    return;
                                }
                            }
                        }
                        if options.auto_play {
                            let _ = tauri_plugin_opener::open_path(
                                media_file.to_string_lossy().as_ref(),
                                None::<&str>,
                            );
                        }
                        finish_item(
                            &inner,
                            &id,
                            JobOutcome::Done {
                                file: media_file,
                                runtime,
                                elapsed_ms,
                                requested_height,
                                actual_height,
                            },
                        );
                    }
                    crate::services::ffmpeg::EncodeOutcome::Cancelled => {
                        finish_item(&inner, &id, JobOutcome::Cancelled);
                    }
                    crate::services::ffmpeg::EncodeOutcome::Failed { error, stderr_tail } => {
                        // Spec §9.2 and §12: keep the original file on failure!
                        {
                            let mut state = lock(&inner.state);
                            if let Some(item) = state.item_mut(&id) {
                                for line in &stderr_tail {
                                    item.push_log(line);
                                }
                            }
                        }
                        finish_item(
                            &inner,
                            &id,
                            JobOutcome::Failed(YtdlpError::new(
                                YtdlpErrorCode::Unknown,
                                format!("Encode failed: {error}\n{}", stderr_tail.join("\n")),
                            )),
                        );
                    }
                }
            } else {
                let media_file = file;
                if options.transcribe {
                    match run_transcribe_stage(
                        &inner,
                        &id,
                        media_file.clone(),
                        duration_sec,
                        &environment,
                        &options,
                        &token,
                    )
                    .await
                    {
                        Ok(res) => {
                            {
                                let mut state = lock(&inner.state);
                                if let Some(item) = state.item_mut(&id) {
                                    item.transcript_files = res.transcript_files;
                                    item.transcript_origin = Some(res.origin);
                                    item.transcript_preview = None;
                                    if res.used_cpu_fallback {
                                        item.push_log(
                                            "Whisper GPU execution failed; fell back to CPU (-ng).",
                                        );
                                    }
                                    if res.hallucination_warning.is_some() {
                                        item.push_log("Warning: possible hallucination detected (repeated loops or long silence). Consider enabling VAD, using a larger model, or setting the language explicitly.");
                                    }
                                }
                            }
                            if res.used_cpu_fallback {
                                push_toast(&inner, QueueToast::new("toast.whisperCpuFallback"));
                            }
                            if res.hallucination_warning.is_some() {
                                push_toast(&inner, QueueToast::new("toast.hallucinationWarning"));
                            }
                        }
                        Err(err) => {
                            if token.is_cancelled() {
                                finish_item(&inner, &id, JobOutcome::Cancelled);
                                return;
                            }
                            let err_msg = err.to_string();
                            {
                                let mut state = lock(&inner.state);
                                if let Some(item) = state.item_mut(&id) {
                                    item.push_log(&err_msg);
                                }
                            }
                            finish_item(&inner, &id, transcription_error_outcome(err));
                            return;
                        }
                    }
                }
                if options.auto_play {
                    let _ = tauri_plugin_opener::open_path(
                        media_file.to_string_lossy().as_ref(),
                        None::<&str>,
                    );
                }
                finish_item(
                    &inner,
                    &id,
                    JobOutcome::Done {
                        file: media_file,
                        runtime,
                        elapsed_ms,
                        requested_height,
                        actual_height,
                    },
                );
            }
        }
        other => {
            finish_item(&inner, &id, other);
        }
    }
}

/// A held encode slot; dropping it frees the slot and wakes the waiters.
struct EncodeSlot(Arc<Inner>);

impl Drop for EncodeSlot {
    fn drop(&mut self) {
        {
            let mut running = lock(&self.0.encodes_running);
            *running = running.saturating_sub(1);
        }
        self.0.encode_slot.notify_waiters();
    }
}

/// Wait for a free encode slot (spec §7.1, §7.2), checked against the lane
/// capacity in force *now*. `None` when cancelled first.
async fn acquire_encode_slot(inner: &Arc<Inner>, token: &CancellationToken) -> Option<EncodeSlot> {
    loop {
        // Register for the wake-up before checking, so none is missed.
        let notified = inner.encode_slot.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        {
            let capacity = lock(&inner.lanes).capacity(Lane::Encode);
            let mut running = lock(&inner.encodes_running);
            if *running < capacity {
                *running += 1;
                return Some(EncodeSlot(Arc::clone(inner)));
            }
        }
        tokio::select! {
            _ = token.cancelled() => return None,
            _ = &mut notified => {}
        }
    }
}

/// Fold one ffmpeg encode progress payload into the item (spec §3, §7.6).
fn on_encode_progress(
    inner: &Arc<Inner>,
    id: &str,
    progress: &crate::services::ffmpeg::FfmpegProgress,
) {
    let milestone = {
        let mut state = lock(&inner.state);
        let Some(item) = state.item_mut(id) else {
            return;
        };
        if item.control != ControlSignal::None {
            return;
        }
        item.state = ItemState::Encoding;
        item.progress.label = "encoding".to_owned();
        item.progress.percent = progress.percent;
        item.progress.speed_bps = None;
        item.progress.eta_sec = progress.eta_sec;
        item.updated_at = now_unix();
        progress.is_end
    };
    mark_dirty(inner, id, milestone);
}

/// A held transcribe slot; dropping it frees the slot and wakes the waiters.
struct TranscribeSlot(Arc<Inner>);

impl Drop for TranscribeSlot {
    fn drop(&mut self) {
        {
            let mut running = lock(&self.0.transcribes_running);
            *running = running.saturating_sub(1);
        }
        self.0.transcribe_slot.notify_waiters();
    }
}

/// Wait for a free transcribe slot (spec §7.1, §7.2, §10.2), checked against the lane
/// capacity in force *now*. `None` when cancelled first.
async fn acquire_transcribe_slot(
    inner: &Arc<Inner>,
    token: &CancellationToken,
) -> Option<TranscribeSlot> {
    loop {
        let notified = inner.transcribe_slot.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        {
            let capacity = lock(&inner.lanes).capacity(Lane::Transcribe);
            let mut running = lock(&inner.transcribes_running);
            if *running < capacity {
                *running += 1;
                return Some(TranscribeSlot(Arc::clone(inner)));
            }
        }
        tokio::select! {
            _ = token.cancelled() => return None,
            _ = &mut notified => {}
        }
    }
}

/// Fold one Whisper progress payload into the item (spec §3, §7.6, §10.2).
fn on_transcribe_progress(inner: &Arc<Inner>, id: &str, percent: f64, segment: Option<&str>) {
    let milestone = {
        let mut state = lock(&inner.state);
        let Some(item) = state.item_mut(id) else {
            return;
        };
        if item.control != ControlSignal::None {
            return;
        }
        item.state = ItemState::Transcribing;
        item.progress.label = "transcribing".to_string();
        item.progress.percent = Some(percent);
        item.progress.speed_bps = None;
        item.progress.eta_sec = None;
        if let Some(text) = segment {
            item.transcript_preview = Some(text.to_string());
        }
        item.updated_at = now_unix();
        percent >= 100.0
    };
    mark_dirty(inner, id, milestone);
}

/// Result of the transcription and summary stage for an item.
#[derive(Debug, Clone)]
pub struct TranscribeStageOutcome {
    pub transcript_files: Vec<PathBuf>,
    pub origin: String,
    pub used_cpu_fallback: bool,
    pub hallucination_warning: Option<crate::services::transcribe::whisper::HallucinationWarning>,
}

/// Run transcription stage for an item according to configured engine ordering (spec §10.1, §10.2, §10.3, §10.4).
async fn run_transcribe_stage(
    inner: &Arc<Inner>,
    id: &str,
    media_file: PathBuf,
    duration_sec: Option<u64>,
    environment: &JobEnvironment,
    options: &ItemOptions,
    token: &CancellationToken,
) -> Result<TranscribeStageOutcome, crate::error::AppError> {
    // 1. Move to Transcribing state
    let cancelled = {
        let mut state = lock(&inner.state);
        let Some(item) = state.item_mut(id) else {
            return Err(crate::error::AppError::Process("Item removed".to_string()));
        };
        if item.control == ControlSignal::None {
            item.state = ItemState::Transcribing;
            item.progress.label = "waitingTranscribe".to_string();
            item.progress.percent = None;
            item.progress.speed_bps = None;
            item.progress.eta_sec = None;
            item.updated_at = now_unix();
            false
        } else {
            true
        }
    };
    if cancelled {
        return Err(crate::error::AppError::Process("Cancelled".to_string()));
    }
    self_persist(inner);
    mark_dirty(inner, id, true);
    // Wake scheduler: previous lane is now free
    inner.wake.notify_waiters();

    // 2. Acquire transcribe slot
    let Some(_slot) = acquire_transcribe_slot(inner, token).await else {
        return Err(crate::error::AppError::Process("Cancelled".to_string()));
    };

    // 3. Update status once slot acquired
    {
        let mut state = lock(&inner.state);
        if let Some(item) = state.item_mut(id) {
            item.progress.label = "transcribing".to_string();
            item.progress.percent = Some(0.0);
            item.updated_at = now_unix();
        }
    }
    mark_dirty(inner, id, true);

    // Re-check stop signal
    let stopped = lock(&inner.state)
        .item(id)
        .is_some_and(|item| item.control != ControlSignal::None);
    if stopped || token.is_cancelled() {
        return Err(crate::error::AppError::Process("Cancelled".to_string()));
    }

    let engine_order = options
        .transcription_engine
        .as_deref()
        .or(environment.default_transcription_engine_order.as_deref())
        .unwrap_or("subtitles_whisper_gemini");

    let url = lock(&inner.state)
        .item(id)
        .map(|i| i.url.clone())
        .unwrap_or_default();
    let base_filename = media_file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "media".to_string());
    let temp_dir = process::item_temp_dir(&options.output_dir, id);
    let transcripts_subfolder = options.transcripts_subfolder.unwrap_or(false);

    let mut outcome: Option<TranscribeStageOutcome> = None;
    let mut last_error: Option<crate::error::AppError> = None;

    // Subtitles attempt
    let try_subtitles = || async {
        if url.starts_with("local://") {
            return Ok(None);
        }
        let sub_req = crate::services::transcribe::subtitles::SubtitleFetchRequest {
            ytdlp_path: environment.ytdlp.clone(),
            url: url.clone(),
            language: options
                .transcript_language
                .clone()
                .or_else(|| environment.default_transcript_language.clone()),
            temp_dir: temp_dir.clone(),
            output_dir: options.output_dir.clone(),
            base_filename: base_filename.clone(),
            transcripts_subfolder,
        };
        crate::services::transcribe::subtitles::fetch_subtitles(sub_req, token.clone()).await
    };

    // Whisper attempt
    let try_whisper = || async {
        let Some(whisper_path) = environment.whisper.as_ref() else {
            return Err(crate::error::AppError::WhisperMissing);
        };
        let Some(ffmpeg_path) = environment.ffmpeg.as_ref() else {
            return Err(crate::error::AppError::Process(
                "ffmpeg is required for transcription preparation but was not found".to_string(),
            ));
        };

        let catalogue = crate::services::models::ModelCatalogue::parse(
            crate::services::models::MODELS_CATALOGUE_JSON,
        )?;
        let model_id = options
            .whisper_model
            .as_deref()
            .or(environment.default_whisper_model.as_deref())
            .unwrap_or("large-v3-turbo-q5_0");

        let model_path = if let Some(models_dir) = environment.models_dir.as_ref() {
            if let Some(entry) = catalogue
                .find(model_id)
                .or_else(|| catalogue.find_by_file(model_id))
            {
                let p = models_dir.join(&entry.file);
                if p.is_file() { Some(p) } else { None }
            } else {
                let p = models_dir.join(model_id);
                if p.is_file() {
                    Some(p)
                } else if std::path::Path::new(model_id).is_file() {
                    Some(std::path::PathBuf::from(model_id))
                } else {
                    None
                }
            }
        } else if std::path::Path::new(model_id).is_file() {
            Some(std::path::PathBuf::from(model_id))
        } else {
            None
        };

        let Some(model_path) = model_path else {
            return Err(crate::error::AppError::WhisperModelMissing(
                model_id.to_string(),
            ));
        };

        let vad_model_path = if environment.skip_silence_vad {
            environment
                .models_dir
                .as_ref()
                .map(|d| d.join(&catalogue.vad.file))
                .filter(|p| p.is_file())
        } else {
            None
        };

        let language = options
            .transcript_language
            .clone()
            .or_else(|| environment.default_transcript_language.clone());
        let translate_to_english = options.translate_to_english.unwrap_or(false);
        let mut duration_sec_f64 = duration_sec.map(|s| s as f64);
        if duration_sec_f64.is_none() {
            if let Some(ffprobe) =
                crate::services::ffmpeg::find_ffprobe(ffmpeg_path, environment.ffprobe.as_deref())
            {
                if let Some(dur) = crate::services::ffmpeg::probe_file_duration(
                    &ffprobe,
                    &media_file,
                    environment.private_temp.as_deref(),
                    token.clone(),
                )
                .await
                {
                    duration_sec_f64 = Some(dur);
                    let dur_u64 = dur.round() as u64;
                    let mut state = lock(&inner.state);
                    if let Some(item) = state.item_mut(id) {
                        item.duration_sec = Some(dur_u64);
                    }
                }
            }
        }

        let req = crate::services::transcribe::whisper::TranscribeRequest {
            ffmpeg_path: ffmpeg_path.clone(),
            whisper_path: whisper_path.clone(),
            model_path,
            vad_model_path,
            input_media: media_file.clone(),
            language,
            translate_to_english,
            skip_silence_vad: environment.skip_silence_vad,
            whisper_threads: environment.whisper_threads,
            transcripts_subfolder,
            duration_sec: duration_sec_f64,
            temp_dir: Some(temp_dir.clone()),
        };

        let progress_inner = Arc::clone(inner);
        let progress_id = id.to_string();
        let sink: crate::services::transcribe::whisper::TranscribeProgressSink =
            Arc::new(move |percent: f64, segment: Option<&str>| {
                on_transcribe_progress(&progress_inner, &progress_id, percent, segment);
            });

        crate::services::transcribe::whisper::run_transcribe(req, token.clone(), Some(sink)).await
    };

    // Gemini attempt
    let try_gemini = || async {
        let Some(ffmpeg_path) = environment.ffmpeg.as_ref() else {
            return Err(crate::error::AppError::Process(
                "ffmpeg is required for Gemini audio preparation but was not found".to_string(),
            ));
        };
        let api_key = environment
            .config_dir
            .as_ref()
            .and_then(|cd| crate::services::secrets::get_gemini_api_key(cd));
        let Some(api_key) = api_key else {
            return Err(crate::error::AppError::GeminiKeyMissing);
        };

        let model = options
            .gemini_model
            .clone()
            .or_else(|| environment.default_gemini_model.clone())
            .unwrap_or_else(|| {
                crate::services::transcribe::gemini::DEFAULT_GEMINI_MODEL.to_string()
            });

        let req = crate::services::transcribe::gemini::GeminiTranscribeRequest {
            ffmpeg_path: ffmpeg_path.clone(),
            input_media: media_file.clone(),
            temp_dir: temp_dir.clone(),
            output_dir: options.output_dir.clone(),
            base_filename: base_filename.clone(),
            transcripts_subfolder,
            api_key,
            model,
            target_language: options
                .transcript_language
                .clone()
                .or_else(|| environment.default_transcript_language.clone()),
            base_url: None,
            poll_interval: None,
        };

        let progress_inner = Arc::clone(inner);
        let progress_id = id.to_string();
        let sink: crate::services::transcribe::gemini::GeminiProgressSink =
            Arc::new(move |percent: f64, _label: &str| {
                on_transcribe_progress(&progress_inner, &progress_id, percent * 100.0, None);
            });

        crate::services::transcribe::gemini::run_gemini_transcription(
            req,
            token.clone(),
            Some(sink),
        )
        .await
    };

    match engine_order {
        "whisper_only" => match try_whisper().await {
            Ok(whisper_res) => {
                outcome = Some(TranscribeStageOutcome {
                    transcript_files: whisper_res.paths.all(),
                    origin: "whisper".to_string(),
                    used_cpu_fallback: whisper_res.used_cpu_fallback,
                    hallucination_warning: whisper_res.hallucination_warning,
                });
            }
            Err(e) => return Err(e),
        },
        "gemini_only" => match try_gemini().await {
            Ok(txt_path) => {
                outcome = Some(TranscribeStageOutcome {
                    transcript_files: vec![txt_path],
                    origin: "gemini".to_string(),
                    used_cpu_fallback: false,
                    hallucination_warning: None,
                });
            }
            Err(e) => return Err(e),
        },
        "subtitles_only" => match try_subtitles().await {
            Ok(Some(sub_res)) => {
                outcome = Some(TranscribeStageOutcome {
                    transcript_files: vec![sub_res.srt_file, sub_res.txt_file],
                    origin: sub_res.origin.as_str().to_string(),
                    used_cpu_fallback: false,
                    hallucination_warning: None,
                });
            }
            Ok(None) => return Err(crate::error::AppError::NoSubtitles),
            Err(e) => return Err(e),
        },
        _ => {
            // "subtitles_whisper_gemini" (default fallback pipeline)
            // 1. Try Subtitles
            match try_subtitles().await {
                Ok(Some(sub_res)) => {
                    outcome = Some(TranscribeStageOutcome {
                        transcript_files: vec![sub_res.srt_file, sub_res.txt_file],
                        origin: sub_res.origin.as_str().to_string(),
                        used_cpu_fallback: false,
                        hallucination_warning: None,
                    });
                }
                Ok(None) => {}
                Err(e) => {
                    last_error = Some(e);
                }
            }

            // 2. Try Whisper if subtitles not found
            if outcome.is_none() {
                match try_whisper().await {
                    Ok(whisper_res) => {
                        outcome = Some(TranscribeStageOutcome {
                            transcript_files: whisper_res.paths.all(),
                            origin: "whisper".to_string(),
                            used_cpu_fallback: whisper_res.used_cpu_fallback,
                            hallucination_warning: whisper_res.hallucination_warning,
                        });
                    }
                    Err(e) => {
                        last_error = Some(e);
                    }
                }
            }

            // 3. Try Gemini if Whisper failed or unavailable
            if outcome.is_none() {
                let has_gemini_key = environment
                    .config_dir
                    .as_ref()
                    .and_then(|cd| crate::services::secrets::get_gemini_api_key(cd))
                    .is_some();

                if has_gemini_key || last_error.is_none() {
                    match try_gemini().await {
                        Ok(txt_path) => {
                            outcome = Some(TranscribeStageOutcome {
                                transcript_files: vec![txt_path],
                                origin: "gemini".to_string(),
                                used_cpu_fallback: false,
                                hallucination_warning: None,
                            });
                        }
                        Err(e) => {
                            if last_error.is_none()
                                || !matches!(e, crate::error::AppError::GeminiKeyMissing)
                            {
                                last_error = Some(e);
                            }
                        }
                    }
                }
            }
        }
    }

    let Some(mut stage_outcome) = outcome else {
        drop(_slot);
        inner.wake.notify_waiters();
        return Err(last_error.unwrap_or(crate::error::AppError::WhisperMissing));
    };

    // Optional summarization step (spec §10.4)
    let should_summarize = options.summarize.unwrap_or(environment.default_summarize);
    if should_summarize {
        let txt_file = stage_outcome
            .transcript_files
            .iter()
            .find(|f| f.extension().is_some_and(|ext| ext == "txt"))
            .cloned();

        if let Some(txt_path) = txt_file {
            if let Ok(transcript_text) = std::fs::read_to_string(&txt_path) {
                let api_key = environment
                    .config_dir
                    .as_ref()
                    .and_then(|cd| crate::services::secrets::get_gemini_api_key(cd));

                if let Some(api_key) = api_key {
                    on_transcribe_progress(inner, id, 95.0, Some("Generating summary..."));
                    let sum_req = crate::services::transcribe::summary::SummaryRequest {
                        transcript_text,
                        language: options
                            .summary_language
                            .clone()
                            .or_else(|| environment.default_summary_language.clone()),
                        model: options
                            .gemini_model
                            .clone()
                            .or_else(|| environment.default_gemini_model.clone()),
                        api_key,
                        output_dir: options.output_dir.clone(),
                        base_filename: base_filename.clone(),
                        transcripts_subfolder,
                        base_url: None,
                    };

                    match crate::services::transcribe::summary::generate_summary(
                        sum_req,
                        token.clone(),
                    )
                    .await
                    {
                        Ok(summary_path) => {
                            stage_outcome.transcript_files.push(summary_path);
                        }
                        Err(e) => {
                            let mut state = lock(&inner.state);
                            if let Some(item) = state.item_mut(id) {
                                item.push_log(&format!("Summary generation warning: {e}"));
                            }
                        }
                    }
                } else {
                    let mut state = lock(&inner.state);
                    if let Some(item) = state.item_mut(id) {
                        item.push_log("Gemini API key is not configured; summary skipped.");
                    }
                }
            }
        }
    }

    drop(_slot);
    inner.wake.notify_waiters();
    Ok(stage_outcome)
}

/// Read the item out of the state, resolve its tools and build the job request.
async fn prepare(inner: &Arc<Inner>, id: &str) -> Option<(JobEnvironment, JobRequest)> {
    let (url, options) = {
        let state = lock(&inner.state);
        let item = state.items.iter().find(|item| item.id == id)?;
        (item.url.clone(), item.options.clone())
    };

    let temp = {
        let state = lock(&inner.state);
        item_temp_for(&state.items, id)
    }
    .unwrap_or_else(|| process::item_temp_dir(&options.output_dir, id));

    match inner.provider.environment(options.clone()).await {
        Ok(environment) => {
            let request = JobRequest {
                url,
                spec: options.to_download_spec(),
                dirs: JobDirs {
                    home: options.output_dir.clone(),
                    temp,
                },
                stall_timeout: job::STALL_TIMEOUT,
            };
            Some((environment, request))
        }
        Err(error) => {
            // The item cannot run at all: record it like any other failure, so
            // the retry policy and the UI see one vocabulary (spec §8.5).
            finish_item(
                inner,
                id,
                JobOutcome::Failed(YtdlpError::new(
                    classify(&error.code),
                    error.detail.unwrap_or_default(),
                )),
            );
            None
        }
    }
}

/// Map a command-level [`ErrorInfo`] onto a classified ytdlp error.
fn classify(code: &str) -> YtdlpErrorCode {
    match code {
        "UNSUPPORTED" => YtdlpErrorCode::Unsupported,
        "IO" => YtdlpErrorCode::DiskFull,
        _ => YtdlpErrorCode::Unknown,
    }
}

/// Fold one progress payload into the item (spec §3, §7.6).
fn on_progress(inner: &Arc<Inner>, id: &str, progress: &JobProgress) {
    let milestone = {
        let mut state = lock(&inner.state);
        let Some(item) = state.item_mut(id) else {
            return;
        };
        // A control signal means the item is on its way out; a late progress
        // line must not drag it back into a running state.
        if item.control != ControlSignal::None {
            return;
        }
        let next = ItemProgress::state_of(progress.stage);
        // "Preparing" is progress inside a running item, not a state of its
        // own: the item is downloading whatever the stage label says.
        let next = if next == ItemState::Queued {
            item.state
        } else {
            next
        };
        let milestone = next != item.state;
        item.state = next;
        item.progress.apply(progress);
        item.updated_at = now_unix();
        milestone
    };
    mark_dirty(inner, id, milestone);
}

/// Record the end of an item: done, failed, cancelled, paused, removed.
fn finish_item(inner: &Arc<Inner>, id: &str, outcome: JobOutcome) {
    let now = now_unix();
    // An item that is being cancelled or removed loses its temp directory
    // **before** the row says so, and with the lock released: nothing may
    // observe "Cancelled" and then look for a folder that is still being
    // deleted — the startup sweep trusts that the two go together (spec §4.10).
    //
    // No TOCTOU here: the item is finished, so the scheduler will not start it
    // again, and its temp path does not depend on the state being written.
    let doomed = {
        let state = lock(&inner.state);
        state.item(id).and_then(|item| {
            let doomed = matches!(item.control, ControlSignal::Cancel | ControlSignal::Remove)
                || matches!(&outcome, JobOutcome::Cancelled);
            doomed.then(|| item.temp_dir())
        })
    };
    if let Some(temp) = &doomed {
        job::cleanup_job_dir(temp);
    }

    let mut toast: Option<QueueToast> = None;
    let notify;
    {
        let mut state = lock(&inner.state);
        let Some(index) = state.index_of(id) else {
            return;
        };
        state.items[index].transcript_preview = None;
        let control = state.items[index].control;

        match control {
            ControlSignal::Pause => {
                let item = &mut state.items[index];
                item.paused_from = Some(item.state);
                item.state = ItemState::Paused;
                item.control = ControlSignal::None;
                item.progress.speed_bps = None;
                item.progress.eta_sec = None;
                item.updated_at = now;
                // Partial files stay (spec §7.3): yt-dlp continues from them.
            }
            ControlSignal::Cancel | ControlSignal::Remove => {
                if control == ControlSignal::Remove {
                    state.items.remove(index);
                    state.order_dirty = true;
                    note_removed(inner, id);
                } else {
                    let item = &mut state.items[index];
                    item.state = ItemState::Cancelled;
                    item.control = ControlSignal::None;
                    item.error = None;
                    item.finished_at = Some(now);
                    item.updated_at = now;
                    item.progress = ItemProgress::empty();
                }
                // The temp directory is already gone: see the top of the
                // function.
            }
            ControlSignal::None => match outcome {
                JobOutcome::Done {
                    file,
                    runtime,
                    requested_height,
                    actual_height,
                    ..
                } => {
                    // Spec §11.5 asks the card for the size. The only honest
                    // source is the file yt-dlp has just produced, and a file
                    // that vanished between the two lines is not a failure of
                    // the download: the size is simply unknown.
                    let size_bytes = std::fs::metadata(&file).ok().map(|meta| meta.len());
                    let item = &mut state.items[index];
                    item.state = ItemState::Done;
                    item.file = Some(file);
                    item.size_bytes = size_bytes;
                    item.runtime = Some(runtime.as_str().to_owned());
                    item.requested_height = requested_height;
                    item.actual_height = actual_height;
                    item.error = None;
                    item.finished_at = Some(now);
                    item.updated_at = now;
                    item.progress.percent = Some(100.0);
                    item.progress.speed_bps = None;
                    item.progress.eta_sec = None;
                }
                JobOutcome::SkippedArchive { runtime, .. } => {
                    let item = &mut state.items[index];
                    item.state = ItemState::Done;
                    item.file = None;
                    item.size_bytes = None;
                    item.runtime = Some(runtime.as_str().to_owned());
                    item.error = None;
                    item.finished_at = Some(now);
                    item.updated_at = now;
                    item.progress.percent = Some(100.0);
                    item.progress.speed_bps = None;
                    item.progress.eta_sec = None;
                    item.progress.label = "skippedArchive".to_owned();
                }
                JobOutcome::Failed(error) => {
                    let code = error.code;
                    let retry = {
                        let item = &state.items[index];
                        should_auto_retry(code, item.auto_retries)
                            .then(|| item.backoff_for_next_retry())
                            .flatten()
                    };
                    let item = &mut state.items[index];
                    item.finished_at = Some(now);
                    item.updated_at = now;
                    item.progress.speed_bps = None;
                    item.progress.eta_sec = None;
                    item.error = Some(error.to_error_info());
                    match retry {
                        Some(delay) => {
                            // Spec §7.3: up to 2 automatic retries, 5 s then 20 s.
                            item.auto_retries += 1;
                            item.retry_at = Some(now + delay.as_secs());
                            item.state = ItemState::Queued;
                            item.started_at = None;
                            toast = Some(
                                QueueToast::new("toast.queueRetrying")
                                    .with("count", item.auto_retries),
                            );
                        }
                        None => item.state = ItemState::Failed,
                    }
                }
                JobOutcome::Cancelled => {
                    let item = &mut state.items[index];
                    item.state = ItemState::Cancelled;
                    item.finished_at = Some(now);
                    item.updated_at = now;
                    item.progress = ItemProgress::empty();
                    // Temp directory already gone, see above.
                }
            },
        }

        // Spec §7.3: keep the last 500 finished items, then roll off.
        for removed in roll_off_history(&mut state.items) {
            note_removed(inner, &removed);
        }
        state.order_dirty = true;

        // Spec §7.7: notify on the *transition* to idle, once.
        let summary = state.summary(*lock(&inner.lanes));
        notify = summary.became_idle(&state.previous);
        state.previous = summary;
    };

    lock(&inner.running).remove(id);
    if let Some(toast) = toast {
        push_toast(inner, toast);
    }

    self_persist(inner);
    // A freed lane may let something else run, and a retry timer may have just
    // been set: both tasks have to look at the queue again.
    mark_dirty(inner, id, true);
    inner.dirty.notify_waiters();
    inner.wake.notify_waiters();

    if notify && *lock(&inner.notify_on_idle) {
        let summary = {
            let lanes = *lock(&inner.lanes);
            lock(&inner.state).summary(lanes)
        };
        if summary.failed > 0 {
            push_toast(inner, QueueToast::new("toast.queueIdleWithErrors"));
        }
        if let Some(notifier) = inner.notifier.as_ref() {
            notifier.notify(&summary);
        }
    }
}

/// Mark an item dirty for the flusher.
fn mark_dirty(inner: &Arc<Inner>, id: &str, force: bool) {
    {
        let mut coalescer = lock(&inner.coalescer);
        coalescer.mark(id, force);
        coalescer.mark_summary();
    }
    inner.dirty.notify_waiters();
}

/// Build and send `queue:update` payloads (spec §7.6).
async fn flusher_loop(inner: Arc<Inner>) {
    loop {
        if inner.stopped.load(Ordering::SeqCst) {
            return;
        }
        let notified = inner.dirty.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        let batch = take_batch(&inner);
        if let Some(update) = batch {
            (inner.sink)(&update);
        }

        if inner.stopped.load(Ordering::SeqCst) {
            return;
        }
        let deadline = lock(&inner.coalescer).next_deadline(Instant::now());
        match deadline {
            Some(at) => {
                let delay = at.saturating_duration_since(Instant::now());
                tokio::select! {
                    () = &mut notified => {}
                    () = tokio::time::sleep(delay) => {}
                }
            }
            None => notified.await,
        }
    }
}

/// Whatever is due right now, as one event — or `None` when nothing is.
///
/// The lock is taken and released before the sink runs: emitting a Tauri event
/// can wake the webview, and nothing should hold the queue's lock while that
/// happens.
fn take_batch(inner: &Arc<Inner>) -> Option<QueueUpdate> {
    let (due, summary_due) = {
        let mut coalescer = lock(&inner.coalescer);
        coalescer.drain(Instant::now())
    };
    let removed = std::mem::take(&mut *lock(&inner.pending_removed));
    let toast = lock(&inner.pending_toasts).pop();
    if due.is_empty() && !summary_due && removed.is_empty() && toast.is_none() {
        return None;
    }

    let mut state = lock(&inner.state);
    let items: Vec<QueueItem> = due
        .iter()
        .filter_map(|id| state.items.iter().find(|item| &item.id == id))
        .cloned()
        .collect();
    let order = if state.order_dirty {
        Some(state.items.iter().map(|item| item.id.clone()).collect())
    } else {
        None
    };
    let groups = if state.groups_dirty {
        Some(state.groups.clone())
    } else {
        None
    };
    state.order_dirty = false;
    state.groups_dirty = false;
    let summary = state.summary(*lock(&inner.lanes));
    let seq = inner.seq.fetch_add(1, Ordering::Relaxed) + 1;
    drop(state);

    Some(QueueUpdate {
        seq,
        items,
        removed,
        order,
        groups,
        summary,
        toast,
    })
}

// ---------------------------------------------------------------------------
// Small helpers shared with the command layer
// ---------------------------------------------------------------------------

/// The `JobDirs` of an item (spec §4.9).
pub fn dirs_of(item: &QueueItem) -> JobDirs {
    job_dirs(item)
}

/// Where the queue file lives inside a data directory.
pub fn file_in(data_dir: &Path) -> PathBuf {
    data_dir.join(store::QUEUE_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::queue::model::{Kind, QualityChoice};
    use crate::services::ytdlp::CookieSource;
    use tempfile::TempDir;

    /// A provider that always resolves the same (fake) environment.
    struct StubProvider {
        result: Mutex<Result<JobEnvironment, ErrorInfo>>,
    }

    impl StubProvider {
        fn ok() -> Self {
            Self {
                result: Mutex::new(Ok(JobEnvironment {
                    ytdlp: PathBuf::from("/nonexistent/yt-dlp"),
                    ffmpeg: None,
                    ffprobe: None,
                    whisper: None,
                    models_dir: None,
                    config_dir: None,
                    whisper_threads: 0,
                    skip_silence_vad: true,
                    default_whisper_model: None,
                    default_transcript_language: None,
                    default_transcription_engine_order: None,
                    default_gemini_model: None,
                    default_summarize: false,
                    default_summary_language: None,
                    runtimes: crate::services::inspect::Runtimes {
                        mode: crate::services::jsruntime::JsRuntimeMode::QuickJsOnly,
                        qjs: None,
                        deno: None,
                        ytdlp_version: String::new(),
                    },
                    memory: Arc::new(crate::services::jsruntime::RuntimeMemoryStore::load(
                        PathBuf::from("/nonexistent/runtime-memory.json"),
                    )),
                    private_temp: None,
                    youtube_clients: None,
                })),
            }
        }
    }

    impl ToolProvider for StubProvider {
        fn environment(
            &self,
            _options: ItemOptions,
        ) -> BoxFuture<Result<JobEnvironment, ErrorInfo>> {
            let result = match self.result.lock() {
                Ok(guard) => guard.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            };
            Box::pin(std::future::ready(result))
        }
    }

    fn collector() -> (EventSink, Arc<Mutex<Vec<QueueUpdate>>>) {
        let seen: Arc<Mutex<Vec<QueueUpdate>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: EventSink = {
            let seen = Arc::clone(&seen);
            Arc::new(move |update: &QueueUpdate| {
                if let Ok(mut seen) = seen.lock() {
                    seen.push(update.clone());
                }
            })
        };
        (sink, seen)
    }

    /// An engine that never starts its tasks: these tests exercise the state
    /// machine synchronously.
    fn engine(dir: &TempDir) -> QueueEngine {
        let (sink, _seen) = collector();
        QueueEngine::open(EngineConfig {
            path: dir.path().join("queue.json"),
            provider: Arc::new(StubProvider::ok()),
            sink,
            lanes: LaneConfig::default(),
            notify_on_idle: false,
            notifier: None,
        })
    }

    fn request(url: &str, out: &Path) -> AddRequest {
        AddRequest {
            url: url.to_owned(),
            title: Some(format!("title of {url}")),
            site: Some("Youtube".into()),
            duration_sec: Some(10),
            options: ItemOptions {
                kind: Kind::Video,
                quality: QualityChoice::MaxHeight { height: 1080 },
                output_dir: out.to_path_buf(),
                cookies: crate::services::queue::model::CookieChoice::None,
                runtime_mode: crate::services::jsruntime::JsRuntimeMode::Auto,
                ..ItemOptions::default()
            },
            group_id: None,
            file: None,
        }
    }

    #[test]
    fn adding_two_links_creates_two_items_in_order() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        let outcome = engine.add(
            vec![
                request("https://a.example/1", dir.path()),
                request("https://b.example/2", dir.path()),
            ],
            false,
        );
        assert_eq!(outcome.added.len(), 2);
        assert!(outcome.duplicates.is_empty());
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.items.len(), 2);
        assert_eq!(snapshot.items[0].url, "https://a.example/1");
        assert_eq!(snapshot.summary.queued, 2);
        assert!(snapshot.summary.is_active(), "queued work counts as active");
    }

    #[test]
    fn the_same_url_with_the_same_options_is_refused_and_can_be_forced() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        let first = engine.add(vec![request("https://a.example/1", dir.path())], false);
        let id = first.added[0].clone();

        // Same URL, different spelling, same options → one duplicate.
        let second = engine.add(vec![request("https://www.a.example/1", dir.path())], false);
        assert!(second.added.is_empty());
        assert_eq!(second.duplicates.len(), 1);
        assert_eq!(second.duplicates[0].item_id, id);
        assert_eq!(engine.snapshot().items.len(), 1);

        // The user says yes.
        let third = engine.add(vec![request("https://www.a.example/1", dir.path())], true);
        assert_eq!(third.added.len(), 1);
        assert_eq!(engine.snapshot().items.len(), 2);
    }

    #[test]
    fn a_different_quality_is_not_a_duplicate() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        engine.add(vec![request("https://a.example/1", dir.path())], false);
        let mut other = request("https://a.example/1", dir.path());
        other.options.quality = QualityChoice::AudioOnly;
        let outcome = engine.add(vec![other], false);
        assert_eq!(outcome.added.len(), 1);
        assert!(outcome.duplicates.is_empty());
    }

    #[test]
    fn a_finished_item_does_not_block_the_same_link_again() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        let id = engine
            .add(vec![request("https://a.example/1", dir.path())], false)
            .added[0]
            .clone();
        // Finish it without ever running: a terminal state is what matters.
        {
            let inner = &engine.inner;
            let mut state = lock(&inner.state);
            state.item_mut(&id).expect("the item is there").state = ItemState::Done;
        }
        let outcome = engine.add(vec![request("https://a.example/1", dir.path())], false);
        assert_eq!(outcome.added.len(), 1);
        assert!(outcome.duplicates.is_empty());
    }

    #[test]
    fn remove_and_clear_finished_keep_what_can_still_run() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        let outcome = engine.add(
            vec![
                request("https://a.example/1", dir.path()),
                request("https://a.example/2", dir.path()),
                request("https://a.example/3", dir.path()),
            ],
            false,
        );
        let ids = outcome.added;
        {
            let mut state = lock(&engine.inner.state);
            state.items[0].state = ItemState::Done;
            state.items[1].state = ItemState::Failed;
        }

        let cleared = engine.clear_finished();
        assert_eq!(cleared.len(), 2);
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].id, ids[2]);

        assert_eq!(engine.remove("nope"), Outcome::NotFound);
        assert_eq!(engine.remove(&ids[2]), Outcome::Ok);
        assert!(engine.snapshot().items.is_empty());
    }

    #[test]
    fn reorder_puts_the_named_items_first_and_keeps_the_rest_behind() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        let ids = engine
            .add(
                vec![
                    request("https://a.example/1", dir.path()),
                    request("https://a.example/2", dir.path()),
                    request("https://a.example/3", dir.path()),
                ],
                false,
            )
            .added;
        engine.reorder(&[ids[2].clone()]);
        let order: Vec<String> = engine
            .snapshot()
            .items
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(order, vec![ids[2].clone(), ids[0].clone(), ids[1].clone()]);

        engine.move_to_top(&ids[0]);
        let order: Vec<String> = engine
            .snapshot()
            .items
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(order[0], ids[0]);
        assert_eq!(engine.move_to_top("nope"), Outcome::NotFound);
    }

    #[test]
    fn retry_requeues_a_failed_item_and_gives_the_budget_back() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        let id = engine
            .add(vec![request("https://a.example/1", dir.path())], false)
            .added[0]
            .clone();
        {
            let mut state = lock(&engine.inner.state);
            let item = state.item_mut(&id).expect("the item is there");
            item.state = ItemState::Failed;
            item.auto_retries = 2;
            item.error = Some(ErrorInfo::new(crate::error::ErrorCode::Io, "boom"));
        }
        assert_eq!(engine.retry(&id), Outcome::Ok);
        let item = engine
            .snapshot()
            .items
            .into_iter()
            .find(|item| item.id == id)
            .expect("still there");
        assert_eq!(item.state, ItemState::Queued);
        assert_eq!(item.auto_retries, 0);
        assert!(item.error.is_none());
        assert_eq!(engine.retry("nope"), Outcome::NotFound);
    }

    #[test]
    fn pause_all_stops_the_scheduler_and_resume_all_lets_them_go() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        let ids = engine
            .add(
                vec![
                    request("https://a.example/1", dir.path()),
                    request("https://a.example/2", dir.path()),
                ],
                false,
            )
            .added;
        engine.pause_all();
        assert!(engine.snapshot().summary.paused_all);
        // Nothing runs (the tasks are not even started here), so pause_all has
        // nothing to cancel; the flag is what matters and resume clears it.
        let resumed = engine.resume_all();
        assert!(resumed.is_empty(), "queued items are not paused items");
        assert!(!engine.snapshot().summary.paused_all);
        let _ = ids;
    }

    #[test]
    fn live_output_dirs_are_the_ones_a_pending_item_writes_to() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        let out = dir.path().join("out");
        let other = dir.path().join("other");
        let id = engine
            .add(vec![request("https://a.example/1", &out)], false)
            .added[0]
            .clone();
        let done_id = engine
            .add(vec![request("https://a.example/2", &other)], false)
            .added[0]
            .clone();
        {
            let mut state = lock(&engine.inner.state);
            state.item_mut(&done_id).expect("there").state = ItemState::Done;
        }
        assert_eq!(engine.live_output_dirs(), vec![out.clone()]);
        assert_eq!(engine.owned_temp_names(), vec![id]);
    }

    #[test]
    fn adding_writes_the_file_and_the_file_comes_back_on_its_own() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        engine.add(vec![request("https://a.example/1", dir.path())], false);
        assert!(dir.path().join("queue.json").is_file());

        // A second engine over the same file is what a restart looks like.
        let reopened = self::tests::engine(&dir);
        let snapshot = reopened.snapshot();
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].url, "https://a.example/1");
    }

    #[test]
    fn paused_all_survives_a_restart() {
        let dir = TempDir::new().expect("temp dir");
        let engine = self::tests::engine(&dir);
        engine.pause_all();
        let reopened = self::tests::engine(&dir);
        assert!(reopened.snapshot().summary.paused_all);
    }

    /// A provider whose tool lookup never finishes, so an item started against
    /// it stays in its first running state and nothing can finish it by
    /// accident.
    struct HangingProvider;

    impl ToolProvider for HangingProvider {
        fn environment(
            &self,
            _options: ItemOptions,
        ) -> BoxFuture<Result<JobEnvironment, ErrorInfo>> {
            Box::pin(std::future::pending())
        }
    }

    #[tokio::test]
    async fn the_file_says_an_item_is_running_before_the_tool_is_spawned() {
        let dir = TempDir::new().expect("temp dir");
        let (sink, _seen) = collector();
        let engine = QueueEngine::open(EngineConfig {
            path: dir.path().join("queue.json"),
            provider: Arc::new(HangingProvider),
            sink,
            lanes: LaneConfig::default(),
            notify_on_idle: false,
            notifier: None,
        });
        let id = engine
            .add(vec![request("https://a.example/1", dir.path())], false)
            .added[0]
            .clone();

        // What the scheduler does, without the scheduler: the item is running
        // and a tool may be about to exist.
        start_item(&engine.inner, id.clone());

        // Read the **file**, not the engine. The invariant is about what a
        // restart would find: a document that still says `Queued` for an item
        // with yt-dlp behind it comes back from a crash as "not started yet",
        // and the scheduler would run it again by itself (spec §7.4, D91).
        let on_disk = read_queue_file(&dir);
        let recorded = on_disk
            .items
            .iter()
            .find(|item| item.id == id)
            .expect("the item is in the file")
            .state;
        assert!(
            recorded.lane().is_some(),
            "a running item is recorded as active, never as queued: {recorded:?}"
        );

        // And the recovery of spec §7.4 works off that: the next launch parks
        // it and waits for the user instead of downloading it again.
        let reopened = self::tests::engine(&dir);
        assert_eq!(state_of_id(&reopened, &id), ItemState::Interrupted);
        assert_eq!(
            reopened.owned_temp_names(),
            vec![id],
            "its temp dir is still its own (spec §4.10)"
        );
        engine.shutdown();
    }

    /// Spec §4.2: "On app exit cancel all running items" — and an item the
    /// scheduler planned a moment before the exit must not start *after* it.
    ///
    /// The assertion is the item's state, not "the test survived": on the old
    /// code this test reached `tokio::spawn` from a synchronous `#[test]` and
    /// panicked with "there is no reactor running". The panic **is** the bug,
    /// seen from a test that has no runtime to spawn into.
    #[test]
    fn nothing_starts_after_the_engine_is_shut_down() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        let id = engine
            .add(vec![request("https://a.example/1", dir.path())], false)
            .added[0]
            .clone();
        engine.shutdown();

        // Exactly what the scheduler's loop does with a plan it computed a
        // moment earlier.
        start_item(&engine.inner, id.clone());

        let item = engine
            .snapshot()
            .items
            .into_iter()
            .find(|item| item.id == id)
            .expect("the item is still there");
        assert_eq!(item.state, ItemState::Queued, "it never started");
        assert!(
            lock(&engine.inner.running).is_empty(),
            "and no process is registered for it"
        );
    }

    fn read_queue_file(dir: &TempDir) -> crate::services::queue::store::QueueFile {
        let raw = std::fs::read_to_string(dir.path().join("queue.json"))
            .expect("the queue file is there");
        serde_json::from_str(&raw).expect("the queue file is valid JSON")
    }

    #[test]
    fn pausing_an_item_that_is_not_running_parks_it_instead_of_cancelling_it() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        let id = engine
            .add(vec![request("https://a.example/1", dir.path())], false)
            .added[0]
            .clone();

        // Only reachable from a stale view of the row (the engine pauses running
        // items through their token), and it must not pretend the item was
        // running: a queued item the user paused comes back with Resume.
        assert_eq!(engine.pause(&id), Outcome::Ok);
        let item = engine
            .snapshot()
            .items
            .into_iter()
            .find(|item| item.id == id)
            .expect("still there");
        assert_eq!(item.state, ItemState::Paused);
        assert_eq!(item.paused_from, Some(ItemState::Queued));
        assert_eq!(engine.resume(&id), Outcome::Ok);
        assert_eq!(state_of_id(&engine, &id), ItemState::Queued);

        // And a parked item is not paused again into something else.
        engine.pause(&id);
        assert_eq!(state_of_id(&engine, &id), ItemState::Paused);
    }

    fn state_of_id(engine: &QueueEngine, id: &str) -> ItemState {
        engine
            .snapshot()
            .items
            .into_iter()
            .find(|item| item.id == id)
            .expect("the item")
            .state
    }

    #[test]
    fn the_classifier_maps_command_errors_onto_the_error_vocabulary() {
        assert_eq!(classify("UNSUPPORTED"), YtdlpErrorCode::Unsupported);
        assert_eq!(classify("IO"), YtdlpErrorCode::DiskFull);
        assert_eq!(classify("WHATEVER"), YtdlpErrorCode::Unknown);
    }

    #[test]
    fn an_item_without_cookies_asks_for_none() {
        let options = ItemOptions::default();
        assert_eq!(options.to_download_spec().cookies, CookieSource::None);
    }

    #[test]
    fn group_actions_apply_only_to_the_named_group() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        engine.upsert_group(QueueGroup {
            id: "g1".into(),
            title: "Playlist One".into(),
            folder: Some(dir.path().join("Playlist One")),
            index: 0,
        });
        let mut r1 = request("https://a.example/1", dir.path());
        r1.group_id = Some("g1".into());
        let mut r2 = request("https://a.example/2", dir.path());
        r2.group_id = Some("g1".into());
        let r3 = request("https://a.example/3", dir.path());
        let ids = engine.add(vec![r1, r2, r3], false).added;

        let paused = engine.pause_group("g1");
        assert_eq!(paused.len(), 2);
        assert_eq!(state_of_id(&engine, &ids[0]), ItemState::Paused);
        assert_eq!(state_of_id(&engine, &ids[1]), ItemState::Paused);
        assert_eq!(state_of_id(&engine, &ids[2]), ItemState::Queued);

        let resumed = engine.resume_group("g1");
        assert_eq!(resumed.len(), 2);
        assert_eq!(state_of_id(&engine, &ids[0]), ItemState::Queued);

        {
            let mut state = lock(&engine.inner.state);
            state.item_mut(&ids[0]).expect("there").state = ItemState::Failed;
        }
        let retried = engine.retry_group_failed("g1");
        assert_eq!(retried, vec![ids[0].clone()]);
        assert_eq!(state_of_id(&engine, &ids[0]), ItemState::Queued);

        let removed = engine.remove_group("g1");
        assert_eq!(removed.len(), 2);
        let snap = engine.snapshot();
        assert_eq!(snap.items.len(), 1);
        assert_eq!(snap.items[0].id, ids[2]);
        assert!(snap.groups.is_empty());
    }

    #[tokio::test]
    async fn queued_local_transcription_items_plan_on_transcribe_lane_not_download_lane() {
        let dir = TempDir::new().expect("temp dir");
        let (sink, _seen) = collector();
        let engine = QueueEngine::open(EngineConfig {
            path: dir.path().join("queue.json"),
            provider: Arc::new(HangingProvider),
            sink,
            lanes: LaneConfig::default(), // download: 2, encode: 1, transcribe: 1
            notify_on_idle: false,
            notifier: None,
        });

        let local1 = dir.path().join("local1.mp4");
        let local2 = dir.path().join("local2.mp4");
        std::fs::write(&local1, b"fake media 1").expect("write local1");
        std::fs::write(&local2, b"fake media 2").expect("write local2");

        // Case (b): Add 2 local files for transcription when Download lane has 2 free slots
        // and Transcribe lane has capacity 1. Only 1 may start in a scheduler tick!
        let mut req1 = request("local://1", dir.path());
        req1.file = Some(local1);
        req1.options.transcribe = true;

        let mut req2 = request("local://2", dir.path());
        req2.file = Some(local2);
        req2.options.transcribe = true;

        let ids = engine.add(vec![req1, req2], true).added;
        tick(&engine.inner).await;

        assert_eq!(
            state_of_id(&engine, &ids[0]),
            ItemState::Transcribing,
            "first local transcription item should start on Transcribe lane"
        );
        assert_eq!(
            state_of_id(&engine, &ids[1]),
            ItemState::Queued,
            "second local transcription item must stay Queued because Transcribe lane cap is 1"
        );

        engine.shutdown();
    }

    #[tokio::test]
    async fn local_transcription_starts_when_download_lane_is_full_and_transcribe_lane_is_idle() {
        let dir = TempDir::new().expect("temp dir");
        let (sink, _seen) = collector();
        let engine = QueueEngine::open(EngineConfig {
            path: dir.path().join("queue.json"),
            provider: Arc::new(HangingProvider),
            sink,
            lanes: LaneConfig::default(), // download: 2, encode: 1, transcribe: 1
            notify_on_idle: false,
            notifier: None,
        });

        // Start 2 downloads so Lane::Download (cap 2) is completely full
        let dl_ids = engine
            .add(
                vec![
                    request("https://site1.example/1", dir.path()),
                    request("https://site2.example/2", dir.path()),
                ],
                true,
            )
            .added;
        tick(&engine.inner).await;
        assert_eq!(state_of_id(&engine, &dl_ids[0]), ItemState::Downloading);
        assert_eq!(state_of_id(&engine, &dl_ids[1]), ItemState::Downloading);

        // Case (a): Now add a local file for transcription while Lane::Download is full
        // and Lane::Transcribe is idle. It must start immediately on the next tick!
        let local = dir.path().join("local.mp4");
        std::fs::write(&local, b"fake media").expect("write local");
        let mut local_req = request("local://media", dir.path());
        local_req.file = Some(local);
        local_req.options.transcribe = true;

        let local_id = engine.add(vec![local_req], true).added[0].clone();
        tick(&engine.inner).await;

        assert_eq!(
            state_of_id(&engine, &local_id),
            ItemState::Transcribing,
            "local transcription item must start even when Download lane is full"
        );

        engine.shutdown();
    }

    #[test]
    fn finish_item_clears_transcript_preview() {
        let dir = TempDir::new().expect("temp dir");
        let engine = engine(&dir);
        let id = engine
            .add(vec![request("https://a.example/1", dir.path())], false)
            .added[0]
            .clone();

        {
            let mut state = lock(&engine.inner.state);
            let item = state.item_mut(&id).expect("the item is there");
            item.state = ItemState::Transcribing;
            item.transcript_preview = Some("live segment preview".to_string());
        }

        finish_item(
            &engine.inner,
            &id,
            JobOutcome::Done {
                file: dir.path().join("out.mp4"),
                runtime: crate::services::jsruntime::RuntimeKind::QuickJs,
                elapsed_ms: 100,
                requested_height: None,
                actual_height: None,
            },
        );

        let snapshot = engine.snapshot();
        let item = snapshot
            .items
            .iter()
            .find(|i| i.id == id)
            .expect("item exists");
        assert_eq!(item.state, ItemState::Done);
        assert_eq!(
            item.transcript_preview, None,
            "transcript_preview must be cleared when item finishes"
        );
    }
}
