//! Per-hash inbound-ledger lifecycle.
//!
//! The structure follows rippled's `InboundLedger` and `TimeoutCounter`:
//! `init` checks local storage, adds peers, queues an immediate timeout job,
//! and every timeout job re-arms only its own three-second timer.

use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::random::rand_int_to;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::{KeyCache, MonotonicClock};
use ledger::ledger_fetcher::INBOUND_LEDGER_MAX_PACKET_NODES_PER_STEP;
use ledger::{
    FetchPackCache, FetchPackContainer, FetchPackStore, INBOUND_LEDGER_MAX_USEFUL_PEERS,
    InboundLedgerJournal, InboundLedgerLocal, InboundLedgerPacket, InboundLedgerPacketError,
    InboundLedgerReason, InboundLedgerRequestTrigger, InboundLedgerStore, InboundLedgerTimerResult,
    Ledger,
};
use overlay::{Peer, PeerSet as _};
use shamap::family::{FullBelowCacheImpl, NullMissingNodeReporter, SHAMapFamily};
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

use super::registry::AcquireReason;
use super::worker_pool::WorkerPool;

const PEER_COUNT_START: usize = 5;
const PEER_COUNT_ADD: usize = 3;
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);

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
        }?;
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

/// Synchronous node-store adapter. `SHAMapSyncFilter::got_node` does not return
/// until the accepted node is durable, matching rippled's `db_.store` call.
pub struct WorkerStore {
    node_store: SHAMapStoreNodeStore,
    shared_stored: Arc<KeyCache<Uint256>>,
}

impl WorkerStore {
    fn sync(&self) {
        match &self.node_store {
            SHAMapStoreNodeStore::Single(db) => db.sync(),
            SHAMapStoreNodeStore::Rotating(db) => db.sync(),
        }
    }

    fn store_object(
        &mut self,
        object_type: nodestore::NodeObjectType,
        data: Vec<u8>,
        hash: Uint256,
        seq: u32,
    ) {
        if !self.shared_stored.insert(hash) {
            return;
        }
        match &self.node_store {
            SHAMapStoreNodeStore::Single(db) => db.store(object_type, data, hash, seq),
            SHAMapStoreNodeStore::Rotating(db) => db.store(object_type, data, hash, seq),
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
        }?;
        Some(fetched.data().to_vec())
    }
}

pub struct AcqMutableState {
    pub inbound: InboundLedgerLocal,
    pub store: WorkerStore,
    pub(crate) fetch_pack: WorkerFetchPack,
}

struct ActivePacket {
    peer_id: u64,
    packet: InboundLedgerPacket,
    next_node: usize,
    stats: shamap::sync::SHAMapAddNode,
}

#[derive(Default)]
struct ReceivedDataState {
    active_packet: Option<ActivePacket>,
    peer_counts: BTreeMap<u64, i32>,
    max_useful_count: i32,
}

impl ReceivedDataState {
    fn record_packet(&mut self, peer_id: u64, useful_count: i32) {
        if useful_count <= 0 {
            return;
        }
        self.max_useful_count = self.max_useful_count.max(useful_count);
        self.peer_counts
            .entry(peer_id)
            .and_modify(|count| *count = (*count).max(useful_count))
            .or_insert(useful_count);
    }

    fn take_reply_peers(&mut self) -> Vec<u64> {
        let threshold = self.max_useful_count / 2;
        let mut peers: Vec<_> = self
            .peer_counts
            .iter()
            .filter_map(|(&peer_id, &count)| (count >= threshold).then_some(peer_id))
            .collect();
        while peers.len() > INBOUND_LEDGER_MAX_USEFUL_PEERS {
            peers.swap_remove(rand_int_to(peers.len() - 1));
        }
        self.peer_counts.clear();
        self.max_useful_count = 0;
        peers
    }
}

