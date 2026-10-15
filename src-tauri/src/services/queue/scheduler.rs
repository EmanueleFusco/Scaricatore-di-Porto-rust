//! What to start next (spec §7.2, §7.3, §6.4).
//!
//! The scheduling decision is a **pure function** over a snapshot of the queue:
//! given the items in order, the free capacity of each lane, how many items are
//! already running per host and the current time, it returns the ids to start.
//! No locks, no clock, no channels — which is what makes the two rules that
//! actually matter ("2 downloads in parallel", "max 2 per host") testable
//! without spawning a single process (spec §15).
//!
//! The engine owns the side effects: marking the items running, spawning the
//! tasks and waking the loop. It calls this function again after every change.
//!
//! ## Two rules that look contradictory
//!
//! Spec §7.2 caps each **lane**; spec §6.4 caps each **host**. An item consumes
//! both, so it starts only when *both* have room. A full download lane stops
//! everything — that is the point of the lane — but a host at its cap does not
//! stop items from other hosts, which is why [`Capacity::try_take`] is asked
//! per item instead of once per pass.

use std::collections::BTreeMap;

use crate::services::queue::model::{ItemState, Lane, LaneConfig, host_key};

/// Free room in the lanes and per host (spec §7.2, §6.4).
///
/// Mutable on purpose: the planner decrements as it assigns, so one pass can
/// fill a lane without handing out more room than it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capacity {
    free: BTreeMap<Lane, usize>,
    hosts: BTreeMap<String, usize>,
    per_host_cap: usize,
}

impl Capacity {
    /// `free` is what is **left** of each lane, `running_by_host` counts the
    /// items already running on each host.
    pub fn new(
        free: BTreeMap<Lane, usize>,
        running_by_host: BTreeMap<String, usize>,
        per_host_cap: usize,
    ) -> Self {
        let mut all = BTreeMap::new();
        for lane in Lane::ALL {
            all.insert(lane, *free.get(&lane).unwrap_or(&0));
        }
        Self {
            free: all,
            hosts: running_by_host,
            per_host_cap,
        }
    }

    /// Capacity built from a lane configuration: everything is free.
    pub fn all_free(config: LaneConfig) -> Self {
        let mut free = BTreeMap::new();
        for lane in Lane::ALL {
            free.insert(lane, config.capacity(lane));
        }
        Self::new(free, BTreeMap::new(), config.per_host_cap)
    }

    pub fn free(&self, lane: Lane) -> usize {
        self.free.get(&lane).copied().unwrap_or(0)
    }

    pub fn running_on(&self, host: &str) -> usize {
        self.hosts.get(host).copied().unwrap_or(0)
    }

    /// Reserve a lane slot and a host slot for one item. `false` when either is
    /// exhausted, and nothing is consumed in that case.
    pub fn try_take(&mut self, lane: Lane, host: &str) -> bool {
        if self.free(lane) == 0 {
            return false;
        }
        if self.running_on(host) >= self.per_host_cap {
            return false;
        }
        if let Some(slot) = self.free.get_mut(&lane) {
            *slot -= 1;
        }
        *self.hosts.entry(host.to_owned()).or_insert(0) += 1;
        true
    }
}

/// A snapshot of one item, so the planner needs nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate<'a> {
    pub id: &'a str,
    pub url: &'a str,
    pub state: ItemState,
    /// Unix seconds before which the item must not start (automatic retry).
    pub retry_at: Option<u64>,
}

/// Which items the engine should start now, in queue order (spec §7.3: "the
/// scheduler always picks the first runnable item").
///
/// Returns `(id, lane)` pairs: the lane is what the engine needs room for, and
/// recomputing it here keeps the two in step.
pub fn plan<'a>(
    candidates: &[Candidate<'a>],
    capacity: &mut Capacity,
    now: u64,
) -> Vec<(String, Lane)> {
    let mut plan = Vec::new();
    for candidate in candidates {
        if !is_runnable(candidate, now) {
            continue;
        }
        // Which lane the item needs. A queued item has no lane of its own:
        // spec §3's pipeline starts at inspect/download, and that is the only
        // stage M2 runs, so it takes the download lane. M4 and M5 give their
        // stages a lane when they arrive.
        let lane = candidate.state.lane().unwrap_or(Lane::Download);
        let host = host_key(candidate.url);
        if capacity.try_take(lane, &host) {
            plan.push((candidate.id.to_owned(), lane));
        }
    }
    plan
}

