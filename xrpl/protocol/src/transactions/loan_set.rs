use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSet {
    base: crate::TransactionBase,
}

impl LoanSet {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(80);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for LoanSet".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_loan_broker_id(&self) -> basics::base_uint::Uint256 {
        self.base
            .as_sttx()
            .get_field_h256(crate::get_field_by_symbol("sfLoanBrokerID"))
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

    pub fn get_counterparty(&self) -> Option<crate::AccountID> {
        self.has_counterparty().then(|| {
            self.base
                .as_sttx()
                .get_account_id(crate::get_field_by_symbol("sfCounterparty"))
        })
    }

    pub fn has_counterparty(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfCounterparty"))
    }

    pub fn get_counterparty_signature(&self) -> Option<crate::STObject> {
        self.has_counterparty_signature().then(|| {
            self.base
                .as_sttx()
                .get_field_object(crate::get_field_by_symbol("sfCounterpartySignature"))
        })
    }

    pub fn has_counterparty_signature(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfCounterpartySignature"))
    }

    pub fn get_loan_origination_fee(&self) -> Option<crate::STNumber> {
        self.has_loan_origination_fee().then(|| {
            self.base
                .as_sttx()
                .get_field_number(crate::get_field_by_symbol("sfLoanOriginationFee"))
        })
    }

    pub fn has_loan_origination_fee(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfLoanOriginationFee"))
    }

    pub fn get_loan_service_fee(&self) -> Option<crate::STNumber> {
        self.has_loan_service_fee().then(|| {
            self.base
                .as_sttx()
                .get_field_number(crate::get_field_by_symbol("sfLoanServiceFee"))
        })
    }

    pub fn has_loan_service_fee(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfLoanServiceFee"))
    }

    pub fn get_late_payment_fee(&self) -> Option<crate::STNumber> {
        self.has_late_payment_fee().then(|| {
            self.base
                .as_sttx()
                .get_field_number(crate::get_field_by_symbol("sfLatePaymentFee"))
        })
    }

    pub fn has_late_payment_fee(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfLatePaymentFee"))
    }

    pub fn get_close_payment_fee(&self) -> Option<crate::STNumber> {
        self.has_close_payment_fee().then(|| {
            self.base
                .as_sttx()
                .get_field_number(crate::get_field_by_symbol("sfClosePaymentFee"))
        })
    }

    pub fn has_close_payment_fee(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfClosePaymentFee"))
    }

    pub fn get_overpayment_fee(&self) -> Option<u32> {
        self.has_overpayment_fee().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfOverpaymentFee"))
        })
    }

    pub fn has_overpayment_fee(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfOverpaymentFee"))
    }

    pub fn get_interest_rate(&self) -> Option<u32> {
        self.has_interest_rate().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfInterestRate"))
        })
    }

    pub fn has_interest_rate(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfInterestRate"))
    }

    pub fn get_late_interest_rate(&self) -> Option<u32> {
        self.has_late_interest_rate().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfLateInterestRate"))
        })
    }

    pub fn has_late_interest_rate(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfLateInterestRate"))
    }

    pub fn get_close_interest_rate(&self) -> Option<u32> {
        self.has_close_interest_rate().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfCloseInterestRate"))
        })
    }

    pub fn has_close_interest_rate(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfCloseInterestRate"))
    }

    pub fn get_overpayment_interest_rate(&self) -> Option<u32> {
        self.has_overpayment_interest_rate().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfOverpaymentInterestRate"))
        })
    }

    pub fn has_overpayment_interest_rate(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfOverpaymentInterestRate"))
    }

    pub fn get_principal_requested(&self) -> crate::STNumber {
        self.base
            .as_sttx()
            .get_field_number(crate::get_field_by_symbol("sfPrincipalRequested"))
    }

    pub fn get_payment_total(&self) -> Option<u32> {
        self.has_payment_total().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfPaymentTotal"))
        })
    }

    pub fn has_payment_total(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfPaymentTotal"))
    }

    pub fn get_payment_interval(&self) -> Option<u32> {
        self.has_payment_interval().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfPaymentInterval"))
        })
    }

    pub fn has_payment_interval(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfPaymentInterval"))
    }

    pub fn get_grace_period(&self) -> Option<u32> {
        self.has_grace_period().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfGracePeriod"))
        })
    }

    pub fn has_grace_period(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfGracePeriod"))
    }
}

impl Deref for LoanSet {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetBuilder {
    base: crate::TransactionBuilderBase,
}

impl LoanSetBuilder {
    pub fn new(
        account: crate::AccountID,
        loan_broker_id: basics::base_uint::Uint256,
        principal_requested: crate::STNumber,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(LoanSet::TX_TYPE, account, sequence, fee),
        };
        builder = builder.set_loan_broker_id(loan_broker_id);
        builder = builder.set_principal_requested(principal_requested);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != LoanSet::TX_TYPE {
            return Err("Invalid transaction type for LoanSetBuilder".to_owned());
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

    pub fn set_counterparty(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfCounterparty"), value);
        self
    }

    pub fn set_counterparty_signature(mut self, value: crate::STObject) -> Self {
        self.base
            .object_mut()
            .set_field_object(crate::get_field_by_symbol("sfCounterpartySignature"), value);
        self
    }

    pub fn set_loan_origination_fee(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfLoanOriginationFee"), value);
        self
    }

    pub fn set_loan_service_fee(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfLoanServiceFee"), value);
        self
    }

    pub fn set_late_payment_fee(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfLatePaymentFee"), value);
        self
    }

    pub fn set_close_payment_fee(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfClosePaymentFee"), value);
        self
    }

    pub fn set_overpayment_fee(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfOverpaymentFee"), value);
        self
    }

    pub fn set_interest_rate(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfInterestRate"), value);
        self
    }

    pub fn set_late_interest_rate(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfLateInterestRate"), value);
        self
    }

    pub fn set_close_interest_rate(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfCloseInterestRate"), value);
        self
    }

    pub fn set_overpayment_interest_rate(mut self, value: u32) -> Self {
        self.base.object_mut().set_field_u32(
            crate::get_field_by_symbol("sfOverpaymentInterestRate"),
            value,
        );
        self
    }

    pub fn set_principal_requested(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfPrincipalRequested"), value);
        self
    }

    pub fn set_payment_total(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfPaymentTotal"), value);
        self
    }

    pub fn set_payment_interval(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfPaymentInterval"), value);
        self
    }

    pub fn set_grace_period(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfGracePeriod"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<LoanSet, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(LoanSet::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