/// Per-ledger state owned by the registry.
pub struct AcquisitionState {
    pub data_buffer: Mutex<Vec<(u64, InboundLedgerPacket)>>,
    received_data: Mutex<ReceivedDataState>,
    pub mutable: Mutex<AcqMutableState>,
    pub hash: SHAMapHash,
    pub seq: u32,
    pub reason: AcquireReason,
    pub peer_set: overlay::SimplePeerSet,
    pub worker_full_below: FullBelowCacheImpl<MonotonicClock, HardenedHashBuilder>,
    pub node_store: SHAMapStoreNodeStore,
    pub shared_tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
    pub store_tx: std::sync::mpsc::Sender<Arc<Ledger>>,
    pub stopped: AtomicBool,
    pub completed: AtomicBool,
    pub failed: AtomicBool,
    pub fetch_pack_ready: AtomicBool,
    data_job_queued: AtomicBool,
    timer_armed: AtomicBool,
    worker_pool: Arc<WorkerPool>,
}

impl AcquisitionState {
    /// Perform `InboundLedger::init`: try local storage, add peers, then queue
    /// the immediate TimeoutCounter job.
    pub fn start(self: &Arc<Self>) {
        process_init(self);
    }

    /// Equivalent to `InboundLedger::gotData` dispatch coalescing.
    pub fn submit_data_job(self: &Arc<Self>) {
        if self.is_done()
            || self
                .data_job_queued
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
        let state = Arc::clone(self);
        self.worker_pool
            .submit_ledger_data(Box::new(move || process_data_job(&state)));
    }

