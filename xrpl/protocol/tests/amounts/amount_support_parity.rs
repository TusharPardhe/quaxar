use protocol::{
    MAX_MP_TOKEN_AMOUNT, ST_AMOUNT_ISSUED_CURRENCY_FLAG, ST_AMOUNT_MAX_MANTISSA,
    ST_AMOUNT_MAX_NATIVE, ST_AMOUNT_MAX_NATIVE_NETWORK, ST_AMOUNT_MAX_OFFSET,
    ST_AMOUNT_MIN_MANTISSA, ST_AMOUNT_MIN_OFFSET, ST_AMOUNT_MP_TOKEN_FLAG, ST_AMOUNT_POSITIVE_FLAG,
    ST_AMOUNT_VALUE_MASK, is_issued_zero_header_bits, is_valid_st_amount_mantissa,
    is_valid_st_amount_mpt_value, is_valid_st_amount_native_internal_value,
    is_valid_st_amount_native_network_value, is_valid_st_amount_nonzero_iou,
    is_valid_st_amount_offset, issued_exponent_from_nonzero_header_bits, issued_header_bits,
    issued_header_bits_from_word, issued_header_is_negative, issued_header_word,
    issued_mantissa_from_word, issued_zero_header_bits, issued_zero_header_word,
    mpt_wire_header_byte, native_wire_word,
};

#[test]
fn shared_amount_constants_match_cpp_values() {
    assert_eq!(ST_AMOUNT_MIN_OFFSET, -96);
    assert_eq!(ST_AMOUNT_MAX_OFFSET, 80);
    assert_eq!(ST_AMOUNT_MIN_MANTISSA, 1_000_000_000_000_000);
    assert_eq!(ST_AMOUNT_MAX_MANTISSA, 9_999_999_999_999_999);
    assert_eq!(ST_AMOUNT_MAX_NATIVE, 9_000_000_000_000_000_000);
    assert_eq!(ST_AMOUNT_MAX_NATIVE_NETWORK, 100_000_000_000_000_000);
    assert_eq!(ST_AMOUNT_ISSUED_CURRENCY_FLAG, 0x8_000_000_000_000_000);
    assert_eq!(ST_AMOUNT_POSITIVE_FLAG, 0x4_000_000_000_000_000);
    assert_eq!(ST_AMOUNT_MP_TOKEN_FLAG, 0x2_000_000_000_000_000);
    assert_eq!(
        ST_AMOUNT_VALUE_MASK,
        !(ST_AMOUNT_POSITIVE_FLAG | ST_AMOUNT_MP_TOKEN_FLAG)
    );
}

#[test]
fn shared_amount_range_helpers_match_cpp_boundaries() {
    assert!(is_valid_st_amount_offset(ST_AMOUNT_MIN_OFFSET));
    assert!(is_valid_st_amount_offset(ST_AMOUNT_MAX_OFFSET));
    assert!(!is_valid_st_amount_offset(ST_AMOUNT_MIN_OFFSET - 1));
    assert!(!is_valid_st_amount_offset(ST_AMOUNT_MAX_OFFSET + 1));

    assert!(is_valid_st_amount_mantissa(ST_AMOUNT_MIN_MANTISSA));
    assert!(is_valid_st_amount_mantissa(ST_AMOUNT_MAX_MANTISSA));
    assert!(!is_valid_st_amount_mantissa(ST_AMOUNT_MIN_MANTISSA - 1));
    assert!(!is_valid_st_amount_mantissa(ST_AMOUNT_MAX_MANTISSA + 1));

    assert!(is_valid_st_amount_nonzero_iou(
        ST_AMOUNT_MIN_MANTISSA,
        ST_AMOUNT_MIN_OFFSET
    ));
    assert!(is_valid_st_amount_nonzero_iou(
        ST_AMOUNT_MAX_MANTISSA,
        ST_AMOUNT_MAX_OFFSET
    ));
    assert!(!is_valid_st_amount_nonzero_iou(0, 0));

    assert!(is_valid_st_amount_native_internal_value(
        ST_AMOUNT_MAX_NATIVE
    ));
    assert!(!is_valid_st_amount_native_internal_value(
        ST_AMOUNT_MAX_NATIVE + 1
    ));
    assert!(is_valid_st_amount_native_network_value(
        ST_AMOUNT_MAX_NATIVE_NETWORK
    ));
    assert!(!is_valid_st_amount_native_network_value(
        ST_AMOUNT_MAX_NATIVE_NETWORK + 1
    ));

    assert!(is_valid_st_amount_mpt_value(MAX_MP_TOKEN_AMOUNT as u64));
    assert!(!is_valid_st_amount_mpt_value(
        (MAX_MP_TOKEN_AMOUNT as u64) + 1
    ));
}

#[test]
fn issued_header_helpers_match_cpp_encoding() {
    let positive_bits = issued_header_bits(ST_AMOUNT_MIN_OFFSET, false).expect("header bits");
    assert_eq!(positive_bits, 512 + 256 + 1);
    assert!(!issued_header_is_negative(positive_bits));
    assert_eq!(
        issued_exponent_from_nonzero_header_bits(positive_bits),
        ST_AMOUNT_MIN_OFFSET
    );

    let negative_bits = issued_header_bits(ST_AMOUNT_MAX_OFFSET, true).expect("header bits");
    assert_eq!(negative_bits, 512 + 177);
    assert!(issued_header_is_negative(negative_bits));
    assert_eq!(
        issued_exponent_from_nonzero_header_bits(negative_bits),
        ST_AMOUNT_MAX_OFFSET
    );

    assert_eq!(issued_zero_header_bits(), 512);
    assert!(is_issued_zero_header_bits(issued_zero_header_bits()));
    assert_eq!(issued_zero_header_word(), ST_AMOUNT_ISSUED_CURRENCY_FLAG);
    assert_eq!(
        issued_header_bits_from_word(issued_zero_header_word()),
        issued_zero_header_bits()
    );

    let mantissa = 1_234_567_890_123_456u64;
    let word = issued_header_word(mantissa, -5, false).expect("header word");
    assert_eq!(issued_mantissa_from_word(word), mantissa);
    assert_eq!(
        issued_exponent_from_nonzero_header_bits(issued_header_bits_from_word(word)),
        -5
    );
    assert!(!issued_header_is_negative(issued_header_bits_from_word(
        word
    )));

    assert!(issued_header_word(ST_AMOUNT_MIN_MANTISSA - 1, 0, false).is_none());
    assert!(issued_header_word(ST_AMOUNT_MIN_MANTISSA, ST_AMOUNT_MAX_OFFSET + 1, false).is_none());
}

#[test]
fn native_and_mpt_wire_helpers_match_cpp_flags() {
    assert_eq!(native_wire_word(25, false), 25 | ST_AMOUNT_POSITIVE_FLAG);
    assert_eq!(native_wire_word(25, true), 25);
    assert_eq!(mpt_wire_header_byte(false), 0x60);
    assert_eq!(mpt_wire_header_byte(true), 0x20);
}
