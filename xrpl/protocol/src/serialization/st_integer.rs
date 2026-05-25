//! Fixed-width integer `ST*` leaves.

use std::fmt::Display;
use std::marker::PhantomData;

use crate::keylet::ledger_entry_type_from_code;
use crate::ter::{trans_human, trans_token};
use crate::{
    JsonOptions, JsonValue, LedgerFormats, SField, SerialIter, SerializedTypeId, Serializer,
    StBase, StBaseCore, Ter, TxFormats, TxType, downcast_stbase_ref, get_field_by_symbol,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct STInteger<T, K> {
    core: StBaseCore,
    value: T,
    kind: PhantomData<K>,
}

pub type STUInt8 = STInteger<u8, UInt8Kind>;
pub type STUInt16 = STInteger<u16, UInt16Kind>;
pub type STUInt32 = STInteger<u32, UInt32Kind>;
pub type STUInt64 = STInteger<u64, UInt64Kind>;
pub type STInt32 = STInteger<i32, Int32Kind>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UInt8Kind;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UInt16Kind;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UInt32Kind;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UInt64Kind;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Int32Kind;

pub trait STIntegerKind<T>: Clone + PartialEq + Eq + 'static {
    const STYPE: SerializedTypeId;

    fn read(sit: &mut SerialIter<'_>) -> T;
    fn text(field: &'static SField, value: T) -> String;
    fn json(field: &'static SField, value: T) -> JsonValue;
}

impl<T, K> STInteger<T, K> {
    pub fn new(value: T) -> Self
    where
        T: Default,
    {
        Self {
            core: StBaseCore::new(),
            value,
            kind: PhantomData,
        }
    }

    pub fn with_field(field: &'static SField, value: T) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            value,
            kind: PhantomData,
        }
    }

    pub fn value(&self) -> T
    where
        T: Copy,
    {
        self.value
    }

    pub fn set_value(&mut self, value: T) {
        self.value = value;
    }
}

impl STUInt8 {
    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        Self::with_field(field, UInt8Kind::read(sit))
    }
}

impl STUInt16 {
    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        Self::with_field(field, UInt16Kind::read(sit))
    }
}

impl STUInt32 {
    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        Self::with_field(field, UInt32Kind::read(sit))
    }
}

impl STUInt64 {
    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        Self::with_field(field, UInt64Kind::read(sit))
    }
}

impl STInt32 {
    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        Self::with_field(field, Int32Kind::read(sit))
    }
}

impl<T, K> StBase for STInteger<T, K>
where
    T: Copy + Default + Display + Eq + crate::serializer::SerializerInteger + 'static + Send + Sync,
    K: STIntegerKind<T> + Send + Sync,
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
        K::text(self.fname(), self.value)
    }

    fn json(&self, _options: JsonOptions) -> JsonValue {
        K::json(self.fname(), self.value)
    }

    fn add(&self, serializer: &mut Serializer) {
        assert!(
            self.fname().is_binary(),
            "xrpl::STInteger::add : field is binary"
        );
        assert_eq!(
            self.fname().field_type(),
            self.stype(),
            "xrpl::STInteger::add : field type match"
        );
        serializer.add_integer(self.value);
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        downcast_stbase_ref::<Self>(other).value == self.value
    }

    fn is_default(&self) -> bool {
        self.value == T::default()
    }
}

impl STIntegerKind<u8> for UInt8Kind {
    const STYPE: SerializedTypeId = SerializedTypeId::UInt8;

    fn read(sit: &mut SerialIter<'_>) -> u8 {
        sit.get8()
    }

    fn text(field: &'static SField, value: u8) -> String {
        if field == get_field_by_symbol("sfTransactionResult") {
            return trans_human(Ter::from_int(i32::from(value))).to_string();
        }
        value.to_string()
    }

    fn json(field: &'static SField, value: u8) -> JsonValue {
        if field == get_field_by_symbol("sfTransactionResult") {
            return JsonValue::String(trans_token(Ter::from_int(i32::from(value))).to_string());
        }
        JsonValue::Unsigned(u64::from(value))
    }
}

