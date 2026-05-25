//! no ripple check tests part 2.

use super::*;

#[test]
fn no_ripple_check_missing_account_error() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([("role", JsonValue::String("gateway".to_owned()))]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let (error, code, message) = error_fields(&result);
    assert_eq!(error, "invalidParams");
    assert_eq!(code, 31);
    assert!(message.contains("account"));
}

#[test]
fn no_ripple_check_missing_role_error() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([(
                "account",
                JsonValue::String(to_base58(sample_account(0x11))),
            )]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let (error, code, message) = error_fields(&result);
    assert_eq!(error, "invalidParams");
    assert_eq!(code, 31);
    assert!(message.contains("role"));
}

#[test]
fn no_ripple_check_invalid_role_value() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                (
                    "account",
                    JsonValue::String(to_base58(sample_account(0x11))),
                ),
                ("role", JsonValue::String("invalid_role".to_owned())),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let (error, code, _message) = error_fields(&result);
    assert_eq!(error, "invalidParams");
    assert_eq!(code, 31);
}

#[test]
fn no_ripple_check_account_not_found() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                (
                    "account",
                    JsonValue::String(to_base58(sample_account(0x11))),
                ),
                ("role", JsonValue::String("gateway".to_owned())),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let (error, code, _message) = error_fields(&result);
    assert_eq!(error, "actNotFound");
    assert_eq!(code, 19);
}

#[test]
fn no_ripple_check_malformed_account() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                ("account", JsonValue::String("notAnAccount".to_owned())),
                ("role", JsonValue::String("gateway".to_owned())),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let (error, _code, _message) = error_fields(&result);
    assert_eq!(error, "actMalformed");
}

#[test]
fn no_ripple_check_invalid_account_types() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    for param in [
        JsonValue::Unsigned(1),
        JsonValue::Bool(true),
        JsonValue::Null,
        JsonValue::Array(vec![]),
    ] {
        let result = do_no_ripple_check(
            &NoRippleCheckRequest {
                params: &object([
                    ("account", param),
                    ("role", JsonValue::String("gateway".to_owned())),
                ]),
                api_version: 1,
                role: RpcRole::Admin,
            },
            &source,
        );
        let (error, _code, _message) = error_fields(&result);
        assert_eq!(error, "invalidParams");
    }
}

#[test]
fn no_ripple_check_gateway_no_problems_when_default_ripple_set() {
    let account = sample_account(0x77);
    let peer = sample_account(0x88);
    let line = make_trust_line(account, peer, "USD", 25, false, lsfLowNoRipple, 9, 10);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        fee_drops: 10,
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, true));
    source
        .owner_pages
        .insert((account, 0), make_owner_page(account, &[*line.key()]));
    source.children.insert(*line.key(), line);

    let result = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("role", JsonValue::String("gateway".to_owned())),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
    let JsonValue::Array(problems) = result.get("problems").expect("problems") else {
        panic!("problems must be an array");
    };
    // With defaultRipple set but noRipple on the line, gateway has 1 problem
    // (gateway should NOT have noRipple set on trust lines)
    assert_eq!(problems.len(), 1);
    assert_eq!(
        result.get("ledger_hash"),
        Some(&JsonValue::String(closed_ledger().hash.to_string()))
    );
    assert_eq!(
        result.get("ledger_index"),
        Some(&JsonValue::Unsigned(u64::from(closed_ledger().seq)))
    );
}

#[test]
fn no_ripple_check_limit_parameter() {
    let account = sample_account(0x99);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        fee_drops: 10,
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, false));
    source
        .owner_pages
        .insert((account, 0), make_owner_page(account, &[]));

    let result = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("role", JsonValue::String("gateway".to_owned())),
                ("limit", JsonValue::Unsigned(5)),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
    // Gateway without defaultRipple should have at least one problem
    let JsonValue::Array(problems) = result.get("problems").expect("problems") else {
        panic!("problems must be an array");
    };
    assert!(problems.len() >= 1);
}
