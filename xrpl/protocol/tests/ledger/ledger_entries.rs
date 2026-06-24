//! Integration tests for the typed ledger-entry dispatch surface.

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    ConstructorAccountRootEntry, ConstructorAmendmentsEntry, ConstructorFeeSettingsEntry,
    ConstructorLedgerEntry, ConstructorLedgerEntryDecodeError, DecodedLedgerEntry,
    DecodedLedgerHashesEntry, PortedLedgerEntryDecodeError, account_root_key, amendments_key,
    build_genesis_setup_constructor_entries, build_genesis_state_constructor_entries,
    constructor_ledger_entry_key, constructor_ledger_item, constructor_ledger_items,
    decode_constructor_fee_settings_entry, decode_constructor_ledger_entry,
    decode_ledger_entry_type_code, decode_ported_ledger_entry, encode_account_root_entry,
    encode_constructor_ledger_entry, encode_fee_settings_entry, encode_ledger_hashes_entry,
    feature_xrp_fees, fees_key, genesis_account_id, make_constructor_fee_settings_entry,
};

#[test]
fn typed_ported_decode_dispatches_account_root() {
    let payload = encode_account_root_entry(Uint160::from_u64(7), 9, 11);

    let decoded = decode_ported_ledger_entry(&payload).expect("account root should decode");

    assert_eq!(decoded.entry_type(), protocol::LedgerEntryType::AccountRoot);
    match decoded {
        DecodedLedgerEntry::AccountRoot(entry) => {
            assert_eq!(entry.account_id, Some(Uint160::from_u64(7)));
            assert_eq!(entry.sequence, Some(9));
            assert_eq!(entry.balance.expect("balance should exist").drops, 11);
        }
        other => panic!("unexpected variant: {other:?}"),
    }
}

#[test]
fn typed_ported_decode_dispatches_ledger_hashes() {
    let payload = encode_ledger_hashes_entry(&DecodedLedgerHashesEntry {
        hashes: vec![Uint256::from_u64(1), Uint256::from_u64(2)],
        last_ledger_sequence: Some(99),
    });

    let decoded = decode_ported_ledger_entry(&payload).expect("ledger hashes should decode");

    assert_eq!(
        decoded.entry_type(),
        protocol::LedgerEntryType::LedgerHashes
    );
    match decoded {
        DecodedLedgerEntry::LedgerHashes(entry) => {
            assert_eq!(
                entry.hashes,
                vec![Uint256::from_u64(1), Uint256::from_u64(2)]
            );
            assert_eq!(entry.last_ledger_sequence, Some(99));
        }
        other => panic!("unexpected variant: {other:?}"),
    }
}

#[test]
fn typed_constructor_decode_dispatches_fee_settings() {
    let payload = encode_fee_settings_entry(10, 20, 30, true);

    let decoded =
        decode_constructor_ledger_entry(&payload).expect("constructor fee settings should decode");

    assert_eq!(decoded.entry_type(), protocol::LedgerEntryType::FeeSettings);
    assert_eq!(
        decoded,
        ConstructorLedgerEntry::FeeSettings(ConstructorFeeSettingsEntry::XrpDrops {
            base_fee_drops: 10,
            reserve_base_drops: 20,
            reserve_increment_drops: 30,
        })
    );
}

#[test]
fn typed_constructor_encode_round_trips_amendments() {
    let entry = ConstructorLedgerEntry::Amendments(ConstructorAmendmentsEntry {
        amendments: vec![Uint256::from_u64(5), Uint256::from_u64(6)],
    });

    let encoded = encode_constructor_ledger_entry(&entry);
    let decoded = decode_constructor_ledger_entry(&encoded).expect("amendments should decode");

    assert_eq!(decoded, entry);
}

#[test]
fn typed_decode_reports_unsupported_types() {
    let payload = vec![0x11, 0x12, 0x34, 0xE1];

    assert_eq!(
        decode_ported_ledger_entry(&payload),
        Err(PortedLedgerEntryDecodeError::UnsupportedLedgerEntryType(
            0x1234
        ))
    );
    assert_eq!(
        decode_constructor_ledger_entry(&payload),
        Err(ConstructorLedgerEntryDecodeError::UnsupportedLedgerEntryType(0x1234))
    );
}

#[test]
fn typed_entry_type_decode_reads_current_field_code() {
    let payload = encode_constructor_ledger_entry(&ConstructorLedgerEntry::AccountRoot(
        ConstructorAccountRootEntry {
            sequence: 1,
            balance_drops: 2,
            account_id: Uint160::from_u64(3),
        },
    ));

    assert_eq!(decode_ledger_entry_type_code(&payload), Ok(0x0061));
}

