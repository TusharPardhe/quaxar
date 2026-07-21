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
use std::time::{Duration, Instant};

use crate::runtime::overlay_runtime::AppOverlayRuntime;
use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

use super::acquisition::{AcquisitionBuilder, AcquisitionState};
use super::worker_pool::WorkerPool;

// ─── Constants ───────────────────────────────────────────────────────────────

/// How long a failed hash stays in recent_failures (prevents retry storms).
const FAILURE_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// Entries idle longer than this are swept.
const SWEEP_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Rust worker count for the `JtLedgerData`-equivalent queue. This does not
/// limit the number of tracked acquisitions.
const WORKER_COUNT: usize = 64;

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
    inner: Arc<Mutex<RegistryInner>>,
    worker_pool: Arc<WorkerPool>,
    // Shared resources for creating acquisitions
    node_store: Arc<RwLock<Option<SHAMapStoreNodeStore>>>,
    tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    full_below: Arc<FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>>,
    fetch_pack: Arc<FetchPackCache>,
    shared_stored: Arc<KeyCache<Uint256>>,
    overlay_rt: Arc<RwLock<Option<Arc<AppOverlayRuntime>>>>,
    completed_ledgers_tx: Sender<Arc<Ledger>>,
    stopping: AtomicBool,
}

impl InboundLedgers {
    /// Create a new InboundLedgers service.
    pub fn new(
        tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
        full_below: Arc<FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>>,
        fetch_pack: Arc<FetchPackCache>,
        shared_stored: Arc<KeyCache<Uint256>>,
        completed_ledgers_tx: Sender<Arc<Ledger>>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RegistryInner {
                entries: HashMap::new(),
                recent_failures: HashMap::new(),
            })),
            worker_pool: Arc::new(WorkerPool::new(WORKER_COUNT)),
            node_store: Arc::new(RwLock::new(None)),
            tree_cache,
            full_below,
            fetch_pack,
            shared_stored,
            overlay_rt: Arc::new(RwLock::new(None)),
            completed_ledgers_tx,
            stopping: AtomicBool::new(false),
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

        // Existing acquisition: update an unknown sequence, retain it for the
        // sweep window, and return the ledger once it is complete.
        if let Some(entry) = inner.entries.get_mut(&hash) {
            entry.last_touched = Instant::now();
            if entry.seq == 0 && seq != 0 {
                entry.seq = seq;
                entry.state.update_seq(seq);
            }
            if entry.failed || entry.state.failed.load(Ordering::Acquire) {
                inner.recent_failures.insert(hash, Instant::now());
                return None;
            }
            if entry.state.completed.load(Ordering::Acquire) {
                return entry.state.completed_ledger();
            }
            return entry.completed_ledger.clone();
        }

        // rippled has NO capacity limit on InboundLedgers — it creates entries
        // for every unique hash requested. Memory is bounded by the 60s sweep
        // (entries go idle when no longer touched). The only backpressure is
        // indirect via job queue limits and peer capacity.
        //
        // Previously quaxar had a MAX_CONCURRENT=64 cap here which caused
        // permanent stalls: entries got keep-alived by the timer thread,
        // never went idle, filled the cap, and new consensus-requested
        // ledgers were rejected indefinitely.

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

        let acq_state = AcquisitionBuilder {
            hash: SHAMapHash::new(hash),
            seq,
            reason,
            node_store: ns,
            tree_cache: Arc::clone(&self.tree_cache),
            fetch_pack: Arc::clone(&self.fetch_pack),
            shared_stored: Arc::clone(&self.shared_stored),
            store_tx: self.completed_ledgers_tx.clone(),
            full_below_generation: full_below_gen,
            worker_pool: Arc::clone(&self.worker_pool),
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
        drop(inner);

        tracing::info!(target: "inbound_ledger", seq, %hash, "Acquisition started");
        acq_state.start();
        None
    }

    /// Fire-and-forget acquire (for consensus/validation callers).
    pub fn acquire_async(&self, hash: Uint256, seq: u32, reason: AcquireReason) {
        let _ = self.acquire(hash, seq, reason);
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
        let state = {
            let inner = self.inner.lock().expect("inbound_ledgers lock");
            let Some(entry) = inner.entries.get(hash) else {
                tracing::debug!(target: "inbound_ledger", %hash, peer_id, "route_response: registry miss");
                return false;
            };
            if let Some(response_seq) = response_seq
                && entry.seq != 0
                && response_seq != 0
                && entry.seq != response_seq
            {
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
            Arc::clone(&entry.state)
        };

        {
            let mut buf = state.data_buffer.lock().expect("data_buffer push lock");
            buf.push((peer_id, packet));
        }
        state.submit_data_job();
        tracing::debug!(target: "inbound_ledger", %hash, peer_id, "route_response: registry hit");
        true
    }

    /// Remove entries idle for more than one minute, matching
    /// `InboundLedgersImp::sweep`.
    pub fn sweep(&self) {
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        let mut to_remove = Vec::new();

        for (hash, entry) in &inner.entries {
            let failed = entry.failed || entry.state.failed.load(Ordering::Acquire);
            if failed || now.duration_since(entry.last_touched) > SWEEP_IDLE_TIMEOUT {
                to_remove.push((*hash, failed));
            }
        }

        for (hash, failed) in to_remove {
            if let Some(entry) = inner.entries.remove(&hash) {
                entry.state.stopped.store(true, Ordering::Release);
            }
            if failed {
                inner.recent_failures.insert(hash, now);
            }
        }
        inner
            .recent_failures
            .retain(|_, when| when.elapsed() < FAILURE_COOLDOWN);
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
        let mut inner = self.inner.lock().expect("inbound_ledgers lock");
        if let Some(entry) = inner.entries.remove(hash) {
            entry.state.stopped.store(true, Ordering::Release);
        }
    }

    /// Stop all acquisitions and shut down the worker pool.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Release);

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
            !e.failed && e.completed_ledger.is_none() && !e.state.completed.load(Ordering::Acquire)
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

    /// Check whether an in-progress acquisition has the given sequence or hash.
    /// Completed entries remain in the registry until its sweep but are already
    /// represented in LedgerHistory, so they must not block the next history
    /// predecessor request.
    pub fn has_entry_for_seq_or_hash(&self, seq: u32, hash: &Uint256) -> bool {
        let inner = self.inner.lock().expect("inbound_ledgers lock");
        inner.entries.iter().any(|(entry_hash, entry)| {
            !entry.failed
                && !entry.state.failed.load(Ordering::Acquire)
                && !entry.state.completed.load(Ordering::Acquire)
                && (*entry_hash == *hash || entry.seq == seq)
        })
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
            let is_complete =
                entry.completed_ledger.is_some() || entry.state.completed.load(Ordering::Acquire);
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
                let mutable = entry
                    .state
                    .mutable
                    .lock()
                    .expect("acq mutable (hash lookup)");
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
            let is_complete =
                entry.completed_ledger.is_some() || entry.state.completed.load(Ordering::Acquire);
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
