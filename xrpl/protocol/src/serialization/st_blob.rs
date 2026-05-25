//! Variable-length byte-string `STBlob`.

use basics::{buffer::Buffer, str_hex::str_hex};

use crate::{
    JsonValue, SField, SerialIter, SerializedTypeId, Serializer, StBase, StBaseCore,
    downcast_stbase_ref,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct STBlob {
    core: StBaseCore,
    value: Buffer,
}

impl STBlob {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_field(field: &'static SField) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            value: Buffer::new(),
        }
    }

    pub fn from_buffer(field: &'static SField, value: Buffer) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            value,
        }
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        Self::from_buffer(field, sit.get_vl_buffer())
    }

    pub fn size(&self) -> usize {
        self.value.size()
    }

    pub fn data(&self) -> &[u8] {
        self.value.data()
    }

    pub fn value(&self) -> &Buffer {
        &self.value
    }

    pub fn set_value(&mut self, value: Buffer) {
        self.value = value;
    }
}

impl StBase for STBlob {
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
        SerializedTypeId::VariableLength
    }

    fn text(&self) -> String {
        str_hex(self.value.data())
    }

    fn json(&self, _options: crate::JsonOptions) -> JsonValue {
        JsonValue::String(self.text())
    }

    fn add(&self, serializer: &mut Serializer) {
        assert!(
            self.fname().is_binary(),
            "xrpl::STBlob::add : field is binary"
        );
        assert!(
            matches!(
                self.fname().field_type(),
                SerializedTypeId::VariableLength | SerializedTypeId::Account
            ),
            "xrpl::STBlob::add : valid field type"
        );
        serializer.add_vl(self.value.data());
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        downcast_stbase_ref::<Self>(other).value == self.value
    }

    fn is_default(&self) -> bool {
        self.value.empty()
    }
}
