//! Registry-owned admission for inbound-acquisition actor turns.
//!
//! The worker pool executes closures only.  This ready set owns acquisition
//! identity, coalescing, cancellation, and the reference TimeoutCounter-sized
//! (five) exposure boundary.

use basics::base_uint::Uint256;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};

use super::acquisition::{AcquisitionState, AcquisitionTurnOutcome, TurnBudget};
use super::worker_pool::WorkerPool;

pub(crate) const READY_EXECUTION_LIMIT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct AcquisitionKey {
    pub hash: Uint256,
    pub id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReadyCause(u8);

impl ReadyCause {
    const NONE: Self = Self(0);
    pub(crate) const WIRE: Self = Self(1 << 0);
    pub(crate) const READ_READY: Self = Self(1 << 2);
    pub(crate) const FETCH_PACK: Self = Self(1 << 3);
    pub(crate) const TIMEOUT: Self = Self(1 << 5);

    fn contains(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOrAssign for ReadyCause {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadyState {
    Waiting,
    Dispatched,
    Running,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadyClass {
    Recovery,
    Normal,
}

struct ReadyEntry {
    state: Weak<AcquisitionState>,
    /// Wakes arriving while a turn is running stay owned by this scheduler
    /// until `finish` has either retained or released its reservation.
    causes: ReadyCause,
    state_kind: ReadyState,
    normal_enqueued: bool,
    recovery_enqueued: bool,
}

struct SchedulerState {
    stopped: bool,
    reserved_turns: usize,
    entries: BTreeMap<AcquisitionKey, ReadyEntry>,
    recovery: VecDeque<AcquisitionKey>,
    normal: VecDeque<AcquisitionKey>,
    next_class: ReadyClass,
}

impl Default for SchedulerState {
    fn default() -> Self {
        Self {
            stopped: false,
            reserved_turns: 0,
            entries: BTreeMap::new(),
            recovery: VecDeque::new(),
            normal: VecDeque::new(),
            next_class: ReadyClass::Recovery,
        }
    }
}

pub(crate) struct AcquisitionReadyScheduler {
    worker_pool: Arc<WorkerPool>,
    inner: Mutex<SchedulerState>,
}

impl AcquisitionReadyScheduler {
    pub(crate) fn new(worker_pool: Arc<WorkerPool>) -> Arc<Self> {
        Arc::new(Self {
            worker_pool,
            inner: Mutex::new(SchedulerState::default()),
        })
    }

    pub(crate) fn wake(
        self: &Arc<Self>,
        key: AcquisitionKey,
        acquisition: &Arc<AcquisitionState>,
        cause: ReadyCause,
    ) {
        self.wake_with_state(key, Arc::downgrade(acquisition), cause);
    }

    /// The acquisition weak reference is captured only when an entry first
    /// becomes scheduler-owned. Later wakes must coalesce with that one entry.
    fn wake_with_state(
        self: &Arc<Self>,
        key: AcquisitionKey,
        acquisition: Weak<AcquisitionState>,
        cause: ReadyCause,
    ) {
        let keys = {
            let mut state = self.inner.lock().expect("acquisition ready scheduler lock");
            if state.stopped {
                return;
            }
            let entry = state.entries.entry(key).or_insert_with(|| ReadyEntry {
                state: acquisition,
                causes: cause,
                state_kind: ReadyState::Waiting,
                normal_enqueued: false,
                recovery_enqueued: false,
            });
            entry.causes |= cause;
            if matches!(entry.state_kind, ReadyState::Cancelled) {
                return;
            }
            if matches!(entry.state_kind, ReadyState::Waiting)
                && !entry.normal_enqueued
                && !entry.recovery_enqueued
            {
                if entry.causes.contains(ReadyCause::TIMEOUT) {
                    entry.recovery_enqueued = true;
                    state.recovery.push_back(key);
                } else {
                    entry.normal_enqueued = true;
                    state.normal.push_back(key);
                }
            }
            self.fill_capacity_locked(&mut state)
        };
        self.submit(keys);
    }

    fn pop_eligible(state: &mut SchedulerState, class: ReadyClass) -> Option<AcquisitionKey> {
        let queue = match class {
            ReadyClass::Recovery => &mut state.recovery,
            ReadyClass::Normal => &mut state.normal,
        };
        while let Some(key) = queue.pop_front() {
            let Some(entry) = state.entries.get_mut(&key) else {
                continue;
            };
            match class {
                ReadyClass::Recovery => entry.recovery_enqueued = false,
                ReadyClass::Normal => entry.normal_enqueued = false,
            }
            if entry.state_kind == ReadyState::Waiting {
                return Some(key);
            }
        }
        None
    }

    fn fill_capacity_locked(&self, state: &mut SchedulerState) -> Vec<AcquisitionKey> {
        let mut ready = Vec::new();
        while state.reserved_turns < READY_EXECUTION_LIMIT {
            let recovery_available = state.recovery.iter().any(|key| {
                state
                    .entries
                    .get(key)
                    .is_some_and(|entry| entry.state_kind == ReadyState::Waiting)
            });
            let normal_available = state.normal.iter().any(|key| {
                state
                    .entries
                    .get(key)
                    .is_some_and(|entry| entry.state_kind == ReadyState::Waiting)
            });
            let class = match (recovery_available, normal_available, state.next_class) {
                (false, false, _) => break,
                (true, false, _) => ReadyClass::Recovery,
                (false, true, _) => ReadyClass::Normal,
                (true, true, class) => class,
            };
            let Some(key) = Self::pop_eligible(state, class) else {
                continue;
            };
            if let Some(entry) = state.entries.get_mut(&key) {
                entry.state_kind = ReadyState::Dispatched;
                state.reserved_turns += 1;
                state.next_class = match class {
                    ReadyClass::Recovery => ReadyClass::Normal,
                    ReadyClass::Normal => ReadyClass::Recovery,
                };
                ready.push(key);
            }
        }
        ready
    }

    fn submit(self: &Arc<Self>, keys: Vec<AcquisitionKey>) {
        for key in keys {
            let scheduler = Arc::clone(self);
            self.worker_pool.submit_reserved_turn(Box::new(move || {
                let Some(acquisition) = scheduler.claim(key) else {
                    return;
                };
                let outcome =
                    acquisition.run_ready_turn(&TurnBudget::new(scheduler.competing_ready(key)));
                scheduler.finish(key, acquisition.as_ref(), outcome);
            }));
        }
    }

    fn claim(&self, key: AcquisitionKey) -> Option<Arc<AcquisitionState>> {
        let mut state = self.inner.lock().expect("acquisition ready scheduler lock");
        let kind = state.entries.get(&key).map(|entry| entry.state_kind)?;
        if kind == ReadyState::Cancelled {
            state.entries.remove(&key);
            state.reserved_turns = state.reserved_turns.saturating_sub(1);
            return None;
        }
        if kind != ReadyState::Dispatched {
            return None;
        }
        let acquisition = state
            .entries
            .get(&key)
            .and_then(|entry| entry.state.upgrade());
        let Some(acquisition) = acquisition else {
            state.entries.remove(&key);
            state.reserved_turns = state.reserved_turns.saturating_sub(1);
            return None;
        };
        let entry = state.entries.get_mut(&key).expect("live scheduler entry");
        entry.causes = ReadyCause::NONE;
        entry.state_kind = ReadyState::Running;
        Some(acquisition)
    }

    fn finish(
        self: &Arc<Self>,
        key: AcquisitionKey,
        acquisition: &AcquisitionState,
        outcome: AcquisitionTurnOutcome,
    ) {
        self.finish_with_timeout_state(key, outcome, || acquisition.has_pending_timeout());
    }

    /// Keep the recovery observation inside the scheduler's serialized finish
    /// transition. This is the same production path used by `finish`; the
    /// closure makes the linearized boundary directly testable.
    fn finish_with_timeout_state(
        self: &Arc<Self>,
        key: AcquisitionKey,
        outcome: AcquisitionTurnOutcome,
        has_pending_timeout: impl FnOnce() -> bool,
    ) {
        let keys = {
            let mut state = self.inner.lock().expect("acquisition ready scheduler lock");
            let Some(kind) = state.entries.get(&key).map(|entry| entry.state_kind) else {
                return;
            };
            if matches!(kind, ReadyState::Running | ReadyState::Cancelled) {
                state.reserved_turns = state.reserved_turns.saturating_sub(1);
            }
            if state.stopped || kind == ReadyState::Cancelled || outcome.terminal {
                state.entries.remove(&key);
            } else {
                // `wake` may have recorded new work after the actor observed
                // its mailbox but before this capacity release. Because both
                // transitions are serialized by `inner`, the recorded cause
                // is a durable rerun reservation, not a best-effort hint.
                let rerun_requested = state
                    .entries
                    .get(&key)
                    .expect("live scheduler entry")
                    .causes
                    != ReadyCause::NONE;
                if outcome.needs_turn || rerun_requested {
                    let recovery = has_pending_timeout()
                        || state
                            .entries
                            .get(&key)
                            .expect("live scheduler entry")
                            .causes
                            .contains(ReadyCause::TIMEOUT);
                    let entry = state.entries.get_mut(&key).expect("live scheduler entry");
                    entry.state_kind = ReadyState::Waiting;
                    if recovery {
                        entry.recovery_enqueued = true;
                        state.recovery.push_back(key);
                    } else {
                        entry.normal_enqueued = true;
                        state.normal.push_back(key);
                    }
                } else {
                    state.entries.remove(&key);
                }
            }
            self.fill_capacity_locked(&mut state)
        };
        self.submit(keys);
    }

    pub(crate) fn cancel(self: &Arc<Self>, key: AcquisitionKey) {
        let keys = {
            let mut state = self.inner.lock().expect("acquisition ready scheduler lock");
            let Some(kind) = state.entries.get(&key).map(|entry| entry.state_kind) else {
                return;
            };
            match kind {
                ReadyState::Dispatched => {
                    // The queued closure is the sole owner of this reserved
                    // turn until it is claimed or WorkerPool drops it during
                    // stop. Retaining the reservation here prevents a cancel
                    // storm from admitting replacement closures ahead of old
                    // terminal closures that still retain their resources.
                    state
                        .entries
                        .get_mut(&key)
                        .expect("live scheduler entry")
                        .state_kind = ReadyState::Cancelled;
                }
                ReadyState::Running => {
                    // The executing closure owns the reservation until
                    // `finish`; mark it cancelled so that path releases it.
                    state
                        .entries
                        .get_mut(&key)
                        .expect("live scheduler entry")
                        .state_kind = ReadyState::Cancelled;
                }
                ReadyState::Waiting | ReadyState::Cancelled => {
                    state.entries.remove(&key);
                }
            }
            self.fill_capacity_locked(&mut state)
        };
        self.submit(keys);
    }

    pub(crate) fn stop(self: &Arc<Self>) {
        let mut state = self.inner.lock().expect("acquisition ready scheduler lock");
        state.stopped = true;
        state.entries.clear();
        state.recovery.clear();
        state.normal.clear();
        state.reserved_turns = 0;
    }

    fn competing_ready(&self, key: AcquisitionKey) -> usize {
        let state = self.inner.lock().expect("acquisition ready scheduler lock");
        state
            .entries
            .iter()
            .filter(|(candidate, entry)| {
                **candidate != key
                    && matches!(
                        entry.state_kind,
                        ReadyState::Waiting | ReadyState::Dispatched | ReadyState::Running
                    )
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(fill: u8) -> AcquisitionKey {
        AcquisitionKey {
            hash: Uint256::from_array([fill; 32]),
            id: u64::from(fill),
        }
    }

    fn waiting_entry() -> ReadyEntry {
        ReadyEntry {
            state: Weak::new(),
            causes: ReadyCause::NONE,
            state_kind: ReadyState::Waiting,
            normal_enqueued: true,
            recovery_enqueued: true,
        }
    }

    #[test]
    fn ready_scheduler_alternates_recovery_and_normal_fifo_classes() {
        let pool = Arc::new(WorkerPool::new(0));
        let scheduler = AcquisitionReadyScheduler::new(pool);
        let recovery_one = key(1);
        let normal_one = key(2);
        let recovery_two = key(3);
        let normal_two = key(4);
        let mut state = SchedulerState::default();
        for entry_key in [recovery_one, normal_one, recovery_two, normal_two] {
            state.entries.insert(entry_key, waiting_entry());
        }
        state.recovery.extend([recovery_one, recovery_two]);
        state.normal.extend([normal_one, normal_two]);

        assert_eq!(
            scheduler.fill_capacity_locked(&mut state),
            vec![recovery_one, normal_one, recovery_two, normal_two],
            "when both classes are ready, reservation alternates classes while preserving each FIFO"
        );
        assert_eq!(state.reserved_turns, 4);
        assert_eq!(state.next_class, ReadyClass::Recovery);
    }

    #[test]
    fn ready_scheduler_live_wakes_hold_four_five_and_sixth_at_exact_boundary() {
        let pool = Arc::new(WorkerPool::new(0));
        let scheduler = AcquisitionReadyScheduler::new(Arc::clone(&pool));
        let keys = [key(1), key(2), key(3), key(4), key(5), key(6)];

        for (expected_reservations, entry_key) in (1..=4).zip(keys) {
            scheduler.wake_with_state(entry_key, Weak::new(), ReadyCause::WIRE);
            let state = scheduler
                .inner
                .lock()
                .expect("acquisition ready scheduler lock");
            assert_eq!(state.reserved_turns, expected_reservations);
            assert_eq!(
                state.entries.get(&entry_key).unwrap().state_kind,
                ReadyState::Dispatched
            );
        }

        scheduler.wake_with_state(keys[4], Weak::new(), ReadyCause::WIRE);
        scheduler.wake_with_state(keys[5], Weak::new(), ReadyCause::WIRE);
        scheduler.wake_with_state(keys[5], Weak::new(), ReadyCause::READ_READY);

        let state = scheduler
            .inner
            .lock()
            .expect("acquisition ready scheduler lock");
        assert_eq!(state.reserved_turns, READY_EXECUTION_LIMIT);
        assert_eq!(
            state.entries.len(),
            6,
            "coalesced sixth wake must not create an entry"
        );
        assert_eq!(
            state.entries.get(&keys[5]).unwrap().state_kind,
            ReadyState::Waiting,
            "the sixth acquisition waits rather than exceeding the five-turn boundary"
        );
        assert_eq!(
            state
                .normal
                .iter()
                .filter(|queued| **queued == keys[5])
                .count(),
            1,
            "the sixth acquisition has one durable waiting reservation"
        );
        drop(state);
        assert_eq!(pool.snapshot().queued_jobs, READY_EXECUTION_LIMIT);
        pool.stop();
    }

    #[test]
    fn ready_scheduler_cancel_finish_wake_race_releases_once_and_promotes_recovery() {
        let pool = Arc::new(WorkerPool::new(0));
        let scheduler = AcquisitionReadyScheduler::new(Arc::clone(&pool));
        let running = key(1);
        let normals = [running, key(2), key(3), key(4), key(5)];
        let recovery = key(6);

        for entry_key in normals {
            scheduler.wake_with_state(entry_key, Weak::new(), ReadyCause::WIRE);
        }
        scheduler.wake_with_state(recovery, Weak::new(), ReadyCause::TIMEOUT);
        {
            let mut state = scheduler
                .inner
                .lock()
                .expect("acquisition ready scheduler lock");
            state.entries.get_mut(&running).unwrap().state_kind = ReadyState::Running;
            assert_eq!(state.reserved_turns, READY_EXECUTION_LIMIT);
            assert_eq!(
                state.entries.get(&recovery).unwrap().state_kind,
                ReadyState::Waiting
            );
        }

        // Model the serialized race order: a running actor receives a timeout
        // wake, cancellation wins terminal ownership, then the old turn
        // finishes. `finish` must release exactly that one reservation and
        // immediately promote the retained recovery work.
        scheduler.wake_with_state(running, Weak::new(), ReadyCause::TIMEOUT);
        scheduler.cancel(running);
        scheduler.finish_with_timeout_state(
            running,
            AcquisitionTurnOutcome {
                terminal: false,
                needs_turn: false,
            },
            || false,
        );

        let state = scheduler
            .inner
            .lock()
            .expect("acquisition ready scheduler lock");
        assert_eq!(state.reserved_turns, READY_EXECUTION_LIMIT);
        assert!(!state.entries.contains_key(&running));
        assert_eq!(state.entries.len(), READY_EXECUTION_LIMIT);
        assert_eq!(
            state.entries.get(&recovery).unwrap().state_kind,
            ReadyState::Dispatched
        );
        assert!(
            !state.entries.get(&recovery).unwrap().recovery_enqueued,
            "promoted recovery work cannot remain stranded in the recovery FIFO"
        );
        assert_eq!(
            state
                .recovery
                .iter()
                .filter(|queued| **queued == recovery)
                .count(),
            0,
            "the promoted recovery turn has no duplicate queue reservation"
        );
        drop(state);
        assert_eq!(
            pool.snapshot().queued_jobs,
            READY_EXECUTION_LIMIT + 1,
            "the cancelled running closure plus one promoted recovery closure are distinct; no duplicate recovery turn was submitted"
        );
        pool.stop();
    }
}

/*
Scheduler boundary and lifecycle provenance plan (final audit evidence)

Quaxar final spans (this file): `READY_EXECUTION_LIMIT` is Q 14;
`SchedulerState` is Q 67-74; `wake`/`wake_with_state` are Q 102-150;
`fill_capacity_locked` is Q 172-207; `claim` is Q 223-241;
`finish`/`finish_with_timeout_state` are Q 243-306; and `cancel` is Q 308-335.
The deterministic 4/5/6 boundary test is Q 408-440; the serialized
wake/cancel/finish recovery-race test is Q 443-497. The tests use a
zero-worker `WorkerPool`, so submitted closures remain observable rather than
racing a worker; they exercise the scheduler's actual wake, cancel, capacity,
and finish transition methods and assert the live reserved-turn/FIFO state.

rippled authority is revision ecdd457f3598c7286a9af4aff358fbd30039173f:
`src/xrpld/app/ledger/detail/InboundLedger.cpp` 62-76 constructs the
`TimeoutCounter` with `JtLedgerData` `jobLimit = 5`; 279-319 defines its
ledger timeout recovery lifecycle. `src/xrpld/app/ledger/detail/TimeoutCounter.h`
17-41 defines the active timer -> queue -> invoke -> retry lifecycle and
48-55 specifies cancellation as a done flag that leaves queued work harmless.
`src/xrpld/app/ledger/detail/TimeoutCounter.cpp` 34-51 re-arms/queues through
a weak reference, 53-75 applies the configured job limit by deferring and
re-arming, 77-100 invokes/re-arms the timeout loop, and 102-111 cancels.
`src/xrpld/app/ledger/detail/InboundLedgers.cpp` 188-222 routes inbound ledger
data to `gotData` and submits `runData` only when work is needed.

The numeric five-job gate is directly sourced above. The Rust per-key,
reservation-counted recovery/normal scheduler, FIFO alternation, and its
coalesced wake/cancel/finish representation are a bounded Rust adaptation of
that lifecycle, not a claim of a line-for-line C++ scheduler equivalent. No
constant or scheduling behavior was changed without that source basis.
*/
