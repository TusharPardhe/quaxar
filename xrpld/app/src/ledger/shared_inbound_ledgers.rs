//! Shared inbound ledger acquisition service.
//!
//! Wraps the core acquisition logic (registry, worker spawning, dedup, sweep)
//! behind an `Arc`-friendly interface so both the main catchup loop and the
//! consensus driver can trigger full ledger fetches without shared mutable
//! access to a single owner struct.

use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::{KeyCache, MonotonicClock};
use ledger::{FetchPackCache, InboundLedgerPacket, Ledger};
use overlay::Peer;
use shamap::family::FullBelowCacheImpl;
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use crate::runtime::overlay_runtime::AppOverlayRuntime;
use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

/// Messages sent to acquisition worker threads.
pub enum AcqMsg {
    /// Raw TmLedgerData packet from a peer.
    LedgerData {
        peer_id: u64,
        packet: InboundLedgerPacket,
    },
    /// Shared fetch-pack cache was populated; re-check local missing nodes.
    FetchPackReady,
    /// Update the peer list for sending requests.
    Peers(Vec<Arc<dyn Peer>>),
    /// Shutdown.
    Stop,
}

/// Result sent back from the acquisition worker to the owner.
pub enum AcqResult {
    /// Ledger acquisition complete.
    Complete(Ledger),
    /// Acquisition failed permanently.
    Failed,
    /// Still in progress (periodic status).
    Progress { good_nodes: usize },
}

/// Shared registry of active acquisition channels, keyed by ledger hash.
pub type AcqRegistry = Arc<Mutex<HashMap<Uint256, Sender<AcqMsg>>>>;

/// Pending node-store write object used by acquisition workers.
#[derive(Debug, Clone)]
pub struct PendingNodeStoreObject {
    pub obj_type: nodestore::NodeObjectType,
    pub data: Vec<u8>,
    pub hash: Uint256,
}

impl PendingNodeStoreObject {
    pub fn shamap_type(&self) -> shamap::storage::NodeObjectType {
        match self.obj_type {
            nodestore::NodeObjectType::AccountNode => shamap::storage::NodeObjectType::AccountNode,
            nodestore::NodeObjectType::TransactionNode => {
                shamap::storage::NodeObjectType::TransactionNode
            }
            nodestore::NodeObjectType::Ledger => shamap::storage::NodeObjectType::Ledger,
            _ => shamap::storage::NodeObjectType::Unknown,
        }
    }
}

/// Messages for the background node-store writer thread.
pub enum NodeStoreWriteMsg {
    Write {
        obj_type: nodestore::NodeObjectType,
        data: Vec<u8>,
        hash: Uint256,
        seq: u32,
    },
    Flush(Sender<()>),
    Stop,
}

/// Spawn a dedicated background thread that flushes acquired SHAMap nodes
/// to the node store, draining `NodeStoreWriteMsg`s sent by acquisition
/// workers. Ported from `xrpld/main`'s own `spawn_nodestore_writer` (used
/// there for the standalone/normal catchup loop's `InboundLedgers`) so
/// `--start` mode's `SharedInboundLedgers` instance -- previously
/// constructed but never wired to a real node-store write path via
/// `set_write_tx`/`set_pending_writes` -- has the same real persistence
/// pipeline. Without this, `SharedInboundLedgers::acquire` early-returns
/// unconditionally (its `write_tx`/`pending_writes` guards are `None`),
/// silently no-opping every active-acquisition request `--start` mode's
/// `Consensus::checkLedger` -> `handleWrongLedger` -> `acquireLedger` path
/// needs to catch a node up to a ledger it doesn't have cached locally.
pub fn spawn_nodestore_writer(
    ns: crate::shamap::shamap_store_backend::SHAMapStoreNodeStore,
    pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
) -> (Sender<NodeStoreWriteMsg>, thread::JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::channel::<NodeStoreWriteMsg>();
    let handle = thread::Builder::new()
        .name("xrpld-nudb-writer".to_owned())
        .spawn(move || {
            let mut total_writes = 0u64;
            let mut last_log = Instant::now();
            let do_store = |ns: &crate::shamap::shamap_store_backend::SHAMapStoreNodeStore, obj_type, data, hash, seq| match ns {
                crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Single(db) => db.store(obj_type, data, hash, seq),
                crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Rotating(db) => db.store(obj_type, data, hash, seq),
            };
            let mut total_store_us = 0u64;
            loop {
                // Block waiting for first message
                let first = match rx.recv() {
                    Ok(NodeStoreWriteMsg::Write { obj_type, data, hash, seq }) => Some((obj_type, data, hash, seq)),
                    Ok(NodeStoreWriteMsg::Flush(ack)) => {
                        let _ = ack.send(());
                        None
                    }
                    Ok(NodeStoreWriteMsg::Stop) | Err(_) => return,
                };
                // Process the first write
                if let Some((obj_type, data, hash, seq)) = first {
                    let t = Instant::now();
                    do_store(&ns, obj_type, data, hash, seq);
                    pending_writes.lock().expect("pending node-store writes mutex").remove(&hash);
                    total_store_us += t.elapsed().as_micros() as u64;
                    total_writes += 1;
                }
                // Drain ALL queued writes without blocking
                loop {
                    match rx.try_recv() {
                        Ok(NodeStoreWriteMsg::Write { obj_type, data, hash, seq }) => {
                            let t = Instant::now();
                            do_store(&ns, obj_type, data, hash, seq);
                            pending_writes.lock().expect("pending node-store writes mutex").remove(&hash);
                            total_store_us += t.elapsed().as_micros() as u64;
                            total_writes += 1;
                        }
                        Ok(NodeStoreWriteMsg::Flush(ack)) => {
                            let _ = ack.send(());
                        }
                        Ok(NodeStoreWriteMsg::Stop) => return,
                        Err(_) => break,
                    }
                }
                if last_log.elapsed() >= std::time::Duration::from_secs(10) {
                    let avg_us = if total_writes > 0 { total_store_us / total_writes } else { 0 };
                    tracing::debug!(target: "nodestore", total_writes, avg_us, "NuDB writer status (start-mode)");
                    last_log = Instant::now();
                }
            }
        })
        .expect("nudb writer thread");
    (tx, handle)
}

