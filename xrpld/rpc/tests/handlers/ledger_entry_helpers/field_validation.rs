//! ledger entry helpers tests part B.

use super::*;

#[test]
fn parse_account_id_validates_all_types() {
    // Non-string types should return None
    assert!(parse_account_id(&JsonValue::Unsigned(42)).is_none());
    assert!(parse_account_id(&JsonValue::Bool(true)).is_none());
    assert!(parse_account_id(&JsonValue::Null).is_none());
    assert!(parse_account_id(&JsonValue::Array(vec![])).is_none());
    assert!(parse_account_id(&JsonValue::Object(Default::default())).is_none());

    // Malformed string
    assert!(parse_account_id(&JsonValue::String("notAnAccount".to_owned())).is_none());

    // Valid account
    let valid = parse_account_id(&JsonValue::String(
        "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned(),
    ));
    assert!(valid.is_some());
    assert_eq!(
        valid.unwrap(),
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").unwrap()
    );
}

#[test]
fn parse_uint32_validates_types() {
    assert_eq!(parse_uint32(&JsonValue::Unsigned(42)), Some(42));
    assert_eq!(parse_uint32(&JsonValue::Signed(100)), Some(100));
    assert_eq!(parse_uint32(&JsonValue::Signed(-1)), None);
    assert_eq!(parse_uint32(&JsonValue::String("42".to_owned())), Some(42));
    assert_eq!(parse_uint32(&JsonValue::Bool(true)), None);
    assert_eq!(parse_uint32(&JsonValue::Null), None);
    assert_eq!(parse_uint32(&JsonValue::Array(vec![])), None);
}

#[test]
fn parse_uint256_validates_types() {
    let valid_hex = "AABBCCDDAABBCCDDAABBCCDDAABBCCDDAABBCCDDAABBCCDDAABBCCDDAABBCCDD";
    assert!(parse_uint256(&JsonValue::String(valid_hex.to_owned())).is_some());

    let short_hex = "AABB";
    assert!(parse_uint256(&JsonValue::String(short_hex.to_owned())).is_none());

    let not_hex = "ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ";
    assert!(parse_uint256(&JsonValue::String(not_hex.to_owned())).is_none());

    assert!(parse_uint256(&JsonValue::Unsigned(42)).is_none());
    assert!(parse_uint256(&JsonValue::Bool(true)).is_none());
    assert!(parse_uint256(&JsonValue::Null).is_none());
}

#[test]
fn parse_hex_blob_validates_types() {
    let valid = parse_hex_blob(&JsonValue::String("DEADBEEF".to_owned()), 64);
    assert!(valid.is_some());
    assert_eq!(valid.unwrap(), vec![0xDE, 0xAD, 0xBE, 0xEF]);

    // Odd length is valid (first char treated as single nibble)
    let odd = parse_hex_blob(&JsonValue::String("ABC".to_owned()), 64);
    assert!(odd.is_some());
    assert_eq!(odd.unwrap(), vec![0x0A, 0xBC]);

    // Not hex returns None
    assert!(parse_hex_blob(&JsonValue::String("ZZZZ".to_owned()), 64).is_none());

    // Non-string returns None
    assert!(parse_hex_blob(&JsonValue::Unsigned(42), 64).is_none());

    // Empty string returns None
    assert!(parse_hex_blob(&JsonValue::String("".to_owned()), 64).is_none());

    // Exceeds max_length returns None
    let long = "AA".repeat(65);
    assert!(parse_hex_blob(&JsonValue::String(long), 64).is_none());

    // Exactly at max_length is fine
    let exact = "AA".repeat(64);
    assert!(parse_hex_blob(&JsonValue::String(exact), 64).is_some());
}

#[test]
fn required_account_id_returns_missing_field_error() {
    let params = object([]);
    let result = required_account_id(&params, "owner", "malformedAddress");
    assert!(result.is_err());
    let err = result.unwrap_err();
    let (error, _code, message) = error_fields(&err);
    assert_eq!(error, "malformedAddress");
    assert!(message.contains("owner"));
}

#[test]
fn required_uint32_returns_missing_field_error() {
    let params = object([]);
    let result = required_uint32(&params, "seq", "invalidParams");
    assert!(result.is_err());
    let err = result.unwrap_err();
    let (error, _code, message) = error_fields(&err);
    assert_eq!(error, "invalidParams");
    assert!(message.contains("seq"));
}

#[test]
fn required_uint256_returns_missing_field_error() {
    let params = object([]);
    let result = required_uint256(&params, "hash", "invalidParams");
    assert!(result.is_err());
    let err = result.unwrap_err();
    let (error, _code, message) = error_fields(&err);
    assert_eq!(error, "invalidParams");
    assert!(message.contains("hash"));
}

#[test]
fn parse_account_root_validates_account_field() {
    let valid = parse_account_root(
        &object([(
            "account",
            JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
        )]),
        "account",
        2,
    );
    assert!(valid.is_ok());
    let key = valid.unwrap();
    assert_eq!(
        key,
        account_keylet(
            Uint160::from_slice(
                parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh")
                    .unwrap()
                    .data()
            )
            .unwrap()
        )
        .key
    );

    let malformed = parse_account_root(
        &object([("account", JsonValue::String("notAnAccount".to_owned()))]),
        "account",
        2,
    );
    assert!(malformed.is_err());
    let malformed_err = malformed.unwrap_err();
    let (error, _code, _message) = error_fields(&malformed_err);
    assert_eq!(error, "malformedAddress");
}

#[test]
fn parse_directory_node_validates_owner_field() {
    let valid_owner = parse_directory_node(
        &object([(
            "owner",
            JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
        )]),
        "directory",
        2,
    );
    assert!(valid_owner.is_ok());
    let key = valid_owner.unwrap();
    let account = parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").unwrap();
    assert_eq!(
        key,
        owner_dir_keylet(Uint160::from_slice(account.data()).unwrap()).key
    );
}

#[test]
fn parse_ripple_state_validates_accounts_and_currency() {
    let valid = parse_ripple_state(
        &object([
            (
                "accounts",
                JsonValue::Array(vec![
                    JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
                    JsonValue::String("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe".to_owned()),
                ]),
            ),
            ("currency", JsonValue::String("USD".to_owned())),
        ]),
        "ripple_state",
    );
    assert!(valid.is_ok());
    let key = valid.unwrap();

    let low = parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").unwrap();
    let high = parse_base58_account_id("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe").unwrap();
    let (actual_low, actual_high) = if low < high { (low, high) } else { (high, low) };
    let expected = line(
        actual_low,
        actual_high,
        protocol::currency_from_string("USD"),
    );
    assert_eq!(key, expected.key);

    // Missing currency
    let missing_currency = parse_ripple_state(
        &object([(
            "accounts",
            JsonValue::Array(vec![
                JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
                JsonValue::String("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe".to_owned()),
            ]),
        )]),
        "ripple_state",
    );
    assert!(missing_currency.is_err());

    // Missing accounts
    let missing_accounts = parse_ripple_state(
        &object([("currency", JsonValue::String("USD".to_owned()))]),
        "ripple_state",
    );
    assert!(missing_accounts.is_err());
}
