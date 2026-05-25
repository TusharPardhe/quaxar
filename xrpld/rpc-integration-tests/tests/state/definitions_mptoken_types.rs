//! Tests for definitions mptoken types.

use protocol::JsonValue;
use rpc_integration_tests::helpers::*;
#[test]
fn zz1() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("MPTokenIssuanceCreate"), Some(&si(54)));
    assert_eq!(t.get("MPTokenIssuanceDestroy"), Some(&si(55)));
    assert_eq!(t.get("MPTokenAuthorize"), Some(&si(57)));
}
#[test]
fn zz2() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("AMMClawback"), Some(&si(31)));
}
#[test]
fn zz3() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("MPTokenIssuance"));
    assert!(t.contains_key("MPToken"));
    assert!(t.contains_key("AMM"));
    assert!(t.contains_key("Oracle"));
    assert!(t.contains_key("DID"));
}
