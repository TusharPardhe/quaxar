use basics::str_hex::str_hex;
use protocol::{
    IOUAmount, JsonOptions, JsonValue, MPTAmount, MPTIssue, STAmount, STVar, Serializer, StBase,
    currency_from_string, get_field_by_symbol, issued_zero_header_word, make_mpt_id,
    parse_base58_account_id, xrp_currency,
};

#[test]
fn native_stamount_round_trip_wire_and_default_contract() {
    let zero = STAmount::new_native(0, false);
    let positive = STAmount::new_native(100, false);
    let negative = STAmount::new_native(100, true);

    let mut zero_ser = Serializer::default();
    zero.add(&mut zero_ser);
    assert_eq!(str_hex(zero_ser.data()), "4000000000000000");
    let mut zero_iter = protocol::SerialIter::new(zero_ser.data());
    let zero_round_trip =
        STAmount::from_serial_iter(&mut zero_iter, get_field_by_symbol("sfBalance"));
    assert_eq!(zero_round_trip, zero);
    assert!(zero_round_trip.is_default());
    assert_eq!(zero_round_trip.text(), "0");
    assert_eq!(
        zero_round_trip.json(JsonOptions::NONE),
        JsonValue::String("0".to_string())
    );

    let mut positive_ser = Serializer::default();
    positive.add(&mut positive_ser);
    assert_eq!(str_hex(positive_ser.data()), "4000000000000064");
    let mut positive_iter = protocol::SerialIter::new(positive_ser.data());
    assert_eq!(
        STAmount::from_serial_iter(&mut positive_iter, get_field_by_symbol("sfBalance")),
        positive
    );
    assert_eq!(positive.xrp(), protocol::XRPAmount::from(100));
    assert!(positive > zero);
    assert!(negative < zero);
}

#[test]
fn iou_zero_and_nonzero_match_cpp_json_and_text_rules() {
    let issuer =
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("issuer account");
    let issue = protocol::Issue::new(currency_from_string("USD"), issuer);

    let zero = STAmount::from_iou_amount(
        get_field_by_symbol("sfAmount"),
        IOUAmount::from_parts(0, 0).expect("zero"),
        issue,
    );
    assert_eq!(zero.text(), "0");
    assert!(!zero.is_default());
    assert_eq!(
        zero.json(JsonOptions::NONE),
        JsonValue::Object(
            [
                ("currency".to_string(), JsonValue::String("USD".to_string())),
                (
                    "issuer".to_string(),
                    JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_string())
                ),
                ("value".to_string(), JsonValue::String("0".to_string())),
            ]
            .into_iter()
            .collect()
        )
    );

    let mut zero_ser = Serializer::default();
    zero.add(&mut zero_ser);
    let expected_prefix = format!("{:016X}", issued_zero_header_word());
    assert!(str_hex(zero_ser.data()).starts_with(&expected_prefix));
    let mut zero_iter = protocol::SerialIter::new(zero_ser.data());
    assert_eq!(
        STAmount::from_serial_iter(&mut zero_iter, get_field_by_symbol("sfAmount")),
        zero
    );

    let decimal_edge = STAmount::from_iou_amount(
        get_field_by_symbol("sfAmount"),
        IOUAmount::from_parts(1_000_000_000_000_000, -25).expect("edge"),
        issue,
    );
    let scientific_low = STAmount::from_iou_amount(
        get_field_by_symbol("sfAmount"),
        IOUAmount::from_parts(1_000_000_000_000_000, -26).expect("scientific"),
        issue,
    );
    let decimal_high = STAmount::from_iou_amount(
        get_field_by_symbol("sfAmount"),
        IOUAmount::from_parts(1_000_000_000_000_000, -5).expect("edge"),
        issue,
    );
    let scientific_high = STAmount::from_iou_amount(
        get_field_by_symbol("sfAmount"),
        IOUAmount::from_parts(1_000_000_000_000_000, -4).expect("scientific"),
        issue,
    );

    assert_eq!(decimal_edge.text(), "0.0000000001");
    assert_eq!(scientific_low.text(), "1000000000000000e-26");
    assert_eq!(decimal_high.text(), "10000000000");
    assert_eq!(scientific_high.text(), "1000000000000000e-4");
    assert_eq!(
        decimal_edge.iou(),
        IOUAmount::from_parts(1_000_000_000_000_000, -25).unwrap()
    );
}

