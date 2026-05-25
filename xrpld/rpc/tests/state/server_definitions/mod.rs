//! Tests for server definitions.

pub(super) use std::collections::BTreeMap;

pub(super) use protocol::JsonValue;
pub(super) use rpc::do_server_definitions;

pub(super) fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

pub(super) fn get_object(value: &JsonValue) -> &BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = value else {
        panic!("expected object");
    };
    object
}

pub(super) fn get_array(value: &JsonValue) -> &[JsonValue] {
    let JsonValue::Array(array) = value else {
        panic!("expected array");
    };
    array
}

pub(super) fn get_str(value: &JsonValue) -> &str {
    let JsonValue::String(text) = value else {
        panic!("expected string");
    };
    text
}

mod fields_and_formats;
mod hash_and_sections;
