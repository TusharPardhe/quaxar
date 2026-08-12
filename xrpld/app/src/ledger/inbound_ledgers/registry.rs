//! Global inbound ledger registry — matching rippled's InboundLedgersImp.
//!
//! ONE global registry: HashMap<Uint256, Entry>. A single Mutex protects
//! the map. Each entry holds an Arc<AcquisitionState> for the per-ledger
//! state machine.

use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::MonotonicClock;
use ledger::{FetchPackCache, InboundLedgerPacket, Ledger};
use overlay::Peer;
use protocol::JsonValue;
use shamap::family::{FullBelowCache, FullBelowCacheImpl};
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::runtime::overlay_runtime::AppOverlayRuntime;
use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

use super::acquisition::{
    AcquisitionBuilder, AcquisitionCompletionRecorder, AcquisitionFailureRecorder,
    AcquisitionPeerProvider, AcquisitionSnapshot, AcquisitionState,
};
use super::read_broker::{NodeReadBroker, ReadBrokerConfig};
use super::worker_pool::WorkerPool;

// ─── Constants ───────────────────────────────────────────────────────────────

/// How long a failed hash stays in recent_failures (prevents retry storms).
const FAILURE_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// Entries idle longer than this are swept. rippled removes an inbound ledger
/// once its last action is more than one minute old; its separate failure cache
/// retains failed hashes for five minutes.
const SWEEP_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// `fetch_info` must remain diagnostic-only and bounded even during a recovery
/// flood. Operators can use the aggregate fields to see work beyond this set.
const FETCH_INFO_MAX_ACQUISITIONS: usize = 16;

fn response_sequence_matches_request(expected_seq: u32, response_seq: u32) -> bool {
    expected_seq == 0 || response_seq == 0 || expected_seq == response_seq
}

/// rippled's `JtLedgerData` JobType permits at most three running jobs.
/// This bounds packet processing while leaving the inbound registry free to
/// track any number of hash-deduplicated acquisitions.
const WORKER_COUNT: usize = 3;

/// Cumulative, process-lifetime counters covering every inbound-ledger
/// lifecycle boundary. They are sampled by NetworkOPs at most once every five
/// seconds, avoiding per-packet journal traffic during recovery pressure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AcquisitionLifecycleSnapshot {
    pub acquisition_starts: u64,
    pub acquisition_existing: u64,
    pub wire_ledger_data: u64,
    pub wire_relayed: u64,
    pub wire_invalid_hash: u64,
    pub wire_nodes: u64,
    pub route_attempts: u64,
    pub route_accepted: u64,
    pub route_misses: u64,
    pub route_terminal: u64,
    pub route_sequence_mismatch: u64,
    pub stale_packet_attempts: u64,
    pub stale_packets_stored: u64,
    pub initialization_jobs: u64,
    pub request_triggers: u64,
    pub request_messages: u64,
    pub peers_added: u64,
    pub data_jobs_submitted: u64,
    pub data_jobs_coalesced: u64,
    pub data_jobs_started: u64,
    pub packet_steps: u64,
    pub packet_step_errors: u64,
    pub packet_steps_completed: u64,
    pub timeout_jobs: u64,
    pub timeout_no_progress: u64,
    pub timeout_retries: u64,
    pub timeout_queue_rejected: u64,
    pub terminal_completed: u64,
    pub terminal_failed: u64,
}

#[derive(Default)]
pub(crate) struct AcquisitionLifecycleCounters {
    pub acquisition_starts: AtomicU64,
    pub acquisition_existing: AtomicU64,
    pub wire_ledger_data: AtomicU64,
    pub wire_relayed: AtomicU64,
    pub wire_invalid_hash: AtomicU64,
    pub wire_nodes: AtomicU64,
    pub route_attempts: AtomicU64,
    pub route_accepted: AtomicU64,
    pub route_misses: AtomicU64,
    pub route_terminal: AtomicU64,
    pub route_sequence_mismatch: AtomicU64,
    pub stale_packet_attempts: AtomicU64,
    pub stale_packets_stored: AtomicU64,
    pub initialization_jobs: AtomicU64,
    pub request_triggers: AtomicU64,
    pub request_messages: AtomicU64,
    pub peers_added: AtomicU64,
    pub data_jobs_submitted: AtomicU64,
    pub data_jobs_coalesced: AtomicU64,
    pub data_jobs_started: AtomicU64,
    pub packet_steps: AtomicU64,
    pub packet_step_errors: AtomicU64,
    pub packet_steps_completed: AtomicU64,
    pub timeout_jobs: AtomicU64,
    pub timeout_no_progress: AtomicU64,
    pub timeout_retries: AtomicU64,
    pub timeout_queue_rejected: AtomicU64,
    pub terminal_completed: AtomicU64,
    pub terminal_failed: AtomicU64,
}

impl AcquisitionLifecycleCounters {
    fn snapshot(&self) -> AcquisitionLifecycleSnapshot {
        macro_rules! load {
            ($field:ident) => {
                self.$field.load(Ordering::Relaxed)
            };
        }
        AcquisitionLifecycleSnapshot {
            acquisition_starts: load!(acquisition_starts),
            acquisition_existing: load!(acquisition_existing),
            wire_ledger_data: load!(wire_ledger_data),
            wire_relayed: load!(wire_relayed),
            wire_invalid_hash: load!(wire_invalid_hash),
            wire_nodes: load!(wire_nodes),
            route_attempts: load!(route_attempts),
            route_accepted: load!(route_accepted),
            route_misses: load!(route_misses),
            route_terminal: load!(route_terminal),
            route_sequence_mismatch: load!(route_sequence_mismatch),
            stale_packet_attempts: load!(stale_packet_attempts),
            stale_packets_stored: load!(stale_packets_stored),
            initialization_jobs: load!(initialization_jobs),
            request_triggers: load!(request_triggers),
            request_messages: load!(request_messages),
            peers_added: load!(peers_added),
            data_jobs_submitted: load!(data_jobs_submitted),
            data_jobs_coalesced: load!(data_jobs_coalesced),
            data_jobs_started: load!(data_jobs_started),
            packet_steps: load!(packet_steps),
            packet_step_errors: load!(packet_step_errors),
            packet_steps_completed: load!(packet_steps_completed),
            timeout_jobs: load!(timeout_jobs),
            timeout_no_progress: load!(timeout_no_progress),
            timeout_retries: load!(timeout_retries),
            timeout_queue_rejected: load!(timeout_queue_rejected),
            terminal_completed: load!(terminal_completed),
            terminal_failed: load!(terminal_failed),
        }
    }
}

// ─── Reason enum ─────────────────────────────────────────────────────────────

/// Why a ledger is being acquired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquireReason {
    /// Consensus / validation path.
    Consensus,
    /// LedgerMaster, catchup, publication.
    Generic,
    /// History fill, sequential catchup.
    History,
}

/// A completed acquisition retains its origin until LedgerMaster consumes it.
/// This distinguishes rippled's non-validating `storeLedger` paths from its
/// history-only `setFullLedger` path.
#[derive(Debug, Clone)]
pub struct CompletedInboundLedger {
    pub ledger: Arc<Ledger>,
    pub reason: AcquireReason,
    pub acquisition_id: u64,
}

// ─── Entry ───────────────────────────────────────────────────────────────────

struct Entry {
    id: u64,
    /// Requested sequence constraint. Zero means the hash is the only
    /// pre-completion identity; a completed ledger header supplies its
    /// authoritative sequence to the strand.
    seq: u32,
    #[allow(dead_code)]
    reason: AcquireReason,
    state: Arc<AcquisitionState>,
    last_touched: Instant,
    #[allow(dead_code)]
    started_at: Instant,
    completed_ledger: Option<Arc<Ledger>>,
    /// Completion has been durably consumed by the app layer. Keep the
    /// completed object in the registry until the normal sweep, matching
    /// rippled's InboundLedgers map, while suppressing duplicate handoffs.
    completion_acknowledged: bool,
    failed: bool,
}

// ─── RegistryInner ───────────────────────────────────────────────────────────

struct RegistryInner {
    entries: BTreeMap<Uint256, Entry>,
    recent_failures: HashMap<Uint256, Instant>,
    /// Successful terminal completions in exact completion order. Each item
    /// remains queued until acknowledged, and polling rotates unacknowledged
    /// items fairly so a failed persistence attempt cannot block later work.
    /// This is the direct equivalent of rippled's `InboundLedger::done()`
    /// dispatching `AcqDone`; it avoids discovering a ready acquisition by
    /// scanning an arbitrary bounded slice of the full registry.
    completed_ready: VecDeque<(Uint256, u64)>,
}

fn failure_matches_entry(acquisition_id: Option<u64>, entry_id: u64) -> bool {
    acquisition_id.is_none_or(|id| entry_id == id)
}

fn record_recent_failure_at(
    inner: &mut RegistryInner,
    hash: Uint256,
    _acquisition_id: Option<u64>,
    now: Instant,
) {
    // Match InboundLedgersImp::logFailure: AcqDone records a hash-wide
    // cooldown even if the failed object has already been swept. Admission
    // policy decides where that cooldown is consulted (History only).

    inner.recent_failures.entry(hash).or_insert(now);
    if let Some(entry) = inner.entries.get_mut(&hash)
        && failure_matches_entry(_acquisition_id, entry.id)
        && !entry.failed
    {
        // InboundLedger::done touches the acquisition before queued AcqDone
        // records the failure. Preserve that terminal sweep lifetime without
        // extending it on later poll passes.
        entry.failed = true;
        entry.last_touched = now;
    }
}

fn record_recent_failure(inner: &mut RegistryInner, hash: Uint256, acquisition_id: Option<u64>) {
    record_recent_failure_at(inner, hash, acquisition_id, Instant::now());
}

/// Match `InboundLedger::done()`'s terminal `touch()`. The callback carries an
/// acquisition identity so a delayed terminal event from a swept predecessor
/// cannot extend the lifetime of a replacement entry for the same hash.
fn touch_terminal_entry_at(
    inner: &mut RegistryInner,
    hash: Uint256,
    acquisition_id: u64,
    now: Instant,
) {
    if let Some(entry) = inner.entries.get_mut(&hash)
        && entry.id == acquisition_id
    {
        entry.last_touched = now;
    }
}

#[derive(Clone)]
struct RecoveryLclDecision {
    recorded_at: Instant,
    preferred_hash: Uint256,
    candidate_hash: Option<Uint256>,
    candidate_seq: Option<u32>,
    source: String,
    decision: String,
}

fn acquire_reason_name(reason: AcquireReason) -> &'static str {
    match reason {
        AcquireReason::Consensus => "consensus",
        AcquireReason::Generic => "generic",
        AcquireReason::History => "history",
    }
}

