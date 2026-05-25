use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeSettings {
    base: crate::LedgerEntryBase,
}

impl FeeSettings {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::FeeSettings;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for FeeSettings".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBase::new(sle),
        })
    }

    pub fn get_base_fee(&self) -> Option<u64> {
        self.has_base_fee().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u64(crate::get_field_by_symbol("sfBaseFee"))
        })
    }

    pub fn has_base_fee(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfBaseFee"))
    }

    pub fn get_reference_fee_units(&self) -> Option<u32> {
        self.has_reference_fee_units().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfReferenceFeeUnits"))
        })
    }

    pub fn has_reference_fee_units(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfReferenceFeeUnits"))
    }

    pub fn get_reserve_base(&self) -> Option<u32> {
        self.has_reserve_base().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfReserveBase"))
        })
    }

    pub fn has_reserve_base(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfReserveBase"))
    }

    pub fn get_reserve_increment(&self) -> Option<u32> {
        self.has_reserve_increment().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfReserveIncrement"))
        })
    }

    pub fn has_reserve_increment(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfReserveIncrement"))
    }

    pub fn get_base_fee_drops(&self) -> Option<crate::STAmount> {
        self.has_base_fee_drops().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_amount(crate::get_field_by_symbol("sfBaseFeeDrops"))
        })
    }

    pub fn has_base_fee_drops(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfBaseFeeDrops"))
    }

    pub fn get_reserve_base_drops(&self) -> Option<crate::STAmount> {
        self.has_reserve_base_drops().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_amount(crate::get_field_by_symbol("sfReserveBaseDrops"))
        })
    }

    pub fn has_reserve_base_drops(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfReserveBaseDrops"))
    }

    pub fn get_reserve_increment_drops(&self) -> Option<crate::STAmount> {
        self.has_reserve_increment_drops().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_amount(crate::get_field_by_symbol("sfReserveIncrementDrops"))
        })
    }

    pub fn has_reserve_increment_drops(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfReserveIncrementDrops"))
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

impl Deref for FeeSettings {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeSettingsBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl Default for FeeSettingsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl FeeSettingsBuilder {
    pub fn new() -> Self {
        Self {
            base: crate::LedgerEntryBuilderBase::new(FeeSettings::ENTRY_TYPE),
        }
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != FeeSettings::ENTRY_TYPE {
            return Err("Invalid ledger entry type for FeeSettings".to_owned());
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

    pub fn set_base_fee(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfBaseFee"), value);
        self
    }

    pub fn set_reference_fee_units(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfReferenceFeeUnits"), value);
        self
    }

    pub fn set_reserve_base(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfReserveBase"), value);
        self
    }

    pub fn set_reserve_increment(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfReserveIncrement"), value);
        self
    }

    pub fn set_base_fee_drops(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfBaseFeeDrops"), value);
        self
    }

    pub fn set_reserve_base_drops(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfReserveBaseDrops"), value);
        self
    }

    pub fn set_reserve_increment_drops(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfReserveIncrementDrops"), value);
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

    pub fn build(self, index: Uint256) -> FeeSettings {
        FeeSettings::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for FeeSettingsBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
