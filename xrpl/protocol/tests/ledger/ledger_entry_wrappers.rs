use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, Bridge, BridgeBuilder, Currency, Issue, LedgerEntryType, STAmount, STArray,
    STLedgerEntry, STXChainBridge, XChainOwnedClaimID, XChainOwnedClaimIDBuilder,
    XChainOwnedCreateAccountClaimID, XChainOwnedCreateAccountClaimIDBuilder, XRPAmount,
    bridge_keylet_from_door_issue, get_field_by_symbol, xchain_owned_claim_id_keylet_from_bridge,
    xchain_owned_create_account_claim_id_keylet_from_bridge,
};

fn sample_account(seed: u64) -> AccountID {
    AccountID::from_u64(seed)
}

fn sample_amount(drops: i64) -> STAmount {
    STAmount::from_xrp_amount(XRPAmount::from_drops(drops))
}

fn sample_bridge() -> STXChainBridge {
    STXChainBridge::from_parts(
        sample_account(11),
        sample_locking_issue(),
        sample_account(13),
        sample_issuing_issue(),
    )
}

fn sample_locking_issue() -> Issue {
    Issue::new(Currency::from_u64(0x1001), sample_account(12))
}

fn sample_issuing_issue() -> Issue {
    Issue::new(Currency::from_u64(0x1002), sample_account(14))
}

fn empty_attestations(field: &'static protocol::SField) -> STArray {
    STArray::new(field)
}

fn raw_account_id(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

#[test]
fn bridge_builder_and_wrapper_keep_typed_bridge_fields() {
    let key_bridge = sample_bridge();
    let account = sample_account(21);
    let signature_reward = sample_amount(25);
    let min_account_create_amount = sample_amount(7);
    let index = bridge_keylet_from_door_issue(
        raw_account_id(key_bridge.locking_chain_door()),
        sample_locking_issue(),
    )
    .key;

    let wrapper = BridgeBuilder::new(
        account,
        signature_reward.clone(),
        key_bridge.clone(),
        91,
        92,
        93,
        94,
        Uint256::from_u64(95),
        96,
    )
    .set_min_account_create_amount(min_account_create_amount.clone())
    .set_ledger_index(Uint256::from_u64(97))
    .set_flags(0xA5A5_A5A5)
    .build(index);

    assert_eq!(wrapper.get_type(), LedgerEntryType::Bridge);
    assert_eq!(wrapper.get_key(), index);
    assert_eq!(wrapper.get_account(), account);
    assert_eq!(wrapper.get_signature_reward(), signature_reward);
    assert_eq!(
        wrapper.get_min_account_create_amount(),
        Some(min_account_create_amount)
    );
    assert!(wrapper.has_min_account_create_amount());
    assert_eq!(wrapper.get_x_chain_bridge(), key_bridge);
    assert_eq!(wrapper.get_x_chain_claim_id(), 91);
    assert_eq!(wrapper.get_x_chain_account_create_count(), 92);
    assert_eq!(wrapper.get_x_chain_account_claim_count(), 93);
    assert_eq!(wrapper.get_owner_node(), 94);
    assert_eq!(wrapper.get_previous_txn_id(), Uint256::from_u64(95));
    assert_eq!(wrapper.get_previous_txn_lgr_seq(), 96);
    assert_eq!(wrapper.get_ledger_index(), Some(Uint256::from_u64(97)));
    assert_eq!(wrapper.get_flags(), 0xA5A5_A5A5);

    let sle = Arc::new(wrapper.as_st_ledger_entry().clone());
    let rebuilt = Bridge::new(sle.clone()).expect("existing SLE should wrap");
    assert_eq!(rebuilt.get_x_chain_bridge(), key_bridge);

    let rebuilt_from_builder = BridgeBuilder::from_sle(sle)
        .expect("existing Bridge SLE should seed the builder")
        .build(index);
    assert_eq!(rebuilt_from_builder.get_account(), account);
    assert_eq!(rebuilt_from_builder.get_x_chain_claim_id(), 91);
}

#[test]
fn bridge_rejects_wrong_ledger_type() {
    let sle = Arc::new(STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        Uint256::from_u64(1),
    ));

    assert!(Bridge::new(sle.clone()).is_err());
    assert!(BridgeBuilder::from_sle(sle).is_err());
}

