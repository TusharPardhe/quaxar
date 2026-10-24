//! Tests for common ledger entry selector and response shapes.

use super::*;

#[test]
fn ledger_entry_supports_common_selector_and_response_shapes() {
    let owner = account(0x11);
    let other = account(0x22);
    let issuer = account(0x33);
    let ripple_currency_text = "99".repeat(20);
    let mpt_id = MPTID::from_hex("000102030405060708090A0B0C0D0E0F1011121314151617")
        .expect("mpt id should parse");
    let mpt_issuance_id = mpt_issuance_key(mpt_id);
    let mpt_holder = account(0x88);

    let account_key = account_keylet(account160(owner)).key;
    let state_key = line(owner, other, currency(0x99)).key;
    let directory_key = page_keylet(owner_dir_keylet(account160(owner)), 3).key;
    let offer_key = offer_keylet(account160(owner), 7).key;
    let amendments_key = amendments_key();
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

    // rippled #8255: the XChain selectors are gone, so a request naming only
    // `bridge` has no ledger_entry params.
    for (api_version, error) in [(1, "unknownOption"), (2, "invalidParams")] {
        let removed = request(
            object([
                ("ledger_index", JsonValue::Unsigned(9)),
                ("bridge", JsonValue::Object(BTreeMap::new())),
            ]),
            api_version,
        );
        assert_error(&do_ledger_entry(&removed, &source), error);
    }

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
