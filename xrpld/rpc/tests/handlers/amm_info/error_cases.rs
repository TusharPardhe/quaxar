//! AMM info handler tests part 2.

use super::*;

#[test]
fn amm_info_account_not_found() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_amm_info(
        &AmmInfoRequest {
            params: &object([(
                "amm_account",
                JsonValue::String(to_base58(sample_account(0x99))),
            )]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    // When account doesn't exist, handler returns actMalformed
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("actMalformed".to_owned()))
    );
}

#[test]
fn amm_info_malformed_account() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_amm_info(
        &AmmInfoRequest {
            params: &object([("amm_account", JsonValue::String("notAnAccount".to_owned()))]),
            api_version: 2,
            role: RpcRole::Admin,
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
    assert_eq!(
        result.get("error_message"),
        Some(&JsonValue::String("Account malformed.".to_owned()))
    );
}

#[test]
fn amm_info_invalid_asset_params() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    // Non-object asset
    let result = do_amm_info(
        &AmmInfoRequest {
            params: &object([
                ("asset", JsonValue::String("XRP".to_owned())),
                ("asset2", JsonValue::String("USD".to_owned())),
            ]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("error"));
}

#[test]
fn amm_info_missing_both_account_and_assets() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_amm_info(
        &AmmInfoRequest {
            params: &object([]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("error"));
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
}
