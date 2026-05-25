//! Mutation tail of `LoanManage::defaultLoan(...)`.
//!
//! This models the deterministic update order around:
//!
//! - the impaired-loan loss-realization branch,
//! - vault update before broker update before loan update,
//! - zeroing the loan fields including `NextPaymentDueDate`,
//! - and the final broker->vault account-send tail with transfer fee waived.

use protocol::Ter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanManageDefaultMutationPlan<Amount> {
    pub default_covered: Amount,
    pub broker_debt_after: Amount,
    pub cover_available_after: Amount,
    pub vault_total_after: Amount,
    pub vault_available_after: Amount,
    pub vault_loss_unrealized_after: Option<Amount>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoanManageDefaultMutationFacts<Amount, Time> {
    pub loan_is_impaired: bool,
    pub fee_waived: bool,
    pub zero_amount: Amount,
    pub zero_time: Time,
}

pub trait LoanManageDefaultMutationLoan {
    type Amount;
    type Time;

    fn set_defaulted(&mut self);
    fn set_total_value_outstanding(&mut self, amount: Self::Amount);
    fn set_payment_remaining(&mut self, amount: Self::Amount);
    fn set_principal_outstanding(&mut self, amount: Self::Amount);
    fn set_management_fee_outstanding(&mut self, amount: Self::Amount);
    fn set_next_payment_due_date(&mut self, amount: Self::Time);
}

pub trait LoanManageDefaultMutationBroker {
    type AccountId;
    type Amount;

    fn account_id(&self) -> &Self::AccountId;
    fn set_debt_total(&mut self, amount: Self::Amount);
    fn set_cover_available(&mut self, amount: Self::Amount);
}

pub trait LoanManageDefaultMutationVault {
    type AccountId;
    type Amount;
    type Asset;

    fn account_id(&self) -> &Self::AccountId;
    fn set_assets_total(&mut self, amount: Self::Amount);
    fn set_assets_available(&mut self, amount: Self::Amount);
    fn set_loss_unrealized(&mut self, amount: Self::Amount);
}

pub trait LoanManageDefaultMutationSink {
    type AccountId;
    type Asset;
    type Amount;

    fn update_vault(&mut self);
    fn update_broker(&mut self);
    fn update_loan(&mut self);
    fn account_send(
        &mut self,
        source: &Self::AccountId,
        destination: &Self::AccountId,
        asset: &Self::Asset,
        amount: &Self::Amount,
        fee_waived: bool,
    ) -> Ter;
}

pub fn run_loan_manage_default_mutation<Loan, Broker, Vault, Sink>(
    sink: &mut Sink,
    loan: &mut Loan,
    broker: &mut Broker,
    vault: &mut Vault,
    asset: &Sink::Asset,
    plan: LoanManageDefaultMutationPlan<Sink::Amount>,
    facts: LoanManageDefaultMutationFacts<Sink::Amount, Loan::Time>,
) -> Ter
where
    Loan: LoanManageDefaultMutationLoan<Amount = Sink::Amount>,
    Broker: LoanManageDefaultMutationBroker<AccountId = Sink::AccountId, Amount = Sink::Amount>,
    Vault: LoanManageDefaultMutationVault<
            AccountId = Sink::AccountId,
            Amount = Sink::Amount,
            Asset = Sink::Asset,
        >,
    Sink: LoanManageDefaultMutationSink,
    Sink::Amount: Clone,
    Sink::AccountId: Clone,
    Sink::Asset: Clone,
{
    vault.set_assets_total(plan.vault_total_after.clone());
    vault.set_assets_available(plan.vault_available_after.clone());
    if facts.loan_is_impaired
        && let Some(loss_unrealized_after) = plan.vault_loss_unrealized_after.clone()
    {
        vault.set_loss_unrealized(loss_unrealized_after);
    }
    sink.update_vault();

    broker.set_debt_total(plan.broker_debt_after.clone());
    broker.set_cover_available(plan.cover_available_after.clone());
    sink.update_broker();

    loan.set_defaulted();
    loan.set_total_value_outstanding(facts.zero_amount.clone());
    loan.set_payment_remaining(facts.zero_amount.clone());
    loan.set_principal_outstanding(facts.zero_amount.clone());
    loan.set_management_fee_outstanding(facts.zero_amount);
    loan.set_next_payment_due_date(facts.zero_time);
    sink.update_loan();

    sink.account_send(
        broker.account_id(),
        vault.account_id(),
        asset,
        &plan.default_covered,
        facts.fee_waived,
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use protocol::Ter;

    use super::{
        LoanManageDefaultMutationBroker, LoanManageDefaultMutationFacts,
        LoanManageDefaultMutationLoan, LoanManageDefaultMutationPlan,
        LoanManageDefaultMutationSink, LoanManageDefaultMutationVault,
        run_loan_manage_default_mutation,
    };

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
    fn loan_manage_default_mutation_updates_vault_before_broker_before_loan() {
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

        let result = run_loan_manage_default_mutation(
            &mut sink,
            &mut loan,
            &mut broker,
            &mut vault,
            &"USD",
            LoanManageDefaultMutationPlan {
                default_covered: 7,
                broker_debt_after: 93,
                cover_available_after: 5,
                vault_total_after: 88,
                vault_available_after: 12,
                vault_loss_unrealized_after: Some(61),
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
    fn loan_manage_default_mutation_skips_loss_realization_when_not_impaired() {
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

        let result = run_loan_manage_default_mutation(
            &mut sink,
            &mut loan,
            &mut broker,
            &mut vault,
            &"USD",
            LoanManageDefaultMutationPlan {
                default_covered: 7,
                broker_debt_after: 93,
                cover_available_after: 5,
                vault_total_after: 88,
                vault_available_after: 12,
                vault_loss_unrealized_after: None,
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
}
