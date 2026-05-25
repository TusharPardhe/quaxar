//! Current Rust helper mirroring the bottom-of-the LoanPay transactor
//! post-transfer balance assertions.
//!
//! This module preserves the current deterministic debug-style facts around:
//!
//! - funds conserved across borrower, vault, and broker balances,
//! - non-negative borrower, vault, and broker balances,
//! - borrower balance decreasing unless the borrower is the issuer,
//! - vault and broker balances not decreasing, and
//! - at least one of vault or broker increasing.

use std::ops::Add;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPostBalanceFacts<Balance> {
    pub borrower_balance_before: Balance,
    pub borrower_balance_after: Balance,
    pub vault_balance_before: Balance,
    pub vault_balance_after: Balance,
    pub broker_balance_before: Balance,
    pub broker_balance_after: Balance,
    pub total_balance_before: Balance,
    pub total_balance_after: Balance,
    pub funds_conserved: bool,
    pub borrower_balance_non_negative: bool,
    pub vault_balance_non_negative: bool,
    pub broker_balance_non_negative: bool,
    pub borrower_balance_decreased_unless_issuer: bool,
    pub vault_balance_did_not_decrease: bool,
    pub broker_balance_did_not_decrease: bool,
    pub vault_or_broker_increased: bool,
}

pub fn compute_loan_pay_post_balances<Balance>(
    borrower_balance_before: &Balance,
    borrower_balance_after: &Balance,
    vault_balance_before: &Balance,
    vault_balance_after: &Balance,
    broker_balance_before: &Balance,
    broker_balance_after: &Balance,
    zero_balance: &Balance,
    borrower_is_issuer: bool,
) -> LoanPayPostBalanceFacts<Balance>
where
    Balance: Clone + Add<Output = Balance> + PartialEq + PartialOrd,
{
    let total_balance_before = borrower_balance_before.clone()
        + vault_balance_before.clone()
        + broker_balance_before.clone();
    let total_balance_after =
        borrower_balance_after.clone() + vault_balance_after.clone() + broker_balance_after.clone();

    LoanPayPostBalanceFacts {
        borrower_balance_before: borrower_balance_before.clone(),
        borrower_balance_after: borrower_balance_after.clone(),
        vault_balance_before: vault_balance_before.clone(),
        vault_balance_after: vault_balance_after.clone(),
        broker_balance_before: broker_balance_before.clone(),
        broker_balance_after: broker_balance_after.clone(),
        total_balance_before: total_balance_before.clone(),
        total_balance_after: total_balance_after.clone(),
        funds_conserved: total_balance_before == total_balance_after,
        borrower_balance_non_negative: borrower_balance_after >= zero_balance,
        vault_balance_non_negative: vault_balance_after >= zero_balance,
        broker_balance_non_negative: broker_balance_after >= zero_balance,
        borrower_balance_decreased_unless_issuer: borrower_balance_after < borrower_balance_before
            || borrower_is_issuer,
        vault_balance_did_not_decrease: vault_balance_after >= vault_balance_before,
        broker_balance_did_not_decrease: broker_balance_after >= broker_balance_before,
        vault_or_broker_increased: vault_balance_after > vault_balance_before
            || broker_balance_after > broker_balance_before,
    }
}

#[cfg(test)]
mod tests {
    use super::compute_loan_pay_post_balances;

    #[test]
    fn compute_loan_pay_post_balances_debug_assertions() {
        let facts = compute_loan_pay_post_balances(&100_i64, &80, &20, &30, &5, &15, &0, false);

        assert_eq!(facts.total_balance_before, 125);
        assert_eq!(facts.total_balance_after, 125);
        assert!(facts.funds_conserved);
        assert!(facts.borrower_balance_non_negative);
        assert!(facts.vault_balance_non_negative);
        assert!(facts.broker_balance_non_negative);
        assert!(facts.borrower_balance_decreased_unless_issuer);
        assert!(facts.vault_balance_did_not_decrease);
        assert!(facts.broker_balance_did_not_decrease);
        assert!(facts.vault_or_broker_increased);
    }

    #[test]
    fn compute_loan_pay_post_balances_accepts_issuer_flat_borrower_balance() {
        let facts = compute_loan_pay_post_balances(&100_i64, &100, &20, &20, &5, &5, &0, true);

        assert!(facts.funds_conserved);
        assert!(facts.borrower_balance_non_negative);
        assert!(facts.vault_balance_non_negative);
        assert!(facts.broker_balance_non_negative);
        assert!(facts.borrower_balance_decreased_unless_issuer);
        assert!(facts.vault_balance_did_not_decrease);
        assert!(facts.broker_balance_did_not_decrease);
        assert!(!facts.vault_or_broker_increased);
    }

    #[test]
    fn compute_loan_pay_post_balances_flags_negative_or_non_conserved_outcomes() {
        let facts = compute_loan_pay_post_balances(&50_i64, &40, &10, &9, &5, &-4, &0, false);

        assert!(!facts.funds_conserved);
        assert!(facts.borrower_balance_non_negative);
        assert!(facts.vault_balance_non_negative);
        assert!(!facts.broker_balance_non_negative);
        assert!(facts.borrower_balance_decreased_unless_issuer);
        assert!(!facts.vault_balance_did_not_decrease);
        assert!(!facts.broker_balance_did_not_decrease);
        assert!(!facts.vault_or_broker_increased);
    }
}
