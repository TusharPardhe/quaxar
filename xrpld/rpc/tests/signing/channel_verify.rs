//! Tests for channel verify.

#![allow(clippy::needless_borrows_for_generic_args)]

use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use basics::str_hex::str_hex;
use protocol::{
    JsonValue, KeyType, SecretKey, TokenType, derive_public_key, encode_base58_token,
    serialize_pay_chan_authorization, sign,
};
use rpc::do_channel_verify;

fn object(fields: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn encode_account_public_base58(public_key: &[u8]) -> String {
    encode_base58_token(TokenType::AccountPublic, public_key)
}

#[test]
fn channel_verify_accepts_base58_secp_and_hex_public_keys() {
    let secret = SecretKey::from_bytes([1u8; 32]);
    let public_key = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let public_key_bytes = public_key.as_bytes();
    let channel_id =
        Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
            .expect("channel id should parse");
    let amount = 1234_u64;
    let message = serialize_pay_chan_authorization(&channel_id, amount);
    let signature = sign(&public_key, &secret, &message).expect("secp signature");

    let base58_result = do_channel_verify(&object([
        (
            "public_key",
            JsonValue::String(encode_account_public_base58(public_key_bytes)),
        ),
        ("channel_id", JsonValue::String(channel_id.to_string())),
        ("amount", JsonValue::String(amount.to_string())),
        ("signature", JsonValue::String(str_hex(&signature))),
    ]));
    let JsonValue::Object(base58_result) = base58_result else {
        panic!("expected object");
    };
    assert_eq!(
        base58_result.get("signature_verified"),
        Some(&JsonValue::Bool(true))
    );

    let hex_result = do_channel_verify(&object([
        ("public_key", JsonValue::String(str_hex(&public_key_bytes))),
        ("channel_id", JsonValue::String(channel_id.to_string())),
        ("amount", JsonValue::String(amount.to_string())),
        ("signature", JsonValue::String(str_hex(&signature))),
    ]));
    let JsonValue::Object(hex_result) = hex_result else {
        panic!("expected object");
    };
    assert_eq!(
        hex_result.get("signature_verified"),
        Some(&JsonValue::Bool(true))
    );
}

#[test]
fn channel_verify_accepts_ed25519_public_keys() {
    let secret = SecretKey::from_bytes([7u8; 32]);
    let public_key = derive_public_key(KeyType::Ed25519, &secret).expect("public key");
    let channel_id =
        Uint256::from_hex("1111111111111111111111111111111111111111111111111111111111111111")
            .expect("channel id should parse");
    let amount = 9999_u64;
    let message = serialize_pay_chan_authorization(&channel_id, amount);
    let signature = sign(&public_key, &secret, &message).expect("ed25519 signature");

    let result = do_channel_verify(&object([
        (
            "public_key",
            JsonValue::String(encode_account_public_base58(public_key.as_bytes())),
        ),
        ("channel_id", JsonValue::String(channel_id.to_string())),
        ("amount", JsonValue::String(amount.to_string())),
        ("signature", JsonValue::String(str_hex(&signature))),
    ]));
    let JsonValue::Object(result) = result else {
        panic!("expected object");
    };
    assert_eq!(
        result.get("signature_verified"),
        Some(&JsonValue::Bool(true))
    );
}

#[test]
fn channel_verify_reports_the_current_malformed_error_surface() {
    let missing = do_channel_verify(&JsonValue::Object(BTreeMap::new()));
    let JsonValue::Object(missing) = missing else {
        panic!("missing response must be an object");
    };
    assert_eq!(
        missing.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    let malformed_key = do_channel_verify(&object([
        ("public_key", JsonValue::String("foo".to_owned())),
        (
            "channel_id",
            JsonValue::String(
                "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDE".to_owned(),
            ),
        ),
        ("amount", JsonValue::String("1".to_owned())),
        ("signature", JsonValue::String("DEADBEEF".to_owned())),
    ]));
    let JsonValue::Object(malformed_key) = malformed_key else {
        panic!("malformed response must be an object");
    };
    assert_eq!(
        malformed_key.get("error"),
        Some(&JsonValue::String("publicMalformed".to_owned()))
    );

    let malformed_channel = do_channel_verify(&object([
        (
            "public_key",
            JsonValue::String("aB4BXXLuPu8DpVuyq1DBiu3SrPdtK9AYZisKhu8mvkoiUD8J9Gov".to_owned()),
        ),
        ("channel_id", JsonValue::String("1234".to_owned())),
        ("amount", JsonValue::String("1".to_owned())),
        ("signature", JsonValue::String("DEADBEEF".to_owned())),
    ]));
    let JsonValue::Object(malformed_channel) = malformed_channel else {
        panic!("malformed response must be an object");
    };
    assert_eq!(
        malformed_channel.get("error"),
        Some(&JsonValue::String("channelMalformed".to_owned()))
    );

    let malformed_amount = do_channel_verify(&object([
        (
            "public_key",
            JsonValue::String("aB4BXXLuPu8DpVuyq1DBiu3SrPdtK9AYZisKhu8mvkoiUD8J9Gov".to_owned()),
        ),
        (
            "channel_id",
            JsonValue::String(
                "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF".to_owned(),
            ),
        ),
        (
            "amount",
            JsonValue::String("18446744073709551616".to_owned()),
        ),
        ("signature", JsonValue::String("DEADBEEF".to_owned())),
    ]));
    let JsonValue::Object(malformed_amount) = malformed_amount else {
        panic!("malformed response must be an object");
    };
    assert_eq!(
        malformed_amount.get("error"),
        Some(&JsonValue::String("channelAmtMalformed".to_owned()))
    );

    let typed_amount = do_channel_verify(&object([
        (
            "public_key",
            JsonValue::String("aB4BXXLuPu8DpVuyq1DBiu3SrPdtK9AYZisKhu8mvkoiUD8J9Gov".to_owned()),
        ),
        (
            "channel_id",
            JsonValue::String(
                "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF".to_owned(),
            ),
        ),
        ("amount", JsonValue::Unsigned(1)),
        ("signature", JsonValue::String("DEADBEEF".to_owned())),
    ]));
    let JsonValue::Object(typed_amount) = typed_amount else {
        panic!("typed response must be an object");
    };
    assert_eq!(
        typed_amount.get("error"),
        Some(&JsonValue::String("channelAmtMalformed".to_owned()))
    );
}
