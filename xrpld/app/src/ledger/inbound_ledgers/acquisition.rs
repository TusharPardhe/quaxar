//! Per-hash acquisition state.
//!
//! Wraps `InboundLedgerLocal` (the existing per-ledger state machine from the
//! `ledger` crate) and provides the job-queue integration: buffered data
//! arrival, self-resubmission, and completion signaling.

use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::{KeyCache, MonotonicClock};
use ledger::{
    FetchPackCache, FetchPackContainer, FetchPackStore, InboundLedgerJournal,
    InboundLedgerLocal, InboundLedgerPacket, InboundLedgerReceivedPacket,
    InboundLedgerRequestTrigger, InboundLedgerStore, Ledger,
};
use overlay::Peer;
use shamap::family::{FullBelowCache, FullBelowCacheImpl, NullMissingNodeReporter, SHAMapFamily};
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

use super::worker_pool::JobQueue;

// ─── Constants ───────────────────────────────────────────────────────────────

const PEER_COUNT_START: usize = 5;
const PEER_COUNT_ADD: usize = 3;
const SPILL_INTERVAL_NODES: u64 = 2_048;

// ─── Public types ────────────────────────────────────────────────────────────

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

// ─── Worker helper types ─────────────────────────────────────────────────────

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

/// Worker store implementation for InboundLedgerStore trait.
pub struct WorkerStore {
    node_store: SHAMapStoreNodeStore,
    write_tx: Sender<NodeStoreWriteMsg>,
    write_count: u64,
    pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
    shared_stored: Arc<KeyCache<Uint256>>,
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

impl InboundLedgerStore for WorkerStore {
    fn fetch_ledger_header(&mut self, hash: SHAMapHash, _seq: u32) -> Option<Vec<u8>> {
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

// ─── Mutable state ───────────────────────────────────────────────────────────

/// Mutable state for a single acquisition — protected by AcquisitionState.mutable.
pub struct AcqMutableState {
    pub inbound: InboundLedgerLocal,
    pub store: WorkerStore,
    pub(crate) fetch_pack: WorkerFetchPack,
    pub first_add_peers: bool,
    pub last_timer: Instant,
    pub just_processed: bool,
    pub outbound_requests: u64,
    pub last_request_at: Instant,
}

// ─── AcquisitionState ────────────────────────────────────────────────────────

/// Per-acquisition state, stored in the registry, accessed by pool threads.
/// Jobs lock `mutable` briefly, do work, unlock, return.
pub struct AcquisitionState {
    /// Incoming data packets buffer — separate lock from mutable.
    pub data_buffer: Mutex<Vec<(u64, InboundLedgerPacket)>>,
    /// Peer updates buffer — separate lock for low-contention push.
    pub peer_updates: Mutex<Vec<Vec<Arc<dyn Peer>>>>,
    /// All processing state behind one lock per tick.
    pub mutable: Mutex<AcqMutableState>,
    /// Immutable after construction:
    pub hash: SHAMapHash,
    pub seq: u32,
    pub peer_set: overlay::SimplePeerSet,
    pub worker_full_below: FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>,
    pub node_store: SHAMapStoreNodeStore,
    pub shared_tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    pub shared_pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
    pub run_data_limiter: Arc<RunDataLimiter>,
    pub store_tx: Sender<Arc<Ledger>>,
    /// Signals this acquisition should stop processing.
    pub stopped: AtomicBool,
    /// Signals acquisition completed successfully.
    pub completed: AtomicBool,
    /// Signals fetch-pack was populated — next tick should re-check.
    pub fetch_pack_ready: AtomicBool,
    /// Reference to the work pool queue for self-resubmission.
    pub work_pool: JobQueue,
}

// Safety: all fields are Send+Sync (Mutex-wrapped or atomic)
unsafe impl Send for AcquisitionState {}
unsafe impl Sync for AcquisitionState {}

impl AcquisitionState {
    /// Submit a tick job for this acquisition to the work pool.
    pub fn submit_tick(self: &Arc<Self>) {
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

/// Builder for creating AcquisitionState instances.
pub struct AcquisitionBuilder {
    pub hash: SHAMapHash,
    pub seq: u32,
    pub node_store: SHAMapStoreNodeStore,
    pub write_tx: Sender<NodeStoreWriteMsg>,
    pub pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
    pub tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    pub fetch_pack: Arc<FetchPackCache>,
    pub run_data_limiter: Arc<RunDataLimiter>,
    pub shared_stored: Arc<KeyCache<Uint256>>,
    pub store_tx: Sender<Arc<Ledger>>,
    pub full_below_generation: u32,
    pub work_pool: JobQueue,
    pub initial_peers: Vec<Arc<dyn Peer>>,
}

impl AcquisitionBuilder {
    pub fn build(self) -> Arc<AcquisitionState> {
        let worker_full_below = FullBelowCacheImpl::new(
            self.full_below_generation,
            MonotonicClock::default(),
            HardenedHashBuilder::default(),
            524_288,
        );
        let now = Instant::now();

        Arc::new(AcquisitionState {
            data_buffer: Mutex::new(Vec::new()),
            peer_updates: Mutex::new(Vec::new()),
            mutable: Mutex::new(AcqMutableState {
                inbound: InboundLedgerLocal::new(self.hash, self.seq),
                store: WorkerStore {
                    node_store: self.node_store.clone(),
                    write_tx: self.write_tx,
                    write_count: 0,
                    pending_writes: Arc::clone(&self.pending_writes),
                    shared_stored: self.shared_stored,
                },
                fetch_pack: WorkerFetchPack {
                    cache: self.fetch_pack,
                },
                first_add_peers: true,
                last_timer: now,
                just_processed: false,
                outbound_requests: 0,
                last_request_at: now,
            }),
            hash: self.hash,
            seq: self.seq,
            peer_set: overlay::SimplePeerSet::new(self.initial_peers.into_iter()),
            worker_full_below,
            node_store: self.node_store,
            shared_tree_cache: self.tree_cache,
            shared_pending_writes: self.pending_writes,
            run_data_limiter: self.run_data_limiter,
            store_tx: self.store_tx,
            stopped: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            fetch_pack_ready: AtomicBool::new(false),
            work_pool: self.work_pool,
        })
    }
}

// ─── Flush helper ────────────────────────────────────────────────────────────

fn flush_writes(write_tx: &Sender<NodeStoreWriteMsg>) -> bool {
    let (ack_tx, ack_rx) = std::sync::mpsc::channel();
    if write_tx.send(NodeStoreWriteMsg::Flush(ack_tx)).is_err() {
        return false;
    }
    ack_rx.recv_timeout(Duration::from_secs(30)).is_ok()
}

// ─── Spawn node-store writer ─────────────────────────────────────────────────

/// Spawn a dedicated background thread that flushes acquired SHAMap nodes
/// to the node store.
pub fn spawn_nodestore_writer(
    ns: SHAMapStoreNodeStore,
    pending_writes: Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>,
) -> (Sender<NodeStoreWriteMsg>, std::thread::JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::channel::<NodeStoreWriteMsg>();
    let handle = std::thread::Builder::new()
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
                    Ok(NodeStoreWriteMsg::Write {
                        obj_type,
                        data,
                        hash,
                        seq,
                    }) => Some((obj_type, data, hash, seq)),
                    Ok(NodeStoreWriteMsg::Flush(ack)) => {
                        let _ = ack.send(());
                        None
                    }
                    Ok(NodeStoreWriteMsg::Stop) | Err(_) => return,
                };
                if let Some((obj_type, data, hash, seq)) = first {
                    let t = Instant::now();
                    do_store(&ns, obj_type, data, hash, seq);
                    pending_writes
                        .lock()
                        .expect("pending node-store writes mutex")
                        .remove(&hash);
                    total_store_us += t.elapsed().as_micros() as u64;
                    total_writes += 1;
                }
                loop {
                    match rx.try_recv() {
                        Ok(NodeStoreWriteMsg::Write {
                            obj_type,
                            data,
                            hash,
                            seq,
                        }) => {
                            let t = Instant::now();
                            do_store(&ns, obj_type, data, hash, seq);
                            pending_writes
                                .lock()
                                .expect("pending node-store writes mutex")
                                .remove(&hash);
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
                    let avg_us = if total_writes > 0 {
                        total_store_us / total_writes
                    } else {
                        0
                    };
                    tracing::debug!(target: "nodestore", total_writes, avg_us, "NuDB writer status");
                    last_log = Instant::now();
                }
            }
        })
        .expect("nudb writer thread");
    (tx, handle)
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
    use overlay::PeerSet as _;

    if state.stopped.load(Ordering::Acquire) || state.completed.load(Ordering::Acquire) {
        return;
    }

    let journal = WorkerJournal;
    let config = ledger::LedgerConfig::default();
    let hash = state.hash;
    let seq = state.seq;

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
    let peers_updated = !packets.is_empty() || (state.peer_set.peer_count() > 0 && *first_add_peers);

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

    if fetch_pack_ready_flag || *first_add_peers {
        inbound.check_local_with_family_and_config(
            &journal, &config, store, fetch_pack, &family,
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
                &journal, &config, store, fetch_pack, &family, &mut send_fn,
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

        let data_buffer_ref = &state.data_buffer;
        let run_result = inbound.run_data_with_family_and_config_and_refill(
            &journal, &config, store, fetch_pack, &family,
            &mut || {
                let mut refill = Vec::new();
                let mut buf_guard = data_buffer_ref.lock().expect("data_buffer refill lock");
                for (peer_id, packet) in buf_guard.drain(..) {
                    refill.push(InboundLedgerReceivedPacket::new(Some(peer_id), packet));
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
                    &journal, &config, store, fetch_pack, &family, &mut send_fn,
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
                        &journal, &config, store, fetch_pack, &family, &mut fan_send,
                    );
                }
            }

            *last_request_at = Instant::now();
        }
    } else {
        *just_processed = false;
    }

    // Progressive memory spill — leveraging quaxar's release_loaded_children
    // mechanism (which rippled lacks). After nodes are written to NuDB, we
    // release loaded children at depth >= 3, bounding memory to the top 3
    // levels (~4K inner nodes) + whatever frontier nodes getMissingNodes
    // re-loads on the next tick. This is BETTER than rippled's TreeNodeCache
    // TTL sweep because it's immediate and deterministic.
    {
        let current_writes = store.write_count;
        if current_writes > 0
            && current_writes % SPILL_INTERVAL_NODES == 0
            && inbound.planner_state().have_header
        {
            let _ = flush_writes(&store.write_tx);
            if let Some(ledger) = inbound.ledger() {
                // First: release completed subtrees (free, no re-fetch needed for these)
                let fb_gen = state.worker_full_below.generation();
                let full_released = ledger.state_map().spill_full_below_subtrees(fb_gen);
                // Second: aggressively release deep nodes regardless of completion.
                // keep_depth=3 means we keep root + 2 levels (~272 inner nodes max)
                // loaded, and release everything deeper. getMissingNodes will re-load
                // only the frontier branches it needs from NuDB.
                let deep_released = ledger.state_map().release_deep_children(3);
                if full_released + deep_released > 0 {
                    tracing::debug!(
                        target: "inbound_ledger",
                        seq, writes = current_writes, full_released, deep_released,
                        "Progressive spill: released subtrees for memory bound"
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
                &journal, &config, store, fetch_pack, &family, &mut send_fn,
            );
            if failed {
                tracing::debug!(target: "inbound_ledger", seq, "Acquisition timer failure");
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
                    &journal, &config, store, fetch_pack, &family, &mut send_fn,
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

// ─── Finalization ────────────────────────────────────────────────────────────

/// Finalize a completed acquisition — publish the ledger.
fn finalize_acquisition(state: &Arc<AcquisitionState>) {
    // Check if actually complete BEFORE claiming the completed flag.
    // This prevents the tick→finalize→reset→tick loop.
    {
        let mutable = state.mutable.lock().expect("acq mutable lock (pre-check)");
        let ready = mutable.inbound.is_complete()
            || (mutable.inbound.planner_state().have_header
                && mutable.inbound.planner_state().have_state
                && mutable.inbound.planner_state().have_transactions);
        if !ready || mutable.inbound.is_failed() {
            return;
        }
    }

    if state.completed.swap(true, Ordering::AcqRel) {
        return; // Already finalized
    }

    let mut mutable = state.mutable.lock().expect("acq mutable lock (finalize)");
    let journal = WorkerJournal;
    let config = ledger::LedgerConfig::default();
    let seq = state.seq;

    let family = SHAMapFamily::new(
        state.shared_tree_cache.clone(),
        &state.worker_full_below,
        WorkerNodeFetcher {
            node_store: state.node_store.clone(),
            pending_writes: Arc::clone(&state.shared_pending_writes),
        },
        NullMissingNodeReporter,
    );

    if !mutable.inbound.is_complete() {
        if mutable.inbound.planner_state().have_header
            && mutable.inbound.planner_state().have_state
            && mutable.inbound.planner_state().have_transactions
        {
            mutable.inbound.set_complete();
        } else {
            // Pre-check should have caught this — should not reach here
            return;
        }
    }

    if mutable.inbound.is_failed() {
        return;
    }

    mutable
        .inbound
        .finish_if_done_with_family_and_config(&journal, &config, &family);
    if mutable.inbound.is_failed() {
        tracing::warn!(target: "inbound_ledger", seq, "Acquisition finalization failed");
        return;
    }

    let Some(mut ledger) = mutable.inbound.ledger().cloned() else {
        tracing::warn!(target: "inbound_ledger", seq, "Missing ledger on completion");
        return;
    };

    if !ledger.is_immutable() {
        ledger.set_immutable(true);
    }
    ledger.set_full();
    if !flush_writes(&mutable.store.write_tx) {
        tracing::warn!(target: "inbound_ledger", seq, "Flush failed on finalization");
        return;
    }
    state
        .shared_pending_writes
        .lock()
        .expect("pending writes lock")
        .clear();
    tracing::info!(
        target: "inbound_ledger",
        acq_cache_size = state.shared_tree_cache.size(),
        "LEDGER ACQUIRED: acquisition complete"
    );

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
                    )
                    .ok();
                }
            }
            let data = match &fetcher_ns {
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
            let obj = data?;
            shamap::nodes::tree_node::SHAMapTreeNode::make_from_prefix(obj.data(), hash).ok()
        }));
    }
    tracing::info!(target: "inbound_ledger", seq = ledger.header().seq, "LEDGER ACQUIRED");
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
                "LEDGER ACQUIRED: jemalloc stats after release"
            );
        }
    }
    let _ = state.store_tx.send(Arc::new(ledger));
}

// ─── Stale packet stashing ───────────────────────────────────────────────────

/// Stash account-state nodes from an unroutable packet into the fetch-pack.
/// Returns true if all nodes were successfully stashed.
pub fn stash_stale_packet<FP>(packet: &InboundLedgerPacket, stale_data_store: &mut FP) -> bool
where
    FP: FetchPackStore,
{
    for node in &packet.nodes {
        if node.node_id.is_none() {
            return false;
        }

        let Ok(Some(new_node)) =
            shamap::nodes::tree_node::SHAMapTreeNode::make_from_wire(&node.node_data)
        else {
            return false;
        };
        let Ok(prefixed) = new_node.serialize_with_prefix() else {
            return false;
        };

        stale_data_store.add_fetch_pack(*new_node.get_hash().as_uint256(), prefixed);
    }

    true
}
