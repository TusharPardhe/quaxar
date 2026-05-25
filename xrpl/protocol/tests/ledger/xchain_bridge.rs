use protocol::{
    Asset, Issue, JsonOptions, JsonValue, STXChainBridge, Serializer, StBase,
    XChainBridgeChainType, currency_from_string, get_field_by_symbol, parse_base58_account_id,
    xrp_issue,
};

fn account(value: &str) -> protocol::AccountID {
    parse_base58_account_id(value).expect("account should parse")
}

#[test]
fn protocol_xchain_bridge_json_validation_rejects_extra_fields_and_invalid_doors() {
    let extra_field = JsonValue::Object(
        [
            (
                "LockingChainDoor".to_string(),
                JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_string()),
            ),
            (
                "LockingChainIssue".to_string(),
                JsonValue::Object(
                    [("currency".to_string(), JsonValue::String("XRP".to_string()))]
                        .into_iter()
                        .collect(),
                ),
            ),
            (
                "IssuingChainDoor".to_string(),
                JsonValue::String("r3kmLJN5D28dHuH8vZNUZpMC43pEHpaocV".to_string()),
            ),
            (
                "IssuingChainIssue".to_string(),
                JsonValue::Object(
                    [
                        ("currency".to_string(), JsonValue::String("USD".to_string())),
                        (
                            "issuer".to_string(),
                            JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_string()),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ),
            (
                "Unexpected".to_string(),
                JsonValue::String("extra".to_string()),
            ),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(
        STXChainBridge::from_json_value(get_field_by_symbol("sfXChainBridge"), &extra_field),
        Err("STXChainBridge extra field detected: Unexpected".to_string())
    );

    let invalid_door = JsonValue::Object(
        [
            (
                "LockingChainDoor".to_string(),
                JsonValue::String("not-an-account".to_string()),
            ),
            (
                "LockingChainIssue".to_string(),
                JsonValue::Object(
                    [("currency".to_string(), JsonValue::String("XRP".to_string()))]
                        .into_iter()
                        .collect(),
                ),
            ),
            (
                "IssuingChainDoor".to_string(),
                JsonValue::String("r3kmLJN5D28dHuH8vZNUZpMC43pEHpaocV".to_string()),
            ),
            (
                "IssuingChainIssue".to_string(),
                JsonValue::Object(
                    [("currency".to_string(), JsonValue::String("XRP".to_string()))]
                        .into_iter()
                        .collect(),
                ),
            ),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(
        STXChainBridge::from_json_value(get_field_by_symbol("sfXChainBridge"), &invalid_door),
        Err("STXChainBridge LockingChainDoor must be a valid account".to_string())
    );
}

#[test]
fn protocol_xchain_bridge_round_trips_through_object_json_and_serial_order() {
    let locking_door = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let issuing_door = account("r3kmLJN5D28dHuH8vZNUZpMC43pEHpaocV");
    let issuing_issue = Issue::new(currency_from_string("USD"), locking_door);
    let bridge = STXChainBridge::from_parts(locking_door, xrp_issue(), issuing_door, issuing_issue);

    let object = bridge.to_st_object();
    let from_object = STXChainBridge::from_st_object(&object);
    assert_eq!(from_object, bridge);

    let json = bridge.json(JsonOptions::NONE);
    let from_json = STXChainBridge::from_json_value(get_field_by_symbol("sfXChainBridge"), &json)
        .expect("bridge json");
    assert_eq!(from_json, bridge);

    let mut bridge_serializer = Serializer::default();
    bridge.add(&mut bridge_serializer);

    let mut expected = Serializer::default();
    protocol::STAccount::from_value(get_field_by_symbol("sfLockingChainDoor"), locking_door)
        .add(&mut expected);
    protocol::STIssue::new_with_asset(get_field_by_symbol("sfLockingChainIssue"), xrp_issue())
        .add(&mut expected);
    protocol::STAccount::from_value(get_field_by_symbol("sfIssuingChainDoor"), issuing_door)
        .add(&mut expected);
    protocol::STIssue::new_with_asset(get_field_by_symbol("sfIssuingChainIssue"), issuing_issue)
        .add(&mut expected);
    assert_eq!(bridge_serializer.data(), expected.data());

    let reparsed = STXChainBridge::from_serial_iter(
        &mut protocol::SerialIter::new(bridge_serializer.data()),
        get_field_by_symbol("sfXChainBridge"),
    );
    assert_eq!(reparsed, bridge);
}

#[test]
fn protocol_xchain_bridge_helpers_map_chain_direction() {
    let locking_door = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let issuing_door = account("r3kmLJN5D28dHuH8vZNUZpMC43pEHpaocV");
    let issuing_issue = Issue::new(currency_from_string("USD"), locking_door);
    let bridge = STXChainBridge::from_parts(locking_door, xrp_issue(), issuing_door, issuing_issue);

    assert_eq!(
        STXChainBridge::other_chain(XChainBridgeChainType::Locking),
        XChainBridgeChainType::Issuing
    );
    assert_eq!(
        STXChainBridge::src_chain(true),
        XChainBridgeChainType::Locking
    );
    assert_eq!(
        STXChainBridge::dst_chain(true),
        XChainBridgeChainType::Issuing
    );
    assert_eq!(bridge.door(XChainBridgeChainType::Locking), locking_door);
    assert_eq!(bridge.door(XChainBridgeChainType::Issuing), issuing_door);
    assert_eq!(
        bridge.issue(XChainBridgeChainType::Locking),
        Asset::Issue(xrp_issue())
    );
    assert_eq!(
        bridge.issue(XChainBridgeChainType::Issuing),
        Asset::Issue(issuing_issue)
    );
}
