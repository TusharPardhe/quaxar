//! Tests for definitions paychan types.

use protocol::JsonValue;
use rpc_integration_tests::helpers::*;
#[test]
fn mptoken_issuance_set_type_code() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("MPTokenIssuanceSet"), Some(&si(56)));
}
#[test]
fn payment_channel_type_codes() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("PaymentChannelCreate"), Some(&si(13)));
    assert_eq!(t.get("PaymentChannelFund"), Some(&si(14)));
    assert_eq!(t.get("PaymentChannelClaim"), Some(&si(15)));
}
