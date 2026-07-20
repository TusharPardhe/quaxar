//! Global inbound ledger registry — matching rippled's InboundLedgersImp.
//!
//! ONE global registry: HashMap<Uint256, Entry>. A single Mutex protects
//! the map. Each entry holds an Arc<AcquisitionState> for the per-ledger
//! state machine.

use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::{KeyCache, MonotonicClock};
use ledger::{FetchPackCache, InboundLedgerPacket, Ledger};
use overlay::Peer;
use shamap::family::{FullBelowCache, FullBelowCacheImpl};
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::runtime::overlay_runtime::AppOverlayRuntime;
use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

use super::acquisition::{
    AcquisitionBuilder, AcquisitionState, NodeStoreWriteMsg, PendingNodeStoreObject,
    RunDataLimiter,
};
use super::worker_pool::WorkerPool;

// ─── Constants ───────────────────────────────────────────────────────────────

/// How long a failed hash stays in recent_failures (prevents retry storms).
const FAILURE_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// Entries idle longer than this are swept.
const SWEEP_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Maximum concurrent in-progress acquisitions.
const MAX_CONCURRENT: usize = 8;

/// Timer tick interval for submitting timer jobs to all active acquisitions.
const TIMER_TICK_INTERVAL: Duration = Duration::from_secs(1);

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

// ─── Entry ───────────────────────────────────────────────────────────────────

struct Entry {
    seq: u32,
    #[allow(dead_code)]
    reason: AcquireReason,
    state: Arc<AcquisitionState>,
    last_touched: Instant,
    #[allow(dead_code)]
    started_at: Instant,
    completed_ledger: Option<Arc<Ledger>>,
    failed: bool,
}

// ─── RegistryInner ───────────────────────────────────────────────────────────

struct RegistryInner {
    entries: HashMap<Uint256, Entry>,
    recent_failures: HashMap<Uint256, Instant>,
}

// ─── InboundLedgers ──────────────────────────────────────────────────────────

/// Thread-safe global service for inbound ledger acquisition.
///
/// Matches rippled's InboundLedgers: one entry per hash, touch-on-access,
/// sweep idle entries, route peer responses, fixed worker pool.
pub struct InboundLedgers {
    inner: Mutex<RegistryInner>,
    worker_pool: Arc<WorkerPool>,
    // Shared resources for creating acquisitions
    node_store: Arc<RwLock<Option<SHAMapStoreNodeStore>>>,
    tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    full_below: Arc<FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>>,
    fetch_pack: Arc<FetchPackCache>,
    write_tx: Arc<RwLock<Option<Sender<NodeStoreWriteMsg>>>>,
    pending_writes: Arc<RwLock<Option<Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>>>>,
    run_data_limiter: Arc<RunDataLimiter>,
    shared_stored: Arc<KeyCache<Uint256>>,
    overlay_rt: Arc<RwLock<Option<Arc<AppOverlayRuntime>>>>,
    completed_ledgers_tx: Sender<Arc<Ledger>>,
    stopping: AtomicBool,
    /// Timer thread handle.
    _timer_handle: Mutex<Option<JoinHandle<()>>>,
    /// Stop flag for timer thread.
    timer_stop: Arc<AtomicBool>,
}

impl InboundLedgers {
    /// Create a new InboundLedgers service.
    pub fn new(
        tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
        full_below: Arc<FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>>,
        fetch_pack: Arc<FetchPackCache>,
        run_data_limiter: Arc<RunDataLimiter>,
        shared_stored: Arc<KeyCache<Uint256>>,
        completed_ledgers_tx: Sender<Arc<Ledger>>,
    ) -> Self {
        let timer_stop = Arc::new(AtomicBool::new(false));
        let pool = Arc::new(WorkerPool::new(MAX_CONCURRENT));

        // Spawn timer thread: periodically submits tick jobs for all active
        // acquisitions (drives re-requests for stalled fetches).
        let timer_stop_clone = Arc::clone(&timer_stop);
        let timer_pool = Arc::clone(&pool);
        let timer_handle = thread::Builder::new()
            .name("xrpld-acq-timer".to_owned())
            .spawn(move || {
                while !timer_stop_clone.load(Ordering::Acquire) {
                    thread::sleep(TIMER_TICK_INTERVAL);
                    if timer_stop_clone.load(Ordering::Acquire) {
                        break;
                    }
                    // Collect active states via the pool queue trick:
                    // We can't access InboundLedgers inner from here, so we
                    // rely on each AcquisitionState having a reference to the
                    // pool queue. The timer just needs to wake the pool.
                    // In practice, timer ticks work because acquire_inner
                    // already started the first tick and process_acquisition_tick
                    // re-submits if data was processed. For truly idle
                    // acquisitions, the timer nudges via condvar wake.
                    let (_, cvar) = &*timer_pool.queue();
                    cvar.notify_all();
                }
            })
            .expect("timer thread");

        Self {
            inner: Mutex::new(RegistryInner {
                entries: HashMap::new(),
                recent_failures: HashMap::new(),
            }),
            worker_pool: pool,
            node_store: Arc::new(RwLock::new(None)),
            tree_cache,
            full_below,
            fetch_pack,
            write_tx: Arc::new(RwLock::new(None)),
            pending_writes: Arc::new(RwLock::new(None)),
            run_data_limiter,
            shared_stored,
            overlay_rt: Arc::new(RwLock::new(None)),
            completed_ledgers_tx,
            stopping: AtomicBool::new(false),
            _timer_handle: Mutex::new(Some(timer_handle)),
            timer_stop,
        }
    }

