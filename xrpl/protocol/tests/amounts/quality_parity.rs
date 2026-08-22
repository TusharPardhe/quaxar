use basics::number::{MantissaScale, NumberMantissaScaleGuard};
use protocol::{
    Amounts, Quality, STAmount, StBase, amount_from_quality, composed_quality, divide, get_rate,
    multiply, no_issue, sf_generic,
};

fn native_amount(value: u64) -> STAmount {
    STAmount::new_native(value, false)
}

fn issue_amount(mantissa: u64, exponent: i32) -> STAmount {
    STAmount::new_with_asset(sf_generic(), no_issue(), mantissa, exponent, false)
}

#[test]
fn stamount_rate_vectors_match_cpp_examples() {
    let expected_low = ((100u64 - 14) << 56) | 1_000_000_000_000_000u64;
    let expected_high = ((100u64 - 16) << 56) | 1_000_000_000_000_000u64;

    assert_eq!(
        get_rate(&native_amount(1), &native_amount(10)),
        expected_low
    );
    assert_eq!(
        get_rate(&native_amount(10), &native_amount(1)),
        expected_high
    );
    assert_eq!(
        get_rate(&issue_amount(1, 0), &issue_amount(10, 0)),
        expected_low
    );
    assert_eq!(
        get_rate(&issue_amount(10, 0), &issue_amount(1, 0)),
        expected_high
    );
    assert_eq!(
        get_rate(&issue_amount(1, 0), &native_amount(10)),
        expected_low
    );
    assert_eq!(
        get_rate(&native_amount(10), &issue_amount(1, 0)),
        expected_high
    );
}

#[test]
fn amount_from_quality_round_trips_through_divide() {
    let a1 = issue_amount(60, 0);
    let a2 = issue_amount(10, -1);

    assert_eq!(
        divide(&a2, &a1, no_issue()),
        amount_from_quality(get_rate(&a1, &a2))
    );
    assert_eq!(
        divide(&a1, &a2, no_issue()),
        amount_from_quality(get_rate(&a2, &a1))
    );
}

#[test]
fn quality_math_underflow_zero_behavior() {
    let big_value = issue_amount(5_499_999_999_999_999, 79);
    let small_value = issue_amount(5_499_999_999_999_999, -95);

    assert_eq!(multiply(&small_value, &small_value, no_issue()).signum(), 0);
    assert_eq!(divide(&small_value, &big_value, no_issue()).signum(), 0);
    assert_eq!(get_rate(&small_value, &big_value), 0);
    assert_eq!(get_rate(&big_value, &small_value), 0);
}

#[test]
fn quality_comparisons_composition_and_increment_match_cpp() {
    let amount1 = issue_amount(231, 0);
    let amount2 = issue_amount(462, 0);
    let amount3 = issue_amount(924, 0);

    let q11 = Quality::from_amounts(&Amounts::new(amount1.clone(), amount1.clone()));
    let q12 = Quality::from_amounts(&Amounts::new(amount1.clone(), amount2.clone()));
    let q13 = Quality::from_amounts(&Amounts::new(amount1.clone(), amount3.clone()));
    let q21 = Quality::from_amounts(&Amounts::new(amount2.clone(), amount1.clone()));
    let q31 = Quality::from_amounts(&Amounts::new(amount3.clone(), amount1.clone()));

    assert_eq!(q11, q11);
    assert!(q11 < q12);
    assert!(q12 < q13);
    assert!(q31 < q21);
    assert!(q21 < q11);
    assert_eq!(composed_quality(q12, q21), q11);
    assert_eq!(composed_quality(q13, q31), q11);
    assert_eq!(composed_quality(q31, q13), q11);

    let mut qa = q11;
    let mut qb = q11;
    qa.increment();
    qb.decrement();
    assert_ne!(qa, q11);
    assert_ne!(qb, q11);
    assert!(qb < qa);
    qb.increment();
    qb.increment();
    qb.increment();
    assert!(qa < qb);
}

#[test]
fn quality_ceil_and_round_examples_match_cpp() {
    let q11 = Quality::from_amounts(&Amounts::new(issue_amount(1, 0), issue_amount(1, 0)));
    let q12 = Quality::from_amounts(&Amounts::new(issue_amount(1, 0), issue_amount(2, 0)));
    let q21 = Quality::from_amounts(&Amounts::new(issue_amount(2, 0), issue_amount(1, 0)));

    assert_eq!(
        q11.ceil_in(
            &Amounts::new(issue_amount(10, 0), issue_amount(10, 0)),
            &issue_amount(5, 0)
        ),
        Amounts::new(issue_amount(5, 0), issue_amount(5, 0))
    );
    assert_eq!(
        q12.ceil_in(
            &Amounts::new(issue_amount(40, 0), issue_amount(80, 0)),
            &issue_amount(20, 0)
        ),
        Amounts::new(issue_amount(20, 0), issue_amount(40, 0))
    );
    assert_eq!(
        q21.ceil_out(
            &Amounts::new(issue_amount(40, 0), issue_amount(20, 0)),
            &issue_amount(10, 0)
        ),
        Amounts::new(issue_amount(20, 0), issue_amount(10, 0))
    );

    let q = Quality::from_value(0x5914_8191_fb91_3522);
    assert_eq!(q.round(3).rate().text(), "57800");
    assert_eq!(q.round(4).rate().text(), "57720");
    assert_eq!(q.round(8).rate().text(), "57719.636");
    assert_eq!(q.round(16).rate().text(), "57719.63525051682");
}

