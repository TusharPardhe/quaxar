//! STBase kernel port from `xrpl/protocol/STBase.*`.

use std::any::Any;
use std::collections::BTreeMap;
use std::ops::{BitAnd, BitOr, Not};

use crate::{SField, SerializedTypeId, Serializer, sf_generic};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonOptions {
    value: u32,
}

impl JsonOptions {
    pub const NONE: Self = Self { value: 0b00 };
    pub const INCLUDE_DATE: Self = Self { value: 0b01 };
    pub const DISABLE_API_PRIOR_V2: Self = Self { value: 0b10 };
    pub const ALL: Self = Self { value: 0b11 };

    pub const fn new(value: u32) -> Self {
        Self {
            value: value & Self::ALL.value,
        }
    }

    pub const fn bits(self) -> u32 {
        self.value
    }
}

impl BitOr for JsonOptions {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self::new(self.value | rhs.value)
    }
}

impl BitAnd for JsonOptions {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self::new(self.value & rhs.value)
    }
}

impl Not for JsonOptions {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self::new(!self.value & Self::ALL.value)
    }
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
    /// Already-serialized JSON text (one complete value), emitted verbatim.
    /// Produced by the direct JSON writer (`json_writer`) so responses can
    /// embed ledger objects without building a `JsonValue` tree for them.
    /// Never produced by deserialization.
    #[serde(skip_deserializing)]
    Raw(RawJson),
}

/// A complete, valid JSON value in serialized form. Cheap to clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawJson(std::sync::Arc<str>);

impl RawJson {
    /// Wrap `text`, which must be one complete, valid JSON value.
    pub fn from_trusted(text: impl Into<std::sync::Arc<str>>) -> Self {
        Self(text.into())
    }

    /// Wrap UTF-8 JSON bytes produced by a trusted writer.
    pub fn from_trusted_bytes(bytes: Vec<u8>) -> Self {
        Self(
            String::from_utf8(bytes)
                .expect("JSON writer produces UTF-8")
                .into(),
        )
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Materialize as a `JsonValue` tree (slow path, for code that needs to
    /// inspect the value).
    pub fn to_tree(&self) -> JsonValue {
        let value: serde_json::Value =
            serde_json::from_str(&self.0).expect("RawJson holds valid JSON");
        json_from_serde_exact(value)
    }
}

/// `serde_json::Value` -> `JsonValue` with the same number mapping the RPC
/// transport uses (non-integers become strings).
fn json_from_serde_exact(value: serde_json::Value) -> JsonValue {
    match value {
        serde_json::Value::Null => JsonValue::Null,
        serde_json::Value::Bool(b) => JsonValue::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                JsonValue::Unsigned(u)
            } else if let Some(i) = n.as_i64() {
                JsonValue::Signed(i)
            } else {
                JsonValue::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => JsonValue::String(s),
        serde_json::Value::Array(a) => {
            JsonValue::Array(a.into_iter().map(json_from_serde_exact).collect())
        }
        serde_json::Value::Object(o) => JsonValue::Object(
            o.into_iter()
                .map(|(k, v)| (k, json_from_serde_exact(v)))
                .collect(),
        ),
    }
}

impl Serialize for RawJson {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // sonic-rs writes a value serialized as its LazyValue token struct
        // verbatim (sonic-rs `lazyvalue/ser.rs`). Any other serializer gets
        // the equivalent parsed value, so output is correct everywhere and
        // zero-copy on the sonic-rs response path.
        if std::any::type_name::<S>().starts_with("&mut sonic_rs::") {
            use serde::ser::SerializeStruct;
            const TOKEN: &str = "$sonic_rs::LazyValue";
            let mut raw = serializer.serialize_struct(TOKEN, 1)?;
            raw.serialize_field(TOKEN, &*self.0)?;
            raw.end()
        } else {
            let value: serde_json::Value =
                serde_json::from_str(&self.0).map_err(serde::ser::Error::custom)?;
            value.serialize(serializer)
        }
    }
}

impl JsonValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Unsigned(u) => Some(*u),
            Self::Signed(s) if *s >= 0 => Some(*s as u64),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            Self::Object(m) => m.get(key),
            _ => None,
        }
    }
}