    // ─── Configuration setters (called during app startup) ───────────────

    pub fn set_overlay_rt(&self, rt: Arc<AppOverlayRuntime>) {
        let mut guard = self.overlay_rt.write().expect("overlay_rt write");
        *guard = Some(rt);
    }

    pub fn set_node_store(&self, ns: SHAMapStoreNodeStore) {
        let mut guard = self.node_store.write().expect("node_store write");
        *guard = Some(ns);
    }

    pub fn set_write_tx(&self, tx: Sender<NodeStoreWriteMsg>) {
        let mut guard = self.write_tx.write().expect("write_tx write");
        *guard = Some(tx);
    }

    pub fn set_pending_writes(
        &self,
        pending: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
    ) {
        let mut guard = self.pending_writes.write().expect("pending_writes write");
        *guard = Some(pending);
    }

    // ─── Core API ────────────────────────────────────────────────────────

    /// Acquire a ledger by hash. Returns immediately if already complete.
    /// If not tracked, starts a new acquisition. If in-progress, touches
    /// the entry and returns None.
    ///
    /// Matches rippled's `InboundLedgers::acquire()`.
    pub fn acquire(
        &self,
        hash: Uint256,
        seq: u32,
        reason: AcquireReason,
    ) -> Option<Arc<Ledger>> {
        if hash.is_zero() {
            tracing::warn!(target: "inbound_ledger", "acquire: REJECTED zero hash");
            return None;
        }
        if self.stopping.load(Ordering::Acquire) {
            tracing::warn!(target: "inbound_ledger", %hash, "acquire: REJECTED stopping");
            return None;
        }

        let mut inner = self.inner.lock().expect("inbound_ledgers lock");

        // Check recent failures (5-min cooldown)
        if let Some(failed_at) = inner.recent_failures.get(&hash) {
            if failed_at.elapsed() < FAILURE_COOLDOWN {
                tracing::info!(target: "inbound_ledger", %hash, seq, "acquire: REJECTED recent failure");
                return None;
            }
        }
        // Prune expired failures while we're here
        inner
            .recent_failures
            .retain(|_, t| t.elapsed() < FAILURE_COOLDOWN);

        // Already tracked — touch and return result if complete
        if let Some(entry) = inner.entries.get_mut(&hash) {
            entry.last_touched = Instant::now();
            if entry.failed {
                tracing::debug!(target: "inbound_ledger", %hash, seq, "acquire: already tracked (failed)");
                return None;
            }
            tracing::debug!(target: "inbound_ledger", %hash, seq, complete = entry.completed_ledger.is_some(), "acquire: already tracked");
            return entry.completed_ledger.clone();
        }

        // Bound concurrent acquisitions
        let in_progress_count = inner
            .entries
            .values()
            .filter(|e| !e.failed && e.completed_ledger.is_none())
            .count();
        if in_progress_count >= MAX_CONCURRENT {
            // During cold start (all entries have seq=0), evict the OLDEST
            // entry to make room for the latest network hash. Older hashes
            // become stale as the network moves forward.
            // With known seqs, only evict if new seq is higher priority.
            let oldest_entry = inner
                .entries
                .iter()
                .filter(|(_, e)| !e.failed && e.completed_ledger.is_none())
                .min_by_key(|(_, e)| e.started_at);
            let all_seq_zero = inner
                .entries
                .values()
                .filter(|e| !e.failed && e.completed_ledger.is_none())
                .all(|e| e.seq == 0);
            let should_evict = if all_seq_zero && seq == 0 {
                // Cold start: evict oldest to chase latest network hash
                true
            } else {
                match oldest_entry {
                    Some((_, e)) if seq > 0 && e.seq > 0 && seq > e.seq => true,
                    _ => false,
                }
            };
            if !should_evict {
                tracing::info!(target: "inbound_ledger", %hash, seq, in_progress_count, "acquire: REJECTED at capacity");
                return None;
            }
            if let Some((evict_hash, _)) = oldest_entry {
                let evict_hash = *evict_hash;
                if let Some(evicted) = inner.entries.remove(&evict_hash) {
                    evicted.state.stopped.store(true, Ordering::Release);
                    tracing::debug!(target: "inbound_ledger",
                        evicted_seq = evicted.seq,
                        new_seq = seq,
                        "Evicting oldest acquisition for latest network hash"
                    );
                }
            }
        }

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
        let wt = {
            let guard = self.write_tx.read().expect("write_tx read");
            match guard.as_ref() {
                Some(tx) => tx.clone(),
                None => {
                    tracing::warn!(target: "inbound_ledger", %hash, seq, "acquire: REJECTED write_tx not attached");
                    return None;
                }
            }
        };
        let pending = {
            let guard = self.pending_writes.read().expect("pending_writes read");
            match guard.as_ref() {
                Some(p) => Arc::clone(p),
                None => {
                    tracing::warn!(target: "inbound_ledger", %hash, seq, "acquire: REJECTED pending_writes not attached");
                    return None;
                }
            }
        };

        // Get initial peers from overlay
        let initial_peers: Vec<Arc<dyn Peer>> = {
            let guard = self.overlay_rt.read().expect("overlay_rt read");
            if let Some(overlay_rt) = guard.as_ref() {
                use overlay::Overlay as _;
                overlay_rt.overlay().active_peers()
            } else {
                Vec::new()
            }
        };

        let full_below_gen = self.full_below.generation().wrapping_add(1);

        // Build the acquisition state
        let acq_state = AcquisitionBuilder {
            hash: SHAMapHash::new(hash),
            seq,
            node_store: ns,
            write_tx: wt,
            pending_writes: pending,
            tree_cache: Arc::clone(&self.tree_cache),
            fetch_pack: Arc::clone(&self.fetch_pack),
            run_data_limiter: Arc::clone(&self.run_data_limiter),
            shared_stored: Arc::clone(&self.shared_stored),
            store_tx: self.completed_ledgers_tx.clone(),
            full_below_generation: full_below_gen,
            work_pool: self.worker_pool.queue(),
            initial_peers,
        }
        .build();

        let now = Instant::now();
        inner.entries.insert(
            hash,
            Entry {
                seq,
                reason,
                state: Arc::clone(&acq_state),
                last_touched: now,
                started_at: now,
                completed_ledger: None,
                failed: false,
            },
        );

        tracing::info!(target: "inbound_ledger", seq, %hash, "Acquisition started — new entry created");

        // Submit the first tick to kick off acquisition
        acq_state.submit_tick();

        None
    }

