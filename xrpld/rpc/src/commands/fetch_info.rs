//! Narrow `fetch_info` RPC wrapper.

use std::collections::BTreeMap;

use protocol::JsonValue;

use crate::JsonContext;

pub trait FetchInfoSource {
    fn clear_ledger_fetch(&self);

    fn get_ledger_fetch_info(&self) -> JsonValue;
}

fn json_value_as_bool(value: &JsonValue) -> bool {
    match value {
        JsonValue::Bool(value) => *value,
        JsonValue::Signed(value) => *value != 0,
        JsonValue::Unsigned(value) => *value != 0,
        JsonValue::String(value) => !value.is_empty(),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => false,
    }
}

pub fn do_fetch_info<S: FetchInfoSource>(context: &JsonContext<'_, S>) -> JsonValue {
    let mut result = BTreeMap::new();

    if let JsonValue::Object(object) = context.params
        && object.get("clear").is_some_and(json_value_as_bool)
    {
        context.env.clear_ledger_fetch();
        result.insert("clear".to_owned(), JsonValue::Bool(true));
    }

    result.insert("info".to_owned(), context.env.get_ledger_fetch_info());

    JsonValue::Object(result)
}
