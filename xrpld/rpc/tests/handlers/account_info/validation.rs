//! Account info validation tests.

use super::*;

#[test]
fn account_info_reports_amm_pseudo_account_type() {
    let ledger = closed_ledger();
    let account = sample_account(0x20);

    let mut source = FakeSource {
        ledger: Some(ledger),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_pseudo_account_root(account, "sfAMMID"));

    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 2,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("pseudo_account"),
        Some(&JsonValue::Object(std::collections::BTreeMap::from([(
            "type".to_owned(),
            JsonValue::String("AMM".to_owned()),
        )])))
    );
}

#[test]
fn account_info_trims_pseudo_account_id_suffix() {
    let ledger = closed_ledger();
    let vault = sample_account(0x21);
    let broker = sample_account(0x22);

    let mut source = FakeSource {
        ledger: Some(ledger),
        ..Default::default()
    };
    source
        .account_roots
        .insert(vault, make_pseudo_account_root(vault, "sfVaultID"));
    source
        .account_roots
        .insert(broker, make_pseudo_account_root(broker, "sfLoanBrokerID"));

    let vault_result = do_account_info(
        &AccountInfoRequest {
            params: &object([("account", JsonValue::String(to_base58(vault)))]),
            api_version: 2,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(vault_result) = vault_result else {
        panic!("vault result must be an object");
    };
    assert_eq!(
        vault_result.get("pseudo_account"),
        Some(&JsonValue::Object(std::collections::BTreeMap::from([(
            "type".to_owned(),
            JsonValue::String("Vault".to_owned()),
        )])))
    );

    let broker_result = do_account_info(
        &AccountInfoRequest {
            params: &object([("account", JsonValue::String(to_base58(broker)))]),
            api_version: 2,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(broker_result) = broker_result else {
        panic!("broker result must be an object");
    };
    assert_eq!(
        broker_result.get("pseudo_account"),
        Some(&JsonValue::Object(std::collections::BTreeMap::from([(
            "type".to_owned(),
            JsonValue::String("LoanBroker".to_owned()),
        )])))
    );
}

#[test]
fn account_info_reports_missing_and_invalid_account_fields() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let missing = do_account_info(
        &AccountInfoRequest {
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

    let invalid = do_account_info(
        &AccountInfoRequest {
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

    let malformed = do_account_info(
        &AccountInfoRequest {
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
    assert_eq!(malformed.get("error_code"), Some(&JsonValue::Signed(35)));
}

#[test]
fn account_info_reports_account_not_found() {
    let ledger = closed_ledger();
    let account = sample_account(0x11);
    let source = FakeSource {
        ledger: Some(ledger),
        ..Default::default()
    };

    let result = do_account_info(
        &AccountInfoRequest {
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
        result.get("account"),
        Some(&JsonValue::String(to_base58(account)))
    );
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("actNotFound".to_owned()))
    );
    assert_eq!(result.get("error_code"), Some(&JsonValue::Signed(19)));
}
