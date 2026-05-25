use basics::base_uint::Uint256;
use basics::blob::Blob;
use basics::intrusive_pointer::make_shared_intrusive;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::ManualClock;
use ledger::{
    FetchPackContainer, FetchPackStore, InboundLedgerDataType, InboundLedgerJournal,
    InboundLedgerLocal, InboundLedgerNodeData, InboundLedgerPacket, InboundLedgerRoute,
    InboundLedgerStore, InboundLedgersLocal, LedgerConfig, LedgerHeader, calculate_ledger_hash,
    serialize_prefixed_ledger_header, stash_stale_packet,
};
use protocol::JsonValue;
use shamap::family::{NullFullBelowCache, NullMissingNodeReporter, NullNodeFetcher, SHAMapFamily};
use shamap::item::SHAMapItem;
use shamap::storage::{NodeObjectType, StoredNode};
use shamap::tree_node::{SHAMapNodeType, SHAMapTreeNode};
use shamap::tree_node_cache::TreeNodeCache;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use time::Duration;

fn sample_hash(fill: u8) -> SHAMapHash {
    SHAMapHash::new(Uint256::from_array([fill; 32]))
}

#[derive(Debug, Default)]
struct RecordingFetchPackStore {
    blobs: HashMap<Uint256, Blob>,
}

impl FetchPackStore for RecordingFetchPackStore {
    fn add_fetch_pack(&mut self, hash: Uint256, data: Blob) {
        self.blobs.insert(hash, data);
    }
}

impl FetchPackContainer for RecordingFetchPackStore {
    fn get_fetch_pack(&mut self, hash: Uint256) -> Option<Blob> {
        self.blobs.get(&hash).cloned()
    }
}

#[derive(Debug, Default)]
struct RecordingInboundStore {
    headers: Rc<RefCell<HashMap<Uint256, Blob>>>,
    stored_headers: Rc<RefCell<Vec<(Uint256, u32, Blob)>>>,
    stored_nodes: Rc<RefCell<Vec<StoredNode>>>,
}

impl InboundLedgerStore for RecordingInboundStore {
    fn fetch_ledger_header(&mut self, hash: SHAMapHash, _ledger_seq: u32) -> Option<Blob> {
        self.headers.borrow().get(hash.as_uint256()).cloned()
    }

    fn store_ledger_header(&mut self, data: Blob, hash: SHAMapHash, ledger_seq: u32) {
        self.headers
            .borrow_mut()
            .insert(*hash.as_uint256(), data.clone());
        self.stored_headers
            .borrow_mut()
            .push((*hash.as_uint256(), ledger_seq, data));
    }

    fn store_shamap_node(
        &mut self,
        object_type: NodeObjectType,
        data: Blob,
        hash: Uint256,
        ledger_seq: u32,
    ) {
        self.stored_nodes
            .borrow_mut()
            .push(StoredNode::new(object_type, data, hash, ledger_seq));
    }
}

#[derive(Debug, Default)]
struct RecordingJournal;

impl InboundLedgerJournal for RecordingJournal {
    fn trace(&self, _message: &str) {}

    fn debug(&self, _message: &str) {}

    fn warn(&self, _message: &str) {}

    fn fatal(&self, _message: &str) {}
}

fn family(
    label: &'static str,
) -> SHAMapFamily<
    ManualClock,
    basics::hardened_hash::HardenedHashBuilder,
    NullFullBelowCache,
    NullNodeFetcher,
    NullMissingNodeReporter,
> {
    SHAMapFamily::new(
        Arc::new(TreeNodeCache::new(
            label,
            8,
            Duration::seconds(1),
            ManualClock::new(0),
        )),
        NullFullBelowCache::new(1),
        NullNodeFetcher,
        NullMissingNodeReporter,
    )
}

fn sample_header(seq: u32, account_hash: SHAMapHash, tx_hash: SHAMapHash) -> LedgerHeader {
    LedgerHeader {
        seq,
        drops: 55,
        parent_hash: sample_hash(0x01),
        account_hash,
        tx_hash,
        parent_close_time: 22,
        close_time: 33,
        close_time_resolution: 30,
        close_flags: 0,
        ..LedgerHeader::default()
    }
}

#[test]
fn stale_state_packet_is_reencoded_with_prefix_for_fetch_pack_storage() {
    let leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
        SHAMapNodeType::AccountState,
        SHAMapItem::new(Uint256::from_array([0x41; 32]), vec![1; 12]),
        0,
    ));
    let wire = leaf
        .serialize_for_wire()
        .expect("wire serialization should succeed");
    let prefixed = leaf
        .serialize_with_prefix()
        .expect("prefix serialization should succeed");
    let packet = InboundLedgerPacket::new(
        InboundLedgerDataType::StateNode,
        vec![InboundLedgerNodeData::new(Some(vec![0; 33]), wire)],
    );
    let mut store = RecordingFetchPackStore::default();

    assert!(stash_stale_packet(&packet, &mut store));
    assert_eq!(
        store.blobs.get(leaf.get_hash().as_uint256()),
        Some(&prefixed)
    );
}

