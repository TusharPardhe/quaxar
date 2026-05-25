use basics::base_uint::{Uint160, Uint256};
use ledger::{
    ACCOUNT_TRANSFER_RATE_PARITY, Ledger, LedgerHeader, check_destination_and_tag,
    is_global_frozen, transfer_rate,
};
use protocol::{
    AccountID, LedgerEntryType, STAmount, STLedgerEntry, Ter, account_keylet, get_field_by_symbol,
    lsfGlobalFreeze, lsfRequireDestTag,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

fn sample_uint160(fill: u8) -> Uint160 {
    Uint160::from_array([fill; Uint160::BYTES])
}

fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; Uint256::BYTES])
}

fn account_root_entry(account: Uint160, flags: u32, transfer_rate: Option<u32>) -> STLedgerEntry {
    let mut entry =
        STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, account_keylet(account).key);
    entry.set_account_id(
        get_field_by_symbol("sfAccount"),
        AccountID::from_slice(account.data()).expect("account width"),
    );
    entry.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    entry.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(100, false),
    );
    entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), 0);
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0xA5));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 2);
    entry.set_field_u32(get_field_by_symbol("sfFlags"), flags);
    if let Some(transfer_rate) = transfer_rate {
        entry.set_field_u32(get_field_by_symbol("sfTransferRate"), transfer_rate);
    }
    entry
}

fn ledger_with_account_roots(entries: impl IntoIterator<Item = STLedgerEntry>) -> Ledger {
    let mut tree = MutableTree::new(1);

    for entry in entries {
        let key = *entry.key();
        let payload = entry.get_serializer().data().to_vec();
        tree.add_item(SHAMapNodeType::AccountState, SHAMapItem::new(key, payload))
            .expect("account-root insertion should succeed");
    }

    Ledger::from_maps(
        LedgerHeader {
            seq: 1,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            tree.root(),
            SHAMapType::State,
            false,
            1,
            SyncState::Immutable,
        ),
        SyncTree::new_with_type(SHAMapType::Transaction, false, 1),
    )
}

#[test]
fn is_global_frozen_read_only_cases() {
    let frozen_account = sample_uint160(0x11);
    let unfrozen_account = sample_uint160(0x22);
    let missing_account = sample_uint160(0x33);
    let ledger = ledger_with_account_roots([
        account_root_entry(frozen_account, lsfGlobalFreeze, None),
        account_root_entry(unfrozen_account, 0, None),
    ]);

    assert!(is_global_frozen(&ledger, frozen_account).expect("frozen lookup should succeed"));
    assert!(!is_global_frozen(&ledger, unfrozen_account).expect("unfrozen lookup should succeed"));
    assert!(!is_global_frozen(&ledger, Uint160::zero()).expect("xrp short-circuit should succeed"));
    assert!(!is_global_frozen(&ledger, missing_account).expect("missing lookup should succeed"));
}

#[test]
fn transfer_rate_fallback_and_account_value() {
    let default_account = sample_uint160(0x44);
    let rated_account = sample_uint160(0x55);
    let ledger = ledger_with_account_roots([
        account_root_entry(rated_account, 0, Some(1_250_000_000)),
        account_root_entry(default_account, 0, None),
    ]);

    assert_eq!(
        transfer_rate(&ledger, rated_account).expect("rate lookup should succeed"),
        1_250_000_000
    );
    assert_eq!(
        transfer_rate(&ledger, default_account).expect("missing lookup should succeed"),
        ACCOUNT_TRANSFER_RATE_PARITY
    );
    assert_eq!(
        transfer_rate(&ledger, Uint160::zero()).expect("xrp fallback should succeed"),
        ACCOUNT_TRANSFER_RATE_PARITY
    );
}

#[test]
fn check_destination_and_tag_ter_results() {
    let mut tagged_entry = account_root_entry(sample_uint160(0x66), lsfRequireDestTag, None);
    let mut untagged_entry = account_root_entry(sample_uint160(0x77), 0, None);

    assert_eq!(check_destination_and_tag(None, false), Ter::TEC_NO_DST);
    assert_eq!(
        check_destination_and_tag(Some(&tagged_entry), false),
        Ter::TEC_DST_TAG_NEEDED
    );
    assert_eq!(
        check_destination_and_tag(Some(&tagged_entry), true),
        Ter::TES_SUCCESS
    );
    assert_eq!(
        check_destination_and_tag(Some(&untagged_entry), false),
        Ter::TES_SUCCESS
    );

    tagged_entry.set_field_u32(get_field_by_symbol("sfFlags"), 0);
    untagged_entry.set_field_u32(get_field_by_symbol("sfFlags"), lsfRequireDestTag);
    assert_eq!(
        check_destination_and_tag(Some(&tagged_entry), false),
        Ter::TES_SUCCESS
    );
    assert_eq!(
        check_destination_and_tag(Some(&untagged_entry), false),
        Ter::TEC_DST_TAG_NEEDED
    );
}