    fn queue_timeout_job(self: &Arc<Self>) {
        if self.is_done() {
            return;
        }
        let state = Arc::clone(self);
        if !self
            .worker_pool
            .try_submit_timeout(Box::new(move || process_timeout_job(&state)))
        {
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

    fn is_done(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
            || self.completed.load(Ordering::Acquire)
            || self.failed.load(Ordering::Acquire)
    }

    fn mark_failed(&self) {
        self.failed.store(true, Ordering::Release);
        self.stopped.store(true, Ordering::Release);
    }

    fn take_next_packet(&self) -> Option<ActivePacket> {
        let mut received = self
            .received_data
            .lock()
            .expect("acquisition received data lock");
        if let Some(packet) = received.active_packet.take() {
            return Some(packet);
        }
        let mut buffer = self
            .data_buffer
            .lock()
            .expect("acquisition data buffer lock");
        let (peer_id, packet) = (!buffer.is_empty()).then(|| buffer.remove(0))?;
        Some(ActivePacket {
            peer_id,
            packet,
            next_node: 0,
            stats: shamap::sync::SHAMapAddNode::default(),
        })
    }

    fn resume_packet(&self, packet: ActivePacket) {
        self.received_data
            .lock()
            .expect("acquisition received data lock")
            .active_packet = Some(packet);
    }

    fn finish_packet(&self, packet: ActivePacket, useful_count: Option<i32>) -> Vec<u64> {
        let mut received = self
            .received_data
            .lock()
            .expect("acquisition received data lock");
        if let Some(useful_count) = useful_count {
            received.record_packet(packet.peer_id, useful_count);
        }
        if !self
            .data_buffer
            .lock()
            .expect("acquisition data buffer lock")
            .is_empty()
        {
            return Vec::new();
        }
        received.take_reply_peers()
    }

    fn has_pending_data(&self) -> bool {
        self.received_data
            .lock()
            .expect("acquisition received data lock")
            .active_packet
            .is_some()
            || !self
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

    pub(crate) fn update_seq(&self, seq: u32) {
        let mut mutable = self.mutable.lock().expect("acquisition mutable lock");
        mutable.inbound.update(seq, time::Duration::ZERO);
    }

    pub(crate) fn completed_ledger(&self) -> Option<Arc<Ledger>> {
        self.mutable
            .lock()
            .expect("acquisition mutable lock")
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
    pub shared_stored: Arc<KeyCache<Uint256>>,
    pub store_tx: std::sync::mpsc::Sender<Arc<Ledger>>,
    pub full_below_generation: u32,
    pub worker_pool: Arc<WorkerPool>,
    pub initial_peers: Vec<Arc<dyn Peer>>,
}

impl AcquisitionBuilder {
    pub fn build(self) -> Arc<AcquisitionState> {
        let reason = match self.reason {
            AcquireReason::History => InboundLedgerReason::History,
            AcquireReason::Generic => InboundLedgerReason::Generic,
            AcquireReason::Consensus => InboundLedgerReason::Consensus,
        };
        Arc::new(AcquisitionState {
            data_buffer: Mutex::new(Vec::new()),
            received_data: Mutex::new(ReceivedDataState::default()),
            mutable: Mutex::new(AcqMutableState {
                inbound: InboundLedgerLocal::new_with_reason(self.hash, self.seq, reason),
                store: WorkerStore {
                    node_store: self.node_store.clone(),
                    shared_stored: self.shared_stored,
                },
                fetch_pack: WorkerFetchPack {
                    cache: self.fetch_pack,
                },
            }),
            hash: self.hash,
            seq: self.seq,
            reason: self.reason,
            peer_set: overlay::SimplePeerSet::new(self.initial_peers),
            worker_full_below: FullBelowCacheImpl::new(
                self.full_below_generation,
                MonotonicClock::default(),
                HardenedHashBuilder::default(),
                524_288,
            ),
            node_store: self.node_store,
            shared_tree_cache: self.tree_cache,
            store_tx: self.store_tx,
            stopped: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            fetch_pack_ready: AtomicBool::new(false),
            data_job_queued: AtomicBool::new(false),
            timer_armed: AtomicBool::new(false),
            worker_pool: self.worker_pool,
        })
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
        },
        NullMissingNodeReporter,
    )
}

fn trigger(
    state: &AcquisitionState,
    reason: InboundLedgerRequestTrigger,
    peer: Option<Arc<dyn Peer>>,
) {
    let mut mutable = state.mutable.lock().expect("acquisition mutable lock");
    let AcqMutableState {
        inbound,
        store,
        fetch_pack,
    } = &mut *mutable;
    let journal = WorkerJournal;
    let config = ledger::LedgerConfig::default();
    let mut send = |message: overlay::ProtocolMessage| {
        state.peer_set.send_request(&message, peer.as_ref());
    };
    let family = family(state);
    inbound.trigger_with_family(
        reason, &journal, &config, store, fetch_pack, &family, &mut send,
    );
}

fn add_peers(state: &AcquisitionState) -> Vec<Arc<dyn Peer>> {
    let limit = if state.peer_set.peer_count() == 0 {
        PEER_COUNT_START
    } else {
        PEER_COUNT_ADD
    };
    let hash = *state.hash.as_uint256();
    let mut added = Vec::new();
    state.peer_set.add_peers(
        limit,
        &mut |peer| peer.has_ledger(hash, state.seq),
        &mut |peer| added.push(Arc::clone(peer)),
    );
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
        let mut mutable = state.mutable.lock().expect("acquisition mutable lock");
        check_local(state, &mut mutable);
        if mutable.inbound.is_failed() {
            drop(mutable);
            state.mark_failed();
            return;
        }
        if mutable.inbound.is_complete() {
            drop(mutable);
            finalize_acquisition(state);
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

    let mut mutable = state.mutable.lock().expect("acquisition mutable lock");
    if state.fetch_pack_ready.swap(false, Ordering::AcqRel) {
        check_local(state, &mut mutable);
    }
    if mutable.inbound.is_failed() {
        drop(mutable);
        state.mark_failed();
        state.finish_data_job();
        return;
    }
    if mutable.inbound.is_complete() {
        drop(mutable);
        finalize_acquisition(state);
        state.finish_data_job();
        return;
    }

    let Some(mut active) = state.take_next_packet() else {
        drop(mutable);
        state.finish_data_job();
        return;
    };

    let journal = WorkerJournal;
    let config = ledger::LedgerConfig::default();
    let family = family(state);
    let step = {
        let AcqMutableState {
            inbound,
            store,
            fetch_pack,
        } = &mut *mutable;
        inbound.process_packet_step_with_family_and_config(
            &active.packet,
            active.next_node,
            INBOUND_LEDGER_MAX_PACKET_NODES_PER_STEP,
            &journal,
            &config,
            store,
            fetch_pack,
            &family,
        )
    };

    let reply_peers = match step {
        Ok(step) => {
            active.next_node = step.next_node;
            mutable.inbound.record_packet_progress(step.stats);
            active.stats += step.stats;
            if step.complete {
                let useful_count = active.stats.get_good();
                mutable.inbound.record_packet_stats_with_family_and_config(
                    active.stats,
                    &journal,
                    &config,
                    &family,
                );
                state.finish_packet(active, Some(useful_count))
            } else {
                state.resume_packet(active);
                Vec::new()
            }
        }
        Err(error) => {
            charge_malformed_packet(state, active.peer_id, active.packet.packet_type, error);
            state.finish_packet(active, None)
        }
    };

    let reply_requests: Vec<_> = reply_peers
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
    let complete = mutable.inbound.is_complete();
    let failed = mutable.inbound.is_failed();
    drop(mutable);
    if failed {
        state.mark_failed();
    } else if complete {
        finalize_acquisition(state);
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
        (_, InboundLedgerPacketError::InvalidHeader) => "ledger_data invalid header",
        (_, InboundLedgerPacketError::MissingNodeId) => "ledger_data bad node",
    };
    peer.charge(
        (*resource::FEE_MALFORMED_REQUEST).clone(),
        context.to_owned(),
    );
}

fn process_timeout_job(state: &Arc<AcquisitionState>) {
    if state.is_done() {
        return;
    }

    let mut added = Vec::new();
    let mut retry = false;
    let mut finalize = false;
    let mut failed = false;
    {
        let mut mutable = state.mutable.lock().expect("acquisition mutable lock");
        match mutable.inbound.timeout_expired() {
            InboundLedgerTimerResult::Progress => {}
            InboundLedgerTimerResult::Done => finalize = true,
            InboundLedgerTimerResult::Failed => failed = true,
            InboundLedgerTimerResult::NoProgress => {
                check_local(state, &mut mutable);
                if mutable.inbound.is_failed() {
                    failed = true;
                } else if mutable.inbound.is_complete() {
                    finalize = true;
                } else {
                    mutable.inbound.set_by_hash(true);
                    added = add_peers(state);
                    retry = true;
                }
            }
        }
    }

    if failed {
        state.mark_failed();
        return;
    }
    if finalize {
        finalize_acquisition(state);
        return;
    }
    if retry {
        if state.reason != AcquireReason::History {
            trigger(state, InboundLedgerRequestTrigger::Timeout, None);
            for peer in added {
                trigger(state, InboundLedgerRequestTrigger::Added, Some(peer));
            }
        } else {
            trigger(state, InboundLedgerRequestTrigger::Timeout, None);
        }
    }
    state.arm_timer();
}

fn finalize_acquisition(state: &Arc<AcquisitionState>) {
    if state.is_done() {
        return;
    }
    let mut mutable = state.mutable.lock().expect("acquisition mutable lock");
    if mutable.inbound.is_failed() || !mutable.inbound.is_complete() {
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
        state.mark_failed();
        return;
    }
    let Some(mut ledger) = mutable.inbound.ledger().cloned() else {
        return;
    };
    // All SHAMap nodes arrived through `WorkerStore`, but NuDB can keep their
    // bucket metadata in its active burst until a checkpoint. Commit that
    // burst before publishing this ledger's SQL header through `setFullLedger`.
    // Otherwise a graceful restart can see the header while neither map root
    // is findable by hash in the NodeStore.
    mutable.store.sync();
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

    if state.completed.swap(true, Ordering::AcqRel) {
        return;
    }
    tracing::info!(target: "inbound_ledger", seq = ledger.header().seq, "LEDGER ACQUIRED");
    let _ = state.store_tx.send(Arc::new(ledger));
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
