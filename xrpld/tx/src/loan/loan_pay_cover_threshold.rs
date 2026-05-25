//! Current Rust helper mirroring the the reference implementation broker-owner cover threshold
//! comparison.
//!
//! This helper preserves the deterministic comparison that decides whether the
//! broker owner is eligible to receive the fee.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayCoverThresholdFacts<Amount, Asset, CoverRateMinimum, Scale> {
    pub cover_available: Amount,
    pub asset: Asset,
    pub debt_total: Amount,
    pub cover_rate_minimum: CoverRateMinimum,
    pub loan_scale: Scale,
    pub required_cover: Amount,
    pub cover_available_meets_minimum: bool,
}

pub trait LoanPayCoverThresholdSink {
    type Amount;
    type Asset;
    type CoverRateMinimum;
    type Scale;

    fn compute_required_cover_threshold(
        &mut self,
        asset: &Self::Asset,
        debt_total: &Self::Amount,
        cover_rate_minimum: Self::CoverRateMinimum,
        loan_scale: Self::Scale,
    ) -> Self::Amount;
}

pub fn compute_loan_pay_cover_threshold_facts<Sink>(
    sink: &mut Sink,
    cover_available: &Sink::Amount,
    asset: &Sink::Asset,
    debt_total: &Sink::Amount,
    cover_rate_minimum: Sink::CoverRateMinimum,
    loan_scale: Sink::Scale,
) -> LoanPayCoverThresholdFacts<Sink::Amount, Sink::Asset, Sink::CoverRateMinimum, Sink::Scale>
where
    Sink: LoanPayCoverThresholdSink,
    Sink::Amount: Clone + PartialOrd,
    Sink::Asset: Clone,
    Sink::CoverRateMinimum: Clone,
    Sink::Scale: Clone,
{
    let required_cover = sink.compute_required_cover_threshold(
        asset,
        debt_total,
        cover_rate_minimum.clone(),
        loan_scale.clone(),
    );

    LoanPayCoverThresholdFacts {
        cover_available: cover_available.clone(),
        asset: asset.clone(),
        debt_total: debt_total.clone(),
        cover_rate_minimum,
        loan_scale,
        required_cover: required_cover.clone(),
        cover_available_meets_minimum: cover_available >= &required_cover,
    }
}

#[cfg(test)]
mod tests {
    use super::{LoanPayCoverThresholdSink, compute_loan_pay_cover_threshold_facts};

    #[derive(Default)]
    struct TestSink {
        calls: Vec<String>,
    }

    impl LoanPayCoverThresholdSink for TestSink {
        type Amount = i64;
        type Asset = &'static str;
        type CoverRateMinimum = u32;
        type Scale = i32;

        fn compute_required_cover_threshold(
            &mut self,
            asset: &Self::Asset,
            debt_total: &Self::Amount,
            cover_rate_minimum: Self::CoverRateMinimum,
            loan_scale: Self::Scale,
        ) -> Self::Amount {
            self.calls.push(format!(
                "compute_required_cover_threshold asset={asset} debt_total={debt_total} rate={cover_rate_minimum} scale={loan_scale}"
            ));
            (*debt_total * i64::from(cover_rate_minimum)) + i64::from(loan_scale)
        }
    }

    #[test]
    fn loan_pay_cover_threshold_allows_equal_cover() {
        let mut sink = TestSink::default();
        let facts = compute_loan_pay_cover_threshold_facts(
            &mut sink, &204_i64, &"USD", &100_i64, 2_u32, 4_i32,
        );

        assert_eq!(facts.cover_available, 204);
        assert_eq!(facts.asset, "USD");
        assert_eq!(facts.debt_total, 100);
        assert_eq!(facts.cover_rate_minimum, 2);
        assert_eq!(facts.loan_scale, 4);
        assert_eq!(facts.required_cover, 204);
        assert!(facts.cover_available_meets_minimum);
        assert_eq!(
            sink.calls,
            vec!["compute_required_cover_threshold asset=USD debt_total=100 rate=2 scale=4"]
        );
    }

    #[test]
    fn loan_pay_cover_threshold_allows_more_cover() {
        let mut sink = TestSink::default();
        let facts = compute_loan_pay_cover_threshold_facts(
            &mut sink, &205_i64, &"USD", &100_i64, 2_u32, 4_i32,
        );

        assert!(facts.cover_available_meets_minimum);
    }

    #[test]
    fn loan_pay_cover_threshold_rejects_insufficient_cover() {
        let mut sink = TestSink::default();
        let facts = compute_loan_pay_cover_threshold_facts(
            &mut sink, &203_i64, &"USD", &100_i64, 2_u32, 4_i32,
        );

        assert!(!facts.cover_available_meets_minimum);
    }
}