impl From<serde_json::Value> for JsonValue {
    fn from(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    Self::Unsigned(u)
                } else if let Some(i) = n.as_i64() {
                    Self::Signed(i)
                } else {
                    // Fallback for floats if needed, but XRPL usually uses fixed point
                    Self::Null
                }
            }
            serde_json::Value::String(s) => Self::String(s),
            serde_json::Value::Array(a) => Self::Array(a.into_iter().map(Self::from).collect()),
            serde_json::Value::Object(o) => {
                Self::Object(o.into_iter().map(|(k, v)| (k, Self::from(v))).collect())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    MissingField(&'static str),
    InvalidField(&'static str),
    Custom(String),
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(name) => write!(f, "Missing field: {}", name),
            Self::InvalidField(name) => write!(f, "Invalid field: {}", name),
            Self::Custom(msg) => write!(f, "Validation error: {}", msg),
        }
    }
}

impl std::error::Error for ValidationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StBaseCore {
    f_name: &'static SField,
}

impl Default for StBaseCore {
    fn default() -> Self {
        Self::new()
    }
}

impl StBaseCore {
    pub fn new() -> Self {
        Self {
            f_name: sf_generic(),
        }
    }

    pub fn with_field(field: &'static SField) -> Self {
        Self { f_name: field }
    }

    pub fn assign_from(&mut self, other: &Self) {
        if !self.f_name.is_useful() {
            self.f_name = other.f_name;
        }
    }

    pub fn set_fname(&mut self, field: &'static SField) {
        self.f_name = field;
    }

    pub fn fname(&self) -> &'static SField {
        self.f_name
    }

    pub fn add_field_id(&self, serializer: &mut Serializer) {
        assert!(
            self.f_name.is_binary(),
            "xrpl::STBase::addFieldID : field is binary"
        );
        serializer.add_field_id(self.f_name.field_type().as_i32(), self.f_name.field_value());
    }
}

pub trait StBase: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn core(&self) -> &StBaseCore;
    fn core_mut(&mut self) -> &mut StBaseCore;

    fn stype(&self) -> SerializedTypeId {
        SerializedTypeId::NotPresent
    }

    fn full_text(&self) -> String {
        if self.stype() == SerializedTypeId::NotPresent {
            return String::new();
        }

        let mut text = String::new();
        if self.fname().has_name() {
            text.push_str(self.fname().name());
            text.push_str(" = ");
        }
        text.push_str(&self.text());
        text
    }

    fn text(&self) -> String {
        String::new()
    }

    fn json(&self, _options: JsonOptions) -> JsonValue {
        JsonValue::String(self.text())
    }

    fn add(&self, _serializer: &mut Serializer) {
        unreachable!("xrpl::STBase::add should never be called");
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        assert_eq!(
            self.stype(),
            SerializedTypeId::NotPresent,
            "xrpl::STBase::isEquivalent : type not present"
        );
        other.stype() == SerializedTypeId::NotPresent
    }

    fn is_default(&self) -> bool {
        true
    }

    fn is_valid(&self) -> bool {
        true
    }

    fn check(&self) -> Result<(), ValidationError> {
        Ok(())
    }

    fn set_fname(&mut self, field: &'static SField) {
        self.core_mut().set_fname(field);
    }

    fn fname(&self) -> &'static SField {
        self.core().fname()
    }

    fn add_field_id(&self, serializer: &mut Serializer) {
        self.core().add_field_id(serializer);
    }
}

pub fn st_base_eq(left: &dyn StBase, right: &dyn StBase) -> bool {
    left.stype() == right.stype() && left.is_equivalent(right)
}

pub fn st_base_ne(left: &dyn StBase, right: &dyn StBase) -> bool {
    left.stype() != right.stype() || !left.is_equivalent(right)
}

pub fn downcast_stbase_ref<D: Any>(value: &dyn StBase) -> &D {
    value.as_any().downcast_ref::<D>().expect("bad cast")
}

pub fn downcast_stbase_mut<D: Any>(value: &mut dyn StBase) -> &mut D {
    value.as_any_mut().downcast_mut::<D>().expect("bad cast")
}

pub fn to_json<T>(value: &T) -> JsonValue
where
    T: ?Sized + StBase,
{
    value.json(JsonOptions::NONE)
}
