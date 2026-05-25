//! Integration tests that pin the narrowed Rust `LoanBrokerCoverDeposit.cpp`
//! control flow to the current C++ behavior.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use protocol::{Ter, trans_token};
use tx::loan_broker_cover_deposit::{
    LoanBrokerCoverDepositDoApplyBroker, LoanBrokerCoverDepositDoApplyVault,
    run_loan_broker_cover_deposit_check_extra_features, run_loan_broker_cover_deposit_do_apply,
};
use tx::{
    LoanBrokerCoverDepositPreclaimFacts, LoanBrokerCoverDepositPreflightFacts,
    run_loan_broker_cover_deposit_preclaim, run_loan_broker_cover_deposit_preflight,
};

fn preflight_base() -> LoanBrokerCoverDepositPreflightFacts {
    LoanBrokerCoverDepositPreflightFacts {
        broker_id_is_zero: false,
        amount_is_positive: true,
        amount_is_legal_net: true,
    }
}

fn preclaim_base() -> LoanBrokerCoverDepositPreclaimFacts {
    LoanBrokerCoverDepositPreclaimFacts {
        broker_exists: true,
        submitter_is_broker_owner: true,
        vault_exists: true,
        amount_matches_vault_asset: true,
        can_transfer_result: Ter::TES_SUCCESS,
        frozen_result: Ter::TES_SUCCESS,
        deep_frozen_result: Ter::TES_SUCCESS,
        require_auth_result: Ter::TES_SUCCESS,
        balance_is_less_than_amount: false,
    }
}

#[derive(Clone)]
struct TestBroker {
    vault_id: &'static str,
    pseudo_account: &'static str,
    cover_available: i64,
    steps: Rc<RefCell<Vec<&'static str>>>,
}

impl LoanBrokerCoverDepositDoApplyBroker for TestBroker {
    type AccountId = &'static str;
    type Amount = i64;
    type Asset = &'static str;
    type VaultId = &'static str;

    fn vault_id(&self) -> &Self::VaultId {
        self.steps.borrow_mut().push("read-broker-vault-id");
        &self.vault_id
    }

    fn pseudo_account_id(&self) -> &Self::AccountId {
        self.steps.borrow_mut().push("read-broker-pseudo-account");
        &self.pseudo_account
    }

    fn add_cover_available(&mut self, amount: Self::Amount) {
        self.steps.borrow_mut().push("add-cover-available");
        self.cover_available += amount;
    }
}

#[derive(Clone)]
struct TestVault {
    asset: &'static str,
    steps: Rc<RefCell<Vec<&'static str>>>,
}

impl LoanBrokerCoverDepositDoApplyVault for TestVault {
    type Asset = &'static str;

    fn asset(&self) -> &Self::Asset {
        self.steps.borrow_mut().push("read-vault-asset");
        &self.asset
    }
}

fn make_broker(steps: Rc<RefCell<Vec<&'static str>>>) -> TestBroker {
    TestBroker {
        vault_id: "vault-id",
        pseudo_account: "broker-pseudo",
        cover_available: 5,
        steps,
    }
}

fn make_vault(steps: Rc<RefCell<Vec<&'static str>>>) -> TestVault {
    TestVault {
        asset: "USD",
        steps,
    }
}

#[test]
fn tx_loan_broker_cover_deposit_check_extra_features_delegates_to_lending_gate() {
    let helper_called = Cell::new(false);

    let disabled = run_loan_broker_cover_deposit_check_extra_features(false, || {
        helper_called.set(true);
        true
    });
    assert!(!disabled);
    assert!(!helper_called.get());

    assert!(run_loan_broker_cover_deposit_check_extra_features(
        true,
        || true
    ));
    assert!(!run_loan_broker_cover_deposit_check_extra_features(
        true,
        || false
    ));
}

#[test]
fn tx_loan_broker_cover_deposit_preflight_rejects_zero_broker_id() {
    let result = run_loan_broker_cover_deposit_preflight(LoanBrokerCoverDepositPreflightFacts {
        broker_id_is_zero: true,
        ..preflight_base()
    });

    assert_eq!(result, Ter::TEM_INVALID);
}

#[test]
fn tx_loan_broker_cover_deposit_preflight_prioritizes_zero_broker_id_over_bad_amount() {
    let result = run_loan_broker_cover_deposit_preflight(LoanBrokerCoverDepositPreflightFacts {
        broker_id_is_zero: true,
        amount_is_positive: false,
        amount_is_legal_net: false,
    });

    assert_eq!(result, Ter::TEM_INVALID);
}

