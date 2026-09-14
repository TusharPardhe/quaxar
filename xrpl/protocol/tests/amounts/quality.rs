use basics::number::{NumberParts as RuntimeNumber, RoundingMode};
use protocol::{
    AccountID, Amounts, Asset, CurrentTransactionRulesGuard, MPTIssue, Quality, QualityFunction,
    QualityFunctionAmmTag, QualityFunctionClobLikeTag, Rules, STAmount, StBase, XRPAmount,
    div_round, feature_id, make_mpt_id, mul_round, no_issue, sf_generic, to_amount_from_number,
    xrp_issue,
};

fn issue_amount(mantissa: u64, exponent: i32) -> STAmount {
    STAmount::new_with_asset(sf_generic(), no_issue(), mantissa, exponent, false)
}

#[test]
fn protocol_quality_function_clob_like_matches_current_cpp_constant_quality_shape() {
    let quality = Quality::from_amounts(&Amounts::new(issue_amount(1, 0), issue_amount(2, 0)));
    let function = QualityFunction::from_quality(quality, QualityFunctionClobLikeTag);

    assert!(function.is_const());
    assert_eq!(function.quality(), Some(quality));
    assert_eq!(function.slope(), RuntimeNumber::zero());
    assert_eq!(function.intercept().to_string(), "2");
    assert_eq!(function.out_from_avg_q(quality), None);
}

#[test]
fn protocol_quality_function_amm_formula_matches_current_cpp_out_limit_math() {
    let function = QualityFunction::from_amm(
        &Amounts::new(issue_amount(10, 0), issue_amount(20, 0)),
        0,
        QualityFunctionAmmTag,
    );
    let requested_quality =
        Quality::from_amounts(&Amounts::new(issue_amount(2, 0), issue_amount(3, 0)));

    assert!(!function.is_const());
    assert_eq!(function.quality(), None);
    assert_eq!(function.slope().to_string(), "-0.1");
    assert_eq!(function.intercept().to_string(), "2");
    assert_eq!(
        function
            .out_from_avg_q(requested_quality)
            .map(|value| value.to_string()),
        Some("5.00000000000000075".to_string())
    );
    assert!(function.satisfies_avg_q(requested_quality, RuntimeNumber::from_i64(5)));
    assert!(!function.satisfies_avg_q(requested_quality, RuntimeNumber::from_i64(6)));
}

#[test]
fn protocol_quality_function_combine_matches_current_cpp_affine_composition() {
    let mut function = QualityFunction::from_quality(
        Quality::from_amounts(&Amounts::new(issue_amount(1, 0), issue_amount(2, 0))),
        QualityFunctionClobLikeTag,
    );
    let next = QualityFunction::from_amm(
        &Amounts::new(issue_amount(10, 0), issue_amount(20, 0)),
        0,
        QualityFunctionAmmTag,
    );

    function.combine(&next);

    assert!(!function.is_const());
    assert_eq!(function.quality(), None);
    assert_eq!(function.slope().to_string(), "-0.2");
    assert_eq!(function.intercept().to_string(), "4");
    assert_eq!(
        function
            .out_from_avg_q(Quality::from_amounts(&Amounts::new(
                issue_amount(1, 0),
                issue_amount(3, 0),
            )))
            .map(|value| value.to_string()),
        Some("4.9999999999999985".to_string())
    );
}

#[test]
fn integral_amount_conversion_honors_directed_rounding_for_xrp_and_mpt() {
    let one_and_a_half = RuntimeNumber::from_i64(3) / RuntimeNumber::from_i64(2);
    let xrp = Asset::Issue(xrp_issue());
    let mpt = Asset::MPTIssue(MPTIssue::new(make_mpt_id(
        7,
        AccountID::from_array([7; 20]),
    )));

    let xrp_down: STAmount =
        to_amount_from_number(xrp, one_and_a_half, RoundingMode::Downward).expect("XRP floor");
    let xrp_up: STAmount =
        to_amount_from_number(xrp, one_and_a_half, RoundingMode::Upward).expect("XRP ceiling");
    assert_eq!(
        xrp_down,
        STAmount::from_xrp_amount(XRPAmount::from_drops(1))
    );
    assert_eq!(xrp_up, STAmount::from_xrp_amount(XRPAmount::from_drops(2)));

    let mpt_down: STAmount =
        to_amount_from_number(mpt, one_and_a_half, RoundingMode::Downward).expect("MPT floor");
    let mpt_up: STAmount =
        to_amount_from_number(mpt, one_and_a_half, RoundingMode::Upward).expect("MPT ceiling");
    assert_eq!(mpt_down.mpt().value(), 1);
    assert_eq!(mpt_up.mpt().value(), 2);

    // IOUs remain fractional; conversion direction must not quantize them.
    let iou_down: STAmount = to_amount_from_number(
        Asset::Issue(no_issue()),
        one_and_a_half,
        RoundingMode::Downward,
    )
    .expect("IOU floor conversion");
    let iou_up: STAmount = to_amount_from_number(
        Asset::Issue(no_issue()),
        one_and_a_half,
        RoundingMode::Upward,
    )
    .expect("IOU ceiling conversion");
    assert_eq!(iou_down.text(), "1.5");
    assert_eq!(iou_up.text(), "1.5");
}

#[test]
fn mpt_mul_and_div_round_use_number_directed_rounding_only_with_mptokens_v2() {
    let asset = Asset::MPTIssue(MPTIssue::new(make_mpt_id(
        7,
        AccountID::from_array([7; 20]),
    )));
    let amount = STAmount::new_with_asset(sf_generic(), asset, 10_000, 0, false);
    let legacy_mul_input = STAmount::new_with_asset(sf_generic(), asset, 9_990, 0, false);
    let rate = STAmount::new_with_asset(sf_generic(), no_issue(), 1_001_000_000, -9, false);

    for (rules, expected_div_down, expected_mul_down, expected_div_up, expected_mul_up) in [
        (Rules::new([]), 9_990, 10_000, 9_990, 10_000),
        (
            Rules::new([feature_id("MPTokensV2")]),
            9_990,
            9_999,
            9_991,
            10_000,
        ),
    ] {
        let _rules = CurrentTransactionRulesGuard::new(rules);

        assert_eq!(
            div_round(&amount, &rate, asset, false).mpt().value(),
            expected_div_down,
            "downward division must preserve the amendment-specific semantics"
        );
        assert_eq!(
            mul_round(&legacy_mul_input, &rate, asset, false)
                .mpt()
                .value(),
            expected_mul_down,
            "downward multiplication must preserve the amendment-specific semantics"
        );
        assert_eq!(
            div_round(&amount, &rate, asset, true).mpt().value(),
            expected_div_up,
            "upward division must retain the amendment-specific semantics"
        );
        assert_eq!(
            mul_round(&legacy_mul_input, &rate, asset, true)
                .mpt()
                .value(),
            expected_mul_up,
            "upward multiplication must retain the amendment-specific semantics"
        );
    }
}
