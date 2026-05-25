//! Post-transfer loan-allocation helper for the LoanSet transactor.
//!
//! This module preserves the exact deterministic behavior around:
//!
//! - deriving the loan ledger key from `keylet::loan(brokerID, loanSequence)`,
//! - calling the loan allocator exactly once with that key, and
//! - returning the allocated loan object unchanged.

use basics::base_uint::Uint256;
use protocol::loan_key;

pub fn run_loan_set_do_apply_loan_allocation<Loan, AllocateLoan>(
    loan_broker_id: Uint256,
    loan_sequence: u32,
    allocate_loan: AllocateLoan,
) -> Loan
where
    AllocateLoan: FnOnce(Uint256) -> Loan,
{
    allocate_loan(loan_key(loan_broker_id, loan_sequence))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use basics::base_uint::Uint256;

    use super::run_loan_set_do_apply_loan_allocation;

    #[test]
    fn loan_set_do_apply_loan_allocation_passes_current_cpp_loan_key_to_allocator() {
        let loan_broker_id =
            Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
                .expect("expected loan broker id should parse");

        let allocated = run_loan_set_do_apply_loan_allocation(loan_broker_id, 7, |loan_id| {
            assert_eq!(
                loan_id,
                Uint256::from_hex(
                    "B9CF90CA6D45957E6BB9A59666C328113077AA775B5B6516C8AFDDC507647E90"
                )
                .expect("expected loan id should parse")
            );
            "loan"
        });

        assert_eq!(allocated, "loan");
    }

    #[test]
    fn loan_set_do_apply_loan_allocation_calls_allocator_once() {
        let calls = Cell::new(0_u32);
        let loan_broker_id =
            Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
                .expect("expected loan broker id should parse");

        let allocated = run_loan_set_do_apply_loan_allocation(loan_broker_id, 7, |_| {
            calls.set(calls.get() + 1);
            9_u32
        });

        assert_eq!(allocated, 9_u32);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn loan_set_do_apply_loan_allocation_keeps_zero_sequence_path() {
        let loan_broker_id =
            Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
                .expect("expected loan broker id should parse");

        let allocated = run_loan_set_do_apply_loan_allocation(loan_broker_id, 0, |loan_id| {
            assert_eq!(
                loan_id,
                Uint256::from_hex(
                    "4F928E3A90E31D809116F6FB8366154DC50B607DB1199FB6167534B97C0C4C5A"
                )
                .expect("expected zero-sequence loan id should parse")
            );
            "zero-seq-loan"
        });

        assert_eq!(allocated, "zero-seq-loan");
    }
}
