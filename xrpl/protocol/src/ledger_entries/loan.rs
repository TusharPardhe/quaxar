use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loan {
    base: crate::LedgerEntryBase,
}

impl Loan {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::Loan;

    #[allow(clippy::too_many_arguments)]
    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Loan".to_owned());
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

    pub fn get_owner_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfOwnerNode"))
    }

    pub fn get_loan_broker_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfLoanBrokerNode"))
    }

    pub fn get_loan_broker_id(&self) -> basics::base_uint::Uint256 {
        self.base
            .as_st_ledger_entry()
            .get_field_h256(crate::get_field_by_symbol("sfLoanBrokerID"))
    }

    pub fn get_loan_sequence(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfLoanSequence"))
    }

    pub fn get_borrower(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfBorrower"))
    }

    pub fn get_loan_origination_fee(&self) -> Option<crate::STNumber> {
        self.has_loan_origination_fee().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfLoanOriginationFee"))
        })
    }

    pub fn has_loan_origination_fee(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfLoanOriginationFee"))
    }

    pub fn get_loan_service_fee(&self) -> Option<crate::STNumber> {
        self.has_loan_service_fee().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfLoanServiceFee"))
        })
    }

    pub fn has_loan_service_fee(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfLoanServiceFee"))
    }

    pub fn get_late_payment_fee(&self) -> Option<crate::STNumber> {
        self.has_late_payment_fee().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfLatePaymentFee"))
        })
    }

    pub fn has_late_payment_fee(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfLatePaymentFee"))
    }

    pub fn get_close_payment_fee(&self) -> Option<crate::STNumber> {
        self.has_close_payment_fee().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfClosePaymentFee"))
        })
    }

    pub fn has_close_payment_fee(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfClosePaymentFee"))
    }

    pub fn get_overpayment_fee(&self) -> Option<u32> {
        self.has_overpayment_fee().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfOverpaymentFee"))
        })
    }

    pub fn has_overpayment_fee(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfOverpaymentFee"))
    }

    pub fn get_interest_rate(&self) -> Option<u32> {
        self.has_interest_rate().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfInterestRate"))
        })
    }

    pub fn has_interest_rate(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfInterestRate"))
    }

    pub fn get_late_interest_rate(&self) -> Option<u32> {
        self.has_late_interest_rate().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfLateInterestRate"))
        })
    }

    pub fn has_late_interest_rate(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfLateInterestRate"))
    }

    pub fn get_close_interest_rate(&self) -> Option<u32> {
        self.has_close_interest_rate().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfCloseInterestRate"))
        })
    }

    pub fn has_close_interest_rate(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfCloseInterestRate"))
    }

    pub fn get_overpayment_interest_rate(&self) -> Option<u32> {
        self.has_overpayment_interest_rate().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfOverpaymentInterestRate"))
        })
    }

    pub fn has_overpayment_interest_rate(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfOverpaymentInterestRate"))
    }

    pub fn get_start_date(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfStartDate"))
    }

    pub fn get_payment_interval(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfPaymentInterval"))
    }

    pub fn get_grace_period(&self) -> Option<u32> {
        self.has_grace_period().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfGracePeriod"))
        })
    }

    pub fn has_grace_period(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfGracePeriod"))
    }

    pub fn get_previous_payment_due_date(&self) -> Option<u32> {
        self.has_previous_payment_due_date().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfPreviousPaymentDueDate"))
        })
    }

    pub fn has_previous_payment_due_date(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfPreviousPaymentDueDate"))
    }

    pub fn get_next_payment_due_date(&self) -> Option<u32> {
        self.has_next_payment_due_date().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfNextPaymentDueDate"))
        })
    }

    pub fn has_next_payment_due_date(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfNextPaymentDueDate"))
    }

    pub fn get_payment_remaining(&self) -> Option<u32> {
        self.has_payment_remaining().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfPaymentRemaining"))
        })
    }

    pub fn has_payment_remaining(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfPaymentRemaining"))
    }

    pub fn get_periodic_payment(&self) -> crate::STNumber {
        self.base
            .as_st_ledger_entry()
            .get_field_number(crate::get_field_by_symbol("sfPeriodicPayment"))
    }

    pub fn get_principal_outstanding(&self) -> Option<crate::STNumber> {
        self.has_principal_outstanding().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfPrincipalOutstanding"))
        })
    }

    pub fn has_principal_outstanding(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfPrincipalOutstanding"))
    }

    pub fn get_total_value_outstanding(&self) -> Option<crate::STNumber> {
        self.has_total_value_outstanding().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfTotalValueOutstanding"))
        })
    }

    pub fn has_total_value_outstanding(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfTotalValueOutstanding"))
    }

    pub fn get_management_fee_outstanding(&self) -> Option<crate::STNumber> {
        self.has_management_fee_outstanding().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfManagementFeeOutstanding"))
        })
    }

    pub fn has_management_fee_outstanding(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfManagementFeeOutstanding"))
    }

    pub fn get_loan_scale(&self) -> Option<i32> {
        self.has_loan_scale().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_i32(crate::get_field_by_symbol("sfLoanScale"))
        })
    }

    pub fn has_loan_scale(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfLoanScale"))
    }
}

