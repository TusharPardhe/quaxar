//! tx tests part 1.

use super::*;

#[test]
fn tx_requires_enabled_tables_and_valid_selectors() {
    let source = FakeTxSource {
        enabled: false,
        synced: true,
        network_id: 0,
        by_hash: BTreeMap::new(),
        by_ctid: BTreeMap::new(),
    };
    assert_eq!(
        do_tx(
            &TxRequest {
                params: &object([(
                    "transaction",
                    JsonValue::String(Uint256::from_array([0x11; 32]).to_string()),
                )]),
                api_version: 1,
            },
            &source
        ),
        JsonValue::Object(BTreeMap::from([
            (
                "error".to_owned(),
                JsonValue::String("notEnabled".to_owned())
            ),
            ("error_code".to_owned(), JsonValue::Signed(12)),
            (
                "error_message".to_owned(),
                JsonValue::String("Not enabled in configuration.".to_owned())
            ),
        ]))
    );

    let source = FakeTxSource {
        enabled: true,
        synced: true,
        network_id: 0,
        by_hash: BTreeMap::new(),
        by_ctid: BTreeMap::new(),
    };
    assert_eq!(
        do_tx(
            &TxRequest {
                params: &object([
                    (
                        "transaction",
                        JsonValue::String(Uint256::from_array([0x11; 32]).to_string()),
                    ),
                    ("ctid", JsonValue::String("C000000300030000".to_owned())),
                ]),
                api_version: 1,
            },
            &source
        ),
        JsonValue::Object(BTreeMap::from([
            (
                "error".to_owned(),
                JsonValue::String("invalidParams".to_owned())
            ),
            ("error_code".to_owned(), JsonValue::Signed(31)),
            (
                "error_message".to_owned(),
                JsonValue::String("Invalid parameters.".to_owned())
            ),
        ]))
    );

    assert_eq!(
        do_tx(
            &TxRequest {
                params: &object([("transaction", JsonValue::String("DEADBEEF".to_owned()))]),
                api_version: 1,
            },
            &source
        ),
        JsonValue::Object(BTreeMap::from([
            ("error".to_owned(), JsonValue::String("notImpl".to_owned())),
            ("error_code".to_owned(), JsonValue::Signed(74)),
            (
                "error_message".to_owned(),
                JsonValue::String("Not implemented.".to_owned())
            ),
        ]))
    );
}

#[test]
fn tx_rejects_wrong_network_and_bad_ranges() {
    let source = FakeTxSource {
        enabled: true,
        synced: true,
        network_id: 21337,
        by_hash: BTreeMap::new(),
        by_ctid: BTreeMap::new(),
    };

    assert_eq!(
        do_tx(
            &TxRequest {
                params: &object([("ctid", JsonValue::String("C00000030003535A".to_owned()))]),
                api_version: 2,
            },
            &source
        ),
        JsonValue::Object(BTreeMap::from([
            (
                "error".to_owned(),
                JsonValue::String("wrongNetwork".to_owned())
            ),
            ("error_code".to_owned(), JsonValue::Signed(4)),
            (
                "error_message".to_owned(),
                JsonValue::String(
                    "Wrong network. You should submit this request to a node running on NetworkID: 21338"
                        .to_owned()
                )
            ),
        ]))
    );

    let hash = Uint256::from_array([0x55; 32]).to_string();
    let invalid_range = do_tx(
        &TxRequest {
            params: &object([
                ("transaction", JsonValue::String(hash.clone())),
                ("min_ledger", JsonValue::Unsigned(5)),
                ("max_ledger", JsonValue::Unsigned(4)),
            ]),
            api_version: 1,
        },
        &source,
    );
    assert_eq!(
        invalid_range,
        JsonValue::Object(BTreeMap::from([
            (
                "error".to_owned(),
                JsonValue::String("invalidLgrRange".to_owned())
            ),
            ("error_code".to_owned(), JsonValue::Signed(79)),
            (
                "error_message".to_owned(),
                JsonValue::String("Ledger range is invalid.".to_owned())
            ),
        ]))
    );

    let excessive_range = do_tx(
        &TxRequest {
            params: &object([
                ("transaction", JsonValue::String(hash)),
                ("min_ledger", JsonValue::Unsigned(1)),
                ("max_ledger", JsonValue::Unsigned(1002)),
            ]),
            api_version: 1,
        },
        &source,
    );
    assert_eq!(
        excessive_range,
        JsonValue::Object(BTreeMap::from([
            (
                "error".to_owned(),
                JsonValue::String("excessiveLgrRange".to_owned())
            ),
            ("error_code".to_owned(), JsonValue::Signed(78)),
            (
                "error_message".to_owned(),
                JsonValue::String("Ledger range exceeds 1000.".to_owned())
            ),
        ]))
    );
}