fn acquisition_snapshot_json(
    hash: Uint256,
    requested_seq: u32,
    reason: AcquireReason,
    idle_ms: u64,
    complete: bool,
    failed: bool,
    snapshot: AcquisitionSnapshot,
) -> JsonValue {
    let lookup_total = snapshot
        .node_store_fetch_hits
        .saturating_add(snapshot.node_store_fetch_misses);
    let lookup_hit_rate_ppm = (lookup_total != 0).then(|| {
        snapshot
            .node_store_fetch_hits
            .saturating_mul(1_000_000)
            .saturating_div(lookup_total)
    });
    let average_worker_queue_wait_us = (snapshot.worker_jobs != 0).then(|| {
        snapshot
            .worker_queue_wait_us
            .saturating_div(snapshot.worker_jobs)
    });
    let mut values = BTreeMap::from([
        ("hash".to_owned(), JsonValue::String(hash.to_string())),
        (
            "requested_seq".to_owned(),
            JsonValue::Unsigned(requested_seq as u64),
        ),
        ("seq".to_owned(), JsonValue::Unsigned(snapshot.seq as u64)),
        (
            "reason".to_owned(),
            JsonValue::String(acquire_reason_name(reason).to_owned()),
        ),
        ("age_ms".to_owned(), JsonValue::Unsigned(snapshot.age_ms)),
        ("idle_ms".to_owned(), JsonValue::Unsigned(idle_ms)),
        ("complete".to_owned(), JsonValue::Bool(complete)),
        ("failed".to_owned(), JsonValue::Bool(failed)),
        (
            "have_header".to_owned(),
            JsonValue::Bool(snapshot.have_header),
        ),
        (
            "have_state".to_owned(),
            JsonValue::Bool(snapshot.have_state),
        ),
        (
            "have_transactions".to_owned(),
            JsonValue::Bool(snapshot.have_transactions),
        ),
        (
            "timeouts".to_owned(),
            JsonValue::Unsigned(snapshot.timeouts as u64),
        ),
        ("packets".to_owned(), JsonValue::Unsigned(snapshot.packets)),
        (
            "useful_packets".to_owned(),
            JsonValue::Unsigned(snapshot.useful_packets),
        ),
        (
            "useful_nodes".to_owned(),
            JsonValue::Unsigned(snapshot.useful_nodes),
        ),
        (
            "state_packets".to_owned(),
            JsonValue::Unsigned(snapshot.state_packets),
        ),
        (
            "state_useful_nodes".to_owned(),
            JsonValue::Unsigned(snapshot.state_useful_nodes),
        ),
        (
            "state_duplicate_nodes".to_owned(),
            JsonValue::Unsigned(snapshot.state_duplicate_nodes),
        ),
        (
            "malformed_packets".to_owned(),
            JsonValue::Unsigned(snapshot.malformed_packets),
        ),
        (
            "state_scan_runs".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_runs),
        ),
        (
            "state_missing_nodes".to_owned(),
            JsonValue::Unsigned(snapshot.state_missing_nodes),
        ),
        (
            "tx_missing_nodes".to_owned(),
            JsonValue::Unsigned(snapshot.tx_missing_nodes),
        ),
        (
            "state_scan_us".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_us),
        ),
        (
            "state_scan_branch_steps".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_branch_steps),
        ),
        (
            "state_scan_branches_seen".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_branches_seen),
        ),
        (
            "state_scan_missing_nodes_recorded".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_missing_nodes_recorded),
        ),
        (
            "state_scan_positive_progress_slices".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_positive_progress_slices),
        ),
        (
            "state_scan_branch_budget_yields".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_branch_budget_yields),
        ),
        (
            "state_scan_deferred_read_budget_yields".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_deferred_read_budget_yields),
        ),
        (
            "state_scan_deferred_read_resume_yields".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_deferred_read_resume_yields),
        ),
        (
            "state_scan_missing_node_limit_yields".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_missing_node_limit_yields),
        ),
        (
            "state_scan_completed_slices".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_completed_slices),
        ),
        (
            "state_scan_last_yield".to_owned(),
            JsonValue::String(snapshot.state_scan_last_yield.to_owned()),
        ),
        (
            "state_scan_last_branch_steps".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_last_branch_steps),
        ),
        (
            "state_scan_last_deferred_reads".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_last_deferred_reads),
        ),
        (
            "state_scan_last_deferred_resumes".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_last_deferred_resumes),
        ),
        (
            "state_scan_last_missing_nodes".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_last_missing_nodes),
        ),
        (
            "state_scan_duplicate_missing_hashes".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_duplicate_missing_hashes),
        ),
        (
            "state_scan_full_below_hits".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_full_below_hits),
        ),
        (
            "state_scan_loaded_or_cached_children".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_loaded_or_cached_children),
        ),
        (
            "state_scan_pending_reads".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_pending_reads),
        ),
        (
            "state_scan_read_slot_full".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_read_slot_full),
        ),
        (
            "state_scan_read_admission_accepted".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_read_admission_accepted),
        ),
        (
            "state_scan_read_admission_deferred".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_read_admission_deferred),
        ),
        (
            "state_scan_read_admission_attached".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_read_admission_attached),
        ),
        (
            "state_scan_read_broker_rejected".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_read_broker_rejected),
        ),
        (
            "state_scan_max_pending_reads".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_max_pending_reads),
        ),
        (
            "state_scan_pending_hits".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_pending_hits),
        ),
        (
            "state_scan_pending_misses".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_pending_misses),
        ),
        (
            "state_scan_deferred_resumes".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_deferred_resumes),
        ),
        (
            "state_scan_yields".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_yields),
        ),
        (
            "state_scan_continuations".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_continuations),
        ),
        (
            "timeout_dispatches".to_owned(),
            JsonValue::Unsigned(snapshot.timeout_dispatches),
        ),
        (
            "state_scan_max_buffered_packets".to_owned(),
            JsonValue::Unsigned(snapshot.state_scan_max_buffered_packets),
        ),
        (
            "data_drain_runs".to_owned(),
            JsonValue::Unsigned(snapshot.data_drain_runs),
        ),
        (
            "data_drain_us".to_owned(),
            JsonValue::Unsigned(snapshot.data_drain_us),
        ),
        (
            "data_drain_max_us".to_owned(),
            JsonValue::Unsigned(snapshot.data_drain_max_us),
        ),
        (
            "data_drain_max_packets".to_owned(),
            JsonValue::Unsigned(snapshot.data_drain_max_packets),
        ),
        (
            "tx_scan_us".to_owned(),
            JsonValue::Unsigned(snapshot.tx_scan_us),
        ),
        (
            "worker_jobs".to_owned(),
            JsonValue::Unsigned(snapshot.worker_jobs),
        ),
        (
            "worker_queue_wait_us".to_owned(),
            JsonValue::Unsigned(snapshot.worker_queue_wait_us),
        ),
        (
            "node_store_lookup_hits".to_owned(),
            JsonValue::Unsigned(snapshot.node_store_fetch_hits),
        ),
        (
            "node_store_lookup_misses".to_owned(),
            JsonValue::Unsigned(snapshot.node_store_fetch_misses),
        ),
        (
            "tracked_peers".to_owned(),
            JsonValue::Unsigned(snapshot.tracked_peers as u64),
        ),
        (
            "buffered_packets".to_owned(),
            JsonValue::Unsigned(snapshot.buffered_packets as u64),
        ),
        (
            "buffered_packets_high_water".to_owned(),
            JsonValue::Unsigned(snapshot.buffered_packets_high_water as u64),
        ),
        (
            "mailbox_token".to_owned(),
            JsonValue::String(snapshot.mailbox_token.to_owned()),
        ),
        (
            "scan_continuation_pending".to_owned(),
            JsonValue::Bool(snapshot.scan_continuation_pending),
        ),
        (
            "pending_admitted_timeouts".to_owned(),
            JsonValue::Unsigned(snapshot.pending_admitted_timeouts as u64),
        ),
        (
            "has_active_packet".to_owned(),
            JsonValue::Bool(snapshot.has_active_packet),
        ),
    ]);
    values.insert(
        "header_after_ms".to_owned(),
        snapshot
            .header_after_ms
            .map(JsonValue::Unsigned)
            .unwrap_or(JsonValue::Null),
    );
    values.insert(
        "node_store_lookup_hit_rate_ppm".to_owned(),
        lookup_hit_rate_ppm
            .map(JsonValue::Unsigned)
            .unwrap_or(JsonValue::Null),
    );
    values.insert(
        "average_worker_queue_wait_us".to_owned(),
        average_worker_queue_wait_us
            .map(JsonValue::Unsigned)
            .unwrap_or(JsonValue::Null),
    );
    JsonValue::Object(values)
}

fn recovery_lcl_decision_json(decision: Option<RecoveryLclDecision>) -> JsonValue {
    let Some(decision) = decision else {
        return JsonValue::Null;
    };
    let mut value = BTreeMap::from([
        (
            "age_ms".to_owned(),
            JsonValue::Unsigned(decision.recorded_at.elapsed().as_millis() as u64),
        ),
        (
            "preferred_hash".to_owned(),
            JsonValue::String(decision.preferred_hash.to_string()),
        ),
        ("source".to_owned(), JsonValue::String(decision.source)),
        ("decision".to_owned(), JsonValue::String(decision.decision)),
    ]);
    value.insert(
        "candidate_hash".to_owned(),
        decision
            .candidate_hash
            .map(|hash| JsonValue::String(hash.to_string()))
            .unwrap_or(JsonValue::Null),
    );
    value.insert(
        "candidate_seq".to_owned(),
        decision
            .candidate_seq
            .map(|seq| JsonValue::Unsigned(seq as u64))
            .unwrap_or(JsonValue::Null),
    );
    JsonValue::Object(value)
}

/// Thread-safe global service for inbound ledger acquisition.
///
/// Matches rippled's InboundLedgers: one entry per hash, touch-on-access,
/// sweep idle entries, route peer responses, fixed worker pool.
pub struct InboundLedgers {
    inner: Arc<Mutex<RegistryInner>>,
    worker_pool: Arc<WorkerPool>,
    read_broker: NodeReadBroker,
    // Shared resources for creating acquisitions
    node_store: Arc<RwLock<Option<SHAMapStoreNodeStore>>>,
    tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    full_below: Arc<FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>>,
    fetch_pack: Arc<FetchPackCache>,
    overlay_rt: Arc<RwLock<Option<Arc<AppOverlayRuntime>>>>,
    completed_ledgers_tx: SyncSender<CompletedInboundLedger>,
    stopping: AtomicBool,
    need_network_ledger: Arc<AtomicBool>,
    pending_acquires: Arc<Mutex<HashSet<Uint256>>>,
    next_acquisition_id: AtomicU64,
    /// Last preferred-LCL selection outcome, retained solely for bounded
    /// operator diagnostics.
    recovery_lcl_decision: Mutex<Option<RecoveryLclDecision>>,
    /// Shared lifecycle counters incremented at request, wire, worker, retry,
    /// and terminal boundaries. The sampled snapshot never mutates state.
    lifecycle: Arc<AcquisitionLifecycleCounters>,
}

