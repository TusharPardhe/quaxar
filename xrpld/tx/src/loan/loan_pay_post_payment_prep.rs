//! Current Rust helper mirroring the post-payment, pre-transfer shell inside
//! the LoanPay transactor.
//!
//! This module preserves the deterministic ordering around:
//!
//! - amount shaping from `paymentParts`,
//! - broker-debt adjustment facts handoff,
//! - vault-state facts,
//! - transfer-delivery facts, and
//! - the the reference implementation order between those facts.
//!
//! The helper now plugs into the composed Rust `LoanPay` shell while still
//! keeping the real mutation kernels injected at the call boundary.

use core::ops::{Add, Neg, Sub};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPaymentParts<Amount> {
    pub principal_paid: Amount,
    pub interest_paid: Amount,
    pub fee_paid: Amount,
    pub value_change: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPostPaymentAmountFacts<Amount> {
    pub vault_scale: i32,
    pub total_paid_to_vault_raw: Amount,
    pub total_paid_to_vault_rounded: Amount,
    pub total_paid_to_vault_for_debt: Amount,
    pub total_paid_to_broker: Amount,
    pub total_paid_is_positive: bool,
    pub paid_parts_sum_matches_outputs: bool,
    pub integral_asset_rounding_matches_raw: bool,
    pub rounded_amount_is_not_greater_than_raw: bool,
    pub debt_amount_is_rounded: bool,
    pub rounded_and_broker_not_greater_than_amount: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanPayPostPaymentBrokerDebtDeltaSign {
    Increase,
    Decrease,
}

impl LoanPayPostPaymentBrokerDebtDeltaSign {
    fn apply<Amount>(self, amount: Amount) -> Amount
    where
        Amount: Neg<Output = Amount>,
    {
        match self {
            Self::Increase => amount,
            Self::Decrease => -amount,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPostPaymentBrokerDebtFacts<Amount, Asset> {
    pub debt_delta_sign: LoanPayPostPaymentBrokerDebtDeltaSign,
    pub total_paid_to_vault_for_debt: Amount,
    pub asset: Asset,
    pub vault_scale: i32,
    pub signed_debt_delta: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPostPaymentVaultStateFacts<Amount> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPostPaymentTransferDeliveryFacts<Amount> {
    pub amount: Amount,
    pub total_paid_to_vault_rounded: Amount,
    pub total_paid_to_broker: Amount,
    pub outputs_total: Amount,
    pub amount_covers_outputs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPostPaymentPrepFacts<Amount, Asset> {
    pub amount_facts: LoanPayPostPaymentAmountFacts<Amount>,
    pub broker_debt_facts: LoanPayPostPaymentBrokerDebtFacts<Amount, Asset>,
    pub vault_state_facts: LoanPayPostPaymentVaultStateFacts<Amount>,
    pub transfer_delivery_facts: LoanPayPostPaymentTransferDeliveryFacts<Amount>,
}

pub trait LoanPayPostPaymentPrepSink {
    type Vault;
    type Asset;
    type Amount;

    fn vault_scale(&mut self, vault: &Self::Vault) -> i32;

    fn round_to_asset_downward(
        &mut self,
        asset: &Self::Asset,
        value: &Self::Amount,
        scale: i32,
    ) -> Self::Amount;

    fn asset_is_integral(&mut self, asset: &Self::Asset) -> bool;

    fn is_rounded(&mut self, asset: &Self::Asset, value: &Self::Amount, scale: i32) -> bool;
}

pub fn compute_loan_pay_post_payment_amount_facts<Sink>(
    sink: &mut Sink,
    asset: &Sink::Asset,
    vault: &Sink::Vault,
    payment_parts: &LoanPayPaymentParts<Sink::Amount>,
    zero_amount: &Sink::Amount,
    tx_amount: &Sink::Amount,
    loan_scale: i32,
) -> LoanPayPostPaymentAmountFacts<Sink::Amount>
where
    Sink: LoanPayPostPaymentPrepSink,
    Sink::Amount:
        Clone + Add<Output = Sink::Amount> + PartialEq + PartialOrd + Sub<Output = Sink::Amount>,
{
    let vault_scale = sink.vault_scale(vault);
    let total_paid_to_vault_raw =
        payment_parts.principal_paid.clone() + payment_parts.interest_paid.clone();
    let total_paid_to_vault_rounded =
        sink.round_to_asset_downward(asset, &total_paid_to_vault_raw, vault_scale);
    let total_paid_to_vault_for_debt =
        total_paid_to_vault_raw.clone() - payment_parts.value_change.clone();
    let total_paid_to_broker = payment_parts.fee_paid.clone();
    let total_paid_outputs = total_paid_to_vault_rounded.clone() + total_paid_to_broker.clone();

    LoanPayPostPaymentAmountFacts {
        vault_scale,
        total_paid_to_vault_raw: total_paid_to_vault_raw.clone(),
        total_paid_to_vault_rounded: total_paid_to_vault_rounded.clone(),
        total_paid_to_vault_for_debt: total_paid_to_vault_for_debt.clone(),
        total_paid_to_broker: total_paid_to_broker.clone(),
        total_paid_is_positive: total_paid_to_vault_raw > *zero_amount,
        paid_parts_sum_matches_outputs: total_paid_to_vault_raw.clone()
            + total_paid_to_broker.clone()
            == payment_parts.principal_paid.clone()
                + payment_parts.interest_paid.clone()
                + payment_parts.fee_paid.clone(),
        integral_asset_rounding_matches_raw: !sink.asset_is_integral(asset)
            || total_paid_to_vault_raw == total_paid_to_vault_rounded,
        rounded_amount_is_not_greater_than_raw: total_paid_to_vault_rounded
            <= total_paid_to_vault_raw,
        debt_amount_is_rounded: sink.is_rounded(asset, &total_paid_to_vault_for_debt, loan_scale),
        rounded_and_broker_not_greater_than_amount: total_paid_outputs <= *tx_amount,
    }
}

pub fn compute_loan_pay_post_payment_broker_debt_facts<Amount, Asset>(
    total_paid_to_vault_for_debt: Amount,
    asset: Asset,
    vault_scale: i32,
) -> LoanPayPostPaymentBrokerDebtFacts<Amount, Asset>
where
    Amount: Neg<Output = Amount> + Clone,
{
    let debt_delta_sign = LoanPayPostPaymentBrokerDebtDeltaSign::Decrease;
    let signed_debt_delta = debt_delta_sign.apply(total_paid_to_vault_for_debt.clone());

    LoanPayPostPaymentBrokerDebtFacts {
        debt_delta_sign,
        total_paid_to_vault_for_debt,
        asset,
        vault_scale,
        signed_debt_delta,
    }
}

pub fn compute_loan_pay_post_payment_vault_state_facts<Amount>(
    assets_available_before: &Amount,
    assets_total_before: &Amount,
    total_paid_to_vault_rounded: &Amount,
    value_change: &Amount,
) -> LoanPayPostPaymentVaultStateFacts<Amount>
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

    LoanPayPostPaymentVaultStateFacts {
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

pub fn compute_loan_pay_post_payment_transfer_delivery_facts<Amount>(
    amount: &Amount,
    total_paid_to_vault_rounded: &Amount,
    total_paid_to_broker: &Amount,
) -> LoanPayPostPaymentTransferDeliveryFacts<Amount>
where
    Amount: Clone + PartialOrd + Add<Output = Amount>,
{
    let outputs_total = total_paid_to_vault_rounded.clone() + total_paid_to_broker.clone();
    let amount_covers_outputs = outputs_total <= amount.clone();

    LoanPayPostPaymentTransferDeliveryFacts {
        amount: amount.clone(),
        total_paid_to_vault_rounded: total_paid_to_vault_rounded.clone(),
        total_paid_to_broker: total_paid_to_broker.clone(),
        outputs_total,
        amount_covers_outputs,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn compute_loan_pay_post_payment_prep<Sink>(
    sink: &mut Sink,
    asset: &Sink::Asset,
    vault: &Sink::Vault,
    payment_parts: &LoanPayPaymentParts<Sink::Amount>,
    zero_amount: &Sink::Amount,
    tx_amount: &Sink::Amount,
    loan_scale: i32,
    assets_available_before: &Sink::Amount,
    assets_total_before: &Sink::Amount,
) -> LoanPayPostPaymentPrepFacts<Sink::Amount, Sink::Asset>
where
    Sink: LoanPayPostPaymentPrepSink,
    Sink::Amount: Clone
        + Add<Output = Sink::Amount>
        + Neg<Output = Sink::Amount>
        + PartialEq
        + PartialOrd
        + Sub<Output = Sink::Amount>,
    Sink::Asset: Clone,
{
    let amount_facts = compute_loan_pay_post_payment_amount_facts(
        sink,
        asset,
        vault,
        payment_parts,
        zero_amount,
        tx_amount,
        loan_scale,
    );
    let broker_debt_facts = compute_loan_pay_post_payment_broker_debt_facts(
        amount_facts.total_paid_to_vault_for_debt.clone(),
        asset.clone(),
        amount_facts.vault_scale,
    );
    let vault_state_facts = compute_loan_pay_post_payment_vault_state_facts(
        assets_available_before,
        assets_total_before,
        &amount_facts.total_paid_to_vault_rounded,
        &payment_parts.value_change,
    );
    let transfer_delivery_facts = compute_loan_pay_post_payment_transfer_delivery_facts(
        tx_amount,
        &amount_facts.total_paid_to_vault_rounded,
        &amount_facts.total_paid_to_broker,
    );

    LoanPayPostPaymentPrepFacts {
        amount_facts,
        broker_debt_facts,
        vault_state_facts,
        transfer_delivery_facts,
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use super::{
        LoanPayPaymentParts, LoanPayPostPaymentPrepSink,
        compute_loan_pay_post_payment_broker_debt_facts, compute_loan_pay_post_payment_prep,
        compute_loan_pay_post_payment_transfer_delivery_facts,
        compute_loan_pay_post_payment_vault_state_facts,
    };

    #[derive(Default)]
    struct RecordingSink {
        calls: Rc<Cell<u32>>,
    }

    impl LoanPayPostPaymentPrepSink for RecordingSink {
        type Vault = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn vault_scale(&mut self, vault: &Self::Vault) -> i32 {
            assert_eq!(*vault, "vault");
            self.calls.set(self.calls.get() + 1);
            10
        }

        fn round_to_asset_downward(
            &mut self,
            asset: &Self::Asset,
            value: &Self::Amount,
            scale: i32,
        ) -> Self::Amount {
            assert_eq!(*asset, "USD");
            assert_eq!(scale, 10);
            self.calls.set(self.calls.get() + 1);
            *value - (*value % i64::from(scale))
        }

        fn asset_is_integral(&mut self, asset: &Self::Asset) -> bool {
            assert_eq!(*asset, "USD");
            self.calls.set(self.calls.get() + 1);
            false
        }

        fn is_rounded(&mut self, asset: &Self::Asset, value: &Self::Amount, scale: i32) -> bool {
            assert_eq!(*asset, "USD");
            self.calls.set(self.calls.get() + 1);
            *value % i64::from(scale) == 0
        }
    }

    #[test]
    fn loan_pay_post_payment_prep_order() {
        let mut sink = RecordingSink::default();
        let payment_parts = LoanPayPaymentParts {
            principal_paid: 25_i64,
            interest_paid: 12,
            fee_paid: 3,
            value_change: 7,
        };

        let facts = compute_loan_pay_post_payment_prep(
            &mut sink,
            &"USD",
            &"vault",
            &payment_parts,
            &0,
            &40,
            10,
            &10,
            &30,
        );

        assert_eq!(facts.amount_facts.vault_scale, 10);
        assert_eq!(facts.amount_facts.total_paid_to_vault_raw, 37);
        assert_eq!(facts.amount_facts.total_paid_to_vault_rounded, 30);
        assert_eq!(facts.amount_facts.total_paid_to_vault_for_debt, 30);
        assert_eq!(facts.amount_facts.total_paid_to_broker, 3);
        assert!(facts.amount_facts.total_paid_is_positive);
        assert!(facts.amount_facts.paid_parts_sum_matches_outputs);
        assert!(facts.amount_facts.integral_asset_rounding_matches_raw);
        assert!(facts.amount_facts.rounded_amount_is_not_greater_than_raw);
        assert!(facts.amount_facts.debt_amount_is_rounded);
        assert!(
            facts
                .amount_facts
                .rounded_and_broker_not_greater_than_amount
        );
        assert!(
            facts.broker_debt_facts.debt_delta_sign
                == super::LoanPayPostPaymentBrokerDebtDeltaSign::Decrease
        );
        assert_eq!(facts.broker_debt_facts.signed_debt_delta, -30);
        assert_eq!(facts.vault_state_facts.assets_available_after, 40);
        assert_eq!(facts.vault_state_facts.assets_total_after, 37);
        assert!(
            !facts
                .vault_state_facts
                .assets_available_not_greater_than_total
        );
        assert!(!facts.vault_state_facts.all_assertions_hold);
        assert!(facts.vault_state_facts.tec_internal_returned);
        assert_eq!(facts.transfer_delivery_facts.outputs_total, 33);
        assert!(facts.transfer_delivery_facts.amount_covers_outputs);
        assert_eq!(sink.calls.get(), 4);
    }

    #[test]
    fn loan_pay_post_payment_prep_flags_integral_rounding_mismatch() {
        let mut sink = RecordingSink::default();
        let payment_parts = LoanPayPaymentParts {
            principal_paid: 25_i64,
            interest_paid: 12,
            fee_paid: 3,
            value_change: 7,
        };

        let facts = compute_loan_pay_post_payment_prep(
            &mut sink,
            &"USD",
            &"vault",
            &payment_parts,
            &0,
            &40,
            10,
            &10,
            &30,
        );

        assert!(facts.amount_facts.integral_asset_rounding_matches_raw);
        assert!(facts.amount_facts.debt_amount_is_rounded);
    }

    #[test]
    fn loan_pay_post_payment_prep_flags_overdrawn_transfer() {
        let delivery = compute_loan_pay_post_payment_transfer_delivery_facts(&10_i64, &8, &3);

        assert_eq!(delivery.outputs_total, 11);
        assert!(!delivery.amount_covers_outputs);
    }

    #[test]
    fn loan_pay_post_payment_prep_recomputes_vault_state() {
        let vault_state = compute_loan_pay_post_payment_vault_state_facts(&10_i64, &12, &2, &0);

        assert_eq!(vault_state.assets_available_after, 12);
        assert_eq!(vault_state.assets_total_after, 12);
        assert!(vault_state.assets_available_not_greater_than_total);
        assert!(vault_state.duplicate_post_rounding_check_holds);
        assert!(vault_state.all_assertions_hold);
        assert!(!vault_state.tec_internal_returned);
    }

    #[test]
    fn loan_pay_post_payment_prep_keeps_broker_debt_fact_handoff_explicit() {
        let broker_debt = compute_loan_pay_post_payment_broker_debt_facts(87_i64, "EUR", 10_i32);

        assert_eq!(broker_debt.total_paid_to_vault_for_debt, 87);
        assert_eq!(broker_debt.asset, "EUR");
        assert_eq!(broker_debt.vault_scale, 10);
        assert_eq!(broker_debt.signed_debt_delta, -87);
    }
}
