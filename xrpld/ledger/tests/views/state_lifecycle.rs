use basics::base_uint::Uint256;
use basics::intrusive_pointer::make_shared_intrusive;
use basics::sha_map_hash::SHAMapHash;
use ledger::{
    ApplyView, Fees, Ledger, LedgerHeader, SLCF_NO_CONSENSUS_TIME, Sandbox, amendments_key,
    calculate_ledger_hash, encode_amendments_entry, encode_fee_settings_entry, fees_key,
};
use protocol::{
    AccountID, STAmount, STLedgerEntry, XRPAmount, account_keylet, feature_xrp_fees,
    get_field_by_symbol,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::{SHAMapNodeType, SHAMapTreeNode};

fn sample_hash(fill: u8) -> SHAMapHash {
    SHAMapHash::new(Uint256::from_array([fill; 32]))
}

fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn build_state_map_with_items(items: &[(Uint256, Vec<u8>)], ledger_seq: u32) -> SyncTree {
    let mut tree = MutableTree::new(ledger_seq);
    for (key, payload) in items {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*key, payload.clone()),
        )
        .expect("state map item insertion should succeed");
    }

    SyncTree::from_root_with_type(
        tree.root(),
        SHAMapType::State,
        true,
        ledger_seq,
        SyncState::Modifying,
    )
}

#[test]
fn sandbox_apply_with_tx_thread_updates_threaded_sles() {
    let ledger_seq = 17254325;
    let account = account(0x33);
    let account_keylet = account_keylet(
        basics::base_uint::Uint160::from_slice(account.data()).expect("account width"),
    );
    let previous_tx = sample_uint256(0xA1);
    let current_tx = sample_uint256(0xB2);

    let mut account_root = STLedgerEntry::new(account_keylet);
    account_root.set_account_id(get_field_by_symbol("sfAccount"), account);
    account_root.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(1_000)),
    );
    account_root.set_field_u32(get_field_by_symbol("sfSequence"), 10);
    account_root.set_field_u32(get_field_by_symbol("sfOwnerCount"), 0);
    account_root.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), previous_tx);
    account_root.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), ledger_seq - 1);

    let state_map = build_state_map_with_items(
        &[(
            account_keylet.key,
            account_root.get_serializer().data().to_vec(),
        )],
        ledger_seq,
    );
    let base = Ledger::from_maps(
        LedgerHeader {
            seq: ledger_seq,
            ..LedgerHeader::default()
        },
        state_map.clone(),
        SyncTree::from_root_with_type(
            make_shared_intrusive(SHAMapTreeNode::new_inner(0)),
            SHAMapType::Transaction,
            true,
            ledger_seq,
            SyncState::Modifying,
        ),
    );
    let mut built = Ledger::from_maps(
        base.header(),
        state_map,
        SyncTree::from_root_with_type(
            make_shared_intrusive(SHAMapTreeNode::new_inner(0)),
            SHAMapType::Transaction,
            true,
            ledger_seq,
            SyncState::Modifying,
        ),
    );

    let mut sandbox = Sandbox::new(std::sync::Arc::new(base), protocol::ApplyFlags::default());
    let checked_out = sandbox
        .peek(account_keylet)
        .expect("peek should succeed")
        .expect("account exists");
    let mut modified = checked_out.clone_as_object();
    modified.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(900)),
    );
    sandbox
        .update(std::sync::Arc::new(STLedgerEntry::from_stobject(
            modified,
            account_keylet.key,
        )))
        .expect("update should succeed");

    let rules = built.rules().clone();
    sandbox
        .apply_with_tx_thread(&mut built, current_tx, ledger_seq, &rules)
        .expect("apply should succeed");

    let threaded = built
        .read(account_keylet)
        .expect("read should succeed")
        .expect("account remains");
    assert_eq!(
        threaded.get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
        current_tx
    );
    assert_eq!(
        threaded.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")),
        ledger_seq
    );
    assert_eq!(
        threaded
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        900
    );
}

#[test]
fn ledger_new_matches_narrow_cpp_map_roles() {
    let ledger = Ledger::new(
        LedgerHeader {
            seq: 500,
            ..LedgerHeader::default()
        },
        true,
    );

    assert_eq!(ledger.state_map().map_type(), SHAMapType::State);
    assert_eq!(ledger.tx_map().map_type(), SHAMapType::Transaction);
}

#[test]
fn ledger_set_immutable_with_rehash_pulls_map_hashes_into_header_and_hashes_ledger() {
    let tx_root = make_shared_intrusive(SHAMapTreeNode::new_leaf(
        SHAMapNodeType::TransactionNm,
        SHAMapItem::new(sample_uint256(0x71), vec![0x11; 20]),
        0,
    ));
    let state_root = make_shared_intrusive(SHAMapTreeNode::new_leaf(
        SHAMapNodeType::AccountState,
        SHAMapItem::new(sample_uint256(0x72), vec![0x22; 20]),
        0,
    ));
    let tx_hash = tx_root.get_hash();
    let account_hash = state_root.get_hash();
    let mut expected_header = LedgerHeader {
        seq: 802,
        drops: 50,
        tx_hash,
        account_hash,
        parent_hash: sample_hash(0x73),
        parent_close_time: 60,
        close_time: 61,
        close_time_resolution: 62,
        close_flags: 63,
        ..LedgerHeader::default()
    };
    let expected_hash = calculate_ledger_hash(&expected_header);

    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq: 802,
            drops: 50,
            parent_hash: sample_hash(0x73),
            parent_close_time: 60,
            close_time: 61,
            close_time_resolution: 62,
            close_flags: 63,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state_root,
            SHAMapType::State,
            true,
            802,
            SyncState::Modifying,
        ),
        SyncTree::from_root_with_type(
            tx_root,
            SHAMapType::Transaction,
            true,
            802,
            SyncState::Modifying,
        ),
    );

    ledger.set_immutable(true);

    assert!(ledger.is_immutable());
    assert_eq!(ledger.tx_map().state(), SyncState::Immutable);
    assert_eq!(ledger.state_map().state(), SyncState::Immutable);
    expected_header.hash = expected_hash;
    assert_eq!(ledger.header(), expected_header);
}

