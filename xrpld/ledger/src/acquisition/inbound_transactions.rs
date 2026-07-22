//! `InboundTransactions` owner port above the landed transaction-acquire and
//! overlay peer-set seams.

use crate::{TransactionAcquire, TransactionAcquireFilterFactory};
use basics::base_uint::Uint256;
use overlay::{Peer, PeerSetBuilder, TmLedgerData};
use shamap::sync::{SHAMapType, SyncTree};
use std::collections::BTreeMap;
use std::sync::Arc;

const START_PEERS: usize = 2;
const SET_KEEP_ROUNDS: u32 = 3;

#[derive(Default)]
struct InboundTransactionSet {
    seq: u32,
    acquire: Option<TransactionAcquire>,
    set: Option<Arc<SyncTree>>,
}

pub struct InboundTransactions {
    sets: BTreeMap<Uint256, InboundTransactionSet>,
    seq: u32,
    stopping: bool,
    peer_set_builder: Arc<dyn PeerSetBuilder>,
    filter_factory: Option<Arc<dyn TransactionAcquireFilterFactory>>,
    /// Sends (txset_hash, txset_map) to the consensus thread.
    map_complete_tx: Option<std::sync::mpsc::Sender<(Uint256, Arc<SyncTree>)>>,
}

impl InboundTransactions {
    pub fn new(peer_set_builder: Arc<dyn PeerSetBuilder>) -> Self {
        Self::new_with_filter_factory(peer_set_builder, None)
    }

    pub fn new_with_filter_factory(
        peer_set_builder: Arc<dyn PeerSetBuilder>,
        filter_factory: Option<Arc<dyn TransactionAcquireFilterFactory>>,
    ) -> Self {
        let mut sets = BTreeMap::new();
        sets.insert(Uint256::zero(), InboundTransactionSet::default());

        let zero = sets
            .get_mut(&Uint256::zero())
            .expect("zero tx set slot should exist");
        zero.set = Some(Arc::new(empty_transaction_set()));

        Self {
            sets,
            seq: 0,
            stopping: false,
            peer_set_builder,
            filter_factory,
            map_complete_tx: None,
        }
    }

    pub fn get_set(&mut self, hash: Uint256, acquire: bool) -> Option<Arc<SyncTree>> {
        if self.stopping {
            return None;
        }

        if let Some(inbound) = self.sets.get_mut(&hash) {
            if acquire {
                inbound.seq = self.seq;
                if let Some(acquire) = inbound.acquire.as_mut() {
                    acquire.still_need();
                }
            }
            return inbound.set.clone();
        }

        if !acquire || self.stopping {
            return None;
        }

        let mut tx_acquire = TransactionAcquire::with_filter_factory(
            hash,
            self.peer_set_builder.build(),
            self.filter_factory.clone(),
        );
        tx_acquire.init(START_PEERS);

        self.sets.insert(
            hash,
            InboundTransactionSet {
                seq: self.seq,
                acquire: Some(tx_acquire),
                set: None,
            },
        );

        None
    }

    pub fn acquire(&self, hash: Uint256) -> Option<&TransactionAcquire> {
        self.sets.get(&hash)?.acquire.as_ref()
    }

    pub fn acquire_mut(&mut self, hash: Uint256) -> Option<&mut TransactionAcquire> {
        self.sets.get_mut(&hash)?.acquire.as_mut()
    }

    pub fn got_data(
        &mut self,
        hash: Uint256,
        peer: Option<Arc<dyn Peer>>,
        packet: &TmLedgerData,
    ) -> InboundTransactionsDataStatus {
        let Some(acquire) = self.acquire_mut(hash) else {
            return InboundTransactionsDataStatus::NoAcquire;
        };

        match acquire.take_ledger_data(packet, peer) {
            crate::TransactionAcquireDataResult::MissingNodeId => {
                InboundTransactionsDataStatus::MissingNodeId
            }
            crate::TransactionAcquireDataResult::InvalidNodeId => {
                InboundTransactionsDataStatus::InvalidNodeId
            }
            crate::TransactionAcquireDataResult::Applied(result) => {
                InboundTransactionsDataStatus::Applied(result)
            }
        }
    }