#[test]
fn inbound_ledgers_local_routes_active_packets_and_stashes_unknown_state_packets() {
    let active_hash = sample_hash(0x51);
    let mut manager = InboundLedgersLocal::with_clock(ManualClock::new(0));
    manager.insert(InboundLedgerLocal::new(active_hash, 0));

    let base_packet = InboundLedgerPacket::new(
        InboundLedgerDataType::Base,
        vec![InboundLedgerNodeData::new(None, vec![1, 2, 3])],
    );
    let mut stale_store = RecordingFetchPackStore::default();
    let route = manager.got_ledger_data(active_hash, Some(9), base_packet, &mut stale_store);
    assert_eq!(route, InboundLedgerRoute::ActiveNeedsDispatch);
    assert!(
        manager
            .find(active_hash)
            .expect("active inbound ledger should remain present")
            .receive_dispatched()
    );

    let stale_leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
        SHAMapNodeType::AccountState,
        SHAMapItem::new(Uint256::from_array([0x52; 32]), vec![2; 12]),
        0,
    ));
    let stale_packet = InboundLedgerPacket::new(
        InboundLedgerDataType::StateNode,
        vec![InboundLedgerNodeData::new(
            Some(vec![0; 33]),
            stale_leaf
                .serialize_for_wire()
                .expect("wire serialization should succeed"),
        )],
    );
    let route = manager.got_ledger_data(sample_hash(0x99), None, stale_packet, &mut stale_store);
    assert_eq!(route, InboundLedgerRoute::MissingStateStashed);
    assert!(
        stale_store
            .blobs
            .contains_key(stale_leaf.get_hash().as_uint256())
    );
}

#[test]
fn inbound_ledgers_local_can_resume_by_sequence_acquire_cache() {
    let hash = sample_hash(0x53);
    let mut manager = InboundLedgersLocal::with_clock(ManualClock::new(0));
    manager.insert(InboundLedgerLocal::new(hash, 915));

    assert_eq!(
        manager
            .find_by_seq(915)
            .expect("sequence lookup should find the cached inbound ledger")
            .hash(),
        hash
    );

    let resumed = manager
        .remove_by_seq(915)
        .expect("sequence resume should take the cached inbound ledger");
    assert_eq!(resumed.hash(), hash);
    assert!(manager.find(hash).is_none());
}

#[test]
fn inbound_ledgers_local_recent_failures_expire_after_reacquire_interval() {
    let clock = Arc::new(ManualClock::new(0));
    let mut manager = InboundLedgersLocal::with_clock(clock.clone());
    let hash = sample_hash(0x61);

    manager.log_failure(hash, 900);
    assert!(manager.is_failure(hash));
    assert_eq!(manager.recent_failure_seq(hash), Some(900));

    clock.advance_seconds(301);
    assert!(!manager.is_failure(hash));
    assert_eq!(manager.recent_failure_seq(hash), None);
}

#[test]
fn inbound_ledgers_local_stop_clears_state_and_ignores_future_packets() {
    let hash = sample_hash(0x71);
    let mut manager = InboundLedgersLocal::with_clock(ManualClock::new(0));
    manager.insert(InboundLedgerLocal::new(hash, 0));
    manager.log_failure(sample_hash(0x72), 902);

    manager.stop();

    assert!(manager.is_stopped());
    assert_eq!(manager.cache_size(), 0);
    assert!(!manager.is_failure(sample_hash(0x72)));

    let packet = InboundLedgerPacket::new(
        InboundLedgerDataType::Base,
        vec![InboundLedgerNodeData::new(None, vec![1, 2, 3])],
    );
    let mut stale_store = RecordingFetchPackStore::default();
    assert_eq!(
        manager.got_ledger_data(hash, Some(5), packet, &mut stale_store),
        InboundLedgerRoute::MissingIgnored
    );
}

#[test]
fn inbound_ledgers_local_sweep_removes_stale_ledgers_and_expires_failures() {
    let clock = Arc::new(ManualClock::new(0));
    let mut manager = InboundLedgersLocal::with_clock(clock.clone());
    let stale_hash = sample_hash(0x73);
    let failure_hash = sample_hash(0x74);
    manager.insert(InboundLedgerLocal::new(stale_hash, 0));
    manager.log_failure(failure_hash, 903);

    clock.advance_seconds(61);
    assert_eq!(manager.sweep(), 1);
    assert!(manager.find(stale_hash).is_none());
    assert!(manager.is_failure(failure_hash));

    clock.advance_seconds(240);
    assert_eq!(manager.sweep(), 0);
    assert!(!manager.is_failure(failure_hash));
}

