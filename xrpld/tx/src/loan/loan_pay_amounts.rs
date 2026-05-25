//! Current Rust helper mirroring the the reference implementation amount-shaping slice
//! between `paymentParts` and the tail transfer facts.
//!
//! This module preserves the current deterministic fact derivation around:
//!
//! - `totalPaidToVaultRaw = principalPaid + interestPaid`,
//! - `totalPaidToVaultRounded` through a caller-supplied vault-rounding policy,
//! - `totalPaidToVaultForDebt = totalPaidToVaultRaw - valueChange`,
//! - `totalPaidToBroker = feePaid`, and
//! - the currently expressible ordering / representability checks on those
//!   values, including the current positive-total and amount-sufficient
//!   invariants.
//!
//! The helper stays generic so callers can supply the rounding and roundedness
//! checks from their own precision policy.

use std::ops::{Add, Sub};

use crate::loan_pay::LoanPayPaymentParts;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayDoApplyAmountFacts<Amount> {
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

pub trait LoanPayDoApplyAmountsSink {
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

pub fn compute_loan_pay_do_apply_amounts<Sink>(
    sink: &mut Sink,
    asset: &Sink::Asset,
    vault: &Sink::Vault,
    payment_parts: &LoanPayPaymentParts<Sink::Amount>,
    zero_amount: &Sink::Amount,
    tx_amount: &Sink::Amount,
    loan_scale: i32,
) -> LoanPayDoApplyAmountFacts<Sink::Amount>
where
    Sink: LoanPayDoApplyAmountsSink,
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

    LoanPayDoApplyAmountFacts {
        vault_scale,
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
        total_paid_to_vault_raw,
        total_paid_to_vault_rounded,
        total_paid_to_vault_for_debt,
        total_paid_to_broker,
    }
}

#[cfg(test)]
mod tests {
    use super::{LoanPayDoApplyAmountsSink, compute_loan_pay_do_apply_amounts};
    use crate::loan_pay::LoanPayPaymentParts;

    #[derive(Debug, Default)]
    struct TestSink {
        vault_scale_seen: Vec<i32>,
        round_inputs: Vec<(i64, i32)>,
        rounded_inputs: Vec<(i64, i32)>,
        asset_is_integral: bool,
    }

    impl LoanPayDoApplyAmountsSink for TestSink {
        type Vault = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn vault_scale(&mut self, vault: &Self::Vault) -> i32 {
            assert_eq!(*vault, "vault");
            self.vault_scale_seen.push(10);
            10
        }

        fn round_to_asset_downward(
            &mut self,
            asset: &Self::Asset,
            value: &Self::Amount,
            scale: i32,
        ) -> Self::Amount {
            assert_eq!(*asset, "USD");
            self.round_inputs.push((*value, scale));
            if scale == 0 {
                return *value;
            }

            *value - (*value % i64::from(scale))
        }

        fn asset_is_integral(&mut self, asset: &Self::Asset) -> bool {
            assert_eq!(*asset, "USD");
            self.asset_is_integral
        }

        fn is_rounded(&mut self, asset: &Self::Asset, value: &Self::Amount, scale: i32) -> bool {
            assert_eq!(*asset, "USD");
            self.rounded_inputs.push((*value, scale));
            if scale == 0 {
                return true;
            }

            *value % i64::from(scale) == 0
        }
    }

    #[test]
    fn compute_loan_pay_do_apply_amounts_order() {
        let mut sink = TestSink::default();
        sink.asset_is_integral = false;

        let facts = compute_loan_pay_do_apply_amounts(
            &mut sink,
            &"USD",
            &"vault",
            &LoanPayPaymentParts {
                principal_paid: 25,
                interest_paid: 12,
                fee_paid: 3,
                value_change: 7,
            },
            &0,
            &40,
            10,
        );

        assert_eq!(facts.total_paid_to_vault_raw, 37);
        assert_eq!(facts.vault_scale, 10);
        assert_eq!(facts.total_paid_to_vault_rounded, 30);
        assert_eq!(facts.total_paid_to_vault_for_debt, 30);
        assert_eq!(facts.total_paid_to_broker, 3);
        assert!(facts.total_paid_is_positive);
        assert!(facts.paid_parts_sum_matches_outputs);
        assert!(facts.integral_asset_rounding_matches_raw);
        assert!(facts.rounded_amount_is_not_greater_than_raw);
        assert!(facts.debt_amount_is_rounded);
        assert!(facts.rounded_and_broker_not_greater_than_amount);
        assert_eq!(sink.vault_scale_seen, vec![10]);
        assert_eq!(sink.round_inputs, vec![(37, 10)]);
        assert_eq!(sink.rounded_inputs, vec![(30, 10)]);
    }

    #[test]
    fn compute_loan_pay_do_apply_amounts_flags_integral_rounding_mismatch_assert() {
        let mut sink = TestSink::default();
        sink.asset_is_integral = true;

        let facts = compute_loan_pay_do_apply_amounts(
            &mut sink,
            &"USD",
            &"vault",
            &LoanPayPaymentParts {
                principal_paid: 25,
                interest_paid: 12,
                fee_paid: 3,
                value_change: 7,
            },
            &0,
            &40,
            10,
        );

        assert!(!facts.integral_asset_rounding_matches_raw);
        assert!(facts.rounded_amount_is_not_greater_than_raw);
    }

    #[test]
    fn compute_loan_pay_do_apply_amounts_flags_rounding_increase_invariant() {
        struct UpwardSink;

        impl LoanPayDoApplyAmountsSink for UpwardSink {
            type Vault = &'static str;
            type Asset = &'static str;
            type Amount = i64;

            fn vault_scale(&mut self, _vault: &Self::Vault) -> i32 {
                10
            }

            fn round_to_asset_downward(
                &mut self,
                _asset: &Self::Asset,
                value: &Self::Amount,
                _scale: i32,
            ) -> Self::Amount {
                *value + 1
            }

            fn asset_is_integral(&mut self, _asset: &Self::Asset) -> bool {
                false
            }

            fn is_rounded(
                &mut self,
                _asset: &Self::Asset,
                _value: &Self::Amount,
                _scale: i32,
            ) -> bool {
                true
            }
        }

        let mut sink = UpwardSink;
        let facts = compute_loan_pay_do_apply_amounts(
            &mut sink,
            &"USD",
            &"vault",
            &LoanPayPaymentParts {
                principal_paid: 25,
                interest_paid: 12,
                fee_paid: 3,
                value_change: 7,
            },
            &0,
            &40,
            10,
        );

        assert!(!facts.rounded_amount_is_not_greater_than_raw);
    }

    #[test]
    fn compute_loan_pay_do_apply_amounts_flags_unrounded_debt_assertion() {
        struct UnroundedDebtSink;

        impl LoanPayDoApplyAmountsSink for UnroundedDebtSink {
            type Vault = &'static str;
            type Asset = &'static str;
            type Amount = i64;

            fn vault_scale(&mut self, _vault: &Self::Vault) -> i32 {
                10
            }

            fn round_to_asset_downward(
                &mut self,
                _asset: &Self::Asset,
                value: &Self::Amount,
                _scale: i32,
            ) -> Self::Amount {
                *value
            }

            fn asset_is_integral(&mut self, _asset: &Self::Asset) -> bool {
                false
            }

            fn is_rounded(
                &mut self,
                _asset: &Self::Asset,
                _value: &Self::Amount,
                _scale: i32,
            ) -> bool {
                false
            }
        }

        let mut sink = UnroundedDebtSink;
        let facts = compute_loan_pay_do_apply_amounts(
            &mut sink,
            &"USD",
            &"vault",
            &LoanPayPaymentParts {
                principal_paid: 25,
                interest_paid: 12,
                fee_paid: 3,
                value_change: 7,
            },
            &0,
            &40,
            10,
        );

        assert!(!facts.debt_amount_is_rounded);
    }

    #[test]
    fn compute_loan_pay_do_apply_amounts_flags_zero_total_paid_assertion() {
        let mut sink = TestSink::default();

        let facts = compute_loan_pay_do_apply_amounts(
            &mut sink,
            &"USD",
            &"vault",
            &LoanPayPaymentParts {
                principal_paid: 0,
                interest_paid: 0,
                fee_paid: 3,
                value_change: 0,
            },
            &0,
            &10,
            10,
        );

        assert!(!facts.total_paid_is_positive);
        assert!(facts.paid_parts_sum_matches_outputs);
    }

    #[test]
    fn compute_loan_pay_do_apply_amounts_flags_amount_insufficient_assertion() {
        let mut sink = TestSink::default();
        sink.asset_is_integral = false;

        let facts = compute_loan_pay_do_apply_amounts(
            &mut sink,
            &"USD",
            &"vault",
            &LoanPayPaymentParts {
                principal_paid: 25,
                interest_paid: 12,
                fee_paid: 3,
                value_change: 7,
            },
            &0,
            &31,
            10,
        );

        assert!(!facts.rounded_and_broker_not_greater_than_amount);
    }
}
