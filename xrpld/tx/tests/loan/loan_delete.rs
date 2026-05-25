//! Integration tests that pin the narrowed Rust `LoanDelete.cpp` behavior to
//! the current C++ control flow.

use std::{cell::Cell, rc::Rc};

use tx::loan_delete::run_loan_delete_check_extra_features;
use tx::{
    LoanDeleteDoApplyBroker, LoanDeleteDoApplyLoan, LoanDeleteDoApplyVault,
    LoanDeleteForgiveDebtBroker, LoanDeleteForgiveDebtError, LoanDeleteForgiveDebtOutcome,
    LoanDeleteForgiveDebtVault, LoanDeleteLoan, LoanDeleteLoanBroker, LoanDeleteTx, LoanDeleteView,
    run_loan_delete_do_apply, run_loan_delete_forgive_last_broker_debt, run_loan_delete_preclaim,
    run_loan_delete_preflight,
};

#[derive(Clone, Copy)]
struct TestTx {
    account: &'static str,
    loan_id: &'static str,
}

impl LoanDeleteTx for TestTx {
    type AccountId = &'static str;
    type LoanId = &'static str;

    fn account(&self) -> &Self::AccountId {
        &self.account
    }

    fn loan_id(&self) -> &Self::LoanId {
        &self.loan_id
    }
}

#[derive(Clone, Copy)]
struct TestLoan {
    payment_remaining: u32,
    loan_broker_id: &'static str,
    borrower: &'static str,
}

impl LoanDeleteLoan for TestLoan {
    type AccountId = &'static str;
    type BrokerId = &'static str;
    type PaymentRemaining = u32;

    fn payment_remaining(&self) -> &Self::PaymentRemaining {
        &self.payment_remaining
    }

    fn loan_broker_id(&self) -> &Self::BrokerId {
        &self.loan_broker_id
    }

    fn borrower(&self) -> &Self::AccountId {
        &self.borrower
    }
}

#[derive(Clone, Copy)]
struct TestLoanBroker {
    owner: &'static str,
}

impl LoanDeleteLoanBroker for TestLoanBroker {
    type AccountId = &'static str;

    fn owner(&self) -> &Self::AccountId {
        &self.owner
    }
}

#[derive(Clone, Copy)]
struct TestApplyLoan {
    borrower: &'static str,
    loan_broker_id: &'static str,
    loan_broker_node: u64,
    owner_node: u64,
}

impl LoanDeleteDoApplyLoan for TestApplyLoan {
    type AccountId = &'static str;
    type BrokerId = &'static str;
    type DirNode = u64;

    fn borrower(&self) -> &Self::AccountId {
        &self.borrower
    }

    fn loan_broker_id(&self) -> &Self::BrokerId {
        &self.loan_broker_id
    }

    fn loan_broker_node(&self) -> &Self::DirNode {
        &self.loan_broker_node
    }

    fn owner_node(&self) -> &Self::DirNode {
        &self.owner_node
    }
}

#[derive(Clone, Copy)]
struct TestApplyBroker {
    pseudo_account_id: &'static str,
    vault_id: &'static str,
}

impl LoanDeleteDoApplyBroker for TestApplyBroker {
    type AccountId = &'static str;
    type VaultId = &'static str;

    fn pseudo_account_id(&self) -> &Self::AccountId {
        &self.pseudo_account_id
    }

    fn vault_id(&self) -> &Self::VaultId {
        &self.vault_id
    }
}

#[derive(Clone, Copy)]
struct TestApplyVault {
    asset: &'static str,
}

impl LoanDeleteDoApplyVault for TestApplyVault {
    type Asset = &'static str;

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }
}

#[derive(Clone, Copy)]
struct TestForgiveBroker {
    owner_count: u32,
    debt_total: i64,
}

impl LoanDeleteForgiveDebtBroker for TestForgiveBroker {
    type Amount = i64;
    type OwnerCount = u32;

    fn owner_count(&self) -> &Self::OwnerCount {
        &self.owner_count
    }

    fn debt_total(&self) -> &Self::Amount {
        &self.debt_total
    }

    fn clear_debt_total(&mut self) {
        self.debt_total = 0;
    }
}

impl LoanDeleteForgiveDebtVault for TestApplyVault {
    type Asset = &'static str;

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }
}

struct TestView {
    loan: Option<TestLoan>,
    loanbroker: Option<TestLoanBroker>,
}

impl LoanDeleteView for TestView {
    type LoanId = &'static str;
    type BrokerId = &'static str;
    type AccountId = &'static str;
    type Loan = TestLoan;
    type LoanBroker = TestLoanBroker;

