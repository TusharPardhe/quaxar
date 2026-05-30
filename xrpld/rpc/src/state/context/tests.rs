use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use ledger::InboundLedgerPlannerState;
use overlay::{Message, Peer, ProtocolFeature, ProtocolMessage, ProtocolPayload, TmGetLedger};
use protocol::{JsonValue, PublicKey};
use resource::Charge;
use shamap::node_id::SHAMapNodeId;
use shamap::storage::NodeObjectType as SHAMapNodeObjectType;

use super::{
    InboundLedgerStore, LEDGER_REQUEST_AGGRESSIVE_BY_HASH_AFTER,
    LEDGER_REQUEST_BECOME_AGGRESSIVE_TIMEOUTS, LEDGER_REQUEST_REPLY_QUERY_DEPTH,
    LEDGER_REQUEST_RETRY_INTERVAL, LEDGER_REQUEST_TIMEOUT_MODE_STALL,
    LEDGER_REQUEST_TIMEOUT_NO_PROGRESS_CYCLES, LedgerRequestTarget, LedgerSyncFilterStore,
    RpcSyncFilterStore, TM_QUERY_INDIRECT, can_send_primary_request, extend_acquisition_peer_set,
    get_ledger_query_type, select_missing_node_ids_for_request, send_get_ledger_to_peers,
    send_reply_mode_requests_to_peers, send_to_peers, set_get_ledger_query_depth,
    should_arm_timeout_local_check, should_run_local_check, target_made_progress,
    use_aggressive_by_hash, use_aggressive_by_hash_timeout, use_blind_request_trigger,
    use_reply_trigger, use_timeout_mode,
};

#[derive(Default)]
struct RecordingInboundStore {
    shamap_writes: Vec<(SHAMapNodeObjectType, Vec<u8>, Uint256, u32)>,
}

impl InboundLedgerStore for RecordingInboundStore {
    fn fetch_ledger_header(&mut self, _hash: SHAMapHash, _ledger_seq: u32) -> Option<Vec<u8>> {
        None
    }

    fn store_ledger_header(&mut self, _data: Vec<u8>, _hash: SHAMapHash, _ledger_seq: u32) {}

    fn store_shamap_node(
        &mut self,
        object_type: SHAMapNodeObjectType,
        data: Vec<u8>,
        hash: Uint256,
        ledger_seq: u32,
    ) {
        self.shamap_writes
            .push((object_type, data, hash, ledger_seq));
    }
}

#[derive(Debug)]
struct RecordingPeer {
    id: u32,
    high_latency: bool,
    score: i32,
    ledgers: Mutex<HashSet<(Uint256, u32)>>,
    sends: Mutex<usize>,
    sent_messages: Mutex<Vec<ProtocolMessage>>,
}

impl RecordingPeer {
    fn new(id: u32) -> Self {
        Self::new_with_latency(id, false)
    }

    fn new_with_latency(id: u32, high_latency: bool) -> Self {
        Self {
            id,
            high_latency,
            score: 0,
            ledgers: Mutex::new(HashSet::new()),
            sends: Mutex::new(0),
            sent_messages: Mutex::new(Vec::new()),
        }
    }

    fn new_with_score(id: u32, score: i32) -> Self {
        Self {
            id,
            high_latency: false,
            score,
            ledgers: Mutex::new(HashSet::new()),
            sends: Mutex::new(0),
            sent_messages: Mutex::new(Vec::new()),
        }
    }

    fn record_ledger(&self, hash: Uint256, sequence: u32) {
        self.ledgers
            .lock()
            .expect("recording peer ledgers mutex must not be poisoned")
            .insert((hash, sequence));
    }

    fn send_count(&self) -> usize {
        *self
            .sends
            .lock()
            .expect("recording peer send mutex must not be poisoned")
    }

    fn sent_query_depths(&self) -> Vec<Option<u32>> {
        self.sent_messages
            .lock()
            .expect("recording peer sent-messages mutex must not be poisoned")
            .iter()
            .map(|message| match &message.payload {
                ProtocolPayload::GetLedger(body) => body.query_depth,
                _ => None,
            })
            .collect()
    }
}

impl Peer for RecordingPeer {
    fn send(&self, message: Message) {
        *self
            .sends
            .lock()
            .expect("recording peer send mutex must not be poisoned") += 1;
        self.sent_messages
            .lock()
            .expect("recording peer sent-messages mutex must not be poisoned")
            .push(message.protocol().clone());
    }

