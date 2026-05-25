//! Current Rust helper mirroring the post-transfer observation and assertion
//! shell inside the LoanPay transactor.
//!
//! This module preserves the current deterministic sequence around:
//!
//! - sampling the vault pseudo-account balance after transfer,
//! - sampling borrower, vault, and broker balances after transfer,
//! - the borrower/vault/broker zero-substitution rules,
//! - the vault pseudo-account consistency facts,
//! - the post-transfer balance facts, and
//! - the composed debug-style assertion bundle.

use crate::{
    LoanPayAssertionFacts, LoanPayBalanceSnapshotFacts, LoanPayPostBalanceFacts,
    LoanPayVaultBalanceCheckFacts, compute_loan_pay_assertion_facts,
    compute_loan_pay_balance_snapshot, compute_loan_pay_post_balances,
    compute_loan_pay_vault_balance_checks,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPostTransferChecksFacts<AccountId, Asset, Amount> {
    pub account: AccountId,
    pub vault_pseudo_account: AccountId,
    pub broker_payee: AccountId,
    pub asset: Asset,
    pub zero_amount: Amount,
    pub assets_available_before: Amount,
    pub pseudo_account_balance_before: Amount,
    pub borrower_balance_before: Amount,
    pub vault_balance_before: Amount,
    pub broker_balance_before: Amount,
    pub assets_available_after: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPostTransferChecksResult<Amount> {
    pub pseudo_account_balance_after: Amount,
    pub balance_snapshot: LoanPayBalanceSnapshotFacts<Amount>,
    pub vault_balance_checks: LoanPayVaultBalanceCheckFacts<Amount>,
    pub post_balances: LoanPayPostBalanceFacts<Amount>,
    pub assertion_facts: LoanPayAssertionFacts,
}

pub trait LoanPayPostTransferChecksSink {
    type AccountId;
    type Asset;
    type Amount;

    fn sample_balance(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> Self::Amount;
    fn account_is_asset_issuer(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> bool;
}

pub fn run_loan_pay_post_transfer_checks<Sink>(
    sink: &mut Sink,
    facts: LoanPayPostTransferChecksFacts<Sink::AccountId, Sink::Asset, Sink::Amount>,
) -> LoanPayPostTransferChecksResult<Sink::Amount>
where
    Sink: LoanPayPostTransferChecksSink,
    Sink::AccountId: Clone + PartialEq,
    Sink::Amount: Clone + PartialEq + PartialOrd + std::ops::Add<Output = Sink::Amount>,
{
    let pseudo_account_balance_after =
        sink.sample_balance(&facts.vault_pseudo_account, &facts.asset);
    let borrower_balance_after = sink.sample_balance(&facts.account, &facts.asset);
    let vault_balance_after = sink.sample_balance(&facts.vault_pseudo_account, &facts.asset);
    let broker_balance_after = sink.sample_balance(&facts.broker_payee, &facts.asset);

    let balance_snapshot = compute_loan_pay_balance_snapshot(
        &facts.borrower_balance_before,
        &borrower_balance_after,
        &facts.vault_balance_before,
        &vault_balance_after,
        &facts.broker_balance_before,
        &broker_balance_after,
        &facts.zero_amount,
        facts.account == facts.vault_pseudo_account,
        facts.account == facts.broker_payee,
    );
    let vault_balance_checks = compute_loan_pay_vault_balance_checks(
        &facts.assets_available_before,
        &facts.pseudo_account_balance_before,
        &facts.assets_available_after,
        &pseudo_account_balance_after,
    );
    let post_balances = compute_loan_pay_post_balances(
        &balance_snapshot.borrower_balance_before,
        &balance_snapshot.borrower_balance_after,
        &balance_snapshot.vault_balance_before,
        &balance_snapshot.vault_balance_after,
        &balance_snapshot.broker_balance_before,
        &balance_snapshot.broker_balance_after,
        &facts.zero_amount,
        sink.account_is_asset_issuer(&facts.account, &facts.asset),
    );
    let assertion_facts = compute_loan_pay_assertion_facts(&vault_balance_checks, &post_balances);

    LoanPayPostTransferChecksResult {
        pseudo_account_balance_after,
        balance_snapshot,
        vault_balance_checks,
        post_balances,
        assertion_facts,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        LoanPayPostTransferChecksFacts, LoanPayPostTransferChecksSink,
        run_loan_pay_post_transfer_checks,
    };

    struct TestSink {
        balances: HashMap<&'static str, i64>,
        issuer: &'static str,
        steps: Vec<&'static str>,
    }

    impl LoanPayPostTransferChecksSink for TestSink {
        type AccountId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn sample_balance(
            &mut self,
            account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> Self::Amount {
            self.steps.push(match *account {
                "vault" => "sample_vault",
                "borrower" => "sample_borrower",
                "broker" => "sample_broker",
                _ => "sample_other",
            });
            *self.balances.get(account).unwrap_or(&0)
        }

        fn account_is_asset_issuer(
            &mut self,
            account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.push("issuer_check");
            *account == self.issuer
        }
    }

    #[test]
    fn loan_pay_post_transfer_checks_samples_balances_in() {
        let mut sink = TestSink {
            balances: HashMap::from([("borrower", 80), ("vault", 30), ("broker", 15)]),
            issuer: "issuer",
            steps: Vec::new(),
        };

        let result = run_loan_pay_post_transfer_checks(
            &mut sink,
            LoanPayPostTransferChecksFacts {
                account: "borrower",
                vault_pseudo_account: "vault",
                broker_payee: "broker",
                asset: "USD",
                zero_amount: 0,
                assets_available_before: 10,
                pseudo_account_balance_before: 10,
                borrower_balance_before: 100,
                vault_balance_before: 20,
                broker_balance_before: 5,
                assets_available_after: 30,
            },
        );

        assert_eq!(
            sink.steps,
            [
                "sample_vault",
                "sample_borrower",
                "sample_vault",
                "sample_broker",
                "issuer_check",
            ]
        );
        assert_eq!(result.pseudo_account_balance_after, 30);
        assert!(result.assertion_facts.all_assertions_hold);
    }

    #[test]
    fn loan_pay_post_transfer_checks_zero_substitutes_alias_balances() {
        let mut sink = TestSink {
            balances: HashMap::from([("borrower", 50), ("vault", 20)]),
            issuer: "issuer",
            steps: Vec::new(),
        };

        let result = run_loan_pay_post_transfer_checks(
            &mut sink,
            LoanPayPostTransferChecksFacts {
                account: "borrower",
                vault_pseudo_account: "borrower",
                broker_payee: "borrower",
                asset: "USD",
                zero_amount: 0,
                assets_available_before: 20,
                pseudo_account_balance_before: 20,
                borrower_balance_before: 70,
                vault_balance_before: 20,
                broker_balance_before: 5,
                assets_available_after: 50,
            },
        );

        assert_eq!(result.balance_snapshot.vault_balance_before, 0);
        assert_eq!(result.balance_snapshot.vault_balance_after, 0);
        assert_eq!(result.balance_snapshot.broker_balance_before, 0);
        assert_eq!(result.balance_snapshot.broker_balance_after, 0);
    }

    #[test]
    fn loan_pay_post_transfer_checks_surfaces_failed_assertions() {
        let mut sink = TestSink {
            balances: HashMap::from([("borrower", 100), ("vault", 19), ("broker", 5)]),
            issuer: "issuer",
            steps: Vec::new(),
        };

        let result = run_loan_pay_post_transfer_checks(
            &mut sink,
            LoanPayPostTransferChecksFacts {
                account: "borrower",
                vault_pseudo_account: "vault",
                broker_payee: "broker",
                asset: "USD",
                zero_amount: 0,
                assets_available_before: 10,
                pseudo_account_balance_before: 9,
                borrower_balance_before: 100,
                vault_balance_before: 20,
                broker_balance_before: 5,
                assets_available_after: 25,
            },
        );

        assert!(
            !result
                .vault_balance_checks
                .vault_pseudo_balance_agrees_before
        );
        assert!(
            !result
                .vault_balance_checks
                .vault_pseudo_balance_agrees_after
        );
        assert!(!result.assertion_facts.all_assertions_hold);
    }
}
