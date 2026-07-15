//! `InboundLedger` helpers above the landed `Ledger` and `SHAMap` owners.
//!
//! This module ports the planning, packet-ingestion, and local
//! queued-dispatch logic from `xrpld/app/ledger/detail/the reference source`:
//! - `neededHashes(...)`,
//! - `neededTxHashes(...)`,
//! - `neededStateHashes(...)`,
//! - the hash-only portion of `getNeededHashes()`,
//! - `takeHeader(...)`,
//! - `takeAsRootNode(...)`,
//! - `takeTxRootNode(...)`,
//! - `receiveNode(...)`,
//! - `gotData(...)`,
//! - the local non-network half of `processData(...)`,
//! - and the local queued-drain half of `runData()`.

use basics::base_uint::Uint256;
use basics::blob::Blob;
use basics::random::rand_int_to;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::CacheClock;
use overlay::{ProtocolMessage, ProtocolPayload, TmGetLedger, TmGetObjectByHash};
use protocol::JsonValue;
use shamap::family::{FullBelowCache, MissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher};
use shamap::fetch::SHAMapSyncFilter;
use shamap::sync::{MissingNodeRef, SHAMapAddNode, SHAMapMissingNode, SHAMapType, SyncTree};
use std::collections::BTreeMap;
use std::hash::BuildHasher;
use time::Duration;

use crate::fetch_pack::LedgerSyncFilterStore;
use crate::{
    AccountStateSF, FetchPackContainer, Ledger, LedgerConfig, TransactionStateSF,
    XRP_LEDGER_EARLIEST_FEES, calculate_ledger_hash, deserialize_ledger_header,
    deserialize_prefixed_ledger_header, serialize_prefixed_ledger_header,
};
use crate::{Fees, Rules};
use shamap::storage::NodeObjectType;
use shamap::tree_node::SHAMapTreeNode;

pub const INBOUND_LEDGER_MAX_NEEDED_STATE_HASHES: i32 = 4;
pub const INBOUND_LEDGER_MAX_NEEDED_TX_HASHES: i32 = 4;
pub const INBOUND_LEDGER_MAX_USEFUL_PEERS: usize = 6;
const TM_GET_OBJECT_BY_HASH_LEDGER: i32 = 1;
const TM_GET_OBJECT_BY_HASH_TRANSACTION_NODE: i32 = 3;
const TM_GET_OBJECT_BY_HASH_STATE_NODE: i32 = 4;
const TM_GET_LEDGER_BASE: i32 = 0;
const TM_GET_LEDGER_TX_NODE: i32 = 1;
const TM_GET_LEDGER_AS_NODE: i32 = 2;
const TM_QUERY_INDIRECT: i32 = 0;
fn next_missing_scan_first_child() -> u8 {
    rand_int_to(255u8)
}

// Acquisition constants
const INBOUND_LEDGER_TIMEOUT_RETRIES_MAX: u32 = 6;
const INBOUND_LEDGER_BECOME_AGGRESSIVE: u32 = 4;
const MISSING_NODES_FIND: i32 = 256;
/// During cold-start state acquisition, discover more missing nodes per cycle
/// to supply work for parallel fan-out across multiple peers (6 peers × 128 nodes = 768).
const MISSING_NODES_FIND_COLD_START: i32 = 1024;
const REQ_NODES_REPLY: usize = 128;
const REQ_NODES: usize = 12;

fn full_sync_debug_enabled() -> bool {
    std::env::var("XRPLD_FULL_SYNC_DEBUG")
        .map(|value| value != "0")
        .unwrap_or(false)
}

fn acq_packet_debug_enabled() -> bool {
    std::env::var("XRPLD_ACQ_PACKET_DEBUG")
        .map(|value| value != "0")
        .unwrap_or(false)
}

fn acq_packet_debug_verbose_enabled() -> bool {
    std::env::var("XRPLD_ACQ_PACKET_DEBUG_VERBOSE")
        .map(|value| value != "0")
        .unwrap_or(false)
}

fn inbound_ledger_timeout_retries_max() -> u32 {
    std::env::var("XRPLD_INBOUND_LEDGER_TIMEOUT_RETRIES_MAX")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value >= INBOUND_LEDGER_TIMEOUT_RETRIES_MAX)
        .unwrap_or(INBOUND_LEDGER_TIMEOUT_RETRIES_MAX)
}

fn log_acq_request_nodes(
    seq: u32,
    map: &str,
    root: SHAMapHash,
    missing: usize,
    fresh: usize,
    limit: usize,
    reason: InboundLedgerRequestTrigger,
    recent_nodes: usize,
    query_depth: u32,
    query_type: Option<i32>,
    node_ids: &[shamap::node_id::SHAMapNodeId],
) {
    if !acq_packet_debug_enabled() {
        return;
    }

    let requested = node_ids.len();
    let min_depth = node_ids.iter().map(|id| id.get_depth()).min().unwrap_or(0);
    let max_depth = node_ids.iter().map(|id| id.get_depth()).max().unwrap_or(0);
    let first = node_ids
        .first()
        .map(|id| id.to_string())
        .unwrap_or_else(|| "none".to_owned());
    let last = node_ids
        .last()
        .map(|id| id.to_string())
        .unwrap_or_else(|| "none".to_owned());

    tracing::debug!(target: "ledger",
        "[acq][request_shape] seq={} map={} root={} missing={} fresh={} limit={} requested={} min_depth={} max_depth={} first={} last={} reason={:?} recent_nodes={} query_depth={} query_type={}",
        seq,
        map,
        root,
        missing,
        fresh,
        limit,
        requested,
        min_depth,
        max_depth,
        first,
        last,
        reason,
        recent_nodes,
        query_depth,
        query_type
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned())
    );
}

/// Build a GetLedger request with specific node IDs.
///
/// the function, then sets `ledgerseq` from `mLedger->header().seq` for node
/// requests. Both fields are always present for node requests.
pub fn make_get_ledger_with_node_ids(
    hash: SHAMapHash,
    seq: u32,
    itype: i32,
    node_ids: &[shamap::node_id::SHAMapNodeId],
    query_depth: u32,
    query_type: Option<i32>,
) -> ProtocolMessage {
    ProtocolMessage::new(ProtocolPayload::GetLedger(TmGetLedger {
        itype,
        ltype: None,
        ledger_hash: Some(hash.as_uint256().data().to_vec()),
        ledger_seq: (seq != 0).then_some(seq),
        node_i_ds: node_ids.iter().map(|id| id.get_raw_string()).collect(),
        request_cookie: None,
        query_type,
        query_depth: Some(query_depth),
    }))
}