impl InboundLedgers {
    /// Create a new InboundLedgers service.
    pub fn new(
        tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
        full_below: Arc<FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>>,
        fetch_pack: Arc<FetchPackCache>,
        completed_ledgers_tx: SyncSender<CompletedInboundLedger>,
        need_network_ledger: Arc<AtomicBool>,
    ) -> Self {
        Self::with_worker_pool(
            tree_cache,
            full_below,
            fetch_pack,
            completed_ledgers_tx,
            need_network_ledger,
            Arc::new(WorkerPool::new(WORKER_COUNT)),
        )
    }

    /// Construct the registry around its worker queue. Production always uses
    /// the fixed-size pool above; tests provide a zero-worker pool so they can
    /// prove real ingress scheduling and draining without timing races.
    fn with_worker_pool(
        tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
        full_below: Arc<FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>>,
        fetch_pack: Arc<FetchPackCache>,
        completed_ledgers_tx: SyncSender<CompletedInboundLedger>,
        need_network_ledger: Arc<AtomicBool>,
        worker_pool: Arc<WorkerPool>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RegistryInner {
                entries: BTreeMap::new(),
                recent_failures: HashMap::new(),
                completed_ready: VecDeque::new(),
            })),
            worker_pool,
            read_broker: NodeReadBroker::new(ReadBrokerConfig::default())
                .expect("default inbound read broker bounds are valid"),
            node_store: Arc::new(RwLock::new(None)),
            tree_cache,
            full_below,
            fetch_pack,
            overlay_rt: Arc::new(RwLock::new(None)),
            completed_ledgers_tx,
            stopping: AtomicBool::new(false),
            need_network_ledger,
            pending_acquires: Arc::new(Mutex::new(HashSet::new())),
            next_acquisition_id: AtomicU64::new(1),
            recovery_lcl_decision: Mutex::new(None),
            lifecycle: Arc::new(AcquisitionLifecycleCounters::default()),
        }
    }

    // ─── Configuration setters (called during app startup) ───────────────

    /// Returns the shared full-below cache Arc for metrics attachment.
    pub fn full_below_cache(
        &self,
    ) -> &Arc<FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>> {
        &self.full_below
    }

    pub fn set_overlay_rt(&self, rt: Arc<AppOverlayRuntime>) {
        let mut guard = self.overlay_rt.write().expect("overlay_rt write");
        *guard = Some(rt);
    }

    pub fn set_node_store(&self, ns: SHAMapStoreNodeStore) {
        let mut guard = self.node_store.write().expect("node_store write");
        *guard = Some(ns);
    }

    /// Return a read-only, cumulative trace of inbound ledger work across all
    /// active and completed acquisitions. This is intentionally aggregate and
    /// is emitted only through sampled diagnostics or explicit fetch_info.
    pub(crate) fn lifecycle_snapshot(&self) -> AcquisitionLifecycleSnapshot {
        self.lifecycle.snapshot()
    }

    /// Record a parsed wire-level ledger-data packet before registry routing.
    pub fn note_wire_ledger_data(&self, node_count: usize) {
        self.lifecycle
            .wire_ledger_data
            .fetch_add(1, Ordering::Relaxed);
        self.lifecycle
            .wire_nodes
            .fetch_add(node_count as u64, Ordering::Relaxed);
    }

    /// Record a ledger-data relay that is intentionally not processed locally.
    pub fn note_wire_ledger_data_relayed(&self) {
        self.lifecycle.wire_relayed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a malformed ledger-data message whose hash could not be parsed.
    pub fn note_wire_ledger_data_invalid_hash(&self) {
        self.lifecycle
            .wire_invalid_hash
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record whether an unroutable state packet was saved for later fetch-pack
    /// use. The call does not change routing or peer charging behavior.
    pub fn note_stale_packet_result(&self, stored: bool) {
        self.lifecycle
            .stale_packet_attempts
            .fetch_add(1, Ordering::Relaxed);
        if stored {
            self.lifecycle
                .stale_packets_stored
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    // ─── Core API ────────────────────────────────────────────────────────

    /// Acquire a ledger by hash. Returns immediately if already complete.
    /// If not tracked, starts a new acquisition. If in-progress, touches
    /// the entry and returns None.
    ///
    /// Matches rippled's `InboundLedgers::acquire()`.
    pub fn acquire(&self, hash: Uint256, seq: u32, reason: AcquireReason) -> Option<Arc<Ledger>> {
        if hash.is_zero() {
            tracing::warn!(target: "inbound_ledger", "acquire: REJECTED zero hash");
            return None;
        }
        if self.stopping.load(Ordering::Acquire) {
            tracing::warn!(target: "inbound_ledger", %hash, "acquire: REJECTED stopping");
            return None;
        }
        if self.need_network_ledger.load(Ordering::Acquire)
            && reason != AcquireReason::Generic
            && reason != AcquireReason::Consensus
        {
            tracing::info!(target: "inbound_ledger", %hash, seq, "acquire: REJECTED need_network_ledger");
            return None;
        }

        let mut inner = self.inner.lock().expect("inbound_ledgers lock");

        // Existing acquisition: a failed one returns immediately, exactly as
        // rippled checks `InboundLedger::isFailed()` before `update()`. Do
        // not touch it or refresh recent_failures: its first failure owns the
        // one-minute sweep age and five-minute cooldown. A live acquisition
        // can update an unknown sequence and is retained for the sweep window.
        if let Some(entry) = inner.entries.get_mut(&hash) {
            self.lifecycle
                .acquisition_existing
                .fetch_add(1, Ordering::Relaxed);
            let entry_failed = entry.failed || entry.state.failed.load(Ordering::Acquire);
            let entry_id = entry.id;
            let entry_reason = entry.reason;
            let completion_acknowledged = entry.completion_acknowledged;
            if entry_failed {
                if reason == AcquireReason::Consensus {
                    tracing::info!(
                        target: "lcl_trace",
                        event = "preferred_lcl_registry_existing",
                        %hash,
                        requested_seq = seq,
                        requested_reason = ?reason,
                        acquisition_id = entry_id,
                        entry_reason = ?entry_reason,
                        entry_seq = entry.seq,
                        completed = entry.completed_ledger.is_some()
                            || entry.state.completed.load(Ordering::Acquire),
                        failed = true,
                        completion_acknowledged,
                        returned_ledger = false,
                        "LCL trace: preferred target found as failed inbound entry"
                    );
                }
                return None;
            }
            entry.last_touched = Instant::now();
            let update_seq = (entry.seq == 0 && seq != 0).then_some(seq);
            if let Some(update_seq) = update_seq {
                entry.seq = update_seq;
            }
            let entry_seq = entry.seq;
            let is_completed = entry.state.completed.load(Ordering::Acquire);
            let completed_ledger = entry.completed_ledger.clone();
            let state = Arc::clone(&entry.state);

            // Do not call into AcquisitionState while holding the registry
            // mutex: worker failure reporting can arrive from acquisition
            // state and then lock this registry.
            drop(inner);
            if let Some(update_seq) = update_seq {
                state.update_seq(update_seq);
            }
            let result = if is_completed {
                completed_ledger.or_else(|| state.completed_ledger())
            } else {
                completed_ledger
            };
            // An in-progress entry is already correlated by the single
            // `preferred_lcl_registry_started` event and the periodic
            // `preferred_lcl_candidate_lookup` result. Logging every
            // consensus caller that observes it produces an unbounded trace
            // storm without adding a lifecycle transition. Emit this event
            // only when the registry can actually return the completed ledger.
            if reason == AcquireReason::Consensus && result.is_some() {
                tracing::info!(
                    target: "lcl_trace",
                    event = "preferred_lcl_registry_existing",
                    %hash,
                    requested_seq = seq,
                    requested_reason = ?reason,
                    acquisition_id = entry_id,
                    entry_reason = ?entry_reason,
                    entry_seq,
                    completed = is_completed,
                    failed = false,
                    completion_acknowledged,
                    returned_ledger = result.is_some(),
                    returned_seq = result.as_ref().map(|ledger| ledger.header().seq),
                    "LCL trace: preferred target found in inbound registry"
                );
            }
            return result;
        }

        // Match rippled InboundLedgers::acquire: retain one acquisition per
        // hash and let the normal per-hash lifecycle, failure cooldown, and
        // idle sweep bound work. Do not globally serialize hash-only
        // consensus requests while the preferred LCL advances.

        // Validate required resources
        let ns = {
            let guard = self.node_store.read().expect("node_store read");
            match guard.as_ref() {
                Some(ns) => ns.clone(),
                None => {
                    tracing::warn!(target: "inbound_ledger", %hash, seq, "acquire: REJECTED node_store not attached");
                    return None;
                }
            }
        };
        // The reference PeerSet consults the live overlay on each addPeers
        // turn. Keep a cheap provider in the acquisition rather than freezing
        // a construction-time snapshot of peer sessions.
        let peer_provider: AcquisitionPeerProvider = {
            let overlay_rt = Arc::clone(&self.overlay_rt);
            Arc::new(move || {
                use overlay::Overlay as _;
                overlay_rt
                    .read()
                    .expect("inbound overlay_rt read")
                    .as_ref()
                    .map(|runtime| runtime.overlay().active_peers())
                    .unwrap_or_default()
            })
        };
        let initial_peers = peer_provider();
        let initial_peer_count = initial_peers.len();
        let acquisition_id = self.next_acquisition_id.fetch_add(1, Ordering::Relaxed);
        let failure_recorder: AcquisitionFailureRecorder = {
            let inner = Arc::clone(&self.inner);
            Arc::new(move |failed_hash| {
                let mut inner = inner.lock().expect("inbound_ledgers failure recorder lock");
                record_recent_failure(&mut inner, failed_hash, Some(acquisition_id));
            })
        };
        let completion_recorder: AcquisitionCompletionRecorder = {
            let inner = Arc::clone(&self.inner);
            Arc::new(move |completed_hash, ledger| {
                let mut inner = inner
                    .lock()
                    .expect("inbound_ledgers completion recorder lock");
                // Match InboundLedger::done(): terminal completion owns the
                // last-action update, so queue latency cannot make a newly
                // completed ledger eligible for an immediate sweep.
                let registered = if let Some(entry) = inner.entries.get_mut(&completed_hash)
                    && entry.id == acquisition_id
                {
                    entry.completed_ledger = Some(ledger);
                    entry.last_touched = Instant::now();
                    true
                } else {
                    false
                };
                if registered {
                    // `InboundLedger::done()` reaches `AcqDone` only after
                    // the completed ledger has been stored. Publish the exact
                    // acquisition identity here, rather than asking the
                    // strand to eventually discover it by scanning entries.
                    inner
                        .completed_ready
                        .push_back((completed_hash, acquisition_id));
                } else {
                    // Match rippled's InboundLedgers::sweep ownership: once
                    // an entry is no longer resident, a late completion must
                    // not keep a full ledger alive outside the inbound map.
                    // A later validation/history request can acquire it again.
                    tracing::debug!(
                        target: "inbound_ledger",
                        hash = %completed_hash,
                        acquisition_id,
                        "dropping completion for swept inbound ledger"
                    );
                }
            })
        };
        let full_below_gen = self.full_below.generation().wrapping_add(1);

        let acq_state = AcquisitionBuilder {
            hash: SHAMapHash::new(hash),
            acquisition_id,
            seq,
            reason,
            node_store: ns,
            read_broker: self.read_broker.clone(),
            tree_cache: Arc::clone(&self.tree_cache),
            fetch_pack: Arc::clone(&self.fetch_pack),
            store_tx: self.completed_ledgers_tx.clone(),
            failure_recorder,
            completion_recorder,
            full_below_generation: full_below_gen,
            worker_pool: Arc::clone(&self.worker_pool),
            initial_peers,
            peer_provider,
            lifecycle: Arc::clone(&self.lifecycle),
        }
        .build();

        let now = Instant::now();
        inner.entries.insert(
            hash,
            Entry {
                id: acquisition_id,
                seq,
                reason,
                state: Arc::clone(&acq_state),
                last_touched: now,
                started_at: now,
                completed_ledger: None,
                completion_acknowledged: false,
                failed: false,
            },
        );
        drop(inner);

        if reason == AcquireReason::Consensus {
            tracing::info!(
                target: "lcl_trace",
                event = "preferred_lcl_registry_started",
                %hash,
                seq,
                reason = ?reason,
                acquisition_id,
                initial_peer_count,
                "LCL trace: started a new preferred-target inbound acquisition"
            );
        }
        tracing::info!(target: "inbound_ledger", seq, %hash, reason = ?reason, acquisition_id, "Acquisition started");
        self.lifecycle
            .acquisition_starts
            .fetch_add(1, Ordering::Relaxed);
        acq_state.start();
        None
    }

    /// Fire-and-forget acquire (for consensus/validation callers).
    /// Checks a pending set to avoid duplicate acquisitions.
    pub fn acquire_async(&self, hash: Uint256, seq: u32, reason: AcquireReason) {
        {
            let mut pending = self.pending_acquires.lock().expect("pending_acquires lock");
            if !pending.insert(hash) {
                return;
            }
        }
        let _ = self.acquire(hash, seq, reason);
        self.pending_acquires
            .lock()
            .expect("pending_acquires lock")
            .remove(&hash);
    }

    /// Start a closed-ledger acquisition when only its hash is known.
    ///
    /// A peer's advertised history range is not a reliable sequence binding
    /// for its current closed-ledger hash. Keep the sequence unknown until the
    /// response header establishes it, while retaining the hash as the primary
    /// acquisition key.
    pub fn acquire_closed_ledger_async(&self, hash: Uint256, reason: AcquireReason) {
        self.acquire_async(hash, 0, reason);
    }

    /// Route a TMLedgerData response to the correct acquisition.
    pub fn route_response(&self, hash: &Uint256, peer_id: u64, packet: InboundLedgerPacket) {
        let _ = self.route_response_with_seq(hash, peer_id, None, packet);
    }

    /// Route a response while checking the sequence advertised on the wire.
    ///
    /// The ledger hash is the primary acquisition key. When a nonzero
    /// sequence is available, it is also checked against the acquisition's
    /// requested sequence so a peer cannot feed a response for another
    /// ledger into an active acquisition.
    pub fn route_response_with_seq(
        &self,
        hash: &Uint256,
        peer_id: u64,
        response_seq: Option<u32>,
        packet: InboundLedgerPacket,
    ) -> bool {
        self.lifecycle
            .route_attempts
            .fetch_add(1, Ordering::Relaxed);
        let state = {
            let mut inner = self.inner.lock().expect("inbound_ledgers lock");
            let Some(entry) = inner.entries.get_mut(hash) else {
                self.lifecycle.route_misses.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(target: "inbound_ledger", %hash, peer_id, "route_response: registry miss");
                return false;
            };
            if entry.failed
                || entry.state.failed.load(Ordering::Acquire)
                || entry.state.stopped.load(Ordering::Acquire)
            {
                self.lifecycle
                    .route_terminal
                    .fetch_add(1, Ordering::Relaxed);
                tracing::debug!(target: "inbound_ledger", %hash, peer_id, "route_response: ignored terminal acquisition");
                return false;
            }
            if let Some(response_seq) = response_seq
                && !response_sequence_matches_request(entry.seq, response_seq)
            {
                self.lifecycle
                    .route_sequence_mismatch
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    target: "inbound_ledger",
                    %hash,
                    expected_seq = entry.seq,
                    response_seq,
                    peer_id,
                    "route_response: sequence mismatch"
                );
                return false;
            }
            // Wire receipt does not change rippled InboundLedger::lastAction.
            // Only construction, duplicate acquire/update, and terminal done
            // refresh the sweep clock.
            Arc::clone(&entry.state)
        };

        if state.enqueue_packet(peer_id, packet) {
            self.lifecycle
                .route_accepted
                .fetch_add(1, Ordering::Relaxed);
            tracing::debug!(target: "inbound_ledger", %hash, peer_id, "route_response: registry hit");
            true
        } else {
            self.lifecycle
                .route_terminal
                .fetch_add(1, Ordering::Relaxed);
            tracing::warn!(target: "inbound_ledger", %hash, peer_id, "route_response: acquisition mailbox overloaded or terminal");
            false
        }
    }

    /// Remove entries idle for more than one minute, matching
    /// `InboundLedgersImp::sweep`. Failed hashes separately remain in the
    /// five-minute recent-failure cache.
    pub fn sweep(&self) {
        // TaggedCache only expires age/size entries when swept. Keep the
        // shared fetch-pack source on the same lifecycle cadence as rippled's
        // inbound-ledger cache rather than retaining stale peer data forever.
        self.fetch_pack.sweep();
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        let mut to_remove = Vec::new();

        for (hash, entry) in &inner.entries {
            let idle_for = now.duration_since(entry.last_touched);
            // A successful completion must remain resolver-visible until its
            // AcqDone-equivalent consumer acknowledges persistence/acceptance.
            // Rippled stores before dispatching AcqDone; retaining this entry
            // prevents a delayed strand turn from silently losing that ledger.
            if idle_for > SWEEP_IDLE_TIMEOUT
                && !entry.state.has_pending_durability()
                && (!entry.state.completed.load(Ordering::Acquire)
                    && entry.completed_ledger.is_none()
                    || entry.completion_acknowledged)
            {
                to_remove.push((
                    *hash,
                    entry.seq,
                    entry.reason,
                    idle_for,
                    entry.failed || entry.state.failed.load(Ordering::Acquire),
                    entry.completed_ledger.is_some()
                        || entry.state.completed.load(Ordering::Acquire),
                ));
            }
        }

        let mut swept_states = Vec::new();
        for (hash, seq, reason, idle_for, failed, completed) in to_remove {
            if let Some(entry) = inner.entries.remove(&hash) {
                tracing::info!(
                    target: "lcl_trace",
                    event = "inbound_swept",
                    %hash,
                    seq,
                    reason = ?reason,
                    idle_ms = idle_for.as_millis() as u64,
                    failed,
                    completed,
                    "LCL trace: inbound acquisition removed by sweep"
                );
                // Match rippled's sweep: removing an idle InboundLedger
                // releases its completed ledger. Remove its direct completion
                // handoff as well; future validation/history work will
                // reacquire the exact hash.
                inner
                    .completed_ready
                    .retain(|(ready_hash, _)| *ready_hash != hash);
                swept_states.push(entry.state);
            }
        }
        inner
            .recent_failures
            .retain(|_, when| when.elapsed() < FAILURE_COOLDOWN);
        drop(inner);

        // Mirrors InboundLedger destruction: useful state-node packets that
        // were received but not yet processed can seed a later acquisition.
        for state in swept_states {
            let buffered = state.take_buffered_packets();
            state.cancel();
            for received in buffered {
                if received.packet.packet_type == ledger::InboundLedgerDataType::StateNode {
                    let stored = self.stash_stale_packet(&received.packet);
                    self.note_stale_packet_result(stored);
                }
            }
        }
    }

    /// Record a preferred-LCL request, selection, installation, or rejection
    /// without mutating acquisition or consensus state. `fetch_info` exposes
    /// only this most recent decision so normal recovery does not allocate an
    /// unbounded event history.
    pub fn record_recovery_lcl_decision(
        &self,
        preferred_hash: Uint256,
        candidate: Option<&Ledger>,
        source: &str,
        decision: &str,
    ) {
        let (candidate_hash, candidate_seq) = candidate
            .map(|ledger| (*ledger.header().hash.as_uint256(), ledger.header().seq))
            .unzip();
        *self
            .recovery_lcl_decision
            .lock()
            .expect("recovery_lcl_decision lock") = Some(RecoveryLclDecision {
            recorded_at: Instant::now(),
            preferred_hash,
            candidate_hash,
            candidate_seq,
            source: source.to_owned(),
            decision: decision.to_owned(),
        });
    }

    /// Return a bounded, read-only acquisition snapshot for `fetch_info`.
    /// It does not execute a SHAMap walk, read NodeStore, add peers, or alter
    /// selection state.
    pub fn fetch_info_bounded(&self, limit: usize) -> JsonValue {
        let limit = limit.min(FETCH_INFO_MAX_ACQUISITIONS);
        let (entries, active, completed, failed, recent_failures, decision) = {
            let inner = self.inner.lock().expect("inbound_ledgers lock");
            let mut active = 0u64;
            let mut completed = 0u64;
            let mut failed = 0u64;
            for entry in inner.entries.values() {
                if entry.failed || entry.state.failed.load(Ordering::Acquire) {
                    failed += 1;
                } else if entry.completed_ledger.is_some()
                    || entry.state.completed.load(Ordering::Acquire)
                {
                    completed += 1;
                } else {
                    active += 1;
                }
            }
            let entries = inner
                .entries
                .iter()
                .take(limit)
                .map(|(hash, entry)| {
                    (
                        *hash,
                        entry.seq,
                        entry.reason,
                        entry.last_touched.elapsed().as_millis() as u64,
                        entry.completed_ledger.is_some()
                            || entry.state.completed.load(Ordering::Acquire),
                        entry.failed || entry.state.failed.load(Ordering::Acquire),
                        Arc::clone(&entry.state),
                    )
                })
                .collect::<Vec<_>>();
            (
                entries,
                active,
                completed,
                failed,
                inner.recent_failures.len() as u64,
                self.recovery_lcl_decision
                    .lock()
                    .expect("recovery_lcl_decision lock")
                    .clone(),
            )
        };

        let acquisitions = entries
            .into_iter()
            .map(
                |(hash, requested_seq, reason, idle_ms, complete, failed, state)| {
                    acquisition_snapshot_json(
                        hash,
                        requested_seq,
                        reason,
                        idle_ms,
                        complete,
                        failed,
                        state.diagnostics(),
                    )
                },
            )
            .collect();
        let worker = self.worker_pool.snapshot();
        let lifecycle = self.lifecycle_snapshot();
        let mut result = BTreeMap::from([
            ("active".to_owned(), JsonValue::Unsigned(active)),
            ("completed".to_owned(), JsonValue::Unsigned(completed)),
            ("failed".to_owned(), JsonValue::Unsigned(failed)),
            (
                "recent_failures".to_owned(),
                JsonValue::Unsigned(recent_failures),
            ),
            ("acquisitions".to_owned(), JsonValue::Array(acquisitions)),
            (
                "worker_pool".to_owned(),
                JsonValue::Object(BTreeMap::from([
                    (
                        "queued_jobs".to_owned(),
                        JsonValue::Unsigned(worker.queued_jobs as u64),
                    ),
                    (
                        "outstanding_ledger_data_jobs".to_owned(),
                        JsonValue::Unsigned(worker.outstanding_ledger_data_jobs as u64),
                    ),
                    (
                        "worker_count".to_owned(),
                        JsonValue::Unsigned(worker.worker_count as u64),
                    ),
                    (
                        "ledger_data_job_limit".to_owned(),
                        JsonValue::Unsigned(worker.ledger_data_job_limit as u64),
                    ),
                    (
                        "timeout_submission_attempts".to_owned(),
                        JsonValue::Unsigned(worker.timeout_submission_attempts),
                    ),
                    (
                        "timeout_submission_rejected".to_owned(),
                        JsonValue::Unsigned(worker.timeout_submission_rejected),
                    ),
                ])),
            ),
        ]);
        result.insert(
            "lifecycle".to_owned(),
            JsonValue::String(format!("{lifecycle:?}")),
        );
        result.insert(
            "last_recovery_lcl_decision".to_owned(),
            recovery_lcl_decision_json(decision),
        );
        JsonValue::Object(result)
    }

    /// Check if tracking a hash.
    pub fn contains(&self, hash: &Uint256) -> bool {
        let inner = self.inner.lock().expect("inbound_ledgers lock");
        inner.entries.contains_key(hash)
    }

    /// Number of in-progress acquisitions.
    pub fn active_count(&self) -> usize {
        let inner = self.inner.lock().expect("inbound_ledgers lock");
        inner
            .entries
            .values()
            .filter(|e| !e.failed && e.completed_ledger.is_none())
            .count()
    }

    /// Notify that a ledger was completed (called externally or by sweep).
    pub fn on_complete(&self, hash: Uint256, ledger: Arc<Ledger>) {
        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        let completion_id = if let Some(entry) = inner.entries.get_mut(&hash) {
            if entry.completed_ledger.is_none() {
                entry.completed_ledger = Some(ledger);
                entry.last_touched = Instant::now();
                Some(entry.id)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(acquisition_id) = completion_id {
            inner.completed_ready.push_back((hash, acquisition_id));
        }
    }

    /// Notify that a ledger acquisition failed.
    pub fn on_failed(&self, hash: Uint256) {
        let state = {
            let mut inner = self.inner.lock().expect("inbound_ledgers lock");
            record_recent_failure(&mut inner, hash, None);
            inner
                .entries
                .get(&hash)
                .map(|entry| Arc::clone(&entry.state))
        };
        if let Some(state) = state {
            state.cancel();
        }
    }

    /// Log a failure for the given hash/seq (matches rippled's `logFailure`).
    pub fn log_failure(&self, hash: Uint256, _seq: u32) {
        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        record_recent_failure(&mut inner, hash, None);
    }

    /// Check whether a hash is recorded as a recent failure (matches rippled's
    /// `isFailure`). Expires entries older than `FAILURE_COOLDOWN` (5 minutes).
    pub fn is_failure(&self, hash: &Uint256) -> bool {
        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        inner
            .recent_failures
            .retain(|_, t| t.elapsed() < FAILURE_COOLDOWN);
        inner
            .recent_failures
            .get(hash)
            .is_some_and(|t| t.elapsed() < FAILURE_COOLDOWN)
    }

    /// Clear both `recent_failures` and `ledgers_` (matches rippled's
    /// `clearFailures`).
    pub fn clear_failures(&self) {
        let entries = {
            let mut inner = self.inner.lock().expect("inbound_ledgers lock");
            inner.recent_failures.clear();
            inner.completed_ready.clear();
            std::mem::take(&mut inner.entries)
        };
        for entry in entries.into_values() {
            entry.state.cancel();
        }
    }

    /// Send current peers to all active acquisition workers.
    pub fn send_peers(&self, peers: &[Arc<dyn Peer>]) {
        let states: Vec<Arc<AcquisitionState>> = {
            let inner = self.inner.lock().expect("inbound_ledgers lock");
            inner
                .entries
                .values()
                .filter(|e| !e.failed && e.completed_ledger.is_none())
                .map(|e| Arc::clone(&e.state))
                .collect()
        };
        for state in states {
            if state.stopped.load(Ordering::Acquire) || state.completed.load(Ordering::Acquire) {
                continue;
            }
            state.peer_set.refresh_peers(peers.iter().cloned());
        }
    }

    /// Store an object received in a fetch-pack response in the cache read by
    /// all acquisition workers.
    pub fn store_fetch_pack(&self, hash: Uint256, data: Vec<u8>) {
        self.fetch_pack.add_fetch_pack(hash, data);
    }

    /// Stash state-node data from an untracked ledger response, matching
    /// `InboundLedgersImp::gotStaleData`.
    pub fn stash_stale_packet(&self, packet: &InboundLedgerPacket) -> bool {
        if packet.packet_type != ledger::InboundLedgerDataType::StateNode {
            return false;
        }
        for node in &packet.nodes {
            if node.node_id.is_none() {
                return false;
            }
            let Ok(Some(decoded)) =
                shamap::tree_node::SHAMapTreeNode::make_from_wire(&node.node_data)
            else {
                return false;
            };
            let Ok(prefixed) = decoded.serialize_with_prefix() else {
                return false;
            };
            self.fetch_pack
                .add_fetch_pack(*decoded.get_hash().as_uint256(), prefixed);
        }
        true
    }

    /// Send fetch-pack-ready signal to all in-progress acquisitions.
    pub fn notify_fetch_pack_ready(&self) {
        let states: Vec<Arc<AcquisitionState>> = {
            let inner = self.inner.lock().expect("inbound_ledgers lock");
            inner
                .entries
                .values()
                .filter(|e| !e.failed && e.completed_ledger.is_none())
                .map(|e| Arc::clone(&e.state))
                .collect()
        };
        for state in states {
            if state.stopped.load(Ordering::Acquire) || state.completed.load(Ordering::Acquire) {
                continue;
            }
            state.fetch_pack_ready.store(true, Ordering::Release);
            state.submit_data_job();
        }
    }

    /// Remove a specific entry.
    pub fn remove(&self, hash: &Uint256) {
        let state = {
            let mut inner = self.inner.lock().expect("inbound_ledgers lock");
            let state = inner.entries.remove(hash).map(|entry| entry.state);
            if state.is_some() {
                inner
                    .completed_ready
                    .retain(|(ready_hash, _)| ready_hash != hash);
            }
            state
        };
        if let Some(state) = state {
            state.cancel();
        }
    }

    /// Stop all acquisitions and shut down the worker pool.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Release);

        let entries = {
            let mut inner = self.inner.lock().expect("inbound_ledgers lock");
            let entries = std::mem::take(&mut inner.entries);
            inner.recent_failures.clear();
            inner.completed_ready.clear();
            entries
        };
        for (_, entry) in entries {
            entry.state.cancel();
        }
        self.read_broker.stop();
        self.worker_pool.stop();
    }

    // ─── Catchup loop compatibility API ──────────────────────────────────

    /// Poll all completed acquisitions. Prefer `poll_results_bounded` from
    /// timer-driven consensus paths so a completion flood cannot starve the
    /// heartbeat.
    pub fn poll_results(&self) -> Vec<(Uint256, u64, Ledger, AcquireReason)> {
        self.poll_results_bounded(usize::MAX)
    }

    /// Poll at most `budget` successful terminal handoffs. A completion is
    /// enqueued by its terminal callback after it has been made registry-
    /// visible; polling therefore never scans arbitrary in-progress entries.
    /// Unacknowledged results rotate to the back of the queue, so failed
    /// persistence retries remain durable without starving later completions.
    pub fn poll_results_bounded(
        &self,
        budget: usize,
    ) -> Vec<(Uint256, u64, Ledger, AcquireReason)> {
        if budget == 0 {
            return Vec::new();
        }

        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        // Examine only the entries that were ready at this poll's start. This
        // avoids returning the same unacknowledged completion repeatedly when
        // the caller's budget exceeds the ready queue length. The budget counts
        // live handoffs, not stale identities that must be discarded.
        let ready_this_turn = inner.completed_ready.len();
        let mut completed = Vec::with_capacity(ready_this_turn.min(budget));
        for _ in 0..ready_this_turn {
            if completed.len() >= budget {
                break;
            }
            let Some((hash, acquisition_id)) = inner.completed_ready.pop_front() else {
                break;
            };
            let ready = inner.entries.get(&hash).and_then(|entry| {
                if entry.id != acquisition_id
                    || entry.completion_acknowledged
                    || entry.failed
                    || entry.state.failed.load(Ordering::Acquire)
                {
                    return None;
                }
                entry
                    .completed_ledger
                    .clone()
                    .map(|ledger| (ledger, entry.reason))
            });
            let Some((ledger, reason)) = ready else {
                // Completion recording inserts the ledger before enqueuing
                // this exact (hash, acquisition_id), so a non-ready item is
                // stale: it was failed, acknowledged, swept, or replaced.
                // Drop it rather than rotating it indefinitely and consuming
                // later bounded polls ahead of live completions.
                continue;
            };
            inner.completed_ready.push_back((hash, acquisition_id));
            completed.push((hash, acquisition_id, (*ledger).clone(), reason));
        }
        completed
    }

    /// Acknowledge that a consumer has durably handled this completed result.
    /// A failed persistence attempt must not call this: the completed state is
    /// retained so the owner can retry on a later bounded poll.
    pub fn acknowledge_completed(&self, hash: &Uint256, acquisition_id: u64) {
        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        let acknowledged = inner.entries.get_mut(hash).and_then(|entry| {
            let completed = !entry.failed
                && !entry.state.failed.load(Ordering::Acquire)
                && entry.id == acquisition_id
                && (entry.completed_ledger.is_some()
                    || entry.state.completed.load(Ordering::Acquire));
            if !completed {
                return None;
            }
            // Keep completed ledgers resident until the ordinary inbound
            // sweep. `acquire(hash, ...)` must still return this ledger,
            // just as rippled finds completed InboundLedger objects.
            entry.completion_acknowledged = true;
            Some((entry.seq, entry.reason, entry.id))
        });
        if let Some((seq, reason, entry_id)) = acknowledged {
            inner.completed_ready.retain(|(ready_hash, ready_id)| {
                *ready_hash != *hash || *ready_id != acquisition_id
            });
            tracing::info!(
                target: "lcl_trace",
                event = "inbound_completion_acknowledged",
                %hash,
                seq,
                reason = ?reason,
                acquisition_id = entry_id,
                "LCL trace: completed inbound ledger retained until sweep after acknowledgement"
            );
        } else {
            tracing::warn!(
                target: "lcl_trace",
                event = "inbound_completion_ack_rejected",
                %hash,
                "LCL trace: completion acknowledgement found no completed registry entry"
            );
        }
    }

    /// Check if a specific hash is currently in-progress (not completed, not failed).
    pub fn is_in_progress(&self, hash: &Uint256) -> bool {
        let inner = self.inner.lock().expect("inbound_ledgers lock");
        inner.entries.get(hash).is_some_and(|e| {
            !e.failed && e.completed_ledger.is_none() && !e.state.completed.load(Ordering::Acquire)
        })
    }

    /// Remove non-in-progress entries with seq below `min_seq`.
    /// Returns number of entries removed.
    pub fn remove_in_progress_below_seq(&self, min_seq: u32) -> usize {
        if min_seq <= 1 {
            return 0;
        }
        let (count, states) = {
            let mut inner = self.inner.lock().expect("inbound_ledgers lock");
            let stale: Vec<Uint256> = inner
                .entries
                .iter()
                .filter(|(_, entry)| {
                    (entry.completed_ledger.is_some() || entry.failed)
                        && !entry.state.has_pending_durability()
                        && entry.seq > 1
                        && entry.seq < min_seq
                })
                .map(|(hash, _)| *hash)
                .collect();
            let count = stale.len();
            let mut states = Vec::with_capacity(count);
            for hash in stale {
                if let Some(entry) = inner.entries.remove(&hash) {
                    inner
                        .completed_ready
                        .retain(|(ready_hash, _)| *ready_hash != hash);
                    states.push(entry.state);
                }
            }
            (count, states)
        };
        for state in states {
            state.cancel();
        }
        count
    }

    /// Log-visible summary shaped after reference InboundLedgers::getInfo.
    pub fn info_summary(&self) -> String {
        let inner = self.inner.lock().expect("inbound_ledgers lock");
        let active = inner
            .entries
            .values()
            .filter(|e| {
                !e.failed
                    && e.completed_ledger.is_none()
                    && !e.state.completed.load(Ordering::Acquire)
            })
            .count();
        let complete = inner
            .entries
            .values()
            .filter(|e| e.completed_ledger.is_some() || e.state.completed.load(Ordering::Acquire))
            .count();
        let failed = inner.recent_failures.len();
        let mut entries: Vec<String> = inner
            .entries
            .iter()
            .map(|(hash, entry)| {
                let key = if entry.seq > 1 {
                    entry.seq.to_string()
                } else {
                    hash.to_string()
                };
                let state_label = if entry.failed {
                    "failed"
                } else if entry.completed_ledger.is_some()
                    || entry.state.completed.load(Ordering::Acquire)
                {
                    "complete"
                } else {
                    "in_progress"
                };
                format!("{}:{}", key, state_label)
            })
            .collect();
        entries.sort();
        format!(
            "active={} complete={} failed={} entries=[{}]",
            active,
            complete,
            failed,
            entries.join(",")
        )
    }

    /// Check whether an in-progress acquisition has the given sequence or hash.
    /// Completed entries remain until their consumer acknowledges successful
    /// cache/persistence handling, so they must not block the next history
    /// predecessor request.
    pub fn has_entry_for_seq_or_hash(&self, seq: u32, hash: &Uint256) -> bool {
        let inner = self.inner.lock().expect("inbound_ledgers lock");
        inner.entries.iter().any(|(entry_hash, entry)| {
            !entry.failed
                && !entry.state.failed.load(Ordering::Acquire)
                && entry.completed_ledger.is_none()
                && !entry.state.completed.load(Ordering::Acquire)
                && (*entry_hash == *hash || entry.seq == seq)
        })
    }

    /// Remove stale in-progress acquisitions that have had no progress.
    /// Used during cold bootstrap to free slots for new targets.
    /// Returns the number of entries removed.
    pub fn remove_stale_no_progress(&self, idle_timeout: Duration) -> Vec<(Uint256, u32)> {
        let now = Instant::now();
        let (stale, states) = {
            let mut inner = self.inner.lock().expect("inbound_ledgers lock");
            let stale: Vec<(Uint256, u32)> = inner
                .entries
                .iter()
                .filter(|(_, e)| {
                    !e.failed
                        && e.completed_ledger.is_none()
                        && !e.state.completed.load(Ordering::Acquire)
                        && now.duration_since(e.last_touched) > idle_timeout
                })
                .map(|(hash, e)| (*hash, e.seq))
                .collect();
            let mut states = Vec::with_capacity(stale.len());
            for (hash, _) in &stale {
                if let Some(entry) = inner.entries.remove(hash) {
                    inner
                        .completed_ready
                        .retain(|(ready_hash, _)| ready_hash != hash);
                    states.push(entry.state);
                }
            }
            (stale, states)
        };
        for state in states {
            state.cancel();
        }
        stale
    }

    /// Look up a hash for a target sequence from completed (but not yet polled)
    /// acquisitions' ledger skip lists.
    pub fn hash_for_seq_from_completed(
        &self,
        target_seq: u32,
    ) -> Option<basics::sha_map_hash::SHAMapHash> {
        // Snapshot candidates before consulting the acquisition's mutable
        // state. Workers can report failure while holding acquisition state,
        // so taking `inner` and then `state.mutable` would invert the
        // registry's worker callback lock order. The requested sequence may
        // be zero for preferred-LCL recovery; rank by the actual completed
        // ledger header rather than the original request sequence.
        let candidates: Vec<_> = {
            let inner = self.inner.lock().expect("inbound_ledgers lock");
            inner
                .entries
                .values()
                .filter(|entry| {
                    !entry.failed
                        && !entry.state.failed.load(Ordering::Acquire)
                        && (entry.completed_ledger.is_some()
                            || entry.state.completed.load(Ordering::Acquire))
                })
                .map(|entry| (entry.completed_ledger.clone(), Arc::clone(&entry.state)))
                .collect()
        };

        let mut best: Option<(u32, basics::sha_map_hash::SHAMapHash)> = None;
        for (completed_ledger, state) in candidates {
            let mut consider = |ledger: &ledger::Ledger| {
                let ledger_seq = ledger.header().seq;
                if ledger_seq < target_seq {
                    return;
                }
                if let Some(hash) = ledger
                    .hash_of_seq(target_seq, &ledger::NullLedgerJournal)
                    .filter(|hash| !hash.is_zero())
                    && best.is_none_or(|(best_seq, _)| ledger_seq < best_seq)
                {
                    best = Some((ledger_seq, hash));
                }
            };

            if let Some(ledger) = completed_ledger {
                consider(&ledger);
            } else {
                let mutable = state.mutable.lock().expect("acq mutable (hash lookup)");
                if let Some(ledger) = mutable.inbound.ledger() {
                    consider(ledger);
                }
            }
        }
        best.map(|(_, hash)| hash)
    }

    /// Find a candidate reference hash from completed acquisitions for hash
    /// discovery when direct lookup fails.
    pub fn candidate_reference_hash_from_completed(
        &self,
        target_seq: u32,
    ) -> Option<(u32, basics::sha_map_hash::SHAMapHash)> {
        // Use candidate_ledger_for_seq logic inline: round up to next 256 boundary.
        let candidate_seq = target_seq.saturating_add(255) & !255;
        if candidate_seq <= target_seq {
            return None;
        }

        // As above, snapshot under the registry mutex and inspect acquisition
        // mutable state only after releasing it. Rank by an acquisition's
        // learned ledger header so completed hash-only recovery candidates
        // participate even though their initial request sequence was zero.
        let candidates: Vec<_> = {
            let inner = self.inner.lock().expect("inbound_ledgers lock");
            inner
                .entries
                .values()
                .filter(|entry| {
                    !entry.failed
                        && !entry.state.failed.load(Ordering::Acquire)
                        && (entry.completed_ledger.is_some()
                            || entry.state.completed.load(Ordering::Acquire))
                })
                .map(|entry| (entry.completed_ledger.clone(), Arc::clone(&entry.state)))
                .collect()
        };

        let mut best: Option<(u32, basics::sha_map_hash::SHAMapHash)> = None;
        for (completed_ledger, state) in candidates {
            let mut consider = |ledger: &ledger::Ledger| {
                let ledger_seq = ledger.header().seq;
                if ledger_seq < target_seq {
                    return;
                }
                if let Some(hash) = ledger
                    .hash_of_seq(candidate_seq, &ledger::NullLedgerJournal)
                    .filter(|hash| !hash.is_zero())
                    && best.is_none_or(|(best_seq, _)| ledger_seq < best_seq)
                {
                    best = Some((ledger_seq, hash));
                }
            };

            if let Some(ledger) = completed_ledger {
                consider(&ledger);
            } else {
                let mutable = state.mutable.lock().expect("acq mutable (candidate)");
                if let Some(ledger) = mutable.inbound.ledger() {
                    consider(ledger);
                }
            }
        }
        best.map(|(_, hash)| (candidate_seq, hash))
    }
}

impl std::fmt::Debug for InboundLedgers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboundLedgers").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::super::acquisition::AcquisitionSnapshot;
    use super::super::worker_pool::WorkerPool;
    use super::{
        AcquireReason, AcquisitionLifecycleCounters, AcquisitionLifecycleSnapshot, InboundLedgers,
        RecoveryLclDecision, RegistryInner, SWEEP_IDLE_TIMEOUT, acquisition_snapshot_json,
        failure_matches_entry, record_recent_failure, record_recent_failure_at,
        recovery_lcl_decision_json, response_sequence_matches_request, touch_terminal_entry_at,
    };
    use basics::base_uint::Uint256;
    use basics::basic_config::BasicConfig;
    use basics::hardened_hash::HardenedHashBuilder;
    use basics::tagged_cache::MonotonicClock;
    use ledger::{FetchPackCache, InboundLedgerDataType, InboundLedgerPacket, Ledger};
    use nodestore::{DummyScheduler, ManagerImp, NullJournal, Scheduler};
    use overlay::{Peer, PeerImp, PeerSet as _};
    use protocol::{JsonValue, PublicKey};
    use shamap::family::FullBelowCacheImpl;
    use shamap::tree_node_cache::TreeNodeCache;
    use std::collections::{BTreeMap, HashMap, VecDeque};
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    /// Build the same real in-memory node store used by the acquisition
    /// integration fixtures. `acquire` performs a local-store check before
    /// processing ingress, so a genuine store keeps this route test on the
    /// production path.
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

    fn registry_with_manual_worker_pool(worker_pool: Arc<WorkerPool>) -> (TempDir, InboundLedgers) {
        let (dir, node_store) = test_node_store();
        let (completed_tx, _completed_rx) = mpsc::sync_channel(1);
        let registry = InboundLedgers::with_worker_pool(
            Arc::new(TreeNodeCache::new(
                "registry-ingress-test",
                8,
                time::Duration::seconds(60),
                MonotonicClock::default(),
            )),
            Arc::new(FullBelowCacheImpl::new(
                1,
                MonotonicClock::default(),
                HardenedHashBuilder::default(),
                8,
            )),
            Arc::new(FetchPackCache::new(
                8,
                time::Duration::seconds(60),
                MonotonicClock::default(),
            )),
            completed_tx,
            Arc::new(AtomicBool::new(false)),
            worker_pool,
        );
        registry.set_node_store(node_store);
        (dir, registry)
    }

    #[test]
    fn registry_ingress_coalesces_and_charges_selected_peer_malformed_packets() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, registry) = registry_with_manual_worker_pool(Arc::clone(&worker_pool));
        let hash = Uint256::from_array([0xC7; 32]);
        let peer = PeerImp::new(
            77,
            SocketAddr::from(([127, 0, 0, 1], 51235)),
            PublicKey::from_bytes([0x03; 33]),
            "registry-ingress-parity-peer",
        );
        peer.record_ledger(hash, 1);

        assert!(
            registry.acquire(hash, 1, AcquireReason::Generic).is_none(),
            "production registry creates the active acquisition"
        );
        let state = {
            let inner = registry.inner.lock().expect("registry lock");
            Arc::clone(
                &inner
                    .entries
                    .get(&hash)
                    .expect("active acquisition entry")
                    .state,
            )
        };
        let queued_before_ingress = worker_pool.snapshot().queued_jobs;
        assert_eq!(
            queued_before_ingress, 1,
            "acquire queues its real initialization job"
        );

        let malformed = || InboundLedgerPacket::new(InboundLedgerDataType::Base, Vec::new());
        assert!(registry.route_response_with_seq(&hash, 77, Some(1), malformed()));
        assert!(registry.route_response_with_seq(&hash, 77, Some(1), malformed()));
        assert_eq!(
            worker_pool.snapshot().queued_jobs,
            queued_before_ingress + 1,
            "two production registry routes queue one coalesced packet worker"
        );
        let routed = registry.lifecycle_snapshot();
        assert_eq!(routed.route_attempts, 2);
        assert_eq!(routed.route_accepted, 2);
        assert_eq!(routed.data_jobs_submitted, 1);
        assert_eq!(routed.data_jobs_coalesced, 1);

        // Initialization refreshes from the absent overlay runtime. Install the
        // selected live peer after that production setup turn, before the
        // first bounded FIFO packet step runs.
        assert!(worker_pool.run_next_job_for_test());
        state
            .peer_set
            .refresh_peers(vec![Arc::clone(&peer) as Arc<dyn Peer>]);
        state.peer_set.add_peers(1, &mut |_| true, &mut |_| {});
        assert!(worker_pool.run_next_job_for_test());
        assert!(
            worker_pool.run_next_job_for_test(),
            "the first bounded packet step must schedule one continuation"
        );

        let charges = peer.charges();
        assert_eq!(
            charges.len(),
            2,
            "both routed packets are processed in FIFO continuation order"
        );
        for (fee, context) in charges {
            assert_eq!(fee, (*resource::FEE_MALFORMED_REQUEST).clone());
            assert_eq!(context, "ledger_data empty header");
        }
        let drained = registry.lifecycle_snapshot();
        assert_eq!(drained.data_jobs_started, 2);
        assert_eq!(drained.packet_steps, 2);
        assert_eq!(drained.packet_steps_completed, 2);
        assert_eq!(drained.packet_step_errors, 2);
        registry.stop();
    }

    #[test]
    fn acquire_async_initializes_once_before_ledger_data_work() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, registry) = registry_with_manual_worker_pool(Arc::clone(&worker_pool));
        let hash = Uint256::from_array([0xC8; 32]);

        registry.acquire_async(hash, 1, AcquireReason::Consensus);
        registry.acquire_async(hash, 1, AcquireReason::Consensus);

        // rippled runs InboundLedger::init from acquire(), so this must not
        // be a queued JtLedgerData-equivalent job. The only queued work is
        // the first TimeoutCounter callback armed by initialization.
        assert_eq!(registry.lifecycle_snapshot().initialization_jobs, 1);
        assert_eq!(
            worker_pool.snapshot().queued_jobs,
            1,
            "only the timeout callback may remain queued after synchronous initialization"
        );
        assert!(worker_pool.run_next_job_for_test());
        assert_eq!(registry.lifecycle_snapshot().initialization_jobs, 1);
        registry.stop();
    }

    #[test]
    fn acquire_initialization_does_not_starve_timeout_admission() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, registry) = registry_with_manual_worker_pool(Arc::clone(&worker_pool));

        // rippled's acquire() runs InboundLedger::init immediately. Each new
        // incomplete ledger then asks TimeoutCounter to queue recovery work;
        // with a five-job admission limit, the first five are live and only
        // the sixth is deferred. Deferred initialization would instead fill
        // the ledger-data queue before any timeout callback could be admitted.
        for suffix in 1..=6u8 {
            registry.acquire(
                Uint256::from_array([suffix; 32]),
                u32::from(suffix),
                AcquireReason::Consensus,
            );
        }

        let lifecycle = registry.lifecycle_snapshot();
        assert_eq!(lifecycle.initialization_jobs, 6);
        assert_eq!(worker_pool.snapshot().queued_jobs, 5);
        assert_eq!(lifecycle.timeout_queue_rejected, 1);
        registry.stop();
    }

    #[test]
    fn acquisition_lifecycle_snapshot_exposes_route_and_worker_boundaries() {
        let counters = AcquisitionLifecycleCounters::default();
        assert_eq!(counters.snapshot(), AcquisitionLifecycleSnapshot::default());

        counters.acquisition_starts.fetch_add(1, Ordering::Relaxed);
        counters.wire_ledger_data.fetch_add(2, Ordering::Relaxed);
        counters.route_attempts.fetch_add(3, Ordering::Relaxed);
        counters.route_accepted.fetch_add(1, Ordering::Relaxed);
        counters.route_misses.fetch_add(1, Ordering::Relaxed);
        counters
            .route_sequence_mismatch
            .fetch_add(1, Ordering::Relaxed);
        counters.data_jobs_submitted.fetch_add(1, Ordering::Relaxed);
        counters.data_jobs_started.fetch_add(1, Ordering::Relaxed);
        counters.packet_steps.fetch_add(4, Ordering::Relaxed);
        counters.terminal_completed.fetch_add(1, Ordering::Relaxed);

        assert_eq!(
            counters.snapshot(),
            AcquisitionLifecycleSnapshot {
                acquisition_starts: 1,
                wire_ledger_data: 2,
                route_attempts: 3,
                route_accepted: 1,
                route_misses: 1,
                route_sequence_mismatch: 1,
                data_jobs_submitted: 1,
                data_jobs_started: 1,
                packet_steps: 4,
                terminal_completed: 1,
                ..AcquisitionLifecycleSnapshot::default()
            }
        );
    }

    #[test]
    fn unknown_closed_ledger_sequence_accepts_authoritative_response_sequence() {
        assert!(response_sequence_matches_request(0, 105_847_104));
        assert!(response_sequence_matches_request(105_847_104, 0));
        assert!(response_sequence_matches_request(105_847_104, 105_847_104));
        assert!(!response_sequence_matches_request(105_847_103, 105_847_104));
    }

    #[test]
    fn stale_inbound_acquisitions_match_rippleds_one_minute_sweep() {
        assert_eq!(SWEEP_IDLE_TIMEOUT, Duration::from_secs(60));
    }

    #[test]
    fn recovery_lcl_diagnostic_reports_selected_candidate() {
        let preferred = Uint256::from_array([0xA5; 32]);
        let candidate = Uint256::from_array([0xB6; 32]);
        let JsonValue::Object(value) = recovery_lcl_decision_json(Some(RecoveryLclDecision {
            recorded_at: Instant::now(),
            preferred_hash: preferred,
            candidate_hash: Some(candidate),
            candidate_seq: Some(123),
            source: "completed_recovery".to_owned(),
            decision: "installed".to_owned(),
        })) else {
            panic!("recovery diagnostic must be an object");
        };

        assert_eq!(
            value.get("preferred_hash"),
            Some(&JsonValue::String(preferred.to_string()))
        );
        assert_eq!(
            value.get("candidate_hash"),
            Some(&JsonValue::String(candidate.to_string()))
        );
        assert_eq!(value.get("candidate_seq"), Some(&JsonValue::Unsigned(123)));
        assert_eq!(
            value.get("decision"),
            Some(&JsonValue::String("installed".to_owned()))
        );
    }

    #[test]
    fn failed_acquisition_enters_cooldown_without_waiting_for_sweep() {
        let hash = Uint256::from_array([0xF1; 32]);
        let mut inner = RegistryInner {
            entries: BTreeMap::new(),
            recent_failures: HashMap::new(),
            completed_ready: VecDeque::new(),
        };

        record_recent_failure(&mut inner, hash, None);

        assert!(
            inner
                .recent_failures
                .get(&hash)
                .is_some_and(|recorded_at| recorded_at.elapsed() < Duration::from_secs(1))
        );
    }

    #[test]
    fn repeated_failure_records_preserve_the_original_cooldown_time() {
        let hash = Uint256::from_array([0xF2; 32]);
        let first_failure = Instant::now() - Duration::from_secs(240);
        let mut inner = RegistryInner {
            entries: BTreeMap::new(),
            recent_failures: HashMap::new(),
            completed_ready: VecDeque::new(),
        };

        record_recent_failure_at(&mut inner, hash, None, first_failure);
        record_recent_failure_at(&mut inner, hash, None, Instant::now());

        assert_eq!(inner.recent_failures.get(&hash), Some(&first_failure));
    }

    #[test]
    fn delayed_failure_cannot_match_a_replacement_acquisition() {
        assert!(failure_matches_entry(None, 2));
        assert!(failure_matches_entry(Some(2), 2));
        assert!(!failure_matches_entry(Some(1), 2));
    }

    #[test]
    fn terminal_touch_refreshes_only_the_matching_acquisition() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, registry) = registry_with_manual_worker_pool(worker_pool);
        let hash = Uint256::from_array([0xF6; 32]);
        assert!(
            registry
                .acquire(hash, 0, AcquireReason::Consensus)
                .is_none()
        );

        let first_id = {
            let inner = registry.inner.lock().expect("registry lock");
            inner.entries.get(&hash).expect("first entry").id
        };
        registry.remove(&hash);
        assert!(
            registry
                .acquire(hash, 0, AcquireReason::Consensus)
                .is_none()
        );

        let before = Instant::now() - Duration::from_secs(30);
        let refreshed = Instant::now();
        {
            let mut inner = registry.inner.lock().expect("registry lock");
            let replacement = inner.entries.get_mut(&hash).expect("replacement entry");
            replacement.last_touched = before;
            let replacement_id = replacement.id;
            touch_terminal_entry_at(&mut inner, hash, first_id, refreshed);
            assert_eq!(
                inner
                    .entries
                    .get(&hash)
                    .expect("replacement entry")
                    .last_touched,
                before,
                "a delayed predecessor terminal callback must not refresh a replacement"
            );
            touch_terminal_entry_at(&mut inner, hash, replacement_id, refreshed);
            assert_eq!(
                inner
                    .entries
                    .get(&hash)
                    .expect("replacement entry")
                    .last_touched,
                refreshed,
                "the matching terminal callback must own the sweep timestamp"
            );
        }
        registry.stop();
    }

    #[test]
    fn swept_unacknowledged_completion_remains_resolver_visible() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, registry) = registry_with_manual_worker_pool(worker_pool);
        let mut ledger = Ledger::from_ledger_seq_and_close_time(44, 1_000, false);
        ledger.set_immutable(true);
        let ledger = Arc::new(ledger);
        let hash = *ledger.header().hash.as_uint256();
        assert!(
            registry
                .acquire(hash, 0, AcquireReason::Consensus)
                .is_none()
        );
        registry.on_complete(hash, Arc::clone(&ledger));
        {
            let mut inner = registry.inner.lock().expect("registry lock");
            inner
                .entries
                .get_mut(&hash)
                .expect("completed entry")
                .last_touched = Instant::now() - SWEEP_IDLE_TIMEOUT - Duration::from_secs(1);
        }

        registry.sweep();
        assert!(registry.contains(&hash));
        assert_eq!(registry.poll_results_bounded(1).len(), 1);
        registry.stop();
    }

    #[test]
    fn completion_ready_queue_bypasses_in_progress_registry_prefix() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, registry) = registry_with_manual_worker_pool(worker_pool);
        // These lower hashes would consume a scan budget before the completed
        // target in the former BTreeMap polling implementation.
        for fill in [0x01, 0x02, 0x03] {
            assert!(
                registry
                    .acquire(Uint256::from_array([fill; 32]), 0, AcquireReason::Consensus)
                    .is_none()
            );
        }

        let mut ledger = Ledger::from_ledger_seq_and_close_time(45, 1_000, false);
        ledger.set_immutable(true);
        let ledger = Arc::new(ledger);
        let hash = *ledger.header().hash.as_uint256();
        assert!(
            registry
                .acquire(hash, 0, AcquireReason::Consensus)
                .is_none()
        );
        registry.on_complete(hash, Arc::clone(&ledger));

        let first = registry.poll_results_bounded(1);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].0, hash);
        assert_eq!(
            first[0].1,
            registry
                .inner
                .lock()
                .expect("registry lock")
                .entries
                .get(&hash)
                .expect("completed entry")
                .id
        );

        // The retained handoff retries after a failed consumer turn, but it
        // is not duplicated within a single oversized poll.
        assert_eq!(registry.poll_results_bounded(8).len(), 1);
        registry.acknowledge_completed(&hash, first[0].1);
        assert!(registry.poll_results_bounded(1).is_empty());
        registry.stop();
    }

    #[test]
    fn stale_failed_completion_does_not_consume_next_ready_poll_slot() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, registry) = registry_with_manual_worker_pool(worker_pool);

        let stale_hash = Uint256::from_array([0xF4; 32]);
        assert!(
            registry
                .acquire(stale_hash, 0, AcquireReason::Consensus)
                .is_none()
        );
        let mut stale_ledger = Ledger::from_ledger_seq_and_close_time(46, 1_000, false);
        stale_ledger.set_immutable(true);
        registry.on_complete(stale_hash, Arc::new(stale_ledger));
        registry.on_failed(stale_hash);

        let mut live_ledger = Ledger::from_ledger_seq_and_close_time(47, 1_000, false);
        live_ledger.set_immutable(true);
        let live_ledger = Arc::new(live_ledger);
        let live_hash = *live_ledger.header().hash.as_uint256();
        assert!(
            registry
                .acquire(live_hash, 0, AcquireReason::Consensus)
                .is_none()
        );
        registry.on_complete(live_hash, Arc::clone(&live_ledger));

        let ready = registry.poll_results_bounded(1);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].0, live_hash);
        registry.acknowledge_completed(&live_hash, ready[0].1);
        assert!(registry.poll_results_bounded(1).is_empty());
        registry.stop();
    }

    #[test]
    fn acknowledged_completion_remains_acquirable_until_sweep() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, registry) = registry_with_manual_worker_pool(worker_pool);
        let hash = Uint256::from_array([0xF5; 32]);
        assert!(
            registry
                .acquire(hash, 0, AcquireReason::Consensus)
                .is_none()
        );

        let mut ledger = Ledger::from_ledger_seq_and_close_time(42, 1_000, false);
        ledger.set_immutable(true);
        let ledger = Arc::new(ledger);
        registry.on_complete(hash, Arc::clone(&ledger));
        let acquisition_id = registry
            .inner
            .lock()
            .expect("registry lock")
            .entries
            .get(&hash)
            .expect("completed entry")
            .id;
        registry.acknowledge_completed(&hash, acquisition_id);

        assert!(registry.contains(&hash));
        assert_eq!(
            registry
                .acquire(hash, 0, AcquireReason::Consensus)
                .expect("completed acquisition remains available until sweep")
                .header()
                .seq,
            ledger.header().seq
        );
        registry.stop();
    }

    #[test]
    fn consensus_acquire_ignores_recent_failure_cooldown() {
        let worker_pool = Arc::new(WorkerPool::new(0));
        let (_dir, registry) = registry_with_manual_worker_pool(worker_pool);
        let hash = Uint256::from_array([0xF4; 32]);
        registry
            .inner
            .lock()
            .expect("registry lock")
            .recent_failures
            .insert(hash, Instant::now());

        assert!(
            registry
                .acquire(hash, 0, AcquireReason::Consensus)
                .is_none(),
            "consensus must create the hash-only target even after a History cooldown"
        );
        assert!(registry.contains(&hash));
        registry.stop();
    }

    #[test]
    fn delayed_failure_from_a_swept_acquisition_records_hash_cooldown() {
        let hash = Uint256::from_array([0xF3; 32]);
        let mut inner = RegistryInner {
            entries: BTreeMap::new(),
            recent_failures: HashMap::new(),
            completed_ready: VecDeque::new(),
        };

        record_recent_failure_at(&mut inner, hash, Some(17), Instant::now());

        assert!(
            inner.recent_failures.contains_key(&hash),
            "rippled AcqDone records the hash-wide cooldown even after the acquisition was swept"
        );
    }

    #[test]
    fn fetch_info_handles_zero_lookup_and_worker_counts_without_panicking() {
        let snapshot = AcquisitionSnapshot {
            age_ms: 1,
            header_after_ms: None,
            seq: 0,
            have_header: false,
            have_state: false,
            have_transactions: false,
            timeouts: 0,
            packets: 0,
            useful_packets: 0,
            useful_nodes: 0,
            state_packets: 0,
            state_useful_nodes: 0,
            state_duplicate_nodes: 0,
            malformed_packets: 0,
            state_scan_runs: 0,
            state_missing_nodes: 0,
            tx_missing_nodes: 0,
            state_scan_us: 0,
            state_scan_branch_steps: 0,
            state_scan_missing_nodes_recorded: 0,
            state_scan_positive_progress_slices: 0,
            state_scan_branch_budget_yields: 0,
            state_scan_deferred_read_budget_yields: 0,
            state_scan_deferred_read_resume_yields: 0,
            state_scan_missing_node_limit_yields: 0,
            state_scan_completed_slices: 0,
            state_scan_last_yield: "none",
            state_scan_last_branch_steps: 0,
            state_scan_last_deferred_reads: 0,
            state_scan_last_deferred_resumes: 0,
            state_scan_last_missing_nodes: 0,
            state_scan_branches_seen: 0,
            state_scan_duplicate_missing_hashes: 0,
            state_scan_full_below_hits: 0,
            state_scan_loaded_or_cached_children: 0,
            state_scan_pending_reads: 0,
            state_scan_read_slot_full: 0,
            state_scan_read_admission_accepted: 0,
            state_scan_read_admission_deferred: 0,
            state_scan_read_admission_attached: 0,
            state_scan_read_broker_rejected: 0,
            state_scan_max_pending_reads: 0,
            state_scan_pending_hits: 0,
            state_scan_pending_misses: 0,
            state_scan_deferred_resumes: 0,
            state_scan_yields: 0,
            state_scan_continuations: 0,
            timeout_dispatches: 0,
            state_scan_max_buffered_packets: 0,
            data_drain_runs: 0,
            data_drain_us: 0,
            data_drain_max_us: 0,
            data_drain_max_packets: 0,
            tx_scan_us: 0,
            worker_jobs: 0,
            worker_queue_wait_us: 7,
            node_store_fetch_hits: 0,
            node_store_fetch_misses: 0,
            tracked_peers: 0,
            buffered_packets: 0,
            buffered_packets_high_water: 0,
            mailbox_bytes: 0,
            mailbox_bytes_high_water: 0,
            mailbox_events: 0,
            stale_events: 0,
            overload_rejections: 0,
            active_plan_id: None,
            active_plan_kind: None,
            plan_pending_hashes: 0,
            plan_pending_edges: 0,
            broker_queued_keys: 0,
            broker_in_flight_keys: 0,
            mailbox_token: "idle",
            scan_continuation_pending: false,
            pending_admitted_timeouts: 0,
            has_active_packet: false,
        };

        let JsonValue::Object(values) = acquisition_snapshot_json(
            Uint256::from_u64(1),
            0,
            AcquireReason::Consensus,
            0,
            false,
            false,
            snapshot,
        ) else {
            panic!("acquisition diagnostics must be an object");
        };

        assert_eq!(values.get("state_packets"), Some(&JsonValue::Unsigned(0)));
        assert_eq!(
            values.get("state_useful_nodes"),
            Some(&JsonValue::Unsigned(0))
        );
        assert_eq!(
            values.get("state_duplicate_nodes"),
            Some(&JsonValue::Unsigned(0))
        );
        assert_eq!(values.get("state_scan_runs"), Some(&JsonValue::Unsigned(0)));
        assert_eq!(
            values.get("state_scan_pending_reads"),
            Some(&JsonValue::Unsigned(0))
        );
        assert_eq!(values.get("data_drain_runs"), Some(&JsonValue::Unsigned(0)));
        assert_eq!(
            values.get("node_store_lookup_hit_rate_ppm"),
            Some(&JsonValue::Null)
        );
        assert_eq!(
            values.get("average_worker_queue_wait_us"),
            Some(&JsonValue::Null)
        );
    }
}
