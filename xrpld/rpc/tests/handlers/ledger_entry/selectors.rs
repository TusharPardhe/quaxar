//! Tests for common ledger entry selector and response shapes.

use super::*;

#[test]
fn ledger_entry_supports_common_selector_and_response_shapes() {
    let owner = account(0x11);
    let other = account(0x22);
    let issuer = account(0x33);
    let bridge_locking_door = account(0x44);
    let bridge_issuing_door = account(0x55);
    let bridge_locking_currency = currency(0x66);
    let bridge_issuing_currency = currency(0x77);
    let bridge_locking_currency_text = "66".repeat(20);
    let bridge_issuing_currency_text = "77".repeat(20);
    let ripple_currency_text = "99".repeat(20);
    let mpt_id = MPTID::from_hex("000102030405060708090A0B0C0D0E0F1011121314151617")
        .expect("mpt id should parse");
    let mpt_issuance_id = mpt_issuance_key(mpt_id);
    let mpt_holder = account(0x88);
    let bridge_fields = (
        bridge_locking_door,
        bridge_locking_currency,
        bridge_issuing_door,
        bridge_issuing_currency,
    );

    let account_key = account_keylet(account160(owner)).key;
    let state_key = line(owner, other, currency(0x99)).key;
    let directory_key = page_keylet(owner_dir_keylet(account160(owner)), 3).key;
    let offer_key = offer_keylet(account160(owner), 7).key;
    let amendments_key = amendments_key();
    let bridge_key = bridge_key(
        bridge_fields.0,
        bridge_fields.1,
        bridge_fields.2,
        bridge_fields.3,
        true,
    );
    let xchain_claim_key = xchain_claim_id_key(
        bridge_fields.0,
        Issue::new(bridge_fields.1, issuer),
        bridge_fields.2,
        Issue::new(bridge_fields.3, other),
        5,
    );
    let xchain_create_key = xchain_create_account_claim_id_key(
        bridge_fields.0,
        Issue::new(bridge_fields.1, issuer),
        bridge_fields.2,
        Issue::new(bridge_fields.3, other),
        6,
    );
    let deposit_key =
        deposit_preauth_credentials_key(owner, &[(issuer, &[0xbb]), (other, &[0xaa])]);
    let mptoken_key = mptoken_key(mpt_issuance_id, mpt_holder);

    let source = source_with(vec![
        STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, account_key),
        STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, state_key),
        STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, directory_key),
        STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, offer_key),
        STLedgerEntry::from_type_and_key(LedgerEntryType::Amendments, amendments_key),
        STLedgerEntry::from_type_and_key(LedgerEntryType::DepositPreauth, deposit_key),
    ]);

    let account_request = request(
        object([
            ("ledger_index", JsonValue::Unsigned(9)),
            ("account", JsonValue::String(to_base58(owner))),
            ("binary", JsonValue::Bool(true)),
        ]),
        2,
    );
    let account_result = do_ledger_entry(&account_request, &source);
    let JsonValue::Object(account_object) = account_result else {
        panic!("result must be an object");
    };
    assert_eq!(
        account_object.get("node_binary"),
        Some(&JsonValue::String(serialize_hex(
            &STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, account_key)
        )))
    );
    assert_result_index(&JsonValue::Object(account_object.clone()), account_key);

    let ripple_request = request(
        object([
            ("ledger_index", JsonValue::Unsigned(9)),
            (
                "ripple_state",
                JsonValue::Object(BTreeMap::from([
                    (
                        "currency".to_owned(),
                        JsonValue::String(ripple_currency_text.clone()),
                    ),
                    (
                        "accounts".to_owned(),
                        JsonValue::Array(vec![
                            JsonValue::String(to_base58(owner)),
                            JsonValue::String(to_base58(other)),
                        ]),
                    ),
                ])),
            ),
        ]),
        2,
    );
    let ripple_result = do_ledger_entry(&ripple_request, &source);
    let JsonValue::Object(ripple_object) = ripple_result else {
        panic!("result must be an object");
    };
    assert_eq!(
        ripple_object.get("node").and_then(|value| match value {
            JsonValue::Object(object) => object.get("index"),
            _ => None,
        }),
        Some(&JsonValue::String(state_key.to_string()))
    );

    let directory_request = request(
        object([
            ("ledger_index", JsonValue::Unsigned(9)),
            (
                "directory",
                JsonValue::Object(BTreeMap::from([
                    ("owner".to_owned(), JsonValue::String(to_base58(owner))),
                    ("sub_index".to_owned(), JsonValue::Unsigned(3)),
                ])),
            ),
        ]),
        2,
    );
    let directory_result = do_ledger_entry(&directory_request, &source);
    assert_result_index(&directory_result, directory_key);

    let offer_request = request(
        object([
            ("ledger_index", JsonValue::Unsigned(9)),
            (
                "offer",
                JsonValue::Object(BTreeMap::from([
                    ("account".to_owned(), JsonValue::String(to_base58(owner))),
                    ("seq".to_owned(), JsonValue::Unsigned(7)),
                ])),
            ),
        ]),
        2,
    );
    let offer_result = do_ledger_entry(&offer_request, &source);
    assert_result_index(&offer_result, offer_key);

    let amendments_request = request(
        object([
            ("ledger_index", JsonValue::Unsigned(9)),
            ("index", JsonValue::String("amendments".to_owned())),
        ]),
        3,
    );
    let amendments_result = do_ledger_entry(&amendments_request, &source);
    assert_result_index(&amendments_result, amendments_key);

    let deposit_request = request(
        object([
            ("ledger_index", JsonValue::Unsigned(9)),
            (
                "deposit_preauth",
                JsonValue::Object(BTreeMap::from([
                    ("owner".to_owned(), JsonValue::String(to_base58(owner))),
                    (
                        "authorized_credentials".to_owned(),
                        JsonValue::Array(vec![
                            JsonValue::Object(BTreeMap::from([
                                ("issuer".to_owned(), JsonValue::String(to_base58(issuer))),
                                (
                                    "credential_type".to_owned(),
                                    JsonValue::String("bb".to_owned()),
                                ),
                            ])),
                            JsonValue::Object(BTreeMap::from([
                                ("issuer".to_owned(), JsonValue::String(to_base58(other))),
                                (
                                    "credential_type".to_owned(),
                                    JsonValue::String("aa".to_owned()),
                                ),
                            ])),
                        ]),
                    ),
                ])),
            ),
        ]),
        2,
    );
    let deposit_result = do_ledger_entry(&deposit_request, &source);
    assert_result_index(&deposit_result, deposit_key);

    let bridge_request = request(
        object([
            ("ledger_index", JsonValue::Unsigned(9)),
            (
                "bridge_account",
                JsonValue::String(to_base58(bridge_locking_door)),
            ),
            (
                "bridge",
                JsonValue::Object(BTreeMap::from([
                    (
                        "LockingChainDoor".to_owned(),
                        JsonValue::String(to_base58(bridge_locking_door)),
                    ),
                    (
                        "LockingChainIssue".to_owned(),
                        JsonValue::Object(BTreeMap::from([
                            (
                                "currency".to_owned(),
                                JsonValue::String(bridge_locking_currency_text.clone()),
                            ),
                            ("issuer".to_owned(), JsonValue::String(to_base58(issuer))),
                        ])),
                    ),
                    (
                        "IssuingChainDoor".to_owned(),
                        JsonValue::String(to_base58(bridge_issuing_door)),
                    ),
                    (
                        "IssuingChainIssue".to_owned(),
                        JsonValue::Object(BTreeMap::from([
                            (
                                "currency".to_owned(),
                                JsonValue::String(bridge_issuing_currency_text.clone()),
                            ),
                            ("issuer".to_owned(), JsonValue::String(to_base58(other))),
                        ])),
                    ),
                ])),
            ),
        ]),
        2,
    );
    let bridge_result = do_ledger_entry(&bridge_request, &source);
    assert_result_index(&bridge_result, bridge_key);
    assert_error(&bridge_result, "entryNotFound");
    assert_error(&bridge_result, "entryNotFound");

    let xchain_request = request(
        object([
            ("ledger_index", JsonValue::Unsigned(9)),
            (
                "xchain_owned_claim_id",
                JsonValue::Object(BTreeMap::from([
                    (
                        "LockingChainDoor".to_owned(),
                        JsonValue::String(to_base58(bridge_locking_door)),
                    ),
                    (
                        "LockingChainIssue".to_owned(),
                        JsonValue::Object(BTreeMap::from([
                            (
                                "currency".to_owned(),
                                JsonValue::String(bridge_locking_currency_text.clone()),
                            ),
                            ("issuer".to_owned(), JsonValue::String(to_base58(issuer))),
                        ])),
                    ),
                    (
                        "IssuingChainDoor".to_owned(),
                        JsonValue::String(to_base58(bridge_issuing_door)),
                    ),
                    (
                        "IssuingChainIssue".to_owned(),
                        JsonValue::Object(BTreeMap::from([
                            (
                                "currency".to_owned(),
                                JsonValue::String(bridge_issuing_currency_text.clone()),
                            ),
                            ("issuer".to_owned(), JsonValue::String(to_base58(other))),
                        ])),
                    ),
                    ("xchain_owned_claim_id".to_owned(), JsonValue::Unsigned(5)),
                ])),
            ),
        ]),
        2,
    );
    let xchain_result = do_ledger_entry(&xchain_request, &source);
    assert_result_index(&xchain_result, xchain_claim_key);
    assert_error(&xchain_result, "entryNotFound");
    assert_error(&xchain_result, "entryNotFound");

    let xchain_create_request = request(
        object([
            ("ledger_index", JsonValue::Unsigned(9)),
            (
                "xchain_owned_create_account_claim_id",
                JsonValue::Object(BTreeMap::from([
                    (
                        "LockingChainDoor".to_owned(),
                        JsonValue::String(to_base58(bridge_locking_door)),
                    ),
                    (
                        "LockingChainIssue".to_owned(),
                        JsonValue::Object(BTreeMap::from([
                            (
                                "currency".to_owned(),
                                JsonValue::String(bridge_locking_currency_text.clone()),
                            ),
                            ("issuer".to_owned(), JsonValue::String(to_base58(issuer))),
                        ])),
                    ),
                    (
                        "IssuingChainDoor".to_owned(),
                        JsonValue::String(to_base58(bridge_issuing_door)),
                    ),
                    (
                        "IssuingChainIssue".to_owned(),
                        JsonValue::Object(BTreeMap::from([
                            (
                                "currency".to_owned(),
                                JsonValue::String(bridge_issuing_currency_text.clone()),
                            ),
                            ("issuer".to_owned(), JsonValue::String(to_base58(other))),
                        ])),
                    ),
                    (
                        "xchain_owned_create_account_claim_id".to_owned(),
                        JsonValue::Unsigned(6),
                    ),
                ])),
            ),
        ]),
        2,
    );
    let xchain_create_result = do_ledger_entry(&xchain_create_request, &source);
    assert_result_index(&xchain_create_result, xchain_create_key);
    assert_error(&xchain_create_result, "entryNotFound");
    assert_error(&xchain_create_result, "entryNotFound");

    let mpt_issuance_request = request(
        object([
            ("ledger_index", JsonValue::Unsigned(9)),
            (
                "mpt_issuance",
                JsonValue::String("000102030405060708090A0B0C0D0E0F1011121314151617".to_owned()),
            ),
        ]),
        2,
    );
    let mpt_issuance_result = do_ledger_entry(&mpt_issuance_request, &source);
    assert_result_index(&mpt_issuance_result, mpt_issuance_id);
    assert_error(&mpt_issuance_result, "entryNotFound");

    let mptoken_request = request(
        object([
            ("ledger_index", JsonValue::Unsigned(9)),
            (
                "mptoken",
                JsonValue::Object(BTreeMap::from([
                    (
                        "mpt_issuance_id".to_owned(),
                        JsonValue::String(
                            "000102030405060708090A0B0C0D0E0F1011121314151617".to_owned(),
                        ),
                    ),
                    (
                        "account".to_owned(),
                        JsonValue::String(to_base58(mpt_holder)),
                    ),
                ])),
            ),
        ]),
        2,
    );
    let mptoken_result = do_ledger_entry(&mptoken_request, &source);
    assert_result_index(&mptoken_result, mptoken_key);
    assert_error(&mptoken_result, "entryNotFound");
}
