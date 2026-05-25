use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use ledger::{
    Fees, Ledger, LedgerConfig, LedgerHeader, amendments_key, get_enabled_amendments,
    get_majority_amendments, get_next_ledger_time_resolution, is_flag_ledger, is_voting_ledger,
    round_close_time,
};
use protocol::{
    FeatureSet, LedgerEntryType, STArray, STLedgerEntry, STObject, STVector256, feature_xrp_fees,
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

fn typed_amendments_entry_bytes(
    amendments: &[Uint256],
    majority: Uint256,
    close_time: u32,
) -> Vec<u8> {
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::Amendments, amendments_key());
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0xF1));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 901);
    entry.set_field_v256(
        get_field_by_symbol("sfAmendments"),
        STVector256::from_values(get_field_by_symbol("sfAmendments"), amendments.to_vec()),
    );

    let mut majorities = STArray::new(get_field_by_symbol("sfMajorities"));
    let mut majority_entry = STObject::new(get_field_by_symbol("sfMajority"));
    majority_entry.set_field_h256(get_field_by_symbol("sfAmendment"), majority);
    majority_entry.set_field_u32(get_field_by_symbol("sfCloseTime"), close_time);
    majorities.push_back(majority_entry);
    entry.set_field_array(get_field_by_symbol("sfMajorities"), majorities);

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

fn encode_u32_field(field_name: u8, value: u32) -> Vec<u8> {
    let mut bytes = encode_field_id(2, field_name);
    bytes.extend_from_slice(&value.to_be_bytes());
    bytes
}

#[test]
fn timing_helpers_match_current_cpp_examples() {
    assert_eq!(get_next_ledger_time_resolution(30, true, 8), 20);
    assert_eq!(get_next_ledger_time_resolution(20, true, 16), 10);
    assert_eq!(get_next_ledger_time_resolution(10, true, 24), 10);
    assert_eq!(get_next_ledger_time_resolution(30, false, 1), 60);
    assert_eq!(get_next_ledger_time_resolution(60, false, 2), 90);
    assert_eq!(get_next_ledger_time_resolution(120, false, 3), 120);

    assert_eq!(round_close_time(0, 30), 0);
    assert_eq!(round_close_time(29, 60), 0);
    assert_eq!(round_close_time(30, 1), 30);
    assert_eq!(round_close_time(31, 60), 60);
    assert_eq!(round_close_time(30, 60), 60);
    assert_eq!(round_close_time(59, 60), 60);
    assert_eq!(round_close_time(60, 60), 60);
    assert_eq!(round_close_time(61, 60), 60);
}

#[test]
fn amendment_helpers_match_current_cpp_enabled_and_majority_rules() {
    let enabled_one = feature_xrp_fees();
    let enabled_two = sample_uint256(0xE1);
    let majority = sample_uint256(0xA6);
    let state_map = build_state_map_with_items(
        &[(
            amendments_key(),
            typed_amendments_entry_bytes(&[enabled_one, enabled_two], majority, 999),
        )],
        false,
        1,
    );
    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq: 1,
            ..LedgerHeader::default()
        },
        state_map,
        SyncTree::new_with_type(SHAMapType::Transaction, false, 1),
    );
    let _ = ledger.setup_from_state_map_with_config(&sample_ledger_config([]));

    let expected_enabled = [enabled_one, enabled_two].into_iter().collect();
    assert_eq!(get_enabled_amendments(&ledger), expected_enabled);
    assert_eq!(ledger.get_enabled_amendments(), expected_enabled);

    let expected_majorities = [(majority, NetClockTimePoint::from(999))]
        .into_iter()
        .collect();
    assert_eq!(get_majority_amendments(&ledger), expected_majorities);
    assert_eq!(ledger.get_majority_amendments(), expected_majorities);
}

#[test]
fn amendment_helpers_return_empty_when_amendments_entry_is_missing_or_empty() {
    let missing_entry = Ledger::from_maps(
        LedgerHeader {
            seq: 1,
            ..LedgerHeader::default()
        },
        build_state_map_with_items(&[], false, 1),
        SyncTree::new_with_type(SHAMapType::Transaction, false, 1),
    );
    assert!(get_enabled_amendments(&missing_entry).is_empty());
    assert!(get_majority_amendments(&missing_entry).is_empty());

    let mut empty_amendments_payload = Vec::new();
    empty_amendments_payload.extend_from_slice(&encode_field_id(1, 1));
    empty_amendments_payload.extend_from_slice(&0x0066u16.to_be_bytes());
    empty_amendments_payload.extend_from_slice(&encode_u32_field(4, 1));
    empty_amendments_payload.extend_from_slice(&encode_u32_field(5, 2));
    empty_amendments_payload.extend_from_slice(&encode_u32_field(6, 3));
    empty_amendments_payload.push(OBJECT_END);
    let empty_entry = Ledger::from_maps(
        LedgerHeader {
            seq: 1,
            ..LedgerHeader::default()
        },
        build_state_map_with_items(&[(amendments_key(), empty_amendments_payload)], false, 1),
        SyncTree::new_with_type(SHAMapType::Transaction, false, 1),
    );
    assert!(get_enabled_amendments(&empty_entry).is_empty());
    assert!(get_majority_amendments(&empty_entry).is_empty());
}

#[test]
fn flag_and_voting_ledger_helpers_match_current_cpp_interval_rules() {
    assert!(is_flag_ledger(256));
    assert!(!is_flag_ledger(255));
    assert!(is_voting_ledger(256));
    assert!(!is_voting_ledger(255));

    let flag_ledger = Ledger::new(
        LedgerHeader {
            seq: 256,
            ..LedgerHeader::default()
        },
        false,
    );
    assert!(flag_ledger.is_flag_ledger());
    assert!(!flag_ledger.is_voting_ledger());

    let voting_ledger = Ledger::new(
        LedgerHeader {
            seq: 255,
            ..LedgerHeader::default()
        },
        false,
    );
    assert!(voting_ledger.is_voting_ledger());
    assert!(!voting_ledger.is_flag_ledger());
}
