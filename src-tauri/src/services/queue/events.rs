//! The `queue:update` event of spec §3 / §7.6.
//!
//! One event carries the **changed items**, the ids that went away, the queue
//! order when it changed, and the counters of spec §7.5. The frontend keys its
//! store by item id and patches single rows, so a tick with one changed item
//! repaints one row (spec §11.7).
//!
//! ## What is coalesced, and what is sent whole
//!
//! Spec §7.6 asks for two things: "coalesce progress per item and emit at most
//! ~10 updates/s" and "send only changed fields".
//!
//! * The **first** is implemented here, in [`Coalescer`], with the same escape
//!   hatch the M1 job emitter uses: a stage change goes out immediately, because
//!   hiding it for 100 ms would make the UI flicker back to the old stage.
//! * The second is implemented at **item granularity**: an item that changed is
//!   sent whole, and an item that did not change is not sent at all. Field-level
//!   patches would need `Option<Option<T>>` on the wire and a merge step in the
//!   store; a queue item is a few hundred bytes, and the win from not sending
//!   three unchanged strings is smaller than the risk of a frontend store that
//!   drifts from the backend (see D81).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::services::queue::model::{ItemState, QueueGroup, QueueItem, QueueSummary};

/// Spec §7.6: "emit at most ~10 updates/s" per item.
pub const UPDATE_INTERVAL: Duration = Duration::from_millis(100);

/// The `queue:update` payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueUpdate {
    /// Monotonic; the frontend can drop anything older than what it has.
    pub seq: u64,
    /// New or changed items, complete.
    pub items: Vec<QueueItem>,
    /// Ids that no longer exist (removed, rolled off by the history cap).
    pub removed: Vec<String>,
    /// The queue order, **only when it changed**. Re-sending a 5 000-entry
    /// array ten times a second would cost more than the items themselves.
    pub order: Option<Vec<String>>,
    /// The group list, **only when it changed** (playlist added or group removed).
    pub groups: Option<Vec<QueueGroup>>,
    pub summary: QueueSummary,
    /// A transient message for the toast area (spec §11.7).
    pub toast: Option<QueueToast>,
}

/// A toast. i18n keys only: no sentence is ever built in the backend
/// (spec §11.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueToast {
    pub message_key: String,
    /// Placeholders for `t()`, e.g. `{count}`.
    pub params: BTreeMap<String, String>,
}

impl QueueToast {
    pub fn new(message_key: impl Into<String>) -> Self {
        Self {
            message_key: message_key.into(),
            params: BTreeMap::new(),
        }
    }

    pub fn with(mut self, name: impl Into<String>, value: impl ToString) -> Self {
        self.params.insert(name.into(), value.to_string());
        self
    }
}

/// The sink the engine hands its events to (the Tauri emitter in the app, a
/// collector in the tests).
pub type EventSink = std::sync::Arc<dyn Fn(&QueueUpdate) + Send + Sync>;

/// Everything a fresh frontend needs to render the queue (spec §3 `queue_list`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub seq: u64,
    pub items: Vec<QueueItem>,
    pub groups: Vec<QueueGroup>,
    pub summary: QueueSummary,
    /// Set when `queue.json` was broken or held interrupted items (spec §7.4).
    pub toast: Option<QueueToast>,
}

/// Per-item rate limiting (spec §7.6).
///
/// Pure clock arithmetic: `Instant` values are passed in, so the tests do not
/// sleep and do not depend on the wall clock.
#[derive(Debug, Default)]
pub struct Coalescer {
    interval: Duration,
    last_sent: BTreeMap<String, Instant>,
    pending: BTreeSet<String>,
    /// Ids that bypass the window because their state changed.
    forced: BTreeSet<String>,
    last_summary: Option<Instant>,
    /// Set when something changed the counters of spec §7.5.
    summary_dirty: bool,
}

