//! Integration tests that pin the narrowed Rust `LoanSet.cpp::preflight(...)`
//! wrapper to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    LoanSetPreflightSignatureGate, LoanSetPreflightTx, run_loan_set_preflight,
    run_loan_set_preflight_signature_gate,
};

struct StubTx {
    is_inner_batch_txn: bool,
    has_counterparty: bool,
    counterparty_signature: Option<&'static str>,
}

impl LoanSetPreflightTx for StubTx {
    type CounterpartySignature = &'static str;

    fn is_inner_batch_txn(&self) -> bool {
        self.is_inner_batch_txn
    }

    fn has_counterparty(&self) -> bool {
        self.has_counterparty
    }

    fn counterparty_signature(&self) -> Option<&Self::CounterpartySignature> {
        self.counterparty_signature.as_ref()
    }
}

#[test]
fn loan_set_preflight_signature_gate_rejects_inner_batch_without_counterparty() {
    let signing_key_called = Cell::new(false);

    let result = run_loan_set_preflight_signature_gate(
        &StubTx {
            is_inner_batch_txn: true,
            has_counterparty: false,
            counterparty_signature: Some("sig"),
        },
        true,
        |_| {
            signing_key_called.set(true);
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Err(Ter::TEM_BAD_SIGNER));
    assert_eq!(trans_token(result.unwrap_err()), "temBAD_SIGNER");
    assert!(!signing_key_called.get());
}

#[test]
fn loan_set_preflight_signature_gate_requires_signature_for_outer_tx() {
    let signing_key_called = Cell::new(false);

    let result = run_loan_set_preflight_signature_gate(
        &StubTx {
            is_inner_batch_txn: false,
            has_counterparty: true,
            counterparty_signature: None,
        },
        true,
        |_| {
            signing_key_called.set(true);
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Err(Ter::TEM_BAD_SIGNER));
    assert_eq!(trans_token(result.unwrap_err()), "temBAD_SIGNER");
    assert!(!signing_key_called.get());
}

#[test]
fn loan_set_preflight_signature_gate_returns_signing_key_failure_unchanged() {
    let result = run_loan_set_preflight_signature_gate(
        &StubTx {
            is_inner_batch_txn: false,
            has_counterparty: true,
            counterparty_signature: Some("sig"),
        },
        true,
        |signature| {
            assert_eq!(*signature, "sig");
            Ter::TEM_MALFORMED
        },
    );

    assert_eq!(result, Err(Ter::TEM_MALFORMED));
    assert_eq!(trans_token(result.unwrap_err()), "temMALFORMED");
}

#[test]
fn loan_set_preflight_signature_gate_allows_inner_without_signature_when_batch_disabled() {
    let signing_key_called = Cell::new(false);

    let result = run_loan_set_preflight_signature_gate(
        &StubTx {
            is_inner_batch_txn: true,
            has_counterparty: false,
            counterparty_signature: None,
        },
        false,
        |_| {
            signing_key_called.set(true);
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(
        result,
        Ok(LoanSetPreflightSignatureGate {
            counterparty_signature: None,
        })
    );
    assert!(!signing_key_called.get());
}

#[test]
fn loan_set_preflight_signature_gate_returns_signature_on_success() {
    let result = run_loan_set_preflight_signature_gate(
        &StubTx {
            is_inner_batch_txn: false,
            has_counterparty: true,
            counterparty_signature: Some("sig"),
        },
        true,
        |signature| {
            assert_eq!(*signature, "sig");
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(
        result,
        Ok(LoanSetPreflightSignatureGate {
            counterparty_signature: Some(&"sig"),
        })
    );
}

#[test]
fn loan_set_preflight_returns_signature_gate_failure_before_numeric_tail() {
    let data_length_called = Cell::new(false);

    let result = run_loan_set_preflight(
        &StubTx {
            is_inner_batch_txn: false,
            has_counterparty: true,
            counterparty_signature: None,
        },
        true,
        |_| Ter::TES_SUCCESS,
        || {
            data_length_called.set(true);
            true
        },
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        |_| Ter::TES_SUCCESS,
        || true,
    );

    assert_eq!(result, Ter::TEM_BAD_SIGNER);
    assert!(!data_length_called.get());
}

#[test]
fn loan_set_preflight_returns_teminvalid_on_first_invalid_numeric_check() {
    let late_payment_called = Cell::new(false);

    let result = run_loan_set_preflight(
        &StubTx {
            is_inner_batch_txn: false,
            has_counterparty: true,
            counterparty_signature: Some("sig"),
        },
        true,
        |_| Ter::TES_SUCCESS,
        || true,
        || false,
        || {
            late_payment_called.set(true);
            true
        },
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        || true,
        |_| Ter::TES_SUCCESS,
        || true,
    );

    assert_eq!(result, Ter::TEM_INVALID);
    assert!(!late_payment_called.get());
}

#[test]
fn loan_set_preflight_returns_simulate_key_failure_after_numeric_tail() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_loan_set_preflight(
        &StubTx {
            is_inner_batch_txn: false,
            has_counterparty: true,
            counterparty_signature: Some("sig"),
        },
        true,
        |_| Ter::TES_SUCCESS,
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("data");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("service");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("late");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("close");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("principal");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("origination");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("interest");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("overpayment");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("late-interest");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("close-interest");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("overpayment-interest");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("payment-total");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("payment-interval");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("grace");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move |signature| {
                assert_eq!(*signature, "sig");
                seen.borrow_mut().push("simulate");
                Ter::TEM_MALFORMED
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("broker");
                true
            }
        },
    );

    assert_eq!(result, Ter::TEM_MALFORMED);
    assert_eq!(
        *seen.borrow(),
        vec![
            "data",
            "service",
            "late",
            "close",
            "principal",
            "origination",
            "interest",
            "overpayment",
            "late-interest",
            "close-interest",
            "overpayment-interest",
            "payment-total",
            "payment-interval",
            "grace",
            "simulate",
        ]
    );
}

#[test]
fn loan_set_preflight_uses_current_on_success() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_loan_set_preflight(
        &StubTx {
            is_inner_batch_txn: false,
            has_counterparty: true,
            counterparty_signature: Some("sig"),
        },
        true,
        |_| Ter::TES_SUCCESS,
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("data");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("service");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("late");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("close");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("principal");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("origination");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("interest");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("overpayment");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("late-interest");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("close-interest");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("overpayment-interest");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("payment-total");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("payment-interval");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("grace");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move |signature| {
                assert_eq!(*signature, "sig");
                seen.borrow_mut().push("simulate");
                Ter::TES_SUCCESS
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("broker");
                true
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        *seen.borrow(),
        vec![
            "data",
            "service",
            "late",
            "close",
            "principal",
            "origination",
            "interest",
            "overpayment",
            "late-interest",
            "close-interest",
            "overpayment-interest",
            "payment-total",
            "payment-interval",
            "grace",
            "simulate",
            "broker",
        ]
    );
}
