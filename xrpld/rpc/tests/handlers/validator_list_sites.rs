//! Tests for the validator list sites RPC handler.

use std::{cell::RefCell, collections::BTreeMap};

use protocol::JsonValue;
use rpc::{
    JsonContext, JsonContextHeaders, RpcRole, ValidatorListSitesSource, do_validator_list_sites,
};

#[derive(Debug, Default)]
struct FakeValidatorListSitesSource {
    calls: RefCell<u32>,
}

impl ValidatorListSitesSource for FakeValidatorListSitesSource {
    fn get_validator_list_sites(&self) -> JsonValue {
        *self.calls.borrow_mut() += 1;
        JsonValue::Object(BTreeMap::from([
            ("sites".to_owned(), JsonValue::Unsigned(2)),
            ("active".to_owned(), JsonValue::Bool(true)),
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
fn validator_list_sites_returns_source_json() {
    let params = JsonValue::Object(Default::default());
    let source = FakeValidatorListSitesSource::default();
    let result = do_validator_list_sites(&context(&params, &source, RpcRole::User));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(result.get("sites"), Some(&JsonValue::Unsigned(2)));
    assert_eq!(result.get("active"), Some(&JsonValue::Bool(true)));
    assert_eq!(*source.calls.borrow(), 1);
}