impl Coalescer {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_sent: BTreeMap::new(),
            pending: BTreeSet::new(),
            forced: BTreeSet::new(),
            last_summary: None,
            summary_dirty: false,
        }
    }

    /// Something about `id` changed. `force` is for state changes, which must
    /// not wait for the window: hiding a stage change behind a 100 ms window is
    /// what makes a UI flicker back to the previous state.
    pub fn mark(&mut self, id: &str, force: bool) {
        self.pending.insert(id.to_owned());
        if force {
            self.forced.insert(id.to_owned());
        }
    }

    /// The counters of spec §7.5 changed.
    ///
    /// Dirty is a flag, not a timestamp: a queue whose last item was removed has
    /// no item left to carry the summary, and the frontend still has to learn
    /// that the queue is empty.
    pub fn mark_summary(&mut self) {
        self.summary_dirty = true;
    }

    /// Has this item's window elapsed?
    pub fn is_due(&self, id: &str, now: Instant) -> bool {
        self.forced.contains(id)
            || self
                .last_sent
                .get(id)
                .is_none_or(|last| now.duration_since(*last) >= self.interval)
    }

    fn mark_sent(&mut self, id: &str, now: Instant) {
        self.last_sent.insert(id.to_owned(), now);
        self.pending.remove(id);
        self.forced.remove(id);
    }

    /// Take the ids that may go out now, plus whether the summary may as well.
    pub fn drain(&mut self, now: Instant) -> (Vec<String>, bool) {
        let due: Vec<String> = self
            .pending
            .iter()
            .filter(|id| self.is_due(id, now))
            .cloned()
            .collect();
        for id in &due {
            self.mark_sent(id, now);
        }
        let window_elapsed = self
            .last_summary
            .is_none_or(|last| now.duration_since(last) >= self.interval);
        let summary_due = self.summary_dirty && window_elapsed;
        if summary_due {
            self.last_summary = Some(now);
            self.summary_dirty = false;
        }
        (due, summary_due)
    }

    /// When the next pending item becomes due, so the flusher knows how long it
    /// may sleep. `None` when nothing is pending.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        if self.pending.is_empty() {
            return None;
        }
        if self.pending.iter().any(|id| self.is_due(id, now)) {
            return Some(now);
        }
        self.pending
            .iter()
            .filter_map(|id| self.last_sent.get(id))
            .map(|last| *last + self.interval)
            .filter(|at| *at > now)
            .min()
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Forget an item entirely: after this it is emitted at most once per
    /// window again, and never sits in the pending set.
    pub fn forget(&mut self, id: &str) {
        self.last_sent.remove(id);
        self.pending.remove(id);
        self.forced.remove(id);
    }
}

