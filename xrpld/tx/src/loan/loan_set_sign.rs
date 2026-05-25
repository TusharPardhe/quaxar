//! the reference implementation helper mirrored into Rust.
//!
//! This module ports the deterministic outer signing behavior around:
//!
//! - returning the primary `Transactor::checkSign(ctx)` failure unchanged,
//! - choosing the explicit `sfCounterparty` when present,
//! - otherwise falling back to the current `LoanBroker.Owner`,
//! - rejecting the missing-counter-signer case with `temBAD_SIGNER`,
//! - resolving the counter-signer before checking whether the optional
//!   `CounterpartySignature` is present, and
//! - invoking the lower transactor sign check only when that signature field
//!   is present.

use protocol::{NotTec, Ter, is_tes_success};

pub trait LoanSetSignTx {
    type AccountId: Clone;
    type CounterpartySignature;

    fn counterparty(&self) -> Option<Self::AccountId>;
    fn has_counterparty_signature(&self) -> bool;
    fn counterparty_signature(&self) -> &Self::CounterpartySignature;
}

pub fn run_loan_set_check_sign<Tx, ReadBrokerOwner, CheckSign, CheckCounterpartySign>(
    tx: &Tx,
    mut read_broker_owner: ReadBrokerOwner,
    check_sign: CheckSign,
    check_counterparty_sign: CheckCounterpartySign,
) -> NotTec
where
    Tx: LoanSetSignTx,
    ReadBrokerOwner: FnMut() -> Option<Tx::AccountId>,
    CheckSign: FnOnce() -> NotTec,
    CheckCounterpartySign: FnOnce(Tx::AccountId, &Tx::CounterpartySignature) -> NotTec,
{
    let ret = check_sign();
    if !is_tes_success(ret) {
        return ret;
    }

    let Some(counter_signer) = tx.counterparty().or_else(&mut read_broker_owner) else {
        return Ter::TEM_BAD_SIGNER;
    };

    if !tx.has_counterparty_signature() {
        return Ter::TES_SUCCESS;
    }

    check_counterparty_sign(counter_signer, tx.counterparty_signature())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::trans_token;

    use super::{LoanSetSignTx, run_loan_set_check_sign};

    struct TestTx {
        counterparty: Option<&'static str>,
        has_counterparty_signature: bool,
        counterparty_signature: &'static str,
    }

    impl LoanSetSignTx for TestTx {
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
    fn loan_set_sign_returns_primary_failure_without_more_lookup() {
        let broker_lookup_called = Cell::new(false);

        let result = run_loan_set_check_sign(
            &TestTx {
                counterparty: Some("borrower"),
                has_counterparty_signature: true,
                counterparty_signature: "sig",
            },
            || {
                broker_lookup_called.set(true);
                Some("broker-owner")
            },
            || protocol::Ter::TEF_BAD_AUTH,
            |_, _| protocol::Ter::TES_SUCCESS,
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_AUTH);
        assert_eq!(trans_token(result), "tefBAD_AUTH");
        assert!(!broker_lookup_called.get());
    }

    #[test]
    fn loan_set_sign_prefers_explicit_counterparty() {
        let broker_lookup_called = Cell::new(false);

        let result = run_loan_set_check_sign(
            &TestTx {
                counterparty: Some("borrower"),
                has_counterparty_signature: true,
                counterparty_signature: "sig",
            },
            || {
                broker_lookup_called.set(true);
                Some("broker-owner")
            },
            || protocol::Ter::TES_SUCCESS,
            |counter_signer, signature| {
                assert_eq!(counter_signer, "borrower");
                assert_eq!(*signature, "sig");
                protocol::Ter::TEF_BAD_AUTH
            },
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_AUTH);
        assert!(!broker_lookup_called.get());
    }

    #[test]
    fn loan_set_sign_falls_back_to_broker_owner() {
        let result = run_loan_set_check_sign(
            &TestTx {
                counterparty: None,
                has_counterparty_signature: true,
                counterparty_signature: "sig",
            },
            || Some("broker-owner"),
            || protocol::Ter::TES_SUCCESS,
            |counter_signer, signature| {
                assert_eq!(counter_signer, "broker-owner");
                assert_eq!(*signature, "sig");
                protocol::Ter::TEF_BAD_AUTH
            },
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_AUTH);
        assert_eq!(trans_token(result), "tefBAD_AUTH");
    }

    #[test]
    fn loan_set_sign_returns_bad_signer_when_counter_signer_is_missing() {
        let counterparty_sign_called = Cell::new(false);

        let result = run_loan_set_check_sign(
            &TestTx {
                counterparty: None,
                has_counterparty_signature: false,
                counterparty_signature: "sig",
            },
            || None::<&'static str>,
            || protocol::Ter::TES_SUCCESS,
            |_, _| {
                counterparty_sign_called.set(true);
                protocol::Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, protocol::Ter::TEM_BAD_SIGNER);
        assert_eq!(trans_token(result), "temBAD_SIGNER");
        assert!(!counterparty_sign_called.get());
    }

    #[test]
    fn loan_set_sign_succeeds_without_counterparty_signature_after_signer_lookup() {
        let counterparty_sign_called = Cell::new(false);

        let result = run_loan_set_check_sign(
            &TestTx {
                counterparty: Some("borrower"),
                has_counterparty_signature: false,
                counterparty_signature: "sig",
            },
            || Some("broker-owner"),
            || protocol::Ter::TES_SUCCESS,
            |_, _| {
                counterparty_sign_called.set(true);
                protocol::Ter::TEF_BAD_AUTH
            },
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(trans_token(result), "tesSUCCESS");
        assert!(!counterparty_sign_called.get());
    }

    #[test]
    fn loan_set_sign_returns_counterparty_sign_result_unchanged() {
        let result = run_loan_set_check_sign(
            &TestTx {
                counterparty: None,
                has_counterparty_signature: true,
                counterparty_signature: "counter-sig",
            },
            || Some("broker-owner"),
            || protocol::Ter::TES_SUCCESS,
            |counter_signer, signature| {
                assert_eq!(counter_signer, "broker-owner");
                assert_eq!(*signature, "counter-sig");
                protocol::Ter::TER_NO_ACCOUNT
            },
        );

        assert_eq!(result, protocol::Ter::TER_NO_ACCOUNT);
        assert_eq!(trans_token(result), "terNO_ACCOUNT");
    }
}