#[test]
fn tx_loan_broker_cover_deposit_preflight_rejects_non_positive_amount() {
    let result = run_loan_broker_cover_deposit_preflight(LoanBrokerCoverDepositPreflightFacts {
        amount_is_positive: false,
        ..preflight_base()
    });

    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

#[test]
fn tx_loan_broker_cover_deposit_preflight_rejects_illegal_net_amount() {
    let result = run_loan_broker_cover_deposit_preflight(LoanBrokerCoverDepositPreflightFacts {
        amount_is_legal_net: false,
        ..preflight_base()
    });

    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
    assert_eq!(trans_token(result), "temBAD_AMOUNT");
}

#[test]
fn tx_loan_broker_cover_deposit_preflight_accepts_valid_payload() {
    assert_eq!(
        run_loan_broker_cover_deposit_preflight(preflight_base()),
        Ter::TES_SUCCESS
    );
}

#[test]
fn tx_loan_broker_cover_deposit_preclaim_rejects_missing_broker() {
    let result = run_loan_broker_cover_deposit_preclaim(LoanBrokerCoverDepositPreclaimFacts {
        broker_exists: false,
        ..preclaim_base()
    });

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result), "tecNO_ENTRY");
}

#[test]
fn tx_loan_broker_cover_deposit_preclaim_prioritizes_missing_broker_over_other_failures() {
    let result = run_loan_broker_cover_deposit_preclaim(LoanBrokerCoverDepositPreclaimFacts {
        broker_exists: false,
        submitter_is_broker_owner: false,
        vault_exists: false,
        amount_matches_vault_asset: false,
        can_transfer_result: Ter::TEC_NO_PERMISSION,
        frozen_result: Ter::TEC_FROZEN,
        deep_frozen_result: Ter::TEC_FROZEN,
        require_auth_result: Ter::TER_NO_AUTH,
        balance_is_less_than_amount: true,
    });

    assert_eq!(result, Ter::TEC_NO_ENTRY);
}