/// Does this change deserve to bypass the window?
///
/// A stage or state change is what the eye follows; a percentage is not. This
/// is the rule M1's emitter used, kept in one place so both agree.
pub fn is_milestone(previous: ItemState, next: ItemState) -> bool {
    previous != next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_of_ticks_reaches_the_sink_about_ten_times() {
        let mut coalescer = Coalescer::new(UPDATE_INTERVAL);
        let start = Instant::now();
        let mut sent = 0;
        // One second of progress ticks, 1 ms apart: one emission per 100 ms
        // window, which is the "≤ 10/s" of spec §7.6.
        for tick in 0..1_000 {
            coalescer.mark("q1", false);
            let (ids, _) = coalescer.drain(start + Duration::from_millis(tick));
            sent += ids.len();
        }
        assert!(
            (9..=11).contains(&sent),
            "{sent} updates in one second, expected about 10"
        );
    }

    #[test]
    fn a_state_change_is_never_coalesced_away() {
        let mut coalescer = Coalescer::new(UPDATE_INTERVAL);
        let start = Instant::now();
        coalescer.mark("q1", false);
        let (ids, _) = coalescer.drain(start);
        assert_eq!(ids, vec!["q1".to_owned()]);

        // Twenty ticks inside the same window: none of them is due.
        for tick in 1..20 {
            coalescer.mark("q1", false);
            let (ids, _) = coalescer.drain(start + Duration::from_millis(tick));
            assert!(ids.is_empty());
        }
        // The state change is `force`d through immediately.
        coalescer.mark("q1", true);
        let (ids, _) = coalescer.drain(start + Duration::from_millis(20));
        assert_eq!(ids, vec!["q1".to_owned()]);
    }

    #[test]
    fn a_forced_change_does_not_open_the_gate_for_the_next_ordinary_one() {
        let mut coalescer = Coalescer::new(UPDATE_INTERVAL);
        let start = Instant::now();
        coalescer.mark("q1", true);
        let (ids, _) = coalescer.drain(start);
        assert_eq!(ids.len(), 1);
        // The forced emission consumed the window like any other one.
        coalescer.mark("q1", false);
        let (ids, _) = coalescer.drain(start + Duration::from_millis(10));
        assert!(ids.is_empty());
    }

    #[test]
    fn items_are_limited_independently_of_each_other() {
        let mut coalescer = Coalescer::new(UPDATE_INTERVAL);
        let start = Instant::now();
        coalescer.mark("q1", false);
        coalescer.mark("q2", false);
        let (ids, _) = coalescer.drain(start);
        assert_eq!(ids, vec!["q1".to_owned(), "q2".to_owned()]);

        // One item ticks late: the other one is not held back by it.
        coalescer.mark("q1", false);
        coalescer.mark("q2", false);
        let (ids, _) = coalescer.drain(start + Duration::from_millis(99));
        assert!(ids.is_empty(), "both are inside their window");
        let (ids, _) = coalescer.drain(start + Duration::from_millis(100));
        assert_eq!(ids, vec!["q1".to_owned(), "q2".to_owned()]);

        // And the reverse: only the item that was marked is due.
        coalescer.mark("q2", false);
        coalescer.mark("q1", true);
        let (ids, _) = coalescer.drain(start + Duration::from_millis(110));
        assert_eq!(ids, vec!["q1".to_owned()]);
    }

    #[test]
    fn a_pending_item_reports_when_it_will_be_due() {
        let mut coalescer = Coalescer::new(Duration::from_millis(100));
        let start = Instant::now();
        assert!(!coalescer.has_pending());
        coalescer.mark("q1", false);
        coalescer.mark("q2", false);
        let (ids, _) = coalescer.drain(start);
        assert_eq!(ids.len(), 2);
        assert!(!coalescer.has_pending());

        coalescer.mark("q1", false);
        assert!(coalescer.has_pending());
        assert_eq!(
            coalescer.next_deadline(start + Duration::from_millis(1)),
            Some(start + Duration::from_millis(100))
        );
        let (ids, _) = coalescer.drain(start + Duration::from_millis(100));
        assert_eq!(ids, vec!["q1".to_owned()]);
    }

    #[test]
    fn something_ready_now_reports_now() {
        let mut coalescer = Coalescer::new(Duration::from_millis(100));
        let start = Instant::now();
        coalescer.mark("q1", false);
        // Never sent: it is due at once, so the flusher has nothing to wait for.
        assert_eq!(coalescer.next_deadline(start), Some(start));
        let (ids, _) = coalescer.drain(start);
        assert_eq!(ids.len(), 1);
        assert_eq!(coalescer.next_deadline(start), None);
    }

    #[test]
    fn the_summary_keeps_the_same_budget_as_the_items() {
        let mut coalescer = Coalescer::new(UPDATE_INTERVAL);
        let start = Instant::now();
        // Nothing changed: the summary is not due, however long we drain.
        for tick in 0..5 {
            let (_, due) = coalescer.drain(start + Duration::from_millis(tick));
            assert!(!due);
        }
        coalescer.mark_summary();
        let (_, first) = coalescer.drain(start + Duration::from_millis(5));
        assert!(first, "a marked summary goes out");
        let (_, again) = coalescer.drain(start + Duration::from_millis(6));
        assert!(!again, "and not again inside the window");
        coalescer.mark_summary();
        let (_, inside) = coalescer.drain(start + Duration::from_millis(7));
        assert!(!inside, "a mark inside the window waits for it");
        let (_, after) = coalescer.drain(start + Duration::from_millis(105));
        assert!(after);
    }

    #[test]
    fn a_removed_item_leaves_no_trace() {
        let mut coalescer = Coalescer::new(UPDATE_INTERVAL);
        let start = Instant::now();
        coalescer.mark("gone", false);
        let _ = coalescer.drain(start);
        coalescer.mark("gone", false);
        coalescer.forget("gone");
        assert!(!coalescer.has_pending());
        let (ids, _) = coalescer.drain(start + Duration::from_millis(1));
        assert!(ids.is_empty());
    }

    #[test]
    fn only_a_state_change_is_a_milestone() {
        assert!(is_milestone(ItemState::Queued, ItemState::Downloading));
        assert!(is_milestone(ItemState::Downloading, ItemState::Done));
        assert!(!is_milestone(
            ItemState::Downloading,
            ItemState::Downloading
        ));
    }

    #[test]
    fn the_update_payload_is_camel_case_and_omits_an_unchanged_order() {
        let update = QueueUpdate {
            seq: 7,
            items: Vec::new(),
            removed: vec!["q9".into()],
            order: None,
            groups: None,
            summary: QueueSummary::default(),
            toast: Some(QueueToast::new("toast.queueIdle").with("count", 3)),
        };
        let json = serde_json::to_value(&update).expect("serialise");
        assert_eq!(json["seq"], 7);
        assert_eq!(json["removed"][0], "q9");
        assert!(json["order"].is_null());
        assert_eq!(json["toast"]["messageKey"], "toast.queueIdle");
        assert_eq!(json["toast"]["params"]["count"], "3");
        assert!(json["summary"]["parallelDownloads"].is_number());
    }
}
