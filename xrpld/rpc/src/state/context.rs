//! JSON RPC context and runtime seams aligned with `xrpld/rpc/Context.h`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread;
use std::time::{Duration as StdDuration, Instant};

use app::{ApplicationRoot, JobType, NetworkOpsOperatingMode, ServiceRegistry};
use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::random::rand_int_to;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::MonotonicClock;
use ledger::{
    AccountStateSF, FetchPackCache, FetchPackContainer, InboundLedgerDataType, InboundLedgerLocal,
    InboundLedgerNodeData, InboundLedgerObjectType, InboundLedgerPacket, InboundLedgerPlannerState,
    InboundLedgerReason, InboundLedgerStore, LedgerConfig, LedgerPersistence,
    LedgerSyncFilterStore, LedgerTxReadError, TransactionStateSF,
    make_inbound_needed_by_hash_request,
};
use nodestore::{FetchType, NodeObjectType as NodeStoreObjectType};
use overlay::{
    Message, Overlay, Peer, ProtocolMessage, ProtocolPayload, TmGetLedger, TmGetObjectByHash,
    TmLedgerData,
};
use protocol::{JsonValue, fee_settings_keylet, sha512_half};
use shamap::family::{
    FullBelowCacheImpl, JournalLevel as SHAMapJournalLevel, NullMissingNodeReporter, SHAMapFamily,
    SHAMapJournal, SHAMapNodeFetcher,
};
use shamap::fetch::SHAMapSyncFilter;
use shamap::node_id::SHAMapNodeId;
use shamap::node_object::NodeObject as SHAMapNodeObject;
use shamap::storage::NodeObjectType as SHAMapNodeObjectType;
use shamap::sync::SHAMapType;
use shamap::traversal::TraversalError;
use shamap::tree_node::SHAMapTreeNode;
use shamap::tree_node_cache::TreeNodeCache;
use time::Duration;
use xrpl_core::PeerReservation;

use crate::state::role::Role;
use crate::state::tuning::Tuning;
use crate::status::{RpcErrorCode, Status};
use crate::{InfoSub, WsInfoSub};

#[derive(Debug, Default)]
struct RpcInboundLedgerJournal;

impl ledger::InboundLedgerJournal for RpcInboundLedgerJournal {
    fn trace(&self, message: &str) {
        tracing::trace!(target: "rpc", "[ledger_request] {message}");
    }

    fn debug(&self, message: &str) {
        tracing::debug!(target: "rpc", "[ledger_request] {message}");
    }

    fn warn(&self, message: &str) {
        tracing::warn!(target: "rpc", "[ledger_request] {message}");
    }

    fn fatal(&self, message: &str) {
        tracing::error!(target: "rpc", "[ledger_request][fatal] {message}");
    }
}

#[derive(Debug, Default)]
struct RpcLedgerWalkJournal;

impl ledger::LedgerJournal for RpcLedgerWalkJournal {
    fn info(&self, message: &str) {
        tracing::info!(target: "rpc", "[ledger_request][walk] {message}");
    }

    fn warn(&self, message: &str) {
        tracing::warn!(target: "rpc", "[ledger_request][walk] {message}");
    }
}

const NODE_FAMILY_FULL_BELOW_TARGET_SIZE: usize = 524_288;
const NODE_FAMILY_FULL_BELOW_EXPIRATION: Duration = Duration::minutes(10);

#[derive(Debug, Default)]
struct RpcSHAMapJournal;

impl SHAMapJournal for RpcSHAMapJournal {
    fn log(&self, level: SHAMapJournalLevel, message: &str) {
        match level {
            SHAMapJournalLevel::Trace => {
                tracing::trace!(target: "rpc", "[ledger_request][shamap] {message}")
            }
            SHAMapJournalLevel::Debug => {
                tracing::debug!(target: "rpc", "[ledger_request][shamap] {message}")
            }
            SHAMapJournalLevel::Info => {
                tracing::info!(target: "rpc", "[ledger_request][shamap] {message}")
            }
            SHAMapJournalLevel::Warn => {
                tracing::warn!(target: "rpc", "[ledger_request][shamap] {message}")
            }
            SHAMapJournalLevel::Error => {
                tracing::error!(target: "rpc", "[ledger_request][shamap] {message}")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JsonContextHeaders<'a> {
    pub user: &'a str,
    pub forwarded_for: &'a str,
}

pub struct JsonContext<'a, Env> {
    pub params: &'a JsonValue,
    pub env: &'a Env,
    pub role: Role,
    pub api_version: u32,
    pub headers: JsonContextHeaders<'a>,
    pub unlimited: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RpcLoadType {
    #[default]
    Reference,
    MediumBurden,
    HeavyBurden,
    Exception,
}

const LEDGER_REQUEST_POLL_INTERVAL: StdDuration = StdDuration::from_millis(250);
const LEDGER_REQUEST_TIMEOUT: StdDuration = StdDuration::from_secs(60);
const LEDGER_REQUEST_RETRY_INTERVAL: StdDuration = StdDuration::from_secs(1);
const LEDGER_REQUEST_ROOT_OBJECT_INTERVAL: StdDuration = StdDuration::from_secs(3);
const LEDGER_REQUEST_NEEDED_BY_HASH_INTERVAL: StdDuration = StdDuration::from_secs(1);
const LEDGER_REQUEST_PEER_COUNT_START: usize = 5;
const LEDGER_REQUEST_PEER_COUNT_ADD: usize = 3;
const LEDGER_REQUEST_REPLY_QUERY_DEPTH: u32 = 1;
const LEDGER_REQUEST_HIGH_LATENCY_REPLY_QUERY_DEPTH: u32 = 2;
const LEDGER_REQUEST_TIMEOUT_QUERY_DEPTH: u32 = 0;
#[cfg(test)]
const LEDGER_REQUEST_TIMEOUT_NO_PROGRESS_CYCLES: u32 = 4;
const LEDGER_REQUEST_TIMEOUT_MODE_STALL: StdDuration = StdDuration::from_secs(3);
#[cfg(test)]
const LEDGER_REQUEST_AGGRESSIVE_BY_HASH_AFTER: StdDuration = StdDuration::from_secs(15);
const LEDGER_REQUEST_BECOME_AGGRESSIVE_TIMEOUTS: u32 = 4;
const TM_QUERY_INDIRECT: i32 = 0;
fn next_missing_node_scan_first_child() -> u8 {
    rand_int_to(255u8)
}

#[cfg(test)]
fn use_timeout_mode(no_progress_cycles: u32, stalled_for: StdDuration) -> bool {
    stalled_for >= LEDGER_REQUEST_TIMEOUT_MODE_STALL
        || no_progress_cycles >= LEDGER_REQUEST_TIMEOUT_NO_PROGRESS_CYCLES
}

#[cfg(test)]
fn use_aggressive_by_hash(stalled_for: StdDuration) -> bool {
    stalled_for >= LEDGER_REQUEST_AGGRESSIVE_BY_HASH_AFTER
}

fn get_ledger_query_type(timeout_mode: bool) -> Option<i32> {
    timeout_mode.then_some(TM_QUERY_INDIRECT)
}

fn use_reply_trigger(has_reply_peers: bool, timeout_mode: bool) -> bool {
    has_reply_peers && !timeout_mode
}

fn use_blind_request_trigger(
    reply_trigger: bool,
    timeout_mode: bool,
    peer_count: usize,
    last_blind_request_peer_count: usize,
) -> bool {
    reply_trigger || timeout_mode || peer_count > last_blind_request_peer_count
}

fn use_aggressive_by_hash_timeout(
    timeout_count: u32,
    timeout_request_pending: bool,
    by_hash_armed: bool,
) -> bool {
    timeout_request_pending
        && by_hash_armed
        && timeout_count > LEDGER_REQUEST_BECOME_AGGRESSIVE_TIMEOUTS
}

fn target_made_progress(
    planner_before: InboundLedgerPlannerState,
    planner_after: InboundLedgerPlannerState,
    max_useful_packet_count: i32,
) -> bool {
    planner_after != planner_before || max_useful_packet_count > 0
}

fn should_run_local_check(local_check_pending: bool) -> bool {
    local_check_pending
}

fn should_arm_timeout_local_check(progress_in_timeout_window: bool) -> bool {
    !progress_in_timeout_window
}

fn can_send_primary_request(
    reply_trigger: bool,
    last_request_at: Instant,
    retry_interval: StdDuration,
) -> bool {
    reply_trigger || last_request_at.elapsed() >= retry_interval
}

fn peer_has_acquisition_target(
    peer: &Arc<dyn Peer>,
    target: LedgerRequestTarget,
    known_sequence: u32,
) -> bool {
    match target {
        LedgerRequestTarget::Seq(sequence) => peer.has_ledger(Uint256::zero(), sequence),
        LedgerRequestTarget::Hash(hash) => peer.has_ledger(*hash.as_uint256(), known_sequence),
    }
}

fn select_acquisition_peer_ids(
    peers: &[Arc<dyn Peer>],
    target: LedgerRequestTarget,
    known_sequence: u32,
    tracked_peer_ids: &HashSet<u64>,
    limit: usize,
) -> Vec<u64> {
    let mut scored: Vec<(i32, u64)> = peers
        .iter()
        .filter(|peer| !tracked_peer_ids.contains(&u64::from(peer.id())))
        .map(|peer| {
            (
                peer.score(peer_has_acquisition_target(peer, target, known_sequence)),
                u64::from(peer.id()),
            )
        })
        .collect();
    scored.sort_by(|lhs, rhs| rhs.cmp(lhs));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, peer_id)| peer_id)
        .collect()
}

fn extend_acquisition_peer_set(
    peers: &[Arc<dyn Peer>],
    target: LedgerRequestTarget,
    known_sequence: u32,
    tracked_peer_ids: &mut HashSet<u64>,
    limit: usize,
) -> usize {
    let additions =
        select_acquisition_peer_ids(peers, target, known_sequence, tracked_peer_ids, limit);
    let added = additions.len();
    tracked_peer_ids.extend(additions);
    added
}

fn acquisition_peers(
    peers: &[Arc<dyn Peer>],
    tracked_peer_ids: &HashSet<u64>,
) -> Vec<Arc<dyn Peer>> {
    peers
        .iter()
        .filter(|peer| tracked_peer_ids.contains(&u64::from(peer.id())))
        .cloned()
        .collect()
}

fn take_resumable_inbound_ledger(
    app: &ApplicationRoot,
    target: LedgerRequestTarget,
) -> Option<InboundLedgerLocal> {
    let mut inbound_ledgers = app
        .inbound_ledgers()
        .lock()
        .expect("inbound ledgers mutex must not be poisoned");
    match target {
        LedgerRequestTarget::Seq(seq) => inbound_ledgers.remove_by_seq(seq),
        LedgerRequestTarget::Hash(hash) => inbound_ledgers.remove(hash),
    }
}

fn persist_resumable_inbound_ledger(app: &ApplicationRoot, inbound: InboundLedgerLocal) {
    if inbound.is_done() {
        return;
    }

    app.inbound_ledgers()
        .lock()
        .expect("inbound ledgers mutex must not be poisoned")
        .insert(inbound);
}

#[derive(Clone, Copy)]
enum LedgerRequestTarget {
    Seq(u32),
    Hash(SHAMapHash),
}

#[derive(Clone)]
struct RpcNodeStoreFetcher {
    node_store: app::SHAMapStoreNodeStore,
}

impl RpcNodeStoreFetcher {
    fn new(node_store: app::SHAMapStoreNodeStore) -> Self {
        Self { node_store }
    }
}

impl SHAMapNodeFetcher for RpcNodeStoreFetcher {
    fn fetch_node_object(&mut self, hash: SHAMapHash, ledger_seq: u32) -> Option<SHAMapNodeObject> {
        let fetched = match &self.node_store {
            app::SHAMapStoreNodeStore::Single(database) => database.fetch_node_object(
                hash.as_uint256(),
                ledger_seq,
                FetchType::Synchronous,
                false,
            ),
            app::SHAMapStoreNodeStore::Rotating(database) => database.fetch_node_object(
                hash.as_uint256(),
                ledger_seq,
                FetchType::Synchronous,
                false,
            ),
        }?;

        let object_type = match fetched.object_type() {
            NodeStoreObjectType::Ledger => SHAMapNodeObjectType::Ledger,
            NodeStoreObjectType::AccountNode => SHAMapNodeObjectType::AccountNode,
            NodeStoreObjectType::TransactionNode => SHAMapNodeObjectType::TransactionNode,
            NodeStoreObjectType::Unknown | NodeStoreObjectType::Dummy => {
                SHAMapNodeObjectType::Unknown
            }
        };

        Some(SHAMapNodeObject::new(
            object_type,
            fetched.data().to_vec(),
            *fetched.hash(),
        ))
    }
}

struct RpcInboundLedgerStore {
    node_store: app::SHAMapStoreNodeStore,
}

impl RpcInboundLedgerStore {
    fn new(node_store: app::SHAMapStoreNodeStore) -> Self {
        Self { node_store }
    }

