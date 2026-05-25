//! Shared ledger-entry builder base mirroring `protocol_autogen/LedgerEntryBuilderBase.h`.

use std::sync::Arc;

use basics::base_uint::Uint256;

use crate::{
    LedgerEntryType, LedgerFormats, STLedgerEntry, STObject, get_field_by_symbol,
    keylet::ledger_entry_type_from_code, validate_st_object,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntryBuilderBase {
    object: STObject,
}

impl LedgerEntryBuilderBase {
    pub fn new(entry_type: LedgerEntryType) -> Self {
        Self::new_with_flags(entry_type, 0)
    }

    pub fn new_with_flags(entry_type: LedgerEntryType, flags: u32) -> Self {
        let mut object = STObject::new(get_field_by_symbol("sfLedgerEntry"));
        object.set_field_u16(get_field_by_symbol("sfLedgerEntryType"), entry_type as u16);
        object.set_field_u32(get_field_by_symbol("sfFlags"), flags);
        Self { object }
    }

    pub fn from_sle(sle: Arc<STLedgerEntry>) -> Self {
        Self {
            object: sle.as_ref().clone_as_object(),
        }
    }

    pub fn validate(&self) -> bool {
        let entry_type = self
            .object
            .is_field_present(get_field_by_symbol("sfLedgerEntryType"))
            .then(|| {
                self.object
                    .get_field_u16(get_field_by_symbol("sfLedgerEntryType"))
            });
        let Some(entry_type) = entry_type else {
            return false;
        };
        let Some(entry_type) = ledger_entry_type_from_code(entry_type) else {
            return false;
        };
        let Some(format) = LedgerFormats::get_instance().find_by_type(entry_type) else {
            return false;
        };
        validate_st_object(&self.object, format.so_template())
    }

    pub fn object(&self) -> &STObject {
        &self.object
    }

    pub fn object_mut(&mut self) -> &mut STObject {
        &mut self.object
    }

    pub fn into_object(self) -> STObject {
        self.object
    }

    pub fn set_ledger_index(&mut self, value: Uint256) {
        self.object
            .set_field_h256(get_field_by_symbol("sfLedgerIndex"), value);
    }

    pub fn set_flags(&mut self, value: u32) {
        self.object
            .set_field_u32(get_field_by_symbol("sfFlags"), value);
    }
}
