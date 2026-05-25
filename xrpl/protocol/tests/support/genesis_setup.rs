//! Isolated integration tests for genesis/setup constructor-entry helpers.

use basics::base_uint::Uint256;
use protocol::{
    ConstructorAccountRootEntry, ConstructorAmendmentsEntry, ConstructorFeeSettingsEntry,
    ConstructorLedgerEntry, account_root_key, amendments_key,
    build_genesis_setup_constructor_entries, build_genesis_state_constructor_entries,
    constructor_ledger_entry_key, constructor_ledger_item, constructor_ledger_items,
    decode_constructor_account_root_entry, decode_constructor_amendments_entry,
    decode_constructor_fee_settings_entry, feature_xrp_fees, fees_key, genesis_account_id,
    make_constructor_fee_settings_entry,
};

fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

#[test]
fn setup_constructor_entries_keep_amendments_before_fees_and_skip_empty_amendments() {
    let entries = build_genesis_setup_constructor_entries(10, 20, 30, &[feature_xrp_fees()]);
    assert_eq!(entries.len(), 2);
    assert!(matches!(entries[0], ConstructorLedgerEntry::Amendments(_)));
    assert!(matches!(entries[1], ConstructorLedgerEntry::FeeSettings(_)));

    let without_amendments = build_genesis_setup_constructor_entries(10, 20, 30, &[]);
    assert_eq!(without_amendments.len(), 1);
    assert!(matches!(
        without_amendments[0],
        ConstructorLedgerEntry::FeeSettings(_)
    ));
}

#[test]
fn state_constructor_entries_keep_master_account_before_singletons() {
    let entries = build_genesis_state_constructor_entries(
        100_000_000_000_000_000,
        10,
        20,
        30,
        &[feature_xrp_fees(), sample_uint256(0xAB)],
    );

    assert_eq!(entries.len(), 3);
    assert!(matches!(entries[0], ConstructorLedgerEntry::AccountRoot(_)));
    assert!(matches!(entries[1], ConstructorLedgerEntry::Amendments(_)));
    assert!(matches!(entries[2], ConstructorLedgerEntry::FeeSettings(_)));
}

#[test]
fn constructor_item_helpers_preserve_current_key_and_payload_shapes() {
    let account_root = ConstructorLedgerEntry::AccountRoot(ConstructorAccountRootEntry {
        sequence: 1,
        balance_drops: 100_000_000_000_000_000,
        account_id: genesis_account_id(),
    });
    let amendments = ConstructorLedgerEntry::Amendments(ConstructorAmendmentsEntry {
        amendments: vec![feature_xrp_fees(), sample_uint256(0xCD)],
    });
    let fees = ConstructorLedgerEntry::FeeSettings(make_constructor_fee_settings_entry(
        10,
        20,
        30,
        &[feature_xrp_fees()],
    ));

    let items = constructor_ledger_items(&[account_root.clone(), amendments.clone(), fees.clone()]);

    assert_eq!(items[0].0, account_root_key(genesis_account_id()));
    assert_eq!(items[1].0, amendments_key());
    assert_eq!(items[2].0, fees_key());
    assert_eq!(
        decode_constructor_account_root_entry(&items[0].1),
        Ok(ConstructorAccountRootEntry {
            sequence: 1,
            balance_drops: 100_000_000_000_000_000,
            account_id: genesis_account_id(),
        })
    );
    assert_eq!(
        decode_constructor_amendments_entry(&items[1].1),
        Ok(ConstructorAmendmentsEntry {
            amendments: vec![feature_xrp_fees(), sample_uint256(0xCD)],
        })
    );
    assert_eq!(
        decode_constructor_fee_settings_entry(&items[2].1),
        Ok(ConstructorFeeSettingsEntry::XrpDrops {
            base_fee_drops: 10,
            reserve_base_drops: 20,
            reserve_increment_drops: 30,
        })
    );
}

#[test]
fn constructor_item_key_helper_matches_direct_item_encoding() {
    let entry = ConstructorLedgerEntry::FeeSettings(ConstructorFeeSettingsEntry::Legacy {
        base_fee: 10,
        reference_fee_units: protocol::REFERENCE_FEE_UNITS_DEPRECATED,
        reserve_base: Some(20),
        reserve_increment: Some(30),
    });

    let key = constructor_ledger_entry_key(&entry);
    let (item_key, payload) = constructor_ledger_item(&entry);

    assert_eq!(key, fees_key());
    assert_eq!(item_key, key);
    assert_eq!(
        decode_constructor_fee_settings_entry(&payload),
        Ok(ConstructorFeeSettingsEntry::Legacy {
            base_fee: 10,
            reference_fee_units: protocol::REFERENCE_FEE_UNITS_DEPRECATED,
            reserve_base: Some(20),
            reserve_increment: Some(30),
        })
    );
}