#[test]
fn inbound_ledgers_local_fetch_rate_tracks_recent_fetches_only() {
    let clock = Arc::new(ManualClock::new(0));
    let mut manager = InboundLedgersLocal::with_clock(clock.clone());

    manager.on_ledger_fetched();
    manager.on_ledger_fetched();
    assert_eq!(manager.fetch_rate(), 4);

    clock.advance_seconds(31);
    assert_eq!(manager.fetch_rate(), 0);
}

#[test]
fn inbound_ledgers_local_got_fetch_pack_rechecks_active_ledgers() {
    let state_root = make_shared_intrusive(SHAMapTreeNode::new_leaf(
        SHAMapNodeType::AccountState,
        SHAMapItem::new(
            Uint256::from_array([0x81; 32]),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        ),
        0,
    ));
    let state_blob = state_root
        .serialize_with_prefix()
        .expect("state root prefix serialization should succeed");
    let header = sample_header(904, state_root.get_hash(), SHAMapHash::default());
    let header_hash = calculate_ledger_hash(&header);
    let header_blob = serialize_prefixed_ledger_header(&header, false);

    let mut store = RecordingInboundStore::default();
    let mut fetch_pack = RecordingFetchPackStore::default();
    fetch_pack
        .blobs
        .insert(*header_hash.as_uint256(), header_blob.clone());
    fetch_pack
        .blobs
        .insert(*state_root.get_hash().as_uint256(), state_blob);

    let family = family("inbound-ledgers-local-fetch-pack");
    let journal = RecordingJournal;
    let mut manager = InboundLedgersLocal::with_clock(ManualClock::new(0));
    manager.insert(InboundLedgerLocal::new(header_hash, 0));

    assert_eq!(
        manager.got_fetch_pack_with_family_and_config(
            &journal,
            &LedgerConfig::default(),
            &mut store,
            &mut fetch_pack,
            &family,
        ),
        1
    );

    let inbound = manager
        .find(header_hash)
        .expect("active inbound ledger should remain present");
    assert!(inbound.is_complete());
    assert!(!inbound.is_failed());
    assert_eq!(store.stored_headers.borrow().len(), 1);
    assert_eq!(store.stored_nodes.borrow().len(), 1);
}

#[test]
fn inbound_ledgers_local_get_info_reports_active_and_failed_entries() {
    let account_hash = sample_hash(0x91);
    let header = sample_header(905, account_hash, SHAMapHash::default());
    let header_hash = calculate_ledger_hash(&header);
    let header_blob = serialize_prefixed_ledger_header(&header, false);

    let mut store = RecordingInboundStore::default();
    store
        .headers
        .borrow_mut()
        .insert(*header_hash.as_uint256(), header_blob);

    let family = family("inbound-ledgers-local-info");
    let mut inbound = InboundLedgerLocal::new(header_hash, 0);
    inbound.try_db_with_family_and_config(
        &RecordingJournal,
        &LedgerConfig::default(),
        &mut store,
        &mut RecordingFetchPackStore::default(),
        &family,
    );

    let failed_hash = sample_hash(0x92);
    let mut manager = InboundLedgersLocal::with_clock(ManualClock::new(0));
    manager.insert(inbound);
    manager.log_failure(failed_hash, 906);

    let JsonValue::Object(root) = manager.get_info_with_family(&family) else {
        panic!("manager info should be a JSON object");
    };

    let JsonValue::Object(active) = root
        .get("905")
        .cloned()
        .expect("active inbound ledger info should be keyed by sequence")
    else {
        panic!("active entry should be an object");
    };
    assert_eq!(
        active.get("hash"),
        Some(&JsonValue::String(header_hash.to_string()))
    );
    assert_eq!(active.get("have_header"), Some(&JsonValue::Bool(true)));
    assert_eq!(active.get("have_state"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        active.get("have_transactions"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(active.get("timeouts"), Some(&JsonValue::Unsigned(0)));
    assert_eq!(
        active.get("needed_state_hashes"),
        Some(&JsonValue::Array(vec![JsonValue::String(
            account_hash.to_string()
        )]))
    );
    assert!(!active.contains_key("needed_transaction_hashes"));

    let JsonValue::Object(failed) = root
        .get("906")
        .cloned()
        .expect("failed inbound ledger info should be keyed by failure sequence")
    else {
        panic!("failed entry should be an object");
    };
    assert_eq!(failed.get("failed"), Some(&JsonValue::Bool(true)));
}
