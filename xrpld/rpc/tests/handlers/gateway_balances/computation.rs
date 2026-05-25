//! gateway balances tests part 1.

use super::*;

#[test]
fn gateway_balances_reports_summary_and_locked() {
    let alice = sample_account(0x11);
    let hw = sample_account(0x22);
    let bob = sample_account(0x33);
    let charley = sample_account(0x44);
    let dave = sample_account(0x55);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source.account_roots.insert(alice, make_account_root(alice));

    let trustlines = vec![
        (
            sample_hash(0x01),
            make_trust_line(alice, hw, "USD", 5_000, true, 0),
        ),
        (
            sample_hash(0x02),
            make_trust_line(alice, hw, "JPY", 5_000, true, 0),
        ),
        (
            sample_hash(0x03),
            make_trust_line(alice, bob, "USD", 50, true, 0),
        ),
        (
            sample_hash(0x04),
            make_trust_line(alice, bob, "CNY", 0, true, 0),
        ),
        (
            sample_hash(0x05),
            make_trust_line(alice, charley, "CNY", 250, true, 0),
        ),
        (
            sample_hash(0x06),
            make_trust_line(alice, charley, "JPY", 250, true, 0),
        ),
        (
            sample_hash(0x07),
            make_trust_line(alice, dave, "CNY", 30, true, protocol::lsfLowFreeze),
        ),
        (
            sample_hash(0x08),
            make_trust_line(alice, charley, "USD", 10, false, 0),
        ),
    ];

    let escrow_iou = sample_hash(0x09);
    let escrow_mpt = sample_hash(0x0A);

    source
        .children
        .insert(trustlines[0].0, trustlines[0].1.clone());
    source
        .children
        .insert(trustlines[1].0, trustlines[1].1.clone());
    source
        .children
        .insert(trustlines[2].0, trustlines[2].1.clone());
    source
        .children
        .insert(trustlines[3].0, trustlines[3].1.clone());
    source
        .children
        .insert(trustlines[4].0, trustlines[4].1.clone());
    source
        .children
        .insert(trustlines[5].0, trustlines[5].1.clone());
    source
        .children
        .insert(trustlines[6].0, trustlines[6].1.clone());
    source
        .children
        .insert(trustlines[7].0, trustlines[7].1.clone());
    source
        .children
        .insert(escrow_iou, make_escrow_iou(alice, "USD", 7));
    source
        .children
        .insert(escrow_mpt, make_escrow_mpt(alice, 9, 100));

    source.owner_pages.insert(
        (alice, 0),
        make_owner_page(
            alice,
            0,
            &[
                trustlines[0].0,
                trustlines[1].0,
                trustlines[2].0,
                trustlines[3].0,
                trustlines[4].0,
                trustlines[5].0,
                trustlines[6].0,
                trustlines[7].0,
                escrow_iou,
                escrow_mpt,
            ],
            0,
        ),
    );

    let result = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(alice))),
                ("hotwallet", JsonValue::String(to_base58(hw))),
            ]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );

    let JsonValue::Object(result) = result else {
        panic!("response must be an object");
    };
    assert_eq!(
        result.get("account"),
        Some(&JsonValue::String(to_base58(alice)))
    );
    assert!(!result.contains_key("error"));

    let JsonValue::Object(balances) = result.get("balances").expect("balances") else {
        panic!("balances must be an object");
    };
    let JsonValue::Array(hw_balances) = balances.get(&to_base58(hw)).expect("hotwallet") else {
        panic!("hotwallet balances must be an array");
    };
    assert_eq!(hw_balances.len(), 2);
    assert!(
        hw_balances
            .iter()
            .any(|entry| json_object_get(entry, "currency")
                == Some(&JsonValue::String("USD".to_owned()))
                && json_object_get(entry, "value") == Some(&JsonValue::String("5000".to_owned())))
    );
    assert!(
        hw_balances
            .iter()
            .any(|entry| json_object_get(entry, "currency")
                == Some(&JsonValue::String("JPY".to_owned()))
                && json_object_get(entry, "value") == Some(&JsonValue::String("5000".to_owned())))
    );

    let JsonValue::Object(frozen_balances) = result.get("frozen_balances").expect("frozen") else {
        panic!("frozen_balances must be an object");
    };
    let JsonValue::Array(dave_balances) = frozen_balances.get(&to_base58(dave)).expect("dave")
    else {
        panic!("dave frozen balances must be an array");
    };
    assert_eq!(dave_balances.len(), 1);
    assert_eq!(
        json_object_get(&dave_balances[0], "currency"),
        Some(&JsonValue::String("CNY".to_owned()))
    );
    assert_eq!(
        json_object_get(&dave_balances[0], "value"),
        Some(&JsonValue::String("30".to_owned()))
    );

    let JsonValue::Object(assets) = result.get("assets").expect("assets") else {
        panic!("assets must be an object");
    };
    let JsonValue::Array(charley_assets) = assets.get(&to_base58(charley)).expect("charley") else {
        panic!("charley assets must be an array");
    };
    assert_eq!(charley_assets.len(), 1);
    assert_eq!(
        json_object_get(&charley_assets[0], "currency"),
        Some(&JsonValue::String("USD".to_owned()))
    );
    assert_eq!(
        json_object_get(&charley_assets[0], "value"),
        Some(&JsonValue::String("10".to_owned()))
    );

    let JsonValue::Object(obligations) = result.get("obligations").expect("obligations") else {
        panic!("obligations must be an object");
    };
    assert_eq!(
        obligations.get("CNY"),
        Some(&JsonValue::String("250".to_owned()))
    );
    assert_eq!(
        obligations.get("JPY"),
        Some(&JsonValue::String("250".to_owned()))
    );
    assert_eq!(
        obligations.get("USD"),
        Some(&JsonValue::String("50".to_owned()))
    );

    let JsonValue::Object(locked) = result.get("locked").expect("locked") else {
        panic!("locked must be an object");
    };
    assert_eq!(locked.get("USD"), Some(&JsonValue::String("7".to_owned())));
    assert_eq!(locked.len(), 1);
}

