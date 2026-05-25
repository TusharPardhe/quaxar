use basics::base_uint::Uint256;
use ledger::{Fees, Ledger, LedgerConfig, amendments_key, fees_key};
use protocol::{
    FeatureSet, LedgerEntryType, STAmount, STLedgerEntry, STVector256, feature_xrp_fees,
    get_field_by_symbol,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

const OBJECT_END: u8 = 0xE1;

fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn sample_ledger_config(features: impl IntoIterator<Item = Uint256>) -> LedgerConfig {
    LedgerConfig::new(
        Fees {
            base: 10,
            reserve: 20,
            increment: 30,
        },
        FeatureSet::new(features),
    )
}

fn build_state_map_with_items(items: &[(Uint256, Vec<u8>)]) -> SyncTree {
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
        false,
        1,
        SyncState::Immutable,
    )
}

fn typed_amendments_entry_bytes(amendments: &[Uint256]) -> Vec<u8> {
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::Amendments, amendments_key());
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0x9E));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 452);
    entry.set_field_v256(
        get_field_by_symbol("sfAmendments"),
        STVector256::from_values(get_field_by_symbol("sfAmendments"), amendments.to_vec()),
    );
    entry.get_serializer().data().to_vec()
}

fn typed_legacy_fee_settings_entry_bytes(base: u64, reserve: u32, increment: u32) -> Vec<u8> {
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::FeeSettings, fees_key());
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0x9F));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 453);
    entry.set_field_u64(get_field_by_symbol("sfBaseFee"), base);
    entry.set_field_u32(get_field_by_symbol("sfReferenceFeeUnits"), 256);
    entry.set_field_u32(get_field_by_symbol("sfReserveBase"), reserve);
    entry.set_field_u32(get_field_by_symbol("sfReserveIncrement"), increment);
    entry.get_serializer().data().to_vec()
}

fn typed_xrp_fee_settings_entry_bytes(base: u64, reserve: u64, increment: u64) -> Vec<u8> {
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::FeeSettings, fees_key());
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0xA0));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 454);
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

fn encode_field_id(field_type: u8, field_name: u8) -> Vec<u8> {
    if field_type < 16 && field_name < 16 {
        vec![(field_type << 4) | field_name]
    } else if field_type < 16 {
        vec![field_type << 4, field_name]
    } else if field_name < 16 {
        vec![field_name, field_type]
    } else {
        vec![0, field_type, field_name]
    }
}

fn encode_u16_field(field_name: u8, value: u16) -> Vec<u8> {
    let mut bytes = encode_field_id(1, field_name);
    bytes.extend_from_slice(&value.to_be_bytes());
    bytes
}

fn encode_u32_field(field_name: u8, value: u32) -> Vec<u8> {
    let mut bytes = encode_field_id(2, field_name);
    bytes.extend_from_slice(&value.to_be_bytes());
    bytes
}

fn encode_u64_field(field_name: u8, value: u64) -> Vec<u8> {
    let mut bytes = encode_field_id(3, field_name);
    bytes.extend_from_slice(&value.to_be_bytes());
    bytes
}

fn encode_native_amount_field(field_name: u8, value: u64) -> Vec<u8> {
    let mut bytes = encode_field_id(6, field_name);
    bytes.extend_from_slice(&(value | 0x4000_0000_0000_0000).to_be_bytes());
    bytes
}

fn mixed_fee_settings_entry_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&encode_u16_field(1, 0x0073));
    bytes.extend_from_slice(&encode_u64_field(5, 10));
    bytes.extend_from_slice(&encode_u32_field(30, 256));
    bytes.extend_from_slice(&encode_u32_field(31, 20));
    bytes.extend_from_slice(&encode_u32_field(32, 30));
    bytes.extend_from_slice(&encode_native_amount_field(22, 11));
    bytes.extend_from_slice(&encode_native_amount_field(23, 21));
    bytes.extend_from_slice(&encode_native_amount_field(24, 31));
    bytes.push(OBJECT_END);
    bytes
}

