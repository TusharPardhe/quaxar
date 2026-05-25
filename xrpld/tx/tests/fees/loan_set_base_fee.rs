//! Integration tests that pin the narrowed Rust
//! `LoanSet.cpp::calculateBaseFee(...)` wrapper to the current C++ behavior.

use tx::{
    LoanSetBaseFeeTx, LoanSetCounterpartySignature, count_loan_set_counterparty_signers,
    run_loan_set_calculate_base_fee,
};

#[derive(Clone, Copy)]
struct StubCounterpartySignature {
    has_signers: bool,
    signers_len: usize,
    has_txn_signature: bool,
}

impl LoanSetCounterpartySignature for StubCounterpartySignature {
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

struct StubTx {
    counterparty_signature: StubCounterpartySignature,
}

impl LoanSetBaseFeeTx for StubTx {
    type CounterpartySignature = StubCounterpartySignature;

    fn counterparty_signature(&self) -> &Self::CounterpartySignature {
        &self.counterparty_signature
    }
}

#[test]
fn tx_loan_set_counterparty_signer_count_is_zero_for_empty_signature() {
    let count = count_loan_set_counterparty_signers(&StubCounterpartySignature {
        has_signers: false,
        signers_len: 0,
        has_txn_signature: false,
    });

    assert_eq!(count, 0);
}

#[test]
fn tx_loan_set_counterparty_signer_count_is_one_for_single_signature() {
    let count = count_loan_set_counterparty_signers(&StubCounterpartySignature {
        has_signers: false,
        signers_len: 0,
        has_txn_signature: true,
    });

    assert_eq!(count, 1);
}

#[test]
fn tx_loan_set_counterparty_signer_count_uses_signer_array_length() {
    let count = count_loan_set_counterparty_signers(&StubCounterpartySignature {
        has_signers: true,
        signers_len: 4,
        has_txn_signature: false,
    });

    assert_eq!(count, 4);
}

#[test]
fn tx_loan_set_counterparty_signer_count_prefers_signer_array() {
    let count = count_loan_set_counterparty_signers(&StubCounterpartySignature {
        has_signers: true,
        signers_len: 3,
        has_txn_signature: true,
    });

    assert_eq!(count, 3);
}

#[test]
fn tx_loan_set_calculate_base_fee_adds_one_base_fee_per_counted_signer() {
    let fee = run_loan_set_calculate_base_fee(
        &StubTx {
            counterparty_signature: StubCounterpartySignature {
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