/// Limits concurrent run_data processing to reduce cache mutex contention.
pub struct RunDataLimiter {
    state: Mutex<usize>,
    cv: std::sync::Condvar,
    max_concurrent: usize,
}

impl RunDataLimiter {
    pub fn new(max: usize) -> Self {
        Self {
            state: Mutex::new(0),
            cv: std::sync::Condvar::new(),
            max_concurrent: max,
        }
    }

    pub fn acquire(&self) {
        let mut count = self.state.lock().expect("run_data_limiter lock");
        while *count >= self.max_concurrent {
            count = self.cv.wait(count).expect("run_data_limiter wait");
        }
        *count += 1;
    }

    pub fn release(&self) {
        let mut count = self.state.lock().expect("run_data_limiter lock");
        *count -= 1;
        self.cv.notify_one();
    }
}

/// Reacquire interval for failed ledgers (reference kREACQUIRE_INTERVAL = 5 min).
const REACQUIRE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// Sweep timeout for completed entries (reference 1 minute after last action).
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
/// Timeout for stuck InProgress entries (reference ~180s with no progress).
const STUCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
/// Maximum number of concurrent in-progress ledger acquisitions. Bounds
/// resource usage when a node has diverged and is receiving a steady stream
/// of validations/proposals referencing many distinct unfamiliar ledger
/// hashes — without this cap each one would spawn its own persistent
/// worker thread with no upper limit. Matches the general spirit of
/// rippled's PeerSet-based request pacing, which naturally limits how many
/// ledgers are chased at once.
const MAX_CONCURRENT_ACQUISITIONS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryState {
    InProgress,
    Complete,
    #[allow(dead_code)]
    Failed,
}

struct InboundEntry {
    seq: u32,
    tx: Sender<AcqMsg>,
    started_at: Instant,
    last_touched: Instant,
    completed_at: Option<Instant>,
    state: EntryState,
}

struct Inner {
    entries: HashMap<Uint256, InboundEntry>,
    recent_failures: HashMap<Uint256, Instant>,
}

/// Thread-safe shared service for inbound ledger acquisition.
///
/// Accessible from multiple threads (main catchup loop, consensus driver,
/// bootstrap). Internally mutex-protected with fine-grained locking.
pub struct SharedInboundLedgers {
    inner: Mutex<Inner>,
    registry: AcqRegistry,
    node_store: Arc<std::sync::RwLock<Option<SHAMapStoreNodeStore>>>,
    tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    full_below: Arc<FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>>,
    fetch_pack: Arc<FetchPackCache>,
    write_tx: Arc<std::sync::RwLock<Option<Sender<NodeStoreWriteMsg>>>>,
    pending_writes: Arc<std::sync::RwLock<Option<Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>>>>,
    run_data_limiter: Arc<RunDataLimiter>,
    shared_stored: Arc<KeyCache<Uint256>>,
    overlay_rt: Arc<std::sync::RwLock<Option<Arc<AppOverlayRuntime>>>>,
    completed_ledgers_tx: Sender<Arc<Ledger>>,
}

