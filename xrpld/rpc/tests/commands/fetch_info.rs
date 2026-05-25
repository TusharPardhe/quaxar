//! Tests for fetch info.

use std::{cell::RefCell, collections::BTreeMap};

use protocol::JsonValue;
use rpc::{FetchInfoSource, JsonContext, JsonContextHeaders, RpcRole, do_fetch_info};

#[derive(Debug, Default)]
struct FakeFetchInfoSource {
    clears: RefCell<u32>,
}

impl FetchInfoSource for FakeFetchInfoSource {
    fn clear_ledger_fetch(&self) {
        *self.clears.borrow_mut() += 1;
    }

    fn get_ledger_fetch_info(&self) -> JsonValue {
        JsonValue::Object(BTreeMap::from([(
            "info".to_owned(),
            JsonValue::String("ledger-fetch".to_owned()),
        )]))
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

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

#[test]
fn fetch_info_returns_info_without_clear() {
    let params = JsonValue::Object(Default::default());
    let source = FakeFetchInfoSource::default();
    let result = do_fetch_info(&context(&params, &source, RpcRole::User));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(result.get("clear"), None);
    let JsonValue::Object(info) = result.get("info").expect("info") else {
        panic!("info must be an object");
    };
    assert_eq!(
        info.get("info"),
        Some(&JsonValue::String("ledger-fetch".to_owned()))
    );
    assert_eq!(*source.clears.borrow(), 0);
}

#[test]
fn fetch_info_clears_when_requested() {
    let params = object([("clear", JsonValue::Unsigned(1))]);
    let source = FakeFetchInfoSource::default();
    let result = do_fetch_info(&context(&params, &source, RpcRole::Admin));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(result.get("clear"), Some(&JsonValue::Bool(true)));
    let JsonValue::Object(info) = result.get("info").expect("info") else {
        panic!("info must be an object");
    };
    assert_eq!(
        info.get("info"),
        Some(&JsonValue::String("ledger-fetch".to_owned()))
    );
    assert_eq!(*source.clears.borrow(), 1);
}
