//! Current the reference implementation
//! `impairLoan(...)` and `unimpairLoan(...)` kernels.
//!
//! This helper ports the deterministic amount, guard, and next-due-date
//! branching.

use std::ops::{Add, Sub};

use protocol::Ter;

use crate::loan_manage::{LoanManageOwedToVaultFacts, run_loan_manage_owed_to_vault};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanManageImpairFacts<Amount, Time> {
    pub total_value_outstanding: Amount,
    pub management_fee_outstanding: Amount,
    pub vault_loss_unrealized: Amount,
    pub vault_assets_total: Amount,
    pub vault_assets_available: Amount,
    pub loan_next_payment_due_date: Time,
    pub loan_next_payment_due_has_expired: bool,
    pub parent_close_time: Time,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanManageImpairOutcome<Amount, Time> {
    pub loss_unrealized: Amount,
    pub vault_loss_unrealized: Amount,
    pub loan_is_impaired: bool,
    pub loan_next_payment_due_date: Time,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanManageUnimpairFacts<Amount, Time> {
    pub total_value_outstanding: Amount,
    pub management_fee_outstanding: Amount,
    pub vault_loss_unrealized: Amount,
    pub previous_payment_due_date: Time,
    pub start_date: Time,
    pub payment_interval: Time,
    pub normal_payment_due_has_expired: bool,
    pub parent_close_time: Time,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanManageUnimpairOutcome<Amount, Time> {
    pub loss_reversed: Amount,
    pub vault_loss_unrealized: Amount,
    pub loan_is_impaired: bool,
    pub loan_next_payment_due_date: Time,
}

pub fn run_loan_manage_impair<Amount, Time>(
    facts: LoanManageImpairFacts<Amount, Time>,
) -> Result<LoanManageImpairOutcome<Amount, Time>, Ter>
where
    Amount: Add<Output = Amount> + Sub<Output = Amount> + Ord + Clone,
    Time: Clone,
{
    let loss_unrealized = run_loan_manage_owed_to_vault(LoanManageOwedToVaultFacts {
        total_value_outstanding: facts.total_value_outstanding,
        management_fee_outstanding: facts.management_fee_outstanding,
    });
    let updated_vault_loss = facts.vault_loss_unrealized + loss_unrealized.clone();
    if updated_vault_loss > facts.vault_assets_total - facts.vault_assets_available {
        return Err(Ter::TEC_LIMIT_EXCEEDED);
    }

    let next_due = if facts.loan_next_payment_due_has_expired {
        facts.loan_next_payment_due_date
    } else {
        facts.parent_close_time
    };

    Ok(LoanManageImpairOutcome {
        loss_unrealized,
        vault_loss_unrealized: updated_vault_loss,
        loan_is_impaired: true,
        loan_next_payment_due_date: next_due,
    })
}

pub fn run_loan_manage_unimpair<Amount, Time>(
    facts: LoanManageUnimpairFacts<Amount, Time>,
) -> Result<LoanManageUnimpairOutcome<Amount, Time>, Ter>
where
    Amount: Sub<Output = Amount> + Ord + Clone,
    Time: Add<Output = Time> + Ord + Clone,
{
    let loss_reversed = run_loan_manage_owed_to_vault(LoanManageOwedToVaultFacts {
        total_value_outstanding: facts.total_value_outstanding,
        management_fee_outstanding: facts.management_fee_outstanding,
    });
    if facts.vault_loss_unrealized < loss_reversed.clone() {
        return Err(Ter::TEF_BAD_LEDGER);
    }

    let normal_payment_due_date = std::cmp::max(facts.previous_payment_due_date, facts.start_date)
        + facts.payment_interval.clone();
    let next_due = if facts.normal_payment_due_has_expired {
        facts.parent_close_time + facts.payment_interval
    } else {
        normal_payment_due_date
    };

    Ok(LoanManageUnimpairOutcome {
        loss_reversed: loss_reversed.clone(),
        vault_loss_unrealized: facts.vault_loss_unrealized - loss_reversed,
        loan_is_impaired: false,
        loan_next_payment_due_date: next_due,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        LoanManageImpairFacts, LoanManageImpairOutcome, LoanManageUnimpairFacts,
        LoanManageUnimpairOutcome, run_loan_manage_impair, run_loan_manage_unimpair,
    };
    use protocol::Ter;

    #[test]
    fn loan_manage_impair_rejects_unrealized_loss_that_exceeds_unavailable_assets() {
        let result = run_loan_manage_impair(LoanManageImpairFacts {
            total_value_outstanding: 150_i64,
            management_fee_outstanding: 25_i64,
            vault_loss_unrealized: 60_i64,
            vault_assets_total: 200_i64,
            vault_assets_available: 20_i64,
            loan_next_payment_due_date: 10_i64,
            loan_next_payment_due_has_expired: false,
            parent_close_time: 30_i64,
        });

        assert_eq!(result, Err(Ter::TEC_LIMIT_EXCEEDED));
    }

    #[test]
    fn loan_manage_impair_sets_due_date_to_close_time_when_not_yet_late() {
        let result = run_loan_manage_impair(LoanManageImpairFacts {
            total_value_outstanding: 125_i64,
            management_fee_outstanding: 25_i64,
            vault_loss_unrealized: 10_i64,
            vault_assets_total: 500_i64,
            vault_assets_available: 300_i64,
            loan_next_payment_due_date: 10_i64,
            loan_next_payment_due_has_expired: false,
            parent_close_time: 40_i64,
        });

        assert_eq!(
            result,
            Ok(LoanManageImpairOutcome {
                loss_unrealized: 100_i64,
                vault_loss_unrealized: 110_i64,
                loan_is_impaired: true,
                loan_next_payment_due_date: 40_i64,
            })
        );
    }

    #[test]
    fn loan_manage_impair_keeps_existing_due_date_when_already_late() {
        let result = run_loan_manage_impair(LoanManageImpairFacts {
            total_value_outstanding: 125_i64,
            management_fee_outstanding: 25_i64,
            vault_loss_unrealized: 10_i64,
            vault_assets_total: 500_i64,
            vault_assets_available: 300_i64,
            loan_next_payment_due_date: 10_i64,
            loan_next_payment_due_has_expired: true,
            parent_close_time: 40_i64,
        });

        assert_eq!(
            result,
            Ok(LoanManageImpairOutcome {
                loss_unrealized: 100_i64,
                vault_loss_unrealized: 110_i64,
                loan_is_impaired: true,
                loan_next_payment_due_date: 10_i64,
            })
        );
    }

    #[test]
    fn loan_manage_unimpair_rejects_reversing_more_loss_than_exists() {
        let result = run_loan_manage_unimpair(LoanManageUnimpairFacts {
            total_value_outstanding: 125_i64,
            management_fee_outstanding: 25_i64,
            vault_loss_unrealized: 90_i64,
            previous_payment_due_date: 10_i64,
            start_date: 5_i64,
            payment_interval: 30_i64,
            normal_payment_due_has_expired: false,
            parent_close_time: 40_i64,
        });

        assert_eq!(result, Err(Ter::TEF_BAD_LEDGER));
    }

    #[test]
    fn loan_manage_unimpair_restores_normal_due_date_when_still_within_interval() {
        let result = run_loan_manage_unimpair(LoanManageUnimpairFacts {
            total_value_outstanding: 125_i64,
            management_fee_outstanding: 25_i64,
            vault_loss_unrealized: 150_i64,
            previous_payment_due_date: 10_i64,
            start_date: 5_i64,
            payment_interval: 30_i64,
            normal_payment_due_has_expired: false,
            parent_close_time: 40_i64,
        });

        assert_eq!(
            result,
            Ok(LoanManageUnimpairOutcome {
                loss_reversed: 100_i64,
                vault_loss_unrealized: 50_i64,
                loan_is_impaired: false,
                loan_next_payment_due_date: 40_i64,
            })
        );
    }

    #[test]
    fn loan_manage_unimpair_shifts_due_date_forward_when_interval_has_passed() {
        let result = run_loan_manage_unimpair(LoanManageUnimpairFacts {
            total_value_outstanding: 125_i64,
            management_fee_outstanding: 25_i64,
            vault_loss_unrealized: 150_i64,
            previous_payment_due_date: 10_i64,
            start_date: 50_i64,
            payment_interval: 30_i64,
            normal_payment_due_has_expired: true,
            parent_close_time: 40_i64,
        });

        assert_eq!(
            result,
            Ok(LoanManageUnimpairOutcome {
                loss_reversed: 100_i64,
                vault_loss_unrealized: 50_i64,
                loan_is_impaired: false,
                loan_next_payment_due_date: 70_i64,
            })
        );
    }
}