#[test]
fn tx_injects_searched_all_on_not_found() {
    let source = FakeTxSource {
        enabled: true,
        synced: true,
        network_id: 0,
        by_hash: BTreeMap::from([
            (
                Uint256::from_array([0x11; 32]).to_string(),
                Ok(TxLookupOutcome::NotFound(TxSearched::All)),
            ),
            (
                Uint256::from_array([0x22; 32]).to_string(),
                Ok(TxLookupOutcome::NotFound(TxSearched::Unknown)),
            ),
        ]),
        by_ctid: BTreeMap::new(),
    };

    let searched_all = do_tx(
        &TxRequest {
            params: &object([(
                "transaction",
                JsonValue::String(Uint256::from_array([0x11; 32]).to_string()),
            )]),
            api_version: 1,
        },
        &source,
    );
    let JsonValue::Object(searched_all) = searched_all else {
        panic!("result must be an object");
    };
    assert_eq!(
        searched_all.get("searched_all"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(
        searched_all.get("error"),
        Some(&JsonValue::String("txnNotFound".to_owned()))
    );

    let unknown = do_tx(
        &TxRequest {
            params: &object([(
                "transaction",
                JsonValue::String(Uint256::from_array([0x22; 32]).to_string()),
            )]),
            api_version: 1,
        },
        &source,
    );
    let JsonValue::Object(unknown) = unknown else {
        panic!("result must be an object");
    };
    assert!(!unknown.contains_key("searched_all"));
}

#[test]
fn tx_renders_json_and_binary_shapes() {
    let tx = Arc::new(payment_tx());
    let tx_id = tx.get_transaction_id();
    let meta = payment_meta(tx_id);
    let record = found_record(Arc::clone(&tx), Some(meta.clone()));
    let source = FakeTxSource {
        enabled: true,
        synced: true,
        network_id: 0,
        by_hash: BTreeMap::from([(
            tx_id.to_string(),
            Ok(TxLookupOutcome::Found(record.clone())),
        )]),
        by_ctid: BTreeMap::from([((3, 3), Ok(TxLookupOutcome::Found(record)))]),
    };

    let v1 = do_tx(
        &TxRequest {
            params: &object([("transaction", JsonValue::String(tx_id.to_string()))]),
            api_version: 1,
        },
        &source,
    );
    let JsonValue::Object(v1) = v1 else {
        panic!("result must be an object");
    };
    assert_eq!(v1.get("hash"), Some(&JsonValue::String(tx_id.to_string())));
    assert_eq!(v1.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        v1.get("ctid"),
        Some(&JsonValue::String("C000000300030000".to_owned()))
    );
    assert_eq!(v1.get("date"), Some(&JsonValue::Signed(10)));
    assert!(v1.contains_key("meta"));
    let JsonValue::Object(v1_meta) = v1.get("meta").expect("meta must exist") else {
        panic!("meta must be an object");
    };
    assert_eq!(
        v1_meta.get("delivered_amount"),
        Some(&JsonValue::String("1000000".to_owned()))
    );
    assert_eq!(v1.get("DeliverMax"), v1.get("Amount"));

    let v2 = do_tx(
        &TxRequest {
            params: &object([("transaction", JsonValue::String(tx_id.to_string()))]),
            api_version: 2,
        },
        &source,
    );
    let JsonValue::Object(v2) = v2 else {
        panic!("result must be an object");
    };
    let JsonValue::Object(v2_tx) = v2.get("tx_json").expect("tx_json must exist") else {
        panic!("tx_json must be an object");
    };
    assert_eq!(v2.get("hash"), Some(&JsonValue::String(tx_id.to_string())));
    assert_eq!(v2.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(v2.get("ledger_index"), Some(&JsonValue::Unsigned(3)));
    assert_eq!(
        v2.get("close_time_iso"),
        Some(&JsonValue::String("2000-01-01T00:00:10Z".to_owned()))
    );
    assert_eq!(
        v2.get("ledger_hash"),
        Some(&JsonValue::String(
            Uint256::from_array([0x33; 32]).to_string()
        ))
    );
    assert_eq!(
        v2.get("ctid"),
        Some(&JsonValue::String("C000000300030000".to_owned()))
    );
    assert!(v2.contains_key("meta"));
    assert_eq!(v2_tx.get("ledger_index"), Some(&JsonValue::Unsigned(3)));
    assert_eq!(v2_tx.get("date"), Some(&JsonValue::Signed(10)));
    assert!(!v2_tx.contains_key("Amount"));
    assert!(v2_tx.contains_key("DeliverMax"));

    let binary = do_tx(
        &TxRequest {
            params: &object([
                ("ctid", JsonValue::String("C000000300030000".to_owned())),
                ("binary", JsonValue::Bool(true)),
            ]),
            api_version: 2,
        },
        &source,
    );
    let JsonValue::Object(binary) = binary else {
        panic!("result must be an object");
    };
    assert_eq!(
        binary.get("tx_blob"),
        Some(&JsonValue::String(basics::str_hex::str_hex(
            tx.get_serializer().data()
        )))
    );
    assert_eq!(
        binary.get("meta_blob"),
        Some(&JsonValue::String(basics::str_hex::str_hex(
            meta.get_as_object().get_serializer().data()
        )))
    );
}

#[test]
fn fix_mpt_delivered_amount_tx_rpc_preserves_canonical_json_and_binary_meta() {
    let mpt_issue = MPTIssue::new(protocol::make_mpt_id(1, account(3)));
    let tx = Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(1));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(2));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::from_mpt_amount(
                get_field_by_symbol("sfAmount"),
                MPTAmount::from_value(1_000),
                mpt_issue,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 5);
    }));
    let tx_id = tx.get_transaction_id();
    let delivered = STAmount::from_mpt_amount(
        get_field_by_symbol("sfDeliveredAmount"),
        MPTAmount::from_value(800),
        mpt_issue,
    );
    let mut meta = payment_meta(tx_id);
    meta.set_delivered_amount(Some(delivered));
    let source = FakeTxSource {
        enabled: true,
        synced: true,
        network_id: 0,
        by_hash: BTreeMap::from([(
            tx_id.to_string(),
            Ok(TxLookupOutcome::Found(found_record(
                Arc::clone(&tx),
                Some(meta.clone()),
            ))),
        )]),
        by_ctid: BTreeMap::new(),
    };

    let json = do_tx(
        &TxRequest {
            params: &object([("transaction", JsonValue::String(tx_id.to_string()))]),
            api_version: 2,
        },
        &source,
    );
    let JsonValue::Object(json) = json else {
        panic!("tx response should be an object");
    };
    let JsonValue::Object(json_meta) = json.get("meta").expect("meta should be present") else {
        panic!("metadata should be an object");
    };
    assert_eq!(
        json_meta.get("DeliveredAmount"),
        json_meta.get("delivered_amount")
    );

    let binary = do_tx(
        &TxRequest {
            params: &object([
                ("transaction", JsonValue::String(tx_id.to_string())),
                ("binary", JsonValue::Bool(true)),
            ]),
            api_version: 2,
        },
        &source,
    );
    let JsonValue::Object(binary) = binary else {
        panic!("binary tx response should be an object");
    };
    assert_eq!(
        binary.get("meta_blob"),
        Some(&JsonValue::String(basics::str_hex::str_hex(
            meta.get_as_object().get_serializer().data()
        )))
    );
}

