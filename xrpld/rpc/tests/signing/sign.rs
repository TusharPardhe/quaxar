//! `sign` and sign-and-submit preprocessing against a ledger: rippled
//! `transactionPreProcessImpl`, `checkFee`, `checkPayment` and
//! `acctMatchesPubKey`.

use std::collections::BTreeMap;
use std::sync::Arc;

use app::{ApplicationRoot, ApplicationRootOptions};
use ledger::{Fees, Ledger, LedgerConfig};
use protocol::{AccountID, JsonValue, KeyType, Seed, calc_account_id, json};
use rpc::state::context::JsonContextHeaders;
use rpc::{
    RpcErrorCode, RpcLoadType, RpcRequestContext, RpcRole, RpcStatus, SignSource, SubmitSource,
    do_sign, do_submit, transaction_sign,
};

/// The genesis account, funded by `Ledger::create_genesis`.
const GENESIS: &str = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh";
const DESTINATION: &str = "rPMh7Pi9ct699iZUTWaytJUoHcJ7cgyziK";

fn standalone_app() -> ApplicationRoot {
    let options = ApplicationRootOptions {
        standalone: true,
        ..ApplicationRootOptions::default()
    };
    let app = ApplicationRoot::with_options(options).expect("app");
    // Mainnet fees: base 10 drops.
    let config = LedgerConfig {
        fees: Fees {
            base: 10,
            reserve: 1_000_000,
            increment: 200_000,
        },
        ..LedgerConfig::default()
    };
    let parent =
        Arc::new(Ledger::create_genesis(false, &config, std::iter::empty()).expect("genesis"));
    app.on_closed_ledger(Arc::clone(&parent));
    let _ = app.open_ledger().modify(|view| {
        *view = app::AppOpenLedgerView::with_parent_hash(2, 10, *parent.header().hash.as_uint256());
        true
    });
    app
}

fn context<'a, Source>(
    app: &'a ApplicationRoot,
    params: &'a JsonValue,
    env: &'a Source,
    role: RpcRole,
) -> RpcRequestContext<'a, Source, ApplicationRoot> {
    RpcRequestContext {
        params,
        env,
        runtime: app,
        role,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        request_headers: BTreeMap::new(),
        unlimited: role == RpcRole::Admin,
        remote_ip: None,
        load_type: RpcLoadType::Reference,
    }
}

fn payment(extra: &[(&str, JsonValue)]) -> JsonValue {
    let mut tx = BTreeMap::from([
        ("TransactionType".to_owned(), json!("Payment")),
        ("Account".to_owned(), json!(GENESIS)),
        ("Destination".to_owned(), json!(DESTINATION)),
        ("Amount".to_owned(), json!("1000000")),
    ]);
    for (key, value) in extra {
        tx.insert((*key).to_owned(), value.clone());
    }
    JsonValue::Object(tx)
}

fn request(tx_json: JsonValue, extra: &[(&str, JsonValue)]) -> JsonValue {
    let mut params = BTreeMap::from([
        ("secret".to_owned(), json!("snoPBrXtMeMyMHUVTgbuqAfg1SUTb")),
        ("tx_json".to_owned(), tx_json),
    ]);
    for (key, value) in extra {
        params.insert((*key).to_owned(), value.clone());
    }
    JsonValue::Object(params)
}

fn sign(
    app: &ApplicationRoot,
    params: &JsonValue,
) -> Result<BTreeMap<String, JsonValue>, RpcStatus> {
    let ctx = context(app, params, &SignSource, RpcRole::Admin);
    match transaction_sign(&ctx)? {
        JsonValue::Object(result) => Ok(result),
        other => panic!("sign result must be an object: {other:?}"),
    }
}

fn tx_json(result: &BTreeMap<String, JsonValue>) -> BTreeMap<String, JsonValue> {
    match result.get("tx_json") {
        Some(JsonValue::Object(tx)) => tx.clone(),
        other => panic!("tx_json object: {other:?}"),
    }
}

fn account_of(seed_fill: u8) -> (String, AccountID) {
    let seed = Seed::from_slice(&[seed_fill; 16]).expect("seed");
    let (public, _) = protocol::generate_key_pair(KeyType::Secp256k1, &seed).expect("keys");
    (
        protocol::encode_base58_token(protocol::TokenType::FamilySeed, seed.data()),
        calc_account_id(public.as_bytes()),
    )
}

