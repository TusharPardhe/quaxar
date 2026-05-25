//! no ripple check tests part 1.

use super::*;

#[test]
fn no_ripple_check_reports_account_role_and_request_errors() {
    let account = sample_account(0x11);
    let unrelated = sample_account(0x22);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, false));

    let missing = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &JsonValue::Object(Default::default()),
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    assert_eq!(
        error_fields(&missing),
        ("invalidParams", 31, "Missing field 'account'.")
    );

    let missing_role = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    assert_eq!(
        error_fields(&missing_role),
        ("invalidParams", 31, "Missing field 'role'.")
    );

    let invalid_account = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                ("account", JsonValue::Unsigned(1)),
                ("role", JsonValue::String("user".to_owned())),
            ]),
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    assert_eq!(
        error_fields(&invalid_account),
        ("invalidParams", 31, "Invalid field 'account'.")
    );

    let invalid_role = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("role", JsonValue::Unsigned(1)),
            ]),
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    assert_eq!(
        error_fields(&invalid_role),
        ("invalidParams", 31, "Invalid field 'role'.")
    );

    let bad_limit = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("role", JsonValue::String("user".to_owned())),
                ("limit", JsonValue::String("ten".to_owned())),
            ]),
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    assert_eq!(
        error_fields(&bad_limit),
        (
            "invalidParams",
            31,
            "Invalid field 'limit', not unsigned integer."
        )
    );

    let bad_transactions = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("role", JsonValue::String("user".to_owned())),
                ("transactions", JsonValue::String("yes".to_owned())),
            ]),
            api_version: 2,
            role: RpcRole::User,
        },
        &source,
    );
    assert_eq!(
        error_fields(&bad_transactions),
        (
            "invalidParams",
            31,
            "Invalid field 'transactions', not bool."
        )
    );

    let malformed = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                ("account", JsonValue::String("foo".to_owned())),
                ("role", JsonValue::String("user".to_owned())),
            ]),
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    assert_eq!(error_fields(&malformed).0, "actMalformed");
    assert_eq!(error_fields(&malformed).1, 35);

    let not_found = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(unrelated))),
                ("role", JsonValue::String("user".to_owned())),
            ]),
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    assert_eq!(error_fields(&not_found).0, "actNotFound");
}

#[test]
fn no_ripple_check_reports_gateway_problems_and_transactions() {
    let account = sample_account(0x11);
    let peer = sample_account(0x22);
    let line_key = sample_hash(0x33);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        fee_drops: 12,
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, false));
    source
        .owner_pages
        .insert((account, 0), make_owner_page(account, &[line_key]));
    source.children.insert(
        line_key,
        make_trust_line(account, peer, "USD", 25, false, lsfLowNoRipple, 9, 10),
    );

    let response = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("role", JsonValue::String("gateway".to_owned())),
                ("transactions", JsonValue::Bool(true)),
            ]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );

    let JsonValue::Object(object) = response else {
        panic!("expected object response");
    };
    let JsonValue::Array(problems) = object.get("problems").expect("problems") else {
        panic!("problems should be an array");
    };
    assert_eq!(problems.len(), 2);
    assert_eq!(
        problems[0],
        JsonValue::String("You should immediately set your default ripple flag".to_owned())
    );
    assert_eq!(
        problems[1],
        JsonValue::String(format!(
            "You should clear the no ripple flag on your USD line to {}",
            to_base58(peer)
        ))
    );

    let JsonValue::Array(transactions) = object.get("transactions").expect("transactions") else {
        panic!("transactions should be an array");
    };
    assert_eq!(transactions.len(), 2);

    let JsonValue::Object(account_set) = &transactions[0] else {
        panic!("first transaction should be an object");
    };
    assert_eq!(
        account_set.get("TransactionType"),
        Some(&JsonValue::String(
            protocol::TxType::ACCOUNT_SET
                .format_name()
                .expect("account set format")
                .to_owned()
        ))
    );
    assert_eq!(account_set.get("Sequence"), Some(&JsonValue::Unsigned(7)));
    assert_eq!(
        account_set.get("Account"),
        Some(&JsonValue::String(to_base58(account)))
    );
    assert_eq!(account_set.get("Fee"), Some(&JsonValue::Signed(12)));
    assert_eq!(account_set.get("SetFlag"), Some(&JsonValue::Unsigned(8)));

    let JsonValue::Object(trust_set) = &transactions[1] else {
        panic!("second transaction should be an object");
    };
    assert_eq!(
        trust_set.get("TransactionType"),
        Some(&JsonValue::String(
            protocol::TxType::TRUST_SET
                .format_name()
                .expect("trust set format")
                .to_owned()
        ))
    );
    assert_eq!(trust_set.get("Sequence"), Some(&JsonValue::Unsigned(8)));
    assert_eq!(
        trust_set.get("Account"),
        Some(&JsonValue::String(to_base58(account)))
    );
    assert_eq!(trust_set.get("Fee"), Some(&JsonValue::Signed(12)));
    assert_eq!(
        trust_set.get("Flags"),
        Some(&JsonValue::Unsigned(tfClearNoRipple as u64))
    );

    let JsonValue::Object(limit_amount) = trust_set.get("LimitAmount").expect("limit amount")
    else {
        panic!("limit amount should be an object");
    };
    assert_eq!(
        limit_amount.get("value"),
        Some(&JsonValue::String("500".to_owned()))
    );
    assert_eq!(
        limit_amount.get("currency"),
        Some(&JsonValue::String("USD".to_owned()))
    );
    assert_eq!(
        limit_amount.get("issuer"),
        Some(&JsonValue::String(to_base58(peer)))
    );
}

#[test]
fn no_ripple_check_reports_user_set_no_ripple() {
    let account = sample_account(0x44);
    let peer = sample_account(0x55);
    let line_key = sample_hash(0x66);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        fee_drops: 15,
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, false));
    source
        .owner_pages
        .insert((account, 0), make_owner_page(account, &[line_key]));
    source.children.insert(
        line_key,
        make_trust_line(account, peer, "EUR", 40, false, 0, 19, 20),
    );

    let response = do_no_ripple_check(
        &NoRippleCheckRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("role", JsonValue::String("user".to_owned())),
                ("transactions", JsonValue::Bool(true)),
            ]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );

    let JsonValue::Object(object) = response else {
        panic!("expected object response");
    };
    let JsonValue::Array(problems) = object.get("problems").expect("problems") else {
        panic!("problems should be an array");
    };
    assert_eq!(problems.len(), 1);
    assert_eq!(
        problems[0],
        JsonValue::String(format!(
            "You should probably set the no ripple flag on your EUR line to {}",
            to_base58(peer)
        ))
    );

    let JsonValue::Array(transactions) = object.get("transactions").expect("transactions") else {
        panic!("transactions should be an array");
    };
    assert_eq!(transactions.len(), 1);

    let JsonValue::Object(trust_set) = &transactions[0] else {
        panic!("transaction should be an object");
    };
    assert_eq!(
        trust_set.get("TransactionType"),
        Some(&JsonValue::String(
            protocol::TxType::TRUST_SET
                .format_name()
                .expect("trust set format")
                .to_owned()
        ))
    );
    assert_eq!(trust_set.get("Sequence"), Some(&JsonValue::Unsigned(7)));
    assert_eq!(trust_set.get("Fee"), Some(&JsonValue::Signed(15)));
    assert_eq!(
        trust_set.get("Flags"),
        Some(&JsonValue::Unsigned(tfSetNoRipple as u64))
    );
}
