//! Per-hash inbound-ledger lifecycle.
//!
//! The structure follows rippled's `InboundLedger` and `TimeoutCounter`:
//! `init` checks local storage, adds peers, queues an immediate timeout job,
//! and every timeout job re-arms only its own three-second timer.

use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::MonotonicClock;
#[cfg(test)]
use ledger::uses_aggressive_by_hash_timeout;
use ledger::{
    FetchPackCache, FetchPackContainer, FetchPackStore, InboundLedgerDataType,
    InboundLedgerJournal, InboundLedgerLocal, InboundLedgerNodeAdmission, InboundLedgerObjectType, InboundLedgerPacket,
    InboundLedgerPacketError, InboundLedgerPeerScore, InboundLedgerReason,
    InboundLedgerRequestTrigger, InboundLedgerStore, InboundLedgerTimerResult, Ledger, TreeAdvance,
    TreeKind, TreePlan, TreePlanId, make_get_ledger_with_node_ids,
    make_inbound_needed_by_hash_request, select_inbound_ledger_reply_peers,
};
use ledger::ledger_fetcher::{
    INBOUND_LEDGER_NORMAL_NODE_REQUEST_CAP, INBOUND_LEDGER_REPLY_NODE_REQUEST_CAP,
};
use overlay::{Peer, PeerSet as _};
use shamap::family::{FullBelowCache, FullBelowCacheImpl, NullMissingNodeReporter, SHAMapFamily};
use shamap::sync::{
    MissingNodeReadApply, MissingNodeReadOutcome, MissingNodeResidentLookup, ReadNeed,
    SHAMapAddNode,
};
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
#[cfg(test)]
use std::sync::Condvar;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

use super::read_broker::{
    NodeReadBroker, ReadAdmission, ReadKey, ReadOutcome, ReadReady, ReadReadySink,
    ReadRejectReason, ReadTicket,
};
use super::registry::{AcquireReason, AcquisitionLifecycleCounters};
use super::worker_pool::WorkerPool;

const PEER_COUNT_START: usize = 5;
const PEER_COUNT_ADD: usize = 3;
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);
const LOCAL_PROBE_PLAN_ID: u64 = 0;

/// A brokered local probe precedes peer traffic for the header and both roots.
/// Descendant/full-tree local discovery is then performed by the same brokered
/// TreePlan continuation, not a detached synchronous scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LocalProbeKind {
    Header,
    StateRoot,
    TransactionRoot,
}

#[derive(Clone)]
struct LocalProbe {
    /// Every accepted broker subscription stays actor-owned until its one
    /// completion is reduced or terminal teardown explicitly cancels it.
    ticket: ReadTicket,
    kind: LocalProbeKind,
    reason: InboundLedgerRequestTrigger,
    peer: Option<Arc<dyn Peer>>,
}

/// Bounded actor CPU admission. These caps only yield between turns; they do
/// not alter packet-batch accounting, peer selection, or retry semantics.
/// These remain separate from broker limits and preserve an explicit yield so
/// a packet flood cannot monopolize a ledger-data worker.
pub const ACQ_TURN_MAX_BRANCH_STEPS: usize = 512;
pub const ACQ_TURN_MAX_NEW_READS: usize = 32;
/// Maximum settled broker completions reduced in one actor turn. This is half
/// the experiment's per-acquisition logical-read limit, preserving a bounded
/// successor turn when the completion mailbox is full.
pub const ACQ_TURN_MAX_READ_EVENTS: usize = 16;
/// A persistence command is one bounded NodeStore batch. The lower NodeStore
/// writer batches further across acquisitions; this batch removes one worker
/// dispatch and acknowledgement round-trip per individual node write.
const PERSISTENCE_WRITE_BATCH_SIZE: usize = 256;
pub const ACQ_MAILBOX_PACKET_CAPACITY: usize = 128;
pub const ACQ_MAILBOX_BYTE_CAPACITY: usize = 4 * 1024 * 1024;

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
        assert!(
            state.entered,
            "state scan did not reach its post-advance pause"
        );
        assert!(
            !timeout.timed_out(),
            "state scan post-advance pause timed out"
        );
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
    state_scan_read_slot_full: AtomicU64,
    state_scan_read_admission_accepted: AtomicU64,
    state_scan_read_admission_deferred: AtomicU64,
    state_scan_read_admission_attached: AtomicU64,
    state_scan_read_broker_rejected: AtomicU64,
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
            state_scan_last_outcome: AtomicU8::new(0),
            state_scan_last_branch_steps: AtomicU64::new(0),
            state_scan_last_deferred_reads: AtomicU64::new(0),
            state_scan_last_deferred_resumes: AtomicU64::new(0),
            state_scan_last_missing_nodes: AtomicU64::new(0),
            state_scan_branches_seen: AtomicU64::new(0),
            state_scan_duplicate_missing_hashes: AtomicU64::new(0),
            state_scan_full_below_hits: AtomicU64::new(0),
            state_scan_loaded_or_cached_children: AtomicU64::new(0),
            state_scan_pending_reads: AtomicU64::new(0),
            state_scan_read_slot_full: AtomicU64::new(0),
            state_scan_read_admission_accepted: AtomicU64::new(0),
            state_scan_read_admission_deferred: AtomicU64::new(0),
            state_scan_read_admission_attached: AtomicU64::new(0),
            state_scan_read_broker_rejected: AtomicU64::new(0),
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
    pub state_scan_read_slot_full: u64,
    pub state_scan_read_admission_accepted: u64,
    pub state_scan_read_admission_deferred: u64,
    pub state_scan_read_admission_attached: u64,
    pub state_scan_read_broker_rejected: u64,
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
    pub mailbox_bytes: usize,
    pub mailbox_bytes_high_water: usize,
    pub mailbox_events: usize,
    pub stale_events: u64,
    pub overload_rejections: u64,
    pub active_plan_id: Option<u64>,
    pub active_plan_kind: Option<&'static str>,
    pub plan_pending_hashes: usize,
    pub plan_pending_edges: usize,
    pub broker_queued_keys: usize,
    pub broker_in_flight_keys: usize,
    pub mailbox_token: &'static str,
    pub scan_continuation_pending: bool,
    pub pending_admitted_timeouts: u32,
    pub has_active_packet: bool,
}

/// One idempotent accepted-node persistence command. Commands are collected
/// while packet validation owns the actor, but are dispatched only after that
/// ownership is released. The key gives duplicate packet data one durable
/// command and one terminal acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PersistenceKey {
    hash: Uint256,
    ledger_seq: u32,
    object_type: u8,
}

#[derive(Clone)]
struct PersistenceWrite {
    key: PersistenceKey,
    object_type: nodestore::NodeObjectType,
    data: Vec<u8>,
}

enum PersistenceCommand {
    Writes {
        id: u64,
        writes: Vec<PersistenceWrite>,
    },
    DurabilityBarrier {
        id: u64,
    },
}

impl PersistenceCommand {
    fn id(&self) -> u64 {
        match self {
            Self::Writes { id, .. } | Self::DurabilityBarrier { id } => *id,
        }
    }

    fn is_durability_barrier(&self) -> bool {
        matches!(self, Self::DurabilityBarrier { .. })
    }
}

/// The actor retains only completion identity while a worker owns the write
/// payload. That avoids cloning each node's bytes merely to preserve FIFO ack
/// ordering.
#[derive(Clone, Copy)]
struct InFlightPersistenceCommand {
    id: u64,
    durability_barrier: bool,
}

#[derive(Clone)]
struct PersistenceReady {
    id: u64,
    result: Result<(), Arc<str>>,
    durability_barrier: bool,
}

/// Every accepted persistence command has one durable terminal disposition.
/// Duplicate writes are a visible logical settlement, while terminal teardown
/// settles both queued and executing commands as cancelled before callbacks
/// are allowed to disappear.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PersistenceSettlement {
    Written,
    Duplicate,
    Failed(Arc<str>),
    Cancelled,
}

/// Actor-external, per-acquisition FIFO persistence owner. Exactly one command
/// is in flight, so a successful durability barrier is ordered after every
/// accepted write. The actor observes the acknowledgement before the next
/// command is dispatched.
struct PersistenceQueue {
    next_id: u64,
    queued: VecDeque<PersistenceCommand>,
    in_flight: Option<InFlightPersistenceCommand>,
    accepted: BTreeSet<PersistenceKey>,
    settled: BTreeMap<u64, PersistenceSettlement>,
    barrier_enqueued: bool,
    barrier_acknowledged: bool,
    failed: Option<Arc<str>>,
}

impl Default for PersistenceQueue {
    fn default() -> Self {
        Self {
            next_id: 1,
            queued: VecDeque::new(),
            in_flight: None,
            accepted: BTreeSet::new(),
            settled: BTreeMap::new(),
            barrier_enqueued: false,
            barrier_acknowledged: false,
            failed: None,
        }
    }
}

impl PersistenceQueue {
    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("persistence command id overflow");
        id
    }

    fn enqueue_writes(&mut self, writes: Vec<PersistenceWrite>) {
        let mut batch = Vec::with_capacity(PERSISTENCE_WRITE_BATCH_SIZE);
        for write in writes {
            if !self.accepted.insert(write.key) {
                let id = self.next_id();
                self.settled.insert(id, PersistenceSettlement::Duplicate);
                continue;
            }
            batch.push(write);
            if batch.len() == PERSISTENCE_WRITE_BATCH_SIZE {
                let id = self.next_id();
                self.queued.push_back(PersistenceCommand::Writes {
                    id,
                    writes: std::mem::take(&mut batch),
                });
                batch = Vec::with_capacity(PERSISTENCE_WRITE_BATCH_SIZE);
            }
        }
        if !batch.is_empty() {
            let id = self.next_id();
            self.queued
                .push_back(PersistenceCommand::Writes { id, writes: batch });
        }
    }

    fn enqueue_barrier(&mut self) {
        if self.barrier_enqueued {
            return;
        }
        self.barrier_enqueued = true;
        let id = self.next_id();
        self.queued
            .push_back(PersistenceCommand::DurabilityBarrier { id });
    }

    /// Transition at most one queued command into the one in-flight slot.
    /// An already dispatched command is never returned a second time.
    fn take_next(&mut self) -> Option<PersistenceCommand> {
        if self.in_flight.is_some() {
            return None;
        }
        let command = self.queued.pop_front()?;
        self.in_flight = Some(InFlightPersistenceCommand {
            id: command.id(),
            durability_barrier: command.is_durability_barrier(),
        });
        Some(command)
    }

    fn acknowledge(&mut self, ready: &PersistenceReady) -> bool {
        let Some(command) = self.in_flight.take() else {
            return false;
        };
        if command.id != ready.id || command.durability_barrier != ready.durability_barrier {
            self.in_flight = Some(command);
            return false;
        }
        match &ready.result {
            Ok(()) => {
                self.settled
                    .insert(command.id, PersistenceSettlement::Written);
                if ready.durability_barrier {
                    self.barrier_acknowledged = true;
                }
            }
            Err(error) => {
                self.failed = Some(Arc::clone(error));
                self.settled
                    .insert(command.id, PersistenceSettlement::Failed(Arc::clone(error)));
            }
        }
        true
    }

    /// Settle all still-owned commands during a terminal transition. A late
    /// worker completion is stale by construction because its in-flight slot
    /// has already been visibly settled as cancelled.
    fn cancel(&mut self) {
        if let Some(command) = self.in_flight.take() {
            self.settled
                .insert(command.id, PersistenceSettlement::Cancelled);
        }
        for command in self.queued.drain(..) {
            self.settled
                .insert(command.id(), PersistenceSettlement::Cancelled);
        }
    }

    fn settlement(&self, id: u64) -> Option<&PersistenceSettlement> {
        self.settled.get(&id)
    }
}

/// Local acquisition storage facade. It is the only synchronous local source
/// exposed to `InboundLedgerLocal`: validated fetch-pack/resident objects are
/// read through this adapter, while physical NodeStore reads arrive as typed
/// `NodeReadBroker` completions and are reduced by the same SHAMap validators.
///
/// It deliberately does not own NodeStore I/O. The broker may coalesce or
/// defer reads, but it cannot create a second semantic local-read path.
pub struct LocalHydratorStore {
    pending_writes: Vec<PersistenceWrite>,
    pending_keys: BTreeSet<PersistenceKey>,
    cache: Arc<FetchPackCache>,
}

impl LocalHydratorStore {
    fn take_pending_writes(&mut self) -> Vec<PersistenceWrite> {
        self.pending_keys.clear();
        std::mem::take(&mut self.pending_writes)
    }

    fn store_object(
        &mut self,
        object_type: nodestore::NodeObjectType,
        data: Vec<u8>,
        hash: Uint256,
        seq: u32,
    ) {
        let object_type_key = match object_type {
            nodestore::NodeObjectType::AccountNode => 1,
            nodestore::NodeObjectType::TransactionNode => 2,
            nodestore::NodeObjectType::Ledger => 3,
            _ => 0,
        };
        let key = PersistenceKey {
            hash,
            ledger_seq: seq,
            object_type: object_type_key,
        };
        if self.pending_keys.insert(key) {
            self.pending_writes.push(PersistenceWrite {
                key,
                object_type,
                data,
            });
        }
    }
}

impl InboundLedgerStore for LocalHydratorStore {
    fn fetch_ledger_header(&mut self, hash: SHAMapHash, _seq: u32) -> Option<Vec<u8>> {
        // Fetch-pack headers take the same `try_load_header_from_bytes`
        // validation path as every other local header. NodeStore headers are
        // delivered by the broker into `process_local_probe` instead of being
        // read synchronously under actor ownership.
        self.cache.get_fetch_pack(*hash.as_uint256())
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
        // The sync filter still validates content/path before it becomes map
        // state. This is the same local object source used by fetch-pack and
        // stale-state reuse, not a bypass around SHAMap admission.
        self.cache.get_fetch_pack(hash)
    }
}

pub struct AcqMutableState {
    pub inbound: InboundLedgerLocal,
    pub store: LocalHydratorStore,
    pub(crate) fetch_pack: WorkerFetchPack,
}

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

/// One packet remains exclusively owned by this acquisition until all of its
/// 128-node application steps have settled. It is never silently dropped on a
/// full mailbox; ingress receives an explicit rejection instead.
struct PacketWork {
    peer_id: u64,
    packet: InboundLedgerPacket,
    next_node: usize,
    bytes: usize,
}

struct ActorTreePlan {
    plan: TreePlan,
    reason: InboundLedgerRequestTrigger,
    peer: Option<Arc<dyn Peer>>,
    tickets: BTreeMap<SHAMapHash, ReadTicket>,
    /// Read needs removed from TreePlan's unannounced set but not yet admitted
    /// to the broker. This FIFO is the actor-owned admission backlog: it
    /// prevents a full completion mailbox from reannouncing the same hashes
    /// and repeatedly re-entering `TreePlan::advance` with zero branch work.
    read_admission_backlog: VecDeque<ReadNeed>,
    /// A plan is runnable only when it has retained CPU frontier. Pending
    /// broker/network work is deliberately waiting, not self-rescheduling.
    runnable: bool,
    /// Timeout retargeting may use `TMGetObjectByHash` only after the planner's
    /// existing aggressive threshold has been crossed.
    aggressive_by_hash: bool,
}

