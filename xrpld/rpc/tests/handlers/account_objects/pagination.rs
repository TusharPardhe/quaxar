//! account objects tests part 2.

use super::*;

#[test]
fn account_objects_returns_multiple_objects_unstepped() {
    let account = sample_account(0x33);
    let offer_key1 = sample_hash(0x61);
    let offer_key2 = sample_hash(0x62);
    let check_key = sample_hash(0x63);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .entries
        .insert(account_root_key(account), make_account_root(account));
    source.entries.insert(
        owner_root_key(account),
        make_owner_dir_page(account, &[offer_key1, offer_key2, check_key], None),
    );
    source
        .entries
        .insert(child_keylet(offer_key1), make_offer_entry(offer_key1));
    source
        .entries
        .insert(child_keylet(offer_key2), make_offer_entry(offer_key2));
    source
        .entries
        .insert(child_keylet(check_key), make_check_entry(check_key));

    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(response) = response else {
        panic!("response must be an object");
    };
    assert_eq!(
        response.get("account"),
        Some(&JsonValue::String(to_base58(account)))
    );
    assert!(!response.contains_key("marker"));
    assert_eq!(
        response.get("ledger_hash"),
        Some(&JsonValue::String(closed_ledger().hash.to_string()))
    );
    assert_eq!(
        response.get("ledger_index"),
        Some(&JsonValue::Unsigned(u64::from(closed_ledger().seq)))
    );
    assert_eq!(response.get("validated"), Some(&JsonValue::Bool(true)));

    let JsonValue::Array(items) = response.get("account_objects").expect("account_objects") else {
        panic!("account_objects must be an array");
    };
    assert_eq!(items.len(), 3);
}