    fn fetch_object(&self, hash: SHAMapHash, ledger_seq: u32) -> Option<Vec<u8>> {
        match &self.node_store {
            app::SHAMapStoreNodeStore::Single(database) => database
                .fetch_node_object(hash.as_uint256(), ledger_seq, FetchType::Synchronous, false)
                .map(|object| object.data().to_vec()),
            app::SHAMapStoreNodeStore::Rotating(database) => database
                .fetch_node_object(hash.as_uint256(), ledger_seq, FetchType::Synchronous, false)
                .map(|object| object.data().to_vec()),
        }
    }

    fn store_object(
        &self,
        object_type: NodeStoreObjectType,
        data: Vec<u8>,
        hash: Uint256,
        ledger_seq: u32,
    ) {
        match &self.node_store {
            app::SHAMapStoreNodeStore::Single(database) => {
                database.store(object_type, data, hash, ledger_seq)
            }
            app::SHAMapStoreNodeStore::Rotating(database) => {
                database.store(object_type, data, hash, ledger_seq)
            }
        }
    }
}

impl InboundLedgerStore for RpcInboundLedgerStore {
    fn fetch_ledger_header(&mut self, hash: SHAMapHash, ledger_seq: u32) -> Option<Vec<u8>> {
        self.fetch_object(hash, ledger_seq)
    }

    fn store_ledger_header(&mut self, data: Vec<u8>, hash: SHAMapHash, ledger_seq: u32) {
        self.store_object(
            NodeStoreObjectType::Ledger,
            data,
            *hash.as_uint256(),
            ledger_seq,
        );
    }

    fn store_shamap_node(
        &mut self,
        object_type: SHAMapNodeObjectType,
        data: Vec<u8>,
        hash: Uint256,
        ledger_seq: u32,
    ) {
        let mapped_type = match object_type {
            SHAMapNodeObjectType::Ledger => NodeStoreObjectType::Ledger,
            SHAMapNodeObjectType::AccountNode => NodeStoreObjectType::AccountNode,
            SHAMapNodeObjectType::TransactionNode => NodeStoreObjectType::TransactionNode,
            SHAMapNodeObjectType::Unknown | SHAMapNodeObjectType::Dummy => {
                NodeStoreObjectType::Unknown
            }
        };

        self.store_object(mapped_type, data, hash, ledger_seq);
    }
}

struct RpcSyncFilterStore<'a, DB: InboundLedgerStore + ?Sized>(&'a mut DB);

impl<DB> LedgerSyncFilterStore for RpcSyncFilterStore<'_, DB>
where
    DB: InboundLedgerStore + ?Sized,
{
    fn store_shamap_node(
        &mut self,
        object_type: SHAMapNodeObjectType,
        data: Vec<u8>,
        hash: Uint256,
        ledger_seq: u32,
    ) {
        InboundLedgerStore::store_shamap_node(self.0, object_type, data, hash, ledger_seq);
    }
}

fn send_message_to_selected_peers(
    peers: &[Arc<dyn Peer>],
    preferred_peer_ids: &[u64],
    message: &ProtocolMessage,
) -> bool {
    if preferred_peer_ids.is_empty() {
        return false;
    }

    let wrapped = Message::new(message.clone(), None);
    let mut sent = false;
    for peer in peers {
        if preferred_peer_ids.contains(&(u64::from(peer.id()))) {
            peer.send(wrapped.clone());
            sent = true;
        }
    }
    sent
}

fn broadcast_to_peers(peers: &[Arc<dyn Peer>], message: ProtocolMessage) {
    let wrapped = Message::new(message, None);
    for peer in peers {
        peer.send(wrapped.clone());
    }
}

fn send_to_peers(peers: &[Arc<dyn Peer>], preferred_peer_ids: &[u64], message: ProtocolMessage) {
    if send_message_to_selected_peers(peers, preferred_peer_ids, &message) {
        return;
    }

    broadcast_to_peers(peers, message);
}

fn set_get_ledger_query_depth(message: &mut ProtocolMessage, query_depth: u32) -> bool {
    let ProtocolPayload::GetLedger(body) = &mut message.payload else {
        return false;
    };
    body.query_depth = Some(query_depth);
    true
}

fn send_get_ledger_to_peers(
    peers: &[Arc<dyn Peer>],
    preferred_peer_ids: &[u64],
    message: ProtocolMessage,
    reply_trigger: bool,
) {
    if !reply_trigger || preferred_peer_ids.is_empty() {
        send_to_peers(peers, preferred_peer_ids, message);
        return;
    }

    let mut normal_peer_ids = Vec::new();
    let mut high_latency_peer_ids = Vec::new();
    for peer in peers {
        let peer_id = u64::from(peer.id());
        if !preferred_peer_ids.contains(&peer_id) {
            continue;
        }
        if peer.is_high_latency() {
            high_latency_peer_ids.push(peer_id);
        } else {
            normal_peer_ids.push(peer_id);
        }
    }

    let mut sent = false;
    if send_message_to_selected_peers(peers, &normal_peer_ids, &message) {
        sent = true;
    }

    if !high_latency_peer_ids.is_empty() {
        let mut high_latency_message = message.clone();
        if set_get_ledger_query_depth(
            &mut high_latency_message,
            LEDGER_REQUEST_HIGH_LATENCY_REPLY_QUERY_DEPTH,
        ) && send_message_to_selected_peers(peers, &high_latency_peer_ids, &high_latency_message)
        {
            sent = true;
        }
    }

    if !sent {
        send_to_peers(peers, preferred_peer_ids, message);
    }
}

fn send_reply_mode_requests_to_peers<BUILD>(
    peers: &[Arc<dyn Peer>],
    preferred_peer_ids: &[u64],
    mut build_message: BUILD,
) -> bool
where
    BUILD: FnMut(u32) -> Option<ProtocolMessage>,
{
    let mut sent = false;

    for preferred_peer_id in preferred_peer_ids {
        let Some(peer) = peers
            .iter()
            .find(|peer| u64::from(peer.id()) == *preferred_peer_id)
        else {
            continue;
        };

        let query_depth = if peer.is_high_latency() {
            LEDGER_REQUEST_HIGH_LATENCY_REPLY_QUERY_DEPTH
        } else {
            LEDGER_REQUEST_REPLY_QUERY_DEPTH
        };
        let Some(message) = build_message(query_depth) else {
            continue;
        };

        peer.send(Message::new(message, None));
        sent = true;
    }

    sent
}

fn record_requested_object_hashes(message: &ProtocolMessage, requested: &mut HashSet<Uint256>) {
    let ProtocolPayload::GetObjects(request) = &message.payload else {
        return;
    };
    for object in &request.objects {
        let Some(hash_bytes) = &object.hash else {
            continue;
        };
        let Some(hash) = Uint256::from_slice(hash_bytes) else {
            continue;
        };
        requested.insert(hash);
    }
}

fn summarize_unanswered_requested_hashes(
    requested: &HashSet<Uint256>,
    answered: &HashSet<Uint256>,
) -> (usize, String) {
    let mut count = 0usize;
    let mut sample = Vec::new();
    for hash in requested {
        if answered.contains(hash) {
            continue;
        }
        count += 1;
        if sample.len() < 3 {
            sample.push(hash.to_string());
        }
    }
    let sample = if sample.is_empty() {
        "none".to_owned()
    } else {
        sample.join(",")
    };
    (count, sample)
}

fn select_missing_node_ids_for_request(
    missing: Vec<(SHAMapNodeId, Uint256)>,
    allow_duplicate_only_request: bool,
    request_node_limit: usize,
    recent_node_requests: &mut HashSet<Uint256>,
) -> Vec<SHAMapNodeId> {
    let mut preferred_node_ids = Vec::new();
    let mut fallback_node_ids = Vec::new();

    for (node_id, hash) in missing {
        if preferred_node_ids.len() >= request_node_limit
            && fallback_node_ids.len() >= request_node_limit
        {
            break;
        }

        if recent_node_requests.contains(&hash) {
            if fallback_node_ids.len() < request_node_limit {
                fallback_node_ids.push((node_id, hash));
            }
        } else if preferred_node_ids.len() < request_node_limit {
            preferred_node_ids.push((node_id, hash));
        }
    }

    let selected_node_ids = if !preferred_node_ids.is_empty() {
        preferred_node_ids
    } else if allow_duplicate_only_request {
        fallback_node_ids
    } else {
        Vec::new()
    };

    for (_, hash) in &selected_node_ids {
        recent_node_requests.insert(*hash);
    }

    selected_node_ids
        .into_iter()
        .map(|(node_id, _)| node_id)
        .collect()
}

#[derive(Debug, Default, Clone, Copy)]
struct GetObjectsTypeCounts {
    ledger_messages: usize,
    tx_node_messages: usize,
    state_node_messages: usize,
    fetch_pack_messages: usize,
    unknown_messages: usize,
    ledger_objects: usize,
    tx_node_objects: usize,
    state_node_objects: usize,
    fetch_pack_objects: usize,
    unknown_objects: usize,
}

