//! Integration tests that pin the narrowed Rust
//! `SetRegularKey.cpp::calculateBaseFee(...)` wrapper to the current C++
//! behavior.

use tx::{SetRegularKeyBaseFeeAccountState, run_set_regular_key_calculate_base_fee};

#[derive(Clone, Copy)]
struct StubAccountState {
    password_spent: bool,
}

impl SetRegularKeyBaseFeeAccountState for StubAccountState {
    fn password_spent(&self) -> bool {
        self.password_spent
    }
}

#[test]
fn tx_set_regular_key_calculate_base_fee_is_zero_for_matching_master_key_before_password_spend() {
    let fee = run_set_regular_key_calculate_base_fee(
        10_u64,
        0_u64,
        true,
        Some(StubAccountState {
            password_spent: false,
        }),
    );

    assert_eq!(fee, 0);
}

#[test]
fn tx_set_regular_key_calculate_base_fee_is_normal_when_signing_key_does_not_match() {
    let fee = run_set_regular_key_calculate_base_fee(
        10_u64,
        0_u64,
        false,
        Some(StubAccountState {
            password_spent: false,
        }),
    );

    assert_eq!(fee, 10);
}

#[test]
fn tx_set_regular_key_calculate_base_fee_is_normal_when_account_lookup_misses() {
    let fee = run_set_regular_key_calculate_base_fee(10_u64, 0_u64, true, None::<StubAccountState>);

    assert_eq!(fee, 10);
}

#[test]
fn tx_set_regular_key_calculate_base_fee_is_normal_after_password_spend() {
    let fee = run_set_regular_key_calculate_base_fee(
        10_u64,
        0_u64,
        true,
        Some(StubAccountState {
            password_spent: true,
        }),
    );

    assert_eq!(fee, 10);
}
