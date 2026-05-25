//! Owner-permission branch for the reference implementation.
//!
//! This module ports the deterministic behavior around:
//!
//! - defaulting the optional counterparty to `LoanBroker.Owner`,
//! - rejecting the transaction when neither `Account` nor `Counterparty` is
//!   the broker owner,
//! - mapping that failure to `tecNO_PERMISSION` with the current warning text,
//!   and
//! - deriving the borrower from the resolved counterparty after the permission
//!   gate passes.

use protocol::Ter;

pub const LOAN_SET_NEITHER_ACCOUNT_NOR_COUNTERPARTY_OWNS_BROKER_WARNING: &str =
    "Neither Account nor Counterparty are the owner of the LoanBroker.";

pub trait LoanSetPreclaimPermissionTx {
    type AccountId: Clone + PartialEq + Eq;

    fn account(&self) -> Self::AccountId;
    fn counterparty(&self) -> Option<Self::AccountId>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetPreclaimPermissionOutcome<AccountId> {
    pub counterparty: AccountId,
    pub borrower: AccountId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSetPreclaimPermissionFailure {
    NeitherAccountNorCounterpartyOwnsBroker,
}

impl LoanSetPreclaimPermissionFailure {
    pub const fn ter(self) -> Ter {
        match self {
            Self::NeitherAccountNorCounterpartyOwnsBroker => Ter::TEC_NO_PERMISSION,
        }
    }

    pub const fn warning_message(self) -> &'static str {
        match self {
            Self::NeitherAccountNorCounterpartyOwnsBroker => {
                LOAN_SET_NEITHER_ACCOUNT_NOR_COUNTERPARTY_OWNS_BROKER_WARNING
            }
        }
    }
}

pub fn check_loan_set_preclaim_permission<Tx>(
    tx: &Tx,
    broker_owner: Tx::AccountId,
) -> Result<LoanSetPreclaimPermissionOutcome<Tx::AccountId>, LoanSetPreclaimPermissionFailure>
where
    Tx: LoanSetPreclaimPermissionTx,
{
    let account = tx.account();
    let counterparty = tx.counterparty().unwrap_or_else(|| broker_owner.clone());

    if account != broker_owner && counterparty != broker_owner {
        return Err(LoanSetPreclaimPermissionFailure::NeitherAccountNorCounterpartyOwnsBroker);
    }

    let borrower = if counterparty == broker_owner {
        account
    } else {
        counterparty.clone()
    };

    Ok(LoanSetPreclaimPermissionOutcome {
        counterparty,
        borrower,
    })
}

#[cfg(test)]
mod tests {
    use protocol::trans_token;

    use super::{
        LOAN_SET_NEITHER_ACCOUNT_NOR_COUNTERPARTY_OWNS_BROKER_WARNING,
        LoanSetPreclaimPermissionFailure, LoanSetPreclaimPermissionOutcome,
        LoanSetPreclaimPermissionTx, check_loan_set_preclaim_permission,
    };

    struct TestTx {
        account: &'static str,
        counterparty: Option<&'static str>,
    }

    impl LoanSetPreclaimPermissionTx for TestTx {
        type AccountId = &'static str;

        fn account(&self) -> Self::AccountId {
            self.account
        }

        fn counterparty(&self) -> Option<Self::AccountId> {
            self.counterparty
        }
    }

    #[test]
    fn loan_set_preclaim_permission_defaults_counterparty_to_broker_owner() {
        let result = check_loan_set_preclaim_permission(
            &TestTx {
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
    fn loan_set_preclaim_permission_returns_counterparty_as_borrower_when_not_owner() {
        let result = check_loan_set_preclaim_permission(
            &TestTx {
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
    fn loan_set_preclaim_permission_rejects_when_neither_side_is_owner() {
        let result = check_loan_set_preclaim_permission(
            &TestTx {
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
        assert_eq!(err.ter(), protocol::Ter::TEC_NO_PERMISSION);
        assert_eq!(trans_token(err.ter()), "tecNO_PERMISSION");
        assert_eq!(
            err.warning_message(),
            LOAN_SET_NEITHER_ACCOUNT_NOR_COUNTERPARTY_OWNS_BROKER_WARNING
        );
    }
}
