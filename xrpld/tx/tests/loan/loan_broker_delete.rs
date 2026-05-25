//! Integration tests that pin the narrowed Rust `LoanBrokerDelete.cpp`
//! metadata, `preflight(...)`, and `preclaim(...)` wrappers to the current
//! C++ behavior.

use std::cell::Cell;

use protocol::{Ter, trans_token};
use tx::loan_broker_delete::{
    LoanBrokerDeleteDoApplyBroker, LoanBrokerDeleteDoApplyPseudoAccount,
    LoanBrokerDeleteDoApplyVault, run_loan_broker_delete_do_apply,
};
use tx::{
    LoanBrokerDeletePreclaimFacts, run_loan_broker_delete_check_extra_features,
    run_loan_broker_delete_preclaim, run_loan_broker_delete_preflight,
};

fn base() -> LoanBrokerDeletePreclaimFacts {
    LoanBrokerDeletePreclaimFacts {
        broker_exists: true,
        submitter_is_broker_owner: true,
        owner_count_is_zero: true,
        vault_exists: true,
        rounded_debt_total_is_zero: true,
        cover_available_is_positive: false,
        deep_frozen_result: Ter::TES_SUCCESS,
    }
}

#[derive(Clone)]
struct TestBroker {
    pseudo_account_id: &'static str,
    vault_id: &'static str,
    owner_node: u64,
    vault_node: u64,
    key: &'static str,
    cover_available: i64,
}

impl LoanBrokerDeleteDoApplyBroker for TestBroker {
    type AccountId = &'static str;
    type VaultId = &'static str;
    type DirNode = u64;
    type BrokerKey = &'static str;
    type Amount = i64;

    fn pseudo_account_id(&self) -> &Self::AccountId {
        &self.pseudo_account_id
    }

    fn vault_id(&self) -> &Self::VaultId {
        &self.vault_id
    }

    fn owner_node(&self) -> &Self::DirNode {
        &self.owner_node
    }

    fn vault_node(&self) -> &Self::DirNode {
        &self.vault_node
    }

    fn key(&self) -> &Self::BrokerKey {
        &self.key
    }

    fn cover_available(&self) -> &Self::Amount {
        &self.cover_available
    }
}

#[derive(Clone)]
struct TestVault {
    pseudo_id: &'static str,
    asset: &'static str,
}

impl LoanBrokerDeleteDoApplyVault for TestVault {
    type AccountId = &'static str;
    type Asset = &'static str;

    fn pseudo_id(&self) -> &Self::AccountId {
        &self.pseudo_id
    }

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }
}

#[derive(Clone)]
struct TestPseudoAccount {
    balance: i64,
    owner_count: u32,
}

impl LoanBrokerDeleteDoApplyPseudoAccount for TestPseudoAccount {
    type Amount = i64;
    type OwnerCount = u32;

    fn balance(&self) -> &Self::Amount {
        &self.balance
    }

    fn owner_count(&self) -> &Self::OwnerCount {
        &self.owner_count
    }
}

#[test]
fn tx_loan_broker_delete_check_extra_features_delegates_to_lending_gate() {
    let helper_called = Cell::new(false);

    let disabled = run_loan_broker_delete_check_extra_features(false, || {
        helper_called.set(true);
        true
    });
    assert!(!disabled);
    assert!(!helper_called.get());

    assert!(run_loan_broker_delete_check_extra_features(true, || true));
    assert!(!run_loan_broker_delete_check_extra_features(true, || false));
}

#[test]
fn tx_loan_broker_delete_preflight_rejects_zero_broker_id() {
    assert_eq!(run_loan_broker_delete_preflight(true), Ter::TEM_INVALID);
}

#[test]
fn tx_loan_broker_delete_preclaim_rejects_missing_broker() {
    let result = run_loan_broker_delete_preclaim(LoanBrokerDeletePreclaimFacts::default());

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result), "tecNO_ENTRY");
}

