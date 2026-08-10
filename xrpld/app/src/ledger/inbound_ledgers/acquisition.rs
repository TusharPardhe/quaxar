//! Per-hash inbound-ledger lifecycle.
//!
//! The structure follows rippled's `InboundLedger` and `TimeoutCounter`:
//! `init` checks local storage, adds peers, queues an immediate timeout job,
//! and every timeout job re-arms only its own three-second timer.

use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::MonotonicClock;
use ledger::{
    select_inbound_ledger_reply_peers, FetchPackCache, FetchPackContainer, FetchPackStore,
    InboundLedgerJournal, InboundLedgerLocal, InboundLedgerPacket, InboundLedgerPacketError,
    InboundLedgerPeerScore, InboundLedgerReason, InboundLedgerRequestTrigger, InboundLedgerStore,
    InboundLedgerTimerResult, Ledger, StateScanParams,
};
use overlay::{Peer, PeerSet as _};
use shamap::family::{FullBelowCacheImpl, NullMissingNodeReporter, SHAMapFamily};
use shamap::sync::{
    DeferredMissingNodeScan, DeferredMissingNodeScanProgress, DeferredMissingNodeScanStats,
    SHAMapAddNode,
};
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
#[cfg(test)]
use std::sync::Condvar;
use std::time::{Duration, Instant};

use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

use super::registry::{AcquireReason, AcquisitionLifecycleCounters, CompletedInboundLedger};
use super::worker_pool::WorkerPool;

const PEER_COUNT_START: usize = 5;
const PEER_COUNT_ADD: usize = 3;
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);

#[cfg(test)]
#[derive(Default)]
struct StateScanAfterAdvancePauseState {
    entered: bool,
    released: bool,
}

/// Per-acquisition test seam placed after a bounded state-scan advance and
/// before the production terminal guard. It is compiled only for the private
/// module tests, so independent tests cannot pause an unrelated acquisition.
#[cfg(test)]
#[derive(Default)]
struct StateScanAfterAdvancePause {
    state: Mutex<StateScanAfterAdvancePauseState>,
    wake: Condvar,
}

#[cfg(test)]
impl StateScanAfterAdvancePause {
    fn wait_until_entered(&self) {
        let state = self.state.lock().expect("state scan pause lock");
        let (state, timeout) = self
            .wake
            .wait_timeout_while(state, Duration::from_secs(5), |state| !state.entered)
            .expect("state scan pause wait");
        assert!(state.entered, "state scan did not reach its post-advance pause");
        assert!(!timeout.timed_out(), "state scan post-advance pause timed out");
    }

    fn pause_after_advance(&self) {
        let mut state = self.state.lock().expect("state scan pause lock");
        state.entered = true;
        self.wake.notify_all();
        while !state.released {
            state = self.wake.wait(state).expect("state scan pause wait");
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().expect("state scan pause lock");
        state.released = true;
        self.wake.notify_all();
    }
}

struct WorkerJournal;

impl InboundLedgerJournal for WorkerJournal {
    fn trace(&self, message: &str) {
        tracing::trace!(target: "inbound_ledger", "{message}");
    }

    fn debug(&self, message: &str) {
        tracing::debug!(target: "inbound_ledger", "{message}");
    }

    fn warn(&self, message: &str) {
        tracing::warn!(target: "inbound_ledger", "{message}");
    }

    fn fatal(&self, message: &str) {
        tracing::error!(target: "inbound_ledger", "{message}");
    }
}

#[derive(Clone)]
pub(crate) struct WorkerFetchPack {
    cache: Arc<FetchPackCache>,
}

impl FetchPackContainer for WorkerFetchPack {
    fn get_fetch_pack(&mut self, hash: Uint256) -> Option<Vec<u8>> {
        self.cache.get_fetch_pack(hash)
    }
}

impl FetchPackStore for WorkerFetchPack {
    fn add_fetch_pack(&mut self, hash: Uint256, data: Vec<u8>) {
        self.cache.add_fetch_pack(hash, data);
    }
}

/// Live-overlay source used to mirror rippled `PeerSetImpl::addPeers`, which
/// enumerates the overlay at every peer-add turn rather than keeping the
/// construction-time peer list.
pub(crate) type AcquisitionPeerProvider =
    Arc<dyn Fn() -> Vec<Arc<dyn Peer>> + Send + Sync + 'static>;

/// Registry-owned callback invoked exactly once when this acquisition fails.
/// Mirrors rippled's deferred `InboundLedgers::logFailure` lifecycle while
/// making the five-minute failure cooldown visible before the next sweep.
pub(crate) type AcquisitionFailureRecorder = Arc<dyn Fn(Uint256) + Send + Sync + 'static>;

/// Registry-owned callback invoked exactly once after a successful terminal
/// acquisition has made its completed ledger visible. This mirrors
/// `InboundLedger::done()` calling `touch()` before it dispatches `AcqDone`,
/// preserving the completed entry for its normal sweep lifetime.
pub(crate) type AcquisitionCompletionRecorder =
    Arc<dyn Fn(Uint256, Arc<Ledger>) + Send + Sync + 'static>;

/// Read-only outcome of an internal state-map scan pass. The resume value is
/// intentionally distinct from a read-budget pass: a synchronous deferred
/// read can produce locally runnable traversal before the read budget is
/// spent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum StateScanSliceOutcome {
    None = 0,
    DeferredReadBudget = 2,
    DeferredReadResume = 3,
    MissingNodeLimit = 4,
    Complete = 5,
}

