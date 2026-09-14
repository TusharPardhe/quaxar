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
fn transaction_sign_for_rejects_delegate_fee_payer_as_signer() {
    let (_seed, secret_text, delegate) = seeded_account(0x34);
    let source = AccountID::from_array([0x44; 20]);
    let destination = AccountID::from_array([0x55; 20]);
    let params = object([
        ("secret", JsonValue::String(secret_text)),
        ("account", JsonValue::String(protocol::to_base58(delegate))),
        (
            "tx_json",
            object([
                ("TransactionType", JsonValue::String("Payment".to_owned())),
                ("Account", JsonValue::String(protocol::to_base58(source))),
                ("Delegate", JsonValue::String(protocol::to_base58(delegate))),
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

    assert_eq!(
        transaction_sign_for(&ctx),
        Err(RpcStatus::with_message(
            RpcErrorCode::InvalidParams,
            format!(
                "A Signer may not be the transaction's Account ({}).",
                protocol::to_base58(delegate)
            )
        ))
    );
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

#[test]
fn transaction_sign_rejects_non_role_signature_targets() {
    let (_seed, secret_text, account) = seeded_account(0x77);
    let params = object([
        ("secret", JsonValue::String(secret_text)),
        (
            "signature_target",
            JsonValue::String("Destination".to_owned()),
        ),
        (
            "tx_json",
            object([
                ("TransactionType", JsonValue::String("Payment".to_owned())),
                ("Account", JsonValue::String(protocol::to_base58(account))),
                (
                    "Destination",
                    JsonValue::String(protocol::to_base58(AccountID::from_array([0x78; 20]))),
                ),
                ("Amount", JsonValue::String("1000".to_owned())),
                ("Fee", JsonValue::String("10".to_owned())),
                ("Sequence", JsonValue::Unsigned(1)),
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

    assert_eq!(
        transaction_sign(&ctx),
        Err(RpcStatus::with_message(
            RpcErrorCode::InvalidParams,
            "Invalid field 'signature_target'."
        ))
    );
}

#[test]
fn transaction_sign_accepts_every_supported_signature_role() {
    let (_seed, secret_text, account) = seeded_account(0x79);
    let destination = AccountID::from_array([0x7A; 20]);

    for (offset, signature_target) in [
        None,
        Some("CounterpartySignature"),
        Some("SponsorSignature"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut tx_json = BTreeMap::from([
            (
                "TransactionType".to_owned(),
                JsonValue::String("Payment".to_owned()),
            ),
            (
                "Account".to_owned(),
                JsonValue::String(protocol::to_base58(account)),
            ),
            (
                "Destination".to_owned(),
                JsonValue::String(protocol::to_base58(destination)),
            ),
            ("Amount".to_owned(), JsonValue::String("1000".to_owned())),
            ("Fee".to_owned(), JsonValue::String("10".to_owned())),
            (
                "Sequence".to_owned(),
                JsonValue::Unsigned(20 + offset as u64),
            ),
        ]);
        if let Some(signature_target) = signature_target {
            tx_json.insert(signature_target.to_owned(), object([]));
        }
        let mut params = BTreeMap::from([
            ("secret".to_owned(), JsonValue::String(secret_text.clone())),
            ("tx_json".to_owned(), JsonValue::Object(tx_json)),
        ]);
        if let Some(signature_target) = signature_target {
            params.insert(
                "signature_target".to_owned(),
                JsonValue::String(signature_target.to_owned()),
            );
        }
        let params = JsonValue::Object(params);
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

        let JsonValue::Object(result) = transaction_sign(&ctx).expect("supported role signs")
        else {
            panic!("expected object");
        };
        let JsonValue::Object(tx_json) = result.get("tx_json").cloned().expect("tx_json") else {
            panic!("tx_json object");
        };
        let signed_object = match signature_target {
            Some(target) => match tx_json.get(target) {
                Some(JsonValue::Object(object)) => object,
                _ => panic!("{target} signature object"),
            },
            None => &tx_json,
        };
        assert!(matches!(
            signed_object.get("SigningPubKey"),
            Some(JsonValue::String(value)) if !value.is_empty()
        ));
        assert!(matches!(
            signed_object.get("TxnSignature"),
            Some(JsonValue::String(value)) if !value.is_empty()
        ));
    }
}

#[test]
fn transaction_sign_for_accepts_every_supported_signature_role() {
    let (_seed, secret_text, signer_account) = seeded_account(0x7B);
    let source = AccountID::from_array([0x7C; 20]);
    let destination = AccountID::from_array([0x7D; 20]);

    for (offset, signature_target) in [
        None,
        Some("CounterpartySignature"),
        Some("SponsorSignature"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut tx_json = BTreeMap::from([
            (
                "TransactionType".to_owned(),
                JsonValue::String("Payment".to_owned()),
            ),
            (
                "Account".to_owned(),
                JsonValue::String(protocol::to_base58(source)),
            ),
            (
                "Destination".to_owned(),
                JsonValue::String(protocol::to_base58(destination)),
            ),
            ("Amount".to_owned(), JsonValue::String("1000".to_owned())),
            ("Fee".to_owned(), JsonValue::String("10".to_owned())),
            (
                "Sequence".to_owned(),
                JsonValue::Unsigned(30 + offset as u64),
            ),
            ("SigningPubKey".to_owned(), JsonValue::String(String::new())),
        ]);
        if let Some(signature_target) = signature_target {
            tx_json.insert(signature_target.to_owned(), object([]));
        }
        let mut params = BTreeMap::from([
            ("secret".to_owned(), JsonValue::String(secret_text.clone())),
            (
                "account".to_owned(),
                JsonValue::String(protocol::to_base58(signer_account)),
            ),
            ("tx_json".to_owned(), JsonValue::Object(tx_json)),
        ]);
        if let Some(signature_target) = signature_target {
            params.insert(
                "signature_target".to_owned(),
                JsonValue::String(signature_target.to_owned()),
            );
        }
        let params = JsonValue::Object(params);
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

        let JsonValue::Object(result) =
            transaction_sign_for(&ctx).expect("supported role multisigns")
        else {
            panic!("expected object");
        };
        let JsonValue::Object(tx_json) = result.get("tx_json").cloned().expect("tx_json") else {
            panic!("tx_json object");
        };
        let signed_object = match signature_target {
            Some(target) => match tx_json.get(target) {
                Some(JsonValue::Object(object)) => object,
                _ => panic!("{target} signature object"),
            },
            None => &tx_json,
        };
        if signature_target.is_none() {
            assert!(matches!(
                signed_object.get("SigningPubKey"),
                Some(JsonValue::String(value)) if value.is_empty()
            ));
        } else {
            assert!(!signed_object.contains_key("SigningPubKey"));
        }
        assert!(matches!(
            signed_object.get("Signers"),
            Some(JsonValue::Array(signers)) if signers.len() == 1
        ));
    }
}

#[test]
fn transaction_sign_for_rejects_non_role_signature_targets() {
    let (_seed, secret_text, signer_account) = seeded_account(0x7E);
    let source = AccountID::from_array([0x7F; 20]);
    let destination = AccountID::from_array([0x80; 20]);
    let params = object([
        ("secret", JsonValue::String(secret_text)),
        (
            "account",
            JsonValue::String(protocol::to_base58(signer_account)),
        ),
        (
            "signature_target",
            JsonValue::String("Destination".to_owned()),
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
                ("Sequence", JsonValue::Unsigned(1)),
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

    assert_eq!(
        transaction_sign_for(&ctx),
        Err(RpcStatus::with_message(
            RpcErrorCode::InvalidParams,
            "Invalid field 'signature_target'."
        ))
    );
}
