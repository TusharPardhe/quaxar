//! Integration tests that pin the narrowed Rust `LoanSet.cpp` metadata helpers
//! to the current C++ behavior.

use std::cell::Cell;

use tx::loan::loan_set_metadata::{FULLY_CANONICAL_SIGNATURE_FLAG, INNER_BATCH_TRANSACTION_FLAG};
use tx::{
    LOAN_SET_FLAGS_MASK, LOAN_SET_OVERPAYMENT_FLAG, get_loan_set_flags_mask,
    run_loan_set_check_extra_features,
};

#[test]
fn loan_set_check_extra_features_short_circuits_when_single_asset_vault_is_disabled() {
    let vault_helper_called = Cell::new(false);

    let result = run_loan_set_check_extra_features(false, || {
        vault_helper_called.set(true);
        true
    });

    assert!(!result);
    assert!(!vault_helper_called.get());
}

#[test]
fn loan_set_check_extra_features_returns_vault_helper_result() {
    assert!(run_loan_set_check_extra_features(true, || true));
    assert!(!run_loan_set_check_extra_features(true, || false));
}

#[test]
fn loan_set_flags_mask_txflags() {
    assert_eq!(LOAN_SET_OVERPAYMENT_FLAG, 0x0001_0000);
    assert_eq!(FULLY_CANONICAL_SIGNATURE_FLAG, 0x8000_0000);
    assert_eq!(INNER_BATCH_TRANSACTION_FLAG, 0x4000_0000);
    assert_eq!(LOAN_SET_FLAGS_MASK, 0x3ffe_ffff);
    assert_eq!(get_loan_set_flags_mask(), 0x3ffe_ffff);
}
