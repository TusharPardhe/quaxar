//! Integration tests that pin the caller-owned mutation tail for
//! `LoanManage::defaultLoan(...)` to the current C++ update order.

use std::{cell::RefCell, rc::Rc};

use protocol::Ter;

use tx::loan_manage_default::run_loan_manage_default;
use tx::loan_manage_default::{
    LoanManageDefaultFacts, LoanManageDefaultMath, LoanManageDefaultRoundingMode,
};
use tx::loan_manage_default_mutation::{
    LoanManageDefaultMutationBroker, LoanManageDefaultMutationFacts, LoanManageDefaultMutationLoan,
    LoanManageDefaultMutationPlan, LoanManageDefaultMutationSink, LoanManageDefaultMutationVault,
    run_loan_manage_default_mutation,
};

#[derive(Default)]
struct TestMath;

impl LoanManageDefaultMath for TestMath {
    type Amount = i64;
    type Asset = &'static str;

    fn tenth_bips_of_value(&mut self, value: Self::Amount, rate: u32) -> Self::Amount {
        value * rate as i64 / 10_000
    }

    fn round_to_asset(
        &mut self,
        _: &Self::Asset,
        value: Self::Amount,
        _: i32,
        mode: LoanManageDefaultRoundingMode,
    ) -> Self::Amount {
        match mode {
            LoanManageDefaultRoundingMode::Upward => value,
            LoanManageDefaultRoundingMode::Downward => value,
        }
    }

    fn asset_is_integral(&mut self, _: &Self::Asset) -> bool {
        true
    }

    fn exponent(&mut self, _: Self::Amount) -> i32 {
        0
    }

    fn adjust_imprecise_subtract(
        &mut self,
        _: &Self::Asset,
        value: Self::Amount,
        decrement: Self::Amount,
        _: i32,
    ) -> Self::Amount {
        (value - decrement).max(0)
    }
}

#[derive(Clone)]
struct TestLoan {
    steps: Rc<RefCell<Vec<&'static str>>>,
}

impl LoanManageDefaultMutationLoan for TestLoan {
    type Amount = i64;
    type Time = i64;

    fn set_defaulted(&mut self) {
        self.steps.borrow_mut().push("loan_defaulted");
    }

    fn set_total_value_outstanding(&mut self, _: Self::Amount) {
        self.steps.borrow_mut().push("loan_total_zero");
    }

    fn set_payment_remaining(&mut self, _: Self::Amount) {
        self.steps.borrow_mut().push("loan_payment_zero");
    }

    fn set_principal_outstanding(&mut self, _: Self::Amount) {
        self.steps.borrow_mut().push("loan_principal_zero");
    }

    fn set_management_fee_outstanding(&mut self, _: Self::Amount) {
        self.steps.borrow_mut().push("loan_fee_zero");
    }

    fn set_next_payment_due_date(&mut self, _: Self::Time) {
        self.steps.borrow_mut().push("loan_next_due_zero");
    }
}

#[derive(Clone)]
struct TestBroker {
    account: &'static str,
    steps: Rc<RefCell<Vec<&'static str>>>,
}

impl LoanManageDefaultMutationBroker for TestBroker {
    type AccountId = &'static str;
    type Amount = i64;

    fn account_id(&self) -> &Self::AccountId {
        &self.account
    }

    fn set_debt_total(&mut self, _: Self::Amount) {
        self.steps.borrow_mut().push("broker_debt_zero");
    }

    fn set_cover_available(&mut self, _: Self::Amount) {
        self.steps.borrow_mut().push("broker_cover_zero");
    }
}

#[derive(Clone)]
struct TestVault {
    account: &'static str,
    steps: Rc<RefCell<Vec<&'static str>>>,
}

impl LoanManageDefaultMutationVault for TestVault {
    type AccountId = &'static str;
    type Amount = i64;
    type Asset = &'static str;

    fn account_id(&self) -> &Self::AccountId {
        &self.account
    }

    fn set_assets_total(&mut self, _: Self::Amount) {
        self.steps.borrow_mut().push("vault_total_set");
    }

    fn set_assets_available(&mut self, _: Self::Amount) {
        self.steps.borrow_mut().push("vault_available_set");
    }

    fn set_loss_unrealized(&mut self, _: Self::Amount) {
        self.steps.borrow_mut().push("vault_loss_set");
    }
}

struct TestSink {
    steps: Rc<RefCell<Vec<&'static str>>>,
    send_result: Ter,
}

impl LoanManageDefaultMutationSink for TestSink {
    type AccountId = &'static str;
    type Asset = &'static str;
    type Amount = i64;

    fn update_vault(&mut self) {
        self.steps.borrow_mut().push("update_vault");
    }

    fn update_broker(&mut self) {
        self.steps.borrow_mut().push("update_broker");
    }

    fn update_loan(&mut self) {
        self.steps.borrow_mut().push("update_loan");
    }