    fn remote_address(&self) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.id as u16)
    }

    fn send_tx_queue(&self) {}

    fn add_tx_queue(&self, _hash: Uint256) {}

    fn remove_tx_queue(&self, _hash: Uint256) {}

    fn charge(&self, _fee: Charge, _context: String) {}

    fn id(&self) -> u32 {
        self.id
    }

    fn cluster(&self) -> bool {
        false
    }

    fn is_high_latency(&self) -> bool {
        self.high_latency
    }

    fn score(&self, _clustered: bool) -> i32 {
        self.score
    }

    fn node_public(&self) -> PublicKey {
        PublicKey::from_bytes([0x02; 33])
    }

    fn json(&self) -> JsonValue {
        JsonValue::Null
    }

    fn supports_feature(&self, _feature: ProtocolFeature) -> bool {
        false
    }

    fn publisher_list_sequence(&self, _publisher: PublicKey) -> Option<usize> {
        None
    }

    fn set_publisher_list_sequence(&self, _publisher: PublicKey, _sequence: usize) {}

    fn fingerprint(&self) -> String {
        format!("peer-{}", self.id)
    }

    fn closed_ledger_hash(&self) -> Uint256 {
        Uint256::zero()
    }

    fn previous_ledger_hash(&self) -> Uint256 {
        Uint256::zero()
    }

    fn has_ledger(&self, _hash: Uint256, _sequence: u32) -> bool {
        self.ledgers
            .lock()
            .expect("recording peer ledgers mutex must not be poisoned")
            .contains(&(_hash, _sequence))
    }

    fn ledger_range(&self) -> (u32, u32) {
        (0, 0)
    }

    fn has_tx_set(&self, _hash: Uint256) -> bool {
        false
    }

    fn cycle_status(&self) {}

    fn has_range(&self, _min_sequence: u32, _max_sequence: u32) -> bool {
        false
    }

    fn compression_enabled(&self) -> bool {
        false
    }

    fn tx_reduce_relay_enabled(&self) -> bool {
        false
    }

    fn features(&self) -> HashSet<ProtocolFeature> {
        HashSet::new()
    }
}

fn sample_get_ledger_message() -> ProtocolMessage {
    ProtocolMessage::new(ProtocolPayload::GetLedger(TmGetLedger {
        itype: 0,
        ltype: None,
        ledger_hash: None,
        ledger_seq: Some(1),
        node_i_ds: Vec::new(),
        request_cookie: None,
        query_type: None,
        query_depth: None,
    }))
}

#[test]
fn timeout_mode_trips_at_no_progress_threshold() {
    assert!(!use_timeout_mode(0, StdDuration::from_millis(0)));
    assert!(!use_timeout_mode(
        LEDGER_REQUEST_TIMEOUT_NO_PROGRESS_CYCLES.saturating_sub(1),
        StdDuration::from_millis(0)
    ));
    assert!(use_timeout_mode(
        LEDGER_REQUEST_TIMEOUT_NO_PROGRESS_CYCLES,
        StdDuration::from_millis(0)
    ));
    assert!(use_timeout_mode(
        LEDGER_REQUEST_TIMEOUT_NO_PROGRESS_CYCLES + 10,
        StdDuration::from_millis(0)
    ));
}

#[test]
fn timeout_mode_trips_after_stall_interval_without_progress() {
    assert!(!use_timeout_mode(
        0,
        LEDGER_REQUEST_TIMEOUT_MODE_STALL.saturating_sub(StdDuration::from_millis(1))
    ));
    assert!(use_timeout_mode(0, LEDGER_REQUEST_TIMEOUT_MODE_STALL));
}

#[test]
fn aggressive_by_hash_waits_for_sustained_stall() {
    assert!(!use_aggressive_by_hash(
        LEDGER_REQUEST_AGGRESSIVE_BY_HASH_AFTER.saturating_sub(StdDuration::from_millis(1))
    ));
    assert!(use_aggressive_by_hash(
        LEDGER_REQUEST_AGGRESSIVE_BY_HASH_AFTER
    ));
}

#[test]
fn ledger_query_type_stays_none_until_timeout_mode() {
    assert_eq!(get_ledger_query_type(false), None);
    assert_eq!(get_ledger_query_type(true), Some(TM_QUERY_INDIRECT));
}