    /// Fire-and-forget acquire (for consensus/validation callers).
    pub fn acquire_async(&self, hash: Uint256, seq: u32, reason: AcquireReason) {
        let _ = self.acquire(hash, seq, reason);
    }

    /// Route a TMLedgerData response to the correct acquisition.
    pub fn route_response(&self, hash: &Uint256, peer_id: u64, packet: InboundLedgerPacket) {
        let state = {
            let inner = self.inner.lock().expect("inbound_ledgers lock");
            inner.entries.get(hash).map(|e| Arc::clone(&e.state))
        };
        if let Some(state) = state {
            {
                let mut buf = state.data_buffer.lock().expect("data_buffer push lock");
                buf.push((peer_id, packet));
            }
            state.submit_tick();
        }
    }

    /// Remove entries idle for >60s. Matches rippled's sweep().
    pub fn sweep(&self) {
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        let mut to_remove = Vec::new();

        for (hash, entry) in &inner.entries {
            if entry.completed_ledger.is_some() || entry.failed {
                // Completed/failed entries swept after idle timeout
                if now.duration_since(entry.last_touched) > SWEEP_IDLE_TIMEOUT {
                    to_remove.push(*hash);
                }
            } else {
                // In-progress: check if acquisition completed itself
                if entry.state.completed.load(Ordering::Acquire) {
                    to_remove.push(*hash);
                } else if now.duration_since(entry.last_touched) > SWEEP_IDLE_TIMEOUT {
                    to_remove.push(*hash);
                }
            }
        }

        for hash in &to_remove {
            if let Some(entry) = inner.entries.remove(hash) {
                entry.state.stopped.store(true, Ordering::Release);
            }
        }

        // Record swept entries as recent failures
        for hash in &to_remove {
            inner.recent_failures.insert(*hash, now);
        }

        // Prune expired failures
        inner
            .recent_failures
            .retain(|_, t| t.elapsed() < FAILURE_COOLDOWN);

        if !to_remove.is_empty() {
            tracing::debug!(target: "inbound_ledger", swept = to_remove.len(), "Sweep");
        }
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
        if let Some(entry) = inner.entries.get_mut(&hash) {
            entry.completed_ledger = Some(ledger);
            entry.last_touched = Instant::now();
        }
    }

