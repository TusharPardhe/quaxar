//! account objects tests part 1.

use super::*;

#[test]
fn account_objects_requires_a_real_account_root() {
    let account = sample_account(0x11);
    let offer_key = sample_hash(0x21);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source.entries.insert(
        owner_root_key(account),
        make_owner_dir_page(account, &[offer_key], None),
    );
    source
        .entries
        .insert(child_keylet(offer_key), make_offer_entry(offer_key));

    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    assert_eq!(
        error_fields(&response),
        ("actNotFound", 19, "Account not found.")
    );
}

#[test]
fn account_objects_honors_type_filter_without_deletion_blockers_flag() {
    let account = sample_account(0x22);
    let offer_key = sample_hash(0x31);
    let check_key = sample_hash(0x32);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .entries
        .insert(account_root_key(account), make_account_root(account));
    source.entries.insert(
        owner_root_key(account),
        make_owner_dir_page(account, &[offer_key, check_key], None),
    );
    source
        .entries
        .insert(child_keylet(offer_key), make_offer_entry(offer_key));
    source
        .entries
        .insert(child_keylet(check_key), make_check_entry(check_key));

    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("type", JsonValue::String("offer".to_owned())),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(response) = response else {
        panic!("account_objects response must be an object");
    };
    let JsonValue::Array(items) = response.get("account_objects").expect("account_objects") else {
        panic!("account_objects must be an array");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(
        response.get("account"),
        Some(&JsonValue::String(to_base58(account)))
    );
    assert!(!response.contains_key("marker"));
}

#[test]
fn account_objects_honors_deletion_blockers_only_filtering() {
    let account = sample_account(0x23);
    let offer_key = sample_hash(0x41);
    let check_key = sample_hash(0x42);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .entries
        .insert(account_root_key(account), make_account_root(account));
    source.entries.insert(
        owner_root_key(account),
        make_owner_dir_page(account, &[offer_key, check_key], None),
    );
    source
        .entries
        .insert(child_keylet(offer_key), make_offer_entry(offer_key));
    source
        .entries
        .insert(child_keylet(check_key), make_check_entry(check_key));

    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("deletion_blockers_only", JsonValue::Bool(true)),
                ("type", JsonValue::String("check".to_owned())),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(response) = response else {
        panic!("account_objects response must be an object");
    };
    let JsonValue::Array(items) = response.get("account_objects").expect("account_objects") else {
        panic!("account_objects must be an array");
    };
    assert_eq!(items.len(), 1);
}

#[test]
fn account_objects_reports_traversal_boundaries_as_db_deserialization() {
    let account = sample_account(0x24);
    let offer_key = sample_hash(0x51);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        fail_on_read: Some(child_keylet(offer_key)),
        ..Default::default()
    };
    source
        .entries
        .insert(account_root_key(account), make_account_root(account));
    source.entries.insert(
        owner_root_key(account),
        make_owner_dir_page(account, &[offer_key], None),
    );
    source
        .entries
        .insert(child_keylet(offer_key), make_offer_entry(offer_key));

    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    assert_eq!(
        error_fields(&response),
        ("dbDeserialization", 77, "Database deserialization error.")
    );
}

#[test]
fn account_nft_support_decodes_taxon_and_marker() {
    let account = sample_account(0x31);
    let issuer = sample_account(0x32);
    let first_token = make_nft_id(0x0010, 0x0000, issuer, 0x01020304, 1);
    let second_token = make_nft_id(0x0011, 0x0032, issuer, 0x11121314, 2);
    let start_key = nft_page_keylet(
        nft_page_min_keylet(Uint160::from_slice(account.data()).expect("account width")),
        Uint256::zero(),
    )
    .key;
    let first_page_key = start_key.next();
    let second_page_key = first_page_key.next();

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source.entries.insert(
        Keylet::new(LedgerEntryType::NFTokenPage, first_page_key),
        make_nft_page(first_page_key, &[first_token], Some(second_page_key)),
    );
    source.entries.insert(
        Keylet::new(LedgerEntryType::NFTokenPage, second_page_key),
        make_nft_page(second_page_key, &[second_token], None),
    );

    let traversal = collect_account_nfts(&source, account, Uint256::zero(), 1)
        .expect("nft traversal should succeed");
    assert_eq!(traversal.marker, Some(second_page_key));
    assert_eq!(traversal.items.len(), 1);

    let JsonValue::Object(token) = &traversal.items[0] else {
        panic!("token must be an object");
    };
    assert_eq!(
        token.get("NFTokenTaxon"),
        Some(&JsonValue::Unsigned(0x01020304))
    );
    assert_eq!(token.get("nft_serial"), Some(&JsonValue::Unsigned(1)));
    assert_eq!(token.get("Flags"), Some(&JsonValue::Unsigned(0x0010)));
    assert_eq!(
        token.get("Issuer"),
        Some(&JsonValue::String(to_base58(issuer)))
    );
}

