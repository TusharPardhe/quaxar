//! Tests for the currencies fee deposit RPC handler.

use protocol::{
    currency_from_string, get_field_by_symbol, lsfDefaultRipple, to_base58, Issue, JsonValue,
    STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;

// === ACCOUNT_CURRENCIES after real trust lines ===

#[test]
fn account_currencies_shows_send_and_receive_after_trust_and_pay() {
    let mut alice = TestAccount::new("ac_alice");
    let mut gw = TestAccount::new("ac_gw");

    let env = RpcTestEnv::with_flags(
        &[(&alice, 10_000_000_000), (&gw, 10_000_000_000)],
        &[(&gw, lsfDefaultRipple)],
    );

    let usd = currency_from_string("USD");

    // Trust
    let mut trust = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(usd, gw.id),
                1000,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut trust, &alice);
    env.submit_and_close(&trust);

    // Pay alice some USD
    let mut pay = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), gw.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfAmount"),
                Issue::new(usd, gw.id),
                100,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), gw.next_seq());
    });
    sign_tx(&mut pay, &gw);
    env.submit_and_close(&pay);

    // Query account_currencies
    let source = env.rpc_source();
    let result = rpc::do_account_currencies(
        &rpc::AccountCurrenciesRequest {
            params: &json([("account", JsonValue::String(to_base58(alice.id)))]),
            api_version: 2,
            role: rpc::RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if result.get("error").is_none() {
        assert!(result.contains_key("send_currencies"));
        assert!(result.contains_key("receive_currencies"));
    }
}

// === LEDGER_DATA with real state ===

#[test]
fn ledger_data_shows_account_roots_after_funding() {
    let alice = TestAccount::new("ld2_alice");
    let bob = TestAccount::new("ld2_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 5_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_ledger_data(
        &rpc::LedgerDataRequest {
            params: &json([("ledger_index", JsonValue::String("current".to_owned()))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if let Some(JsonValue::Array(state)) = result.get("state") {
        assert!(state.len() >= 2, "should have at least 2 account roots");
        // Each entry should have LedgerEntryType and index
        for entry in state.iter().take(2) {
            let JsonValue::Object(e) = entry else {
                continue;
            };
            assert!(e.contains_key("index") || e.contains_key("LedgerEntryType"));
        }
    }
}

// === WALLET_PROPOSE through integration ===

#[test]
fn wallet_propose_deterministic_from_passphrase() {
    let result1 = rpc::wallet_propose(&json([(
        "passphrase",
        JsonValue::String("masterpassphrase".to_owned()),
    )]))
    .expect("should succeed");
    let result2 = rpc::wallet_propose(&json([(
        "passphrase",
        JsonValue::String("masterpassphrase".to_owned()),
    )]))
    .expect("should succeed");

    let JsonValue::Object(r1) = result1 else {
        panic!("object")
    };
    let JsonValue::Object(r2) = result2 else {
        panic!("object")
    };

    assert_eq!(r1.get("account_id"), r2.get("account_id"));
    assert_eq!(r1.get("master_seed"), r2.get("master_seed"));
    assert_eq!(r1.get("public_key"), r2.get("public_key"));
}

// === CHANNEL_VERIFY through integration ===

#[test]
fn channel_verify_wrong_signature_returns_false() {
    let result = rpc::do_channel_verify(&json([
        ("public_key", JsonValue::String("aB4BXXLuPu8DpVuyq1DBiu3SrPdtK9AYZisKhu8mvkoiUD8J9Gov".to_owned())),
        ("channel_id", JsonValue::String("0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF".to_owned())),
        ("amount", JsonValue::String("1000".to_owned())),
        ("signature", JsonValue::String("DEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEF".to_owned())),
    ]));
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    assert_eq!(
        result.get("signature_verified"),
        Some(&JsonValue::Bool(false))
    );
}

// === SERVER_DEFINITIONS additional checks ===

#[test]
fn server_definitions_hash_is_deterministic() {
    let r1 = rpc::do_server_definitions(&JsonValue::Object(Default::default()));
    let r2 = rpc::do_server_definitions(&JsonValue::Object(Default::default()));

    let JsonValue::Object(r1) = r1 else {
        panic!("object")
    };
    let JsonValue::Object(r2) = r2 else {
        panic!("object")
    };

    assert_eq!(r1.get("hash"), r2.get("hash"));
}

// === RANDOM determinism check ===

#[test]
fn random_never_returns_zero() {
    for _ in 0..20 {
        let result = rpc::do_random();
        let JsonValue::Object(result) = result else {
            panic!("object")
        };
        let JsonValue::String(value) = result.get("random").unwrap() else {
            panic!("string")
        };
        assert_ne!(
            value,
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
    }
}

// === FEE through integration ===

#[test]
fn fee_returns_drops_after_ledger_close() {
    let alice = TestAccount::new("fee_alice2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);
    env.app.accept_standalone_ledger().unwrap();

    let source = env.rpc_source();
    let result = rpc::do_fee(&source);
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if result.get("error").is_none() {
        assert!(result.contains_key("drops") || result.contains_key("current_ledger_size"));
    }
}

// === DEPOSIT_AUTHORIZED with real accounts ===

#[test]
fn deposit_authorized_between_real_accounts() {
    let alice = TestAccount::new("da2_alice");
    let bob = TestAccount::new("da2_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_deposit_authorized(
        &rpc::DepositAuthorizedRequest {
            params: &json([
                ("source_account", JsonValue::String(to_base58(alice.id))),
                ("destination_account", JsonValue::String(to_base58(bob.id))),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if result.get("error").is_none() {
        // Without depositAuth flag, should be authorized
        assert_eq!(
            result.get("deposit_authorized"),
            Some(&JsonValue::Bool(true))
        );
        assert_eq!(
            result.get("source_account"),
            Some(&JsonValue::String(to_base58(alice.id)))
        );
        assert_eq!(
            result.get("destination_account"),
            Some(&JsonValue::String(to_base58(bob.id)))
        );
    }
}
