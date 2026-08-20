//! `LedgerReplay` ownership surface ported into the ledger crate.
//!
//! The app crate already owns replay application order when it actually builds
//! a ledger. This module carries the replay data object itself so the ledger
//! crate can own acquisition and task orchestration without inventing a second
//! copy of the ordering rules.

use crate::{Ledger, LedgerTxReadError};
use protocol::{STTx, Serializer};
use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ReplayTransaction {
    transaction: Arc<STTx>,
    metadata: Arc<Serializer>,
}

impl ReplayTransaction {
    /// A verified transaction and its exact TransactionMd metadata payload.
    /// Replay reconstruction must retain both: the metadata bytes are part of
    /// the transaction SHAMap leaf committed by the replay ledger header.
    pub fn new(transaction: Arc<STTx>, metadata: Arc<Serializer>) -> Self {
        Self {
            transaction,
            metadata,
        }
    }

    pub fn transaction(&self) -> &Arc<STTx> {
        &self.transaction
    }

    pub fn metadata(&self) -> &Arc<Serializer> {
        &self.metadata
    }
}

impl Deref for ReplayTransaction {
    type Target = STTx;

    fn deref(&self) -> &Self::Target {
        self.transaction.as_ref()
    }
}

#[derive(Debug, Clone)]
pub struct LedgerReplay {
    parent: Arc<Ledger>,
    replay: Arc<Ledger>,
    ordered_txs: BTreeMap<u32, ReplayTransaction>,
}

impl LedgerReplay {
    pub fn new(
        parent: Arc<Ledger>,
        replay: Arc<Ledger>,
        ordered_txs: BTreeMap<u32, Arc<STTx>>,
    ) -> Self {
        let ordered_txs = ordered_txs
            .into_iter()
            .map(|(index, transaction)| {
                (
                    index,
                    ReplayTransaction::new(transaction, Arc::new(Serializer::default())),
                )
            })
            .collect();
        Self::new_with_metadata(parent, replay, ordered_txs)
    }

    /// Constructs a replay from TransactionMd-verified entries. Production
    /// replay-delta reconstruction must use this constructor so the rebuilt
    /// ledger retains its committed transaction-map leaves.
    pub fn new_with_metadata(
        parent: Arc<Ledger>,
        replay: Arc<Ledger>,
        ordered_txs: BTreeMap<u32, ReplayTransaction>,
    ) -> Self {
        Self {
            parent,
            replay,
            ordered_txs,
        }
    }

    pub fn from_replay_ledger(
        parent: Arc<Ledger>,
        replay: Arc<Ledger>,
    ) -> Result<Self, LedgerReplayError> {
        let mut ordered_txs = BTreeMap::new();

        for (tx, mut meta) in replay.tx_snapshot()? {
            let mut raw_metadata = Serializer::default();
            meta.add_raw(&mut raw_metadata, meta.get_result_ter(), meta.get_index());
            ordered_txs
                .entry(meta.get_index())
                .or_insert_with(|| ReplayTransaction::new(tx, Arc::new(raw_metadata)));
        }

        Ok(Self::new_with_metadata(parent, replay, ordered_txs))
    }

    pub fn parent(&self) -> &Arc<Ledger> {
        &self.parent
    }

    pub fn replay(&self) -> &Arc<Ledger> {
        &self.replay
    }

    pub fn ordered_txs(&self) -> &BTreeMap<u32, ReplayTransaction> {
        &self.ordered_txs
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerReplayError {
    TxRead(LedgerTxReadError),
}

impl From<LedgerTxReadError> for LedgerReplayError {
    fn from(value: LedgerTxReadError) -> Self {
        Self::TxRead(value)
    }
}
