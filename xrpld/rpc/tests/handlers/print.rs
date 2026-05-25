//! Tests for the print RPC handler.

use std::{cell::RefCell, collections::BTreeMap};

use protocol::JsonValue;
use rpc::{PrintSource, do_print, requested_path};

#[derive(Debug, Default)]
struct FakePrintSource {
    calls: RefCell<Vec<Option<String>>>,
}

impl PrintSource for FakePrintSource {
    fn print_json(&self, path: Option<&str>) -> JsonValue {
        self.calls
            .borrow_mut()
            .push(path.map(std::borrow::ToOwned::to_owned));

        match path {
            Some(path) => JsonValue::Object(BTreeMap::from([
                ("path".to_owned(), JsonValue::String(path.to_owned())),
                ("scope".to_owned(), JsonValue::String("subtree".to_owned())),
            ])),
            None => JsonValue::Object(BTreeMap::from([(
                "scope".to_owned(),
                JsonValue::String("root".to_owned()),
            )])),
        }
    }
}

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[test]
fn print_uses_full_tree_when_params_are_absent() {
    let params = JsonValue::Object(Default::default());
    assert_eq!(requested_path(&params), None);

    let source = FakePrintSource::default();
    let JsonValue::Object(result) = do_print(&params, &source) else {
        panic!("response must be an object");
    };

    assert_eq!(
        result.get("scope"),
        Some(&JsonValue::String("root".to_owned()))
    );
    assert_eq!(source.calls.borrow().as_slice(), &[None]);
}

#[test]
fn print_uses_first_string_param_as_subtree() {
    let params = object([(
        "params",
        JsonValue::Array(vec![
            JsonValue::String("ledger".to_owned()),
            JsonValue::String("ignored".to_owned()),
        ]),
    )]);
    assert_eq!(requested_path(&params), Some("ledger"));

    let source = FakePrintSource::default();
    let JsonValue::Object(result) = do_print(&params, &source) else {
        panic!("response must be an object");
    };

    assert_eq!(
        result.get("path"),
        Some(&JsonValue::String("ledger".to_owned()))
    );
    assert_eq!(
        result.get("scope"),
        Some(&JsonValue::String("subtree".to_owned()))
    );
    assert_eq!(
        source.calls.borrow().as_slice(),
        &[Some("ledger".to_owned())]
    );
}

#[test]
fn print_falls_back_to_full_tree_when_params_are_not_a_string_array() {
    let params = object([("params", JsonValue::Array(vec![JsonValue::Unsigned(1)]))]);
    assert_eq!(requested_path(&params), None);

    let source = FakePrintSource::default();
    let JsonValue::Object(result) = do_print(&params, &source) else {
        panic!("response must be an object");
    };

    assert_eq!(
        result.get("scope"),
        Some(&JsonValue::String("root".to_owned()))
    );
    assert_eq!(source.calls.borrow().as_slice(), &[None]);
}
