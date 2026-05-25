//! `LedgerReplay` ownership surface ported into the ledger crate.
//!
//! The app crate already owns replay application order when it actually builds
//! a ledger. This module carries the replay data object itself so the ledger
//! crate can own acquisition and task orchestration without inventing a second
//! copy of the ordering rules.

use crate::{Ledger, LedgerTxReadError};
use protocol::STTx;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct LedgerReplay {
    parent: Arc<Ledger>,
    replay: Arc<Ledger>,
    ordered_txs: BTreeMap<u32, Arc<STTx>>,
}

impl LedgerReplay {
    pub fn new(
        parent: Arc<Ledger>,
        replay: Arc<Ledger>,
        ordered_txs: BTreeMap<u32, Arc<STTx>>,
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

        for (tx, meta) in replay.tx_snapshot()? {
            ordered_txs.entry(meta.get_index()).or_insert(tx);
        }

        Ok(Self::new(parent, replay, ordered_txs))
    }

    pub fn parent(&self) -> &Arc<Ledger> {
        &self.parent
    }

    pub fn replay(&self) -> &Arc<Ledger> {
        &self.replay
    }

    pub fn ordered_txs(&self) -> &BTreeMap<u32, Arc<STTx>> {
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
