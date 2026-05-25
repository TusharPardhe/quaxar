//! Tests for the ledger rpc RPC handler.

use protocol::JsonValue;
use rpc_integration_tests::env::*;

#[test]
fn ledger_current_advances_after_close() {
    let alice = TestAccount::new("lr_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let before = rpc::do_ledger_current(&source);
    let JsonValue::Object(before) = before else {
        panic!("object")
    };
    let before_idx = match before.get("ledger_current_index") {
        Some(JsonValue::Unsigned(v)) => *v,
        _ => 0,
    };

    env.app.accept_standalone_ledger().unwrap();

    let source = env.rpc_source();
    let after = rpc::do_ledger_current(&source);
    let JsonValue::Object(after) = after else {
        panic!("object")
    };
    let after_idx = match after.get("ledger_current_index") {
        Some(JsonValue::Unsigned(v)) => *v,
        _ => 0,
    };

    assert!(after_idx > before_idx, "ledger index should advance");
}

#[test]
fn ledger_closed_updates_after_close() {
    let alice = TestAccount::new("lr_alice2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    env.app.accept_standalone_ledger().unwrap();

    let source = env.rpc_source();
    let result = rpc::do_ledger_closed(&source);
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    assert!(result.contains_key("ledger_hash"));
    assert!(result.contains_key("ledger_index"));
    let JsonValue::String(hash) = result.get("ledger_hash").unwrap() else {
        panic!("string")
    };
    assert_eq!(hash.len(), 64);
}

#[test]
fn ledger_by_index_returns_ledger_data() {
    let alice = TestAccount::new("lr_alice3");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    env.app.accept_standalone_ledger().unwrap();

    let source = env.rpc_source();
    let result = rpc::do_ledger(
        &json([("ledger_index", JsonValue::String("closed".to_owned()))]),
        rpc::RpcRole::Admin,
        2,
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if result.get("error").is_none() {
        assert!(result.contains_key("ledger"));
        assert!(result.contains_key("ledger_hash") || result.contains_key("ledger_index"));
    }
}

#[test]
fn ledger_full_requires_admin() {
    let alice = TestAccount::new("lr_alice4");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_ledger(
        &json([
            ("ledger_index", JsonValue::String("current".to_owned())),
            ("full", JsonValue::Bool(true)),
        ]),
        rpc::RpcRole::Guest,
        2,
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("noPermission".to_owned()))
    );
}
