//! Current Rust helper mirroring the final auth and send block inside
//! the LoanPay transactor.
//!
//! This module preserves the deterministic sequence around:
//!
//! - vault pseudo-account `requireAuth(...)` when the vault payment is
//!   non-zero,
//! - borrower broker-payee `addEmptyHolding(...)` with `tecDUPLICATE`
//!   tolerated as success,
//! - broker-payee `requireAuth(...)` when the broker fee is non-zero, and
//! - the final `accountSendMulti(...)` passthrough.

use protocol::{Ter, is_tes_success};

use crate::{LoanPayTransferPrepFacts, compute_loan_pay_transfer_prep_facts};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayTailTransferFacts<AccountId, Asset, Amount> {
    pub account: AccountId,
    pub vault_pseudo_account: AccountId,
    pub broker_payee: AccountId,
    pub asset: Asset,
    pub zero_amount: Amount,
    pub total_paid_to_vault_rounded: Amount,
    pub total_paid_to_broker: Amount,
}

pub trait LoanPayTailTransferSink {
    type AccountId;
    type Asset;
    type Amount;

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

pub fn run_loan_pay_tail_transfer<Sink>(
    sink: &mut Sink,
    facts: LoanPayTailTransferFacts<Sink::AccountId, Sink::Asset, Sink::Amount>,
) -> Ter
where
    Sink: LoanPayTailTransferSink,
    Sink::AccountId: Clone + PartialEq,
    Sink::Amount: Clone + PartialEq,
{
    let transfer_prep = compute_loan_pay_transfer_prep_facts(
        &facts.total_paid_to_vault_rounded,
        &facts.total_paid_to_broker,
        &facts.zero_amount,
        facts.broker_payee == facts.account,
    );

    run_loan_pay_tail_transfer_with_prep(sink, facts, transfer_prep)
}

pub fn run_loan_pay_tail_transfer_with_prep<Sink>(
    sink: &mut Sink,
    facts: LoanPayTailTransferFacts<Sink::AccountId, Sink::Asset, Sink::Amount>,
    transfer_prep: LoanPayTransferPrepFacts<Sink::Amount>,
) -> Ter
where
    Sink: LoanPayTailTransferSink,
    Sink::AccountId: Clone + PartialEq,
    Sink::Amount: Clone + PartialEq,
{
    if transfer_prep.vault_auth_required {
        let ter = sink.require_auth(&facts.vault_pseudo_account, &facts.asset);
        if !is_tes_success(ter) {
            return ter;
        }
    }

    if transfer_prep.broker_payment_present {
        if transfer_prep.add_empty_holding_required {
            let broker_payee_balance =
                sink.broker_payee_balance_for_empty_holding(&facts.broker_payee);
            let ter =
                sink.add_empty_holding(&facts.broker_payee, &broker_payee_balance, &facts.asset);
            if !is_tes_success(ter) && ter != Ter::TEC_DUPLICATE {
                return ter;
            }
        }

        let ter = sink.require_auth(&facts.broker_payee, &facts.asset);
        if !is_tes_success(ter) {
            return ter;
        }
    }

    sink.account_send_multi(
        &facts.account,
        &facts.asset,
        [
            (
                facts.vault_pseudo_account,
                facts.total_paid_to_vault_rounded,
            ),
            (facts.broker_payee, facts.total_paid_to_broker),
        ],
    )
}

#[cfg(test)]
mod tests {
    use protocol::Ter;

    use super::{LoanPayTailTransferFacts, LoanPayTailTransferSink, run_loan_pay_tail_transfer};

    struct TestSink<'a> {
        steps: &'a mut Vec<&'static str>,
        vault_auth_result: Ter,
        broker_auth_result: Ter,
        add_empty_holding_result: Ter,
        account_send_multi_result: Ter,
        expected_vault_amount: i64,
        expected_broker_payee: &'static str,
        expected_broker_amount: i64,
    }