#[test]
fn tx_ctid_only_uses_explicit_lookup_network_id() {
    let tx = Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(1));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(2));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 5);
        tx.set_field_u32(get_field_by_symbol("sfNetworkID"), 9);
    }));
    let tx_id = tx.get_transaction_id();
    let record = found_record_with_network(Arc::clone(&tx), Some(payment_meta(tx_id)), None);
    let source = FakeTxSource {
        enabled: true,
        synced: true,
        network_id: 0,
        by_hash: BTreeMap::from([(tx_id.to_string(), Ok(TxLookupOutcome::Found(record)))]),
        by_ctid: BTreeMap::new(),
    };

    let v1 = do_tx(
        &TxRequest {
            params: &object([("transaction", JsonValue::String(tx_id.to_string()))]),
            api_version: 1,
        },
        &source,
    );
    let JsonValue::Object(v1) = v1 else {
        panic!("result must be an object");
    };
    assert_eq!(v1.get("ctid"), None);

    let v2 = do_tx(
        &TxRequest {
            params: &object([("transaction", JsonValue::String(tx_id.to_string()))]),
            api_version: 2,
        },
        &source,
    );
    let JsonValue::Object(v2) = v2 else {
        panic!("result must be an object");
    };
    assert_eq!(v2.get("ctid"), None);
}