#[test]
fn account_objects_missing_account_field() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &JsonValue::Object(Default::default()),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    assert_eq!(
        error_fields(&response),
        ("invalidParams", 31, "Missing field 'account'.")
    );
}

#[test]
fn account_objects_invalid_account_types() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    // Integer
    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([("account", JsonValue::Unsigned(1))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    assert_eq!(
        error_fields(&response),
        ("invalidParams", 31, "Invalid field 'account'.")
    );

    // Boolean
    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([("account", JsonValue::Bool(true))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    assert_eq!(
        error_fields(&response),
        ("invalidParams", 31, "Invalid field 'account'.")
    );

    // Null
    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([("account", JsonValue::Null)]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    assert_eq!(
        error_fields(&response),
        ("invalidParams", 31, "Invalid field 'account'.")
    );

    // Array
    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([("account", JsonValue::Array(vec![]))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    assert_eq!(
        error_fields(&response),
        ("invalidParams", 31, "Invalid field 'account'.")
    );

    // Object
    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([("account", JsonValue::Object(Default::default()))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    assert_eq!(
        error_fields(&response),
        ("invalidParams", 31, "Invalid field 'account'.")
    );
}

#[test]
fn account_objects_malformed_account_string() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([(
                "account",
                JsonValue::String(
                    "n94JNrQYkDrpt62bbSR7nVEhdyAvcJXRAsjEkFYyqRkh9SUTYEqV".to_owned(),
                ),
            )]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let (error, _code, message) = error_fields(&response);
    assert_eq!(error, "actMalformed");
    assert_eq!(message, "Account malformed.");
}

#[test]
fn account_objects_invalid_type_param() {
    let account = sample_account(0x30);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .entries
        .insert(account_root_key(account), make_account_root(account));

    // Non-string type
    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("type", JsonValue::Unsigned(10)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let (error, _code, message) = error_fields(&response);
    assert_eq!(error, "invalidParams");
    assert_eq!(message, "Invalid field 'type', not string.");

    // Invalid type string
    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("type", JsonValue::String("expedited".to_owned())),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let (error, _code, message) = error_fields(&response);
    assert_eq!(error, "invalidParams");
    assert_eq!(message, "Invalid field 'type'.");
}

#[test]
fn account_objects_invalid_limit() {
    let account = sample_account(0x31);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .entries
        .insert(account_root_key(account), make_account_root(account));

    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("limit", JsonValue::Signed(-1)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let (error, _code, message) = error_fields(&response);
    assert_eq!(error, "invalidParams");
    assert!(
        message.contains("limit"),
        "error message should mention limit: {message}"
    );
}

#[test]
fn account_objects_invalid_marker_types() {
    let account = sample_account(0x32);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .entries
        .insert(account_root_key(account), make_account_root(account));
    source.entries.insert(
        owner_root_key(account),
        make_owner_dir_page(account, &[], None),
    );

    // Non-string marker
    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("marker", JsonValue::Unsigned(10)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let (error, _code, message) = error_fields(&response);
    assert_eq!(error, "invalidParams");
    assert_eq!(message, "Invalid field 'marker', not string.");

    // Marker without comma
    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                (
                    "marker",
                    JsonValue::String("This is a string with no comma".to_owned()),
                ),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let (error, _code, message) = error_fields(&response);
    assert_eq!(error, "invalidParams");
    assert_eq!(message, "Invalid field 'marker'.");

    // Marker with comma but not hex
    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                (
                    "marker",
                    JsonValue::String("This string has a comma, but is not hex".to_owned()),
                ),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let (error, _code, message) = error_fields(&response);
    assert_eq!(error, "invalidParams");
    assert_eq!(message, "Invalid field 'marker'.");
}
