use std::net::SocketAddr;
use std::sync::Arc;

use basics::base_uint::Uint256;
use ledger::{InboundTransactions, InboundTransactionsDataStatus};
use overlay::message::wire::TmLedgerNode;
use overlay::{Peer, PeerImp, SimplePeerSetBuilder, TmLedgerData};
use protocol::PublicKey;
use shamap::{
    item::SHAMapItem, mutation::MutableTree, node_id::SHAMapNodeId, sync::SHAMapAddNode,
    sync::SyncTree, tree_node::SHAMapNodeType,
};

fn test_peer(id: u32) -> Arc<PeerImp> {
    PeerImp::new(
        id,
        SocketAddr::from(([127, 0, 0, 1], 5000 + id as u16)),
        PublicKey::from_bytes([0x02; 33]),
        format!("peer-{id}"),
    )
}

fn tx_set_root() -> (Uint256, Vec<u8>) {
    let mut tree = MutableTree::new(1);
    tree.add_item(
        SHAMapNodeType::TransactionNm,
        SHAMapItem::new(
            Uint256::from_array([0x11; 32]),
            vec![
                0xCA, 0xFE, 0xBA, 0xBE, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80,
            ],
        ),
    )
    .expect("tx leaf should insert");

    let root = tree.root();
    (
        *root.get_hash().as_uint256(),
        root.serialize_for_wire()
            .expect("root wire form should serialize"),
    )
}

fn tx_data_packet(hash: Uint256, root_bytes: Vec<u8>) -> TmLedgerData {
    TmLedgerData {
        ledger_hash: hash.data().to_vec(),
        ledger_seq: 0,
        r#type: 3,
        nodes: vec![TmLedgerNode {
            nodedata: root_bytes,
            nodeid: Some(SHAMapNodeId::default().get_raw_string()),
        }],
        request_cookie: None,
        error: None,
    }
}

#[test]
fn inbound_transactions_keeps_zero_hash_set() {
    let mut inbound = InboundTransactions::new(Arc::new(SimplePeerSetBuilder::new(Vec::new())));

    assert!(inbound.acquire(Uint256::zero()).is_none());
    assert!(inbound.get_set(Uint256::zero(), false).is_some());
}

#[test]
fn inbound_transactions_acquires_and_caches_transaction_sets() {
    let (hash, root_bytes) = tx_set_root();
    let peer = test_peer(7);
    peer.record_tx_set(hash);
    let peer_dyn: Arc<dyn Peer> = peer.clone();

    let mut inbound =
        InboundTransactions::new(Arc::new(SimplePeerSetBuilder::new(vec![Arc::clone(
            &peer_dyn,
        )])));

    assert!(inbound.get_set(hash, false).is_none());
    assert!(inbound.get_set(hash, true).is_none());

    let queued = peer.queued_messages();
    assert_eq!(queued.len(), 1);

    let packet = tx_data_packet(hash, root_bytes);
    let status = inbound.got_data(hash, Some(peer_dyn), &packet);
    assert_eq!(
        status,
        InboundTransactionsDataStatus::Applied(SHAMapAddNode::useful())
    );

    inbound
        .acquire_mut(hash)
        .expect("acquire state should still exist")
        .invoke_on_timer();

    let acquired = inbound
        .acquire(hash)
        .expect("acquire state should still be available");
    assert!(acquired.has_root());
    assert!(!acquired.is_failed());

    let set = Arc::new(acquired.map().clone());
    assert!(inbound.give_set(hash, Arc::clone(&set), true));
    assert!(inbound.acquire(hash).is_none());
    assert!(inbound.get_set(hash, false).is_some());
}

#[test]
fn inbound_transactions_stop_clears_active_acquires_shutdown() {
    let hash = Uint256::from_array([0x33; 32]);
    let peer = test_peer(9);
    peer.record_tx_set(hash);
    let peer_dyn: Arc<dyn Peer> = peer.clone();

    let mut inbound =
        InboundTransactions::new(Arc::new(SimplePeerSetBuilder::new(vec![Arc::clone(
            &peer_dyn,
        )])));

    assert!(inbound.get_set(hash, true).is_none());
    assert!(inbound.acquire(hash).is_some());

    inbound.stop();

    assert!(inbound.is_stopped());
    assert!(inbound.acquire(hash).is_none());
    assert!(inbound.get_set(hash, true).is_none());
    assert!(!inbound.give_set(
        hash,
        Arc::new(SyncTree::new_with_type(
            shamap::sync::SHAMapType::Transaction,
            true,
            0
        )),
        false
    ));
    assert_eq!(inbound.len(), 0);
}
