use basics::base_uint::Uint256;
use basics::intrusive_pointer::make_shared_intrusive;
use ledger::{Ledger, LedgerHeader};
use shamap::item::SHAMapItem;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::{SHAMapNodeType, SHAMapTreeNode};

fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

#[test]
fn ledger_tx_exists_reports_exact_membership() {
    let tx_key = sample_uint256(0x41);
    let tx_root = make_shared_intrusive(SHAMapTreeNode::new_leaf(
        SHAMapNodeType::TransactionNm,
        SHAMapItem::new(tx_key, vec![0xAB; 20]),
        0,
    ));
    let ledger = Ledger::from_maps(
        LedgerHeader {
            seq: 900,
            ..LedgerHeader::default()
        },
        SyncTree::new_with_type(SHAMapType::State, false, 900),
        SyncTree::from_root_with_type(
            tx_root,
            SHAMapType::Transaction,
            false,
            900,
            SyncState::Immutable,
        ),
    );

    assert!(ledger.tx_exists(tx_key));
    assert!(!ledger.tx_exists(sample_uint256(0x42)));
}

#[test]
fn ledger_tx_exists_does_not_need_fetch_for_missing_child_paths() {
    let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
    root.set_child_hash(
        0,
        ledger::calculate_ledger_hash(&LedgerHeader {
            seq: 901,
            ..LedgerHeader::default()
        }),
    );
    root.update_hash_deep();

    let ledger = Ledger::from_maps(
        LedgerHeader {
            seq: 901,
            ..LedgerHeader::default()
        },
        SyncTree::new_with_type(SHAMapType::State, true, 901),
        SyncTree::from_root_with_type(
            root,
            SHAMapType::Transaction,
            true,
            901,
            SyncState::Immutable,
        ),
    );

    let missing_key = sample_uint256(0x00);
    assert!(!ledger.tx_exists(missing_key));
    assert!(ledger.tx_map().root().get_child(0).is_none());
}
