//! Integration tests that pin the narrowed Rust `LoanSet.cpp::preclaim(...)`
//! `canAddHolding(...)` precheck to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::run_loan_set_preclaim_can_add_holding;

#[test]
fn tx_loan_set_preclaim_can_add_holding_returns_success_unchanged() {
    let result = run_loan_set_preclaim_can_add_holding(&"XRP", |asset| {
        assert_eq!(*asset, "XRP");
        Ter::TES_SUCCESS
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_set_preclaim_can_add_holding_returns_no_ripple_unchanged() {
    let result = run_loan_set_preclaim_can_add_holding(&"USD", |_| Ter::TER_NO_RIPPLE);

    assert_eq!(result, Ter::TER_NO_RIPPLE);
    assert_eq!(trans_token(result), "terNO_RIPPLE");
}

#[test]
fn tx_loan_set_preclaim_can_add_holding_returns_no_account_unchanged() {
    let result = run_loan_set_preclaim_can_add_holding(&"USD", |_| Ter::TER_NO_ACCOUNT);

    assert_eq!(result, Ter::TER_NO_ACCOUNT);
    assert_eq!(trans_token(result), "terNO_ACCOUNT");
}

#[test]
fn tx_loan_set_preclaim_can_add_holding_checks_asset_exactly_once() {
    let mut seen = Vec::new();

    let result = run_loan_set_preclaim_can_add_holding(&"USD", |asset| {
        seen.push(*asset);
        Ter::TER_NO_RIPPLE
    });

    assert_eq!(result, Ter::TER_NO_RIPPLE);
    assert_eq!(seen, vec!["USD"]);
}
