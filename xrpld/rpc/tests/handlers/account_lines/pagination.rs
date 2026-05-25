//! Account lines pagination and marker tests.

use super::*;

#[test]
fn account_lines_non_string_marker_rejected() {
    let account = sample_account(0x88);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account));

    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("marker", JsonValue::Bool(true)),
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
        "non-string marker should produce an error"
    );
}

#[test]
fn account_lines_malformed_peer_rejected() {
    let account = sample_account(0x99);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account));

    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("peer", JsonValue::String("notAValidAddress".to_owned())),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("actMalformed".to_owned()))
    );
}

#[test]
fn account_lines_account_not_found() {
    let account = sample_account(0xAA);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
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
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("actNotFound".to_owned()))
    );
    assert_eq!(
        result.get("error_message"),
        Some(&JsonValue::String("Account not found.".to_owned()))
    );
    assert_eq!(result.get("error_code"), Some(&JsonValue::Signed(19)));
}

#[test]
fn account_lines_empty_lines_array_for_funded_account_with_no_lines() {
    let account = sample_account(0xBB);
    let page0 = make_owner_page(account, 0, &[], 0);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(account, make_account_root(account))]),
        owner_pages: BTreeMap::from([((account, 0), page0)]),
        children: BTreeMap::new(),
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
    assert_eq!(result.get("error"), None);
    let JsonValue::Array(lines) = result.get("lines").expect("lines") else {
        panic!("lines must be an array");
    };
    assert_eq!(lines.len(), 0);
    assert_eq!(
        result.get("account"),
        Some(&JsonValue::String(to_base58(account)))
    );
}

#[test]
fn account_lines_pagination_marker_continuation() {
    let account = sample_account(0xCC);
    let peer1 = sample_account(0xDD);
    let peer2 = sample_account(0xEE);
    let peer3 = sample_account(0xFF);
    let line1 = make_trust_line(account, peer1, "USD", 10, false, lsfLowReserve, 1, 0);
    let line2 = make_trust_line(account, peer2, "EUR", 20, false, lsfLowReserve, 2, 0);
    let line3 = make_trust_line(account, peer3, "JPY", 30, false, lsfLowReserve, 3, 0);

    let page0 = make_owner_page(account, 0, &[*line1.key(), *line2.key(), *line3.key()], 0);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(account, make_account_root(account))]),
        owner_pages: BTreeMap::from([((account, 0), page0)]),
        children: BTreeMap::from([
            (*line1.key(), line1.clone()),
            (*line2.key(), line2.clone()),
            (*line3.key(), line3.clone()),
        ]),
    };

    // Request with limit=1
    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("limit", JsonValue::Unsigned(1)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("limit"), Some(&JsonValue::Unsigned(1)));
    let JsonValue::Array(lines) = result.get("lines").expect("lines") else {
        panic!("lines must be an array");
    };
    assert_eq!(lines.len(), 1);
    // Should have a marker for continuation
    assert!(
        result.contains_key("marker"),
        "should have marker for continuation"
    );
    let JsonValue::String(marker) = result.get("marker").unwrap() else {
        panic!("marker must be a string");
    };
    assert!(!marker.is_empty());

    // Continue with marker
    let result2 = do_account_lines(
        &AccountLinesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("marker", JsonValue::String(marker.clone())),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result2) = result2 else {
        panic!("result2 must be an object");
    };
    let JsonValue::Array(lines2) = result2.get("lines").expect("lines") else {
        panic!("lines must be an array");
    };
    // Should get remaining 2 lines
    assert_eq!(lines2.len(), 2);
    assert!(!result2.contains_key("marker"), "no more pages");
}

#[test]
fn account_lines_invalid_account_types_rejected() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    // Integer account
    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([("account", JsonValue::Unsigned(42))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
    assert_eq!(
        result.get("error_message"),
        Some(&JsonValue::String("Invalid field 'account'.".to_owned()))
    );

    // Boolean account
    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([("account", JsonValue::Bool(true))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    // Null account
    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([("account", JsonValue::Null)]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    // Array account
    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([("account", JsonValue::Array(vec![]))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    // Object account
    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([("account", JsonValue::Object(Default::default()))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
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
fn account_lines_negative_balance_display() {
    let account = sample_account(0x11);
    let peer = sample_account(0x22);
    // balance_negative=true means the stored balance is negative
    let line = make_trust_line(account, peer, "BTC", 99, true, lsfLowReserve, 1, 2);

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
    assert_eq!(
        line_obj.get("balance"),
        Some(&JsonValue::String("-99".to_owned()))
    );
    assert_eq!(
        line_obj.get("currency"),
        Some(&JsonValue::String("BTC".to_owned()))
    );
}

#[test]
fn account_lines_marker_points_to_correct_entry() {
    let account = sample_account(0xD1);
    let peer1 = sample_account(0xD2);
    let peer2 = sample_account(0xD3);
    let line1 = make_trust_line(account, peer1, "USD", 10, false, lsfLowReserve, 1, 0);
    let line2 = make_trust_line(account, peer2, "EUR", 20, false, lsfLowReserve, 2, 0);

    let page0 = make_owner_page(account, 0, &[*line1.key(), *line2.key()], 0);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(account, make_account_root(account))]),
        owner_pages: BTreeMap::from([((account, 0), page0)]),
        children: BTreeMap::from([(*line1.key(), line1.clone()), (*line2.key(), line2.clone())]),
    };

    // Get first line with limit=1
    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("limit", JsonValue::Unsigned(1)),
            ]),
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

    // Marker should contain the key of the last returned entry
    let JsonValue::String(marker) = result.get("marker").expect("marker") else {
        panic!("marker must be a string");
    };
    // Marker format is "key,hint"
    let parts: Vec<&str> = marker.split(',').collect();
    assert_eq!(parts.len(), 2, "marker should be key,hint format");
    assert_eq!(parts[0].len(), 64, "marker key should be 64 hex chars");
}

#[test]
fn account_lines_both_hash_and_index_rejected() {
    let account = sample_account(0xD4);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(account, make_account_root(account))]),
        ..Default::default()
    };

    let result = do_account_lines(
        &AccountLinesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                (
                    "ledger_hash",
                    JsonValue::String(closed_ledger().hash.to_string()),
                ),
                ("ledger_index", JsonValue::Unsigned(101)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    // Should reject specifying both hash and index
    assert!(
        result.contains_key("error"),
        "specifying both ledger_hash and ledger_index should error"
    );
}

#[test]
fn account_lines_quality_fields_present_for_non_default() {
    let account = sample_account(0xD5);
    let peer = sample_account(0xD6);
    // Create line with non-default quality values
    let line = make_trust_line(account, peer, "USD", 100, false, lsfLowReserve, 1, 0);

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
    let JsonValue::Object(line_obj) = &lines[0] else {
        panic!("line must be an object");
    };
    // Quality fields should be present (our make_trust_line sets them to 11/12/21/22)
    assert!(
        matches!(line_obj.get("quality_in"), Some(JsonValue::Unsigned(v)) if *v > 0),
        "quality_in should be present and non-zero"
    );
    assert!(
        matches!(line_obj.get("quality_out"), Some(JsonValue::Unsigned(v)) if *v > 0),
        "quality_out should be present and non-zero"
    );
}
