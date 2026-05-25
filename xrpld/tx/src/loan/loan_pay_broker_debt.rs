//! Current Rust helper mirroring the the reference implementation broker-debt adjustment
//! seam.
//!
//! This keeps the deterministic inputs explicit:
//!
//! - the debt-delta sign,
//! - `totalPaidToVaultForDebt`,
//! - the asset,
//! - the vault scale, and
//! - the signed debt delta produced from those inputs.
//!
//! The helper preserves the the reference implementation order:
//! amount shaping first, then broker debt adjustment, then vault mutation.

use core::ops::Neg;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanPayBrokerDebtDeltaSign {
    Increase,
    Decrease,
}

impl LoanPayBrokerDebtDeltaSign {
    pub fn apply<Amount>(&self, amount: Amount) -> Amount
    where
        Amount: Neg<Output = Amount>,
    {
        match self {
            Self::Increase => amount,
            Self::Decrease => -amount,
        }
    }

    pub fn is_decrease(self) -> bool {
        matches!(self, Self::Decrease)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayBrokerDebtFacts<Amount, Asset, Scale> {
    pub debt_delta_sign: LoanPayBrokerDebtDeltaSign,
    pub total_paid_to_vault_for_debt: Amount,
    pub asset: Asset,
    pub vault_scale: Scale,
    pub signed_debt_delta: Amount,
}

pub trait LoanPayBrokerDebtAdjustmentSink {
    type Amount;
    type Asset;
    type Scale;

    fn adjust_debt_total(
        &mut self,
        debt_delta: Self::Amount,
        asset: Self::Asset,
        vault_scale: Self::Scale,
    );
}

pub fn compute_loan_pay_broker_debt_facts<Amount, Asset, Scale>(
    debt_delta_sign: LoanPayBrokerDebtDeltaSign,
    total_paid_to_vault_for_debt: Amount,
    asset: Asset,
    vault_scale: Scale,
) -> LoanPayBrokerDebtFacts<Amount, Asset, Scale>
where
    Amount: Neg<Output = Amount> + Clone,
{
    let signed_debt_delta = debt_delta_sign.apply(total_paid_to_vault_for_debt.clone());

    LoanPayBrokerDebtFacts {
        debt_delta_sign,
        total_paid_to_vault_for_debt,
        asset,
        vault_scale,
        signed_debt_delta,
    }
}

pub fn run_loan_pay_broker_debt_adjustment<Sink>(
    sink: &mut Sink,
    facts: LoanPayBrokerDebtFacts<Sink::Amount, Sink::Asset, Sink::Scale>,
) -> LoanPayBrokerDebtFacts<Sink::Amount, Sink::Asset, Sink::Scale>
where
    Sink: LoanPayBrokerDebtAdjustmentSink,
    Sink::Amount: Neg<Output = Sink::Amount> + Clone,
    Sink::Asset: Clone,
    Sink::Scale: Clone,
{
    sink.adjust_debt_total(
        facts.signed_debt_delta.clone(),
        facts.asset.clone(),
        facts.vault_scale.clone(),
    );

    facts
}

#[cfg(test)]
mod tests {
    use super::{
        LoanPayBrokerDebtAdjustmentSink, LoanPayBrokerDebtDeltaSign,
        compute_loan_pay_broker_debt_facts, run_loan_pay_broker_debt_adjustment,
    };

    #[derive(Debug, Default)]
    struct RecordingSink {
        debt_total: i64,
        steps: Vec<String>,
    }

    impl LoanPayBrokerDebtAdjustmentSink for RecordingSink {
        type Amount = i64;
        type Asset = &'static str;
        type Scale = i32;

        fn adjust_debt_total(
            &mut self,
            debt_delta: Self::Amount,
            asset: Self::Asset,
            vault_scale: Self::Scale,
        ) {
            self.debt_total += debt_delta;
            self.steps.push(format!(
                "adjust_debt_total delta={debt_delta} asset={asset} scale={vault_scale}"
            ));
        }
    }

    #[test]
    fn loan_pay_broker_debt_computes_negative_delta() {
        let facts = compute_loan_pay_broker_debt_facts(
            LoanPayBrokerDebtDeltaSign::Decrease,
            304_i64,
            "USD",
            6_i32,
        );

        assert!(facts.debt_delta_sign.is_decrease());
        assert_eq!(facts.total_paid_to_vault_for_debt, 304);
        assert_eq!(facts.asset, "USD");
        assert_eq!(facts.vault_scale, 6);
        assert_eq!(facts.signed_debt_delta, -304);
    }

    #[test]
    fn loan_pay_broker_debt_keeps_positive_delta_available_for_future_callers() {
        let facts = compute_loan_pay_broker_debt_facts(
            LoanPayBrokerDebtDeltaSign::Increase,
            19_i64,
            "XRP",
            0_i32,
        );

        assert!(!facts.debt_delta_sign.is_decrease());
        assert_eq!(facts.signed_debt_delta, 19);
    }

    #[test]
    fn loan_pay_broker_debt_adjustment_preserves_cxx_adjustment_order() {
        let mut sink = RecordingSink::default();

        let facts = compute_loan_pay_broker_debt_facts(
            LoanPayBrokerDebtDeltaSign::Decrease,
            87_i64,
            "EUR",
            10_i32,
        );
        let returned = run_loan_pay_broker_debt_adjustment(&mut sink, facts);

        assert_eq!(sink.debt_total, -87);
        assert_eq!(
            sink.steps,
            vec!["adjust_debt_total delta=-87 asset=EUR scale=10"]
        );
        assert_eq!(returned.signed_debt_delta, -87);
        assert_eq!(returned.total_paid_to_vault_for_debt, 87);
    }
}
