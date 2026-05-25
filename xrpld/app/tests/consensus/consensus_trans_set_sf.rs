use app::{
    ConsensusTransSetApp, ConsensusTransSetFilterFactory, ConsensusTransSetNodeCache,
    ConsensusTransSetSF, ConsensusTransSetSubmitSink, Transaction, TransactionMaster,
    TransactionMasterApp, decode_consensus_trans_set_transaction,
    encode_consensus_trans_set_transaction_node,
};
use basics::intrusive_pointer::make_shared_intrusive;
use basics::sha_map_hash::SHAMapHash;
use ledger::TransactionAcquire;
use overlay::{PeerSet, SimplePeerSet};
use protocol::{STAmount, STTx, TxType, get_field_by_symbol};
use shamap::fetch::SHAMapSyncFilter;
use shamap::item::SHAMapItem;
use shamap::node_id::{SHAMapNodeId, select_branch};
use shamap::tree_node::{SHAMapNodeType, SHAMapTreeNode};
use std::sync::{Arc, Mutex};

fn account(fill: u8) -> protocol::AccountID {
    protocol::AccountID::from_array([fill; 20])
}

fn payment_tx(sequence: u32, fill: u8) -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(fill));
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            account(fill.wrapping_add(1)),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

#[derive(Debug, Default)]
struct SubmitRecorder {
    submitted: Mutex<Vec<basics::base_uint::Uint256>>,
}

impl ConsensusTransSetSubmitSink for SubmitRecorder {
    fn submit_transaction(&self, tx: Arc<STTx>) {
        self.submitted
            .lock()
            .expect("submit mutex must not be poisoned")
            .push(tx.get_transaction_id());
    }
}

struct OwnedConsensusApp {
    master: Arc<TransactionMaster>,
    submit_sink: Arc<SubmitRecorder>,
}

impl ConsensusTransSetApp for OwnedConsensusApp {
    fn submit_transaction(&self, tx: Arc<STTx>) {
        self.submit_sink.submit_transaction(tx);
    }

    fn fetch_from_cache(
        &self,
        hash: &basics::base_uint::Uint256,
    ) -> Option<app::SharedTransaction> {
        self.master.fetch_from_cache(hash)
    }
}

fn tx_leaf_fixture(
    sequence: u32,
    fill: u8,
) -> (
    STTx,
    SHAMapHash,
    SHAMapHash,
    Vec<u8>,
    Vec<u8>,
    SHAMapNodeId,
    usize,
) {
    let tx = payment_tx(sequence, fill);
    let raw = tx.get_serializer().data().to_vec();
    let leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
        SHAMapNodeType::TransactionNm,
        SHAMapItem::new(tx.get_transaction_id(), raw),
        0,
    ));
    let child_wire = leaf
        .serialize_for_wire()
        .expect("transaction wire serialization should succeed");
    let node_hash = leaf.get_hash();

    let branch = select_branch(SHAMapNodeId::default(), tx.get_transaction_id());
    let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
    root.set_child_hash(branch, node_hash);
    root.update_hash_deep();
    let root_wire = root
        .serialize_for_wire()
        .expect("root wire serialization should succeed");
    let root_hash = root.get_hash();
    let child_id = SHAMapNodeId::default()
        .get_child_node_id(branch)
        .expect("child node id should exist");

    (
        tx, root_hash, node_hash, root_wire, child_wire, child_id, branch,
    )
}

#[test]
fn consensus_trans_set_sf_got_node_returns_early_for_filter_hits() {
    let master = TransactionMaster::new();
    let recorder = SubmitRecorder::default();
    let app = TransactionMasterApp::new(&master, &recorder);
    let cache = ConsensusTransSetSF::new_cache();
    let mut filter = ConsensusTransSetSF::from_app(&app, &cache);

    let tx = payment_tx(1, 0x11);
    let node_hash = SHAMapHash::new(tx.get_transaction_id());
    let node_blob = encode_consensus_trans_set_transaction_node(node_hash, &tx);

    filter.got_node(true, node_hash, 0, node_blob, SHAMapNodeType::TransactionNm);

    assert!(cache.retrieve(&node_hash).is_none());
    assert!(
        recorder
            .submitted
            .lock()
            .expect("submit mutex must not be poisoned")
            .is_empty()
    );
}

#[test]
fn consensus_trans_set_sf_inserts_and_schedules_transactions() {
    let master = TransactionMaster::new();
    let recorder = SubmitRecorder::default();
    let app = TransactionMasterApp::new(&master, &recorder);
    let cache = ConsensusTransSetSF::new_cache();
    let mut filter = ConsensusTransSetSF::from_app(&app, &cache);

    let tx = payment_tx(2, 0x21);
    let node_hash = SHAMapHash::new(tx.get_transaction_id());
    let node_blob = encode_consensus_trans_set_transaction_node(node_hash, &tx);

    filter.got_node(
        false,
        node_hash,
        0,
        node_blob.clone(),
        SHAMapNodeType::TransactionNm,
    );

    assert_eq!(cache.retrieve(&node_hash), Some(node_blob));
    assert_eq!(
        recorder
            .submitted
            .lock()
            .expect("submit mutex must not be poisoned")
            .as_slice(),
        &[tx.get_transaction_id()]
    );
}