impl SharedInboundLedgers {
    pub fn new(
        registry: AcqRegistry,
        tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
        full_below: Arc<FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>>,
        fetch_pack: Arc<FetchPackCache>,
        run_data_limiter: Arc<RunDataLimiter>,
        shared_stored: Arc<KeyCache<Uint256>>,
        completed_ledgers_tx: Sender<Arc<Ledger>>,
    ) -> Self {
        Self {
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                recent_failures: HashMap::new(),
            }),
            registry,
            node_store: Arc::new(std::sync::RwLock::new(None)),
            tree_cache,
            full_below,
            fetch_pack,
            write_tx: Arc::new(std::sync::RwLock::new(None)),
            pending_writes: Arc::new(std::sync::RwLock::new(None)),
            run_data_limiter,
            shared_stored,
            overlay_rt: Arc::new(std::sync::RwLock::new(None)),
            completed_ledgers_tx,
        }
    }

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

    /// Acquire a ledger by hash. Deduplicates requests. Spawns a worker if new.
    ///
    /// Called from the consensus-driver thread or the main catchup loop.
    /// Worker completion sends to `completed_ledgers_tx`.
    pub fn acquire(&self, hash: Uint256, seq: u32) {
        if hash.is_zero() {
            return;
        }

        let mut inner = self.inner.lock().expect("shared_inbound lock");

        // Check recent failures
        if let Some(failed_at) = inner.recent_failures.get(&hash) {
            if failed_at.elapsed() < REACQUIRE_INTERVAL {
                return;
            }
        }
        inner
            .recent_failures
            .retain(|_, t| t.elapsed() < REACQUIRE_INTERVAL);

        // Already tracked — touch and return
        if let Some(entry) = inner.entries.get_mut(&hash) {
            entry.last_touched = Instant::now();
            return;
        }

        // Bound concurrent in-progress acquisitions. Without this cap, a
        // node that has diverged from the network receives a steady stream
        // of validations/proposals referencing unfamiliar ledger hashes for
        // many different (often stale) sequences, each of which would
        // otherwise spawn its own persistent acquisition worker with no
        // upper bound — starving I/O and thread capacity for the sequences
        // that actually matter (the newest ones) and preventing the node
        // from ever catching up. When at capacity, evict the in-progress
        // entry with the lowest target sequence to make room for this one,
        // since more recent acquisitions are more likely to still be
        // relevant to where the network currently is.
        let in_progress_count = inner
            .entries
            .values()
            .filter(|e| e.state == EntryState::InProgress)
            .count();
        if in_progress_count >= MAX_CONCURRENT_ACQUISITIONS {
            let lowest_seq_hash = inner
                .entries
                .iter()
                .filter(|(_, e)| e.state == EntryState::InProgress)
                .min_by_key(|(_, e)| e.seq)
                .map(|(h, _)| *h);
            if let Some(evict_hash) = lowest_seq_hash {
                if let Some(evicted) = inner.entries.remove(&evict_hash) {
                    let _ = evicted.tx.send(AcqMsg::Stop);
                    tracing::debug!(target: "inbound_ledger",
                        evicted_seq = evicted.seq,
                        evicted_hash = %evict_hash,
                        new_seq = seq,
                        new_hash = %hash,
                        "Evicting lowest-seq acquisition to bound concurrency"
                    );
                }
                self.registry.lock().expect("acq registry").remove(&evict_hash);
            }
        }

        // Validate required resources
        let ns = {
            let guard = self.node_store.read().expect("node_store read");
            match guard.as_ref() {
                Some(ns) => ns.clone(),
                None => return,
            }
        };
        let wt = {
            let guard = self.write_tx.read().expect("write_tx read");
            match guard.as_ref() {
                Some(tx) => tx.clone(),
                None => return,
            }
        };
        let pending = {
            let guard = self.pending_writes.read().expect("pending_writes read");
            match guard.as_ref() {
                Some(p) => Arc::clone(p),
                None => return,
            }
        };

        let shamap_hash = SHAMapHash::new(hash);
        let (acq_tx, acq_rx) = std::sync::mpsc::channel::<AcqMsg>();
        let tc = Arc::clone(&self.tree_cache);
        let fb = Arc::clone(&self.full_below);
        let fp = Arc::clone(&self.fetch_pack);
        let rl = Arc::clone(&self.run_data_limiter);
        let ss = Arc::clone(&self.shared_stored);
        let store_tx = self.completed_ledgers_tx.clone();

        // Register in overlay router BEFORE spawning the worker thread to
        // avoid a race where rippled's TmLedgerData response arrives before
        // the registry entry exists (causing route_response to drop the data).
        self.registry
            .lock()
            .expect("acq registry")
            .insert(hash, acq_tx.clone());

        let _acq_handle = thread::Builder::new()
            .name("xrpld-acq-process".to_owned())
            .spawn(move || {
                run_acquisition_worker(
                    acq_rx, shamap_hash, seq, ns, tc, fb, fp, wt, pending, rl, ss, store_tx,
                );
            })
            .expect("acquisition thread should spawn");

        let now = Instant::now();

        // Send initial peers so the worker has them on first trigger
        {
            let guard = self.overlay_rt.read().expect("overlay_rt read");
            if let Some(overlay_rt) = guard.as_ref() {
                use overlay::Overlay as _;
                let peers = overlay_rt.overlay().active_peers();
                let _ = acq_tx.send(AcqMsg::Peers(peers));
            }
        }

        inner.entries.insert(
            hash,
            InboundEntry {
                seq,
                tx: acq_tx,
                started_at: now,
                last_touched: now,
                completed_at: None,
                state: EntryState::InProgress,
            },
        );

        tracing::debug!(target: "inbound_ledger", seq, hash = %shamap_hash, "Shared acquire started");
    }

    /// Route a TmLedgerData response to the correct acquisition worker.
    ///
    /// Called from the overlay I/O thread (ledger data router).
    pub fn route_response(&self, hash: &Uint256, peer_id: u64, packet: InboundLedgerPacket) {
        let guard = self.registry.lock().expect("acq registry lock");
        if let Some(tx) = guard.get(hash) {
            let _ = tx.send(AcqMsg::LedgerData { peer_id, packet });
        }
    }

    /// Check if already tracking this hash.
    pub fn contains(&self, hash: &Uint256) -> bool {
        let inner = self.inner.lock().expect("shared_inbound lock");
        inner.entries.contains_key(hash)
    }

    /// Sweep stale and completed entries.
    pub fn sweep(&self) {
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("shared_inbound lock");
        let mut to_remove = Vec::new();

        for (hash, entry) in &inner.entries {
            match entry.state {
                EntryState::InProgress => {
                    if now.duration_since(entry.started_at) > STUCK_TIMEOUT
                        && now.duration_since(entry.last_touched) > SWEEP_INTERVAL
                    {
                        to_remove.push(*hash);
                    }
                }
                EntryState::Complete | EntryState::Failed => {
                    let sweep_since = entry.completed_at.unwrap_or(entry.last_touched);
                    if now.duration_since(sweep_since) > SWEEP_INTERVAL {
                        to_remove.push(*hash);
                    }
                }
            }
        }

        for hash in &to_remove {
            if let Some(entry) = inner.entries.remove(hash) {
                let _ = entry.tx.send(AcqMsg::Stop);
                self.registry.lock().expect("acq registry").remove(hash);
            }
        }

        // Mark stuck entries as failed in recent_failures
        for hash in &to_remove {
            inner.recent_failures.insert(*hash, now);
        }

        inner
            .recent_failures
            .retain(|_, t| t.elapsed() < REACQUIRE_INTERVAL);

        let swept = to_remove.len();
        if swept > 0 {
            tracing::debug!(target: "inbound_ledger", swept, "Shared sweep");
        }
    }

    /// Send current peers to all active acquisition workers.
    pub fn send_peers(&self, peers: &[Arc<dyn Peer>]) {
        let inner = self.inner.lock().expect("shared_inbound lock");
        for entry in inner.entries.values() {
            if entry.state == EntryState::InProgress {
                let _ = entry.tx.send(AcqMsg::Peers(peers.to_vec()));
            }
        }
    }

    /// Mark an entry as completed (called when the completed_ledgers_tx receiver
    /// processes the finished ledger).
    pub fn mark_complete(&self, hash: &Uint256) {
        let mut inner = self.inner.lock().expect("shared_inbound lock");
        if let Some(entry) = inner.entries.get_mut(hash) {
            entry.state = EntryState::Complete;
            entry.completed_at = Some(Instant::now());
            entry.last_touched = Instant::now();
        }
    }

    /// Mark an entry as failed and record in recent failures.
    pub fn mark_failed(&self, hash: &Uint256) {
        let mut inner = self.inner.lock().expect("shared_inbound lock");
        inner.recent_failures.insert(*hash, Instant::now());
        if let Some(entry) = inner.entries.remove(hash) {
            let _ = entry.tx.send(AcqMsg::Stop);
            self.registry.lock().expect("acq registry").remove(hash);
        }
    }

    /// Remove a specific entry (e.g., after accepting a ledger).
    pub fn remove(&self, hash: &Uint256) {
        let mut inner = self.inner.lock().expect("shared_inbound lock");
        if let Some(entry) = inner.entries.remove(hash) {
            let _ = entry.tx.send(AcqMsg::Stop);
            self.registry.lock().expect("acq registry").remove(hash);
        }
    }

    /// Number of in-progress acquisitions.
    pub fn active_count(&self) -> usize {
        let inner = self.inner.lock().expect("shared_inbound lock");
        inner
            .entries
            .values()
            .filter(|e| e.state == EntryState::InProgress)
            .count()
    }

    /// Send a fetch-pack-ready signal to all in-progress workers.
    pub fn notify_fetch_pack_ready(&self) {
        let inner = self.inner.lock().expect("shared_inbound lock");
        for entry in inner.entries.values() {
            if entry.state == EntryState::InProgress {
                let _ = entry.tx.send(AcqMsg::FetchPackReady);
            }
        }
    }

    /// Stop all active acquisitions.
    pub fn stop(&self) {
        let mut inner = self.inner.lock().expect("shared_inbound lock");
        for (hash, entry) in inner.entries.drain() {
            let _ = entry.tx.send(AcqMsg::Stop);
            self.registry.lock().expect("acq registry").remove(&hash);
        }
        inner.recent_failures.clear();
    }
}