#[test]
fn mpt_round_trip_wire_and_json_shape() {
    let issuer =
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("issuer account");
    let issue = MPTIssue::new(make_mpt_id(7, issuer));
    let positive =
        STAmount::from_mpt_amount(get_field_by_symbol("sfAmount"), MPTAmount::from(25), issue);
    let negative =
        STAmount::from_mpt_amount(get_field_by_symbol("sfAmount"), MPTAmount::from(-25), issue);

    let mut positive_ser = Serializer::default();
    positive.add(&mut positive_ser);
    assert_eq!(&positive_ser.data()[..1], &[0x60]);
    let mut positive_iter = protocol::SerialIter::new(positive_ser.data());
    assert_eq!(
        STAmount::from_serial_iter(&mut positive_iter, get_field_by_symbol("sfAmount")),
        positive
    );
    assert_eq!(positive.mpt(), MPTAmount::from(25));

    let mut negative_ser = Serializer::default();
    negative.add(&mut negative_ser);
    assert_eq!(&negative_ser.data()[..1], &[0x20]);
    let mut negative_iter = protocol::SerialIter::new(negative_ser.data());
    assert_eq!(
        STAmount::from_serial_iter(&mut negative_iter, get_field_by_symbol("sfAmount")),
        negative
    );

    assert_eq!(
        positive.json(JsonOptions::NONE),
        JsonValue::Object(
            [
                (
                    "mpt_issuance_id".to_string(),
                    JsonValue::String(issue.text())
                ),
                ("value".to_string(), JsonValue::String("25".to_string())),
            ]
            .into_iter()
            .collect()
        )
    );
}

#[test]
fn amount_comparison_and_stvar_support_match_current_surface() {
    let left = STVar::from_serialized_type(
        protocol::SerializedTypeId::Amount,
        get_field_by_symbol("sfBalance"),
    );
    assert_eq!(left.stype(), protocol::SerializedTypeId::Amount);

    let issuer_one =
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("issuer account");
    let issuer_two =
        parse_base58_account_id("r3kmLJN5D28dHuH8vZNUZpMC43pEHpaocV").expect("issuer account");
    let first = STAmount::from_iou_amount(
        get_field_by_symbol("sfAmount"),
        IOUAmount::from_parts(1_000_000_000_000_000, 0).expect("amount"),
        protocol::Issue::new(currency_from_string("USD"), issuer_one),
    );
    let second = STAmount::from_iou_amount(
        get_field_by_symbol("sfAmount"),
        IOUAmount::from_parts(1_000_000_000_000_000, 0).expect("amount"),
        protocol::Issue::new(currency_from_string("USD"), issuer_two),
    );
    assert_eq!(first, second);
}

#[test]
fn parser_rejects_noncanonical_xrp_and_invalid_iou_identity() {
    let field = get_field_by_symbol("sfAmount");
    let issuer =
        parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").expect("issuer account");
    let usd = currency_from_string("USD");

    let negative_zero = std::panic::catch_unwind(|| {
        let bytes = [0u8; 8];
        let mut iter = protocol::SerialIter::new(&bytes);
        let _ = STAmount::from_serial_iter(&mut iter, field);
    });
    assert!(negative_zero.is_err());

    let invalid_currency = std::panic::catch_unwind(|| {
        let mut serializer = Serializer::default();
        serializer.add64(issued_zero_header_word());
        serializer.add_bit_string(xrp_currency());
        serializer.add_bit_string(issuer);
        let mut iter = protocol::SerialIter::new(serializer.data());
        let _ = STAmount::from_serial_iter(&mut iter, field);
    });
    assert!(invalid_currency.is_err());

    let invalid_account = std::panic::catch_unwind(|| {
        let mut serializer = Serializer::default();
        serializer.add64(issued_zero_header_word());
        serializer.add_bit_string(usd);
        serializer.add_bit_string(protocol::AccountID::zero());
        let mut iter = protocol::SerialIter::new(serializer.data());
        let _ = STAmount::from_serial_iter(&mut iter, field);
    });
    assert!(invalid_account.is_err());
}

// ─── Round 5: STAmount parity with C++ Issue_test.cpp ───
