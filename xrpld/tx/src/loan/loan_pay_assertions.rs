//! Current Rust helper mirroring the composed debug-style assertions at the
//! bottom of the LoanPay transactor.
//!
//! This helper collects the currently expressible post-transfer facts around:
//!
//! - vault pseudo-account balance agreement before and after transfer,
//! - borrower/vault/broker funds conservation,
//! - non-negative borrower/vault/broker balances,
//! - borrower balance decreasing unless the borrower is the issuer,
//! - vault and broker balances not decreasing, and
//! - at least one of vault or broker increasing.

use crate::{LoanPayPostBalanceFacts, LoanPayVaultBalanceCheckFacts};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayAssertionFacts {
    pub vault_pseudo_balance_agrees_before: bool,
    pub vault_pseudo_balance_agrees_after: bool,
    pub funds_conserved: bool,
    pub borrower_balance_non_negative: bool,
    pub vault_balance_non_negative: bool,
    pub broker_balance_non_negative: bool,
    pub borrower_balance_decreased_unless_issuer: bool,
    pub vault_balance_did_not_decrease: bool,
    pub broker_balance_did_not_decrease: bool,
    pub vault_or_broker_increased: bool,
    pub all_assertions_hold: bool,
}

pub fn compute_loan_pay_assertion_facts<Balance>(
    vault_checks: &LoanPayVaultBalanceCheckFacts<Balance>,
    post_balances: &LoanPayPostBalanceFacts<Balance>,
) -> LoanPayAssertionFacts {
    let vault_pseudo_balance_agrees_before = vault_checks.vault_pseudo_balance_agrees_before;
    let vault_pseudo_balance_agrees_after = vault_checks.vault_pseudo_balance_agrees_after;
    let funds_conserved = post_balances.funds_conserved;
    let borrower_balance_non_negative = post_balances.borrower_balance_non_negative;
    let vault_balance_non_negative = post_balances.vault_balance_non_negative;
    let broker_balance_non_negative = post_balances.broker_balance_non_negative;
    let borrower_balance_decreased_unless_issuer =
        post_balances.borrower_balance_decreased_unless_issuer;
    let vault_balance_did_not_decrease = post_balances.vault_balance_did_not_decrease;
    let broker_balance_did_not_decrease = post_balances.broker_balance_did_not_decrease;
    let vault_or_broker_increased = post_balances.vault_or_broker_increased;
    let all_assertions_hold = vault_pseudo_balance_agrees_before
        && vault_pseudo_balance_agrees_after
        && funds_conserved
        && borrower_balance_non_negative
        && vault_balance_non_negative
        && broker_balance_non_negative
        && borrower_balance_decreased_unless_issuer
        && vault_balance_did_not_decrease
        && broker_balance_did_not_decrease
        && vault_or_broker_increased;

    LoanPayAssertionFacts {
        vault_pseudo_balance_agrees_before,
        vault_pseudo_balance_agrees_after,
        funds_conserved,
        borrower_balance_non_negative,
        vault_balance_non_negative,
        broker_balance_non_negative,
        borrower_balance_decreased_unless_issuer,
        vault_balance_did_not_decrease,
        broker_balance_did_not_decrease,
        vault_or_broker_increased,
        all_assertions_hold,
    }
}

#[cfg(test)]
mod tests {
    use super::compute_loan_pay_assertion_facts;
    use crate::{LoanPayPostBalanceFacts, LoanPayVaultBalanceCheckFacts};

    #[test]
    fn compute_loan_pay_assertion_facts_match_cpp_debug_assertion_bundle() {
        let vault_checks = LoanPayVaultBalanceCheckFacts {
            assets_available_before: 10_i64,
            pseudo_account_balance_before: 10,
            assets_available_after: 25,
            pseudo_account_balance_after: 25,
            vault_pseudo_balance_agrees_before: true,
            vault_pseudo_balance_agrees_after: true,
        };
        let post_balances = LoanPayPostBalanceFacts {
            borrower_balance_before: 100,
            borrower_balance_after: 80,
            vault_balance_before: 20,
            vault_balance_after: 30,
            broker_balance_before: 5,
            broker_balance_after: 15,
            total_balance_before: 125,
            total_balance_after: 125,
            funds_conserved: true,
            borrower_balance_non_negative: true,
            vault_balance_non_negative: true,
            broker_balance_non_negative: true,
            borrower_balance_decreased_unless_issuer: true,
            vault_balance_did_not_decrease: true,
            broker_balance_did_not_decrease: true,
            vault_or_broker_increased: true,
        };

        let facts = compute_loan_pay_assertion_facts(&vault_checks, &post_balances);

        assert!(facts.all_assertions_hold);
    }

    #[test]
    fn compute_loan_pay_assertion_facts_fail_when_any_component_fails() {
        let vault_checks = LoanPayVaultBalanceCheckFacts {
            assets_available_before: 10_i64,
            pseudo_account_balance_before: 9,
            assets_available_after: 25,
            pseudo_account_balance_after: 25,
            vault_pseudo_balance_agrees_before: false,
            vault_pseudo_balance_agrees_after: true,
        };
        let post_balances = LoanPayPostBalanceFacts {
            borrower_balance_before: 100,
            borrower_balance_after: 100,
            vault_balance_before: 20,
            vault_balance_after: 20,
            broker_balance_before: 5,
            broker_balance_after: 5,
            total_balance_before: 125,
            total_balance_after: 125,
            funds_conserved: true,
            borrower_balance_non_negative: true,
            vault_balance_non_negative: true,
            broker_balance_non_negative: true,
            borrower_balance_decreased_unless_issuer: false,
            vault_balance_did_not_decrease: true,
            broker_balance_did_not_decrease: true,
            vault_or_broker_increased: false,
        };

        let facts = compute_loan_pay_assertion_facts(&vault_checks, &post_balances);

        assert!(!facts.all_assertions_hold);
        assert!(!facts.vault_pseudo_balance_agrees_before);
        assert!(!facts.borrower_balance_decreased_unless_issuer);
        assert!(!facts.vault_or_broker_increased);
    }
}
