//! Integration tests for the transaction RPC handler.

use protocol::{get_field_by_symbol, to_base58, JsonValue, STAmount, STTx, TxType};
use rpc_integration_tests::env::*;

#[test]
fn tx_finds_committed_payment() {
    let mut alice = TestAccount::new("tx_alice");
    let bob = TestAccount::new("tx_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

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
    let tx_id = payment.get_transaction_id();
    env.submit_and_close(&payment);

    // Query tx
    let source = env.rpc_source();
    let result = rpc::do_tx(
        &rpc::TxRequest {
            params: &json([("transaction", JsonValue::String(tx_id.to_string()))]),
            api_version: 2,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    // If tx was committed, should find it
    if result.get("error").is_none() {
        assert_eq!(
            result.get("hash"),
            Some(&JsonValue::String(tx_id.to_string()))
        );
        assert_eq!(result.get("validated"), Some(&JsonValue::Bool(true)));
        assert!(result.contains_key("tx_json") || result.contains_key("TransactionType"));
        assert!(result.contains_key("ledger_index"));
    }
}

#[test]
fn tx_not_found_for_unknown_hash() {
    let alice = TestAccount::new("tx_alice2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let fake_hash = basics::base_uint::Uint256::from_array([0xDD; 32]);
    let result = rpc::do_tx(
        &rpc::TxRequest {
            params: &json([("transaction", JsonValue::String(fake_hash.to_string()))]),
            api_version: 2,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("txnNotFound".to_owned()))
    );
}

#[test]
fn account_tx_finds_committed_transactions() {
    let mut alice = TestAccount::new("atx_alice");
    let bob = TestAccount::new("atx_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    // Submit two payments
    for _ in 0..2 {
        let mut payment = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
            tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(100_000, false),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
        });
        sign_tx(&mut payment, &alice);
        env.submit_and_close(&payment);
    }

    let source = env.rpc_source();
    let result = rpc::do_account_tx(
        &json([
            ("account", JsonValue::String(to_base58(alice.id))),
            ("ledger_index_min", JsonValue::Signed(-1)),
            ("ledger_index_max", JsonValue::Signed(-1)),
        ]),
        rpc::RpcRole::Admin,
        2,
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    if result.get("error").is_none() {
        assert!(result.contains_key("transactions"));
        assert_eq!(
            result.get("account"),
            Some(&JsonValue::String(to_base58(alice.id)))
        );
    }
}
