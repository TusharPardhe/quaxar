//! server definitions tests part A.

use super::*;

#[test]
fn server_definitions_matches_current_cpp_catalog_hash_and_shape() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);

    let hash = match root.get("hash") {
        Some(JsonValue::String(hash)) => hash,
        other => panic!("expected hash string, got {other:?}"),
    };
    assert_eq!(hash.len(), 64);
    assert!(
        hash.chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_lowercase())
    );
    assert_eq!(
        get_array(root.get("FIELDS").expect("FIELDS"))
            .first()
            .and_then(|field| match field {
                JsonValue::Array(items) => items.first(),
                _ => None,
            })
            .map(get_str),
        Some("Invalid")
    );
    assert_eq!(
        get_object(root.get("LEDGER_ENTRY_TYPES").expect("LEDGER_ENTRY_TYPES")).get("AccountRoot"),
        Some(&JsonValue::Signed(97))
    );
    assert_eq!(
        get_object(
            root.get("TRANSACTION_RESULTS")
                .expect("TRANSACTION_RESULTS")
        )
        .get("tecDIR_FULL"),
        Some(&JsonValue::Signed(121))
    );
    assert_eq!(
        get_object(root.get("TRANSACTION_TYPES").expect("TRANSACTION_TYPES")).get("Payment"),
        Some(&JsonValue::Signed(0))
    );
    assert_eq!(
        get_object(root.get("TYPES").expect("TYPES")).get("Hash256"),
        Some(&JsonValue::Signed(5))
    );
    assert_eq!(
        get_array(
            get_object(
                root.get("TRANSACTION_FORMATS")
                    .expect("TRANSACTION_FORMATS")
            )
            .get("common")
            .expect("common"),
        )
        .first()
        .and_then(|entry| match entry {
            JsonValue::Object(object) => object.get("name"),
            _ => None,
        }),
        Some(&JsonValue::String("TransactionType".to_owned()))
    );
    assert_eq!(
        get_array(
            get_object(
                root.get("LEDGER_ENTRY_FORMATS")
                    .expect("LEDGER_ENTRY_FORMATS")
            )
            .get("common")
            .expect("common"),
        )
        .first()
        .and_then(|entry| match entry {
            JsonValue::Object(object) => object.get("name"),
            _ => None,
        }),
        Some(&JsonValue::String("LedgerIndex".to_owned()))
    );
    assert_eq!(
        get_object(
            get_object(root.get("TRANSACTION_FLAGS").expect("TRANSACTION_FLAGS"))
                .get("AccountSet")
                .expect("AccountSet")
        )
        .get("tfAllowXRP"),
        Some(&JsonValue::Signed(0x0020_0000))
    );
    assert_eq!(
        get_object(
            get_object(root.get("LEDGER_ENTRY_FLAGS").expect("LEDGER_ENTRY_FLAGS"))
                .get("Vault")
                .expect("Vault")
        )
        .get("lsfVaultPrivate"),
        Some(&JsonValue::Signed(0x0001_0000))
    );
    assert_eq!(
        get_object(root.get("ACCOUNT_SET_FLAGS").expect("ACCOUNT_SET_FLAGS")).get("asfDisallowXRP"),
        Some(&JsonValue::Signed(3))
    );
}

#[test]
fn server_definitions_returns_only_hash_when_the_hash_matches() {
    let full_response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let hash = match get_object(&full_response)
        .get("hash")
        .expect("hash should exist")
    {
        JsonValue::String(hash) => hash.clone(),
        _ => panic!("hash should be a string"),
    };

    let response = do_server_definitions(&object([("hash", JsonValue::String(hash.clone()))]));
    let root = get_object(&response);

    assert_eq!(root.len(), 1);
    assert_eq!(root.get("hash"), Some(&JsonValue::String(hash)));
    assert!(!root.contains_key("FIELDS"));
    assert!(!root.contains_key("TRANSACTION_FORMATS"));
    assert!(!root.contains_key("LEDGER_ENTRY_FORMATS"));
}

#[test]
fn server_definitions_rejects_non_string_hash_values() {
    let response = do_server_definitions(&object([("hash", JsonValue::Unsigned(10))]));
    let root = get_object(&response);

    assert_eq!(
        root.get("error"),
        Some(&JsonValue::String("invalidParams".into()))
    );
    assert_eq!(root.get("error_code"), Some(&JsonValue::Signed(31)));
    assert_eq!(
        root.get("error_message"),
        Some(&JsonValue::String("Invalid field 'hash'.".into()))
    );
}