#[test]
fn ledger_set_immutable_without_rehash_keeps_existing_header_hashes() {
    let tx_root = make_shared_intrusive(SHAMapTreeNode::new_leaf(
        SHAMapNodeType::TransactionNm,
        SHAMapItem::new(sample_uint256(0x81), vec![0x33; 20]),
        0,
    ));
    let state_root = make_shared_intrusive(SHAMapTreeNode::new_leaf(
        SHAMapNodeType::AccountState,
        SHAMapItem::new(sample_uint256(0x82), vec![0x44; 20]),
        0,
    ));
    let original = LedgerHeader {
        seq: 803,
        hash: sample_hash(0x84),
        tx_hash: sample_hash(0x85),
        account_hash: sample_hash(0x86),
        ..LedgerHeader::default()
    };
    let mut ledger = Ledger::from_maps(
        original,
        SyncTree::from_root_with_type(
            state_root,
            SHAMapType::State,
            true,
            803,
            SyncState::Modifying,
        ),
        SyncTree::from_root_with_type(
            tx_root,
            SHAMapType::Transaction,
            true,
            803,
            SyncState::Modifying,
        ),
    );

    ledger.set_immutable(false);

    assert!(ledger.is_immutable());
    assert_eq!(ledger.tx_map().state(), SyncState::Immutable);
    assert_eq!(ledger.state_map().state(), SyncState::Immutable);
    assert_eq!(ledger.header(), original);
}

#[test]
fn ledger_set_immutable_refreshes_rules_and_fees() {
    let preset_amendment = sample_uint256(0x87);
    let ledger_seq = 804;
    let original = LedgerHeader {
        seq: ledger_seq,
        hash: sample_hash(0x88),
        tx_hash: sample_hash(0x89),
        account_hash: sample_hash(0x8A),
        ..LedgerHeader::default()
    };
    let state_map = build_state_map_with_items(
        &[
            (
                amendments_key(),
                encode_amendments_entry(&[feature_xrp_fees(), preset_amendment]),
            ),
            (
                fees_key(),
                encode_fee_settings_entry(
                    Fees {
                        base: 44,
                        reserve: 55,
                        increment: 66,
                    },
                    true,
                ),
            ),
        ],
        ledger_seq,
    );

    let mut ledger = Ledger::from_maps(
        original,
        state_map,
        SyncTree::new_with_type(SHAMapType::Transaction, true, ledger_seq),
    );

    ledger.set_immutable(false);

    assert!(ledger.is_immutable());
    assert_eq!(ledger.header().hash, original.hash);
    assert_eq!(ledger.header().tx_hash, original.tx_hash);
    assert_eq!(ledger.header().account_hash, original.account_hash);
    assert_eq!(ledger.tx_map().state(), SyncState::Immutable);
    assert_eq!(ledger.state_map().state(), SyncState::Immutable);
    assert_eq!(
        ledger.fees(),
        Fees {
            base: 44,
            reserve: 55,
            increment: 66,
        }
    );
    assert!(ledger.rules().enabled(&feature_xrp_fees()));
    assert!(ledger.rules().enabled(&preset_amendment));
}

#[test]
fn ledger_set_validated_flips_only_the_validated_flag() {
    let original = LedgerHeader {
        seq: 806,
        hash: sample_hash(0xB1),
        parent_hash: sample_hash(0xB2),
        tx_hash: sample_hash(0xB3),
        account_hash: sample_hash(0xB4),
        drops: 90,
        parent_close_time: 10,
        close_time: 20,
        close_time_resolution: 30,
        close_flags: SLCF_NO_CONSENSUS_TIME,
        ..LedgerHeader::default()
    };
    let mut ledger = Ledger::new(original, true);

    ledger.set_validated();

    assert!(ledger.header().validated);
    assert!(!ledger.header().accepted);
    assert_eq!(ledger.header().seq, original.seq);
    assert_eq!(ledger.header().hash, original.hash);
    assert_eq!(ledger.header().parent_hash, original.parent_hash);
    assert_eq!(ledger.header().tx_hash, original.tx_hash);
    assert_eq!(ledger.header().account_hash, original.account_hash);
    assert_eq!(ledger.header().drops, original.drops);
    assert_eq!(
        ledger.header().parent_close_time,
        original.parent_close_time
    );
    assert_eq!(ledger.header().close_time, original.close_time);
    assert_eq!(
        ledger.header().close_time_resolution,
        original.close_time_resolution
    );
    assert_eq!(ledger.header().close_flags, original.close_flags);
}
