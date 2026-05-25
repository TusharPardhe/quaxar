//! Tests for the random RPC handler.

use basics::base_uint::Uint256;
use protocol::JsonValue;
use rpc::do_random;

#[test]
fn random_returns_a_uint256_string() {
    let first = do_random();
    let second = do_random();

    let JsonValue::Object(first) = first else {
        panic!("random response must be an object");
    };
    let JsonValue::Object(second) = second else {
        panic!("random response must be an object");
    };

    let first_value = match first.get("random") {
        Some(JsonValue::String(value)) => value,
        other => panic!("unexpected random payload: {other:?}"),
    };
    let second_value = match second.get("random") {
        Some(JsonValue::String(value)) => value,
        other => panic!("unexpected random payload: {other:?}"),
    };

    assert_eq!(first_value.len(), 64);
    assert_eq!(second_value.len(), 64);
    assert!(Uint256::from_hex(first_value).is_ok());
    assert!(Uint256::from_hex(second_value).is_ok());
    assert_ne!(first_value, second_value);
}

#[test]
fn random_response_has_exactly_one_field() {
    let result = do_random();
    let JsonValue::Object(object) = result else {
        panic!("random response must be an object");
    };

    assert_eq!(object.len(), 1);
    assert!(object.contains_key("random"));
    assert!(!object.contains_key("error"));
}

#[test]
fn random_produces_unique_values_across_calls() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..10 {
        let result = do_random();
        let JsonValue::Object(object) = result else {
            panic!("random response must be an object");
        };
        let JsonValue::String(value) = object.get("random").unwrap() else {
            panic!("random must be a string");
        };
        assert!(seen.insert(value.clone()), "random should be unique");
    }
    assert_eq!(seen.len(), 10);
}

#[test]
fn random_value_is_valid_uint256() {
    for _ in 0..5 {
        let result = do_random();
        let JsonValue::Object(object) = result else {
            panic!("random response must be an object");
        };
        let JsonValue::String(value) = object.get("random").unwrap() else {
            panic!("random must be a string");
        };
        assert_eq!(value.len(), 64, "random should be 64 hex chars");
        let parsed = Uint256::from_hex(value);
        assert!(parsed.is_ok(), "random should be valid hex: {value}");
        assert_ne!(
            parsed.unwrap(),
            Uint256::zero(),
            "random should not be zero"
        );
    }
}
