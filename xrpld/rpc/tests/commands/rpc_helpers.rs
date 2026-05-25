//! Tests for rpc helpers.

use std::collections::BTreeMap;

use basics::{base_uint::Uint256, str_hex::str_hex};
use protocol::{
    AccountID, JsonValue, KeyType, LedgerEntryType, Seed, TokenType, calc_account_id,
    derive_public_key, encode_base58_token, generate_secret_key, serialize_pay_chan_authorization,
    sign,
};
use rpc::{
    ChannelAuthorizeSource, RpcErrorCode, RpcRequestContext, RpcRole, RpcStatus, SignForSource,
    SignSource, channel_authorize, read_limit_field, read_limit_field_with_cap,
    rpc_helpers::choose_ledger_entry_type, transaction_sign, transaction_sign_for,
    tuning::LimitRange,
};

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn seeded_account(fill: u8) -> (Seed, String, AccountID) {
    let seed = Seed::from_slice(&[fill; 16]).expect("seed");
    let secret = generate_secret_key(KeyType::Secp256k1, &seed).expect("secret");
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public");
    let account = calc_account_id(public.as_bytes());
    (
        seed.clone(),
        encode_base58_token(TokenType::FamilySeed, seed.data()),
        account,
    )
}

#[test]
fn choose_ledger_entry_type_name_rules() {
    assert_eq!(choose_ledger_entry_type(&object([])), Ok(None));
    assert_eq!(
        choose_ledger_entry_type(&object([(
            "type",
            JsonValue::String("MPTokenIssuance".into())
        )])),
        Ok(Some(LedgerEntryType::MPTokenIssuance))
    );
    assert_eq!(
        choose_ledger_entry_type(&object([(
            "type",
            JsonValue::String("mptokenissuance".into())
        )])),
        Ok(Some(LedgerEntryType::MPTokenIssuance))
    );
    assert_eq!(
        choose_ledger_entry_type(&object([(
            "type",
            JsonValue::String("mpt_issuance".into())
        )])),
        Ok(Some(LedgerEntryType::MPTokenIssuance))
    );
    assert_eq!(
        choose_ledger_entry_type(&object([(
            "type",
            JsonValue::String("MPT_Issuance".into())
        )])),
        Err(RpcStatus::with_message(
            RpcErrorCode::InvalidParams,
            "Invalid field 'type'."
        ))
    );
    assert_eq!(
        choose_ledger_entry_type(&object([("type", JsonValue::Unsigned(1234))])),
        Err(RpcStatus::with_message(
            RpcErrorCode::InvalidParams,
            "Invalid field 'type', not string."
        ))
    );
}

#[test]
fn read_limit_field_limit_validation() {
    let range = LimitRange {
        rmin: 10,
        r_default: 200,
        rmax: 400,
    };

    assert_eq!(
        read_limit_field(&object([]), RpcRole::Guest, range),
        Ok(200)
    );
    assert_eq!(
        read_limit_field(
            &object([("limit", JsonValue::Unsigned(500))]),
            RpcRole::Guest,
            range
        ),
        Ok(400)
    );
    assert_eq!(
        read_limit_field(
            &object([("limit", JsonValue::Unsigned(500))]),
            RpcRole::Admin,
            range
        ),
        Ok(500)
    );
    assert_eq!(
        read_limit_field(
            &object([("limit", JsonValue::Unsigned(0))]),
            RpcRole::Guest,
            range
        ),
        Err(RpcStatus::with_message(
            RpcErrorCode::InvalidParams,
            "Invalid field 'limit'."
        ))
    );
}

#[test]
fn read_limit_field_with_cap_keeps_unlimited_role_behavior() {
    assert_eq!(
        read_limit_field_with_cap(
            &object([("limit", JsonValue::Unsigned(5000))]),
            RpcRole::Guest,
            256,
            2048
        ),
        Ok(2048)
    );
    assert_eq!(
        read_limit_field_with_cap(
            &object([("limit", JsonValue::Unsigned(5000))]),
            RpcRole::Identified,
            256,
            2048
        ),
        Ok(5000)
    );
}

