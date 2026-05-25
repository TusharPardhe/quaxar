//! Tests for the validators RPC handler.

use std::{cell::RefCell, collections::BTreeMap};

use protocol::JsonValue;
use rpc::{JsonContext, JsonContextHeaders, RpcRole, ValidatorsSource, do_validators};

#[derive(Debug, Default)]
struct FakeValidatorsSource {
    calls: RefCell<u32>,
}

impl ValidatorsSource for FakeValidatorsSource {
    fn get_validators(&self) -> JsonValue {
        *self.calls.borrow_mut() += 1;
        JsonValue::Object(BTreeMap::from([
            ("validators".to_owned(), JsonValue::Unsigned(3)),
            ("status".to_owned(), JsonValue::String("ok".to_owned())),
        ]))
    }
}

fn context<'a, Env>(params: &'a JsonValue, env: &'a Env, role: RpcRole) -> JsonContext<'a, Env> {
    JsonContext {
        params,
        env,
        role,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
    }
}

#[test]
fn validators_returns_source_json() {
    let params = JsonValue::Object(Default::default());
    let source = FakeValidatorsSource::default();
    let result = do_validators(&context(&params, &source, RpcRole::Admin));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(result.get("validators"), Some(&JsonValue::Unsigned(3)));
    assert_eq!(
        result.get("status"),
        Some(&JsonValue::String("ok".to_owned()))
    );
    assert_eq!(*source.calls.borrow(), 1);
}

#[test]
fn validators_returns_local_static_keys() {
    let params = JsonValue::Object(Default::default());
    let source = FakeValidatorsSource::default();
    let result = do_validators(&context(&params, &source, RpcRole::Admin));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    // Should not have error
    assert!(!result.contains_key("error"));
}

#[test]
fn validators_non_admin_can_read() {
    let params = JsonValue::Object(Default::default());
    let source = FakeValidatorsSource::default();
    let result = do_validators(&context(&params, &source, RpcRole::User));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    // validators is a public command
    assert!(!result.contains_key("error"));
    assert_eq!(*source.calls.borrow(), 1);
}
