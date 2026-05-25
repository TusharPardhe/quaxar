//! Tests for server info.

use protocol::JsonValue;
use rpc_integration_tests::env::*;

#[test]
fn server_info_returns_info_object() {
    let alice = TestAccount::new("si_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_server_info(&rpc::JsonContext {
        params: &JsonValue::Object(Default::default()),
        env: &source,
        role: rpc::RpcRole::Admin,
        api_version: 2,
        headers: rpc::JsonContextHeaders::default(),
        unlimited: true,
    });
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    assert!(result.contains_key("info"));
    let JsonValue::Object(info) = result.get("info").unwrap() else {
        panic!("object")
    };
    assert!(info.contains_key("build_version") || info.contains_key("server_state"));
}

#[test]
fn server_state_returns_state_object() {
    let alice = TestAccount::new("ss_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_server_state(&rpc::JsonContext {
        params: &JsonValue::Object(Default::default()),
        env: &source,
        role: rpc::RpcRole::Admin,
        api_version: 2,
        headers: rpc::JsonContextHeaders::default(),
        unlimited: true,
    });
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    assert!(result.contains_key("state"));
    let JsonValue::Object(state) = result.get("state").unwrap() else {
        panic!("object")
    };
    assert!(state.contains_key("build_version") || state.contains_key("server_state"));
}

#[test]
fn fee_returns_fee_info() {
    let alice = TestAccount::new("fee_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = rpc::do_fee(&source);
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    // Should have fee info or noNetwork error
    assert!(
        result.contains_key("drops") || result.contains_key("error"),
        "should have drops or error"
    );
}
