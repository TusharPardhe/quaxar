//! `AcceptedLedger` owner port.

use crate::{AcceptedLedgerTx, Ledger, LedgerTxReadError};
use basics::tagged_cache::CacheClock;
use shamap::family::{FullBelowCache, MissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher};
use shamap::traversal::TraversalError;
use std::hash::BuildHasher;
use std::slice;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptedLedgerBuildError {
    TxRead(LedgerTxReadError),
    Traversal(TraversalError),
}

impl From<LedgerTxReadError> for AcceptedLedgerBuildError {
    fn from(value: LedgerTxReadError) -> Self {
        Self::TxRead(value)
    }
}

impl From<TraversalError> for AcceptedLedgerBuildError {
    fn from(value: TraversalError) -> Self {
        Self::Traversal(value)
    }
}

#[derive(Debug)]
pub struct AcceptedLedger {
    ledger: Arc<Ledger>,
    transactions: Vec<AcceptedLedgerTx>,
}

impl AcceptedLedger {
    pub fn new(ledger: Arc<Ledger>) -> Result<Self, AcceptedLedgerBuildError> {
        let mut transactions = Vec::with_capacity(256);
        for (txn, meta) in ledger.tx_snapshot()? {
            transactions.push(AcceptedLedgerTx::new(ledger.as_ref(), txn, meta)?);
        }

        transactions.sort_by_key(AcceptedLedgerTx::get_txn_seq);

        Ok(Self {
            ledger,
            transactions,
        })
    }

    pub fn new_with_family<CLOCK, S, FB, F, MR, NS>(
        ledger: Arc<Ledger>,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<Self, AcceptedLedgerBuildError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        let mut transactions = Vec::with_capacity(256);
        for (txn, meta) in ledger.tx_snapshot_with_family(family)? {
            transactions.push(AcceptedLedgerTx::new(ledger.as_ref(), txn, meta)?);
        }

        transactions.sort_by_key(AcceptedLedgerTx::get_txn_seq);

        Ok(Self {
            ledger,
            transactions,
        })
    }

    pub fn get_ledger(&self) -> &Arc<Ledger> {
        &self.ledger
    }

    pub fn size(&self) -> usize {
        self.transactions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }

    pub fn iter(&self) -> slice::Iter<'_, AcceptedLedgerTx> {
        self.transactions.iter()
    }

    pub fn as_slice(&self) -> &[AcceptedLedgerTx] {
        &self.transactions
    }
}

impl<'a> IntoIterator for &'a AcceptedLedger {
    type Item = &'a AcceptedLedgerTx;
    type IntoIter = slice::Iter<'a, AcceptedLedgerTx>;

    fn into_iter(self) -> Self::IntoIter {
        self.transactions.iter()
    }
}
