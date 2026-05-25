//! Current Rust helper mirroring the balance-sampling shape inside
//! the LoanPay transactor.
//!
//! This helper stays dependency-free so the surrounding `LoanPay` shell can
//! supply the real before/after balances while preserving the current
//! deterministic debug-style sampling behavior around the borrower, vault
//! pseudo-account, and broker payee:
//!
//! - borrower balances are sampled directly,
//! - vault balances are zero-substituted when the borrower is the vault
//!   pseudo-account, and
//! - broker balances are zero-substituted when the borrower is the broker
//!   payee.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayBalanceSnapshotFacts<Balance> {
    pub borrower_balance_before: Balance,
    pub borrower_balance_after: Balance,
    pub vault_balance_before: Balance,
    pub vault_balance_after: Balance,
    pub broker_balance_before: Balance,
    pub broker_balance_after: Balance,
    pub borrower_is_vault_pseudo: bool,
    pub borrower_is_broker_payee: bool,
}

pub fn compute_loan_pay_balance_snapshot<Balance>(
    borrower_balance_before: &Balance,
    borrower_balance_after: &Balance,
    vault_balance_before: &Balance,
    vault_balance_after: &Balance,
    broker_balance_before: &Balance,
    broker_balance_after: &Balance,
    zero_balance: &Balance,
    borrower_is_vault_pseudo: bool,
    borrower_is_broker_payee: bool,
) -> LoanPayBalanceSnapshotFacts<Balance>
where
    Balance: Clone,
{
    LoanPayBalanceSnapshotFacts {
        borrower_balance_before: borrower_balance_before.clone(),
        borrower_balance_after: borrower_balance_after.clone(),
        vault_balance_before: if borrower_is_vault_pseudo {
            zero_balance.clone()
        } else {
            vault_balance_before.clone()
        },
        vault_balance_after: if borrower_is_vault_pseudo {
            zero_balance.clone()
        } else {
            vault_balance_after.clone()
        },
        broker_balance_before: if borrower_is_broker_payee {
            zero_balance.clone()
        } else {
            broker_balance_before.clone()
        },
        broker_balance_after: if borrower_is_broker_payee {
            zero_balance.clone()
        } else {
            broker_balance_after.clone()
        },
        borrower_is_vault_pseudo,
        borrower_is_broker_payee,
    }
}

#[cfg(test)]
mod tests {
    use super::compute_loan_pay_balance_snapshot;

    #[test]
    fn compute_loan_pay_balance_snapshot_samples_direct_and_zero_substituted_balances() {
        let facts =
            compute_loan_pay_balance_snapshot(&100_i64, &80, &25, &30, &5, &15, &0, false, false);

        assert_eq!(facts.borrower_balance_before, 100);
        assert_eq!(facts.borrower_balance_after, 80);
        assert_eq!(facts.vault_balance_before, 25);
        assert_eq!(facts.vault_balance_after, 30);
        assert_eq!(facts.broker_balance_before, 5);
        assert_eq!(facts.broker_balance_after, 15);
        assert!(!facts.borrower_is_vault_pseudo);
        assert!(!facts.borrower_is_broker_payee);
    }

    #[test]
    fn compute_loan_pay_balance_snapshot_zero_substitutes_vault_when_borrower_is_pseudo() {
        let facts =
            compute_loan_pay_balance_snapshot(&100_i64, &80, &25, &30, &5, &15, &0, true, false);

        assert_eq!(facts.borrower_balance_before, 100);
        assert_eq!(facts.borrower_balance_after, 80);
        assert_eq!(facts.vault_balance_before, 0);
        assert_eq!(facts.vault_balance_after, 0);
        assert_eq!(facts.broker_balance_before, 5);
        assert_eq!(facts.broker_balance_after, 15);
        assert!(facts.borrower_is_vault_pseudo);
        assert!(!facts.borrower_is_broker_payee);
    }

    #[test]
    fn compute_loan_pay_balance_snapshot_zero_substitutes_broker_when_borrower_is_payee() {
        let facts =
            compute_loan_pay_balance_snapshot(&100_i64, &80, &25, &30, &5, &15, &0, false, true);

        assert_eq!(facts.borrower_balance_before, 100);
        assert_eq!(facts.borrower_balance_after, 80);
        assert_eq!(facts.vault_balance_before, 25);
        assert_eq!(facts.vault_balance_after, 30);
        assert_eq!(facts.broker_balance_before, 0);
        assert_eq!(facts.broker_balance_after, 0);
        assert!(!facts.borrower_is_vault_pseudo);
        assert!(facts.borrower_is_broker_payee);
    }
}
