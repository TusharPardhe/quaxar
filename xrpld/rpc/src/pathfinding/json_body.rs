//! json_body equivalent ported from `xrpld/rpc/json_body.h`.
//!
//! In reference this is a Boost.Beast HTTP body type that serializes/deserializes
//! JSON for the HTTP server handler. In Rust (using hyper/axum), this maps to
//! a simple JSON body extractor/responder pattern.
//!
//! This module provides:
//! - `JsonBody`: a wrapper that can serialize a `JsonValue` to bytes for HTTP responses
//! - `parse_json_body`: parse raw bytes into a `JsonValue` for HTTP requests

#![allow(dead_code)]

use protocol::JsonValue;

/// A JSON HTTP body that wraps a `JsonValue` for serialization into HTTP responses.
///
/// In reference this is `struct JsonBody` with a `reader` (for responses) and `writer`
/// (for requests). In Rust we just provide serialize/deserialize helpers since
/// the HTTP framework handles the body transport.
#[derive(Debug, Clone)]
pub struct JsonBody {
    value: JsonValue,
}

impl JsonBody {
    /// Create a new JsonBody from a JsonValue.
    pub fn new(value: JsonValue) -> Self {
        Self { value }
    }

    /// Get a reference to the inner JsonValue.
    pub fn value(&self) -> &JsonValue {
        &self.value
    }

    /// Consume and return the inner JsonValue.
    pub fn into_value(self) -> JsonValue {
        self.value
    }

    /// Serialize the JSON value to bytes (UTF-8).
    /// This is the equivalent of the reference `reader` which streams the body.
    pub fn to_bytes(&self) -> Vec<u8> {
        protocol::serde_json::to_string(&self.value)
            .unwrap_or_default()
            .into_bytes()
    }

    /// Get the content length of the serialized body.
    pub fn content_length(&self) -> usize {
        self.to_bytes().len()
    }
}

/// Parse raw bytes into a JsonValue.
/// This is the equivalent of the reference `writer` which parses incoming body bytes.
///
/// Returns `None` if the bytes are not valid JSON.
pub fn parse_json_body(bytes: &[u8]) -> Option<JsonValue> {
    let s = std::str::from_utf8(bytes).ok()?;
    // Use serde_json to parse, then convert to protocol JsonValue
    let serde_val: protocol::serde_json::Value = protocol::serde_json::from_str(s).ok()?;
    Some(protocol::JsonValue::from(serde_val))
}

/// Content type for JSON HTTP bodies.
pub const JSON_CONTENT_TYPE: &str = "application/json";

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn round_trip() {
        let mut map = BTreeMap::new();
        map.insert("key".to_owned(), JsonValue::String("value".to_owned()));
        let value = JsonValue::Object(map);

        let body = JsonBody::new(value.clone());
        assert_eq!(body.value(), &value);

        let bytes = body.to_bytes();
        assert!(!bytes.is_empty());
    }
}
