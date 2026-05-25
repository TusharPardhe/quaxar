//! Integration tests that pin the narrowed Rust `LoanSet.cpp::preclaim(...)`
//! owner-permission branch to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    LOAN_SET_NEITHER_ACCOUNT_NOR_COUNTERPARTY_OWNS_BROKER_WARNING,
    LoanSetPreclaimPermissionFailure, LoanSetPreclaimPermissionOutcome,
    LoanSetPreclaimPermissionTx, check_loan_set_preclaim_permission,
};

struct StubTx {
    account: &'static str,
    counterparty: Option<&'static str>,
}

impl LoanSetPreclaimPermissionTx for StubTx {
    type AccountId = &'static str;

    fn account(&self) -> Self::AccountId {
        self.account
    }

    fn counterparty(&self) -> Option<Self::AccountId> {
        self.counterparty
    }
}

#[test]
fn tx_loan_set_preclaim_permission_defaults_counterparty_to_owner() {
    let result = check_loan_set_preclaim_permission(
        &StubTx {
            account: "borrower",
            counterparty: None,
        },
        "broker-owner",
    );

    assert_eq!(
        result,
        Ok(LoanSetPreclaimPermissionOutcome {
            counterparty: "broker-owner",
            borrower: "borrower",
        })
    );
}

#[test]
fn tx_loan_set_preclaim_permission_derives_borrower_from_counterparty() {
    let result = check_loan_set_preclaim_permission(
        &StubTx {
            account: "broker-owner",
            counterparty: Some("borrower"),
        },
        "broker-owner",
    );

    assert_eq!(
        result,
        Ok(LoanSetPreclaimPermissionOutcome {
            counterparty: "borrower",
            borrower: "borrower",
        })
    );
}

#[test]
fn tx_loan_set_preclaim_permission_returns_no_permission() {
    let result = check_loan_set_preclaim_permission(
        &StubTx {
            account: "evan",
            counterparty: Some("borrower"),
        },
        "broker-owner",
    );

    assert_eq!(
        result,
        Err(LoanSetPreclaimPermissionFailure::NeitherAccountNorCounterpartyOwnsBroker)
    );
    let err = result.unwrap_err();
    assert_eq!(err.ter(), Ter::TEC_NO_PERMISSION);
    assert_eq!(trans_token(err.ter()), "tecNO_PERMISSION");
    assert_eq!(
        err.warning_message(),
        LOAN_SET_NEITHER_ACCOUNT_NOR_COUNTERPARTY_OWNS_BROKER_WARNING
    );
}
