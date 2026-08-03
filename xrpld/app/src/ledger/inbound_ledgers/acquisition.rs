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
    FetchPackCache, FetchPackContainer, FetchPackStore, InboundLedgerJournal, InboundLedgerLocal,
    InboundLedgerPacket, InboundLedgerPacketError, InboundLedgerReason,
    InboundLedgerRequestTrigger, InboundLedgerStore, InboundLedgerTimerResult, Ledger,
};
use overlay::{Peer, PeerSet as _};
use shamap::family::{FullBelowCacheImpl, NullMissingNodeReporter, SHAMapFamily};
use shamap::sync::DeferredMissingNodeScanStats;
use shamap::tree_node_cache::TreeNodeCache;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
#[cfg(test)]
use std::sync::{Condvar, OnceLock};
use std::time::{Duration, Instant};

use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

use super::registry::{AcquireReason, AcquisitionLifecycleCounters, CompletedInboundLedger};
use super::worker_pool::WorkerPool;

const PEER_COUNT_START: usize = 5;
const PEER_COUNT_ADD: usize = 3;
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);

#[cfg(test)]
#[derive(Default)]
struct DetachedStateScanPauseState {
    entered: bool,
    released: bool,
}

/// Test-only synchronization seam placed after `trigger` has leased the real
/// ledger and marked the detached state scan active. Release builds do not
/// compile this state or call site.
#[cfg(test)]
#[derive(Default)]
struct DetachedStateScanPause {
    state: Mutex<DetachedStateScanPauseState>,
    wake: Condvar,
}

#[cfg(test)]
impl DetachedStateScanPause {
    fn wait_until_entered(&self) {
        let state = self.state.lock().expect("detached scan pause lock");
        let (state, timeout) = self
            .wake
            .wait_timeout_while(state, Duration::from_secs(5), |state| !state.entered)
            .expect("detached scan pause wait");
        assert!(state.entered, "detached state scan did not reach its pause");
        assert!(!timeout.timed_out(), "detached state scan pause timed out");
    }

    fn release(&self) {
        let mut state = self.state.lock().expect("detached scan pause lock");
        state.released = true;
        self.wake.notify_all();
    }
}