#[test]
fn xchain_owned_claim_id_builder_and_wrapper_keep_array_and_bridge_fields() {
    let bridge = sample_bridge();
    let account = sample_account(31);
    let other_chain_source = sample_account(32);
    let attestations = empty_attestations(get_field_by_symbol("sfXChainClaimAttestations"));
    let index = xchain_owned_claim_id_keylet_from_bridge(
        raw_account_id(bridge.locking_chain_door()),
        sample_locking_issue(),
        raw_account_id(bridge.issuing_chain_door()),
        sample_issuing_issue(),
        123,
    )
    .key;

    let wrapper = XChainOwnedClaimIDBuilder::new(
        account,
        bridge.clone(),
        123,
        other_chain_source,
        attestations.clone(),
        sample_amount(77),
        55,
        Uint256::from_u64(66),
        67,
    )
    .set_ledger_index(Uint256::from_u64(68))
    .set_flags(0x55AA_55AA)
    .build(index);

    assert_eq!(wrapper.get_type(), LedgerEntryType::XChainOwnedClaimId);
    assert_eq!(wrapper.get_key(), index);
    assert_eq!(wrapper.get_account(), account);
    assert_eq!(wrapper.get_x_chain_bridge(), bridge);
    assert_eq!(wrapper.get_x_chain_claim_id(), 123);
    assert_eq!(wrapper.get_other_chain_source(), other_chain_source);
    assert_eq!(wrapper.get_x_chain_claim_attestations().len(), 0);
    assert_eq!(wrapper.get_signature_reward(), sample_amount(77));
    assert_eq!(wrapper.get_owner_node(), 55);
    assert_eq!(wrapper.get_previous_txn_id(), Uint256::from_u64(66));
    assert_eq!(wrapper.get_previous_txn_lgr_seq(), 67);
    assert_eq!(wrapper.get_ledger_index(), Some(Uint256::from_u64(68)));
    assert_eq!(wrapper.get_flags(), 0x55AA_55AA);

    let sle = Arc::new(wrapper.as_st_ledger_entry().clone());
    let rebuilt = XChainOwnedClaimID::new(sle.clone()).expect("existing SLE should wrap");
    assert_eq!(rebuilt.get_x_chain_claim_id(), 123);

    let rebuilt_from_builder = XChainOwnedClaimIDBuilder::from_sle(sle)
        .expect("existing XChainOwnedClaimID SLE should seed the builder")
        .build(index);
    assert_eq!(
        rebuilt_from_builder.get_other_chain_source(),
        other_chain_source
    );
}

#[test]
fn xchain_owned_claim_id_rejects_wrong_ledger_type() {
    let sle = Arc::new(STLedgerEntry::from_type_and_key(
        LedgerEntryType::Bridge,
        Uint256::from_u64(2),
    ));

    assert!(XChainOwnedClaimID::new(sle.clone()).is_err());
    assert!(XChainOwnedClaimIDBuilder::from_sle(sle).is_err());
}

#[test]
fn xchain_owned_create_account_claim_id_builder_and_wrapper_keep_array_and_bridge_fields() {
    let bridge = sample_bridge();
    let account = sample_account(41);
    let attestations = empty_attestations(get_field_by_symbol("sfXChainCreateAccountAttestations"));
    let index = xchain_owned_create_account_claim_id_keylet_from_bridge(
        raw_account_id(bridge.locking_chain_door()),
        sample_locking_issue(),
        raw_account_id(bridge.issuing_chain_door()),
        sample_issuing_issue(),
        321,
    )
    .key;

    let wrapper = XChainOwnedCreateAccountClaimIDBuilder::new(
        account,
        bridge.clone(),
        321,
        attestations.clone(),
        71,
        Uint256::from_u64(72),
        73,
    )
    .set_ledger_index(Uint256::from_u64(74))
    .set_flags(0xA55A_A55A)
    .build(index);

    assert_eq!(
        wrapper.get_type(),
        LedgerEntryType::XChainOwnedCreateAccountClaimId
    );
    assert_eq!(wrapper.get_key(), index);
    assert_eq!(wrapper.get_account(), account);
    assert_eq!(wrapper.get_x_chain_bridge(), bridge);
    assert_eq!(wrapper.get_x_chain_account_create_count(), 321);
    assert_eq!(wrapper.get_x_chain_create_account_attestations().len(), 0);
    assert_eq!(wrapper.get_owner_node(), 71);
    assert_eq!(wrapper.get_previous_txn_id(), Uint256::from_u64(72));
    assert_eq!(wrapper.get_previous_txn_lgr_seq(), 73);
    assert_eq!(wrapper.get_ledger_index(), Some(Uint256::from_u64(74)));
    assert_eq!(wrapper.get_flags(), 0xA55A_A55A);

    let sle = Arc::new(wrapper.as_st_ledger_entry().clone());
    let rebuilt =
        XChainOwnedCreateAccountClaimID::new(sle.clone()).expect("existing SLE should wrap");
    assert_eq!(rebuilt.get_x_chain_account_create_count(), 321);

    let rebuilt_from_builder = XChainOwnedCreateAccountClaimIDBuilder::from_sle(sle)
        .expect("existing XChainOwnedCreateAccountClaimID SLE should seed the builder")
        .build(index);
    assert_eq!(rebuilt_from_builder.get_account(), account);
}

#[test]
fn xchain_owned_create_account_claim_id_rejects_wrong_ledger_type() {
    let sle = Arc::new(STLedgerEntry::from_type_and_key(
        LedgerEntryType::XChainOwnedClaimId,
        Uint256::from_u64(3),
    ));

    assert!(XChainOwnedCreateAccountClaimID::new(sle.clone()).is_err());
    assert!(XChainOwnedCreateAccountClaimIDBuilder::from_sle(sle).is_err());
}