/// May this item start at all, ignoring the retry timer?
///
/// **Only a queued item.** Spec §7.4 is explicit about the alternative: items
/// that were running when the app stopped come back `Interrupted` and "are shown
/// with a 'Resume' button (or auto-resume if the setting is on)". M2 has no
/// auto-resume setting, so they wait — a paused one for its own Resume, a
/// terminal one for a Retry.
pub fn is_eligible(candidate: &Candidate<'_>) -> bool {
    candidate.state == ItemState::Queued
}

/// Can this item start right now? (spec §7.3)
pub fn is_runnable(candidate: &Candidate<'_>, now: u64) -> bool {
    is_eligible(candidate) && candidate.retry_at.is_none_or(|at| at <= now)
}

/// The earliest moment at which a waiting item may start, so the engine knows
/// how long it can sleep instead of spinning (spec §7.3 automatic retry).
///
/// The items that are waiting are exactly the ones that are **not** runnable
/// yet, so eligibility is what is filtered on, not runnability.
pub fn next_wakeup(items: &[Candidate<'_>], now: u64) -> Option<u64> {
    items
        .iter()
        .filter(|candidate| is_eligible(candidate))
        .filter_map(|candidate| candidate.retry_at)
        .filter(|at| *at > now)
        .min()
}

/// How many items are running on each host (spec §6.4). Counts only items that
/// hold a lane: a queued item from the same host must not consume the budget of
/// a running one.
pub fn running_by_host<'a>(items: &[Candidate<'a>]) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for item in items.iter().filter(|item| item.state.is_active()) {
        *counts.entry(host_key(item.url)).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate<'a>(id: &'a str, url: &'a str, state: ItemState) -> Candidate<'a> {
        Candidate {
            id,
            url,
            state,
            retry_at: None,
        }
    }

    /// Borrowed forever, so a test can build candidates from generated strings.
    /// Leaking a few hundred bytes per test is the price of not making
    /// `Candidate` own its strings: the planner must borrow the queue's own
    /// strings, or it would clone the whole queue on every pass.
    fn leak(text: &str) -> &'static str {
        Box::leak(text.to_owned().into_boxed_str())
    }

    /// `count` queued items `q0..qN`, each on its **own** host: the lane tests
    /// are about lanes, and one host would cap everything at 2 first.
    fn queued_spread(count: u32) -> Vec<Candidate<'static>> {
        (0..count)
            .map(|n| {
                candidate(
                    leak(&format!("q{n}")),
                    leak(&format!("https://site{n}.example/{n}")),
                    ItemState::Queued,
                )
            })
            .collect()
    }

    /// `count` queued items `q0..qN` on **one** host.
    fn queued_same_host(count: u32, host: &str) -> Vec<Candidate<'static>> {
        (0..count)
            .map(|n| {
                candidate(
                    leak(&format!("q{n}")),
                    leak(&format!("{host}/{n}")),
                    ItemState::Queued,
                )
            })
            .collect()
    }

    fn started_ids(plan: &[(String, Lane)]) -> Vec<&str> {
        plan.iter().map(|(id, _)| id.as_str()).collect()
    }

    #[test]
    fn the_download_lane_caps_parallelism_at_two_by_default() {
        let items = queued_spread(5);
        let mut capacity = Capacity::all_free(LaneConfig::default());
        let plan = plan(&items, &mut capacity, 0);
        assert_eq!(
            started_ids(&plan),
            vec!["q0", "q1"],
            "spec §7.2 default is 2 downloads in parallel"
        );
        assert_eq!(plan[0].1, Lane::Download);
        assert_eq!(capacity.free(Lane::Download), 0);
    }

    #[test]
    fn one_parallel_download_means_one_download() {
        let items = queued_spread(3);
        let config = LaneConfig {
            download: 1,
            ..LaneConfig::default()
        };
        let mut capacity = Capacity::all_free(config);
        assert_eq!(plan(&items, &mut capacity, 0).len(), 1);
    }

    #[test]
    fn four_parallel_downloads_are_allowed_when_asked() {
        let items = queued_spread(6);
        let config = LaneConfig {
            download: 4,
            ..LaneConfig::default()
        };
        let mut capacity = Capacity::all_free(config);
        assert_eq!(plan(&items, &mut capacity, 0).len(), 4);
    }

    #[test]
    fn a_second_pass_starts_the_rest_once_the_lane_has_room_again() {
        let items = queued_spread(4);
        let mut first = Capacity::all_free(LaneConfig::default());
        assert_eq!(plan(&items, &mut first, 0).len(), 2);

        // Two are running now, so the engine hands the next capacity what is
        // left and the two that are already active are not candidates.
        let mut capacity = Capacity::all_free(LaneConfig::default());
        let started = plan(&items[2..], &mut capacity, 0);
        assert_eq!(started_ids(&started), vec!["q2", "q3"]);
    }

    #[test]
    fn the_per_host_cap_stops_the_third_item_of_one_host_but_not_another() {
        let items = vec![
            candidate("a1", "https://youtube.com/watch?v=1", ItemState::Queued),
            candidate("a2", "https://www.youtube.com/watch?v=2", ItemState::Queued),
            candidate("b1", "https://vimeo.com/1", ItemState::Queued),
            candidate("a3", "https://youtube.com/watch?v=3", ItemState::Queued),
        ];
        // A wide lane, so what is being measured is the host cap and not the
        // lane: with the default lane of 2 the third item would be stopped by
        // the lane anyway and the test would prove nothing.
        let config = LaneConfig {
            download: 4,
            ..LaneConfig::default()
        };
        let mut capacity = Capacity::all_free(config);
        let plan = plan(&items, &mut capacity, 0);
        assert_eq!(
            started_ids(&plan),
            vec!["a1", "a2", "b1"],
            "www. counts as the same host; the other site is unaffected"
        );
    }

    #[test]
    fn a_queued_item_takes_the_download_lane() {
        // The only lane M2 runs: spec §3's pipeline starts at download.
        let items = queued_spread(3);
        let mut capacity = Capacity::all_free(LaneConfig::default());
        let plan = plan(&items, &mut capacity, 0);
        assert!(plan.iter().all(|(_, lane)| *lane == Lane::Download));
    }

    #[test]
    fn a_higher_per_host_cap_allows_more_of_the_same_host() {
        let items = queued_same_host(3, "https://youtube.com");
        let config = LaneConfig {
            download: 3,
            per_host_cap: 3,
            ..LaneConfig::default()
        };
        let mut capacity = Capacity::all_free(config);
        assert_eq!(plan(&items, &mut capacity, 0).len(), 3);
    }

    #[test]
    fn the_host_cap_applies_even_with_a_wide_lane() {
        // The two caps are independent: a lane of 4 does not override a host cap
        // of 2, because they are two different resources.
        let items = queued_same_host(4, "https://youtube.com");
        let config = LaneConfig {
            download: 4,
            per_host_cap: 2,
            ..LaneConfig::default()
        };
        let mut capacity = Capacity::all_free(config);
        assert_eq!(plan(&items, &mut capacity, 0).len(), 2);
    }

    #[test]
    fn items_running_on_a_host_already_consume_its_budget() {
        let items = vec![candidate("a3", "https://youtube.com/3", ItemState::Queued)];
        let mut capacity = Capacity::all_free(LaneConfig::default());
        capacity.hosts.insert("youtube.com".to_owned(), 2);
        assert!(plan(&items, &mut capacity, 0).is_empty());
    }

    #[test]
    fn only_a_queued_item_starts_by_itself() {
        assert!(is_runnable(
            &candidate("q", "https://a.example/1", ItemState::Queued),
            0
        ));
        for state in [
            ItemState::Interrupted,
            ItemState::Downloading,
            ItemState::PostProcessing,
            ItemState::Encoding,
            ItemState::Done,
            ItemState::Failed,
            ItemState::Cancelled,
            ItemState::Paused,
        ] {
            assert!(
                !is_runnable(&candidate("q", "https://a.example/1", state), 0),
                "{state:?} must not start on its own"
            );
        }
    }

    #[test]
    fn an_item_waiting_for_its_retry_timer_does_not_start_early() {
        let mut waiting = candidate("q1", "https://a.example/1", ItemState::Queued);
        waiting.retry_at = Some(1_000);
        assert!(!is_runnable(&waiting, 999));
        assert!(is_runnable(&waiting, 1_000), "the timer fires on time");

        let mut capacity = Capacity::all_free(LaneConfig::default());
        assert!(plan(std::slice::from_ref(&waiting), &mut capacity, 999).is_empty());
        assert_eq!(
            plan(std::slice::from_ref(&waiting), &mut capacity, 1_000).len(),
            1
        );
    }

    #[test]
    fn an_interrupted_item_waits_for_its_resume_button() {
        // Spec §7.4: it comes back with a Resume button, not by running again
        // behind the user's back.
        let item = candidate("q1", "https://a.example/1", ItemState::Interrupted);
        let mut capacity = Capacity::all_free(LaneConfig::default());
        assert!(!is_runnable(&item, 0));
        assert!(plan(std::slice::from_ref(&item), &mut capacity, 0).is_empty());
        assert_eq!(
            capacity.free(Lane::Download),
            LaneConfig::default().download,
            "and it must not have consumed a slot"
        );
    }

    #[test]
    fn the_wakeup_is_the_earliest_pending_timer_and_nothing_when_all_is_ready() {
        let mut early = candidate("q1", "https://a.example/1", ItemState::Queued);
        early.retry_at = Some(500);
        let mut late = candidate("q2", "https://a.example/2", ItemState::Queued);
        late.retry_at = Some(900);
        let items = vec![early.clone(), late.clone()];

        assert_eq!(next_wakeup(&items, 0), Some(500));
        assert_eq!(next_wakeup(&items, 500), Some(900));
        assert_eq!(next_wakeup(&items, 900), None, "nothing left to wait for");

        // An item with no timer must not keep the engine awake.
        let ready = vec![candidate("q3", "https://a.example/3", ItemState::Queued)];
        assert_eq!(next_wakeup(&ready, 0), None);
        assert!(
            is_runnable(&ready[0], 0),
            "and it is runnable right now, so there is no timer to wait for"
        );
    }

    #[test]
    fn only_running_items_count_towards_the_host_budget() {
        let items = vec![
            candidate("q1", "https://youtube.com/1", ItemState::Queued),
            candidate("q2", "https://youtube.com/2", ItemState::Downloading),
            candidate("q3", "https://vimeo.com/1", ItemState::PostProcessing),
            candidate("q4", "https://youtube.com/3", ItemState::Done),
        ];
        let counts = running_by_host(&items);
        assert_eq!(counts.get("youtube.com"), Some(&1));
        assert_eq!(counts.get("vimeo.com"), Some(&1));
        assert!(!counts.contains_key("example.com"));
    }

    #[test]
    fn take_is_all_or_nothing() {
        let mut capacity = Capacity::all_free(LaneConfig {
            per_host_cap: 1,
            ..LaneConfig::default()
        });
        assert!(capacity.try_take(Lane::Download, "youtube.com"));
        assert!(!capacity.try_take(Lane::Download, "youtube.com"));
        assert_eq!(
            capacity.free(Lane::Download),
            LaneConfig::default().download - 1,
            "a refused item must not consume the host slot either"
        );
        assert!(capacity.try_take(Lane::Download, "vimeo.com"));
    }
}
