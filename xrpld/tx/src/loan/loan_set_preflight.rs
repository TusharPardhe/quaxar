//! Front preflight helpers for the reference implementation.
//!
//! This module ports the exact deterministic behavior around:
//!
//! - the inner-batch-without-counterparty `temBAD_SIGNER` special case when the
//!   batch amendment is enabled,
//! - requiring `CounterpartySignature` only for non-inner transactions,
//! - allowing inner transactions without that signature when the earlier batch
//!   special case does not reject them, and
//! - returning the first lower signing-key preflight failure unchanged when a
//!   counterparty signature is present.

use protocol::{NotTec, Ter, is_tes_success};

pub trait LoanSetPreflightTx {
    type CounterpartySignature;

    fn is_inner_batch_txn(&self) -> bool;
    fn has_counterparty(&self) -> bool;
    fn counterparty_signature(&self) -> Option<&Self::CounterpartySignature>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoanSetPreflightSignatureGate<'a, Signature> {
    pub counterparty_signature: Option<&'a Signature>,
}

#[allow(clippy::too_many_arguments)]
pub fn run_loan_set_preflight<
    Tx,
    CheckSigningKey,
    ValidateDataLength,
    ValidateLoanServiceFee,
    ValidateLatePaymentFee,
    ValidateClosePaymentFee,
    ValidatePrincipalRequested,
    ValidateLoanOriginationFee,
    ValidateInterestRate,
    ValidateOverpaymentFee,
    ValidateLateInterestRate,
    ValidateCloseInterestRate,
    ValidateOverpaymentInterestRate,
    ValidatePaymentTotal,
    ValidatePaymentInterval,
    ValidateGracePeriod,
    CheckSimulateKeys,
    CheckBrokerId,
>(
    tx: &Tx,
    batch_enabled: bool,
    check_signing_key: CheckSigningKey,
    validate_data_length: ValidateDataLength,
    validate_loan_service_fee: ValidateLoanServiceFee,
    validate_late_payment_fee: ValidateLatePaymentFee,
    validate_close_payment_fee: ValidateClosePaymentFee,
    validate_principal_requested: ValidatePrincipalRequested,
    validate_loan_origination_fee: ValidateLoanOriginationFee,
    validate_interest_rate: ValidateInterestRate,
    validate_overpayment_fee: ValidateOverpaymentFee,
    validate_late_interest_rate: ValidateLateInterestRate,
    validate_close_interest_rate: ValidateCloseInterestRate,
    validate_overpayment_interest_rate: ValidateOverpaymentInterestRate,
    validate_payment_total: ValidatePaymentTotal,
    validate_payment_interval: ValidatePaymentInterval,
    validate_grace_period: ValidateGracePeriod,
    check_simulate_keys: CheckSimulateKeys,
    check_broker_id: CheckBrokerId,
) -> NotTec
where
    Tx: LoanSetPreflightTx,
    CheckSigningKey: FnOnce(&Tx::CounterpartySignature) -> NotTec,
    ValidateDataLength: FnOnce() -> bool,
    ValidateLoanServiceFee: FnOnce() -> bool,
    ValidateLatePaymentFee: FnOnce() -> bool,
    ValidateClosePaymentFee: FnOnce() -> bool,
    ValidatePrincipalRequested: FnOnce() -> bool,
    ValidateLoanOriginationFee: FnOnce() -> bool,
    ValidateInterestRate: FnOnce() -> bool,
    ValidateOverpaymentFee: FnOnce() -> bool,
    ValidateLateInterestRate: FnOnce() -> bool,
    ValidateCloseInterestRate: FnOnce() -> bool,
    ValidateOverpaymentInterestRate: FnOnce() -> bool,
    ValidatePaymentTotal: FnOnce() -> bool,
    ValidatePaymentInterval: FnOnce() -> bool,
    ValidateGracePeriod: FnOnce() -> bool,
    CheckSimulateKeys: FnOnce(&Tx::CounterpartySignature) -> NotTec,
    CheckBrokerId: FnOnce() -> bool,
{
    let signature_gate =
        match run_loan_set_preflight_signature_gate(tx, batch_enabled, check_signing_key) {
            Ok(signature_gate) => signature_gate,
            Err(ret) => return ret,
        };

    if !validate_data_length() {
        return Ter::TEM_INVALID;
    }
    if !validate_loan_service_fee() {
        return Ter::TEM_INVALID;
    }
    if !validate_late_payment_fee() {
        return Ter::TEM_INVALID;
    }
    if !validate_close_payment_fee() {
        return Ter::TEM_INVALID;
    }
    if !validate_principal_requested() {
        return Ter::TEM_INVALID;
    }
    if !validate_loan_origination_fee() {
        return Ter::TEM_INVALID;
    }
    if !validate_interest_rate() {
        return Ter::TEM_INVALID;
    }
    if !validate_overpayment_fee() {
        return Ter::TEM_INVALID;
    }
    if !validate_late_interest_rate() {
        return Ter::TEM_INVALID;
    }
    if !validate_close_interest_rate() {
        return Ter::TEM_INVALID;
    }
    if !validate_overpayment_interest_rate() {
        return Ter::TEM_INVALID;
    }
    if !validate_payment_total() {
        return Ter::TEM_INVALID;
    }
    if !validate_payment_interval() {
        return Ter::TEM_INVALID;
    }
    if !validate_grace_period() {
        return Ter::TEM_INVALID;
    }

    if let Some(counterparty_signature) = signature_gate.counterparty_signature {
        let ret = check_simulate_keys(counterparty_signature);
        if !is_tes_success(ret) {
            return ret;
        }
    }

    if !check_broker_id() {
        return Ter::TEM_INVALID;
    }

    Ter::TES_SUCCESS
}

