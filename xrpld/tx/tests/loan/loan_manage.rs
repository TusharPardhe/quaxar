//! Integration tests that pin the narrowed Rust `LoanManage.cpp` metadata,
//! `preflight(...)`, and `preclaim(...)` wrappers to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::loan_manage::{LoanManageOwedToVaultFacts, run_loan_manage_owed_to_vault};
use tx::{
    LOAN_DEFAULT_FLAG, LOAN_IMPAIR_FLAG, LOAN_UNIMPAIR_FLAG, LoanManageDoApplyBroker,
    LoanManageDoApplyFacts, LoanManageDoApplyLoan, LoanManageDoApplyVault, LoanManagePreclaimFacts,
    LoanManagePreflightFacts, get_loan_manage_flags_mask, run_loan_manage_check_extra_features,
    run_loan_manage_do_apply, run_loan_manage_preclaim, run_loan_manage_preflight,
};

fn base() -> LoanManagePreclaimFacts {
    LoanManagePreclaimFacts {
        loan_exists: true,
        loan_is_defaulted: false,
        loan_is_impaired: false,
        tx_requests_impair: false,
        tx_requests_unimpair: false,
        tx_requests_default: false,
        payment_remaining_is_zero: false,
        default_is_too_soon: false,
        broker_exists: true,
        submitter_is_broker_owner: true,
    }
}

#[derive(Clone)]
struct TestLoan {
    broker_id: &'static str,
    steps: Rc<std::cell::RefCell<Vec<String>>>,
}

impl LoanManageDoApplyLoan for TestLoan {
    type BrokerId = &'static str;
    type Asset = &'static str;

    fn broker_id(&self) -> &Self::BrokerId {
        &self.broker_id
    }

    fn associate_asset(&mut self, asset: &Self::Asset) {
        self.steps
            .borrow_mut()
            .push(format!("associate_loan_asset:{asset}"));
    }
}

#[derive(Clone)]
struct TestBroker {
    vault_id: &'static str,
    steps: Rc<std::cell::RefCell<Vec<String>>>,
}

impl LoanManageDoApplyBroker for TestBroker {
    type VaultId = &'static str;
    type Asset = &'static str;

    fn vault_id(&self) -> &Self::VaultId {
        &self.vault_id
    }

    fn associate_asset(&mut self, asset: &Self::Asset) {
        self.steps
            .borrow_mut()
            .push(format!("associate_broker_asset:{asset}"));
    }
}

#[derive(Clone)]
struct TestVault {
    asset: &'static str,
    steps: Rc<std::cell::RefCell<Vec<String>>>,
}

impl LoanManageDoApplyVault for TestVault {
    type Asset = &'static str;

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }

    fn associate_asset(&mut self, asset: &Self::Asset) {
        self.steps
            .borrow_mut()
            .push(format!("associate_vault_asset:{asset}"));
    }
}

#[test]
fn tx_loan_manage_check_extra_features_delegates_to_lending_gate() {
    let helper_called = Cell::new(false);

    let disabled = run_loan_manage_check_extra_features(false, || {
        helper_called.set(true);
        true
    });
    assert!(!disabled);
    assert!(!helper_called.get());

    assert!(run_loan_manage_check_extra_features(true, || true));
    assert!(!run_loan_manage_check_extra_features(true, || false));
}

#[test]
fn tx_loan_manage_flags_mask_metadata() {
    assert_eq!(get_loan_manage_flags_mask(), 0x3ff8_ffff);
}

#[test]
fn tx_loan_manage_preflight_rejects_zero_loan_id() {
    let result = run_loan_manage_preflight(LoanManagePreflightFacts {
        loan_id_is_zero: true,
        tx_specific_flags: 0,
    });

    assert_eq!(result, Ter::TEM_INVALID);
}

#[test]
fn tx_loan_manage_preflight_rejects_multiple_tx_specific_flags() {
    let result = run_loan_manage_preflight(LoanManagePreflightFacts {
        loan_id_is_zero: false,
        tx_specific_flags: LOAN_DEFAULT_FLAG | LOAN_IMPAIR_FLAG,
    });

    assert_eq!(result, Ter::TEM_INVALID_FLAG);
    assert_eq!(trans_token(result), "temINVALID_FLAG");
}

