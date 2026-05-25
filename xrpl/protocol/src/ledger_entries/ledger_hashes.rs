use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerHashes {
    base: crate::LedgerEntryBase,
}

impl LedgerHashes {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::LedgerHashes;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for LedgerHashes".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBase::new(sle),
        })
    }

    pub fn get_first_ledger_sequence(&self) -> Option<u32> {
        self.has_first_ledger_sequence().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfFirstLedgerSequence"))
        })
    }

    pub fn has_first_ledger_sequence(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfFirstLedgerSequence"))
    }

    pub fn get_last_ledger_sequence(&self) -> Option<u32> {
        self.has_last_ledger_sequence().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfLastLedgerSequence"))
        })
    }

    pub fn has_last_ledger_sequence(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfLastLedgerSequence"))
    }

    pub fn get_hashes(&self) -> crate::STVector256 {
        self.base
            .as_st_ledger_entry()
            .get_field_v256(crate::get_field_by_symbol("sfHashes"))
    }
}

impl Deref for LedgerHashes {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerHashesBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl LedgerHashesBuilder {
    pub fn new(hashes: crate::STVector256) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(LedgerHashes::ENTRY_TYPE),
        };
        builder = builder.set_hashes(hashes);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != LedgerHashes::ENTRY_TYPE {
            return Err("Invalid ledger entry type for LedgerHashes".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBuilderBase::from_sle(sle),
        })
    }

    pub fn set_ledger_index(mut self, value: Uint256) -> Self {
        self.base.set_ledger_index(value);
        self
    }

    pub fn set_flags(mut self, value: u32) -> Self {
        self.base.set_flags(value);
        self
    }

    pub fn set_first_ledger_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfFirstLedgerSequence"), value);
        self
    }

    pub fn set_last_ledger_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfLastLedgerSequence"), value);
        self
    }

    pub fn set_hashes(mut self, value: crate::STVector256) -> Self {
        self.base
            .object_mut()
            .set_field_v256(crate::get_field_by_symbol("sfHashes"), value);
        self
    }

    pub fn build(self, index: Uint256) -> LedgerHashes {
        LedgerHashes::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for LedgerHashesBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
