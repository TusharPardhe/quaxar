use std::collections::BTreeMap;

use protocol::JsonValue;

pub fn to_protocol_json(value: serde_json::Value) -> JsonValue {
    match value {
        serde_json::Value::Null => JsonValue::Null,
        serde_json::Value::Bool(value) => JsonValue::Bool(value),
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                JsonValue::Signed(value)
            } else if let Some(value) = number.as_u64() {
                JsonValue::Unsigned(value)
            } else {
                JsonValue::String(number.to_string())
            }
        }
        serde_json::Value::String(value) => JsonValue::String(value),
        serde_json::Value::Array(values) => {
            JsonValue::Array(values.into_iter().map(to_protocol_json).collect())
        }
        serde_json::Value::Object(object) => JsonValue::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, to_protocol_json(value)))
                .collect::<BTreeMap<_, _>>(),
        ),
    }
}

pub fn from_protocol_json(value: &JsonValue) -> serde_json::Value {
    match value {
        JsonValue::Null => serde_json::Value::Null,
        JsonValue::Bool(value) => serde_json::Value::Bool(*value),
        JsonValue::Signed(value) => serde_json::Value::Number((*value).into()),
        JsonValue::Unsigned(value) => serde_json::Value::Number((*value).into()),
        JsonValue::String(value) => serde_json::Value::String(value.clone()),
        JsonValue::Array(values) => {
            serde_json::Value::Array(values.iter().map(from_protocol_json).collect())
        }
        JsonValue::Object(object) => serde_json::Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), from_protocol_json(value)))
                .collect(),
        ),
    }
}
