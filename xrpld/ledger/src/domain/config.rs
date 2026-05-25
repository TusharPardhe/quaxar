//! Config-facing seam for the current `Ledger` constructors.
//!
//! This carries the fee and preset-feature fields that the widened Rust ledger
//! caller paths currently consume.

use crate::Fees;
use protocol::FeatureSet;
use time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LedgerConfig {
    pub fees: Fees,
    pub features: FeatureSet,
}

impl LedgerConfig {
    pub fn new(fees: Fees, features: FeatureSet) -> Self {
        Self { fees, features }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerHistorySyncConfig {
    pub ledger_history: u32,
    pub ledger_fetch_size: u32,
    pub fetch_pack_stale_after: Duration,
}

impl LedgerHistorySyncConfig {
    pub fn new(
        ledger_history: u32,
        ledger_fetch_size: u32,
        fetch_pack_stale_after: Duration,
    ) -> Self {
        Self {
            ledger_history,
            ledger_fetch_size,
            fetch_pack_stale_after,
        }
    }
}
