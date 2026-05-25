//! Integration tests that pin the small default `Transactor.cpp` helpers to
//! the current C++ behavior.

use protocol::{INNER_BATCH_TRANSACTION_FLAG, Ter};
use tx::{
    TRANSACTOR_FLAGS_MASK, TRANSACTOR_FULLY_CANONICAL_SIGNATURE_FLAG,
    run_transactor_calculate_base_fee, run_transactor_get_flags_mask,
    run_transactor_preflight_sig_validated, run_transactor_valid_data_length,
};

#[test]
fn transactor_valid_data_length_rejects_empty_and_oversized_slices() {
    assert!(run_transactor_valid_data_length(None, 256));
    assert!(!run_transactor_valid_data_length(Some(0), 256));
    assert!(run_transactor_valid_data_length(Some(256), 256));
    assert!(!run_transactor_valid_data_length(Some(257), 256));
}

#[test]
fn transactor_get_flags_mask_matches_current_universal_mask() {
    assert_eq!(TRANSACTOR_FULLY_CANONICAL_SIGNATURE_FLAG, 0x8000_0000);
    assert_eq!(INNER_BATCH_TRANSACTION_FLAG, 0x4000_0000);
    assert_eq!(TRANSACTOR_FLAGS_MASK, 0x3fff_ffff);
    assert_eq!(run_transactor_get_flags_mask(), 0x3fff_ffff);
}

#[test]
fn transactor_preflight_sig_validated_returns_success() {
    assert_eq!(run_transactor_preflight_sig_validated(), Ter::TES_SUCCESS);
}

#[test]
fn transactor_calculate_base_fee_counts_one_extra_base_fee_per_signer() {
    assert_eq!(run_transactor_calculate_base_fee(10_u64, 0), 10);
    assert_eq!(run_transactor_calculate_base_fee(10_u64, 2), 30);
}
