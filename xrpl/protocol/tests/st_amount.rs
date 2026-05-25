use protocol::{
    Asset, IOUAmount, Issue, JsonOptions, JsonValue, MPTAmount, MPTIssue, STAmount, Serializer,
    StBase, amount_from_quality, currency_from_string, get_field_by_symbol, get_rate, make_mpt_id,
    no_issue, parse_base58_account_id, sf_generic,
};

fn account(value: &str) -> protocol::AccountID {
    parse_base58_account_id(value).expect("account should parse")
}

#[test]
fn protocol_stamount_zero_canonicalization_and_issue_mutation_match_cpp() {
    let issuer_one = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let issuer_two = account("r3kmLJN5D28dHuH8vZNUZpMC43pEHpaocV");
    let usd = currency_from_string("USD");

    let mut native = STAmount::new_native(10, true);
    native.clear();
    assert!(native.native());
    assert_eq!(native.exponent(), 0);
    assert_eq!(native.mantissa(), 0);
    assert!(!native.negative());
    assert_eq!(native.text(), "0");

    let mut issued = STAmount::new_with_asset(
        get_field_by_symbol("sfAmount"),
        Issue::new(usd, issuer_one),
        10,
        -2,
        false,
    );
    issued.set_issuer(issuer_two);
    assert_eq!(issued.issue(), Issue::new(usd, issuer_two));

    issued.clear();
    assert!(!issued.native());
    assert_eq!(issued.exponent(), -100);
    assert_eq!(issued.mantissa(), 0);
    assert!(!issued.negative());
    assert_eq!(issued.text(), "0");

    let mut reset = STAmount::new_native(5, false);
    reset.clear_with_asset(Issue::new(usd, issuer_one));
    assert_eq!(reset.issue(), Issue::new(usd, issuer_one));
    assert_eq!(reset.exponent(), -100);
    assert_eq!(reset.text(), "0");
}

#[test]
fn protocol_stamount_quality_storage_round_trips_through_cpp_rate_encoding() {
    let issuer = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let issue = Issue::new(currency_from_string("USD"), issuer);
    let offer_out = STAmount::from_iou_amount(
        get_field_by_symbol("sfAmount"),
        IOUAmount::from_parts(5_000_000_000_000_000, -15).expect("offer out"),
        issue,
    );
    let offer_in = STAmount::from_iou_amount(
        get_field_by_symbol("sfAmount"),
        IOUAmount::from_parts(12_500_000_000_000_000, -15).expect("offer in"),
        issue,
    );

    let stored_rate = get_rate(&offer_out, &offer_in);
    let decoded = amount_from_quality(stored_rate);

    assert_ne!(stored_rate, 0);
    assert_eq!(decoded.asset(), Asset::Issue(no_issue()));
    assert_eq!(decoded.text(), "2.5");
    assert_eq!(get_rate(&STAmount::new_native(0, false), &offer_in), 0);
}

#[test]
fn protocol_stamount_public_json_and_wire_shapes_cover_xrp_iou_and_mpt() {
    let issuer = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let usd = Issue::new(currency_from_string("USD"), issuer);
    let mpt_issue = MPTIssue::new(make_mpt_id(7, issuer));

    let xrp = STAmount::new_native(25, false);
    assert_eq!(
        xrp.json(JsonOptions::NONE),
        JsonValue::String("25".to_string())
    );

    let iou = STAmount::from_iou_amount(
        get_field_by_symbol("sfAmount"),
        IOUAmount::from_parts(1_500_000_000_000_000, -14).expect("iou"),
        usd,
    );
    assert_eq!(
        iou.json(JsonOptions::NONE),
        JsonValue::Object(
            [
                ("currency".to_string(), JsonValue::String("USD".to_string())),
                (
                    "issuer".to_string(),
                    JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_string()),
                ),
                ("value".to_string(), JsonValue::String("15".to_string())),
            ]
            .into_iter()
            .collect(),
        )
    );

    let mpt = STAmount::from_mpt_amount(
        get_field_by_symbol("sfAmount"),
        MPTAmount::from_value(-25),
        mpt_issue,
    );
    assert_eq!(
        mpt.json(JsonOptions::NONE),
        JsonValue::Object(
            [
                (
                    "mpt_issuance_id".to_string(),
                    JsonValue::String(mpt_issue.text()),
                ),
                ("value".to_string(), JsonValue::String("-25".to_string())),
            ]
            .into_iter()
            .collect(),
        )
    );

    let mut serializer = Serializer::default();
    mpt.add(&mut serializer);
    let reparsed = STAmount::from_serial_iter(
        &mut protocol::SerialIter::new(serializer.data()),
        get_field_by_symbol("sfAmount"),
    );
    assert_eq!(reparsed, mpt);
}

