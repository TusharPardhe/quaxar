use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanBrokerSet {
    base: crate::TransactionBase,
}

impl LoanBrokerSet {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(74);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for LoanBrokerSet".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_vault_id(&self) -> basics::base_uint::Uint256 {
        self.base
            .as_sttx()
            .get_field_h256(crate::get_field_by_symbol("sfVaultID"))
    }

    pub fn get_loan_broker_id(&self) -> Option<basics::base_uint::Uint256> {
        self.has_loan_broker_id().then(|| {
            self.base
                .as_sttx()
                .get_field_h256(crate::get_field_by_symbol("sfLoanBrokerID"))
        })
    }

    pub fn has_loan_broker_id(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfLoanBrokerID"))
    }

    pub fn get_data(&self) -> Option<Vec<u8>> {
        self.has_data().then(|| {
            self.base
                .as_sttx()
                .get_field_vl(crate::get_field_by_symbol("sfData"))
        })
    }

    pub fn has_data(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfData"))
    }

    pub fn get_management_fee_rate(&self) -> Option<u16> {
        self.has_management_fee_rate().then(|| {
            self.base
                .as_sttx()
                .get_field_u16(crate::get_field_by_symbol("sfManagementFeeRate"))
        })
    }

    pub fn has_management_fee_rate(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfManagementFeeRate"))
    }

    pub fn get_debt_maximum(&self) -> Option<crate::STNumber> {
        self.has_debt_maximum().then(|| {
            self.base
                .as_sttx()
                .get_field_number(crate::get_field_by_symbol("sfDebtMaximum"))
        })
    }

    pub fn has_debt_maximum(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfDebtMaximum"))
    }

    pub fn get_cover_rate_minimum(&self) -> Option<u32> {
        self.has_cover_rate_minimum().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfCoverRateMinimum"))
        })
    }

    pub fn has_cover_rate_minimum(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfCoverRateMinimum"))
    }

    pub fn get_cover_rate_liquidation(&self) -> Option<u32> {
        self.has_cover_rate_liquidation().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfCoverRateLiquidation"))
        })
    }

    pub fn has_cover_rate_liquidation(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfCoverRateLiquidation"))
    }
}

impl Deref for LoanBrokerSet {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanBrokerSetBuilder {
    base: crate::TransactionBuilderBase,
}

impl LoanBrokerSetBuilder {
    pub fn new(
        account: crate::AccountID,
        vault_id: basics::base_uint::Uint256,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(
                LoanBrokerSet::TX_TYPE,
                account,
                sequence,
                fee,
            ),
        };
        builder = builder.set_vault_id(vault_id);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != LoanBrokerSet::TX_TYPE {
            return Err("Invalid transaction type for LoanBrokerSetBuilder".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBuilderBase::from_tx(tx),
        })
    }

    pub fn set_account(mut self, value: crate::AccountID) -> Self {
        self.base.set_account(value);
        self
    }

    pub fn set_fee(mut self, value: crate::STAmount) -> Self {
        self.base.set_fee(value);
        self
    }

    pub fn set_sequence(mut self, value: u32) -> Self {
        self.base.set_sequence(value);
        self
    }

    pub fn set_ticket_sequence(mut self, value: u32) -> Self {
        self.base.set_ticket_sequence(value);
        self
    }

    pub fn set_flags(mut self, value: u32) -> Self {
        self.base.set_flags(value);
        self
    }

    pub fn set_source_tag(mut self, value: u32) -> Self {
        self.base.set_source_tag(value);
        self
    }

    pub fn set_last_ledger_sequence(mut self, value: u32) -> Self {
        self.base.set_last_ledger_sequence(value);
        self
    }

    pub fn set_account_txn_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base.set_account_txn_id(value);
        self
    }

    pub fn set_previous_txn_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base.set_previous_txn_id(value);
        self
    }

    pub fn set_operation_limit(mut self, value: u32) -> Self {
        self.base.set_operation_limit(value);
        self
    }

    pub fn set_memos(mut self, value: crate::STArray) -> Self {
        self.base.set_memos(value);
        self
    }

    pub fn set_signers(mut self, value: crate::STArray) -> Self {
        self.base.set_signers(value);
        self
    }

    pub fn set_network_id(mut self, value: u32) -> Self {
        self.base.set_network_id(value);
        self
    }

    pub fn set_delegate(mut self, value: crate::AccountID) -> Self {
        self.base.set_delegate(value);
        self
    }

    pub fn get_st_object(&self) -> &crate::STObject {
        self.base.object()
    }

    pub fn set_vault_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfVaultID"), value);
        self
    }

    pub fn set_loan_broker_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfLoanBrokerID"), value);
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

    pub fn set_debt_maximum(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfDebtMaximum"), value);
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

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<LoanBrokerSet, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(LoanBrokerSet::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