#[allow(clippy::too_many_arguments)]
fn run_acquisition_worker(
    rx: std::sync::mpsc::Receiver<AcqMsg>,
    hash: SHAMapHash,
    seq: u32,
    ns: SHAMapStoreNodeStore,
    shared_tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    shared_full_below: Arc<FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>>,
    shared_fetch_pack: Arc<FetchPackCache>,
    shared_write_tx: Sender<NodeStoreWriteMsg>,
    shared_pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
    run_data_limiter: Arc<RunDataLimiter>,
    shared_stored: Arc<KeyCache<Uint256>>,
    store_tx: Sender<Arc<Ledger>>,
) {
    use ledger::{
        FetchPackCache as _, FetchPackContainer, FetchPackStore, InboundLedgerJournal,
        InboundLedgerLocal, InboundLedgerReceivedPacket, InboundLedgerRequestTrigger,
        InboundLedgerStore, LedgerConfig,
    };
    use overlay::{Peer as _, PeerSet as _};
    use shamap::family::{NullMissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher};
    use std::time::{Duration, Instant};

    struct WorkerJournal;
    impl InboundLedgerJournal for WorkerJournal {
        fn trace(&self, _msg: &str) {}
        fn debug(&self, msg: &str) {
            tracing::debug!(target: "inbound_ledger", "{msg}");
        }
        fn warn(&self, msg: &str) {
            tracing::debug!(target: "inbound_ledger", "{msg}");
        }
        fn fatal(&self, msg: &str) {
            tracing::error!(target: "inbound_ledger", "{msg}");
        }
    }

    struct WorkerFetchPack {
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

    struct WorkerNodeFetcher {
        node_store: SHAMapStoreNodeStore,
        pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
    }
    impl SHAMapNodeFetcher for WorkerNodeFetcher {
        fn fetch_node_object(
            &self,
            hash: basics::sha_map_hash::SHAMapHash,
            ledger_seq: u32,
        ) -> Option<shamap::node_object::NodeObject> {
            if let Some(pending) = self
                .pending_writes
                .lock()
                .expect("pending writes lock")
                .get(hash.as_uint256())
                .cloned()
            {
                return Some(shamap::node_object::NodeObject::new(
                    pending.shamap_type(),
                    pending.data,
                    pending.hash,
                ));
            }
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
            }?;
            Some(shamap::node_object::NodeObject::new(
                match fetched.object_type() {
                    nodestore::NodeObjectType::AccountNode => {
                        shamap::storage::NodeObjectType::AccountNode
                    }
                    nodestore::NodeObjectType::TransactionNode => {
                        shamap::storage::NodeObjectType::TransactionNode
                    }
                    nodestore::NodeObjectType::Ledger => shamap::storage::NodeObjectType::Ledger,
                    _ => shamap::storage::NodeObjectType::Unknown,
                },
                fetched.data().to_vec(),
                *fetched.hash(),
            ))
        }
    }

    struct WorkerStore {
        node_store: SHAMapStoreNodeStore,
        write_tx: Sender<NodeStoreWriteMsg>,
        write_count: std::cell::Cell<u64>,
        pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
        shared_stored: Arc<KeyCache<Uint256>>,
    }
    impl InboundLedgerStore for WorkerStore {
        fn fetch_ledger_header(
            &mut self,
            hash: basics::sha_map_hash::SHAMapHash,
            _seq: u32,
        ) -> Option<Vec<u8>> {
            if let Some(pending) = self
                .pending_writes
                .lock()
                .expect("pending writes lock")
                .get(hash.as_uint256())
                .cloned()
            {
                return Some(pending.data);
            }
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
            }?;
            Some(fetched.data().to_vec())
        }

        fn store_ledger_header(
            &mut self,
            data: Vec<u8>,
            hash: basics::sha_map_hash::SHAMapHash,
            seq: u32,
        ) {
            self.store_object(
                nodestore::NodeObjectType::Ledger,
                data,
                *hash.as_uint256(),
                seq,
            );
        }

        fn store_shamap_node(
            &mut self,
            obj_type: shamap::storage::NodeObjectType,
            data: Vec<u8>,
            hash: Uint256,
            seq: u32,
        ) {
            let mapped = match obj_type {
                shamap::storage::NodeObjectType::AccountNode => {
                    nodestore::NodeObjectType::AccountNode
                }
                shamap::storage::NodeObjectType::TransactionNode => {
                    nodestore::NodeObjectType::TransactionNode
                }
                shamap::storage::NodeObjectType::Ledger => nodestore::NodeObjectType::Ledger,
                _ => nodestore::NodeObjectType::Unknown,
            };
            self.store_object(mapped, data, hash, seq);
        }

        fn should_store_hash(&mut self, hash: Uint256) -> bool {
            self.shared_stored.insert(hash)
        }

        fn fetch_node_data(&self, hash: Uint256) -> Option<basics::blob::Blob> {
            if let Some(pending) = self
                .pending_writes
                .lock()
                .expect("pending writes lock")
                .get(&hash)
                .cloned()
            {
                return Some(pending.data);
            }
            let fetched = match &self.node_store {
                SHAMapStoreNodeStore::Single(db) => {
                    db.fetch_node_object(&hash, 0, nodestore::FetchType::Synchronous, false)
                }
                SHAMapStoreNodeStore::Rotating(db) => {
                    db.fetch_node_object(&hash, 0, nodestore::FetchType::Synchronous, false)
                }
            }?;
            Some(fetched.data().to_vec())
        }
    }
    impl WorkerStore {
        fn store_object(
            &mut self,
            obj_type: nodestore::NodeObjectType,
            data: Vec<u8>,
            hash: Uint256,
            seq: u32,
        ) {
            self.pending_writes
                .lock()
                .expect("pending writes lock")
                .insert(
                    hash,
                    PendingNodeStoreObject {
                        obj_type,
                        data: data.clone(),
                        hash,
                    },
                );
            let _ = self.write_tx.send(NodeStoreWriteMsg::Write {
                obj_type,
                data,
                hash,
                seq,
            });
            self.write_count.set(self.write_count.get() + 1);
        }
    }

    fn flush_writes(write_tx: &Sender<NodeStoreWriteMsg>) -> bool {
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        if write_tx.send(NodeStoreWriteMsg::Flush(ack_tx)).is_err() {
            return false;
        }
        ack_rx.recv_timeout(Duration::from_secs(30)).is_ok()
    }

    let mut fetch_pack = WorkerFetchPack {
        cache: shared_fetch_pack,
    };
    let mut inbound = InboundLedgerLocal::new(hash, seq);
    let peer_set = overlay::SimplePeerSet::new(std::iter::empty::<Arc<dyn overlay::Peer>>());
    let mut first_add_peers = true;
    let mut last_timer = Instant::now();
    let journal = WorkerJournal;
    let config = LedgerConfig::default();
    let mut outbound_requests = 0u64;
    let mut last_request_at = Instant::now();
    const PEER_COUNT_START: usize = 5;
    const PEER_COUNT_ADD: usize = 3;

    let shared_queue: Arc<Mutex<Vec<AcqMsg>>> = Arc::new(Mutex::new(Vec::new()));
    let queue_condvar = Arc::new(std::sync::Condvar::new());

    let recv_queue = Arc::clone(&shared_queue);
    let recv_condvar = Arc::clone(&queue_condvar);
    let _receiver_handle = thread::Builder::new()
        .name("xrpld-sacq-recv".to_owned())
        .spawn(move || {
            loop {
                match rx.recv() {
                    Ok(msg) => {
                        let is_stop = matches!(msg, AcqMsg::Stop);
                        {
                            let mut queue = recv_queue.lock().expect("acq queue");
                            queue.push(msg);
                            while let Ok(extra) = rx.try_recv() {
                                let extra_stop = matches!(extra, AcqMsg::Stop);
                                queue.push(extra);
                                if extra_stop {
                                    break;
                                }
                            }
                        }
                        recv_condvar.notify_one();
                        if is_stop {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        })
        .expect("shared acq receiver thread");

    let mut just_processed = false;

    let mut store = WorkerStore {
        node_store: ns.clone(),
        write_tx: shared_write_tx,
        write_count: std::cell::Cell::new(0),
        pending_writes: Arc::clone(&shared_pending_writes),
        shared_stored,
    };

    loop {
        let msgs = {
            let mut queue = shared_queue.lock().expect("acq queue");
            if queue.is_empty() {
                let timeout = if last_timer.elapsed() >= Duration::from_secs(1) {
                    Duration::from_millis(0)
                } else if just_processed {
                    Duration::from_millis(1)
                } else {
                    Duration::from_secs(1)
                        .saturating_sub(last_timer.elapsed())
                        .min(Duration::from_millis(50))
                };
                if !timeout.is_zero() {
                    let (guard, _) = queue_condvar
                        .wait_timeout(queue, timeout)
                        .expect("acq condvar");
                    queue = guard;
                }
            }
            std::mem::take(&mut *queue)
        };

        let mut got_stop = false;
        let mut fetch_pack_ready = false;
        let mut peers_updated = false;
        for msg in msgs {
            match msg {
                AcqMsg::LedgerData { peer_id, packet } => {
                    let _ = inbound.got_data(Some(peer_id), packet);
                }
                AcqMsg::FetchPackReady => {
                    fetch_pack_ready = true;
                }
                AcqMsg::Peers(p) => {
                    peer_set.refresh_peers(p.iter().cloned());
                    peers_updated = true;
                }
                AcqMsg::Stop => {
                    got_stop = true;
                }
            }
        }
        if got_stop {
            break;
        }

        let has_queued_data = inbound.received_data_len() > 0;
        let timer_due = last_timer.elapsed() >= Duration::from_secs(1);
        if !has_queued_data && !timer_due && !fetch_pack_ready && !(first_add_peers && peers_updated)
        {
            if !inbound.is_done()
                && inbound.planner_state().have_header
                && inbound.planner_state().have_state
                && inbound.planner_state().have_transactions
            {
                inbound.set_complete();
            }
            if inbound.is_done() {
                break;
            }
            just_processed = false;
            continue;
        }

        let family = SHAMapFamily::new(
            shared_tree_cache.clone(),
            &*shared_full_below,
            WorkerNodeFetcher {
                node_store: ns.clone(),
                pending_writes: Arc::clone(&shared_pending_writes),
            },
            NullMissingNodeReporter,
        );

        if fetch_pack_ready {
            inbound.check_local_with_family_and_config(
                &journal,
                &config,
                &mut store,
                &mut fetch_pack,
                &family,
            );
        }

        if first_add_peers {
            let acq_hash = *hash.as_uint256();
            let mut newly_added = Vec::new();
            peer_set.add_peers(
                PEER_COUNT_START,
                &mut |peer| peer.has_ledger(acq_hash, seq),
                &mut |peer| newly_added.push(Arc::clone(peer)),
            );
            if newly_added.is_empty() {
                peer_set.add_peers(PEER_COUNT_START, &mut |_peer| true, &mut |peer| {
                    newly_added.push(Arc::clone(peer))
                });
            }
            for peer in &newly_added {
                let peer_ref = peer.clone();
                let mut send_fn = |msg: overlay::ProtocolMessage| {
                    outbound_requests += 1;
                    let wire = overlay::Message::new(msg, None);
                    peer_ref.send(wire);
                };
                inbound.trigger_with_family(
                    InboundLedgerRequestTrigger::Added,
                    &journal,
                    &config,
                    &mut store,
                    &mut fetch_pack,
                    &family,
                    &mut send_fn,
                );
                last_request_at = Instant::now();
            }
            if !newly_added.is_empty() {
                first_add_peers = false;
                last_timer = Instant::now();
            }
        }

        if inbound.received_data_len() > 0 {
            run_data_limiter.acquire();
            let run_result = inbound.run_data_with_family_and_config_and_refill(
                &journal,
                &config,
                &mut store,
                &mut fetch_pack,
                &family,
                &mut || {
                    let mut refill = Vec::new();
                    let mut queue = shared_queue.lock().expect("acq queue");
                    if queue.is_empty() {
                        return refill;
                    }
                    let mut retained = Vec::new();
                    for msg in std::mem::take(&mut *queue) {
                        match msg {
                            AcqMsg::LedgerData { peer_id, packet } => {
                                refill.push(InboundLedgerReceivedPacket::new(
                                    Some(peer_id),
                                    packet,
                                ));
                            }
                            other => retained.push(other),
                        }
                    }
                    *queue = retained;
                    refill
                },
            );
            run_data_limiter.release();

            just_processed = run_result.processed_packets > 0;

            if peer_set.peer_count() > 0 {
                for peer_id in &run_result.triggered_peer_ids {
                    let Some(peer) = peer_set.find_peer(*peer_id as u32) else {
                        continue;
                    };
                    let peer_ref = peer.clone();
                    let trigger_reason = if peer_ref.is_high_latency() {
                        InboundLedgerRequestTrigger::ReplyHighLatency
                    } else {
                        InboundLedgerRequestTrigger::Reply
                    };
                    let mut send_fn = |msg: overlay::ProtocolMessage| {
                        outbound_requests += 1;
                        peer_set.send_request(&msg, Some(&peer_ref));
                    };
                    inbound.trigger_with_family(
                        trigger_reason,
                        &journal,
                        &config,
                        &mut store,
                        &mut fetch_pack,
                        &family,
                        &mut send_fn,
                    );
                }

                if !inbound.planner_state().have_state {
                    peer_set.add_peers(6, &mut |_peer| true, &mut |_peer| {});
                    let all_peers = peer_set.get_peers();
                    let triggered: std::collections::HashSet<u32> = run_result
                        .triggered_peer_ids
                        .iter()
                        .map(|id| *id as u32)
                        .collect();
                    let extra_peers: Vec<_> = all_peers
                        .iter()
                        .filter(|p| !triggered.contains(&p.id()))
                        .take(5)
                        .cloned()
                        .collect();
                    for peer in &extra_peers {
                        let peer_ref = peer.clone();
                        let mut fan_send = |msg: overlay::ProtocolMessage| {
                            outbound_requests += 1;
                            peer_set.send_request(&msg, Some(&peer_ref));
                        };
                        inbound.trigger_with_family(
                            InboundLedgerRequestTrigger::Reply,
                            &journal,
                            &config,
                            &mut store,
                            &mut fetch_pack,
                            &family,
                            &mut fan_send,
                        );
                    }
                }

                last_request_at = Instant::now();
            }
        } else {
            just_processed = false;
        }

        if !inbound.is_done()
            && inbound.planner_state().have_header
            && inbound.planner_state().have_state
            && inbound.planner_state().have_transactions
            && !inbound.is_complete()
        {
            inbound.set_complete();
        }

        if inbound.is_done() {
            break;
        }

        if last_timer.elapsed() >= Duration::from_secs(3) {
            last_timer = Instant::now();
            let was_progress = inbound.progress();
            if was_progress {
                inbound.clear_progress();
                inbound.clear_recent_nodes();
            } else {
                let peer_limit = if peer_set.peer_count() == 0 {
                    PEER_COUNT_START
                } else {
                    PEER_COUNT_ADD
                };
                let acq_hash = *hash.as_uint256();

                let mut send_fn = |msg: overlay::ProtocolMessage| {
                    outbound_requests += 1;
                    peer_set.send_request(&msg, None);
                };
                let failed = inbound.on_timer_with_family(
                    &journal,
                    &config,
                    &mut store,
                    &mut fetch_pack,
                    &family,
                    &mut send_fn,
                );
                if failed {
                    tracing::debug!(target: "inbound_ledger", seq, "Shared acq timer failure");
                    break;
                }

                let mut newly_added = Vec::new();
                peer_set.add_peers(
                    peer_limit,
                    &mut |peer| peer.has_ledger(acq_hash, seq),
                    &mut |peer| newly_added.push(Arc::clone(peer)),
                );
                for peer in &newly_added {
                    let peer_ref = peer.clone();
                    let mut send_fn = |msg: overlay::ProtocolMessage| {
                        outbound_requests += 1;
                        peer_ref.send(overlay::Message::new(msg, None));
                    };
                    inbound.trigger_with_family(
                        InboundLedgerRequestTrigger::Added,
                        &journal,
                        &config,
                        &mut store,
                        &mut fetch_pack,
                        &family,
                        &mut send_fn,
                    );
                    last_request_at = Instant::now();
                }
                if first_add_peers && !newly_added.is_empty() {
                    first_add_peers = false;
                }
            }
        }

        if inbound.is_complete() {
            if inbound.is_failed() {
                break;
            }
            // Match the reference's InboundLedger::done() contract:
            // finalize completion/signaling before publishing the ledger.
            inbound.finish_if_done_with_family_and_config(&journal, &config, &family);
            if inbound.is_failed() {
                tracing::warn!(target: "inbound_ledger", seq, "Shared acq completion finalization failed");
                break;
            }
            let Some(mut ledger) = inbound.ledger().cloned() else {
                tracing::warn!(target: "inbound_ledger", seq, "Shared acq missing ledger on completion");
                break;
            };
            // C++ parity: InboundLedger::done() sets immutable before storeLedger().
            if !ledger.is_immutable() {
                ledger.set_immutable(true);
            }
            ledger.set_full();
            if !flush_writes(&store.write_tx) {
                tracing::warn!(target: "inbound_ledger", seq, "Shared acq flush failed");
                break;
            }
            // Attach a node_fetcher so reads can resolve nodes from NuDB.
            // Without this, any state/tx map traversal hits MissingNode
            // because the SyncTree only has root+inner nodes in memory but
            // the full_below cache marked subtrees as "complete" (they ARE
            // in NuDB, just not loaded into the tree).
            {
                let fetcher_ns = ns.clone();
                let fetcher_pending = Arc::clone(&shared_pending_writes);
                let fetcher_tc = shared_tree_cache.clone();
                ledger.set_node_fetcher(Arc::new(move |hash| {
                    // Check tree cache first
                    if let Some(node) = fetcher_tc.fetch(hash.as_uint256()) {
                        return Some(node);
                    }
                    // Check pending writes (not yet flushed to NuDB)
                    if let Ok(pending) = fetcher_pending.lock() {
                        if let Some(obj) = pending.get(hash.as_uint256()) {
                            return shamap::nodes::tree_node::SHAMapTreeNode::make_from_prefix(
                                &obj.data, hash,
                            ).ok();
                        }
                    }
                    // Fetch from NuDB
                    let data = match &fetcher_ns {
                        crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Single(db) => db
                            .fetch_node_object(
                                &hash.as_uint256(),
                                0,
                                nodestore::FetchType::Synchronous,
                                false,
                            ),
                        crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Rotating(db) => db
                            .fetch_node_object(
                                &hash.as_uint256(),
                                0,
                                nodestore::FetchType::Synchronous,
                                false,
                            ),
                    };
                    let obj = data?;
                    shamap::nodes::tree_node::SHAMapTreeNode::make_from_prefix(
                        obj.data(), hash,
                    ).ok()
                }));
            }
            tracing::info!(target: "inbound_ledger", seq, "SHARED LEDGER ACQUIRED");
            let _ = store_tx.send(Arc::new(ledger));
            return;
        }
    }

    // Post-loop completion check
    if inbound.is_complete() && !inbound.is_failed() {
        if let Some(mut ledger) = inbound.ledger().cloned() {
            if !ledger.is_immutable() {
                ledger.set_immutable(true);
            }
            ledger.set_full();
            let _ = flush_writes(&store.write_tx);
            // Attach node_fetcher (same as primary path above)
            {
                let fetcher_ns = ns.clone();
                let fetcher_pending = Arc::clone(&shared_pending_writes);
                let fetcher_tc = shared_tree_cache.clone();
                ledger.set_node_fetcher(Arc::new(move |hash| {
                    if let Some(node) = fetcher_tc.fetch(hash.as_uint256()) {
                        return Some(node);
                    }
                    if let Ok(pending) = fetcher_pending.lock() {
                        if let Some(obj) = pending.get(hash.as_uint256()) {
                            return shamap::nodes::tree_node::SHAMapTreeNode::make_from_prefix(
                                &obj.data, hash,
                            ).ok();
                        }
                    }
                    let data = match &fetcher_ns {
                        crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Single(db) => db
                            .fetch_node_object(
                                &hash.as_uint256(),
                                0,
                                nodestore::FetchType::Synchronous,
                                false,
                            ),
                        crate::shamap::shamap_store_backend::SHAMapStoreNodeStore::Rotating(db) => db
                            .fetch_node_object(
                                &hash.as_uint256(),
                                0,
                                nodestore::FetchType::Synchronous,
                                false,
                            ),
                    };
                    let obj = data?;
                    shamap::nodes::tree_node::SHAMapTreeNode::make_from_prefix(
                        obj.data(), hash,
                    ).ok()
                }));
            }
            tracing::info!(target: "inbound_ledger", seq, "SHARED LEDGER ACQUIRED");
            let _ = store_tx.send(Arc::new(ledger));
        }
    }
}

impl std::fmt::Debug for SharedInboundLedgers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedInboundLedgers").finish_non_exhaustive()
    }
}
