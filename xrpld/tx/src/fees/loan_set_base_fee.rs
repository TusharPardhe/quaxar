//! the reference implementation helper mirrored into Rust.
//!
//! This module ports the deterministic base-fee behavior around:
//!
//! - starting from the lower `Transactor::calculateBaseFee(...)` result,
//! - treating the optional `CounterpartySignature` object as present even when
//!   empty,
//! - counting multisign entries from `sfSigners` when present,
//! - otherwise counting a single signer when `sfTxnSignature` is present, and
//! - adding one base fee per counted signer.

use std::ops::{Add, Mul};

pub trait LoanSetCounterpartySignature {
    fn has_signers(&self) -> bool;
    fn signers_len(&self) -> usize;
    fn has_txn_signature(&self) -> bool;
}

pub trait LoanSetBaseFeeTx {
    type CounterpartySignature: LoanSetCounterpartySignature;

    fn counterparty_signature(&self) -> &Self::CounterpartySignature;
}

pub fn count_loan_set_counterparty_signers<Signature>(counter_sig: &Signature) -> u64
where
    Signature: LoanSetCounterpartySignature,
{
    if counter_sig.has_signers() {
        return u64::try_from(counter_sig.signers_len())
            .expect("counterparty signer count should fit into u64");
    }

    if counter_sig.has_txn_signature() {
        1
    } else {
        0
    }
}

pub fn run_loan_set_calculate_base_fee<Tx, Fee>(tx: &Tx, normal_cost: Fee, base_fee: Fee) -> Fee
where
    Tx: LoanSetBaseFeeTx,
    Fee: Copy + Add<Output = Fee> + Mul<u64, Output = Fee>,
{
    let signer_count = count_loan_set_counterparty_signers(tx.counterparty_signature());
    normal_cost + (base_fee * signer_count)
}

#[cfg(test)]
mod tests {
    use super::{
        LoanSetBaseFeeTx, LoanSetCounterpartySignature, count_loan_set_counterparty_signers,
        run_loan_set_calculate_base_fee,
    };

    #[derive(Clone, Copy)]
    struct TestCounterpartySignature {
        has_signers: bool,
        signers_len: usize,
        has_txn_signature: bool,
    }

    impl LoanSetCounterpartySignature for TestCounterpartySignature {
        fn has_signers(&self) -> bool {
            self.has_signers
        }

        fn signers_len(&self) -> usize {
            self.signers_len
        }

        fn has_txn_signature(&self) -> bool {
            self.has_txn_signature
        }
    }

    struct TestTx {
        counterparty_signature: TestCounterpartySignature,
    }

    impl LoanSetBaseFeeTx for TestTx {
        type CounterpartySignature = TestCounterpartySignature;

        fn counterparty_signature(&self) -> &Self::CounterpartySignature {
            &self.counterparty_signature
        }
    }

    #[test]
    fn loan_set_counterparty_signer_count_is_zero_for_empty_signature_object() {
        let count = count_loan_set_counterparty_signers(&TestCounterpartySignature {
            has_signers: false,
            signers_len: 0,
            has_txn_signature: false,
        });

        assert_eq!(count, 0);
    }

    #[test]
    fn loan_set_counterparty_signer_count_is_one_for_single_signature() {
        let count = count_loan_set_counterparty_signers(&TestCounterpartySignature {
            has_signers: false,
            signers_len: 0,
            has_txn_signature: true,
        });

        assert_eq!(count, 1);
    }

    #[test]
    fn loan_set_counterparty_signer_count_uses_signer_array_length() {
        let count = count_loan_set_counterparty_signers(&TestCounterpartySignature {
            has_signers: true,
            signers_len: 4,
            has_txn_signature: false,
        });

        assert_eq!(count, 4);
    }

    #[test]
    fn loan_set_counterparty_signer_count_prefers_signer_array_over_txn_signature() {
        let count = count_loan_set_counterparty_signers(&TestCounterpartySignature {
            has_signers: true,
            signers_len: 3,
            has_txn_signature: true,
        });

        assert_eq!(count, 3);
    }

    #[test]
    fn loan_set_calculate_base_fee_leaves_normal_cost_unchanged_for_empty_signature() {
        let fee = run_loan_set_calculate_base_fee(
            &TestTx {
                counterparty_signature: TestCounterpartySignature {
                    has_signers: false,
                    signers_len: 0,
                    has_txn_signature: false,
                },
            },
            10_u64,
            2_u64,
        );

        assert_eq!(fee, 10);
    }

    #[test]
    fn loan_set_calculate_base_fee_adds_one_base_fee_for_single_signature() {
        let fee = run_loan_set_calculate_base_fee(
            &TestTx {
                counterparty_signature: TestCounterpartySignature {
                    has_signers: false,
                    signers_len: 0,
                    has_txn_signature: true,
                },
            },
            10_u64,
            2_u64,
        );

        assert_eq!(fee, 12);
    }

    #[test]
    fn loan_set_calculate_base_fee_adds_one_base_fee_per_multisigner() {
        let fee = run_loan_set_calculate_base_fee(
            &TestTx {
                counterparty_signature: TestCounterpartySignature {
                    has_signers: true,
                    signers_len: 4,
                    has_txn_signature: false,
                },
            },
            10_u64,
            2_u64,
        );

        assert_eq!(fee, 18);
    }
}