#[test]
fn tx_loan_broker_delete_preclaim_rejects_wrong_owner() {
    let result = run_loan_broker_delete_preclaim(LoanBrokerDeletePreclaimFacts {
        broker_exists: true,
        ..LoanBrokerDeletePreclaimFacts::default()
    });

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

#[test]
fn tx_loan_broker_delete_preclaim_rejects_outstanding_owner_count() {
    let result = run_loan_broker_delete_preclaim(LoanBrokerDeletePreclaimFacts {
        owner_count_is_zero: false,
        ..base()
    });

    assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
}

#[test]
fn tx_loan_broker_delete_preclaim_maps_missing_vault_to_bad_ledger() {
    let result = run_loan_broker_delete_preclaim(LoanBrokerDeletePreclaimFacts {
        vault_exists: false,
        ..base()
    });

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
}

#[test]
fn tx_loan_broker_delete_preclaim_rejects_nonzero_rounded_debt() {
    let result = run_loan_broker_delete_preclaim(LoanBrokerDeletePreclaimFacts {
        rounded_debt_total_is_zero: false,
        ..base()
    });

    assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
}

#[test]
fn tx_loan_broker_delete_preclaim_returns_deep_freeze_failure_when_cover_exists() {
    let result = run_loan_broker_delete_preclaim(LoanBrokerDeletePreclaimFacts {
        cover_available_is_positive: true,
        deep_frozen_result: Ter::TEC_FROZEN,
        ..base()
    });

    assert_eq!(result, Ter::TEC_FROZEN);
}

#[test]
fn tx_loan_broker_delete_preclaim_accepts_empty_broker() {
    assert_eq!(run_loan_broker_delete_preclaim(base()), Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_broker_delete_do_apply_runs_current() {
    let steps = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_loan_broker_delete_do_apply(
        &"broker-1",
        &"account-1",
        |_| {
            Some(TestBroker {
                pseudo_account_id: "broker-pseudo",
                vault_id: "vault-1",
                owner_node: 7,
                vault_node: 9,
                key: "broker-key",
                cover_available: 42,
            })
        },
        |_| {
            Some(TestVault {
                pseudo_id: "vault-pseudo",
                asset: "USD",
            })
        },
        {
            let steps = std::rc::Rc::clone(&steps);
            move |account, node, key| {
                steps
                    .borrow_mut()
                    .push(format!("remove_owner_dir:{account}:{node}:{key}"));
                true
            }
        },
        {
            let steps = std::rc::Rc::clone(&steps);
            move |account, node, key| {
                steps
                    .borrow_mut()
                    .push(format!("remove_vault_dir:{account}:{node}:{key}"));
                true
            }
        },
        {
            let steps = std::rc::Rc::clone(&steps);
            move |pseudo, account, amount| {
                steps
                    .borrow_mut()
                    .push(format!("account_send:{pseudo}:{account}:{amount}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = std::rc::Rc::clone(&steps);
            move |pseudo, asset| {
                steps
                    .borrow_mut()
                    .push(format!("remove_empty_holding:{pseudo}:{asset}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = std::rc::Rc::clone(&steps);
            move |pseudo| {
                steps.borrow_mut().push(format!("read_pseudo:{pseudo}"));
                Some(TestPseudoAccount {
                    balance: 0,
                    owner_count: 0,
                })
            }
        },
        {
            let steps = std::rc::Rc::clone(&steps);
            move |account| {
                steps.borrow_mut().push(format!("read_owner:{account}"));
                Some("owner-account")
            }
        },
        {
            let steps = std::rc::Rc::clone(&steps);
            move |pseudo| {
                steps.borrow_mut().push(format!("read_directory:{pseudo}"));
                false
            }
        },
        {
            let steps = std::rc::Rc::clone(&steps);
            move |_| steps.borrow_mut().push("erase_pseudo".to_string())
        },
        {
            let steps = std::rc::Rc::clone(&steps);
            move |_| steps.borrow_mut().push("erase_broker".to_string())
        },
        {
            let steps = std::rc::Rc::clone(&steps);
            move |owner, delta| {
                steps
                    .borrow_mut()
                    .push(format!("adjust_owner:{owner}:{delta}"))
            }
        },
        {
            let steps = std::rc::Rc::clone(&steps);
            move |_, asset| steps.borrow_mut().push(format!("associate_asset:{asset}"))
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "remove_owner_dir:account-1:7:broker-key",
            "remove_vault_dir:vault-pseudo:9:broker-key",
            "account_send:broker-pseudo:account-1:42",
            "remove_empty_holding:broker-pseudo:USD",
            "read_pseudo:broker-pseudo",
            "read_directory:broker-pseudo",
            "erase_pseudo",
            "erase_broker",
            "read_owner:account-1",
            "adjust_owner:owner-account:-2",
            "associate_asset:USD",
        ]
    );
}

#[test]
fn tx_loan_broker_delete_do_apply_returns_missing_broker() {
    let result = run_loan_broker_delete_do_apply(
        &"broker-1",
        &"account-1",
        |_| None::<TestBroker>,
        |_| None::<TestVault>,
        |_, _, _| true,
        |_, _, _| true,
        |_, _, _| Ter::TES_SUCCESS,
        |_, _| Ter::TES_SUCCESS,
        |_| None,
        |_| Some("owner"),
        |_| false,
        |_: TestPseudoAccount| {},
        |_: TestBroker| {},
        |_, _| {},
        |_, _| {},
    );

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
}
