use basics::base_uint::Uint256;
use basics::chrono::{NetClockTimePoint, to_string, to_string_iso};
use basics::sha_map_hash::SHAMapHash;
use basics::str_hex::str_hex;
use ledger::{
    Ledger, LedgerConfig, LedgerFill, LedgerFillOptions, LedgerHeader, SLCF_NO_CONSENSUS_TIME,
    add_json, amendments_key, copy_from, fees_key, get_json, serialize_ledger_header,
};
use protocol::{
    ConstructorLedgerEntry, JsonOptions, JsonValue, LedgerEntryType, STAmount, STLedgerEntry,
    STVector256, SerialIter, StBase, decode_constructor_ledger_entry, feature_xrp_fees,
    get_field_by_symbol,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;
use std::collections::BTreeMap;

fn sample_hash(fill: u8) -> SHAMapHash {
    SHAMapHash::new(Uint256::from_array([fill; 32]))
}

fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn build_state_map_with_items(
    items: &[(Uint256, Vec<u8>)],
    backed: bool,
    ledger_seq: u32,
) -> SyncTree {
    let mut tree = MutableTree::new(1);
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
        backed,
        ledger_seq,
        SyncState::Immutable,
    )
}

fn typed_amendments_entry_bytes(amendments: &[Uint256]) -> Vec<u8> {
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::Amendments, amendments_key());
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0xA1));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 700);
    entry.set_field_v256(
        get_field_by_symbol("sfAmendments"),
        STVector256::from_values(get_field_by_symbol("sfAmendments"), amendments.to_vec()),
    );
    entry.get_serializer().data().to_vec()
}

fn typed_xrp_fee_settings_entry_bytes(base: u64, reserve: u64, increment: u64) -> Vec<u8> {
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::FeeSettings, fees_key());
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0xA2));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 701);
    entry.set_field_amount(
        get_field_by_symbol("sfBaseFeeDrops"),
        STAmount::new_native(base, false),
    );
    entry.set_field_amount(
        get_field_by_symbol("sfReserveBaseDrops"),
        STAmount::new_native(reserve, false),
    );
    entry.set_field_amount(
        get_field_by_symbol("sfReserveIncrementDrops"),
        STAmount::new_native(increment, false),
    );
    entry.get_serializer().data().to_vec()
}

fn empty_tx_map(ledger_seq: u32) -> SyncTree {
    SyncTree::new_with_type(SHAMapType::Transaction, false, ledger_seq)
}

fn build_ledger(header: LedgerHeader, items: &[(Uint256, Vec<u8>)]) -> Ledger {
    Ledger::from_maps(
        header,
        build_state_map_with_items(items, false, header.seq),
        empty_tx_map(header.seq),
    )
}

fn object(value: JsonValue) -> BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = value else {
        panic!("json value must be an object");
    };
    object
}

fn array(value: &JsonValue) -> &[JsonValue] {
    let JsonValue::Array(values) = value else {
        panic!("json value must be an array");
    };
    values
}

fn parsed_entry_json(key: Uint256, payload: &[u8]) -> JsonValue {
    let mut serial = SerialIter::new(payload);
    STLedgerEntry::from_serial_iter(&mut serial, key).json(JsonOptions::NONE)
}

fn decode_hex_bytes(hex: &str) -> Vec<u8> {
    assert!(
        hex.len().is_multiple_of(2),
        "hex payloads must have an even number of chars"
    );
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16).expect("hex payloads must contain valid bytes")
        })
        .collect()
}

