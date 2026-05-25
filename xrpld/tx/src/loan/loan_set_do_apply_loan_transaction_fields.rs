//! Transaction-copy helper for `setLoanField(...)` inside
//! the LoanSet transactor.
//!
//! This module preserves the exact deterministic behavior around:
//!
//! - the copied field list,
//! - the current field-copy order, and
//! - the one explicit `GracePeriod` default.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSetDoApplyLoanTransactionField {
    LoanOriginationFee,
    LoanServiceFee,
    LatePaymentFee,
    ClosePaymentFee,
    OverpaymentFee,
    InterestRate,
    LateInterestRate,
    CloseInterestRate,
    OverpaymentInterestRate,
    GracePeriod,
}

pub fn run_loan_set_do_apply_loan_transaction_fields<CopyField>(
    default_grace_period: u32,
    mut copy_field: CopyField,
) where
    CopyField: FnMut(LoanSetDoApplyLoanTransactionField, Option<u32>),
{
    copy_field(LoanSetDoApplyLoanTransactionField::LoanOriginationFee, None);
    copy_field(LoanSetDoApplyLoanTransactionField::LoanServiceFee, None);
    copy_field(LoanSetDoApplyLoanTransactionField::LatePaymentFee, None);
    copy_field(LoanSetDoApplyLoanTransactionField::ClosePaymentFee, None);
    copy_field(LoanSetDoApplyLoanTransactionField::OverpaymentFee, None);
    copy_field(LoanSetDoApplyLoanTransactionField::InterestRate, None);
    copy_field(LoanSetDoApplyLoanTransactionField::LateInterestRate, None);
    copy_field(LoanSetDoApplyLoanTransactionField::CloseInterestRate, None);
    copy_field(
        LoanSetDoApplyLoanTransactionField::OverpaymentInterestRate,
        None,
    );
    copy_field(
        LoanSetDoApplyLoanTransactionField::GracePeriod,
        Some(default_grace_period),
    );
}

#[cfg(test)]
mod tests {
    use super::{
        LoanSetDoApplyLoanTransactionField, run_loan_set_do_apply_loan_transaction_fields,
    };

    #[test]
    fn loan_set_do_apply_loan_transaction_fields_uses_current_cpp_copy_order() {
        let mut seen = Vec::new();

        run_loan_set_do_apply_loan_transaction_fields(30, |field, default_value| {
            seen.push((field, default_value));
        });

        assert_eq!(
            seen,
            vec![
                (LoanSetDoApplyLoanTransactionField::LoanOriginationFee, None),
                (LoanSetDoApplyLoanTransactionField::LoanServiceFee, None),
                (LoanSetDoApplyLoanTransactionField::LatePaymentFee, None),
                (LoanSetDoApplyLoanTransactionField::ClosePaymentFee, None),
                (LoanSetDoApplyLoanTransactionField::OverpaymentFee, None),
                (LoanSetDoApplyLoanTransactionField::InterestRate, None),
                (LoanSetDoApplyLoanTransactionField::LateInterestRate, None),
                (LoanSetDoApplyLoanTransactionField::CloseInterestRate, None),
                (
                    LoanSetDoApplyLoanTransactionField::OverpaymentInterestRate,
                    None
                ),
                (LoanSetDoApplyLoanTransactionField::GracePeriod, Some(30)),
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_loan_transaction_fields_only_defaults_grace_period() {
        let mut seen_non_none = Vec::new();

        run_loan_set_do_apply_loan_transaction_fields(45, |field, default_value| {
            if default_value.is_some() {
                seen_non_none.push((field, default_value));
            }
        });

        assert_eq!(
            seen_non_none,
            vec![(LoanSetDoApplyLoanTransactionField::GracePeriod, Some(45))]
        );
    }

    #[test]
    fn loan_set_do_apply_loan_transaction_fields_passes_default_grace_period_unchanged() {
        let mut grace_default = None;

        run_loan_set_do_apply_loan_transaction_fields(0, |field, default_value| {
            if field == LoanSetDoApplyLoanTransactionField::GracePeriod {
                grace_default = default_value;
            }
        });

        assert_eq!(grace_default, Some(0));
    }
}