#[test]
fn fee_is_the_current_network_fee() {
    let app = standalone_app();
    // Remote load raises the fee without making the cluster "loaded".
    app.load_fee_track().set_remote_fee(256 * 3);
    let mut app = app;
    app.set_signing_support(true);
    let expected = app.current_network_fee_for(10, false);
    assert_ne!(
        expected, 12,
        "the test needs a fee other than the old constant"
    );

    let params = request(payment(&[]), &[]);
    let ctx = context(&app, &params, &SignSource, RpcRole::User);
    let JsonValue::Object(result) = do_sign(&ctx).expect("sign") else {
        panic!("object");
    };
    assert_eq!(result.get("error"), None, "{result:?}");
    let tx = tx_json(&result);
    assert_eq!(tx.get("Fee"), Some(&json!(expected.to_string())));
    assert_eq!(tx.get("Sequence"), Some(&json!(1u64)));
}

#[test]
fn given_fee_is_kept_even_when_zero() {
    let app = standalone_app();
    let result = sign(&app, &request(payment(&[("Fee", json!("0"))]), &[])).expect("sign");
    assert_eq!(tx_json(&result).get("Fee"), Some(&json!("0")));
}

#[test]
fn flags_are_not_filled() {
    let app = standalone_app();
    let result = sign(&app, &request(payment(&[]), &[])).expect("sign");
    assert_eq!(tx_json(&result).get("Flags"), None);
}

#[test]
fn fee_limit_follows_fee_mult_max_and_fee_div_max() {
    let app = standalone_app();
    let error =
        |extra: &[(&str, JsonValue)]| sign(&app, &request(payment(&[]), extra)).unwrap_err();
    assert_eq!(
        error(&[("fee_mult_max", json!(0u64))]),
        RpcStatus::with_message(
            RpcErrorCode::HighFee,
            "Fee of 10 exceeds the requested tx limit of 0"
        )
    );
    assert_eq!(
        error(&[("fee_mult_max", json!("10"))]),
        RpcStatus::with_message(
            RpcErrorCode::HighFee,
            "Invalid field 'fee_mult_max', not a positive integer."
        )
    );
    assert_eq!(
        error(&[("fee_mult_max", json!(-1i64))]),
        RpcStatus::with_message(
            RpcErrorCode::InvalidParams,
            "Invalid field 'fee_mult_max', not a positive integer."
        )
    );
    assert_eq!(
        error(&[("fee_div_max", json!(0u64))]),
        RpcStatus::with_message(
            RpcErrorCode::InvalidParams,
            "Invalid field 'fee_div_max', not a positive integer."
        )
    );
    assert_eq!(
        error(&[("fee_mult_max", json!(1u64)), ("fee_div_max", json!(2u64))]),
        RpcStatus::with_message(
            RpcErrorCode::HighFee,
            "Fee of 10 exceeds the requested tx limit of 5"
        )
    );
}

#[test]
fn secret_must_belong_to_the_account() {
    let app = standalone_app();
    let (other_secret, _) = account_of(0x21);
    let params = json!({ "secret": other_secret, "tx_json": payment(&[]) });
    assert_eq!(
        sign(&app, &params).unwrap_err(),
        RpcStatus::new(RpcErrorCode::BadSecret)
    );
}

#[test]
fn unknown_source_account_is_reported() {
    let app = standalone_app();
    let (secret, account) = account_of(0x22);
    let tx = payment(&[("Account", json!(protocol::to_base58(account)))]);
    let params = json!({ "secret": secret, "tx_json": tx });
    assert_eq!(
        sign(&app, &params).unwrap_err(),
        RpcStatus::new(RpcErrorCode::SrcActNotFound)
    );
}

#[test]
fn offline_signing_needs_sequence_and_fee() {
    let app = standalone_app();
    let offline = [("offline", json!(true))];
    assert_eq!(
        sign(&app, &request(payment(&[]), &offline)).unwrap_err(),
        RpcStatus::missing_field_error("tx_json.Sequence")
    );
    assert_eq!(
        sign(
            &app,
            &request(payment(&[("Sequence", json!(5u64))]), &offline)
        )
        .unwrap_err(),
        RpcStatus::missing_field_error("tx_json.Fee")
    );
    let tx = payment(&[("Sequence", json!(5u64)), ("Fee", json!("15"))]);
    let result = sign(&app, &request(tx, &offline)).expect("offline sign");
    assert_eq!(tx_json(&result).get("Fee"), Some(&json!("15")));
}

