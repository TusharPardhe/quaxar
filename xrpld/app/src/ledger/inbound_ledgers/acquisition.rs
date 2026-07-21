//! Per-hash inbound-ledger lifecycle.
//!
//! The structure follows rippled's `InboundLedger` and `TimeoutCounter`:
//! `init` checks local storage, adds peers, queues an immediate timeout job,
//! and every timeout job re-arms only its own three-second timer.

use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::{KeyCache, MonotonicClock};
use ledger::{
    FetchPackCache, FetchPackContainer, FetchPackStore, InboundLedgerJournal, InboundLedgerLocal,
    InboundLedgerPacket, InboundLedgerReason, InboundLedgerReceivedPacket,
    InboundLedgerRequestTrigger, InboundLedgerStore, InboundLedgerTimerResult, Ledger,
};
use overlay::{Peer, PeerSet as _};
use shamap::family::{FullBelowCacheImpl, NullMissingNodeReporter, SHAMapFamily};
use shamap::tree_node_cache::TreeNodeCache;
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

/// Per-ledger state owned by the registry.
pub struct AcquisitionState {
    pub data_buffer: Mutex<Vec<(u64, InboundLedgerPacket)>>,
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
    mutable: &mut AcqMutableState,
    reason: InboundLedgerRequestTrigger,
    peer: Option<Arc<dyn Peer>>,
) {
    let AcqMutableState {
        inbound,
        store,
        fetch_pack,
    } = mutable;
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

fn add_peers(state: &AcquisitionState, mutable: &mut AcqMutableState) {
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
    if state.reason == AcquireReason::History {
        return;
    }
    for peer in added {
        trigger(
            state,
            mutable,
            InboundLedgerRequestTrigger::Added,
            Some(peer),
        );
    }
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
    add_peers(state, &mut mutable);
    drop(mutable);
    state.queue_timeout_job();
}

fn process_data_job(state: &Arc<AcquisitionState>) {
    if state.is_done() {
        state.data_job_queued.store(false, Ordering::Release);
        return;
    }

    let packets = {
        let mut buffer = state
            .data_buffer
            .lock()
            .expect("acquisition data buffer lock");
        std::mem::take(&mut *buffer)
    };
    let mut mutable = state.mutable.lock().expect("acquisition mutable lock");
    for (peer_id, packet) in packets {
        mutable.inbound.got_data(Some(peer_id), packet);
    }

    if state.fetch_pack_ready.swap(false, Ordering::AcqRel) {
        check_local(state, &mut mutable);
    }
    if mutable.inbound.is_failed() {
        drop(mutable);
        state.mark_failed();
        state.data_job_queued.store(false, Ordering::Release);
        return;
    }
    if mutable.inbound.is_complete() {
        drop(mutable);
        finalize_acquisition(state);
        state.data_job_queued.store(false, Ordering::Release);
        return;
    }

    let journal = WorkerJournal;
    let config = ledger::LedgerConfig::default();
    let family = family(state);
    let data_buffer = &state.data_buffer;
    let AcqMutableState {
        inbound,
        store,
        fetch_pack,
    } = &mut *mutable;
    let result = inbound.run_data_with_family_and_config_and_refill(
        &journal,
        &config,
        store,
        fetch_pack,
        &family,
        &mut || {
            let mut buffer = data_buffer.lock().expect("acquisition data refill lock");
            std::mem::take(&mut *buffer)
                .into_iter()
                .map(|(peer_id, packet)| InboundLedgerReceivedPacket::new(Some(peer_id), packet))
                .collect()
        },
    );

    // `InboundLedger::processData` charges only structurally malformed active
    // packets. Invalid or duplicate SHAMap content remains uncharged.
    for (peer_id, packet_type, error) in &result.malformed_packets {
        let Some(peer) = state.peer_set.find_peer(*peer_id as u32) else {
            continue;
        };
        let context = match (packet_type, error) {
            (ledger::InboundLedgerDataType::Base, ledger::InboundLedgerPacketError::EmptyNodes) => {
                "ledger_data empty header"
            }
            (_, ledger::InboundLedgerPacketError::EmptyNodes) => "ledger_data no nodes",
            (_, ledger::InboundLedgerPacketError::InvalidHeader) => "ledger_data invalid header",
            (_, ledger::InboundLedgerPacketError::MissingNodeId) => "ledger_data bad node",
        };
        peer.charge(
            (*resource::FEE_MALFORMED_REQUEST).clone(),
            context.to_owned(),
        );
    }

    // `InboundLedger::runData` samples useful peers and immediately triggers
    // each selected peer. It does not clear recentNodes here.
    for peer_id in result.triggered_peer_ids {
        let Some(peer) = state.peer_set.find_peer(peer_id as u32) else {
            continue;
        };
        let reason = if peer.is_high_latency() {
            InboundLedgerRequestTrigger::ReplyHighLatency
        } else {
            InboundLedgerRequestTrigger::Reply
        };
        trigger(state, &mut mutable, reason, Some(peer));
    }

    let complete = mutable.inbound.is_complete();
    let failed = mutable.inbound.is_failed();
    drop(mutable);
    if failed {
        state.mark_failed();
    } else if complete {
        finalize_acquisition(state);
    }

    state.data_job_queued.store(false, Ordering::Release);
    if !state.is_done()
        && !state
            .data_buffer
            .lock()
            .expect("acquisition data buffer lock")
            .is_empty()
    {
        state.submit_data_job();
    }
}

fn process_timeout_job(state: &Arc<AcquisitionState>) {
    if state.is_done() {
        return;
    }

    let mut mutable = state.mutable.lock().expect("acquisition mutable lock");
    let timer_result = mutable.inbound.timeout_expired();
    match timer_result {
        InboundLedgerTimerResult::Progress => {}
        InboundLedgerTimerResult::Done => {
            drop(mutable);
            finalize_acquisition(state);
            return;
        }
        InboundLedgerTimerResult::Failed => {
            drop(mutable);
            state.mark_failed();
            return;
        }
        InboundLedgerTimerResult::NoProgress => {
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

            mutable.inbound.set_by_hash(true);
            if state.reason != AcquireReason::History {
                trigger(
                    state,
                    &mut mutable,
                    InboundLedgerRequestTrigger::Timeout,
                    None,
                );
            }
            add_peers(state, &mut mutable);
            if state.reason == AcquireReason::History {
                trigger(
                    state,
                    &mut mutable,
                    InboundLedgerRequestTrigger::Timeout,
                    None,
                );
            }
        }
    }

    drop(mutable);
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
