//! `STVector256` port.

use basics::{base_uint::Uint256, str_hex::str_hex_iter};

use crate::{
    JsonOptions, JsonValue, SField, SerialIter, SerializedTypeId, Serializer, StBase, StBaseCore,
    downcast_stbase_ref,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct STVector256 {
    core: StBaseCore,
    value: Vec<Uint256>,
}

impl STVector256 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_field(field: &'static SField) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            value: Vec::new(),
        }
    }

    pub fn from_values(field: &'static SField, value: Vec<Uint256>) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            value,
        }
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        let bytes = sit.get_vl();
        if !bytes.len().is_multiple_of(Uint256::size()) {
            panic!("Bad serialization for STVector256: {}", bytes.len());
        }

        let mut value = Vec::with_capacity(bytes.len() / Uint256::size());
        for chunk in bytes.chunks_exact(Uint256::size()) {
            value.push(Uint256::from_slice(chunk).expect("Uint256 chunk width should match"));
        }
        Self::from_values(field, value)
    }

    pub fn value(&self) -> &[Uint256] {
        &self.value
    }

    pub fn push_back(&mut self, value: Uint256) {
        self.value.push(value);
    }
}

impl StBase for STVector256 {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn core(&self) -> &StBaseCore {
        &self.core
    }

    fn core_mut(&mut self) -> &mut StBaseCore {
        &mut self.core
    }

    fn stype(&self) -> SerializedTypeId {
        SerializedTypeId::Vector256
    }

    fn json(&self, _options: JsonOptions) -> JsonValue {
        JsonValue::Array(
            self.value
                .iter()
                .map(|value| JsonValue::String(value.to_string()))
                .collect(),
        )
    }

    fn add(&self, serializer: &mut Serializer) {
        assert!(
            self.fname().is_binary(),
            "xrpl::STVector256::add : field is binary"
        );
        assert_eq!(
            self.fname().field_type(),
            SerializedTypeId::Vector256,
            "xrpl::STVector256::add : valid field type"
        );
        serializer.add_vl_chunks(
            self.value.iter().map(|value| value.data()),
            self.value.len() * 32,
        );
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        downcast_stbase_ref::<Self>(other).value == self.value
    }

    fn is_default(&self) -> bool {
        self.value.is_empty()
    }

    fn text(&self) -> String {
        str_hex_iter(
            self.value
                .iter()
                .flat_map(|value| value.data().iter().copied()),
        )
    }
}
