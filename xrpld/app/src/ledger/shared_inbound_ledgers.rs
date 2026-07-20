//! Shared inbound ledger acquisition service — TRUE JOB QUEUE model.
//!
//! Architecture: rippled parity with JtLedgerData / JtLedgerReq job model.
//!
//! Instead of one blocking thread per acquisition, acquisition state lives in
//! a shared registry (`HashMap<Uint256, Arc<AcquisitionState>>`). Worker
//! threads from a fixed-size pool process SHORT, non-blocking ticks for ANY
//! acquisition. When data arrives from a peer or a timer fires, a small job
//! is posted to the pool. The job locks the target acquisition's state,
//! processes available data, triggers peer requests if needed, checks for
//! completion, then RETURNS — freeing the thread for the next job from any
//! other acquisition.
//!
//! This means: when acquisition A is waiting for peer data, its pool thread
//! handles jobs for acquisitions B, C, D — no idle threads.

use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::{KeyCache, MonotonicClock};
use ledger::{FetchPackCache, InboundLedgerPacket, Ledger};
use overlay::Peer;
use shamap::family::FullBelowCacheImpl;
use shamap::family::FullBelowCache as FullBelowCacheTrait;
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::runtime::overlay_runtime::AppOverlayRuntime;
use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

// ─── Public types kept for backward compat ───────────────────────────────────

/// Messages sent to acquisition worker threads (legacy type alias).
/// In the new job-queue model, data is pushed directly to AcquisitionState
/// buffers, but this enum is kept for the `notify_fetch_pack_ready` and
/// `send_peers` external interfaces that still exist on SharedInboundLedgers.
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

