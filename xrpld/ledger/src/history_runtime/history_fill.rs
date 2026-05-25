//! Pure `LedgerMaster::tryFill(...)` backwalk logic above the current Rust
//! ledger-info and SHAMap storage seams.
//!
//! The wider reference owner path still depends on `Application`, `JobQueue`,
//! `RangeSet`, `InboundLedgers`, and SQL/node-store ownership. This module
//! ports the deterministic gap-analysis core only:
//! - walk backward from an acquired ledger,
//! - refresh SQL hash windows in 500-ledger chunks,
//! - stop when a prior ledger is already known,
//! - stop when SQL rows are missing,
//! - stop when node-store backing is missing,
//! - stop when the SQL ledger hash no longer matches the expected parent hash,
//! - and emit the same intermediate/final complete ranges the reference owner
//!   inserts into `mCompleteLedgers`.

use crate::LedgerHeader;
use basics::sha_map_hash::SHAMapHash;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerHashPair {
    pub ledger_hash: SHAMapHash,
    pub parent_hash: SHAMapHash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerFillRange {
    pub min: u32,
    pub max: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerHistoryFillStopReason {
    Stopping,
    AlreadyHaveLedger {
        seq: u32,
    },
    MissingSqlWindow {
        seq: u32,
    },
    NodeStoreMismatch {
        seq: u32,
    },
    ParentHashMismatch {
        seq: u32,
        expected: SHAMapHash,
        found: SHAMapHash,
    },
    ReachedGenesis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerHistoryFillPlan {
    pub inserted_ranges: Vec<LedgerFillRange>,
    pub stop_reason: LedgerHistoryFillStopReason,
}

pub trait LedgerPresence {
    fn have_ledger(&self, ledger_index: u32) -> bool;
}

pub trait LedgerHashPairProvider {
    fn get_hashes_by_index(&self, min_seq: u32, max_seq: u32) -> Vec<(u32, LedgerHashPair)>;
}

pub trait LedgerObjectPresence {
    fn has_ledger_object(&self, ledger_hash: SHAMapHash, ledger_seq: u32) -> bool;
}

pub trait Stopper {
    fn is_stopping(&self) -> bool;
}

pub fn run_try_fill_backwalk<P, DB, NS, ST>(
    ledger: &LedgerHeader,
    presence: &P,
    hash_pairs: &DB,
    node_store: &NS,
    stopper: &ST,
) -> LedgerHistoryFillPlan
where
    P: LedgerPresence,
    DB: LedgerHashPairProvider,
    NS: LedgerObjectPresence,
    ST: Stopper,
{
    let mut seq = ledger.seq;
    let mut prev_hash = ledger.parent_hash;
    let mut window = Vec::new();
    let mut min_has = seq;
    let mut max_has = seq;
    let mut inserted_ranges = Vec::new();

    let stop_reason = loop {
        if stopper.is_stopping() {
            break LedgerHistoryFillStopReason::Stopping;
        }

        if seq == 0 {
            break LedgerHistoryFillStopReason::ReachedGenesis;
        }

        min_has = seq;
        seq -= 1;

        if presence.have_ledger(seq) {
            break LedgerHistoryFillStopReason::AlreadyHaveLedger { seq };
        }

        let mut pair = window
            .iter()
            .find(|(window_seq, _)| *window_seq == seq)
            .map(|(_, pair)| *pair);

        if pair.is_none() {
            inserted_ranges.push(LedgerFillRange {
                min: min_has,
                max: max_has,
            });
            max_has = min_has;

            let min_window = seq.saturating_sub(499);
            window = hash_pairs.get_hashes_by_index(min_window, seq);
            window.sort_by_key(|(window_seq, _)| *window_seq);
            pair = window
                .iter()
                .find(|(window_seq, _)| *window_seq == seq)
                .map(|(_, pair)| *pair);

            if pair.is_none() {
                break LedgerHistoryFillStopReason::MissingSqlWindow { seq };
            }

            let Some(first) = window.first() else {
                break LedgerHistoryFillStopReason::MissingSqlWindow { seq };
            };

            if !node_store.has_ledger_object(first.1.ledger_hash, first.0) {
                break LedgerHistoryFillStopReason::NodeStoreMismatch { seq };
            }
        }

        let Some(pair) = pair else {
            break LedgerHistoryFillStopReason::MissingSqlWindow { seq };
        };

        if pair.ledger_hash != prev_hash {
            break LedgerHistoryFillStopReason::ParentHashMismatch {
                seq,
                expected: prev_hash,
                found: pair.ledger_hash,
            };
        }

        prev_hash = pair.parent_hash;
    };

    inserted_ranges.push(LedgerFillRange {
        min: min_has,
        max: max_has,
    });

    LedgerHistoryFillPlan {
        inserted_ranges,
        stop_reason,
    }
}