#[test]
fn setup_from_state_map_with_config_applies_legacy_fees_without_xrp_amendment() {
    let config = sample_ledger_config([sample_uint256(0xA1)]);
    let items = vec![
        (
            amendments_key(),
            typed_amendments_entry_bytes(&[sample_uint256(0xA2)]),
        ),
        (
            fees_key(),
            typed_legacy_fee_settings_entry_bytes(10, 20, 30),
        ),
    ];
    let mut ledger = Ledger::new(Default::default(), false);
    *ledger.state_map_mut() = build_state_map_with_items(&items);

    let loaded = ledger
        .setup_from_state_map_with_config(&config)
        .expect("legacy setup should decode");

    assert!(loaded);
    assert_eq!(ledger.fees(), config.fees);
    assert!(ledger.rules().enabled(&sample_uint256(0xA1)));
    assert!(ledger.rules().enabled(&sample_uint256(0xA2)));
    assert!(!ledger.rules().enabled(&feature_xrp_fees()));
}

#[test]
fn setup_from_state_map_with_config_applies_xrp_fees_from_typed_singletons() {
    let config = sample_ledger_config([sample_uint256(0xA3)]);
    let items = vec![
        (
            amendments_key(),
            typed_amendments_entry_bytes(&[feature_xrp_fees(), sample_uint256(0xA4)]),
        ),
        (fees_key(), typed_xrp_fee_settings_entry_bytes(11, 21, 31)),
    ];
    let mut ledger = Ledger::new(Default::default(), false);
    *ledger.state_map_mut() = build_state_map_with_items(&items);
    ledger.set_rules(protocol::Rules::new([sample_uint256(0xA3)]));

    let loaded = ledger
        .setup_from_state_map_with_config(&config)
        .expect("typed xrp singleton setup should decode");

    assert!(loaded);
    assert_eq!(
        ledger.fees(),
        Fees {
            base: 11,
            reserve: 21,
            increment: 31,
        }
    );
    assert!(ledger.rules().enabled(&sample_uint256(0xA3)));
    assert!(ledger.rules().enabled(&feature_xrp_fees()));
    assert!(ledger.rules().enabled(&sample_uint256(0xA4)));
}

#[test]
fn setup_from_state_map_returns_false_for_mixed_legacy_and_xrp_fee_fields() {
    let config = sample_ledger_config([]);
    let items = vec![
        (
            amendments_key(),
            protocol::encode_amendments_entry(&[feature_xrp_fees()]),
        ),
        (fees_key(), mixed_fee_settings_entry_bytes()),
    ];
    let mut ledger = Ledger::new(Default::default(), false);
    *ledger.state_map_mut() = build_state_map_with_items(&items);

    let loaded = ledger
        .setup_from_state_map_with_default_fees(config.fees, &feature_xrp_fees())
        .expect("mixed fee entry should decode through current narrowed port");

    assert!(!loaded);
    assert_eq!(
        ledger.fees(),
        Fees {
            base: 11,
            reserve: 21,
            increment: 31,
        }
    );
}

#[test]
fn setup_from_state_map_returns_false_for_xrp_fees_without_amendment_enabled() {
    let config = sample_ledger_config([sample_uint256(0xB1)]);
    let items = vec![(
        fees_key(),
        protocol::encode_fee_settings_entry(10, 20, 30, true),
    )];
    let mut ledger = Ledger::new(Default::default(), false);
    *ledger.state_map_mut() = build_state_map_with_items(&items);
    ledger.set_rules(protocol::Rules::new([sample_uint256(0xB1)]));

    let loaded = ledger
        .setup_from_state_map_with_default_fees(config.fees, &feature_xrp_fees())
        .expect("xrp fee entry should decode through current narrowed port");

    assert!(!loaded);
    assert!(!ledger.rules().enabled(&feature_xrp_fees()));
    assert_eq!(ledger.fees(), config.fees);
}

#[test]
fn setup_from_state_map_with_config_keeps_presets_when_amendments_object_is_missing() {
    let config = sample_ledger_config([sample_uint256(0xC1)]);
    let items = vec![(
        fees_key(),
        protocol::encode_fee_settings_entry(10, 20, 30, false),
    )];
    let mut ledger = Ledger::new(Default::default(), false);
    *ledger.state_map_mut() = build_state_map_with_items(&items);

    let loaded = ledger
        .setup_from_state_map_with_config(&config)
        .expect("fees-only setup should decode");

    assert!(loaded);
    assert!(ledger.rules().enabled(&sample_uint256(0xC1)));
    assert!(!ledger.rules().enabled(&feature_xrp_fees()));
    assert_eq!(ledger.fees(), config.fees);
}