impl ActorTreePlan {
    fn retarget(
        &mut self,
        reason: InboundLedgerRequestTrigger,
        peer: Option<Arc<dyn Peer>>,
        aggressive_by_hash: bool,
    ) {
        let priority = |trigger| match trigger {
            InboundLedgerRequestTrigger::Timeout => 4,
            InboundLedgerRequestTrigger::ReplyHighLatency => 3,
            InboundLedgerRequestTrigger::Reply => 2,
            InboundLedgerRequestTrigger::Added => 1,
            InboundLedgerRequestTrigger::Blind => 0,
        };
        if reason == InboundLedgerRequestTrigger::Timeout {
            self.reason = reason;
            self.peer = None;
            self.plan.clear_recent_requests();
            let admission_backlog_waiting = !self.read_admission_backlog.is_empty();
            let retried = (!admission_backlog_waiting)
                .then(|| {
                    self.plan
                        .network_candidates()
                        .into_iter()
                        .filter(|(_, hash)| {
                            self.plan.retry_network_candidate(SHAMapHash::new(*hash))
                        })
                        .count()
                })
                .unwrap_or_default();
            // A lost peer reply wakes only the retained verified network
            // frontier. No plan is rebuilt and an empty frontier stays idle.
            // A callback-less broker rejection leaves no ticket to wake this
            // actor. Timeout is its bounded retry source; it drains the FIFO
            // directly without re-running TreePlan for the same hashes. Do
            // not append network retries while that FIFO remains blocked:
            // they cannot be emitted until the local read gate reopens.
            self.runnable |= retried != 0 || admission_backlog_waiting;
            self.aggressive_by_hash = aggressive_by_hash && retried != 0;
        } else if self.reason != InboundLedgerRequestTrigger::Timeout
            && priority(reason) >= priority(self.reason)
        {
            self.reason = reason;
            self.peer = peer;
        }
    }
}

/// A `Ready` result may be requeued only if the retained continuation actually
/// consumed a branch during this actor turn. This is deliberately checked at
/// the actor boundary: a stale/inconsistent continuation must become idle
/// rather than consume every worker with zero-work successor turns.
fn ready_turn_can_requeue(plan: &TreePlan, branch_steps_before: u64) -> bool {
    plan.has_runnable_frontier() && plan.branch_steps() > branch_steps_before
}

/// A deferred or rejected read admission leaves work waiting on the broker.
/// Deferred tickets retain a callback-bearing broker subscription; rejected
/// needs remain in the actor FIFO for a later retry. Neither may manufacture
/// an immediate successor turn.
fn needs_reads_turn_can_requeue(plan: &TreePlan, read_admission_blocked: bool) -> bool {
    !read_admission_blocked && plan.has_runnable_frontier()
}

const SCAN_OUTCOME_NOT_RUN: u8 = 0;
const SCAN_OUTCOME_READY: u8 = 1;
const SCAN_OUTCOME_NEEDS_READS: u8 = 2;
const SCAN_OUTCOME_NEEDS_NETWORK: u8 = 3;
const SCAN_OUTCOME_COMPLETE: u8 = 4;
const SCAN_OUTCOME_INVALID: u8 = 5;

fn scan_outcome_name(outcome: u8) -> &'static str {
    match outcome {
        SCAN_OUTCOME_READY => "ready",
        SCAN_OUTCOME_NEEDS_READS => "needs_reads",
        SCAN_OUTCOME_NEEDS_NETWORK => "needs_network",
        SCAN_OUTCOME_COMPLETE => "complete",
        SCAN_OUTCOME_INVALID => "invalid",
        SCAN_OUTCOME_NOT_RUN | _ => "not_run",
    }
}

/// Fully-owned peer work emitted by an actor turn. The target is cloned while
/// the detached plan is still available, so no plan/actor borrow survives the
/// outbound boundary.
struct OwnedOutboundRequest {
    message: overlay::ProtocolMessage,
    target: Option<Arc<dyn Peer>>,
}

/// The outbound boundary is deliberately expressed as a sequencing helper:
/// return the detached TreePlan to its mailbox first, then run the send hook.
/// Both ordinary and aggressive `NeedsNetwork` branches use this helper.
fn restore_tree_plan_before_peer_send(
    actor_plan: ActorTreePlan,
    restore_plan: impl FnOnce(ActorTreePlan),
    send_hook: impl FnOnce(),
) {
    restore_plan(actor_plan);
    send_hook();
}

fn send_owned_outbound_request(state: &AcquisitionState, outbound: OwnedOutboundRequest) {
    state
        .lifecycle
        .request_messages
        .fetch_add(1, Ordering::Relaxed);
    state
        .peer_set
        .send_request(&outbound.message, outbound.target.as_ref());
}

/// The bounded actor mailbox. Its reducer consumes, in order, one timeout,
/// one packet step, one persistence event, up to a fixed number of settled
/// broker results, and one tree CPU step before yielding. No tree plan owns
/// NodeStore I/O or a peer send.
struct AcquisitionMailbox {
    packets: VecDeque<PacketWork>,
    packet_bytes: usize,
    /// ReadReady is separately capacity-reserved before broker admission.
    /// This queue therefore cannot drop an accepted completion.
    events: VecDeque<ReadReady>,
    read_event_reservations: usize,
    persistence_events: VecDeque<PersistenceReady>,
    local_probes: BTreeMap<u64, LocalProbe>,
    completed_local_probes: BTreeSet<LocalProbeKind>,
    token: AcquisitionWorkToken,
    pending_timeouts: u32,
    plan: Option<ActorTreePlan>,
    /// Number of packets captured at the current `runData` batch boundary.
    /// Packets arriving after the first packet is claimed start the next epoch
    /// and cannot alter this batch's credit/prune/sample result.
    batch_packets_remaining: usize,
    batch_useful_peer_counts: BTreeMap<u64, i32>,
    buffered_packets_high_water: usize,
    buffered_bytes_high_water: usize,
    stale_events: u64,
    overload_rejections: u64,
}

impl Default for AcquisitionMailbox {
    fn default() -> Self {
        Self {
            packets: VecDeque::new(),
            packet_bytes: 0,
            events: VecDeque::new(),
            read_event_reservations: 0,
            persistence_events: VecDeque::new(),
            local_probes: BTreeMap::new(),
            completed_local_probes: BTreeSet::new(),
            token: AcquisitionWorkToken::Idle,
            pending_timeouts: 0,
            plan: None,
            batch_packets_remaining: 0,
            batch_useful_peer_counts: BTreeMap::new(),
            buffered_packets_high_water: 0,
            buffered_bytes_high_water: 0,
            stale_events: 0,
            overload_rejections: 0,
        }
    }
}

impl AcquisitionMailbox {
    fn buffered_packet_count(&self) -> usize {
        self.packets.len()
    }

    /// A runData batch owns the actor until its snapshotted packets have all
    /// reduced. Newly queued packets likewise begin a batch before unrelated
    /// timeout, persistence, broker, or tree-plan work can interleave.
    fn has_active_packet_batch(&self) -> bool {
        self.batch_packets_remaining != 0 || !self.packets.is_empty()
    }

    fn has_work(&self, fetch_pack_ready: bool) -> bool {
        !self.packets.is_empty()
            || !self.events.is_empty()
            || !self.persistence_events.is_empty()
            || self.pending_timeouts != 0
            || self.plan.as_ref().is_some_and(|plan| plan.runnable)
            || fetch_pack_ready
    }

    /// Wake a retained plan without recursively advancing it. An idle mailbox
    /// claims exactly one bounded actor turn; a running turn observes the
    /// runnable plan in `finish_acquisition_turn` and queues the next turn.
    fn wake_tree_plan(&mut self) -> bool {
        let Some(plan) = self.plan.as_mut() else {
            return false;
        };
        plan.runnable = true;
        if self.token == AcquisitionWorkToken::Idle {
            self.token = AcquisitionWorkToken::Queued;
            true
        } else {
            false
        }
    }

    fn record_late_read_event(&mut self) {
        self.stale_events += 1;
    }

    /// Finish one running actor turn. The caller submits exactly one successor
    /// when this reports remaining work, preserving ingress coalescing.
    fn finish_turn(&mut self, fetch_pack_ready: bool) -> bool {
        if self.has_work(fetch_pack_ready) {
            self.token = AcquisitionWorkToken::Queued;
            true
        } else {
            self.token = AcquisitionWorkToken::Idle;
            false
        }
    }

    /// Pop one settled completion and release exactly its pre-reserved slot.
    fn take_read_event(&mut self) -> Option<ReadReady> {
        let event = self.events.pop_front()?;
        self.read_event_reservations = self.read_event_reservations.saturating_sub(1);
        Some(event)
    }

