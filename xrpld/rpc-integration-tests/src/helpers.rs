//! Shared test helper functions for JSON value construction.

use protocol::JsonValue;

/// Create a JSON string value.
pub fn sv(v: &str) -> JsonValue {
    JsonValue::String(v.to_owned())
}

/// Create a JSON signed integer value.
pub fn si(v: i64) -> JsonValue {
    JsonValue::Signed(v)
}

/// Create a JSON unsigned integer value.
pub fn u(v: u64) -> JsonValue {
    JsonValue::Unsigned(v)
}

/// Create a JSON boolean value.
pub fn b(v: bool) -> JsonValue {
    JsonValue::Bool(v)
}

/// Create an empty JSON object.
pub fn obj() -> JsonValue {
    JsonValue::Object(Default::default())
}
