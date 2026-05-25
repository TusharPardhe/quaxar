//! `checkLoanGuards(...)` wrapper for the LoanSet transactor.
//!
//! This module ports the deterministic behavior around:
//!
//! - invoking `checkLoanGuards(...)` exactly once after the current
//!   representability loop, and
//! - returning the helper's `TER` unchanged.

use protocol::Ter;

pub fn run_loan_set_do_apply_check_loan_guards<Asset, Amount, Properties, CheckLoanGuards>(
    vault_asset: &Asset,
    principal_requested: &Amount,
    expect_interest: bool,
    payment_total: u32,
    properties: &Properties,
    check_loan_guards: CheckLoanGuards,
) -> Ter
where
    CheckLoanGuards: FnOnce(&Asset, &Amount, bool, u32, &Properties) -> Ter,
{
    check_loan_guards(
        vault_asset,
        principal_requested,
        expect_interest,
        payment_total,
        properties,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::{Ter, trans_token};

    use super::run_loan_set_do_apply_check_loan_guards;

    #[test]
    fn loan_set_do_apply_check_loan_guards_returns_success_unchanged() {
        let result = run_loan_set_do_apply_check_loan_guards(
            &"USD",
            &"100",
            true,
            12,
            &"props",
            |asset, principal, expect_interest, payment_total, properties| {
                assert_eq!(*asset, "USD");
                assert_eq!(*principal, "100");
                assert!(expect_interest);
                assert_eq!(payment_total, 12);
                assert_eq!(*properties, "props");
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_set_do_apply_check_loan_guards_returns_precision_loss_unchanged() {
        let result = run_loan_set_do_apply_check_loan_guards(
            &"USD",
            &"100",
            true,
            12,
            &"props",
            |_, _, _, _, _| Ter::TEC_PRECISION_LOSS,
        );

        assert_eq!(result, Ter::TEC_PRECISION_LOSS);
        assert_eq!(trans_token(result), "tecPRECISION_LOSS");
    }

    #[test]
    fn loan_set_do_apply_check_loan_guards_returns_internal_unchanged() {
        let result = run_loan_set_do_apply_check_loan_guards(
            &"USD",
            &"100",
            false,
            12,
            &"props",
            |_, _, _, _, _| Ter::TEC_INTERNAL,
        );

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(trans_token(result), "tecINTERNAL");
    }

    #[test]
    fn loan_set_do_apply_check_loan_guards_invokes_helper_once_with_cpp_argument_order() {
        let seen = RefCell::new(Vec::new());

        let result = run_loan_set_do_apply_check_loan_guards(
            &"XRP",
            &"250",
            false,
            24,
            &"loan-props",
            |asset, principal, expect_interest, payment_total, properties| {
                seen.borrow_mut().push(format!(
                    "{asset}:{principal}:{expect_interest}:{payment_total}:{properties}"
                ));
                Ter::TEC_PRECISION_LOSS
            },
        );

        assert_eq!(result, Ter::TEC_PRECISION_LOSS);
        assert_eq!(
            *seen.borrow(),
            vec!["XRP:250:false:24:loan-props".to_string()]
        );
    }
}