#[cfg(test)]
fn detached_state_scan_pause_slot() -> &'static Mutex<Option<Arc<DetachedStateScanPause>>> {
    static SLOT: OnceLock<Mutex<Option<Arc<DetachedStateScanPause>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn pause_detached_state_scan_for_test() {
    let pause = detached_state_scan_pause_slot()
        .lock()
        .expect("detached scan pause slot lock")
        .clone();
    let Some(pause) = pause else {
        return;
    };
    let mut state = pause.state.lock().expect("detached scan pause lock");
    state.entered = true;
    pause.wake.notify_all();
    while !state.released {
        state = pause.wake.wait(state).expect("detached scan pause wait");
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
    state_scan_branches_seen: AtomicU64,
    state_scan_duplicate_missing_hashes: AtomicU64,
    state_scan_full_below_hits: AtomicU64,
    state_scan_loaded_or_cached_children: AtomicU64,
    state_scan_pending_reads: AtomicU64,
    state_scan_max_pending_reads: AtomicU64,
    state_scan_pending_hits: AtomicU64,
    state_scan_pending_misses: AtomicU64,
    state_scan_deferred_resumes: AtomicU64,
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
            state_scan_branches_seen: AtomicU64::new(0),
            state_scan_duplicate_missing_hashes: AtomicU64::new(0),
            state_scan_full_below_hits: AtomicU64::new(0),
            state_scan_loaded_or_cached_children: AtomicU64::new(0),
            state_scan_pending_reads: AtomicU64::new(0),
            state_scan_max_pending_reads: AtomicU64::new(0),
            state_scan_pending_hits: AtomicU64::new(0),
            state_scan_pending_misses: AtomicU64::new(0),
            state_scan_deferred_resumes: AtomicU64::new(0),
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
    pub state_scan_branches_seen: u64,
    pub state_scan_duplicate_missing_hashes: u64,
    pub state_scan_full_below_hits: u64,
    pub state_scan_loaded_or_cached_children: u64,
    pub state_scan_pending_reads: u64,
    pub state_scan_max_pending_reads: u64,
    pub state_scan_pending_hits: u64,
    pub state_scan_pending_misses: u64,
    pub state_scan_deferred_resumes: u64,
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

/// Peer packets retained until the acquisition's dispatched worker drains them,
/// matching rippled `InboundLedger::receivedData_`.

/// Per-ledger state owned by the registry.
pub struct AcquisitionState {
    pub data_buffer: Mutex<Vec<(u64, InboundLedgerPacket)>>,
    pub mutable: Mutex<AcqMutableState>,
    pub hash: SHAMapHash,
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
    pub stopped: AtomicBool,
    pub completed: AtomicBool,
    completed_ledger: Mutex<Option<Arc<Ledger>>>,
    pub failed: AtomicBool,
    // Mirrors rippled InboundLedger::done's signaled_ guard: exactly one
    // caller owns expensive successful-terminal finalization.
    finalization_claimed: AtomicBool,
    pub fetch_pack_ready: AtomicBool,
    /// True while the mutable Ledger is leased to an out-of-lock state scan.
    /// Router ingress remains buffered until the ledger is restored.
    state_scan_in_progress: AtomicBool,
    data_job_queued: AtomicBool,
    timer_armed: AtomicBool,
    worker_pool: Arc<WorkerPool>,
    lifecycle: Arc<AcquisitionLifecycleCounters>,
}

impl AcquisitionState {
    /// Perform `InboundLedger::init`: try local storage, add peers, then queue
    /// the immediate TimeoutCounter job.
    pub fn start(self: &Arc<Self>) {
        self.lifecycle
            .initialization_jobs
            .fetch_add(1, Ordering::Relaxed);
        run_acquisition_job(self, "initialization", || process_init(self));
    }

    /// Equivalent to `InboundLedger::gotData` dispatch coalescing.
    pub fn submit_data_job(self: &Arc<Self>) {
        if self.is_done() {
            return;
        }
        if self
            .data_job_queued
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.lifecycle
                .data_jobs_coalesced
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
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
            run_acquisition_job(&state, "data", || process_data_job(&state));
        }));
    }

    fn queue_timeout_job(self: &Arc<Self>) {
        if self.is_done() {
            return;
        }
        let state = Arc::clone(self);
        if !self.worker_pool.try_submit_timeout(Box::new(move || {
            run_acquisition_job(&state, "timeout", || process_timeout_job(&state));
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
        let has_active_packet = false;
        let buffered_packets = self
            .data_buffer
            .lock()
            .expect("acquisition data_buffer lock")
            .len();
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
        if self.failed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.stopped.store(true, Ordering::Release);
        self.lifecycle
            .terminal_failed
            .fetch_add(1, Ordering::Relaxed);
        (self.failure_recorder)(*self.hash.as_uint256());
    }

    pub(crate) fn take_buffered_packets(&self) -> Vec<ledger::InboundLedgerReceivedPacket> {
        std::mem::take(
            &mut *self
                .data_buffer
                .lock()
                .expect("acquisition data buffer lock"),
        )
        .into_iter()
        .map(|(peer_id, packet)| ledger::InboundLedgerReceivedPacket::new(Some(peer_id), packet))
        .collect()
    }

    fn has_pending_data(&self) -> bool {
        !self
            .data_buffer
            .lock()
            .expect("acquisition data buffer lock")
            .is_empty()
    }

    fn finish_data_job(self: &Arc<Self>) {
        self.data_job_queued.store(false, Ordering::Release);
        if !self.is_done() && self.has_pending_data() {
            self.submit_data_job();
        }
    }

    /// Complete an exclusive state-map scan after its leased ledger has been
    /// restored under `mutable`. This schedules exactly one delayed buffer
    /// drain, rather than spinning data workers while the scan owns the tree.
    fn finish_detached_state_scan(self: &Arc<Self>) {
        self.state_scan_in_progress.store(false, Ordering::Release);
        let buffered_packets = self
            .data_buffer
            .lock()
            .expect("acquisition data buffer lock")
            .len();
        self.stats
            .record_state_scan_buffered_packets(buffered_packets);
        if !self.is_done() && buffered_packets != 0 {
            self.submit_data_job();
        }
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
    pub seq: u32,
    pub reason: AcquireReason,
    pub node_store: SHAMapStoreNodeStore,
    pub tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    pub fetch_pack: Arc<FetchPackCache>,
    pub store_tx: std::sync::mpsc::SyncSender<CompletedInboundLedger>,
    pub failure_recorder: AcquisitionFailureRecorder,
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
            data_buffer: Mutex::new(Vec::new()),
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
            stopped: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            completed_ledger: Mutex::new(None),
            failed: AtomicBool::new(false),
            finalization_claimed: AtomicBool::new(false),
            fetch_pack_ready: AtomicBool::new(false),
            state_scan_in_progress: AtomicBool::new(false),
            data_job_queued: AtomicBool::new(false),
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
    if state.is_done() || state.state_scan_in_progress.load(Ordering::Acquire) {
        return;
    }
    state
        .lifecycle
        .request_triggers
        .fetch_add(1, Ordering::Relaxed);

    // rippled releases InboundLedger's mutex for the large state-map walk.
    // SyncTree's walk mutates the tree, so lease the entire Ledger instead of
    // sharing a mutable tree across threads. Packets remain buffered while the
    // lease is outstanding and the exact Ledger is restored before recheck.
    let Some(mut mutable) = state.lock_mutable("trigger") else {
        return;
    };
    if state.state_scan_in_progress.load(Ordering::Acquire) {
        return;
    }
    let AcqMutableState {
        inbound,
        store,
        fetch_pack,
    } = &mut *mutable;
    let journal = WorkerJournal;
    let config = ledger::LedgerConfig::default();
    let family = family(state);
    let setup = inbound.prepare_trigger(reason, &journal, &config, store, fetch_pack, &family);

    for msg in &setup.messages_to_send {
        state
            .lifecycle
            .request_messages
            .fetch_add(1, Ordering::Relaxed);
        state.peer_set.send_request(msg, peer.as_ref());
    }

    if let Some(params) = setup.state_scan {
        let Some(mut leased_ledger) = inbound.take_ledger_for_state_scan(&params) else {
            return;
        };
        let mut scan_store = store.clone();
        let mut scan_fetch_pack = fetch_pack.clone();
        state.state_scan_in_progress.store(true, Ordering::Release);
        #[cfg(test)]
        pause_detached_state_scan_for_test();
        drop(mutable);

        let state_scan_started = Instant::now();
        let scan_result = catch_unwind(AssertUnwindSafe(|| {
            InboundLedgerLocal::do_state_map_scan_for_ledger(
                &mut leased_ledger,
                &params,
                &mut scan_store,
                &mut scan_fetch_pack,
                &family,
            )
        }));
        let state_scan_us = state_scan_started.elapsed().as_micros() as u64;

        let Some(mut mutable) = state.lock_mutable("state scan restore") else {
            state.finish_detached_state_scan();
            return;
        };
        mutable
            .inbound
            .restore_ledger_after_state_scan(leased_ledger);
        state.stats.state_scan_runs.fetch_add(1, Ordering::Relaxed);
        state
            .stats
            .state_scan_us
            .fetch_add(state_scan_us, Ordering::Relaxed);
        let state_missing = match scan_result {
            Ok((missing, scan_stats)) => {
                state.stats.record_state_scan(&scan_stats);
                if state.stats.should_emit_sampled_diagnostic() {
                    tracing::debug!(
                        target: "inbound_ledger",
                        seq = state.seq,
                        hash = %state.hash,
                        scan_us = state_scan_us,
                        missing = missing.len(),
                        branches = scan_stats.branches_seen,
                        duplicate_missing = scan_stats.duplicate_missing_hashes,
                        pending = scan_stats.pending_reads,
                        max_pending = scan_stats.max_pending_reads,
                        pending_hits = scan_stats.completed_pending_reads,
                        pending_misses = scan_stats.completed_pending_misses,
                        "sampled state-map scan"
                    );
                }
                missing
            }
            Err(payload) => {
                drop(mutable);
                state.finish_detached_state_scan();
                std::panic::resume_unwind(payload);
            }
        };
        state
            .stats
            .state_missing_nodes
            .store(state_missing.len() as u64, Ordering::Relaxed);
        let mut send_fn = |message: overlay::ProtocolMessage| {
            state
                .lifecycle
                .request_messages
                .fetch_add(1, Ordering::Relaxed);
            // rippled broadcasts TMGetObjectByHash to ALL tracked peers
            // (InboundLedger.cpp:542-548) regardless of trigger source.
            let target = if matches!(message.payload, overlay::ProtocolPayload::GetObjects(_)) {
                None
            } else {
                peer.as_ref()
            };
            state.peer_set.send_request(&message, target);
        };
        let state_request_sent =
            mutable
                .inbound
                .apply_state_scan_results(state_missing, &params, &family, &mut send_fn);

        // State requests return early in rippled. Only an empty/filtered state
        // result falls through to transaction work in this same trigger.
        if !state_request_sent {
            let tx_setup = mutable.inbound.prepare_tx_after_state_scan(reason);
            for message in tx_setup.messages_to_send {
                send_fn(message);
            }
            if let Some(tx_params) = tx_setup.tx_scan.as_ref() {
                let tx_scan_started = Instant::now();
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
                    tx_scan_started.elapsed().as_micros() as u64,
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
        state.finish_detached_state_scan();
        if terminal {
            finalize_terminal(state);
        }
        return;
    }

    // Preserve the normal transaction-only fallback when state was already
    // complete, invalid, or setup emitted a state-root request.
    let state_request_sent = setup.state_request_pending;
    if !state_request_sent {
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
            let mut send_fn = |message: overlay::ProtocolMessage| {
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

fn process_data_job(state: &Arc<AcquisitionState>) {
    if state.is_done() {
        state.data_job_queued.store(false, Ordering::Release);
        return;
    }
    if state.state_scan_in_progress.load(Ordering::Acquire) {
        // The scan completion path restores the ledger and schedules one
        // buffer drain. Do not turn buffered ingress into worker churn here.
        state.data_job_queued.store(false, Ordering::Release);
        return;
    }

    let Some(mut mutable) = state.lock_mutable("data processing") else {
        state.finish_data_job();
        return;
    };
    if state.state_scan_in_progress.load(Ordering::Acquire) {
        drop(mutable);
        state.data_job_queued.store(false, Ordering::Release);
        return;
    }
    if state.fetch_pack_ready.swap(false, Ordering::AcqRel) {
        check_local(state, &mut mutable);
    }
    if mutable.inbound.is_failed() || mutable.inbound.is_complete() {
        drop(mutable);
        finalize_terminal(state);
        state.finish_data_job();
        return;
    }

    // Match rippled InboundLedger::runData: one coalesced dispatch repeatedly
    // drains everything received while it runs, then samples useful peers once
    // for the next reply-triggered request round. The refill closure closes the
    // race with concurrent router ingress without holding the buffer lock while
    // packet processing or NodeStore writes occur.
    let had_header = mutable.inbound.planner_state().have_header;
    let journal = WorkerJournal;
    let config = ledger::LedgerConfig::default();
    let family = family(state);
    let data_drain_started = Instant::now();
    let result = {
        let AcqMutableState {
            inbound,
            store,
            fetch_pack,
        } = &mut *mutable;
        let mut refill = || state.take_buffered_packets();
        inbound.run_data_with_family_and_config_and_refill(
            &journal,
            &config,
            store,
            fetch_pack,
            &family,
            &mut refill,
        )
    };
    let data_drain_us = data_drain_started.elapsed().as_micros() as u64;
    state
        .stats
        .record_data_drain(data_drain_us, result.processed_packets);
    if state.stats.should_emit_sampled_diagnostic() {
        tracing::debug!(
            target: "inbound_ledger",
            seq = state.seq,
            hash = %state.hash,
            drain_us = data_drain_us,
            packets = result.processed_packets,
            useful_packets = result.useful_packets,
            useful_nodes = result.useful_nodes,
            state_packets = result.state_packets,
            state_nodes = result.state_useful_nodes,
            "sampled inbound data drain"
        );
    }

    if !had_header && mutable.inbound.planner_state().have_header {
        state.stats.mark_header_received();
    }
    state
        .lifecycle
        .packet_steps
        .fetch_add(result.processed_packets as u64, Ordering::Relaxed);
    state
        .lifecycle
        .packet_steps_completed
        .fetch_add(result.processed_packets as u64, Ordering::Relaxed);
    state
        .stats
        .packets
        .fetch_add(result.processed_packets as u64, Ordering::Relaxed);
    state
        .stats
        .useful_packets
        .fetch_add(result.useful_packets as u64, Ordering::Relaxed);
    state
        .stats
        .useful_nodes
        .fetch_add(result.useful_nodes, Ordering::Relaxed);
    state
        .stats
        .state_packets
        .fetch_add(result.state_packets as u64, Ordering::Relaxed);
    state
        .stats
        .state_useful_nodes
        .fetch_add(result.state_useful_nodes, Ordering::Relaxed);
    state
        .stats
        .state_duplicate_nodes
        .fetch_add(result.state_duplicate_nodes, Ordering::Relaxed);
    state.lifecycle.packet_step_errors.fetch_add(
        (result.malformed_packets.len() + result.invalid_packets.len()) as u64,
        Ordering::Relaxed,
    );
    state
        .stats
        .malformed_packets
        .fetch_add(result.malformed_packets.len() as u64, Ordering::Relaxed);
    for (peer_id, packet_type, error) in result.malformed_packets {
        charge_malformed_packet(state, peer_id, packet_type, error);
    }
    for (peer_id, packet_type, error) in result.invalid_packets {
        charge_invalid_data_packet(state, peer_id, packet_type, error);
    }

    let reply_requests: Vec<_> = result
        .triggered_peer_ids
        .into_iter()
        .filter_map(|peer_id| {
            let peer = state.peer_set.find_peer(peer_id as u32)?;
            let reason = if peer.is_high_latency() {
                InboundLedgerRequestTrigger::ReplyHighLatency
            } else {
                InboundLedgerRequestTrigger::Reply
            };
            Some((reason, peer))
        })
        .collect();
    let terminal = mutable.inbound.is_failed() || mutable.inbound.is_complete();
    drop(mutable);
    if terminal {
        finalize_terminal(state);
    }
    state.finish_data_job();
    for (reason, peer) in reply_requests {
        trigger(state, reason, Some(peer));
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
    if state.state_scan_in_progress.load(Ordering::Acquire) {
        // rippled's timer cannot mutate the inbound planner until the map walk
        // releases its lock; re-arm rather than treating this as no progress.
        state.arm_timer();
        return;
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
    let _ = store_tx.try_send(CompletedInboundLedger { ledger, reason });
    true
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
    if !record_completed_ledger(
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
        ACQUIRE_TIMEOUT, AcquisitionBuilder, AcquisitionState, DetachedStateScanPause,
        detached_state_scan_pause_slot, peer_has_acquisition_target, record_completed_ledger,
        trigger,
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

    fn timeout_state_with_hash(
        worker_pool: Arc<WorkerPool>,
        hash: SHAMapHash,
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
            failure_recorder: Arc::new(|_| {}),
            full_below_generation: 1,
            worker_pool,
            initial_peers: Vec::new(),
            peer_provider: Arc::new(Vec::new),
            lifecycle: Arc::clone(&lifecycle),
        }
        .build();
        (dir, state, lifecycle)
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

    fn state_scan_pause() -> Arc<DetachedStateScanPause> {
        let pause = Arc::new(DetachedStateScanPause::default());
        let mut slot = detached_state_scan_pause_slot()
            .lock()
            .expect("detached scan pause slot lock");
        assert!(
            slot.is_none(),
            "a detached state scan pause is already installed"
        );
        *slot = Some(Arc::clone(&pause));
        pause
    }

    fn clear_state_scan_pause(pause: &DetachedStateScanPause) {
        pause.release();
        *detached_state_scan_pause_slot()
            .lock()
            .expect("detached scan pause slot lock") = None;
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
    fn acquisition_real_detached_state_scan_defers_and_then_drains_packet() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (wanted_hash, header_packet, state_root_packet) =
            header_and_state_root_packets_with_missing_child();
        let (_dir, state, lifecycle) =
            timeout_state_with_hash(Arc::clone(&worker_pool), wanted_hash);

        state
            .data_buffer
            .lock()
            .expect("data buffer lock")
            .push((7, header_packet));
        state
            .data_buffer
            .lock()
            .expect("data buffer lock")
            .push((7, state_root_packet));
        state.submit_data_job();
        assert!(worker_pool.run_next_job_for_test());
        assert!(state.diagnostics().have_header);
        assert!(!state.diagnostics().have_state);

        let pause = state_scan_pause();
        let scan_state = Arc::clone(&state);
        let scan_thread = thread::spawn(move || {
            trigger(&scan_state, InboundLedgerRequestTrigger::Timeout, None);
        });
        pause.wait_until_entered();
        assert!(state.state_scan_in_progress.load(Ordering::Acquire));

        let buffered = InboundLedgerPacket::new(InboundLedgerDataType::Base, Vec::new());
        state
            .data_buffer
            .lock()
            .expect("data buffer lock")
            .push((7, buffered));
        state.submit_data_job();
        assert!(worker_pool.run_next_job_for_test());
        let buffered_packets = state.data_buffer.lock().expect("data buffer lock").len();
        assert_eq!(buffered_packets, 1);
        assert_eq!(lifecycle.packet_steps.load(Ordering::Relaxed), 2);

        clear_state_scan_pause(&pause);
        scan_thread.join().expect("detached state scan thread");
        assert_eq!(state.diagnostics().state_scan_runs, 1);
        assert!(!state.state_scan_in_progress.load(Ordering::Acquire));
        assert_eq!(worker_pool.snapshot().queued_jobs, 1);

        assert!(worker_pool.run_next_job_for_test());
        assert_eq!(state.diagnostics().buffered_packets, 0);
        assert_eq!(lifecycle.packet_steps.load(Ordering::Relaxed), 3);
        assert_eq!(lifecycle.packet_step_errors.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn acquisition_buffered_packet_waits_for_detached_scan_completion() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, state, lifecycle) = timeout_state(Arc::clone(&worker_pool));
        let packet = InboundLedgerPacket::new(InboundLedgerDataType::Base, Vec::new());

        // Model the production interval in which trigger has leased the
        // mutable ledger to its out-of-lock state-map walk. Router ingress
        // remains live, but its worker must not access the leased planner.
        state.state_scan_in_progress.store(true, Ordering::Release);
        state
            .data_buffer
            .lock()
            .expect("data buffer lock")
            .push((7, packet));
        state.submit_data_job();
        assert_eq!(worker_pool.snapshot().queued_jobs, 1);

        assert!(worker_pool.run_next_job_for_test());
        assert_eq!(lifecycle.data_jobs_started.load(Ordering::Relaxed), 1);
        assert_eq!(state.diagnostics().buffered_packets, 1);
        assert_eq!(lifecycle.packet_steps.load(Ordering::Relaxed), 0);
        assert_eq!(worker_pool.snapshot().queued_jobs, 0);

        // This is the production completion edge after the detached scan has
        // restored the ledger. It must schedule one delayed buffer drain.
        state.finish_detached_state_scan();
        assert_eq!(worker_pool.snapshot().queued_jobs, 1);
        assert_eq!(
            lifecycle.data_jobs_submitted.load(Ordering::Relaxed),
            2,
            "scan completion must enqueue exactly one follow-up data drain"
        );

        assert!(worker_pool.run_next_job_for_test());
        assert_eq!(state.diagnostics().buffered_packets, 0);
        assert_eq!(lifecycle.packet_steps.load(Ordering::Relaxed), 1);
        assert_eq!(lifecycle.packet_step_errors.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn acquisition_data_worker_coalesces_and_drains_buffered_packets() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, state, lifecycle) = timeout_state(Arc::clone(&worker_pool));
        let packet = InboundLedgerPacket::new(InboundLedgerDataType::Base, Vec::new());

        // This is the same buffer-then-dispatch sequence used by registry
        // routing. Two arrivals before the worker runs must produce one
        // coalesced data job, then that job must drain both packets.
        state
            .data_buffer
            .lock()
            .expect("data buffer lock")
            .push((7, packet.clone()));
        state.submit_data_job();
        state
            .data_buffer
            .lock()
            .expect("data buffer lock")
            .push((7, packet));
        state.submit_data_job();

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

        state.data_buffer.lock().expect("data buffer lock").push((
            73,
            InboundLedgerPacket::new(InboundLedgerDataType::Base, Vec::new()),
        ));
        state.submit_data_job();
        assert!(worker_pool.run_next_job_for_test());

        let charges = peer.charges();
        assert_eq!(charges.len(), 1);
        assert_eq!(charges[0].0, (*resource::FEE_MALFORMED_REQUEST).clone());
        assert_eq!(charges[0].1, "ledger_data empty header");
    }

    #[test]
    fn acquisition_timeout_timer_queues_worker_recovery_and_rearms_after_drain() {
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

        // Draining the real queued timeout calls process_timeout_job, performs
        // the local-store check, applies the no-progress retry policy, and
        // arms the next three-second timer.
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
    fn completed_ledger_remains_recoverable_when_notification_channel_is_closed() {
        let (tx, rx) = mpsc::sync_channel(1);
        drop(rx);
        let completed = AtomicBool::new(false);
        let cache = Mutex::new(None);
        let ledger = Arc::new(Ledger::from_ledger_seq_and_close_time(1, 100, false));

        assert!(record_completed_ledger(
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
