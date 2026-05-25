//! Integration tests that pin the narrowed Rust
//! `EscrowFinish.cpp::calculateBaseFee(...)` wrapper to the current C++
//! behavior.

use tx::run_escrow_finish_calculate_base_fee;

#[test]
fn tx_escrow_finish_calculate_base_fee_keeps_transactor_fee_without_fulfillment() {
    let fee = run_escrow_finish_calculate_base_fee(10_u64, 10_u64, None);

    assert_eq!(fee, 10);
}

#[test]
fn tx_escrow_finish_calculate_base_fee_adds_base_times_thirty_two_for_empty_fulfillment() {
    let fee = run_escrow_finish_calculate_base_fee(10_u64, 10_u64, Some(0));

    assert_eq!(fee, 330);
}

#[test]
fn tx_escrow_finish_calculate_base_fee_uses_integer_chunks_of_sixteen_bytes() {
    let fee = run_escrow_finish_calculate_base_fee(10_u64, 10_u64, Some(31));

    assert_eq!(fee, 340);
}