#[test]
fn consensus_trans_set_sf_get_node_prefers_node_cache() {
    let master = TransactionMaster::new();
    let recorder = SubmitRecorder::default();
    let app = TransactionMasterApp::new(&master, &recorder);
    let cache = ConsensusTransSetSF::new_cache();
    let mut filter = ConsensusTransSetSF::from_app(&app, &cache);

    let tx = payment_tx(3, 0x31);
    let node_hash = SHAMapHash::new(tx.get_transaction_id());
    let node_blob = vec![1, 2, 3, 4];
    cache.insert(node_hash, node_blob.clone());

    assert_eq!(filter.get_node(node_hash), Some(node_blob));
}

#[test]
fn consensus_trans_set_sf_get_node_serializes_cached_transactions() {
    let master = TransactionMaster::new();
    let recorder = SubmitRecorder::default();
    let app = TransactionMasterApp::new(&master, &recorder);
    let cache = ConsensusTransSetSF::new_cache();
    let mut filter = ConsensusTransSetSF::from_app(&app, &cache);

    let tx = Arc::new(payment_tx(4, 0x41));
    let node_hash = SHAMapHash::new(tx.get_transaction_id());
    let expected = encode_consensus_trans_set_transaction_node(node_hash, tx.as_ref());
    let mut shared = Arc::new(Mutex::new(Transaction::new(Arc::clone(&tx))));
    master.canonicalize(&mut shared);

    assert_eq!(filter.get_node(node_hash), Some(expected));
}

#[test]
fn consensus_trans_set_helpers_round_trip_prefixed_transaction_payload() {
    let tx = payment_tx(5, 0x51);
    let node_hash = SHAMapHash::new(tx.get_transaction_id());
    let encoded = encode_consensus_trans_set_transaction_node(node_hash, &tx);
    let decoded = decode_consensus_trans_set_transaction(node_hash, &encoded[4..])
        .expect("prefixed transaction payload should decode");

    assert_eq!(decoded.get_transaction_id(), tx.get_transaction_id());
}

#[test]
fn consensus_trans_set_sf_returns_none_on_total_cache_miss() {
    let master = TransactionMaster::new();
    let recorder = SubmitRecorder::default();
    let app = TransactionMasterApp::new(&master, &recorder);
    let cache = ConsensusTransSetSF::new_cache();
    let mut filter = ConsensusTransSetSF::from_app(&app, &cache);

    assert_eq!(filter.get_node(SHAMapHash::default()), None);
}

#[test]
fn transaction_acquire_take_nodes_uses_real_consensus_filter_for_non_root_tx_nodes() {
    let master = Arc::new(TransactionMaster::new());
    let recorder = Arc::new(SubmitRecorder::default());
    let app = Arc::new(OwnedConsensusApp {
        master,
        submit_sink: Arc::clone(&recorder),
    });
    let node_cache = Arc::new(ConsensusTransSetNodeCache::new(
        "tx-acquire-consensus-filter",
        32,
        time::Duration::minutes(30),
        basics::tagged_cache::MonotonicClock::default(),
    ));
    let peer_set: Arc<dyn PeerSet> = Arc::new(SimplePeerSet::new(Vec::new()));
    let (tx, root_hash, node_hash, root_wire, child_wire, child_id, _branch) =
        tx_leaf_fixture(6, 0x61);
    let mut acquire = TransactionAcquire::with_filter_factory(
        *root_hash.as_uint256(),
        peer_set,
        Some(Arc::new(ConsensusTransSetFilterFactory::new(
            Arc::clone(&app),
            Arc::clone(&node_cache),
        ))),
    );

    let result = acquire.take_nodes(
        &[(SHAMapNodeId::default(), root_wire), (child_id, child_wire)],
        None,
    );

    assert!(result.is_useful());
    assert_eq!(
        node_cache.retrieve(&node_hash),
        Some(encode_consensus_trans_set_transaction_node(node_hash, &tx))
    );
    assert_eq!(
        recorder
            .submitted
            .lock()
            .expect("submit mutex must not be poisoned")
            .as_slice(),
        &[tx.get_transaction_id()]
    );
}

#[test]
fn transaction_acquire_trigger_uses_real_consensus_filter_for_missing_tx_children() {
    let master = Arc::new(TransactionMaster::new());
    let recorder = Arc::new(SubmitRecorder::default());
    let app = Arc::new(OwnedConsensusApp {
        master,
        submit_sink: Arc::clone(&recorder),
    });
    let node_cache = Arc::new(ConsensusTransSetNodeCache::new(
        "tx-acquire-missing-consensus-filter",
        32,
        time::Duration::minutes(30),
        basics::tagged_cache::MonotonicClock::default(),
    ));
    let peer_set: Arc<dyn PeerSet> = Arc::new(SimplePeerSet::new(Vec::new()));
    let (tx, root_hash, node_hash, root_wire, _child_wire, _child_id, branch) =
        tx_leaf_fixture(7, 0x71);
    let encoded = encode_consensus_trans_set_transaction_node(node_hash, &tx);
    node_cache.insert(node_hash, encoded);

    let mut acquire = TransactionAcquire::with_filter_factory(
        *root_hash.as_uint256(),
        peer_set,
        Some(Arc::new(ConsensusTransSetFilterFactory::new(
            Arc::clone(&app),
            Arc::clone(&node_cache),
        ))),
    );

    let result = acquire.take_nodes(&[(SHAMapNodeId::default(), root_wire)], None);

    assert!(result.is_useful());
    assert!(acquire.is_complete());
    assert!(acquire.map().root().get_child(branch).is_some());
    assert!(
        recorder
            .submitted
            .lock()
            .expect("submit mutex must not be poisoned")
            .is_empty()
    );
}
