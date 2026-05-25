//! Shared read-only tx-query types for the bounded RPC transaction handlers.

#![allow(clippy::large_enum_variant)]

use std::sync::Arc;

use app::Transaction;
use basics::{base_uint::Uint256, chrono::NetClockTimePoint};
pub use protocol::TxSearched;
use protocol::{STTx, TxMeta};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxLookupError {
    DatabaseDeserialization,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxLookupOutcome {
    Found(TxRecord),
    NotFound(TxSearched),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxRecord {
    pub txn: Arc<STTx>,
    pub meta: Option<TxMeta>,
    pub ledger_index: u32,
    pub close_time: Option<NetClockTimePoint>,
    pub ledger_hash: Option<Uint256>,
    pub validated: bool,
    pub txn_index: Option<u32>,
    pub network_id: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct TxHistoryRow {
    pub transaction: Arc<Transaction>,
}
