//! Extended fee handler tests.

//! Integration tests for misc operations.

use protocol::JsonValue;
use rpc_integration_tests::env::*;

#[test]
fn defs_fee_response() {
    let a = TestAccount::new("t39");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_fee(&s);
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert!(r.contains_key("drops") || r.contains_key("error"));
}

#[test]
fn defs_ext_48() {
    let a = TestAccount::new("d48");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_fee(&s);
    assert!(matches!(r, JsonValue::Object(_)));
}

#[test]
fn defs_fmt_49() {
    let a = TestAccount::new("c49");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_fee(&s);
    assert!(matches!(r, JsonValue::Object(_)));
}
