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
        let keys = {
            let mut state = self.inner.lock().expect("acquisition ready scheduler lock");
            if state.stopped {
                return;
            }
            let entry = state.entries.entry(key).or_insert_with(|| ReadyEntry {
                state: Arc::downgrade(acquisition),
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
        let acquisition = {
            let entry = state.entries.get_mut(&key)?;
            if entry.state_kind != ReadyState::Dispatched {
                return None;
            }
            entry.state.upgrade()
        };
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
                // its mailbox but before this capacity release.  Because both
                // transitions are serialized by `inner`, the recorded cause
                // is a durable rerun reservation, not a best-effort hint.
                let rerun_requested = state
                    .entries
                    .get(&key)
                    .expect("live scheduler entry")
                    .causes
                    != ReadyCause::NONE;
                if outcome.needs_turn || rerun_requested {
                    let recovery = acquisition.has_pending_timeout()
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
                    state.entries.remove(&key);
                    state.reserved_turns = state.reserved_turns.saturating_sub(1);
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