impl Deref for Loan {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl LoanBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        previous_txn_id: basics::base_uint::Uint256,
        previous_txn_lgr_seq: u32,
        owner_node: u64,
        loan_broker_node: u64,
        loan_broker_id: basics::base_uint::Uint256,
        loan_sequence: u32,
        borrower: crate::AccountID,
        start_date: u32,
        payment_interval: u32,
        periodic_payment: crate::STNumber,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(Loan::ENTRY_TYPE),
        };
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder = builder.set_owner_node(owner_node);
        builder = builder.set_loan_broker_node(loan_broker_node);
        builder = builder.set_loan_broker_id(loan_broker_id);
        builder = builder.set_loan_sequence(loan_sequence);
        builder = builder.set_borrower(borrower);
        builder = builder.set_start_date(start_date);
        builder = builder.set_payment_interval(payment_interval);
        builder = builder.set_periodic_payment(periodic_payment);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Loan::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Loan".to_owned());
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

    pub fn set_owner_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfOwnerNode"), value);
        self
    }

    pub fn set_loan_broker_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfLoanBrokerNode"), value);
        self
    }

    pub fn set_loan_broker_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfLoanBrokerID"), value);
        self
    }

    pub fn set_loan_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfLoanSequence"), value);
        self
    }

    pub fn set_borrower(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfBorrower"), value);
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

    pub fn set_start_date(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfStartDate"), value);
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

    pub fn set_previous_payment_due_date(mut self, value: u32) -> Self {
        self.base.object_mut().set_field_u32(
            crate::get_field_by_symbol("sfPreviousPaymentDueDate"),
            value,
        );
        self
    }

    pub fn set_next_payment_due_date(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfNextPaymentDueDate"), value);
        self
    }

    pub fn set_payment_remaining(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfPaymentRemaining"), value);
        self
    }

    pub fn set_periodic_payment(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfPeriodicPayment"), value);
        self
    }

    pub fn set_principal_outstanding(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfPrincipalOutstanding"), value);
        self
    }

    pub fn set_total_value_outstanding(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfTotalValueOutstanding"), value);
        self
    }

    pub fn set_management_fee_outstanding(mut self, value: crate::STNumber) -> Self {
        self.base.object_mut().set_field_number(
            crate::get_field_by_symbol("sfManagementFeeOutstanding"),
            value,
        );
        self
    }

    pub fn set_loan_scale(mut self, value: i32) -> Self {
        self.base
            .object_mut()
            .set_field_i32(crate::get_field_by_symbol("sfLoanScale"), value);
        self
    }

    pub fn build(self, index: Uint256) -> Loan {
        Loan::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for LoanBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