impl StateScanSliceOutcome {
    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::DeferredReadBudget => "deferred_read_budget",
            Self::DeferredReadResume => "deferred_read_resume",
            Self::MissingNodeLimit => "missing_node_limit",
            Self::Complete => "complete",
        }
    }

    fn from_raw(raw: u8) -> Self {
        match raw {
            2 => Self::DeferredReadBudget,
            3 => Self::DeferredReadResume,
            4 => Self::MissingNodeLimit,
            5 => Self::Complete,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StateScanSliceDeltas {
    branch_steps: u64,
    deferred_reads: u64,
    deferred_resumes: u64,
    missing_nodes: u64,
}

fn classify_state_scan_slice(
    before: DeferredMissingNodeScanProgress,
    after: DeferredMissingNodeScanProgress,
) -> (StateScanSliceOutcome, StateScanSliceDeltas) {
    let deltas = StateScanSliceDeltas {
        branch_steps: after.branch_steps.saturating_sub(before.branch_steps),
        deferred_reads: after.deferred_reads.saturating_sub(before.deferred_reads),
        deferred_resumes: after.deferred_resumes.saturating_sub(before.deferred_resumes),
        missing_nodes: after.missing_nodes.saturating_sub(before.missing_nodes),
    };
    let outcome = if after.remaining <= 0 {
        StateScanSliceOutcome::MissingNodeLimit
    } else if after.complete {
        StateScanSliceOutcome::Complete
    } else if deltas.deferred_reads >= STATE_SCAN_MAX_DEFERRED_READS_PER_TURN as u64 {
        StateScanSliceOutcome::DeferredReadBudget
    } else {
        // Deferred NodeStore completions may create a resume after fewer than
        // the read budget. It remains local work, not a network-wait state.
        StateScanSliceOutcome::DeferredReadResume
    };
    (outcome, deltas)
}

#[derive(Debug)]
struct AcquisitionStats {
    started_at: Instant,
    first_header_at: Mutex<Option<Instant>>,
    packets: AtomicU64,
    useful_packets: AtomicU64,
    useful_nodes: AtomicU64,
    state_packets: AtomicU64,
    state_useful_nodes: AtomicU64,
    state_duplicate_nodes: AtomicU64,
    malformed_packets: AtomicU64,
    state_scan_runs: AtomicU64,
    state_missing_nodes: AtomicU64,
    tx_missing_nodes: AtomicU64,
    state_scan_us: AtomicU64,
    state_scan_branch_steps: AtomicU64,
    state_scan_missing_nodes_recorded: AtomicU64,
    state_scan_positive_progress_slices: AtomicU64,
    state_scan_branch_budget_yields: AtomicU64,
    state_scan_deferred_read_budget_yields: AtomicU64,
    state_scan_deferred_read_resume_yields: AtomicU64,
    state_scan_missing_node_limit_yields: AtomicU64,
    state_scan_completed_slices: AtomicU64,
    state_scan_last_outcome: AtomicU8,
    state_scan_last_branch_steps: AtomicU64,
    state_scan_last_deferred_reads: AtomicU64,
    state_scan_last_deferred_resumes: AtomicU64,
    state_scan_last_missing_nodes: AtomicU64,
    state_scan_branches_seen: AtomicU64,
    state_scan_duplicate_missing_hashes: AtomicU64,
    state_scan_full_below_hits: AtomicU64,
    state_scan_loaded_or_cached_children: AtomicU64,
    state_scan_pending_reads: AtomicU64,
    state_scan_max_pending_reads: AtomicU64,
    state_scan_pending_hits: AtomicU64,
    state_scan_pending_misses: AtomicU64,
    state_scan_deferred_resumes: AtomicU64,
    state_scan_yields: AtomicU64,
    state_scan_continuations: AtomicU64,
    timeout_dispatches: AtomicU64,
    state_scan_max_buffered_packets: AtomicU64,
    data_drain_runs: AtomicU64,
    data_drain_us: AtomicU64,
    data_drain_max_us: AtomicU64,
    data_drain_max_packets: AtomicU64,
    tx_scan_us: AtomicU64,
    worker_jobs: AtomicU64,
    worker_queue_wait_us: AtomicU64,
    node_store_fetch_hits: AtomicU64,
    node_store_fetch_misses: AtomicU64,
    last_diagnostic_at: Mutex<Instant>,
}

impl AcquisitionStats {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            first_header_at: Mutex::new(None),
            packets: AtomicU64::new(0),
            useful_packets: AtomicU64::new(0),
            useful_nodes: AtomicU64::new(0),
            state_packets: AtomicU64::new(0),
            state_useful_nodes: AtomicU64::new(0),
            state_duplicate_nodes: AtomicU64::new(0),
            malformed_packets: AtomicU64::new(0),
            state_scan_runs: AtomicU64::new(0),
            state_missing_nodes: AtomicU64::new(0),
            tx_missing_nodes: AtomicU64::new(0),
            state_scan_us: AtomicU64::new(0),
            state_scan_branch_steps: AtomicU64::new(0),
            state_scan_missing_nodes_recorded: AtomicU64::new(0),
            state_scan_positive_progress_slices: AtomicU64::new(0),
            state_scan_branch_budget_yields: AtomicU64::new(0),
            state_scan_deferred_read_budget_yields: AtomicU64::new(0),
            state_scan_deferred_read_resume_yields: AtomicU64::new(0),
            state_scan_missing_node_limit_yields: AtomicU64::new(0),
            state_scan_completed_slices: AtomicU64::new(0),
            state_scan_last_outcome: AtomicU8::new(StateScanSliceOutcome::None as u8),
            state_scan_last_branch_steps: AtomicU64::new(0),
            state_scan_last_deferred_reads: AtomicU64::new(0),
            state_scan_last_deferred_resumes: AtomicU64::new(0),
            state_scan_last_missing_nodes: AtomicU64::new(0),
            state_scan_branches_seen: AtomicU64::new(0),
            state_scan_duplicate_missing_hashes: AtomicU64::new(0),
            state_scan_full_below_hits: AtomicU64::new(0),
            state_scan_loaded_or_cached_children: AtomicU64::new(0),
            state_scan_pending_reads: AtomicU64::new(0),
            state_scan_max_pending_reads: AtomicU64::new(0),
            state_scan_pending_hits: AtomicU64::new(0),
            state_scan_pending_misses: AtomicU64::new(0),
            state_scan_deferred_resumes: AtomicU64::new(0),
            state_scan_yields: AtomicU64::new(0),
            state_scan_continuations: AtomicU64::new(0),
            timeout_dispatches: AtomicU64::new(0),
            state_scan_max_buffered_packets: AtomicU64::new(0),
            data_drain_runs: AtomicU64::new(0),
            data_drain_us: AtomicU64::new(0),
            data_drain_max_us: AtomicU64::new(0),
            data_drain_max_packets: AtomicU64::new(0),
            tx_scan_us: AtomicU64::new(0),
            worker_jobs: AtomicU64::new(0),
            worker_queue_wait_us: AtomicU64::new(0),
            node_store_fetch_hits: AtomicU64::new(0),
            node_store_fetch_misses: AtomicU64::new(0),
            last_diagnostic_at: Mutex::new(Instant::now()),
        }
    }

    fn mark_header_received(&self) {
        let mut first_header_at = self
            .first_header_at
            .lock()
            .expect("acquisition first_header_at lock");
        if first_header_at.is_none() {
            *first_header_at = Some(Instant::now());
        }
    }

    fn record_state_scan(&self, scan: &DeferredMissingNodeScanStats) {
        self.state_scan_branches_seen
            .fetch_add(scan.branches_seen, Ordering::Relaxed);
        self.state_scan_duplicate_missing_hashes
            .fetch_add(scan.duplicate_missing_hashes, Ordering::Relaxed);
        self.state_scan_full_below_hits
            .fetch_add(scan.full_below_hits, Ordering::Relaxed);
        self.state_scan_loaded_or_cached_children
            .fetch_add(scan.loaded_or_cached_children, Ordering::Relaxed);
        self.state_scan_pending_reads
            .fetch_add(scan.pending_reads, Ordering::Relaxed);
        self.state_scan_max_pending_reads
            .fetch_max(scan.max_pending_reads, Ordering::Relaxed);
        self.state_scan_pending_hits
            .fetch_add(scan.completed_pending_reads, Ordering::Relaxed);
        self.state_scan_pending_misses
            .fetch_add(scan.completed_pending_misses, Ordering::Relaxed);
        self.state_scan_deferred_resumes
            .fetch_add(scan.deferred_resumes, Ordering::Relaxed);
    }

    fn record_state_scan_slice(
        &self,
        outcome: StateScanSliceOutcome,
        deltas: StateScanSliceDeltas,
    ) {
        self.state_scan_branch_steps
            .fetch_add(deltas.branch_steps, Ordering::Relaxed);
        self.state_scan_missing_nodes_recorded
            .fetch_add(deltas.missing_nodes, Ordering::Relaxed);
        if deltas.branch_steps != 0
            || deltas.deferred_reads != 0
            || deltas.deferred_resumes != 0
            || deltas.missing_nodes != 0
        {
            self.state_scan_positive_progress_slices
                .fetch_add(1, Ordering::Relaxed);
        }
        let outcome_counter = match outcome {
            StateScanSliceOutcome::DeferredReadBudget => {
                Some(&self.state_scan_deferred_read_budget_yields)
            }
            StateScanSliceOutcome::DeferredReadResume => {
                Some(&self.state_scan_deferred_read_resume_yields)
            }
            StateScanSliceOutcome::MissingNodeLimit => {
                Some(&self.state_scan_missing_node_limit_yields)
            }
            StateScanSliceOutcome::Complete => Some(&self.state_scan_completed_slices),
            StateScanSliceOutcome::None => None,
        };
        if let Some(counter) = outcome_counter {
            counter.fetch_add(1, Ordering::Relaxed);
        }
        self.state_scan_last_outcome
            .store(outcome as u8, Ordering::Relaxed);
        self.state_scan_last_branch_steps
            .store(deltas.branch_steps, Ordering::Relaxed);
        self.state_scan_last_deferred_reads
            .store(deltas.deferred_reads, Ordering::Relaxed);
        self.state_scan_last_deferred_resumes
            .store(deltas.deferred_resumes, Ordering::Relaxed);
        self.state_scan_last_missing_nodes
            .store(deltas.missing_nodes, Ordering::Relaxed);
    }

    fn record_state_scan_buffered_packets(&self, packets: usize) {
        self.state_scan_max_buffered_packets
            .fetch_max(packets as u64, Ordering::Relaxed);
    }

    fn record_data_drain(&self, elapsed_us: u64, packets: usize) {
        self.data_drain_runs.fetch_add(1, Ordering::Relaxed);
        self.data_drain_us.fetch_add(elapsed_us, Ordering::Relaxed);
        self.data_drain_max_us
            .fetch_max(elapsed_us, Ordering::Relaxed);
        self.data_drain_max_packets
            .fetch_max(packets as u64, Ordering::Relaxed);
    }

    fn should_emit_sampled_diagnostic(&self) -> bool {
        if !tracing::enabled!(target: "inbound_ledger", tracing::Level::DEBUG) {
            return false;
        }

        let now = Instant::now();
        let mut last = self
            .last_diagnostic_at
            .lock()
            .expect("acquisition diagnostic sampling lock");
        if now.duration_since(*last) >= Duration::from_secs(5) {
            *last = now;
            return true;
        }
        false
    }

    fn record_node_store_fetch(&self, hit: bool) {
        let counter = if hit {
            &self.node_store_fetch_hits
        } else {
            &self.node_store_fetch_misses
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// Lightweight per-acquisition state used by `fetch_info`. All counts are
/// bounded counters or last-observed values; producing this snapshot never
/// walks a SHAMap or performs a NodeStore read.
#[derive(Debug, Clone)]
pub(crate) struct AcquisitionSnapshot {
    pub age_ms: u64,
    pub header_after_ms: Option<u64>,
    pub seq: u32,
    pub have_header: bool,
    pub have_state: bool,
    pub have_transactions: bool,
    pub timeouts: u32,
    pub packets: u64,
    pub useful_packets: u64,
    pub useful_nodes: u64,
    pub state_packets: u64,
    pub state_useful_nodes: u64,
    pub state_duplicate_nodes: u64,
    pub malformed_packets: u64,
    pub state_scan_runs: u64,
    pub state_missing_nodes: u64,
    pub tx_missing_nodes: u64,
    pub state_scan_us: u64,
    pub state_scan_branch_steps: u64,
    pub state_scan_missing_nodes_recorded: u64,
    pub state_scan_positive_progress_slices: u64,
    pub state_scan_branch_budget_yields: u64,
    pub state_scan_deferred_read_budget_yields: u64,
    pub state_scan_deferred_read_resume_yields: u64,
    pub state_scan_missing_node_limit_yields: u64,
    pub state_scan_completed_slices: u64,
    pub state_scan_last_yield: &'static str,
    pub state_scan_last_branch_steps: u64,
    pub state_scan_last_deferred_reads: u64,
    pub state_scan_last_deferred_resumes: u64,
    pub state_scan_last_missing_nodes: u64,
    pub state_scan_branches_seen: u64,
    pub state_scan_duplicate_missing_hashes: u64,
    pub state_scan_full_below_hits: u64,
    pub state_scan_loaded_or_cached_children: u64,
    pub state_scan_pending_reads: u64,
    pub state_scan_max_pending_reads: u64,
    pub state_scan_pending_hits: u64,
    pub state_scan_pending_misses: u64,
    pub state_scan_deferred_resumes: u64,
    pub state_scan_yields: u64,
    pub state_scan_continuations: u64,
    pub timeout_dispatches: u64,
    pub state_scan_max_buffered_packets: u64,
    pub data_drain_runs: u64,
    pub data_drain_us: u64,
    pub data_drain_max_us: u64,
    pub data_drain_max_packets: u64,
    pub tx_scan_us: u64,
    pub worker_jobs: u64,
    pub worker_queue_wait_us: u64,
    pub node_store_fetch_hits: u64,
    pub node_store_fetch_misses: u64,
    pub tracked_peers: usize,
    pub buffered_packets: usize,
    pub buffered_packets_high_water: usize,
    pub mailbox_token: &'static str,
    pub scan_continuation_pending: bool,
    pub pending_admitted_timeouts: u32,
    pub has_active_packet: bool,
}

struct WorkerNodeFetcher {
    node_store: SHAMapStoreNodeStore,
    stats: Arc<AcquisitionStats>,
}

impl shamap::family::SHAMapNodeFetcher for WorkerNodeFetcher {
    fn fetch_node_object(
        &self,
        hash: SHAMapHash,
        ledger_seq: u32,
    ) -> Option<shamap::node_object::NodeObject> {
        let fetched = match &self.node_store {
            SHAMapStoreNodeStore::Single(db) => db.fetch_node_object(
                hash.as_uint256(),
                ledger_seq,
                nodestore::FetchType::Synchronous,
                false,
            ),
            SHAMapStoreNodeStore::Rotating(db) => db.fetch_node_object(
                hash.as_uint256(),
                ledger_seq,
                nodestore::FetchType::Synchronous,
                false,
            ),
        };
        self.stats.record_node_store_fetch(fetched.is_some());
        let fetched = fetched?;
        let object_type = match fetched.object_type() {
            nodestore::NodeObjectType::AccountNode => shamap::storage::NodeObjectType::AccountNode,
            nodestore::NodeObjectType::TransactionNode => {
                shamap::storage::NodeObjectType::TransactionNode
            }
            nodestore::NodeObjectType::Ledger => shamap::storage::NodeObjectType::Ledger,
            _ => shamap::storage::NodeObjectType::Unknown,
        };
        Some(shamap::node_object::NodeObject::new(
            object_type,
            fetched.data().to_vec(),
            *fetched.hash(),
        ))
    }
}

/// Synchronous node-store adapter matching rippled's `AccountStateSF::gotNode`
/// and `TransactionStateSF::gotNode`, which call `db_.store(...)` unconditionally
/// on every accepted node with no dedup gate. NuDB's own bucket lookup safely
/// no-ops on a true duplicate key.
#[derive(Clone)]
pub struct WorkerStore {
    node_store: SHAMapStoreNodeStore,
    stats: Arc<AcquisitionStats>,
    write_error: Option<String>,
}

impl WorkerStore {
    fn sync(&self) -> Result<(), String> {
        match &self.node_store {
            SHAMapStoreNodeStore::Single(db) => db.sync_result(),
            SHAMapStoreNodeStore::Rotating(db) => db.sync_result(),
        }
    }

    fn take_write_error(&mut self) -> Option<String> {
        self.write_error.take()
    }

    fn store_object(
        &mut self,
        object_type: nodestore::NodeObjectType,
        data: Vec<u8>,
        hash: Uint256,
        seq: u32,
    ) {
        let result = match &self.node_store {
            SHAMapStoreNodeStore::Single(db) => db.store(object_type, data, hash, seq),
            SHAMapStoreNodeStore::Rotating(db) => db.store(object_type, data, hash, seq),
        };
        if let Err(error) = result {
            let message = error.to_string();
            tracing::error!(target: "nodestore", %error, "Failed to persist acquired SHAMap node");
            self.write_error.get_or_insert(message);
        }
    }
}

impl InboundLedgerStore for WorkerStore {
    fn fetch_ledger_header(&mut self, hash: SHAMapHash, _seq: u32) -> Option<Vec<u8>> {
        let fetched = match &self.node_store {
            SHAMapStoreNodeStore::Single(db) => db.fetch_node_object(
                hash.as_uint256(),
                0,
                nodestore::FetchType::Synchronous,
                false,
            ),
            SHAMapStoreNodeStore::Rotating(db) => db.fetch_node_object(
                hash.as_uint256(),
                0,
                nodestore::FetchType::Synchronous,
                false,
            ),
        };
        self.stats.record_node_store_fetch(fetched.is_some());
        let fetched = fetched?;
        Some(fetched.data().to_vec())
    }

    fn store_ledger_header(&mut self, data: Vec<u8>, hash: SHAMapHash, seq: u32) {
        self.store_object(
            nodestore::NodeObjectType::Ledger,
            data,
            *hash.as_uint256(),
            seq,
        );
    }

    fn store_shamap_node(
        &mut self,
        object_type: shamap::storage::NodeObjectType,
        data: Vec<u8>,
        hash: Uint256,
        seq: u32,
    ) {
        let object_type = match object_type {
            shamap::storage::NodeObjectType::AccountNode => nodestore::NodeObjectType::AccountNode,
            shamap::storage::NodeObjectType::TransactionNode => {
                nodestore::NodeObjectType::TransactionNode
            }
            shamap::storage::NodeObjectType::Ledger => nodestore::NodeObjectType::Ledger,
            _ => nodestore::NodeObjectType::Unknown,
        };
        self.store_object(object_type, data, hash, seq);
    }

    fn should_store_hash(&mut self, _hash: Uint256) -> bool {
        true
    }

    fn fetch_node_data(&self, hash: Uint256) -> Option<basics::blob::Blob> {
        let fetched = match &self.node_store {
            SHAMapStoreNodeStore::Single(db) => {
                db.fetch_node_object(&hash, 0, nodestore::FetchType::Synchronous, false)
            }
            SHAMapStoreNodeStore::Rotating(db) => {
                db.fetch_node_object(&hash, 0, nodestore::FetchType::Synchronous, false)
            }
        };
        self.stats.record_node_store_fetch(fetched.is_some());
        let fetched = fetched?;
        Some(fetched.data().to_vec())
    }
}

pub struct AcqMutableState {
    pub inbound: InboundLedgerLocal,
    pub store: WorkerStore,
    pub(crate) fetch_pack: WorkerFetchPack,
}

/// Matches rippled `SHAMap::getMissingNodes()`'s 512 deferred NodeStore-read
/// batch for a resumable inbound state scan. Every admitted read is still
/// synchronously completed before this continuation can be restored.
const STATE_SCAN_MAX_DEFERRED_READS_PER_TURN: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcquisitionWorkToken {
    Idle,
    Queued,
    Running,
}

impl AcquisitionWorkToken {
    fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Queued => "queued",
            Self::Running => "running",
        }
    }
}

/// Scan state is retained in the mailbox until its acquisition turn. A
/// nonterminal advance restores it only after a genuine zero-progress pass;
/// every locally completed deferred-read batch resumes in that same turn.
struct ResumableStateScan {
    params: StateScanParams,
    scan: DeferredMissingNodeScan,
    peer: Option<Arc<dyn Peer>>,
}

/// The sole synchronization point for ingress and acquisition scheduling.
/// A token is either idle, queued, or running. A persisted scan continuation
/// is itself work, so `finish_acquisition_turn` never returns the token idle
/// while packets, timeouts, or a continuation are present.
struct AcquisitionMailbox {
    packets: Vec<(u64, InboundLedgerPacket)>,
    token: AcquisitionWorkToken,
    pending_timeouts: u32,
    scan: Option<ResumableStateScan>,
    // Mirrors rippled `runData`: retain useful-peer scores until the
    // coalesced FIFO batch is observed empty, then prune/sample once before
    // emitting reply triggers. A packet arriving after that observation is a
    // new mailbox batch and cannot be folded into the already-selected peers.
    batch_useful_peer_counts: BTreeMap<u64, i32>,
    buffered_packets_high_water: usize,
}

impl Default for AcquisitionMailbox {
    fn default() -> Self {
        Self {
            packets: Vec::new(),
            token: AcquisitionWorkToken::Idle,
            pending_timeouts: 0,
            scan: None,
            batch_useful_peer_counts: BTreeMap::new(),
            buffered_packets_high_water: 0,
        }
    }
}

impl AcquisitionMailbox {
    fn buffered_packet_count(&self) -> usize {
        self.packets.len()
    }

    fn has_work(&self, fetch_pack_ready: bool) -> bool {
        !self.packets.is_empty()
            || self.pending_timeouts != 0
            || self.scan.is_some()
            || fetch_pack_ready
    }

    fn clear_terminal_work(&mut self) {
        self.packets.clear();
        self.pending_timeouts = 0;
        self.scan = None;
        self.batch_useful_peer_counts.clear();
        self.token = AcquisitionWorkToken::Idle;
    }
}

/// Per-ledger state owned by the registry.
pub struct AcquisitionState {
    mailbox: Mutex<AcquisitionMailbox>,
    #[cfg(test)]
    state_scan_after_advance_pause: Mutex<Option<Arc<StateScanAfterAdvancePause>>>,
    pub mutable: Mutex<AcqMutableState>,
    pub hash: SHAMapHash,
    pub acquisition_id: u64,
    pub seq: u32,
    pub reason: AcquireReason,
    pub peer_set: overlay::SimplePeerSet,
    peer_provider: AcquisitionPeerProvider,
    stats: Arc<AcquisitionStats>,
    pub worker_full_below: FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>,
    pub node_store: SHAMapStoreNodeStore,
    pub shared_tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    pub store_tx: std::sync::mpsc::SyncSender<CompletedInboundLedger>,
    failure_recorder: AcquisitionFailureRecorder,
    completion_recorder: AcquisitionCompletionRecorder,
    pub stopped: AtomicBool,
    pub completed: AtomicBool,
    completed_ledger: Mutex<Option<Arc<Ledger>>>,
    pub failed: AtomicBool,
    // Mirrors rippled InboundLedger::done's signaled_ guard: terminal failure
    // must be claimed before its registry touch/cooldown callback, but not
    // published to poll/sweep consumers until that callback has completed.
    failure_claimed: AtomicBool,
    // Mirrors rippled InboundLedger::done's signaled_ guard: exactly one
    // caller owns expensive successful-terminal finalization.
    finalization_claimed: AtomicBool,
    pub fetch_pack_ready: AtomicBool,
    timer_armed: AtomicBool,
    worker_pool: Arc<WorkerPool>,
    lifecycle: Arc<AcquisitionLifecycleCounters>,
}

impl AcquisitionState {
    /// Initialize an inbound ledger at the acquire boundary.
    ///
    /// This matches `InboundLedger::init` in rippled: after the registry has
    /// installed the per-hash entry, initialization immediately checks local
    /// storage, selects peers, and sends the first requests. Only later packet
    /// processing and timeout callbacks occupy the JtLedgerData-equivalent
    /// queue. In particular, initialization must not consume the timeout
    /// admission budget used by `TimeoutCounter` recovery jobs.
    pub fn start(self: &Arc<Self>) {
        self.lifecycle
            .initialization_jobs
            .fetch_add(1, Ordering::Relaxed);
        self.stats.worker_jobs.fetch_add(1, Ordering::Relaxed);
        // Initialization remains synchronous and precedes its first mailbox
        // token. If it creates a scan, `trigger` installs that continuation
        // before requesting the token; ingress that arrives afterward only
        // coalesces behind that established work.
        run_acquisition_job(self, "initialization", || process_init(self));
    }

    /// Request a single runnable acquisition turn. The mailbox lock makes the
    /// idle-to-queued transition atomic with respect to ingress, timer jobs,
    /// and scan continuation persistence; all later events coalesce behind the
    /// same token.
    pub fn submit_data_job(self: &Arc<Self>) {
        if self.request_acquisition_turn() {
            self.enqueue_acquisition_turn();
        }
    }

    /// Append a routed response and schedule only when the shared work token
    /// moves from idle to queued. Router ingress never touches the mutable
    /// planner or the SyncTree.
    pub fn enqueue_packet(self: &Arc<Self>, peer_id: u64, packet: InboundLedgerPacket) {
        if self.is_done() {
            return;
        }
        let (should_enqueue, buffered_during_scan) = {
            let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
            mailbox.packets.push((peer_id, packet));
            mailbox.buffered_packets_high_water = mailbox
                .buffered_packets_high_water
                .max(mailbox.buffered_packet_count());
            let buffered_during_scan = mailbox.scan.as_ref().map(|_| mailbox.buffered_packet_count());
            let should_enqueue = if mailbox.token == AcquisitionWorkToken::Idle {
                mailbox.token = AcquisitionWorkToken::Queued;
                true
            } else {
                self.lifecycle
                    .data_jobs_coalesced
                    .fetch_add(1, Ordering::Relaxed);
                false
            };
            (should_enqueue, buffered_during_scan)
        };
        if let Some(buffered_packets) = buffered_during_scan {
            self.stats.record_state_scan_buffered_packets(buffered_packets);
        }
        if should_enqueue {
            self.enqueue_acquisition_turn();
        }
    }

    fn request_acquisition_turn(&self) -> bool {
        if self.is_done() {
            return false;
        }
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        if mailbox.token == AcquisitionWorkToken::Idle {
            mailbox.token = AcquisitionWorkToken::Queued;
            true
        } else {
            self.lifecycle
                .data_jobs_coalesced
                .fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    fn enqueue_acquisition_turn(self: &Arc<Self>) {
        self.lifecycle
            .data_jobs_submitted
            .fetch_add(1, Ordering::Relaxed);
        let state = Arc::clone(self);
        let queued_at = Instant::now();
        self.worker_pool.submit_ledger_data(Box::new(move || {
            state
                .lifecycle
                .data_jobs_started
                .fetch_add(1, Ordering::Relaxed);
            state.stats.worker_jobs.fetch_add(1, Ordering::Relaxed);
            state
                .stats
                .worker_queue_wait_us
                .fetch_add(queued_at.elapsed().as_micros() as u64, Ordering::Relaxed);
            run_acquisition_job(&state, "mailbox", || process_acquisition_turn(&state));
        }));
    }

    fn begin_acquisition_turn(&self) -> bool {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        if mailbox.token != AcquisitionWorkToken::Queued {
            return false;
        }
        mailbox.token = AcquisitionWorkToken::Running;
        true
    }

    /// Return a running token to queued only when work was observed under the
    /// same mailbox lock. Ingress that arrives before the observation is seen;
    /// ingress that arrives after it sees queued or idle and schedules itself.
    fn finish_acquisition_turn(self: &Arc<Self>) {
        let should_enqueue = {
            let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
            if self.is_done() {
                mailbox.clear_terminal_work();
                false
            } else if mailbox.has_work(self.fetch_pack_ready.load(Ordering::Acquire)) {
                mailbox.token = AcquisitionWorkToken::Queued;
                true
            } else {
                mailbox.token = AcquisitionWorkToken::Idle;
                false
            }
        };
        if should_enqueue {
            self.enqueue_acquisition_turn();
        }
    }

    fn take_packet_for_turn(&self) -> Option<(u64, InboundLedgerPacket)> {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        (!mailbox.packets.is_empty()).then(|| mailbox.packets.remove(0))
    }

    /// Record one completed packet's useful nodes and, only after observing
    /// its coalesced FIFO queue empty under this same mailbox lock,
    /// prune/sample the entire batch for reply triggers.
    fn finish_packet_batch(&self, peer_id: u64, useful_nodes: u64) -> Vec<u64> {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        if self.is_done() {
            mailbox.clear_terminal_work();
            return Vec::new();
        }
        if useful_nodes != 0 {
            let useful_nodes = useful_nodes.min(i32::MAX as u64) as i32;
            mailbox
                .batch_useful_peer_counts
                .entry(peer_id)
                .and_modify(|count| *count = (*count).max(useful_nodes))
                .or_insert(useful_nodes);
        }

        if !mailbox.packets.is_empty() {
            return Vec::new();
        }

        let scores: Vec<_> = mailbox
            .batch_useful_peer_counts
            .iter()
            .map(|(&peer_id, &useful_count)| InboundLedgerPeerScore {
                peer_id,
                useful_count,
            })
            .collect();
        mailbox.batch_useful_peer_counts.clear();
        select_inbound_ledger_reply_peers(&scores)
    }

    fn take_admitted_timeout(&self) -> bool {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        if mailbox.pending_timeouts == 0 {
            return false;
        }
        mailbox.pending_timeouts -= 1;
        true
    }

    fn record_admitted_timeout(self: &Arc<Self>) {
        if self.is_done() {
            return;
        }
        let (should_enqueue, pending_timeouts, scan_pending, buffered_packets) = {
            let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
            mailbox.pending_timeouts = mailbox.pending_timeouts.saturating_add(1);
            let should_enqueue = if mailbox.token == AcquisitionWorkToken::Idle {
                mailbox.token = AcquisitionWorkToken::Queued;
                true
            } else {
                false
            };
            (
                should_enqueue,
                mailbox.pending_timeouts,
                mailbox.scan.is_some(),
                mailbox.buffered_packet_count(),
            )
        };
        if self.stats.should_emit_sampled_diagnostic() {
            tracing::debug!(
                target: "inbound_ledger",
                seq = self.seq,
                hash = %self.hash,
                pending_timeouts,
                scan_pending,
                buffered_packets,
                "sampled admitted timeout parked in acquisition mailbox"
            );
        }
        if should_enqueue {
            self.enqueue_acquisition_turn();
        }
    }

    fn install_state_scan(&self, scan: ResumableStateScan) -> bool {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        if self.is_done() {
            mailbox.clear_terminal_work();
            return false;
        }
        assert!(mailbox.scan.is_none(), "one acquisition may own only one state scan");
        mailbox.scan = Some(scan);
        self.stats
            .state_scan_continuations
            .fetch_add(1, Ordering::Relaxed);
        true
    }

    fn take_state_scan(&self) -> Option<ResumableStateScan> {
        self.mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .scan
            .take()
    }

    #[cfg(test)]
    fn pause_after_state_scan_advance_for_test(&self) {
        if let Some(pause) = self
            .state_scan_after_advance_pause
            .lock()
            .expect("state scan pause slot lock")
            .clone()
        {
            pause.pause_after_advance();
        }
    }

    fn restore_state_scan(&self, scan: ResumableStateScan) {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        if self.is_done() {
            mailbox.clear_terminal_work();
            return;
        }
        assert!(mailbox.scan.is_none(), "state scan continuation must have one owner");
        mailbox.scan = Some(scan);
        self.stats.state_scan_yields.fetch_add(1, Ordering::Relaxed);
        self.stats
            .state_scan_continuations
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Merge a trigger into a persisted scan without weakening request
    /// semantics. Priority is `Timeout > ReplyHighLatency > Reply > Added >
    /// Blind`; an equal non-timeout trigger keeps the newest peer target. A
    /// Timeout is special: it is always broadcast (`peer = None`) and forces
    /// `query_type = qtINDIRECT`, so later peer-specific triggers cannot turn
    /// a timeout retry into a directed, duplicate-suppressed reply request.
    fn retarget_state_scan(
        &self,
        reason: InboundLedgerRequestTrigger,
        peer: Option<Arc<dyn Peer>>,
    ) -> bool {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        let Some(scan) = mailbox.scan.as_mut() else {
            return false;
        };

        if reason == InboundLedgerRequestTrigger::Timeout {
            scan.params.reason = InboundLedgerRequestTrigger::Timeout;
            scan.params.query_depth = 0;
            scan.params.query_type = Some(0);
            scan.peer = None;
            return true;
        }
        if scan.params.reason == InboundLedgerRequestTrigger::Timeout {
            return true;
        }

        let priority = |trigger| match trigger {
            InboundLedgerRequestTrigger::Timeout => 4,
            InboundLedgerRequestTrigger::ReplyHighLatency => 3,
            InboundLedgerRequestTrigger::Reply => 2,
            InboundLedgerRequestTrigger::Added => 1,
            InboundLedgerRequestTrigger::Blind => 0,
        };
        if priority(reason) >= priority(scan.params.reason) {
            scan.params.reason = reason;
            scan.params.query_depth = match reason {
                InboundLedgerRequestTrigger::Reply => 1,
                InboundLedgerRequestTrigger::ReplyHighLatency => 2,
                InboundLedgerRequestTrigger::Timeout
                | InboundLedgerRequestTrigger::Added
                | InboundLedgerRequestTrigger::Blind => 0,
            };
            scan.peer = peer;
        }
        true
    }

    fn has_state_scan(&self) -> bool {
        self.mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .scan
            .is_some()
    }

    fn queue_timeout_job(self: &Arc<Self>) {
        if self.is_done() {
            return;
        }
        let state = Arc::clone(self);
        if !self.worker_pool.try_submit_timeout(Box::new(move || {
            // Admission was decided by WorkerPool's unchanged aggregate < 5
            // gate. Delivery is mailbox work so a scan can never discard it.
            state.record_admitted_timeout();
        })) {
            self.lifecycle
                .timeout_queue_rejected
                .fetch_add(1, Ordering::Relaxed);
            if self.stats.should_emit_sampled_diagnostic() {
                let worker = self.worker_pool.snapshot();
                tracing::debug!(
                    target: "inbound_ledger",
                    seq = self.seq,
                    hash = %self.hash,
                    queued_jobs = worker.queued_jobs,
                    outstanding_jobs = worker.outstanding_ledger_data_jobs,
                    job_limit = worker.ledger_data_job_limit,
                    timeout_attempts = worker.timeout_submission_attempts,
                    timeout_rejections = worker.timeout_submission_rejected,
                    "sampled timeout admission rejected"
                );
            }
            self.arm_timer();
        }
    }

    fn arm_timer(self: &Arc<Self>) {
        if self.is_done()
            || self
                .timer_armed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
        let state = Arc::clone(self);
        self.worker_pool.schedule_after(
            ACQUIRE_TIMEOUT,
            Box::new(move || {
                state.timer_armed.store(false, Ordering::Release);
                state.queue_timeout_job();
            }),
        );
    }

    fn refresh_peers(&self) {
        self.peer_set.refresh_peers((self.peer_provider)());
    }

    pub(crate) fn diagnostics(&self) -> AcquisitionSnapshot {
        let (seq, planner, timeouts) = self
            .lock_mutable("diagnostics")
            .map(|mutable| {
                (
                    mutable.inbound.seq(),
                    mutable.inbound.planner_state(),
                    mutable.inbound.timeout_count(),
                )
            })
            .unwrap_or((self.seq, Default::default(), 0));
        let (
            has_active_packet,
            buffered_packets,
            buffered_packets_high_water,
            mailbox_token,
            scan_continuation_pending,
            pending_admitted_timeouts,
        ) = {
            let mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
            (
                false,
                mailbox.buffered_packet_count(),
                mailbox.buffered_packets_high_water,
                mailbox.token.name(),
                mailbox.scan.is_some(),
                mailbox.pending_timeouts,
            )
        };
        let header_after_ms = self
            .stats
            .first_header_at
            .lock()
            .expect("acquisition first_header_at lock")
            .map(|at| at.duration_since(self.stats.started_at).as_millis() as u64);
        AcquisitionSnapshot {
            age_ms: self.stats.started_at.elapsed().as_millis() as u64,
            header_after_ms,
            seq,
            have_header: planner.have_header,
            have_state: planner.have_state,
            have_transactions: planner.have_transactions,
            timeouts,
            packets: self.stats.packets.load(Ordering::Relaxed),
            useful_packets: self.stats.useful_packets.load(Ordering::Relaxed),
            useful_nodes: self.stats.useful_nodes.load(Ordering::Relaxed),
            state_packets: self.stats.state_packets.load(Ordering::Relaxed),
            state_useful_nodes: self.stats.state_useful_nodes.load(Ordering::Relaxed),
            state_duplicate_nodes: self.stats.state_duplicate_nodes.load(Ordering::Relaxed),
            malformed_packets: self.stats.malformed_packets.load(Ordering::Relaxed),
            state_scan_runs: self.stats.state_scan_runs.load(Ordering::Relaxed),
            state_missing_nodes: self.stats.state_missing_nodes.load(Ordering::Relaxed),
            tx_missing_nodes: self.stats.tx_missing_nodes.load(Ordering::Relaxed),
            state_scan_us: self.stats.state_scan_us.load(Ordering::Relaxed),
            state_scan_branch_steps: self.stats.state_scan_branch_steps.load(Ordering::Relaxed),
            state_scan_missing_nodes_recorded: self
                .stats
                .state_scan_missing_nodes_recorded
                .load(Ordering::Relaxed),
            state_scan_positive_progress_slices: self
                .stats
                .state_scan_positive_progress_slices
                .load(Ordering::Relaxed),
            state_scan_branch_budget_yields: self
                .stats
                .state_scan_branch_budget_yields
                .load(Ordering::Relaxed),
            state_scan_deferred_read_budget_yields: self
                .stats
                .state_scan_deferred_read_budget_yields
                .load(Ordering::Relaxed),
            state_scan_deferred_read_resume_yields: self
                .stats
                .state_scan_deferred_read_resume_yields
                .load(Ordering::Relaxed),
            state_scan_missing_node_limit_yields: self
                .stats
                .state_scan_missing_node_limit_yields
                .load(Ordering::Relaxed),
            state_scan_completed_slices: self
                .stats
                .state_scan_completed_slices
                .load(Ordering::Relaxed),
            state_scan_last_yield: StateScanSliceOutcome::from_raw(
                self.stats.state_scan_last_outcome.load(Ordering::Relaxed),
            )
            .name(),
            state_scan_last_branch_steps: self
                .stats
                .state_scan_last_branch_steps
                .load(Ordering::Relaxed),
            state_scan_last_deferred_reads: self
                .stats
                .state_scan_last_deferred_reads
                .load(Ordering::Relaxed),
            state_scan_last_deferred_resumes: self
                .stats
                .state_scan_last_deferred_resumes
                .load(Ordering::Relaxed),
            state_scan_last_missing_nodes: self
                .stats
                .state_scan_last_missing_nodes
                .load(Ordering::Relaxed),
            state_scan_branches_seen: self.stats.state_scan_branches_seen.load(Ordering::Relaxed),
            state_scan_duplicate_missing_hashes: self
                .stats
                .state_scan_duplicate_missing_hashes
                .load(Ordering::Relaxed),
            state_scan_full_below_hits: self
                .stats
                .state_scan_full_below_hits
                .load(Ordering::Relaxed),
            state_scan_loaded_or_cached_children: self
                .stats
                .state_scan_loaded_or_cached_children
                .load(Ordering::Relaxed),
            state_scan_pending_reads: self.stats.state_scan_pending_reads.load(Ordering::Relaxed),
            state_scan_max_pending_reads: self
                .stats
                .state_scan_max_pending_reads
                .load(Ordering::Relaxed),
            state_scan_pending_hits: self.stats.state_scan_pending_hits.load(Ordering::Relaxed),
            state_scan_pending_misses: self.stats.state_scan_pending_misses.load(Ordering::Relaxed),
            state_scan_deferred_resumes: self
                .stats
                .state_scan_deferred_resumes
                .load(Ordering::Relaxed),
            state_scan_yields: self.stats.state_scan_yields.load(Ordering::Relaxed),
            state_scan_continuations: self
                .stats
                .state_scan_continuations
                .load(Ordering::Relaxed),
            timeout_dispatches: self.stats.timeout_dispatches.load(Ordering::Relaxed),
            state_scan_max_buffered_packets: self
                .stats
                .state_scan_max_buffered_packets
                .load(Ordering::Relaxed),
            data_drain_runs: self.stats.data_drain_runs.load(Ordering::Relaxed),
            data_drain_us: self.stats.data_drain_us.load(Ordering::Relaxed),
            data_drain_max_us: self.stats.data_drain_max_us.load(Ordering::Relaxed),
            data_drain_max_packets: self.stats.data_drain_max_packets.load(Ordering::Relaxed),
            tx_scan_us: self.stats.tx_scan_us.load(Ordering::Relaxed),
            worker_jobs: self.stats.worker_jobs.load(Ordering::Relaxed),
            worker_queue_wait_us: self.stats.worker_queue_wait_us.load(Ordering::Relaxed),
            node_store_fetch_hits: self.stats.node_store_fetch_hits.load(Ordering::Relaxed),
            node_store_fetch_misses: self.stats.node_store_fetch_misses.load(Ordering::Relaxed),
            tracked_peers: self.peer_set.peer_count(),
            buffered_packets,
            buffered_packets_high_water,
            mailbox_token,
            scan_continuation_pending,
            pending_admitted_timeouts,
            has_active_packet,
        }
    }

    fn is_done(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
            || self.completed.load(Ordering::Acquire)
            || self.failed.load(Ordering::Acquire)
    }

    /// Refuse to continue an acquisition after an unwind poisoned its mutable
    /// planner state. Dropping the recovered guard first prevents the failure
    /// recorder from re-entering while this mutex remains locked.
    fn lock_mutable(&self, operation: &'static str) -> Option<MutexGuard<'_, AcqMutableState>> {
        match self.mutable.lock() {
            Ok(mutable) => Some(mutable),
            Err(poison) => {
                drop(poison.into_inner());
                tracing::error!(
                    target: "inbound_ledger",
                    operation,
                    seq = self.seq,
                    hash = %self.hash,
                    "acquisition mutable state was poisoned; failing acquisition"
                );
                self.mark_failed();
                None
            }
        }
    }

    fn mark_failed(&self) {
        if self
            .failure_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        // InboundLedger::done touches before dispatching AcqDone. Keep the
        // matching registry entry alive and record its cooldown before any
        // strand poll or sweep can observe the terminal failure flags.
        (self.failure_recorder)(*self.hash.as_uint256());
        self.failed.store(true, Ordering::Release);
        self.stopped.store(true, Ordering::Release);
        self.lifecycle
            .terminal_failed
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn take_buffered_packets(&self) -> Vec<ledger::InboundLedgerReceivedPacket> {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        let packets = std::mem::take(&mut mailbox.packets);
        packets
            .into_iter()
            .map(|(peer_id, packet)| ledger::InboundLedgerReceivedPacket::new(Some(peer_id), packet))
            .collect()
    }

    fn has_pending_packets(&self) -> bool {
        let mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        !mailbox.packets.is_empty()
    }

    pub(crate) fn update_seq(&self, seq: u32) {
        let Some(mut mutable) = self.lock_mutable("update sequence") else {
            return;
        };
        mutable.inbound.update(seq, time::Duration::ZERO);
    }

    pub(crate) fn completed_ledger(&self) -> Option<Arc<Ledger>> {
        if let Some(ledger) = self
            .completed_ledger
            .lock()
            .expect("acquisition completed ledger lock")
            .clone()
        {
            return Some(ledger);
        }
        self.lock_mutable("completed ledger")?
            .inbound
            .ledger()
            .cloned()
            .map(Arc::new)
    }
}

pub struct AcquisitionBuilder {
    pub hash: SHAMapHash,
    pub acquisition_id: u64,
    pub seq: u32,
    pub reason: AcquireReason,
    pub node_store: SHAMapStoreNodeStore,
    pub tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    pub fetch_pack: Arc<FetchPackCache>,
    pub store_tx: std::sync::mpsc::SyncSender<CompletedInboundLedger>,
    pub failure_recorder: AcquisitionFailureRecorder,
    pub completion_recorder: AcquisitionCompletionRecorder,
    pub full_below_generation: u32,
    pub worker_pool: Arc<WorkerPool>,
    pub initial_peers: Vec<Arc<dyn Peer>>,
    pub peer_provider: AcquisitionPeerProvider,
    pub lifecycle: Arc<AcquisitionLifecycleCounters>,
}

impl AcquisitionBuilder {
    pub fn build(self) -> Arc<AcquisitionState> {
        let reason = match self.reason {
            AcquireReason::History => InboundLedgerReason::History,
            AcquireReason::Generic => InboundLedgerReason::Generic,
            AcquireReason::Consensus => InboundLedgerReason::Consensus,
        };
        let stats = Arc::new(AcquisitionStats::new());
        Arc::new(AcquisitionState {
            mailbox: Mutex::new(AcquisitionMailbox::default()),
            #[cfg(test)]
            state_scan_after_advance_pause: Mutex::new(None),
            mutable: Mutex::new(AcqMutableState {
                inbound: InboundLedgerLocal::new_with_reason(self.hash, self.seq, reason),
                store: WorkerStore {
                    node_store: self.node_store.clone(),
                    stats: Arc::clone(&stats),
                    write_error: None,
                },
                fetch_pack: WorkerFetchPack {
                    cache: self.fetch_pack,
                },
            }),
            hash: self.hash,
            acquisition_id: self.acquisition_id,
            seq: self.seq,
            reason: self.reason,
            peer_set: overlay::SimplePeerSet::new(self.initial_peers),
            peer_provider: self.peer_provider,
            stats,
            worker_full_below: FullBelowCacheImpl::new(
                self.full_below_generation,
                MonotonicClock::default(),
                HardenedHashBuilder::default(),
                524_288,
            ),
            node_store: self.node_store,
            shared_tree_cache: self.tree_cache,
            store_tx: self.store_tx,
            failure_recorder: self.failure_recorder,
            completion_recorder: self.completion_recorder,
            stopped: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            completed_ledger: Mutex::new(None),
            failed: AtomicBool::new(false),
            failure_claimed: AtomicBool::new(false),
            finalization_claimed: AtomicBool::new(false),
            fetch_pack_ready: AtomicBool::new(false),
            timer_armed: AtomicBool::new(false),
            worker_pool: self.worker_pool,
            lifecycle: self.lifecycle,
        })
    }
}

fn run_acquisition_job(state: &Arc<AcquisitionState>, operation: &'static str, job: impl FnOnce()) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(job)) {
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                payload
                    .downcast_ref::<&str>()
                    .map(|message| (*message).to_owned())
            })
            .unwrap_or_else(|| "non-string panic payload".to_owned());
        tracing::error!(
            target: "inbound_ledger",
            operation,
            seq = state.seq,
            hash = %state.hash,
            %message,
            "acquisition job panicked; failing acquisition"
        );
        state.mark_failed();
        state.finish_acquisition_turn();
    }
}

fn family<'a>(
    state: &'a AcquisitionState,
) -> SHAMapFamily<
    MonotonicClock,
    HardenedHashBuilder,
    &'a FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>,
    WorkerNodeFetcher,
    NullMissingNodeReporter,
    (),
> {
    SHAMapFamily::new(
        Arc::clone(&state.shared_tree_cache),
        &state.worker_full_below,
        WorkerNodeFetcher {
            node_store: state.node_store.clone(),
            stats: Arc::clone(&state.stats),
        },
        NullMissingNodeReporter,
    )
}

fn trigger(
    state: &Arc<AcquisitionState>,
    reason: InboundLedgerRequestTrigger,
    peer: Option<Arc<dyn Peer>>,
) {
    if state.is_done() {
        return;
    }
    state
        .lifecycle
        .request_triggers
        .fetch_add(1, Ordering::Relaxed);
    if state.retarget_state_scan(reason, peer.clone()) {
        // A timeout or reply arriving between scan slices updates the eventual
        // request semantics without restarting the expensive traversal.
        return;
    }

    let Some(mut mutable) = state.lock_mutable("trigger") else {
        return;
    };
    let AcqMutableState {
        inbound,
        store,
        fetch_pack,
    } = &mut *mutable;
    let journal = WorkerJournal;
    let config = ledger::LedgerConfig::default();
    let family = family(state);
    let setup = inbound.prepare_trigger(reason, &journal, &config, store, fetch_pack, &family);

    // Registry sweep/remove/clear/stop can become terminal after planner
    // mutation but before this trigger emits its deferred requests. Do not
    // create a scan continuation or send any request once that is observed.
    if state.is_done() {
        return;
    }
    for msg in &setup.messages_to_send {
        if state.is_done() {
            return;
        }
        state
            .lifecycle
            .request_messages
            .fetch_add(1, Ordering::Relaxed);
        state.peer_set.send_request(msg, peer.as_ref());
    }

    if let Some(params) = setup.state_scan {
        if state.is_done() {
            return;
        }
        let Some(scan) = inbound.start_resumable_state_map_scan(&params, &family) else {
            return;
        };
        // The acquisition turn still owns the token. Retain traversal state
        // rather than leasing a Ledger to a whole worker job; the next turn
        // advances this exact stack and pending-read set within fixed bounds.
        if state.install_state_scan(ResumableStateScan { params, scan, peer }) {
            state.submit_data_job();
        }
        return;
    }

    // Preserve the normal transaction-only fallback when state was already
    // complete, invalid, or setup emitted a state-root request.
    let state_request_sent = setup.state_request_pending;
    if !state_request_sent {
        if state.is_done() {
            return;
        }
        if let Some(params) = setup.tx_scan.as_ref() {
            let tx_scan_started = Instant::now();
            let tx_missing = inbound.do_tx_map_scan(params, store, fetch_pack, &family);
            state
                .stats
                .tx_missing_nodes
                .store(tx_missing.len() as u64, Ordering::Relaxed);
            state.stats.tx_scan_us.fetch_add(
                tx_scan_started.elapsed().as_micros() as u64,
                Ordering::Relaxed,
            );
            if state.is_done() {
                return;
            }
            let mut send_fn = |message: overlay::ProtocolMessage| {
                if state.is_done() {
                    return;
                }
                state
                    .lifecycle
                    .request_messages
                    .fetch_add(1, Ordering::Relaxed);
                let target = if matches!(message.payload, overlay::ProtocolPayload::GetObjects(_)) {
                    None
                } else {
                    peer.as_ref()
                };
                state.peer_set.send_request(&message, target);
            };
            inbound.apply_tx_scan_results(tx_missing, params, &family, &mut send_fn);
        }
    }
    if inbound.planner_state().have_header
        && inbound.planner_state().have_state
        && inbound.planner_state().have_transactions
    {
        inbound.set_complete();
    }
    let terminal = inbound.is_failed() || inbound.is_complete();
    drop(mutable);
    if terminal {
        finalize_terminal(state);
    }
}

fn peer_has_acquisition_target(peer: &Arc<dyn Peer>, hash: Uint256, seq: u32) -> bool {
    if peer.has_ledger(hash, seq) {
        return true;
    }

    // A hash-only closed-ledger request deliberately has no sequence claim.
    // Prefer peers that explicitly advertise the hash, but do not make that
    // advertisement a routing precondition: trusted validation can name a
    // preferred LCL before a peer's StatusChange reaches us. The wire request
    // still carries the exact hash and response routing keeps its hash and
    // known-sequence checks, so this broadens discovery without weakening
    // target identity.
    seq == 0 || peer.closed_ledger_hash() == hash
}

fn add_peers(state: &AcquisitionState) -> Vec<Arc<dyn Peer>> {
    // Rippled's PeerSet walks the live overlay on every addPeers call. Refresh
    // immediately before scoring so an acquisition started before a usable
    // peer connected can recruit that peer on the next no-progress turn.
    state.refresh_peers();
    let limit = if state.peer_set.peer_count() == 0 {
        PEER_COUNT_START
    } else {
        PEER_COUNT_ADD
    };
    let hash = *state.hash.as_uint256();
    let mut added = Vec::new();
    state.peer_set.add_peers(
        limit,
        &mut |peer| peer_has_acquisition_target(peer, hash, state.seq),
        &mut |peer| added.push(Arc::clone(peer)),
    );
    state
        .lifecycle
        .peers_added
        .fetch_add(added.len() as u64, Ordering::Relaxed);
    added
}

fn check_local(state: &AcquisitionState, mutable: &mut AcqMutableState) {
    let AcqMutableState {
        inbound,
        store,
        fetch_pack,
    } = mutable;
    let journal = WorkerJournal;
    let config = ledger::LedgerConfig::default();
    let family = family(state);
    inbound.check_local_with_family_and_config(&journal, &config, store, fetch_pack, &family);
}

fn process_init(state: &Arc<AcquisitionState>) {
    if state.is_done() {
        return;
    }
    let added = {
        let Some(mut mutable) = state.lock_mutable("initialization") else {
            return;
        };
        check_local(state, &mut mutable);
        if mutable.inbound.is_failed() || mutable.inbound.is_complete() {
            drop(mutable);
            finalize_terminal(state);
            return;
        }
        add_peers(state)
    };
    if state.reason != AcquireReason::History {
        for peer in added {
            trigger(state, InboundLedgerRequestTrigger::Added, Some(peer));
        }
    }
    state.queue_timeout_job();
}

fn process_acquisition_turn(state: &Arc<AcquisitionState>) {
    if !state.begin_acquisition_turn() {
        return;
    }
    if state.is_done() {
        state.finish_acquisition_turn();
        return;
    }

    // Timeout bookkeeping remains first. Then mirror rippled `runData()` by
    // draining the FIFO batch already coalesced behind this one acquisition
    // token before state scanning. A fetch-pack-only pass remains singular so
    // a no-packet check cannot create an accidental unbounded loop.
    if state.take_admitted_timeout() {
        process_timeout_job(state);
    }
    if !state.is_done() && state.has_pending_packets() {
        while !state.is_done() && state.has_pending_packets() {
            process_data_job(state);
        }
    } else if !state.is_done() && state.fetch_pack_ready.load(Ordering::Acquire) {
        process_data_job(state);
    }
    if !state.is_done() && state.has_state_scan() {
        process_state_scan_turn(state);
    }
    state.finish_acquisition_turn();
}

fn process_data_job(state: &Arc<AcquisitionState>) {
    if state.is_done() {
        return;
    }

    // The FIFO owner removes one whole packet per invocation. The enclosing
    // acquisition turn drains its coalesced batch, matching `InboundLedger::runData()`
    // without retaining a 128-node continuation in the mailbox.
    let Some((peer_id, packet)) = state.take_packet_for_turn() else {
        let Some(mut mutable) = state.lock_mutable("data processing") else {
            return;
        };
        let data_drain_started = Instant::now();
        if state.fetch_pack_ready.swap(false, Ordering::AcqRel) {
            check_local(state, &mut mutable);
        }
        let terminal = mutable.inbound.is_failed() || mutable.inbound.is_complete();
        drop(mutable);
        state
            .stats
            .record_data_drain(data_drain_started.elapsed().as_micros() as u64, 0);
        if terminal {
            finalize_terminal(state);
        }
        return;
    };

    let data_drain_started = Instant::now();
    let packet_type = packet.packet_type;
    let mut packet_stats = SHAMapAddNode::default();
    let mut malformed = None;
    let mut invalid = false;
    let mut had_header = false;
    let terminal;
    {
        let Some(mut mutable) = state.lock_mutable("data processing") else {
            return;
        };
        if state.fetch_pack_ready.swap(false, Ordering::AcqRel) {
            check_local(state, &mut mutable);
        }
        let AcqMutableState {
            inbound,
            store,
            fetch_pack,
        } = &mut *mutable;
        if !(inbound.is_failed() || inbound.is_complete()) {
            had_header = inbound.planner_state().have_header;
            let journal = WorkerJournal;
            let config = ledger::LedgerConfig::default();
            let family = family(state);
            match inbound.process_packet_with_family_and_config(
                &packet,
                &journal,
                &config,
                store,
                fetch_pack,
                &family,
            ) {
                Ok(stats) => {
                    packet_stats = stats;
                    inbound.record_packet_stats_with_family_and_config(
                        packet_stats,
                        &journal,
                        &config,
                        &family,
                    );
                    invalid = packet_stats.is_invalid();
                }
                Err(error) => malformed = Some(error),
            }
        }
        terminal = inbound.is_failed() || inbound.is_complete();
    }

    let data_drain_us = data_drain_started.elapsed().as_micros() as u64;
    state.stats.record_data_drain(data_drain_us, 1);
    if state.stats.should_emit_sampled_diagnostic() {
        tracing::debug!(
            target: "inbound_ledger",
            seq = state.seq,
            hash = %state.hash,
            drain_us = data_drain_us,
            nodes = packet.nodes.len(),
            "sampled full inbound data packet"
        );
    }

    if !had_header && !terminal {
        let has_header = state
            .lock_mutable("data header diagnostics")
            .is_some_and(|mutable| mutable.inbound.planner_state().have_header);
        if has_header {
            state.stats.mark_header_received();
        }
    }
    state.lifecycle.packet_steps.fetch_add(1, Ordering::Relaxed);
    state
        .lifecycle
        .packet_steps_completed
        .fetch_add(1, Ordering::Relaxed);
    state.stats.packets.fetch_add(1, Ordering::Relaxed);
    let useful_nodes = packet_stats.get_good().max(0) as u64;
    state
        .stats
        .useful_packets
        .fetch_add(u64::from(useful_nodes != 0), Ordering::Relaxed);
    state.stats.useful_nodes.fetch_add(useful_nodes, Ordering::Relaxed);
    if packet_type == ledger::InboundLedgerDataType::StateNode {
        state.stats.state_packets.fetch_add(1, Ordering::Relaxed);
        state
            .stats
            .state_useful_nodes
            .fetch_add(useful_nodes, Ordering::Relaxed);
        state.stats.state_duplicate_nodes.fetch_add(
            packet_stats.get_duplicate().max(0) as u64,
            Ordering::Relaxed,
        );
    }
    let packet_error_count = usize::from(malformed.is_some()) + usize::from(invalid);
    state
        .lifecycle
        .packet_step_errors
        .fetch_add(packet_error_count as u64, Ordering::Relaxed);
    if let Some(error) = malformed {
        state.stats.malformed_packets.fetch_add(1, Ordering::Relaxed);
        charge_malformed_packet(state, peer_id, packet_type, error);
    }
    if invalid {
        charge_invalid_data_packet(
            state,
            peer_id,
            packet_type,
            InboundLedgerPacketError::InvalidData,
        );
    }

    if terminal {
        finalize_terminal(state);
        return;
    }
    // A registry terminal transition can race after the packet mutation lock
    // is released. It must suppress this packet's deferred reply triggers;
    // `finish_acquisition_turn` performs the ordinary mailbox cleanup.
    if state.is_done() {
        return;
    }
    // Do not trigger once per completed packet. The mailbox records useful
    // results in FIFO order and returns peers only after it observed the
    // coalesced queue empty under the same lock; a concurrent later arrival
    // becomes the next batch.
    for reply_peer_id in state.finish_packet_batch(peer_id, useful_nodes) {
        if let Some(peer) = state.peer_set.find_peer(reply_peer_id as u32) {
            let reason = if peer.is_high_latency() {
                InboundLedgerRequestTrigger::ReplyHighLatency
            } else {
                InboundLedgerRequestTrigger::Reply
            };
            trigger(state, reason, Some(peer));
        }
    }
}

fn process_state_scan_turn(state: &Arc<AcquisitionState>) {
    let Some(mut continuation) = state.take_state_scan() else {
        return;
    };
    // Once taken, this continuation has exactly two live outcomes: it is
    // restored below only after a nonterminal zero-progress advance, or it is
    // consumed by result application. Every locally completed deferred-read
    // batch resumes within the helper call. Any early return here follows `is_done` or
    // `lock_mutable`, the latter marking failure, and terminal cleanup clears
    // the mailbox instead of reviving the continuation.
    if state.is_done() {
        return;
    }

    let scan_started = Instant::now();
    let progress_before = continuation.scan.progress();
    let family = family(state);
    let complete = {
        let Some(mut mutable) = state.lock_mutable("resumable state scan") else {
            return;
        };
        let AcqMutableState {
            inbound,
            store,
            fetch_pack,
        } = &mut *mutable;
        inbound.advance_resumable_state_map_scan(
            &mut continuation.scan,
            &continuation.params,
            store,
            fetch_pack,
            &family,
            STATE_SCAN_MAX_DEFERRED_READS_PER_TURN,
            usize::MAX,
        )
    };
    #[cfg(test)]
    state.pause_after_state_scan_advance_for_test();

    // These before/after values are sampled after synchronous NodeStore reads
    // have completed, so they describe actual local work and never steer the
    // retained continuation or mailbox token.
    let progress_after = continuation.scan.progress();
    let (slice_outcome, slice_deltas) =
        classify_state_scan_slice(progress_before, progress_after);
    let scan_us = scan_started.elapsed().as_micros() as u64;
    state.stats.state_scan_runs.fetch_add(1, Ordering::Relaxed);
    state.stats.state_scan_us.fetch_add(scan_us, Ordering::Relaxed);
    state
        .stats
        .record_state_scan_slice(slice_outcome, slice_deltas);

    // A registry sweep/remove/clear/stop can set terminal while the exclusive
    // SyncTree advance is in progress. This check is deliberately before any
    // continuation consumption, result application, transaction follow-up,
    // or peer request. Dropping the local continuation lets the normal turn
    // finalizer clear the mailbox; terminal work is never restored.
    if state.is_done() {
        return;
    }

    if !complete {
        if state.is_done() {
            return;
        }
        let pending_reads = continuation.scan.pending_requests().len();
        if state.stats.should_emit_sampled_diagnostic() {
            tracing::debug!(
                target: "inbound_ledger",
                seq = state.seq,
                hash = %state.hash,
                scan_us,
                read_budget = STATE_SCAN_MAX_DEFERRED_READS_PER_TURN,
                pending_reads,
                slice_outcome = slice_outcome.name(),
                branch_steps = slice_deltas.branch_steps,
                deferred_reads = slice_deltas.deferred_reads,
                deferred_resumes = slice_deltas.deferred_resumes,
                missing_nodes = slice_deltas.missing_nodes,
                "sampled resumable state-map scan yielded"
            );
        }
        state.restore_state_scan(continuation);
        return;
    }

    // Do not consume final scan results if terminal was published after the
    // bounded advance but before this final-slice handoff.
    if state.is_done() {
        return;
    }
    let (state_missing, scan_stats) = continuation.scan.into_missing_nodes_and_stats();
    state.stats.record_state_scan(&scan_stats);
    state
        .stats
        .state_missing_nodes
        .store(state_missing.len() as u64, Ordering::Relaxed);
    if state.stats.should_emit_sampled_diagnostic() {
        tracing::debug!(
            target: "inbound_ledger",
            seq = state.seq,
            hash = %state.hash,
            scan_us,
            missing = state_missing.len(),
            branches = scan_stats.branches_seen,
            pending = scan_stats.pending_reads,
            slice_outcome = slice_outcome.name(),
            branch_steps = slice_deltas.branch_steps,
            deferred_reads = slice_deltas.deferred_reads,
            deferred_resumes = slice_deltas.deferred_resumes,
            missing_nodes = slice_deltas.missing_nodes,
            "sampled resumable state-map scan completed"
        );
    }

    let Some(mut mutable) = state.lock_mutable("state scan results") else {
        return;
    };
    // The mutable lock can be contended while registry terminal state changes,
    // so recheck directly before applying the already-computed scan results.
    if state.is_done() {
        return;
    }
    let mut send_fn = |message: overlay::ProtocolMessage| {
        if state.is_done() {
            return;
        }
        state
            .lifecycle
            .request_messages
            .fetch_add(1, Ordering::Relaxed);
        let target = if matches!(message.payload, overlay::ProtocolPayload::GetObjects(_)) {
            None
        } else {
            continuation.peer.as_ref()
        };
        state.peer_set.send_request(&message, target);
    };
    let state_request_sent = mutable.inbound.apply_state_scan_results(
        state_missing,
        &continuation.params,
        &family,
        &mut send_fn,
    );

    // State remains first. A terminal state observed after result application
    // must still suppress transaction follow-up and every deferred send.
    if state.is_done() {
        return;
    }
    // Only no-request state results fall through to the existing transaction
    // logic, preserving request filtering and completion.
    if !state_request_sent {
        let tx_setup = mutable
            .inbound
            .prepare_tx_after_state_scan(continuation.params.reason);
        for message in tx_setup.messages_to_send {
            send_fn(message);
        }
        if let Some(tx_params) = tx_setup.tx_scan.as_ref() {
            let tx_started = Instant::now();
            let tx_missing = {
                let AcqMutableState {
                    inbound,
                    store,
                    fetch_pack,
                } = &mut *mutable;
                inbound.do_tx_map_scan(tx_params, store, fetch_pack, &family)
            };
            state
                .stats
                .tx_missing_nodes
                .store(tx_missing.len() as u64, Ordering::Relaxed);
            state.stats.tx_scan_us.fetch_add(
                tx_started.elapsed().as_micros() as u64,
                Ordering::Relaxed,
            );
            mutable
                .inbound
                .apply_tx_scan_results(tx_missing, tx_params, &family, &mut send_fn);
        }
    }
    if mutable.inbound.planner_state().have_header
        && mutable.inbound.planner_state().have_state
        && mutable.inbound.planner_state().have_transactions
    {
        mutable.inbound.set_complete();
    }
    let terminal = mutable.inbound.is_failed() || mutable.inbound.is_complete();
    drop(mutable);
    if terminal {
        finalize_terminal(state);
    }
}

fn charge_malformed_packet(
    state: &AcquisitionState,
    peer_id: u64,
    packet_type: ledger::InboundLedgerDataType,
    error: InboundLedgerPacketError,
) {
    let Some(peer) = state.peer_set.find_peer(peer_id as u32) else {
        return;
    };
    let context = match (packet_type, error) {
        (ledger::InboundLedgerDataType::Base, InboundLedgerPacketError::EmptyNodes) => {
            "ledger_data empty header"
        }
        (_, InboundLedgerPacketError::EmptyNodes) => "ledger_data no nodes",
        (_, InboundLedgerPacketError::EmptyNodeData) => "ledger_data empty node",
        (_, InboundLedgerPacketError::InvalidHeader) => "ledger_data invalid header",
        (_, InboundLedgerPacketError::MissingNodeId) => "ledger_data missing node id",
        (_, InboundLedgerPacketError::InvalidNodeId) => "ledger_data invalid node id",
        (_, InboundLedgerPacketError::InvalidNodeData) => "ledger_data malformed node data",
        (_, InboundLedgerPacketError::InvalidData) => "ledger_data invalid data",
    };
    peer.charge(
        (*resource::FEE_MALFORMED_REQUEST).clone(),
        context.to_owned(),
    );
}

fn charge_invalid_data_packet(
    state: &AcquisitionState,
    peer_id: u64,
    packet_type: ledger::InboundLedgerDataType,
    error: InboundLedgerPacketError,
) {
    let Some(peer) = state.peer_set.find_peer(peer_id as u32) else {
        return;
    };
    let context = match (packet_type, error) {
        (ledger::InboundLedgerDataType::Base, _) => "ledger_data invalid root",
        (_, InboundLedgerPacketError::InvalidData) => "ledger_data invalid node",
        (_, _) => "ledger_data invalid data",
    };
    peer.charge((*resource::FEE_INVALID_DATA).clone(), context.to_owned());
}

fn process_timeout_job(state: &Arc<AcquisitionState>) {
    if state.is_done() {
        return;
    }
    // This function runs only after `take_admitted_timeout` consumes one
    // mailbox event. The one acquisition token makes the mutable planner safe
    // without a scan-active discard/re-arm branch.
    state.stats.timeout_dispatches.fetch_add(1, Ordering::Relaxed);
    if state.stats.should_emit_sampled_diagnostic() {
        tracing::debug!(
            target: "inbound_ledger",
            seq = state.seq,
            hash = %state.hash,
            timeout_dispatches = state.stats.timeout_dispatches.load(Ordering::Relaxed),
            "sampled admitted timeout dispatched from acquisition mailbox"
        );
    }
    state.lifecycle.timeout_jobs.fetch_add(1, Ordering::Relaxed);

    let mut retry = false;
    let mut finalize = false;
    let mut failed = false;
    {
        let Some(mut mutable) = state.lock_mutable("timeout") else {
            return;
        };
        match mutable.inbound.timeout_expired() {
            InboundLedgerTimerResult::Progress => {}
            InboundLedgerTimerResult::Done => finalize = true,
            InboundLedgerTimerResult::Failed => failed = true,
            InboundLedgerTimerResult::NoProgress => {
                state
                    .lifecycle
                    .timeout_no_progress
                    .fetch_add(1, Ordering::Relaxed);
                check_local(state, &mut mutable);
                if mutable.inbound.is_failed() {
                    failed = true;
                } else if mutable.inbound.is_complete() {
                    finalize = true;
                } else {
                    mutable.inbound.set_by_hash(true);
                    retry = true;
                    state
                        .lifecycle
                        .timeout_retries
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    if failed || finalize {
        finalize_terminal(state);
        return;
    }
    // Timeout recovery mutates planner state before it fans out requests. A
    // concurrent registry terminal transition must stop that follow-up; the
    // running turn will perform mailbox cleanup.
    if state.is_done() {
        return;
    }
    if retry {
        // Match InboundLedger::onTimer exactly: for non-history work the
        // tracked peers receive Timeout before new peers receive Added; for
        // history work addPeers does not trigger Added, then Timeout fans out.
        if state.reason != AcquireReason::History {
            trigger(state, InboundLedgerRequestTrigger::Timeout, None);
            for peer in add_peers(state) {
                trigger(state, InboundLedgerRequestTrigger::Added, Some(peer));
            }
        } else {
            let _ = add_peers(state);
            trigger(state, InboundLedgerRequestTrigger::Timeout, None);
        }
    }
    state.arm_timer();
}

/// Finalize the terminal planner state after its mutable acquisition lock has
/// been released. Failure deliberately wins if both flags are visible, as in
/// rippled's `done()` completion predicate (`complete_ && !failed_`). Both
/// finalizers are idempotent: `mark_failed` records once and
/// `record_completed_ledger` publishes once.
fn finalize_terminal(state: &Arc<AcquisitionState>) {
    let Some(mutable) = state.lock_mutable("terminal finalization") else {
        return;
    };

    let (failed, complete) = (mutable.inbound.is_failed(), mutable.inbound.is_complete());
    drop(mutable);

    if failed {
        state.mark_failed();
    } else if complete {
        finalize_acquisition(state);
    }
}

fn record_completed_ledger(
    acquisition_id: u64,
    completed: &AtomicBool,
    completed_ledger: &Mutex<Option<Arc<Ledger>>>,
    store_tx: &std::sync::mpsc::SyncSender<CompletedInboundLedger>,
    reason: AcquireReason,
    ledger: Arc<Ledger>,
) -> bool {
    // Record first: the registry's polling path is the authoritative
    // recovery path when the notification receiver is disconnected or late.
    // Holding this small cache lock closes the completed=true/cache-empty
    // window for concurrent acquire/poll callers.
    let mut cached = completed_ledger
        .lock()
        .expect("acquisition completed ledger lock");
    if completed.swap(true, Ordering::AcqRel) {
        return false;
    }
    *cached = Some(Arc::clone(&ledger));
    drop(cached);

    // This channel only wakes a consumer; failure must not revoke a completed
    // acquisition or make the ledger unrecoverable through the registry.
    let _ = store_tx.try_send(CompletedInboundLedger {
        ledger,
        reason,
        acquisition_id,
    });
    true
}

fn publish_completed_ledger(
    hash: Uint256,
    acquisition_id: u64,
    completion_recorder: &AcquisitionCompletionRecorder,
    completed: &AtomicBool,
    completed_ledger: &Mutex<Option<Arc<Ledger>>>,
    store_tx: &std::sync::mpsc::SyncSender<CompletedInboundLedger>,
    reason: AcquireReason,
    ledger: Arc<Ledger>,
) -> bool {
    // Match InboundLedger::done(): terminal touch precedes both storing the
    // completed result and dispatching AcqDone. A concurrent sweep/consumer
    // must never observe a completed acquisition with its old idle timestamp.
    completion_recorder(hash, Arc::clone(&ledger));
    record_completed_ledger(
        acquisition_id,
        completed,
        completed_ledger,
        store_tx,
        reason,
        ledger,
    )
}

fn finalize_acquisition(state: &Arc<AcquisitionState>) {
    if state.is_done() {
        return;
    }
    let Some(mut mutable) = state.lock_mutable("successful finalization") else {
        return;
    };
    if mutable.inbound.is_failed() {
        drop(mutable);
        // Do not let a completion owner suppress failure recording. This
        // preserves failure precedence if the terminal state changed while
        // this caller was waiting for the acquisition lock.
        state.mark_failed();
        return;
    }
    if let Some(error) = mutable.store.take_write_error() {
        let seq = state.seq;
        let hash = state.hash;
        drop(mutable);
        tracing::error!(target: "inbound_ledger", seq, hash = %hash, %error,
            "acquisition cannot complete because node persistence failed");
        state.mark_failed();
        return;
    }
    if !mutable.inbound.is_complete() {
        return;
    }
    if state
        .finalization_claimed
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    let journal = WorkerJournal;
    let config = ledger::LedgerConfig::default();
    let family = family(state);
    mutable
        .inbound
        .finish_if_done_with_family_and_config(&journal, &config, &family);
    if mutable.inbound.is_failed() {
        drop(mutable);
        // The completion owner can discover a terminal failure while
        // `finish_if_done...` validates the assembled ledger. Record it
        // directly so the once-only completion claim cannot strand this
        // acquisition in an unreported terminal state.
        state.mark_failed();
        return;
    }
    let Some(mut ledger) = mutable.inbound.ledger().cloned() else {
        // A planner may not remain terminal-complete without an assembled
        // ledger. The successful terminal owner has already been claimed, so
        // record this as failure rather than leaving the acquisition claimed
        // and permanently unpublished.
        drop(mutable);
        state.mark_failed();
        return;
    };
    // All SHAMap nodes arrived through `WorkerStore`, but NuDB can keep their
    // bucket metadata in its active burst until a checkpoint. Commit that
    // burst before publishing this ledger's SQL header through `setFullLedger`.
    // Otherwise a graceful restart can see the header while neither map root
    // is findable by hash in the NodeStore.
    if let Err(error) = mutable.store.sync() {
        let seq = state.seq;
        let hash = state.hash;
        drop(mutable);
        tracing::error!(target: "inbound_ledger", seq, hash = %hash, %error,
            "acquisition cannot complete because final NodeStore sync failed");
        state.mark_failed();
        return;
    }
    if !ledger.is_immutable() {
        ledger.set_immutable(true);
    }
    ledger.set_full();

    let node_store = state.node_store.clone();
    let tree_cache = Arc::clone(&state.shared_tree_cache);
    ledger.set_node_fetcher(Arc::new(move |hash| {
        if let Some(node) = tree_cache.fetch(hash.as_uint256()) {
            return Some(node);
        }
        let object = match &node_store {
            SHAMapStoreNodeStore::Single(db) => db.fetch_node_object(
                hash.as_uint256(),
                0,
                nodestore::FetchType::Synchronous,
                false,
            ),
            SHAMapStoreNodeStore::Rotating(db) => db.fetch_node_object(
                hash.as_uint256(),
                0,
                nodestore::FetchType::Synchronous,
                false,
            ),
        }?;
        shamap::nodes::tree_node::SHAMapTreeNode::make_from_prefix(object.data(), hash).ok()
    }));

    let ledger = Arc::new(ledger);
    let ledger_seq = ledger.header().seq;
    // Do not hold the acquisition state lock while publishing completion.
    // The registry/promotion path may immediately re-enter this state.
    drop(mutable);
    if !publish_completed_ledger(
        *state.hash.as_uint256(),
        state.acquisition_id,
        &state.completion_recorder,
        &state.completed,
        &state.completed_ledger,
        &state.store_tx,
        state.reason,
        ledger,
    ) {
        return;
    }
    state
        .lifecycle
        .terminal_completed
        .fetch_add(1, Ordering::Relaxed);
    tracing::info!(target: "inbound_ledger", seq = ledger_seq, "LEDGER ACQUIRED");
}

#[cfg(test)]
mod tests {
    use super::super::registry::{AcquireReason, AcquisitionLifecycleCounters};
    use super::super::worker_pool::WorkerPool;
    use super::{
        ACQUIRE_TIMEOUT, STATE_SCAN_MAX_DEFERRED_READS_PER_TURN,
        AcquisitionBuilder, AcquisitionCompletionRecorder, AcquisitionFailureRecorder,
        AcquisitionState, StateScanAfterAdvancePause, StateScanSliceOutcome,
        classify_state_scan_slice, peer_has_acquisition_target, publish_completed_ledger,
        record_completed_ledger, trigger,
    };
    use basics::base_uint::Uint256;
    use basics::basic_config::BasicConfig;
    use basics::intrusive_pointer::make_shared_intrusive;
    use basics::sha_map_hash::SHAMapHash;
    use basics::tagged_cache::MonotonicClock;
    use ledger::{
        FetchPackCache, InboundLedgerDataType, InboundLedgerNodeData, InboundLedgerPacket,
        InboundLedgerRequestTrigger, Ledger, LedgerHeader,
    };
    use nodestore::{DummyScheduler, ManagerImp, NullJournal, Scheduler};
    use overlay::{Peer, PeerImp, PeerSet, ProtocolPayload};
    use protocol::PublicKey;
    use shamap::node_id::SHAMapNodeId;
    use shamap::sync::DeferredMissingNodeScanProgress;
    use shamap::tree_node::SHAMapTreeNode;
    use shamap::tree_node_cache::TreeNodeCache;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use tempfile::TempDir;

    /// Build the same valid in-memory SHAMap store used by the app ledger
    /// fetcher integration tests. Timeout recovery calls `check_local`, so a
    /// real store is required even though this fixture intentionally has no
    /// local target ledger.
    fn test_node_store() -> (TempDir, crate::SHAMapStoreNodeStore) {
        let dir = TempDir::new().expect("tempdir");
        let mut config = BasicConfig::new();
        config.set_legacy("database_path", dir.path().join("sql").to_string_lossy());
        let node_db = config.section_mut("node_db");
        node_db.set("type", "Memory");
        node_db.set("path", dir.path().join("node").to_string_lossy());

        let bootstrap = crate::bootstrap_shamap_store(
            &config,
            false,
            128,
            1,
            8,
            64,
            2,
            &ManagerImp::new(),
            Arc::new(DummyScheduler) as Arc<dyn Scheduler>,
            Arc::new(NullJournal),
        )
        .expect("bootstrap");
        (dir, bootstrap.node_store)
    }

    fn timeout_state_with_failure_recorder(
        worker_pool: Arc<WorkerPool>,
        hash: SHAMapHash,
        failure_recorder: AcquisitionFailureRecorder,
    ) -> (
        TempDir,
        Arc<AcquisitionState>,
        Arc<AcquisitionLifecycleCounters>,
    ) {
        let (dir, node_store) = test_node_store();
        let lifecycle = Arc::new(AcquisitionLifecycleCounters::default());
        let (store_tx, _store_rx) = mpsc::sync_channel(1);
        let state = AcquisitionBuilder {
            hash,
            acquisition_id: 0,
            seq: 1,
            reason: AcquireReason::Generic,
            node_store,
            tree_cache: Arc::new(TreeNodeCache::new(
                "acquisition-timeout-test",
                8,
                time::Duration::seconds(60),
                MonotonicClock::default(),
            )),
            fetch_pack: Arc::new(FetchPackCache::new(
                8,
                time::Duration::seconds(60),
                MonotonicClock::default(),
            )),
            store_tx,
            failure_recorder,
            completion_recorder: Arc::new(|_, _| {}),
            full_below_generation: 1,
            worker_pool,
            initial_peers: Vec::new(),
            peer_provider: Arc::new(Vec::new),
            lifecycle: Arc::clone(&lifecycle),
        }
        .build();
        (dir, state, lifecycle)
    }

    fn timeout_state_with_hash(
        worker_pool: Arc<WorkerPool>,
        hash: SHAMapHash,
    ) -> (
        TempDir,
        Arc<AcquisitionState>,
        Arc<AcquisitionLifecycleCounters>,
    ) {
        timeout_state_with_failure_recorder(worker_pool, hash, Arc::new(|_| {}))
    }

    fn timeout_state(
        worker_pool: Arc<WorkerPool>,
    ) -> (
        TempDir,
        Arc<AcquisitionState>,
        Arc<AcquisitionLifecycleCounters>,
    ) {
        timeout_state_with_hash(
            worker_pool,
            SHAMapHash::new(Uint256::from_array([0xA5; 32])),
        )
    }

    fn install_state_scan_after_advance_pause(
        state: &AcquisitionState,
    ) -> Arc<StateScanAfterAdvancePause> {
        let pause = Arc::new(StateScanAfterAdvancePause::default());
        let mut slot = state
            .state_scan_after_advance_pause
            .lock()
            .expect("state scan pause slot lock");
        assert!(slot.is_none(), "an acquisition already has a scan pause");
        *slot = Some(Arc::clone(&pause));
        pause
    }

    fn clear_state_scan_after_advance_pause(
        state: &AcquisitionState,
        pause: &StateScanAfterAdvancePause,
    ) {
        pause.release();
        *state
            .state_scan_after_advance_pause
            .lock()
            .expect("state scan pause slot lock") = None;
    }

    fn header_and_state_root_packets_with_missing_child()
    -> (SHAMapHash, InboundLedgerPacket, InboundLedgerPacket) {
        let state_root = SHAMapTreeNode::new_inner(0);
        state_root.set_child_hash(3, SHAMapHash::new(Uint256::from_array([0xB5; 32])));
        state_root.update_hash();
        let state_root = make_shared_intrusive(state_root);
        let header = LedgerHeader {
            seq: 1,
            account_hash: state_root.get_hash(),
            ..LedgerHeader::default()
        };
        let hash = ledger::calculate_ledger_hash(&header);
        let prefixed = ledger::serialize_prefixed_ledger_header(&header, false);
        let header_packet = InboundLedgerPacket::new(
            InboundLedgerDataType::Base,
            vec![InboundLedgerNodeData::new(None, prefixed[4..].to_vec())],
        );
        let state_root_packet = InboundLedgerPacket::new(
            InboundLedgerDataType::StateNode,
            vec![InboundLedgerNodeData::new(
                Some(SHAMapNodeId::default().get_raw_string()),
                state_root
                    .serialize_for_wire()
                    .expect("state root wire serialization should succeed"),
            )],
        );
        (hash, header_packet, state_root_packet)
    }

    #[test]
    fn acquisition_state_scan_uses_parity_batch_and_preserves_missing_results() {
        assert_eq!(
            STATE_SCAN_MAX_DEFERRED_READS_PER_TURN, 512,
            "resumable inbound state scans must match rippled's deferred-read batch"
        );

        let worker_pool = Arc::new(WorkerPool::new(0));
        let (wanted_hash, header_packet, state_root_packet) =
            header_and_state_root_packets_with_missing_child();
        let (_dir, state, _lifecycle) =
            timeout_state_with_hash(Arc::clone(&worker_pool), wanted_hash);

        state.enqueue_packet(7, header_packet);
        state.enqueue_packet(7, state_root_packet);
        assert!(worker_pool.run_next_job_for_test());
        assert!(state.diagnostics().have_header);
        assert!(!state.diagnostics().have_state);

        trigger(&state, InboundLedgerRequestTrigger::Timeout, None);
        assert!(state.diagnostics().scan_continuation_pending);
        assert_eq!(worker_pool.snapshot().queued_jobs, 1);
        assert!(worker_pool.run_next_job_for_test());
        let diagnostics = state.diagnostics();
        assert!(diagnostics.state_scan_runs >= 1);
        assert_eq!(diagnostics.state_scan_last_yield, "complete");
        assert_eq!(diagnostics.state_scan_completed_slices, 1);
        assert_eq!(
            diagnostics.state_missing_nodes, 1,
            "a completed scan hands its missing result to the existing request boundary"
        );
        assert_eq!(diagnostics.state_scan_last_missing_nodes, 1);
        assert!(!diagnostics.scan_continuation_pending);
        assert!(diagnostics.state_scan_last_branch_steps > 0);
        assert!(diagnostics.state_scan_positive_progress_slices >= 1);
    }

    #[test]
    fn state_scan_slice_outcomes_preserve_progress_contract() {
        let before = DeferredMissingNodeScanProgress {
            remaining: 8,
            ..Default::default()
        };
        let former_branch_boundary = DeferredMissingNodeScanProgress {
            branch_steps: 512,
            remaining: 8,
            ..Default::default()
        };
        let deferred_read_budget = DeferredMissingNodeScanProgress {
            deferred_reads: STATE_SCAN_MAX_DEFERRED_READS_PER_TURN as u64,
            remaining: 8,
            ..Default::default()
        };
        let missing_limit = DeferredMissingNodeScanProgress {
            missing_nodes: 1,
            remaining: 0,
            complete: true,
            ..Default::default()
        };
        let complete = DeferredMissingNodeScanProgress {
            branch_steps: 1,
            remaining: 7,
            complete: true,
            ..Default::default()
        };

        assert_eq!(
            classify_state_scan_slice(before, former_branch_boundary).0,
            StateScanSliceOutcome::DeferredReadResume,
            "512 traversal steps alone must not create a worker-turn boundary"
        );
        assert_eq!(
            classify_state_scan_slice(before, deferred_read_budget).0,
            StateScanSliceOutcome::DeferredReadBudget
        );
        let (outcome, deltas) = classify_state_scan_slice(before, missing_limit);
        assert_eq!(outcome, StateScanSliceOutcome::MissingNodeLimit);
        assert_eq!(deltas.missing_nodes, 1);
        assert_eq!(
            classify_state_scan_slice(before, complete).0,
            StateScanSliceOutcome::Complete
        );
    }

    #[test]
    fn acquisition_turn_advances_scan_with_pending_packet_and_timeout() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (wanted_hash, header_packet, _) = header_and_state_root_packets_with_missing_child();
        let (_dir, state, lifecycle) =
            timeout_state_with_hash(Arc::clone(&worker_pool), wanted_hash);

        state.enqueue_packet(7, header_packet);
        assert!(worker_pool.run_next_job_for_test());
        trigger(&state, InboundLedgerRequestTrigger::Reply, None);
        assert!(state.diagnostics().scan_continuation_pending);

        state.enqueue_packet(
            7,
            InboundLedgerPacket::new(InboundLedgerDataType::Base, Vec::new()),
        );
        state.record_admitted_timeout();
        assert_eq!(worker_pool.snapshot().queued_jobs, 1);

        assert!(worker_pool.run_next_job_for_test());
        assert_eq!(lifecycle.timeout_jobs.load(Ordering::Relaxed), 1);
        assert_eq!(lifecycle.packet_steps.load(Ordering::Relaxed), 2);
        assert!(
            state.diagnostics().state_scan_runs >= 1,
            "a persisted scan receives a slice despite packet and timeout work"
        );
    }

    #[test]
    fn acquisition_timeout_retarget_is_strongest_and_remains_broadcast() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (wanted_hash, header_packet, _) = header_and_state_root_packets_with_missing_child();
        let (_dir, state, _lifecycle) =
            timeout_state_with_hash(Arc::clone(&worker_pool), wanted_hash);
        let peer: Arc<dyn Peer> = PeerImp::new(
            71,
            SocketAddr::from(([127, 0, 0, 1], 51235)),
            PublicKey::from_bytes([0x02; 33]),
            "retarget-peer",
        );

        state.enqueue_packet(7, header_packet);
        assert!(worker_pool.run_next_job_for_test());
        trigger(
            &state,
            InboundLedgerRequestTrigger::Reply,
            Some(Arc::clone(&peer)),
        );
        trigger(&state, InboundLedgerRequestTrigger::Timeout, None);
        trigger(
            &state,
            InboundLedgerRequestTrigger::ReplyHighLatency,
            Some(peer),
        );

        let mailbox = state.mailbox.lock().expect("acquisition mailbox lock");
        let scan = mailbox.scan.as_ref().expect("persisted scan continuation");
        assert_eq!(scan.params.reason, InboundLedgerRequestTrigger::Timeout);
        assert_eq!(scan.params.query_depth, 0);
        assert_eq!(scan.params.query_type, Some(0));
        assert!(scan.peer.is_none(), "Timeout remains broadcast after retargets");
    }

    #[test]
    fn acquisition_batch_reply_selection_waits_for_mailbox_emptiness() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, state, _lifecycle) = timeout_state(Arc::clone(&worker_pool));
        {
            let mut mailbox = state.mailbox.lock().expect("acquisition mailbox lock");
            mailbox.packets.push((
                8,
                InboundLedgerPacket::new(InboundLedgerDataType::Base, Vec::new()),
            ));
        }

        assert!(
            state.finish_packet_batch(7, 4).is_empty(),
            "reply peers must remain buffered while the mailbox batch has packets"
        );
        state
            .mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .packets
            .clear();
        assert_eq!(
            state.finish_packet_batch(7, 0),
            vec![7],
            "the same batch becomes triggerable only after empty observation"
        );
    }

    #[test]
    fn acquisition_stopped_after_scan_advance_sends_no_request_or_continuation() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (wanted_hash, header_packet, _) = header_and_state_root_packets_with_missing_child();
        let (_dir, state, _lifecycle) =
            timeout_state_with_hash(Arc::clone(&worker_pool), wanted_hash);
        let peer = PeerImp::new(
            72,
            SocketAddr::from(([127, 0, 0, 1], 51235)),
            PublicKey::from_bytes([0x03; 33]),
            "post-scan-stop-peer",
        );
        peer.record_ledger(*wanted_hash.as_uint256(), 1);
        state
            .peer_set
            .refresh_peers(vec![Arc::clone(&peer) as Arc<dyn Peer>]);
        state.peer_set.add_peers(1, &mut |_| true, &mut |_| {});

        state.enqueue_packet(72, header_packet);
        assert!(worker_pool.run_next_job_for_test());
        trigger(&state, InboundLedgerRequestTrigger::Timeout, None);
        assert!(state.diagnostics().scan_continuation_pending);
        let sent_before_scan = peer.queued_messages().len();
        let pause = install_state_scan_after_advance_pause(&state);

        let advancing_pool = Arc::clone(&worker_pool);
        let advancing = thread::spawn(move || {
            assert!(advancing_pool.run_next_job_for_test());
        });
        pause.wait_until_entered();
        state.stopped.store(true, Ordering::Release);
        pause.release();
        advancing.join().expect("bounded scan worker thread");
        clear_state_scan_after_advance_pause(&state, &pause);

        assert_eq!(peer.queued_messages().len(), sent_before_scan);
        let diagnostics = state.diagnostics();
        assert!(!diagnostics.scan_continuation_pending);
        assert_eq!(diagnostics.mailbox_token, "idle");
    }

    #[test]
    fn acquisition_mailbox_packet_is_processed_without_scan_active_noop() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, state, lifecycle) = timeout_state(Arc::clone(&worker_pool));
        state.enqueue_packet(
            7,
            InboundLedgerPacket::new(InboundLedgerDataType::Base, Vec::new()),
        );
        assert_eq!(worker_pool.snapshot().queued_jobs, 1);
        assert!(worker_pool.run_next_job_for_test());
        assert_eq!(state.diagnostics().buffered_packets, 0);
        assert_eq!(lifecycle.packet_steps.load(Ordering::Relaxed), 1);
        assert_eq!(lifecycle.packet_step_errors.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn acquisition_data_worker_drains_coalesced_packets_in_one_fifo_turn() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, state, lifecycle) = timeout_state(Arc::clone(&worker_pool));
        let packet = InboundLedgerPacket::new(InboundLedgerDataType::Base, Vec::new());

        // This is the same buffer-then-dispatch sequence used by registry
        // routing. Two arrivals before the worker runs produce one coalesced
        // job, which must drain both whole packets before state scanning.
        state.enqueue_packet(7, packet.clone());
        state.enqueue_packet(7, packet);

        assert_eq!(worker_pool.snapshot().queued_jobs, 1);
        assert_eq!(lifecycle.data_jobs_submitted.load(Ordering::Relaxed), 1);
        assert_eq!(lifecycle.data_jobs_coalesced.load(Ordering::Relaxed), 1);
        assert_eq!(state.diagnostics().buffered_packets, 2);

        assert!(worker_pool.run_next_job_for_test());
        assert_eq!(worker_pool.snapshot().queued_jobs, 0);
        assert_eq!(state.diagnostics().buffered_packets, 0);
        assert_eq!(lifecycle.data_jobs_started.load(Ordering::Relaxed), 1);
        assert_eq!(lifecycle.packet_steps.load(Ordering::Relaxed), 2);
        assert_eq!(lifecycle.packet_steps_completed.load(Ordering::Relaxed), 2);
        assert_eq!(
            lifecycle.packet_step_errors.load(Ordering::Relaxed),
            2,
            "the real parser must classify both empty Base packets as malformed"
        );
    }

    #[test]
    fn acquisition_node_packet_is_consumed_whole_without_continuation() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (wanted_hash, header_packet, state_root_packet) =
            header_and_state_root_packets_with_missing_child();
        let (_dir, state, lifecycle) =
            timeout_state_with_hash(Arc::clone(&worker_pool), wanted_hash);
        let node_packet = InboundLedgerPacket::new(
            InboundLedgerDataType::StateNode,
            vec![state_root_packet.nodes[0].clone(); 129],
        );

        state.enqueue_packet(7, header_packet);
        state.enqueue_packet(7, node_packet);
        assert!(worker_pool.run_next_job_for_test());

        let diagnostics = state.diagnostics();
        assert_eq!(lifecycle.packet_steps.load(Ordering::Relaxed), 2);
        assert_eq!(diagnostics.buffered_packets, 0);
        assert!(!diagnostics.has_active_packet);
        assert_eq!(worker_pool.snapshot().queued_jobs, 0);
    }

    #[test]
    fn acquisition_real_peer_set_routes_wire_message_and_charges_malformed_packet() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let hash = SHAMapHash::new(Uint256::from_array([0xA7; 32]));
        let (_dir, state, _lifecycle) = timeout_state_with_hash(Arc::clone(&worker_pool), hash);
        let peer = PeerImp::new(
            73,
            SocketAddr::from(([127, 0, 0, 1], 51235)),
            PublicKey::from_bytes([0x03; 33]),
            "acquisition-parity-peer",
        );
        peer.record_ledger(*hash.as_uint256(), 1);
        state
            .peer_set
            .refresh_peers(vec![Arc::clone(&peer) as Arc<dyn Peer>]);
        state.peer_set.add_peers(1, &mut |_| true, &mut |_| {});

        trigger(
            &state,
            InboundLedgerRequestTrigger::Added,
            Some(Arc::clone(&peer) as Arc<dyn Peer>),
        );
        let sent = peer.queued_messages();
        assert_eq!(sent.len(), 1, "the selected live peer receives one request");
        assert!(
            sent[0].get_buffer_size() > 6,
            "the request has a six-byte frame and payload"
        );
        let ProtocolPayload::GetLedger(request) = sent[0].protocol().payload.clone() else {
            panic!("acquisition trigger must send TMGetLedger");
        };
        assert_eq!(request.itype, 0, "the initial request is liBASE");
        assert_eq!(
            request.ledger_hash.as_deref(),
            Some(hash.as_uint256().data().as_slice())
        );
        assert_eq!(request.ledger_seq, Some(1));

        state.enqueue_packet(
            73,
            InboundLedgerPacket::new(InboundLedgerDataType::Base, Vec::new()),
        );
        assert!(worker_pool.run_next_job_for_test());

        let charges = peer.charges();
        assert_eq!(charges.len(), 1);
        assert_eq!(charges[0].0, (*resource::FEE_MALFORMED_REQUEST).clone());
        assert_eq!(charges[0].1, "ledger_data empty header");
    }

    #[test]
    fn acquisition_start_initializes_synchronously_before_timeout_work() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, state, lifecycle) = timeout_state(Arc::clone(&worker_pool));

        state.start();

        assert_eq!(
            lifecycle.initialization_jobs.load(Ordering::Relaxed),
            1,
            "initialization runs synchronously before the first mailbox token"
        );
        assert_eq!(
            worker_pool.snapshot().queued_jobs,
            1,
            "synchronous initialization admits only its first timeout callback"
        );
        assert_eq!(state.diagnostics().pending_admitted_timeouts, 0);

        // The first queued timeout callback only parks the admitted event and
        // queues the acquisition turn that will perform recovery.
        assert!(worker_pool.run_next_job_for_test());
        assert_eq!(state.diagnostics().pending_admitted_timeouts, 1);
        assert_eq!(worker_pool.snapshot().queued_jobs, 1);
        worker_pool.stop();
    }

    #[test]
    fn acquisition_timeout_timer_parks_then_dispatches_recovery_and_rearms() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, state, lifecycle) = timeout_state(Arc::clone(&worker_pool));

        // Exercise AcquisitionState::arm_timer directly. The deterministic
        // TimerService hook advances the scheduled three-second callback
        // without sleeping, but the callback must only enqueue recovery work.
        state.arm_timer();
        assert_eq!(
            worker_pool.scheduled_timer_delays_for_test(),
            vec![ACQUIRE_TIMEOUT]
        );
        assert_eq!(worker_pool.snapshot().queued_jobs, 0);
        assert_eq!(
            worker_pool.fire_next_timer_for_test(),
            Some(ACQUIRE_TIMEOUT)
        );
        assert_eq!(
            lifecycle.timeout_jobs.load(Ordering::Relaxed),
            0,
            "timer callbacks must not execute acquisition recovery inline"
        );
        assert_eq!(worker_pool.snapshot().queued_jobs, 1);

        // The admitted timeout closure only parks one mailbox event and queues
        // an acquisition turn; it must not execute recovery itself.
        assert!(worker_pool.run_next_job_for_test());
        assert_eq!(lifecycle.timeout_jobs.load(Ordering::Relaxed), 0);
        assert_eq!(state.diagnostics().pending_admitted_timeouts, 1);
        assert_eq!(worker_pool.snapshot().queued_jobs, 1);

        // The following acquisition turn consumes the parked event, performs
        // the local-store check and retry policy, then arms the next timer.
        assert!(worker_pool.run_next_job_for_test());
        assert_eq!(lifecycle.timeout_jobs.load(Ordering::Relaxed), 1);
        assert_eq!(lifecycle.timeout_no_progress.load(Ordering::Relaxed), 1);
        assert_eq!(lifecycle.timeout_retries.load(Ordering::Relaxed), 1);
        assert_eq!(worker_pool.snapshot().queued_jobs, 0);
        assert_eq!(
            worker_pool.scheduled_timer_delays_for_test(),
            vec![ACQUIRE_TIMEOUT]
        );
    }

    #[test]
    fn acquisition_timeout_rejection_rearms_without_running_recovery_inline() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, state, lifecycle) = timeout_state(Arc::clone(&worker_pool));
        let limit = worker_pool.snapshot().ledger_data_job_limit;
        for _ in 0..limit {
            worker_pool.submit_ledger_data(Box::new(|| {}));
        }
        assert_eq!(worker_pool.snapshot().outstanding_ledger_data_jobs, limit);

        state.arm_timer();
        assert_eq!(
            worker_pool.fire_next_timer_for_test(),
            Some(ACQUIRE_TIMEOUT)
        );

        assert_eq!(
            lifecycle.timeout_queue_rejected.load(Ordering::Relaxed),
            1,
            "the real timeout admission failure must be recorded"
        );
        assert_eq!(lifecycle.timeout_jobs.load(Ordering::Relaxed), 0);
        assert_eq!(worker_pool.snapshot().queued_jobs, limit);
        assert_eq!(
            worker_pool.scheduled_timer_delays_for_test(),
            vec![ACQUIRE_TIMEOUT],
            "rejected timeout work must be retried through a new timer"
        );
    }

    #[test]
    fn hash_only_acquisition_can_probe_peer_without_exact_advertisement() {
        let hash = Uint256::from_array([0xA5; 32]);
        let peer = PeerImp::new(
            42,
            SocketAddr::from(([127, 0, 0, 1], 51235)),
            PublicKey::from_bytes([0x02; 33]),
            "status-only-peer",
        );
        peer.set_closed_ledger_hash(hash);
        let peer: Arc<dyn Peer> = peer;

        // A sequence-less StatusChange does not populate known_ledgers.
        assert!(!peer.has_ledger(hash, 0));
        assert!(peer_has_acquisition_target(&peer, hash, 0));
        // Hash-only recovery may ask a live peer before that peer advertises
        // the target. The request still carries the target hash and registry
        // response routing enforces the hash/known-sequence identity.
        assert!(peer_has_acquisition_target(
            &peer,
            Uint256::from_array([0xA6; 32]),
            0,
        ));
    }

    #[test]
    fn failure_touches_registry_before_failure_visibility() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let release_rx = Arc::new(Mutex::new(release_rx));
        let release_waiter = Arc::clone(&release_rx);
        let recorder: AcquisitionFailureRecorder = Arc::new(move |_| {
            entered_tx.send(()).expect("failure callback entry signal");
            release_waiter
                .lock()
                .expect("failure callback release lock")
                .recv()
                .expect("failure callback release signal");
        });
        let (_dir, state, lifecycle) = timeout_state_with_failure_recorder(
            worker_pool,
            SHAMapHash::new(Uint256::from_array([0xD2; 32])),
            recorder,
        );
        let failing_state = Arc::clone(&state);
        let failure_thread = thread::spawn(move || failing_state.mark_failed());

        entered_rx.recv().expect("failure callback must run first");
        assert!(
            !state.failed.load(Ordering::Acquire) && !state.stopped.load(Ordering::Acquire),
            "poll and sweep must not observe failure before its terminal registry touch"
        );
        release_tx.send(()).expect("release failure callback");
        failure_thread.join().expect("failure transition thread");
        assert!(state.failed.load(Ordering::Acquire));
        assert!(state.stopped.load(Ordering::Acquire));
        assert_eq!(lifecycle.terminal_failed.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn completion_touches_registry_before_notification() {
        let (tx, rx) = mpsc::sync_channel(1);
        let completed = AtomicBool::new(false);
        let cache = Mutex::new(None);
        let touched = Arc::new(AtomicBool::new(false));
        let touch_observer = Arc::clone(&touched);
        let recorder: AcquisitionCompletionRecorder = Arc::new(move |_, _| {
            touch_observer.store(true, Ordering::Release);
        });
        let hash = Uint256::from_array([0xD1; 32]);
        let ledger = Arc::new(Ledger::from_ledger_seq_and_close_time(1, 100, false));

        assert!(publish_completed_ledger(
            hash,
            0,
            &recorder,
            &completed,
            &cache,
            &tx,
            AcquireReason::Consensus,
            ledger,
        ));
        let _ = rx.recv().expect("completion notification");
        assert!(
            touched.load(Ordering::Acquire),
            "terminal touch must occur before a consumer can observe completion"
        );
    }

    #[test]
    fn completed_ledger_remains_recoverable_when_notification_channel_is_closed() {
        let (tx, rx) = mpsc::sync_channel(1);
        drop(rx);
        let completed = AtomicBool::new(false);
        let cache = Mutex::new(None);
        let ledger = Arc::new(Ledger::from_ledger_seq_and_close_time(1, 100, false));

        assert!(record_completed_ledger(
            0,
            &completed,
            &cache,
            &tx,
            AcquireReason::Consensus,
            Arc::clone(&ledger)
        ));
        assert!(completed.load(Ordering::Acquire));
        assert_eq!(
            cache
                .lock()
                .expect("completion cache lock")
                .as_ref()
                .expect("completion must be cached")
                .header()
                .hash,
            ledger.header().hash
        );
    }
}

/// Stash state nodes from an unroutable response in the fetch pack, matching
/// `InboundLedgersImp::gotStaleData`.
pub fn stash_stale_packet<FP>(packet: &InboundLedgerPacket, stale_data_store: &mut FP) -> bool
where
    FP: FetchPackStore,
{
    for node in &packet.nodes {
        if node.node_id.is_none() {
            return false;
        }
        let Ok(Some(decoded)) =
            shamap::nodes::tree_node::SHAMapTreeNode::make_from_wire(&node.node_data)
        else {
            return false;
        };
        let Ok(prefixed) = decoded.serialize_with_prefix() else {
            return false;
        };
        stale_data_store.add_fetch_pack(*decoded.get_hash().as_uint256(), prefixed);
    }
    true
}