impl GetObjectsTypeCounts {
    fn record_message(&mut self, object_type: i32, object_count: usize) {
        match object_type {
            1 => {
                self.ledger_messages += 1;
                self.ledger_objects += object_count;
            }
            3 => {
                self.tx_node_messages += 1;
                self.tx_node_objects += object_count;
            }
            4 => {
                self.state_node_messages += 1;
                self.state_node_objects += object_count;
            }
            6 => {
                self.fetch_pack_messages += 1;
                self.fetch_pack_objects += object_count;
            }
            _ => {
                self.unknown_messages += 1;
                self.unknown_objects += object_count;
            }
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct GetLedgerRequestCounts {
    base_messages: usize,
    tx_messages: usize,
    state_messages: usize,
    tx_node_ids: usize,
    state_node_ids: usize,
}

impl GetLedgerRequestCounts {
    fn record_message(&mut self, request: &TmGetLedger) {
        match request.itype {
            0 => self.base_messages += 1,
            1 => {
                self.tx_messages += 1;
                self.tx_node_ids += request.node_i_ds.len();
            }
            2 => {
                self.state_messages += 1;
                self.state_node_ids += request.node_i_ds.len();
            }
            _ => {}
        }
    }
}

fn record_requested_get_ledger(message: &ProtocolMessage, counts: &mut GetLedgerRequestCounts) {
    let ProtocolPayload::GetLedger(request) = &message.payload else {
        return;
    };
    counts.record_message(request);
}

fn is_relevant_get_objects_type_for_inbound(object_type: i32) -> bool {
    matches!(object_type, 1 | 3 | 4 | 6)
}

fn map_ledger_data_type(kind: i32) -> Option<InboundLedgerDataType> {
    match kind {
        0 => Some(InboundLedgerDataType::Base),
        1 => Some(InboundLedgerDataType::TransactionNode),
        2 => Some(InboundLedgerDataType::StateNode),
        _ => None,
    }
}

fn inbound_packet_from_wire(message: &TmLedgerData) -> Option<(SHAMapHash, InboundLedgerPacket)> {
    let hash = SHAMapHash::new(Uint256::from_slice(&message.ledger_hash)?);
    let packet_type = map_ledger_data_type(message.r#type)?;
    if message.nodes.is_empty() {
        return None;
    }

    let nodes = message
        .nodes
        .iter()
        .map(|node| InboundLedgerNodeData::new(node.nodeid.clone(), node.nodedata.clone()))
        .collect();

    Some((hash, InboundLedgerPacket::new(packet_type, nodes)))
}

fn inbound_request_seq(inbound: &InboundLedgerLocal) -> Option<u32> {
    inbound
        .ledger()
        .map(|ledger| ledger.header().seq)
        .filter(|seq| *seq != 0)
        .or_else(|| (inbound.seq() != 0).then_some(inbound.seq()))
}

fn make_missing_node_request<CLOCK, S, C, F, MR, NS, DB, FP>(
    inbound: &mut InboundLedgerLocal,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    store: &mut DB,
    fetch_pack: &mut FP,
    query_depth: u32,
    query_type: Option<i32>,
    allow_duplicate_only_request: bool,
    request_node_limit: usize,
    recent_node_requests: &mut HashSet<Uint256>,
) -> Option<ProtocolMessage>
where
    CLOCK: basics::tagged_cache::CacheClock,
    S: std::hash::BuildHasher + Clone,
    C: shamap::family::FullBelowCache,
    F: shamap::family::SHAMapNodeFetcher,
    MR: shamap::family::MissingNodeReporter,
    DB: InboundLedgerStore,
    FP: FetchPackContainer,
{
    const MISSING_NODES_FIND: i32 = 256;

    let mut planner = inbound.planner_state();
    if !planner.have_header {
        return None;
    }

    let ledger_hash = inbound.hash().as_uint256().data().to_vec();
    let ledger_seq = inbound_request_seq(inbound);

    if !planner.have_state {
        let missing = {
            let ledger = inbound.ledger_mut()?;
            let mut state_filter =
                AccountStateSF::new(RpcSyncFilterStore(&mut *store), &mut *fetch_pack);
            let mut filter: Option<&mut dyn SHAMapSyncFilter> = Some(&mut state_filter);
            ledger.state_map_mut().get_missing_nodes_with_family(
                MISSING_NODES_FIND,
                &mut filter,
                family,
                &mut || next_missing_node_scan_first_child(),
            )
        };
        if !missing.is_empty() {
            let selected_node_ids = select_missing_node_ids_for_request(
                missing,
                allow_duplicate_only_request,
                request_node_limit,
                recent_node_requests,
            );
            if selected_node_ids.is_empty() {
                // Parity with reference trigger(): if AS nodes are all filtered as recent
                // duplicates for reply-mode, continue and allow TX path to request.
                // Timeout-mode will clear recent requests and allow duplicates.
            } else {
                return Some(ProtocolMessage::new(ProtocolPayload::GetLedger(
                    TmGetLedger {
                        itype: 2,
                        ltype: None,
                        ledger_hash: Some(ledger_hash),
                        ledger_seq,
                        node_i_ds: selected_node_ids
                            .into_iter()
                            .map(|node_id| node_id.get_raw_string())
                            .collect(),
                        request_cookie: None,
                        query_type,
                        query_depth: Some(query_depth),
                    },
                )));
            }
        }

        let _ = inbound.revalidate_map_sync_with_family(family);
        planner = inbound.planner_state();
    }

    if !planner.have_transactions {
        let missing = {
            let ledger = inbound.ledger_mut()?;
            let mut tx_filter =
                TransactionStateSF::new(RpcSyncFilterStore(&mut *store), &mut *fetch_pack);
            let mut filter: Option<&mut dyn SHAMapSyncFilter> = Some(&mut tx_filter);
            ledger.tx_map_mut().get_missing_nodes_with_family(
                MISSING_NODES_FIND,
                &mut filter,
                family,
                &mut || next_missing_node_scan_first_child(),
            )
        };
        if !missing.is_empty() {
            let selected_node_ids = select_missing_node_ids_for_request(
                missing,
                allow_duplicate_only_request,
                request_node_limit,
                recent_node_requests,
            );
            if selected_node_ids.is_empty() {
                return None;
            }
            return Some(ProtocolMessage::new(ProtocolPayload::GetLedger(
                TmGetLedger {
                    itype: 1,
                    ltype: None,
                    ledger_hash: Some(ledger_hash),
                    ledger_seq,
                    node_i_ds: selected_node_ids
                        .into_iter()
                        .map(|node_id| node_id.get_raw_string())
                        .collect(),
                    request_cookie: None,
                    query_type,
                    query_depth: Some(query_depth),
                },
            )));
        }
    }

    None
}

fn make_root_object_request(inbound: &mut InboundLedgerLocal) -> Option<ProtocolMessage> {
    let planner = inbound.planner_state();
    if !planner.have_header {
        return None;
    }

    let ledger = inbound.ledger_mut()?;
    let (object_type, root_hash, root_missing) = if !planner.have_state {
        (
            overlay::message::wire::tm_get_object_by_hash::ObjectType::OtStateNode as i32,
            ledger.header().account_hash,
            ledger.state_map_mut().hash().is_zero(),
        )
    } else if !planner.have_transactions {
        (
            overlay::message::wire::tm_get_object_by_hash::ObjectType::OtTransactionNode as i32,
            ledger.header().tx_hash,
            ledger.tx_map_mut().hash().is_zero(),
        )
    } else {
        return None;
    };

    if root_hash.is_zero() || !root_missing {
        return None;
    }
    let ledger_seq = inbound_request_seq(inbound);

    Some(ProtocolMessage::new(ProtocolPayload::GetObjects(
        TmGetObjectByHash {
            r#type: object_type,
            query: true,
            ledger_hash: Some(inbound.hash().as_uint256().data().to_vec()),
            fat: None,
            objects: vec![overlay::message::wire::TmIndexedObject {
                hash: Some(root_hash.as_uint256().data().to_vec()),
                index: None,
                data: None,
                node_id: None,
                ledger_seq,
            }],
        },
    )))
}

fn make_root_node_request(
    inbound: &mut InboundLedgerLocal,
    query_depth: u32,
    query_type: Option<i32>,
) -> Option<ProtocolMessage> {
    let planner = inbound.planner_state();
    if !planner.have_header {
        return None;
    }

    let ledger = inbound.ledger_mut()?;
    let itype = if !planner.have_state {
        if !ledger.state_map_mut().hash().is_zero() {
            return None;
        }
        2
    } else if !planner.have_transactions {
        if !ledger.tx_map_mut().hash().is_zero() {
            return None;
        }
        1
    } else {
        return None;
    };

    Some(ProtocolMessage::new(ProtocolPayload::GetLedger(
        TmGetLedger {
            itype,
            ltype: None,
            ledger_hash: Some(inbound.hash().as_uint256().data().to_vec()),
            ledger_seq: inbound_request_seq(inbound),
            node_i_ds: vec![SHAMapNodeId::default().get_raw_string()],
            request_cookie: None,
            query_type,
            query_depth: Some(query_depth),
        },
    )))
}

fn make_snapshot_missing_request<CLOCK, S, C, F, MR, NS, DB, FP>(
    inbound: &mut InboundLedgerLocal,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    store: &mut DB,
    fetch_pack: &mut FP,
) -> Option<ProtocolMessage>
where
    CLOCK: basics::tagged_cache::CacheClock,
    S: std::hash::BuildHasher + Clone,
    C: shamap::family::FullBelowCache,
    F: shamap::family::SHAMapNodeFetcher,
    MR: shamap::family::MissingNodeReporter,
    DB: InboundLedgerStore,
    FP: FetchPackContainer,
{
    let mut planner = inbound.planner_state();
    if !planner.have_state {
        let mut state_filter =
            AccountStateSF::new(RpcSyncFilterStore(&mut *store), &mut *fetch_pack);
        let mut state_filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut state_filter);
        let mut tx_filter_ref: Option<&mut dyn SHAMapSyncFilter> = None;
        if let Some(request) =
            inbound.make_needed_by_hash_request(&mut state_filter_ref, &mut tx_filter_ref, family)
        {
            return Some(request);
        }
        let _ = inbound.revalidate_map_sync_with_family(family);
        planner = inbound.planner_state();
    }

    if !planner.have_transactions {
        let mut tx_filter =
            TransactionStateSF::new(RpcSyncFilterStore(&mut *store), &mut *fetch_pack);
        let mut state_filter_ref: Option<&mut dyn SHAMapSyncFilter> = None;
        let mut tx_filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut tx_filter);
        return inbound.make_needed_by_hash_request(
            &mut state_filter_ref,
            &mut tx_filter_ref,
            family,
        );
    }

    None
}

fn make_walk_missing_hash_request<CLOCK, S, C, F, MR, NS>(
    inbound: &mut InboundLedgerLocal,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
) -> Option<ProtocolMessage>
where
    CLOCK: basics::tagged_cache::CacheClock,
    S: std::hash::BuildHasher + Clone,
    C: shamap::family::FullBelowCache,
    F: shamap::family::SHAMapNodeFetcher,
    MR: shamap::family::MissingNodeReporter,
{
    const WALK_MISSING_FIND: i32 = 32;
    const WALK_MISSING_REQUEST_LIMIT: usize = 32;

    let planner = inbound.planner_state();
    if !planner.have_header {
        return None;
    }
    let ledger = inbound.ledger_mut()?;
    let mut needed = Vec::<(InboundLedgerObjectType, Uint256)>::new();

    if !planner.have_state {
        let mut missing = Vec::new();
        ledger.state_map().walk_map_with_family(
            SHAMapType::State,
            &mut missing,
            WALK_MISSING_FIND,
            family,
        );
        for node in missing {
            let Some(hash) = node.hash() else {
                continue;
            };
            needed.push((InboundLedgerObjectType::StateNode, *hash.as_uint256()));
            if needed.len() >= WALK_MISSING_REQUEST_LIMIT {
                break;
            }
        }
    }

    if needed.is_empty() && !planner.have_transactions {
        let mut missing = Vec::new();
        ledger.tx_map().walk_map_with_family(
            SHAMapType::Transaction,
            &mut missing,
            WALK_MISSING_FIND,
            family,
        );
        for node in missing {
            let Some(hash) = node.hash() else {
                continue;
            };
            needed.push((InboundLedgerObjectType::TransactionNode, *hash.as_uint256()));
            if needed.len() >= WALK_MISSING_REQUEST_LIMIT {
                break;
            }
        }
    }

    if needed.is_empty() {
        return None;
    }

    make_inbound_needed_by_hash_request(inbound.hash(), inbound.seq(), &needed)
}

fn ledger_is_locally_complete<CLOCK, S, C, F, MR, NS>(
    ledger: &ledger::Ledger,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
) -> bool
where
    CLOCK: basics::tagged_cache::CacheClock,
    S: std::hash::BuildHasher + Clone + Send + Sync,
    C: shamap::family::FullBelowCache + Send,
    F: shamap::family::SHAMapNodeFetcher,
    MR: shamap::family::MissingNodeReporter,
    NS: Send,
{
    let mut probe = ledger.clone();
    if !probe.walk_ledger_with_family(&RpcLedgerWalkJournal, false, family) {
        return false;
    }

    true
}

fn request_ledger_via_overlay(
    app: &ApplicationRoot,
    target: LedgerRequestTarget,
) -> Result<Option<Arc<ledger::Ledger>>, Status> {
    let target_label = match target {
        LedgerRequestTarget::Seq(seq) => format!("seq={seq}"),
        LedgerRequestTarget::Hash(hash) => format!("hash={hash}"),
    };
    let local_match = match target {
        LedgerRequestTarget::Seq(seq) => app
            .validated_ledger()
            .filter(|ledger| ledger.header().seq == seq)
            .or_else(|| {
                app.closed_ledger()
                    .filter(|ledger| ledger.header().seq == seq)
            })
            .or_else(|| {
                app.published_ledger()
                    .filter(|ledger| ledger.header().seq == seq)
            }),
        LedgerRequestTarget::Hash(hash) => app
            .validated_ledger()
            .filter(|ledger| *ledger.header().hash.as_uint256() == *hash.as_uint256())
            .or_else(|| {
                app.closed_ledger()
                    .filter(|ledger| *ledger.header().hash.as_uint256() == *hash.as_uint256())
            })
            .or_else(|| {
                app.published_ledger()
                    .filter(|ledger| *ledger.header().hash.as_uint256() == *hash.as_uint256())
            }),
    };

    let node_store_opt = app.node_store().clone();
    if let Some(ledger) = local_match {
        if let Some(node_store) = node_store_opt.clone() {
            let probe_family = SHAMapFamily::new_with_journal(
                Arc::new(TreeNodeCache::new(
                    "rpc-ledger-request-local-probe",
                    512,
                    Duration::seconds(15),
                    MonotonicClock::default(),
                )),
                FullBelowCacheImpl::new(
                    1,
                    MonotonicClock::default(),
                    HardenedHashBuilder::default(),
                    NODE_FAMILY_FULL_BELOW_TARGET_SIZE,
                ),
                RpcNodeStoreFetcher::new(node_store),
                NullMissingNodeReporter,
                Arc::new(RpcSHAMapJournal),
            );
            if ledger_is_locally_complete(ledger.as_ref(), &probe_family) {
                return Ok(Some(ledger));
            }
            tracing::warn!(target: "rpc", "[ledger_request][warn] local target ledger is incomplete; forcing overlay acquisition target={}",
                target_label
            );
        } else {
            return Ok(Some(ledger));
        }
    }

    let Some(overlay_runtime) = app.overlay_runtime() else {
        return Err(Status::new(RpcErrorCode::NoNetwork));
    };
    let Some(node_store) = node_store_opt else {
        return Err(Status::with_message(
            RpcErrorCode::Internal,
            "NodeStore is not configured.",
        ));
    };

    // Take any resumable inbound BEFORE the peers check so that a locally
    // completable resumed acquisition is not blocked by a transient no-peers
    // state.  compatibility: `InboundLedger::tryDB()` is called unconditionally
    // on resume regardless of peer availability.
    let mut inbound = take_resumable_inbound_ledger(app, target);
    if let Some(ref mut resumed) = inbound {
        let probe_family = SHAMapFamily::new_with_journal(
            Arc::new(TreeNodeCache::new(
                "rpc-ledger-request-resume-probe",
                512,
                Duration::seconds(15),
                MonotonicClock::default(),
            )),
            FullBelowCacheImpl::new(
                1,
                MonotonicClock::default(),
                HardenedHashBuilder::default(),
                NODE_FAMILY_FULL_BELOW_TARGET_SIZE,
            ),
            RpcNodeStoreFetcher::new(node_store.clone()),
            NullMissingNodeReporter,
            Arc::new(RpcSHAMapJournal),
        );
        let probe_journal = RpcInboundLedgerJournal;
        let probe_config = LedgerConfig::default();
        let mut probe_store = RpcInboundLedgerStore::new(node_store.clone());
        let mut probe_fetch_pack =
            FetchPackCache::<MonotonicClock, HardenedHashBuilder>::with_hasher(
                4096,
                Duration::minutes(5),
                MonotonicClock::default(),
                HardenedHashBuilder::default(),
            );
        let _ = resumed.check_local_with_family_and_config(
            &probe_journal,
            &probe_config,
            &mut probe_store,
            &mut probe_fetch_pack,
            &probe_family,
        );
        if resumed.is_complete() {
            if let Some(ledger) = resumed.ledger().cloned() {
                tracing::debug!(target: "rpc", "[ledger_request] resumed inbound completed locally target={}",
                    target_label
                );
                return Ok(Some(Arc::new(ledger)));
            }
        }
    }

    let overlay = overlay_runtime.overlay();
    let peers = overlay.active_peers();
    if peers.is_empty() {
        // No peers and no locally-completable resumed inbound: persist the
        // resumed inbound (if any) so the next call can try again.
        if let Some(resumed) = inbound {
            persist_resumable_inbound_ledger(app, resumed);
        }
        return Err(Status::new(RpcErrorCode::NoNetwork));
    }

    let _ = overlay.take_queued_inbound_snapshot();

    let (ledger_hash, ledger_seq) = match target {
        LedgerRequestTarget::Seq(seq) => (None, Some(seq)),
        LedgerRequestTarget::Hash(hash) => (Some(hash.as_uint256().data().to_vec()), None),
    };

    let mut requested_get_ledger_counts = GetLedgerRequestCounts::default();
    let mut acquisition_peer_ids = HashSet::new();
    extend_acquisition_peer_set(
        &peers,
        target,
        ledger_seq.unwrap_or_default(),
        &mut acquisition_peer_ids,
        LEDGER_REQUEST_PEER_COUNT_START,
    );
    let mut selected_peers = acquisition_peers(&peers, &acquisition_peer_ids);
    // `inbound` was already taken from the resumable store before the peers
    // check above; do not call take_resumable_inbound_ledger again here.
    let mut last_request_at = Instant::now()
        .checked_sub(LEDGER_REQUEST_RETRY_INTERVAL)
        .unwrap_or_else(Instant::now);
    if inbound.is_none() {
        let initial_request = ProtocolMessage::new(ProtocolPayload::GetLedger(TmGetLedger {
            itype: 0,
            ltype: None,
            ledger_hash,
            ledger_seq,
            node_i_ds: Vec::new(),
            request_cookie: None,
            query_type: None,
            query_depth: None,
        }));
        record_requested_get_ledger(&initial_request, &mut requested_get_ledger_counts);
        send_to_peers(&selected_peers, &[], initial_request);
        last_request_at = Instant::now();
    }
    let mut last_root_object_at = Instant::now()
        .checked_sub(LEDGER_REQUEST_ROOT_OBJECT_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut last_needed_by_hash_at = Instant::now()
        .checked_sub(LEDGER_REQUEST_NEEDED_BY_HASH_INTERVAL)
        .unwrap_or_else(Instant::now);

    if inbound.is_none()
        && let LedgerRequestTarget::Hash(hash) = target
    {
        inbound = Some(InboundLedgerLocal::new_with_reason(
            hash,
            0,
            InboundLedgerReason::Generic,
        ));
    }
    let mut store = RpcInboundLedgerStore::new(node_store.clone());
    let mut fetch_pack = FetchPackCache::<MonotonicClock, HardenedHashBuilder>::with_hasher(
        4096,
        Duration::minutes(5),
        MonotonicClock::default(),
        HardenedHashBuilder::default(),
    );
    let family = SHAMapFamily::new_with_journal(
        Arc::new(TreeNodeCache::new(
            "rpc-ledger-request",
            2048,
            Duration::seconds(90),
            MonotonicClock::default(),
        )),
        // Keep generation non-zero so default node `full_below_gen=0` does not
        // make missing-node scans incorrectly short-circuit as complete.
        FullBelowCacheImpl::new_with_expiration(
            1,
            MonotonicClock::default(),
            HardenedHashBuilder::default(),
            NODE_FAMILY_FULL_BELOW_TARGET_SIZE,
            NODE_FAMILY_FULL_BELOW_EXPIRATION,
        ),
        RpcNodeStoreFetcher::new(node_store),
        NullMissingNodeReporter,
        Arc::new(RpcSHAMapJournal),
    );
    let journal = RpcInboundLedgerJournal;
    let config = LedgerConfig::default();
    let request_started_at = Instant::now();
    let mut packets_seen = 0usize;
    let mut packets_for_target = 0usize;
    let mut packets_hash_mismatch = 0usize;
    let mut inbound_created = 0usize;
    let mut base_packets = 0usize;
    let mut tx_packets = 0usize;
    let mut state_packets = 0usize;
    let mut get_objects_blobs = 0usize;
    let mut tx_root_blob_hits = 0usize;
    let mut state_root_blob_hits = 0usize;
    let mut requested_object_hashes = HashSet::new();
    let mut answered_requested_hashes = HashSet::new();
    let mut recent_node_requests = HashSet::new();
    let mut last_recent_node_reset_at = Instant::now();
    let mut requested_blob_hits = 0usize;
    let mut requested_blob_parse_ok = 0usize;
    let mut requested_blob_parse_err = 0usize;
    let mut valid_blob_hashes = 0usize;
    let mut invalid_blob_hashes = 0usize;
    let mut requested_get_objects_type_counts = GetObjectsTypeCounts::default();
    let mut response_get_objects_type_counts = GetObjectsTypeCounts::default();
    let mut requested_needed_by_hash_type_counts = GetObjectsTypeCounts::default();

    let mut last_target_progress_at = Instant::now();
    let mut last_timeout_tick_at = Instant::now();
    let mut timeout_count = 0u32;
    let mut timeout_request_pending = false;
    let mut progress_in_timeout_window = false;
    let mut by_hash_armed = true;
    let mut no_progress_cycles = 0u32;
    let mut last_blind_request_peer_count = selected_peers.len();
    let mut local_check_pending = inbound.is_some();
    while last_target_progress_at.elapsed() < LEDGER_REQUEST_TIMEOUT {
        thread::sleep(LEDGER_REQUEST_POLL_INTERVAL);

        let snapshot = overlay.take_queued_inbound_snapshot();

        for message in snapshot.get_objects {
            if message.message.query {
                continue;
            }

            response_get_objects_type_counts
                .record_message(message.message.r#type, message.message.objects.len());

            // Keep the fetch-pack cache focused on ledger-acquisition node payloads.
            // Transaction payload bursts (`otTRANSACTIONS`) can evict needed state nodes.
            if !is_relevant_get_objects_type_for_inbound(message.message.r#type) {
                continue;
            }

            let store_object_type = match message.message.r#type {
                1 => Some(SHAMapNodeObjectType::Ledger),
                3 => Some(SHAMapNodeObjectType::TransactionNode),
                4 => Some(SHAMapNodeObjectType::AccountNode),
                6 => Some(SHAMapNodeObjectType::Unknown),
                _ => None,
            };

            for object in message.message.objects {
                let ledger_seq_hint = object.ledger_seq;
                let Some(data) = object.data else {
                    continue;
                };
                if data.is_empty() {
                    continue;
                }
                let computed_hash = sha512_half(&data);
                let hash = match object.hash.as_deref() {
                    Some(hash_bytes) => {
                        let Some(wire_hash) = Uint256::from_slice(hash_bytes) else {
                            continue;
                        };
                        wire_hash
                    }
                    None => computed_hash,
                };

                let blob_hash_ok = computed_hash == hash;
                if blob_hash_ok {
                    valid_blob_hashes += 1;
                } else {
                    invalid_blob_hashes += 1;
                }

                let requested_parse_result = if requested_object_hashes.contains(&hash) {
                    Some(SHAMapTreeNode::make_from_prefix(&data, SHAMapHash::new(hash)).is_ok())
                } else {
                    None
                };

                if let Some(current) = inbound.as_ref().and_then(|ledger| ledger.ledger()) {
                    if hash == *current.header().tx_hash.as_uint256() {
                        tx_root_blob_hits += 1;
                    }
                    if hash == *current.header().account_hash.as_uint256() {
                        state_root_blob_hits += 1;
                    }
                }

                if requested_object_hashes.contains(&hash) {
                    answered_requested_hashes.insert(hash);
                    requested_blob_hits += 1;
                    if requested_parse_result.unwrap_or(false) {
                        requested_blob_parse_ok += 1;
                        // Exact by-hash replies only become useful once the local
                        // completeness pass can attach them into the active frontier.
                        local_check_pending = true;
                    } else {
                        requested_blob_parse_err += 1;
                    }
                }

                if blob_hash_ok {
                    if let Some(object_type) = store_object_type {
                        let ledger_seq = ledger_seq_hint
                            .or_else(|| inbound.as_ref().map(|ledger| ledger.seq()))
                            .unwrap_or(0);
                        store.store_shamap_node(object_type, data.clone(), hash, ledger_seq);
                    }

                    // By-hash responses can carry additional useful blobs beyond the
                    // exact requested hashes. Re-arm local attachment whenever we
                    // cache a valid blob so the active frontier can consume it
                    // immediately instead of waiting for a later stall/timer pass.
                    local_check_pending = true;
                }

                fetch_pack.add_fetch_pack(hash, data);
                get_objects_blobs += 1;
            }
        }

        for message in snapshot.ledger_data {
            packets_seen += 1;
            let Some((hash, packet)) = inbound_packet_from_wire(&message.message) else {
                continue;
            };

            match packet.packet_type {
                InboundLedgerDataType::Base => base_packets += 1,
                InboundLedgerDataType::TransactionNode => tx_packets += 1,
                InboundLedgerDataType::StateNode => state_packets += 1,
            }

            if inbound.is_none() && packet.packet_type == InboundLedgerDataType::Base {
                match target {
                    // Avoid latching onto unrelated base packets when multiple overlay
                    // replies are in-flight; only bind inbound state to the requested seq.
                    LedgerRequestTarget::Seq(requested_seq) => {
                        if message.message.ledger_seq != requested_seq {
                            continue;
                        }
                    }
                    // Hash-target requests should only bind to the exact requested hash.
                    LedgerRequestTarget::Hash(requested_hash) => {
                        if hash != requested_hash {
                            continue;
                        }
                    }
                }
                let seq = match target {
                    LedgerRequestTarget::Seq(seq) => seq,
                    LedgerRequestTarget::Hash(_) => 0,
                };
                inbound = Some(InboundLedgerLocal::new_with_reason(
                    hash,
                    seq,
                    InboundLedgerReason::Generic,
                ));
                inbound_created += 1;
                local_check_pending = true;
            }

            let Some(inbound) = inbound.as_mut() else {
                continue;
            };
            if inbound.hash() != hash {
                packets_hash_mismatch += 1;
                continue;
            }
            packets_for_target += 1;
            inbound.touch(time::Duration::milliseconds(
                request_started_at.elapsed().as_millis() as i64,
            ));
            inbound.update(message.message.ledger_seq, inbound.last_action());
            let _ = inbound.got_data(Some(u64::from(message.peer_id)), packet);
        }

        let peers = overlay.active_peers();
        selected_peers = acquisition_peers(&peers, &acquisition_peer_ids);
        if !peers.is_empty()
            && selected_peers.is_empty()
            && extend_acquisition_peer_set(
                &peers,
                target,
                inbound
                    .as_ref()
                    .map(|ledger| ledger.seq())
                    .unwrap_or_default(),
                &mut acquisition_peer_ids,
                LEDGER_REQUEST_PEER_COUNT_START,
            ) > 0
        {
            selected_peers = acquisition_peers(&peers, &acquisition_peer_ids);
        }

        if inbound.is_none() {
            if last_request_at.elapsed() >= LEDGER_REQUEST_RETRY_INTERVAL {
                let request = ProtocolMessage::new(ProtocolPayload::GetLedger(TmGetLedger {
                    itype: 0,
                    ltype: None,
                    ledger_hash: None,
                    ledger_seq: match target {
                        LedgerRequestTarget::Seq(seq) => Some(seq),
                        LedgerRequestTarget::Hash(_) => None,
                    },
                    node_i_ds: Vec::new(),
                    request_cookie: None,
                    query_type: None,
                    query_depth: None,
                }));
                record_requested_get_ledger(&request, &mut requested_get_ledger_counts);
                send_to_peers(&selected_peers, &[], request);
                last_request_at = Instant::now();
            }
            continue;
        }

        let Some(inbound) = inbound.as_mut() else {
            continue;
        };

        let planner_before = inbound.planner_state();
        let run = if inbound.received_data_len() > 0 {
            Some(inbound.run_data_with_family_and_config(
                &journal,
                &config,
                &mut store,
                &mut fetch_pack,
                &family,
            ))
        } else {
            None
        };

        if run
            .as_ref()
            .map(|state| state.max_useful_count > 0)
            .unwrap_or(false)
        {
            local_check_pending = true;
        }

        if should_run_local_check(local_check_pending) {
            let _ = inbound.check_local_with_family_and_config(
                &journal,
                &config,
                &mut store,
                &mut fetch_pack,
                &family,
            );
            local_check_pending = false;
        }
        let planner_after = inbound.planner_state();
        let target_progress = target_made_progress(
            planner_before,
            planner_after,
            run.as_ref()
                .map(|state| state.max_useful_count)
                .unwrap_or_default(),
        );
        // Reset the overall timeout only when the planner state actually advances.
        // Useful packets that don't flip have_header/have_state/have_transactions
        // (e.g. AS nodes arriving while TX is the bottleneck) should not mask a
        // stalled side and prevent the timeout from firing.
        let planner_advanced = planner_after != planner_before;
        if planner_advanced {
            last_target_progress_at = Instant::now();
            progress_in_timeout_window = true;
            no_progress_cycles = 0;
        } else if target_progress {
            // Useful packets arrived but planner didn't advance — keep request
            // pacing responsive but don't reset the overall timeout.
            no_progress_cycles = 0;
        } else {
            no_progress_cycles = no_progress_cycles.saturating_add(1);
        }

        if inbound.is_done() {
            let (unanswered_requested_hashes, unanswered_requested_sample) =
                summarize_unanswered_requested_hashes(
                    &requested_object_hashes,
                    &answered_requested_hashes,
                );
            let mut answered_fetchable_hashes = 0usize;
            for hash in &answered_requested_hashes {
                if family.fetch_cached_node(SHAMapHash::new(*hash)).is_some() {
                    answered_fetchable_hashes += 1;
                }
            }
            if inbound.is_complete() {
                if let Some(ledger) = inbound.ledger().cloned() {
                    let local_complete = ledger_is_locally_complete(&ledger, &family);
                    if !local_complete {
                        tracing::warn!(target: "rpc", "[ledger_request][warn] inbound complete ledger failed full verification target={} hash={} seq={}",
                            target_label,
                            inbound.hash(),
                            inbound.seq()
                        );
                    }
                    tracing::debug!(target: "rpc", "[ledger_request] complete target={} packets_seen={} packets_for_target={} mismatches={} inbound_created={} base={} tx={} state={} get_objects_blobs={} valid_blob_hashes={} invalid_blob_hashes={} tx_root_blobs={} state_root_blobs={} requested_hashes={} requested_blob_hits={} requested_blob_parse_ok={} requested_blob_parse_err={} unanswered_requested_hashes={} unanswered_requested_sample={} answered_fetchable_hashes={} req_get_objects[msg:ledger={} tx={} state={} fetch={} unknown={}] req_get_objects[obj:ledger={} tx={} state={} fetch={} unknown={}] req_needed_by_hash[msg:ledger={} tx={} state={} fetch={} unknown={}] req_needed_by_hash[obj:ledger={} tx={} state={} fetch={} unknown={}] resp_get_objects[msg:ledger={} tx={} state={} fetch={} unknown={}] resp_get_objects[obj:ledger={} tx={} state={} fetch={} unknown={}]",
                        target_label,
                        packets_seen,
                        packets_for_target,
                        packets_hash_mismatch,
                        inbound_created,
                        base_packets,
                        tx_packets,
                        state_packets,
                        get_objects_blobs,
                        valid_blob_hashes,
                        invalid_blob_hashes,
                        tx_root_blob_hits,
                        state_root_blob_hits,
                        requested_object_hashes.len(),
                        requested_blob_hits,
                        requested_blob_parse_ok,
                        requested_blob_parse_err,
                        unanswered_requested_hashes,
                        unanswered_requested_sample,
                        answered_fetchable_hashes,
                        requested_get_objects_type_counts.ledger_messages,
                        requested_get_objects_type_counts.tx_node_messages,
                        requested_get_objects_type_counts.state_node_messages,
                        requested_get_objects_type_counts.fetch_pack_messages,
                        requested_get_objects_type_counts.unknown_messages,
                        requested_get_objects_type_counts.ledger_objects,
                        requested_get_objects_type_counts.tx_node_objects,
                        requested_get_objects_type_counts.state_node_objects,
                        requested_get_objects_type_counts.fetch_pack_objects,
                        requested_get_objects_type_counts.unknown_objects,
                        requested_needed_by_hash_type_counts.ledger_messages,
                        requested_needed_by_hash_type_counts.tx_node_messages,
                        requested_needed_by_hash_type_counts.state_node_messages,
                        requested_needed_by_hash_type_counts.fetch_pack_messages,
                        requested_needed_by_hash_type_counts.unknown_messages,
                        requested_needed_by_hash_type_counts.ledger_objects,
                        requested_needed_by_hash_type_counts.tx_node_objects,
                        requested_needed_by_hash_type_counts.state_node_objects,
                        requested_needed_by_hash_type_counts.fetch_pack_objects,
                        requested_needed_by_hash_type_counts.unknown_objects,
                        response_get_objects_type_counts.ledger_messages,
                        response_get_objects_type_counts.tx_node_messages,
                        response_get_objects_type_counts.state_node_messages,
                        response_get_objects_type_counts.fetch_pack_messages,
                        response_get_objects_type_counts.unknown_messages,
                        response_get_objects_type_counts.ledger_objects,
                        response_get_objects_type_counts.tx_node_objects,
                        response_get_objects_type_counts.state_node_objects,
                        response_get_objects_type_counts.fetch_pack_objects,
                        response_get_objects_type_counts.unknown_objects
                    );
                    return Ok(Some(Arc::new(ledger)));
                }
            }
            tracing::debug!(target: "rpc", "[ledger_request] done-incomplete target={} packets_seen={} packets_for_target={} mismatches={} inbound_created={} base={} tx={} state={} get_objects_blobs={} valid_blob_hashes={} invalid_blob_hashes={} tx_root_blobs={} state_root_blobs={} requested_hashes={} requested_blob_hits={} requested_blob_parse_ok={} requested_blob_parse_err={} unanswered_requested_hashes={} unanswered_requested_sample={} answered_fetchable_hashes={} req_get_objects[msg:ledger={} tx={} state={} fetch={} unknown={}] req_get_objects[obj:ledger={} tx={} state={} fetch={} unknown={}] req_needed_by_hash[msg:ledger={} tx={} state={} fetch={} unknown={}] req_needed_by_hash[obj:ledger={} tx={} state={} fetch={} unknown={}] resp_get_objects[msg:ledger={} tx={} state={} fetch={} unknown={}] resp_get_objects[obj:ledger={} tx={} state={} fetch={} unknown={}] have_header={} have_state={} have_tx={}",
                target_label,
                packets_seen,
                packets_for_target,
                packets_hash_mismatch,
                inbound_created,
                base_packets,
                tx_packets,
                state_packets,
                get_objects_blobs,
                valid_blob_hashes,
                invalid_blob_hashes,
                tx_root_blob_hits,
                state_root_blob_hits,
                requested_object_hashes.len(),
                requested_blob_hits,
                requested_blob_parse_ok,
                requested_blob_parse_err,
                unanswered_requested_hashes,
                unanswered_requested_sample,
                answered_fetchable_hashes,
                requested_get_objects_type_counts.ledger_messages,
                requested_get_objects_type_counts.tx_node_messages,
                requested_get_objects_type_counts.state_node_messages,
                requested_get_objects_type_counts.fetch_pack_messages,
                requested_get_objects_type_counts.unknown_messages,
                requested_get_objects_type_counts.ledger_objects,
                requested_get_objects_type_counts.tx_node_objects,
                requested_get_objects_type_counts.state_node_objects,
                requested_get_objects_type_counts.fetch_pack_objects,
                requested_get_objects_type_counts.unknown_objects,
                requested_needed_by_hash_type_counts.ledger_messages,
                requested_needed_by_hash_type_counts.tx_node_messages,
                requested_needed_by_hash_type_counts.state_node_messages,
                requested_needed_by_hash_type_counts.fetch_pack_messages,
                requested_needed_by_hash_type_counts.unknown_messages,
                requested_needed_by_hash_type_counts.ledger_objects,
                requested_needed_by_hash_type_counts.tx_node_objects,
                requested_needed_by_hash_type_counts.state_node_objects,
                requested_needed_by_hash_type_counts.fetch_pack_objects,
                requested_needed_by_hash_type_counts.unknown_objects,
                response_get_objects_type_counts.ledger_messages,
                response_get_objects_type_counts.tx_node_messages,
                response_get_objects_type_counts.state_node_messages,
                response_get_objects_type_counts.fetch_pack_messages,
                response_get_objects_type_counts.unknown_messages,
                response_get_objects_type_counts.ledger_objects,
                response_get_objects_type_counts.tx_node_objects,
                response_get_objects_type_counts.state_node_objects,
                response_get_objects_type_counts.fetch_pack_objects,
                response_get_objects_type_counts.unknown_objects,
                inbound.planner_state().have_header,
                inbound.planner_state().have_state,
                inbound.planner_state().have_transactions
            );
            return Ok(None);
        }

        if inbound.revalidate_map_sync_with_family(&family) {
            tracing::debug!(target: "rpc", "[ledger_request] reactivated sync target={} have_header={} have_state={} have_tx={}",
                target_label,
                inbound.planner_state().have_header,
                inbound.planner_state().have_state,
                inbound.planner_state().have_transactions
            );
        }
        inbound.maybe_finish(&journal);

        if inbound.is_done() {
            let (unanswered_requested_hashes, unanswered_requested_sample) =
                summarize_unanswered_requested_hashes(
                    &requested_object_hashes,
                    &answered_requested_hashes,
                );
            let mut answered_fetchable_hashes = 0usize;
            for hash in &answered_requested_hashes {
                if family.fetch_cached_node(SHAMapHash::new(*hash)).is_some() {
                    answered_fetchable_hashes += 1;
                }
            }
            if inbound.is_complete() {
                if let Some(ledger) = inbound.ledger().cloned() {
                    let local_complete = ledger_is_locally_complete(&ledger, &family);
                    if !local_complete {
                        tracing::warn!(target: "rpc", "[ledger_request][warn] inbound complete ledger failed full verification target={} hash={} seq={}",
                            target_label,
                            inbound.hash(),
                            inbound.seq()
                        );
                    }
                    tracing::debug!(target: "rpc", "[ledger_request] complete target={} packets_seen={} packets_for_target={} mismatches={} inbound_created={} base={} tx={} state={} get_objects_blobs={} valid_blob_hashes={} invalid_blob_hashes={} tx_root_blobs={} state_root_blobs={} requested_hashes={} requested_blob_hits={} requested_blob_parse_ok={} requested_blob_parse_err={} unanswered_requested_hashes={} unanswered_requested_sample={} answered_fetchable_hashes={} req_get_objects[msg:ledger={} tx={} state={} fetch={} unknown={}] req_get_objects[obj:ledger={} tx={} state={} fetch={} unknown={}] req_needed_by_hash[msg:ledger={} tx={} state={} fetch={} unknown={}] req_needed_by_hash[obj:ledger={} tx={} state={} fetch={} unknown={}] resp_get_objects[msg:ledger={} tx={} state={} fetch={} unknown={}] resp_get_objects[obj:ledger={} tx={} state={} fetch={} unknown={}]",
                        target_label,
                        packets_seen,
                        packets_for_target,
                        packets_hash_mismatch,
                        inbound_created,
                        base_packets,
                        tx_packets,
                        state_packets,
                        get_objects_blobs,
                        valid_blob_hashes,
                        invalid_blob_hashes,
                        tx_root_blob_hits,
                        state_root_blob_hits,
                        requested_object_hashes.len(),
                        requested_blob_hits,
                        requested_blob_parse_ok,
                        requested_blob_parse_err,
                        unanswered_requested_hashes,
                        unanswered_requested_sample,
                        answered_fetchable_hashes,
                        requested_get_objects_type_counts.ledger_messages,
                        requested_get_objects_type_counts.tx_node_messages,
                        requested_get_objects_type_counts.state_node_messages,
                        requested_get_objects_type_counts.fetch_pack_messages,
                        requested_get_objects_type_counts.unknown_messages,
                        requested_get_objects_type_counts.ledger_objects,
                        requested_get_objects_type_counts.tx_node_objects,
                        requested_get_objects_type_counts.state_node_objects,
                        requested_get_objects_type_counts.fetch_pack_objects,
                        requested_get_objects_type_counts.unknown_objects,
                        requested_needed_by_hash_type_counts.ledger_messages,
                        requested_needed_by_hash_type_counts.tx_node_messages,
                        requested_needed_by_hash_type_counts.state_node_messages,
                        requested_needed_by_hash_type_counts.fetch_pack_messages,
                        requested_needed_by_hash_type_counts.unknown_messages,
                        requested_needed_by_hash_type_counts.ledger_objects,
                        requested_needed_by_hash_type_counts.tx_node_objects,
                        requested_needed_by_hash_type_counts.state_node_objects,
                        requested_needed_by_hash_type_counts.fetch_pack_objects,
                        requested_needed_by_hash_type_counts.unknown_objects,
                        response_get_objects_type_counts.ledger_messages,
                        response_get_objects_type_counts.tx_node_messages,
                        response_get_objects_type_counts.state_node_messages,
                        response_get_objects_type_counts.fetch_pack_messages,
                        response_get_objects_type_counts.unknown_messages,
                        response_get_objects_type_counts.ledger_objects,
                        response_get_objects_type_counts.tx_node_objects,
                        response_get_objects_type_counts.state_node_objects,
                        response_get_objects_type_counts.fetch_pack_objects,
                        response_get_objects_type_counts.unknown_objects
                    );
                    return Ok(Some(Arc::new(ledger)));
                }
            }
            tracing::debug!(target: "rpc", "[ledger_request] done-incomplete target={} packets_seen={} packets_for_target={} mismatches={} inbound_created={} base={} tx={} state={} get_objects_blobs={} valid_blob_hashes={} invalid_blob_hashes={} tx_root_blobs={} state_root_blobs={} requested_hashes={} requested_blob_hits={} requested_blob_parse_ok={} requested_blob_parse_err={} unanswered_requested_hashes={} unanswered_requested_sample={} answered_fetchable_hashes={} req_get_objects[msg:ledger={} tx={} state={} fetch={} unknown={}] req_get_objects[obj:ledger={} tx={} state={} fetch={} unknown={}] req_needed_by_hash[msg:ledger={} tx={} state={} fetch={} unknown={}] req_needed_by_hash[obj:ledger={} tx={} state={} fetch={} unknown={}] resp_get_objects[msg:ledger={} tx={} state={} fetch={} unknown={}] resp_get_objects[obj:ledger={} tx={} state={} fetch={} unknown={}] have_header={} have_state={} have_tx={}",
                target_label,
                packets_seen,
                packets_for_target,
                packets_hash_mismatch,
                inbound_created,
                base_packets,
                tx_packets,
                state_packets,
                get_objects_blobs,
                valid_blob_hashes,
                invalid_blob_hashes,
                tx_root_blob_hits,
                state_root_blob_hits,
                requested_object_hashes.len(),
                requested_blob_hits,
                requested_blob_parse_ok,
                requested_blob_parse_err,
                unanswered_requested_hashes,
                unanswered_requested_sample,
                answered_fetchable_hashes,
                requested_get_objects_type_counts.ledger_messages,
                requested_get_objects_type_counts.tx_node_messages,
                requested_get_objects_type_counts.state_node_messages,
                requested_get_objects_type_counts.fetch_pack_messages,
                requested_get_objects_type_counts.unknown_messages,
                requested_get_objects_type_counts.ledger_objects,
                requested_get_objects_type_counts.tx_node_objects,
                requested_get_objects_type_counts.state_node_objects,
                requested_get_objects_type_counts.fetch_pack_objects,
                requested_get_objects_type_counts.unknown_objects,
                requested_needed_by_hash_type_counts.ledger_messages,
                requested_needed_by_hash_type_counts.tx_node_messages,
                requested_needed_by_hash_type_counts.state_node_messages,
                requested_needed_by_hash_type_counts.fetch_pack_messages,
                requested_needed_by_hash_type_counts.unknown_messages,
                requested_needed_by_hash_type_counts.ledger_objects,
                requested_needed_by_hash_type_counts.tx_node_objects,
                requested_needed_by_hash_type_counts.state_node_objects,
                requested_needed_by_hash_type_counts.fetch_pack_objects,
                requested_needed_by_hash_type_counts.unknown_objects,
                response_get_objects_type_counts.ledger_messages,
                response_get_objects_type_counts.tx_node_messages,
                response_get_objects_type_counts.state_node_messages,
                response_get_objects_type_counts.fetch_pack_messages,
                response_get_objects_type_counts.unknown_messages,
                response_get_objects_type_counts.ledger_objects,
                response_get_objects_type_counts.tx_node_objects,
                response_get_objects_type_counts.state_node_objects,
                response_get_objects_type_counts.fetch_pack_objects,
                response_get_objects_type_counts.unknown_objects,
                inbound.planner_state().have_header,
                inbound.planner_state().have_state,
                inbound.planner_state().have_transactions
            );
            return Ok(None);
        }

        if last_timeout_tick_at.elapsed() >= LEDGER_REQUEST_TIMEOUT_MODE_STALL {
            recent_node_requests.clear();
            last_recent_node_reset_at = Instant::now();
            last_timeout_tick_at = Instant::now();

            if progress_in_timeout_window {
                progress_in_timeout_window = false;
            } else {
                local_check_pending = should_arm_timeout_local_check(progress_in_timeout_window);
                timeout_count = timeout_count.saturating_add(1);
                timeout_request_pending = true;
                by_hash_armed = true;
                extend_acquisition_peer_set(
                    &peers,
                    target,
                    inbound.seq(),
                    &mut acquisition_peer_ids,
                    LEDGER_REQUEST_PEER_COUNT_ADD,
                );
                selected_peers = acquisition_peers(&peers, &acquisition_peer_ids);
            }
        }

        // timeout tick forces the next request cycle down the timeout path.
        let timeout_mode = timeout_request_pending;
        if last_recent_node_reset_at.elapsed() >= LEDGER_REQUEST_TIMEOUT_MODE_STALL {
            recent_node_requests.clear();
            last_recent_node_reset_at = Instant::now();
        }
        let preferred_peer_ids: Vec<u64> = run
            .as_ref()
            .map(|state| {
                state
                    .triggered_peer_ids
                    .iter()
                    .copied()
                    .filter(|peer_id| acquisition_peer_ids.contains(peer_id))
                    .collect()
            })
            .unwrap_or_default();
        let reply_trigger = use_reply_trigger(!preferred_peer_ids.is_empty(), timeout_mode);
        let query_depth = if reply_trigger {
            LEDGER_REQUEST_REPLY_QUERY_DEPTH
        } else {
            LEDGER_REQUEST_TIMEOUT_QUERY_DEPTH
        };
        let query_type = get_ledger_query_type(timeout_count > 0);
        let request_node_limit = if reply_trigger { 128 } else { 12 };
        let allow_primary_request = use_blind_request_trigger(
            reply_trigger,
            timeout_mode,
            selected_peers.len(),
            last_blind_request_peer_count,
        ) && can_send_primary_request(
            reply_trigger,
            last_request_at,
            LEDGER_REQUEST_RETRY_INTERVAL,
        );

        let mut sent_primary_request = false;
        if allow_primary_request && reply_trigger && !preferred_peer_ids.is_empty() {
            sent_primary_request = send_reply_mode_requests_to_peers(
                &selected_peers,
                preferred_peer_ids.as_slice(),
                |per_peer_query_depth| {
                    if !inbound.planner_state().have_header {
                        let request = inbound.make_header_request();
                        record_requested_get_ledger(&request, &mut requested_get_ledger_counts);
                        return Some(request);
                    }

                    if let Some(request) = make_missing_node_request(
                        inbound,
                        &family,
                        &mut store,
                        &mut fetch_pack,
                        per_peer_query_depth,
                        query_type,
                        false,
                        request_node_limit,
                        &mut recent_node_requests,
                    ) {
                        record_requested_get_ledger(&request, &mut requested_get_ledger_counts);
                        return Some(request);
                    }

                    if let Some(request) =
                        make_root_node_request(inbound, per_peer_query_depth, query_type)
                    {
                        record_requested_get_ledger(&request, &mut requested_get_ledger_counts);
                        return Some(request);
                    }

                    if let Some(request) = make_root_object_request(inbound) {
                        record_requested_object_hashes(&request, &mut requested_object_hashes);
                        if let ProtocolPayload::GetObjects(body) = &request.payload {
                            requested_get_objects_type_counts
                                .record_message(body.r#type, body.objects.len());
                        }
                        last_root_object_at = Instant::now();
                        return Some(request);
                    }

                    None
                },
            );
            if sent_primary_request {
                last_request_at = Instant::now();
            }
        } else if allow_primary_request && !inbound.planner_state().have_header {
            let request = inbound.make_header_request();
            record_requested_get_ledger(&request, &mut requested_get_ledger_counts);
            send_to_peers(&selected_peers, preferred_peer_ids.as_slice(), request);
            last_request_at = Instant::now();
            sent_primary_request = true;
        } else if allow_primary_request
            && let Some(request) = make_missing_node_request(
                inbound,
                &family,
                &mut store,
                &mut fetch_pack,
                query_depth,
                query_type,
                timeout_mode,
                request_node_limit,
                &mut recent_node_requests,
            )
        {
            record_requested_get_ledger(&request, &mut requested_get_ledger_counts);
            send_get_ledger_to_peers(
                &selected_peers,
                preferred_peer_ids.as_slice(),
                request,
                reply_trigger,
            );
            last_request_at = Instant::now();
            sent_primary_request = true;
        } else if allow_primary_request
            && let Some(request) = make_root_node_request(inbound, query_depth, query_type)
        {
            record_requested_get_ledger(&request, &mut requested_get_ledger_counts);
            send_get_ledger_to_peers(
                &selected_peers,
                preferred_peer_ids.as_slice(),
                request,
                reply_trigger,
            );
            last_request_at = Instant::now();
            sent_primary_request = true;
        } else if allow_primary_request && let Some(request) = make_root_object_request(inbound) {
            record_requested_object_hashes(&request, &mut requested_object_hashes);
            if let ProtocolPayload::GetObjects(body) = &request.payload {
                requested_get_objects_type_counts.record_message(body.r#type, body.objects.len());
            }
            send_to_peers(&selected_peers, preferred_peer_ids.as_slice(), request);
            last_request_at = Instant::now();
            last_root_object_at = Instant::now();
            sent_primary_request = true;
        }
        if sent_primary_request && !reply_trigger && !timeout_mode {
            last_blind_request_peer_count = selected_peers.len();
        }

        let mut sent_by_hash_request = false;
        if last_needed_by_hash_at.elapsed() >= LEDGER_REQUEST_NEEDED_BY_HASH_INTERVAL
            && inbound.planner_state().have_header
            && (!inbound.planner_state().have_transactions || !inbound.planner_state().have_state)
            && use_aggressive_by_hash_timeout(timeout_count, timeout_request_pending, by_hash_armed)
            && let Some(request) =
                make_snapshot_missing_request(inbound, &family, &mut store, &mut fetch_pack)
                    .or_else(|| make_walk_missing_hash_request(inbound, &family))
        {
            let preferred = &[];
            record_requested_object_hashes(&request, &mut requested_object_hashes);
            if let ProtocolPayload::GetObjects(body) = &request.payload {
                requested_needed_by_hash_type_counts
                    .record_message(body.r#type, body.objects.len());
            }
            send_to_peers(&selected_peers, preferred, request);
            last_needed_by_hash_at = Instant::now();
            by_hash_armed = false;
            sent_by_hash_request = true;
            if !sent_primary_request {
                last_request_at = Instant::now();
            }
        }

        if timeout_request_pending && (sent_primary_request || sent_by_hash_request) {
            timeout_request_pending = false;
        }

        if last_root_object_at.elapsed() >= LEDGER_REQUEST_ROOT_OBJECT_INTERVAL
            && inbound.planner_state().have_header
            && (!inbound.planner_state().have_transactions || !inbound.planner_state().have_state)
            && let Some(request) = make_root_object_request(inbound)
        {
            record_requested_object_hashes(&request, &mut requested_object_hashes);
            if let ProtocolPayload::GetObjects(body) = &request.payload {
                requested_get_objects_type_counts.record_message(body.r#type, body.objects.len());
            }
            send_to_peers(&selected_peers, &[], request);
            last_root_object_at = Instant::now();
        }
    }

    let (unanswered_requested_hashes, unanswered_requested_sample) =
        summarize_unanswered_requested_hashes(&requested_object_hashes, &answered_requested_hashes);
    let mut answered_fetchable_hashes = 0usize;
    for hash in &answered_requested_hashes {
        if family.fetch_cached_node(SHAMapHash::new(*hash)).is_some() {
            answered_fetchable_hashes += 1;
        }
    }
    let mut walk_state_missing_total = 0usize;
    let mut walk_state_missing_not_requested = 0usize;
    let mut walk_state_missing_not_fetchable = 0usize;
    let mut walk_state_missing_sample = "none".to_owned();
    let mut walk_tx_missing_total = 0usize;
    let mut walk_tx_missing_not_requested = 0usize;
    let mut walk_tx_missing_not_fetchable = 0usize;
    let mut walk_tx_missing_sample = "none".to_owned();
    let inbound_stats = inbound
        .as_ref()
        .map(|ledger| ledger.stats().get())
        .unwrap_or_else(|| "none".to_owned());
    if let Some(acquired) = inbound.as_ref().and_then(|ledger| ledger.ledger()) {
        let mut state_missing = Vec::new();
        acquired.state_map().walk_map_with_family(
            SHAMapType::State,
            &mut state_missing,
            32,
            &family,
        );
        walk_state_missing_total = state_missing.len();
        for missing in state_missing {
            let Some(hash) = missing.hash() else {
                continue;
            };
            let key = *hash.as_uint256();
            if !requested_object_hashes.contains(&key) {
                walk_state_missing_not_requested += 1;
                if walk_state_missing_sample == "none" {
                    walk_state_missing_sample = key.to_string();
                }
            }
            if family.fetch_cached_node(hash).is_none() {
                walk_state_missing_not_fetchable += 1;
            }
        }

        let mut tx_missing = Vec::new();
        acquired.tx_map().walk_map_with_family(
            SHAMapType::Transaction,
            &mut tx_missing,
            32,
            &family,
        );
        walk_tx_missing_total = tx_missing.len();
        for missing in tx_missing {
            let Some(hash) = missing.hash() else {
                continue;
            };
            let key = *hash.as_uint256();
            if !requested_object_hashes.contains(&key) {
                walk_tx_missing_not_requested += 1;
                if walk_tx_missing_sample == "none" {
                    walk_tx_missing_sample = key.to_string();
                }
            }
            if family.fetch_cached_node(hash).is_none() {
                walk_tx_missing_not_fetchable += 1;
            }
        }
    }
    tracing::debug!(target: "rpc", "[ledger_request] timeout target={} packets_seen={} packets_for_target={} mismatches={} inbound_created={} base={} tx={} state={} get_objects_blobs={} valid_blob_hashes={} invalid_blob_hashes={} tx_root_blobs={} state_root_blobs={} requested_hashes={} requested_blob_hits={} requested_blob_parse_ok={} requested_blob_parse_err={} unanswered_requested_hashes={} unanswered_requested_sample={} answered_fetchable_hashes={} inbound_stats={} walk_state_missing_total={} walk_state_missing_not_requested={} walk_state_missing_not_fetchable={} walk_state_missing_sample={} walk_tx_missing_total={} walk_tx_missing_not_requested={} walk_tx_missing_not_fetchable={} walk_tx_missing_sample={} req_get_objects[msg:ledger={} tx={} state={} fetch={} unknown={}] req_get_objects[obj:ledger={} tx={} state={} fetch={} unknown={}] req_needed_by_hash[msg:ledger={} tx={} state={} fetch={} unknown={}] req_needed_by_hash[obj:ledger={} tx={} state={} fetch={} unknown={}] resp_get_objects[msg:ledger={} tx={} state={} fetch={} unknown={}] resp_get_objects[obj:ledger={} tx={} state={} fetch={} unknown={}]",
        target_label,
        packets_seen,
        packets_for_target,
        packets_hash_mismatch,
        inbound_created,
        base_packets,
        tx_packets,
        state_packets,
        get_objects_blobs,
        valid_blob_hashes,
        invalid_blob_hashes,
        tx_root_blob_hits,
        state_root_blob_hits,
        requested_object_hashes.len(),
        requested_blob_hits,
        requested_blob_parse_ok,
        requested_blob_parse_err,
        unanswered_requested_hashes,
        unanswered_requested_sample,
        answered_fetchable_hashes,
        inbound_stats,
        walk_state_missing_total,
        walk_state_missing_not_requested,
        walk_state_missing_not_fetchable,
        walk_state_missing_sample,
        walk_tx_missing_total,
        walk_tx_missing_not_requested,
        walk_tx_missing_not_fetchable,
        walk_tx_missing_sample,
        requested_get_objects_type_counts.ledger_messages,
        requested_get_objects_type_counts.tx_node_messages,
        requested_get_objects_type_counts.state_node_messages,
        requested_get_objects_type_counts.fetch_pack_messages,
        requested_get_objects_type_counts.unknown_messages,
        requested_get_objects_type_counts.ledger_objects,
        requested_get_objects_type_counts.tx_node_objects,
        requested_get_objects_type_counts.state_node_objects,
        requested_get_objects_type_counts.fetch_pack_objects,
        requested_get_objects_type_counts.unknown_objects,
        requested_needed_by_hash_type_counts.ledger_messages,
        requested_needed_by_hash_type_counts.tx_node_messages,
        requested_needed_by_hash_type_counts.state_node_messages,
        requested_needed_by_hash_type_counts.fetch_pack_messages,
        requested_needed_by_hash_type_counts.unknown_messages,
        requested_needed_by_hash_type_counts.ledger_objects,
        requested_needed_by_hash_type_counts.tx_node_objects,
        requested_needed_by_hash_type_counts.state_node_objects,
        requested_needed_by_hash_type_counts.fetch_pack_objects,
        requested_needed_by_hash_type_counts.unknown_objects,
        response_get_objects_type_counts.ledger_messages,
        response_get_objects_type_counts.tx_node_messages,
        response_get_objects_type_counts.state_node_messages,
        response_get_objects_type_counts.fetch_pack_messages,
        response_get_objects_type_counts.unknown_messages,
        response_get_objects_type_counts.ledger_objects,
        response_get_objects_type_counts.tx_node_objects,
        response_get_objects_type_counts.state_node_objects,
        response_get_objects_type_counts.fetch_pack_objects,
        response_get_objects_type_counts.unknown_objects
    );
    tracing::debug!(target: "rpc", "[ledger_request] req_get_ledger[msg:base={} tx={} state={}] req_get_ledger[node_ids:tx={} state={}]",
        requested_get_ledger_counts.base_messages,
        requested_get_ledger_counts.tx_messages,
        requested_get_ledger_counts.state_messages,
        requested_get_ledger_counts.tx_node_ids,
        requested_get_ledger_counts.state_node_ids
    );

    if let Some(inbound) = inbound.take() {
        persist_resumable_inbound_ledger(app, inbound);
    }

    Ok(None)
}

pub trait RpcRuntime {
    fn app(&self) -> Option<&ApplicationRoot> {
        None
    }

    fn network_ops_runtime(&self) -> Option<std::sync::Arc<app::AppNetworkOpsRuntime>> {
        self.app().and_then(|app| app.network_ops_runtime())
    }

    fn job_queue(&self) -> Option<app::JobQueue> {
        self.app().map(|app| app.job_queue().clone())
    }

    fn beta_rpc_api(&self) -> bool {
        false
    }

    fn client_job_count(&self) -> u32 {
        0
    }

    fn max_job_queue_clients(&self) -> u32 {
        Tuning::MAX_JOB_QUEUE_CLIENTS
    }

    fn has_current_ledger(&self) -> bool {
        true
    }

    fn has_closed_ledger(&self) -> bool {
        true
    }

    fn path_search_max(&self) -> u32 {
        0
    }

    fn path_search_old(&self) -> u32 {
        2
    }

    fn path_search(&self) -> u32 {
        2
    }

    fn path_search_fast(&self) -> u32 {
        2
    }

    fn network_synced(&self) -> bool {
        true
    }

    fn current_ledger_index(&self) -> Option<u32> {
        None
    }

    /// Get the current open ledger for transaction simulation.
    /// Returns None if no ledger is available.
    fn current_ledger_for_simulation(&self) -> Option<std::sync::Arc<ledger::Ledger>> {
        None
    }

    fn standalone(&self) -> bool {
        false
    }

    fn ledger_accept(&self) -> Status {
        Status::new(RpcErrorCode::NotStandalone)
    }

    fn stop(&self) -> Status {
        Status::OK
    }

    fn peer_connect(&self, _ip: String, _port: u16) -> Status {
        Status::OK
    }

    fn peers_get(&self) -> JsonValue {
        protocol::json!({ "peers": [] })
    }

    fn can_delete_get(&self) -> u32 {
        0
    }

    fn can_delete_enabled(&self) -> bool {
        false
    }

    fn can_delete_last_rotated(&self) -> u32 {
        0
    }

    fn can_delete_seq_by_hash(&self, _hash: Uint256) -> Option<u32> {
        None
    }

    fn can_delete_set(&self, _seq: u32) -> Status {
        Status::OK
    }

    fn ledger_cleaner_trigger(&self, _params: &JsonValue) -> Status {
        Status::OK
    }

    fn ledger_request(&self, _seq: u32) -> Status {
        Status::OK
    }

    fn ledger_request_by_hash(&self, _hash: Uint256) -> Status {
        Status::new(RpcErrorCode::NotImplemented)
    }

    fn log_level_set(&self, _partition: String, _level: String) -> Status {
        Status::OK
    }

    fn log_level_get(&self) -> JsonValue {
        protocol::json!({ "levels": {} })
    }

    fn log_rotate(&self) -> Status {
        Status::OK
    }

    fn peer_reservations_add(
        &self,
        _public_key: protocol::PublicKey,
        _description: String,
    ) -> Status {
        Status::OK
    }

    fn peer_reservations_del(&self, _public_key: protocol::PublicKey) -> Status {
        Status::OK
    }

    fn peer_reservations_list(&self) -> JsonValue {
        protocol::json!({ "reservations": [] })
    }

    fn export_snapshot(&self, _output_path: &str) -> Result<JsonValue, String> {
        Err("Not implemented".to_owned())
    }
}

impl RpcRuntime for () {}

impl RpcRuntime for ApplicationRoot {
    fn app(&self) -> Option<&ApplicationRoot> {
        Some(self)
    }

    fn client_job_count(&self) -> u32 {
        u32::try_from(self.job_queue().get_job_count_ge(JobType::Client)).unwrap_or(u32::MAX)
    }

    fn has_current_ledger(&self) -> bool {
        self.status_rpc_current_ledger_index().is_some()
            || self.live_current_ledger_index().is_some()
            || self.validated_ledger_seq().is_some()
            || self.closed_ledger_seq().is_some()
    }

    fn has_closed_ledger(&self) -> bool {
        self.closed_ledger().is_some()
    }

    fn path_search_max(&self) -> u32 {
        self.path_search_max()
    }

    fn path_search_old(&self) -> u32 {
        self.path_search_old()
    }

    fn path_search(&self) -> u32 {
        self.path_search()
    }

    fn path_search_fast(&self) -> u32 {
        self.path_search_fast()
    }

    fn network_synced(&self) -> bool {
        matches!(
            self.network_ops_operating_mode(),
            NetworkOpsOperatingMode::Tracking | NetworkOpsOperatingMode::Full
        )
    }

    fn current_ledger_index(&self) -> Option<u32> {
        self.status_rpc_current_ledger_index()
            .or_else(|| self.live_current_ledger_index())
            .or_else(|| self.validated_ledger_seq().map(|seq| seq.saturating_add(1)))
    }

    fn standalone(&self) -> bool {
        self.standalone()
    }

    fn ledger_accept(&self) -> Status {
        if !self.standalone() {
            return Status::new(RpcErrorCode::NotStandalone);
        }
        self.accept_standalone_ledger()
            .map(|_| Status::OK)
            .unwrap_or_else(|_| Status::new(RpcErrorCode::Internal))
    }

    fn stop(&self) -> Status {
        self.signal_stop("RPC stop command");
        Status::OK
    }

    fn peer_connect(&self, ip: String, port: u16) -> Status {
        if let Some(runtime) = self.overlay_runtime() {
            let address = format!("{}:{}", ip, port)
                .parse()
                .map_err(|_| Status::new(RpcErrorCode::InvalidParams));
            let address = match address {
                Ok(a) => a,
                Err(s) => return s,
            };
            let future = runtime.overlay().connect(address);
            tokio::spawn(async move {
                let _ = future.await;
            });
        }
        Status::OK
    }

    fn peers_get(&self) -> JsonValue {
        self.overlay_runtime()
            .as_ref()
            .map(|o| {
                let overlay = o.overlay();
                let peers = overlay.peers_json();
                protocol::json!({ "peers": peers })
            })
            .unwrap_or_else(|| protocol::json!({ "peers": [] }))
    }

    fn can_delete_get(&self) -> u32 {
        self.shamap_store_service()
            .map(|service| service.component().get_can_delete())
            .unwrap_or(0)
    }

    fn can_delete_enabled(&self) -> bool {
        self.shamap_store_service()
            .map(|service| service.component().advisory_delete())
            .unwrap_or(false)
    }

    fn can_delete_last_rotated(&self) -> u32 {
        self.shamap_store_service()
            .map(|service| service.component().get_last_rotated())
            .unwrap_or(0)
    }

    fn can_delete_seq_by_hash(&self, hash: Uint256) -> Option<u32> {
        self.ledger_master_runtime()
            .and_then(|runtime| {
                runtime
                    .ledger_master()
                    .get_ledger_by_hash(SHAMapHash::new(hash))
            })
            .map(|ledger| ledger.header().seq)
    }

    fn can_delete_set(&self, seq: u32) -> Status {
        match self.shamap_store_service() {
            Some(service) => match service.component().set_can_delete(seq) {
                Ok(_) => Status::OK,
                Err(_) => Status::new(RpcErrorCode::NotEnabled),
            },
            None => Status::new(RpcErrorCode::NotEnabled),
        }
    }

    fn ledger_cleaner_trigger(&self, params: &JsonValue) -> Status {
        let min_ledger = 0;
        let max_ledger = self
            .app()
            .and_then(|a| a.validated_ledger_seq())
            .unwrap_or(0);

        let mut request = ledger::LedgerCleanerRequest {
            validated_min: min_ledger,
            validated_max: max_ledger,
            ledger: None,
            min_ledger: Some(min_ledger),
            max_ledger: Some(max_ledger),
            full: None,
            fix_txns: None,
            check_nodes: None,
            stop: false,
        };

        if let JsonValue::Object(map) = params {
            if let Some(JsonValue::Unsigned(l)) = map.get("ledger") {
                request.ledger = Some(*l as u32);
            }
            if let Some(JsonValue::Unsigned(m)) = map.get("min_ledger") {
                request.min_ledger = Some(*m as u32);
            }
            if let Some(JsonValue::Unsigned(m)) = map.get("max_ledger") {
                request.max_ledger = Some(*m as u32);
            }
            if let Some(JsonValue::Bool(f)) = map.get("full") {
                request.full = Some(*f);
            }
            if let Some(JsonValue::Bool(f)) = map.get("fix_txns") {
                request.fix_txns = Some(*f);
            }
            if let Some(JsonValue::Bool(c)) = map.get("check_nodes") {
                request.check_nodes = Some(*c);
            }
            if let Some(JsonValue::Bool(s)) = map.get("stop") {
                request.stop = *s;
            }
        }

        if let Some(app) = self.app() {
            app.get_ledger_cleaner().clean(request);
        }
        Status::OK
    }

    fn ledger_request(&self, seq: u32) -> Status {
        if seq == 0 {
            return Status::make_param_error("Ledger index too small");
        }

        let acquired = match request_ledger_via_overlay(self, LedgerRequestTarget::Seq(seq)) {
            Ok(ledger) => ledger,
            Err(status) => return status,
        };

        let Some(acquired_ledger) = acquired else {
            return Status::new(RpcErrorCode::LedgerNotFound);
        };

        let Some(ledger_master_runtime) = self.ledger_master_runtime() else {
            return Status::with_message(
                RpcErrorCode::Internal,
                "LedgerMaster runtime is not configured.",
            );
        };

        let persistence = LedgerPersistence::new(Arc::new(self.build_ledger_persistence_runtime()));
        let is_current = self
            .validated_ledger_seq()
            .is_none_or(|validated| acquired_ledger.header().seq >= validated);

        if ledger_master_runtime
            .ledger_master()
            .set_full_ledger(
                &persistence,
                Arc::clone(&acquired_ledger),
                true,
                is_current,
                None,
                None,
            )
            .is_err()
        {
            return Status::new(RpcErrorCode::Internal);
        }

        if is_current {
            self.on_closed_ledger(Arc::clone(&acquired_ledger));
            self.on_published_ledger(Arc::clone(&acquired_ledger));
            let _ = self.on_validated_ledger(Arc::clone(&acquired_ledger));
            self.set_status_rpc_current_ledger_index(Some(
                acquired_ledger.header().seq.saturating_add(1),
            ));
            self.set_need_network_ledger(false);
            if matches!(
                self.network_ops_operating_mode(),
                NetworkOpsOperatingMode::Disconnected | NetworkOpsOperatingMode::Connected
            ) {
                let _ = self.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
            }
        }

        Status::OK
    }

    fn ledger_request_by_hash(&self, hash: Uint256) -> Status {
        let acquired = match request_ledger_via_overlay(
            self,
            LedgerRequestTarget::Hash(SHAMapHash::new(hash)),
        ) {
            Ok(ledger) => ledger,
            Err(status) => return status,
        };

        let Some(acquired_ledger) = acquired else {
            return Status::new(RpcErrorCode::LedgerNotFound);
        };

        let Some(ledger_master_runtime) = self.ledger_master_runtime() else {
            return Status::with_message(
                RpcErrorCode::Internal,
                "LedgerMaster runtime is not configured.",
            );
        };

        let persistence = LedgerPersistence::new(Arc::new(self.build_ledger_persistence_runtime()));
        let is_current = self
            .validated_ledger_seq()
            .is_none_or(|validated| acquired_ledger.header().seq >= validated);

        if ledger_master_runtime
            .ledger_master()
            .set_full_ledger(
                &persistence,
                Arc::clone(&acquired_ledger),
                true,
                is_current,
                None,
                None,
            )
            .is_err()
        {
            return Status::new(RpcErrorCode::Internal);
        }

        if is_current {
            self.on_closed_ledger(Arc::clone(&acquired_ledger));
            self.on_published_ledger(Arc::clone(&acquired_ledger));
            let _ = self.on_validated_ledger(Arc::clone(&acquired_ledger));
            self.set_status_rpc_current_ledger_index(Some(
                acquired_ledger.header().seq.saturating_add(1),
            ));
            self.set_need_network_ledger(false);
            if matches!(
                self.network_ops_operating_mode(),
                NetworkOpsOperatingMode::Disconnected | NetworkOpsOperatingMode::Connected
            ) {
                let _ = self.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
            }
        }

        Status::OK
    }

    fn log_level_set(&self, partition: String, _level: String) -> Status {
        self.logs().journal(&partition); // Simplified: actual level set not in AppLogs
        Status::OK
    }

    fn log_level_get(&self) -> JsonValue {
        protocol::json!({ "levels": {} })
    }

    fn log_rotate(&self) -> Status {
        // Log rotation not yet in AppLogs
        Status::OK
    }

    fn peer_reservations_add(
        &self,
        public_key: protocol::PublicKey,
        description: String,
    ) -> Status {
        self.get_peer_reservations()
            .insert_or_assign(PeerReservation::new(public_key, description));
        Status::OK
    }

    fn peer_reservations_del(&self, public_key: protocol::PublicKey) -> Status {
        self.get_peer_reservations().erase(&public_key);
        Status::OK
    }

    fn peer_reservations_list(&self) -> JsonValue {
        let list = self.get_peer_reservations().list();
        protocol::json!({
            "reservations": list.into_iter().map(|r| r.to_json()).collect::<Vec<_>>()
        })
    }

    fn export_snapshot(&self, output_path: &str) -> Result<JsonValue, String> {
        use nodestore::snapshot::{SnapshotManifest, manifest::SNAPSHOT_VERSION, export_snapshot};
        use std::path::Path;

        let validated = self.validated_ledger()
            .ok_or_else(|| "No validated ledger available".to_owned())?;
        let header = validated.header();

        let node_store = self.node_store().as_ref()
            .ok_or_else(|| "NodeStore not configured".to_owned())?;
        let backend = node_store.export_backend()
            .ok_or_else(|| "Backend not available for export".to_owned())?;

        let manifest = SnapshotManifest {
            version: SNAPSHOT_VERSION,
            ledger_seq: header.seq,
            ledger_hash: *header.hash.as_uint256().data(),
            account_hash: *header.account_hash.as_uint256().data(),
            tx_hash: *header.tx_hash.as_uint256().data(),
            parent_hash: *header.parent_hash.as_uint256().data(),
            drops: header.drops,
            close_time: header.close_time,
            parent_close_time: header.parent_close_time,
            close_time_res: header.close_time_resolution,
            close_flags: header.close_flags,
            chunks: Vec::new(),
        };

        let path = Path::new(output_path);
        export_snapshot(backend.as_ref(), &manifest, path)
            .map_err(|e| format!("{e}"))?;

        Ok(protocol::json!({
            "status": "success",
            "ledger_seq": header.seq,
            "ledger_hash": header.hash.to_string(),
            "account_hash": header.account_hash.to_string(),
            "output": output_path
        }))
    }
}

impl crate::commands::black_list::BlackListSource for ApplicationRoot {
    fn black_list_json(&self) -> JsonValue {
        JsonValue::from(self.get_resource_manager().get_json())
    }

    fn black_list_json_with_threshold(&self, threshold: i64) -> JsonValue {
        JsonValue::from(
            self.get_resource_manager()
                .get_json_with_threshold(threshold),
        )
    }
}

pub struct RpcRequestContext<'a, Env, Runtime = ()> {
    pub params: &'a JsonValue,
    pub env: &'a Env,
    pub runtime: &'a Runtime,
    pub role: Role,
    pub api_version: u32,
    pub headers: JsonContextHeaders<'a>,
    pub request_headers: BTreeMap<String, String>,
    pub unlimited: bool,
    pub remote_ip: Option<IpAddr>,
    pub load_type: RpcLoadType,
}

impl<'a, Env, Runtime> RpcRequestContext<'a, Env, Runtime> {
    pub fn json_context(&self) -> JsonContext<'a, Env> {
        if let Some(client_ip) = self.remote_ip {
            tracing::debug!(target: "rpc", role = ?self.role, ip = %client_ip, "RPC access check");
        }
        JsonContext {
            params: self.params,
            env: self.env,
            role: self.role,
            api_version: self.api_version,
            headers: self.headers,
            unlimited: self.unlimited,
        }
    }

    pub fn websocket_session(&self, remote_endpoint: SocketAddr) -> WsInfoSub {
        WsInfoSub::from_request(
            InfoSub::new(self.role),
            remote_endpoint,
            self.request_headers.clone(),
            self.api_version,
            (!self.headers.user.is_empty()).then_some(self.headers.user),
            (!self.headers.forwarded_for.is_empty()).then_some(self.headers.forwarded_for),
        )
    }

    pub fn remote_ip_or_internal(&self) -> Result<IpAddr, Status> {
        self.remote_ip
            .ok_or_else(|| Status::new(RpcErrorCode::Internal))
    }
}

#[cfg(test)]
mod tests;
