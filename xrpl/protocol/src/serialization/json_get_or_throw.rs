//! JSON field extraction helpers mirroring `xrpl/protocol/json_get_or_throw.h`.

use std::fmt;

use basics::{buffer::Buffer, string_utilities::str_unhex};

use crate::{
    AccountID, Asset, JsonValue, MPTAmount, PublicKey, SField, STAmount, STXChainBridge, XRPAmount,
    asset_from_json, normalized_parts_from_string, parse_base58_account_id,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonMissingKeyError {
    key: String,
}

impl JsonMissingKeyError {
    pub fn new(key: &str) -> Self {
        Self {
            key: key.to_owned(),
        }
    }
}

impl fmt::Display for JsonMissingKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Missing json key: {}", self.key)
    }
}

impl std::error::Error for JsonMissingKeyError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonTypeMismatchError {
    key: String,
    expected_type: String,
}

impl JsonTypeMismatchError {
    pub fn new(key: &str, expected_type: impl Into<String>) -> Self {
        Self {
            key: key.to_owned(),
            expected_type: expected_type.into(),
        }
    }
}

impl fmt::Display for JsonTypeMismatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Type mismatch on json key: {}; expected type: {}",
            self.key, self.expected_type
        )
    }
}

impl std::error::Error for JsonTypeMismatchError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonGetOrThrowError {
    MissingKey(JsonMissingKeyError),
    TypeMismatch(JsonTypeMismatchError),
}

impl fmt::Display for JsonGetOrThrowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingKey(error) => error.fmt(f),
            Self::TypeMismatch(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for JsonGetOrThrowError {}

pub trait JsonFieldRead: Sized {
    fn read_json_field(
        value: &JsonValue,
        field: &'static SField,
    ) -> Result<Self, JsonGetOrThrowError>;
}

pub fn get_or_throw<T: JsonFieldRead>(
    value: &JsonValue,
    field: &'static SField,
) -> Result<T, JsonGetOrThrowError> {
    T::read_json_field(value, field)
}

pub fn get_optional<T: JsonFieldRead>(value: &JsonValue, field: &'static SField) -> Option<T> {
    get_or_throw(value, field).ok()
}

fn get_member<'a>(
    value: &'a JsonValue,
    field: &'static SField,
) -> Result<&'a JsonValue, JsonGetOrThrowError> {
    let key = field.name();
    let JsonValue::Object(object) = value else {
        return Err(JsonGetOrThrowError::TypeMismatch(
            JsonTypeMismatchError::new(key, "object"),
        ));
    };

    object
        .get(key)
        .ok_or_else(|| JsonGetOrThrowError::MissingKey(JsonMissingKeyError::new(key)))
}

impl JsonFieldRead for String {
    fn read_json_field(
        value: &JsonValue,
        field: &'static SField,
    ) -> Result<Self, JsonGetOrThrowError> {
        match get_member(value, field)? {
            JsonValue::String(inner) => Ok(inner.clone()),
            _ => Err(JsonGetOrThrowError::TypeMismatch(
                JsonTypeMismatchError::new(field.name(), "string"),
            )),
        }
    }
}

impl JsonFieldRead for bool {
    fn read_json_field(
        value: &JsonValue,
        field: &'static SField,
    ) -> Result<Self, JsonGetOrThrowError> {
        match get_member(value, field)? {
            JsonValue::Bool(inner) => Ok(*inner),
            JsonValue::Signed(inner) => Ok(*inner != 0),
            JsonValue::Unsigned(inner) => Ok(*inner != 0),
            _ => Err(JsonGetOrThrowError::TypeMismatch(
                JsonTypeMismatchError::new(field.name(), "bool"),
            )),
        }
    }
}

impl JsonFieldRead for u64 {
    fn read_json_field(
        value: &JsonValue,
        field: &'static SField,
    ) -> Result<Self, JsonGetOrThrowError> {
        match get_member(value, field)? {
            JsonValue::Unsigned(inner) => Ok(*inner),
            JsonValue::Signed(inner) if *inner >= 0 => Ok(*inner as u64),
            JsonValue::String(inner) => u64::from_str_radix(inner, 16).map_err(|_| {
                JsonGetOrThrowError::TypeMismatch(JsonTypeMismatchError::new(
                    field.name(),
                    "uint64",
                ))
            }),
            _ => Err(JsonGetOrThrowError::TypeMismatch(
                JsonTypeMismatchError::new(field.name(), "uint64"),
            )),
        }
    }
}

