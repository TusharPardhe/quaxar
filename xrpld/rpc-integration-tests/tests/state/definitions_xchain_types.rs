//! Tests for definitions xchain types.

use protocol::JsonValue;
use rpc_integration_tests::helpers::*;
#[test]
fn xchain_type_1() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("AMMWithdraw"), Some(&si(37)));
    assert_eq!(t.get("AMMVote"), Some(&si(38)));
    assert_eq!(t.get("AMMBid"), Some(&si(39)));
}
#[test]
fn xchain_type_2() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("AMMDelete"), Some(&si(40)));
}
#[test]
fn xchain_type_3() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Clawback"), Some(&si(30)));
}
#[test]
fn xchain_type_4() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("MPTokenIssuanceCreate"));
    assert!(t.contains_key("MPTokenIssuanceDestroy"));
    assert!(t.contains_key("MPTokenAuthorize"));
}
#[test]
fn xchain_type_5() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("MPTokenIssuance"));
    assert!(t.contains_key("MPToken"));
}

/// rippled #8255: server_definitions no longer lists the XChainBridge
/// serialized type, its fields, ledger entry types or transaction types.
#[test]
fn xchain_bridge_is_absent_from_definitions() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("definitions must be an object")
    };
    let section = |name: &str| match r.get(name) {
        Some(JsonValue::Object(map)) => map.clone(),
        other => panic!("{name}: {other:?}"),
    };
    assert!(!section("TYPES").contains_key("XChainBridge"));
    assert!(!section("TRANSACTION_TYPES").contains_key("XChainCommit"));
    assert!(!section("LEDGER_ENTRY_TYPES").contains_key("Bridge"));
    let JsonValue::Array(fields) = r.get("FIELDS").expect("FIELDS") else {
        panic!("FIELDS must be an array")
    };
    let names: Vec<String> = fields
        .iter()
        .filter_map(|entry| match entry {
            JsonValue::Array(pair) => match pair.first() {
                Some(JsonValue::String(name)) => Some(name.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert!(!names.is_empty());
    for removed in ["XChainBridge", "SignatureReward", "XChainClaimID", "LockingChainDoor"] {
        assert!(!names.iter().any(|name| name == removed), "{removed}");
    }
}
