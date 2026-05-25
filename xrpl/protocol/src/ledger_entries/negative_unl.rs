use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegativeUNL {
    base: crate::LedgerEntryBase,
}

impl NegativeUNL {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::NegativeUnl;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for NegativeUNL".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBase::new(sle),
        })
    }

    pub fn get_disabled_validators(&self) -> Option<&crate::STArray> {
        self.base
            .as_st_ledger_entry()
            .peek_at_pfield(crate::get_field_by_symbol("sfDisabledValidators"))
            .and_then(|value| value.as_any().downcast_ref::<crate::STArray>())
    }

    pub fn has_disabled_validators(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfDisabledValidators"))
    }

    pub fn get_validator_to_disable(&self) -> Option<Vec<u8>> {
        self.has_validator_to_disable().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_vl(crate::get_field_by_symbol("sfValidatorToDisable"))
        })
    }

    pub fn has_validator_to_disable(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfValidatorToDisable"))
    }

    pub fn get_validator_to_re_enable(&self) -> Option<Vec<u8>> {
        self.has_validator_to_re_enable().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_vl(crate::get_field_by_symbol("sfValidatorToReEnable"))
        })
    }

    pub fn has_validator_to_re_enable(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfValidatorToReEnable"))
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

impl Deref for NegativeUNL {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegativeUNLBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl Default for NegativeUNLBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl NegativeUNLBuilder {
    pub fn new() -> Self {
        Self {
            base: crate::LedgerEntryBuilderBase::new(NegativeUNL::ENTRY_TYPE),
        }
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != NegativeUNL::ENTRY_TYPE {
            return Err("Invalid ledger entry type for NegativeUNL".to_owned());
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

    pub fn set_disabled_validators(mut self, value: crate::STArray) -> Self {
        self.base
            .object_mut()
            .set_field_array(crate::get_field_by_symbol("sfDisabledValidators"), value);
        self
    }

    pub fn set_validator_to_disable(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base.object_mut().set_field_vl(
            crate::get_field_by_symbol("sfValidatorToDisable"),
            value.as_ref(),
        );
        self
    }

    pub fn set_validator_to_re_enable(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base.object_mut().set_field_vl(
            crate::get_field_by_symbol("sfValidatorToReEnable"),
            value.as_ref(),
        );
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

    pub fn build(self, index: Uint256) -> NegativeUNL {
        NegativeUNL::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for NegativeUNLBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
