use basics::number::{
    MantissaScale, NumberMantissaScaleGuard, NumberParts as RuntimeNumber, NumberRoundModeGuard,
    RoundingMode,
};
use protocol::st_number::{STNumber, number_from_json_input};
use protocol::{
    NumberJsonInput, NumberParts, NumberPartsError, normalized_parts_from_json_input,
    normalized_parts_from_string, parts_from_json_input, parts_from_string,
};

#[test]
fn parts_from_string_matches_current_cpp_valid_shapes() {
    assert_eq!(
        parts_from_string("123"),
        Ok(NumberParts {
            mantissa: 123,
            exponent: 0,
            negative: false,
        })
    );
    assert_eq!(
        parts_from_string("-123"),
        Ok(NumberParts {
            mantissa: 123,
            exponent: 0,
            negative: true,
        })
    );
    assert_eq!(
        parts_from_string("3.14"),
        Ok(NumberParts {
            mantissa: 314,
            exponent: -2,
            negative: false,
        })
    );
    assert_eq!(
        parts_from_string("-3.14e2"),
        Ok(NumberParts {
            mantissa: 314,
            exponent: 0,
            negative: true,
        })
    );
    assert_eq!(
        parts_from_string("1000e-2"),
        Ok(NumberParts {
            mantissa: 1000,
            exponent: -2,
            negative: false,
        })
    );
    assert_eq!(
        parts_from_string("0e6"),
        Ok(NumberParts {
            mantissa: 0,
            exponent: 6,
            negative: false,
        })
    );
    assert_eq!(
        parts_from_string("0.0e6"),
        Ok(NumberParts {
            mantissa: 0,
            exponent: 5,
            negative: false,
        })
    );
    assert_eq!(
        parts_from_string("-0.000e6"),
        Ok(NumberParts {
            mantissa: 0,
            exponent: 3,
            negative: true,
        })
    );
    assert_eq!(
        parts_from_string("-0e6"),
        Ok(NumberParts {
            mantissa: 0,
            exponent: 6,
            negative: true,
        })
    );
}

#[test]
fn parts_from_string_matches_current_cpp_zero_forms() {
    for input in ["0", "-0", "+0"] {
        assert_eq!(
            parts_from_string(input),
            Ok(NumberParts {
                mantissa: 0,
                exponent: 0,
                negative: input.starts_with('-'),
            })
        );
    }

    for input in ["0.0", "-0.0", "+0.0"] {
        assert_eq!(
            parts_from_string(input),
            Ok(NumberParts {
                mantissa: 0,
                exponent: -1,
                negative: input.starts_with('-'),
            })
        );
    }
}

#[test]
fn parts_from_string_rejects_current_cpp_invalid_shapes() {
    for input in ["", "e", "1e", "e2", "001", "000.0", ".1", "1.", "1.e3"] {
        assert_eq!(
            parts_from_string(input),
            Err(NumberPartsError::NotANumber(input.to_owned()))
        );
    }
}

#[test]
fn parts_from_string_reports_mantissa_overflow_for_valid_number_shapes() {
    assert_eq!(
        parts_from_string("18446744073709551616"),
        Err(NumberPartsError::MantissaOverflow)
    );
}

#[test]
fn parts_from_json_input_matches_current_cpp_front_half() {
    assert_eq!(
        parts_from_json_input(NumberJsonInput::Int(-42)),
        Ok(NumberParts {
            mantissa: 42,
            exponent: 0,
            negative: true,
        })
    );
    assert_eq!(
        parts_from_json_input(NumberJsonInput::UInt(42)),
        Ok(NumberParts {
            mantissa: 42,
            exponent: 0,
            negative: false,
        })
    );
    assert_eq!(
        parts_from_json_input(NumberJsonInput::String("3.14e2")),
        Ok(NumberParts {
            mantissa: 314,
            exponent: 0,
            negative: false,
        })
    );
    assert_eq!(
        parts_from_json_input(NumberJsonInput::Other),
        Err(NumberPartsError::NotANumber("not a number".to_owned()))
    );
}

#[test]
fn parts_from_json_input_extreme_integers() {
    assert_eq!(
        parts_from_json_input(NumberJsonInput::Int(i64::MAX)),
        Ok(NumberParts::from_unsigned_integer(i64::MAX as u64))
    );
    assert_eq!(
        parts_from_json_input(NumberJsonInput::Int(i64::MIN)),
        Ok(NumberParts {
            mantissa: 9_223_372_036_854_775_808,
            exponent: 0,
            negative: true,
        })
    );
    assert_eq!(
        parts_from_json_input(NumberJsonInput::UInt(u64::MAX)),
        Ok(NumberParts::from_unsigned_integer(u64::MAX))
    );
}

#[test]
fn number_parts_value_helpers_match_cpp_front_half() {
    assert_eq!(NumberParts::zero(), NumberParts::default());
    assert!(NumberParts::zero().is_zero());
    assert!(!NumberParts::from_signed_integer(-1).is_zero());
    assert_eq!(
        NumberParts::from_signed_integer(-42),
        NumberParts {
            mantissa: 42,
            exponent: 0,
            negative: true,
        }
    );
    assert_eq!(
        NumberParts::from_signed_integer(42),
        NumberParts::from_unsigned_integer(42)
    );
}