/// Shared registry of active acquisition states, keyed by ledger hash.
/// Changed from channel-based to direct state access for the job-queue model.
pub type AcqRegistry = Arc<Mutex<HashMap<Uint256, Arc<AcquisitionState>>>>;

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
/// to the node store.
pub fn spawn_nodestore_writer(
    ns: SHAMapStoreNodeStore,
    pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
) -> (Sender<NodeStoreWriteMsg>, thread::JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::channel::<NodeStoreWriteMsg>();
    let handle = thread::Builder::new()
        .name("xrpld-nudb-writer".to_owned())
        .spawn(move || {
            let mut total_writes = 0u64;
            let mut last_log = Instant::now();
            let do_store = |ns: &SHAMapStoreNodeStore, obj_type, data, hash, seq| match ns {
                SHAMapStoreNodeStore::Single(db) => db.store(obj_type, data, hash, seq),
                SHAMapStoreNodeStore::Rotating(db) => db.store(obj_type, data, hash, seq),
            };
            let mut total_store_us = 0u64;
            loop {
                let first = match rx.recv() {
                    Ok(NodeStoreWriteMsg::Write { obj_type, data, hash, seq }) => Some((obj_type, data, hash, seq)),
                    Ok(NodeStoreWriteMsg::Flush(ack)) => {
                        let _ = ack.send(());
                        None
                    }
                    Ok(NodeStoreWriteMsg::Stop) | Err(_) => return,
                };
                if let Some((obj_type, data, hash, seq)) = first {
                    let t = Instant::now();
                    do_store(&ns, obj_type, data, hash, seq);
                    pending_writes.lock().expect("pending node-store writes mutex").remove(&hash);
                    total_store_us += t.elapsed().as_micros() as u64;
                    total_writes += 1;
                }
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
                if last_log.elapsed() >= Duration::from_secs(10) {
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

// ─── Work Pool ───────────────────────────────────────────────────────────────

/// A fixed-size thread pool for acquisition job ticks. Worker threads pull
/// short-lived jobs from a shared queue. Each job processes one tick of one
/// acquisition and returns — the thread immediately picks up the next job
/// for ANY acquisition.
struct AcquisitionWorkPool {
    queue: Arc<(Mutex<VecDeque<Box<dyn FnOnce() + Send>>>, Condvar)>,
    stop: Arc<AtomicBool>,
    _workers: Vec<JoinHandle<()>>,
}

impl AcquisitionWorkPool {
    fn new(size: usize) -> Self {
        let queue: Arc<(Mutex<VecDeque<Box<dyn FnOnce() + Send>>>, Condvar)> =
            Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let mut workers = Vec::with_capacity(size);
        for i in 0..size {
            let q = Arc::clone(&queue);
            let s = Arc::clone(&stop);
            let handle = thread::Builder::new()
                .name(format!("xrpld-acq-pool-{i}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let (lock, cvar) = &*q;
                            let mut jobs = lock.lock().expect("work pool lock");
                            while jobs.is_empty() {
                                if s.load(Ordering::Acquire) {
                                    return;
                                }
                                jobs = cvar.wait(jobs).expect("work pool cvar wait");
                            }
                            if s.load(Ordering::Acquire) {
                                return;
                            }
                            jobs.pop_front()
                        };
                        if let Some(job) = job {
                            job();
                        }
                    }
                })
                .expect("acquisition pool thread should spawn");
            workers.push(handle);
        }

        Self {
            queue,
            stop,
            _workers: workers,
        }
    }

    #[allow(dead_code)]
    fn submit(&self, job: Box<dyn FnOnce() + Send>) {
        let (lock, cvar) = &*self.queue;
        let mut jobs = lock.lock().expect("work pool submit lock");
        jobs.push_back(job);
        cvar.notify_one();
    }

    fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        let (_, cvar) = &*self.queue;
        cvar.notify_all();
    }
}

impl Drop for AcquisitionWorkPool {
    fn drop(&mut self) {
        self.shutdown();
        for handle in self._workers.drain(..) {
            let _ = handle.join();
        }
    }
}

// ─── Constants ───────────────────────────────────────────────────────────────

const REACQUIRE_INTERVAL: Duration = Duration::from_secs(5 * 60);
const SWEEP_INTERVAL: Duration = Duration::from_secs(5);
const STUCK_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONCURRENT_ACQUISITIONS: usize = 8;
/// Timer tick interval for stall detection and re-requests.
const TIMER_TICK_INTERVAL: Duration = Duration::from_secs(1);

// ─── Per-acquisition state (the "job queue" model) ───────────────────────────

/// Mutable state for a single acquisition — protected by AcquisitionState.mutable.
/// ALL processing state lives here so a single lock covers a tick.
struct AcqMutableState {
    inbound: ledger::InboundLedgerLocal,
    store: WorkerStore,
    fetch_pack: WorkerFetchPack,
    first_add_peers: bool,
    last_timer: Instant,
    just_processed: bool,
    outbound_requests: u64,
    last_request_at: Instant,
}

/// Per-acquisition state, stored in the registry, accessed by pool threads.
/// Jobs lock `mutable` briefly, do work, unlock, return.
pub struct AcquisitionState {
    /// Incoming data packets buffer — separate lock from mutable so
    /// route_response (overlay I/O thread) doesn't contend with processing.
    data_buffer: Mutex<Vec<(u64, InboundLedgerPacket)>>,
    /// Peer updates buffer — separate lock for low-contention push.
    peer_updates: Mutex<Vec<Vec<Arc<dyn Peer>>>>,
    /// All processing state behind one lock per tick.
    mutable: Mutex<AcqMutableState>,
    /// Immutable after construction:
    hash: SHAMapHash,
    seq: u32,
    peer_set: overlay::SimplePeerSet,
    worker_full_below: FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>,
    node_store: SHAMapStoreNodeStore,
    shared_tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    shared_pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
    run_data_limiter: Arc<RunDataLimiter>,
    store_tx: Sender<Arc<Ledger>>,
    /// Signals this acquisition should stop processing.
    stopped: AtomicBool,
    /// Signals acquisition completed successfully.
    completed: AtomicBool,
    /// Signals fetch-pack was populated — next tick should re-check.
    fetch_pack_ready: AtomicBool,
    /// Reference to the work pool for self-resubmission when more work needed.
    work_pool: Arc<(Mutex<VecDeque<Box<dyn FnOnce() + Send>>>, Condvar)>,
}

// Safety: all fields are Send+Sync (Mutex-wrapped or atomic)
unsafe impl Send for AcquisitionState {}
unsafe impl Sync for AcquisitionState {}

impl AcquisitionState {
    /// Submit a tick job for this acquisition to the work pool.
    fn submit_tick(self: &Arc<Self>) {
        if self.stopped.load(Ordering::Acquire) || self.completed.load(Ordering::Acquire) {
            return;
        }
        let state = Arc::clone(self);
        let job: Box<dyn FnOnce() + Send> = Box::new(move || {
            process_acquisition_tick(&state);
        });
        let (lock, cvar) = &*self.work_pool;
        let mut jobs = lock.lock().expect("work pool submit lock");
        jobs.push_back(job);
        cvar.notify_one();
    }
}

// ─── Worker helper types (same as before, inside processing) ─────────────────

struct WorkerJournal;
impl ledger::InboundLedgerJournal for WorkerJournal {
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
impl ledger::FetchPackContainer for WorkerFetchPack {
    fn get_fetch_pack(&mut self, hash: Uint256) -> Option<Vec<u8>> {
        self.cache.get_fetch_pack(hash)
    }
}
impl ledger::FetchPackStore for WorkerFetchPack {
    fn add_fetch_pack(&mut self, hash: Uint256, data: Vec<u8>) {
        self.cache.add_fetch_pack(hash, data);
    }
}

struct WorkerNodeFetcher {
    node_store: SHAMapStoreNodeStore,
    pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
}
impl shamap::family::SHAMapNodeFetcher for WorkerNodeFetcher {
    fn fetch_node_object(
        &self,
        hash: SHAMapHash,
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
    write_count: u64,
    pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
    shared_stored: Arc<KeyCache<Uint256>>,
}
impl ledger::InboundLedgerStore for WorkerStore {
    fn fetch_ledger_header(
        &mut self,
        hash: SHAMapHash,
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
        hash: SHAMapHash,
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
        if !self.shared_stored.insert(hash) {
            return;
        }

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
        self.write_count += 1;
    }
}

fn flush_writes(write_tx: &Sender<NodeStoreWriteMsg>) -> bool {
    let (ack_tx, ack_rx) = std::sync::mpsc::channel();
    if write_tx.send(NodeStoreWriteMsg::Flush(ack_tx)).is_err() {
        return false;
    }
    ack_rx.recv_timeout(Duration::from_secs(30)).is_ok()
}

// ─── Job tick: the heart of the job-queue model ──────────────────────────────

/// Process one tick of an acquisition. This is the job that pool threads
/// execute. It is SHORT and NON-BLOCKING:
/// 1. Drain data buffer (packets from peers)
/// 2. Process queued data (run_data)
/// 3. Timer check: re-request if stalled
/// 4. Completion check
/// Then RETURN — thread picks up next job for any acquisition.
fn process_acquisition_tick(state: &Arc<AcquisitionState>) {
    use ledger::InboundLedgerReceivedPacket;
    use ledger::InboundLedgerRequestTrigger;
    use overlay::PeerSet as _;
    use shamap::family::{FullBelowCache, NullMissingNodeReporter, SHAMapFamily};

    if state.stopped.load(Ordering::Acquire) || state.completed.load(Ordering::Acquire) {
        return;
    }

    let journal = WorkerJournal;
    let config = ledger::LedgerConfig::default();
    let hash = state.hash;
    let seq = state.seq;

    const PEER_COUNT_START: usize = 5;
    const PEER_COUNT_ADD: usize = 3;

    // 1. Drain peer updates (low-contention lock)
    {
        let peer_batches: Vec<Vec<Arc<dyn Peer>>> = {
            let mut guard = state.peer_updates.lock().expect("peer_updates lock");
            std::mem::take(&mut *guard)
        };
        for batch in peer_batches {
            state.peer_set.refresh_peers(batch.into_iter());
        }
    }

    // 2. Drain data buffer into inbound (low-contention lock on data_buffer)
    let packets: Vec<(u64, InboundLedgerPacket)> = {
        let mut guard = state.data_buffer.lock().expect("data_buffer lock");
        std::mem::take(&mut *guard)
    };

    // 3. Lock mutable state for processing.
    // We destructure to get independent mutable borrows of each field.
    let mut guard = state.mutable.lock().expect("acq mutable lock");
    let m = &mut *guard;
    let inbound = &mut m.inbound;
    let store = &mut m.store;
    let fetch_pack = &mut m.fetch_pack;
    let outbound_requests = &mut m.outbound_requests;
    let first_add_peers = &mut m.first_add_peers;
    let last_timer = &mut m.last_timer;
    let last_request_at = &mut m.last_request_at;
    let just_processed = &mut m.just_processed;

    // Feed packets to inbound
    for (peer_id, packet) in &packets {
        let _ = inbound.got_data(Some(*peer_id), packet.clone());
    }

    let fetch_pack_ready_flag = state.fetch_pack_ready.swap(false, Ordering::AcqRel);
    let has_queued_data = inbound.received_data_len() > 0;
    let timer_due = last_timer.elapsed() >= Duration::from_secs(1);
    let peers_updated = !packets.is_empty() || {
        state.peer_set.peer_count() > 0 && *first_add_peers
    };

    if !has_queued_data && !timer_due && !fetch_pack_ready_flag && !(*first_add_peers && peers_updated) {
        if !inbound.is_done()
            && inbound.planner_state().have_header
            && inbound.planner_state().have_state
            && inbound.planner_state().have_transactions
        {
            inbound.set_complete();
        }
        if inbound.is_done() {
            drop(guard);
            finalize_acquisition(state);
            return;
        }
        return;
    }

    // Build family for this tick
    let family = SHAMapFamily::new(
        state.shared_tree_cache.clone(),
        &state.worker_full_below,
        WorkerNodeFetcher {
            node_store: state.node_store.clone(),
            pending_writes: Arc::clone(&state.shared_pending_writes),
        },
        NullMissingNodeReporter,
    );

    if fetch_pack_ready_flag {
        inbound.check_local_with_family_and_config(
            &journal,
            &config,
            store,
            fetch_pack,
            &family,
        );
    }

    // First add peers trigger
    if *first_add_peers {
        let acq_hash = *hash.as_uint256();
        let mut newly_added = Vec::new();
        state.peer_set.add_peers(
            PEER_COUNT_START,
            &mut |peer| peer.has_ledger(acq_hash, seq),
            &mut |peer| newly_added.push(Arc::clone(peer)),
        );
        if newly_added.is_empty() {
            state.peer_set.add_peers(PEER_COUNT_START, &mut |_peer| true, &mut |peer| {
                newly_added.push(Arc::clone(peer))
            });
        }
        for peer in &newly_added {
            let peer_ref = peer.clone();
            *outbound_requests += 1;
            let mut send_fn = |msg: overlay::ProtocolMessage| {
                let wire = overlay::Message::new(msg, None);
                peer_ref.send(wire);
            };
            inbound.trigger_with_family(
                InboundLedgerRequestTrigger::Added,
                &journal,
                &config,
                store,
                fetch_pack,
                &family,
                &mut send_fn,
            );
            *last_request_at = Instant::now();
        }
        if !newly_added.is_empty() {
            *first_add_peers = false;
            *last_timer = Instant::now();
        }
    }

    // Process received data
    if inbound.received_data_len() > 0 {
        state.run_data_limiter.acquire();

        // The refill closure can drain more data that arrived while we process
        let data_buffer_ref = &state.data_buffer;
        let run_result = inbound.run_data_with_family_and_config_and_refill(
            &journal,
            &config,
            store,
            fetch_pack,
            &family,
            &mut || {
                let mut refill = Vec::new();
                let mut buf_guard = data_buffer_ref.lock().expect("data_buffer refill lock");
                for (peer_id, packet) in buf_guard.drain(..) {
                    refill.push(InboundLedgerReceivedPacket::new(
                        Some(peer_id),
                        packet,
                    ));
                }
                refill
            },
        );
        state.run_data_limiter.release();

        *just_processed = run_result.processed_packets > 0;

        if state.peer_set.peer_count() > 0 {
            for peer_id in &run_result.triggered_peer_ids {
                let Some(peer) = state.peer_set.find_peer(*peer_id as u32) else {
                    continue;
                };
                let peer_ref = peer.clone();
                let trigger_reason = if peer_ref.is_high_latency() {
                    InboundLedgerRequestTrigger::ReplyHighLatency
                } else {
                    InboundLedgerRequestTrigger::Reply
                };
                *outbound_requests += 1;
                let mut send_fn = |msg: overlay::ProtocolMessage| {
                    state.peer_set.send_request(&msg, Some(&peer_ref));
                };
                inbound.trigger_with_family(
                    trigger_reason,
                    &journal,
                    &config,
                    store,
                    fetch_pack,
                    &family,
                    &mut send_fn,
                );
            }

            if !inbound.planner_state().have_state {
                state.peer_set.add_peers(6, &mut |_peer| true, &mut |_peer| {});
                let all_peers = state.peer_set.get_peers();
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
                    *outbound_requests += 1;
                    let mut fan_send = |msg: overlay::ProtocolMessage| {
                        state.peer_set.send_request(&msg, Some(&peer_ref));
                    };
                    inbound.trigger_with_family(
                        InboundLedgerRequestTrigger::Reply,
                        &journal,
                        &config,
                        store,
                        fetch_pack,
                        &family,
                        &mut fan_send,
                    );
                }
            }

            *last_request_at = Instant::now();
        }
    } else {
        *just_processed = false;
    }

    // Progressive memory spill
    const SPILL_INTERVAL_NODES: u64 = 50_000;
    {
        let current_writes = store.write_count;
        if current_writes > 0
            && current_writes % SPILL_INTERVAL_NODES == 0
            && inbound.planner_state().have_header
        {
            let _ = flush_writes(&store.write_tx);
            let fb_gen = state.worker_full_below.generation();
            if let Some(ledger) = inbound.ledger() {
                let released = ledger.state_map().spill_full_below_subtrees(fb_gen);
                if released > 0 {
                    tracing::debug!(
                        target: "inbound_ledger",
                        seq, writes = current_writes, released,
                        "Progressive spill: released full-below subtrees"
                    );
                }
            }
        }
    }

    // Completion check
    if !inbound.is_done()
        && inbound.planner_state().have_header
        && inbound.planner_state().have_state
        && inbound.planner_state().have_transactions
        && !inbound.is_complete()
    {
        inbound.set_complete();
    }

    if inbound.is_done() || inbound.is_complete() {
        drop(guard);
        finalize_acquisition(state);
        return;
    }

    // Timer logic (re-request stalled acquisitions)
    if last_timer.elapsed() >= Duration::from_secs(3) {
        *last_timer = Instant::now();
        let was_progress = inbound.progress();
        if was_progress {
            inbound.clear_progress();
            inbound.clear_recent_nodes();
        } else {
            let peer_limit = if state.peer_set.peer_count() == 0 {
                PEER_COUNT_START
            } else {
                PEER_COUNT_ADD
            };
            let acq_hash = *hash.as_uint256();

            *outbound_requests += 1;
            let mut send_fn = |msg: overlay::ProtocolMessage| {
                state.peer_set.send_request(&msg, None);
            };
            let failed = inbound.on_timer_with_family(
                &journal,
                &config,
                store,
                fetch_pack,
                &family,
                &mut send_fn,
            );
            if failed {
                tracing::debug!(target: "inbound_ledger", seq, "Shared acq timer failure");
                state.stopped.store(true, Ordering::Release);
                return;
            }

            let mut newly_added = Vec::new();
            state.peer_set.add_peers(
                peer_limit,
                &mut |peer| peer.has_ledger(acq_hash, seq),
                &mut |peer| newly_added.push(Arc::clone(peer)),
            );
            for peer in &newly_added {
                let peer_ref = peer.clone();
                *outbound_requests += 1;
                let mut send_fn = |msg: overlay::ProtocolMessage| {
                    peer_ref.send(overlay::Message::new(msg, None));
                };
                inbound.trigger_with_family(
                    InboundLedgerRequestTrigger::Added,
                    &journal,
                    &config,
                    store,
                    fetch_pack,
                    &family,
                    &mut send_fn,
                );
                *last_request_at = Instant::now();
            }
            if *first_add_peers && !newly_added.is_empty() {
                *first_add_peers = false;
            }
        }
    }

    // If we just processed data, self-resubmit to process more quickly
    let should_resubmit = *just_processed;
    drop(guard);
    if should_resubmit {
        state.submit_tick();
    }
}

/// Finalize a completed acquisition — publish the ledger.
fn finalize_acquisition(state: &Arc<AcquisitionState>) {
    if state.completed.swap(true, Ordering::AcqRel) {
        return; // Already finalized
    }

    let mut mutable = state.mutable.lock().expect("acq mutable lock (finalize)");
    let journal = WorkerJournal;
    let config = ledger::LedgerConfig::default();
    let seq = state.seq;

    let family = shamap::family::SHAMapFamily::new(
        state.shared_tree_cache.clone(),
        &state.worker_full_below,
        WorkerNodeFetcher {
            node_store: state.node_store.clone(),
            pending_writes: Arc::clone(&state.shared_pending_writes),
        },
        shamap::family::NullMissingNodeReporter,
    );

    if !mutable.inbound.is_complete() {
        if mutable.inbound.planner_state().have_header
            && mutable.inbound.planner_state().have_state
            && mutable.inbound.planner_state().have_transactions
        {
            mutable.inbound.set_complete();
        } else {
            state.completed.store(false, Ordering::Release);
            return;
        }
    }

    if mutable.inbound.is_failed() {
        return;
    }

    mutable.inbound.finish_if_done_with_family_and_config(&journal, &config, &family);
    if mutable.inbound.is_failed() {
        tracing::warn!(target: "inbound_ledger", seq, "Shared acq completion finalization failed");
        return;
    }

    let Some(mut ledger) = mutable.inbound.ledger().cloned() else {
        tracing::warn!(target: "inbound_ledger", seq, "Shared acq missing ledger on completion");
        state.completed.store(false, Ordering::Release);
        return;
    };

    if !ledger.is_immutable() {
        ledger.set_immutable(true);
    }
    ledger.set_full();
    if !flush_writes(&mutable.store.write_tx) {
        tracing::warn!(target: "inbound_ledger", seq, "Shared acq flush failed");
        return;
    }
    state.shared_pending_writes.lock().expect("pending writes lock").clear();
    tracing::info!(target: "inbound_ledger", acq_cache_size = state.shared_tree_cache.size(), "SHARED LEDGER ACQUIRED: acquisition complete");

    // Attach node_fetcher for NuDB-backed reads
    {
        let fetcher_ns = state.node_store.clone();
        let fetcher_pending = Arc::clone(&state.shared_pending_writes);
        let fetcher_tc = state.shared_tree_cache.clone();
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
                SHAMapStoreNodeStore::Single(db) => db
                    .fetch_node_object(
                        &hash.as_uint256(),
                        0,
                        nodestore::FetchType::Synchronous,
                        false,
                    ),
                SHAMapStoreNodeStore::Rotating(db) => db
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
    tracing::info!(target: "inbound_ledger", seq = ledger.header().seq, "SHARED LEDGER ACQUIRED");
    ledger.release_maps_to_disk();
    {
        #[cfg(not(target_env = "msvc"))]
        {
            use tikv_jemalloc_ctl::{epoch, stats};
            epoch::advance().ok();
            let allocated = stats::allocated::read().unwrap_or(0);
            let resident = stats::resident::read().unwrap_or(0);
            tracing::info!(target: "inbound_ledger", seq,
                allocated_mb = allocated / 1024 / 1024,
                resident_mb = resident / 1024 / 1024,
                "SHARED LEDGER ACQUIRED: jemalloc stats after release"
            );
        }
    }
    let _ = state.store_tx.send(Arc::new(ledger));
}

// ─── Entry tracking ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryState {
    InProgress,
    Complete,
    #[allow(dead_code)]
    Failed,
}

struct InboundEntry {
    seq: u32,
    state: Arc<AcquisitionState>,
    started_at: Instant,
    last_touched: Instant,
    completed_at: Option<Instant>,
    state_flag: EntryState,
}

struct Inner {
    entries: HashMap<Uint256, InboundEntry>,
    recent_failures: HashMap<Uint256, Instant>,
}

// ─── SharedInboundLedgers ────────────────────────────────────────────────────

/// Thread-safe shared service for inbound ledger acquisition.
///
/// Uses a TRUE JOB QUEUE model: worker threads process short ticks for any
/// acquisition, never blocking on a single one.
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
    has_validated_ledger: AtomicBool,
    /// Fixed-size thread pool shared by ALL acquisitions. Jobs are short ticks.
    work_pool: AcquisitionWorkPool,
    /// Timer thread handle — fires periodic ticks for all active acquisitions.
    _timer_handle: Mutex<Option<JoinHandle<()>>>,
    /// Stop flag for the timer thread.
    timer_stop: Arc<AtomicBool>,
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
        let timer_stop = Arc::new(AtomicBool::new(false));
        let pool = AcquisitionWorkPool::new(MAX_CONCURRENT_ACQUISITIONS);

        // Spawn the timer thread: periodically submits timer tick jobs for
        // all active acquisitions. This replaces the per-worker condvar
        // timeout approach.
        let timer_registry = Arc::clone(&registry);
        let timer_stop_clone = Arc::clone(&timer_stop);
        let timer_pool_queue = Arc::clone(&pool.queue);
        let timer_handle = thread::Builder::new()
            .name("xrpld-acq-timer".to_owned())
            .spawn(move || {
                while !timer_stop_clone.load(Ordering::Acquire) {
                    thread::sleep(TIMER_TICK_INTERVAL);
                    if timer_stop_clone.load(Ordering::Acquire) {
                        break;
                    }
                    // Submit a tick job for each active acquisition
                    let states: Vec<Arc<AcquisitionState>> = {
                        let guard = timer_registry.lock().expect("timer registry lock");
                        guard.values().cloned().collect()
                    };
                    for state in states {
                        if state.stopped.load(Ordering::Acquire)
                            || state.completed.load(Ordering::Acquire)
                        {
                            continue;
                        }
                        let s = Arc::clone(&state);
                        let job: Box<dyn FnOnce() + Send> = Box::new(move || {
                            process_acquisition_tick(&s);
                        });
                        let (lock, cvar) = &*timer_pool_queue;
                        let mut jobs = lock.lock().expect("timer pool submit");
                        jobs.push_back(job);
                        cvar.notify_one();
                    }
                }
            })
            .expect("timer thread");

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
            has_validated_ledger: AtomicBool::new(false),
            work_pool: pool,
            _timer_handle: Mutex::new(Some(timer_handle)),
            timer_stop,
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

    pub fn mark_has_validated_ledger(&self) {
        self.has_validated_ledger.store(true, Ordering::Release);
    }

    pub fn acquire(&self, hash: Uint256, seq: u32) {
        self.acquire_inner(hash, seq, false);
    }

    pub fn acquire_for_consensus(&self, hash: Uint256, seq: u32) {
        self.acquire_inner(hash, seq, true);
    }

    fn acquire_inner(&self, hash: Uint256, seq: u32, _force_bypass_cold_start: bool) {
        if hash.is_zero() {
            return;
        }
        tracing::debug!(target: "inbound_ledger", %hash, seq, "SharedInboundLedgers::acquire called");

        let mut inner = self.inner.lock().expect("shared_inbound lock");

        // Check recent failures
        if let Some(failed_at) = inner.recent_failures.get(&hash) {
            if failed_at.elapsed() < REACQUIRE_INTERVAL {
                tracing::trace!(target: "inbound_ledger", %hash, "acquire: skipped (recent failure)");
                return;
            }
        }
        inner
            .recent_failures
            .retain(|_, t| t.elapsed() < REACQUIRE_INTERVAL);

        // Already tracked — touch and return
        if let Some(entry) = inner.entries.get_mut(&hash) {
            entry.last_touched = Instant::now();
            tracing::trace!(target: "inbound_ledger", %hash, "acquire: already tracked, touching");
            return;
        }

        // Bound concurrent in-progress acquisitions
        let in_progress_count = inner
            .entries
            .values()
            .filter(|e| e.state_flag == EntryState::InProgress)
            .count();
        if in_progress_count >= MAX_CONCURRENT_ACQUISITIONS {
            let lowest_seq_hash = inner
                .entries
                .iter()
                .filter(|(_, e)| e.state_flag == EntryState::InProgress)
                .min_by_key(|(_, e)| e.seq)
                .map(|(h, _)| *h);
            if let Some(evict_hash) = lowest_seq_hash {
                if let Some(evicted) = inner.entries.remove(&evict_hash) {
                    evicted.state.stopped.store(true, Ordering::Release);
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
        let tc = Arc::clone(&self.tree_cache);
        let fp = Arc::clone(&self.fetch_pack);
        let rl = Arc::clone(&self.run_data_limiter);
        let ss = Arc::clone(&self.shared_stored);
        let store_tx = self.completed_ledgers_tx.clone();

        // Build per-acquisition full_below cache (isolated per worker)
        let worker_full_below = FullBelowCacheImpl::new(
            (*self.full_below).generation().wrapping_add(1),
            MonotonicClock::default(),
            HardenedHashBuilder::default(),
            524_288,
        );

        let now = Instant::now();

        // Create the acquisition state
        let acq_state = Arc::new(AcquisitionState {
            data_buffer: Mutex::new(Vec::new()),
            peer_updates: Mutex::new(Vec::new()),
            mutable: Mutex::new(AcqMutableState {
                inbound: ledger::InboundLedgerLocal::new(shamap_hash, seq),
                store: WorkerStore {
                    node_store: ns.clone(),
                    write_tx: wt,
                    write_count: 0,
                    pending_writes: Arc::clone(&pending),
                    shared_stored: ss,
                },
                fetch_pack: WorkerFetchPack { cache: fp },
                first_add_peers: true,
                last_timer: now,
                just_processed: false,
                outbound_requests: 0,
                last_request_at: now,
            }),
            hash: shamap_hash,
            seq,
            peer_set: overlay::SimplePeerSet::new(std::iter::empty::<Arc<dyn overlay::Peer>>()),
            worker_full_below,
            node_store: ns,
            shared_tree_cache: tc,
            shared_pending_writes: pending,
            run_data_limiter: rl,
            store_tx,
            stopped: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            fetch_pack_ready: AtomicBool::new(false),
            work_pool: Arc::clone(&self.work_pool.queue),
        });

        // Register in the state registry
        self.registry
            .lock()
            .expect("acq registry")
            .insert(hash, Arc::clone(&acq_state));

        // Send initial peers so the first tick has them
        {
            let guard = self.overlay_rt.read().expect("overlay_rt read");
            if let Some(overlay_rt) = guard.as_ref() {
                use overlay::Overlay as _;
                let peers = overlay_rt.overlay().active_peers();
                acq_state.peer_set.refresh_peers(peers.into_iter());
            }
        }

        inner.entries.insert(
            hash,
            InboundEntry {
                seq,
                state: Arc::clone(&acq_state),
                started_at: now,
                last_touched: now,
                completed_at: None,
                state_flag: EntryState::InProgress,
            },
        );

        tracing::debug!(target: "inbound_ledger", seq, hash = %shamap_hash, "Shared acquire started (job-queue model)");

        // Submit the first tick job to kick off the acquisition
        acq_state.submit_tick();
    }

    /// Route a TmLedgerData response to the correct acquisition.
    /// Pushes data to the state's buffer and submits a ProcessData tick job.
    pub fn route_response(&self, hash: &Uint256, peer_id: u64, packet: InboundLedgerPacket) {
        let state = {
            let guard = self.registry.lock().expect("acq registry lock");
            guard.get(hash).cloned()
        };
        if let Some(state) = state {
            // Push to data buffer (fast, separate lock from processing)
            {
                let mut buf = state.data_buffer.lock().expect("data_buffer push lock");
                buf.push((peer_id, packet));
            }
            // Submit a tick job so a pool thread processes this data
            state.submit_tick();
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
            match entry.state_flag {
                EntryState::InProgress => {
                    // Also check if the acquisition completed itself
                    if entry.state.completed.load(Ordering::Acquire) {
                        to_remove.push(*hash);
                    } else if now.duration_since(entry.started_at) > STUCK_TIMEOUT
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
                entry.state.stopped.store(true, Ordering::Release);
                self.registry.lock().expect("acq registry").remove(hash);
            }
        }

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
        let states: Vec<Arc<AcquisitionState>> = {
            let guard = self.registry.lock().expect("acq registry lock");
            guard.values().cloned().collect()
        };
        for state in states {
            if state.stopped.load(Ordering::Acquire) || state.completed.load(Ordering::Acquire) {
                continue;
            }
            state.peer_set.refresh_peers(peers.iter().cloned());
            // Submit a tick so the acquisition can use new peers
            state.submit_tick();
        }
    }

    /// Mark an entry as completed.
    pub fn mark_complete(&self, hash: &Uint256) {
        let mut inner = self.inner.lock().expect("shared_inbound lock");
        if let Some(entry) = inner.entries.get_mut(hash) {
            entry.state_flag = EntryState::Complete;
            entry.completed_at = Some(Instant::now());
            entry.last_touched = Instant::now();
        }
    }

    /// Mark an entry as failed and record in recent failures.
    pub fn mark_failed(&self, hash: &Uint256) {
        let mut inner = self.inner.lock().expect("shared_inbound lock");
        inner.recent_failures.insert(*hash, Instant::now());
        if let Some(entry) = inner.entries.remove(hash) {
            entry.state.stopped.store(true, Ordering::Release);
            self.registry.lock().expect("acq registry").remove(hash);
        }
    }

    /// Remove a specific entry.
    pub fn remove(&self, hash: &Uint256) {
        let mut inner = self.inner.lock().expect("shared_inbound lock");
        if let Some(entry) = inner.entries.remove(hash) {
            entry.state.stopped.store(true, Ordering::Release);
            self.registry.lock().expect("acq registry").remove(hash);
        }
    }

    /// Number of in-progress acquisitions.
    pub fn active_count(&self) -> usize {
        let inner = self.inner.lock().expect("shared_inbound lock");
        inner
            .entries
            .values()
            .filter(|e| e.state_flag == EntryState::InProgress)
            .count()
    }

    /// Send a fetch-pack-ready signal to all in-progress workers.
    pub fn notify_fetch_pack_ready(&self) {
        let states: Vec<Arc<AcquisitionState>> = {
            let guard = self.registry.lock().expect("acq registry lock");
            guard.values().cloned().collect()
        };
        for state in states {
            if state.stopped.load(Ordering::Acquire) || state.completed.load(Ordering::Acquire) {
                continue;
            }
            state.fetch_pack_ready.store(true, Ordering::Release);
            state.submit_tick();
        }
    }

    /// Stop all active acquisitions.
    pub fn stop(&self) {
        // Stop the timer thread first
        self.timer_stop.store(true, Ordering::Release);

        let mut inner = self.inner.lock().expect("shared_inbound lock");
        for (hash, entry) in inner.entries.drain() {
            entry.state.stopped.store(true, Ordering::Release);
            self.registry.lock().expect("acq registry").remove(&hash);
        }
        inner.recent_failures.clear();
        self.work_pool.shutdown();
    }
}

impl std::fmt::Debug for SharedInboundLedgers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedInboundLedgers").finish_non_exhaustive()
    }
}