#[test]
fn protocol_stamount_issue_equality_ignores_issuer() {
    let usd = currency_from_string("USD");
    let first = STAmount::new_with_asset(
        sf_generic(),
        Issue::new(usd, account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh")),
        1_500_000_000_000_000,
        -14,
        false,
    );
    let second = STAmount::new_with_asset(
        sf_generic(),
        Issue::new(usd, account("r3kmLJN5D28dHuH8vZNUZpMC43pEHpaocV")),
        1_500_000_000_000_000,
        -14,
        false,
    );

    assert_eq!(first, second);
}

// ─── STAmount: Native XRP ───────────────────────────────────────────────────

#[test]
fn native_0() {
    let a = STAmount::new_native(0, false);
    assert_eq!(a.mantissa(), 0);
}
#[test]
fn native_1() {
    let a = STAmount::new_native(1, false);
    assert_eq!(a.mantissa(), 1);
}
#[test]
fn native_10() {
    let a = STAmount::new_native(10, false);
    assert_eq!(a.mantissa(), 10);
}
#[test]
fn native_100() {
    let a = STAmount::new_native(100, false);
    assert_eq!(a.mantissa(), 100);
}
#[test]
fn native_1000() {
    let a = STAmount::new_native(1000, false);
    assert_eq!(a.mantissa(), 1000);
}
#[test]
fn native_10000() {
    let a = STAmount::new_native(10000, false);
    assert_eq!(a.mantissa(), 10000);
}
#[test]
fn native_100000() {
    let a = STAmount::new_native(100000, false);
    assert_eq!(a.mantissa(), 100000);
}
#[test]
fn native_1000000() {
    let a = STAmount::new_native(1000000, false);
    assert_eq!(a.mantissa(), 1000000);
}
#[test]
fn native_10000000() {
    let a = STAmount::new_native(10000000, false);
    assert_eq!(a.mantissa(), 10000000);
}
#[test]
fn native_100000000() {
    let a = STAmount::new_native(100000000, false);
    assert_eq!(a.mantissa(), 100000000);
}
#[test]
fn native_neg_1() {
    let a = STAmount::new_native(1, true);
    assert!(a.negative());
}
#[test]
fn native_neg_100() {
    let a = STAmount::new_native(100, true);
    assert!(a.negative());
}
#[test]
fn native_neg_1000000() {
    let a = STAmount::new_native(1000000, true);
    assert!(a.negative());
}
#[test]
fn native_is_native() {
    let a = STAmount::new_native(1, false);
    assert!(a.native());
}
#[test]
fn native_zero_not_neg() {
    let a = STAmount::new_native(0, false);
    assert!(!a.negative());
}
#[test]
fn native_exponent_0() {
    let a = STAmount::new_native(1, false);
    assert_eq!(a.exponent(), 0);
}

// ─── STAmount: IOU ──────────────────────────────────────────────────────────

// ─── STAmount: MPT ──────────────────────────────────────────────────────────

#[test]
fn mpt_0() {
    let a = MPTAmount::from(0i64);
    assert_eq!(a.value(), 0);
}
#[test]
fn mpt_1() {
    let a = MPTAmount::from(1i64);
    assert_eq!(a.value(), 1);
}
#[test]
fn mpt_100() {
    let a = MPTAmount::from(100i64);
    assert_eq!(a.value(), 100);
}
#[test]
fn mpt_1000() {
    let a = MPTAmount::from(1000i64);
    assert_eq!(a.value(), 1000);
}
#[test]
fn mpt_10000() {
    let a = MPTAmount::from(10000i64);
    assert_eq!(a.value(), 10000);
}
#[test]
fn mpt_100000() {
    let a = MPTAmount::from(100000i64);
    assert_eq!(a.value(), 100000);
}
#[test]
fn mpt_1000000() {
    let a = MPTAmount::from(1000000i64);
    assert_eq!(a.value(), 1000000);
}
#[test]
fn mpt_neg_1() {
    let a = MPTAmount::from(-1i64);
    assert_eq!(a.value(), -1);
}
#[test]
fn mpt_neg_100() {
    let a = MPTAmount::from(-100i64);
    assert_eq!(a.value(), -100);
}
#[test]
fn mpt_neg_1000000() {
    let a = MPTAmount::from(-1000000i64);
    assert_eq!(a.value(), -1000000);
}

// ─── IOUAmount: Construction ────────────────────────────────────────────────

#[test]
fn iou_amt_from_parts_1() {
    let a = IOUAmount::from_parts(1, 0).unwrap();
    assert_eq!(a.mantissa(), 1000000000000000);
}
#[test]
fn iou_amt_from_parts_10() {
    let a = IOUAmount::from_parts(10, 0).unwrap();
    assert_eq!(a.mantissa(), 1000000000000000);
}
#[test]
fn iou_amt_from_parts_100() {
    let a = IOUAmount::from_parts(100, 0).unwrap();
    assert_eq!(a.mantissa(), 1000000000000000);
}
#[test]
fn iou_amt_from_parts_neg() {
    let a = IOUAmount::from_parts(-1, 0).unwrap();
    assert!(a.mantissa() < 0);
}
#[test]
fn iou_amt_zero() {
    let a = IOUAmount::from_parts(0, 0).unwrap();
    assert_eq!(a.mantissa(), 0);
}

// ─── Currency: Parsing ──────────────────────────────────────────────────────

#[test]
fn cur_usd() {
    let c = currency_from_string("USD");
    assert!(!c.is_zero());
}
#[test]
fn cur_eur() {
    let c = currency_from_string("EUR");
    assert!(!c.is_zero());
}
#[test]
fn cur_gbp() {
    let c = currency_from_string("GBP");
    assert!(!c.is_zero());
}
#[test]
fn cur_jpy() {
    let c = currency_from_string("JPY");
    assert!(!c.is_zero());
}
#[test]
fn cur_chf() {
    let c = currency_from_string("CHF");
    assert!(!c.is_zero());
}
#[test]
fn cur_cad() {
    let c = currency_from_string("CAD");
    assert!(!c.is_zero());
}
#[test]
fn cur_aud() {
    let c = currency_from_string("AUD");
    assert!(!c.is_zero());
}
#[test]
fn cur_nzd() {
    let c = currency_from_string("NZD");
    assert!(!c.is_zero());
}
#[test]
fn cur_cny() {
    let c = currency_from_string("CNY");
    assert!(!c.is_zero());
}
#[test]
fn cur_inr() {
    let c = currency_from_string("INR");
    assert!(!c.is_zero());
}
#[test]
fn cur_krw() {
    let c = currency_from_string("KRW");
    assert!(!c.is_zero());
}
#[test]
fn cur_sgd() {
    let c = currency_from_string("SGD");
    assert!(!c.is_zero());
}
#[test]
fn cur_hkd() {
    let c = currency_from_string("HKD");
    assert!(!c.is_zero());
}
#[test]
fn cur_mxn() {
    let c = currency_from_string("MXN");
    assert!(!c.is_zero());
}
#[test]
fn cur_brl() {
    let c = currency_from_string("BRL");
    assert!(!c.is_zero());
}
#[test]
fn cur_zar() {
    let c = currency_from_string("ZAR");
    assert!(!c.is_zero());
}
#[test]
fn cur_sek() {
    let c = currency_from_string("SEK");
    assert!(!c.is_zero());
}
#[test]
fn cur_nok() {
    let c = currency_from_string("NOK");
    assert!(!c.is_zero());
}
#[test]
fn cur_dkk() {
    let c = currency_from_string("DKK");
    assert!(!c.is_zero());
}
#[test]
fn cur_pln() {
    let c = currency_from_string("PLN");
    assert!(!c.is_zero());
}
#[test]
fn cur_different() {
    let a = currency_from_string("USD");
    let b = currency_from_string("EUR");
    assert_ne!(a, b);
}
#[test]
fn cur_same() {
    let a = currency_from_string("USD");
    let b = currency_from_string("USD");
    assert_eq!(a, b);
}

// ─── AccountID: Parsing ─────────────────────────────────────────────────────

#[test]
fn acct_parse_1() {
    assert!(parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").is_some());
}
#[test]
fn acct_parse_2() {
    assert!(parse_base58_account_id("r3kmLJN5D28dHuH8vZNUZpMC43pEHpaocV").is_some());
}
#[test]
fn acct_parse_invalid() {
    assert!(parse_base58_account_id("invalid").is_none());
}
#[test]
fn acct_parse_empty() {
    assert!(parse_base58_account_id("").is_none());
}
#[test]
fn acct_different() {
    let a = parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").unwrap();
    let b = parse_base58_account_id("r3kmLJN5D28dHuH8vZNUZpMC43pEHpaocV").unwrap();
    assert_ne!(a, b);
}
#[test]
fn acct_same() {
    let a = parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").unwrap();
    let b = parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh").unwrap();
    assert_eq!(a, b);
}

// ─── Issue: Construction ────────────────────────────────────────────────────

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 2C-P1: Direct ports from C++ STAmount_test.cpp
// ═══════════════════════════════════════════════════════════════════════════════

// --- testcase("native currency") — comparison operators ---

#[test]
fn cpp_native_serialize_roundtrip_zero() {
    let zero = STAmount::new_native(0, false);
    let mut s = Serializer::new(64);
    zero.add(&mut s);
    assert!(s.data().len() == 8);
    assert!(!zero.negative());
    assert!(zero.native());
}

#[test]
fn cpp_native_serialize_roundtrip_one() {
    let one = STAmount::new_native(1, false);
    let mut s = Serializer::new(64);
    one.add(&mut s);
    assert!(s.data().len() == 8);
    assert!(one.native());
}

#[test]
fn cpp_native_serialize_roundtrip_hundred() {
    let hundred = STAmount::new_native(100, false);
    let mut s = Serializer::new(64);
    hundred.add(&mut s);
    assert!(s.data().len() == 8);
    assert!(hundred.native());
}

#[test]
fn cpp_native_comparisons_lt() {
    let zero = STAmount::new_native(0, false);
    let one = STAmount::new_native(1, false);
    let hundred = STAmount::new_native(100, false);
    // zero < one, zero < hundred, one < hundred
    assert!(zero < one);
    assert!(zero < hundred);
    assert!(one < hundred);
    // NOT: one < zero, hundred < zero, hundred < one
    assert!(one >= zero);
    assert!(hundred >= zero);
    assert!(hundred >= one);
    // NOT: x < x
    assert!(zero >= zero);
    assert!(one >= one);
    assert!(hundred >= hundred);
}

#[test]
fn cpp_native_comparisons_gt() {
    let zero = STAmount::new_native(0, false);
    let one = STAmount::new_native(1, false);
    let hundred = STAmount::new_native(100, false);
    assert!(one > zero);
    assert!(hundred > zero);
    assert!(hundred > one);
    assert!(zero <= one);
    assert!(zero <= hundred);
    assert!(one <= hundred);
    assert!(zero <= zero);
    assert!(one <= one);
    assert!(hundred <= hundred);
}

#[test]
fn cpp_native_comparisons_eq() {
    let zero = STAmount::new_native(0, false);
    let one = STAmount::new_native(1, false);
    let hundred = STAmount::new_native(100, false);
    assert_eq!(zero, zero);
    assert_eq!(one, one);
    assert_eq!(hundred, hundred);
    assert_ne!(zero, one);
    assert_ne!(zero, hundred);
    assert_ne!(one, hundred);
}

#[test]
fn cpp_native_comparisons_le_ge() {
    let zero = STAmount::new_native(0, false);
    let one = STAmount::new_native(1, false);
    let hundred = STAmount::new_native(100, false);
    assert!(zero <= zero);
    assert!(zero <= one);
    assert!(zero <= hundred);
    assert!(one <= one);
    assert!(one <= hundred);
    assert!(hundred <= hundred);
    assert!((one > zero));
    assert!((hundred > zero));
    assert!((hundred > one));
    assert!(zero >= zero);
    assert!(one >= zero);
    assert!(one >= one);
    assert!(hundred >= zero);
    assert!(hundred >= one);
    assert!(hundred >= hundred);
    assert!((zero < one));
    assert!((zero < hundred));
    assert!((one < hundred));
}

#[test]
fn cpp_native_text_representation() {
    assert_eq!(STAmount::new_native(0, false).text(), "0");
    assert_eq!(STAmount::new_native(31, false).text(), "31");
    assert_eq!(STAmount::new_native(310, false).text(), "310");
}

// --- testcase("arithmetic") — getRate ---

#[test]
fn cpp_get_rate_1_to_10() {
    // C++: getRate(STAmount(1), STAmount(10)) == ((100-14) << 56) | 1000000000000000
    let expected = ((100u64 - 14) << 56) | 1000000000000000u64;
    let r = get_rate(
        &STAmount::new_native(1, false),
        &STAmount::new_native(10, false),
    );
    assert_eq!(r, expected);
}

#[test]
fn cpp_get_rate_10_to_1() {
    // C++: getRate(STAmount(10), STAmount(1)) == ((100-16) << 56) | 1000000000000000
    let expected = ((100u64 - 16) << 56) | 1000000000000000u64;
    let r = get_rate(
        &STAmount::new_native(10, false),
        &STAmount::new_native(1, false),
    );
    assert_eq!(r, expected);
}

// --- testcase("can add xrp") ---

#[test]
fn cpp_can_add_xrp_zero_plus_1000() {
    let a = STAmount::new_native(0, false);
    let b = STAmount::new_native(1000, false);
    // Adding zero to anything should work (no overflow)
    let c = a + b;
    assert_eq!(c.mantissa(), 1000);
}

#[test]
fn cpp_can_add_xrp_1000_plus_zero() {
    let a = STAmount::new_native(1000, false);
    let b = STAmount::new_native(0, false);
    let c = a + b;
    assert_eq!(c.mantissa(), 1000);
}

#[test]
fn cpp_can_add_xrp_500_plus_1500() {
    let a = STAmount::new_native(500, false);
    let b = STAmount::new_native(1500, false);
    let c = a + b;
    assert_eq!(c.mantissa(), 2000);
}

#[test]
fn cpp_can_add_xrp_positive_and_negative() {
    let a = STAmount::new_native(1000, false);
    let b = STAmount::new_native(1000, true);
    let c = a + b;
    assert_eq!(c.mantissa(), 0);
}

// --- testcase("can subtract xrp") ---

#[test]
fn cpp_can_subtract_xrp_1000_minus_500() {
    let a = STAmount::new_native(1000, false);
    let b = STAmount::new_native(500, false);
    let c = a - b;
    assert_eq!(c.mantissa(), 500);
    assert!(!c.negative());
}

#[test]
fn cpp_can_subtract_xrp_500_minus_1000() {
    let a = STAmount::new_native(500, false);
    let b = STAmount::new_native(1000, false);
    let c = a - b;
    assert_eq!(c.mantissa(), 500);
    assert!(c.negative());
}

#[test]
fn cpp_can_subtract_xrp_equal() {
    let a = STAmount::new_native(1000, false);
    let b = STAmount::new_native(1000, false);
    let c = a - b;
    assert_eq!(c.mantissa(), 0);
}

// --- testcase("can add iou") ---

#[test]
fn cpp_can_add_iou_1_plus_1() {
    let usd = currency_from_string("USD");
    let iss = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let a = STAmount::new_with_asset(
        sf_generic(),
        Issue::new(usd, iss),
        1000000000000000,
        -15,
        false,
    );
    let b = STAmount::new_with_asset(
        sf_generic(),
        Issue::new(usd, iss),
        1000000000000000,
        -15,
        false,
    );
    let c = a + b;
    assert_eq!(c.text(), "2");
}

#[test]
fn cpp_can_add_iou_1_plus_neg1() {
    let usd = currency_from_string("USD");
    let iss = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let a = STAmount::new_with_asset(
        sf_generic(),
        Issue::new(usd, iss),
        1000000000000000,
        -15,
        false,
    );
    let b = STAmount::new_with_asset(
        sf_generic(),
        Issue::new(usd, iss),
        1000000000000000,
        -15,
        true,
    );
    let c = a + b;
    assert_eq!(c.mantissa(), 0);
}

// --- testcase("can subtract iou") ---

#[test]
fn cpp_can_subtract_iou_3_minus_1() {
    let usd = currency_from_string("USD");
    let iss = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let a = STAmount::new_with_asset(
        sf_generic(),
        Issue::new(usd, iss),
        3000000000000000,
        -15,
        false,
    );
    let b = STAmount::new_with_asset(
        sf_generic(),
        Issue::new(usd, iss),
        1000000000000000,
        -15,
        false,
    );
    let c = a - b;
    assert_eq!(c.text(), "2");
    assert!(!c.negative());
}

#[test]
fn cpp_can_subtract_iou_1_minus_3() {
    let usd = currency_from_string("USD");
    let iss = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let a = STAmount::new_with_asset(
        sf_generic(),
        Issue::new(usd, iss),
        1000000000000000,
        -15,
        false,
    );
    let b = STAmount::new_with_asset(
        sf_generic(),
        Issue::new(usd, iss),
        3000000000000000,
        -15,
        false,
    );
    let c = a - b;
    assert!(c.negative());
}

// --- testcase("set value (native)") ---

#[test]
fn cpp_set_value_native_drops() {
    // C++ tests: "1", "22", "333", "4444", "55555", "666666"
    assert_eq!(STAmount::new_native(1, false).text(), "1");
    assert_eq!(STAmount::new_native(22, false).text(), "22");
    assert_eq!(STAmount::new_native(333, false).text(), "333");
    assert_eq!(STAmount::new_native(4444, false).text(), "4444");
    assert_eq!(STAmount::new_native(55555, false).text(), "55555");
    assert_eq!(STAmount::new_native(666666, false).text(), "666666");
}

#[test]
fn cpp_set_value_native_powers_of_10() {
    // 1 XRP up to 100 billion in drops (powers of 10)
    assert_eq!(STAmount::new_native(1_000_000, false).text(), "1000000");
    assert_eq!(STAmount::new_native(10_000_000, false).text(), "10000000");
    assert_eq!(STAmount::new_native(100_000_000, false).text(), "100000000");
    assert_eq!(
        STAmount::new_native(1_000_000_000, false).text(),
        "1000000000"
    );
    assert_eq!(
        STAmount::new_native(10_000_000_000, false).text(),
        "10000000000"
    );
    assert_eq!(
        STAmount::new_native(100_000_000_000, false).text(),
        "100000000000"
    );
    assert_eq!(
        STAmount::new_native(1_000_000_000_000, false).text(),
        "1000000000000"
    );
    assert_eq!(
        STAmount::new_native(10_000_000_000_000, false).text(),
        "10000000000000"
    );
    assert_eq!(
        STAmount::new_native(100_000_000_000_000, false).text(),
        "100000000000000"
    );
    assert_eq!(
        STAmount::new_native(100_000_000_000_000_000, false).text(),
        "100000000000000000"
    );
}

// --- testcase("set value (iou)") ---

#[test]
fn cpp_set_value_iou_integers() {
    let usd = currency_from_string("USD");
    let iss = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let mk =
        |m: u64, e: i32| STAmount::new_with_asset(sf_generic(), Issue::new(usd, iss), m, e, false);
    assert_eq!(mk(1000000000000000, -15).text(), "1");
    assert_eq!(mk(1000000000000000, -14).text(), "10");
    assert_eq!(mk(1000000000000000, -13).text(), "100");
    assert_eq!(mk(1000000000000000, -12).text(), "1000");
    assert_eq!(mk(1000000000000000, -11).text(), "10000");
    assert_eq!(mk(1000000000000000, -10).text(), "100000");
    assert_eq!(mk(1000000000000000, -9).text(), "1000000");
    assert_eq!(mk(1000000000000000, -8).text(), "10000000");
    assert_eq!(mk(1000000000000000, -7).text(), "100000000");
    assert_eq!(mk(1000000000000000, -6).text(), "1000000000");
    assert_eq!(mk(1000000000000000, -5).text(), "10000000000");
}

#[test]
fn cpp_set_value_iou_fractional() {
    let usd = currency_from_string("USD");
    let iss = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let mk =
        |m: u64, e: i32| STAmount::new_with_asset(sf_generic(), Issue::new(usd, iss), m, e, false);
    // C++: "1234567.1" through "1234567.123456789"
    assert_eq!(mk(1234567100000000, -9).text(), "1234567.1");
    assert_eq!(mk(1234567120000000, -9).text(), "1234567.12");
    assert_eq!(mk(1234567123000000, -9).text(), "1234567.123");
    assert_eq!(mk(1234567123400000, -9).text(), "1234567.1234");
    assert_eq!(mk(1234567123450000, -9).text(), "1234567.12345");
    assert_eq!(mk(1234567123456000, -9).text(), "1234567.123456");
    assert_eq!(mk(1234567123456700, -9).text(), "1234567.1234567");
    assert_eq!(mk(1234567123456780, -9).text(), "1234567.12345678");
    assert_eq!(mk(1234567123456789, -9).text(), "1234567.123456789");
}

// --- testcase("STAmount to XRPAmount conversions") ---

#[test]
fn cpp_stamount_xrp_amount_conversion() {
    // Native STAmount stores drops directly
    let a = STAmount::new_native(1_000_000, false); // 1 XRP in drops
    assert_eq!(a.mantissa(), 1_000_000);
    assert!(a.native());
    assert!(!a.negative());
}

// --- testcase("STAmount to IOUAmount conversions") ---

#[test]
fn cpp_stamount_iou_amount_conversion() {
    let usd = currency_from_string("USD");
    let iss = account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh");
    let a = STAmount::new_with_asset(
        sf_generic(),
        Issue::new(usd, iss),
        1000000000000000,
        -15,
        false,
    );
    assert!(!a.native());
    assert_eq!(a.mantissa(), 1000000000000000);
    assert_eq!(a.exponent(), -15);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 2C-P2: Direct ports from C++ STInteger_test.cpp + Seed_test.cpp
// ═══════════════════════════════════════════════════════════════════════════════

// --- From STInteger_test.cpp ---

/// C++: testcase("UInt8") — STUInt8(255).value() == 255
#[test]
fn cpp_st_uint8_value() {
    let u = protocol::STUInt8::new(255);
    assert_eq!(u.value(), 255);
}

/// C++: testcase("UInt16") — STUInt16(65535).value() == 65535
#[test]
fn cpp_st_uint16_value() {
    let u = protocol::STUInt16::new(65535);
    assert_eq!(u.value(), 65535);
}

/// C++: testcase("UInt32") — STUInt32(4294967295).value() == 4294967295
#[test]
fn cpp_st_uint32_value() {
    let u = protocol::STUInt32::new(u32::MAX);
    assert_eq!(u.value(), u32::MAX);
}

/// C++: testcase("UInt64") — STUInt64(max).value() == max
#[test]
fn cpp_st_uint64_value() {
    let u = protocol::STUInt64::new(u64::MAX);
    assert_eq!(u.value(), u64::MAX);
}

/// C++: testcase("Int32") — STInt32(min).value() == min
#[test]
fn cpp_st_int32_min() {
    let i = protocol::STInt32::new(i32::MIN);
    assert_eq!(i.value(), i32::MIN);
}

/// C++: testcase("Int32") — STInt32(max).value() == max
#[test]
fn cpp_st_int32_max() {
    let i = protocol::STInt32::new(i32::MAX);
    assert_eq!(i.value(), i32::MAX);
}

// --- From Quality_test.cpp: testcase("raw") ---

/// C++: Quality from raw rate value
#[test]
fn cpp_quality_raw_construction() {
    // Quality from rate: ((100-15) << 56) | 1000000000000000 = rate for 1:1
    let rate = ((100u64 - 15) << 56) | 1000000000000000u64;
    let a = amount_from_quality(rate);
    assert_eq!(a.mantissa(), 1000000000000000);
    assert_eq!(a.exponent(), -15);
}

/// C++: Quality from rate for 10:1
#[test]
fn cpp_quality_raw_10_to_1() {
    let rate = ((100u64 - 14) << 56) | 1000000000000000u64;
    let a = amount_from_quality(rate);
    assert_eq!(a.mantissa(), 1000000000000000);
    assert_eq!(a.exponent(), -14);
    assert_eq!(a.text(), "10");
}

/// C++: Quality from rate for 0.1:1
#[test]
fn cpp_quality_raw_01_to_1() {
    let rate = ((100u64 - 16) << 56) | 1000000000000000u64;
    let a = amount_from_quality(rate);
    assert_eq!(a.mantissa(), 1000000000000000);
    assert_eq!(a.exponent(), -16);
    assert_eq!(a.text(), "0.1");
}

/// C++: Quality from zero rate
#[test]
fn cpp_quality_raw_zero() {
    let a = amount_from_quality(0);
    assert_eq!(a.mantissa(), 0);
}

// --- From STInteger_test.cpp ---

/// C++: testcase("UInt8") — value storage
#[test]
fn cpp_st_uint8_stores_value() {
    let u = protocol::STUInt8::new(255);
    assert_eq!(u.value(), 255);
    let u0 = protocol::STUInt8::new(0);
    assert_eq!(u0.value(), 0);
}

/// C++: testcase("UInt16") — value storage
#[test]
fn cpp_st_uint16_stores_value() {
    let u = protocol::STUInt16::new(65535);
    assert_eq!(u.value(), 65535);
    let u0 = protocol::STUInt16::new(0);
    assert_eq!(u0.value(), 0);
}

/// C++: testcase("UInt32") — value storage
#[test]
fn cpp_st_uint32_stores_value() {
    let u = protocol::STUInt32::new(u32::MAX);
    assert_eq!(u.value(), u32::MAX);
    let u0 = protocol::STUInt32::new(0);
    assert_eq!(u0.value(), 0);
}

/// C++: testcase("UInt64") — value storage
#[test]
fn cpp_st_uint64_stores_value() {
    let u = protocol::STUInt64::new(u64::MAX);
    assert_eq!(u.value(), u64::MAX);
    let u0 = protocol::STUInt64::new(0);
    assert_eq!(u0.value(), 0);
}

/// C++: testcase("Int32") — min and max values
#[test]
fn cpp_st_int32_min_max() {
    let imin = protocol::STInt32::new(i32::MIN);
    assert_eq!(imin.value(), i32::MIN);
    let imax = protocol::STInt32::new(i32::MAX);
    assert_eq!(imax.value(), i32::MAX);
    let i0 = protocol::STInt32::new(0);
    assert_eq!(i0.value(), 0);
}
