use std::collections::BTreeMap;
use std::fmt;

use protocol::JsonValue;
use serde::de::{self, Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, Serializer};

pub fn to_protocol_json(value: serde_json::Value) -> JsonValue {
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

// ---------------------------------------------------------------------------
// Inbound: single-pass parse straight into `JsonValue`.
//
// Semantics are identical to `to_protocol_json(serde_json::from_slice(..))`:
// non-negative integers become `Unsigned`, negative `Signed`, and any other
// number (fraction, exponent, or integer outside the 64-bit range) becomes a
// `String` formatted exactly as `serde_json::Number` displays it. Duplicate
// object keys keep the last value, as `serde_json::Map` does.
// ---------------------------------------------------------------------------

/// `JsonValue` that deserializes directly from any serde JSON parser without
/// building an intermediate `serde_json::Value` tree.
pub struct ProtoJson(pub JsonValue);

fn float_string(value: f64) -> JsonValue {
    JsonValue::String(
        serde_json::Number::from_f64(value)
            .map(|number| number.to_string())
            .unwrap_or_default(),
    )
}

struct ProtoJsonVisitor;

impl<'de> Visitor<'de> for ProtoJsonVisitor {
    type Value = JsonValue;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<JsonValue, E> {
        Ok(JsonValue::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<JsonValue, E> {
        Ok(if v >= 0 {
            JsonValue::Unsigned(v as u64)
        } else {
            JsonValue::Signed(v)
        })
    }

    fn visit_u64<E>(self, v: u64) -> Result<JsonValue, E> {
        Ok(JsonValue::Unsigned(v))
    }

    fn visit_f64<E>(self, v: f64) -> Result<JsonValue, E> {
        Ok(float_string(v))
    }

    fn visit_str<E>(self, v: &str) -> Result<JsonValue, E> {
        Ok(JsonValue::String(v.to_owned()))
    }

    fn visit_string<E>(self, v: String) -> Result<JsonValue, E> {
        Ok(JsonValue::String(v))
    }

    fn visit_unit<E>(self) -> Result<JsonValue, E> {
        Ok(JsonValue::Null)
    }

    fn visit_none<E>(self) -> Result<JsonValue, E> {
        Ok(JsonValue::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<JsonValue, D::Error> {
        d.deserialize_any(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<JsonValue, A::Error> {
        let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(4096));
        while let Some(ProtoJson(value)) = seq.next_element()? {
            values.push(value);
        }
        Ok(JsonValue::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JsonValue, A::Error> {
        let mut object = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            let ProtoJson(value) = map.next_value()?;
            object.insert(key, value);
        }
        Ok(JsonValue::Object(object))
    }
}

impl<'de> Deserialize<'de> for ProtoJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer
            .deserialize_any(ProtoJsonVisitor)
            .map(ProtoJson)
    }
}

/// Top-level request document parsed in one pass.
///
/// `id` is additionally retained as a `serde_json::Value` so it is echoed
/// back byte-for-byte as before (including fractional ids). Every other member
/// lands directly in the protocol representation.
pub enum RawRequest {
    Object {
        id: Option<serde_json::Value>,
        fields: BTreeMap<String, JsonValue>,
    },
    /// Valid JSON whose top level is not an object.
    NotObject,
}

struct RawRequestVisitor;

impl<'de> Visitor<'de> for RawRequestVisitor {
    type Value = RawRequest;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON document")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawRequest, A::Error> {
        let mut id = None;
        let mut fields = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            if key == "id" {
                let value: serde_json::Value = map.next_value()?;
                fields.insert(key, to_protocol_json(value.clone()));
                id = Some(value);
            } else {
                let ProtoJson(value) = map.next_value()?;
                fields.insert(key, value);
            }
        }
        Ok(RawRequest::Object { id, fields })
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<RawRequest, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(RawRequest::NotObject)
    }

    fn visit_bool<E>(self, _: bool) -> Result<RawRequest, E> {
        Ok(RawRequest::NotObject)
    }

    fn visit_i64<E>(self, _: i64) -> Result<RawRequest, E> {
        Ok(RawRequest::NotObject)
    }

    fn visit_u64<E>(self, _: u64) -> Result<RawRequest, E> {
        Ok(RawRequest::NotObject)
    }

    fn visit_f64<E>(self, _: f64) -> Result<RawRequest, E> {
        Ok(RawRequest::NotObject)
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<RawRequest, E> {
        Ok(RawRequest::NotObject)
    }

    fn visit_unit<E>(self) -> Result<RawRequest, E> {
        Ok(RawRequest::NotObject)
    }
}

impl<'de> Deserialize<'de> for RawRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(RawRequestVisitor)
    }
}

impl RawRequest {
    /// Parse a request body with the SIMD parser in a single pass.
    pub fn parse(bytes: &[u8]) -> Result<Self, sonic_rs::Error> {
        sonic_rs::from_slice(bytes)
    }

    /// Adapt an already-parsed `serde_json::Value` (used by `/v2/batch`).
    pub fn from_value(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Object(map) => {
                let id = map.get("id").cloned();
                let fields = map
                    .into_iter()
                    .map(|(key, value)| (key, to_protocol_json(value)))
                    .collect();
                RawRequest::Object { id, fields }
            }
            _ => RawRequest::NotObject,
        }
    }
}

// ---------------------------------------------------------------------------
// Outbound: serialize `JsonValue` results directly inside the response
// envelope, without rebuilding a `serde_json::Value` tree.
//
// `serde_json::Map` (no `preserve_order` feature in this workspace) is a
// `BTreeMap`, so the previous output had lexicographically sorted keys at
// every level. `JsonValue::Object` is also a `BTreeMap`, and `Envelope`
// sorts its members, so the produced bytes are identical.
// ---------------------------------------------------------------------------

/// One member of a response envelope.
pub enum EnvelopeField<'a> {
    Json(&'a serde_json::Value),
    Proto(&'a JsonValue),
    /// A JSON-RPC `result`: a `status` member is added to objects that lack
    /// one (`"error"` if an `error` member exists, otherwise `"success"`).
    ProtoWithDefaultStatus(&'a JsonValue),
    Str(&'a str),
    U32(u32),
    Null,
}

impl Serialize for EnvelopeField<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Json(value) => value.serialize(serializer),
            Self::Proto(value) => value.serialize(serializer),
            Self::ProtoWithDefaultStatus(value) => {
                ResultWithDefaultStatus(value).serialize(serializer)
            }
            Self::Str(value) => serializer.serialize_str(value),
            Self::U32(value) => serializer.serialize_u32(*value),
            Self::Null => serializer.serialize_unit(),
        }
    }
}

struct ResultWithDefaultStatus<'a>(&'a JsonValue);

