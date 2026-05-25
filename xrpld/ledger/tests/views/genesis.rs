use basics::base_uint::Uint256;
use ledger::{
    Fees, INITIAL_XRP_DROPS, LEDGER_GENESIS_TIME_RESOLUTION, Ledger, LedgerConfig, amendments_key,
    build_genesis_setup_items, build_genesis_state_items, fees_key,
};
use protocol::{
    ConstructorAccountRootEntry, decode_constructor_account_root_entry,
    decode_constructor_amendments_entry, decode_constructor_fee_settings_entry, feature_xrp_fees,
    genesis_account_id,
};

fn sample_ledger_config(features: impl IntoIterator<Item = Uint256>) -> LedgerConfig {
    LedgerConfig::new(
        Fees {
            base: 10,
            reserve: 20,
            increment: 30,
        },
        protocol::FeatureSet::new(features),
    )
}

fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

#[test]
fn build_genesis_setup_items_orders_amendments_before_fees_and_omits_empty_amendments() {
    let amendment = sample_uint256(0xA1);
    let config = sample_ledger_config([sample_uint256(0xA2)]);

    let items = build_genesis_setup_items(&config, [feature_xrp_fees(), amendment]);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].0, amendments_key());
    assert_eq!(
        decode_constructor_amendments_entry(&items[0].1)
            .expect("genesis amendments entry should decode")
            .amendments,
        vec![feature_xrp_fees(), amendment]
    );
    assert_eq!(items[1].0, fees_key());
    assert_eq!(
        decode_constructor_fee_settings_entry(&items[1].1)
            .expect("genesis fees entry should decode"),
        protocol::ConstructorFeeSettingsEntry::XrpDrops {
            base_fee_drops: 10,
            reserve_base_drops: 20,
            reserve_increment_drops: 30,
        }
    );

    let empty_items = build_genesis_setup_items(&config, std::iter::empty::<Uint256>());
    assert_eq!(empty_items.len(), 1);
    assert_eq!(empty_items[0].0, fees_key());
}

#[test]
fn build_genesis_state_items_orders_master_account_before_singletons() {
    let amendment = sample_uint256(0xB1);
    let config = sample_ledger_config([sample_uint256(0xB2)]);

    let items =
        build_genesis_state_items(&config, [feature_xrp_fees(), amendment], INITIAL_XRP_DROPS);
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].0, ledger::account_root_key(genesis_account_id()));
    assert_eq!(
        decode_constructor_account_root_entry(&items[0].1)
            .expect("genesis master account should decode"),
        ConstructorAccountRootEntry {
            sequence: 1,
            balance_drops: INITIAL_XRP_DROPS,
            account_id: genesis_account_id(),
        }
    );
    assert_eq!(items[1].0, amendments_key());
    assert_eq!(items[2].0, fees_key());
}

#[test]
fn create_genesis_setup_only_keeps_ledger_header_and_immutable_state_without_master_account() {
    let config = sample_ledger_config([sample_uint256(0xC1)]);
    let ledger = Ledger::create_genesis_setup_only(false, &config, [feature_xrp_fees()])
        .expect("setup-only genesis ledger should build");

    assert!(ledger.is_immutable());
    assert_eq!(ledger.header().seq, 1);
    assert_eq!(ledger.header().drops, INITIAL_XRP_DROPS);
    assert_eq!(
        ledger.header().close_time_resolution,
        LEDGER_GENESIS_TIME_RESOLUTION
    );
    assert_eq!(ledger.fees(), config.fees);
    assert!(
        ledger
            .state_map()
            .peek_item_with_hash(ledger::account_root_key(genesis_account_id()), &mut |_| {
                None
            })
            .expect("master account lookup should succeed")
            .is_none()
    );
    let (amendments, _hash) = ledger
        .state_map()
        .peek_item_with_hash(amendments_key(), &mut |_| None)
        .expect("setup-only amendments lookup should succeed")
        .expect("setup-only amendments entry should exist");
    assert_eq!(
        decode_constructor_amendments_entry(amendments.data())
            .expect("setup-only amendments should decode")
            .amendments,
        vec![feature_xrp_fees()]
    );
}

#[test]
fn create_genesis_keeps_master_account_and_singletons_in_order() {
    let amendment = sample_uint256(0xD1);
    let config = sample_ledger_config([sample_uint256(0xD2)]);
    let ledger = Ledger::create_genesis(false, &config, [feature_xrp_fees(), amendment])
        .expect("genesis ledger should build");

    assert!(ledger.is_immutable());
    assert_eq!(ledger.header().seq, 1);
    assert_eq!(ledger.header().drops, INITIAL_XRP_DROPS);
    assert_eq!(
        ledger.header().close_time_resolution,
        LEDGER_GENESIS_TIME_RESOLUTION
    );
    assert_eq!(ledger.fees(), config.fees);
    let (master_account, _master_account_hash) = ledger
        .state_map()
        .peek_item_with_hash(ledger::account_root_key(genesis_account_id()), &mut |_| {
            None
        })
        .expect("master account lookup should succeed")
        .expect("master account entry should exist");
    assert_eq!(
        decode_constructor_account_root_entry(master_account.data())
            .expect("master account should decode"),
        ConstructorAccountRootEntry {
            sequence: 1,
            balance_drops: INITIAL_XRP_DROPS,
            account_id: genesis_account_id(),
        }
    );
    let (amendments, _amendments_hash) = ledger
        .state_map()
        .peek_item_with_hash(amendments_key(), &mut |_| None)
        .expect("amendments lookup should succeed")
        .expect("amendments entry should exist");
    assert_eq!(
        decode_constructor_amendments_entry(amendments.data())
            .expect("amendments should decode")
            .amendments,
        vec![feature_xrp_fees(), amendment]
    );
    let (fees, _fees_hash) = ledger
        .state_map()
        .peek_item_with_hash(fees_key(), &mut |_| None)
        .expect("fees lookup should succeed")
        .expect("fees entry should exist");
    assert_eq!(
        decode_constructor_fee_settings_entry(fees.data()).expect("fees should decode"),
        protocol::ConstructorFeeSettingsEntry::XrpDrops {
            base_fee_drops: 10,
            reserve_base_drops: 20,
            reserve_increment_drops: 30,
        }
    );
}