#[test]
fn normalized_parts_from_json_matches_current_cpp_front_half() {
    let _scale = NumberMantissaScaleGuard::new(MantissaScale::Large);

    assert_eq!(
        normalized_parts_from_json_input(NumberJsonInput::String("3.14e2")),
        Ok(
            RuntimeNumber::try_from_external_parts(314, 0, MantissaScale::Large)
                .expect("normalized 314 should be exact")
        )
    );
    assert_eq!(
        normalized_parts_from_json_input(NumberJsonInput::String("-1000e-2")),
        Ok(
            RuntimeNumber::try_from_external_parts(-10, 0, MantissaScale::Large)
                .expect("normalized -10 should be exact")
        )
    );
    assert_eq!(
        normalized_parts_from_json_input(NumberJsonInput::String("-0.000e6")),
        Ok(RuntimeNumber::zero())
    );
}

#[test]
fn normalized_parts_from_string_matches_current_cpp_zero_canonicalization() {
    let _scale = NumberMantissaScaleGuard::new(MantissaScale::Large);

    for input in ["0", "-0", "0.0", "-0.0", "0.000", "-0.000"] {
        assert_eq!(
            normalized_parts_from_string(input),
            Ok(RuntimeNumber::zero())
        );
    }
}

#[test]
fn normalized_parts_from_json_extreme_integer_behavior_by_scale() {
    let _round = NumberRoundModeGuard::new(RoundingMode::TowardsZero);

    {
        let _scale = NumberMantissaScaleGuard::new(MantissaScale::Small);
        assert_eq!(
            normalized_parts_from_json_input(NumberJsonInput::String("9223372036854775807")),
            Ok(RuntimeNumber::try_from_external_parts(
                9_223_372_036_854_775,
                3,
                MantissaScale::Small
            )
            .expect("small-range max int should normalize exactly"))
        );
        assert_eq!(
            normalized_parts_from_json_input(NumberJsonInput::String("-9223372036854775808")),
            Ok(RuntimeNumber::try_from_external_parts(
                -9_223_372_036_854_775,
                3,
                MantissaScale::Small
            )
            .expect("small-range min int should normalize exactly"))
        );
    }

    {
        let _scale = NumberMantissaScaleGuard::new(MantissaScale::Large);
        assert_eq!(
            normalized_parts_from_json_input(NumberJsonInput::String("9223372036854775807")),
            Ok(
                RuntimeNumber::try_from_external_parts(i64::MAX, 0, MantissaScale::Large)
                    .expect("large-range max int should normalize exactly")
            )
        );
        assert_eq!(
            normalized_parts_from_json_input(NumberJsonInput::String("-9223372036854775808")),
            Ok(RuntimeNumber::unchecked(true, 9_223_372_036_854_775_800, 0))
        );
    }
}

#[test]
fn normalized_parts_from_string_rejects_cpp_invalid_shapes() {
    let _scale = NumberMantissaScaleGuard::new(MantissaScale::Large);

    for input in ["", "e", "1e", "e2", "001", "000.0", ".1", "1.", "1.e3"] {
        assert_eq!(
            normalized_parts_from_string(input),
            Err(NumberPartsError::NotANumber(input.to_owned()))
        );
    }
}

#[test]
fn st_number_wraps_runtime_number_value_and_text() {
    let _scale = NumberMantissaScaleGuard::new(MantissaScale::Large);
    let value = RuntimeNumber::try_from_external_parts(314, -2, MantissaScale::Large)
        .expect("314e-2 should normalize exactly");
    let st_number = STNumber::new(value);

    assert_eq!(st_number.value(), value);
    assert_eq!(st_number.get_text(), "3.14");
    assert_eq!(st_number.to_string(), "3.14");
    assert!(!st_number.is_default());
    assert_eq!(STNumber::default().get_text(), "0");
    assert!(STNumber::default().is_default());

    let mut reassigned = STNumber::default();
    reassigned.set_value(value);
    assert_eq!(reassigned, st_number);

    let round_trip: RuntimeNumber = st_number.into();
    assert_eq!(round_trip, value);
}

#[test]
fn st_number_from_json_input_construction_cases() {
    let _round = NumberRoundModeGuard::new(RoundingMode::TowardsZero);

    {
        let _scale = NumberMantissaScaleGuard::new(MantissaScale::Small);
        assert_eq!(
            number_from_json_input(NumberJsonInput::Int(-42)),
            Ok(STNumber::new(
                RuntimeNumber::try_from_external_parts(-42, 0, MantissaScale::Small)
                    .expect("small-range int should normalize exactly")
            ))
        );
        assert_eq!(
            number_from_json_input(NumberJsonInput::String("3.14e2")),
            Ok(STNumber::new(
                RuntimeNumber::try_from_external_parts(314, 0, MantissaScale::Small)
                    .expect("small-range string should normalize exactly")
            ))
        );
    }

    {
        let _scale = NumberMantissaScaleGuard::new(MantissaScale::Large);
        assert_eq!(
            number_from_json_input(NumberJsonInput::UInt(42)),
            Ok(STNumber::new(
                RuntimeNumber::try_from_external_parts(42, 0, MantissaScale::Large)
                    .expect("large-range int should normalize exactly")
            ))
        );
        assert_eq!(
            number_from_json_input(NumberJsonInput::Other),
            Err(NumberPartsError::NotANumber("not a number".to_owned()))
        );
        assert_eq!(
            number_from_json_input(NumberJsonInput::String("-0.000e6")),
            Ok(STNumber::new(RuntimeNumber::zero()))
        );
    }
}
