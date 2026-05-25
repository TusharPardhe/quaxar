//! Integration tests that pin the narrowed Rust `LoanSet.cpp::checkSign(...)`
//! wrapper to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{LoanSetSignTx, run_loan_set_check_sign};

struct StubTx {
    counterparty: Option<&'static str>,
    has_counterparty_signature: bool,
    counterparty_signature: &'static str,
}

impl LoanSetSignTx for StubTx {
    type AccountId = &'static str;
    type CounterpartySignature = &'static str;

    fn counterparty(&self) -> Option<Self::AccountId> {
        self.counterparty
    }

    fn has_counterparty_signature(&self) -> bool {
        self.has_counterparty_signature
    }

    fn counterparty_signature(&self) -> &Self::CounterpartySignature {
        &self.counterparty_signature
    }
}

#[test]
fn tx_loan_set_sign_returns_primary_failure_unchanged() {
    let result = run_loan_set_check_sign(
        &StubTx {
            counterparty: Some("borrower"),
            has_counterparty_signature: true,
            counterparty_signature: "sig",
        },
        || panic!("broker lookup should not run after primary sign failure"),
        || Ter::TEF_BAD_AUTH,
        |_, _| panic!("counterparty sign should not run after primary sign failure"),
    );

    assert_eq!(result, Ter::TEF_BAD_AUTH);
    assert_eq!(trans_token(result), "tefBAD_AUTH");
}

#[test]
fn tx_loan_set_sign_returns_bad_signer_when_counter_signer_is_missing() {
    let result = run_loan_set_check_sign(
        &StubTx {
            counterparty: None,
            has_counterparty_signature: false,
            counterparty_signature: "sig",
        },
        || None::<&'static str>,
        || Ter::TES_SUCCESS,
        |_, _| panic!("counterparty sign should not run without a counter-signer"),
    );

    assert_eq!(result, Ter::TEM_BAD_SIGNER);
    assert_eq!(trans_token(result), "temBAD_SIGNER");
}

#[test]
fn tx_loan_set_sign_succeeds_without_counterparty_signature_when_signer_exists() {
    let result = run_loan_set_check_sign(
        &StubTx {
            counterparty: Some("borrower"),
            has_counterparty_signature: false,
            counterparty_signature: "sig",
        },
        || panic!("explicit counterparty should avoid broker lookup"),
        || Ter::TES_SUCCESS,
        |_, _| panic!("counterparty sign should not run when the signature is absent"),
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn tx_loan_set_sign_prefers_explicit_counterparty_over_broker_owner() {
    let result = run_loan_set_check_sign(
        &StubTx {
            counterparty: Some("borrower"),
            has_counterparty_signature: true,
            counterparty_signature: "sig",
        },
        || panic!("explicit counterparty should avoid broker lookup"),
        || Ter::TES_SUCCESS,
        |counter_signer, signature| {
            assert_eq!(counter_signer, "borrower");
            assert_eq!(*signature, "sig");
            Ter::TEF_BAD_AUTH
        },
    );

    assert_eq!(result, Ter::TEF_BAD_AUTH);
    assert_eq!(trans_token(result), "tefBAD_AUTH");
}

#[test]
fn tx_loan_set_sign_falls_back_to_broker_owner() {
    let result = run_loan_set_check_sign(
        &StubTx {
            counterparty: None,
            has_counterparty_signature: true,
            counterparty_signature: "sig",
        },
        || Some("broker-owner"),
        || Ter::TES_SUCCESS,
        |counter_signer, signature| {
            assert_eq!(counter_signer, "broker-owner");
            assert_eq!(*signature, "sig");
            Ter::TER_NO_ACCOUNT
        },
    );

    assert_eq!(result, Ter::TER_NO_ACCOUNT);
    assert_eq!(trans_token(result), "terNO_ACCOUNT");
}