#[test]
fn timeout_mode_overrides_reply_trigger_after_stall() {
    assert!(use_reply_trigger(true, false));
    assert!(!use_reply_trigger(false, false));
    assert!(!use_reply_trigger(true, true));
}

#[test]
fn blind_request_trigger_only_rearms_when_peer_set_grows_or_timeout_hits() {
    assert!(use_blind_request_trigger(false, false, 1, 0));
    assert!(!use_blind_request_trigger(false, false, 1, 1));
    assert!(!use_blind_request_trigger(false, false, 3, 3));
    assert!(use_blind_request_trigger(false, false, 4, 3));
    assert!(use_blind_request_trigger(true, false, 1, 1));
    assert!(use_blind_request_trigger(false, true, 1, 1));
}

#[test]
fn reply_trigger_bypasses_retry_interval() {
    let now = Instant::now();

    assert!(can_send_primary_request(
        true,
        now,
        LEDGER_REQUEST_RETRY_INTERVAL
    ));
    assert!(!can_send_primary_request(
        false,
        now,
        LEDGER_REQUEST_RETRY_INTERVAL
    ));
}

#[test]
fn acquisition_peer_set_adds_best_remaining_peers() {
    let hash = SHAMapHash::new(Uint256::from_u64(99));
    let first = Arc::new(RecordingPeer::new_with_score(1, 5));
    let second = Arc::new(RecordingPeer::new_with_score(2, 10));
    let third = Arc::new(RecordingPeer::new_with_score(3, 1));
    second.record_ledger(*hash.as_uint256(), 0);
    first.record_ledger(*hash.as_uint256(), 0);
    let peers: Vec<Arc<dyn Peer>> = vec![first.clone(), second.clone(), third.clone()];
    let mut tracked = HashSet::from([2_u64]);

    let added =
        extend_acquisition_peer_set(&peers, LedgerRequestTarget::Hash(hash), 0, &mut tracked, 2);

    assert_eq!(added, 2);
    assert_eq!(tracked, HashSet::from([1_u64, 2_u64, 3_u64]));
}

#[test]
fn aggressive_by_hash_rearms_on_timeout_ticks() {
    assert!(!use_aggressive_by_hash_timeout(0, false, true));
    assert!(!use_aggressive_by_hash_timeout(
        LEDGER_REQUEST_BECOME_AGGRESSIVE_TIMEOUTS,
        true,
        true
    ));
    assert!(use_aggressive_by_hash_timeout(
        LEDGER_REQUEST_BECOME_AGGRESSIVE_TIMEOUTS + 1,
        true,
        true
    ));
    assert!(!use_aggressive_by_hash_timeout(
        LEDGER_REQUEST_BECOME_AGGRESSIVE_TIMEOUTS + 1,
        true,
        false
    ));
}

#[test]
fn target_progress_requires_frontier_change_or_useful_packets() {
    let before = InboundLedgerPlannerState::default();
    let after = InboundLedgerPlannerState::default();
    assert!(!target_made_progress(before, after, 0));
    assert!(target_made_progress(before, after, 1));

    let after_planner_change = InboundLedgerPlannerState {
        have_header: true,
        ..InboundLedgerPlannerState::default()
    };
    assert!(target_made_progress(before, after_planner_change, 0));
}

#[test]
fn local_check_runs_only_when_explicitly_armed() {
    assert!(!should_run_local_check(false));
    assert!(should_run_local_check(true));
}

#[test]
fn timeout_local_check_only_arms_when_window_had_no_progress() {
    assert!(should_arm_timeout_local_check(false));
    assert!(!should_arm_timeout_local_check(true));
}

#[test]
fn rpc_sync_filter_store_delegates_into_inbound_store() {
    let mut store = RecordingInboundStore::default();
    let hash = Uint256::default();

    {
        let mut adapter = RpcSyncFilterStore(&mut store);
        LedgerSyncFilterStore::store_shamap_node(
            &mut adapter,
            SHAMapNodeObjectType::AccountNode,
            vec![1, 2, 3],
            hash,
            42,
        );
    }

    assert_eq!(store.shamap_writes.len(), 1);
    let (kind, data, written_hash, ledger_seq) = &store.shamap_writes[0];
    assert_eq!(*kind, SHAMapNodeObjectType::AccountNode);
    assert_eq!(data, &vec![1, 2, 3]);
    assert_eq!(*written_hash, hash);
    assert_eq!(*ledger_seq, 42);
}