impl JsonFieldRead for Buffer {
    fn read_json_field(
        value: &JsonValue,
        field: &'static SField,
    ) -> Result<Self, JsonGetOrThrowError> {
        let hex = get_or_throw::<String>(value, field)?;
        let Some(bytes) = str_unhex(&hex) else {
            return Err(JsonGetOrThrowError::TypeMismatch(
                JsonTypeMismatchError::new(field.name(), "Buffer"),
            ));
        };
        Ok(Buffer::from_bytes(&bytes))
    }
}

impl JsonFieldRead for AccountID {
    fn read_json_field(
        value: &JsonValue,
        field: &'static SField,
    ) -> Result<Self, JsonGetOrThrowError> {
        let text = get_or_throw::<String>(value, field)?;
        if let Some(account) = parse_base58_account_id(&text) {
            return Ok(account);
        }
        AccountID::from_hex(&text).map_err(|_| {
            JsonGetOrThrowError::TypeMismatch(JsonTypeMismatchError::new(field.name(), "AccountID"))
        })
    }
}

impl JsonFieldRead for PublicKey {
    fn read_json_field(
        value: &JsonValue,
        field: &'static SField,
    ) -> Result<Self, JsonGetOrThrowError> {
        let hex = get_or_throw::<String>(value, field)?;
        let Some(bytes) = str_unhex(&hex) else {
            return Err(JsonGetOrThrowError::TypeMismatch(
                JsonTypeMismatchError::new(field.name(), "PublicKey"),
            ));
        };
        PublicKey::from_slice(&bytes).map_err(|_| {
            JsonGetOrThrowError::TypeMismatch(JsonTypeMismatchError::new(field.name(), "PublicKey"))
        })
    }
}

impl JsonFieldRead for STAmount {
    fn read_json_field(
        value: &JsonValue,
        field: &'static SField,
    ) -> Result<Self, JsonGetOrThrowError> {
        let member = get_member(value, field)?;
        parse_st_amount(field, member).map_err(|expected| {
            JsonGetOrThrowError::TypeMismatch(JsonTypeMismatchError::new(field.name(), expected))
        })
    }
}

impl JsonFieldRead for STXChainBridge {
    fn read_json_field(
        value: &JsonValue,
        field: &'static SField,
    ) -> Result<Self, JsonGetOrThrowError> {
        let member = get_member(value, field)?;
        STXChainBridge::from_json_value(field, member).map_err(|_| {
            JsonGetOrThrowError::TypeMismatch(JsonTypeMismatchError::new(
                field.name(),
                "STXChainBridge",
            ))
        })
    }
}

fn parse_st_amount(field: &'static SField, value: &JsonValue) -> Result<STAmount, &'static str> {
    match value {
        JsonValue::String(text) => {
            let runtime = normalized_parts_from_string(text).map_err(|_| "STAmount")?;
            let xrp = XRPAmount::from_number(runtime).map_err(|_| "STAmount")?;
            Ok(STAmount::from_xrp_amount(xrp))
        }
        JsonValue::Object(object) => {
            let Some(JsonValue::String(value_text)) = object.get("value") else {
                return Err("STAmount");
            };
            let asset = asset_from_json(value).map_err(|_| "STAmount")?;
            let runtime = normalized_parts_from_string(value_text).map_err(|_| "STAmount")?;
            match asset {
                Asset::Issue(issue) if issue.native() => {
                    let xrp = XRPAmount::from_number(runtime).map_err(|_| "STAmount")?;
                    Ok(STAmount::from_xrp_amount(xrp))
                }
                Asset::Issue(issue) => {
                    let amount = crate::IOUAmount::from_number(runtime).map_err(|_| "STAmount")?;
                    Ok(STAmount::from_iou_amount(field, amount, issue))
                }
                Asset::MPTIssue(issue) => {
                    let amount = MPTAmount::from_number(runtime).map_err(|_| "STAmount")?;
                    Ok(STAmount::from_mpt_amount(field, amount, issue))
                }
            }
        }
        _ => Err("STAmount"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{get_optional, get_or_throw};
    use crate::{JsonValue, STAmount, StBase, get_field_by_symbol};

    #[test]
    fn get_or_throw_reads_common_protocol_field_shapes() {
        let field = get_field_by_symbol("sfAmount");
        let json = JsonValue::Object(BTreeMap::from([(
            field.name().to_owned(),
            JsonValue::Object(BTreeMap::from([
                ("currency".to_owned(), JsonValue::String("XRP".to_owned())),
                ("value".to_owned(), JsonValue::String("100".to_owned())),
            ])),
        )]));

        let amount = get_or_throw::<STAmount>(&json, field).expect("amount should parse");
        assert_eq!(amount.text(), "100");
        assert!(get_optional::<STAmount>(&json, field).is_some());
    }
}
