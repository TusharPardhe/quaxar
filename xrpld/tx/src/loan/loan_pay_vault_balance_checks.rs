//! Current Rust helper mirroring the vault pseudo-account consistency
//! assertions inside the LoanPay transactor.
//!
//! This module preserves the current deterministic debug-style checks around:
//!
//! - `assetsAvailableProxy == pseudoAccountBalanceBefore`, and
//! - `assetsAvailableProxy == pseudoAccountBalanceAfter` after
//!   `accountSendMulti(...)`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayVaultBalanceCheckFacts<Balance> {
    pub assets_available_before: Balance,
    pub pseudo_account_balance_before: Balance,
    pub assets_available_after: Balance,
    pub pseudo_account_balance_after: Balance,
    pub vault_pseudo_balance_agrees_before: bool,
    pub vault_pseudo_balance_agrees_after: bool,
}

pub fn compute_loan_pay_vault_balance_checks<Balance>(
    assets_available_before: &Balance,
    pseudo_account_balance_before: &Balance,
    assets_available_after: &Balance,
    pseudo_account_balance_after: &Balance,
) -> LoanPayVaultBalanceCheckFacts<Balance>
where
    Balance: Clone + PartialEq,
{
    LoanPayVaultBalanceCheckFacts {
        assets_available_before: assets_available_before.clone(),
        pseudo_account_balance_before: pseudo_account_balance_before.clone(),
        assets_available_after: assets_available_after.clone(),
        pseudo_account_balance_after: pseudo_account_balance_after.clone(),
        vault_pseudo_balance_agrees_before: assets_available_before
            == pseudo_account_balance_before,
        vault_pseudo_balance_agrees_after: assets_available_after == pseudo_account_balance_after,
    }
}

#[cfg(test)]
mod tests {
    use super::compute_loan_pay_vault_balance_checks;

    #[test]
    fn compute_loan_pay_vault_balance_checks_before_and_after_assertions() {
        let facts = compute_loan_pay_vault_balance_checks(&10_i64, &10, &25, &25);

        assert_eq!(facts.assets_available_before, 10);
        assert_eq!(facts.pseudo_account_balance_before, 10);
        assert_eq!(facts.assets_available_after, 25);
        assert_eq!(facts.pseudo_account_balance_after, 25);
        assert!(facts.vault_pseudo_balance_agrees_before);
        assert!(facts.vault_pseudo_balance_agrees_after);
    }

    #[test]
    fn compute_loan_pay_vault_balance_checks_flags_before_mismatch_assertion() {
        let facts = compute_loan_pay_vault_balance_checks(&10_i64, &9, &25, &25);

        assert!(!facts.vault_pseudo_balance_agrees_before);
        assert!(facts.vault_pseudo_balance_agrees_after);
    }

    #[test]
    fn compute_loan_pay_vault_balance_checks_flags_after_mismatch_assertion() {
        let facts = compute_loan_pay_vault_balance_checks(&10_i64, &10, &25, &24);

        assert!(facts.vault_pseudo_balance_agrees_before);
        assert!(!facts.vault_pseudo_balance_agrees_after);
    }
}
