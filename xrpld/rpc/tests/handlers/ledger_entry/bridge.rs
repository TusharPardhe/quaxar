//! Tests for bridge and xchain ledger entry wrappers.

use super::*;

#[test]
fn ledger_entry_renders_typed_bridge_and_xchain_wrappers() {
    let locking_door = account(0x44);
    let issuing_door = account(0x55);
    let locking_issue = Issue::new(currency(0x66), account(0x33));
    let issuing_issue = Issue::new(currency(0x77), account(0x22));
    let bridge = protocol::STXChainBridge::from_parts(
        locking_door,
        locking_issue,
        issuing_door,
        issuing_issue,
    );
    let bridge_index = bridge_key(
        locking_door,
        currency(0x66),
        issuing_door,
        currency(0x77),
        true,
    );
    let claim_index =
        xchain_claim_id_key(locking_door, locking_issue, issuing_door, issuing_issue, 5);
    let create_index = xchain_create_account_claim_id_key(
        locking_door,
        locking_issue,
        issuing_door,
        issuing_issue,
        6,
    );

    let bridge_wrapper = BridgeBuilder::new(
        account(0x11),
        protocol::STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(25)),
        bridge.clone(),
        5,
        6,
        7,
        8,
        Uint256::from_u64(9),
        10,
    )
    .build(bridge_index);

    let claim_wrapper = XChainOwnedClaimIDBuilder::new(
        account(0x11),
        bridge.clone(),
        5,
        account(0x22),
        STArray::new(get_field_by_symbol("sfXChainClaimAttestations")),
        protocol::STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1)),
        8,
        Uint256::from_u64(9),
        10,
    )
    .build(claim_index);

    let create_wrapper = XChainOwnedCreateAccountClaimIDBuilder::new(
        account(0x11),
        bridge.clone(),
        6,
        STArray::new(get_field_by_symbol("sfXChainCreateAccountAttestations")),
        8,
        Uint256::from_u64(9),
        10,
    )
    .build(create_index);

    let source = source_with(vec![
        bridge_wrapper.as_st_ledger_entry().clone(),
        claim_wrapper.as_st_ledger_entry().clone(),
        create_wrapper.as_st_ledger_entry().clone(),
    ]);

    let bridge_result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::Unsigned(9)),
                (
                    "bridge",
                    JsonValue::Object(BTreeMap::from([
                        (
                            "LockingChainDoor".to_owned(),
                            JsonValue::String(to_base58(locking_door)),
                        ),
                        (
                            "LockingChainIssue".to_owned(),
                            JsonValue::Object(BTreeMap::from([
                                ("currency".to_owned(), JsonValue::String("66".repeat(20))),
                                (
                                    "issuer".to_owned(),
                                    JsonValue::String(to_base58(account(0x33))),
                                ),
                            ])),
                        ),
                        (
                            "IssuingChainDoor".to_owned(),
                            JsonValue::String(to_base58(issuing_door)),
                        ),
                        (
                            "IssuingChainIssue".to_owned(),
                            JsonValue::Object(BTreeMap::from([
                                ("currency".to_owned(), JsonValue::String("77".repeat(20))),
                                (
                                    "issuer".to_owned(),
                                    JsonValue::String(to_base58(account(0x22))),
                                ),
                            ])),
                        ),
                    ])),
                ),
                ("bridge_account", JsonValue::String(to_base58(locking_door))),
            ]),
            2,
        ),
        &source,
    );
    let JsonValue::Object(bridge_object) = &bridge_result else {
        panic!("expected object");
    };
    let bridge_json = bridge_wrapper.as_st_ledger_entry().json(JsonOptions::NONE);
    assert_eq!(
        bridge_object
            .get("node")
            .and_then(|value| json_object(value).get("index")),
        Some(&JsonValue::String(bridge_wrapper.get_key().to_string()))
    );
    assert_eq!(bridge_object.get("node"), Some(&bridge_json));

    let claim_result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::Unsigned(9)),
                (
                    "xchain_owned_claim_id",
                    JsonValue::Object(BTreeMap::from([
                        (
                            "LockingChainDoor".to_owned(),
                            JsonValue::String(to_base58(locking_door)),
                        ),
                        (
                            "LockingChainIssue".to_owned(),
                            JsonValue::Object(BTreeMap::from([
                                ("currency".to_owned(), JsonValue::String("66".repeat(20))),
                                (
                                    "issuer".to_owned(),
                                    JsonValue::String(to_base58(account(0x33))),
                                ),
                            ])),
                        ),
                        (
                            "IssuingChainDoor".to_owned(),
                            JsonValue::String(to_base58(issuing_door)),
                        ),
                        (
                            "IssuingChainIssue".to_owned(),
                            JsonValue::Object(BTreeMap::from([
                                ("currency".to_owned(), JsonValue::String("77".repeat(20))),
                                (
                                    "issuer".to_owned(),
                                    JsonValue::String(to_base58(account(0x22))),
                                ),
                            ])),
                        ),
                        ("xchain_owned_claim_id".to_owned(), JsonValue::Unsigned(5)),
                    ])),
                ),
            ]),
            2,
        ),
        &source,
    );
    let JsonValue::Object(claim_object) = &claim_result else {
        panic!("expected object");
    };
    assert_eq!(
        claim_object
            .get("node")
            .and_then(|value| json_object(value).get("index")),
        Some(&JsonValue::String(claim_wrapper.get_key().to_string()))
    );

    let create_result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::Unsigned(9)),
                (
                    "xchain_owned_create_account_claim_id",
                    JsonValue::Object(BTreeMap::from([
                        (
                            "LockingChainDoor".to_owned(),
                            JsonValue::String(to_base58(locking_door)),
                        ),
                        (
                            "LockingChainIssue".to_owned(),
                            JsonValue::Object(BTreeMap::from([
                                ("currency".to_owned(), JsonValue::String("66".repeat(20))),
                                (
                                    "issuer".to_owned(),
                                    JsonValue::String(to_base58(account(0x33))),
                                ),
                            ])),
                        ),
                        (
                            "IssuingChainDoor".to_owned(),
                            JsonValue::String(to_base58(issuing_door)),
                        ),
                        (
                            "IssuingChainIssue".to_owned(),
                            JsonValue::Object(BTreeMap::from([
                                ("currency".to_owned(), JsonValue::String("77".repeat(20))),
                                (
                                    "issuer".to_owned(),
                                    JsonValue::String(to_base58(account(0x22))),
                                ),
                            ])),
                        ),
                        (
                            "xchain_owned_create_account_claim_id".to_owned(),
                            JsonValue::Unsigned(6),
                        ),
                    ])),
                ),
            ]),
            2,
        ),
        &source,
    );
    let JsonValue::Object(create_object) = &create_result else {
        panic!("expected object");
    };
    assert_eq!(
        create_object
            .get("node")
            .and_then(|value| json_object(value).get("index")),
        Some(&JsonValue::String(create_wrapper.get_key().to_string()))
    );
}
