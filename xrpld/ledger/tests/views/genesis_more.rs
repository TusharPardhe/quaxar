use basics::base_uint::{Uint160, Uint256};
use ledger::{
    Fees, INITIAL_XRP_DROPS, LEDGER_GENESIS_TIME_RESOLUTION, Ledger, LedgerConfig,
    account_root_key, amendments_key, build_genesis_master_account_root_item,
    build_genesis_setup_items, build_genesis_state_items, fees_key,
};
use protocol::{
    ConstructorAccountRootEntry, ConstructorAmendmentsEntry, ConstructorFeeSettingsEntry,
    FeatureSet, decode_constructor_account_root_entry, decode_constructor_amendments_entry,
    decode_constructor_fee_settings_entry, encode_constructor_account_root_entry, feature_xrp_fees,
    genesis_account_id,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

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

const OBJECT_END: u8 = 0xE1;

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

#[test]
fn build_genesis_setup_items_uses_legacy_fee_object_without_xrp_fees_amendment() {
    let amendment = sample_uint256(0xD1);
    let config = sample_ledger_config([sample_uint256(0xD2)]);
    let mut expected_fee_bytes = Vec::new();
    expected_fee_bytes.extend_from_slice(&encode_u16_field(1, 0x0073));
    expected_fee_bytes.extend_from_slice(&encode_u64_field(5, 10));
    expected_fee_bytes.extend_from_slice(&encode_u32_field(30, 10));
    expected_fee_bytes.extend_from_slice(&encode_u32_field(31, 20));
    expected_fee_bytes.extend_from_slice(&encode_u32_field(32, 30));
    expected_fee_bytes.push(OBJECT_END);

    let items = build_genesis_setup_items(&config, [amendment]);

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].0, amendments_key());
    assert_eq!(items[0].1, protocol::encode_amendments_entry(&[amendment]));
    assert_eq!(items[1], (fees_key(), expected_fee_bytes));
}

#[test]
fn build_genesis_setup_items_uses_xrp_fee_object_when_amendment_enables_it() {
    let config = sample_ledger_config([sample_uint256(0xD3)]);

    let items = build_genesis_setup_items(&config, [feature_xrp_fees()]);

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].0, amendments_key());
    assert_eq!(
        items[0].1,
        protocol::encode_amendments_entry(&[feature_xrp_fees()])
    );
    assert_eq!(
        items[1],
        (
            fees_key(),
            protocol::encode_fee_settings_entry(
                config.fees.base,
                config.fees.reserve,
                config.fees.increment,
                true,
            ),
        )
    );
}

#[test]
fn build_genesis_setup_items_omits_amendments_entry_when_list_is_empty() {
    let config = sample_ledger_config([sample_uint256(0xD3)]);

    let items = build_genesis_setup_items(&config, std::iter::empty::<Uint256>());

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].0, fees_key());
    assert_eq!(
        decode_constructor_fee_settings_entry(&items[0].1)
            .expect("genesis setup fees entry should decode"),
        ConstructorFeeSettingsEntry::Legacy {
            base_fee: config.fees.base,
            reference_fee_units: 10,
            reserve_base: Some(
                u32::try_from(config.fees.reserve).expect("fee reserve should fit in u32"),
            ),
            reserve_increment: Some(
                u32::try_from(config.fees.increment).expect("fee increment should fit in u32"),
            ),
        }
    );
}

#[test]
fn build_genesis_setup_items_round_trip_through_setup_from_state_map() {
    let amendment = sample_uint256(0xD4);
    let config = sample_ledger_config([sample_uint256(0xD5), feature_xrp_fees()]);
    let items = build_genesis_setup_items(&config, [feature_xrp_fees(), amendment]);
    let mut ledger = Ledger::new(
        ledger::LedgerHeader {
            seq: 1,
            ..ledger::LedgerHeader::default()
        },
        false,
    );
    *ledger.state_map_mut() = build_state_map_with_items(&items);
    ledger.apply_default_fees(config.fees);

    let loaded = ledger
        .setup_from_state_map(&feature_xrp_fees())
        .expect("genesis setup entries should decode through current ledger setup path");

    assert!(loaded);
    assert!(ledger.rules().enabled(&feature_xrp_fees()));
    assert!(ledger.rules().enabled(&amendment));
    assert_eq!(ledger.fees(), config.fees);
}