#[test]
fn send_to_peers_sends_to_all_preferred_peers() {
    let first = Arc::new(RecordingPeer::new(1));
    let second = Arc::new(RecordingPeer::new(2));
    let third = Arc::new(RecordingPeer::new(3));
    let peers: Vec<Arc<dyn Peer>> = vec![first.clone(), second.clone(), third.clone()];

    send_to_peers(&peers, &[1, 3], sample_get_ledger_message());

    assert_eq!(first.send_count(), 1);
    assert_eq!(second.send_count(), 0);
    assert_eq!(third.send_count(), 1);
}

#[test]
fn send_to_peers_falls_back_to_broadcast_when_preferred_peer_is_missing() {
    let first = Arc::new(RecordingPeer::new(1));
    let second = Arc::new(RecordingPeer::new(2));
    let peers: Vec<Arc<dyn Peer>> = vec![first.clone(), second.clone()];

    send_to_peers(&peers, &[99], sample_get_ledger_message());

    assert_eq!(first.send_count(), 1);
    assert_eq!(second.send_count(), 1);
}

#[test]
fn send_get_ledger_to_peers_uses_deeper_queries_for_high_latency_reply_peers() {
    let first = Arc::new(RecordingPeer::new(1));
    let second = Arc::new(RecordingPeer::new_with_latency(2, true));
    let third = Arc::new(RecordingPeer::new(3));
    let peers: Vec<Arc<dyn Peer>> = vec![first.clone(), second.clone(), third.clone()];

    let mut message = sample_get_ledger_message();
    assert!(set_get_ledger_query_depth(
        &mut message,
        LEDGER_REQUEST_REPLY_QUERY_DEPTH
    ));

    send_get_ledger_to_peers(&peers, &[1, 2], message, true);

    assert_eq!(first.send_count(), 1);
    assert_eq!(second.send_count(), 1);
    assert_eq!(third.send_count(), 0);
    assert_eq!(first.sent_query_depths(), vec![Some(1)]);
    assert_eq!(second.sent_query_depths(), vec![Some(2)]);
}

#[test]
fn reply_mode_rebuilds_requests_per_preferred_peer() {
    let first = Arc::new(RecordingPeer::new(1));
    let second = Arc::new(RecordingPeer::new_with_latency(2, true));
    let peers: Vec<Arc<dyn Peer>> = vec![first.clone(), second.clone()];
    let mut built_depths = Vec::new();

    let sent = send_reply_mode_requests_to_peers(&peers, &[1, 2], |query_depth| {
        built_depths.push(query_depth);
        let mut message = sample_get_ledger_message();
        assert!(set_get_ledger_query_depth(&mut message, query_depth));
        Some(message)
    });

    assert!(sent);
    assert_eq!(built_depths, vec![1, 2]);
    assert_eq!(first.send_count(), 1);
    assert_eq!(second.send_count(), 1);
    assert_eq!(first.sent_query_depths(), vec![Some(1)]);
    assert_eq!(second.sent_query_depths(), vec![Some(2)]);
}

#[test]
fn select_missing_node_ids_for_request_keeps_recent_hashes_filtered_until_caller_resets_them() {
    let first = SHAMapNodeId::default()
        .get_child_node_id(1)
        .expect("child node id should exist");
    let second = SHAMapNodeId::default()
        .get_child_node_id(2)
        .expect("child node id should exist");
    let first_hash = Uint256::from_array([0x11; 32]);
    let second_hash = Uint256::from_array([0x22; 32]);
    let mut recent = HashSet::from([first_hash]);

    let selected = select_missing_node_ids_for_request(
        vec![(first, first_hash), (second, second_hash)],
        false,
        12,
        &mut recent,
    );

    assert_eq!(selected, vec![second]);
    assert!(recent.contains(&first_hash));
    assert!(recent.contains(&second_hash));

    let filtered = select_missing_node_ids_for_request(
        vec![(first, first_hash), (second, second_hash)],
        false,
        12,
        &mut recent,
    );
    assert!(filtered.is_empty());

    recent.clear();
    let selected_after_reset = select_missing_node_ids_for_request(
        vec![(first, first_hash), (second, second_hash)],
        false,
        12,
        &mut recent,
    );
    assert_eq!(selected_after_reset, vec![first, second]);
}
