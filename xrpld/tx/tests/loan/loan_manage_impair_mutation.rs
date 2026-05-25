//! Integration tests that pin the narrowed Rust mutation/update shell inside
//! `LoanManage.cpp` `impairLoan(...)` and `unimpairLoan(...)`.

use std::{cell::RefCell, rc::Rc};

use protocol::Ter;
use tx::loan_manage_impair::{LoanManageImpairOutcome, LoanManageUnimpairOutcome};
use tx::loan_manage_impair_mutation::{
    LoanManageImpairMutationLoan, LoanManageImpairMutationSink, LoanManageImpairMutationVault,
    run_loan_manage_impair_mutation, run_loan_manage_unimpair_mutation,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestLoan {
    impaired: bool,
    next_payment_due_date: i64,
    steps: Rc<RefCell<Vec<&'static str>>>,
}

impl LoanManageImpairMutationLoan<i64> for TestLoan {
    fn set_impaired(&mut self) {
        self.steps.borrow_mut().push("set_impaired");
        self.impaired = true;
    }

    fn clear_impaired(&mut self) {
        self.steps.borrow_mut().push("clear_impaired");
        self.impaired = false;
    }

    fn set_next_payment_due_date(&mut self, due_date: i64) {
        self.steps.borrow_mut().push("set_next_payment_due_date");
        self.next_payment_due_date = due_date;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestVault {
    loss_unrealized: i64,
    steps: Rc<RefCell<Vec<&'static str>>>,
}

impl LoanManageImpairMutationVault<i64> for TestVault {
    fn set_loss_unrealized(&mut self, loss_unrealized: i64) {
        self.steps.borrow_mut().push("set_loss_unrealized");
        self.loss_unrealized = loss_unrealized;
    }
}

struct TestSink {
    steps: Rc<RefCell<Vec<&'static str>>>,
}

impl LoanManageImpairMutationSink for TestSink {
    type Loan = TestLoan;
    type Vault = TestVault;

    fn update_vault(&mut self, _vault: &Self::Vault) {
        self.steps.borrow_mut().push("update_vault");
    }

    fn update_loan(&mut self, _loan: &Self::Loan) {
        self.steps.borrow_mut().push("update_loan");
    }
}

#[test]
fn tx_loan_manage_impair_mutation_updates_vault_before_loan() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = TestSink {
        steps: Rc::clone(&steps),
    };
    let mut loan = TestLoan {
        impaired: false,
        next_payment_due_date: 10,
        steps: Rc::clone(&steps),
    };

    let result = run_loan_manage_impair_mutation(
        &mut sink,
        &mut loan,
        &mut TestVault {
            loss_unrealized: 10,
            steps: Rc::clone(&steps),
        },
        LoanManageImpairOutcome {
            loss_unrealized: 100_i64,
            vault_loss_unrealized: 110_i64,
            loan_is_impaired: true,
            loan_next_payment_due_date: 40_i64,
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(loan.impaired);
    assert_eq!(loan.next_payment_due_date, 40_i64);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "set_loss_unrealized",
            "update_vault",
            "set_impaired",
            "set_next_payment_due_date",
            "update_loan",
        ]
    );
}

#[test]
fn tx_loan_manage_unimpair_mutation_clears_flag_before_loan_update() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = TestSink {
        steps: Rc::clone(&steps),
    };
    let mut loan = TestLoan {
        impaired: true,
        next_payment_due_date: 10,
        steps: Rc::clone(&steps),
    };

    let result = run_loan_manage_unimpair_mutation(
        &mut sink,
        &mut loan,
        &mut TestVault {
            loss_unrealized: 150,
            steps: Rc::clone(&steps),
        },
        LoanManageUnimpairOutcome {
            loss_reversed: 100_i64,
            vault_loss_unrealized: 50_i64,
            loan_is_impaired: false,
            loan_next_payment_due_date: 70_i64,
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(!loan.impaired);
    assert_eq!(loan.next_payment_due_date, 70_i64);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "set_loss_unrealized",
            "update_vault",
            "clear_impaired",
            "set_next_payment_due_date",
            "update_loan",
        ]
    );
}
