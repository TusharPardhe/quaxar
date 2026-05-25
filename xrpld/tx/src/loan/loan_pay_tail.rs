//! Current Rust helper mirroring the post-payment tail inside
//! the LoanPay transactor.
//!
//! This module composes the landed helpers for:
//!
//! - the vault/cover/asset mutation block, and
//! - the final auth plus send block.

use protocol::Ter;

use super::loan_pay::{
    LoanPayDoApplyBroker, LoanPayDoApplyFrontState, LoanPayDoApplyLoan, LoanPayDoApplyVault,
};
use super::loan_pay_tail_mutation::{
    LoanPayTailMutationFacts, LoanPayTailMutationSink, run_loan_pay_tail_mutation,
};
use super::loan_pay_tail_transfer::{
    LoanPayTailTransferFacts, LoanPayTailTransferSink, run_loan_pay_tail_transfer,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayDoApplyTailFacts<Amount> {
    pub zero_amount: Amount,
    pub total_paid_to_vault_rounded: Amount,
    pub total_paid_to_broker: Amount,
}

pub trait LoanPayDoApplyTailSink {
    type Loan: LoanPayDoApplyLoan;
    type Broker: LoanPayDoApplyBroker<
            AccountId = Self::AccountId,
            VaultId = Self::VaultId,
            Amount = Self::Amount,
            Asset = Self::Asset,
        >;
    type Vault: LoanPayDoApplyVault<AccountId = Self::AccountId, Asset = Self::Asset, Amount = Self::Amount>;
    type AccountId;
    type Asset;
    type Amount;
    type VaultId;

    fn update_vault(&mut self, vault: &Self::Vault);
    fn require_auth(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> Ter;
    fn broker_payee_balance_for_empty_holding(&mut self, account: &Self::AccountId)
    -> Self::Amount;
    fn add_empty_holding(
        &mut self,
        account: &Self::AccountId,
        balance: &Self::Amount,
        asset: &Self::Asset,
    ) -> Ter;
    fn account_send_multi(
        &mut self,
        source: &Self::AccountId,
        asset: &Self::Asset,
        outputs: [(Self::AccountId, Self::Amount); 2],
    ) -> Ter;
}

pub fn run_loan_pay_do_apply_tail<Sink>(
    sink: &mut Sink,
    account: &Sink::AccountId,
    state: &mut LoanPayDoApplyFrontState<
        Sink::Loan,
        Sink::Broker,
        Sink::Vault,
        Sink::AccountId,
        Sink::Asset,
        Sink::Amount,
    >,
    facts: LoanPayDoApplyTailFacts<Sink::Amount>,
) -> Ter
where
    Sink: LoanPayDoApplyTailSink,
    Sink::Loan: LoanPayDoApplyLoan<Asset = Sink::Asset>,
    Sink::Broker: LoanPayDoApplyBroker<
            AccountId = Sink::AccountId,
            VaultId = Sink::VaultId,
            Amount = Sink::Amount,
            Asset = Sink::Asset,
        >,
    Sink::Vault: LoanPayDoApplyVault<AccountId = Sink::AccountId, Asset = Sink::Asset, Amount = Sink::Amount>,
    Sink::AccountId: Clone + PartialEq,
    Sink::Asset: Clone,
    Sink::Amount: Clone + PartialEq + PartialOrd,
{
    let mutation = run_loan_pay_tail_mutation(
        sink,
        &mut state.loan,
        &mut state.broker,
        &mut state.vault,
        LoanPayTailMutationFacts {
            asset: state.asset.clone(),
            payment_value_change: state.payment_parts.value_change.clone(),
            total_paid_to_vault_rounded: facts.total_paid_to_vault_rounded.clone(),
            total_paid_to_broker: facts.total_paid_to_broker.clone(),
            send_broker_fee_to_owner: state.send_broker_fee_to_owner,
        },
    );
    if mutation != Ter::TES_SUCCESS {
        return mutation;
    }

    run_loan_pay_tail_transfer(
        sink,
        LoanPayTailTransferFacts {
            account: account.clone(),
            vault_pseudo_account: state.vault.pseudo_account().clone(),
            broker_payee: state.broker_payee.clone(),
            asset: state.asset.clone(),
            zero_amount: facts.zero_amount,
            total_paid_to_vault_rounded: facts.total_paid_to_vault_rounded,
            total_paid_to_broker: facts.total_paid_to_broker,
        },
    )
}

impl<Sink> LoanPayTailMutationSink for Sink
where
    Sink: LoanPayDoApplyTailSink,
{
    type Vault = Sink::Vault;

    fn update_vault(&mut self, vault: &Self::Vault) {
        LoanPayDoApplyTailSink::update_vault(self, vault);
    }
}

impl<Sink> LoanPayTailTransferSink for Sink
where
    Sink: LoanPayDoApplyTailSink,
{
    type AccountId = Sink::AccountId;
    type Asset = Sink::Asset;
    type Amount = Sink::Amount;

    fn require_auth(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> Ter {
        LoanPayDoApplyTailSink::require_auth(self, account, asset)
    }

    fn broker_payee_balance_for_empty_holding(
        &mut self,
        account: &Self::AccountId,
    ) -> Self::Amount {
        LoanPayDoApplyTailSink::broker_payee_balance_for_empty_holding(self, account)
    }

    fn add_empty_holding(
        &mut self,
        account: &Self::AccountId,
        balance: &Self::Amount,
        asset: &Self::Asset,
    ) -> Ter {
        LoanPayDoApplyTailSink::add_empty_holding(self, account, balance, asset)
    }

    fn account_send_multi(
        &mut self,
        source: &Self::AccountId,
        asset: &Self::Asset,
        outputs: [(Self::AccountId, Self::Amount); 2],
    ) -> Ter {
        LoanPayDoApplyTailSink::account_send_multi(self, source, asset, outputs)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use protocol::Ter;

    use super::super::loan_pay::{
        LoanPayDoApplyBroker, LoanPayDoApplyFrontState, LoanPayDoApplyLoan, LoanPayDoApplyVault,
        LoanPayPaymentParts, LoanPayPaymentType,
    };
    use super::{LoanPayDoApplyTailFacts, LoanPayDoApplyTailSink, run_loan_pay_do_apply_tail};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestLoan {
        broker_id: &'static str,
        impaired: bool,
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl LoanPayDoApplyLoan for TestLoan {
        type BrokerId = &'static str;
        type Asset = &'static str;

        fn broker_id(&self) -> &Self::BrokerId {
            &self.broker_id
        }

        fn scale(&self) -> i32 {
            0
        }

        fn is_impaired(&self) -> bool {
            self.impaired
        }

        fn associate_asset(&mut self, _asset: &Self::Asset) {
            self.steps.borrow_mut().push("associate_loan_asset");
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBroker {
        owner: &'static str,
        pseudo_account: &'static str,
        vault_id: &'static str,
        debt_total: i64,
        cover_available: i64,
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl LoanPayDoApplyBroker for TestBroker {
        type AccountId = &'static str;
        type VaultId = &'static str;
        type Amount = i64;
        type Asset = &'static str;

        fn owner(&self) -> &Self::AccountId {
            &self.owner
        }

        fn pseudo_account(&self) -> &Self::AccountId {
            &self.pseudo_account
        }

        fn vault_id(&self) -> &Self::VaultId {
            &self.vault_id
        }

        fn cover_available(&self) -> &Self::Amount {
            &self.cover_available
        }

        fn debt_total(&self) -> &Self::Amount {
            &self.debt_total
        }

        fn cover_rate_minimum(&self) -> u32 {
            0
        }

        fn add_cover_available(&mut self, amount: Self::Amount) {
            self.steps.borrow_mut().push("add_cover_available");
            self.cover_available += amount;
        }

        fn adjust_debt_total(&mut self, amount: Self::Amount) {
            self.steps.borrow_mut().push("adjust_debt_total");
            self.debt_total -= amount;
        }

        fn associate_asset(&mut self, _asset: &Self::Asset) {
            self.steps.borrow_mut().push("associate_broker_asset");
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_account: &'static str,
        asset: &'static str,
        assets_available: i64,
        assets_total: i64,
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl LoanPayDoApplyVault for TestVault {
        type AccountId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn pseudo_account(&self) -> &Self::AccountId {
            &self.pseudo_account
        }

        fn asset(&self) -> &Self::Asset {
            &self.asset
        }

        fn assets_available(&self) -> &Self::Amount {
            &self.assets_available
        }

        fn assets_total(&self) -> &Self::Amount {
            &self.assets_total
        }

        fn add_assets_available(&mut self, amount: Self::Amount) {
            self.steps.borrow_mut().push("add_assets_available");
            self.assets_available += amount;
        }

        fn add_assets_total(&mut self, amount: Self::Amount) {
            self.steps.borrow_mut().push("add_assets_total");
            self.assets_total += amount;
        }

        fn assets_available_exceeds_total(&self) -> bool {
            self.assets_available > self.assets_total
        }

        fn associate_asset(&mut self, _asset: &Self::Asset) {
            self.steps.borrow_mut().push("associate_vault_asset");
        }
    }

    struct TestSink {
        steps: Rc<RefCell<Vec<&'static str>>>,
        vault_auth_result: Ter,
        broker_auth_result: Ter,
        add_empty_holding_result: Ter,
        send_multi_result: Ter,
        expected_vault_amount: i64,
        expected_broker_payee: &'static str,
        expected_broker_amount: i64,
    }

    impl LoanPayDoApplyTailSink for TestSink {
        type Loan = TestLoan;
        type Broker = TestBroker;
        type Vault = TestVault;
        type AccountId = &'static str;
        type Asset = &'static str;
        type Amount = i64;
        type VaultId = &'static str;

        fn update_vault(&mut self, _vault: &Self::Vault) {
            self.steps.borrow_mut().push("update_vault");
        }

        fn require_auth(&mut self, account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            self.steps.borrow_mut().push(*account);
            if *account == "vault-pseudo" {
                self.vault_auth_result
            } else {
                self.broker_auth_result
            }
        }

        fn broker_payee_balance_for_empty_holding(
            &mut self,
            account: &Self::AccountId,
        ) -> Self::Amount {
            self.steps.borrow_mut().push("broker_payee_balance");
            assert_eq!(*account, "borrower");
            12
        }

        fn add_empty_holding(
            &mut self,
            account: &Self::AccountId,
            balance: &Self::Amount,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.borrow_mut().push("add_empty_holding");
            assert_eq!(*account, "borrower");
            assert_eq!(*balance, 12);
            self.add_empty_holding_result
        }

        fn account_send_multi(
            &mut self,
            source: &Self::AccountId,
            asset: &Self::Asset,
            outputs: [(Self::AccountId, Self::Amount); 2],
        ) -> Ter {
            self.steps.borrow_mut().push("account_send_multi");
            assert_eq!(*source, "borrower");
            assert_eq!(*asset, "USD");
            assert_eq!(outputs[0], ("vault-pseudo", self.expected_vault_amount));
            assert_eq!(
                outputs[1],
                (self.expected_broker_payee, self.expected_broker_amount)
            );
            self.send_multi_result
        }
    }

    fn build_state(
        send_broker_fee_to_owner: bool,
        broker_payee: &'static str,
        vault_available: i64,
        vault_total: i64,
    ) -> LoanPayDoApplyFrontState<TestLoan, TestBroker, TestVault, &'static str, &'static str, i64>
    {
        LoanPayDoApplyFrontState {
            loan: TestLoan {
                broker_id: "broker",
                impaired: false,
                steps: Rc::new(RefCell::new(Vec::new())),
            },
            broker: TestBroker {
                owner: "owner",
                pseudo_account: "broker-pseudo",
                vault_id: "vault",
                debt_total: 50,
                cover_available: 20,
                steps: Rc::new(RefCell::new(Vec::new())),
            },
            vault: TestVault {
                pseudo_account: "vault-pseudo",
                asset: "USD",
                assets_available: vault_available,
                assets_total: vault_total,
                steps: Rc::new(RefCell::new(Vec::new())),
            },
            asset: "USD",
            broker_payee,
            send_broker_fee_to_owner,
            payment_type: LoanPayPaymentType::Full,
            payment_parts: LoanPayPaymentParts {
                principal_paid: 10,
                interest_paid: 3,
                fee_paid: 3,
                value_change: 2,
            },
        }
    }

    #[test]
    fn loan_pay_do_apply_tail_returns_tec_internal_when_vault_assets_exceed_total() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut state = build_state(true, "owner", 10, 10);
        state.loan.steps = Rc::clone(&steps);
        state.broker.steps = Rc::clone(&steps);
        state.vault.steps = Rc::clone(&steps);
        state.payment_parts.value_change = 0;

        let mut sink = TestSink {
            steps: Rc::clone(&steps),
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TES_SUCCESS,
            send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 7,
            expected_broker_payee: "owner",
            expected_broker_amount: 3,
        };

        let result = run_loan_pay_do_apply_tail(
            &mut sink,
            &"borrower",
            &mut state,
            LoanPayDoApplyTailFacts {
                zero_amount: 0,
                total_paid_to_vault_rounded: 1,
                total_paid_to_broker: 0,
            },
        );

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(
            steps.borrow().as_slice(),
            &["update_vault", "add_assets_available", "add_assets_total",]
        );
    }

    #[test]
    fn loan_pay_do_apply_tail_runs_current_on_fallback_fee_with_duplicate_holding() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut state = build_state(false, "borrower", 80, 100);
        state.loan.steps = Rc::clone(&steps);
        state.broker.steps = Rc::clone(&steps);
        state.vault.steps = Rc::clone(&steps);

        let mut sink = TestSink {
            steps: Rc::clone(&steps),
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TEC_DUPLICATE,
            send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 7,
            expected_broker_payee: "borrower",
            expected_broker_amount: 3,
        };

        let result = run_loan_pay_do_apply_tail(
            &mut sink,
            &"borrower",
            &mut state,
            LoanPayDoApplyTailFacts {
                zero_amount: 0,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            &[
                "update_vault",
                "add_assets_available",
                "add_assets_total",
                "add_cover_available",
                "associate_loan_asset",
                "associate_broker_asset",
                "associate_vault_asset",
                "vault-pseudo",
                "broker_payee_balance",
                "add_empty_holding",
                "borrower",
                "account_send_multi",
            ]
        );
        assert_eq!(state.broker.debt_total, 50);
        assert_eq!(state.broker.cover_available, 23);
        assert_eq!(state.vault.assets_available, 87);
        assert_eq!(state.vault.assets_total, 102);
    }

    #[test]
    fn loan_pay_do_apply_tail_returns_non_duplicate_holding_errors() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut state = build_state(false, "borrower", 80, 100);
        state.loan.steps = Rc::clone(&steps);
        state.broker.steps = Rc::clone(&steps);
        state.vault.steps = Rc::clone(&steps);

        let mut sink = TestSink {
            steps: Rc::clone(&steps),
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TEC_PATH_DRY,
            send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 7,
            expected_broker_payee: "borrower",
            expected_broker_amount: 3,
        };

        let result = run_loan_pay_do_apply_tail(
            &mut sink,
            &"borrower",
            &mut state,
            LoanPayDoApplyTailFacts {
                zero_amount: 0,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
            },
        );

        assert_eq!(result, Ter::TEC_PATH_DRY);
        assert_eq!(
            steps.borrow().as_slice(),
            &[
                "update_vault",
                "add_assets_available",
                "add_assets_total",
                "add_cover_available",
                "associate_loan_asset",
                "associate_broker_asset",
                "associate_vault_asset",
                "vault-pseudo",
                "broker_payee_balance",
                "add_empty_holding",
            ]
        );
    }

    #[test]
    fn loan_pay_do_apply_tail_skips_zero_value_auth_and_transfer_helpers() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut state = build_state(true, "owner", 30, 40);
        state.loan.steps = Rc::clone(&steps);
        state.broker.steps = Rc::clone(&steps);
        state.vault.steps = Rc::clone(&steps);

        let mut sink = TestSink {
            steps: Rc::clone(&steps),
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TES_SUCCESS,
            send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 0,
            expected_broker_payee: "owner",
            expected_broker_amount: 0,
        };

        let result = run_loan_pay_do_apply_tail(
            &mut sink,
            &"borrower",
            &mut state,
            LoanPayDoApplyTailFacts {
                zero_amount: 0,
                total_paid_to_vault_rounded: 0,
                total_paid_to_broker: 0,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            &[
                "update_vault",
                "add_assets_available",
                "add_assets_total",
                "associate_loan_asset",
                "associate_broker_asset",
                "associate_vault_asset",
                "account_send_multi",
            ]
        );
    }
}
