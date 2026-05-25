//! `requireAuth(...)` wrapper for the LoanSet transactor.
//!
//! This module preserves the deterministic behavior around:
//!
//! - invoking the auth check exactly once after the current
//!   duplicate-tolerant holding wrapper, and
//! - returning the helper's `TER` result unchanged.

use protocol::Ter;

pub fn run_loan_set_do_apply_require_auth<CheckRequireAuth>(
    check_require_auth: CheckRequireAuth,
) -> Ter
where
    CheckRequireAuth: FnOnce() -> Ter,
{
    check_require_auth()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::{Ter, trans_token};

    use super::run_loan_set_do_apply_require_auth;

    #[test]
    fn loan_set_do_apply_require_auth_returns_success_unchanged() {
        let result = run_loan_set_do_apply_require_auth(|| Ter::TES_SUCCESS);

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_set_do_apply_require_auth_returns_no_ripple_unchanged() {
        let result = run_loan_set_do_apply_require_auth(|| Ter::TER_NO_RIPPLE);

        assert_eq!(result, Ter::TER_NO_RIPPLE);
        assert_eq!(trans_token(result), "terNO_RIPPLE");
    }

    #[test]
    fn loan_set_do_apply_require_auth_returns_no_line_insuf_reserve_unchanged() {
        let result = run_loan_set_do_apply_require_auth(|| Ter::TEC_NO_LINE_INSUF_RESERVE);

        assert_eq!(result, Ter::TEC_NO_LINE_INSUF_RESERVE);
        assert_eq!(trans_token(result), "tecNO_LINE_INSUF_RESERVE");
    }

    #[test]
    fn loan_set_do_apply_require_auth_calls_the_helper_once() {
        let calls = Cell::new(0_u32);

        let result = run_loan_set_do_apply_require_auth(|| {
            calls.set(calls.get() + 1);
            Ter::TER_NO_RIPPLE
        });

        assert_eq!(result, Ter::TER_NO_RIPPLE);
        assert_eq!(calls.get(), 1);
    }
}
