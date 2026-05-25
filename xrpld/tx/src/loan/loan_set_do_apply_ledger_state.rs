//! Front ledger-read shell for the LoanSet transactor.
//!
//! This module ports the deterministic behavior around:
//!
//! - ordered broker, broker-owner, vault, borrower, and broker-pseudo reads,
//! - current counterparty defaulting and borrower derivation, and
//! - first-failure `tefBAD_LEDGER` return semantics.

use protocol::Ter;

pub trait LoanSetDoApplyLedgerStateTx {
    type BrokerId;
    type AccountId;

    fn broker_id(&self) -> &Self::BrokerId;
    fn account(&self) -> &Self::AccountId;
    fn counterparty(&self) -> Option<&Self::AccountId>;
}

pub trait LoanSetDoApplyLedgerStateBroker {
    type AccountId;
    type VaultId;

    fn owner(&self) -> &Self::AccountId;
    fn vault_id(&self) -> &Self::VaultId;
    fn account(&self) -> &Self::AccountId;
}

pub trait LoanSetDoApplyLedgerStateVault {
    type AccountId;
    type Asset;

    fn account(&self) -> &Self::AccountId;
    fn asset(&self) -> &Self::Asset;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetDoApplyLedgerState<Broker, AccountState, Vault, AccountId, Asset> {
    pub broker: Broker,
    pub broker_owner: AccountId,
    pub broker_owner_state: AccountState,
    pub vault: Vault,
    pub vault_pseudo: AccountId,
    pub vault_asset: Asset,
    pub counterparty: AccountId,
    pub borrower: AccountId,
    pub borrower_state: AccountState,
    pub broker_pseudo: AccountId,
    pub broker_pseudo_state: AccountState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSetDoApplyLedgerStateFailure {
    BrokerDoesNotExist,
    BrokerOwnerDoesNotExist,
    VaultDoesNotExist,
    BorrowerDoesNotExist,
    BrokerPseudoDoesNotExist,
}

impl LoanSetDoApplyLedgerStateFailure {
    pub const fn ter(self) -> Ter {
        match self {
            Self::BrokerDoesNotExist
            | Self::BrokerOwnerDoesNotExist
            | Self::VaultDoesNotExist
            | Self::BorrowerDoesNotExist
            | Self::BrokerPseudoDoesNotExist => Ter::TEF_BAD_LEDGER,
        }
    }
}

pub fn load_loan_set_do_apply_ledger_state<
    Tx,
    Broker,
    AccountState,
    Vault,
    ReadBroker,
    ReadVault,
    ReadAccount,
>(
    tx: &Tx,
    read_broker: ReadBroker,
    read_vault: ReadVault,
    mut read_account: ReadAccount,
) -> Result<
    LoanSetDoApplyLedgerState<Broker, AccountState, Vault, Tx::AccountId, Vault::Asset>,
    LoanSetDoApplyLedgerStateFailure,
>
where
    Tx: LoanSetDoApplyLedgerStateTx,
    Tx::AccountId: Clone + Eq,
    Broker: LoanSetDoApplyLedgerStateBroker<AccountId = Tx::AccountId>,
    Vault: LoanSetDoApplyLedgerStateVault<AccountId = Tx::AccountId>,
    Vault::Asset: Clone,
    ReadBroker: FnOnce(&Tx::BrokerId) -> Option<Broker>,
    ReadVault: FnOnce(&Broker::VaultId) -> Option<Vault>,
    ReadAccount: FnMut(&Tx::AccountId) -> Option<AccountState>,
{
    let broker =
        read_broker(tx.broker_id()).ok_or(LoanSetDoApplyLedgerStateFailure::BrokerDoesNotExist)?;
    let broker_owner = broker.owner().clone();
    let broker_owner_state = read_account(&broker_owner)
        .ok_or(LoanSetDoApplyLedgerStateFailure::BrokerOwnerDoesNotExist)?;
    let vault =
        read_vault(broker.vault_id()).ok_or(LoanSetDoApplyLedgerStateFailure::VaultDoesNotExist)?;
    let vault_pseudo = vault.account().clone();
    let vault_asset = vault.asset().clone();

    let counterparty = tx
        .counterparty()
        .cloned()
        .unwrap_or_else(|| broker_owner.clone());
    let borrower = if counterparty == broker_owner {
        tx.account().clone()
    } else {
        counterparty.clone()
    };
    let borrower_state =
        read_account(&borrower).ok_or(LoanSetDoApplyLedgerStateFailure::BorrowerDoesNotExist)?;

    let broker_pseudo = broker.account().clone();
    let broker_pseudo_state = read_account(&broker_pseudo)
        .ok_or(LoanSetDoApplyLedgerStateFailure::BrokerPseudoDoesNotExist)?;

    Ok(LoanSetDoApplyLedgerState {
        broker,
        broker_owner,
        broker_owner_state,
        vault,
        vault_pseudo,
        vault_asset,
        counterparty,
        borrower,
        borrower_state,
        broker_pseudo,
        broker_pseudo_state,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::trans_token;

    use super::{
        LoanSetDoApplyLedgerStateBroker, LoanSetDoApplyLedgerStateFailure,
        LoanSetDoApplyLedgerStateTx, LoanSetDoApplyLedgerStateVault,
        load_loan_set_do_apply_ledger_state,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestTx {
        broker_id: &'static str,
        account: &'static str,
        counterparty: Option<&'static str>,
    }

    impl LoanSetDoApplyLedgerStateTx for TestTx {
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBroker {
        owner: &'static str,
        vault_id: &'static str,
        account: &'static str,
    }

    impl LoanSetDoApplyLedgerStateBroker for TestBroker {
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        account: &'static str,
        asset: &'static str,
    }

    impl LoanSetDoApplyLedgerStateVault for TestVault {
        type AccountId = &'static str;
        type Asset = &'static str;

        fn account(&self) -> &Self::AccountId {
            &self.account
        }

        fn asset(&self) -> &Self::Asset {
            &self.asset
        }
    }

    fn test_broker() -> TestBroker {
        TestBroker {
            owner: "broker-owner",
            vault_id: "vault-id",
            account: "broker-pseudo",
        }
    }

    fn test_vault() -> TestVault {
        TestVault {
            account: "vault-pseudo",
            asset: "USD",
        }
    }

    #[test]
    fn load_loan_set_do_apply_ledger_state_returns_loaded_objects_with_explicit_counterparty() {
        let result = load_loan_set_do_apply_ledger_state(
            &TestTx {
                broker_id: "broker-id",
                account: "txn-account",
                counterparty: Some("borrower"),
            },
            |broker_id| {
                assert_eq!(*broker_id, "broker-id");
                Some(test_broker())
            },
            |vault_id| {
                assert_eq!(*vault_id, "vault-id");
                Some(test_vault())
            },
            |account| Some(format!("state:{account}")),
        );

        let state = result.expect("ledger state should load");
        assert_eq!(state.broker_owner, "broker-owner");
        assert_eq!(state.counterparty, "borrower");
        assert_eq!(state.borrower, "borrower");
        assert_eq!(state.vault_pseudo, "vault-pseudo");
        assert_eq!(state.vault_asset, "USD");
        assert_eq!(state.broker_pseudo, "broker-pseudo");
        assert_eq!(state.broker_owner_state, "state:broker-owner");
        assert_eq!(state.borrower_state, "state:borrower");
        assert_eq!(state.broker_pseudo_state, "state:broker-pseudo");
    }

    #[test]
    fn load_loan_set_do_apply_ledger_state_defaults_counterparty_to_broker_owner() {
        let result = load_loan_set_do_apply_ledger_state(
            &TestTx {
                broker_id: "broker-id",
                account: "txn-account",
                counterparty: None,
            },
            |_| Some(test_broker()),
            |_| Some(test_vault()),
            |account| Some(format!("state:{account}")),
        );

        let state = result.expect("ledger state should load");
        assert_eq!(state.counterparty, "broker-owner");
        assert_eq!(state.borrower, "txn-account");
        assert_eq!(state.borrower_state, "state:txn-account");
    }

    #[test]
    fn load_loan_set_do_apply_ledger_state_returns_bad_ledger_when_broker_missing() {
        let result = load_loan_set_do_apply_ledger_state(
            &TestTx {
                broker_id: "missing-broker",
                account: "txn-account",
                counterparty: Some("borrower"),
            },
            |_| None::<TestBroker>,
            |_| Some(test_vault()),
            |_| Some("unused"),
        );

        assert_eq!(
            result,
            Err(LoanSetDoApplyLedgerStateFailure::BrokerDoesNotExist)
        );
        let err = result.expect_err("missing broker should fail");
        assert_eq!(err.ter(), protocol::Ter::TEF_BAD_LEDGER);
        assert_eq!(trans_token(err.ter()), "tefBAD_LEDGER");
    }

    #[test]
    fn load_loan_set_do_apply_ledger_state_stops_after_missing_borrower() {
        let seen = RefCell::new(Vec::new());

        let result = load_loan_set_do_apply_ledger_state(
            &TestTx {
                broker_id: "broker-id",
                account: "txn-account",
                counterparty: Some("borrower"),
            },
            |_| {
                seen.borrow_mut().push("broker");
                Some(test_broker())
            },
            |_| {
                seen.borrow_mut().push("vault");
                Some(test_vault())
            },
            |account| {
                seen.borrow_mut().push(*account);
                match *account {
                    "broker-owner" => Some("owner-state"),
                    "borrower" => None,
                    "broker-pseudo" => Some("broker-pseudo-state"),
                    _ => None,
                }
            },
        );

        assert_eq!(
            result,
            Err(LoanSetDoApplyLedgerStateFailure::BorrowerDoesNotExist)
        );
        assert_eq!(
            *seen.borrow(),
            vec!["broker", "broker-owner", "vault", "borrower"]
        );
    }

    #[test]
    fn load_loan_set_do_apply_ledger_state_reads_front_objects_in() {
        let seen = RefCell::new(Vec::new());

        let result = load_loan_set_do_apply_ledger_state(
            &TestTx {
                broker_id: "broker-id",
                account: "txn-account",
                counterparty: None,
            },
            |broker_id| {
                seen.borrow_mut().push(format!("broker:{broker_id}"));
                Some(test_broker())
            },
            |vault_id| {
                seen.borrow_mut().push(format!("vault:{vault_id}"));
                Some(test_vault())
            },
            |account| {
                seen.borrow_mut().push(format!("account:{account}"));
                Some("state")
            },
        );

        assert!(result.is_ok());
        assert_eq!(
            *seen.borrow(),
            vec![
                "broker:broker-id".to_string(),
                "account:broker-owner".to_string(),
                "vault:vault-id".to_string(),
                "account:txn-account".to_string(),
                "account:broker-pseudo".to_string()
            ]
        );
    }
}