    pub fn give_set(&mut self, hash: Uint256, set: Arc<SyncTree>, from_acquire: bool) -> bool {
        if self.stopping {
            return false;
        }

        let inbound = self.sets.entry(hash).or_default();
        inbound.seq = inbound.seq.max(self.seq);

        let is_new = inbound.set.is_none();
        if is_new {
            inbound.set = Some(Arc::clone(&set));
        }
        let was_acquiring = inbound.acquire.is_some();
        inbound.acquire = None;

        // Only fire for sets that were being acquired (from_acquire=true or had
        // an active TransactionAcquire), matching reference NetworkOPs::mapComplete.
        if is_new
            && (from_acquire || was_acquiring)
            && let Some(tx) = &self.map_complete_tx
        {
            let _ = tx.send((hash, set));
        }

        is_new
    }

    /// Set the map_complete channel sender (reference mapComplete callback).
    pub fn set_map_complete_sender(
        &mut self,
        tx: std::sync::mpsc::Sender<(Uint256, Arc<SyncTree>)>,
    ) {
        self.map_complete_tx = Some(tx);
    }

    pub fn set_peer_set_builder(&mut self, builder: Arc<dyn PeerSetBuilder>) {
        self.peer_set_builder = builder;
    }

    pub fn set_filter_factory(&mut self, factory: Arc<dyn TransactionAcquireFilterFactory>) {
        self.filter_factory = Some(factory);
    }

    pub fn new_round(&mut self, seq: u32) {
        if let Some(zero) = self.sets.get_mut(&Uint256::zero()) {
            zero.seq = seq;
        }

        if self.seq == seq {
            return;
        }

        self.seq = seq;
        let min_seq = seq.saturating_sub(SET_KEEP_ROUNDS);
        let max_seq = seq.saturating_add(SET_KEEP_ROUNDS);
        self.sets.retain(|hash, inbound| {
            *hash == Uint256::zero() || (inbound.seq >= min_seq && inbound.seq <= max_seq)
        });
    }

    pub fn stop(&mut self) {
        self.stopping = true;
        for inbound in self.sets.values_mut() {
            if let Some(acquire) = inbound.acquire.as_mut() {
                acquire.stop();
            }
        }
        self.sets.clear();
    }

    pub fn is_stopped(&self) -> bool {
        self.stopping
    }

    pub fn len(&self) -> usize {
        self.sets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    pub fn stored_hashes(&self) -> Vec<Uint256> {
        self.sets
            .iter()
            .filter(|(_, v)| v.set.is_some())
            .map(|(h, _)| *h)
            .collect()
    }

    /// Drive pending TransactionAcquire objects forward. Each pending
    /// acquisition's `invoke_on_timer` is called, which handles
    /// retry/re-request logic internally. Acquisitions that complete
    /// (or fail) as a result are finalized here via `give_set`.
    /// Matches rippled's InboundTransactions timer sweep.
    pub fn tick_pending_acquires(&mut self) {
        let hashes: Vec<Uint256> = self
            .sets
            .iter()
            .filter(|(_, v)| v.acquire.is_some())
            .map(|(h, _)| *h)
            .collect();
        for hash in hashes {
            if let Some(inbound) = self.sets.get_mut(&hash) {
                if let Some(acquire) = inbound.acquire.as_mut() {
                    acquire.invoke_on_timer();
                    if acquire.is_complete() {
                        let set = Arc::new(acquire.map().clone());
                        inbound.acquire = None;
                        let is_new = inbound.set.is_none();
                        if is_new {
                            inbound.set = Some(Arc::clone(&set));
                            if let Some(tx) = &self.map_complete_tx {
                                let _ = tx.send((hash, set));
                            }
                        }
                    } else if acquire.is_failed() {
                        inbound.acquire = None;
                    }
                }
            }
        }
    }
}

fn empty_transaction_set() -> SyncTree {
    let mut map = SyncTree::new_with_type(SHAMapType::Transaction, true, 0);
    map.set_unbacked();
    map
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundTransactionsDataStatus {
    NoAcquire,
    MissingNodeId,
    InvalidNodeId,
    Applied(shamap::sync::SHAMapAddNode),
}
