//! Current Rust helper mirroring the duplicate-tolerant
//! the LoanSet transactor `addEmptyHolding(...)` wrapper.
//!
//! This module preserves the deterministic behavior around:
//!
//! - invoking the holding-creation attempt exactly once,
//! - treating `tesSUCCESS` as success,
//! - treating `tecDUPLICATE` as success because the holding already exists,
//! - returning any other `TER` unchanged.

use protocol::Ter;

pub fn run_loan_set_do_apply_add_empty_holding<AddEmptyHolding>(
    add_empty_holding: AddEmptyHolding,
) -> Ter
where
    AddEmptyHolding: FnOnce() -> Ter,
{
    match add_empty_holding() {
        Ter::TES_SUCCESS | Ter::TEC_DUPLICATE => Ter::TES_SUCCESS,
        ter => ter,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::trans_token;

    use super::run_loan_set_do_apply_add_empty_holding;

    #[test]
    fn loan_set_do_apply_add_empty_holding_returns_success() {
        let result = run_loan_set_do_apply_add_empty_holding(|| protocol::Ter::TES_SUCCESS);

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_set_do_apply_add_empty_holding_ignores_duplicate() {
        let result = run_loan_set_do_apply_add_empty_holding(|| protocol::Ter::TEC_DUPLICATE);

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_set_do_apply_add_empty_holding_returns_other_failure_unchanged() {
        let result = run_loan_set_do_apply_add_empty_holding(|| protocol::Ter::TER_NO_RIPPLE);

        assert_eq!(result, protocol::Ter::TER_NO_RIPPLE);
        assert_eq!(trans_token(result), "terNO_RIPPLE");
    }

    #[test]
    fn loan_set_do_apply_add_empty_holding_calls_the_attempt_once() {
        let calls = Cell::new(0_u32);

        let result = run_loan_set_do_apply_add_empty_holding(|| {
            calls.set(calls.get() + 1);
            protocol::Ter::TEC_DUPLICATE
        });

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(calls.get(), 1);
    }
}