#[test]
fn tx_json_checks_match_rippled() {
    let app = standalone_app();
    let mut tx = payment(&[]);
    if let JsonValue::Object(tx) = &mut tx {
        tx.remove("Account");
    }
    assert_eq!(
        sign(&app, &request(tx, &[])).unwrap_err(),
        RpcStatus::with_message(
            RpcErrorCode::SrcActMissing,
            "Missing field 'tx_json.Account'."
        )
    );
    let tx = payment(&[("Signers", json!([]))]);
    assert_eq!(
        sign(&app, &request(tx, &[])).unwrap_err(),
        RpcStatus::new(RpcErrorCode::AlreadyMultisig)
    );
    let tx = payment(&[("DeliverMax", json!("5"))]);
    assert_eq!(
        sign(&app, &request(tx, &[])).unwrap_err(),
        RpcStatus::make_param_error("Cannot specify differing 'Amount' and 'DeliverMax'")
    );
    let tx = payment(&[("Sequence", json!(5u64)), ("Fee", json!("15"))]);
    assert_eq!(
        sign(
            &app,
            &request(tx, &[("offline", json!(true)), ("build_path", json!(true))])
        )
        .unwrap_err(),
        RpcStatus::make_param_error("Field 'build_path' not allowed in this context.")
    );
}

#[test]
fn signing_needs_admin_or_signing_support() {
    let mut app = standalone_app();
    let params = request(payment(&[]), &[]);
    let not_supported = RpcStatus::with_message(
        RpcErrorCode::NotSupported,
        "Signing is not supported by this server.",
    );

    let ctx = context(&app, &params, &SignSource, RpcRole::User);
    assert_eq!(do_sign(&ctx), Err(not_supported.clone()));
    let submit = context(&app, &params, &SubmitSource, RpcRole::User);
    assert_eq!(do_submit(&submit), Err(not_supported));

    let admin = context(&app, &params, &SignSource, RpcRole::Admin);
    assert!(do_sign(&admin).is_ok());
    app.set_signing_support(true);
    let ctx = context(&app, &params, &SignSource, RpcRole::User);
    let JsonValue::Object(result) = do_sign(&ctx).expect("sign") else {
        panic!("object");
    };
    assert!(result.contains_key("tx_blob"));
}

#[test]
fn sign_errors_carry_the_deprecation_notice() {
    let app = standalone_app();
    let (other_secret, _) = account_of(0x23);
    let params = json!({ "secret": other_secret, "tx_json": payment(&[]) });
    let ctx = context(&app, &params, &SignSource, RpcRole::Admin);
    let JsonValue::Object(result) = do_sign(&ctx).expect("error payload") else {
        panic!("object");
    };
    assert_eq!(result.get("error"), Some(&json!("badSecret")));
    assert!(result.contains_key("deprecated"));
}

#[test]
fn key_parameters_follow_keypair_for_signature() {
    let app = standalone_app();
    let tx = payment(&[]);
    let error = |params: JsonValue| sign(&app, &params).unwrap_err();
    assert_eq!(
        error(json!({ "tx_json": tx.clone() })),
        RpcStatus::missing_field_error("secret")
    );
    assert_eq!(
        error(
            json!({ "secret": "snoPBrXtMeMyMHUVTgbuqAfg1SUTb", "seed": "x", "tx_json": tx.clone() })
        ),
        RpcStatus::make_param_error(
            "Exactly one of the following must be specified: passphrase, secret, seed or seed_hex"
        )
    );
    assert_eq!(
        error(
            json!({ "secret": "snoPBrXtMeMyMHUVTgbuqAfg1SUTb", "key_type": "secp256k1", "tx_json": tx.clone() })
        ),
        RpcStatus::make_param_error("The secret field is not allowed if key_type is used.")
    );
    // Without key_type rippled reads `secret` whatever field was given.
    assert_eq!(
        error(json!({ "passphrase": "masterpassphrase", "tx_json": tx.clone() })),
        RpcStatus::expected_field_error("secret", "string")
    );
    assert_eq!(
        error(
            json!({ "passphrase": "masterpassphrase", "key_type": "rsa", "tx_json": tx.clone() })
        ),
        RpcStatus::new(RpcErrorCode::BadKeyType)
    );
    assert_eq!(
        error(json!({ "seed_hex": "00", "key_type": "secp256k1", "tx_json": tx.clone() })),
        RpcStatus::new(RpcErrorCode::BadSeed)
    );
    let result = sign(
        &app,
        &json!({ "passphrase": "masterpassphrase", "key_type": "secp256k1", "tx_json": tx }),
    )
    .expect("passphrase with key_type signs");
    assert!(result.contains_key("tx_blob"));
}