#[test]
fn typed_constructor_key_helper_matches_current_genesis_keys() {
    assert_eq!(
        constructor_ledger_entry_key(&ConstructorLedgerEntry::AccountRoot(
            ConstructorAccountRootEntry {
                sequence: 1,
                balance_drops: 100,
                account_id: genesis_account_id(),
            }
        )),
        account_root_key(genesis_account_id())
    );
    assert_eq!(
        constructor_ledger_entry_key(&ConstructorLedgerEntry::Amendments(
            ConstructorAmendmentsEntry {
                amendments: vec![Uint256::from_u64(1)],
            }
        )),
        amendments_key()
    );
    assert_eq!(
        constructor_ledger_entry_key(&ConstructorLedgerEntry::FeeSettings(
            ConstructorFeeSettingsEntry::Legacy {
                base_fee: 10,
                reference_fee_units: 10,
                reserve_base: Some(20),
                reserve_increment: Some(30),
            }
        )),
        fees_key()
    );
}

#[test]
fn typed_constructor_item_builds_current_genesis_fee_singleton() {
    let entry = ConstructorLedgerEntry::FeeSettings(make_constructor_fee_settings_entry(
        10,
        20,
        30,
        &[feature_xrp_fees()],
    ));

    let (key, payload) = constructor_ledger_item(&entry);

    assert_eq!(key, fees_key());
    assert_eq!(
        decode_constructor_fee_settings_entry(&payload),
        Ok(ConstructorFeeSettingsEntry::Legacy {
            base_fee: 10,
            reference_fee_units: REFERENCE_FEE_UNITS_DEPRECATED,
            reserve_base: Some(20),
            reserve_increment: Some(30),
        })
    );
}

#[test]
fn typed_constructor_item_builds_current_genesis_legacy_fee_singleton() {
    let entry =
        ConstructorLedgerEntry::FeeSettings(make_constructor_fee_settings_entry(10, 20, 30, &[]));

    let (key, payload) = constructor_ledger_item(&entry);

    assert_eq!(key, fees_key());
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

#[test]
fn typed_constructor_item_builds_current_genesis_account_root() {
    let entry = ConstructorLedgerEntry::AccountRoot(ConstructorAccountRootEntry {
        sequence: 1,
        balance_drops: 100_000_000_000_000_000,
        account_id: genesis_account_id(),
    });

    let (key, payload) = constructor_ledger_item(&entry);

    assert_eq!(key, account_root_key(genesis_account_id()));
    assert_eq!(decode_constructor_ledger_entry(&payload), Ok(entry));
}

#[test]
fn legacy_constructor_fee_builder_omits_overflowing_reserve_fields() {
    let entry = make_constructor_fee_settings_entry(10, u64::from(u32::MAX) + 1, 30, &[]);

    assert_eq!(
        entry,
        ConstructorFeeSettingsEntry::Legacy {
            base_fee: 10,
            reference_fee_units: 10,
            reserve_base: None,
            reserve_increment: Some(30),
        }
    );
}

#[test]
fn genesis_setup_constructor_entries_keep_current_singleton_order_and_omission_rules() {
    let entries = build_genesis_setup_constructor_entries(10, 20, 30, &[feature_xrp_fees()]);

    assert_eq!(entries.len(), 2);
    assert!(matches!(entries[0], ConstructorLedgerEntry::Amendments(_)));
    assert!(matches!(entries[1], ConstructorLedgerEntry::FeeSettings(_)));

    let no_amendments = build_genesis_setup_constructor_entries(10, 20, 30, &[]);
    assert_eq!(no_amendments.len(), 1);
    assert!(matches!(
        no_amendments[0],
        ConstructorLedgerEntry::FeeSettings(_)
    ));
}

#[test]
fn genesis_state_constructor_entries_keep_master_account_first() {
    let entries = build_genesis_state_constructor_entries(
        100_000_000_000_000_000,
        10,
        20,
        30,
        &[feature_xrp_fees(), Uint256::from_u64(7)],
    );

    assert_eq!(entries.len(), 3);
    assert!(matches!(entries[0], ConstructorLedgerEntry::AccountRoot(_)));
    assert!(matches!(entries[1], ConstructorLedgerEntry::Amendments(_)));
    assert!(matches!(entries[2], ConstructorLedgerEntry::FeeSettings(_)));

    let items = constructor_ledger_items(&entries);
    assert_eq!(items[0].0, account_root_key(genesis_account_id()));
    assert_eq!(items[1].0, amendments_key());
    assert_eq!(items[2].0, fees_key());
}
