//! Integration tests that pin the narrowed owner-reserve fee helpers to the
//! current C++ behavior.

use tx::{
    owner_reserve_fee_is_reasonable, run_account_delete_calculate_base_fee,
    run_amm_create_calculate_base_fee, run_ledger_state_fix_calculate_base_fee,
    run_owner_reserve_base_fee,
};

#[test]
fn tx_owner_reserve_fee_reasonability_uses_strict_threshold() {
    assert!(!owner_reserve_fee_is_reasonable(10_u64, 1_000_u64));
    assert!(owner_reserve_fee_is_reasonable(10_u64, 1_001_u64));
}

#[test]
fn tx_account_delete_calculate_base_fee_returns_owner_reserve() {
    let fee = run_account_delete_calculate_base_fee(10_u64, 2_000_u64);

    assert_eq!(fee, 2_000_u64);
}

#[test]
fn tx_ledger_state_fix_calculate_base_fee_returns_owner_reserve() {
    let fee = run_ledger_state_fix_calculate_base_fee(10_u64, 2_000_u64);

    assert_eq!(fee, 2_000_u64);
}

#[test]
fn tx_amm_create_calculate_base_fee_returns_owner_reserve() {
    let fee = run_amm_create_calculate_base_fee(10_u64, 2_000_u64);

    assert_eq!(fee, 2_000_u64);
}

#[test]
fn tx_owner_reserve_base_fee_returns_increment() {
    let fee = run_owner_reserve_base_fee(10_u64, 2_000_u64);

    assert_eq!(fee, 2_000_u64);
}
