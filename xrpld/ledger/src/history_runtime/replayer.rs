//! `LedgerReplayer` owner port above the landed replay task and acquire
//! sub-owners.

use crate::{
    InboundLedgerReason, Ledger, LedgerConfig, LedgerDeltaAcquire, LedgerHeader, LedgerReplayTask,
    LedgerReplayTaskParameter, SkipListAcquire,
};
use basics::base_uint::Uint256;
use overlay::PeerSetBuilder;
use protocol::STTx;
use shamap::item::SHAMapItem;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

pub const REPLAY_MAX_TASKS: usize = 10;
pub const REPLAY_MAX_TASK_SIZE: u32 = 256;

/// Bounded operational state for the live replay scheduler. It exposes only
/// aggregate state; timer/task controls remain owned by `LedgerReplayer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayTimerStatus {
    pub active_tasks: usize,
    pub active_skip_lists: usize,
    pub active_deltas: usize,
    pub skip_list_fallbacks: usize,
    pub delta_fallbacks: usize,
    pub skip_list_timer_ticks: u64,
    pub delta_timer_ticks: u64,
    pub task_timer_ticks: u64,
}

struct ReplayTaskDeadline {
    task: Weak<Mutex<LedgerReplayTask>>,
    due_at: Instant,
}

pub struct LedgerReplayer {
    tasks: Vec<Arc<Mutex<LedgerReplayTask>>>,
    deltas: BTreeMap<Uint256, Weak<Mutex<LedgerDeltaAcquire>>>,
    skip_lists: BTreeMap<Uint256, Weak<Mutex<SkipListAcquire>>>,
    peer_set_builder: Arc<dyn PeerSetBuilder>,
    // The app runtime polls this owner, but each owner retains its own due
    // time. This preserves the reference 250 ms subtask / 500 ms parent-task
    // cadence and each subtask's one-second fallback interval.
    skip_list_timers: BTreeMap<Uint256, Instant>,
    delta_timers: BTreeMap<Uint256, Instant>,
    task_timers: Vec<ReplayTaskDeadline>,
    skip_list_timer_ticks: u64,
    delta_timer_ticks: u64,
    task_timer_ticks: u64,
    stopping: bool,
}

impl LedgerReplayer {
    pub fn new(peer_set_builder: Arc<dyn PeerSetBuilder>) -> Self {
        Self {
            tasks: Vec::new(),
            deltas: BTreeMap::new(),
            skip_lists: BTreeMap::new(),
            peer_set_builder,
            skip_list_timers: BTreeMap::new(),
            delta_timers: BTreeMap::new(),
            task_timers: Vec::new(),
            skip_list_timer_ticks: 0,
            delta_timer_ticks: 0,
            task_timer_ticks: 0,
            stopping: false,
        }
    }

    /// Replaces the registry's bootstrap-only peer set builder once the live
    /// overlay exists. Newly created replay acquisitions then discover active
    /// peers at request time, as rippled does through `makePeerSetBuilder`.
    pub fn set_peer_set_builder(&mut self, peer_set_builder: Arc<dyn PeerSetBuilder>) {
        self.peer_set_builder = peer_set_builder;
    }