#[test]
fn gateway_balances_reports_validation_and_missing_account() {
    let alice = sample_account(0x11);
    let missing = sample_account(0x66);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source.account_roots.insert(alice, make_account_root(alice));

    let missing_field = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &JsonValue::Object(BTreeMap::new()),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(missing_field) = missing_field else {
        panic!("missing-field response must be an object");
    };
    assert_eq!(
        missing_field.get("error_message"),
        Some(&JsonValue::String("Missing field 'account'.".to_owned()))
    );

    let malformed = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &object([("account", JsonValue::String("foo".to_owned()))]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    assert_eq!(
        error_fields(&malformed),
        ("actMalformed", 35, "Account malformed.")
    );

    let not_found = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &object([("account", JsonValue::String(to_base58(missing)))]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(not_found) = not_found else {
        panic!("not-found response must be an object");
    };
    assert_eq!(
        not_found.get("account"),
        Some(&JsonValue::String(to_base58(missing)))
    );
    assert_eq!(
        not_found.get("error"),
        Some(&JsonValue::String("actNotFound".to_owned()))
    );

    let v1_missing = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &object([("account", JsonValue::String(to_base58(missing)))]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(v1_missing) = v1_missing else {
        panic!("api v1 missing-account response must be an object");
    };
    assert_eq!(
        v1_missing.get("account"),
        Some(&JsonValue::String(to_base58(missing)))
    );
    assert!(!v1_missing.contains_key("error"));

    let invalid_hotwallet_v1 = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(alice))),
                ("hotwallet", JsonValue::String("asdf".to_owned())),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    assert_eq!(
        error_fields(&invalid_hotwallet_v1),
        ("invalidHotWallet", 30, "Invalid hotwallet.")
    );

    let invalid_hotwallet_v2 = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(alice))),
                ("hotwallet", JsonValue::Unsigned(7)),
            ]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    assert_eq!(
        error_fields(&invalid_hotwallet_v2),
        ("invalidParams", 31, "Invalid parameters.")
    );
}