fn missing_hashes_from_walk(
    missing_nodes: Vec<SHAMapMissingNode>,
    object_type: InboundLedgerObjectType,
) -> Vec<(InboundLedgerObjectType, Uint256)> {
    missing_nodes
        .into_iter()
        .filter_map(|node| match node.locator() {
            MissingNodeRef::Hash(hash) => Some((object_type, *hash.as_uint256())),
            MissingNodeRef::Id(_) => None,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InboundLedgerReason {
    History,
    #[default]
    Generic,
    Consensus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundLedgerCompletionDisposition {
    Complete(InboundLedgerReason),
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum InboundLedgerObjectType {
    Ledger = 1,
    TransactionNode = 3,
    StateNode = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum InboundLedgerDataType {
    Base = 0,
    TransactionNode = 1,
    StateNode = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundLedgerNodeData {
    pub node_id: Option<Blob>,
    pub node_data: Blob,
}

impl InboundLedgerNodeData {
    pub fn new(node_id: Option<Blob>, node_data: Blob) -> Self {
        Self { node_id, node_data }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundLedgerPacket {
    pub packet_type: InboundLedgerDataType,
    pub nodes: Vec<InboundLedgerNodeData>,
}

impl InboundLedgerPacket {
    pub fn new(packet_type: InboundLedgerDataType, nodes: Vec<InboundLedgerNodeData>) -> Self {
        Self { packet_type, nodes }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundLedgerPacketError {
    EmptyNodes,
    InvalidHeader,
    MissingNodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundLedgerReceivedPacket {
    pub peer_id: Option<u64>,
    pub packet: InboundLedgerPacket,
}

impl InboundLedgerReceivedPacket {
    pub fn new(peer_id: Option<u64>, packet: InboundLedgerPacket) -> Self {
        Self { peer_id, packet }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InboundLedgerPacketShape {
    pub nodes: usize,
    pub inner_nodes: usize,
    pub leaf_nodes: usize,
    pub malformed_nodes: usize,
    pub empty_nodes: usize,
}

impl InboundLedgerPacketShape {
    pub fn classify(packet: &InboundLedgerPacket) -> Self {
        let mut shape = Self {
            nodes: packet.nodes.len(),
            ..Self::default()
        };

        for node in &packet.nodes {
            if node.node_data.is_empty() {
                shape.empty_nodes += 1;
                continue;
            }

            match SHAMapTreeNode::make_from_wire(&node.node_data) {
                Ok(Some(decoded)) if decoded.is_inner() => shape.inner_nodes += 1,
                Ok(Some(_)) => shape.leaf_nodes += 1,
                Ok(None) => {} // empty node_data — not malformed, just skip
                Err(_) => shape.malformed_nodes += 1,
            }
        }

        shape
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundLedgerPacketDebugStats {
    pub peer_id: Option<u64>,
    pub packet_type: InboundLedgerDataType,
    pub shape: InboundLedgerPacketShape,
    pub useful: i32,
    pub invalid: i32,
    pub duplicate: i32,
    pub elapsed_ms: u128,
}

impl Default for InboundLedgerPacketDebugStats {
    fn default() -> Self {
        Self {
            peer_id: None,
            packet_type: InboundLedgerDataType::Base,
            shape: InboundLedgerPacketShape::default(),
            useful: 0,
            invalid: 0,
            duplicate: 0,
            elapsed_ms: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboundLedgerPeerScore {
    pub peer_id: u64,
    pub useful_count: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InboundLedgerRunDataResult {
    pub triggered_peer_ids: Vec<u64>,
    pub processed_packets: usize,
    pub max_useful_count: i32,
    pub packet_stats: Vec<InboundLedgerPacketDebugStats>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InboundLedgerRequest {
    pub peer_ids: Vec<u64>,
    pub message: ProtocolMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InboundLedgerPlannerState {
    pub have_header: bool,
    pub have_state: bool,
    pub have_transactions: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundLedgerRequestTrigger {
    Blind,
    Reply,
    /// Reply from a high-latency peer — reference sets querydepth(2) for these.
    ReplyHighLatency,
    Timeout,
    Added,
}

pub trait InboundLedgerJournal {
    fn trace(&self, message: &str);
    fn debug(&self, message: &str);
    fn warn(&self, message: &str);
    fn fatal(&self, message: &str);
}

#[derive(Debug, Default)]
pub struct NullInboundLedgerJournal;

impl InboundLedgerJournal for NullInboundLedgerJournal {
    fn trace(&self, _message: &str) {}

    fn debug(&self, _message: &str) {}

    fn warn(&self, _message: &str) {}

    fn fatal(&self, _message: &str) {}
}

pub trait InboundLedgerStore {
    fn fetch_ledger_header(&mut self, hash: SHAMapHash, ledger_seq: u32) -> Option<Blob>;
    fn store_ledger_header(&mut self, data: Blob, hash: SHAMapHash, ledger_seq: u32);
    fn store_shamap_node(
        &mut self,
        object_type: NodeObjectType,
        data: Blob,
        hash: Uint256,
        ledger_seq: u32,
    );

    /// Check if this hash should be stored. Returns false for duplicates.
    fn should_store_hash(&mut self, hash: Uint256) -> bool {
        let _ = hash;
        true
    }

    /// Fetch node data from local persistent storage (NuDB).
    /// Used for resume-from-disk on restart.
    fn fetch_node_data(&self, hash: Uint256) -> Option<Blob> {
        let _ = hash;
        None
    }
}

impl<T> InboundLedgerStore for &mut T
where
    T: InboundLedgerStore + ?Sized,
{
    fn fetch_ledger_header(&mut self, hash: SHAMapHash, ledger_seq: u32) -> Option<Blob> {
        (**self).fetch_ledger_header(hash, ledger_seq)
    }

    fn store_ledger_header(&mut self, data: Blob, hash: SHAMapHash, ledger_seq: u32) {
        (**self).store_ledger_header(data, hash, ledger_seq);
    }

    fn store_shamap_node(
        &mut self,
        object_type: NodeObjectType,
        data: Blob,
        hash: Uint256,
        ledger_seq: u32,
    ) {
        (**self).store_shamap_node(object_type, data, hash, ledger_seq);
    }
}

pub struct InboundLedgerSyncStore<'a, T: ?Sized>(pub &'a mut T);

impl<T> LedgerSyncFilterStore for InboundLedgerSyncStore<'_, T>
where
    T: InboundLedgerStore + ?Sized,
{
    fn store_shamap_node(
        &mut self,
        object_type: NodeObjectType,
        data: Blob,
        hash: Uint256,
        ledger_seq: u32,
    ) {
        self.0
            .store_shamap_node(object_type, data, hash, ledger_seq);
    }

    fn should_store_hash(&mut self, hash: Uint256) -> bool {
        self.0.should_store_hash(hash)
    }

    fn fetch_node_data(&self, hash: Uint256) -> Option<Blob> {
        self.0.fetch_node_data(hash)
    }
}

/// No-op store — used during getMissingNodes/trigger paths.
pub struct NullSyncStore;
impl LedgerSyncFilterStore for NullSyncStore {
    fn store_shamap_node(&mut self, _: NodeObjectType, _: Blob, _: Uint256, _: u32) {}
}

#[derive(Debug, Clone)]
pub struct InboundLedgerLocal {
    hash: SHAMapHash,
    seq: u32,
    reason: InboundLedgerReason,
    last_action: Duration,
    ledger: Option<Ledger>,
    planner_state: InboundLedgerPlannerState,
    failed: bool,
    complete: bool,
    signaled: bool,
    progress: bool,
    stats: SHAMapAddNode,
    receive_dispatched: bool,
    received_data: Vec<InboundLedgerReceivedPacket>,
    timeouts: u32,
    by_hash: bool,
    recent_nodes: std::collections::HashSet<Uint256>,
    /// When true, skip state tree acquisition. Caller will build state
    /// locally from parent ledger + transaction set.
    pub skip_state: bool,
}

impl InboundLedgerLocal {
    pub fn new(hash: SHAMapHash, seq: u32) -> Self {
        Self::new_with_reason(hash, seq, InboundLedgerReason::Generic)
    }

    pub fn new_with_reason(hash: SHAMapHash, seq: u32, reason: InboundLedgerReason) -> Self {
        let ledger_hash = hash;
        tracing::info!(target: "ledger", seq, hash = %ledger_hash, "Ledger acquisition started");
        Self {
            hash,
            seq,
            reason,
            last_action: Duration::ZERO,
            ledger: None,
            planner_state: InboundLedgerPlannerState::default(),
            failed: false,
            complete: false,
            signaled: false,
            progress: false,
            stats: SHAMapAddNode::default(),
            receive_dispatched: false,
            received_data: Vec::new(),
            timeouts: 0,
            by_hash: false,
            recent_nodes: std::collections::HashSet::new(),
            skip_state: false,
        }
    }

    pub fn hash(&self) -> SHAMapHash {
        self.hash
    }

    pub fn seq(&self) -> u32 {
        self.seq
    }

    pub fn reason(&self) -> InboundLedgerReason {
        self.reason
    }

    pub fn touch(&mut self, now: Duration) {
        self.last_action = now;
    }

    pub fn last_action(&self) -> Duration {
        self.last_action
    }

    pub fn update(&mut self, seq: u32, now: Duration) {
        if seq != 0 && self.seq == 0 {
            self.seq = seq;
        }

        self.touch(now);
    }

    pub fn ledger(&self) -> Option<&Ledger> {
        self.ledger.as_ref()
    }

    pub fn ledger_mut(&mut self) -> Option<&mut Ledger> {
        self.ledger.as_mut()
    }

    pub fn planner_state(&self) -> InboundLedgerPlannerState {
        self.planner_state
    }

    pub fn is_failed(&self) -> bool {
        self.failed
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn set_complete(&mut self) {
        self.complete = true;
    }

    pub fn is_done(&self) -> bool {
        self.complete || self.failed
    }

    pub fn completion_disposition(&self) -> Option<InboundLedgerCompletionDisposition> {
        if !self.signaled || !self.is_done() {
            return None;
        }

        if self.failed {
            Some(InboundLedgerCompletionDisposition::Failed)
        } else {
            Some(InboundLedgerCompletionDisposition::Complete(self.reason))
        }
    }

    pub fn accept_completed_ledger(&mut self) -> Option<InboundLedgerCompletionDisposition> {
        let disposition = self.completion_disposition()?;
        if let InboundLedgerCompletionDisposition::Complete(_) = disposition {
            let ledger = self
                .ledger
                .as_mut()
                .expect("completed inbound ledger must hold a ledger");
            if !ledger.is_immutable() {
                // The acquisition worker only observes completion; the owner
                // performs the final acceptance step.
                ledger.set_immutable(false);
            }
            ledger.set_full();
        }
        Some(disposition)
    }

    pub fn reset_failed(&mut self) {
        self.failed = false;
        self.timeouts = 0;
        self.progress = false;
        self.by_hash = false;
        self.signaled = false;
    }
    pub fn progress(&self) -> bool {
        self.progress
    }

    pub fn clear_progress(&mut self) {
        self.progress = false;
    }

    /// Clear recent_nodes filter — matches reference onTimer which always clears
    /// mRecentNodes on every timer tick regardless of progress.
    pub fn clear_recent_nodes(&mut self) {
        self.recent_nodes.clear();
    }

    pub fn stats(&self) -> SHAMapAddNode {
        self.stats
    }

    pub fn receive_dispatched(&self) -> bool {
        self.receive_dispatched
    }

    pub fn received_data_len(&self) -> usize {
        self.received_data.len()
    }

    pub fn got_data(&mut self, peer_id: Option<u64>, packet: InboundLedgerPacket) -> bool {
        if self.is_done() {
            return false;
        }

        self.received_data
            .push(InboundLedgerReceivedPacket::new(peer_id, packet));

        if self.receive_dispatched {
            return false;
        }

        self.receive_dispatched = true;
        true
    }

    pub fn check_local_with_family_and_config<CLOCK, S, C, F, MR, NS, DB, FP, J>(
        &mut self,
        journal: &J,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> bool
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        if self.is_done() {
            return false;
        }

        self.try_db_with_family_and_config(journal, config, store, fetch_pack, family);
        self.is_done()
    }

    pub fn get_needed_hashes_with_family<CLOCK, S, C, F, MR, NS>(
        &mut self,
        state_filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        tx_filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Vec<(InboundLedgerObjectType, Uint256)>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        get_needed_hashes_with_family(
            self.hash,
            self.ledger.as_mut(),
            self.planner_state,
            state_filter,
            tx_filter,
            family,
        )
    }

    pub fn make_header_request(&self) -> ProtocolMessage {
        let ledger_seq = (self.seq != 0).then_some(self.seq);
        // querytype=qtINDIRECT when timeouts!=0, for ALL request types
        // including header requests.
        let query_type = if self.timeouts > 0 {
            Some(TM_QUERY_INDIRECT)
        } else {
            None
        };
        ProtocolMessage::new(ProtocolPayload::GetLedger(TmGetLedger {
            itype: TM_GET_LEDGER_BASE,
            ltype: None,
            ledger_hash: Some(self.hash.as_uint256().data().to_vec()),
            ledger_seq,
            node_i_ds: Vec::new(),
            request_cookie: None,
            query_type,
            query_depth: None,
        }))
    }

    pub fn make_needed_by_hash_request<CLOCK, S, C, F, MR, NS>(
        &mut self,
        state_filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        tx_filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Option<ProtocolMessage>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        let needed = self.get_needed_hashes_with_family(state_filter, tx_filter, family);
        make_inbound_needed_by_hash_request(self.hash, self.seq, &needed)
    }

    pub fn reopen_if_maps_incomplete_with_family<CLOCK, S, C, F, MR, NS>(
        &mut self,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Option<ProtocolMessage>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        if !self.complete || !self.planner_state.have_header {
            return None;
        }

        let ledger = self.ledger.as_mut()?;
        let mut needed = Vec::new();

        if self.planner_state.have_state && !ledger.header().account_hash.is_zero() {
            let mut missing = Vec::new();
            ledger.state_map().walk_map_with_family(
                SHAMapType::State,
                &mut missing,
                MISSING_NODES_FIND,
                family,
            );
            if !missing.is_empty() {
                needed.extend(missing_hashes_from_walk(
                    missing,
                    InboundLedgerObjectType::StateNode,
                ));
                self.planner_state.have_state = false;
                ledger.state_map_mut().set_synching();
            }
        }

        if self.planner_state.have_transactions && !ledger.header().tx_hash.is_zero() {
            let mut missing = Vec::new();
            ledger.tx_map().walk_map_with_family(
                SHAMapType::Transaction,
                &mut missing,
                MISSING_NODES_FIND,
                family,
            );
            if !missing.is_empty() {
                needed.extend(missing_hashes_from_walk(
                    missing,
                    InboundLedgerObjectType::TransactionNode,
                ));
                self.planner_state.have_transactions = false;
                ledger.tx_map_mut().set_synching();
            }
        }

        if needed.is_empty() {
            return None;
        }

        self.complete = false;
        self.signaled = false;
        make_inbound_needed_by_hash_request(self.hash, self.seq, &needed)
    }

    pub fn revalidate_map_sync_with_family<CLOCK, S, C, F, MR, NS>(
        &mut self,
        _family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> bool
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        let Some(ledger) = self.ledger.as_mut() else {
            return false;
        };

        let mut reactivated = false;
        let state_valid = ledger.header().account_hash.is_zero() || ledger.state_map().is_valid();
        let state_complete =
            ledger.header().account_hash.is_zero() || !ledger.state_map().is_synching();
        let tx_valid = ledger.header().tx_hash.is_zero() || ledger.tx_map().is_valid();
        let tx_complete = ledger.header().tx_hash.is_zero() || !ledger.tx_map().is_synching();

        if self.planner_state.have_state && !state_complete {
            self.planner_state.have_state = false;
            reactivated = true;
        } else if !self.planner_state.have_state && state_complete && state_valid {
            self.planner_state.have_state = true;
        }

        if self.planner_state.have_transactions && !tx_complete {
            self.planner_state.have_transactions = false;
            reactivated = true;
        } else if !self.planner_state.have_transactions && tx_complete && tx_valid {
            self.planner_state.have_transactions = true;
        }

        if !state_valid || !tx_valid {
            self.failed = true;
            self.complete = false;
        } else if self.skip_state && self.planner_state.have_transactions {
            // TX-only mode: complete when we have header + transactions.
            // Caller will build state locally from parent ledger.
            self.complete = true;
        } else if self.planner_state.have_state && self.planner_state.have_transactions {
            self.complete = true;
        } else if reactivated {
            self.complete = false;
            self.signaled = false;
        }

        reactivated
    }

    pub fn maybe_finish<J>(&mut self, journal: &J)
    where
        J: InboundLedgerJournal,
    {
        self.finish_if_done(journal);
    }

    pub fn get_info_with_family<CLOCK, S, C, F, MR, NS>(
        &mut self,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> JsonValue
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        let mut entry = BTreeMap::new();

        entry.insert("hash".to_owned(), JsonValue::String(self.hash.to_string()));
        if self.is_complete() {
            entry.insert("complete".to_owned(), JsonValue::Bool(true));
        }
        if self.is_failed() {
            entry.insert("failed".to_owned(), JsonValue::Bool(true));
        }
        entry.insert(
            "have_header".to_owned(),
            JsonValue::Bool(self.planner_state.have_header),
        );
        if self.planner_state.have_header {
            entry.insert(
                "have_state".to_owned(),
                JsonValue::Bool(self.planner_state.have_state),
            );
            entry.insert(
                "have_transactions".to_owned(),
                JsonValue::Bool(self.planner_state.have_transactions),
            );
        }
        entry.insert("timeouts".to_owned(), JsonValue::Unsigned(0));

        if self.planner_state.have_header {
            if !self.planner_state.have_state {
                let mut state_filter = None;
                let mut tx_filter = None;
                let hashes = self
                    .get_needed_hashes_with_family(&mut state_filter, &mut tx_filter, family)
                    .into_iter()
                    .filter_map(|(object_type, hash)| {
                        (object_type == InboundLedgerObjectType::StateNode)
                            .then_some(JsonValue::String(hash.to_string()))
                    })
                    .collect();
                entry.insert("needed_state_hashes".to_owned(), JsonValue::Array(hashes));
            }

            if !self.planner_state.have_transactions {
                let mut state_filter = None;
                let mut tx_filter = None;
                let hashes = self
                    .get_needed_hashes_with_family(&mut state_filter, &mut tx_filter, family)
                    .into_iter()
                    .filter_map(|(object_type, hash)| {
                        (object_type == InboundLedgerObjectType::TransactionNode)
                            .then_some(JsonValue::String(hash.to_string()))
                    })
                    .collect();
                entry.insert(
                    "needed_transaction_hashes".to_owned(),
                    JsonValue::Array(hashes),
                );
            }
        }

        JsonValue::Object(entry)
    }

    pub fn process_packet_and_update_with_family_and_config<CLOCK, S, C, F, MR, NS, DB, FP, J>(
        &mut self,
        packet: &InboundLedgerPacket,
        journal: &J,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> i32
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        self.process_packet_and_update_stats_with_family_and_config(
            packet, journal, config, store, fetch_pack, family,
        )
        .map(|san| san.get_good())
        .unwrap_or(-1)
    }

    pub fn process_packet_and_update_stats_with_family_and_config<
        CLOCK,
        S,
        C,
        F,
        MR,
        NS,
        DB,
        FP,
        J,
    >(
        &mut self,
        packet: &InboundLedgerPacket,
        journal: &J,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Option<SHAMapAddNode>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        let Ok(san) = self.process_packet_with_family_and_config(
            packet, journal, config, store, fetch_pack, family,
        ) else {
            return None;
        };

        if san.is_useful() {
            self.progress = true;
        }
        self.stats += san;
        {
            let missing_nodes = 0u32; // exact count requires tree walk
            let total_nodes = self.stats.get_good().max(0) as u32;
            let progress = format!("{}%", if total_nodes > 0 { 100 } else { 0 });
            tracing::debug!(target: "ledger", seq = self.seq, missing_nodes, total_nodes, pct = %progress, "Sync progress");
        }
        self.finish_if_done_with_family_and_config(journal, config, family);
        Some(san)
    }

    pub fn run_data_with_family_and_config_and_sampler<CLOCK, S, C, F, MR, NS, DB, FP, J, SAMPLE>(
        &mut self,
        journal: &J,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        sample_peers: &mut SAMPLE,
    ) -> InboundLedgerRunDataResult
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
        SAMPLE: FnMut(&[InboundLedgerPeerScore], usize) -> Vec<u64>,
    {
        let mut result = InboundLedgerRunDataResult::default();
        let mut data_counts = InboundLedgerPeerDataCounts::default();

        loop {
            let batch = std::mem::take(&mut self.received_data);
            if batch.is_empty() {
                self.receive_dispatched = false;
                break;
            }

            result.processed_packets += batch.len();
            for entry in batch {
                let packet_type = entry.packet.packet_type;
                let shape = if acq_packet_debug_enabled() {
                    Some(InboundLedgerPacketShape::classify(&entry.packet))
                } else {
                    None
                };
                let start = std::time::Instant::now();
                let san = self.process_packet_and_update_stats_with_family_and_config(
                    &entry.packet,
                    journal,
                    config,
                    store,
                    fetch_pack,
                    family,
                );
                let count = san.map(|san| san.get_good()).unwrap_or(-1);
                {
                    let peer_id = entry.peer_id.unwrap_or(0);
                    let nodes_received = count.max(0) as u32;
                    tracing::debug!(target: "ledger", seq = self.seq, peer_id, nodes_received, "Ledger data received from peer");
                }
                if let Some(shape) = shape {
                    let stats = InboundLedgerPacketDebugStats {
                        peer_id: entry.peer_id,
                        packet_type,
                        shape,
                        useful: count.max(0),
                        invalid: san.map(|san| san.get_bad()).unwrap_or(1),
                        duplicate: san.map(|san| san.get_duplicate()).unwrap_or(0),
                        elapsed_ms: start.elapsed().as_millis(),
                    };
                    tracing::debug!(target: "ledger",
                        "[acq][packet_yield] seq={} peer={} type={:?} nodes={} inner={} leaf={} malformed={} empty={} useful={} invalid={} duplicate={} elapsed_ms={}",
                        self.seq,
                        stats
                            .peer_id
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "none".to_owned()),
                        stats.packet_type,
                        stats.shape.nodes,
                        stats.shape.inner_nodes,
                        stats.shape.leaf_nodes,
                        stats.shape.malformed_nodes,
                        stats.shape.empty_nodes,
                        stats.useful,
                        stats.invalid,
                        stats.duplicate,
                        stats.elapsed_ms
                    );
                    if stats.invalid > 0
                        || stats.shape.malformed_nodes > 0
                        || acq_packet_debug_verbose_enabled()
                    {
                        for (index, node) in entry.packet.nodes.iter().take(8).enumerate() {
                            tracing::debug!(target: "ledger",
                                "[acq][packet_node] seq={} peer={} type={:?} index={} node_id_len={} data_len={} last_type={}",
                                self.seq,
                                stats
                                    .peer_id
                                    .map(|id| id.to_string())
                                    .unwrap_or_else(|| "none".to_owned()),
                                stats.packet_type,
                                index,
                                node.node_id.as_ref().map(|id| id.len()).unwrap_or(0),
                                node.node_data.len(),
                                node.node_data.last().copied().unwrap_or(255)
                            );
                        }
                    }
                    result.packet_stats.push(stats);
                }
                data_counts.update(entry.peer_id, count);
            }
        }

        let candidates = data_counts.pruned_scores();
        result.max_useful_count = data_counts.max_count;
        result.triggered_peer_ids = sample_peers(&candidates, INBOUND_LEDGER_MAX_USEFUL_PEERS);
        result
    }

    pub fn run_data_with_family_and_config<CLOCK, S, C, F, MR, NS, DB, FP, J>(
        &mut self,
        journal: &J,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> InboundLedgerRunDataResult
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        self.run_data_with_family_and_config_and_sampler(
            journal,
            config,
            store,
            fetch_pack,
            family,
            &mut |scores, max| sample_peer_ids(scores, max),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run_data_with_family_and_config_and_refill<CLOCK, S, C, F, MR, NS, DB, FP, J, REFILL>(
        &mut self,
        journal: &J,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        refill_received_data: &mut REFILL,
    ) -> InboundLedgerRunDataResult
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
        REFILL: FnMut() -> Vec<InboundLedgerReceivedPacket>,
    {
        self.run_data_with_family_and_config_and_sampler_and_refill(
            journal,
            config,
            store,
            fetch_pack,
            family,
            &mut |scores, max| sample_peer_ids(scores, max),
            refill_received_data,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run_data_with_family_and_config_and_sampler_and_refill<
        CLOCK,
        S,
        C,
        F,
        MR,
        NS,
        DB,
        FP,
        J,
        SAMPLE,
        REFILL,
    >(
        &mut self,
        journal: &J,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        sample_peers: &mut SAMPLE,
        refill_received_data: &mut REFILL,
    ) -> InboundLedgerRunDataResult
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
        SAMPLE: FnMut(&[InboundLedgerPeerScore], usize) -> Vec<u64>,
        REFILL: FnMut() -> Vec<InboundLedgerReceivedPacket>,
    {
        let mut result = InboundLedgerRunDataResult::default();
        let mut data_counts = InboundLedgerPeerDataCounts::default();

        loop {
            let mut batch = std::mem::take(&mut self.received_data);
            if batch.is_empty() {
                let refill = refill_received_data();
                if refill.is_empty() {
                    self.receive_dispatched = false;
                    break;
                }
                batch = refill;
            }

            result.processed_packets += batch.len();
            for entry in batch {
                let packet_type = entry.packet.packet_type;
                let shape = if acq_packet_debug_enabled() {
                    Some(InboundLedgerPacketShape::classify(&entry.packet))
                } else {
                    None
                };
                let start = std::time::Instant::now();
                let san = self.process_packet_and_update_stats_with_family_and_config(
                    &entry.packet,
                    journal,
                    config,
                    store,
                    fetch_pack,
                    family,
                );
                let count = san.map(|san| san.get_good()).unwrap_or(-1);
                {
                    let peer_id = entry.peer_id.unwrap_or(0);
                    let nodes_received = count.max(0) as u32;
                    tracing::debug!(target: "ledger", seq = self.seq, peer_id, nodes_received, "Ledger data received from peer");
                }
                if let Some(shape) = shape {
                    let stats = InboundLedgerPacketDebugStats {
                        peer_id: entry.peer_id,
                        packet_type,
                        shape,
                        useful: count.max(0),
                        invalid: san.map(|san| san.get_bad()).unwrap_or(1),
                        duplicate: san.map(|san| san.get_duplicate()).unwrap_or(0),
                        elapsed_ms: start.elapsed().as_millis(),
                    };
                    tracing::debug!(target: "ledger",
                        "[acq][packet_yield] seq={} peer={} type={:?} nodes={} inner={} leaf={} malformed={} empty={} useful={} invalid={} duplicate={} elapsed_ms={}",
                        self.seq,
                        stats
                            .peer_id
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "none".to_owned()),
                        stats.packet_type,
                        stats.shape.nodes,
                        stats.shape.inner_nodes,
                        stats.shape.leaf_nodes,
                        stats.shape.malformed_nodes,
                        stats.shape.empty_nodes,
                        stats.useful,
                        stats.invalid,
                        stats.duplicate,
                        stats.elapsed_ms
                    );
                    if stats.invalid > 0
                        || stats.shape.malformed_nodes > 0
                        || acq_packet_debug_verbose_enabled()
                    {
                        for (index, node) in entry.packet.nodes.iter().take(8).enumerate() {
                            tracing::debug!(target: "ledger",
                                "[acq][packet_node] seq={} peer={} type={:?} index={} node_id_len={} data_len={} last_type={}",
                                self.seq,
                                stats
                                    .peer_id
                                    .map(|id| id.to_string())
                                    .unwrap_or_else(|| "none".to_owned()),
                                stats.packet_type,
                                index,
                                node.node_id.as_ref().map(|id| id.len()).unwrap_or(0),
                                node.node_data.len(),
                                node.node_data.last().copied().unwrap_or(255)
                            );
                        }
                    }
                    result.packet_stats.push(stats);
                }
                data_counts.update(entry.peer_id, count);
            }
        }

        let candidates = data_counts.pruned_scores();
        result.max_useful_count = data_counts.max_count;
        result.triggered_peer_ids = sample_peers(&candidates, INBOUND_LEDGER_MAX_USEFUL_PEERS);
        result
    }

    pub fn take_header_with_config_and_store<DB, J>(
        &mut self,
        data: &[u8],
        config: &LedgerConfig,
        store: &mut DB,
        journal: &J,
    ) -> bool
    where
        DB: InboundLedgerStore,
        J: InboundLedgerJournal,
    {
        journal.trace(&format!("got header acquiring ledger {}", self.hash));

        if self.complete || self.failed || self.planner_state.have_header {
            return true;
        }

        let Ok(mut header) = deserialize_ledger_header(data, false) else {
            self.ledger = None;
            return false;
        };
        header.hash = calculate_ledger_hash(&header);

        if header.hash != self.hash || (self.seq != 0 && self.seq != header.seq) {
            journal.warn(&format!(
                "Acquire hash mismatch: {}!={}",
                header.hash, self.hash
            ));
            self.ledger = None;
            return false;
        }

        if self.seq == 0 {
            self.seq = header.seq;
        }

        let mut ledger = build_loaded_ledger(header, config, self.seq);
        let prefixed = serialize_prefixed_ledger_header(&ledger.header(), false);
        store.store_ledger_header(prefixed, self.hash, self.seq);

        self.planner_state.have_header = true;
        self.planner_state.have_transactions = ledger.header().tx_hash.is_zero();
        self.planner_state.have_state = self.skip_state || ledger.header().account_hash.is_zero();
        if !self.skip_state {
            ledger.state_map_mut().set_synching();
        }
        ledger.tx_map_mut().set_synching();
        self.ledger = Some(ledger);
        true
    }

    pub fn take_as_root_node_with_family<CLOCK, S, C, F, MR, NS, DB, FP>(
        &mut self,
        data: &[u8],
        san: &mut SHAMapAddNode,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> bool
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
    {
        if self.failed || self.planner_state.have_state {
            san.inc_duplicate();
            return true;
        }

        let Some(ledger) = self.ledger.as_mut() else {
            san.inc_invalid();
            return false;
        };

        let account_hash = ledger.header().account_hash;
        let mut filter = AccountStateSF::new(InboundLedgerSyncStore(&mut *store), &mut *fetch_pack);
        let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
        *san += ledger.state_map_mut().add_root_node_with_family(
            account_hash,
            data,
            &mut filter_ref,
            family,
        );
        san.is_good()
    }

    pub fn take_tx_root_node_with_family<CLOCK, S, C, F, MR, NS, DB, FP>(
        &mut self,
        data: &[u8],
        san: &mut SHAMapAddNode,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> bool
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
    {
        if self.failed || self.planner_state.have_transactions {
            san.inc_duplicate();
            return true;
        }

        let Some(ledger) = self.ledger.as_mut() else {
            san.inc_invalid();
            return false;
        };

        let tx_hash = ledger.header().tx_hash;
        let mut filter =
            TransactionStateSF::new(InboundLedgerSyncStore(&mut *store), &mut *fetch_pack);
        let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
        *san +=
            ledger
                .tx_map_mut()
                .add_root_node_with_family(tx_hash, data, &mut filter_ref, family);
        san.is_good()
    }

    pub fn receive_node_packet_with_family_and_config<CLOCK, S, C, F, MR, NS, DB, FP, J>(
        &mut self,
        packet: &InboundLedgerPacket,
        san: &mut SHAMapAddNode,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        journal: &J,
    ) where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        if !self.planner_state.have_header {
            journal.warn("Missing ledger header");
            san.inc_invalid();
            return;
        }

        match packet.packet_type {
            InboundLedgerDataType::TransactionNode => {
                if self.planner_state.have_transactions || self.failed {
                    san.inc_duplicate();
                    return;
                }
                self.receive_tx_nodes(packet, san, config, store, fetch_pack, family, journal);
            }
            InboundLedgerDataType::StateNode => {
                if self.planner_state.have_state || self.failed {
                    san.inc_duplicate();
                    return;
                }
                self.receive_state_nodes(packet, san, config, store, fetch_pack, family, journal);
            }
            InboundLedgerDataType::Base => {
                san.inc_invalid();
            }
        }
    }

    pub fn receive_node_packet_with_family<CLOCK, S, C, F, MR, NS, DB, FP, J>(
        &mut self,
        packet: &InboundLedgerPacket,
        san: &mut SHAMapAddNode,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        journal: &J,
    ) where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        self.receive_node_packet_with_family_and_config(
            packet,
            san,
            &LedgerConfig::default(),
            store,
            fetch_pack,
            family,
            journal,
        );
    }

    pub fn process_packet_with_family_and_config<CLOCK, S, C, F, MR, NS, DB, FP, J>(
        &mut self,
        packet: &InboundLedgerPacket,
        journal: &J,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<SHAMapAddNode, InboundLedgerPacketError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        match packet.packet_type {
            InboundLedgerDataType::Base => {
                if packet.nodes.is_empty() {
                    journal.warn("empty header data");
                    return Err(InboundLedgerPacketError::EmptyNodes);
                }

                let mut san = SHAMapAddNode::default();
                let had_header = self.planner_state.have_header;
                if !had_header {
                    if !self.take_header_with_config_and_store(
                        &packet.nodes[0].node_data,
                        config,
                        store,
                        journal,
                    ) {
                        journal.warn("Got invalid header data");
                        return Err(InboundLedgerPacketError::InvalidHeader);
                    }
                    san.inc_useful();
                }

                if !self.planner_state.have_state
                    && packet.nodes.len() > 1
                    && !self.take_as_root_node_with_family(
                        &packet.nodes[1].node_data,
                        &mut san,
                        store,
                        fetch_pack,
                        family,
                    )
                {
                    journal.warn("Included AS root invalid");
                }

                if !self.planner_state.have_transactions
                    && packet.nodes.len() > 2
                    && !self.take_tx_root_node_with_family(
                        &packet.nodes[2].node_data,
                        &mut san,
                        store,
                        fetch_pack,
                        family,
                    )
                {
                    journal.warn("Included TX root invalid");
                }

                Ok(san)
            }
            InboundLedgerDataType::TransactionNode | InboundLedgerDataType::StateNode => {
                if packet.nodes.is_empty() {
                    return Err(InboundLedgerPacketError::EmptyNodes);
                }
                if packet.nodes.iter().any(|node| node.node_id.is_none()) {
                    journal.warn("Got bad node");
                    return Err(InboundLedgerPacketError::MissingNodeId);
                }

                let mut san = SHAMapAddNode::default();
                self.receive_node_packet_with_family_and_config(
                    packet, &mut san, config, store, fetch_pack, family, journal,
                );
                journal.debug(&format!(
                    "Ledger {} node stats: {}",
                    match packet.packet_type {
                        InboundLedgerDataType::TransactionNode => "TX",
                        InboundLedgerDataType::StateNode => "AS",
                        InboundLedgerDataType::Base => "BASE",
                    },
                    san.get()
                ));
                Ok(san)
            }
        }
    }

    pub fn try_db_with_family_and_config<CLOCK, S, C, F, MR, NS, DB, FP, J>(
        &mut self,
        journal: &J,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        if !self.planner_state.have_header {
            let from_fetch_pack = if let Some(data) = store.fetch_ledger_header(self.hash, self.seq)
            {
                journal.trace("Ledger header found in local store");
                self.try_load_header_from_bytes(data, config, store, false, journal);
                false
            } else if let Some(data) = fetch_pack.get_fetch_pack(*self.hash.as_uint256()) {
                journal.trace("Ledger header found in fetch pack");
                self.try_load_header_from_bytes(data, config, store, true, journal);
                true
            } else {
                return;
            };

            if self.failed || !self.planner_state.have_header {
                return;
            }

            if from_fetch_pack {
                debug_assert!(self.ledger.is_some());
            }
        }

        if !self.planner_state.have_transactions {
            let Some(ledger) = self.ledger.as_mut() else {
                return;
            };

            let tx_hash = ledger.header().tx_hash;
            if tx_hash.is_zero() {
                journal.trace("No TXNs to fetch");
                self.planner_state.have_transactions = true;
            } else {
                let mut filter =
                    TransactionStateSF::new(InboundLedgerSyncStore(&mut *store), &mut *fetch_pack);
                let map_hash_before = ledger.tx_map_mut().hash();
                let fetched = {
                    let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
                    ledger
                        .tx_map_mut()
                        .fetch_root_with_family(tx_hash, &mut filter_ref, family)
                };
                let map_hash_after = ledger.tx_map_mut().hash();
                journal.trace(&format!(
                    "TX root check tx_hash={} fetched={} map_hash_before={} map_hash_after={}",
                    tx_hash, fetched, map_hash_before, map_hash_after
                ));
                if fetched {
                    let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
                    let needed = ledger.needed_tx_hashes_with_family(1, &mut filter_ref, family);
                    if needed.is_empty() {
                        journal.trace("Had full txn map locally");
                        self.planner_state.have_transactions = true;
                    } else if let Some(first_missing) = needed.first() {
                        journal.warn(&format!(
                            "TX map still missing hashes after local fetch needed_count={} first_missing={}",
                            needed.len(),
                            first_missing
                        ));
                    }
                }
            }
        }

        if !self.planner_state.have_state {
            let Some(ledger) = self.ledger.as_mut() else {
                return;
            };

            let account_hash = ledger.header().account_hash;
            if account_hash.is_zero() {
                journal.fatal("We are acquiring a ledger with a zero account hash");
                self.failed = true;
                return;
            }

            let mut filter =
                AccountStateSF::new(InboundLedgerSyncStore(&mut *store), &mut *fetch_pack);
            let map_hash_before = ledger.state_map_mut().hash();
            let fetched = {
                let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
                ledger
                    .state_map_mut()
                    .fetch_root_with_family(account_hash, &mut filter_ref, family)
            };
            let map_hash_after = ledger.state_map_mut().hash();
            journal.trace(&format!(
                "AS root check account_hash={} fetched={} map_hash_before={} map_hash_after={}",
                account_hash, fetched, map_hash_before, map_hash_after
            ));
            if fetched {
                let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
                let needed = ledger.needed_state_hashes_with_family(1, &mut filter_ref, family);
                if needed.is_empty() {
                    journal.trace("Had full AS map locally");
                    self.planner_state.have_state = true;
                } else if let Some(first_missing) = needed.first() {
                    journal.debug(&format!(
                        "AS map still missing hashes after local fetch needed_count={} first_missing={}",
                        needed.len(),
                        first_missing
                    ));
                }
            }
        }

        if self.planner_state.have_transactions && self.planner_state.have_state {
            journal.debug("Had everything locally");
            self.complete = true;
            self.finish_if_done_with_family_and_config(journal, config, family);
        }
    }

    fn try_load_header_from_bytes<DB, J>(
        &mut self,
        data: Blob,
        config: &LedgerConfig,
        store: &mut DB,
        from_fetch_pack: bool,
        journal: &J,
    ) where
        DB: InboundLedgerStore,
        J: InboundLedgerJournal,
    {
        let Ok(mut header) = deserialize_prefixed_ledger_header(&data, false) else {
            journal.warn(&format!(
                "hash {} seq {} cannot be a ledger",
                self.hash, self.seq
            ));
            self.ledger = None;
            self.failed = true;
            return;
        };
        header.hash = calculate_ledger_hash(&header);

        if header.hash != self.hash || (self.seq != 0 && self.seq != header.seq) {
            journal.warn(&format!(
                "hash {} seq {} cannot be a ledger",
                self.hash, self.seq
            ));
            self.ledger = None;
            self.failed = true;
            return;
        }

        if self.seq == 0 {
            self.seq = header.seq;
        }

        let ledger = build_loaded_ledger(header, config, self.seq);

        if from_fetch_pack {
            store.store_ledger_header(data, self.hash, ledger.header().seq);
        }

        self.planner_state.have_transactions = false;
        self.planner_state.have_state = false;
        self.ledger = Some(ledger);
        self.planner_state.have_header = true;
        if let Some(ledger) = self.ledger.as_mut() {
            ledger.state_map_mut().set_synching();
            ledger.tx_map_mut().set_synching();
        }
    }

    fn receive_tx_nodes<CLOCK, S, C, F, MR, NS, DB, FP, J>(
        &mut self,
        packet: &InboundLedgerPacket,
        san: &mut SHAMapAddNode,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        journal: &J,
    ) where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        let Some(ledger) = self.ledger.as_mut() else {
            san.inc_invalid();
            return;
        };

        let tx_hash = ledger.header().tx_hash;
        let mut filter =
            TransactionStateSF::new(InboundLedgerSyncStore(&mut *store), &mut *fetch_pack);
        for node in &packet.nodes {
            let Some(node_id_bytes) = node.node_id.as_deref() else {
                san.inc_invalid();
                return;
            };
            if node.node_data.is_empty() {
                journal.warn("Received bad node data");
                san.inc_invalid();
                return;
            }
            let Some(node_id) = shamap::node_id::deserialize_shamap_node_id(node_id_bytes) else {
                journal.warn("Received bad node data");
                san.inc_invalid();
                return;
            };

            let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
            let added = if node_id.is_root() {
                ledger.tx_map_mut().add_root_node_with_family(
                    tx_hash,
                    &node.node_data,
                    &mut filter_ref,
                    family,
                )
            } else {
                ledger.tx_map_mut().add_known_node_with_family(
                    node_id,
                    &node.node_data,
                    &mut filter_ref,
                    family,
                )
            };
            *san += added;
            if !san.is_good() {
                journal.warn("Received bad node data");
                return;
            }
        }

        if !ledger.tx_map().is_synching() {
            self.planner_state.have_transactions = true;
        }

        if self.planner_state.have_transactions && self.planner_state.have_state {
            self.complete = true;
        }

        self.finish_if_done_with_family_and_config(journal, config, family);
    }

    fn receive_state_nodes<CLOCK, S, C, F, MR, NS, DB, FP, J>(
        &mut self,
        packet: &InboundLedgerPacket,
        san: &mut SHAMapAddNode,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        journal: &J,
    ) where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        let Some(ledger) = self.ledger.as_mut() else {
            san.inc_invalid();
            return;
        };

        let account_hash = ledger.header().account_hash;
        let mut filter = AccountStateSF::new(InboundLedgerSyncStore(&mut *store), &mut *fetch_pack);
        let mut node_count = 0u32;
        let mut walk_ns = 0u64;
        let mut fast_count = 0u32; // <1us
        let mut med_count = 0u32; // 1-100us
        let mut slow_count = 0u32; // >100us
        let mut max_ns = 0u64;
        let loop_start = std::time::Instant::now();
        for node in &packet.nodes {
            let Some(node_id_bytes) = node.node_id.as_deref() else {
                san.inc_invalid();
                return;
            };
            if node.node_data.is_empty() {
                journal.warn("Received bad node data");
                san.inc_invalid();
                return;
            }
            let Some(node_id) = shamap::node_id::deserialize_shamap_node_id(node_id_bytes) else {
                journal.warn("Received bad node data");
                san.inc_invalid();
                return;
            };

            let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
            let t = std::time::Instant::now();
            let added = if node_id.is_root() {
                ledger.state_map_mut().add_root_node_with_family(
                    account_hash,
                    &node.node_data,
                    &mut filter_ref,
                    family,
                )
            } else {
                ledger.state_map_mut().add_known_node_with_family(
                    node_id,
                    &node.node_data,
                    &mut filter_ref,
                    family,
                )
            };
            walk_ns += t.elapsed().as_nanos() as u64;
            let elapsed_ns = t.elapsed().as_nanos() as u64;
            if elapsed_ns < 1_000 {
                fast_count += 1;
            } else if elapsed_ns < 100_000 {
                med_count += 1;
            } else {
                slow_count += 1;
            }
            if elapsed_ns > max_ns {
                max_ns = elapsed_ns;
            }
            node_count += 1;
            *san += added;
            if !san.is_good() {
                journal.warn("Received bad node data");
                return;
            }
        }

        if !ledger.state_map().is_synching() {
            self.planner_state.have_state = true;
            if self.planner_state.have_transactions {
                self.complete = true;
            }
        }
        let total_ms = loop_start.elapsed().as_millis();
        if total_ms > 100 && node_count > 0 {
            let avg_us = walk_ns / 1000 / node_count as u64;
            tracing::debug!(target: "ledger",
                "[acq][node-profile] nodes={} total={}ms avg={}us fast={} med={} slow={} max={}us",
                node_count,
                total_ms,
                avg_us,
                fast_count,
                med_count,
                slow_count,
                max_ns / 1000
            );
        }

        self.finish_if_done_with_family_and_config(journal, config, family);
    }

    fn finish_if_done<J>(&mut self, journal: &J)
    where
        J: InboundLedgerJournal,
    {
        if self.signaled || !self.is_done() {
            return;
        }

        assert!(
            self.complete || self.failed,
            "xrpl::InboundLedger::done : complete or failed"
        );

        if self.complete && !self.failed {
            let ledger = self
                .ledger
                .as_mut()
                .expect("completed inbound ledger must hold a ledger");
            if ledger.has_node_fetcher()
                && ledger.header().seq >= XRP_LEDGER_EARLIEST_FEES
                && !matches!(ledger.read(protocol::fee_settings_keylet()), Ok(Some(_)))
            {
                journal.fatal(
                    "completed inbound ledger missing fee settings entry; acquired state is not usable as a compatibility parent",
                );
                self.failed = true;
                self.complete = false;
                self.signaled = true;
                return;
            }
        }
        self.signaled = true;
    }

    pub fn finish_if_done_with_family_and_config<CLOCK, S, C, F, MR, NS, J>(
        &mut self,
        journal: &J,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: InboundLedgerJournal,
    {
        if self.signaled || !self.is_done() {
            return;
        }

        assert!(
            self.complete || self.failed,
            "xrpl::InboundLedger::done : complete or failed"
        );

        if self.complete && !self.failed {
            let ledger = self
                .ledger
                .as_mut()
                .expect("completed inbound ledger must hold a ledger");
            match ledger.setup_from_state_map_with_config_and_family(config, family) {
                Ok(true) => {}
                Ok(false) if ledger.header().seq < XRP_LEDGER_EARLIEST_FEES => {}
                Ok(false) => {
                    journal.fatal(
                        "completed inbound ledger missing fee settings entry; acquired state is not usable as a compatibility parent",
                    );
                    self.failed = true;
                    self.complete = false;
                    self.signaled = true;
                    return;
                }
                Err(error) => {
                    journal.fatal(&format!(
                        "completed inbound ledger setup failed; acquired state is not usable as a compatibility parent: {error:?}"
                    ));
                    self.failed = true;
                    self.complete = false;
                    self.signaled = true;
                    return;
                }
            }
            if ledger.header().seq >= XRP_LEDGER_EARLIEST_FEES
                && !matches!(
                    ledger.read_with_family(protocol::fee_settings_keylet(), family),
                    Ok(Some(_))
                )
            {
                journal.fatal(
                    "completed inbound ledger missing fee settings entry; acquired state is not usable as a compatibility parent",
                );
                self.failed = true;
                self.complete = false;
                self.signaled = true;
                return;
            }
        }
        self.signaled = true;
    }

    /// Sends requests to peers based on current acquisition state.
    pub fn trigger_with_family<CLOCK, S, C, F, MR, NS, DB, FP, J>(
        &mut self,
        reason: InboundLedgerRequestTrigger,
        journal: &J,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        send_fn: &mut dyn FnMut(ProtocolMessage),
    ) where
        CLOCK: basics::tagged_cache::CacheClock,
        S: std::hash::BuildHasher + Clone,
        C: shamap::family::FullBelowCache,
        F: shamap::family::SHAMapNodeFetcher,
        MR: shamap::family::MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        if self.is_done() {
            return;
        }

        // Try local DB if header missing
        if !self.planner_state.have_header {
            self.try_db_with_family_and_config(journal, config, store, fetch_pack, family);
            if self.failed || self.is_done() {
                return;
            }
        }

        // By-hash fallback after threshold timeouts
        if self.timeouts > 0
            && !self.progress
            && self.by_hash
            && self.timeouts > INBOUND_LEDGER_BECOME_AGGRESSIVE
        {
            let mut state_filter = None;
            let mut tx_filter = None;
            if let Some(request) =
                self.make_needed_by_hash_request(&mut state_filter, &mut tx_filter, family)
            {
                send_fn(request);
                self.by_hash = false;
            }
        }

        // Header
        if !self.planner_state.have_header && !self.failed {
            let request = self.make_header_request();
            send_fn(request);
            return;
        }

        let query_depth = match reason {
            InboundLedgerRequestTrigger::Timeout
            | InboundLedgerRequestTrigger::Added
            | InboundLedgerRequestTrigger::Blind => 0,
            InboundLedgerRequestTrigger::Reply => 1,
            // reference: if (peer && peer->isHighLatency()) tmGL.set_querydepth(2);
            InboundLedgerRequestTrigger::ReplyHighLatency => 2,
        };
        let query_type = if self.timeouts > 0 {
            Some(TM_QUERY_INDIRECT)
        } else {
            None
        };

        // Account state
        if !self.planner_state.have_state && !self.failed {
            let ledger = match self.ledger.as_mut() {
                Some(l) => l,
                None => return,
            };
            let account_hash = ledger.header().account_hash;
            if account_hash.is_zero() {
                self.failed = true;
                return;
            }

            let mut filter =
                AccountStateSF::new(InboundLedgerSyncStore(&mut *store), &mut *fetch_pack);
            let map_hash = ledger.state_map_mut().hash();
            if map_hash.is_zero() {
                // Request root node
                let node_ids = [shamap::node_id::SHAMapNodeId::default()];
                log_acq_request_nodes(
                    self.seq,
                    "state",
                    account_hash,
                    1,
                    1,
                    1,
                    reason,
                    self.recent_nodes.len(),
                    query_depth,
                    query_type,
                    &node_ids,
                );
                let request = make_get_ledger_with_node_ids(
                    self.hash,
                    self.seq,
                    TM_GET_LEDGER_AS_NODE,
                    &node_ids,
                    query_depth,
                    query_type,
                );
                send_fn(request);
            } else {
                let missing_limit = if self.planner_state.have_state {
                    MISSING_NODES_FIND
                } else {
                    MISSING_NODES_FIND_COLD_START
                };
                let mut filter_ref: Option<&mut dyn shamap::fetch::SHAMapSyncFilter> =
                    Some(&mut filter);
                let (missing, scan_stats) = if full_sync_debug_enabled() {
                    ledger
                        .state_map_mut()
                        .get_missing_nodes_with_family_diagnostics(
                            missing_limit,
                            &mut filter_ref,
                            family,
                            &mut next_missing_scan_first_child,
                        )
                } else {
                    (
                        ledger.state_map_mut().get_missing_nodes_with_family(
                            missing_limit,
                            &mut filter_ref,
                            family,
                            &mut next_missing_scan_first_child,
                        ),
                        Default::default(),
                    )
                };
                if full_sync_debug_enabled() {
                    tracing::debug!(target: "ledger",
                        "[full_debug][acq_missing] seq={} map=state root={} missing={} have_state={} valid={} reason={:?}",
                        self.seq,
                        map_hash,
                        missing.len(),
                        self.planner_state.have_state,
                        ledger.state_map().is_valid(),
                        reason
                    );
                    for (index, (node_id, node_hash)) in missing.iter().take(4).enumerate() {
                        tracing::debug!(target: "ledger",
                            "[full_debug][acq_missing_sample] seq={} map=state index={} node_id={} hash={}",
                            self.seq, index, node_id, node_hash
                        );
                    }
                    tracing::debug!(target: "ledger",
                        "[full_debug][acq_scan_stats] seq={} map=state branches={} dup_missing={} full_below_hits={} loaded={} leaves={} inners={} full_below_inners={} pending={} pending_hits={} pending_misses={} missing_recorded={} full_below_marked={} resumes={}",
                        self.seq,
                        scan_stats.branches_seen,
                        scan_stats.duplicate_missing_hashes,
                        scan_stats.full_below_hits,
                        scan_stats.loaded_or_cached_children,
                        scan_stats.leaf_children,
                        scan_stats.inner_children,
                        scan_stats.full_below_inner_children,
                        scan_stats.pending_reads,
                        scan_stats.completed_pending_reads,
                        scan_stats.completed_pending_misses,
                        scan_stats.missing_recorded,
                        scan_stats.full_below_marked,
                        scan_stats.deferred_resumes
                    );
                }
                if missing.is_empty() {
                    if ledger.state_map().is_valid() {
                        self.planner_state.have_state = true;
                        if full_sync_debug_enabled() {
                            tracing::debug!(target: "ledger",
                                "[full_debug][acq_have_map] seq={} map=state root={} valid=true",
                                self.seq, map_hash
                            );
                        }
                    }
                } else {
                    let limit = match reason {
                        InboundLedgerRequestTrigger::Reply
                        | InboundLedgerRequestTrigger::ReplyHighLatency => REQ_NODES_REPLY,
                        _ => REQ_NODES,
                    };
                    let mut fresh: Vec<_> = missing
                        .iter()
                        .filter(|(_, h)| !self.recent_nodes.contains(h))
                        .collect();
                    if fresh.is_empty() {
                        // All duplicates — only send on timeout
                        if reason != InboundLedgerRequestTrigger::Timeout {
                            // skip
                        } else {
                            fresh = missing.iter().collect();
                        }
                    }
                    if full_sync_debug_enabled() {
                        tracing::debug!(target: "ledger",
                            "[full_debug][acq_request_nodes] seq={} map=state root={} missing={} fresh={} limit={} reason={:?} recent_nodes={}",
                            self.seq,
                            map_hash,
                            missing.len(),
                            fresh.len(),
                            limit,
                            reason,
                            self.recent_nodes.len()
                        );
                    }
                    if fresh.is_empty() { /* nothing to send */
                    } else {
                        let node_ids: Vec<_> =
                            fresh.iter().take(limit).map(|(id, _)| *id).collect();
                        log_acq_request_nodes(
                            self.seq,
                            "state",
                            map_hash,
                            missing.len(),
                            fresh.len(),
                            limit,
                            reason,
                            self.recent_nodes.len(),
                            query_depth,
                            query_type,
                            &node_ids,
                        );
                        for (_, h) in fresh.iter().take(limit) {
                            self.recent_nodes.insert(*h);
                        }
                        let request = make_get_ledger_with_node_ids(
                            self.hash,
                            self.seq,
                            TM_GET_LEDGER_AS_NODE,
                            &node_ids,
                            query_depth,
                            query_type,
                        );
                        send_fn(request);
                    }
                }
            }
        }

        // Transactions
        if !self.planner_state.have_transactions && !self.failed {
            let ledger = match self.ledger.as_mut() {
                Some(l) => l,
                None => return,
            };
            let tx_hash = ledger.header().tx_hash;
            if tx_hash.is_zero() {
                self.planner_state.have_transactions = true;
            } else {
                let mut filter =
                    TransactionStateSF::new(InboundLedgerSyncStore(&mut *store), &mut *fetch_pack);
                let map_hash = ledger.tx_map_mut().hash();
                if map_hash.is_zero() {
                    let node_ids = [shamap::node_id::SHAMapNodeId::default()];
                    log_acq_request_nodes(
                        self.seq,
                        "tx",
                        tx_hash,
                        1,
                        1,
                        1,
                        reason,
                        self.recent_nodes.len(),
                        query_depth,
                        query_type,
                        &node_ids,
                    );
                    let request = make_get_ledger_with_node_ids(
                        self.hash,
                        self.seq,
                        TM_GET_LEDGER_TX_NODE,
                        &node_ids,
                        query_depth,
                        query_type,
                    );
                    send_fn(request);
                } else {
                    let mut filter_ref: Option<&mut dyn shamap::fetch::SHAMapSyncFilter> =
                        Some(&mut filter);
                    let missing = ledger.tx_map_mut().get_missing_nodes_with_family(
                        MISSING_NODES_FIND,
                        &mut filter_ref,
                        family,
                        &mut next_missing_scan_first_child,
                    );
                    if full_sync_debug_enabled() {
                        tracing::debug!(target: "ledger",
                            "[full_debug][acq_missing] seq={} map=tx root={} missing={} have_tx={} valid={} reason={:?}",
                            self.seq,
                            map_hash,
                            missing.len(),
                            self.planner_state.have_transactions,
                            ledger.tx_map().is_valid(),
                            reason
                        );
                        for (index, (node_id, node_hash)) in missing.iter().take(4).enumerate() {
                            tracing::debug!(target: "ledger",
                                "[full_debug][acq_missing_sample] seq={} map=tx index={} node_id={} hash={}",
                                self.seq, index, node_id, node_hash
                            );
                        }
                    }
                    if missing.is_empty() {
                        if ledger.tx_map().is_valid() {
                            self.planner_state.have_transactions = true;
                            if full_sync_debug_enabled() {
                                tracing::debug!(target: "ledger",
                                    "[full_debug][acq_have_map] seq={} map=tx root={} valid=true",
                                    self.seq, map_hash
                                );
                            }
                        }
                    } else {
                        let limit = match reason {
                            InboundLedgerRequestTrigger::Reply
                            | InboundLedgerRequestTrigger::ReplyHighLatency => REQ_NODES_REPLY,
                            _ => REQ_NODES,
                        };
                        let mut fresh: Vec<_> = missing
                            .iter()
                            .filter(|(_, h)| !self.recent_nodes.contains(h))
                            .collect();
                        if fresh.is_empty() {
                            if reason != InboundLedgerRequestTrigger::Timeout {
                                // skip
                            } else {
                                fresh = missing.iter().collect();
                            }
                        }
                        if full_sync_debug_enabled() {
                            tracing::debug!(target: "ledger",
                                "[full_debug][acq_request_nodes] seq={} map=tx root={} missing={} fresh={} limit={} reason={:?} recent_nodes={}",
                                self.seq,
                                map_hash,
                                missing.len(),
                                fresh.len(),
                                limit,
                                reason,
                                self.recent_nodes.len()
                            );
                        }
                        if fresh.is_empty() { /* nothing to send */
                        } else {
                            let node_ids: Vec<_> =
                                fresh.iter().take(limit).map(|(id, _)| *id).collect();
                            log_acq_request_nodes(
                                self.seq,
                                "tx",
                                map_hash,
                                missing.len(),
                                fresh.len(),
                                limit,
                                reason,
                                self.recent_nodes.len(),
                                query_depth,
                                query_type,
                                &node_ids,
                            );
                            for (_, h) in fresh.iter().take(limit) {
                                self.recent_nodes.insert(*h);
                            }
                            let request = make_get_ledger_with_node_ids(
                                self.hash,
                                self.seq,
                                TM_GET_LEDGER_TX_NODE,
                                &node_ids,
                                query_depth,
                                query_type,
                            );
                            send_fn(request);
                        }
                    }
                }
            }
        }

        if self.planner_state.have_header
            && self.planner_state.have_state
            && self.planner_state.have_transactions
        {
            self.complete = true;
            let duration_ms = 0u64; // placeholder — real elapsed tracked externally
            tracing::info!(target: "ledger", seq = self.seq, duration_ms, "Ledger acquisition complete");
            tracing::info!(target: "ledger", "[acq][trigger_complete] seq={} set complete=true", self.seq);
            // Don't call finish_if_done here — it traverses the state map
            // (setup_from_state_map) which may block on missing nodes during
            // catchup. The worker loop detects complete=true via is_done()
            // and sends the ledger to the main loop for acceptance.
        }
    }

    /// Timer callback — called every 3 seconds. Returns `true` if failed.
    pub fn on_timer_with_family<CLOCK, S, C, F, MR, NS, DB, FP, J>(
        &mut self,
        journal: &J,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        send_fn: &mut dyn FnMut(ProtocolMessage),
    ) -> bool
    where
        CLOCK: basics::tagged_cache::CacheClock,
        S: std::hash::BuildHasher + Clone,
        C: shamap::family::FullBelowCache,
        F: shamap::family::SHAMapNodeFetcher,
        MR: shamap::family::MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        self.recent_nodes.clear();

        if self.is_done() {
            return self.failed;
        }

        let was_progress = self.progress;
        self.progress = false;

        if !was_progress {
            self.timeouts = self.timeouts.saturating_add(1);
            tracing::debug!(target: "ledger", seq = self.seq, peer_id = 0u64, "Ledger data request timeout");

            // Fail after max timeouts
            if self.timeouts > inbound_ledger_timeout_retries_max() {
                journal.warn(&format!(
                    "{} timeouts for ledger seq={} hash={}",
                    self.timeouts, self.seq, self.hash
                ));
                tracing::debug!(target: "ledger", seq = self.seq, "Ledger acquisition failed — retrying");
                self.failed = true;
                return true;
            }

            // Re-check local DB
            let _ =
                self.check_local_with_family_and_config(journal, config, store, fetch_pack, family);
            if self.is_done() {
                return self.failed;
            }

            self.by_hash = true;

            // Re-trigger
            self.trigger_with_family(
                InboundLedgerRequestTrigger::Timeout,
                journal,
                config,
                store,
                fetch_pack,
                family,
                send_fn,
            );
        }

        self.failed
    }
}

pub fn make_inbound_needed_by_hash_request(
    ledger_hash: SHAMapHash,
    seq: u32,
    needed: &[(InboundLedgerObjectType, Uint256)],
) -> Option<ProtocolMessage> {
    let (first_type, _) = *needed.first()?;
    let request_type = match first_type {
        InboundLedgerObjectType::Ledger => TM_GET_OBJECT_BY_HASH_LEDGER,
        InboundLedgerObjectType::TransactionNode => TM_GET_OBJECT_BY_HASH_TRANSACTION_NODE,
        InboundLedgerObjectType::StateNode => TM_GET_OBJECT_BY_HASH_STATE_NODE,
    };

    let mut objects = Vec::new();
    for &(object_type, hash) in needed {
        if object_type != first_type {
            continue;
        }

        objects.push(overlay::message::wire::TmIndexedObject {
            hash: Some(hash.data().to_vec()),
            index: None,
            data: None,
            node_id: None,
            ledger_seq: (seq != 0).then_some(seq),
        });
    }

    Some(ProtocolMessage::new(ProtocolPayload::GetObjects(
        TmGetObjectByHash {
            r#type: request_type,
            query: true,
            ledger_hash: Some(ledger_hash.as_uint256().data().to_vec()),
            fat: None,
            objects,
        },
    )))
}

pub fn make_inbound_get_ledger_request(
    hash: SHAMapHash,
    seq: u32,
    packet_type: InboundLedgerDataType,
    query_depth: u32,
    trigger: InboundLedgerRequestTrigger,
) -> ProtocolMessage {
    let itype = match packet_type {
        InboundLedgerDataType::Base => TM_GET_LEDGER_BASE,
        InboundLedgerDataType::TransactionNode => TM_GET_LEDGER_TX_NODE,
        InboundLedgerDataType::StateNode => TM_GET_LEDGER_AS_NODE,
    };

    ProtocolMessage::new(ProtocolPayload::GetLedger(TmGetLedger {
        itype,
        ltype: None,
        ledger_hash: Some(hash.as_uint256().data().to_vec()),
        ledger_seq: (seq != 0).then_some(seq),
        node_i_ds: Vec::new(),
        request_cookie: None,
        query_type: (packet_type != InboundLedgerDataType::Base
            && trigger == InboundLedgerRequestTrigger::Blind)
            .then_some(TM_QUERY_INDIRECT),
        query_depth: (packet_type != InboundLedgerDataType::Base).then_some(query_depth),
    }))
}

#[derive(Debug, Default)]
struct InboundLedgerPeerDataCounts {
    counts: BTreeMap<u64, i32>,
    max_count: i32,
}

impl InboundLedgerPeerDataCounts {
    fn update(&mut self, peer_id: Option<u64>, data_count: i32) {
        if data_count <= 0 {
            return;
        }

        let Some(peer_id) = peer_id else {
            return;
        };

        self.max_count = self.max_count.max(data_count);
        self.counts
            .entry(peer_id)
            .and_modify(|count| *count = (*count).max(data_count))
            .or_insert(data_count);
    }

    fn pruned_scores(&self) -> Vec<InboundLedgerPeerScore> {
        let threshold = self.max_count / 2;
        self.counts
            .iter()
            .filter_map(|(&peer_id, &useful_count)| {
                (useful_count >= threshold).then_some(InboundLedgerPeerScore {
                    peer_id,
                    useful_count,
                })
            })
            .collect()
    }
}

fn sample_peer_ids(scores: &[InboundLedgerPeerScore], max: usize) -> Vec<u64> {
    sample_peer_ids_with(scores, max, &mut |len| rand_int_to(len - 1))
}

fn sample_peer_ids_with<PICK>(
    scores: &[InboundLedgerPeerScore],
    max: usize,
    pick_index: &mut PICK,
) -> Vec<u64>
where
    PICK: FnMut(usize) -> usize,
{
    if scores.len() <= max {
        return scores.iter().map(|score| score.peer_id).collect();
    }

    let mut remaining: Vec<u64> = scores.iter().map(|score| score.peer_id).collect();
    let mut sampled = Vec::with_capacity(max);
    while sampled.len() < max && !remaining.is_empty() {
        let index = pick_index(remaining.len());
        sampled.push(remaining.swap_remove(index));
    }
    sampled
}

fn build_loaded_ledger(
    header: crate::LedgerHeader,
    config: &LedgerConfig,
    ledger_seq: u32,
) -> Ledger {
    let mut ledger = Ledger::new(header, true);
    ledger.set_rules(Rules::new(config.features.iter()));
    ledger.set_fees(Fees::default());
    ledger.state_map_mut().set_ledger_seq(ledger_seq);
    ledger.tx_map_mut().set_ledger_seq(ledger_seq);
    ledger
}

pub fn needed_hashes_with_family<CLOCK, S, C, F, MR, NS>(
    root: SHAMapHash,
    map: &mut SyncTree,
    max: i32,
    filter: &mut Option<&mut dyn SHAMapSyncFilter>,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
) -> Vec<Uint256>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    C: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    let mut next_first_child = || next_missing_scan_first_child();
    needed_hashes_with_family_and_first_child(root, map, max, filter, family, &mut next_first_child)
}

pub fn needed_hashes_with_family_and_first_child<CLOCK, S, C, F, R, MR, NS>(
    root: SHAMapHash,
    map: &mut SyncTree,
    max: i32,
    filter: &mut Option<&mut dyn SHAMapSyncFilter>,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    next_first_child: &mut R,
) -> Vec<Uint256>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    C: FullBelowCache,
    F: SHAMapNodeFetcher,
    R: FnMut() -> u8,
    MR: MissingNodeReporter,
{
    let mut needed = Vec::new();

    if root.is_zero() {
        return needed;
    }

    if map.hash().is_zero() {
        needed.push(*root.as_uint256());
        return needed;
    }

    let missing = map.get_missing_nodes_with_family(max, filter, family, next_first_child);
    needed.reserve(missing.len());
    for (_, hash) in missing {
        needed.push(hash);
    }
    needed
}

pub fn get_needed_hashes_with_family<CLOCK, S, C, F, MR, NS>(
    ledger_hash: SHAMapHash,
    ledger: Option<&mut Ledger>,
    planner_state: InboundLedgerPlannerState,
    state_filter: &mut Option<&mut dyn SHAMapSyncFilter>,
    tx_filter: &mut Option<&mut dyn SHAMapSyncFilter>,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
) -> Vec<(InboundLedgerObjectType, Uint256)>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    C: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    let mut needed = Vec::new();

    if !planner_state.have_header {
        needed.push((InboundLedgerObjectType::Ledger, *ledger_hash.as_uint256()));
        return needed;
    }

    let ledger =
        ledger.expect("get_needed_hashes_with_family requires a ledger once header exists");

    if !planner_state.have_state {
        for hash in ledger.needed_state_hashes_with_family(
            INBOUND_LEDGER_MAX_NEEDED_STATE_HASHES,
            state_filter,
            family,
        ) {
            needed.push((InboundLedgerObjectType::StateNode, hash));
        }
    }

    if !planner_state.have_transactions {
        for hash in ledger.needed_tx_hashes_with_family(
            INBOUND_LEDGER_MAX_NEEDED_TX_HASHES,
            tx_filter,
            family,
        ) {
            needed.push((InboundLedgerObjectType::TransactionNode, hash));
        }
    }

    needed
}

#[cfg(test)]
mod tests;