#[test]
fn tx_loan_manage_preflight_accepts_single_tx_specific_flag() {
    let impair = run_loan_manage_preflight(LoanManagePreflightFacts {
        loan_id_is_zero: false,
        tx_specific_flags: LOAN_IMPAIR_FLAG,
    });
    let unimpair = run_loan_manage_preflight(LoanManagePreflightFacts {
        loan_id_is_zero: false,
        tx_specific_flags: LOAN_UNIMPAIR_FLAG,
    });

    assert_eq!(impair, Ter::TES_SUCCESS);
    assert_eq!(unimpair, Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_manage_preclaim_rejects_missing_loan() {
    assert_eq!(
        run_loan_manage_preclaim(LoanManagePreclaimFacts::default()),
        Ter::TEC_NO_ENTRY
    );
}

#[test]
fn tx_loan_manage_preclaim_rejects_defaulted_loan() {
    assert_eq!(
        run_loan_manage_preclaim(LoanManagePreclaimFacts {
            loan_is_defaulted: true,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn tx_loan_manage_preclaim_rejects_duplicate_impair() {
    assert_eq!(
        run_loan_manage_preclaim(LoanManagePreclaimFacts {
            loan_is_impaired: true,
            tx_requests_impair: true,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn tx_loan_manage_preclaim_rejects_unimpairing_unimpaired_loan() {
    assert_eq!(
        run_loan_manage_preclaim(LoanManagePreclaimFacts {
            tx_requests_unimpair: true,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn tx_loan_manage_preclaim_rejects_paid_off_loan() {
    assert_eq!(
        run_loan_manage_preclaim(LoanManagePreclaimFacts {
            payment_remaining_is_zero: true,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn tx_loan_manage_preclaim_rejects_early_default() {
    assert_eq!(
        run_loan_manage_preclaim(LoanManagePreclaimFacts {
            tx_requests_default: true,
            default_is_too_soon: true,
            ..base()
        }),
        Ter::TEC_TOO_SOON
    );
}

#[test]
fn tx_loan_manage_preclaim_maps_missing_broker_to_internal() {
    assert_eq!(
        run_loan_manage_preclaim(LoanManagePreclaimFacts {
            broker_exists: false,
            ..base()
        }),
        Ter::TEC_INTERNAL
    );
}

#[test]
fn tx_loan_manage_preclaim_rejects_non_owner_submitter() {
    assert_eq!(
        run_loan_manage_preclaim(LoanManagePreclaimFacts {
            submitter_is_broker_owner: false,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn tx_loan_manage_preclaim_accepts_allowed_transition() {
    assert_eq!(run_loan_manage_preclaim(base()), Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_manage_owed_to_vault_formula() {
    let amount = run_loan_manage_owed_to_vault(LoanManageOwedToVaultFacts {
        total_value_outstanding: 125_i64,
        management_fee_outstanding: 25_i64,
    });

    assert_eq!(amount, 100_i64);
}

#[test]
fn tx_loan_manage_owed_to_vault_preserves_zero_management_fee() {
    let amount = run_loan_manage_owed_to_vault(LoanManageOwedToVaultFacts {
        total_value_outstanding: 125_i64,
        management_fee_outstanding: 0_i64,
    });

    assert_eq!(amount, 125_i64);
}

#[test]
fn tx_loan_manage_do_apply_runs_current_load_and_default_first_order() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_loan_manage_do_apply(
        &"loan-1",
        LoanManageDoApplyFacts {
            tx_requests_default: true,
            tx_requests_impair: true,
            tx_requests_unimpair: true,
            security_fix_3_1_3_enabled: true,
        },
        {
            let steps = Rc::clone(&steps);
            move |loan_id| {
                steps.borrow_mut().push(format!("read_loan:{loan_id}"));
                Some(TestLoan {
                    broker_id: "broker-1",
                    steps: Rc::clone(&steps),
                })
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |broker_id| {
                steps.borrow_mut().push(format!("read_broker:{broker_id}"));
                Some(TestBroker {
                    vault_id: "vault-1",
                    steps: Rc::clone(&steps),
                })
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |vault_id| {
                steps.borrow_mut().push(format!("read_vault:{vault_id}"));
                Some(TestVault {
                    asset: "USD",
                    steps: Rc::clone(&steps),
                })
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, _, _, asset| {
                steps.borrow_mut().push(format!("default:{asset}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, _, asset| {
                steps.borrow_mut().push(format!("impair:{asset}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, _, asset| {
                steps.borrow_mut().push(format!("unimpair:{asset}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_| steps.borrow_mut().push("update_loan".to_string())
        },
        {
            let steps = Rc::clone(&steps);
            move |_| steps.borrow_mut().push("update_broker".to_string())
        },
        {
            let steps = Rc::clone(&steps);
            move |_| steps.borrow_mut().push("update_vault".to_string())
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read_loan:loan-1",
            "read_broker:broker-1",
            "read_vault:vault-1",
            "default:USD",
            "associate_loan_asset:USD",
            "associate_broker_asset:USD",
            "associate_vault_asset:USD",
            "update_loan",
            "update_broker",
            "update_vault",
        ]
    );
}

#[test]
fn tx_loan_manage_do_apply_maps_missing_objects_to_bad_ledger() {
    let broker_called = Cell::new(false);

    let missing_loan = run_loan_manage_do_apply(
        &"loan-1",
        LoanManageDoApplyFacts::default(),
        |_| None::<TestLoan>,
        |_| {
            broker_called.set(true);
            None::<TestBroker>
        },
        |_| None::<TestVault>,
        |_, _, _, _| Ter::TES_SUCCESS,
        |_, _, _| Ter::TES_SUCCESS,
        |_, _, _| Ter::TES_SUCCESS,
        |_| {},
        |_| {},
        |_| {},
    );
    assert_eq!(missing_loan, Ter::TEF_BAD_LEDGER);
    assert!(!broker_called.get());

    let missing_broker = run_loan_manage_do_apply(
        &"loan-1",
        LoanManageDoApplyFacts::default(),
        |_| {
            Some(TestLoan {
                broker_id: "broker-1",
                steps: Rc::new(std::cell::RefCell::new(Vec::new())),
            })
        },
        |_| None::<TestBroker>,
        |_| None::<TestVault>,
        |_, _, _, _| Ter::TES_SUCCESS,
        |_, _, _| Ter::TES_SUCCESS,
        |_, _, _| Ter::TES_SUCCESS,
        |_| {},
        |_| {},
        |_| {},
    );
    assert_eq!(missing_broker, Ter::TEF_BAD_LEDGER);

    let missing_vault = run_loan_manage_do_apply(
        &"loan-1",
        LoanManageDoApplyFacts::default(),
        |_| {
            Some(TestLoan {
                broker_id: "broker-1",
                steps: Rc::new(std::cell::RefCell::new(Vec::new())),
            })
        },
        |_| {
            Some(TestBroker {
                vault_id: "vault-1",
                steps: Rc::new(std::cell::RefCell::new(Vec::new())),
            })
        },
        |_| None::<TestVault>,
        |_, _, _, _| Ter::TES_SUCCESS,
        |_, _, _| Ter::TES_SUCCESS,
        |_, _, _| Ter::TES_SUCCESS,
        |_| {},
        |_| {},
        |_| {},
    );
    assert_eq!(missing_vault, Ter::TEF_BAD_LEDGER);
}

#[test]
fn tx_loan_manage_do_apply_keeps_associate_asset_gated_by_amendment_and_success() {
    let disabled_steps = Rc::new(std::cell::RefCell::new(Vec::new()));
    let disabled = run_loan_manage_do_apply(
        &"loan-1",
        LoanManageDoApplyFacts {
            tx_requests_impair: true,
            security_fix_3_1_3_enabled: false,
            ..LoanManageDoApplyFacts::default()
        },
        {
            let steps = Rc::clone(&disabled_steps);
            move |_| {
                Some(TestLoan {
                    broker_id: "broker-1",
                    steps: Rc::clone(&steps),
                })
            }
        },
        {
            let steps = Rc::clone(&disabled_steps);
            move |_| {
                Some(TestBroker {
                    vault_id: "vault-1",
                    steps: Rc::clone(&steps),
                })
            }
        },
        {
            let steps = Rc::clone(&disabled_steps);
            move |_| {
                Some(TestVault {
                    asset: "USD",
                    steps: Rc::clone(&steps),
                })
            }
        },
        |_, _, _, _| Ter::TES_SUCCESS,
        {
            let steps = Rc::clone(&disabled_steps);
            move |_, _, asset| {
                steps.borrow_mut().push(format!("impair:{asset}"));
                Ter::TES_SUCCESS
            }
        },
        |_, _, _| Ter::TES_SUCCESS,
        |_| disabled_steps.borrow_mut().push("update_loan".to_string()),
        |_| {
            disabled_steps
                .borrow_mut()
                .push("update_broker".to_string())
        },
        |_| disabled_steps.borrow_mut().push("update_vault".to_string()),
    );
    assert_eq!(disabled, Ter::TES_SUCCESS);
    assert_eq!(disabled_steps.borrow().as_slice(), ["impair:USD"]);

    let failed_steps = Rc::new(std::cell::RefCell::new(Vec::new()));
    let failed = run_loan_manage_do_apply(
        &"loan-1",
        LoanManageDoApplyFacts {
            tx_requests_unimpair: true,
            security_fix_3_1_3_enabled: true,
            ..LoanManageDoApplyFacts::default()
        },
        {
            let steps = Rc::clone(&failed_steps);
            move |_| {
                Some(TestLoan {
                    broker_id: "broker-1",
                    steps: Rc::clone(&steps),
                })
            }
        },
        {
            let steps = Rc::clone(&failed_steps);
            move |_| {
                Some(TestBroker {
                    vault_id: "vault-1",
                    steps: Rc::clone(&steps),
                })
            }
        },
        {
            let steps = Rc::clone(&failed_steps);
            move |_| {
                Some(TestVault {
                    asset: "USD",
                    steps: Rc::clone(&steps),
                })
            }
        },
        |_, _, _, _| Ter::TES_SUCCESS,
        |_, _, _| Ter::TES_SUCCESS,
        {
            let steps = Rc::clone(&failed_steps);
            move |_, _, asset| {
                steps.borrow_mut().push(format!("unimpair:{asset}"));
                Ter::TEC_INTERNAL
            }
        },
        |_| failed_steps.borrow_mut().push("update_loan".to_string()),
        |_| failed_steps.borrow_mut().push("update_broker".to_string()),
        |_| failed_steps.borrow_mut().push("update_vault".to_string()),
    );
    assert_eq!(failed, Ter::TEC_INTERNAL);
    assert_eq!(failed_steps.borrow().as_slice(), ["unimpair:USD"]);
}
