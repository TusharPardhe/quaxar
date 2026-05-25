//! Current Rust helper mirroring the the LoanPay transactor vault-state
//! mutation facts.
//!
//! This module preserves the deterministic arithmetic and invariant checks around
//! `assetsAvailable += totalPaidToVaultRounded`,
//! `assetsTotal += valueChange`, and the duplicated
//! `assetsAvailable <= assetsTotal` guard.

use core::ops::Add;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayVaultStateFacts<Amount> {
    pub assets_available_before: Amount,
    pub assets_total_before: Amount,
    pub total_paid_to_vault_rounded: Amount,
    pub value_change: Amount,
    pub assets_available_after: Amount,
    pub assets_total_after: Amount,
    pub assets_available_not_greater_than_total: bool,
    pub duplicate_post_rounding_check_holds: bool,
    pub all_assertions_hold: bool,
    pub tec_internal_returned: bool,
}

pub fn compute_loan_pay_vault_state_facts<Amount>(
    assets_available_before: &Amount,
    assets_total_before: &Amount,
    total_paid_to_vault_rounded: &Amount,
    value_change: &Amount,
) -> LoanPayVaultStateFacts<Amount>
where
    Amount: Clone + PartialOrd + Add<Output = Amount>,
{
    let assets_available_after =
        assets_available_before.clone() + total_paid_to_vault_rounded.clone();
    let assets_total_after = assets_total_before.clone() + value_change.clone();
    let assets_available_not_greater_than_total = assets_available_after <= assets_total_after;
    let duplicate_post_rounding_check_holds = assets_available_not_greater_than_total;
    let all_assertions_hold =
        assets_available_not_greater_than_total && duplicate_post_rounding_check_holds;
    let tec_internal_returned = !assets_available_not_greater_than_total;

    LoanPayVaultStateFacts {
        assets_available_before: assets_available_before.clone(),
        assets_total_before: assets_total_before.clone(),
        total_paid_to_vault_rounded: total_paid_to_vault_rounded.clone(),
        value_change: value_change.clone(),
        assets_available_after,
        assets_total_after,
        assets_available_not_greater_than_total,
        duplicate_post_rounding_check_holds,
        all_assertions_hold,
        tec_internal_returned,
    }
}

#[cfg(test)]
mod tests {
    use super::compute_loan_pay_vault_state_facts;

    #[test]
    fn loan_pay_vault_state_tracks_after_values() {
        let facts = compute_loan_pay_vault_state_facts(&10_i64, &30_i64, &7_i64, &2_i64);

        assert_eq!(facts.assets_available_after, 17);
        assert_eq!(facts.assets_total_after, 32);
        assert!(facts.assets_available_not_greater_than_total);
        assert!(facts.duplicate_post_rounding_check_holds);
        assert!(facts.all_assertions_hold);
        assert!(!facts.tec_internal_returned);
    }

    #[test]
    fn loan_pay_vault_state_flags_overflow() {
        let facts = compute_loan_pay_vault_state_facts(&10_i64, &10_i64, &1_i64, &0_i64);

        assert_eq!(facts.assets_available_after, 11);
        assert_eq!(facts.assets_total_after, 10);
        assert!(!facts.assets_available_not_greater_than_total);
        assert!(!facts.duplicate_post_rounding_check_holds);
        assert!(!facts.all_assertions_hold);
        assert!(facts.tec_internal_returned);
    }

    #[test]
    fn loan_pay_vault_state_keeps_duplicate_assertions_aligned_on_exact_limit() {
        let facts = compute_loan_pay_vault_state_facts(&10_i64, &12_i64, &2_i64, &0_i64);

        assert_eq!(facts.assets_available_after, 12);
        assert_eq!(facts.assets_total_after, 12);
        assert!(facts.assets_available_not_greater_than_total);
        assert!(facts.duplicate_post_rounding_check_holds);
        assert!(facts.all_assertions_hold);
        assert!(!facts.tec_internal_returned);
    }
}
