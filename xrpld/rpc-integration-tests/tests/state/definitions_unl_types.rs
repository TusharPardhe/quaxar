//! Tests for definitions unl types.

use protocol::JsonValue;
use rpc_integration_tests::helpers::*;
#[test]
fn unl_modify_type_code() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("UNLModify"), Some(&si(102)));
}
#[test]
fn negative_unl_ledger_entry_type_code() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("NegativeUNL"), Some(&si(78)));
}