    fn account_send(
        &mut self,
        source: &Self::AccountId,
        destination: &Self::AccountId,
        asset: &Self::Asset,
        amount: &Self::Amount,
        fee_waived: bool,
    ) -> Ter {
        self.steps.borrow_mut().push("account_send");
        assert_eq!(*source, "broker-account");
        assert_eq!(*destination, "vault-account");
        assert_eq!(*asset, "USD");
        assert_eq!(*amount, 7);
        assert!(fee_waived);
        self.send_result
    }
}

#[test]
fn tx_loan_manage_default_mutation_updates_vault_before_broker_before_loan() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = TestSink {
        steps: Rc::clone(&steps),
        send_result: Ter::TES_SUCCESS,
    };
    let mut loan = TestLoan {
        steps: Rc::clone(&steps),
    };
    let mut broker = TestBroker {
        account: "broker-account",
        steps: Rc::clone(&steps),
    };
    let mut vault = TestVault {
        account: "vault-account",
        steps: Rc::clone(&steps),
    };

    let mut math = TestMath;
    let plan = run_loan_manage_default(
        LoanManageDefaultFacts {
            asset: "USD",
            loan_scale: 0,
            vault_scale: 0,
            total_value_outstanding: 100,
            management_fee_outstanding: 20,
            broker_debt_total: 70000,
            cover_rate_minimum: 100,
            cover_rate_liquidation: 100,
            cover_available: 7,
            vault_total_assets: 100,
            vault_available_assets: 0,
            vault_loss_unrealized: 100,
            loan_is_impaired: true,
        },
        &mut math,
    )
    .expect("plan");

    let result = run_loan_manage_default_mutation(
        &mut sink,
        &mut loan,
        &mut broker,
        &mut vault,
        &"USD",
        LoanManageDefaultMutationPlan {
            default_covered: plan.default_covered,
            broker_debt_after: plan.broker_debt_after,
            cover_available_after: plan.cover_available_after,
            vault_total_after: plan.vault_total_after,
            vault_available_after: plan.vault_available_after,
            vault_loss_unrealized_after: plan.vault_loss_unrealized_after,
        },
        LoanManageDefaultMutationFacts {
            loan_is_impaired: true,
            fee_waived: true,
            zero_amount: 0,
            zero_time: 0,
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "vault_total_set",
            "vault_available_set",
            "vault_loss_set",
            "update_vault",
            "broker_debt_zero",
            "broker_cover_zero",
            "update_broker",
            "loan_defaulted",
            "loan_total_zero",
            "loan_payment_zero",
            "loan_principal_zero",
            "loan_fee_zero",
            "loan_next_due_zero",
            "update_loan",
            "account_send",
        ]
    );
}

#[test]
fn tx_loan_manage_default_mutation_skips_loss_realization_when_not_impaired() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = TestSink {
        steps: Rc::clone(&steps),
        send_result: Ter::TEC_NO_PERMISSION,
    };
    let mut loan = TestLoan {
        steps: Rc::clone(&steps),
    };
    let mut broker = TestBroker {
        account: "broker-account",
        steps: Rc::clone(&steps),
    };
    let mut vault = TestVault {
        account: "vault-account",
        steps: Rc::clone(&steps),
    };

    let mut math = TestMath;
    let plan = run_loan_manage_default(
        LoanManageDefaultFacts {
            asset: "USD",
            loan_scale: 0,
            vault_scale: 0,
            total_value_outstanding: 100,
            management_fee_outstanding: 20,
            broker_debt_total: 70000,
            cover_rate_minimum: 100,
            cover_rate_liquidation: 100,
            cover_available: 7,
            vault_total_assets: 100,
            vault_available_assets: 0,
            vault_loss_unrealized: 60,
            loan_is_impaired: false,
        },
        &mut math,
    )
    .expect("plan");

    let result = run_loan_manage_default_mutation(
        &mut sink,
        &mut loan,
        &mut broker,
        &mut vault,
        &"USD",
        LoanManageDefaultMutationPlan {
            default_covered: plan.default_covered,
            broker_debt_after: plan.broker_debt_after,
            cover_available_after: plan.cover_available_after,
            vault_total_after: plan.vault_total_after,
            vault_available_after: plan.vault_available_after,
            vault_loss_unrealized_after: plan.vault_loss_unrealized_after,
        },
        LoanManageDefaultMutationFacts {
            loan_is_impaired: false,
            fee_waived: true,
            zero_amount: 0,
            zero_time: 0,
        },
    );

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "vault_total_set",
            "vault_available_set",
            "update_vault",
            "broker_debt_zero",
            "broker_cover_zero",
            "update_broker",
            "loan_defaulted",
            "loan_total_zero",
            "loan_payment_zero",
            "loan_principal_zero",
            "loan_fee_zero",
            "loan_next_due_zero",
            "update_loan",
            "account_send",
        ]
    );
}
