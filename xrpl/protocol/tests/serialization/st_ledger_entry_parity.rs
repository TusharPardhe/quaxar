use basics::base_uint::Uint256;
use protocol::{
    JsonValue, LedgerEntryType, Rules, STLedgerEntry, SerializedTypeId, StBase,
    fix_previous_txn_id, get_field_by_symbol, make_mpt_id,
};

#[test]
fn keylet_constructor_sets_type_and_registry_template() {
    let key = Uint256::from_array([0x11; 32]);
    let entry = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, key);

    assert_eq!(entry.stype(), SerializedTypeId::LedgerEntry);
    assert_eq!(entry.key(), &key);
    assert_eq!(entry.get_type(), LedgerEntryType::AccountRoot);
    assert_eq!(
        entry.get_field_u16(get_field_by_symbol("sfLedgerEntryType")),
        LedgerEntryType::AccountRoot as u16
    );
    assert!(
        entry
            .full_text()
            .starts_with(&format!("\"{}\" = {{ AccountRoot, ", key))
    );
    assert!(entry.full_text().contains("LedgerEntryType = AccountRoot"));
}

#[test]
fn threaded_type_matches_fix_previous_txn_id_gate() {
    let fee_settings = STLedgerEntry::from_type_and_key(
        LedgerEntryType::FeeSettings,
        Uint256::from_array([0x22; 32]),
    );
    let account_root = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        Uint256::from_array([0x23; 32]),
    );

    assert!(!fee_settings.is_threaded_type(&Rules::new([])));
    assert!(fee_settings.is_threaded_type(&Rules::new([fix_previous_txn_id()])));
    assert!(account_root.is_threaded_type(&Rules::new([])));
}

#[test]
fn thread_uses_default_previous_values_then_detects_duplicate_thread() {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        Uint256::from_array([0x31; 32]),
    );
    let tx_id = Uint256::from_array([0x44; 32]);
    let mut prev_tx = Uint256::zero();
    let mut prev_ledger = 0;

    assert!(entry.thread(tx_id, 500, &mut prev_tx, &mut prev_ledger));
    assert_eq!(prev_tx, Uint256::zero());
    assert_eq!(prev_ledger, 0);
    assert_eq!(
        entry.get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
        tx_id
    );
    assert_eq!(
        entry.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")),
        500
    );

    prev_tx = Uint256::from_array([0x99; 32]);
    prev_ledger = 99;
    assert!(!entry.thread(tx_id, 500, &mut prev_tx, &mut prev_ledger));
    assert_eq!(prev_tx, Uint256::from_array([0x99; 32]));
    assert_eq!(prev_ledger, 99);
}

#[test]
fn json_adds_index_and_mpt_issuance_id() {
    let key = Uint256::from_array([0x52; 32]);
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::MPTokenIssuance, key);
    let issuer = protocol::calc_account_id(&protocol::genesis_public_key());
    entry.set_field_u32(get_field_by_symbol("sfSequence"), 7);
    entry.set_account_id(get_field_by_symbol("sfIssuer"), issuer);

    let JsonValue::Object(json) = entry.json(protocol::JsonOptions::NONE) else {
        panic!("ledger entry json should be an object");
    };

    assert_eq!(json.get("index"), Some(&JsonValue::String(key.to_string())));
    assert_eq!(
        json.get("mpt_issuance_id"),
        Some(&JsonValue::String(make_mpt_id(7, issuer).to_string()))
    );
}

#[test]
fn serial_round_trip_preserves_key_and_thread_fields() {
    let key = Uint256::from_array([0x61; 32]);
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, key);
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 88);
    entry.set_field_h256(
        get_field_by_symbol("sfPreviousTxnID"),
        Uint256::from_array([0x62; 32]),
    );

    let serializer = entry.get_serializer();
    let mut iter = protocol::SerialIter::new(serializer.data());
    let parsed = STLedgerEntry::from_serial_iter(&mut iter, key);

    assert_eq!(parsed.key(), &key);
    assert_eq!(parsed.get_type(), LedgerEntryType::AccountRoot);
    assert_eq!(
        parsed.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")),
        88
    );
    assert_eq!(
        parsed.get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
        Uint256::from_array([0x62; 32])
    );
}
