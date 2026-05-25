//! Narrow `blacklist` RPC handler port.
//!
//! The reference handler is a thin wrapper around `ResourceManager::getJson(...)`.
//! Rust keeps that exact split: the handler only chooses whether a threshold
//! was supplied, and the injected source owns the JSON rendering.

use protocol::JsonValue;

fn threshold_from_params(params: &JsonValue) -> Option<i64> {
    let JsonValue::Object(object) = params else {
        return None;
    };

    let Some(value) = object.get("threshold") else {
        return None;
    };

    match value {
        JsonValue::Signed(value) => Some(*value),
        JsonValue::Unsigned(value) => Some(i64::try_from(*value).unwrap_or(i64::MAX)),
        _ => Some(0),
    }
}

pub trait BlackListSource {
    fn black_list_json(&self) -> JsonValue;

    fn black_list_json_with_threshold(&self, threshold: i64) -> JsonValue;
}

pub fn do_black_list<S: BlackListSource>(params: &JsonValue, source: &S) -> JsonValue {
    match threshold_from_params(params) {
        Some(threshold) => source.black_list_json_with_threshold(threshold),
        None => source.black_list_json(),
    }
}