#[test]
fn server_definitions_full_response_has_all_sections() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);

    // Verify all expected top-level sections exist
    assert!(root.contains_key("FIELDS"), "FIELDS section missing");
    assert!(
        root.contains_key("TRANSACTION_FORMATS"),
        "TRANSACTION_FORMATS section missing"
    );
    assert!(
        root.contains_key("LEDGER_ENTRY_FORMATS"),
        "LEDGER_ENTRY_FORMATS section missing"
    );
    assert!(root.contains_key("hash"), "hash section missing");
    assert!(root.contains_key("TYPES"), "TYPES section missing");
    assert!(
        root.contains_key("TRANSACTION_RESULTS"),
        "TRANSACTION_RESULTS section missing"
    );

    // Verify FIELDS is an array with entries
    let JsonValue::Array(fields) = root.get("FIELDS").unwrap() else {
        panic!("FIELDS must be an array");
    };
    assert!(fields.len() > 50, "FIELDS should have many entries");

    // Verify TYPES is an object
    let JsonValue::Object(types) = root.get("TYPES").unwrap() else {
        panic!("TYPES must be an object");
    };
    assert!(types.len() > 10, "TYPES should have many entries");
    assert!(types.contains_key("AccountID"));
    assert!(types.contains_key("Amount"));
    assert!(types.contains_key("Hash256"));

    // Verify TRANSACTION_RESULTS is an object
    let JsonValue::Object(results) = root.get("TRANSACTION_RESULTS").unwrap() else {
        panic!("TRANSACTION_RESULTS must be an object");
    };
    assert!(
        results.len() > 10,
        "TRANSACTION_RESULTS should have many entries"
    );
    assert!(results.contains_key("tesSUCCESS"));

    // Verify hash is a string
    let JsonValue::String(hash) = root.get("hash").unwrap() else {
        panic!("hash must be a string");
    };
    assert_eq!(hash.len(), 64, "hash should be 64 hex chars");
}

#[test]
fn server_definitions_wrong_hash_returns_full_response() {
    let response = do_server_definitions(&object([(
        "hash",
        JsonValue::String(
            "0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
        ),
    )]));
    let root = get_object(&response);

    // Wrong hash should return full response
    assert!(root.contains_key("FIELDS"));
    assert!(root.contains_key("TRANSACTION_FORMATS"));
    assert!(root.contains_key("LEDGER_ENTRY_FORMATS"));
    assert!(root.contains_key("hash"));
    assert!(root.contains_key("TYPES"));
}

#[test]
fn server_definitions_fields_have_correct_structure() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);

    let JsonValue::Array(fields) = root.get("FIELDS").unwrap() else {
        panic!("FIELDS must be an array");
    };

    // Each field entry should be a 2-element array: [name, properties]
    for field in fields.iter().take(5) {
        let JsonValue::Array(entry) = field else {
            panic!("each FIELDS entry must be an array");
        };
        assert_eq!(entry.len(), 2, "each FIELDS entry must have 2 elements");
        assert!(
            matches!(&entry[0], JsonValue::String(_)),
            "first element must be field name string"
        );
        assert!(
            matches!(&entry[1], JsonValue::Object(_)),
            "second element must be properties object"
        );

        // Properties should have nth, isVLEncoded, isSerialized, isSigningField, type
        let JsonValue::Object(props) = &entry[1] else {
            panic!("props must be an object");
        };
        assert!(props.contains_key("nth"), "field props should have nth");
        assert!(
            props.contains_key("isVLEncoded"),
            "field props should have isVLEncoded"
        );
        assert!(
            props.contains_key("isSerialized"),
            "field props should have isSerialized"
        );
        assert!(
            props.contains_key("isSigningField"),
            "field props should have isSigningField"
        );
        assert!(props.contains_key("type"), "field props should have type");
    }
}

#[test]
fn server_definitions_types_codes_match_cpp() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(types) = root.get("TYPES").unwrap() else {
        panic!("TYPES must be an object");
    };

    assert_eq!(types.get("Hash128"), Some(&JsonValue::Signed(4)));
    assert_eq!(types.get("Hash160"), Some(&JsonValue::Signed(17)));
    assert_eq!(types.get("Hash192"), Some(&JsonValue::Signed(21)));
    assert_eq!(types.get("Hash256"), Some(&JsonValue::Signed(5)));
    assert_eq!(types.get("AccountID"), Some(&JsonValue::Signed(8)));
    assert_eq!(types.get("Amount"), Some(&JsonValue::Signed(6)));
    assert_eq!(types.get("UInt16"), Some(&JsonValue::Signed(1)));
    assert_eq!(types.get("UInt32"), Some(&JsonValue::Signed(2)));
    assert_eq!(types.get("UInt64"), Some(&JsonValue::Signed(3)));
}

