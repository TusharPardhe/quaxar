use basics::base_uint::Uint256;
use basics::blob::Blob;
use basics::tagged_cache::ManualClock;
use ledger::{
    InboundLedgerCompletionDisposition, InboundLedgerDataType, InboundLedgerJournal,
    InboundLedgerLocal, InboundLedgerNodeData, InboundLedgerPacket, InboundLedgerRunDataResult,
    InboundLedgerStore, LedgerConfig, LedgerHeader, calculate_ledger_hash,
};
use shamap::family::{NullFullBelowCache, NullMissingNodeReporter, NullNodeFetcher, SHAMapFamily};
use shamap::storage::{NodeObjectType, StoredNode};
use shamap::tree_node_cache::TreeNodeCache;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use time::Duration;

fn sample_hash(fill: u8) -> basics::sha_map_hash::SHAMapHash {
    basics::sha_map_hash::SHAMapHash::new(Uint256::from_array([fill; 32]))
}

#[derive(Debug, Default)]
struct RecordingInboundStore {
    headers: Rc<RefCell<HashMap<Uint256, Blob>>>,
    stored_headers: Rc<RefCell<Vec<(Uint256, u32, Blob)>>>,
    stored_nodes: Rc<RefCell<Vec<StoredNode>>>,
}

impl InboundLedgerStore for RecordingInboundStore {
    fn fetch_ledger_header(
        &mut self,
        hash: basics::sha_map_hash::SHAMapHash,
        _ledger_seq: u32,
    ) -> Option<Blob> {
        self.headers.borrow().get(hash.as_uint256()).cloned()
    }

    fn store_ledger_header(
        &mut self,
        data: Blob,
        hash: basics::sha_map_hash::SHAMapHash,
        ledger_seq: u32,
    ) {
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
struct RecordingFetchPack {
    blobs: Rc<RefCell<HashMap<Uint256, Blob>>>,
}

impl ledger::FetchPackContainer for RecordingFetchPack {
    fn get_fetch_pack(&mut self, hash: Uint256) -> Option<Blob> {
        self.blobs.borrow().get(&hash).cloned()
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

fn sample_header(seq: u32) -> LedgerHeader {
    LedgerHeader {
        seq,
        drops: 55,
        parent_hash: sample_hash(0x01),
        account_hash: sample_hash(0x02),
        tx_hash: sample_hash(0x03),
        parent_close_time: 22,
        close_time: 33,
        close_time_resolution: 30,
        close_flags: 0,
        ..LedgerHeader::default()
    }
}

fn raw_header_bytes(header: &LedgerHeader) -> Blob {
    let prefixed = ledger::serialize_prefixed_ledger_header(header, false);
    prefixed[4..].to_vec()
}

#[test]
fn inbound_got_data_latches_dispatch_until_run_data_drains_queue() {
    let header = sample_header(700);
    let wanted_hash = calculate_ledger_hash(&header);
    let mut inbound = InboundLedgerLocal::new(wanted_hash, 0);
    let base = InboundLedgerPacket::new(
        InboundLedgerDataType::Base,
        vec![InboundLedgerNodeData::new(None, raw_header_bytes(&header))],
    );

    assert!(inbound.got_data(Some(11), base.clone()));
    assert!(!inbound.got_data(Some(11), base));
    assert!(inbound.receive_dispatched());
    assert_eq!(inbound.received_data_len(), 2);
}

#[test]
fn inbound_run_data_tracks_useful_peer_counts_and_resets_dispatch_latch() {
    let header = sample_header(701);
    let wanted_hash = calculate_ledger_hash(&header);
    let mut inbound = InboundLedgerLocal::new(wanted_hash, 0);
    let mut store = RecordingInboundStore::default();
    let mut fetch_pack = RecordingFetchPack::default();
    let journal = RecordingJournal;
    let family = family("inbound-dispatch-run-data");

    let header_packet = InboundLedgerPacket::new(
        InboundLedgerDataType::Base,
        vec![InboundLedgerNodeData::new(None, raw_header_bytes(&header))],
    );
    let empty_tx_packet = InboundLedgerPacket::new(InboundLedgerDataType::TransactionNode, vec![]);

    assert!(inbound.got_data(Some(1), header_packet));
    assert!(!inbound.got_data(Some(2), empty_tx_packet));

    let result = inbound.run_data_with_family_and_config_and_sampler(
        &journal,
        &LedgerConfig::default(),
        &mut store,
        &mut fetch_pack,
        &family,
        &mut |scores, max| {
            assert_eq!(max, ledger::INBOUND_LEDGER_MAX_USEFUL_PEERS);
            assert_eq!(scores.len(), 1);
            assert_eq!(scores[0].peer_id, 1);
            vec![scores[0].peer_id]
        },
    );

    assert_eq!(
        result,
        InboundLedgerRunDataResult {
            triggered_peer_ids: vec![1],
            processed_packets: 2,
            max_useful_count: 1,
            packet_stats: Vec::new(),
        }
    );
    assert!(!inbound.receive_dispatched());
    assert_eq!(inbound.received_data_len(), 0);
    assert!(inbound.progress());
    assert_eq!(inbound.stats().get_good(), 1);
}

#[test]
fn inbound_got_data_rejects_new_packets_after_completion_is_observed() {
    let header = sample_header(702);
    let wanted_hash = calculate_ledger_hash(&header);
    let mut inbound = InboundLedgerLocal::new(wanted_hash, 0);
    let mut store = RecordingInboundStore::default();
    let mut fetch_pack = RecordingFetchPack::default();
    let journal = RecordingJournal;
    let family = family("inbound-dispatch-complete-lifecycle");

    let header_packet = InboundLedgerPacket::new(
        InboundLedgerDataType::Base,
        vec![InboundLedgerNodeData::new(None, raw_header_bytes(&header))],
    );

    assert!(inbound.got_data(Some(1), header_packet));
    let _ = inbound.run_data_with_family_and_config_and_sampler(
        &journal,
        &LedgerConfig::default(),
        &mut store,
        &mut fetch_pack,
        &family,
        &mut |_, _| Vec::new(),
    );

    inbound
        .ledger_mut()
        .expect("header processing should create a ledger")
        .tx_map_mut()
        .clear_synching();
    inbound
        .ledger_mut()
        .expect("header processing should create a ledger")
        .state_map_mut()
        .clear_synching();
    assert!(!inbound.revalidate_map_sync_with_family(&family));
    inbound.maybe_finish(&journal);

    assert_eq!(
        inbound.completion_disposition(),
        Some(InboundLedgerCompletionDisposition::Complete(
            ledger::InboundLedgerReason::Generic
        ))
    );
    assert!(!inbound.got_data(
        Some(2),
        InboundLedgerPacket::new(InboundLedgerDataType::TransactionNode, vec![])
    ));
}