    /// Notify that a ledger acquisition failed.
    pub fn on_failed(&self, hash: Uint256) {
        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        inner.recent_failures.insert(hash, Instant::now());
        if let Some(entry) = inner.entries.get_mut(&hash) {
            entry.failed = true;
            entry.state.stopped.store(true, Ordering::Release);
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
            state.submit_tick();
        }
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
            state.submit_tick();
        }
    }

    /// Remove a specific entry.
    pub fn remove(&self, hash: &Uint256) {
        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        if let Some(entry) = inner.entries.remove(hash) {
            entry.state.stopped.store(true, Ordering::Release);
        }
    }

    /// Stop all acquisitions and shut down the worker pool.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        self.timer_stop.store(true, Ordering::Release);

        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        for (_, entry) in inner.entries.drain() {
            entry.state.stopped.store(true, Ordering::Release);
        }
        inner.recent_failures.clear();
        drop(inner);

        self.worker_pool.stop();
    }

    // ─── Catchup loop compatibility API ──────────────────────────────────

    /// Poll for completed acquisitions. Returns `(hash, ledger, skip_state)` tuples
    /// for all entries whose underlying acquisition has finished. Removes those
    /// entries from the registry.
    ///
    /// This is the catchup loop's primary mechanism for consuming results.
    pub fn poll_results(&self) -> Vec<(Uint256, Ledger, bool)> {
        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        let mut completed = Vec::new();
        let mut failed_hashes = Vec::new();
        let mut done_hashes = Vec::new();

        for (hash, entry) in inner.entries.iter_mut() {
            if entry.completed_ledger.is_some() {
                // Already extracted — skip
                continue;
            }
            if entry.failed {
                continue;
            }
            if entry.state.completed.load(Ordering::Acquire) {
                // Acquisition finished — extract ledger from mutable state
                let mutable = entry.state.mutable.lock().expect("acq mutable lock (poll)");
                if let Some(ledger) = mutable.inbound.ledger().cloned() {
                    completed.push((*hash, ledger, false));
                    done_hashes.push(*hash);
                } else {
                    failed_hashes.push(*hash);
                }
            } else if entry.state.stopped.load(Ordering::Acquire) {
                failed_hashes.push(*hash);
            }
        }

        // Remove completed entries
        for hash in &done_hashes {
            if let Some(entry) = inner.entries.remove(hash) {
                entry.state.stopped.store(true, Ordering::Release);
            }
        }

        // Mark failed entries
        for hash in &failed_hashes {
            inner.recent_failures.insert(*hash, Instant::now());
            if let Some(entry) = inner.entries.remove(hash) {
                entry.state.stopped.store(true, Ordering::Release);
            }
        }

        completed
    }

    /// Check if a specific hash is currently in-progress (not completed, not failed).
    pub fn is_in_progress(&self, hash: &Uint256) -> bool {
        let inner = self.inner.lock().expect("inbound_ledgers lock");
        inner.entries.get(hash).is_some_and(|e| {
            !e.failed
                && e.completed_ledger.is_none()
                && !e.state.completed.load(Ordering::Acquire)
        })
    }

    /// Remove non-in-progress entries with seq below `min_seq`.
    /// Returns number of entries removed.
    pub fn remove_in_progress_below_seq(&self, min_seq: u32) -> usize {
        if min_seq <= 1 {
            return 0;
        }
        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        let stale: Vec<Uint256> = inner
            .entries
            .iter()
            .filter(|(_, entry)| {
                (entry.completed_ledger.is_some() || entry.failed)
                    && entry.seq > 1
                    && entry.seq < min_seq
            })
            .map(|(hash, _)| *hash)
            .collect();
        let count = stale.len();
        for hash in stale {
            if let Some(entry) = inner.entries.remove(&hash) {
                entry.state.stopped.store(true, Ordering::Release);
            }
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

    /// Check if any tracked entry has the given seq or contains the hash.
    /// Used by history planner to avoid duplicate acquires.
    pub fn has_entry_for_seq_or_hash(&self, seq: u32, hash: &Uint256) -> bool {
        let inner = self.inner.lock().expect("inbound_ledgers lock");
        inner.entries.contains_key(hash)
            || inner.entries.values().any(|e| e.seq == seq)
    }

    /// Remove stale in-progress acquisitions that have had no progress.
    /// Used during cold bootstrap to free slots for new targets.
    /// Returns the number of entries removed.
    pub fn remove_stale_no_progress(&self, idle_timeout: Duration) -> Vec<(Uint256, u32)> {
        let now = Instant::now();
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
        for (hash, _) in &stale {
            if let Some(entry) = inner.entries.remove(hash) {
                entry.state.stopped.store(true, Ordering::Release);
            }
        }
        stale
    }

    /// Look up a hash for a target sequence from completed (but not yet polled)
    /// acquisitions' ledger skip lists.
    pub fn hash_for_seq_from_completed(
        &self,
        target_seq: u32,
    ) -> Option<basics::sha_map_hash::SHAMapHash> {
        let inner = self.inner.lock().expect("inbound_ledgers lock");
        // Find completed entries with seq >= target_seq
        let mut best: Option<(u32, basics::sha_map_hash::SHAMapHash)> = None;
        for entry in inner.entries.values() {
            if entry.failed {
                continue;
            }
            let is_complete = entry.completed_ledger.is_some()
                || entry.state.completed.load(Ordering::Acquire);
            if !is_complete || entry.seq < target_seq {
                continue;
            }
            // Try to get hash from the entry's ledger
            if let Some(ledger) = &entry.completed_ledger {
                if let Some(hash) = ledger
                    .hash_of_seq(target_seq, &ledger::NullLedgerJournal)
                    .filter(|h| !h.is_zero())
                {
                    if best.is_none() || entry.seq < best.unwrap().0 {
                        best = Some((entry.seq, hash));
                    }
                }
            } else {
                // Try from mutable state
                let mutable = entry.state.mutable.lock().expect("acq mutable (hash lookup)");
                if let Some(ledger) = mutable.inbound.ledger() {
                    if ledger.header().seq >= target_seq {
                        if let Some(hash) = ledger
                            .hash_of_seq(target_seq, &ledger::NullLedgerJournal)
                            .filter(|h| !h.is_zero())
                        {
                            if best.is_none() || entry.seq < best.unwrap().0 {
                                best = Some((entry.seq, hash));
                            }
                        }
                    }
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
        let inner = self.inner.lock().expect("inbound_ledgers lock");
        let mut best: Option<(u32, basics::sha_map_hash::SHAMapHash)> = None;
        for entry in inner.entries.values() {
            if entry.failed {
                continue;
            }
            let is_complete = entry.completed_ledger.is_some()
                || entry.state.completed.load(Ordering::Acquire);
            if !is_complete || entry.seq < target_seq {
                continue;
            }
            // Use candidate_ledger_for_seq logic inline: round up to next 256 boundary
            let candidate_seq = target_seq.saturating_add(255) & !255;
            if candidate_seq <= target_seq {
                continue;
            }
            if let Some(ledger) = &entry.completed_ledger {
                if let Some(hash) = ledger
                    .hash_of_seq(candidate_seq, &ledger::NullLedgerJournal)
                    .filter(|h| !h.is_zero())
                {
                    if best.is_none() || entry.seq < best.unwrap().0 {
                        best = Some((candidate_seq, hash));
                    }
                }
            } else {
                let mutable = entry.state.mutable.lock().expect("acq mutable (candidate)");
                if let Some(ledger) = mutable.inbound.ledger() {
                    if ledger.header().seq >= target_seq {
                        if let Some(hash) = ledger
                            .hash_of_seq(candidate_seq, &ledger::NullLedgerJournal)
                            .filter(|h| !h.is_zero())
                        {
                            if best.is_none() || entry.seq < best.unwrap().0 {
                                best = Some((candidate_seq, hash));
                            }
                        }
                    }
                }
            }
        }
        best
    }
}

impl std::fmt::Debug for InboundLedgers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboundLedgers").finish_non_exhaustive()
    }
}
