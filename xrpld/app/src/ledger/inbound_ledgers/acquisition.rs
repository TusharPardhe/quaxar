//! Per-hash inbound-ledger lifecycle.
//!
//! The structure follows rippled's `InboundLedger` and `TimeoutCounter`:
//! `init` checks local storage, adds peers, queues an immediate timeout job,
//! and every timeout job re-arms only its own three-second timer.

use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::MonotonicClock;
use ledger::ledger_fetcher::{
    INBOUND_LEDGER_NORMAL_NODE_REQUEST_CAP, INBOUND_LEDGER_REPLY_NODE_REQUEST_CAP,
    InboundLedgerSyncStore,
};
use ledger::{
    FetchPackCache, FetchPackContainer, FetchPackStore, InboundLedgerDataType,
    InboundLedgerJournal, InboundLedgerLocal, InboundLedgerPacket, InboundLedgerPacketError,
    InboundLedgerPeerScore, InboundLedgerReason, InboundLedgerRequestTrigger, InboundLedgerStore,
    InboundLedgerTimerResult, Ledger, TreeKind, make_get_ledger_with_node_ids,
    select_inbound_ledger_reply_peers,
};
use overlay::{Peer, PeerSet as _};
use shamap::family::{FullBelowCacheImpl, NullMissingNodeReporter, SHAMapFamily};
use shamap::sync::{
    DEFAULT_MAX_DEFERRED_MISSING_NODE_READS, DeferredFetchRequestInfo, SHAMapAddNode,
};
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Condvar;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

use super::registry::{AcquireReason, AcquisitionLifecycleCounters};
use super::worker_pool::WorkerPool;

const PEER_COUNT_START: usize = 5;
const PEER_COUNT_ADD: usize = 3;
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);
/// A persistence command is one bounded NodeStore batch.
const PERSISTENCE_WRITE_BATCH_SIZE: usize = 256;
pub const ACQ_MAILBOX_PACKET_CAPACITY: usize = 128;
pub const ACQ_MAILBOX_BYTE_CAPACITY: usize = 4 * 1024 * 1024;

/// Callback collector for one SHAMap-owned deferred NodeStore pass.
///
/// `SyncTree::get_missing_nodes_deferred_with_family` owns traversal, callback
/// collection boundaries, canonical parent resumes, and the strict 512-read
/// pass limit. This object only bridges the NodeStore callback API to that
/// SHAMap call. It deliberately has no acquisition IDs, actor tickets,
/// admission backlog, or coalescing policy.
#[derive(Default)]
struct DirectNodeStoreReadState {
    cancelled: bool,
    completed: BTreeMap<SHAMapHash, Option<Arc<nodestore::NodeObject>>>,
}

struct DirectNodeStoreReads {
    state: Mutex<DirectNodeStoreReadState>,
    wake: Condvar,
}

