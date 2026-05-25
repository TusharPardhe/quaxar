use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amendments {
    base: crate::LedgerEntryBase,
}

impl Amendments {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::Amendments;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Amendments".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBase::new(sle),
        })
    }

    pub fn get_amendments(&self) -> Option<crate::STVector256> {
        self.has_amendments().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_v256(crate::get_field_by_symbol("sfAmendments"))
        })
    }

    pub fn has_amendments(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfAmendments"))
    }

    pub fn get_majorities(&self) -> Option<&crate::STArray> {
        self.base
            .as_st_ledger_entry()
            .peek_at_pfield(crate::get_field_by_symbol("sfMajorities"))
            .and_then(|value| value.as_any().downcast_ref::<crate::STArray>())
    }

    pub fn has_majorities(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfMajorities"))
    }

    pub fn get_previous_txn_id(&self) -> Option<basics::base_uint::Uint256> {
        self.has_previous_txn_id().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h256(crate::get_field_by_symbol("sfPreviousTxnID"))
        })
    }

    pub fn has_previous_txn_id(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfPreviousTxnID"))
    }

    pub fn get_previous_txn_lgr_seq(&self) -> Option<u32> {
        self.has_previous_txn_lgr_seq().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfPreviousTxnLgrSeq"))
        })
    }

    pub fn has_previous_txn_lgr_seq(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfPreviousTxnLgrSeq"))
    }
}

impl Deref for Amendments {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendmentsBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl Default for AmendmentsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AmendmentsBuilder {
    pub fn new() -> Self {
        Self {
            base: crate::LedgerEntryBuilderBase::new(Amendments::ENTRY_TYPE),
        }
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Amendments::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Amendments".to_owned());
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

    pub fn set_amendments(mut self, value: crate::STVector256) -> Self {
        self.base
            .object_mut()
            .set_field_v256(crate::get_field_by_symbol("sfAmendments"), value);
        self
    }

    pub fn set_majorities(mut self, value: crate::STArray) -> Self {
        self.base
            .object_mut()
            .set_field_array(crate::get_field_by_symbol("sfMajorities"), value);
        self
    }

    pub fn set_previous_txn_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfPreviousTxnID"), value);
        self
    }

    pub fn set_previous_txn_lgr_seq(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfPreviousTxnLgrSeq"), value);
        self
    }

    pub fn build(self, index: Uint256) -> Amendments {
        Amendments::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for AmendmentsBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