    impl LoanPayTailTransferSink for TestSink<'_> {
        type AccountId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn require_auth(&mut self, account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            self.steps.push(*account);
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
            self.steps.push("broker_payee_balance");
            assert_eq!(*account, "borrower");
            12
        }

        fn add_empty_holding(
            &mut self,
            account: &Self::AccountId,
            balance: &Self::Amount,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.push("add_empty_holding");
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
            self.steps.push("account_send_multi");
            assert_eq!(*source, "borrower");
            assert_eq!(*asset, "USD");
            assert_eq!(outputs[0], ("vault-pseudo", self.expected_vault_amount));
            assert_eq!(
                outputs[1],
                (self.expected_broker_payee, self.expected_broker_amount)
            );
            self.account_send_multi_result
        }
    }

    #[test]
    fn loan_pay_tail_transfer_runs_current() {
        let mut steps = Vec::new();
        let mut sink = TestSink {
            steps: &mut steps,
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TEC_DUPLICATE,
            account_send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 7,
            expected_broker_payee: "borrower",
            expected_broker_amount: 3,
        };

        let result = run_loan_pay_tail_transfer(
            &mut sink,
            LoanPayTailTransferFacts {
                account: "borrower",
                vault_pseudo_account: "vault-pseudo",
                broker_payee: "borrower",
                asset: "USD",
                zero_amount: 0,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps,
            [
                "vault-pseudo",
                "broker_payee_balance",
                "add_empty_holding",
                "borrower",
                "account_send_multi",
            ]
        );
    }

    #[test]
    fn loan_pay_tail_transfer_skips_zero_value_auth_paths() {
        let mut steps = Vec::new();
        let mut sink = TestSink {
            steps: &mut steps,
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TES_SUCCESS,
            account_send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 0,
            expected_broker_payee: "owner",
            expected_broker_amount: 0,
        };

        let result = run_loan_pay_tail_transfer(
            &mut sink,
            LoanPayTailTransferFacts {
                account: "borrower",
                vault_pseudo_account: "vault-pseudo",
                broker_payee: "owner",
                asset: "USD",
                zero_amount: 0,
                total_paid_to_vault_rounded: 0,
                total_paid_to_broker: 0,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(steps, ["account_send_multi"]);
    }

    #[test]
    fn loan_pay_tail_transfer_passthroughs_vault_auth_failure() {
        let mut steps = Vec::new();
        let mut sink = TestSink {
            steps: &mut steps,
            vault_auth_result: Ter::TEC_NO_AUTH,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TES_SUCCESS,
            account_send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 7,
            expected_broker_payee: "owner",
            expected_broker_amount: 0,
        };

        let result = run_loan_pay_tail_transfer(
            &mut sink,
            LoanPayTailTransferFacts {
                account: "borrower",
                vault_pseudo_account: "vault-pseudo",
                broker_payee: "owner",
                asset: "USD",
                zero_amount: 0,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 0,
            },
        );

        assert_eq!(result, Ter::TEC_NO_AUTH);
        assert_eq!(steps, ["vault-pseudo"]);
    }

    #[test]
    fn loan_pay_tail_transfer_passthroughs_account_send_multi_failure() {
        let mut steps = Vec::new();
        let mut sink = TestSink {
            steps: &mut steps,
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TES_SUCCESS,
            account_send_multi_result: Ter::TEC_PATH_DRY,
            expected_vault_amount: 7,
            expected_broker_payee: "owner",
            expected_broker_amount: 3,
        };

        let result = run_loan_pay_tail_transfer(
            &mut sink,
            LoanPayTailTransferFacts {
                account: "borrower",
                vault_pseudo_account: "vault-pseudo",
                broker_payee: "owner",
                asset: "USD",
                zero_amount: 0,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
            },
        );

        assert_eq!(result, Ter::TEC_PATH_DRY);
        assert_eq!(steps, ["vault-pseudo", "owner", "account_send_multi"]);
    }
}