#[test]
fn create_genesis_setup_only_matches_current_ctor_surface_except_master_account_state() {
    let config = sample_ledger_config([sample_uint256(0xD6)]);
    let ledger = Ledger::create_genesis_setup_only(false, &config, [feature_xrp_fees()])
        .expect("setup-only genesis constructor should insert singleton state entries");

    assert!(ledger.is_immutable());
    assert_eq!(ledger.header().seq, 1);
    assert_eq!(ledger.header().drops, INITIAL_XRP_DROPS);
    assert_eq!(
        ledger.header().close_time_resolution,
        LEDGER_GENESIS_TIME_RESOLUTION
    );
    assert_eq!(ledger.fees(), config.fees);
    assert!(ledger.rules().enabled(&feature_xrp_fees()));
    let (amendments, _amendments_hash) = ledger
        .state_map()
        .peek_item_with_hash(amendments_key(), &mut |_| None)
        .expect("genesis setup-only constructor should keep state map readable")
        .expect("genesis setup-only constructor should insert amendments entry");
    assert_eq!(
        decode_constructor_amendments_entry(amendments.data())
            .expect("genesis setup-only constructor should keep amendments decodable"),
        ConstructorAmendmentsEntry {
            amendments: vec![feature_xrp_fees()],
        }
    );
    let (fees, _fees_hash) = ledger
        .state_map()
        .peek_item_with_hash(fees_key(), &mut |_| None)
        .expect("genesis setup-only constructor should keep state map readable")
        .expect("genesis setup-only constructor should insert fees entry");
    assert_eq!(
        decode_constructor_fee_settings_entry(fees.data())
            .expect("genesis setup-only constructor should keep fees decodable"),
        ConstructorFeeSettingsEntry::XrpDrops {
            base_fee_drops: config.fees.base,
            reserve_base_drops: config.fees.reserve,
            reserve_increment_drops: config.fees.increment,
        }
    );
    assert!(
        ledger
            .state_map()
            .peek_item_with_hash(account_root_key(genesis_account_id()), &mut |_| None)
            .expect("genesis setup-only constructor should keep state map readable")
            .is_none()
    );
}

#[test]
fn create_genesis_setup_only_omits_amendments_entry_when_none_are_supplied() {
    let config = sample_ledger_config([]);
    let ledger = Ledger::create_genesis_setup_only(false, &config, [])
        .expect("setup-only genesis constructor should insert the fees singleton only");

    assert!(ledger.is_immutable());
    assert_eq!(ledger.header().seq, 1);
    assert_eq!(ledger.header().drops, INITIAL_XRP_DROPS);
    assert_eq!(
        ledger.header().close_time_resolution,
        LEDGER_GENESIS_TIME_RESOLUTION
    );
    assert_eq!(ledger.fees(), config.fees);
    assert!(!ledger.rules().enabled(&feature_xrp_fees()));
    assert!(
        ledger
            .state_map()
            .peek_item_with_hash(amendments_key(), &mut |_| None)
            .expect("genesis setup-only constructor should keep state map readable")
            .is_none()
    );
    let (fees, _fees_hash) = ledger
        .state_map()
        .peek_item_with_hash(fees_key(), &mut |_| None)
        .expect("genesis setup-only constructor should keep state map readable")
        .expect("genesis setup-only constructor should insert fees entry");
    assert_eq!(
        decode_constructor_fee_settings_entry(fees.data())
            .expect("genesis setup-only constructor should keep fees decodable"),
        ConstructorFeeSettingsEntry::Legacy {
            base_fee: config.fees.base,
            reference_fee_units: 10,
            reserve_base: Some(
                u32::try_from(config.fees.reserve).expect("fee reserve should fit in u32"),
            ),
            reserve_increment: Some(
                u32::try_from(config.fees.increment).expect("fee increment should fit in u32"),
            ),
        }
    );
}