    fn read_loan(&self, _loan_id: &Self::LoanId) -> Option<&Self::Loan> {
        self.loan.as_ref()
    }

    fn read_loanbroker(&self, _broker_id: &Self::BrokerId) -> Option<&Self::LoanBroker> {
        self.loanbroker.as_ref()
    }
}

#[test]
fn loan_delete_check_extra_features_short_circuits() {
    let mut called = false;
    assert!(!run_loan_delete_check_extra_features(false, || {
        called = true;
        true
    }));
    assert!(!called);
    assert!(run_loan_delete_check_extra_features(true, || true));
    assert!(!run_loan_delete_check_extra_features(true, || false));
}

#[test]
fn loan_delete_preflight_rejects_zero_loan_id() {
    let result = run_loan_delete_preflight(&TestTx {
        account: "alice",
        loan_id: "",
    });

    assert_eq!(result, protocol::Ter::TEM_INVALID);
}

#[test]
fn loan_delete_preclaim_returns_missing_loan_active_loan_and_missing_broker() {
    let tx = TestTx {
        account: "alice",
        loan_id: "loan-1",
    };

    assert_eq!(
        run_loan_delete_preclaim(
            &tx,
            &TestView {
                loan: None,
                loanbroker: None,
            }
        ),
        protocol::Ter::TEC_NO_ENTRY
    );

    assert_eq!(
        run_loan_delete_preclaim(
            &tx,
            &TestView {
                loan: Some(TestLoan {
                    payment_remaining: 1,
                    loan_broker_id: "broker-1",
                    borrower: "alice",
                }),
                loanbroker: Some(TestLoanBroker { owner: "alice" }),
            }
        ),
        protocol::Ter::TEC_HAS_OBLIGATIONS
    );

    assert_eq!(
        run_loan_delete_preclaim(
            &tx,
            &TestView {
                loan: Some(TestLoan {
                    payment_remaining: 0,
                    loan_broker_id: "broker-1",
                    borrower: "alice",
                }),
                loanbroker: None,
            }
        ),
        protocol::Ter::TEC_INTERNAL
    );
}

#[test]
fn loan_delete_preclaim_requires_broker_owner_or_borrower() {
    let tx = TestTx {
        account: "charlie",
        loan_id: "loan-1",
    };

    let result = run_loan_delete_preclaim(
        &tx,
        &TestView {
            loan: Some(TestLoan {
                payment_remaining: 0,
                loan_broker_id: "broker-1",
                borrower: "bob",
            }),
            loanbroker: Some(TestLoanBroker { owner: "alice" }),
        },
    );

    assert_eq!(result, protocol::Ter::TEC_NO_PERMISSION);
}

#[test]
fn loan_delete_preclaim_accepts_owner_or_borrower() {
    let owner_tx = TestTx {
        account: "alice",
        loan_id: "loan-1",
    };

    let owner_result = run_loan_delete_preclaim(
        &owner_tx,
        &TestView {
            loan: Some(TestLoan {
                payment_remaining: 0,
                loan_broker_id: "broker-1",
                borrower: "bob",
            }),
            loanbroker: Some(TestLoanBroker { owner: "alice" }),
        },
    );
    assert_eq!(owner_result, protocol::Ter::TES_SUCCESS);

    let borrower_tx = TestTx {
        account: "bob",
        loan_id: "loan-1",
    };
    let borrower_result = run_loan_delete_preclaim(
        &borrower_tx,
        &TestView {
            loan: Some(TestLoan {
                payment_remaining: 0,
                loan_broker_id: "broker-1",
                borrower: "bob",
            }),
            loanbroker: Some(TestLoanBroker { owner: "alice" }),
        },
    );
    assert_eq!(borrower_result, protocol::Ter::TES_SUCCESS);
}