pub fn run_loan_set_preflight_signature_gate<Tx, CheckSigningKey>(
    tx: &Tx,
    batch_enabled: bool,
    check_signing_key: CheckSigningKey,
) -> Result<LoanSetPreflightSignatureGate<'_, Tx::CounterpartySignature>, NotTec>
where
    Tx: LoanSetPreflightTx,
    CheckSigningKey: FnOnce(&Tx::CounterpartySignature) -> NotTec,
{
    if tx.is_inner_batch_txn() && batch_enabled && !tx.has_counterparty() {
        return Err(Ter::TEM_BAD_SIGNER);
    }

    let counterparty_signature = tx.counterparty_signature();
    if !tx.is_inner_batch_txn() && counterparty_signature.is_none() {
        return Err(Ter::TEM_BAD_SIGNER);
    }

    if let Some(counterparty_signature) = counterparty_signature {
        let ret = check_signing_key(counterparty_signature);
        if !is_tes_success(ret) {
            return Err(ret);
        }
    }

    Ok(LoanSetPreflightSignatureGate {
        counterparty_signature,
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{
        LoanSetPreflightSignatureGate, LoanSetPreflightTx, run_loan_set_preflight,
        run_loan_set_preflight_signature_gate,
    };

    struct TestTx {
        is_inner_batch_txn: bool,
        has_counterparty: bool,
        counterparty_signature: Option<&'static str>,
    }

    impl LoanSetPreflightTx for TestTx {
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
            &TestTx {
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
            &TestTx {
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
    fn loan_set_preflight_signature_gate_allows_inner_without_signature_when_batch_rule_does_not_fire()
     {
        let signing_key_called = Cell::new(false);

        let result = run_loan_set_preflight_signature_gate(
            &TestTx {
                is_inner_batch_txn: true,
                has_counterparty: true,
                counterparty_signature: None,
            },
            true,
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
    fn loan_set_preflight_signature_gate_allows_inner_without_signature_when_batch_disabled() {
        let signing_key_called = Cell::new(false);

        let result = run_loan_set_preflight_signature_gate(
            &TestTx {
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
    fn loan_set_preflight_signature_gate_returns_signing_key_failure_unchanged() {
        let result = run_loan_set_preflight_signature_gate(
            &TestTx {
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
    fn loan_set_preflight_signature_gate_returns_signature_on_success() {
        let result = run_loan_set_preflight_signature_gate(
            &TestTx {
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
            &TestTx {
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
            &TestTx {
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
            &TestTx {
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
    fn loan_set_preflight_returns_teminvalid_for_zero_broker_after_simulate_keys() {
        let broker_checked = Cell::new(false);

        let result = run_loan_set_preflight(
            &TestTx {
                is_inner_batch_txn: false,
                has_counterparty: true,
                counterparty_signature: Some("sig"),
            },
            true,
            |_| Ter::TES_SUCCESS,
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
            || true,
            |_| Ter::TES_SUCCESS,
            || {
                broker_checked.set(true);
                false
            },
        );

        assert_eq!(result, Ter::TEM_INVALID);
        assert!(broker_checked.get());
    }

    #[test]
    fn loan_set_preflight_uses_current_on_success() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_loan_set_preflight(
            &TestTx {
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
}
