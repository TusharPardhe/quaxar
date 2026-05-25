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
