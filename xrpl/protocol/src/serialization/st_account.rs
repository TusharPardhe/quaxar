//! `STAccount` port with blob-compatible wire semantics.

use basics::{base_uint::Uint160, buffer::Buffer};

use crate::{
    AccountID, JsonOptions, JsonValue, SField, SerialIter, SerializedTypeId, Serializer, StBase,
    StBaseCore, downcast_stbase_ref, to_base58,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct STAccount {
    core: StBaseCore,
    value: AccountID,
    default: bool,
}

impl Default for STAccount {
    fn default() -> Self {
        Self {
            core: StBaseCore::new(),
            value: AccountID::zero(),
            default: true,
        }
    }
}

impl STAccount {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_value(field: &'static SField, value: AccountID) -> Self {
        let mut account = Self::with_field(field);
        account.set_value(value);
        account
    }

    pub fn with_field(field: &'static SField) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            ..Self::default()
        }
    }

    pub fn from_buffer(field: &'static SField, buffer: Buffer) -> Self {
        let mut account = Self::with_field(field);
        if buffer.empty() {
            return account;
        }
        if buffer.size() != Uint160::size() {
            panic!("Invalid STAccount size");
        }
        account.value = AccountID::from_slice(buffer.data()).expect("AccountID width should match");
        account.default = false;
        account
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        Self::from_buffer(field, sit.get_vl_buffer())
    }

    pub fn value(&self) -> &AccountID {
        &self.value
    }

    pub fn set_value(&mut self, value: AccountID) {
        self.value = value;
        self.default = false;
    }
}

impl StBase for STAccount {
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
        SerializedTypeId::Account
    }

    fn text(&self) -> String {
        if self.default {
            String::new()
        } else {
            to_base58(self.value)
        }
    }

    fn json(&self, _options: JsonOptions) -> JsonValue {
        JsonValue::String(self.text())
    }

    fn add(&self, serializer: &mut Serializer) {
        assert!(
            self.fname().is_binary(),
            "xrpl::STAccount::add : field is binary"
        );
        assert_eq!(
            self.fname().field_type(),
            SerializedTypeId::Account,
            "xrpl::STAccount::add : valid field type"
        );
        let size = if self.default { 0 } else { Uint160::size() };
        serializer.add_vl(&self.value.data()[..size]);
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        let other = downcast_stbase_ref::<Self>(other);
        self.default == other.default && self.value == other.value
    }

    fn is_default(&self) -> bool {
        self.default
    }
}
