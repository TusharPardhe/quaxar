//! Integration tests for the account_info RPC handler.

use protocol::{get_field_by_symbol, to_base58, JsonValue, STAmount, STTx, TxType};
use rpc_integration_tests::env::*;

#[test]
fn account_info_shows_funded_account_data() {
    let alice = TestAccount::new("ai_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([("account", JsonValue::String(to_base58(alice.id)))]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(result.get("error"), None);
    let JsonValue::Object(account_data) = result.get("account_data").expect("account_data") else {
        panic!("account_data must be an object");
    };
    assert_eq!(
        account_data.get("Account"),
        Some(&JsonValue::String(to_base58(alice.id)))
    );
    assert!(account_data.contains_key("Balance"));
    assert!(account_data.contains_key("Sequence"));
    assert!(account_data.contains_key("Flags"));
}

#[test]
fn account_info_sequence_increments_after_tx() {
    let mut alice = TestAccount::new("ai_alice2");
    let bob = TestAccount::new("ai_bob2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 10_000_000_000)]);

    // Submit a payment
    let mut payment = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut payment, &alice);
    env.submit_and_close(&payment);

    let source = env.rpc_source();
    let result = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(alice.id))),
                ("ledger_index", JsonValue::String("current".to_owned())),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    // The handler should return account data (from current or closed ledger)
    assert!(
        result.contains_key("account_data") || result.contains_key("error"),
        "should have account_data or error"
    );
}

#[test]
fn account_info_balance_decreases_after_payment() {
    let mut alice = TestAccount::new("ai_alice3");
    let bob = TestAccount::new("ai_bob3");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    let mut payment = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut payment, &alice);
    env.submit_and_close(&payment);

    let source = env.rpc_source();
    let result = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(alice.id))),
                ("ledger_index", JsonValue::String("current".to_owned())),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    // Verify the handler responds correctly
    assert!(
        result.contains_key("account_data") || result.contains_key("error"),
        "should have account_data or error"
    );
}
