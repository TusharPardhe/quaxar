use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanBroker {
    base: crate::LedgerEntryBase,
}

impl LoanBroker {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::LoanBroker;

    #[allow(clippy::too_many_arguments)]
    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for LoanBroker".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBase::new(sle),
        })
    }

    pub fn get_previous_txn_id(&self) -> basics::base_uint::Uint256 {
        self.base
            .as_st_ledger_entry()
            .get_field_h256(crate::get_field_by_symbol("sfPreviousTxnID"))
    }

    pub fn get_previous_txn_lgr_seq(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfPreviousTxnLgrSeq"))
    }

    pub fn get_sequence(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfSequence"))
    }

    pub fn get_owner_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfOwnerNode"))
    }

    pub fn get_vault_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfVaultNode"))
    }

    pub fn get_vault_id(&self) -> basics::base_uint::Uint256 {
        self.base
            .as_st_ledger_entry()
            .get_field_h256(crate::get_field_by_symbol("sfVaultID"))
    }

    pub fn get_account(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfAccount"))
    }

    pub fn get_owner(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfOwner"))
    }

    pub fn get_loan_sequence(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfLoanSequence"))
    }

    pub fn get_data(&self) -> Option<Vec<u8>> {
        self.has_data().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_vl(crate::get_field_by_symbol("sfData"))
        })
    }

    pub fn has_data(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfData"))
    }

    pub fn get_management_fee_rate(&self) -> Option<u16> {
        self.has_management_fee_rate().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u16(crate::get_field_by_symbol("sfManagementFeeRate"))
        })
    }

    pub fn has_management_fee_rate(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfManagementFeeRate"))
    }

    pub fn get_owner_count(&self) -> Option<u32> {
        self.has_owner_count().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfOwnerCount"))
        })
    }

    pub fn has_owner_count(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfOwnerCount"))
    }

    pub fn get_debt_total(&self) -> Option<crate::STNumber> {
        self.has_debt_total().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfDebtTotal"))
        })
    }

    pub fn has_debt_total(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfDebtTotal"))
    }

    pub fn get_debt_maximum(&self) -> Option<crate::STNumber> {
        self.has_debt_maximum().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfDebtMaximum"))
        })
    }

    pub fn has_debt_maximum(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfDebtMaximum"))
    }

    pub fn get_cover_available(&self) -> Option<crate::STNumber> {
        self.has_cover_available().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfCoverAvailable"))
        })
    }

    pub fn has_cover_available(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfCoverAvailable"))
    }

    pub fn get_cover_rate_minimum(&self) -> Option<u32> {
        self.has_cover_rate_minimum().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfCoverRateMinimum"))
        })
    }

    pub fn has_cover_rate_minimum(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfCoverRateMinimum"))
    }

    pub fn get_cover_rate_liquidation(&self) -> Option<u32> {
        self.has_cover_rate_liquidation().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfCoverRateLiquidation"))
        })
    }

    pub fn has_cover_rate_liquidation(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfCoverRateLiquidation"))
    }
}

impl Deref for LoanBroker {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanBrokerBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl LoanBrokerBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        previous_txn_id: basics::base_uint::Uint256,
        previous_txn_lgr_seq: u32,
        sequence: u32,
        owner_node: u64,
        vault_node: u64,
        vault_id: basics::base_uint::Uint256,
        account: crate::AccountID,
        owner: crate::AccountID,
        loan_sequence: u32,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(LoanBroker::ENTRY_TYPE),
        };
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder = builder.set_sequence(sequence);
        builder = builder.set_owner_node(owner_node);
        builder = builder.set_vault_node(vault_node);
        builder = builder.set_vault_id(vault_id);
        builder = builder.set_account(account);
        builder = builder.set_owner(owner);
        builder = builder.set_loan_sequence(loan_sequence);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != LoanBroker::ENTRY_TYPE {
            return Err("Invalid ledger entry type for LoanBroker".to_owned());
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

    pub fn set_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfSequence"), value);
        self
    }

    pub fn set_owner_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfOwnerNode"), value);
        self
    }

    pub fn set_vault_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfVaultNode"), value);
        self
    }

    pub fn set_vault_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfVaultID"), value);
        self
    }

    pub fn set_account(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfAccount"), value);
        self
    }

    pub fn set_owner(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfOwner"), value);
        self
    }

    pub fn set_loan_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfLoanSequence"), value);
        self
    }

    pub fn set_data(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfData"), value.as_ref());
        self
    }

    pub fn set_management_fee_rate(mut self, value: u16) -> Self {
        self.base
            .object_mut()
            .set_field_u16(crate::get_field_by_symbol("sfManagementFeeRate"), value);
        self
    }

    pub fn set_owner_count(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfOwnerCount"), value);
        self
    }

    pub fn set_debt_total(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfDebtTotal"), value);
        self
    }

    pub fn set_debt_maximum(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfDebtMaximum"), value);
        self
    }

    pub fn set_cover_available(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfCoverAvailable"), value);
        self
    }

    pub fn set_cover_rate_minimum(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfCoverRateMinimum"), value);
        self
    }

    pub fn set_cover_rate_liquidation(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfCoverRateLiquidation"), value);
        self
    }

    pub fn build(self, index: Uint256) -> LoanBroker {
        LoanBroker::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for LoanBrokerBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
