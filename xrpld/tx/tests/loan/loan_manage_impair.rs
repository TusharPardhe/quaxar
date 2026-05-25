//! Integration tests that pin the narrowed Rust `LoanManage.cpp`
//! `impairLoan(...)` and `unimpairLoan(...)` helper behavior to the current
//! C++ branching.

use protocol::Ter;
use tx::loan_manage_impair::{
    LoanManageImpairFacts, LoanManageImpairOutcome, LoanManageUnimpairFacts,
    LoanManageUnimpairOutcome, run_loan_manage_impair, run_loan_manage_unimpair,
};

#[test]
fn tx_loan_manage_impair_rejects_unrealized_loss_that_exceeds_unavailable_assets() {
    let result = run_loan_manage_impair(LoanManageImpairFacts {
        total_value_outstanding: 150_i64,
        management_fee_outstanding: 25_i64,
        vault_loss_unrealized: 60_i64,
        vault_assets_total: 200_i64,
        vault_assets_available: 20_i64,
        loan_next_payment_due_date: 10_i64,
        loan_next_payment_due_has_expired: false,
        parent_close_time: 30_i64,
    });

    assert_eq!(result, Err(Ter::TEC_LIMIT_EXCEEDED));
}

#[test]
fn tx_loan_manage_impair_sets_due_date_to_close_time_when_not_yet_late() {
    let result = run_loan_manage_impair(LoanManageImpairFacts {
        total_value_outstanding: 125_i64,
        management_fee_outstanding: 25_i64,
        vault_loss_unrealized: 10_i64,
        vault_assets_total: 500_i64,
        vault_assets_available: 300_i64,
        loan_next_payment_due_date: 10_i64,
        loan_next_payment_due_has_expired: false,
        parent_close_time: 40_i64,
    });

    assert_eq!(
        result,
        Ok(LoanManageImpairOutcome {
            loss_unrealized: 100_i64,
            vault_loss_unrealized: 110_i64,
            loan_is_impaired: true,
            loan_next_payment_due_date: 40_i64,
        })
    );
}

#[test]
fn tx_loan_manage_impair_keeps_existing_due_date_when_already_late() {
    let result = run_loan_manage_impair(LoanManageImpairFacts {
        total_value_outstanding: 125_i64,
        management_fee_outstanding: 25_i64,
        vault_loss_unrealized: 10_i64,
        vault_assets_total: 500_i64,
        vault_assets_available: 300_i64,
        loan_next_payment_due_date: 10_i64,
        loan_next_payment_due_has_expired: true,
        parent_close_time: 40_i64,
    });

    assert_eq!(
        result,
        Ok(LoanManageImpairOutcome {
            loss_unrealized: 100_i64,
            vault_loss_unrealized: 110_i64,
            loan_is_impaired: true,
            loan_next_payment_due_date: 10_i64,
        })
    );
}

#[test]
fn tx_loan_manage_unimpair_rejects_reversing_more_loss_than_exists() {
    let result = run_loan_manage_unimpair(LoanManageUnimpairFacts {
        total_value_outstanding: 125_i64,
        management_fee_outstanding: 25_i64,
        vault_loss_unrealized: 90_i64,
        previous_payment_due_date: 10_i64,
        start_date: 5_i64,
        payment_interval: 30_i64,
        normal_payment_due_has_expired: false,
        parent_close_time: 40_i64,
    });

    assert_eq!(result, Err(Ter::TEF_BAD_LEDGER));
}

#[test]
fn tx_loan_manage_unimpair_restores_normal_due_date_when_still_within_interval() {
    let result = run_loan_manage_unimpair(LoanManageUnimpairFacts {
        total_value_outstanding: 125_i64,
        management_fee_outstanding: 25_i64,
        vault_loss_unrealized: 150_i64,
        previous_payment_due_date: 10_i64,
        start_date: 5_i64,
        payment_interval: 30_i64,
        normal_payment_due_has_expired: false,
        parent_close_time: 40_i64,
    });

    assert_eq!(
        result,
        Ok(LoanManageUnimpairOutcome {
            loss_reversed: 100_i64,
            vault_loss_unrealized: 50_i64,
            loan_is_impaired: false,
            loan_next_payment_due_date: 40_i64,
        })
    );
}

#[test]
fn tx_loan_manage_unimpair_shifts_due_date_forward_when_interval_has_passed() {
    let result = run_loan_manage_unimpair(LoanManageUnimpairFacts {
        total_value_outstanding: 125_i64,
        management_fee_outstanding: 25_i64,
        vault_loss_unrealized: 150_i64,
        previous_payment_due_date: 10_i64,
        start_date: 50_i64,
        payment_interval: 30_i64,
        normal_payment_due_has_expired: true,
        parent_close_time: 40_i64,
    });

    assert_eq!(
        result,
        Ok(LoanManageUnimpairOutcome {
            loss_reversed: 100_i64,
            vault_loss_unrealized: 50_i64,
            loan_is_impaired: false,
            loan_next_payment_due_date: 70_i64,
        })
    );
}
