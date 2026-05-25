//! Fixed-width bit-string `ST*` leaves.

use std::marker::PhantomData;

use basics::base_uint::{BaseUInt, Uint128, Uint160, Uint192, Uint256};

use crate::{
    JsonOptions, JsonValue, SField, SerialIter, SerializedTypeId, Serializer, StBase, StBaseCore,
    downcast_stbase_ref,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct STBitString<const BYTES: usize, Tag, K> {
    core: StBaseCore,
    value: BaseUInt<BYTES, Tag>,
    kind: PhantomData<K>,
}

pub type STUInt128 = STBitString<16, (), UInt128Kind>;
pub type STUInt160 = STBitString<20, (), UInt160Kind>;
pub type STUInt192 = STBitString<24, (), UInt192Kind>;
pub type STUInt256 = STBitString<32, (), UInt256Kind>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UInt128Kind;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UInt160Kind;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UInt192Kind;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UInt256Kind;

pub trait STBitStringKind: Clone + PartialEq + Eq + 'static {
    const STYPE: SerializedTypeId;
}

impl<const BYTES: usize, Tag, K> STBitString<BYTES, Tag, K> {
    pub fn new(value: BaseUInt<BYTES, Tag>) -> Self {
        Self {
            core: StBaseCore::new(),
            value,
            kind: PhantomData,
        }
    }

    pub fn with_field(field: &'static SField, value: BaseUInt<BYTES, Tag>) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            value,
            kind: PhantomData,
        }
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        Self::with_field(field, sit.get_bit_string())
    }

    pub fn value(&self) -> &BaseUInt<BYTES, Tag> {
        &self.value
    }

    pub fn set_value<OtherTag>(&mut self, value: BaseUInt<BYTES, OtherTag>) {
        self.value = BaseUInt::from_slice(value.data()).expect("bit-string width should match");
    }
}

impl<const BYTES: usize, Tag, K> StBase for STBitString<BYTES, Tag, K>
where
    Tag: 'static + Send + Sync,
    K: STBitStringKind + Send + Sync,
{
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
        K::STYPE
    }

    fn text(&self) -> String {
        self.value.to_string()
    }

    fn json(&self, _options: JsonOptions) -> JsonValue {
        JsonValue::String(self.text())
    }

    fn add(&self, serializer: &mut Serializer) {
        assert!(
            self.fname().is_binary(),
            "xrpl::STBitString::add : field is binary"
        );
        assert_eq!(
            self.fname().field_type(),
            self.stype(),
            "xrpl::STBitString::add : field type match"
        );
        serializer.add_raw(self.value.data());
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        downcast_stbase_ref::<Self>(other).value == self.value
    }

    fn is_default(&self) -> bool {
        self.value.is_zero()
    }
}

impl STBitStringKind for UInt128Kind {
    const STYPE: SerializedTypeId = SerializedTypeId::UInt128;
}

impl STBitStringKind for UInt160Kind {
    const STYPE: SerializedTypeId = SerializedTypeId::UInt160;
}

impl STBitStringKind for UInt192Kind {
    const STYPE: SerializedTypeId = SerializedTypeId::UInt192;
}

impl STBitStringKind for UInt256Kind {
    const STYPE: SerializedTypeId = SerializedTypeId::UInt256;
}

impl From<Uint128> for STUInt128 {
    fn from(value: Uint128) -> Self {
        Self::new(value)
    }
}

impl From<Uint160> for STUInt160 {
    fn from(value: Uint160) -> Self {
        Self::new(value)
    }
}

impl From<Uint192> for STUInt192 {
    fn from(value: Uint192) -> Self {
        Self::new(value)
    }
}

impl From<Uint256> for STUInt256 {
    fn from(value: Uint256) -> Self {
        Self::new(value)
    }
}