#[test]
fn create_genesis_setup_only_uses_legacy_fee_shape_without_xrp_fees_amendment() {
    let amendment = sample_uint256(0xDA);
    let config = sample_ledger_config([sample_uint256(0xDB)]);
    let ledger = Ledger::create_genesis_setup_only(false, &config, [amendment])
        .expect("setup-only genesis constructor should insert singleton state entries");

    let (amendments, _amendments_hash) = ledger
        .state_map()
        .peek_item_with_hash(amendments_key(), &mut |_| None)
        .expect("legacy genesis setup-only constructor should keep state map readable")
        .expect("legacy genesis setup-only constructor should insert amendments entry");
    assert_eq!(
        decode_constructor_amendments_entry(amendments.data())
            .expect("legacy genesis setup-only constructor should keep amendments decodable"),
        ConstructorAmendmentsEntry {
            amendments: vec![amendment],
        }
    );

    let (fees, _fees_hash) = ledger
        .state_map()
        .peek_item_with_hash(fees_key(), &mut |_| None)
        .expect("legacy genesis setup-only constructor should keep state map readable")
        .expect("legacy genesis setup-only constructor should insert fees entry");
    assert_eq!(
        decode_constructor_fee_settings_entry(fees.data())
            .expect("legacy genesis setup-only constructor should keep fees decodable"),
        ConstructorFeeSettingsEntry::Legacy {
            base_fee: config.fees.base,
            reference_fee_units: 10,
            reserve_base: Some(20),
            reserve_increment: Some(30),
        }
    );
}

#[test]
fn build_genesis_master_account_root_item_matches_current_cpp_constants() {
    let account_id = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
        .expect("expected genesis master account id should parse");
    let expected_key = account_root_key(account_id);
    let expected_payload = encode_constructor_account_root_entry(ConstructorAccountRootEntry {
        sequence: 1,
        balance_drops: INITIAL_XRP_DROPS,
        account_id,
    });

    let (key, payload) = build_genesis_master_account_root_item(INITIAL_XRP_DROPS);

    assert_eq!(key, expected_key);
    assert_eq!(payload, expected_payload);
}

#[test]
fn build_genesis_state_items_puts_master_account_before_singletons() {
    let amendment = sample_uint256(0xD7);
    let config = sample_ledger_config([sample_uint256(0xD8)]);

    let items =
        build_genesis_state_items(&config, [feature_xrp_fees(), amendment], INITIAL_XRP_DROPS);

    assert_eq!(items.len(), 3);
    assert_eq!(items[0].0, account_root_key(genesis_account_id()));
    assert_eq!(items[1].0, amendments_key());
    assert_eq!(items[2].0, fees_key());
}

