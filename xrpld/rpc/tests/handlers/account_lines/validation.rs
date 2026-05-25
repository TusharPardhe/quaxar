//! Account lines validation and error tests.

use super::*;

#[test]
fn account_lines_reports_missing_invalid_and_malformed() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let missing = do_account_lines(
        &AccountLinesRequest {
            params: &JsonValue::Object(Default::default()),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(missing) = missing else {
        panic!("missing response must be an object");
    };
    assert_eq!(
        missing.get("error_message"),
        Some(&JsonValue::String("Missing field 'account'.".to_owned()))
    );

    let invalid = do_account_lines(
        &AccountLinesRequest {
            params: &object([("account", JsonValue::Unsigned(1))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(invalid) = invalid else {
        panic!("invalid response must be an object");
    };
    assert_eq!(
        invalid.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    let malformed = do_account_lines(
        &AccountLinesRequest {
            params: &object([("account", JsonValue::String("foo".to_owned()))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(malformed) = malformed else {
        panic!("malformed response must be an object");
    };
    assert_eq!(
        malformed.get("error"),
        Some(&JsonValue::String("actMalformed".to_owned()))
    );
}

#[test]
fn account_lines_respect_marker_limit_peer_and_ignore_default() {
    let account = sample_account(0x11);
    let peer = sample_account(0x22);
    let other_peer = sample_account(0x33);
    let line1 = make_trust_line(account, peer, "USD", 50, false, lsfLowAuth, 9, 0);
    let line2 = make_trust_line(account, other_peer, "EUR", 25, false, lsfLowReserve, 10, 0);
    let line3 = make_trust_line(account, peer, "JPY", 75, true, lsfLowReserve, 11, 0);

    let page0 = make_owner_page(account, 0, &[*line1.key(), *line2.key()], 1);
    let page1 = make_owner_page(account, 1, &[*line3.key()], 0);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(account, make_account_root(account))]),
        owner_pages: BTreeMap::from([((account, 0), page0), ((account, 1), page1)]),
        children: BTreeMap::from([
            (*line1.key(), line1.clone()),
            (*line2.key(), line2.clone()),
            (*line3.key(), line3.clone()),
        ]),
    };

    let limited = do_account_lines(
        &AccountLinesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("limit", JsonValue::Unsigned(2)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(limited) = limited else {
        panic!("limited response must be an object");
    };
    assert_eq!(limited.get("limit"), Some(&JsonValue::Unsigned(2)));
    assert_eq!(
        limited.get("marker"),
        Some(&JsonValue::String(format!("{},{}", line2.key(), 10)))
    );
    let JsonValue::Array(lines) = limited.get("lines").expect("lines array") else {
        panic!("lines must be an array");
    };
    assert_eq!(lines.len(), 2);
    let JsonValue::Object(first) = &lines[0] else {
        panic!("line must be an object");
    };
    assert_eq!(
        first.get("account"),
        Some(&JsonValue::String(to_base58(peer)))
    );
    assert_eq!(
        first.get("balance"),
        Some(&JsonValue::String("50".to_owned()))
    );
    assert_eq!(first.get("authorized"), Some(&JsonValue::Bool(true)));

    let filtered = do_account_lines(
        &AccountLinesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("peer", JsonValue::String(to_base58(peer))),
                ("ignore_default", JsonValue::Bool(true)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(filtered) = filtered else {
        panic!("filtered response must be an object");
    };
    let JsonValue::Array(filtered_lines) = filtered.get("lines").expect("lines array") else {
        panic!("lines must be an array");
    };
    assert_eq!(filtered_lines.len(), 1);
    let JsonValue::Object(only) = &filtered_lines[0] else {
        panic!("line must be an object");
    };
    assert_eq!(
        only.get("balance"),
        Some(&JsonValue::String("-75".to_owned()))
    );
    assert_eq!(
        only.get("account"),
        Some(&JsonValue::String(to_base58(peer)))
    );
}

#[test]
fn account_lines_reject_marker_for_unrelated_object() {
    let account = sample_account(0x44);
    let peer = sample_account(0x55);
    let outsider = sample_account(0x66);
    let related = make_trust_line(account, peer, "USD", 10, false, lsfLowReserve, 3, 0);
    let unrelated = make_trust_line(outsider, peer, "USD", 20, false, lsfLowReserve, 4, 0);
    let page0 = make_owner_page(account, 0, &[*related.key()], 0);

    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                (
                    "marker",
                    JsonValue::String(format!("{},{}", unrelated.key(), 4)),
                ),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &FakeSource {
            ledger: Some(closed_ledger()),
            account_roots: BTreeMap::from([(account, make_account_root(account))]),
            owner_pages: BTreeMap::from([((account, 0), page0)]),
            children: BTreeMap::from([(*related.key(), related), (*unrelated.key(), unrelated)]),
        },
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
}

#[test]
fn account_lines_returns_all_trust_line_fields_for_low_account() {
    // When the querying account is the low account, we should see:
    // - balance as-is (not negated)
    // - limit from sfLowLimit
    // - limit_peer from sfHighLimit
    // - quality_in from sfLowQualityIn
    // - quality_out from sfLowQualityOut
    let account = sample_account(0x11); // low account (0x11 < 0x22)
    let peer = sample_account(0x22);
    let line = make_trust_line(account, peer, "USD", 50, false, 0, 9, 10);

    let page0 = make_owner_page(account, 0, &[*line.key()], 0);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(account, make_account_root(account))]),
        owner_pages: BTreeMap::from([((account, 0), page0)]),
        children: BTreeMap::from([(*line.key(), line.clone())]),
    };

    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    // Verify top-level response fields
    assert_eq!(
        result.get("account"),
        Some(&JsonValue::String(to_base58(account)))
    );
    assert_eq!(
        result.get("ledger_hash"),
        Some(&JsonValue::String(closed_ledger().hash.to_string()))
    );
    assert_eq!(
        result.get("ledger_index"),
        Some(&JsonValue::Unsigned(u64::from(closed_ledger().seq)))
    );
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(true)));
    assert!(!result.contains_key("marker"));

    let JsonValue::Array(lines) = result.get("lines").expect("lines") else {
        panic!("lines must be an array");
    };
    assert_eq!(lines.len(), 1);

    let JsonValue::Object(line_obj) = &lines[0] else {
        panic!("line must be an object");
    };
    assert_eq!(
        line_obj.get("account"),
        Some(&JsonValue::String(to_base58(peer)))
    );
    assert_eq!(
        line_obj.get("balance"),
        Some(&JsonValue::String("50".to_owned()))
    );
    assert_eq!(
        line_obj.get("currency"),
        Some(&JsonValue::String("USD".to_owned()))
    );
    assert_eq!(
        line_obj.get("limit"),
        Some(&JsonValue::String("500".to_owned()))
    );
    assert_eq!(
        line_obj.get("limit_peer"),
        Some(&JsonValue::String("800".to_owned()))
    );
    assert_eq!(line_obj.get("quality_in"), Some(&JsonValue::Unsigned(11)));
    assert_eq!(line_obj.get("quality_out"), Some(&JsonValue::Unsigned(12)));
    // No flags set, so these should be absent
    assert!(!line_obj.contains_key("authorized"));
    assert!(!line_obj.contains_key("peer_authorized"));
    assert!(!line_obj.contains_key("no_ripple"));
    assert!(!line_obj.contains_key("no_ripple_peer"));
    assert!(!line_obj.contains_key("freeze"));
    assert!(!line_obj.contains_key("freeze_peer"));
    assert!(!line_obj.contains_key("deep_freeze"));
    assert!(!line_obj.contains_key("deep_freeze_peer"));
}

#[test]
fn account_lines_returns_negated_balance_for_high_account() {
    // When the querying account is the high account, balance is negated
    let low = sample_account(0x11);
    let high = sample_account(0x22); // querying as high account
    let line = make_trust_line(low, high, "EUR", 75, false, 0, 5, 6);

    let page0 = make_owner_page(high, 0, &[*line.key()], 0);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(high, make_account_root(high))]),
        owner_pages: BTreeMap::from([((high, 0), page0)]),
        children: BTreeMap::from([(*line.key(), line.clone())]),
    };

    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([("account", JsonValue::String(to_base58(high)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Array(lines) = result.get("lines").expect("lines") else {
        panic!("lines must be an array");
    };
    assert_eq!(lines.len(), 1);

    let JsonValue::Object(line_obj) = &lines[0] else {
        panic!("line must be an object");
    };
    // Balance is negated for high account
    assert_eq!(
        line_obj.get("balance"),
        Some(&JsonValue::String("-75".to_owned()))
    );
    // Peer is the low account
    assert_eq!(
        line_obj.get("account"),
        Some(&JsonValue::String(to_base58(low)))
    );
    // limit/limit_peer are swapped for high account
    assert_eq!(
        line_obj.get("limit"),
        Some(&JsonValue::String("800".to_owned()))
    );
    assert_eq!(
        line_obj.get("limit_peer"),
        Some(&JsonValue::String("500".to_owned()))
    );
    // quality_in/out come from high side
    assert_eq!(line_obj.get("quality_in"), Some(&JsonValue::Unsigned(21)));
    assert_eq!(line_obj.get("quality_out"), Some(&JsonValue::Unsigned(22)));
}

#[test]
fn account_lines_shows_all_flags_when_low_account_has_flags() {
    use protocol::{
        lsfHighAuth, lsfHighDeepFreeze, lsfHighFreeze, lsfHighNoRipple, lsfLowAuth,
        lsfLowDeepFreeze, lsfLowFreeze, lsfLowNoRipple,
    };

    let account = sample_account(0x11); // low account
    let peer = sample_account(0x22);
    let all_flags = lsfLowAuth
        | lsfHighAuth
        | lsfLowNoRipple
        | lsfHighNoRipple
        | lsfLowFreeze
        | lsfHighFreeze
        | lsfLowDeepFreeze
        | lsfHighDeepFreeze;
    let line = make_trust_line(account, peer, "JPY", 100, false, all_flags, 1, 2);

    let page0 = make_owner_page(account, 0, &[*line.key()], 0);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(account, make_account_root(account))]),
        owner_pages: BTreeMap::from([((account, 0), page0)]),
        children: BTreeMap::from([(*line.key(), line.clone())]),
    };

    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Array(lines) = result.get("lines").expect("lines") else {
        panic!("lines must be an array");
    };
    assert_eq!(lines.len(), 1);

    let JsonValue::Object(line_obj) = &lines[0] else {
        panic!("line must be an object");
    };
    // Low account sees: authorized (lsfLowAuth), peer_authorized (lsfHighAuth)
    assert_eq!(line_obj.get("authorized"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        line_obj.get("peer_authorized"),
        Some(&JsonValue::Bool(true))
    );
    // Low account sees: no_ripple (lsfLowNoRipple), no_ripple_peer (lsfHighNoRipple)
    assert_eq!(line_obj.get("no_ripple"), Some(&JsonValue::Bool(true)));
    assert_eq!(line_obj.get("no_ripple_peer"), Some(&JsonValue::Bool(true)));
    // Low account sees: freeze (lsfLowFreeze), freeze_peer (lsfHighFreeze)
    assert_eq!(line_obj.get("freeze"), Some(&JsonValue::Bool(true)));
    assert_eq!(line_obj.get("freeze_peer"), Some(&JsonValue::Bool(true)));
    // Low account sees: deep_freeze (lsfLowDeepFreeze), deep_freeze_peer (lsfHighDeepFreeze)
    assert_eq!(line_obj.get("deep_freeze"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        line_obj.get("deep_freeze_peer"),
        Some(&JsonValue::Bool(true))
    );
}

#[test]
fn account_lines_shows_swapped_flags_for_high_account() {
    use protocol::{
        lsfHighAuth, lsfHighFreeze, lsfHighNoRipple, lsfLowAuth, lsfLowFreeze, lsfLowNoRipple,
    };

    let low = sample_account(0x11);
    let high = sample_account(0x22); // querying as high
    let flags =
        lsfLowAuth | lsfHighAuth | lsfLowNoRipple | lsfHighNoRipple | lsfLowFreeze | lsfHighFreeze;
    let line = make_trust_line(low, high, "GBP", 30, false, flags, 1, 2);

    let page0 = make_owner_page(high, 0, &[*line.key()], 0);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(high, make_account_root(high))]),
        owner_pages: BTreeMap::from([((high, 0), page0)]),
        children: BTreeMap::from([(*line.key(), line.clone())]),
    };

    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([("account", JsonValue::String(to_base58(high)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Array(lines) = result.get("lines").expect("lines") else {
        panic!("lines must be an array");
    };
    let JsonValue::Object(line_obj) = &lines[0] else {
        panic!("line must be an object");
    };
    // High account sees: authorized (lsfHighAuth), peer_authorized (lsfLowAuth)
    assert_eq!(line_obj.get("authorized"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        line_obj.get("peer_authorized"),
        Some(&JsonValue::Bool(true))
    );
    // High account sees: no_ripple (lsfHighNoRipple), no_ripple_peer (lsfLowNoRipple)
    assert_eq!(line_obj.get("no_ripple"), Some(&JsonValue::Bool(true)));
    assert_eq!(line_obj.get("no_ripple_peer"), Some(&JsonValue::Bool(true)));
    // High account sees: freeze (lsfHighFreeze), freeze_peer (lsfLowFreeze)
    assert_eq!(line_obj.get("freeze"), Some(&JsonValue::Bool(true)));
    assert_eq!(line_obj.get("freeze_peer"), Some(&JsonValue::Bool(true)));
}

#[test]
fn account_lines_negative_limit_rejected() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    let account = sample_account(0x77);

    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("limit", JsonValue::Signed(-1)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(
        result.get("error").is_some() || result.get("error_message").is_some(),
        "negative limit should produce an error"
    );
}
