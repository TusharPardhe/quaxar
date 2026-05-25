//! Current Rust helper mirroring the pre-transfer the LoanPay transactor
//! ordering facts.
//!
//! This module preserves the deterministic conditions that decide which
//! pre-transfer steps are required and in what order.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayTransferPrepFacts<Amount> {
    pub total_paid_to_vault_rounded: Amount,
    pub total_paid_to_broker: Amount,
    pub vault_auth_required: bool,
    pub broker_payment_present: bool,
    pub broker_payee_is_borrower: bool,
    pub add_empty_holding_required: bool,
    pub broker_auth_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayTransferDeliveryFacts<Amount> {
    pub amount: Amount,
    pub total_paid_to_vault_rounded: Amount,
    pub total_paid_to_broker: Amount,
    pub outputs_total: Amount,
    pub amount_covers_outputs: bool,
}

pub fn compute_loan_pay_transfer_prep_facts<Amount>(
    total_paid_to_vault_rounded: &Amount,
    total_paid_to_broker: &Amount,
    zero_amount: &Amount,
    broker_payee_is_borrower: bool,
) -> LoanPayTransferPrepFacts<Amount>
where
    Amount: Clone + PartialEq,
{
    let vault_auth_required = total_paid_to_vault_rounded != zero_amount;
    let broker_payment_present = total_paid_to_broker != zero_amount;
    let add_empty_holding_required = broker_payment_present && broker_payee_is_borrower;
    let broker_auth_required = broker_payment_present;

    LoanPayTransferPrepFacts {
        total_paid_to_vault_rounded: total_paid_to_vault_rounded.clone(),
        total_paid_to_broker: total_paid_to_broker.clone(),
        vault_auth_required,
        broker_payment_present,
        broker_payee_is_borrower,
        add_empty_holding_required,
        broker_auth_required,
    }
}

pub fn compute_loan_pay_transfer_delivery_facts<Amount>(
    amount: &Amount,
    total_paid_to_vault_rounded: &Amount,
    total_paid_to_broker: &Amount,
) -> LoanPayTransferDeliveryFacts<Amount>
where
    Amount: Clone + PartialOrd + core::ops::Add<Output = Amount>,
{
    let outputs_total = total_paid_to_vault_rounded.clone() + total_paid_to_broker.clone();
    let amount_covers_outputs = outputs_total <= amount.clone();

    LoanPayTransferDeliveryFacts {
        amount: amount.clone(),
        total_paid_to_vault_rounded: total_paid_to_vault_rounded.clone(),
        total_paid_to_broker: total_paid_to_broker.clone(),
        outputs_total,
        amount_covers_outputs,
    }
}

#[cfg(test)]
mod tests {
    use super::{compute_loan_pay_transfer_delivery_facts, compute_loan_pay_transfer_prep_facts};

    #[test]
    fn loan_pay_transfer_prep_requires_only_vault_auth_when_only_vault_receives() {
        let facts = compute_loan_pay_transfer_prep_facts(&7_i64, &0_i64, &0_i64, false);

        assert!(facts.vault_auth_required);
        assert!(!facts.broker_payment_present);
        assert!(!facts.add_empty_holding_required);
        assert!(!facts.broker_auth_required);
    }

    #[test]
    fn loan_pay_transfer_prep_requires_holding_then_broker_auth_when_borrower_is_payee() {
        let facts = compute_loan_pay_transfer_prep_facts(&7_i64, &3_i64, &0_i64, true);

        assert!(facts.vault_auth_required);
        assert!(facts.broker_payment_present);
        assert!(facts.broker_payee_is_borrower);
        assert!(facts.add_empty_holding_required);
        assert!(facts.broker_auth_required);
    }

    #[test]
    fn loan_pay_transfer_prep_skips_holding_recreation_for_non_borrower_broker_payee() {
        let facts = compute_loan_pay_transfer_prep_facts(&0_i64, &3_i64, &0_i64, false);

        assert!(!facts.vault_auth_required);
        assert!(facts.broker_payment_present);
        assert!(!facts.add_empty_holding_required);
        assert!(facts.broker_auth_required);
    }

    #[test]
    fn loan_pay_transfer_delivery_keeps_amount_sufficient_when_outputs_fit() {
        let facts = compute_loan_pay_transfer_delivery_facts(&25_i64, &13_i64, &1_i64);

        assert_eq!(facts.outputs_total, 14);
        assert!(facts.amount_covers_outputs);
    }

    #[test]
    fn loan_pay_transfer_delivery_flags_overdrawn_outputs() {
        let facts = compute_loan_pay_transfer_delivery_facts(&10_i64, &8_i64, &3_i64);

        assert_eq!(facts.outputs_total, 11);
        assert!(!facts.amount_covers_outputs);
    }
}
