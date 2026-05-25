use std::collections::BTreeMap;

use protocol::JsonValue;

pub(crate) type JsonObject = BTreeMap<String, JsonValue>;

pub(crate) fn to_protocol_json(value: serde_json::Value) -> JsonValue {
    match value {
        serde_json::Value::Null => JsonValue::Null,
        serde_json::Value::Bool(value) => JsonValue::Bool(value),
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                JsonValue::Unsigned(value)
            } else if let Some(value) = number.as_i64() {
                JsonValue::Signed(value)
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

pub(crate) fn insert_serde_json_field(
    object: &mut JsonObject,
    key: &str,
    value: serde_json::Value,
) {
    object.insert(key.to_owned(), to_protocol_json(value));
}

pub(crate) fn with_object_field(value: JsonValue, key: &str, field_value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(mut object) => {
            object.insert(key.to_owned(), field_value);
            JsonValue::Object(object)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{insert_serde_json_field, with_object_field};
    use protocol::JsonValue;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn insert_serde_json_field_converts_and_inserts_values() {
        let mut object = BTreeMap::new();

        insert_serde_json_field(&mut object, "load", json!({"job_count": "3"}));

        let JsonValue::Object(load) = object.get("load").expect("load field must exist") else {
            panic!("load field must be an object");
        };
        assert_eq!(
            load.get("job_count"),
            Some(&JsonValue::String("3".to_owned()))
        );
    }

    #[test]
    fn with_object_field_only_extends_object_values() {
        let extended = with_object_field(
            JsonValue::Object(BTreeMap::from([(
                "rpc".to_owned(),
                JsonValue::Object(BTreeMap::new()),
            )])),
            "nodestore",
            JsonValue::Object(BTreeMap::from([(
                "entries".to_owned(),
                JsonValue::String("5".to_owned()),
            )])),
        );
        let JsonValue::Object(extended) = extended else {
            panic!("extended value must stay an object");
        };
        assert!(extended.contains_key("rpc"));
        assert!(extended.contains_key("nodestore"));

        let untouched = with_object_field(
            JsonValue::String("not-an-object".to_owned()),
            "nodestore",
            JsonValue::Null,
        );
        assert_eq!(untouched, JsonValue::String("not-an-object".to_owned()));
    }
}