impl Serialize for ResultWithDefaultStatus<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let JsonValue::Object(object) = self.0 else {
            return self.0.serialize(serializer);
        };
        if object.contains_key("status") {
            return self.0.serialize(serializer);
        }
        let status = if object.contains_key("error") {
            "error"
        } else {
            "success"
        };
        let mut map = serializer.serialize_map(Some(object.len() + 1))?;
        let mut pending = Some(status);
        for (key, value) in object {
            if let Some(status) = pending
                && key.as_str() > "status"
            {
                map.serialize_entry("status", status)?;
                pending = None;
            }
            map.serialize_entry(key, value)?;
        }
        if let Some(status) = pending {
            map.serialize_entry("status", status)?;
        }
        map.end()
    }
}

/// Response envelope serialized with members in sorted key order.
pub struct Envelope<'a> {
    fields: Vec<(&'a str, EnvelopeField<'a>)>,
}

impl<'a> Envelope<'a> {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            fields: Vec::with_capacity(capacity),
        }
    }

    /// Insert or replace a member (later inserts win, as with a map).
    pub fn insert(&mut self, key: &'a str, value: EnvelopeField<'a>) -> &mut Self {
        if let Some(slot) = self.fields.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.fields.push((key, value));
        }
        self
    }
}

impl Serialize for Envelope<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut order: Vec<usize> = (0..self.fields.len()).collect();
        order.sort_unstable_by(|a, b| self.fields[*a].0.cmp(self.fields[*b].0));
        let mut map = serializer.serialize_map(Some(self.fields.len()))?;
        for index in order {
            let (key, value) = &self.fields[index];
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// True when the JSON-RPC result (after default-status injection) reports an
/// error and therefore needs the `request` echo slow path.
pub fn result_reports_error(result: &JsonValue) -> bool {
    match result {
        JsonValue::Object(object) => {
            object.contains_key("error")
                || matches!(object.get("status"), Some(JsonValue::String(s)) if s == "error")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn old_parse(bytes: &[u8]) -> Option<JsonValue> {
        serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .map(to_protocol_json)
    }

    fn new_parse(bytes: &[u8]) -> Option<JsonValue> {
        sonic_rs::from_slice::<ProtoJson>(bytes).ok().map(|v| v.0)
    }

    #[test]
    fn single_pass_parse_matches_two_pass_conversion() {
        let corpus: &[&str] = &[
            r#"{"method":"account_info","params":[{"account":"rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh","ledger_index":"validated"}],"id":1}"#,
            r#"{"command":"ledger","ledger_index":21000000,"transactions":true,"expand":false,"id":"abc"}"#,
            r#"{"a":0,"b":-1,"c":18446744073709551615,"d":-9223372036854775808,"e":18446744073709551616,"f":1.5,"g":1e20,"i":2.5e-7,"j":123456789012345678901234567890}"#,
            r#"{"dup":1,"dup":2,"nested":{"z":[1,{"y":null,"x":true}],"a":"\u00e9\n\"q\""}}"#,
            r#"[1,2,3]"#,
            r#""str""#,
            r#"null"#,
            r#"{"unicode":"\ud83d\ude00","esc":"\\/\b\f\r\t"}"#,
            r#"{"deep":[[[[[[[[[[{"k":[[[[1]]]]}]]]]]]]]]]}"#,
            "  {\"ws\" :\t[ ]\n}  ",
        ];
        for doc in corpus {
            assert_eq!(
                new_parse(doc.as_bytes()),
                old_parse(doc.as_bytes()),
                "{doc}"
            );
        }
    }

    #[test]
    fn negative_zero_is_the_only_known_numeric_divergence() {
        // serde_json keeps the sign of -0 / -0.0 (rendered "-0.0"); sonic-rs
        // parses it as +0.0. Both stay non-integer strings, which every
        // integer-typed RPC field already rejects.
        for doc in [&b"-0"[..], b"-0.0"] {
            assert_eq!(old_parse(doc), Some(JsonValue::String("-0.0".to_owned())));
            assert_eq!(new_parse(doc), Some(JsonValue::String("0.0".to_owned())));
        }
    }

    #[test]
    fn single_pass_parse_rejects_what_serde_json_rejects() {
        let bad: &[&[u8]] = &[
            b"",
            b"{",
            b"{\"a\":}",
            b"{\"a\":1}trailing",
            b"{'a':1}",
            b"{\"a\":01}",
            b"[1,]",
            b"{\"a\":\"\xff\"}",
            b"{\"a\":NaN}",
        ];
        for doc in bad {
            assert!(old_parse(doc).is_none());
            assert!(
                new_parse(doc).is_none(),
                "{:?}",
                String::from_utf8_lossy(doc)
            );
        }
    }

    #[test]
    fn raw_request_keeps_exact_id_and_protocol_fields() {
        let RawRequest::Object { id, fields } =
            RawRequest::parse(br#"{"command":"ping","id":1.25,"api_version":2}"#).unwrap()
        else {
            panic!("object expected");
        };
        assert_eq!(id, Some(serde_json::json!(1.25)));
        assert_eq!(
            fields.get("id"),
            Some(&JsonValue::String("1.25".to_owned()))
        );
        assert_eq!(fields.get("api_version"), Some(&JsonValue::Unsigned(2)));
        assert!(matches!(
            RawRequest::parse(b"[1]").unwrap(),
            RawRequest::NotObject
        ));
        assert!(matches!(
            RawRequest::parse(b"7").unwrap(),
            RawRequest::NotObject
        ));
    }

    fn sample_result() -> JsonValue {
        let mut inner = BTreeMap::new();
        inner.insert("zeta".to_owned(), JsonValue::Signed(-5));
        inner.insert("alpha".to_owned(), JsonValue::Unsigned(u64::MAX));
        inner.insert(
            "txt".to_owned(),
            JsonValue::String("q\"\\\u{1}é".to_owned()),
        );
        let mut object = BTreeMap::new();
        object.insert("account".to_owned(), JsonValue::String("r".to_owned()));
        object.insert("validated".to_owned(), JsonValue::Bool(true));
        object.insert("nested".to_owned(), JsonValue::Object(inner));
        object.insert(
            "list".to_owned(),
            JsonValue::Array(vec![JsonValue::Null, JsonValue::Unsigned(0)]),
        );
        JsonValue::Object(object)
    }

    #[test]
    fn direct_envelope_serialization_is_byte_identical_to_value_path() {
        for result in [
            sample_result(),
            JsonValue::Object(BTreeMap::new()),
            JsonValue::Object(BTreeMap::from([(
                "status".to_owned(),
                JsonValue::String("custom".to_owned()),
            )])),
            JsonValue::Unsigned(3),
        ] {
            let id = serde_json::json!({"x": [1, 2.5]});
            // Old path: json_rpc_response + result_with_default_status.
            let mut old = serde_json::Map::new();
            old.insert("jsonrpc".to_owned(), serde_json::json!("2.0"));
            old.insert("id".to_owned(), id.clone());
            let mut value = from_protocol_json(&result);
            if let serde_json::Value::Object(o) = &mut value
                && !o.contains_key("status")
            {
                let status = if o.contains_key("error") {
                    "error"
                } else {
                    "success"
                };
                o.insert("status".to_owned(), serde_json::json!(status));
            }
            old.insert("result".to_owned(), value);
            let old_bytes = sonic_rs::to_vec(&serde_json::Value::Object(old)).unwrap();

            let mut envelope = Envelope::with_capacity(3);
            envelope
                .insert("result", EnvelopeField::ProtoWithDefaultStatus(&result))
                .insert("jsonrpc", EnvelopeField::Str("2.0"))
                .insert("id", EnvelopeField::Json(&id));
            let new_bytes = sonic_rs::to_vec(&envelope).unwrap();
            assert_eq!(
                String::from_utf8(new_bytes).unwrap(),
                String::from_utf8(old_bytes).unwrap()
            );
        }
    }

    #[test]
    fn direct_value_serialization_matches_converted_tree() {
        let value = sample_result();
        assert_eq!(
            sonic_rs::to_string(&value).unwrap(),
            sonic_rs::to_string(&from_protocol_json(&value)).unwrap()
        );
    }

    #[test]
    fn default_status_is_inserted_in_sorted_position() {
        for keys in [
            vec![],
            vec!["a"],
            vec!["z"],
            vec!["a", "z"],
            vec!["state", "stx"],
        ] {
            let object: BTreeMap<String, JsonValue> = keys
                .iter()
                .map(|k| ((*k).to_owned(), JsonValue::Null))
                .collect();
            let result = JsonValue::Object(object);
            let mut expected = from_protocol_json(&result);
            expected
                .as_object_mut()
                .unwrap()
                .insert("status".to_owned(), serde_json::json!("success"));
            assert_eq!(
                sonic_rs::to_string(&EnvelopeField::ProtoWithDefaultStatus(&result)).unwrap(),
                sonic_rs::to_string(&expected).unwrap()
            );
        }
    }
}
