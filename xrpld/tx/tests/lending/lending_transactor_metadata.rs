//! Integration tests that pin the narrowed Rust remaining lending transactor
//! metadata helpers to the current C++ behavior.

use std::cell::Cell;

use tx::lending::{
    run_loan_broker_cover_deposit_check_extra_features, run_loan_broker_set_check_extra_features,
    run_loan_delete_check_extra_features,
};
use tx::{
    LOAN_DEFAULT_FLAG, LOAN_FULL_PAYMENT_FLAG, LOAN_IMPAIR_FLAG, LOAN_LATE_PAYMENT_FLAG,
    LOAN_MANAGE_FLAGS_MASK, LOAN_PAY_FLAGS_MASK, LOAN_PAY_OVERPAYMENT_FLAG, LOAN_UNIMPAIR_FLAG,
    get_loan_manage_flags_mask, get_loan_pay_flags_mask,
    run_loan_broker_cover_clawback_check_extra_features,
    run_loan_broker_cover_withdraw_check_extra_features,
    run_loan_broker_delete_check_extra_features, run_loan_manage_check_extra_features,
    run_loan_pay_check_extra_features,
};

macro_rules! assert_lending_wrapper {
    ($wrapper:ident) => {{
        let helper_called = Cell::new(false);
        let result = $wrapper(false, || {
            helper_called.set(true);
            true
        });
        assert!(!result);
        assert!(!helper_called.get());
        assert!($wrapper(true, || true));
        assert!(!$wrapper(true, || false));
    }};
}

#[test]
fn lending_transactor_check_extra_features_wrappers_delegate() {
    assert_lending_wrapper!(run_loan_broker_cover_clawback_check_extra_features);
    assert_lending_wrapper!(run_loan_broker_cover_deposit_check_extra_features);
    assert_lending_wrapper!(run_loan_broker_cover_withdraw_check_extra_features);
    assert_lending_wrapper!(run_loan_broker_delete_check_extra_features);
    assert_lending_wrapper!(run_loan_broker_set_check_extra_features);
    assert_lending_wrapper!(run_loan_delete_check_extra_features);
    assert_lending_wrapper!(run_loan_manage_check_extra_features);
    assert_lending_wrapper!(run_loan_pay_check_extra_features);
}

#[test]
fn loan_pay_flags_mask_txflags() {
    assert_eq!(LOAN_PAY_OVERPAYMENT_FLAG, 0x0001_0000);
    assert_eq!(LOAN_FULL_PAYMENT_FLAG, 0x0002_0000);
    assert_eq!(LOAN_LATE_PAYMENT_FLAG, 0x0004_0000);
    assert_eq!(LOAN_PAY_FLAGS_MASK, 0x3ff8_ffff);
    assert_eq!(get_loan_pay_flags_mask(), 0x3ff8_ffff);
}

#[test]
fn loan_manage_flags_mask_txflags() {
    assert_eq!(LOAN_DEFAULT_FLAG, 0x0001_0000);
    assert_eq!(LOAN_IMPAIR_FLAG, 0x0002_0000);
    assert_eq!(LOAN_UNIMPAIR_FLAG, 0x0004_0000);
    assert_eq!(LOAN_MANAGE_FLAGS_MASK, 0x3ff8_ffff);
    assert_eq!(get_loan_manage_flags_mask(), 0x3ff8_ffff);
}
