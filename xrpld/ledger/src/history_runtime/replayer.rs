//! `LedgerReplayer` owner port above the landed replay task and acquire
//! sub-owners.

use crate::{
    InboundLedgerReason, LedgerConfig, LedgerDeltaAcquire, LedgerHeader, LedgerReplayTask,
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

        if let Some(data) = skip_list
            .lock()
            .expect("skip list lock")
            .get_data()
            .cloned()
        {
            let mut task_ref = task.lock().expect("task lock");
            if task_ref.update_skip_list(finish_hash, data.ledger_seq, &data.skip_list) {
                drop(task_ref);
                self.create_deltas(&task);
            }
        }

        Some(task)
    }

    pub fn create_deltas(&mut self, task: &Arc<Mutex<LedgerReplayTask>>) {
        let parameter = task.lock().expect("task lock").parameter().clone();
        if parameter.total_ledgers <= 1 {
            return;
        }

        let Some(mut index) = parameter
            .skip_list
            .iter()
            .position(|hash| *hash == parameter.start_hash)
        else {
            return;
        };
        index += 1;
        if index >= parameter.skip_list.len() {
            return;
        }

        for seq in parameter.start_seq + 1..=parameter.finish_seq {
            let Some(hash) = parameter.skip_list.get(index).copied() else {
                break;
            };
            index += 1;

            let delta = self
                .deltas
                .get(&hash)
                .and_then(Weak::upgrade)
                .unwrap_or_else(|| {
                    let created = Arc::new(Mutex::new(LedgerDeltaAcquire::new(
                        hash,
                        seq,
                        self.peer_set_builder.build(),
                    )));
                    self.deltas.insert(hash, Arc::downgrade(&created));
                    created
                });

            task.lock().expect("task lock").add_delta(delta);
        }
    }

    pub fn got_skip_list(&mut self, info: LedgerHeader, item: &SHAMapItem) {
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
            let mut task_ref = task.lock().expect("task lock");
            if task_ref.parameter().finish_hash == *info.hash.as_uint256()
                && task_ref.update_skip_list(
                    *info.hash.as_uint256(),
                    data.ledger_seq,
                    &data.skip_list,
                )
            {
                drop(task_ref);
                self.create_deltas(&task);
            }
        }
    }

    pub fn got_replay_delta(
        &mut self,
        info: LedgerHeader,
        txns: BTreeMap<u32, Arc<STTx>>,
        config: &LedgerConfig,
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
            .process_data(info, txns, config);
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