#[test]
fn account_objects_stepped_pagination_with_limit() {
    let account = sample_account(0x34);
    let key1 = sample_hash(0x71);
    let key2 = sample_hash(0x72);
    let key3 = sample_hash(0x73);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .entries
        .insert(account_root_key(account), make_account_root(account));
    source.entries.insert(
        owner_root_key(account),
        make_owner_dir_page(account, &[key1, key2, key3], None),
    );
    source
        .entries
        .insert(child_keylet(key1), make_offer_entry(key1));
    source
        .entries
        .insert(child_keylet(key2), make_offer_entry(key2));
    source
        .entries
        .insert(child_keylet(key3), make_check_entry(key3));

    // First page: limit=1
    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("limit", JsonValue::Unsigned(1)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(response) = response else {
        panic!("response must be an object");
    };
    let JsonValue::Array(items) = response.get("account_objects").expect("account_objects") else {
        panic!("account_objects must be an array");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(response.get("limit"), Some(&JsonValue::Unsigned(1)));
    assert!(response.contains_key("marker"), "should have marker");

    // Second page: use marker
    let marker = response.get("marker").unwrap().clone();
    let response2 = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("limit", JsonValue::Unsigned(1)),
                ("marker", marker),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(response2) = response2 else {
        panic!("response2 must be an object");
    };
    let JsonValue::Array(items2) = response2.get("account_objects").expect("account_objects")
    else {
        panic!("account_objects must be an array");
    };
    assert_eq!(items2.len(), 1);
    assert!(
        response2.contains_key("marker"),
        "should have marker for page 3"
    );

    // Third page: last item
    let marker2 = response2.get("marker").unwrap().clone();
    let response3 = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("marker", marker2),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(response3) = response3 else {
        panic!("response3 must be an object");
    };
    let JsonValue::Array(items3) = response3.get("account_objects").expect("account_objects")
    else {
        panic!("account_objects must be an array");
    };
    assert_eq!(items3.len(), 1);
    assert!(!response3.contains_key("marker"), "no marker on last page");
}

#[test]
fn account_objects_marker_transitions_from_last_nft_page_to_owner_directory() {
    let account = sample_account(0x3A);
    let issuer = sample_account(0x3B);
    let token = make_nft_id(0, 0, issuer, 1, 1);
    let check_key = sample_hash(0x8A);
    let nft_start = nft_page_keylet(
        nft_page_min_keylet(Uint160::from_slice(account.data()).expect("account width")),
        Uint256::zero(),
    )
    .key;
    let nft_page_key = nft_start.next();

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .entries
        .insert(account_root_key(account), make_account_root(account));
    source.entries.insert(
        Keylet::new(LedgerEntryType::NFTokenPage, nft_page_key),
        make_nft_page(nft_page_key, &[token], None),
    );
    source.entries.insert(
        owner_root_key(account),
        make_owner_dir_page(account, &[check_key], None),
    );
    source
        .entries
        .insert(child_keylet(check_key), make_check_entry(check_key));

    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("limit", JsonValue::Unsigned(1)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(response) = response else {
        panic!("response must be an object");
    };
    let JsonValue::Array(items) = response.get("account_objects").expect("account_objects") else {
        panic!("account_objects must be an array");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(
        response.get("marker"),
        Some(&JsonValue::String(format!(
            "{},{}",
            owner_root_key(account).key,
            check_key
        )))
    );
}

#[test]
fn account_objects_nft_page_filter_does_not_marker_into_owner_directory() {
    let account = sample_account(0x3C);
    let issuer = sample_account(0x3D);
    let token = make_nft_id(0, 0, issuer, 1, 1);
    let check_key = sample_hash(0x8C);
    let nft_start = nft_page_keylet(
        nft_page_min_keylet(Uint160::from_slice(account.data()).expect("account width")),
        Uint256::zero(),
    )
    .key;
    let nft_page_key = nft_start.next();

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .entries
        .insert(account_root_key(account), make_account_root(account));
    source.entries.insert(
        Keylet::new(LedgerEntryType::NFTokenPage, nft_page_key),
        make_nft_page(nft_page_key, &[token], None),
    );
    source.entries.insert(
        owner_root_key(account),
        make_owner_dir_page(account, &[check_key], None),
    );
    source
        .entries
        .insert(child_keylet(check_key), make_check_entry(check_key));

    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("type", JsonValue::String("nft_page".to_owned())),
                ("limit", JsonValue::Unsigned(1)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(response) = response else {
        panic!("response must be an object");
    };
    let JsonValue::Array(items) = response.get("account_objects").expect("account_objects") else {
        panic!("account_objects must be an array");
    };
    assert_eq!(items.len(), 1);
    assert!(
        !response.contains_key("marker"),
        "nft_page-only scan must not continue into owner directory"
    );
}

#[test]
fn account_objects_type_filter_check_only() {
    let account = sample_account(0x35);
    let offer_key = sample_hash(0x81);
    let check_key = sample_hash(0x82);

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
                ("type", JsonValue::String("check".to_owned())),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(response) = response else {
        panic!("response must be an object");
    };
    let JsonValue::Array(items) = response.get("account_objects").expect("account_objects") else {
        panic!("account_objects must be an array");
    };
    assert_eq!(items.len(), 1);
}

#[test]
fn account_objects_empty_owner_dir() {
    let account = sample_account(0x36);
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

    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(response) = response else {
        panic!("response must be an object");
    };
    let JsonValue::Array(items) = response.get("account_objects").expect("account_objects") else {
        panic!("account_objects must be an array");
    };
    assert_eq!(items.len(), 0);
    assert!(!response.contains_key("marker"));
    assert_eq!(
        response.get("account"),
        Some(&JsonValue::String(to_base58(account)))
    );
}

#[test]
fn account_objects_invalid_type_strings_rejected() {
    let account = sample_account(0x37);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .entries
        .insert(account_root_key(account), make_account_root(account));

    for invalid_type in ["amendments", "directory", "fee", "hashes"] {
        let response = do_account_objects(
            &AccountObjectsRequest {
                params: &object([
                    ("account", JsonValue::String(to_base58(account))),
                    ("type", JsonValue::String(invalid_type.to_owned())),
                ]),
                api_version: 1,
                role: Role::Admin,
            },
            &source,
        );
        let (error, _code, message) = error_fields(&response);
        assert_eq!(
            error, "invalidParams",
            "type '{invalid_type}' should be rejected"
        );
        assert_eq!(message, "Invalid field 'type'.");
    }
}

#[test]
fn account_objects_valid_type_strings_accepted() {
    let account = sample_account(0x38);
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

    for valid_type in ["offer", "check", "state", "ticket", "nft_page"] {
        let response = do_account_objects(
            &AccountObjectsRequest {
                params: &object([
                    ("account", JsonValue::String(to_base58(account))),
                    ("type", JsonValue::String(valid_type.to_owned())),
                ]),
                api_version: 1,
                role: Role::Admin,
            },
            &source,
        );
        let JsonValue::Object(response) = response else {
            panic!("response must be an object for type '{valid_type}'");
        };
        assert_eq!(
            response.get("error"),
            None,
            "type '{valid_type}' should be accepted"
        );
        assert!(response.contains_key("account_objects"));
    }
}

#[test]
fn account_objects_deletion_blockers_only_with_type_filter() {
    let account = sample_account(0x39);
    let offer_key = sample_hash(0x91);
    let check_key = sample_hash(0x92);

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

    // deletion_blockers_only with type=offer should return offers that are deletion blockers
    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("deletion_blockers_only", JsonValue::Bool(true)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(response) = response else {
        panic!("response must be an object");
    };
    assert_eq!(response.get("error"), None);
    let JsonValue::Array(items) = response.get("account_objects").expect("account_objects") else {
        panic!("account_objects must be an array");
    };
    // Check is a deletion blocker, offer is not
    assert_eq!(items.len(), 1);
}

#[test]
fn account_objects_response_includes_ledger_metadata() {
    let account = sample_account(0x3A);
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

    let response = do_account_objects(
        &AccountObjectsRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(response) = response else {
        panic!("response must be an object");
    };
    assert_eq!(
        response.get("account"),
        Some(&JsonValue::String(to_base58(account)))
    );
    assert_eq!(
        response.get("ledger_hash"),
        Some(&JsonValue::String(closed_ledger().hash.to_string()))
    );
    assert_eq!(
        response.get("ledger_index"),
        Some(&JsonValue::Unsigned(u64::from(closed_ledger().seq)))
    );
    assert_eq!(response.get("validated"), Some(&JsonValue::Bool(true)));
}