#[test]
fn loan_delete_do_apply_runs_current_cpp_stage_order() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_loan_delete_do_apply(
        &"loan-1",
        |_| {
            Some(TestApplyLoan {
                borrower: "borrower",
                loan_broker_id: "broker-1",
                loan_broker_node: 7,
                owner_node: 11,
            })
        },
        {
            let steps = Rc::clone(&steps);
            move |borrower| {
                steps.borrow_mut().push(format!("read_borrower:{borrower}"));
                Some("borrower-account")
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |broker_id| {
                steps.borrow_mut().push(format!("read_broker:{broker_id}"));
                Some(TestApplyBroker {
                    pseudo_account_id: "broker-pseudo",
                    vault_id: "vault-1",
                })
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |vault_id| {
                steps.borrow_mut().push(format!("read_vault:{vault_id}"));
                Some(TestApplyVault { asset: "USD" })
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |pseudo_account, node, loan_id| {
                steps.borrow_mut().push(format!(
                    "remove_broker_dir:{pseudo_account}:{node}:{loan_id}"
                ));
                true
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |borrower, node, loan_id| {
                steps
                    .borrow_mut()
                    .push(format!("remove_borrower_dir:{borrower}:{node}:{loan_id}"));
                true
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_| steps.borrow_mut().push("erase_loan".to_string())
        },
        {
            let steps = Rc::clone(&steps);
            move |_| {
                steps
                    .borrow_mut()
                    .push("decrement_broker_owner_count".to_string())
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, _| {
                steps
                    .borrow_mut()
                    .push("forgive_last_broker_debt".to_string())
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |borrower_account| {
                steps
                    .borrow_mut()
                    .push(format!("decrement_borrower_owner_count:{borrower_account}"))
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, asset| {
                steps
                    .borrow_mut()
                    .push(format!("associate_loan_asset:{asset}"))
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, asset| {
                steps
                    .borrow_mut()
                    .push(format!("associate_broker_asset:{asset}"))
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, asset| {
                steps
                    .borrow_mut()
                    .push(format!("associate_vault_asset:{asset}"))
            }
        },
    );

    assert_eq!(result, protocol::Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read_borrower:borrower",
            "read_broker:broker-1",
            "read_vault:vault-1",
            "remove_broker_dir:broker-pseudo:7:loan-1",
            "remove_borrower_dir:borrower:11:loan-1",
            "erase_loan",
            "decrement_broker_owner_count",
            "forgive_last_broker_debt",
            "decrement_borrower_owner_count:borrower-account",
            "associate_loan_asset:USD",
            "associate_broker_asset:USD",
            "associate_vault_asset:USD",
        ]
    );
}

#[test]
fn loan_delete_do_apply_returns_first_bad_ledger_failure() {
    let borrower_called = Cell::new(false);

    let missing_loan = run_loan_delete_do_apply(
        &"loan-1",
        |_| None::<TestApplyLoan>,
        |_| {
            borrower_called.set(true);
            Some("borrower-account")
        },
        |_| {
            Some(TestApplyBroker {
                pseudo_account_id: "broker-pseudo",
                vault_id: "vault-1",
            })
        },
        |_| Some(TestApplyVault { asset: "USD" }),
        |_, _, _| true,
        |_, _, _| true,
        |_| {},
        |_| {},
        |_, _| {},
        |_| {},
        |_, _| {},
        |_, _| {},
        |_, _| {},
    );
    assert_eq!(missing_loan, protocol::Ter::TEF_BAD_LEDGER);
    assert!(!borrower_called.get());

    let missing_borrower = run_loan_delete_do_apply(
        &"loan-1",
        |_| {
            Some(TestApplyLoan {
                borrower: "borrower",
                loan_broker_id: "broker-1",
                loan_broker_node: 7,
                owner_node: 11,
            })
        },
        |_| None::<&'static str>,
        |_| {
            Some(TestApplyBroker {
                pseudo_account_id: "broker-pseudo",
                vault_id: "vault-1",
            })
        },
        |_| Some(TestApplyVault { asset: "USD" }),
        |_, _, _| true,
        |_, _, _| true,
        |_| {},
        |_| {},
        |_, _| {},
        |_| {},
        |_, _| {},
        |_, _| {},
        |_, _| {},
    );
    assert_eq!(missing_borrower, protocol::Ter::TEF_BAD_LEDGER);

    let missing_broker = run_loan_delete_do_apply(
        &"loan-1",
        |_| {
            Some(TestApplyLoan {
                borrower: "borrower",
                loan_broker_id: "broker-1",
                loan_broker_node: 7,
                owner_node: 11,
            })
        },
        |_| Some("borrower-account"),
        |_| None::<TestApplyBroker>,
        |_| Some(TestApplyVault { asset: "USD" }),
        |_, _, _| true,
        |_, _, _| true,
        |_| {},
        |_| {},
        |_, _| {},
        |_| {},
        |_, _| {},
        |_, _| {},
        |_, _| {},
    );
    assert_eq!(missing_broker, protocol::Ter::TEF_BAD_LEDGER);

    let missing_vault = run_loan_delete_do_apply(
        &"loan-1",
        |_| {
            Some(TestApplyLoan {
                borrower: "borrower",
                loan_broker_id: "broker-1",
                loan_broker_node: 7,
                owner_node: 11,
            })
        },
        |_| Some("borrower-account"),
        |_| {
            Some(TestApplyBroker {
                pseudo_account_id: "broker-pseudo",
                vault_id: "vault-1",
            })
        },
        |_| None::<TestApplyVault>,
        |_, _, _| true,
        |_, _, _| true,
        |_| {},
        |_| {},
        |_, _| {},
        |_| {},
        |_, _| {},
        |_, _| {},
        |_, _| {},
    );
    assert_eq!(missing_vault, protocol::Ter::TEF_BAD_LEDGER);
}

#[test]
fn loan_delete_do_apply_returns_dir_remove_failures_before_mutation_tail() {
    let borrower_dir_called = Cell::new(false);

    let broker_dir_failure = run_loan_delete_do_apply(
        &"loan-1",
        |_| {
            Some(TestApplyLoan {
                borrower: "borrower",
                loan_broker_id: "broker-1",
                loan_broker_node: 7,
                owner_node: 11,
            })
        },
        |_| Some("borrower-account"),
        |_| {
            Some(TestApplyBroker {
                pseudo_account_id: "broker-pseudo",
                vault_id: "vault-1",
            })
        },
        |_| Some(TestApplyVault { asset: "USD" }),
        |_, _, _| false,
        |_, _, _| {
            borrower_dir_called.set(true);
            true
        },
        |_| {},
        |_| {},
        |_, _| {},
        |_| {},
        |_, _| {},
        |_, _| {},
        |_, _| {},
    );
    assert_eq!(broker_dir_failure, protocol::Ter::TEF_BAD_LEDGER);
    assert!(!borrower_dir_called.get());

    let tail_called = Cell::new(false);
    let borrower_dir_failure = run_loan_delete_do_apply(
        &"loan-1",
        |_| {
            Some(TestApplyLoan {
                borrower: "borrower",
                loan_broker_id: "broker-1",
                loan_broker_node: 7,
                owner_node: 11,
            })
        },
        |_| Some("borrower-account"),
        |_| {
            Some(TestApplyBroker {
                pseudo_account_id: "broker-pseudo",
                vault_id: "vault-1",
            })
        },
        |_| Some(TestApplyVault { asset: "USD" }),
        |_, _, _| true,
        |_, _, _| false,
        |_| {
            tail_called.set(true);
        },
        |_| {},
        |_, _| {},
        |_| {},
        |_, _| {},
        |_, _| {},
        |_, _| {},
    );
    assert_eq!(borrower_dir_failure, protocol::Ter::TEF_BAD_LEDGER);
    assert!(!tail_called.get());
}

#[test]
fn loan_delete_forgive_last_broker_debt_matches_current_cpp_branching() {
    let round_called = Cell::new(false);
    let mut untouched = TestForgiveBroker {
        owner_count: 1,
        debt_total: 9,
    };
    let result = run_loan_delete_forgive_last_broker_debt(
        &mut untouched,
        &TestApplyVault { asset: "USD" },
        |_, _| {
            round_called.set(true);
            0
        },
    );
    assert_eq!(result, Ok(LoanDeleteForgiveDebtOutcome::Unchanged));
    assert_eq!(untouched.debt_total, 9);
    assert!(!round_called.get());

    let mut zero_debt = TestForgiveBroker {
        owner_count: 0,
        debt_total: 0,
    };
    let result = run_loan_delete_forgive_last_broker_debt(
        &mut zero_debt,
        &TestApplyVault { asset: "USD" },
        |_, _| unreachable!(),
    );
    assert_eq!(result, Ok(LoanDeleteForgiveDebtOutcome::Unchanged));
    assert_eq!(zero_debt.debt_total, 0);

    let mut clearable = TestForgiveBroker {
        owner_count: 0,
        debt_total: 9,
    };
    let result = run_loan_delete_forgive_last_broker_debt(
        &mut clearable,
        &TestApplyVault { asset: "USD" },
        |asset, debt| {
            assert_eq!(*asset, "USD");
            assert_eq!(*debt, 9);
            0
        },
    );
    assert_eq!(result, Ok(LoanDeleteForgiveDebtOutcome::ClearedDebt));
    assert_eq!(clearable.debt_total, 0);

    let mut invalid = TestForgiveBroker {
        owner_count: 0,
        debt_total: 9,
    };
    let result = run_loan_delete_forgive_last_broker_debt(
        &mut invalid,
        &TestApplyVault { asset: "USD" },
        |_, _| 1,
    );
    assert_eq!(
        result,
        Err(LoanDeleteForgiveDebtError::RoundedDebtMustBeZero)
    );
    assert_eq!(invalid.debt_total, 9);
}
