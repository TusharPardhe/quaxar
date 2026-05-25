//! Integration tests that pin the narrowed Rust
//! `LoanSet.cpp::doApply()` loan-allocation wrapper to the current C++
//! behavior.

use basics::base_uint::Uint256;
use tx::run_loan_set_do_apply_loan_allocation;

#[test]
fn tx_loan_set_do_apply_loan_allocation_passes_current_cpp_loan_key_to_allocator() {
    let loan_broker_id =
        Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
            .expect("expected loan broker id should parse");

    let allocated = run_loan_set_do_apply_loan_allocation(loan_broker_id, 7, |loan_id| {
        assert_eq!(
            loan_id,
            Uint256::from_hex("B9CF90CA6D45957E6BB9A59666C328113077AA775B5B6516C8AFDDC507647E90")
                .expect("expected loan id should parse")
        );
        "loan"
    });

    assert_eq!(allocated, "loan");
}

#[test]
fn tx_loan_set_do_apply_loan_allocation_returns_allocator_result_unchanged() {
    let loan_broker_id =
        Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
            .expect("expected loan broker id should parse");

    let allocated = run_loan_set_do_apply_loan_allocation(loan_broker_id, 7, |_| 42_u32);

    assert_eq!(allocated, 42_u32);
}

#[test]
fn tx_loan_set_do_apply_loan_allocation_keeps_zero_sequence_path() {
    let loan_broker_id =
        Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
            .expect("expected loan broker id should parse");

    let allocated = run_loan_set_do_apply_loan_allocation(loan_broker_id, 0, |loan_id| {
        assert_eq!(
            loan_id,
            Uint256::from_hex("4F928E3A90E31D809116F6FB8366154DC50B607DB1199FB6167534B97C0C4C5A")
                .expect("expected zero-sequence loan id should parse")
        );
        "zero-seq-loan"
    });

    assert_eq!(allocated, "zero-seq-loan");
}