impl DirectNodeStoreReads {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(DirectNodeStoreReadState::default()),
            wake: Condvar::new(),
        })
    }

    fn request(self: &Arc<Self>, store: &SHAMapStoreNodeStore, hash: SHAMapHash, ledger_seq: u32) {
        let collector = Arc::clone(self);
        let callback = Box::new(move |object| {
            let mut state = collector.state.lock().expect("direct NodeStore read lock");
            if !state.cancelled {
                state.completed.insert(hash, object);
            }
            collector.wake.notify_all();
        });
        match store {
            SHAMapStoreNodeStore::Single(database) => {
                database.async_fetch(*hash.as_uint256(), ledger_seq, callback)
            }
            SHAMapStoreNodeStore::Rotating(database) => {
                database.async_fetch(*hash.as_uint256(), ledger_seq, callback)
            }
        }
    }

    fn wait_for(
        &self,
        requests: &[DeferredFetchRequestInfo],
    ) -> Option<Vec<Option<Arc<nodestore::NodeObject>>>> {
        let mut state = self.state.lock().expect("direct NodeStore read lock");
        while !state.cancelled
            && requests
                .iter()
                .any(|request| !state.completed.contains_key(&request.hash()))
        {
            state = self.wake.wait(state).expect("direct NodeStore read wait");
        }
        (!state.cancelled).then(|| {
            requests
                .iter()
                .map(|request| {
                    state
                        .completed
                        .remove(&request.hash())
                        .expect("every direct NodeStore callback must settle its SHAMap request")
                })
                .collect()
        })
    }

    #[cfg(test)]
    fn wait_until_cancelled(&self) {
        let mut state = self.state.lock().expect("direct NodeStore read lock");
        while !state.cancelled {
            state = self.wake.wait(state).expect("direct NodeStore read wait");
        }
    }

    fn cancel(&self) {
        let mut state = self.state.lock().expect("direct NodeStore read lock");
        state.cancelled = true;
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
    data_drain_runs: AtomicU64,
    data_drain_us: AtomicU64,
    data_drain_max_us: AtomicU64,
    data_drain_max_packets: AtomicU64,
    tx_scan_us: AtomicU64,
    worker_jobs: AtomicU64,
    worker_queue_wait_us: AtomicU64,
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
            data_drain_runs: AtomicU64::new(0),
            data_drain_us: AtomicU64::new(0),
            data_drain_max_us: AtomicU64::new(0),
            data_drain_max_packets: AtomicU64::new(0),
            tx_scan_us: AtomicU64::new(0),
            worker_jobs: AtomicU64::new(0),
            worker_queue_wait_us: AtomicU64::new(0),
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
    pub data_drain_runs: u64,
    pub data_drain_us: u64,
    pub data_drain_max_us: u64,
    pub data_drain_max_packets: u64,
    pub tx_scan_us: u64,
    pub worker_jobs: u64,
    pub worker_queue_wait_us: u64,
    pub tracked_peers: usize,
    pub buffered_packets: usize,
    pub buffered_packets_high_water: usize,
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
/// `SyncTree` callback completions and are reduced by the same SHAMap validators.
///
/// It deliberately does not own asynchronous descendant NodeStore I/O; the
/// direct SHAMap collector is the sole callback path.
pub struct LocalHydratorStore {
    pending_writes: Vec<PersistenceWrite>,
    pending_keys: BTreeSet<PersistenceKey>,
    cache: Arc<FetchPackCache>,
    node_store: SHAMapStoreNodeStore,
    ledger_seq: u32,
}

impl LocalHydratorStore {
    /// Build a read-only copy for the synchronous tryDB SHAMap fetcher.
    /// Pending persistence ownership stays with the acquisition's mutable store.
    fn local_read_clone(&self) -> Self {
        Self {
            pending_writes: Vec::new(),
            pending_keys: BTreeSet::new(),
            cache: Arc::clone(&self.cache),
            node_store: self.node_store.clone(),
            ledger_seq: self.ledger_seq,
        }
    }

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
    fn fetch_ledger_header(&mut self, hash: SHAMapHash, seq: u32) -> Option<Vec<u8>> {
        // Match rippled's synchronous tryDB probe for the ledger header before
        // network traffic. Descendant acquisition remains in the direct
        // callback collector below.
        match &self.node_store {
            SHAMapStoreNodeStore::Single(database) => database.fetch_node_object(
                hash.as_uint256(),
                seq,
                nodestore::FetchType::Synchronous,
                false,
            ),
            SHAMapStoreNodeStore::Rotating(database) => database.fetch_node_object(
                hash.as_uint256(),
                seq,
                nodestore::FetchType::Synchronous,
                false,
            ),
        }
        .map(|object| object.data().to_vec())
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
        // The direct descent checks cache first. Its sync filter then probes
        // the local root before missing descendants enter the bounded direct
        // asynchronous NodeStore callback pass.
        self.cache.get_fetch_pack(hash).or_else(|| {
            let object = match &self.node_store {
                SHAMapStoreNodeStore::Single(database) => database.fetch_node_object(
                    &hash,
                    self.ledger_seq,
                    nodestore::FetchType::Synchronous,
                    false,
                ),
                SHAMapStoreNodeStore::Rotating(database) => database.fetch_node_object(
                    &hash,
                    self.ledger_seq,
                    nodestore::FetchType::Synchronous,
                    false,
                ),
            }?;
            Some(object.data().to_vec())
        })
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum AcquisitionWorkClass {
    LedgerData,
    Persistence,
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

/// The acquisition mailbox owns only inbound packets, persistence acknowledgements,
/// and timeout coalescing. Direct SHAMap traversal owns its callback collection
/// synchronously and has no retained broker actor state.
struct AcquisitionMailbox {
    packets: VecDeque<PacketWork>,
    packet_bytes: usize,
    persistence_events: VecDeque<PersistenceReady>,
    token: AcquisitionWorkToken,
    pending_timeouts: u32,
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
            persistence_events: VecDeque::new(),
            token: AcquisitionWorkToken::Idle,
            pending_timeouts: 0,
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

    fn has_active_packet_batch(&self) -> bool {
        self.batch_packets_remaining != 0 || !self.packets.is_empty()
    }

    fn has_remaining_packet_batch(&self) -> bool {
        self.batch_packets_remaining != 0
    }

    fn next_work_class(&self, fetch_pack_ready: bool) -> Option<AcquisitionWorkClass> {
        if !self.packets.is_empty() || self.pending_timeouts != 0 || fetch_pack_ready {
            Some(AcquisitionWorkClass::LedgerData)
        } else if !self.persistence_events.is_empty() {
            Some(AcquisitionWorkClass::Persistence)
        } else {
            None
        }
    }

    fn record_late_event(&mut self) {
        self.stale_events += 1;
    }

    fn finish_turn(&mut self, fetch_pack_ready: bool) -> Option<AcquisitionWorkClass> {
        let next = self.next_work_class(fetch_pack_ready);
        self.token = if next.is_some() {
            AcquisitionWorkToken::Queued
        } else {
            AcquisitionWorkToken::Idle
        };
        next
    }

    fn clear_terminal_work(&mut self) {
        self.packets.clear();
        self.packet_bytes = 0;
        self.persistence_events.clear();
        self.pending_timeouts = 0;
        self.batch_packets_remaining = 0;
        self.batch_useful_peer_counts.clear();
        self.token = AcquisitionWorkToken::Idle;
    }
}

/// Per-ledger state owned by the registry.
pub struct AcquisitionState {
    mailbox: Mutex<AcquisitionMailbox>,
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
    /// Direct peer selection preserves rippled's per-acquisition recent-node
    /// filter across scans and clears it on timeout recovery.
    recent_direct_nodes: Mutex<BTreeSet<Uint256>>,
    /// The direct SHAMap pass is explicitly woken on teardown. Late NodeStore
    /// callbacks are ignored and cannot revive a cancelled acquisition.
    direct_reads: Mutex<Option<Arc<DirectNodeStoreReads>>>,
    persistence: Mutex<PersistenceQueue>,
    /// Terminal traversal freezes ingress/request generation while ordered
    /// persistence and its one durability barrier drain.
    draining: AtomicBool,
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
            self.enqueue_acquisition_turn(AcquisitionWorkClass::LedgerData);
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
            self.enqueue_acquisition_turn(AcquisitionWorkClass::LedgerData);
        }
        true
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

    fn enqueue_acquisition_turn(self: &Arc<Self>, class: AcquisitionWorkClass) {
        if class == AcquisitionWorkClass::LedgerData {
            self.lifecycle
                .data_jobs_submitted
                .fetch_add(1, Ordering::Relaxed);
        }
        let state = Arc::clone(self);
        let queued_at = Instant::now();
        let job = Box::new(move || {
            if class == AcquisitionWorkClass::LedgerData {
                state
                    .lifecycle
                    .data_jobs_started
                    .fetch_add(1, Ordering::Relaxed);
            }
            state.stats.worker_jobs.fetch_add(1, Ordering::Relaxed);
            state
                .stats
                .worker_queue_wait_us
                .fetch_add(queued_at.elapsed().as_micros() as u64, Ordering::Relaxed);
            run_acquisition_job(&state, "mailbox", || {
                process_acquisition_turn(&state, class)
            });
        });
        match class {
            AcquisitionWorkClass::LedgerData => self.worker_pool.submit_ledger_data(job),
            AcquisitionWorkClass::Persistence => self.worker_pool.submit_persistence(job),
        }
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
        let next = {
            let mut mailbox = self.mailbox.lock().expect("acquisition mailbox lock");
            if self.is_done() {
                mailbox.clear_terminal_work();
                None
            } else {
                mailbox.finish_turn(self.fetch_pack_ready.load(Ordering::Acquire))
            }
        };
        if let Some(class) = next {
            self.enqueue_acquisition_turn(class);
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
            .record_late_event();
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
            self.enqueue_acquisition_turn(AcquisitionWorkClass::Persistence);
        }
    }

    fn take_persistence_event(&self) -> Option<PersistenceReady> {
        self.mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .persistence_events
            .pop_front()
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
        self.worker_pool.submit_persistence(Box::new(move || {
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

        if !packet_complete {
            return Vec::new();
        }
        // This decrements only for a fully reduced packet. A bounded 128-node
        // reducer chunk is an implementation detail, never a peer-credit
        // boundary.
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
                false,
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
            self.enqueue_acquisition_turn(AcquisitionWorkClass::LedgerData);
        }
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
                false,
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
            state_scan_last_yield: "not_run",
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
            state_scan_continuations: self.stats.state_scan_continuations.load(Ordering::Relaxed),
            timeout_dispatches: self.stats.timeout_dispatches.load(Ordering::Relaxed),
            data_drain_runs: self.stats.data_drain_runs.load(Ordering::Relaxed),
            data_drain_us: self.stats.data_drain_us.load(Ordering::Relaxed),
            data_drain_max_us: self.stats.data_drain_max_us.load(Ordering::Relaxed),
            data_drain_max_packets: self.stats.data_drain_max_packets.load(Ordering::Relaxed),
            tx_scan_us: self.stats.tx_scan_us.load(Ordering::Relaxed),
            worker_jobs: self.stats.worker_jobs.load(Ordering::Relaxed),
            worker_queue_wait_us: self.stats.worker_queue_wait_us.load(Ordering::Relaxed),
            tracked_peers: self.peer_set.peer_count(),
            buffered_packets,
            buffered_packets_high_water,
            mailbox_token,
            scan_continuation_pending,
            pending_admitted_timeouts,
            has_active_packet,
        }
    }

    fn cancel_direct_reads(&self) {
        if let Some(reads) = self
            .direct_reads
            .lock()
            .expect("direct NodeStore pass lock")
            .as_ref()
            .cloned()
        {
            reads.cancel();
        }
    }

    /// Explicit terminal cancellation runs before a registry sweep destroys
    /// the handle. It wakes a blocked direct callback collector and settles
    /// packet and persistence ownership rather than relying on a callback.
    pub(crate) fn cancel(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        self.cancel_direct_reads();
        self.persistence
            .lock()
            .expect("persistence queue lock")
            .cancel();
        self.mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .clear_terminal_work();
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
        // `mark_failed` is a terminal outcome in its own right: wake a direct
        // callback collector even when no registry cancellation follows.
        self.cancel_direct_reads();
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
        self.mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .clear_terminal_work();
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

    fn has_remaining_packet_batch(&self) -> bool {
        self.mailbox
            .lock()
            .expect("acquisition mailbox lock")
            .has_remaining_packet_batch()
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
            mutable: Mutex::new(AcqMutableState {
                inbound: InboundLedgerLocal::new_with_reason(self.hash, self.seq, reason),
                store: LocalHydratorStore {
                    pending_writes: Vec::new(),
                    pending_keys: BTreeSet::new(),
                    cache: Arc::clone(&self.fetch_pack),
                    node_store: self.node_store.clone(),
                    ledger_seq: self.seq,
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
            recent_direct_nodes: Mutex::new(BTreeSet::new()),
            direct_reads: Mutex::new(None),
            persistence: Mutex::new(PersistenceQueue::default()),
            draining: AtomicBool::new(false),
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

struct DirectNodeFetcher {
    /// Present only for rippled's synchronous tryDB header/root hydration.
    /// Descendant scans use `disabled` and retain the callback-only path.
    local_hydrator: Option<LocalHydratorStore>,
}

impl DirectNodeFetcher {
    fn disabled() -> Self {
        Self {
            local_hydrator: None,
        }
    }

    fn for_local_hydration(store: &LocalHydratorStore) -> Self {
        Self {
            local_hydrator: Some(store.local_read_clone()),
        }
    }
}

impl shamap::family::SHAMapNodeFetcher for DirectNodeFetcher {
    fn fetch_node_object(
        &self,
        hash: SHAMapHash,
        _ledger_seq: u32,
    ) -> Option<shamap::node_object::NodeObject> {
        // `fetch_node_data` preserves the local tryDB order exactly:
        // fetch-pack cache, then synchronous NodeStore, then SHAMap's normal
        // decode/filter admission. This fetcher is never installed for the
        // post-root descendant scan.
        let data = self
            .local_hydrator
            .as_ref()?
            .fetch_node_data(*hash.as_uint256())?;
        Some(shamap::node_object::NodeObject::new(
            shamap::storage::NodeObjectType::Unknown,
            data,
            *hash.as_uint256(),
        ))
    }
}

fn family<'a>(
    state: &'a AcquisitionState,
) -> SHAMapFamily<
    MonotonicClock,
    HardenedHashBuilder,
    &'a FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>,
    DirectNodeFetcher,
    NullMissingNodeReporter,
    (),
> {
    SHAMapFamily::new(
        Arc::clone(&state.shared_tree_cache),
        &state.worker_full_below,
        DirectNodeFetcher::disabled(),
        NullMissingNodeReporter,
    )
}

fn local_hydrator_family<'a>(
    state: &'a AcquisitionState,
    store: &LocalHydratorStore,
) -> SHAMapFamily<
    MonotonicClock,
    HardenedHashBuilder,
    &'a FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>,
    DirectNodeFetcher,
    NullMissingNodeReporter,
    (),
> {
    SHAMapFamily::new(
        Arc::clone(&state.shared_tree_cache),
        &state.worker_full_below,
        DirectNodeFetcher::for_local_hydration(store),
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
    // rippled probes the ledger header and roots synchronously through tryDB
    // before any network request. `prepare_trigger` below performs those
    // probes directly; they are not acquisition-broker work.
    state
        .lifecycle
        .request_triggers
        .fetch_add(1, Ordering::Relaxed);
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
        let family = local_hydrator_family(state, store);
        let setup = inbound.prepare_trigger(reason, &journal, &config, store, fetch_pack, &family);
        let kind = if setup.state_plan {
            Some(TreeKind::State)
        } else if setup.tx_plan {
            Some(TreeKind::Transaction)
        } else {
            None
        };
        (
            setup.messages_to_send,
            kind,
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
    if terminal {
        finalize_terminal(state);
        return;
    }
    if let Some(kind) = plan {
        process_direct_shamap_descent(state, kind, reason, peer);
    }
}

/// Run rippled's SHAMap-owned `getMissingNodes` descent on the caller thread.
/// The map performs loaded/cache/filter checks, submits direct asynchronous
/// NodeStore reads, collects one strict 512-read callback pass, canonicalizes
/// completed nodes, and resumes before returning the peer frontier.
fn process_direct_shamap_descent(
    state: &Arc<AcquisitionState>,
    kind: TreeKind,
    reason: InboundLedgerRequestTrigger,
    peer: Option<Arc<dyn Peer>>,
) {
    let reads = DirectNodeStoreReads::new();
    {
        let mut active = state
            .direct_reads
            .lock()
            .expect("direct NodeStore pass lock");
        if state.is_done() || active.is_some() {
            return;
        }
        *active = Some(Arc::clone(&reads));
    }
    let result = (|| {
        let mut mutable = state.lock_mutable("direct SHAMap descent")?;
        let journal = WorkerJournal;
        let config = ledger::LedgerConfig::default();
        let family = family(state);
        let AcqMutableState {
            inbound,
            store,
            fetch_pack,
        } = &mut *mutable;
        let ledger = inbound.ledger_mut()?;
        let mut first_child = || basics::random::rand_int_to(255u8);
        let missing = match kind {
            TreeKind::State => {
                let mut filter = ledger::AccountStateSF::new(
                    InboundLedgerSyncStore(&mut *store),
                    &mut *fetch_pack,
                );
                let mut filter_ref: Option<&mut dyn shamap::fetch::SHAMapSyncFilter> =
                    Some(&mut filter);
                ledger
                    .state_map_mut()
                    .get_missing_nodes_deferred_with_family(
                        256,
                        &mut filter_ref,
                        &family,
                        DEFAULT_MAX_DEFERRED_MISSING_NODE_READS,
                        &mut first_child,
                        &mut |hash, seq| reads.request(&state.node_store, hash, seq),
                        &mut |requests| complete_direct_shamap_reads(&reads, requests, &family),
                    )
            }
            TreeKind::Transaction => {
                let mut filter = ledger::TransactionStateSF::new(
                    InboundLedgerSyncStore(&mut *store),
                    &mut *fetch_pack,
                );
                let mut filter_ref: Option<&mut dyn shamap::fetch::SHAMapSyncFilter> =
                    Some(&mut filter);
                ledger.tx_map_mut().get_missing_nodes_deferred_with_family(
                    256,
                    &mut filter_ref,
                    &family,
                    DEFAULT_MAX_DEFERRED_MISSING_NODE_READS,
                    &mut first_child,
                    &mut |hash, seq| reads.request(&state.node_store, hash, seq),
                    &mut |requests| complete_direct_shamap_reads(&reads, requests, &family),
                )
            }
        };
        if reads
            .state
            .lock()
            .expect("direct NodeStore read lock")
            .cancelled
        {
            return None;
        }
        if missing.is_empty() {
            inbound.complete_tree_plan(kind);
        }
        inbound.maybe_finish(&journal);
        let terminal = inbound.is_failed() || inbound.is_complete();
        Some((missing, store.take_pending_writes(), terminal, config))
    })();
    {
        let mut active = state
            .direct_reads
            .lock()
            .expect("direct NodeStore pass lock");
        if active
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &reads))
        {
            *active = None;
        }
    }
    let Some((missing, writes, terminal, _config)) = result else {
        return;
    };
    state.submit_persistence_writes(writes);
    if terminal {
        finalize_terminal(state);
        return;
    }
    if missing.is_empty() {
        trigger(state, reason, peer);
        return;
    }
    let limit = match reason {
        InboundLedgerRequestTrigger::Reply | InboundLedgerRequestTrigger::ReplyHighLatency => {
            INBOUND_LEDGER_REPLY_NODE_REQUEST_CAP
        }
        _ => INBOUND_LEDGER_NORMAL_NODE_REQUEST_CAP,
    };
    let node_ids = select_direct_peer_nodes(state, &missing, reason, limit);
    if node_ids.is_empty() || state.is_done() {
        return;
    }
    let item_type = match kind {
        TreeKind::State => 2,
        TreeKind::Transaction => 1,
    };
    let query_depth = match reason {
        InboundLedgerRequestTrigger::Reply => 1,
        InboundLedgerRequestTrigger::ReplyHighLatency => 2,
        InboundLedgerRequestTrigger::Blind
        | InboundLedgerRequestTrigger::Timeout
        | InboundLedgerRequestTrigger::Added => 0,
    };
    let message = make_get_ledger_with_node_ids(
        state.hash,
        state.seq,
        item_type,
        &node_ids,
        query_depth,
        (reason == InboundLedgerRequestTrigger::Timeout).then_some(0),
    );
    state
        .lifecycle
        .request_messages
        .fetch_add(1, Ordering::Relaxed);
    state.peer_set.send_request(&message, peer.as_ref());
}

/// Select peer node IDs after the direct 256-node SHAMap discovery pass.
/// This mirrors rippled `filterNodes`: prefer hashes not sent since the last
/// timer tick, send nothing for duplicate reply/add triggers, and fall back to
/// the full frontier only on timeout. Selected hashes become recent before the
/// request is emitted.
fn select_direct_peer_nodes(
    state: &AcquisitionState,
    missing: &[(shamap::node_id::SHAMapNodeId, Uint256)],
    reason: InboundLedgerRequestTrigger,
    limit: usize,
) -> Vec<shamap::node_id::SHAMapNodeId> {
    let mut recent = state
        .recent_direct_nodes
        .lock()
        .expect("direct recent-node filter lock");
    filter_direct_peer_nodes(&mut recent, missing, reason, limit)
}

fn filter_direct_peer_nodes(
    recent: &mut BTreeSet<Uint256>,
    missing: &[(shamap::node_id::SHAMapNodeId, Uint256)],
    reason: InboundLedgerRequestTrigger,
    limit: usize,
) -> Vec<shamap::node_id::SHAMapNodeId> {
    if reason == InboundLedgerRequestTrigger::Timeout {
        recent.clear();
    }
    let mut candidates = missing
        .iter()
        .filter(|(_, hash)| !recent.contains(hash))
        .collect::<Vec<_>>();
    if candidates.is_empty() && reason == InboundLedgerRequestTrigger::Timeout {
        candidates = missing.iter().collect();
    }
    candidates
        .into_iter()
        .take(limit)
        .map(|(node_id, hash)| {
            recent.insert(*hash);
            node_id.clone()
        })
        .collect()
}

fn complete_direct_shamap_reads<C, S, FB, F, MR, NS>(
    reads: &DirectNodeStoreReads,
    requests: Vec<DeferredFetchRequestInfo>,
    family: &SHAMapFamily<C, S, FB, F, MR, NS>,
) -> Vec<Option<basics::intrusive_pointer::SharedIntrusive<shamap::tree_node::SHAMapTreeNode>>>
where
    C: basics::tagged_cache::CacheClock,
    S: std::hash::BuildHasher + Clone,
{
    let Some(objects) = reads.wait_for(&requests) else {
        return vec![None; requests.len()];
    };
    requests
        .into_iter()
        .zip(objects)
        .map(|(request, object)| {
            object.and_then(|object| {
                let mut node = shamap::tree_node::SHAMapTreeNode::make_from_prefix(
                    object.data(),
                    request.hash(),
                )
                .ok()?;
                // Callbacks collect only. This caller-thread reduction is rippled's
                // finishFetch/canonicalize/resume boundary.
                family.canonicalize(request.hash(), &mut node);
                Some(node)
            })
        })
        .collect()
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
    let family = local_hydrator_family(state, store);
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

fn process_acquisition_turn(state: &Arc<AcquisitionState>, class: AcquisitionWorkClass) {
    if !state.begin_acquisition_turn() {
        return;
    }
    if state.is_done() {
        state.finish_acquisition_turn();
        return;
    }

    // A persistence dispatch owns only a persistence acknowledgement. If a
    // packet or timeout arrived while it waited in the lower-priority queue,
    // finish_acquisition_turn promotes that work into a counted logical job.
    if class == AcquisitionWorkClass::Persistence {
        process_persistence_event(state);
        state.finish_acquisition_turn();
        return;
    }

    // Match `InboundLedger::runData`: one coalesced receive dispatch drains
    // its packet snapshot through peer credit, prune, and reply sampling.
    // The mailbox bounds an epoch to 128 packets / 4 MiB, and packet chunks
    // release mutable state between steps, so this keeps Rust actor fairness
    // without turning every 128-node continuation into another logical job.
    if state.has_active_packet_batch() {
        process_data_job(state);
        while !state.is_done() && state.has_remaining_packet_batch() {
            process_data_job(state);
        }
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
    state.finish_acquisition_turn();
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

fn process_data_job(state: &Arc<AcquisitionState>) {
    if state.is_done() {
        return;
    }

    // One bounded packet chunk. `process_acquisition_turn` repeatedly calls
    // this reducer for the already-snapshotted mailbox epoch, retaining the
    // same coalesced logical receive job until the full batch is drained.
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

    let data_drain_started = Instant::now();
    let packet_type = work.packet.packet_type;
    let peer_id = work.peer_id;
    let mut packet_stats = SHAMapAddNode::default();
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
    let packet_error_count = usize::from(malformed.is_some()) + usize::from(base_packet_invalid);
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
    record_completed_ledger(acquisition_id, completed, completed_ledger, reason, ledger)
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
    // fetcher: source-reachable NodeStore I/O stays in direct acquisition and
    // persistence workers, never in finalization.
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
    use basics::basic_config::BasicConfig;
    use basics::intrusive_pointer::make_shared_intrusive;
    use nodestore::{DummyScheduler, ManagerImp, NullJournal, Scheduler};
    use shamap::tree_node::{SHAMapNodeType, SHAMapTreeNode};
    use tempfile::TempDir;

    fn test_node_store() -> (TempDir, SHAMapStoreNodeStore) {
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
        .expect("bootstrap node store");
        (dir, bootstrap.node_store)
    }

    fn build_test_acquisition(
        hash: SHAMapHash,
        seq: u32,
        node_store: SHAMapStoreNodeStore,
    ) -> Arc<AcquisitionState> {
        AcquisitionBuilder {
            hash,
            acquisition_id: 1,
            seq,
            reason: AcquireReason::Generic,
            node_store,
            tree_cache: Arc::new(TreeNodeCache::new(
                "direct-read-terminal-test",
                8,
                time::Duration::seconds(60),
                MonotonicClock::default(),
            )),
            fetch_pack: Arc::new(FetchPackCache::new(
                8,
                time::Duration::seconds(60),
                MonotonicClock::default(),
            )),
            failure_recorder: Arc::new(|_| {}),
            completion_recorder: Arc::new(|_, _| {}),
            full_below_generation: 1,
            worker_pool: Arc::new(WorkerPool::new(0)),
            initial_peers: Vec::new(),
            peer_provider: Arc::new(Vec::new),
            lifecycle: Arc::new(AcquisitionLifecycleCounters::default()),
        }
        .build()
    }

    fn store_local_object(
        node_store: &SHAMapStoreNodeStore,
        object_type: nodestore::NodeObjectType,
        data: Vec<u8>,
        hash: Uint256,
        seq: u32,
    ) {
        let result = match node_store {
            SHAMapStoreNodeStore::Single(database) => database.store(object_type, data, hash, seq),
            SHAMapStoreNodeStore::Rotating(database) => {
                database.store(object_type, data, hash, seq)
            }
        };
        result.expect("store local NodeStore fixture object");
    }

    fn local_header(
        seq: u32,
        account_hash: SHAMapHash,
        tx_hash: SHAMapHash,
    ) -> ledger::LedgerHeader {
        ledger::LedgerHeader {
            seq,
            drops: 55,
            parent_hash: SHAMapHash::new(Uint256::from_array([0x01; 32])),
            account_hash,
            tx_hash,
            parent_close_time: 22,
            close_time: 33,
            close_time_resolution: 30,
            close_flags: 0,
            ..ledger::LedgerHeader::default()
        }
    }

    fn test_acquisition() -> (TempDir, Arc<AcquisitionState>) {
        let (dir, node_store) = test_node_store();
        let state = build_test_acquisition(
            SHAMapHash::new(Uint256::from_array([0xD1; 32])),
            1,
            node_store,
        );
        (dir, state)
    }

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
    fn direct_selection_filters_recent_nodes_and_restores_timeout_retry_caps() {
        let missing = (0u8..=255)
            .map(|byte| {
                (
                    shamap::node_id::SHAMapNodeId::default(),
                    Uint256::from_array([byte; 32]),
                )
            })
            .collect::<Vec<_>>();
        let mut recent = BTreeSet::new();

        assert_eq!(
            filter_direct_peer_nodes(
                &mut recent,
                &missing,
                InboundLedgerRequestTrigger::Blind,
                INBOUND_LEDGER_NORMAL_NODE_REQUEST_CAP,
            )
            .len(),
            12
        );
        assert_eq!(
            filter_direct_peer_nodes(
                &mut recent,
                &missing,
                InboundLedgerRequestTrigger::Reply,
                INBOUND_LEDGER_REPLY_NODE_REQUEST_CAP,
            )
            .len(),
            128
        );
        recent.extend(missing.iter().map(|(_, hash)| *hash));
        assert!(
            filter_direct_peer_nodes(
                &mut recent,
                &missing,
                InboundLedgerRequestTrigger::Added,
                INBOUND_LEDGER_NORMAL_NODE_REQUEST_CAP,
            )
            .is_empty(),
            "duplicate non-timeout triggers must be suppressed"
        );
        assert_eq!(
            filter_direct_peer_nodes(
                &mut recent,
                &missing,
                InboundLedgerRequestTrigger::Timeout,
                INBOUND_LEDGER_NORMAL_NODE_REQUEST_CAP,
            )
            .len(),
            12,
            "timeout clears recent nodes before retrying the direct frontier"
        );
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
    fn try_db_completes_with_resident_header_state_and_transaction_roots_without_peer_requests() {
        let (_dir, node_store) = test_node_store();
        let state_root = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            shamap::item::SHAMapItem::new(
                Uint256::from_array([0xA1; 32]),
                vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
            ),
            0,
        ));
        let tx_root = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::TransactionNm,
            shamap::item::SHAMapItem::new(Uint256::from_array([0xB1; 32]), vec![7; 12]),
            0,
        ));
        let header = local_header(91, state_root.get_hash(), tx_root.get_hash());
        let header_hash = ledger::calculate_ledger_hash(&header);

        store_local_object(
            &node_store,
            nodestore::NodeObjectType::Ledger,
            ledger::serialize_prefixed_ledger_header(&header, false),
            *header_hash.as_uint256(),
            header.seq,
        );
        store_local_object(
            &node_store,
            nodestore::NodeObjectType::AccountNode,
            state_root
                .serialize_with_prefix()
                .expect("state root serialization"),
            *state_root.get_hash().as_uint256(),
            header.seq,
        );
        store_local_object(
            &node_store,
            nodestore::NodeObjectType::TransactionNode,
            tx_root
                .serialize_with_prefix()
                .expect("transaction root serialization"),
            *tx_root.get_hash().as_uint256(),
            header.seq,
        );

        let state = build_test_acquisition(header_hash, header.seq, node_store);
        let mut mutable = state
            .lock_mutable("resident tryDB test")
            .expect("mutable state");
        check_local(&state, &mut mutable);

        assert!(mutable.inbound.is_complete());
        assert_eq!(
            mutable.inbound.planner_state(),
            ledger::InboundLedgerPlannerState {
                have_header: true,
                have_state: true,
                have_transactions: true,
            }
        );
        assert_eq!(
            state.lifecycle.request_messages.load(Ordering::Relaxed),
            0,
            "resident tryDB data must complete before any peer request"
        );
    }

    #[test]
    fn try_db_node_store_miss_falls_through_to_the_state_root_network_request() {
        let (_dir, node_store) = test_node_store();
        let missing_state = SHAMapHash::new(Uint256::from_array([0xA2; 32]));
        let header = local_header(92, missing_state, SHAMapHash::default());
        let header_hash = ledger::calculate_ledger_hash(&header);
        store_local_object(
            &node_store,
            nodestore::NodeObjectType::Ledger,
            ledger::serialize_prefixed_ledger_header(&header, false),
            *header_hash.as_uint256(),
            header.seq,
        );
        let state = build_test_acquisition(header_hash, header.seq, node_store);
        {
            let mut mutable = state
                .lock_mutable("tryDB miss test")
                .expect("mutable state");
            check_local(&state, &mut mutable);
            assert!(mutable.inbound.planner_state().have_header);
            assert!(!mutable.inbound.planner_state().have_state);
            assert!(!mutable.inbound.is_complete());
        }

        trigger(&state, InboundLedgerRequestTrigger::Blind, None);
        assert_eq!(
            state.lifecycle.request_messages.load(Ordering::Relaxed),
            1,
            "a root miss must leave tryDB and issue the normal network request"
        );
    }

    #[test]
    fn local_hydrator_fetcher_is_enabled_only_for_try_db_header_and_roots() {
        let source = include_str!("acquisition.rs");
        let trigger = &source[source.find("fn trigger(").expect("trigger source")
            ..source
                .find("/// Run rippled's SHAMap-owned `getMissingNodes` descent")
                .expect("trigger boundary")];
        let descent = &source[source
            .find("fn process_direct_shamap_descent(")
            .expect("direct descent source")
            ..source
                .find("/// Select peer node IDs after the direct 256-node SHAMap discovery pass.")
                .expect("direct descent boundary")];

        assert!(trigger.contains("local_hydrator_family(state, store)"));
        assert!(
            descent.contains("let family = family(state);"),
            "post-root traversal must use the disabled fetcher family"
        );
        assert!(
            !descent.contains("local_hydrator_family"),
            "post-root traversal must not synchronously probe NodeStore descendants"
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
    fn terminal_failure_wakes_blocked_direct_callback_collector() {
        let (_dir, state) = test_acquisition();
        let reads = DirectNodeStoreReads::new();
        *state
            .direct_reads
            .lock()
            .expect("direct NodeStore pass lock") = Some(Arc::clone(&reads));

        let blocked_callback = std::thread::spawn(move || {
            reads.wait_until_cancelled();
        });
        state.mark_failed();
        blocked_callback
            .join()
            .expect("terminal failure must wake the blocked collector");
        assert!(state.failed.load(Ordering::Acquire));
        assert!(state.stopped.load(Ordering::Acquire));
    }

    #[test]
    fn terminal_clear_releases_mailbox_packet_and_event_ownership() {
        let mut mailbox = AcquisitionMailbox::default();
        mailbox.packets.push_back(packet_work(7, 64));
        mailbox.packet_bytes = 64;
        mailbox.pending_timeouts = 1;
        mailbox.token = AcquisitionWorkToken::Running;

        mailbox.clear_terminal_work();

        assert!(mailbox.packets.is_empty());
        assert_eq!(mailbox.packet_bytes, 0);
        assert_eq!(mailbox.pending_timeouts, 0);
        assert_eq!(mailbox.token, AcquisitionWorkToken::Idle);
    }
}