#[test]
fn server_definitions_ledger_entry_types_match_cpp() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(le_types) = root.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("LEDGER_ENTRY_TYPES must be an object");
    };

    assert_eq!(le_types.get("AccountRoot"), Some(&JsonValue::Signed(97)));
    assert_eq!(le_types.get("Offer"), Some(&JsonValue::Signed(111)));
    assert_eq!(le_types.get("RippleState"), Some(&JsonValue::Signed(114)));
    assert_eq!(le_types.get("DirectoryNode"), Some(&JsonValue::Signed(100)));
    assert!(le_types.contains_key("NFTokenPage"));
    assert!(le_types.contains_key("AMM"));
}

#[test]
fn server_definitions_transaction_results_match_cpp() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(results) = root.get("TRANSACTION_RESULTS").unwrap() else {
        panic!("TRANSACTION_RESULTS must be an object");
    };

    assert!(results.contains_key("tesSUCCESS"));
    assert!(results.contains_key("tecDIR_FULL"));
    assert!(results.contains_key("temMALFORMED"));
    assert!(results.contains_key("tefFAILURE"));
    assert!(results.contains_key("terRETRY"));
}

#[test]
fn server_definitions_transaction_types_match_cpp() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(tx_types) = root.get("TRANSACTION_TYPES").unwrap() else {
        panic!("TRANSACTION_TYPES must be an object");
    };

    assert_eq!(tx_types.get("Payment"), Some(&JsonValue::Signed(0)));
    assert!(tx_types.contains_key("OfferCreate"));
    assert!(tx_types.contains_key("OfferCancel"));
    assert!(tx_types.contains_key("TrustSet"));
    assert!(tx_types.contains_key("AccountSet"));
    assert!(tx_types.contains_key("NFTokenMint"));
}

#[test]
fn server_definitions_has_flags_sections() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);

    assert!(
        root.contains_key("LEDGER_ENTRY_FLAGS"),
        "should have LEDGER_ENTRY_FLAGS"
    );
    assert!(
        root.contains_key("TRANSACTION_FLAGS"),
        "should have TRANSACTION_FLAGS"
    );

    let JsonValue::Object(le_flags) = root.get("LEDGER_ENTRY_FLAGS").unwrap() else {
        panic!("LEDGER_ENTRY_FLAGS must be an object");
    };
    assert!(le_flags.contains_key("AccountRoot"));
    let JsonValue::Object(ar_flags) = le_flags.get("AccountRoot").unwrap() else {
        panic!("AccountRoot flags must be an object");
    };
    assert_eq!(
        ar_flags.get("lsfDisallowXRP"),
        Some(&JsonValue::Signed(0x00080000))
    );
}

#[test]
fn server_definitions_has_transaction_formats() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);

    assert!(
        root.contains_key("TRANSACTION_FORMATS"),
        "should have TRANSACTION_FORMATS"
    );
    let JsonValue::Object(tx_formats) = root.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("TRANSACTION_FORMATS must be an object");
    };

    // Should have Payment format
    assert!(tx_formats.contains_key("Payment"));
    let JsonValue::Array(payment_fields) = tx_formats.get("Payment").unwrap() else {
        panic!("Payment format must be an array");
    };
    assert!(!payment_fields.is_empty());

    // Each field entry should have name and optionality
    let JsonValue::Object(first_field) = &payment_fields[0] else {
        panic!("field entry must be an object");
    };
    assert!(first_field.contains_key("name"));
    assert!(first_field.contains_key("optionality"));
}

#[test]
fn server_definitions_has_ledger_entry_formats() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);

    assert!(
        root.contains_key("LEDGER_ENTRY_FORMATS"),
        "should have LEDGER_ENTRY_FORMATS"
    );
    let JsonValue::Object(le_formats) = root.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("LEDGER_ENTRY_FORMATS must be an object");
    };

    // Should have AccountRoot format
    assert!(le_formats.contains_key("AccountRoot"));
    let JsonValue::Array(ar_fields) = le_formats.get("AccountRoot").unwrap() else {
        panic!("AccountRoot format must be an array");
    };
    assert!(!ar_fields.is_empty());
}

#[test]
fn server_definitions_first_field_is_special() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Array(fields) = root.get("FIELDS").unwrap() else {
        panic!("FIELDS must be an array");
    };

    let JsonValue::Array(first) = &fields[0] else {
        panic!("first field must be an array");
    };
    let JsonValue::String(first_name) = &first[0] else {
        panic!("first name must be a string");
    };
    assert_eq!(first_name, "Invalid");
    // Should have properties object
    assert!(matches!(&first[1], JsonValue::Object(_)));
}