    fn clear_terminal_work(&mut self) -> Vec<ReadTicket> {
        let mut tickets: Vec<ReadTicket> = self
            .plan
            .take()
            .map(|plan| plan.tickets.into_values().collect())
            .unwrap_or_default();
        // Local probes use the same broker admission as tree reads. Preserve
        // their tickets until terminal teardown so they cannot consume broker
        // capacity forever when their callback is never reduced.
        tickets.extend(self.local_probes.values().map(|probe| probe.ticket));
        self.packets.clear();
        self.packet_bytes = 0;
        self.events.clear();
        self.read_event_reservations = 0;
        self.persistence_events.clear();
        self.local_probes.clear();
        self.completed_local_probes.clear();
        self.pending_timeouts = 0;
        self.batch_packets_remaining = 0;
        self.batch_useful_peer_counts.clear();
        self.token = AcquisitionWorkToken::Idle;
        tickets
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
    read_broker: NodeReadBroker,
    persistence: Mutex<PersistenceQueue>,
    /// Terminal traversal freezes ingress/request generation while ordered
    /// persistence and its one durability barrier drain.
    draining: AtomicBool,
    next_plan_id: AtomicU64,
    pub shared_tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
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

    /// Enqueue one immutable packet. Saturation is visible to the caller so
    /// routing can charge/record overload instead of silently losing packet
    /// ownership.
    pub fn enqueue_packet(self: &Arc<Self>, peer_id: u64, packet: InboundLedgerPacket) -> bool {
        if self.is_done() || self.draining.load(Ordering::Acquire) {
            return false;
        }
        let bytes = packet
            .nodes
            .iter()
            .map(|node| node.node_data.len() + node.node_id.as_ref().map_or(0, Vec::len))
            .sum::<usize>();
        let should_enqueue = {
            let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
            if mailbox.packets.len() >= ACQ_MAILBOX_PACKET_CAPACITY
                || mailbox.packet_bytes.saturating_add(bytes) > ACQ_MAILBOX_BYTE_CAPACITY
            {
                mailbox.overload_rejections += 1;
                return false;
            }
            mailbox.packet_bytes += bytes;
            mailbox.packets.push_back(PacketWork {
                peer_id,
                packet,
                next_node: 0,
                bytes,
            });
            mailbox.buffered_packets_high_water = mailbox
                .buffered_packets_high_water
                .max(mailbox.buffered_packet_count());
            mailbox.buffered_bytes_high_water =
                mailbox.buffered_bytes_high_water.max(mailbox.packet_bytes);
            if mailbox.token == AcquisitionWorkToken::Idle {
                mailbox.token = AcquisitionWorkToken::Queued;
                true
            } else {
                self.lifecycle
                    .data_jobs_coalesced
                    .fetch_add(1, Ordering::Relaxed);
                false
            }
        };
        if should_enqueue {
            self.enqueue_acquisition_turn();
        }
        true
    }

    fn enqueue_read_ready(self: &Arc<Self>, ready: ReadReady) {
        if self.is_done() {
            // Broker cancellation and NodeStore completion may race terminal
            // teardown. The event has no remaining owner, but is visible in
            // diagnostics instead of being silently discarded.
            self.mailbox
                .lock()
                .expect("acquisition mailbox lock")
                .record_late_read_event();
            return;
        }
        let should_enqueue = {
            let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
            // A reservation is created before every accepted broker ticket.
            // Reaching it is a source invariant breach, so fail visibly rather
            // than discarding a settled completion and stranding its edges.
            if mailbox.events.len() >= mailbox.read_event_reservations {
                mailbox.stale_events += 1;
                drop(mailbox);
                self.mark_failed();
                return;
            }
            mailbox.events.push_back(ready);
            if mailbox.token == AcquisitionWorkToken::Idle {
                mailbox.token = AcquisitionWorkToken::Queued;
                true
            } else {
                false
            }
        };
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
        let (should_enqueue, cancelled) = {
            let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
            if self.is_done() {
                (false, mailbox.clear_terminal_work())
            } else {
                (
                    mailbox.finish_turn(self.fetch_pack_ready.load(Ordering::Acquire)),
                    Vec::new(),
                )
            }
        };
        for ticket in cancelled {
            self.read_broker.cancel(ticket);
        }
        if should_enqueue {
            self.enqueue_acquisition_turn();
        }
    }

    fn take_packet_for_turn(&self) -> Option<PacketWork> {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        if mailbox.batch_packets_remaining == 0 {
            // Snapshot the batch before removing its first packet. Concurrent
            // ingress after this point is intentionally a later runData epoch.
            mailbox.batch_packets_remaining = mailbox.packets.len();
        }
        let packet = mailbox.packets.pop_front()?;
        mailbox.packet_bytes = mailbox.packet_bytes.saturating_sub(packet.bytes);
        Some(packet)
    }

    fn restore_packet(&self, packet: PacketWork) {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        mailbox.packet_bytes += packet.bytes;
        mailbox.packets.push_front(packet);
    }

    fn record_stale_event(&self) {
        self.mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .record_late_read_event();
    }

    fn take_read_event(&self) -> Option<ReadReady> {
        self.mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .take_read_event()
    }

    fn reserve_read_event_slot(&self) -> bool {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        if mailbox.read_event_reservations >= super::read_broker::ACQ_READS_PER_ACQUISITION {
            mailbox.overload_rejections += 1;
            return false;
        }
        mailbox.read_event_reservations += 1;
        true
    }

    fn release_read_event_slot(&self) {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        mailbox.read_event_reservations = mailbox.read_event_reservations.saturating_sub(1);
    }

    fn enqueue_persistence_ready(self: &Arc<Self>, ready: PersistenceReady) {
        if self.is_done() {
            return;
        }
        let should_enqueue = {
            let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
            mailbox.persistence_events.push_back(ready);
            if mailbox.token == AcquisitionWorkToken::Idle {
                mailbox.token = AcquisitionWorkToken::Queued;
                true
            } else {
                false
            }
        };
        if should_enqueue {
            self.enqueue_acquisition_turn();
        }
    }

    fn take_persistence_event(&self) -> Option<PersistenceReady> {
        self.mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .persistence_events
            .pop_front()
    }

    fn local_probe_candidate(&self) -> Option<(LocalProbeKind, ReadKey)> {
        let mut mutable = self.lock_mutable("local probe candidate")?;
        let planner = mutable.inbound.planner_state();
        let candidate = if !planner.have_header {
            Some((LocalProbeKind::Header, *self.hash.as_uint256(), self.seq))
        } else {
            let ledger = mutable.inbound.ledger_mut()?;
            if !planner.have_state && ledger.state_map_mut().hash().is_zero() {
                Some((
                    LocalProbeKind::StateRoot,
                    *ledger.header().account_hash.as_uint256(),
                    ledger.header().seq,
                ))
            } else if planner.have_state
                && !planner.have_transactions
                && ledger.tx_map_mut().hash().is_zero()
                && !ledger.header().tx_hash.is_zero()
            {
                Some((
                    LocalProbeKind::TransactionRoot,
                    *ledger.header().tx_hash.as_uint256(),
                    ledger.header().seq,
                ))
            } else {
                None
            }
        };
        candidate.map(|(kind, hash, seq)| (kind, ReadKey::new(hash, seq, 0)))
    }

    /// Returns true when a local probe is in flight, so callers must not issue
    /// a network request until that explicit broker outcome is reduced.
    fn request_next_local_probe(
        self: &Arc<Self>,
        reason: InboundLedgerRequestTrigger,
        peer: Option<Arc<dyn Peer>>,
    ) -> bool {
        let Some((kind, key)) = self.local_probe_candidate() else {
            return false;
        };
        {
            let mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
            if mailbox.completed_local_probes.contains(&kind)
                || mailbox
                    .local_probes
                    .values()
                    .any(|probe| probe.kind == kind)
            {
                return mailbox
                    .local_probes
                    .values()
                    .any(|probe| probe.kind == kind);
            }
        }
        if !self.reserve_read_event_slot() {
            return false;
        }
        let weak = Arc::downgrade(self);
        let sink: ReadReadySink = Arc::new(move |ready| {
            if let Some(state) = weak.upgrade() {
                state.enqueue_read_ready(ready);
            }
        });
        match self
            .read_broker
            .request(key, self.acquisition_id, LOCAL_PROBE_PLAN_ID, sink)
        {
            ReadAdmission::Accepted(ticket)
            | ReadAdmission::Deferred(ticket)
            | ReadAdmission::Attached(ticket) => {
                self.mailbox
                    .lock()
                    .expect("acquisition mailbox lock")
                    .local_probes
                    .insert(
                        ticket.id().get(),
                        LocalProbe {
                            ticket,
                            kind,
                            reason,
                            peer,
                        },
                    );
                self.read_broker
                    .submit_ready_to_node_store(&self.node_store);
                true
            }
            ReadAdmission::Rejected(_) => {
                self.release_read_event_slot();
                false
            }
        }
    }

    fn take_local_probe(&self, ready: &ReadReady) -> Option<LocalProbe> {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        let ticket_id = ready.ticket.id().get();
        let probe = mailbox.local_probes.get(&ticket_id)?;
        if probe.ticket != ready.ticket {
            mailbox.record_late_read_event();
            return None;
        }
        let probe = mailbox
            .local_probes
            .remove(&ticket_id)
            .expect("checked local probe must remain present");
        mailbox.completed_local_probes.insert(probe.kind);
        Some(probe)
    }

    /// Submit collected write commands only after an actor mutation guard has
    /// been released. One command remains in flight until its mailbox ack is
    /// reduced, preserving write order and the terminal barrier ordering.
    fn submit_persistence_writes(self: &Arc<Self>, writes: Vec<PersistenceWrite>) {
        if writes.is_empty() || self.is_done() {
            return;
        }
        self.persistence
            .lock()
            .expect("persistence queue lock")
            .enqueue_writes(writes);
        self.dispatch_next_persistence_command();
    }

    fn request_durability_barrier(self: &Arc<Self>) {
        if self.is_done() {
            return;
        }
        self.persistence
            .lock()
            .expect("persistence queue lock")
            .enqueue_barrier();
        self.dispatch_next_persistence_command();
    }

    fn dispatch_next_persistence_command(self: &Arc<Self>) {
        let command = self
            .persistence
            .lock()
            .expect("persistence queue lock")
            .take_next();
        let Some(command) = command else {
            return;
        };
        let state = Arc::clone(self);
        let node_store = state.node_store.clone();
        self.worker_pool.submit_ledger_data(Box::new(move || {
            let (id, durability_barrier, result) = match command {
                PersistenceCommand::Writes { id, writes } => {
                    let result = writes.into_iter().try_for_each(|write| match &node_store {
                        SHAMapStoreNodeStore::Single(database) => database.store(
                            write.object_type,
                            write.data,
                            write.key.hash,
                            write.key.ledger_seq,
                        ),
                        SHAMapStoreNodeStore::Rotating(database) => database.store(
                            write.object_type,
                            write.data,
                            write.key.hash,
                            write.key.ledger_seq,
                        ),
                    });
                    (id, false, result.map_err(|error| Arc::from(error.as_str())))
                }
                PersistenceCommand::DurabilityBarrier { id } => {
                    let result = match &node_store {
                        SHAMapStoreNodeStore::Single(database) => database.sync_result(),
                        SHAMapStoreNodeStore::Rotating(database) => database.sync_result(),
                    };
                    (id, true, result.map_err(|error| Arc::from(error.as_str())))
                }
            };
            state.enqueue_persistence_ready(PersistenceReady {
                id,
                result,
                durability_barrier,
            });
        }));
    }

    /// Record one completed packet's useful nodes and, only after observing
    /// its coalesced FIFO queue empty under this same mailbox lock,
    /// prune/sample the entire batch for reply triggers.
    fn finish_packet_batch(
        &self,
        peer_id: u64,
        useful_nodes: u64,
        packet_complete: bool,
    ) -> Vec<u64> {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        if self.is_done() {
            let cancelled = mailbox.clear_terminal_work();
            drop(mailbox);
            for ticket in cancelled {
                self.read_broker.cancel(ticket);
            }
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

        if !packet_complete {
            return Vec::new();
        }
        // This decrements only for a fully reduced packet. A bounded 128-node
        // actor turn is an implementation yield, never a peer-credit boundary.
        mailbox.batch_packets_remaining = mailbox.batch_packets_remaining.saturating_sub(1);
        if mailbox.batch_packets_remaining != 0 {
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
                mailbox.plan.as_ref().is_some_and(|plan| plan.runnable),
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

    fn install_tree_plan(&self, plan: ActorTreePlan) -> bool {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        if self.is_done() || mailbox.plan.is_some() {
            return false;
        }
        mailbox.plan = Some(plan);
        self.stats
            .state_scan_continuations
            .fetch_add(1, Ordering::Relaxed);
        true
    }

    fn take_tree_plan(&self) -> Option<ActorTreePlan> {
        self.mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .plan
            .take()
    }

    fn restore_tree_plan(&self, plan: ActorTreePlan) {
        let mut pending = Some(plan);
        let rejected_tickets = {
            let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
            if !self.is_done() && mailbox.plan.is_none() {
                mailbox.plan = pending.take();
                self.stats.state_scan_yields.fetch_add(1, Ordering::Relaxed);
                Vec::new()
            } else {
                // A terminal transition can race while the actor temporarily
                // owns its plan. Never drop newly admitted/deferred broker
                // tickets in that gap; settle them after releasing the
                // mailbox lock just like ordinary terminal teardown.
                pending
                    .take()
                    .expect("detached tree plan must remain owned until restoration")
                    .tickets
                    .into_values()
                    .collect::<Vec<_>>()
            }
        };
        for ticket in rejected_tickets {
            self.read_broker.cancel(ticket);
        }
    }

    fn wake_tree_plan(self: &Arc<Self>) {
        let should_enqueue = self
            .mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .wake_tree_plan();
        if should_enqueue {
            self.enqueue_acquisition_turn();
        }
    }

    fn retarget_tree_plan(
        &self,
        reason: InboundLedgerRequestTrigger,
        peer: Option<Arc<dyn Peer>>,
    ) -> Option<bool> {
        // Query the planner before taking the mailbox lock. This retains the
        // exact `> 4`, no-progress, by-hash gate used by `prepare_trigger`
        // without re-entering its whole-map request scan or replacing a plan.
        let aggressive_by_hash = reason == InboundLedgerRequestTrigger::Timeout
            && self
                .lock_mutable("timeout tree-plan policy")
                .is_some_and(|mutable| mutable.inbound.should_use_aggressive_by_hash());
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        let Some(plan) = mailbox.plan.as_mut() else {
            return None;
        };
        plan.retarget(reason, peer, aggressive_by_hash);
        Some(plan.runnable)
    }

    fn has_tree_plan(&self) -> bool {
        self.mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .plan
            .as_ref()
            .is_some_and(|plan| plan.runnable)
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
            mailbox_events,
            mailbox_bytes,
            mailbox_bytes_high_water,
            stale_events,
            overload_rejections,
            active_plan_id,
            active_plan_kind,
            plan_pending_hashes,
            plan_pending_edges,
            mailbox_token,
            scan_continuation_pending,
            pending_admitted_timeouts,
        ) = {
            let mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
            (
                false,
                mailbox.buffered_packet_count(),
                mailbox.buffered_packets_high_water,
                mailbox.events.len(),
                mailbox.packet_bytes,
                mailbox.buffered_bytes_high_water,
                mailbox.stale_events,
                mailbox.overload_rejections,
                mailbox.plan.as_ref().map(|plan| plan.plan.id().get()),
                mailbox.plan.as_ref().map(|plan| match plan.plan.kind() {
                    TreeKind::State => "state",
                    TreeKind::Transaction => "transaction",
                }),
                mailbox
                    .plan
                    .as_ref()
                    .map_or(0, |plan| plan.plan.pending_hashes()),
                mailbox
                    .plan
                    .as_ref()
                    .map_or(0, |plan| plan.plan.pending_edges()),
                mailbox.token.name(),
                mailbox.plan.is_some(),
                mailbox.pending_timeouts,
            )
        };
        let broker = self.read_broker.snapshot();
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
            state_scan_last_yield: scan_outcome_name(
                self.stats.state_scan_last_outcome.load(Ordering::Relaxed),
            ),
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
            state_scan_read_slot_full: self.stats.state_scan_read_slot_full.load(Ordering::Relaxed),
            state_scan_read_admission_accepted: self
                .stats
                .state_scan_read_admission_accepted
                .load(Ordering::Relaxed),
            state_scan_read_admission_deferred: self
                .stats
                .state_scan_read_admission_deferred
                .load(Ordering::Relaxed),
            state_scan_read_admission_attached: self
                .stats
                .state_scan_read_admission_attached
                .load(Ordering::Relaxed),
            state_scan_read_broker_rejected: self
                .stats
                .state_scan_read_broker_rejected
                .load(Ordering::Relaxed),
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
            state_scan_continuations: self.stats.state_scan_continuations.load(Ordering::Relaxed),
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
            mailbox_bytes,
            mailbox_bytes_high_water,
            mailbox_events,
            stale_events,
            overload_rejections,
            active_plan_id,
            active_plan_kind,
            plan_pending_hashes,
            plan_pending_edges,
            broker_queued_keys: broker.queued_keys,
            broker_in_flight_keys: broker.in_flight_keys,
            mailbox_token,
            scan_continuation_pending,
            pending_admitted_timeouts,
            has_active_packet,
        }
    }

    /// Explicit terminal cancellation runs before a registry sweep destroys
    /// the handle. It settles broker subscriptions and packet ownership rather
    /// than relying on a later callback to notice a stopped flag.
    pub(crate) fn cancel(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        self.persistence
            .lock()
            .expect("persistence queue lock")
            .cancel();
        let tickets = self
            .mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .clear_terminal_work();
        for ticket in tickets {
            self.read_broker.cancel(ticket);
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
        self.persistence
            .lock()
            .expect("persistence queue lock")
            .cancel();
        self.failed.store(true, Ordering::Release);
        self.stopped.store(true, Ordering::Release);
        // Failure can be raised outside a normal actor turn (for example from
        // a persistence acknowledgement or a poisoned mutable guard). Settle
        // all retained tree and local-probe subscriptions here rather than
        // relying on a later sweep or callback to reclaim broker capacity.
        let tickets = self
            .mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .clear_terminal_work();
        for ticket in tickets {
            self.read_broker.cancel(ticket);
        }
        self.lifecycle
            .terminal_failed
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn take_buffered_packets(&self) -> Vec<ledger::InboundLedgerReceivedPacket> {
        let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
        let packets = std::mem::take(&mut mailbox.packets);
        mailbox.packet_bytes = 0;
        packets
            .into_iter()
            .map(|work| ledger::InboundLedgerReceivedPacket::new(Some(work.peer_id), work.packet))
            .collect()
    }

    fn has_active_packet_batch(&self) -> bool {
        self.mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .has_active_packet_batch()
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
    pub read_broker: NodeReadBroker,
    pub tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    pub fetch_pack: Arc<FetchPackCache>,
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
                store: LocalHydratorStore {
                    pending_writes: Vec::new(),
                    pending_keys: BTreeSet::new(),
                    cache: Arc::clone(&self.fetch_pack),
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
            read_broker: self.read_broker,
            persistence: Mutex::new(PersistenceQueue::default()),
            draining: AtomicBool::new(false),
            next_plan_id: AtomicU64::new(1),
            shared_tree_cache: self.tree_cache,
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

struct ActorNodeFetcher;

impl shamap::family::SHAMapNodeFetcher for ActorNodeFetcher {
    fn fetch_node_object(
        &self,
        _hash: SHAMapHash,
        _ledger_seq: u32,
    ) -> Option<shamap::node_object::NodeObject> {
        // Acquisition reads are exclusively brokered asynchronously. Packet
        // verification may consult resident/fetch-pack data but never falls
        // through to a synchronous NodeStore read while actor state is held.
        None
    }
}

fn family<'a>(
    state: &'a AcquisitionState,
) -> SHAMapFamily<
    MonotonicClock,
    HardenedHashBuilder,
    &'a FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>,
    ActorNodeFetcher,
    NullMissingNodeReporter,
    (),
> {
    SHAMapFamily::new(
        Arc::clone(&state.shared_tree_cache),
        &state.worker_full_below,
        ActorNodeFetcher,
        NullMissingNodeReporter,
    )
}

fn trigger(
    state: &Arc<AcquisitionState>,
    reason: InboundLedgerRequestTrigger,
    peer: Option<Arc<dyn Peer>>,
) {
    if state.is_done() || state.draining.load(Ordering::Acquire) {
        return;
    }
    if state.request_next_local_probe(reason, peer.clone()) {
        return;
    }
    state
        .lifecycle
        .request_triggers
        .fetch_add(1, Ordering::Relaxed);
    if let Some(should_wake) = state.retarget_tree_plan(reason, peer.clone()) {
        if should_wake {
            // If the trigger arrived while idle, claim one bounded actor turn.
            // A running turn coalesces this wake and observes `runnable` in
            // `finish_acquisition_turn`.
            state.wake_tree_plan();
        }
        return;
    }

    // Planner mutation is isolated from command emission. In particular, peer
    // sends happen only after this guard is dropped.
    let (messages, plan, terminal) = {
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
        let kind = if setup.state_plan {
            Some(TreeKind::State)
        } else if setup.tx_plan {
            Some(TreeKind::Transaction)
        } else {
            None
        };
        let plan = kind
            .and_then(|kind| {
                inbound.start_tree_plan(
                    kind,
                    TreePlanId::new(state.next_plan_id.fetch_add(1, Ordering::Relaxed)),
                    state.worker_full_below.generation(),
                )
            })
            .map(|plan| ActorTreePlan {
                plan,
                reason,
                peer: peer.clone(),
                tickets: BTreeMap::new(),
                read_admission_backlog: VecDeque::new(),
                runnable: true,
                aggressive_by_hash: false,
            });
        (
            setup.messages_to_send,
            plan,
            inbound.is_failed() || inbound.is_complete(),
        )
    };

    for message in messages {
        if state.is_done() {
            return;
        }
        state
            .lifecycle
            .request_messages
            .fetch_add(1, Ordering::Relaxed);
        state.peer_set.send_request(&message, peer.as_ref());
    }
    if let Some(plan) = plan
        && state.install_tree_plan(plan)
    {
        state.submit_data_job();
    }
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
    let (added, persistence_writes, terminal) = {
        let Some(mut mutable) = state.lock_mutable("initialization") else {
            return;
        };
        check_local(state, &mut mutable);
        let persistence_writes = mutable.store.take_pending_writes();
        let terminal = mutable.inbound.is_failed() || mutable.inbound.is_complete();
        let added = (!terminal).then(|| add_peers(state)).unwrap_or_default();
        (added, persistence_writes, terminal)
    };
    state.submit_persistence_writes(persistence_writes);
    if terminal {
        finalize_terminal(state);
        return;
    }
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

    // `runData` owns its snapshotted packet batch through max-peer credit,
    // prune, and reply sampling. Reduce exactly one chunk, then yield without
    // allowing timeout, persistence, broker, or planner work to interleave.
    if state.has_active_packet_batch() {
        process_data_job(state);
        state.finish_acquisition_turn();
        return;
    }

    // Event-first fairness applies only between complete packet-batch epochs.
    if state.take_admitted_timeout() {
        process_timeout_job(state);
    }
    if !state.is_done() && state.fetch_pack_ready.load(Ordering::Acquire) {
        process_data_job(state);
    }
    if !state.is_done() {
        process_persistence_event(state);
    }
    if !state.is_done() {
        process_read_events(state);
    }
    if !state.is_done() && state.has_tree_plan() {
        process_tree_plan_turn(state);
    }
    state.finish_acquisition_turn();
}

/// Apply peer nodes that packet validation has already accepted to a retained
/// plan. A matching network attachment may unblock deferred parent resumes;
/// that is runnable actor work, not a passive cache update.
fn apply_verified_peer_nodes_to_plan(
    actor_plan: &mut ActorTreePlan,
    nodes: impl IntoIterator<
        Item = basics::intrusive_pointer::SharedIntrusive<shamap::tree_node::SHAMapTreeNode>,
    >,
) -> bool {
    let plan_id = actor_plan.plan.id();
    let mut unblocked_deferred_parent = false;
    for node in nodes {
        let hash = node.get_hash();
        if matches!(
            actor_plan.plan.apply_network_node(plan_id, hash, node),
            MissingNodeReadApply::Applied { attached_edges, .. } if attached_edges != 0
        ) {
            unblocked_deferred_parent = true;
        }
    }
    if unblocked_deferred_parent && actor_plan.plan.has_runnable_frontier() {
        actor_plan.runnable = true;
        return true;
    }
    false
}

fn apply_packet_nodes_to_plan(
    state: &Arc<AcquisitionState>,
    packet_type: InboundLedgerDataType,
    nodes: Vec<basics::intrusive_pointer::SharedIntrusive<shamap::tree_node::SHAMapTreeNode>>,
) {
    let expected_kind = match packet_type {
        InboundLedgerDataType::StateNode => TreeKind::State,
        InboundLedgerDataType::TransactionNode => TreeKind::Transaction,
        InboundLedgerDataType::Base => return,
    };
    let Some(mut actor_plan) = state.take_tree_plan() else {
        return;
    };
    if actor_plan.plan.kind() != expected_kind {
        state.restore_tree_plan(actor_plan);
        return;
    }
    let wake_bounded_turn = apply_verified_peer_nodes_to_plan(&mut actor_plan, nodes);
    state.restore_tree_plan(actor_plan);
    if wake_bounded_turn {
        // Idle callers claim one queued turn immediately. During a running
        // packet turn, `finish_acquisition_turn` observes `runnable` and
        // queues the same bounded continuation without recursion.
        state.wake_tree_plan();
    }
}

fn process_data_job(state: &Arc<AcquisitionState>) {
    if state.is_done() {
        return;
    }

    // Exactly one prevalidated packet chunk per actor turn. The remaining
    // packet keeps FIFO ownership and is restored before later packets.
    let Some(mut work) = state.take_packet_for_turn() else {
        let (terminal, persistence_writes) = {
            let Some(mut mutable) = state.lock_mutable("data processing") else {
                return;
            };
            let data_drain_started = Instant::now();
            if state.fetch_pack_ready.swap(false, Ordering::AcqRel) {
                check_local(state, &mut mutable);
            }
            let terminal = mutable.inbound.is_failed() || mutable.inbound.is_complete();
            let persistence_writes = mutable.store.take_pending_writes();
            state
                .stats
                .record_data_drain(data_drain_started.elapsed().as_micros() as u64, 0);
            (terminal, persistence_writes)
        };
        state.submit_persistence_writes(persistence_writes);
        if terminal {
            finalize_terminal(state);
        }
        return;
    };

    let mut accepted_nodes = Vec::new();
    let data_drain_started = Instant::now();
    let packet_type = work.packet.packet_type;
    let peer_id = work.peer_id;
    let mut packet_stats = SHAMapAddNode::default();
    let mut node_failures = Vec::new();
    let mut packet_complete = false;
    let mut malformed = None;
    // Base packets retain their atomic header/root semantics. Node-packet
    // failures are represented by typed per-node outcomes and charged below.
    let mut base_packet_invalid = false;
    let mut had_header = false;
    let terminal;
    let persistence_writes;
    {
        let Some(mut mutable) = state.lock_mutable("data processing") else {
            return;
        };
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
            match inbound.process_packet_step_with_family_and_config(
                &work.packet,
                work.next_node,
                ledger::INBOUND_LEDGER_MAX_PACKET_NODES_PER_STEP,
                &journal,
                &config,
                store,
                fetch_pack,
                &family,
            ) {
                Ok(step) => {
                    packet_stats = step.stats;
                    work.next_node = step.next_node;
                    packet_complete = step.complete;
                    if packet_type == InboundLedgerDataType::Base {
                        // A liBASE packet is one atomic header/root admission;
                        // preserve its packet-level invalid-data semantics.
                        base_packet_invalid = packet_stats.is_invalid();
                    } else {
                        for admission in step.node_admissions {
                            match admission {
                                InboundLedgerNodeAdmission::Accepted { index } => {
                                    // This decode is only a conversion for the
                                    // retained plan. Acceptance was decided by
                                    // SHAMap mutation above, never by wire shape.
                                    if let Some(node) = work.packet.nodes.get(index)
                                        && let Ok(Some(node)) = shamap::tree_node::SHAMapTreeNode::make_from_wire(&node.node_data)
                                    {
                                        accepted_nodes.push(node);
                                    } else {
                                        // An accepted mutation must have a
                                        // decodable tree node. Do not convert
                                        // this impossible mismatch into a plan
                                        // attachment; make it a peer-attributable
                                        // invalid-node failure instead.
                                        node_failures.push(InboundLedgerPacketError::InvalidNodeData);
                                    }
                                }
                                InboundLedgerNodeAdmission::Duplicate { .. } => {}
                                InboundLedgerNodeAdmission::Rejected { error, .. } => {
                                    node_failures.push(error);
                                }
                            }
                        }
                    }
                    inbound.record_packet_stats_with_family_and_config(
                        packet_stats,
                        &journal,
                        &config,
                        &family,
                    );
                }
                Err(error) => {
                    malformed = Some(error);
                    packet_complete = true;
                }
            }
        }
        terminal = inbound.is_failed() || inbound.is_complete();
        persistence_writes = store.take_pending_writes();
    }

    // The packet reducer has released `mutable`; physical NodeStore I/O now
    // enters the actor-external FIFO command queue.
    state.submit_persistence_writes(persistence_writes);

    let data_drain_us = data_drain_started.elapsed().as_micros() as u64;
    state.stats.record_data_drain(data_drain_us, 1);
    if state.stats.should_emit_sampled_diagnostic() {
        tracing::debug!(
            target: "inbound_ledger",
            seq = state.seq,
            hash = %state.hash,
            drain_us = data_drain_us,
            nodes = work.packet.nodes.len(),
            next_node = work.next_node,
            "sampled full inbound data packet"
        );
    }

    // Valid siblings survive a packet fault. The fetcher has already recorded
    // the bad node in `packet_stats`; this plan update is content-addressed and
    // filters malformed wire nodes independently.
    if !accepted_nodes.is_empty() {
        apply_packet_nodes_to_plan(state, packet_type, accepted_nodes);
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
    if packet_complete {
        state
            .lifecycle
            .packet_steps_completed
            .fetch_add(1, Ordering::Relaxed);
    } else {
        state.restore_packet(work);
    }
    state.stats.packets.fetch_add(1, Ordering::Relaxed);
    let useful_nodes = packet_stats.get_good().max(0) as u64;
    state
        .stats
        .useful_packets
        .fetch_add(u64::from(useful_nodes != 0), Ordering::Relaxed);
    state
        .stats
        .useful_nodes
        .fetch_add(useful_nodes, Ordering::Relaxed);
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
    let packet_error_count = usize::from(malformed.is_some())
        + usize::from(base_packet_invalid)
        + node_failures.len();
    state
        .lifecycle
        .packet_step_errors
        .fetch_add(packet_error_count as u64, Ordering::Relaxed);
    if let Some(error) = malformed {
        state
            .stats
            .malformed_packets
            .fetch_add(1, Ordering::Relaxed);
        charge_malformed_packet(state, peer_id, packet_type, error);
    }
    for error in node_failures {
        charge_rejected_node(state, peer_id, packet_type, error);
    }
    if base_packet_invalid {
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
    for reply_peer_id in state.finish_packet_batch(peer_id, useful_nodes, packet_complete) {
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
struct ActorResident<'a> {
    cache: &'a TreeNodeCache<MonotonicClock>,
}

impl MissingNodeResidentLookup for ActorResident<'_> {
    fn load_resident(
        &mut self,
        hash: SHAMapHash,
        _ledger_seq: u32,
    ) -> Option<basics::intrusive_pointer::SharedIntrusive<shamap::tree_node::SHAMapTreeNode>> {
        self.cache.fetch(hash.as_uint256())
    }
}

fn fail_actor_plan(state: &AcquisitionState, actor_plan: ActorTreePlan) {
    for ticket in actor_plan.tickets.into_values() {
        state.read_broker.cancel(ticket);
    }
    state.mark_failed();
}

fn process_persistence_event(state: &Arc<AcquisitionState>) {
    let Some(ready) = state.take_persistence_event() else {
        return;
    };
    let (acknowledged, failed) = {
        let mut queue = state.persistence.lock().expect("persistence queue lock");
        let acknowledged = queue.acknowledge(&ready);
        (acknowledged, queue.failed.clone())
    };
    if !acknowledged {
        state.record_stale_event();
        return;
    }
    if let Some(error) = failed {
        tracing::error!(target: "inbound_ledger", seq = state.seq, hash = %state.hash, %error,
            "acquisition persistence command failed");
        state.mark_failed();
        return;
    }
    state.dispatch_next_persistence_command();
    if ready.durability_barrier {
        finalize_durable_acquisition(state);
    }
}

fn process_local_probe(state: &Arc<AcquisitionState>, probe: LocalProbe, ready: ReadReady) {
    let ReadOutcome::Found(object) = ready.outcome else {
        // A miss/cancel/fault is an explicit completed local probe. It permits
        // the retained normal request policy to proceed; it is never progress.
        trigger(state, probe.reason, probe.peer);
        return;
    };
    let mut verified = false;
    let writes = match probe.kind {
        LocalProbeKind::Header => {
            let Some(mut mutable) = state.lock_mutable("brokered local header") else {
                return;
            };
            let config = ledger::LedgerConfig::default();
            let AcqMutableState { inbound, store, .. } = &mut *mutable;
            verified = inbound.apply_brokered_header(
                object.data().to_vec(),
                &config,
                store,
                &WorkerJournal,
            );
            store.take_pending_writes()
        }
        LocalProbeKind::StateRoot | LocalProbeKind::TransactionRoot => {
            let hash = SHAMapHash::new(ready.ticket.key().hash);
            let packet_type = match probe.kind {
                LocalProbeKind::StateRoot => InboundLedgerDataType::StateNode,
                LocalProbeKind::TransactionRoot => InboundLedgerDataType::TransactionNode,
                LocalProbeKind::Header => unreachable!(),
            };
            let packet = match shamap::tree_node::SHAMapTreeNode::make_from_prefix(object.data(), hash) {
                Ok(node) => match node.serialize_for_wire() {
                    Ok(wire) => Some(InboundLedgerPacket::new(
                        packet_type,
                        vec![ledger::InboundLedgerNodeData::new(
                            Some(shamap::node_id::SHAMapNodeId::default().get_raw_string()),
                            wire,
                        )],
                    )),
                    Err(_) => {
                        tracing::warn!(
                            target: "inbound_ledger",
                            seq = state.seq,
                            hash = %state.hash,
                            probe = ?probe.kind,
                            "local root could not serialize for validation; continuing peer acquisition"
                        );
                        None
                    }
                },
                Err(_) => {
                    tracing::warn!(
                        target: "inbound_ledger",
                        seq = state.seq,
                        hash = %state.hash,
                        probe = ?probe.kind,
                        "local root could not decode for validation; continuing peer acquisition"
                    );
                    None
                }
            };
            if let Some(packet) = packet {
                let Some(mut mutable) = state.lock_mutable("brokered local root") else {
                    return;
                };
                let journal = WorkerJournal;
                let config = ledger::LedgerConfig::default();
                let family = family(state);
                let AcqMutableState {
                    inbound,
                    store,
                    fetch_pack,
                } = &mut *mutable;
                match inbound.process_packet_step_with_family_and_config(
                    &packet,
                    0,
                    ledger::INBOUND_LEDGER_MAX_PACKET_NODES_PER_STEP,
                    &journal,
                    &config,
                    store,
                    fetch_pack,
                    &family,
                ) {
                    Ok(step) if !step.stats.is_invalid() => {
                        inbound.record_packet_stats_with_family_and_config(
                            step.stats, &journal, &config, &family,
                        );
                        verified = step.stats.is_useful();
                    }
                    Ok(_) => tracing::warn!(
                        target: "inbound_ledger",
                        seq = state.seq,
                        hash = %state.hash,
                        probe = ?probe.kind,
                        "local root failed SHAMap validation; continuing peer acquisition"
                    ),
                    Err(_) => tracing::warn!(
                        target: "inbound_ledger",
                        seq = state.seq,
                        hash = %state.hash,
                        probe = ?probe.kind,
                        "local root could not complete SHAMap validation; continuing peer acquisition"
                    ),
                }
                store.take_pending_writes()
            } else {
                Vec::new()
            }
        }
    };
    state.submit_persistence_writes(writes);
    if verified {
        if let Some(mut mutable) = state.lock_mutable("verified local probe progress") {
            mutable.inbound.record_verified_progress();
        }
    }
    if state.is_done() {
        finalize_terminal(state);
    } else {
        trigger(state, probe.reason, probe.peer);
    }
}

/// Drain a fixed number of settled read completions without holding the
/// mailbox lock while their actor-owned plan, persistence, or peer state is
/// reduced. A false result means the mailbox has no more safely runnable work.
fn drain_bounded_read_events(mut reduce_one: impl FnMut() -> bool) -> usize {
    let mut reduced = 0;
    for _ in 0..ACQ_TURN_MAX_READ_EVENTS {
        if !reduce_one() {
            break;
        }
        reduced += 1;
    }
    reduced
}

fn process_read_events(state: &Arc<AcquisitionState>) {
    let _ = drain_bounded_read_events(|| process_one_read_event(state));
}

/// Reduce one settled completion. Returning false leaves the mailbox untouched;
/// returning true means precisely one reservation was released and its event
/// was handled, including stale and terminal paths.
fn process_one_read_event(state: &Arc<AcquisitionState>) -> bool {
    if state.is_done() {
        return false;
    }
    let Some(ready) = state.take_read_event() else {
        return false;
    };
    if ready.ticket.acquisition_id() != state.acquisition_id {
        state.record_stale_event();
        return true;
    }
    if let Some(probe) = state.take_local_probe(&ready) {
        process_local_probe(state, probe, ready);
        return true;
    }
    let Some(mut actor_plan) = state.take_tree_plan() else {
        state.record_stale_event();
        return true;
    };
    if ready.ticket.acquisition_id() != state.acquisition_id
        || actor_plan.plan.id().get() != ready.ticket.plan_id()
    {
        state.record_stale_event();
        state.restore_tree_plan(actor_plan);
        return true;
    }
    let hash = SHAMapHash::new(ready.ticket.key().hash);
    actor_plan.tickets.remove(&hash);
    let outcome = match ready.outcome {
        ReadOutcome::Found(object) => {
            match shamap::tree_node::SHAMapTreeNode::make_from_prefix(object.data(), hash) {
                Ok(node) => MissingNodeReadOutcome::Found(node),
                Err(_) => {
                    tracing::trace!(
                        target: "inbound_ledger",
                        seq = state.seq,
                        hash = %state.hash,
                        node_hash = %hash,
                        "brokered local descendant was unusable; reducing as a local miss"
                    );
                    MissingNodeReadOutcome::Miss
                }
            }
        }
        ReadOutcome::Miss => MissingNodeReadOutcome::Miss,
        ReadOutcome::Cancelled => MissingNodeReadOutcome::Cancelled,
        ReadOutcome::Fault(_) => {
            tracing::warn!(
                target: "inbound_ledger",
                seq = state.seq,
                hash = %state.hash,
                node_hash = %hash,
                "brokered local descendant faulted; reducing as a local miss"
            );
            MissingNodeReadOutcome::Miss
        }
    };
    match actor_plan
        .plan
        .apply_read_result(TreePlanId::new(ready.ticket.plan_id()), hash, outcome)
    {
        MissingNodeReadApply::HashMismatch => fail_actor_plan(state, actor_plan),
        MissingNodeReadApply::Applied { attached_edges, .. } => {
            // Only a matching, verified attachment resets the timeout progress
            // clock. Misses, duplicate admissions, and stale results do not.
            if attached_edges != 0 {
                if let Some(mut mutable) = state.lock_mutable("verified broker read progress") {
                    mutable.inbound.record_verified_progress();
                }
            }
            // `take_read_event` released one reserved completion slot. If the
            // actor owns unadmitted needs, consume that freed capacity directly
            // instead of asking TreePlan to reannounce the same batch.
            actor_plan.runnable = !actor_plan.read_admission_backlog.is_empty()
                || actor_plan.plan.has_runnable_frontier();
            state.restore_tree_plan(actor_plan);
        }
        MissingNodeReadApply::Requeued => {
            actor_plan.runnable = !actor_plan.read_admission_backlog.is_empty();
            state.restore_tree_plan(actor_plan);
        }
        MissingNodeReadApply::Cancelled => {
            fail_actor_plan(state, actor_plan);
        }
        MissingNodeReadApply::StalePlan | MissingNodeReadApply::UnknownRead => {
            state.record_stale_event();
            state.restore_tree_plan(actor_plan);
        }
    }
    true
}

/// Admit retained TreePlan reads while there is mailbox completion capacity.
///
/// `TreePlan::advance` removes emitted needs from its unannounced set. When
/// admission is full, ownership moves to `read_admission_backlog`; those needs
/// are not converted back to `Rejected`, so the next completion retries this
/// FIFO directly and cannot create a zero-branch TreePlan turn.
fn submit_read_admission_backlog(state: &Arc<AcquisitionState>, mut actor_plan: ActorTreePlan) {
    let plan_id = actor_plan.plan.id();
    let mut read_admission_blocked = false;
    while let Some(need) = actor_plan.read_admission_backlog.pop_front() {
        // Reserve delivery capacity before accepting a broker ticket. Keep the
        // need in the actor FIFO when full; a later real completion releases a
        // reservation and wakes exactly one bounded admission attempt.
        if !state.reserve_read_event_slot() {
            read_admission_blocked = true;
            state
                .stats
                .state_scan_read_slot_full
                .fetch_add(1, Ordering::Relaxed);
            actor_plan.read_admission_backlog.push_front(need);
            break;
        }
        let key = ReadKey::new(*need.hash().as_uint256(), need.ledger_seq(), 0);
        let weak = Arc::downgrade(state);
        let sink: ReadReadySink = Arc::new(move |ready| {
            if let Some(state) = weak.upgrade() {
                state.enqueue_read_ready(ready);
            }
        });
        match state
            .read_broker
            .request(key, state.acquisition_id, plan_id.get(), sink)
        {
            ReadAdmission::Accepted(ticket) => {
                state
                    .stats
                    .state_scan_read_admission_accepted
                    .fetch_add(1, Ordering::Relaxed);
                actor_plan.tickets.insert(need.hash(), ticket);
            }
            ReadAdmission::Deferred(ticket) => {
                // A Deferred ticket is callback-bearing waiting work in the
                // broker FIFO. Do not self-requeue retained TreePlan frontier
                // before its matching ReadReady can make progress.
                read_admission_blocked = true;
                state
                    .stats
                    .state_scan_read_admission_deferred
                    .fetch_add(1, Ordering::Relaxed);
                actor_plan.tickets.insert(need.hash(), ticket);
            }
            ReadAdmission::Attached(ticket) => {
                state
                    .stats
                    .state_scan_read_admission_attached
                    .fetch_add(1, Ordering::Relaxed);
                actor_plan.tickets.insert(need.hash(), ticket);
            }
            ReadAdmission::Rejected(ReadRejectReason::Stopped) => {
                // A stopped broker cannot deliver a callback to drain this
                // FIFO. Settle all actor-owned tickets through terminal
                // failure rather than retaining unreachable local reads.
                state.release_read_event_slot();
                fail_actor_plan(state, actor_plan);
                return;
            }
            ReadAdmission::Rejected(_) => {
                read_admission_blocked = true;
                state
                    .stats
                    .state_scan_read_broker_rejected
                    .fetch_add(1, Ordering::Relaxed);
                state.release_read_event_slot();
                actor_plan.read_admission_backlog.push_front(need);
                break;
            }
        }
    }
    // A nonempty FIFO, a Deferred ticket, or a rejection is waiting work,
    // not a runnable TreePlan frontier. It is woken by ReadReady or an
    // existing external trigger such as timeout recovery.
    actor_plan.runnable = actor_plan.read_admission_backlog.is_empty()
        && needs_reads_turn_can_requeue(&actor_plan.plan, read_admission_blocked);
    state.restore_tree_plan(actor_plan);
    // The broker owns physical I/O and runs submission after the actor plan
    // has been returned to the mailbox.
    state
        .read_broker
        .submit_ready_to_node_store(&state.node_store);
}

fn process_tree_plan_turn(state: &Arc<AcquisitionState>) {
    let Some(mut actor_plan) = state.take_tree_plan() else {
        return;
    };
    if !actor_plan.read_admission_backlog.is_empty() {
        submit_read_admission_backlog(state, actor_plan);
        return;
    }
    let retained_reads = actor_plan
        .plan
        .take_read_admission_batch(ACQ_TURN_MAX_NEW_READS);
    if !retained_reads.is_empty() {
        actor_plan.read_admission_backlog.extend(retained_reads);
        submit_read_admission_backlog(state, actor_plan);
        return;
    }
    let started = Instant::now();
    let scan_before = actor_plan.plan.scan_stats();
    let branch_steps_before = scan_before.branch_steps;
    let mut resident = ActorResident {
        cache: &state.shared_tree_cache,
    };
    let advance = actor_plan.plan.advance(
        ACQ_TURN_MAX_BRANCH_STEPS,
        ACQ_TURN_MAX_NEW_READS,
        &mut resident,
        &mut || basics::random::rand_int_to(255u8),
    );
    let scan_after = actor_plan.plan.scan_stats();
    let branch_steps_after = scan_after.branch_steps;
    let branch_steps_delta = branch_steps_after.saturating_sub(branch_steps_before);
    let deferred_reads_delta = scan_after
        .pending_reads
        .saturating_sub(scan_before.pending_reads);
    let deferred_resumes_delta = scan_after
        .deferred_resumes
        .saturating_sub(scan_before.deferred_resumes);
    let missing_nodes_delta = scan_after
        .missing_recorded
        .saturating_sub(scan_before.missing_recorded);
    let outcome = match &advance {
        TreeAdvance::Ready => SCAN_OUTCOME_READY,
        TreeAdvance::NeedsReads(_) => SCAN_OUTCOME_NEEDS_READS,
        TreeAdvance::NeedsNetwork(_) => SCAN_OUTCOME_NEEDS_NETWORK,
        TreeAdvance::Complete => SCAN_OUTCOME_COMPLETE,
        TreeAdvance::Invalid => SCAN_OUTCOME_INVALID,
    };
    state.stats.state_scan_runs.fetch_add(1, Ordering::Relaxed);
    state
        .stats
        .state_scan_last_outcome
        .store(outcome, Ordering::Relaxed);
    state
        .stats
        .state_scan_branch_steps
        .fetch_add(branch_steps_delta, Ordering::Relaxed);
    state
        .stats
        .state_scan_missing_nodes_recorded
        .fetch_add(missing_nodes_delta, Ordering::Relaxed);
    state
        .stats
        .state_missing_nodes
        .fetch_add(missing_nodes_delta, Ordering::Relaxed);
    state.stats.state_scan_branches_seen.fetch_add(
        scan_after
            .branches_seen
            .saturating_sub(scan_before.branches_seen),
        Ordering::Relaxed,
    );
    state.stats.state_scan_duplicate_missing_hashes.fetch_add(
        scan_after
            .duplicate_missing_hashes
            .saturating_sub(scan_before.duplicate_missing_hashes),
        Ordering::Relaxed,
    );
    state.stats.state_scan_full_below_hits.fetch_add(
        scan_after
            .full_below_hits
            .saturating_sub(scan_before.full_below_hits),
        Ordering::Relaxed,
    );
    state.stats.state_scan_loaded_or_cached_children.fetch_add(
        scan_after
            .loaded_or_cached_children
            .saturating_sub(scan_before.loaded_or_cached_children),
        Ordering::Relaxed,
    );
    state.stats.state_scan_pending_hits.fetch_add(
        scan_after
            .completed_pending_reads
            .saturating_sub(scan_before.completed_pending_reads),
        Ordering::Relaxed,
    );
    state.stats.state_scan_pending_misses.fetch_add(
        scan_after
            .completed_pending_misses
            .saturating_sub(scan_before.completed_pending_misses),
        Ordering::Relaxed,
    );
    state
        .stats
        .state_scan_deferred_resumes
        .fetch_add(deferred_resumes_delta, Ordering::Relaxed);
    state
        .stats
        .state_scan_max_pending_reads
        .fetch_max(scan_after.max_pending_reads, Ordering::Relaxed);
    state
        .stats
        .state_scan_pending_reads
        .store(actor_plan.plan.pending_hashes() as u64, Ordering::Relaxed);
    state
        .stats
        .state_scan_last_branch_steps
        .store(branch_steps_delta, Ordering::Relaxed);
    state
        .stats
        .state_scan_last_deferred_reads
        .store(deferred_reads_delta, Ordering::Relaxed);
    state
        .stats
        .state_scan_last_deferred_resumes
        .store(deferred_resumes_delta, Ordering::Relaxed);
    state
        .stats
        .state_scan_last_missing_nodes
        .store(missing_nodes_delta, Ordering::Relaxed);
    if branch_steps_delta != 0 {
        state
            .stats
            .state_scan_positive_progress_slices
            .fetch_add(1, Ordering::Relaxed);
    }
    match &advance {
        TreeAdvance::Ready | TreeAdvance::NeedsReads(_) | TreeAdvance::NeedsNetwork(_) => {
            state
                .stats
                .state_scan_yields
                .fetch_add(1, Ordering::Relaxed);
        }
        TreeAdvance::Complete => {
            state
                .stats
                .state_scan_completed_slices
                .fetch_add(1, Ordering::Relaxed);
        }
        TreeAdvance::Invalid => {}
    }
    if outcome == SCAN_OUTCOME_READY && branch_steps_delta == ACQ_TURN_MAX_BRANCH_STEPS as u64 {
        state
            .stats
            .state_scan_branch_budget_yields
            .fetch_add(1, Ordering::Relaxed);
    }
    if outcome == SCAN_OUTCOME_NEEDS_READS && deferred_reads_delta == ACQ_TURN_MAX_NEW_READS as u64
    {
        state
            .stats
            .state_scan_deferred_read_budget_yields
            .fetch_add(1, Ordering::Relaxed);
    }
    if deferred_resumes_delta != 0 && outcome == SCAN_OUTCOME_READY {
        state
            .stats
            .state_scan_deferred_read_resume_yields
            .fetch_add(1, Ordering::Relaxed);
    }
    state
        .stats
        .state_scan_us
        .fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
    match advance {
        TreeAdvance::Invalid => fail_actor_plan(state, actor_plan),
        TreeAdvance::Complete => {
            let kind = actor_plan.plan.kind();
            let terminal = state
                .lock_mutable("complete tree plan")
                .is_some_and(|mut mutable| {
                    mutable.inbound.complete_tree_plan(kind);
                    mutable.inbound.is_failed() || mutable.inbound.is_complete()
                });
            if terminal {
                finalize_terminal(state);
            } else {
                trigger(state, actor_plan.reason, actor_plan.peer);
            }
        }
        TreeAdvance::NeedsReads(reads) => {
            actor_plan.read_admission_backlog.extend(reads);
            submit_read_admission_backlog(state, actor_plan);
        }
        TreeAdvance::NeedsNetwork(candidates) => {
            let limit = match actor_plan.reason {
                InboundLedgerRequestTrigger::Reply
                | InboundLedgerRequestTrigger::ReplyHighLatency => {
                    INBOUND_LEDGER_REPLY_NODE_REQUEST_CAP
                }
                _ => INBOUND_LEDGER_NORMAL_NODE_REQUEST_CAP,
            };
            let candidates = candidates
                .into_iter()
                .filter(|(_, hash)| actor_plan.plan.mark_request_candidate(*hash))
                .take(limit)
                .collect::<Vec<_>>();
            let mut consume_aggressive_by_hash = false;
            let outbound = if candidates.is_empty() {
                None
            } else if actor_plan.reason == InboundLedgerRequestTrigger::Timeout
                && actor_plan.aggressive_by_hash
            {
                let object_type = match actor_plan.plan.kind() {
                    TreeKind::State => InboundLedgerObjectType::StateNode,
                    TreeKind::Transaction => InboundLedgerObjectType::TransactionNode,
                };
                let needed = candidates
                    .iter()
                    .map(|(_, hash)| (object_type, *hash))
                    .collect::<Vec<_>>();
                make_inbound_needed_by_hash_request(state.hash, state.seq, &needed).map(|message| {
                    // Commit this retained-plan transition before the peer can
                    // synchronously re-enter any acquisition-facing path.
                    actor_plan.aggressive_by_hash = false;
                    consume_aggressive_by_hash = true;
                    OwnedOutboundRequest {
                        message,
                        target: None,
                    }
                })
            } else {
                let ids = candidates
                    .iter()
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                let depth = match actor_plan.reason {
                    InboundLedgerRequestTrigger::Reply => 1,
                    InboundLedgerRequestTrigger::ReplyHighLatency => 2,
                    _ => 0,
                };
                let itype = match actor_plan.plan.kind() {
                    TreeKind::State => 2,
                    TreeKind::Transaction => 1,
                };
                let message = make_get_ledger_with_node_ids(
                    state.hash,
                    state.seq,
                    itype,
                    &ids,
                    depth,
                    (actor_plan.reason == InboundLedgerRequestTrigger::Timeout).then_some(0),
                );
                // Clone the reply target into the owned command before the
                // detached plan is returned to the actor mailbox.
                let target = (actor_plan.reason != InboundLedgerRequestTrigger::Timeout)
                    .then(|| actor_plan.peer.clone())
                    .flatten();
                Some(OwnedOutboundRequest { message, target })
            };

            actor_plan.runnable = false;
            restore_tree_plan_before_peer_send(
                actor_plan,
                |actor_plan| state.restore_tree_plan(actor_plan),
                || {
                    if consume_aggressive_by_hash {
                        // The mutable planner transition also completes before
                        // peer send; this scope cannot cross the boundary.
                        {
                            if let Some(mut mutable) =
                                state.lock_mutable("consume aggressive by-hash request")
                            {
                                mutable.inbound.set_by_hash(false);
                            }
                        }
                    }
                    if let Some(outbound) = outbound {
                        send_owned_outbound_request(state, outbound);
                    }
                },
            );
        }
        TreeAdvance::Ready => {
            actor_plan.runnable = ready_turn_can_requeue(&actor_plan.plan, branch_steps_before);
            state.restore_tree_plan(actor_plan);
        }
    }
}

/// Charge one peer-attributable node failure after the corresponding SHAMap
/// admission has settled. Unlike a base packet, a bad node never erases a
/// useful sibling or turns that sibling's peer credit into a packet-wide fee.
fn charge_rejected_node(
    state: &AcquisitionState,
    peer_id: u64,
    packet_type: ledger::InboundLedgerDataType,
    error: InboundLedgerPacketError,
) {
    match error {
        InboundLedgerPacketError::MissingNodeId
        | InboundLedgerPacketError::EmptyNodeData
        | InboundLedgerPacketError::InvalidNodeId
        | InboundLedgerPacketError::InvalidNodeData => {
            charge_malformed_packet(state, peer_id, packet_type, error);
        }
        InboundLedgerPacketError::InvalidData
        | InboundLedgerPacketError::EmptyNodes
        | InboundLedgerPacketError::InvalidHeader => {
            charge_invalid_data_packet(state, peer_id, packet_type, error);
        }
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
    state
        .stats
        .timeout_dispatches
        .fetch_add(1, Ordering::Relaxed);
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
    let persistence_writes;
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
        persistence_writes = mutable.store.take_pending_writes();
    }

    state.submit_persistence_writes(persistence_writes);
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
        state
            .mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .completed_local_probes
            .clear();
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

    // Registry completion recording is the sole durable handoff. It runs
    // before this state becomes observable as completed and retains the exact
    // acquisition identity until the strand acknowledges successful handling.
    let _ = (reason, acquisition_id, ledger);
    true
}

fn publish_completed_ledger(
    hash: Uint256,
    acquisition_id: u64,
    completion_recorder: &AcquisitionCompletionRecorder,
    completed: &AtomicBool,
    completed_ledger: &Mutex<Option<Arc<Ledger>>>,
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
        reason,
        ledger,
    )
}

fn finalize_acquisition(state: &Arc<AcquisitionState>) {
    if state.is_done()
        || state
            .draining
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return;
    }
    let terminal = state
        .lock_mutable("begin terminal drain")
        .is_some_and(|mutable| mutable.inbound.is_complete() && !mutable.inbound.is_failed());
    if !terminal {
        state.draining.store(false, Ordering::Release);
        state.mark_failed();
        return;
    }
    // All accepted writes were enqueued as commands after their packet guard
    // released. Appending exactly one barrier makes publication wait for their
    // acknowledgements in FIFO order.
    state.request_durability_barrier();
}

/// Snapshot the completed ledger while the acquisition remains frozen, then
/// release actor ownership before any immutable-ledger setup can invoke its
/// node fetcher. Durable completion has already been acknowledged when this
/// helper is called.
fn snapshot_durable_completed_ledger(state: &AcquisitionState) -> Option<Ledger> {
    let mutable = state.lock_mutable("snapshot durable completed ledger")?;
    if mutable.inbound.is_failed() || !mutable.inbound.is_complete() {
        return None;
    }
    mutable.inbound.ledger().cloned()
}

fn finalize_durable_acquisition(state: &Arc<AcquisitionState>) {
    if state.is_done()
        || state
            .finalization_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return;
    }
    let Some(mut ledger) = snapshot_durable_completed_ledger(state) else {
        state.mark_failed();
        return;
    };

    // Final immutable-ledger setup is intentionally after the mutable snapshot
    // has been released. The completed acquisition supplies a cache-only
    // fetcher: source-reachable NodeStore I/O stays exclusively in the broker
    // and persistence workers, never in actor-owned finalization.
    if !ledger.is_immutable() {
        ledger.set_immutable(true);
    }
    ledger.set_full();
    let tree_cache = Arc::clone(&state.shared_tree_cache);
    ledger.set_node_fetcher(Arc::new(move |hash| tree_cache.fetch(hash.as_uint256())));

    let ledger = Arc::new(ledger);
    let ledger_seq = ledger.header().seq;
    let ledger_hash = *ledger.header().hash.as_uint256();
    let target_hash = *state.hash.as_uint256();
    let state_synching = ledger.state_map().is_synching();
    let tx_synching = ledger.tx_map().is_synching();
    if !publish_completed_ledger(
        *state.hash.as_uint256(),
        state.acquisition_id,
        &state.completion_recorder,
        &state.completed,
        &state.completed_ledger,
        state.reason,
        ledger,
    ) {
        return;
    }
    state
        .lifecycle
        .terminal_completed
        .fetch_add(1, Ordering::Relaxed);
    tracing::info!(
        target: "lcl_trace",
        event = "inbound_durable_complete",
        target_hash = %target_hash,
        ledger_hash = %ledger_hash,
        target_matches_header = target_hash == ledger_hash,
        ledger_seq,
        acquisition_id = state.acquisition_id,
        reason = ?state.reason,
        state_synching,
        tx_synching,
        "LCL trace: inbound acquisition completed durable tree finalization"
    );
    tracing::info!(
        target: "inbound_ledger",
        seq = ledger_seq,
        hash = %ledger_hash,
        acquisition_id = state.acquisition_id,
        reason = ?state.reason,
        "LEDGER ACQUIRED"
    );
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

#[cfg(test)]
mod actor_mailbox_tests {
    use super::*;

    fn packet_work(peer_id: u64, bytes: usize) -> PacketWork {
        PacketWork {
            peer_id,
            packet: InboundLedgerPacket::new(
                InboundLedgerDataType::StateNode,
                vec![ledger::InboundLedgerNodeData::new(
                    Some(vec![0; 33]),
                    vec![0; bytes],
                )],
            ),
            next_node: 0,
            bytes,
        }
    }

    #[test]
    fn retained_tree_read_batches_drain_without_another_branch_scan() {
        use basics::intrusive_pointer::make_shared_intrusive;
        use shamap::sync::{SHAMapType, SyncState, SyncTree};
        use shamap::tree_node::SHAMapTreeNode;

        struct NoResident;
        impl MissingNodeResidentLookup for NoResident {
            fn load_resident(
                &mut self,
                _hash: SHAMapHash,
                _ledger_seq: u32,
            ) -> Option<basics::intrusive_pointer::SharedIntrusive<SHAMapTreeNode>> {
                None
            }
        }

        // A two-level inner tree has far more distinct missing leaves than the
        // 16-read actor admission batch. One bounded advance retains later
        // batches internally after returning the first batch.
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        for parent_branch in 0..16 {
            let child = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
            for child_branch in 0..16 {
                let byte = (parent_branch * 16 + child_branch + 1) as u8;
                child.set_child_hash(
                    child_branch,
                    SHAMapHash::new(Uint256::from_array([byte; 32])),
                );
            }
            child.update_hash();
            root.set_child_hash(parent_branch, child.get_hash());
            root.canonicalize_child(parent_branch, child);
        }
        root.update_hash();
        let tree = SyncTree::from_root_with_type(
            root.clone(),
            SHAMapType::State,
            true,
            77,
            SyncState::Synching,
        );
        let mut first_child = || 0;
        let mut plan = TreePlan::new(
            TreePlanId::new(95),
            TreeKind::State,
            &tree,
            root.get_hash(),
            256,
            9,
            &mut first_child,
        );
        let mut resident = NoResident;
        let TreeAdvance::NeedsReads(initial_reads) = plan.advance(
            ACQ_TURN_MAX_BRANCH_STEPS,
            ACQ_TURN_MAX_NEW_READS,
            &mut resident,
            &mut first_child,
        ) else {
            panic!("expected the first bounded local-read batch");
        };
        assert_eq!(initial_reads.len(), ACQ_TURN_MAX_NEW_READS);
        let branch_steps_before = plan.branch_steps();
        let retained_reads = plan.take_read_admission_batch(ACQ_TURN_MAX_NEW_READS);
        assert_eq!(retained_reads.len(), ACQ_TURN_MAX_NEW_READS);
        assert_eq!(
            plan.branch_steps(),
            branch_steps_before,
            "extracting a retained batch must not run another TreePlan scan"
        );
    }

    #[test]
    fn deferred_ticket_with_runnable_frontier_parks_until_read_ready() {
        use super::super::read_broker::ReadBrokerConfig;
        use basics::intrusive_pointer::make_shared_intrusive;
        use shamap::sync::{SHAMapType, SyncState, SyncTree};
        use shamap::tree_node::SHAMapTreeNode;

        struct NoResident;
        impl MissingNodeResidentLookup for NoResident {
            fn load_resident(
                &mut self,
                _hash: SHAMapHash,
                _ledger_seq: u32,
            ) -> Option<basics::intrusive_pointer::SharedIntrusive<SHAMapTreeNode>> {
                None
            }
        }

        // A two-level, 256-leaf tree has more missing children than one
        // bounded read batch, leaving retained CPU frontier after the first
        // `NeedsReads` result.
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        for parent_branch in 0..16 {
            let child = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
            for child_branch in 0..16 {
                let byte = (parent_branch * 16 + child_branch + 1) as u8;
                child.set_child_hash(
                    child_branch,
                    SHAMapHash::new(Uint256::from_array([byte; 32])),
                );
            }
            child.update_hash();
            root.set_child_hash(parent_branch, child.get_hash());
            root.canonicalize_child(parent_branch, child);
        }
        root.update_hash();
        let tree = SyncTree::from_root_with_type(
            root.clone(),
            SHAMapType::State,
            true,
            77,
            SyncState::Synching,
        );
        let mut first_child = || 0;
        let mut plan = TreePlan::new(
            TreePlanId::new(96),
            TreeKind::State,
            &tree,
            root.get_hash(),
            256,
            9,
            &mut first_child,
        );
        let mut resident = NoResident;
        let TreeAdvance::NeedsReads(reads) = plan.advance(
            ACQ_TURN_MAX_BRANCH_STEPS,
            ACQ_TURN_MAX_NEW_READS,
            &mut resident,
            &mut first_child,
        ) else {
            panic!("expected bounded local-read needs");
        };
        assert_eq!(reads.len(), ACQ_TURN_MAX_NEW_READS);
        assert!(
            plan.has_runnable_frontier(),
            "remaining branches keep the TreePlan frontier runnable"
        );

        let broker = NodeReadBroker::new(ReadBrokerConfig {
            global_in_flight: 1,
            per_acquisition_in_flight: ACQ_TURN_MAX_NEW_READS,
            waiters_per_key: 4,
        })
        .expect("valid broker config");
        let delivered = Arc::new(Mutex::new(Vec::<ReadReady>::new()));
        let sink: ReadReadySink = {
            let delivered = Arc::clone(&delivered);
            Arc::new(move |ready| delivered.lock().expect("read events lock").push(ready))
        };
        let first = &reads[0];
        let second = &reads[1];
        assert!(matches!(
            broker.request(
                ReadKey::new(*first.hash().as_uint256(), first.ledger_seq(), 0),
                41,
                plan.id().get(),
                Arc::clone(&sink),
            ),
            ReadAdmission::Accepted(_)
        ));
        assert!(matches!(
            broker.request(
                ReadKey::new(*second.hash().as_uint256(), second.ledger_seq(), 0),
                41,
                plan.id().get(),
                sink,
            ),
            ReadAdmission::Deferred(_)
        ));
        assert!(
            delivered.lock().expect("read events lock").is_empty(),
            "a Deferred ticket has not delivered ReadReady"
        );
        assert_eq!(broker.take_ready_dispatches().len(), 1);
        assert!(
            !needs_reads_turn_can_requeue(&plan, true),
            "a Deferred ticket must park retained frontier until ReadReady"
        );
    }

    #[test]
    fn read_admission_backpressure_waits_instead_of_spinning_zero_branch_needs_reads() {
        use basics::intrusive_pointer::make_shared_intrusive;
        use shamap::sync::{SHAMapType, SyncState, SyncTree};
        use shamap::tree_node::SHAMapTreeNode;

        struct NoResident;
        impl MissingNodeResidentLookup for NoResident {
            fn load_resident(
                &mut self,
                _hash: SHAMapHash,
                _ledger_seq: u32,
            ) -> Option<basics::intrusive_pointer::SharedIntrusive<SHAMapTreeNode>> {
                None
            }
        }

        // One inner root produces exactly 16 local-read needs. Simulating
        // admission rejection reannounces those same needs. Its stack is then
        // exhausted, so a retry returns `NeedsReads` with zero branch work:
        // this matches the live high-rate telemetry shape.
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        for branch in 0..16 {
            root.set_child_hash(
                branch,
                SHAMapHash::new(Uint256::from_array([(branch + 1) as u8; 32])),
            );
        }
        root.update_hash();
        let tree = SyncTree::from_root_with_type(
            root.clone(),
            SHAMapType::State,
            true,
            77,
            SyncState::Synching,
        );
        let mut first_child = || 0;
        let mut plan = TreePlan::new(
            TreePlanId::new(94),
            TreeKind::State,
            &tree,
            root.get_hash(),
            16,
            9,
            &mut first_child,
        );
        let mut resident = NoResident;
        let TreeAdvance::NeedsReads(initial_reads) = plan.advance(
            ACQ_TURN_MAX_BRANCH_STEPS,
            ACQ_TURN_MAX_NEW_READS,
            &mut resident,
            &mut first_child,
        ) else {
            panic!("expected the initial 16 local-read needs");
        };
        // TreePlan has removed these needs from its unannounced set. A full
        // completion mailbox transfers them to the actor FIFO rather than
        // applying `Rejected`, which would recreate the zero-branch batch.
        let read_admission_backlog = VecDeque::from(initial_reads);
        assert_eq!(read_admission_backlog.len(), 16);
        assert!(
            !plan.has_runnable_frontier(),
            "broker-pending hashes alone cannot manufacture another scan"
        );

        let branch_steps_before = plan.branch_steps();
        assert!(matches!(
            plan.advance(
                ACQ_TURN_MAX_BRANCH_STEPS,
                ACQ_TURN_MAX_NEW_READS,
                &mut resident,
                &mut first_child,
            ),
            TreeAdvance::Ready
        ));
        assert_eq!(plan.branch_steps(), branch_steps_before);
        assert!(
            !needs_reads_turn_can_requeue(&plan, false),
            "the plan itself remains idle while its actor-owned FIFO waits"
        );

        let mut actor_plan = ActorTreePlan {
            plan,
            reason: InboundLedgerRequestTrigger::Blind,
            peer: None,
            tickets: BTreeMap::new(),
            read_admission_backlog,
            runnable: false,
            aggressive_by_hash: false,
        };
        actor_plan.retarget(InboundLedgerRequestTrigger::Timeout, None, false);
        assert!(
            actor_plan.runnable,
            "a timeout must retry a callback-less broker-rejected FIFO"
        );
        assert!(
            !actor_plan.aggressive_by_hash,
            "a blocked local-read FIFO must not append a peer retry"
        );
        let ActorTreePlan {
            mut plan,
            read_admission_backlog,
            ..
        } = actor_plan;
        let mut read_admission_backlog = read_admission_backlog;

        // One real completion releases a reservation. The actor takes exactly
        // one FIFO entry for admission; the remaining needs stay retained and
        // do not reappear as another TreePlan NeedsReads result.
        let admitted_after_completion = read_admission_backlog
            .pop_front()
            .expect("retained FIFO must supply the freed completion slot");
        assert_eq!(read_admission_backlog.len(), 15);
        assert!(matches!(
            plan.apply_read_result(
                plan.id(),
                admitted_after_completion.hash(),
                MissingNodeReadOutcome::Miss,
            ),
            MissingNodeReadApply::Applied {
                attached_edges: 0,
                missing_edges: 1,
            }
        ));
        assert!(matches!(
            plan.advance(
                ACQ_TURN_MAX_BRANCH_STEPS,
                ACQ_TURN_MAX_NEW_READS,
                &mut resident,
                &mut first_child,
            ),
            TreeAdvance::NeedsNetwork(_)
        ));
    }

    #[test]
    fn ready_without_branch_progress_is_not_requeued() {
        use basics::intrusive_pointer::make_shared_intrusive;
        use shamap::sync::{SHAMapType, SyncState, SyncTree};
        use shamap::tree_node::SHAMapTreeNode;

        struct NoResident;
        impl MissingNodeResidentLookup for NoResident {
            fn load_resident(
                &mut self,
                _hash: SHAMapHash,
                _ledger_seq: u32,
            ) -> Option<basics::intrusive_pointer::SharedIntrusive<SHAMapTreeNode>> {
                None
            }
        }

        // The stack is intentionally retained and CPU-runnable, but a zero
        // branch budget produces `Ready` before the continuation can select a
        // branch. This is the actor-side shape that previously self-requeued
        // solely from `has_runnable_frontier()`.
        let child = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        child.set_child_hash(0, SHAMapHash::new(Uint256::from_array([0x51; 32])));
        child.update_hash();
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(0, child.get_hash());
        root.canonicalize_child(0, child);
        root.update_hash();
        let tree = SyncTree::from_root_with_type(
            root.clone(),
            SHAMapType::State,
            true,
            77,
            SyncState::Synching,
        );
        let mut first_child = || 0;
        let mut plan = TreePlan::new(
            TreePlanId::new(93),
            TreeKind::State,
            &tree,
            root.get_hash(),
            16,
            9,
            &mut first_child,
        );
        let mut resident = NoResident;
        let branch_steps_before = plan.branch_steps();

        assert!(matches!(
            plan.advance(0, ACQ_TURN_MAX_NEW_READS, &mut resident, &mut first_child),
            TreeAdvance::Ready
        ));
        assert!(
            plan.has_runnable_frontier(),
            "the retained stack still has CPU work"
        );
        assert_eq!(plan.branch_steps(), branch_steps_before);
        assert!(
            !ready_turn_can_requeue(&plan, branch_steps_before),
            "a no-progress Ready turn must leave the mailbox idle"
        );
    }

    #[test]
    fn verified_peer_node_resumes_deferred_parent_and_queues_a_bounded_turn() {
        use basics::intrusive_pointer::make_shared_intrusive;
        use shamap::sync::{SHAMapType, SyncState, SyncTree};
        use shamap::tree_node::SHAMapTreeNode;

        struct NoResident;
        impl MissingNodeResidentLookup for NoResident {
            fn load_resident(
                &mut self,
                _hash: SHAMapHash,
                _ledger_seq: u32,
            ) -> Option<basics::intrusive_pointer::SharedIntrusive<SHAMapTreeNode>> {
                None
            }
        }

        let child = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        child.update_hash();
        let missing = child.get_hash();
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(7, missing);
        root.update_hash();
        let tree = SyncTree::from_root_with_type(
            root.clone(),
            SHAMapType::State,
            true,
            77,
            SyncState::Synching,
        );
        let mut first_child = || 0;
        let mut plan = TreePlan::new(
            TreePlanId::new(92),
            TreeKind::State,
            &tree,
            root.get_hash(),
            16,
            0,
            &mut first_child,
        );
        let mut resident = NoResident;
        let TreeAdvance::NeedsReads(reads) = plan.advance(256, 16, &mut resident, &mut first_child)
        else {
            panic!("expected one brokered read");
        };
        assert_eq!(reads.len(), 1);
        assert!(matches!(
            plan.apply_read_result(plan.id(), missing, MissingNodeReadOutcome::Miss),
            MissingNodeReadApply::Applied {
                missing_edges: 1,
                ..
            }
        ));
        assert!(matches!(
            plan.advance(256, 16, &mut resident, &mut first_child),
            TreeAdvance::NeedsNetwork(_)
        ));
        assert!(
            !plan.has_runnable_frontier(),
            "the retained plan is waiting on its peer edge"
        );

        let mut actor_plan = ActorTreePlan {
            plan,
            reason: InboundLedgerRequestTrigger::Blind,
            peer: None,
            tickets: BTreeMap::new(),
            read_admission_backlog: VecDeque::new(),
            runnable: false,
            aggressive_by_hash: false,
        };
        assert!(
            apply_verified_peer_nodes_to_plan(&mut actor_plan, [child]),
            "a verified peer node must wake its deferred parent"
        );
        assert!(actor_plan.runnable);
        assert!(actor_plan.plan.has_runnable_frontier());

        let mut mailbox = AcquisitionMailbox::default();
        mailbox.plan = Some(actor_plan);
        assert!(
            mailbox.wake_tree_plan(),
            "an idle mailbox must claim one bounded turn"
        );
        assert_eq!(mailbox.token, AcquisitionWorkToken::Queued);
        assert!(mailbox.has_work(false));
    }

    #[test]
    fn terminal_local_probe_cancellation_is_exact_once_and_accounts_late_events() {
        use super::super::read_broker::ReadBrokerConfig;

        let broker = NodeReadBroker::new(ReadBrokerConfig::default()).expect("valid broker config");
        let delivered = Arc::new(Mutex::new(Vec::<ReadReady>::new()));
        let sink: ReadReadySink = {
            let delivered = Arc::clone(&delivered);
            Arc::new(move |ready| {
                delivered
                    .lock()
                    .expect("local probe events lock")
                    .push(ready)
            })
        };
        let key = ReadKey::new(Uint256::from_array([15; 32]), 77, 0);
        let ticket = match broker.request(key, 41, LOCAL_PROBE_PLAN_ID, sink) {
            ReadAdmission::Accepted(ticket) => ticket,
            other => panic!("expected accepted local probe ticket, got {other:?}"),
        };

        let mut mailbox = AcquisitionMailbox::default();
        mailbox.local_probes.insert(
            ticket.id().get(),
            LocalProbe {
                ticket,
                kind: LocalProbeKind::Header,
                reason: InboundLedgerRequestTrigger::Blind,
                peer: None,
            },
        );
        mailbox.read_event_reservations = 1;

        let terminal_tickets = mailbox.clear_terminal_work();
        assert_eq!(terminal_tickets, vec![ticket]);
        assert!(mailbox.local_probes.is_empty());
        assert_eq!(mailbox.read_event_reservations, 0);
        assert!(
            mailbox.clear_terminal_work().is_empty(),
            "terminal cleanup cannot return a ticket twice"
        );

        assert!(broker.cancel(ticket));
        assert!(
            !broker.cancel(ticket),
            "the broker must not settle an already-cancelled ticket twice"
        );
        let delivered = delivered.lock().expect("local probe events lock");
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].ticket, ticket);
        assert_eq!(delivered[0].outcome, ReadOutcome::Cancelled);
        drop(delivered);

        // `enqueue_read_ready` follows this terminal path for the cancelled
        // callback: no work is revived, and diagnostics retain the late event.
        mailbox.record_late_read_event();
        assert_eq!(mailbox.stale_events, 1);
        assert_eq!(broker.snapshot().metrics.cancelled, 1);
    }

    #[test]
    fn packet_continuation_is_restored_to_the_fifo_front_after_a_128_node_step() {
        let mut mailbox = AcquisitionMailbox::default();
        mailbox.packets.push_back(packet_work(11, 128));
        mailbox.packets.push_back(packet_work(22, 1));
        mailbox.packet_bytes = 129;

        let mut active = mailbox.packets.pop_front().expect("first FIFO packet");
        mailbox.packet_bytes = mailbox.packet_bytes.saturating_sub(active.bytes);
        active.next_node = ledger::INBOUND_LEDGER_MAX_PACKET_NODES_PER_STEP;
        mailbox.packet_bytes += active.bytes;
        mailbox.packets.push_front(active);

        assert_eq!(
            mailbox
                .packets
                .front()
                .expect("active continuation")
                .peer_id,
            11
        );
        assert_eq!(
            mailbox
                .packets
                .front()
                .expect("active continuation")
                .next_node,
            128
        );
        assert_eq!(mailbox.packets.get(1).expect("later packet").peer_id, 22);
        assert_eq!(mailbox.packet_bytes, 129);
    }

    #[test]
    fn reserved_read_completion_capacity_never_uses_packet_capacity_or_drops_work() {
        let mut mailbox = AcquisitionMailbox::default();
        mailbox.read_event_reservations = super::super::read_broker::ACQ_READS_PER_ACQUISITION;
        assert_eq!(mailbox.events.len(), 0);
        assert_eq!(
            mailbox.read_event_reservations,
            super::super::read_broker::ACQ_READS_PER_ACQUISITION,
            "each accepted broker ticket owns one reserved completion slot"
        );
        assert!(
            !mailbox.has_work(false),
            "reservations alone are waiting work, not a spin token"
        );
    }

    #[test]
    fn ready_read_events_drain_in_bounded_turns() {
        use super::super::read_broker::ReadBrokerConfig;

        let broker = NodeReadBroker::new(ReadBrokerConfig::default()).expect("valid broker config");
        let delivered = Arc::new(Mutex::new(Vec::<ReadReady>::new()));
        let sink: ReadReadySink = {
            let delivered = Arc::clone(&delivered);
            Arc::new(move |ready| {
                delivered
                    .lock()
                    .expect("read event fixture lock")
                    .push(ready);
            })
        };
        let event_count = ACQ_TURN_MAX_READ_EVENTS * 2;
        for index in 0..event_count {
            let key = ReadKey::new(Uint256::from_array([index as u8; 32]), 77, 0);
            assert!(matches!(
                broker.request(key, 41, 7, Arc::clone(&sink)),
                ReadAdmission::Accepted(_)
            ));
            assert!(broker.complete(key, ReadOutcome::Miss));
        }
        let delivered = std::mem::take(&mut *delivered.lock().expect("read event fixture lock"));
        assert_eq!(delivered.len(), event_count);
        let retained_ticket = delivered[ACQ_TURN_MAX_READ_EVENTS].ticket;

        let mut mailbox = AcquisitionMailbox {
            events: VecDeque::from(delivered),
            read_event_reservations: event_count,
            token: AcquisitionWorkToken::Running,
            ..AcquisitionMailbox::default()
        };
        let first_turn = drain_bounded_read_events(|| mailbox.take_read_event().is_some());
        assert_eq!(first_turn, ACQ_TURN_MAX_READ_EVENTS);
        assert_eq!(mailbox.events.len(), ACQ_TURN_MAX_READ_EVENTS);
        assert_eq!(mailbox.read_event_reservations, ACQ_TURN_MAX_READ_EVENTS);
        assert_eq!(
            mailbox.events.front().expect("retained event").ticket,
            retained_ticket,
            "the next settled completion remains FIFO-owned for the successor"
        );
        assert!(
            mailbox.finish_turn(false),
            "remaining settled completions schedule one successor turn"
        );
        assert_eq!(mailbox.token, AcquisitionWorkToken::Queued);

        mailbox.token = AcquisitionWorkToken::Running;
        let second_turn = drain_bounded_read_events(|| mailbox.take_read_event().is_some());
        assert_eq!(second_turn, ACQ_TURN_MAX_READ_EVENTS);
        assert!(mailbox.events.is_empty());
        assert_eq!(mailbox.read_event_reservations, 0);
        assert!(
            !mailbox.finish_turn(false),
            "an empty mailbox must not submit an extra successor"
        );
        assert_eq!(mailbox.token, AcquisitionWorkToken::Idle);
    }

    fn network_waiting_actor_plan(
        plan_id: u64,
        reason: InboundLedgerRequestTrigger,
        aggressive_by_hash: bool,
    ) -> ActorTreePlan {
        use basics::intrusive_pointer::make_shared_intrusive;
        use shamap::sync::{SHAMapType, SyncState, SyncTree};
        use shamap::tree_node::SHAMapTreeNode;

        struct NoResident;
        impl MissingNodeResidentLookup for NoResident {
            fn load_resident(
                &mut self,
                _hash: SHAMapHash,
                _ledger_seq: u32,
            ) -> Option<basics::intrusive_pointer::SharedIntrusive<SHAMapTreeNode>> {
                None
            }
        }

        let missing = SHAMapHash::new(Uint256::from_array([plan_id as u8; 32]));
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(7, missing);
        root.update_hash();
        let tree = SyncTree::from_root_with_type(
            root.clone(),
            SHAMapType::State,
            true,
            77,
            SyncState::Synching,
        );
        let mut first_child = || 0;
        let mut plan = TreePlan::new(
            TreePlanId::new(plan_id),
            TreeKind::State,
            &tree,
            root.get_hash(),
            16,
            0,
            &mut first_child,
        );
        let mut resident = NoResident;
        let TreeAdvance::NeedsReads(reads) = plan.advance(256, 16, &mut resident, &mut first_child)
        else {
            panic!("expected one brokered read");
        };
        assert_eq!(reads.len(), 1);
        assert!(matches!(
            plan.apply_read_result(plan.id(), missing, MissingNodeReadOutcome::Miss),
            MissingNodeReadApply::Applied {
                missing_edges: 1,
                ..
            }
        ));
        assert!(matches!(
            plan.advance(256, 16, &mut resident, &mut first_child),
            TreeAdvance::NeedsNetwork(_)
        ));
        ActorTreePlan {
            plan,
            reason,
            peer: None,
            tickets: BTreeMap::new(),
            read_admission_backlog: VecDeque::new(),
            runnable: false,
            aggressive_by_hash,
        }
    }

    #[test]
    fn needs_network_send_hook_observes_restored_plan_for_ordinary_and_aggressive_requests() {
        for (plan_id, reason, initial_aggressive_by_hash, restored_aggressive_by_hash) in [
            (93, InboundLedgerRequestTrigger::Reply, false, false),
            (94, InboundLedgerRequestTrigger::Timeout, true, false),
        ] {
            let mut actor_plan =
                network_waiting_actor_plan(plan_id, reason, initial_aggressive_by_hash);
            // Mirror the aggressive branch's pre-restoration commit: the
            // synchronous send hook must see the consumed state, not the
            // detached plan's earlier by-hash value.
            actor_plan.aggressive_by_hash = restored_aggressive_by_hash;
            let mailbox = Arc::new(Mutex::new(AcquisitionMailbox::default()));
            let restored_mailbox = Arc::clone(&mailbox);
            let send_hook_mailbox = Arc::clone(&mailbox);
            let send_hook_observed_restored_plan = Arc::new(AtomicBool::new(false));
            let send_hook_observation = Arc::clone(&send_hook_observed_restored_plan);

            restore_tree_plan_before_peer_send(
                actor_plan,
                move |actor_plan| {
                    restored_mailbox.lock().expect("restore mailbox lock").plan = Some(actor_plan);
                },
                move || {
                    // This deterministic send hook models a synchronous peer
                    // callback and must observe the committed retained plan.
                    let restored = send_hook_mailbox.lock().expect("send-hook mailbox lock");
                    let plan = restored
                        .plan
                        .as_ref()
                        .expect("plan restored before send hook");
                    assert_eq!(plan.plan.id(), TreePlanId::new(plan_id));
                    assert_eq!(plan.reason, reason);
                    assert_eq!(plan.aggressive_by_hash, restored_aggressive_by_hash);
                    assert!(!plan.runnable, "NeedsNetwork is waiting after emission");
                    send_hook_observation.store(true, Ordering::Release);
                },
            );
            assert!(
                send_hook_observed_restored_plan.load(Ordering::Acquire),
                "{reason:?} send hook must run after plan restoration"
            );
        }
    }

    #[test]
    fn timeout_retargets_the_retained_network_frontier_without_rebuilding() {
        use basics::intrusive_pointer::make_shared_intrusive;
        use shamap::sync::{SHAMapType, SyncState, SyncTree};
        use shamap::tree_node::SHAMapTreeNode;

        struct NoResident;
        impl MissingNodeResidentLookup for NoResident {
            fn load_resident(
                &mut self,
                _hash: SHAMapHash,
                _ledger_seq: u32,
            ) -> Option<basics::intrusive_pointer::SharedIntrusive<SHAMapTreeNode>> {
                None
            }
        }

        let missing = SHAMapHash::new(Uint256::from_array([42; 32]));
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(7, missing);
        root.update_hash();
        let tree = SyncTree::from_root_with_type(
            root.clone(),
            SHAMapType::State,
            true,
            77,
            SyncState::Synching,
        );
        let mut first_child = || 0;
        let mut plan = TreePlan::new(
            TreePlanId::new(91),
            TreeKind::State,
            &tree,
            root.get_hash(),
            16,
            0,
            &mut first_child,
        );
        let mut resident = NoResident;
        let TreeAdvance::NeedsReads(reads) = plan.advance(256, 16, &mut resident, &mut first_child)
        else {
            panic!("expected one brokered read");
        };
        assert_eq!(reads.len(), 1);
        assert!(matches!(
            plan.apply_read_result(plan.id(), missing, MissingNodeReadOutcome::Miss),
            MissingNodeReadApply::Applied {
                missing_edges: 1,
                ..
            }
        ));
        assert!(matches!(
            plan.advance(256, 16, &mut resident, &mut first_child),
            TreeAdvance::NeedsNetwork(_)
        ));

        let mut retained = ActorTreePlan {
            plan,
            reason: InboundLedgerRequestTrigger::Blind,
            peer: None,
            tickets: BTreeMap::new(),
            read_admission_backlog: VecDeque::new(),
            runnable: false,
            aggressive_by_hash: false,
        };
        assert!(!uses_aggressive_by_hash_timeout(4));
        assert!(uses_aggressive_by_hash_timeout(5));
        retained.retarget(InboundLedgerRequestTrigger::Timeout, None, true);
        assert!(
            retained.runnable,
            "a lost peer reply wakes the same retained plan"
        );
        assert!(
            retained.aggressive_by_hash,
            "the existing >4 threshold selects by-hash"
        );
        assert_eq!(
            retained.plan.id(),
            TreePlanId::new(91),
            "retarget never rebuilds a plan"
        );
        assert!(matches!(
            retained
                .plan
                .advance(256, 16, &mut resident, &mut first_child),
            TreeAdvance::NeedsNetwork(_)
        ));
    }

    #[test]
    fn persistence_queue_dispatches_once_and_settles_duplicates() {
        let mut queue = PersistenceQueue::default();
        let write = PersistenceWrite {
            key: PersistenceKey {
                hash: Uint256::from_array([8; 32]),
                ledger_seq: 12,
                object_type: 3,
            },
            object_type: nodestore::NodeObjectType::Ledger,
            data: vec![7],
        };
        queue.enqueue_writes(vec![write.clone()]);
        let first = queue.take_next().expect("first write dispatch");
        queue.enqueue_writes(vec![write]);
        queue.enqueue_barrier();
        assert!(
            queue.take_next().is_none(),
            "an in-flight command cannot dispatch twice"
        );
        assert_eq!(
            queue.in_flight.as_ref().map(|command| command.id),
            Some(first.id())
        );
        assert_eq!(queue.settlement(2), Some(&PersistenceSettlement::Duplicate));
        assert!(queue.acknowledge(&PersistenceReady {
            id: first.id(),
            result: Ok(()),
            durability_barrier: false,
        }));
        assert_eq!(
            queue.settlement(first.id()),
            Some(&PersistenceSettlement::Written)
        );
        assert!(matches!(
            queue.take_next(),
            Some(PersistenceCommand::DurabilityBarrier { .. })
        ));
    }

    #[test]
    fn persistence_queue_batches_writes_before_the_durability_barrier() {
        let mut queue = PersistenceQueue::default();
        let writes = (0..=PERSISTENCE_WRITE_BATCH_SIZE)
            .map(|index| {
                let mut hash = [0; 32];
                hash[..8].copy_from_slice(&(index as u64).to_be_bytes());
                PersistenceWrite {
                    key: PersistenceKey {
                        hash: Uint256::from_array(hash),
                        ledger_seq: 14,
                        object_type: 3,
                    },
                    object_type: nodestore::NodeObjectType::Ledger,
                    data: vec![index as u8],
                }
            })
            .collect();
        queue.enqueue_writes(writes);
        queue.enqueue_barrier();

        let first = queue.take_next().expect("first write batch");
        let (first_id, first_count) = match first {
            PersistenceCommand::Writes { id, writes } => (id, writes.len()),
            PersistenceCommand::DurabilityBarrier { .. } => panic!("expected write batch"),
        };
        assert_eq!(first_count, PERSISTENCE_WRITE_BATCH_SIZE);
        assert!(queue.acknowledge(&PersistenceReady {
            id: first_id,
            result: Ok(()),
            durability_barrier: false,
        }));

        let second = queue.take_next().expect("tail write batch");
        let (second_id, second_count) = match second {
            PersistenceCommand::Writes { id, writes } => (id, writes.len()),
            PersistenceCommand::DurabilityBarrier { .. } => panic!("expected tail write batch"),
        };
        assert_eq!(second_count, 1);
        assert!(queue.acknowledge(&PersistenceReady {
            id: second_id,
            result: Ok(()),
            durability_barrier: false,
        }));
        assert!(matches!(
            queue.take_next(),
            Some(PersistenceCommand::DurabilityBarrier { .. })
        ));
    }

    #[test]
    fn terminal_cancellation_visibly_settles_in_flight_and_queued_persistence() {
        let mut queue = PersistenceQueue::default();
        queue.enqueue_writes(vec![PersistenceWrite {
            key: PersistenceKey {
                hash: Uint256::from_array([6; 32]),
                ledger_seq: 13,
                object_type: 3,
            },
            object_type: nodestore::NodeObjectType::Ledger,
            data: vec![9],
        }]);
        queue.enqueue_barrier();
        let in_flight = queue.take_next().expect("write dispatch");
        let queued = queue.queued.front().expect("queued barrier").id();
        queue.cancel();
        assert!(queue.in_flight.is_none());
        assert!(queue.queued.is_empty());
        assert_eq!(
            queue.settlement(in_flight.id()),
            Some(&PersistenceSettlement::Cancelled)
        );
        assert_eq!(
            queue.settlement(queued),
            Some(&PersistenceSettlement::Cancelled)
        );
        assert!(
            !queue.acknowledge(&PersistenceReady {
                id: in_flight.id(),
                result: Ok(()),
                durability_barrier: false,
            }),
            "late worker completion cannot overwrite terminal cancellation"
        );
    }

    #[test]
    fn completed_ledger_fetcher_is_cache_only() {
        let source = include_str!("acquisition.rs");
        let start = source
            .find("fn finalize_durable_acquisition")
            .expect("durable finalizer source");
        let finalizer = &source[start
            ..source[start..]
                .find("\n/// Stash state nodes")
                .map(|offset| start + offset)
                .expect("durable finalizer boundary")];
        assert!(finalizer.contains("snapshot_durable_completed_ledger(state)"));
        assert!(finalizer.contains("tree_cache.fetch(hash.as_uint256())"));
        assert!(!finalizer.contains("fetch_node_object("));
        assert!(!finalizer.contains("FetchType::Synchronous"));
    }

    #[test]
    fn persistence_queue_keeps_write_then_barrier_in_fifo_ack_order() {
        let mut queue = PersistenceQueue::default();
        let write = PersistenceWrite {
            key: PersistenceKey {
                hash: Uint256::from_array([9; 32]),
                ledger_seq: 10,
                object_type: 3,
            },
            object_type: nodestore::NodeObjectType::Ledger,
            data: vec![1, 2, 3],
        };
        queue.enqueue_writes(vec![write]);
        queue.enqueue_barrier();
        let first = queue.take_next().expect("write command");
        assert!(matches!(first, PersistenceCommand::Writes { .. }));
        assert!(queue.acknowledge(&PersistenceReady {
            id: first.id(),
            result: Ok(()),
            durability_barrier: false,
        }));
        let second = queue.take_next().expect("barrier command");
        assert!(matches!(
            second,
            PersistenceCommand::DurabilityBarrier { .. }
        ));
        assert!(queue.acknowledge(&PersistenceReady {
            id: second.id(),
            result: Ok(()),
            durability_barrier: true,
        }));
        assert!(queue.barrier_acknowledged);
    }

    #[test]
    fn persistence_fault_cancels_queued_barrier_before_publication_can_proceed() {
        let mut queue = PersistenceQueue::default();
        queue.enqueue_writes(vec![PersistenceWrite {
            key: PersistenceKey {
                hash: Uint256::from_array([7; 32]),
                ledger_seq: 11,
                object_type: 3,
            },
            object_type: nodestore::NodeObjectType::Ledger,
            data: vec![4, 5, 6],
        }]);
        queue.enqueue_barrier();
        let write = queue.take_next().expect("write command");
        assert!(matches!(write, PersistenceCommand::Writes { .. }));
        assert!(queue.acknowledge(&PersistenceReady {
            id: write.id(),
            result: Err(Arc::from("store fault")),
            durability_barrier: false,
        }));
        assert!(
            queue.failed.is_some(),
            "a write fault is terminal before publication"
        );
        queue.cancel();
        assert!(
            queue.queued.is_empty(),
            "failure cancels the unacknowledged barrier"
        );
        assert!(
            queue.take_next().is_none(),
            "no barrier can acknowledge after a write fault"
        );
    }

    #[test]
    fn terminal_clear_releases_mailbox_packet_and_event_ownership() {
        let mut mailbox = AcquisitionMailbox::default();
        mailbox.packets.push_back(packet_work(7, 64));
        mailbox.packet_bytes = 64;
        mailbox.pending_timeouts = 1;
        mailbox.token = AcquisitionWorkToken::Running;

        let tickets = mailbox.clear_terminal_work();

        assert!(tickets.is_empty());
        assert!(mailbox.packets.is_empty());
        assert!(mailbox.events.is_empty());
        assert_eq!(mailbox.packet_bytes, 0);
        assert_eq!(mailbox.pending_timeouts, 0);
        assert_eq!(mailbox.token, AcquisitionWorkToken::Idle);
    }
}
