//! The queue (spec §7, spec §3 `services/queue/`).
//!
//! The split is the one the spec asks for and the one that keeps the tests
//! honest:
//!
//! * [`model`] — the data: states, the immutable options snapshot, groups,
//!   counters, duplicate detection, the retry policy, the history cap. Pure.
//! * [`store`] — `queue.json`: atomic writes, `.bak` on corruption, the
//!   `Interrupted` recovery of spec §7.4.
//! * [`scheduler`] — the pure planning function: lanes, per-host cap, retry
//!   timers, "first runnable item first".
//! * [`events`] — the `queue:update` payload and the ≤ 10/s per-item
//!   coalescing of spec §7.6.
//! * [`engine`] — the async part that ties them together: the scheduler task,
//!   one task per item (running the M1 pipeline), the flusher, the actions, the
//!   persistence triggers and the idle notification.
//!
//! Nothing in this module spawns a tool by itself: the per-item task goes
//! through [`crate::services::job`], which goes through
//! [`crate::services::process`]. That is the only way a process is ever
//! started (spec §4).

pub mod engine;
pub mod events;
pub mod model;
pub mod scheduler;
pub mod store;

pub use engine::{
    AddOutcome, AddRequest, DuplicateRef, EngineConfig, IdleNotifier, ItemPatch, Outcome,
    QueueEngine, ToolProvider,
};
pub use events::{QueueSnapshot, QueueToast, QueueUpdate};
pub use model::{
    ControlSignal, CookieChoice, GroupProgress, ItemOptions, ItemProgress, ItemState, Kind, Lane,
    LaneConfig, QualityChoice, QueueGroup, QueueItem, QueueSummary,
};
pub use scheduler::Capacity;
