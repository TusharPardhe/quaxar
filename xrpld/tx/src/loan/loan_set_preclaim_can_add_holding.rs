//! `canAddHolding(...)` precheck for the reference implementation.
//!
//! This module ports the deterministic behavior around:
//!
//! - invoking `canAddHolding(view, asset)` exactly once after the current
//!   representability guard, and
//! - returning the helper's `TER` result unchanged.

use protocol::Ter;

pub fn run_loan_set_preclaim_can_add_holding<Asset, CheckCanAddHolding>(
    asset: &Asset,
    check_can_add_holding: CheckCanAddHolding,
) -> Ter
where
    CheckCanAddHolding: FnOnce(&Asset) -> Ter,
{
    check_can_add_holding(asset)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::{Ter, trans_token};

    use super::run_loan_set_preclaim_can_add_holding;

    #[test]
    fn loan_set_preclaim_can_add_holding_returns_success_unchanged() {
        let result = run_loan_set_preclaim_can_add_holding(&"XRP", |asset| {
            assert_eq!(*asset, "XRP");
            Ter::TES_SUCCESS
        });

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_set_preclaim_can_add_holding_returns_no_ripple_unchanged() {
        let result = run_loan_set_preclaim_can_add_holding(&"USD", |_| Ter::TER_NO_RIPPLE);

        assert_eq!(result, Ter::TER_NO_RIPPLE);
        assert_eq!(trans_token(result), "terNO_RIPPLE");
    }

    #[test]
    fn loan_set_preclaim_can_add_holding_returns_no_account_unchanged() {
        let result = run_loan_set_preclaim_can_add_holding(&"USD", |_| Ter::TER_NO_ACCOUNT);

        assert_eq!(result, Ter::TER_NO_ACCOUNT);
        assert_eq!(trans_token(result), "terNO_ACCOUNT");
    }

    #[test]
    fn loan_set_preclaim_can_add_holding_checks_asset_exactly_once() {
        let seen = RefCell::new(Vec::new());

        let result = run_loan_set_preclaim_can_add_holding(&"USD", |asset| {
            seen.borrow_mut().push(*asset);
            Ter::TER_NO_RIPPLE
        });

        assert_eq!(result, Ter::TER_NO_RIPPLE);
        assert_eq!(*seen.borrow(), vec!["USD"]);
    }
}
