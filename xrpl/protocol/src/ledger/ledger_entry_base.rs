//! Shared ledger-entry wrapper base mirroring `protocol_autogen/LedgerEntryBase.h`.

use std::sync::Arc;

use basics::base_uint::Uint256;

use crate::{
    LedgerEntryType, LedgerFormats, STLedgerEntry, get_field_by_symbol,
    keylet::ledger_entry_type_from_code, validate_st_object,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntryBase {
    sle: Arc<STLedgerEntry>,
}

impl LedgerEntryBase {
    pub fn new(sle: Arc<STLedgerEntry>) -> Self {
        Self { sle }
    }

    pub fn validate(&self) -> bool {
        let field = get_field_by_symbol("sfLedgerEntryType");
        if !self.sle.is_field_present(field) {
            return false;
        }

        let Some(entry_type) = ledger_entry_type_from_code(self.sle.get_field_u16(field)) else {
            return false;
        };
        let Some(format) = LedgerFormats::get_instance().find_by_type(entry_type) else {
            return false;
        };
        validate_st_object(self.sle.as_ref(), format.so_template())
    }

    pub fn get_type(&self) -> LedgerEntryType {
        self.sle.get_type()
    }

    pub fn get_key(&self) -> Uint256 {
        *self.sle.key()
    }

    pub fn get_ledger_index(&self) -> Option<Uint256> {
        self.has_ledger_index().then(|| {
            self.sle
                .get_field_h256(get_field_by_symbol("sfLedgerIndex"))
        })
    }

    pub fn has_ledger_index(&self) -> bool {
        self.sle
            .is_field_present(get_field_by_symbol("sfLedgerIndex"))
    }

    pub fn get_ledger_entry_type(&self) -> u16 {
        self.sle
            .get_field_u16(get_field_by_symbol("sfLedgerEntryType"))
    }

    pub fn get_flags(&self) -> u32 {
        self.sle.get_field_u32(get_field_by_symbol("sfFlags"))
    }

    pub fn sle(&self) -> Arc<STLedgerEntry> {
        Arc::clone(&self.sle)
    }

    pub fn sle_ref(&self) -> &Arc<STLedgerEntry> {
        &self.sle
    }

    pub fn get_sle(&self) -> Arc<STLedgerEntry> {
        Arc::clone(&self.sle)
    }

    pub fn as_st_ledger_entry(&self) -> &STLedgerEntry {
        self.sle.as_ref()
    }

    pub fn into_sle(self) -> Arc<STLedgerEntry> {
        self.sle
    }
}
