//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! front ledger-read shell to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    LoanSetDoApplyLedgerStateBroker, LoanSetDoApplyLedgerStateFailure, LoanSetDoApplyLedgerStateTx,
    LoanSetDoApplyLedgerStateVault, load_loan_set_do_apply_ledger_state,
};

struct StubTx {
    broker_id: &'static str,
    account: &'static str,
    counterparty: Option<&'static str>,
}

impl LoanSetDoApplyLedgerStateTx for StubTx {
    type BrokerId = &'static str;
    type AccountId = &'static str;

    fn broker_id(&self) -> &Self::BrokerId {
        &self.broker_id
    }

    fn account(&self) -> &Self::AccountId {
        &self.account
    }

    fn counterparty(&self) -> Option<&Self::AccountId> {
        self.counterparty.as_ref()
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct StubBroker {
    owner: &'static str,
    vault_id: &'static str,
    account: &'static str,
}

impl LoanSetDoApplyLedgerStateBroker for StubBroker {
    type AccountId = &'static str;
    type VaultId = &'static str;

    fn owner(&self) -> &Self::AccountId {
        &self.owner
    }

    fn vault_id(&self) -> &Self::VaultId {
        &self.vault_id
    }

    fn account(&self) -> &Self::AccountId {
        &self.account
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct StubVault {
    account: &'static str,
    asset: &'static str,
}

impl LoanSetDoApplyLedgerStateVault for StubVault {
    type AccountId = &'static str;
    type Asset = &'static str;

    fn account(&self) -> &Self::AccountId {
        &self.account
    }

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }
}

fn stub_broker() -> StubBroker {
    StubBroker {
        owner: "broker-owner",
        vault_id: "vault-id",
        account: "broker-pseudo",
    }
}

fn stub_vault() -> StubVault {
    StubVault {
        account: "vault-pseudo",
        asset: "USD",
    }
}

#[test]
fn tx_loan_set_do_apply_ledger_state_loads_front_objects() {
    let result = load_loan_set_do_apply_ledger_state(
        &StubTx {
            broker_id: "broker-id",
            account: "txn-account",
            counterparty: Some("borrower"),
        },
        |_| Some(stub_broker()),
        |_| Some(stub_vault()),
        |account| Some(format!("state:{account}")),
    );

    let state = result.expect("front ledger state should load");
    assert_eq!(state.counterparty, "borrower");
    assert_eq!(state.borrower, "borrower");
    assert_eq!(state.vault_asset, "USD");
    assert_eq!(state.broker_owner_state, "state:broker-owner");
}

#[test]
fn tx_loan_set_do_apply_ledger_state_defaults_counterparty_to_broker_owner() {
    let result = load_loan_set_do_apply_ledger_state(
        &StubTx {
            broker_id: "broker-id",
            account: "txn-account",
            counterparty: None,
        },
        |_| Some(stub_broker()),
        |_| Some(stub_vault()),
        |account| Some(format!("state:{account}")),
    );

    let state = result.expect("front ledger state should load");
    assert_eq!(state.counterparty, "broker-owner");
    assert_eq!(state.borrower, "txn-account");
}

#[test]
fn tx_loan_set_do_apply_ledger_state_returns_bad_ledger_when_broker_missing() {
    let result = load_loan_set_do_apply_ledger_state(
        &StubTx {
            broker_id: "missing-broker",
            account: "txn-account",
            counterparty: Some("borrower"),
        },
        |_| None::<StubBroker>,
        |_| Some(stub_vault()),
        |_| Some("unused"),
    );

    assert_eq!(
        result,
        Err(LoanSetDoApplyLedgerStateFailure::BrokerDoesNotExist)
    );
    let err = result.expect_err("missing broker should fail");
    assert_eq!(err.ter(), Ter::TEF_BAD_LEDGER);
    assert_eq!(trans_token(err.ter()), "tefBAD_LEDGER");
}

#[test]
fn tx_loan_set_do_apply_ledger_state_returns_bad_ledger_when_vault_missing() {
    let result = load_loan_set_do_apply_ledger_state(
        &StubTx {
            broker_id: "broker-id",
            account: "txn-account",
            counterparty: Some("borrower"),
        },
        |_| Some(stub_broker()),
        |_| None::<StubVault>,
        |_| Some("state"),
    );

    assert_eq!(
        result,
        Err(LoanSetDoApplyLedgerStateFailure::VaultDoesNotExist)
    );
    let err = result.expect_err("missing vault should fail");
    assert_eq!(err.ter(), Ter::TEF_BAD_LEDGER);
}

#[test]
fn tx_loan_set_do_apply_ledger_state_returns_bad_ledger_when_borrower_missing() {
    let result = load_loan_set_do_apply_ledger_state(
        &StubTx {
            broker_id: "broker-id",
            account: "txn-account",
            counterparty: Some("borrower"),
        },
        |_| Some(stub_broker()),
        |_| Some(stub_vault()),
        |account| match *account {
            "broker-owner" => Some("owner-state"),
            "borrower" => None,
            "broker-pseudo" => Some("broker-pseudo-state"),
            _ => None,
        },
    );

    assert_eq!(
        result,
        Err(LoanSetDoApplyLedgerStateFailure::BorrowerDoesNotExist)
    );
    let err = result.expect_err("missing borrower should fail");
    assert_eq!(err.ter(), Ter::TEF_BAD_LEDGER);
}

#[test]
fn tx_loan_set_do_apply_ledger_state_returns_bad_ledger_when_broker_pseudo_missing() {
    let result = load_loan_set_do_apply_ledger_state(
        &StubTx {
            broker_id: "broker-id",
            account: "txn-account",
            counterparty: Some("borrower"),
        },
        |_| Some(stub_broker()),
        |_| Some(stub_vault()),
        |account| match *account {
            "broker-owner" => Some("owner-state"),
            "borrower" => Some("borrower-state"),
            "broker-pseudo" => None,
            _ => None,
        },
    );

    assert_eq!(
        result,
        Err(LoanSetDoApplyLedgerStateFailure::BrokerPseudoDoesNotExist)
    );
    let err = result.expect_err("missing broker pseudo should fail");
    assert_eq!(err.ter(), Ter::TEF_BAD_LEDGER);
}
