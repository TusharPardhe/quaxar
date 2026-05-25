//! Narrow `print` RPC handler port.
//!
//! The reference handler writes through `JsonPropertyStream` and optionally targets
//! a named subtree based on the first string in the `params` array.  The Rust
//! seam keeps that dispatch explicit while delegating the actual output tree
//! to an injected source trait.

use protocol::JsonValue;

pub trait PrintSource {
    fn print_json(&self, path: Option<&str>) -> JsonValue;
}

pub fn requested_path(params: &JsonValue) -> Option<&str> {
    let JsonValue::Object(object) = params else {
        return None;
    };

    let Some(JsonValue::Array(values)) = object.get("params") else {
        return None;
    };

    let Some(JsonValue::String(path)) = values.first() else {
        return None;
    };

    Some(path.as_str())
}

pub fn do_print<S: PrintSource>(params: &JsonValue, source: &S) -> JsonValue {
    source.print_json(requested_path(params))
}
