//! the reference implementation loan-property seams needed by `LoanSet::doApply()`.
//!
//! This module ports the deterministic math and rounding behavior around:
//!
//! - `loanPeriodicRate(...)`,
//! - `detail::computePaymentFactor(...)`,
//! - `detail::loanPeriodicPayment(...)`,
//! - `detail::loanPrincipalFromPeriodicPayment(...)`,
//! - `computeManagementFee(...)`,
//! - `constructLoanState(...)`, and
//! - `computeLoanProperties(...)`.

use basics::number::{NumberParts as RuntimeNumber, NumberRoundModeGuard, RoundingMode, power};
use protocol::{
    Asset, IOUAmount, MPTAmount, Rules, STAmount, TenthBips16, TenthBips32, XRPAmount, feature_id,
    to_amount_from_number,
};

const SECONDS_IN_YEAR: i64 = 31_536_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetLoanState<Amount> {
    pub value_outstanding: Amount,
    pub principal_outstanding: Amount,
    pub interest_due: Amount,
    pub management_fee_due: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetLoanProperties<Amount> {
    pub periodic_payment: Amount,
    pub loan_state: LoanSetLoanState<Amount>,
    pub loan_scale: i32,
    pub first_payment_principal: Amount,
}

fn round_to_asset(
    asset: &Asset,
    value: RuntimeNumber,
    scale: i32,
    mode: RoundingMode,
) -> RuntimeNumber {
    let _guard = NumberRoundModeGuard::new(mode);
    let amount = match asset {
        Asset::Issue(issue) if issue.native() => {
            let amount: XRPAmount =
                to_amount_from_number(Asset::Issue(*issue), value, mode).expect("xrp rounding");
            RuntimeNumber::from(amount)
        }
        Asset::Issue(issue) => {
            let amount: IOUAmount =
                to_amount_from_number(Asset::Issue(*issue), value, mode).expect("iou rounding");
            RuntimeNumber::from(amount)
        }
        Asset::MPTIssue(issue) => {
            let amount: MPTAmount =
                to_amount_from_number(Asset::MPTIssue(*issue), value, mode).expect("mpt rounding");
            RuntimeNumber::from(amount)
        }
    };
    if scale <= amount.exponent {
        return amount;
    }

    let st_amount: STAmount =
        to_amount_from_number(*asset, amount, mode).expect("rounded amount should encode");
    let exponent = st_amount.exponent().max(scale);
    round_runtime_to_scale(amount, exponent, mode)
}

fn round_runtime_to_scale(
    value: RuntimeNumber,
    target_scale: i32,
    rounding: RoundingMode,
) -> RuntimeNumber {
    let Ok((mantissa, mut exponent)) = value.external_parts() else {
        return value;
    };
    if mantissa == 0 || exponent >= target_scale {
        return value;
    }

    let negative = mantissa < 0;
    let mut abs = mantissa.unsigned_abs() as u128;
    let mut removed = Vec::new();
    while exponent < target_scale {
        removed.push((abs % 10) as u8);
        abs /= 10;
        exponent += 1;
    }

    let first = removed.first().copied().unwrap_or(0);
    let has_more = removed.iter().skip(1).any(|digit| *digit != 0);
    let round_up = match rounding {
        RoundingMode::TowardsZero => false,
        RoundingMode::Downward => negative && (first != 0 || has_more),
        RoundingMode::Upward => !negative && (first != 0 || has_more),
        RoundingMode::ToNearest => {
            first > 5 || (first == 5 && (has_more || ((abs as u64) & 1) == 1))
        }
    };
    if round_up {
        abs += 1;
    }

    let signed = if negative { -(abs as i64) } else { abs as i64 };
    RuntimeNumber::from_i64_and_exponent(signed, exponent)
}

pub fn loan_set_periodic_rate(interest_rate: TenthBips32, payment_interval: u32) -> RuntimeNumber {
    RuntimeNumber::from_i64(i64::from(payment_interval))
        * RuntimeNumber::from_i64(i64::from(interest_rate.value()))
        / RuntimeNumber::from_i64(100_000)
        / RuntimeNumber::from_i64(SECONDS_IN_YEAR)
}

fn compute_power_minus_one(periodic_rate: RuntimeNumber, payments_remaining: u32) -> RuntimeNumber {
    if payments_remaining == 0 || periodic_rate == RuntimeNumber::zero() {
        return RuntimeNumber::zero();
    }

    let mut term = RuntimeNumber::from_i64(i64::from(payments_remaining)) * periodic_rate;
    let mut sum = term;
    for k in 1..payments_remaining {
        term = term * periodic_rate * RuntimeNumber::from_i64(i64::from(payments_remaining - k))
            / RuntimeNumber::from_i64(i64::from(k + 1));
        let next = sum + term;
        if next == sum {
            break;
        }
        sum = next;
    }
    sum
}

fn compute_power_minus_one_hybrid(
    periodic_rate: RuntimeNumber,
    payments_remaining: u32,
) -> RuntimeNumber {
    if payments_remaining == 0 || periodic_rate == RuntimeNumber::zero() {
        return RuntimeNumber::zero();
    }

    let cancellation_threshold = RuntimeNumber::from_i64_and_exponent(1, -9);
    if RuntimeNumber::from_i64(i64::from(payments_remaining)) * periodic_rate
        >= cancellation_threshold
    {
        return power(
            RuntimeNumber::from_i64(1) + periodic_rate,
            payments_remaining,
        )
        .expect("power should stay within Number range")
            - RuntimeNumber::from_i64(1);
    }

    compute_power_minus_one(periodic_rate, payments_remaining)
}

fn compute_payment_factor(
    rules: &Rules,
    periodic_rate: RuntimeNumber,
    payments_remaining: u32,
) -> RuntimeNumber {
    if payments_remaining == 0 {
        return RuntimeNumber::zero();
    }
    if periodic_rate == RuntimeNumber::zero() {
        return RuntimeNumber::from_i64(1) / RuntimeNumber::from_i64(i64::from(payments_remaining));
    }

    if rules.enabled(&feature_id("fixCleanup3_2_0")) {
        let raised_rate_minus_one =
            compute_power_minus_one_hybrid(periodic_rate, payments_remaining);
        let raised_rate = RuntimeNumber::from_i64(1) + raised_rate_minus_one;
        return (periodic_rate * raised_rate) / raised_rate_minus_one;
    }

    let raised_rate = power(
        RuntimeNumber::from_i64(1) + periodic_rate,
        payments_remaining,
    )
    .expect("power should stay within Number range");
    (periodic_rate * raised_rate) / (raised_rate - RuntimeNumber::from_i64(1))
}

pub fn loan_periodic_payment(
    rules: &Rules,
    principal_outstanding: RuntimeNumber,
    periodic_rate: RuntimeNumber,
    payments_remaining: u32,
) -> RuntimeNumber {
    if principal_outstanding == RuntimeNumber::zero() || payments_remaining == 0 {
        return RuntimeNumber::zero();
    }
    if periodic_rate == RuntimeNumber::zero() {
        return principal_outstanding / RuntimeNumber::from_i64(i64::from(payments_remaining));
    }
    principal_outstanding * compute_payment_factor(rules, periodic_rate, payments_remaining)
}

pub fn loan_principal_from_periodic_payment(
    rules: &Rules,
    periodic_payment: RuntimeNumber,
    periodic_rate: RuntimeNumber,
    payments_remaining: u32,
) -> RuntimeNumber {
    if payments_remaining == 0 {
        return RuntimeNumber::zero();
    }
    if periodic_rate == RuntimeNumber::zero() {
        return periodic_payment * RuntimeNumber::from_i64(i64::from(payments_remaining));
    }
    periodic_payment / compute_payment_factor(rules, periodic_rate, payments_remaining)
}

pub fn construct_loan_set_state(
    total_value_outstanding: RuntimeNumber,
    principal_outstanding: RuntimeNumber,
    management_fee_outstanding: RuntimeNumber,
) -> LoanSetLoanState<RuntimeNumber> {
    LoanSetLoanState {
        value_outstanding: total_value_outstanding,
        principal_outstanding,
        interest_due: total_value_outstanding - principal_outstanding - management_fee_outstanding,
        management_fee_due: management_fee_outstanding,
    }
}

pub fn compute_theoretical_loan_state(
    rules: &Rules,
    asset: Asset,
    periodic_payment: RuntimeNumber,
    periodic_rate: RuntimeNumber,
    payment_remaining: u32,
    management_fee_rate: TenthBips16,
) -> LoanSetLoanState<RuntimeNumber> {
    if payment_remaining == 0 {
        return construct_loan_set_state(
            RuntimeNumber::zero(),
            RuntimeNumber::zero(),
            RuntimeNumber::zero(),
        );
    }

    let total_value_outstanding =
        periodic_payment * RuntimeNumber::from_i64(i64::from(payment_remaining));
    let principal_outstanding = loan_principal_from_periodic_payment(
        rules,
        periodic_payment,
        periodic_rate,
        payment_remaining,
    );
    let interest_outstanding_gross = total_value_outstanding - principal_outstanding;
    let management_fee_outstanding = compute_management_fee(
        asset,
        interest_outstanding_gross,
        management_fee_rate,
        interest_outstanding_gross.exponent,
    );
    construct_loan_set_state(
        total_value_outstanding,
        principal_outstanding,
        management_fee_outstanding,
    )
}

pub fn compute_management_fee(
    asset: Asset,
    value: RuntimeNumber,
    management_fee_rate: TenthBips16,
    scale: i32,
) -> RuntimeNumber {
    round_to_asset(
        &asset,
        value * RuntimeNumber::from_i64(i64::from(management_fee_rate.value()))
            / RuntimeNumber::from_i64(100_000),
        scale,
        RoundingMode::Downward,
    )
}

pub fn compute_loan_set_properties(
    rules: &Rules,
    asset: Asset,
    principal_outstanding: RuntimeNumber,
    interest_rate: TenthBips32,
    payment_interval: u32,
    payments_remaining: u32,
    management_fee_rate: TenthBips16,
    minimum_scale: i32,
) -> LoanSetLoanProperties<RuntimeNumber> {
    let periodic_rate = loan_set_periodic_rate(interest_rate, payment_interval);
    let periodic_payment = loan_periodic_payment(
        rules,
        principal_outstanding,
        periodic_rate,
        payments_remaining,
    );

    let (total_value_outstanding, loan_scale) = {
        let rounding = if periodic_rate == RuntimeNumber::zero() {
            RoundingMode::ToNearest
        } else {
            RoundingMode::Upward
        };
        let total_unrounded =
            periodic_payment * RuntimeNumber::from_i64(i64::from(payments_remaining));
        let total_amount: STAmount =
            to_amount_from_number(asset, total_unrounded, rounding).expect("total value encode");
        let loan_scale = minimum_scale.max(total_amount.exponent());
        let total_rounded = round_to_asset(&asset, total_unrounded, loan_scale, rounding);
        (total_rounded, loan_scale)
    };

    let rounded_principal_outstanding = round_to_asset(
        &asset,
        principal_outstanding,
        loan_scale,
        RoundingMode::ToNearest,
    );
    let total_interest_outstanding = total_value_outstanding - rounded_principal_outstanding;
    let fee_owed_to_broker = compute_management_fee(
        asset,
        total_interest_outstanding,
        management_fee_rate,
        loan_scale,
    );

    let starting_state = compute_theoretical_loan_state(
        rules,
        asset,
        periodic_payment,
        periodic_rate,
        payments_remaining,
        management_fee_rate,
    );
    let first_payment_state = compute_theoretical_loan_state(
        rules,
        asset,
        periodic_payment,
        periodic_rate,
        payments_remaining.saturating_sub(1),
        management_fee_rate,
    );
    let first_payment_principal =
        starting_state.principal_outstanding - first_payment_state.principal_outstanding;

    LoanSetLoanProperties {
        periodic_payment,
        loan_state: construct_loan_set_state(
            total_value_outstanding,
            rounded_principal_outstanding,
            fee_owed_to_broker,
        ),
        loan_scale,
        first_payment_principal,
    }
}

#[cfg(test)]
mod tests {
    use basics::number::{NumberParts as RuntimeNumber, power};
    use protocol::{
        Asset, Issue, Rules, TenthBips16, TenthBips32, feature_id, percentage_to_tenth_bips,
        xrp_issue,
    };

    use super::{
        LoanSetLoanState, compute_loan_set_properties, compute_power_minus_one,
        compute_power_minus_one_hybrid, compute_theoretical_loan_state, construct_loan_set_state,
        loan_periodic_payment, loan_principal_from_periodic_payment, loan_set_periodic_rate,
    };

    fn no_features() -> Rules {
        Rules::new(std::iter::empty())
    }

    fn cleanup_3_2_0() -> Rules {
        Rules::new([feature_id("fixCleanup3_2_0")])
    }

    #[test]
    fn construct_loan_set_state_derives_interest_from_total_principal_and_fee() {
        let state = construct_loan_set_state(
            RuntimeNumber::from_i64(120),
            RuntimeNumber::from_i64(100),
            RuntimeNumber::from_i64(5),
        );

        assert_eq!(
            state,
            LoanSetLoanState {
                value_outstanding: RuntimeNumber::from_i64(120),
                principal_outstanding: RuntimeNumber::from_i64(100),
                interest_due: RuntimeNumber::from_i64(15),
                management_fee_due: RuntimeNumber::from_i64(5),
            }
        );
    }

    #[test]
    fn loan_set_periodic_rate_prorates_annual_rate_by_interval() {
        let rate = loan_set_periodic_rate(percentage_to_tenth_bips(12), 2_628_000);
        let expected = RuntimeNumber::from_i64(1) / RuntimeNumber::from_i64(100);
        assert_eq!(rate, expected);
    }

    #[test]
    fn compute_loan_set_properties_zero_interest_uses_equal_payments() {
        let asset = Asset::Issue(xrp_issue());
        let props = compute_loan_set_properties(
            &no_features(),
            asset,
            RuntimeNumber::from_i64(120),
            TenthBips32::new(0),
            30,
            12,
            TenthBips16::new(0),
            0,
        );

        assert_eq!(props.periodic_payment, RuntimeNumber::from_i64(10));
        assert_eq!(
            props.loan_state.value_outstanding,
            RuntimeNumber::from_i64(120)
        );
        assert_eq!(
            props.loan_state.principal_outstanding,
            RuntimeNumber::from_i64(120)
        );
        assert_eq!(props.loan_state.interest_due, RuntimeNumber::zero());
        assert_eq!(props.loan_state.management_fee_due, RuntimeNumber::zero());
        assert_eq!(props.loan_scale, 0);
        assert_eq!(props.first_payment_principal, RuntimeNumber::from_i64(10));
    }

    #[test]
    fn compute_loan_set_properties_interest_bearing_loan_keeps_positive_components() {
        let asset = Asset::Issue(Issue::new(
            protocol::Currency::from_array([1; 20]),
            protocol::AccountID::from_array([2; 20]),
        ));
        let props = compute_loan_set_properties(
            &no_features(),
            asset,
            RuntimeNumber::from_i64(1_000),
            percentage_to_tenth_bips(12),
            2_628_000,
            12,
            TenthBips16::new(500),
            -6,
        );

        assert!(props.periodic_payment > RuntimeNumber::zero());
        assert!(props.loan_state.value_outstanding > props.loan_state.principal_outstanding);
        assert!(props.loan_state.interest_due >= RuntimeNumber::zero());
        assert!(props.loan_state.management_fee_due >= RuntimeNumber::zero());
        assert!(props.first_payment_principal > RuntimeNumber::zero());
        assert!(props.loan_scale >= -6);
    }

    #[test]
    fn compute_power_minus_one_hybrid_matches_upstream_threshold_routing() {
        let above_rate = RuntimeNumber::from_i64_and_exponent(5, -2);
        let above_n = 3;
        let closed = power(RuntimeNumber::from_i64(1) + above_rate, above_n)
            .expect("closed form should fit")
            - RuntimeNumber::from_i64(1);
        assert_eq!(compute_power_minus_one_hybrid(above_rate, above_n), closed);

        let below_rate = loan_set_periodic_rate(TenthBips32::new(1), 600);
        let below_n = 2;
        assert_eq!(
            compute_power_minus_one_hybrid(below_rate, below_n),
            compute_power_minus_one(below_rate, below_n)
        );

        let boundary_rate = RuntimeNumber::from_i64_and_exponent(1, -12);
        let boundary_n = 1_000;
        let boundary_closed = power(RuntimeNumber::from_i64(1) + boundary_rate, boundary_n)
            .expect("boundary closed form should fit")
            - RuntimeNumber::from_i64(1);
        assert_eq!(
            compute_power_minus_one_hybrid(boundary_rate, boundary_n),
            boundary_closed
        );
    }

    #[test]
    fn loan_principal_from_periodic_payment_near_zero_rate_respects_payment_bound() {
        let rules = cleanup_3_2_0();
        let periodic_rate = loan_set_periodic_rate(TenthBips32::new(1), 600);
        let periodic_payment =
            loan_periodic_payment(&rules, RuntimeNumber::from_i64(100), periodic_rate, 3);

        for payments_remaining in [3, 2, 1] {
            let computed = loan_principal_from_periodic_payment(
                &rules,
                periodic_payment,
                periodic_rate,
                payments_remaining,
            );
            let upper_bound =
                periodic_payment * RuntimeNumber::from_i64(i64::from(payments_remaining));
            assert!(
                computed <= upper_bound,
                "payments_remaining={payments_remaining}: principal={computed:?} upper_bound={upper_bound:?}"
            );
        }
    }

    #[test]
    fn compute_theoretical_loan_state_near_zero_rate_has_non_negative_interest_due() {
        let rules = cleanup_3_2_0();
        let periodic_rate = loan_set_periodic_rate(TenthBips32::new(1), 600);
        let periodic_payment =
            loan_periodic_payment(&rules, RuntimeNumber::from_i64(100), periodic_rate, 3);
        let state = compute_theoretical_loan_state(
            &rules,
            Asset::Issue(xrp_issue()),
            periodic_payment,
            periodic_rate,
            2,
            TenthBips16::new(0),
        );

        assert!(state.principal_outstanding <= state.value_outstanding);
        assert!(state.interest_due >= RuntimeNumber::zero());
        assert_eq!(state.management_fee_due, RuntimeNumber::zero());
    }
}