#[test]
fn tx_loan_broker_cover_deposit_preclaim_rejects_wrong_owner() {
    let result = run_loan_broker_cover_deposit_preclaim(LoanBrokerCoverDepositPreclaimFacts {
        submitter_is_broker_owner: false,
        ..preclaim_base()
    });

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

#[test]
fn tx_loan_broker_cover_deposit_preclaim_maps_missing_vault_to_bad_ledger() {
    let result = run_loan_broker_cover_deposit_preclaim(LoanBrokerCoverDepositPreclaimFacts {
        vault_exists: false,
        ..preclaim_base()
    });

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
}

#[test]
fn tx_loan_broker_cover_deposit_preclaim_rejects_wrong_asset() {
    let result = run_loan_broker_cover_deposit_preclaim(LoanBrokerCoverDepositPreclaimFacts {
        amount_matches_vault_asset: false,
        ..preclaim_base()
    });

    assert_eq!(result, Ter::TEC_WRONG_ASSET);
}

#[test]
fn tx_loan_broker_cover_deposit_preclaim_returns_transfer_freeze_auth_and_balance_failures() {
    let can_transfer =
        run_loan_broker_cover_deposit_preclaim(LoanBrokerCoverDepositPreclaimFacts {
            can_transfer_result: Ter::TEC_NO_PERMISSION,
            ..preclaim_base()
        });
    let frozen = run_loan_broker_cover_deposit_preclaim(LoanBrokerCoverDepositPreclaimFacts {
        frozen_result: Ter::TEC_FROZEN,
        ..preclaim_base()
    });
    let deep_frozen = run_loan_broker_cover_deposit_preclaim(LoanBrokerCoverDepositPreclaimFacts {
        deep_frozen_result: Ter::TEC_FROZEN,
        ..preclaim_base()
    });
    let auth = run_loan_broker_cover_deposit_preclaim(LoanBrokerCoverDepositPreclaimFacts {
        require_auth_result: Ter::TER_NO_AUTH,
        ..preclaim_base()
    });
    let balance = run_loan_broker_cover_deposit_preclaim(LoanBrokerCoverDepositPreclaimFacts {
        balance_is_less_than_amount: true,
        ..preclaim_base()
    });

    assert_eq!(can_transfer, Ter::TEC_NO_PERMISSION);
    assert_eq!(frozen, Ter::TEC_FROZEN);
    assert_eq!(deep_frozen, Ter::TEC_FROZEN);
    assert_eq!(auth, Ter::TER_NO_AUTH);
    assert_eq!(balance, Ter::TEC_INSUFFICIENT_FUNDS);
}

#[test]
fn tx_loan_broker_cover_deposit_preclaim_accepts_valid_payment() {
    assert_eq!(
        run_loan_broker_cover_deposit_preclaim(preclaim_base()),
        Ter::TES_SUCCESS
    );
}

#[test]
fn tx_loan_broker_cover_deposit_do_apply_runs_current() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let persisted_cover_available = Rc::new(RefCell::new(None));

    let result = run_loan_broker_cover_deposit_do_apply(
        &"depositor",
        &10_i64,
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("read-broker");
                Some(make_broker(Rc::clone(&steps)))
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |vault_id: &&'static str| {
                steps.borrow_mut().push("read-vault");
                assert_eq!(*vault_id, "vault-id");
                Some(make_vault(Rc::clone(&steps)))
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |submitter, broker_pseudo, amount| {
                steps.borrow_mut().push("send-assets");
                assert_eq!(*submitter, "depositor");
                assert_eq!(*broker_pseudo, "broker-pseudo");
                assert_eq!(*amount, 10);
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            let persisted_cover_available = Rc::clone(&persisted_cover_available);
            move |broker: &mut TestBroker| {
                steps.borrow_mut().push("persist-broker");
                *persisted_cover_available.borrow_mut() = Some(broker.cover_available);
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |broker: &TestBroker, asset: &&'static str| {
                steps.borrow_mut().push("associate-asset");
                assert_eq!(broker.cover_available, 15);
                assert_eq!(*asset, "USD");
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(*persisted_cover_available.borrow(), Some(15));
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read-broker",
            "read-broker-vault-id",
            "read-vault",
            "read-broker-pseudo-account",
            "send-assets",
            "add-cover-available",
            "persist-broker",
            "read-vault-asset",
            "associate-asset",
        ]
    );
}

#[test]
fn tx_loan_broker_cover_deposit_do_apply_maps_missing_broker_and_vault_to_internal() {
    let read_vault_called = Rc::new(Cell::new(false));

    let missing_broker =
        run_loan_broker_cover_deposit_do_apply::<TestBroker, TestVault, _, _, _, _, _>(
            &"depositor",
            &10_i64,
            || None,
            {
                let read_vault_called = Rc::clone(&read_vault_called);
                move |_| {
                    read_vault_called.set(true);
                    None
                }
            },
            |_, _, _| Ter::TES_SUCCESS,
            |_: &mut TestBroker| panic!("persist should not run"),
            |_: &TestBroker, _| panic!("associate should not run"),
        );

    assert_eq!(missing_broker, Ter::TEF_INTERNAL);
    assert!(!read_vault_called.get());

    let steps = Rc::new(RefCell::new(Vec::new()));
    let missing_vault = run_loan_broker_cover_deposit_do_apply(
        &"depositor",
        &10_i64,
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("read-broker");
                Some(make_broker(Rc::clone(&steps)))
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |vault_id: &&'static str| {
                steps.borrow_mut().push("read-vault");
                assert_eq!(*vault_id, "vault-id");
                None::<TestVault>
            }
        },
        |_, _, _| Ter::TES_SUCCESS,
        |_: &mut TestBroker| panic!("persist should not run"),
        |_: &TestBroker, _| panic!("associate should not run"),
    );

    assert_eq!(missing_vault, Ter::TEF_INTERNAL);
}

#[test]
fn tx_loan_broker_cover_deposit_do_apply_returns_transfer_failure_unchanged() {
    let steps = Rc::new(RefCell::new(Vec::new()));

    let result = run_loan_broker_cover_deposit_do_apply(
        &"depositor",
        &10_i64,
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("read-broker");
                Some(make_broker(Rc::clone(&steps)))
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_| {
                steps.borrow_mut().push("read-vault");
                Some(make_vault(Rc::clone(&steps)))
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, _, _| {
                steps.borrow_mut().push("send-assets");
                Ter::TER_NO_RIPPLE
            }
        },
        |_: &mut TestBroker| panic!("persist should not run"),
        |_: &TestBroker, _| panic!("associate should not run"),
    );

    assert_eq!(result, Ter::TER_NO_RIPPLE);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read-broker",
            "read-broker-vault-id",
            "read-vault",
            "read-broker-pseudo-account",
            "send-assets"
        ]
    );
}