#[test]
fn number_multiply_matches_live_offer_tick_size_rounding() {
    // Testnet ledger 20,119,713, transaction B77F6E16...: a TickSize=6
    // tfSell OfferCreate rounds its XRP/2RY rate before placing the residual.
    // rippled stores exactly 20.440625.  The legacy scaled-mantissa `+7`
    // multiply produced 20.44062500000001 and the adjacent BookDirectory.
    let taker_gets = native_amount(1_250_000_000);
    let taker_pays = issue_amount(2_044_059_100_000_000, -14);
    let rounded_quality = Quality::from_value(get_rate(&taker_gets, &taker_pays)).round(6);
    assert_eq!(rounded_quality.value(), 0x4d05_cf40_4f5e_7400);
    let rounded_rate = rounded_quality.rate();

    let rounded_pays = multiply(&taker_gets, &rounded_rate, no_issue());
    assert_eq!(rounded_pays.text(), "20.440625");
    assert_eq!(rounded_pays.mantissa(), 2_044_062_500_000_000);
    assert_eq!(rounded_pays.exponent(), -14);
    assert_eq!(get_rate(&taker_gets, &rounded_pays), 0x4d05_cf40_4f5e_7400);
}

#[test]
fn number_multiply_matches_live_offer_tick_size_native_output() {
    // Testnet ledger 20,120,246, transaction 4F741FC8... is the reverse
    // tfSell orientation: issued input rounded at TickSize=6 into XRP output.
    let taker_gets = issue_amount(2_004_744_700_000_000, -14);
    let taker_pays = native_amount(1_250_000_000);
    let rounded_quality = Quality::from_value(get_rate(&taker_gets, &taker_pays)).round(6);
    assert_eq!(rounded_quality.value(), 0x5c16_26e3_f526_6400);

    for scale in [MantissaScale::Small, MantissaScale::Large] {
        let _scale = NumberMantissaScaleGuard::new(scale);
        let rounded_pays = multiply(&taker_gets, &rounded_quality.rate(), taker_pays.asset());
        assert_eq!(rounded_pays.xrp().drops(), 1_250_000_420, "{scale:?}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Direct ports from C++ Quality_test.cpp
// ═══════════════════════════════════════════════════════════════════════════════

// --- testcase("comparisons") ---

#[test]
fn cpp_quality_comparisons() {
    let amount1 = STAmount::new_native(231, false);
    let amount2 = STAmount::new_native(462, false);
    let amount3 = STAmount::new_native(924, false);

    let q11 = Quality::from_amounts(&Amounts::new(amount1.clone(), amount1.clone()));
    let q12 = Quality::from_amounts(&Amounts::new(amount1.clone(), amount2.clone()));
    let q13 = Quality::from_amounts(&Amounts::new(amount1.clone(), amount3.clone()));
    let q21 = Quality::from_amounts(&Amounts::new(amount2.clone(), amount1.clone()));
    let q31 = Quality::from_amounts(&Amounts::new(amount3.clone(), amount1.clone()));

    assert_eq!(q11, q11);
    assert!(q11 < q12);
    assert!(q12 < q13);
    assert!(q31 < q21);
    assert!(q21 < q11);
    assert!(q12 > q11);
    assert!(q13 > q12);
    assert!(q21 > q31);
    assert!(q11 > q21);
    assert_ne!(q31, q21);
}

// --- testcase("composition") ---

#[test]
fn cpp_quality_composition() {
    let amount1 = STAmount::new_native(231, false);
    let amount2 = STAmount::new_native(462, false);
    let amount3 = STAmount::new_native(924, false);

    let q11 = Quality::from_amounts(&Amounts::new(amount1.clone(), amount1.clone()));
    let q12 = Quality::from_amounts(&Amounts::new(amount1.clone(), amount2.clone()));
    let q21 = Quality::from_amounts(&Amounts::new(amount2.clone(), amount1.clone()));
    let q13 = Quality::from_amounts(&Amounts::new(amount1.clone(), amount3.clone()));
    let q31 = Quality::from_amounts(&Amounts::new(amount3.clone(), amount1.clone()));

    assert_eq!(composed_quality(q12, q21), q11);
    let q1331 = composed_quality(q13, q31);
    let q3113 = composed_quality(q31, q13);
    assert_eq!(q1331, q3113);
    assert_eq!(q1331, q11);
}
