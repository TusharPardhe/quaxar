//! Current Rust helper mirroring the pre-payment the reference implementation
//! unimpair branch.
//!
//! This module preserves the deterministic decision around:
//!
//! - if the loan is not impaired, do nothing and fall through;
//! - if the loan is impaired, run the kernel exactly once;
//! - if the kernel fails, return that error unchanged.
//!
//! The helper now owns the explicit branch decision while leaving the live
//! `unimpairLoan(...)` mutation kernel injected at the call boundary.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoanPayUnimpairFacts {
    pub loan_is_impaired: bool,
}

pub fn run_loan_pay_unimpair<F, E>(
    facts: LoanPayUnimpairFacts,
    mut unimpair_loan: F,
) -> Result<(), E>
where
    F: FnMut() -> Result<(), E>,
{
    if !facts.loan_is_impaired {
        return Ok(());
    }

    unimpair_loan()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{LoanPayUnimpairFacts, run_loan_pay_unimpair};

    #[test]
    fn loan_pay_unimpair_skips_the_kernel_when_the_loan_is_not_impaired() {
        let calls = Cell::new(0);

        let result = run_loan_pay_unimpair(
            LoanPayUnimpairFacts {
                loan_is_impaired: false,
            },
            || {
                calls.set(calls.get() + 1);
                Ok::<(), &'static str>(())
            },
        );

        assert_eq!(result, Ok(()));
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn loan_pay_unimpair_runs_the_kernel_exactly_once_when_impaired() {
        let calls = Cell::new(0);

        let result = run_loan_pay_unimpair(
            LoanPayUnimpairFacts {
                loan_is_impaired: true,
            },
            || {
                calls.set(calls.get() + 1);
                Ok::<(), &'static str>(())
            },
        );

        assert_eq!(result, Ok(()));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn loan_pay_unimpair_propagates_the_kernel_error_unchanged() {
        let calls = Cell::new(0);

        let result = run_loan_pay_unimpair(
            LoanPayUnimpairFacts {
                loan_is_impaired: true,
            },
            || {
                calls.set(calls.get() + 1);
                Err::<(), &'static str>("tecNO_PERMISSION")
            },
        );

        assert_eq!(result, Err("tecNO_PERMISSION"));
        assert_eq!(calls.get(), 1);
    }
}