impl STIntegerKind<u16> for UInt16Kind {
    const STYPE: SerializedTypeId = SerializedTypeId::UInt16;

    fn read(sit: &mut SerialIter<'_>) -> u16 {
        sit.get16()
    }

    fn text(field: &'static SField, value: u16) -> String {
        if field == get_field_by_symbol("sfLedgerEntryType")
            && let Some(entry_type) = ledger_entry_type_from_code(value)
            && let Some(item) = LedgerFormats::get_instance().find_by_type(entry_type)
        {
            return item.name().to_string();
        }

        if field == get_field_by_symbol("sfTransactionType")
            && let Some(item) = TxFormats::get_instance().find_by_type(TxType::from_u16(value))
        {
            return item.name().to_string();
        }
        value.to_string()
    }

    fn json(field: &'static SField, value: u16) -> JsonValue {
        if field == get_field_by_symbol("sfLedgerEntryType")
            && let Some(entry_type) = ledger_entry_type_from_code(value)
            && let Some(item) = LedgerFormats::get_instance().find_by_type(entry_type)
        {
            return JsonValue::String(item.name().to_string());
        }

        if field == get_field_by_symbol("sfTransactionType")
            && let Some(item) = TxFormats::get_instance().find_by_type(TxType::from_u16(value))
        {
            return JsonValue::String(item.name().to_string());
        }
        JsonValue::Unsigned(u64::from(value))
    }
}

impl STIntegerKind<u32> for UInt32Kind {
    const STYPE: SerializedTypeId = SerializedTypeId::UInt32;

    fn read(sit: &mut SerialIter<'_>) -> u32 {
        sit.get32()
    }

    fn text(field: &'static SField, value: u32) -> String {
        permission_value_name(field, value).unwrap_or_else(|| value.to_string())
    }

    fn json(field: &'static SField, value: u32) -> JsonValue {
        permission_value_name(field, value)
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Unsigned(u64::from(value)))
    }
}

impl STIntegerKind<u64> for UInt64Kind {
    const STYPE: SerializedTypeId = SerializedTypeId::UInt64;

    fn read(sit: &mut SerialIter<'_>) -> u64 {
        sit.get64()
    }

    fn text(_field: &'static SField, value: u64) -> String {
        value.to_string()
    }

    fn json(field: &'static SField, value: u64) -> JsonValue {
        if field.should_meta(SField::S_MD_BASE_TEN) {
            JsonValue::String(value.to_string())
        } else {
            JsonValue::String(format!("{value:x}"))
        }
    }
}

impl STIntegerKind<i32> for Int32Kind {
    const STYPE: SerializedTypeId = SerializedTypeId::Int32;

    fn read(sit: &mut SerialIter<'_>) -> i32 {
        sit.geti32()
    }

    fn text(_field: &'static SField, value: i32) -> String {
        value.to_string()
    }

    fn json(_field: &'static SField, value: i32) -> JsonValue {
        JsonValue::Signed(i64::from(value))
    }
}

fn permission_value_name(field: &'static SField, value: u32) -> Option<String> {
    if field != get_field_by_symbol("sfPermissionValue") {
        return None;
    }

    match value {
        65_537 => Some("TrustlineAuthorize".to_string()),
        65_538 => Some("TrustlineFreeze".to_string()),
        65_539 => Some("TrustlineUnfreeze".to_string()),
        65_540 => Some("AccountDomainSet".to_string()),
        65_541 => Some("AccountEmailHashSet".to_string()),
        65_542 => Some("AccountMessageKeySet".to_string()),
        65_543 => Some("AccountTransferRateSet".to_string()),
        65_544 => Some("AccountTickSizeSet".to_string()),
        65_545 => Some("PaymentMint".to_string()),
        65_546 => Some("PaymentBurn".to_string()),
        65_547 => Some("MPTokenIssuanceLock".to_string()),
        65_548 => Some("MPTokenIssuanceUnlock".to_string()),
        other if other > u32::from(u16::MAX) => None,
        other => TxFormats::get_instance()
            .find_by_type(TxType::from_u16((other.saturating_sub(1)) as u16))
            .map(|item| item.name().to_string()),
    }
}