#[test]
fn header_json_closed_full_fields() {
    let header = LedgerHeader {
        seq: 91,
        drops: 98_765,
        hash: sample_hash(0x10),
        parent_hash: sample_hash(0x11),
        tx_hash: sample_hash(0x12),
        account_hash: sample_hash(0x13),
        parent_close_time: 501,
        close_time: 600,
        close_time_resolution: 30,
        close_flags: SLCF_NO_CONSENSUS_TIME,
        ..LedgerHeader::default()
    };
    let ledger = build_ledger(header, &[]);
    let json = object(
        get_json(&LedgerFill::new(&ledger, LedgerFillOptions::FULL).with_closed(true))
            .expect("ledger json should render"),
    );

    assert_eq!(
        json.get("parent_hash"),
        Some(&JsonValue::String(header.parent_hash.to_string()))
    );
    assert_eq!(
        json.get("ledger_index"),
        Some(&JsonValue::Unsigned(u64::from(header.seq)))
    );
    assert_eq!(json.get("closed"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        json.get("ledger_hash"),
        Some(&JsonValue::String(header.hash.to_string()))
    );
    assert_eq!(
        json.get("transaction_hash"),
        Some(&JsonValue::String(header.tx_hash.to_string()))
    );
    assert_eq!(
        json.get("account_hash"),
        Some(&JsonValue::String(header.account_hash.to_string()))
    );
    assert_eq!(
        json.get("total_coins"),
        Some(&JsonValue::String(header.drops.to_string()))
    );
    assert_eq!(
        json.get("close_flags"),
        Some(&JsonValue::Unsigned(u64::from(header.close_flags)))
    );
    assert_eq!(
        json.get("parent_close_time"),
        Some(&JsonValue::Unsigned(u64::from(header.parent_close_time)))
    );
    assert_eq!(
        json.get("close_time"),
        Some(&JsonValue::Unsigned(u64::from(header.close_time)))
    );
    assert_eq!(
        json.get("close_time_resolution"),
        Some(&JsonValue::Unsigned(u64::from(
            header.close_time_resolution
        )))
    );
    assert_eq!(
        json.get("close_time_human"),
        Some(&JsonValue::String(to_string(NetClockTimePoint::from(
            header.close_time,
        ))))
    );
    assert_eq!(
        json.get("close_time_iso"),
        Some(&JsonValue::String(to_string_iso(NetClockTimePoint::from(
            header.close_time,
        ))))
    );
    assert_eq!(
        json.get("close_time_estimated"),
        Some(&JsonValue::Bool(true))
    );
}

#[test]
fn open_non_full_header_stops_after_closed_flag() {
    let header = LedgerHeader {
        seq: 44,
        parent_hash: sample_hash(0x20),
        hash: sample_hash(0x21),
        ..LedgerHeader::default()
    };
    let ledger = build_ledger(header, &[]);
    let json = object(
        get_json(
            &LedgerFill::new(&ledger, LedgerFillOptions::default())
                .with_closed(false)
                .with_api_version(1),
        )
        .expect("ledger json should render"),
    );

    assert_eq!(
        json.get("parent_hash"),
        Some(&JsonValue::String(header.parent_hash.to_string()))
    );
    assert_eq!(
        json.get("ledger_index"),
        Some(&JsonValue::String(header.seq.to_string()))
    );
    assert_eq!(json.get("closed"), Some(&JsonValue::Bool(false)));
    assert!(!json.contains_key("ledger_hash"));
    assert!(!json.contains_key("transaction_hash"));
    assert!(!json.contains_key("accountState"));
}

#[test]
fn binary_branch_uses_raw_unprefixed_header_blob() {
    let header = LedgerHeader {
        seq: 55,
        drops: 123,
        parent_hash: sample_hash(0x30),
        tx_hash: sample_hash(0x31),
        account_hash: sample_hash(0x32),
        parent_close_time: 77,
        close_time: 88,
        close_time_resolution: 30,
        close_flags: 0,
        ..LedgerHeader::default()
    };
    let ledger = build_ledger(header, &[]);
    let json = object(
        get_json(
            &LedgerFill::new(&ledger, LedgerFillOptions::BINARY)
                .with_closed(true)
                .with_api_version(2),
        )
        .expect("binary ledger json should render"),
    );

    assert_eq!(json.get("closed"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        json.get("ledger_data"),
        Some(&JsonValue::String(str_hex(serialize_ledger_header(
            &header, false,
        ))))
    );

    let open_json = object(
        get_json(
            &LedgerFill::new(&ledger, LedgerFillOptions::BINARY)
                .with_closed(false)
                .with_api_version(2),
        )
        .expect("binary open ledger json should render"),
    );
    assert_eq!(open_json.get("closed"), Some(&JsonValue::Bool(false)));
    assert!(!open_json.contains_key("ledger_data"));
}

#[test]
fn state_dump_modes_match_cpp_keys_expanded_and_binary_shapes() {
    let amendments = typed_amendments_entry_bytes(&[feature_xrp_fees(), sample_uint256(0x40)]);
    let fees = typed_xrp_fee_settings_entry_bytes(10, 20, 30);
    let items = vec![
        (amendments_key(), amendments.clone()),
        (fees_key(), fees.clone()),
    ];
    let header = LedgerHeader {
        seq: 77,
        parent_hash: sample_hash(0x41),
        ..LedgerHeader::default()
    };
    let ledger = build_ledger(header, &items);

    let keys_json = object(
        get_json(&LedgerFill::new(&ledger, LedgerFillOptions::DUMP_STATE))
            .expect("state key json should render"),
    );
    let mut actual_keys = array(
        keys_json
            .get("accountState")
            .expect("account state should be present"),
    )
    .iter()
    .map(|value| match value {
        JsonValue::String(key) => key.clone(),
        _ => panic!("key-only state entries must be strings"),
    })
    .collect::<Vec<_>>();
    actual_keys.sort();
    let mut expected_keys = items
        .iter()
        .map(|(key, _payload)| key.to_string())
        .collect::<Vec<_>>();
    expected_keys.sort();
    assert_eq!(actual_keys, expected_keys);

    let expanded_json = object(
        get_json(&LedgerFill::new(
            &ledger,
            LedgerFillOptions::DUMP_STATE | LedgerFillOptions::EXPAND,
        ))
        .expect("expanded state json should render"),
    );
    let mut expanded_by_index = BTreeMap::new();
    for entry in array(
        expanded_json
            .get("accountState")
            .expect("expanded account state should be present"),
    ) {
        let JsonValue::Object(object) = entry else {
            panic!("expanded state entries must be objects");
        };
        let Some(JsonValue::String(index)) = object.get("index") else {
            panic!("expanded state entries must carry an index");
        };
        expanded_by_index.insert(index.clone(), JsonValue::Object(object.clone()));
    }
    for (key, payload) in &items {
        assert_eq!(
            expanded_by_index.get(&key.to_string()),
            Some(&parsed_entry_json(*key, payload))
        );
    }

    let binary_json = object(
        get_json(
            &LedgerFill::new(
                &ledger,
                LedgerFillOptions::DUMP_STATE | LedgerFillOptions::BINARY,
            )
            .with_closed(true),
        )
        .expect("binary state json should render"),
    );
    let mut binary_by_hash = BTreeMap::new();
    for entry in array(
        binary_json
            .get("accountState")
            .expect("binary account state should be present"),
    ) {
        let JsonValue::Object(object) = entry else {
            panic!("binary state entries must be objects");
        };
        let Some(JsonValue::String(hash)) = object.get("hash") else {
            panic!("binary state entries must carry a hash");
        };
        let Some(JsonValue::String(blob)) = object.get("tx_blob") else {
            panic!("binary state entries must carry a tx_blob");
        };
        binary_by_hash.insert(hash.clone(), blob.clone());
    }
    for (key, payload) in &items {
        assert_eq!(
            binary_by_hash.get(&key.to_string()),
            Some(&str_hex(payload))
        );
    }
}

#[test]
fn genesis_state_expansion_includes_account_root_without_panicking() {
    let ledger = Ledger::create_genesis(false, &LedgerConfig::default(), [])
        .expect("genesis ledger should build");
    let json = object(
        get_json(&LedgerFill::new(
            &ledger,
            LedgerFillOptions::DUMP_STATE | LedgerFillOptions::BINARY,
        ))
        .expect("binary genesis state should render"),
    );

    let state = array(
        json.get("accountState")
            .expect("binary genesis state should include accountState"),
    );

    let mut saw_account_root = false;
    let mut saw_fee_settings = false;
    for entry in state {
        let JsonValue::Object(object) = entry else {
            panic!("binary genesis state entries must be objects");
        };
        let Some(JsonValue::String(blob)) = object.get("tx_blob") else {
            panic!("binary genesis state entries must include tx_blob");
        };
        let payload = decode_hex_bytes(blob);
        let decoded = decode_constructor_ledger_entry(&payload)
            .expect("genesis constructor entries should decode through constructor codecs");
        saw_account_root |= matches!(decoded, ConstructorLedgerEntry::AccountRoot(_));
        saw_fee_settings |= matches!(decoded, ConstructorLedgerEntry::FeeSettings(_));
    }

    assert!(saw_account_root);
    assert!(saw_fee_settings);
}

#[test]
fn add_json_wraps_ledger_and_copy_from_merges_objects() {
    let header = LedgerHeader {
        seq: 83,
        parent_hash: sample_hash(0x50),
        ..LedgerHeader::default()
    };
    let ledger = build_ledger(header, &[]);
    let fill = LedgerFill::new(&ledger, LedgerFillOptions::FULL).with_closed(true);

    let mut root = JsonValue::Object(BTreeMap::from([(
        "status".to_owned(),
        JsonValue::String("ok".to_owned()),
    )]));
    add_json(&mut root, &fill).expect("wrapped ledger json should render");
    let object = object(root);
    assert_eq!(
        object.get("status"),
        Some(&JsonValue::String("ok".to_owned()))
    );
    assert!(matches!(object.get("ledger"), Some(JsonValue::Object(_))));

    let mut merged =
        JsonValue::Object(BTreeMap::from([("left".to_owned(), JsonValue::Bool(true))]));
    copy_from(
        &mut merged,
        &JsonValue::Object(BTreeMap::from([(
            "right".to_owned(),
            JsonValue::Unsigned(9),
        )])),
    );
    assert_eq!(
        merged,
        JsonValue::Object(BTreeMap::from([
            ("left".to_owned(), JsonValue::Bool(true)),
            ("right".to_owned(), JsonValue::Unsigned(9)),
        ]))
    );

    let mut assigned = JsonValue::Null;
    let source = JsonValue::Object(BTreeMap::from([(
        "copied".to_owned(),
        JsonValue::String("yes".to_owned()),
    )]));
    copy_from(&mut assigned, &source);
    assert_eq!(assigned, source);
}
