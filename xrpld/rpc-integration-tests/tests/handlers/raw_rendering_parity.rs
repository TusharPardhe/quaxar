//! Endpoint-level parity: results rendered with the direct JSON writer
//! (raw rendering, as the server dispatches) are byte-identical to the
//! `JsonValue` tree rendering.

use protocol::JsonValue;
use rpc_integration_tests::render_scenario::{build, call, render, ENDPOINTS};

#[test]
fn raw_rendering_is_byte_identical_for_ledger_object_endpoints() {
    let scenario = build(60, 80);
    for endpoint in ENDPOINTS {
        let tree = render(&scenario, endpoint, false);
        let raw = render(&scenario, endpoint, true);
        assert_eq!(
            String::from_utf8(raw).unwrap(),
            String::from_utf8(tree).unwrap(),
            "{endpoint}: raw rendering diverged"
        );
    }
}

#[test]
fn scenario_populates_the_endpoints() {
    let scenario = build(60, 80);
    let JsonValue::Object(offers) = call(&scenario, "book_offers") else {
        panic!("book_offers result must be an object");
    };
    let Some(JsonValue::Array(offers)) = offers.get("offers") else {
        panic!("book_offers must list offers: {offers:?}");
    };
    assert_eq!(offers.len(), 60);
    let JsonValue::Object(objects) = call(&scenario, "account_objects") else {
        panic!("account_objects result must be an object");
    };
    let Some(JsonValue::Array(objects)) = objects.get("account_objects") else {
        panic!("account_objects must list objects: {objects:?}");
    };
    assert_eq!(objects.len(), 80);
    // Raw rendering really is used under the server's dispatch mode.
    let raw = protocol::json_writer::with_raw_rendering(|| call(&scenario, "account_objects"));
    let JsonValue::Object(raw) = raw else {
        panic!("object")
    };
    let Some(JsonValue::Array(raw_objects)) = raw.get("account_objects") else {
        panic!("array")
    };
    assert!(raw_objects
        .iter()
        .all(|item| matches!(item, JsonValue::Raw(_))));
}

#[test]
fn transaction_endpoints_return_transactions() {
    // The standalone fixture resolves `tx` (without metadata) but has no
    // account_tx index or expandable tx map, so metadata rendering is
    // covered by the protocol corpus test over real mainnet/testnet data.
    let scenario = build(20, 10);
    for endpoint in ["tx", "tx_v1"] {
        let text = String::from_utf8(render(&scenario, endpoint, false)).unwrap();
        assert!(
            !text.contains("\"error\""),
            "{endpoint} returned an error: {text}"
        );
        assert!(
            text.contains("OfferCreate"),
            "{endpoint} must render the tx: {text}"
        );
    }
}