    pub fn replay(
        &mut self,
        reason: InboundLedgerReason,
        finish_hash: Uint256,
        total_ledgers: u32,
    ) -> Option<Arc<Mutex<LedgerReplayTask>>> {
        assert!(
            finish_hash.is_non_zero() && total_ledgers > 0 && total_ledgers <= REPLAY_MAX_TASK_SIZE,
            "xrpl::LedgerReplayer::replay : valid inputs"
        );

        if self.stopping || self.tasks.len() >= REPLAY_MAX_TASKS {
            return None;
        }

        let parameter = LedgerReplayTaskParameter::new(reason, finish_hash, total_ledgers);
        for existing in &self.tasks {
            if parameter.can_merge_into(existing.lock().expect("task lock").parameter()) {
                return None;
            }
        }

        let skip_list = self
            .skip_lists
            .get(&finish_hash)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let created = Arc::new(Mutex::new(SkipListAcquire::new(
                    finish_hash,
                    self.peer_set_builder.build(),
                )));
                self.skip_lists
                    .insert(finish_hash, Arc::downgrade(&created));
                created
            });

        let task = Arc::new(Mutex::new(LedgerReplayTask::new(
            parameter,
            Arc::clone(&skip_list),
        )));
        self.tasks.push(Arc::clone(&task));

        // A caller with initialization dependencies (`replay_and_init` or
        // `got_skip_list`) owns the first trigger for newly allocated deltas.
        // Do not create them here: this synchronous-data branch has no lookup
        // or inbound-acquisition callbacks with which to call `init`.
        if let Some(data) = skip_list
            .lock()
            .expect("skip list lock")
            .get_data()
            .cloned()
        {
            let _ = self.update_task_from_skip_list(&task, finish_hash, data);
        }

        Some(task)
    }

    /// Start a replay task and immediately trigger the exact first skip-list
    /// acquisition. This follows `LedgerReplayer.cpp::replay`: initialize a
    /// newly allocated skip-list before the task, then arm both independent
    /// timeout owners.
    pub fn replay_and_init<LookupLedger, FallbackAcquire>(
        &mut self,
        reason: InboundLedgerReason,
        finish_hash: Uint256,
        total_ledgers: u32,
        num_peers: usize,
        mut lookup_ledger: LookupLedger,
        mut fallback_acquire: FallbackAcquire,
    ) -> Option<Arc<Mutex<LedgerReplayTask>>>
    where
        LookupLedger: FnMut(Uint256) -> Option<Arc<Ledger>>,
        FallbackAcquire: FnMut(Uint256, u32, InboundLedgerReason),
    {
        let had_live_skip_list = self
            .skip_lists
            .get(&finish_hash)
            .and_then(Weak::upgrade)
            .is_some();
        let task = self.replay(reason, finish_hash, total_ledgers)?;
        let mut activated_from_skip_list = false;

        if !had_live_skip_list
            && let Some(skip_list) = self.skip_lists.get(&finish_hash).and_then(Weak::upgrade)
        {
            // `SkipListAcquire.cpp::trigger` owns the fallback decision. A
            // local miss alone is not an inbound request; timed encounters
            // with replay-incompatible peers cause the fallback.
            skip_list.lock().expect("skip list lock").init(
                num_peers,
                &mut |hash| lookup_ledger(hash),
                &mut fallback_acquire,
            );
            self.arm_skip_list_timer(finish_hash, Instant::now());

            if let Some(data) = skip_list
                .lock()
                .expect("skip list lock")
                .get_data()
                .cloned()
            {
                self.activate_skip_list_data(
                    finish_hash,
                    data,
                    num_peers,
                    &mut lookup_ledger,
                    &mut fallback_acquire,
                );
                activated_from_skip_list = true;
            }
        }

        if !activated_from_skip_list {
            self.trigger_task_and_init_deltas(
                &task,
                num_peers,
                &mut lookup_ledger,
                &mut fallback_acquire,
            );
        }
        self.arm_task_timer(&task, Instant::now());
        Some(task)
    }

    /// Mirrors `LedgerReplayTask::init`'s immediate parent trigger and
    /// `LedgerReplayer::createDeltas`' `newDelta->init(1)` branch.
    fn trigger_task_and_init_deltas<LookupLedger, FallbackAcquire>(
        &mut self,
        task: &Arc<Mutex<LedgerReplayTask>>,
        num_peers: usize,
        lookup_ledger: &mut LookupLedger,
        fallback_acquire: &mut FallbackAcquire,
    ) where
        LookupLedger: FnMut(Uint256) -> Option<Arc<Ledger>>,
        FallbackAcquire: FnMut(Uint256, u32, InboundLedgerReason),
    {
        let parameter = task.lock().expect("task lock").parameter().clone();
        if parameter.full && lookup_ledger(parameter.start_hash).is_none() {
            // LedgerReplayTask.cpp::trigger requests a missing parent through
            // normal inbound acquisition before its next parent-task timeout.
            fallback_acquire(
                parameter.start_hash,
                parameter.start_seq,
                InboundLedgerReason::Generic,
            );
        }

        for delta in self.create_deltas(task) {
            let hash = delta.lock().expect("delta lock").hash();
            delta.lock().expect("delta lock").init(
                num_peers,
                &mut |hash| lookup_ledger(hash),
                fallback_acquire,
            );
            self.arm_delta_timer(hash, Instant::now());
        }
    }

    fn update_task_from_skip_list(
        &mut self,
        task: &Arc<Mutex<LedgerReplayTask>>,
        finish_hash: Uint256,
        data: crate::SkipListData,
    ) -> bool {
        let mut task_ref = task.lock().expect("task lock");
        task_ref.update_skip_list(finish_hash, data.ledger_seq, &data.skip_list)
    }

    fn activate_skip_list_data<LookupLedger, FallbackAcquire>(
        &mut self,
        finish_hash: Uint256,
        data: crate::SkipListData,
        num_peers: usize,
        lookup_ledger: &mut LookupLedger,
        fallback_acquire: &mut FallbackAcquire,
    ) where
        LookupLedger: FnMut(Uint256) -> Option<Arc<Ledger>>,
        FallbackAcquire: FnMut(Uint256, u32, InboundLedgerReason),
    {
        for task in self.tasks.clone() {
            if task.lock().expect("task lock").parameter().finish_hash == finish_hash
                && self.update_task_from_skip_list(&task, finish_hash, data.clone())
            {
                self.trigger_task_and_init_deltas(
                    &task,
                    num_peers,
                    lookup_ledger,
                    fallback_acquire,
                );
                self.arm_task_timer(&task, Instant::now());
            }
        }
    }

    /// Creates all delta owners and returns only newly allocated deltas. A
    /// caller that owns an initial trigger must invoke it only for these fresh
    /// owners; shared deltas are already live.
    pub fn create_deltas(
        &mut self,
        task: &Arc<Mutex<LedgerReplayTask>>,
    ) -> Vec<Arc<Mutex<LedgerDeltaAcquire>>> {
        let parameter = task.lock().expect("task lock").parameter().clone();
        if parameter.total_ledgers <= 1 {
            return Vec::new();
        }

        let Some(mut index) = parameter
            .skip_list
            .iter()
            .position(|hash| *hash == parameter.start_hash)
        else {
            return Vec::new();
        };
        index += 1;
        if index >= parameter.skip_list.len() {
            return Vec::new();
        }

        let mut new_deltas = Vec::new();
        for seq in parameter.start_seq + 1..=parameter.finish_seq {
            let Some(hash) = parameter.skip_list.get(index).copied() else {
                break;
            };
            index += 1;

            let existing = self.deltas.get(&hash).and_then(Weak::upgrade);
            let (delta, is_new) = match existing {
                Some(delta) => (delta, false),
                None => {
                    let created = Arc::new(Mutex::new(LedgerDeltaAcquire::new(
                        hash,
                        seq,
                        self.peer_set_builder.build(),
                    )));
                    self.deltas.insert(hash, Arc::downgrade(&created));
                    (created, true)
                }
            };

            task.lock()
                .expect("task lock")
                .add_delta(Arc::clone(&delta));
            if is_new {
                new_deltas.push(delta);
            }
        }
        new_deltas
    }

    /// Deliver a verified skip-list proof and start every newly created delta.
    pub fn got_skip_list<LookupLedger, FallbackAcquire>(
        &mut self,
        info: LedgerHeader,
        item: &SHAMapItem,
        num_peers: usize,
        mut lookup_ledger: LookupLedger,
        mut fallback_acquire: FallbackAcquire,
    ) where
        LookupLedger: FnMut(Uint256) -> Option<Arc<Ledger>>,
        FallbackAcquire: FnMut(Uint256, u32, InboundLedgerReason),
    {
        let finish_hash = *info.hash.as_uint256();
        let Some(skip_list) = self.skip_lists.get(&finish_hash).and_then(Weak::upgrade) else {
            return;
        };
        skip_list
            .lock()
            .expect("skip list lock")
            .process_data(info.seq, item);
        let Some(data) = skip_list
            .lock()
            .expect("skip list lock")
            .get_data()
            .cloned()
        else {
            return;
        };

        self.activate_skip_list_data(
            finish_hash,
            data,
            num_peers,
            &mut lookup_ledger,
            &mut fallback_acquire,
        );
    }

    pub fn got_replay_delta(
        &mut self,
        info: LedgerHeader,
        txns: BTreeMap<u32, Arc<STTx>>,
        config: &LedgerConfig,
    ) {
        self.got_replay_delta_with_rules(info, txns, crate::Rules::new(config.features.iter()));
    }

    /// Deliver an overlay-verified replay delta to its owner.
    pub fn got_replay_delta_with_rules(
        &mut self,
        info: LedgerHeader,
        txns: BTreeMap<u32, Arc<STTx>>,
        rules: crate::Rules,
    ) {
        let Some(delta) = self
            .deltas
            .get(info.hash.as_uint256())
            .and_then(Weak::upgrade)
        else {
            return;
        };
        delta
            .lock()
            .expect("delta lock")
            .process_data_with_rules(info, txns, rules);
    }

    /// Drive due timeout owners. `AppLedgerRuntime` schedules this method on
    /// its production `JtReplayTask` path; it is not a test-only ticking API.
    /// It mirrors the TimeoutCounter callbacks in LedgerReplayTask.cpp,
    /// LedgerDeltaAcquire.cpp, and SkipListAcquire.cpp.
    pub fn drive_timeouts<LookupLedger, BuildReplay, FallbackAcquire, E>(
        &mut self,
        now: Instant,
        lookup_ledger: &mut LookupLedger,
        build_replay: &mut BuildReplay,
        fallback_acquire: &mut FallbackAcquire,
    ) -> Result<Vec<Arc<Ledger>>, crate::ReplayTaskError<E>>
    where
        LookupLedger: FnMut(Uint256) -> Option<Arc<Ledger>>,
        BuildReplay: FnMut(&crate::LedgerReplay) -> Result<Arc<Ledger>, E>,
        FallbackAcquire: FnMut(Uint256, u32, InboundLedgerReason),
    {
        if self.stopping {
            return Ok(Vec::new());
        }

        let due_skip_lists = self
            .skip_list_timers
            .iter()
            .filter(|(_, deadline)| **deadline <= now)
            .filter_map(|(hash, _)| {
                self.skip_lists
                    .get(hash)
                    .and_then(Weak::upgrade)
                    .map(|skip_list| (*hash, skip_list))
            })
            .collect::<Vec<_>>();
        let mut acquired_skip_lists = Vec::new();
        for (hash, skip_list) in due_skip_lists {
            self.skip_list_timer_ticks += 1;
            let mut skip_list = skip_list.lock().expect("skip list lock");
            skip_list.invoke_on_timer(lookup_ledger, fallback_acquire);
            let next = (!skip_list.is_done()).then(|| skip_list.timer_interval());
            let data = skip_list.get_data().cloned();
            drop(skip_list);
            if let Some(interval) = next {
                self.skip_list_timers
                    .insert(hash, deadline_after(now, interval));
            } else {
                self.skip_list_timers.remove(&hash);
            }
            if let Some(data) = data {
                acquired_skip_lists.push((hash, data));
            }
        }
        for (hash, data) in acquired_skip_lists {
            // A timed local retry may find a finish ledger after the initial
            // request. Feed it through the same task/delta activation path as
            // a proof response instead of waiting for an unrelated callback.
            self.activate_skip_list_data(hash, data, 1, lookup_ledger, fallback_acquire);
        }

        let due_deltas = self
            .delta_timers
            .iter()
            .filter(|(_, deadline)| **deadline <= now)
            .filter_map(|(hash, _)| {
                self.deltas
                    .get(hash)
                    .and_then(Weak::upgrade)
                    .map(|delta| (*hash, delta))
            })
            .collect::<Vec<_>>();
        for (hash, delta) in due_deltas {
            self.delta_timer_ticks += 1;
            let mut delta = delta.lock().expect("delta lock");
            delta.invoke_on_timer(lookup_ledger, fallback_acquire);
            let next = (!delta.is_done()).then(|| delta.timer_interval());
            drop(delta);
            if let Some(interval) = next {
                self.delta_timers
                    .insert(hash, deadline_after(now, interval));
            } else {
                self.delta_timers.remove(&hash);
            }
        }

        let due_tasks = self
            .task_timers
            .iter()
            .enumerate()
            .filter(|(_, timer)| timer.due_at <= now)
            .filter_map(|(index, timer)| timer.task.upgrade().map(|task| (index, task)))
            .collect::<Vec<_>>();
        let mut advanced = Vec::new();
        for (index, task) in due_tasks {
            self.task_timer_ticks += 1;
            let mut task = task.lock().expect("task lock");
            task.invoke_on_timer(
                &mut |hash, seq| lookup_ledger(hash).filter(|ledger| ledger.header().seq == seq),
                &mut |delta, parent| {
                    let built = delta.try_build(parent, build_replay)?;
                    if let Some(ledger) = &built
                        && !advanced
                            .iter()
                            .any(|known: &Arc<Ledger>| known.header().hash == ledger.header().hash)
                    {
                        advanced.push(Arc::clone(ledger));
                    }
                    Ok(built)
                },
                fallback_acquire,
            )?;
            let next = (!task.finished()).then(|| task.timer_interval());
            drop(task);
            if let Some(timer) = self.task_timers.get_mut(index) {
                timer.due_at = next.map_or(now, |interval| deadline_after(now, interval));
            }
        }

        self.sweep();
        Ok(advanced)
    }

    /// Drives every live replay task after an overlay delta callback becomes
    /// ready, matching LedgerReplayTask.cpp::deltaReady / tryAdvance.
    pub fn advance_ready_tasks<LookupParent, BuildReplay, FallbackAcquire, E>(
        &mut self,
        lookup_parent: &mut LookupParent,
        build_replay: &mut BuildReplay,
        fallback_acquire: &mut FallbackAcquire,
    ) -> Result<Vec<Arc<Ledger>>, crate::ReplayTaskError<E>>
    where
        LookupParent: FnMut(Uint256, u32) -> Option<Arc<Ledger>>,
        BuildReplay: FnMut(&crate::LedgerReplay) -> Result<Arc<Ledger>, E>,
        FallbackAcquire: FnMut(Uint256, u32, InboundLedgerReason),
    {
        let mut advanced = Vec::new();
        for task in self.tasks.clone() {
            task.lock().expect("task lock").trigger(
                lookup_parent,
                &mut |delta, parent| {
                    let built = delta.try_build(parent, build_replay)?;
                    if let Some(ledger) = &built
                        && !advanced
                            .iter()
                            .any(|known: &Arc<Ledger>| known.header().hash == ledger.header().hash)
                    {
                        advanced.push(Arc::clone(ledger));
                    }
                    Ok(built)
                },
                fallback_acquire,
            )?;
        }
        Ok(advanced)
    }

    pub fn timer_status(&self) -> ReplayTimerStatus {
        ReplayTimerStatus {
            active_tasks: self.tasks.len(),
            active_skip_lists: self.skip_lists.values().filter_map(Weak::upgrade).count(),
            active_deltas: self.deltas.values().filter_map(Weak::upgrade).count(),
            skip_list_fallbacks: self
                .skip_lists
                .values()
                .filter_map(Weak::upgrade)
                .filter(|skip_list| {
                    skip_list
                        .lock()
                        .is_ok_and(|skip_list| skip_list.is_fallback())
                })
                .count(),
            delta_fallbacks: self
                .deltas
                .values()
                .filter_map(Weak::upgrade)
                .filter(|delta| delta.lock().is_ok_and(|delta| delta.is_fallback()))
                .count(),
            skip_list_timer_ticks: self.skip_list_timer_ticks,
            delta_timer_ticks: self.delta_timer_ticks,
            task_timer_ticks: self.task_timer_ticks,
        }
    }

    pub fn sweep(&mut self) {
        self.tasks
            .retain(|task| !task.lock().expect("task lock").finished());
        self.skip_lists.retain(|_, weak| weak.upgrade().is_some());
        self.deltas.retain(|_, weak| weak.upgrade().is_some());
        self.skip_list_timers
            .retain(|hash, _| self.skip_lists.get(hash).and_then(Weak::upgrade).is_some());
        self.delta_timers
            .retain(|hash, _| self.deltas.get(hash).and_then(Weak::upgrade).is_some());
        self.task_timers.retain(|timer| {
            timer
                .task
                .upgrade()
                .is_some_and(|task| !task.lock().expect("task lock").finished())
        });
    }

    pub fn stop(&mut self) {
        self.stopping = true;
        for task in &self.tasks {
            if let Ok(mut task) = task.lock() {
                task.stop();
            }
        }
        for delta in self.deltas.values() {
            if let Some(delta) = delta.upgrade()
                && let Ok(mut delta) = delta.lock()
            {
                delta.stop();
            }
        }
        for skip_list in self.skip_lists.values() {
            if let Some(skip_list) = skip_list.upgrade()
                && let Ok(mut skip_list) = skip_list.lock()
            {
                skip_list.stop();
            }
        }
        self.tasks.clear();
        self.skip_lists.clear();
        self.deltas.clear();
        self.skip_list_timers.clear();
        self.delta_timers.clear();
        self.task_timers.clear();
    }

    pub fn tasks_len(&self) -> usize {
        self.tasks.len()
    }

    pub fn deltas_len(&self) -> usize {
        self.deltas.len()
    }

    pub fn skip_lists_len(&self) -> usize {
        self.skip_lists.len()
    }

    pub fn is_stopped(&self) -> bool {
        self.stopping
    }

    fn arm_skip_list_timer(&mut self, hash: Uint256, now: Instant) {
        if let Some(skip_list) = self.skip_lists.get(&hash).and_then(Weak::upgrade) {
            self.skip_list_timers.insert(
                hash,
                deadline_after(
                    now,
                    skip_list.lock().expect("skip list lock").timer_interval(),
                ),
            );
        }
    }

    fn arm_delta_timer(&mut self, hash: Uint256, now: Instant) {
        if let Some(delta) = self.deltas.get(&hash).and_then(Weak::upgrade) {
            self.delta_timers.insert(
                hash,
                deadline_after(now, delta.lock().expect("delta lock").timer_interval()),
            );
        }
    }

    fn arm_task_timer(&mut self, task: &Arc<Mutex<LedgerReplayTask>>, now: Instant) {
        let due_at = deadline_after(now, task.lock().expect("task lock").timer_interval());
        if let Some(existing) = self.task_timers.iter_mut().find(|existing| {
            existing
                .task
                .upgrade()
                .is_some_and(|known| Arc::ptr_eq(&known, task))
        }) {
            existing.due_at = due_at;
        } else {
            self.task_timers.push(ReplayTaskDeadline {
                task: Arc::downgrade(task),
                due_at,
            });
        }
    }
}

fn deadline_after(now: Instant, interval: time::Duration) -> Instant {
    now + std::time::Duration::from_millis(interval.whole_milliseconds().max(0) as u64)
}