#[test]
fn create_genesis_matches_current_ctor_surface_for_ported_objects() {
    let preset = sample_uint256(0xD9);
    let config = sample_ledger_config([preset]);
    let ledger = Ledger::create_genesis(false, &config, [feature_xrp_fees()])
        .expect("genesis constructor should insert the genesis objects");

    assert!(ledger.is_immutable());
    assert_eq!(ledger.header().seq, 1);
    assert_eq!(ledger.header().drops, INITIAL_XRP_DROPS);
    assert_eq!(
        ledger.header().close_time_resolution,
        LEDGER_GENESIS_TIME_RESOLUTION
    );
    assert_eq!(ledger.fees(), config.fees);
    assert!(ledger.rules().enabled(&preset));
    assert!(ledger.rules().enabled(&feature_xrp_fees()));
    let (master_account, _master_account_hash) = ledger
        .state_map()
        .peek_item_with_hash(account_root_key(genesis_account_id()), &mut |_| None)
        .expect("genesis constructor should keep master account root readable")
        .expect("genesis constructor should insert the master account root");
    let decoded_master_account = decode_constructor_account_root_entry(master_account.data())
        .expect("genesis constructor should keep the master account root decodable");
    assert_eq!(
        decoded_master_account,
        ConstructorAccountRootEntry {
            sequence: 1,
            balance_drops: INITIAL_XRP_DROPS,
            account_id: Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
                .expect("expected genesis master account id should parse"),
        }
    );
    let (amendments, _amendments_hash) = ledger
        .state_map()
        .peek_item_with_hash(amendments_key(), &mut |_| None)
        .expect("genesis constructor should keep amendments entry readable")
        .expect("genesis constructor should insert amendments entry");
    assert_eq!(
        decode_constructor_amendments_entry(amendments.data())
            .expect("genesis constructor should keep amendments decodable"),
        ConstructorAmendmentsEntry {
            amendments: vec![feature_xrp_fees()],
        }
    );
    let (fees, _fees_hash) = ledger
        .state_map()
        .peek_item_with_hash(fees_key(), &mut |_| None)
        .expect("genesis constructor should keep fees entry readable")
        .expect("genesis constructor should insert fees entry");
    assert_eq!(
        decode_constructor_fee_settings_entry(fees.data())
            .expect("genesis constructor should keep fees decodable"),
        ConstructorFeeSettingsEntry::XrpDrops {
            base_fee_drops: config.fees.base,
            reserve_base_drops: config.fees.reserve,
            reserve_increment_drops: config.fees.increment,
        }
    );
}

#[test]
fn create_genesis_omits_amendments_entry_when_none_are_supplied() {
    let config = sample_ledger_config([]);
    let ledger = Ledger::create_genesis(false, &config, [])
        .expect("genesis constructor should insert the current state singleton set");

    assert!(ledger.is_immutable());
    assert_eq!(ledger.header().seq, 1);
    assert_eq!(ledger.header().drops, INITIAL_XRP_DROPS);
    assert_eq!(
        ledger.header().close_time_resolution,
        LEDGER_GENESIS_TIME_RESOLUTION
    );
    assert_eq!(ledger.fees(), config.fees);
    assert!(!ledger.rules().enabled(&feature_xrp_fees()));

    let (master_account, _master_account_hash) = ledger
        .state_map()
        .peek_item_with_hash(account_root_key(genesis_account_id()), &mut |_| None)
        .expect("genesis constructor should keep master account root readable")
        .expect("genesis constructor should insert the master account root");
    assert_eq!(
        decode_constructor_account_root_entry(master_account.data())
            .expect("genesis constructor should keep the master account root decodable"),
        ConstructorAccountRootEntry {
            sequence: 1,
            balance_drops: INITIAL_XRP_DROPS,
            account_id: Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
                .expect("expected genesis master account id should parse"),
        }
    );
    assert!(
        ledger
            .state_map()
            .peek_item_with_hash(amendments_key(), &mut |_| None)
            .expect("genesis constructor should keep state map readable")
            .is_none()
    );
    let (fees, _fees_hash) = ledger
        .state_map()
        .peek_item_with_hash(fees_key(), &mut |_| None)
        .expect("genesis constructor should keep fees entry readable")
        .expect("genesis constructor should insert fees entry");
    assert_eq!(
        decode_constructor_fee_settings_entry(fees.data())
            .expect("genesis constructor should keep fees decodable"),
        ConstructorFeeSettingsEntry::Legacy {
            base_fee: config.fees.base,
            reference_fee_units: 10,
            reserve_base: Some(
                u32::try_from(config.fees.reserve).expect("fee reserve should fit in u32"),
            ),
            reserve_increment: Some(
                u32::try_from(config.fees.increment).expect("fee increment should fit in u32"),
            ),
        }
    );
}
