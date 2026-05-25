//! gateway balances tests part 2.

use super::*;

#[test]
fn gateway_balances_skips_missing_owner_directory_children_iteration() {
    let issuer = sample_account(0x61);
    let peer = sample_account(0x62);
    let missing = sample_hash(0x90);
    let trust_line = sample_hash(0x91);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .account_roots
        .insert(issuer, make_account_root(issuer));
    source.owner_pages.insert(
        (issuer, 0),
        make_owner_page(issuer, 0, &[missing, trust_line], 0),
    );
    source.children.insert(
        trust_line,
        make_trust_line(issuer, peer, "USD", 75, true, 0),
    );

    let result = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &object([("account", JsonValue::String(to_base58(issuer)))]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );

    let JsonValue::Object(result) = result else {
        panic!("response must be an object");
    };
    assert!(!result.contains_key("error"));

    let JsonValue::Object(obligations) = result.get("obligations").expect("obligations") else {
        panic!("obligations must be an object");
    };
    assert_eq!(
        obligations.get("USD"),
        Some(&JsonValue::String("75".to_owned()))
    );
}

#[test]
fn gateway_balances_response_structure_fields() {
    let issuer = sample_account(0x71);
    let peer1 = sample_account(0x72);
    let peer2 = sample_account(0x73);
    let line1 = make_trust_line(issuer, peer1, "USD", 50, true, 0);
    let line2 = make_trust_line(issuer, peer2, "EUR", 100, true, 0);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .account_roots
        .insert(issuer, make_account_root(issuer));
    source.owner_pages.insert(
        (issuer, 0),
        make_owner_page(issuer, 0, &[*line1.key(), *line2.key()], 0),
    );
    source.children.insert(*line1.key(), line1);
    source.children.insert(*line2.key(), line2);

    let result = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &object([("account", JsonValue::String(to_base58(issuer)))]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("response must be an object");
    };
    // Verify top-level response structure
    assert_eq!(result.get("error"), None);
    assert_eq!(
        result.get("account"),
        Some(&JsonValue::String(to_base58(issuer)))
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

    // Verify obligations
    let JsonValue::Object(obligations) = result.get("obligations").expect("obligations") else {
        panic!("obligations must be an object");
    };
    assert_eq!(
        obligations.get("USD"),
        Some(&JsonValue::String("50".to_owned()))
    );
    assert_eq!(
        obligations.get("EUR"),
        Some(&JsonValue::String("100".to_owned()))
    );
}

#[test]
fn gateway_balances_hotwallet_separates_balances() {
    let issuer = sample_account(0x81);
    let hotwallet = sample_account(0x82);
    let client = sample_account(0x83);
    let hw_line = make_trust_line(issuer, hotwallet, "USD", 5000, true, 0);
    let client_line = make_trust_line(issuer, client, "USD", 50, true, 0);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .account_roots
        .insert(issuer, make_account_root(issuer));
    source.owner_pages.insert(
        (issuer, 0),
        make_owner_page(issuer, 0, &[*hw_line.key(), *client_line.key()], 0),
    );
    source.children.insert(*hw_line.key(), hw_line);
    source.children.insert(*client_line.key(), client_line);

    let result = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(issuer))),
                ("hotwallet", JsonValue::String(to_base58(hotwallet))),
            ]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("response must be an object");
    };
    assert_eq!(result.get("error"), None);

    // Hotwallet balance should be in "balances" section
    let JsonValue::Object(balances) = result.get("balances").expect("balances") else {
        panic!("balances must be an object");
    };
    assert!(balances.contains_key(&to_base58(hotwallet)));

    // Client balance should be in "obligations" section
    let JsonValue::Object(obligations) = result.get("obligations").expect("obligations") else {
        panic!("obligations must be an object");
    };
    assert_eq!(
        obligations.get("USD"),
        Some(&JsonValue::String("50".to_owned()))
    );
}

#[test]
fn gateway_balances_missing_account_error() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &JsonValue::Object(Default::default()),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    let (error, code, message) = error_fields(&result);
    assert_eq!(error, "invalidParams");
    assert_eq!(code, 31);
    assert_eq!(message, "Missing field 'account'.");
}

#[test]
fn gateway_balances_account_not_found() {
    let account = sample_account(0x91);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    let (error, code, message) = error_fields(&result);
    assert_eq!(error, "actNotFound");
    assert_eq!(code, 19);
    assert_eq!(message, "Account not found.");
}

#[test]
fn gateway_balances_malformed_account() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_gateway_balances(
        &GatewayBalancesRequest {
            params: &object([("account", JsonValue::String("notAnAccount".to_owned()))]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    let (error, _code, _message) = error_fields(&result);
    assert_eq!(error, "actMalformed");
}
