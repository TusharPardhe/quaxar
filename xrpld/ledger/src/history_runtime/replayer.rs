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

pub const REPLAY_MAX_TASKS: usize = 10;
pub const REPLAY_MAX_TASK_SIZE: u32 = 256;

pub struct LedgerReplayer {
    tasks: Vec<Arc<Mutex<LedgerReplayTask>>>,
    deltas: BTreeMap<Uint256, Weak<Mutex<LedgerDeltaAcquire>>>,
    skip_lists: BTreeMap<Uint256, Weak<Mutex<SkipListAcquire>>>,
    peer_set_builder: Arc<dyn PeerSetBuilder>,
    stopping: bool,
}

impl LedgerReplayer {
    pub fn new(peer_set_builder: Arc<dyn PeerSetBuilder>) -> Self {
        Self {
            tasks: Vec::new(),
            deltas: BTreeMap::new(),
            skip_lists: BTreeMap::new(),
            peer_set_builder,
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
    /// acquisition. This mirrors `LedgerReplayer::replay`: it creates the
    /// task under the owner lock, then calls `skipList->init(1)` before the
    /// task can wait for its first timer
    /// (`../rippled/src/xrpld/app/ledger/detail/LedgerReplayer.cpp::replay`).
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

        // Only a newly-created acquisition receives an initial trigger. A
        // shared live skip-list is already owned and driven by its original
        // task, just as the C++ owner only initializes `newSkipList`.
        if !had_live_skip_list
            && let Some(skip_list) = self.skip_lists.get(&finish_hash).and_then(Weak::upgrade)
        {
            // The peer replay request and normal inbound acquisition begin
            // together. This prevents a new replay task from waiting idle
            // when its first peer set is empty, while a replay-capable peer
            // can still satisfy the narrower proof-path request.
            let mut found_local = false;
            let mut no_op_fallback = |_, _, _| {};
            skip_list.lock().expect("skip list lock").init(
                num_peers,
                &mut |hash| {
                    let ledger = lookup_ledger(hash);
                    found_local = ledger.is_some();
                    ledger
                },
                &mut no_op_fallback,
            );
            if !found_local {
                fallback_acquire(finish_hash, 0, reason);
            }

            // `LedgerReplayer::replay` calls `skipList->init(1)` before
            // `task->init()`. When that synchronous trigger finds the ledger
            // locally, `SkipListAcquire` already has its proof here; consume
            // it now rather than waiting forever for an overlay response that
            // will never arrive. This is the Rust equivalent of the
            // `LedgerReplayTask::init` data callback in
            // ../rippled/src/xrpld/app/ledger/detail/LedgerReplayTask.cpp.
            if let Some(data) = skip_list
                .lock()
                .expect("skip list lock")
                .get_data()
                .cloned()
            {
                let _ = self.update_task_from_skip_list(&task, finish_hash, data);
            }
        }

        self.trigger_task_and_init_deltas(
            &task,
            num_peers,
            &mut lookup_ledger,
            &mut fallback_acquire,
        );

        Some(task)
    }

    /// Mirrors LedgerReplayTask::init's immediate parent trigger and
    /// LedgerReplayer::createDeltas' `newDelta->init(1)` branch. Local or
    /// synchronous skip-list completion must not leave a replay task's parent
    /// or deltas dormant until an unrelated timer tick. See
    /// ../rippled/src/xrpld/app/ledger/detail/LedgerReplayer.cpp and
    /// ../rippled/src/xrpld/app/ledger/detail/LedgerReplayTask.cpp.
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
            fallback_acquire(parameter.start_hash, parameter.start_seq, parameter.reason);
        }

        for delta in self.create_deltas(task) {
            delta.lock().expect("delta lock").init(
                num_peers,
                &mut |hash| lookup_ledger(hash),
                fallback_acquire,
            );
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

    /// Creates all delta owners and returns only newly allocated deltas. A
    /// caller that owns an initial trigger must invoke it only for these
    /// fresh owners; shared deltas are already live.
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

    /// Delivers an asynchronously acquired skip-list item and immediately
    /// starts every delta allocated by the resulting task update. This is the
    /// same `newDelta->init(1)` branch in
    /// `../rippled/src/xrpld/app/ledger/detail/LedgerReplayer.cpp::
    /// LedgerReplayer::createDeltas`; without it a task accepted before its
    /// skip-list response can retain dormant delta owners indefinitely.
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
        let Some(skip_list) = self
            .skip_lists
            .get(info.hash.as_uint256())
            .and_then(Weak::upgrade)
        else {
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

        let tasks = self.tasks.clone();
        for task in tasks {
            if task.lock().expect("task lock").parameter().finish_hash == *info.hash.as_uint256()
                && self.update_task_from_skip_list(&task, *info.hash.as_uint256(), data.clone())
            {
                self.trigger_task_and_init_deltas(
                    &task,
                    num_peers,
                    &mut lookup_ledger,
                    &mut fallback_acquire,
                );
            }
        }
    }

    pub fn got_replay_delta(
        &mut self,
        info: LedgerHeader,
        txns: BTreeMap<u32, Arc<STTx>>,
        config: &LedgerConfig,
    ) {
        self.got_replay_delta_with_rules(info, txns, crate::Rules::new(config.features.iter()));
    }

    /// Delivers a replay response that was verified by the overlay bridge.
    ///
    /// `LedgerReplayMsgHandler::processReplayDeltaResponse` in rippled hands
    /// the active application rules to `LedgerDeltaAcquire::processData` only
    /// after validating the response header and transaction SHAMap. The Rust
    /// bridge preserves that split so wire validation remains outside this
    /// owner while the delta registry remains the sole routing authority.
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

    /// Drives every live replay task after a delta has become ready. This is
    /// the callback progression in `LedgerReplayTask.cpp::deltaReady` and
    /// `tryAdvance`: resolve its starting parent, build each ready consecutive
    /// delta, and retain the reconstructed ledgers for the application owner
    /// to store/publish.
    pub fn advance_ready_tasks<LookupParent, BuildReplay, E>(
        &mut self,
        lookup_parent: &mut LookupParent,
        build_replay: &mut BuildReplay,
    ) -> Result<Vec<Arc<Ledger>>, crate::ReplayTaskError<E>>
    where
        LookupParent: FnMut(Uint256, u32) -> Option<Arc<Ledger>>,
        BuildReplay: FnMut(&crate::LedgerReplay) -> Result<Arc<Ledger>, E>,
    {
        let mut advanced = Vec::new();
        let tasks = self.tasks.clone();
        for task in tasks {
            task.lock()
                .expect("task lock")
                .trigger(lookup_parent, &mut |delta, parent| {
                    let built = delta.try_build(parent, build_replay)?;
                    if let Some(ledger) = &built
                        && !advanced
                            .iter()
                            .any(|known: &Arc<Ledger>| known.header().hash == ledger.header().hash)
                    {
                        advanced.push(Arc::clone(ledger));
                    }
                    Ok(built)
                })?;
        }
        Ok(advanced)
    }

    pub fn sweep(&mut self) {
        self.tasks
            .retain(|task| !task.lock().expect("task lock").finished());
        self.skip_lists.retain(|_, weak| weak.upgrade().is_some());
        self.deltas.retain(|_, weak| weak.upgrade().is_some());
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
}