#[test]
fn transaction_sign_result_shape_for_single_signing() {
    let (_seed, secret_text, account) = seeded_account(0x11);
    let destination = AccountID::from_array([0x22; 20]);
    let params = object([
        ("secret", JsonValue::String(secret_text)),
        (
            "tx_json",
            object([
                ("TransactionType", JsonValue::String("Payment".to_owned())),
                ("Account", JsonValue::String(protocol::to_base58(account))),
                (
                    "Destination",
                    JsonValue::String(protocol::to_base58(destination)),
                ),
                ("Amount", JsonValue::String("1000".to_owned())),
                ("Fee", JsonValue::String("10".to_owned())),
                ("Sequence", JsonValue::Unsigned(7)),
                ("SigningPubKey", JsonValue::String(String::new())),
            ]),
        ),
    ]);
    let ctx = RpcRequestContext {
        params: &params,
        env: &SignSource,
        runtime: &(),
        role: RpcRole::Admin,
        api_version: 2,
        headers: rpc::JsonContextHeaders {
            user: "",
            forwarded_for: "",
        },
        request_headers: BTreeMap::new(),
        unlimited: true,
        remote_ip: None,
        load_type: rpc::RpcLoadType::Reference,
    };

    let JsonValue::Object(result) = transaction_sign(&ctx).expect("sign result") else {
        panic!("expected object");
    };

    assert!(matches!(result.get("tx_blob"), Some(JsonValue::String(blob)) if !blob.is_empty()));
    assert!(matches!(result.get("hash"), Some(JsonValue::String(hash)) if !hash.is_empty()));
    assert!(result.contains_key("deprecated"));
    let JsonValue::Object(tx_json) = result.get("tx_json").cloned().expect("tx_json") else {
        panic!("tx_json object");
    };
    assert!(matches!(
        tx_json.get("SigningPubKey"),
        Some(JsonValue::String(value)) if !value.is_empty()
    ));
    assert!(matches!(
        tx_json.get("TxnSignature"),
        Some(JsonValue::String(value)) if !value.is_empty()
    ));
}

#[test]
fn transaction_sign_for_injects_multisign_entry() {
    let (_seed, secret_text, signer_account) = seeded_account(0x33);
    let source = AccountID::from_array([0x44; 20]);
    let destination = AccountID::from_array([0x55; 20]);
    let params = object([
        ("secret", JsonValue::String(secret_text)),
        (
            "account",
            JsonValue::String(protocol::to_base58(signer_account)),
        ),
        (
            "tx_json",
            object([
                ("TransactionType", JsonValue::String("Payment".to_owned())),
                ("Account", JsonValue::String(protocol::to_base58(source))),
                (
                    "Destination",
                    JsonValue::String(protocol::to_base58(destination)),
                ),
                ("Amount", JsonValue::String("1000".to_owned())),
                ("Fee", JsonValue::String("10".to_owned())),
                ("Sequence", JsonValue::Unsigned(9)),
                ("SigningPubKey", JsonValue::String(String::new())),
            ]),
        ),
    ]);
    let ctx = RpcRequestContext {
        params: &params,
        env: &SignForSource,
        runtime: &(),
        role: RpcRole::Admin,
        api_version: 2,
        headers: rpc::JsonContextHeaders {
            user: "",
            forwarded_for: "",
        },
        request_headers: BTreeMap::new(),
        unlimited: true,
        remote_ip: None,
        load_type: rpc::RpcLoadType::Reference,
    };

    let JsonValue::Object(result) = transaction_sign_for(&ctx).expect("sign_for result") else {
        panic!("expected object");
    };

    assert!(matches!(result.get("tx_blob"), Some(JsonValue::String(blob)) if !blob.is_empty()));
    assert!(result.contains_key("deprecated"));
    let JsonValue::Object(tx_json) = result.get("tx_json").cloned().expect("tx_json") else {
        panic!("tx_json object");
    };
    let JsonValue::Array(signers) = tx_json.get("Signers").cloned().expect("signers array") else {
        panic!("Signers array");
    };
    assert_eq!(signers.len(), 1);
}

#[test]
fn channel_authorize_returns_real_signature_hex() {
    let (seed, secret_text, _account) = seeded_account(0x66);
    let secret = generate_secret_key(KeyType::Secp256k1, &seed).expect("secret");
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public");
    let channel_id =
        Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
            .expect("channel id");
    let amount = 1234_u64;
    let params = object([
        ("secret", JsonValue::String(secret_text)),
        ("channel_id", JsonValue::String(channel_id.to_string())),
        ("amount", JsonValue::String(amount.to_string())),
    ]);
    let ctx = RpcRequestContext {
        params: &params,
        env: &ChannelAuthorizeSource,
        runtime: &(),
        role: RpcRole::Admin,
        api_version: 2,
        headers: rpc::JsonContextHeaders {
            user: "",
            forwarded_for: "",
        },
        request_headers: BTreeMap::new(),
        unlimited: true,
        remote_ip: None,
        load_type: rpc::RpcLoadType::Reference,
    };

    let JsonValue::Object(result) = channel_authorize(&ctx).expect("channel authorize result")
    else {
        panic!("expected object");
    };
    let JsonValue::String(signature_hex) = result.get("signature").cloned().expect("signature")
    else {
        panic!("signature string");
    };

    let expected = sign(
        &public,
        &secret,
        &serialize_pay_chan_authorization(&channel_id, amount),
    )
    .expect("signature");
    assert_eq!(signature_hex, str_hex(&expected));
}
