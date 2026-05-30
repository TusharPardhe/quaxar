use basics::number::NumberParts as RuntimeNumber;
use protocol::{
    Amounts, Quality, QualityFunction, QualityFunctionAmmTag, QualityFunctionClobLikeTag, STAmount,
    no_issue, sf_generic,
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
