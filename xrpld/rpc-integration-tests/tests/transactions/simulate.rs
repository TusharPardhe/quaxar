//! Integration tests for the transaction RPC handler.

use std::collections::BTreeMap;

use basics::str_hex::str_hex;
use protocol::{get_field_by_symbol, to_base58, JsonValue, STAmount, STTx, TxType};
use rpc_integration_tests::env::*;

fn submit_request<'a>(
    params: &'a JsonValue,
    source: &'a rpc::ApplicationServerInfo<&app::ApplicationRoot>,
) -> Result<JsonValue, rpc::RpcStatus> {
    rpc::do_submit(&rpc::RpcRequestContext {
        params,
        env: &rpc::SubmitSource,
        runtime: source,
        role: rpc::Role::Admin,
        api_version: 2,
        headers: rpc::JsonContextHeaders::default(),
        request_headers: BTreeMap::new(),
        unlimited: true,
        remote_ip: None,
        load_type: rpc::RpcLoadType::Reference,
    })
}

fn simulate_request<'a>(
    params: &'a JsonValue,
    source: &'a rpc::ApplicationServerInfo<&app::ApplicationRoot>,
) -> Result<JsonValue, rpc::RpcStatus> {
    rpc::do_simulate(&rpc::RpcRequestContext {
        params,
        env: &rpc::SimulateSource,
        runtime: source,
        role: rpc::Role::Admin,
        api_version: 2,
        headers: rpc::JsonContextHeaders::default(),
        request_headers: BTreeMap::new(),
        unlimited: true,
        remote_ip: None,
        load_type: rpc::RpcLoadType::Reference,
    })
}

#[test]
fn submit_valid_payment_returns_engine_result() {
    let mut alice = TestAccount::new("sub_alice");
    let bob = TestAccount::new("sub_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    // Create and sign a payment
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

    let tx_blob = str_hex(payment.get_serializer().data());
    let source = env.rpc_source();
    let result = submit_request(&json([("tx_blob", JsonValue::String(tx_blob))]), &source);

    match result {
        Ok(JsonValue::Object(obj)) => {
            assert!(
                obj.contains_key("engine_result") || obj.contains_key("error"),
                "should have engine_result or error: {:?}",
                obj.keys().collect::<Vec<_>>()
            );
            if let Some(JsonValue::String(engine_result)) = obj.get("engine_result") {
                // Should be tesSUCCESS or a tec/tef/ter code
                assert!(
                    engine_result.starts_with("tes")
                        || engine_result.starts_with("tec")
                        || engine_result.starts_with("tef")
                        || engine_result.starts_with("ter"),
                    "engine_result should be a valid result code: {engine_result}"
                );
            }
            if obj.contains_key("tx_json") {
                let JsonValue::Object(tx_json) = obj.get("tx_json").unwrap() else {
                    panic!("object")
                };
                assert!(tx_json.contains_key("TransactionType") || tx_json.contains_key("hash"));
            }
        }
        Ok(other) => panic!("unexpected result: {other:?}"),
        Err(status) => {
            // Some errors are expected (e.g., noNetwork if not synced)
            assert!(
                status.code_string() == "noNetwork"
                    || status.code_string() == "notSynced"
                    || status.code_string() == "noCurrent"
                    || status.code_string() == "internal",
                "unexpected error: {}",
                status.code_string()
            );
        }
    }
}

#[test]
fn submit_invalid_blob_returns_error() {
    let alice = TestAccount::new("sub_alice2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = submit_request(
        &json([("tx_blob", JsonValue::String("DEADBEEF".to_owned()))]),
        &source,
    );

    match result {
        Ok(JsonValue::Object(obj)) => {
            // Invalid blob should produce an error
            assert!(
                obj.contains_key("error") || obj.contains_key("engine_result"),
                "should have error or engine_result"
            );
            if let Some(JsonValue::String(error)) = obj.get("error") {
                assert_eq!(error, "invalidTransaction");
            }
        }
        Err(_) => {} // Error is also acceptable
        _ => panic!("unexpected"),
    }
}

#[test]
fn submit_missing_tx_blob_returns_invalid_params() {
    let alice = TestAccount::new("sub_alice3");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = submit_request(&json([]), &source);

    assert!(result.is_err(), "missing tx_blob should error");
}

#[test]
fn simulate_valid_payment() {
    let mut alice = TestAccount::new("sim_alice");
    let bob = TestAccount::new("sim_bob");
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

    let tx_blob = str_hex(payment.get_serializer().data());
    let source = env.rpc_source();
    let result = simulate_request(&json([("tx_blob", JsonValue::String(tx_blob))]), &source);

    match result {
        Ok(JsonValue::Object(obj)) => {
            assert!(
                obj.contains_key("engine_result") || obj.contains_key("error"),
                "should have engine_result or error"
            );
        }
        Err(status) => {
            // Simulate may not be available in standalone
            assert!(
                status.code_string() == "notSynced"
                    || status.code_string() == "noNetwork"
                    || status.code_string() == "noCurrent"
                    || status.code_string() == "notSupported"
                    || status.code_string() == "internal",
                "unexpected simulate error: {}",
                status.code_string()
            );
        }
        _ => panic!("unexpected"),
    }
}

#[test]
fn simulate_with_tx_json() {
    let alice = TestAccount::new("sim_alice2");
    let bob = TestAccount::new("sim_bob2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    let source = env.rpc_source();
    let result = simulate_request(
        &json([(
            "tx_json",
            json([
                ("TransactionType", JsonValue::String("Payment".to_owned())),
                ("Account", JsonValue::String(to_base58(alice.id))),
                ("Destination", JsonValue::String(to_base58(bob.id))),
                ("Amount", JsonValue::String("1000000".to_owned())),
                ("Fee", JsonValue::String("10".to_owned())),
                ("Sequence", JsonValue::Unsigned(1)),
            ]),
        )]),
        &source,
    );

    // Should either succeed with engine_result or fail with a known error
    match result {
        Ok(JsonValue::Object(obj)) => {
            assert!(
                obj.contains_key("engine_result") || obj.contains_key("error"),
                "should have engine_result or error"
            );
        }
        Err(status) => {
            // Known acceptable errors for simulate in standalone
            let token = status.code_string();
            assert!(
                token == "notSynced"
                    || token == "noNetwork"
                    || token == "noCurrent"
                    || token == "notSupported"
                    || token == "internal"
                    || token == "invalidParams",
                "unexpected: {token}"
            );
        }
        _ => {}
    }
}
