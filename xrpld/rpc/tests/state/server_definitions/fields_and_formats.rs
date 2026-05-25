//! server definitions tests part B.

use super::*;

#[test]
fn server_definitions_field_names_are_unique() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Array(fields) = root.get("FIELDS").unwrap() else {
        panic!("FIELDS must be an array");
    };

    let mut names = std::collections::HashSet::new();
    for field in fields {
        let JsonValue::Array(entry) = field else {
            panic!("field must be an array");
        };
        let JsonValue::String(name) = &entry[0] else {
            panic!("name must be a string");
        };
        assert!(names.insert(name.clone()), "duplicate field name: {name}");
    }
}

#[test]
fn server_definitions_includes_special_fields() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Array(fields) = root.get("FIELDS").unwrap() else {
        panic!("FIELDS must be an array");
    };

    let field_names: Vec<String> = fields
        .iter()
        .filter_map(|f| {
            let JsonValue::Array(entry) = f else {
                return None;
            };
            let JsonValue::String(name) = &entry[0] else {
                return None;
            };
            Some(name.clone())
        })
        .collect();

    assert!(field_names.contains(&"Generic".to_owned()));
    assert!(field_names.contains(&"Invalid".to_owned()));
    assert!(field_names.contains(&"ObjectEndMarker".to_owned()));
    assert!(field_names.contains(&"ArrayEndMarker".to_owned()));
    assert!(field_names.contains(&"taker_gets_funded".to_owned()));
    assert!(field_names.contains(&"taker_pays_funded".to_owned()));
    assert!(field_names.contains(&"hash".to_owned()));
    assert!(field_names.contains(&"index".to_owned()));
}

#[test]
fn server_definitions_transaction_formats_payment_fields() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(tx_formats) = root.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("TRANSACTION_FORMATS must be an object");
    };

    // Payment should have Destination, Amount, etc.
    let JsonValue::Array(payment) = tx_formats.get("Payment").unwrap() else {
        panic!("Payment must be an array");
    };
    let field_names: Vec<&str> = payment
        .iter()
        .filter_map(|f| {
            let JsonValue::Object(obj) = f else {
                return None;
            };
            match obj.get("name") {
                Some(JsonValue::String(s)) => Some(s.as_str()),
                _ => None,
            }
        })
        .collect();
    assert!(
        field_names.contains(&"Destination"),
        "Payment should have Destination"
    );
    assert!(
        field_names.contains(&"Amount"),
        "Payment should have Amount"
    );
}

#[test]
fn server_definitions_ledger_entry_formats_account_root_fields() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(le_formats) = root.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("LEDGER_ENTRY_FORMATS must be an object");
    };

    let JsonValue::Array(account_root) = le_formats.get("AccountRoot").unwrap() else {
        panic!("AccountRoot must be an array");
    };
    let field_names: Vec<&str> = account_root
        .iter()
        .filter_map(|f| {
            let JsonValue::Object(obj) = f else {
                return None;
            };
            match obj.get("name") {
                Some(JsonValue::String(s)) => Some(s.as_str()),
                _ => None,
            }
        })
        .collect();
    assert!(
        field_names.contains(&"Account"),
        "AccountRoot should have Account"
    );
    assert!(
        field_names.contains(&"Sequence"),
        "AccountRoot should have Sequence"
    );
    assert!(
        field_names.contains(&"Balance"),
        "AccountRoot should have Balance"
    );
}

#[test]
fn server_definitions_transaction_types_include_all_common() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(tx_types) = root.get("TRANSACTION_TYPES").unwrap() else {
        panic!("TRANSACTION_TYPES must be an object");
    };

    // All common transaction types should be present
    for expected in [
        "Payment",
        "OfferCreate",
        "OfferCancel",
        "TrustSet",
        "AccountSet",
        "SetRegularKey",
        "SignerListSet",
        "EscrowCreate",
        "EscrowFinish",
        "EscrowCancel",
        "PaymentChannelCreate",
        "PaymentChannelFund",
        "PaymentChannelClaim",
        "CheckCreate",
        "CheckCash",
        "CheckCancel",
        "DepositPreauth",
        "NFTokenMint",
        "NFTokenBurn",
        "NFTokenCreateOffer",
        "NFTokenCancelOffer",
        "NFTokenAcceptOffer",
    ] {
        assert!(
            tx_types.contains_key(expected),
            "TRANSACTION_TYPES should contain {expected}"
        );
    }
}

#[test]
fn server_definitions_ledger_entry_types_include_all_common() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(le_types) = root.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("LEDGER_ENTRY_TYPES must be an object");
    };

    for expected in [
        "AccountRoot",
        "DirectoryNode",
        "RippleState",
        "Offer",
        "LedgerHashes",
        "Amendments",
        "FeeSettings",
        "Escrow",
        "PayChannel",
        "Check",
        "DepositPreauth",
        "Ticket",
        "SignerList",
        "NFTokenPage",
        "NFTokenOffer",
    ] {
        assert!(
            le_types.contains_key(expected),
            "LEDGER_ENTRY_TYPES should contain {expected}"
        );
    }
}

#[test]
fn server_definitions_transaction_results_include_all_categories() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(results) = root.get("TRANSACTION_RESULTS").unwrap() else {
        panic!("TRANSACTION_RESULTS must be an object");
    };

    // Should have results from all categories
    assert!(results.contains_key("tesSUCCESS"), "should have tesSUCCESS");
    assert!(results.contains_key("tecCLAIM"), "should have tecCLAIM");
    assert!(
        results.contains_key("tecPATH_PARTIAL"),
        "should have tecPATH_PARTIAL"
    );
    assert!(
        results.contains_key("tecUNFUNDED_ADD"),
        "should have tecUNFUNDED_ADD"
    );
    assert!(
        results.contains_key("tecDIR_FULL"),
        "should have tecDIR_FULL"
    );
    assert!(
        results.contains_key("temMALFORMED"),
        "should have temMALFORMED"
    );
    assert!(
        results.contains_key("temBAD_AMOUNT"),
        "should have temBAD_AMOUNT"
    );
    assert!(results.contains_key("tefFAILURE"), "should have tefFAILURE");
    assert!(results.contains_key("terRETRY"), "should have terRETRY");
    assert!(results.contains_key("terQUEUED"), "should have terQUEUED");
}

#[test]
fn server_definitions_ledger_entry_flags_include_offer_flags() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(le_flags) = root.get("LEDGER_ENTRY_FLAGS").unwrap() else {
        panic!("LEDGER_ENTRY_FLAGS must be an object");
    };

    // Should have Offer flags
    assert!(le_flags.contains_key("Offer"), "should have Offer flags");
    let JsonValue::Object(offer_flags) = le_flags.get("Offer").unwrap() else {
        panic!("Offer flags must be an object");
    };
    assert!(
        offer_flags.contains_key("lsfPassive"),
        "should have lsfPassive"
    );
    assert!(offer_flags.contains_key("lsfSell"), "should have lsfSell");
}

#[test]
fn server_definitions_transaction_flags_include_payment_flags() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(tx_flags) = root.get("TRANSACTION_FLAGS").unwrap() else {
        panic!("TRANSACTION_FLAGS must be an object");
    };

    // Should have Payment flags
    assert!(
        tx_flags.contains_key("Payment"),
        "should have Payment flags"
    );
    let JsonValue::Object(payment_flags) = tx_flags.get("Payment").unwrap() else {
        panic!("Payment flags must be an object");
    };
    assert!(
        payment_flags.contains_key("tfNoRippleDirect")
            || payment_flags.contains_key("tfNoDirectRipple"),
        "should have noRippleDirect flag"
    );
}

#[test]
fn server_definitions_all_transaction_formats_have_fields() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(tx_formats) = root.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("TRANSACTION_FORMATS must be an object");
    };

    // Every tx format should be an array of field objects (some may be empty for simple txs)
    for (tx_name, fields) in tx_formats {
        let JsonValue::Array(fields) = fields else {
            panic!("{tx_name} format must be an array");
        };
        for field in fields {
            let JsonValue::Object(f) = field else {
                panic!("{tx_name} field entry must be an object");
            };
            assert!(f.contains_key("name"), "{tx_name} field should have name");
            assert!(
                f.contains_key("optionality"),
                "{tx_name} field should have optionality"
            );
        }
    }
    // Should have many tx formats
    assert!(tx_formats.len() > 20, "should have many tx formats");
}

#[test]
fn server_definitions_all_ledger_entry_formats_have_fields() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(le_formats) = root.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("LEDGER_ENTRY_FORMATS must be an object");
    };

    for (le_name, fields) in le_formats {
        let JsonValue::Array(fields) = fields else {
            panic!("{le_name} format must be an array");
        };
        assert!(
            !fields.is_empty(),
            "{le_name} should have at least one field"
        );
        for field in fields {
            let JsonValue::Object(f) = field else {
                panic!("{le_name} field entry must be an object");
            };
            assert!(f.contains_key("name"), "{le_name} field should have name");
            assert!(
                f.contains_key("optionality"),
                "{le_name} field should have optionality"
            );
        }
    }
}

#[test]
fn server_definitions_optionality_values_are_valid() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(tx_formats) = root.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("TRANSACTION_FORMATS must be an object");
    };

    let valid_optionalities = ["Required", "Optional", "Default"];
    for (tx_name, fields) in tx_formats.iter().take(5) {
        let JsonValue::Array(fields) = fields else {
            continue;
        };
        for field in fields {
            let JsonValue::Object(f) = field else {
                continue;
            };
            if let Some(JsonValue::String(opt)) = f.get("optionality") {
                assert!(
                    valid_optionalities.contains(&opt.as_str()),
                    "{tx_name} has invalid optionality: {opt}"
                );
            }
        }
    }
}

#[test]
fn server_definitions_common_tx_fields_present() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(tx_formats) = root.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("TRANSACTION_FORMATS must be an object");
    };

    // OfferCreate should have TakerPays, TakerGets
    if let Some(JsonValue::Array(offer_fields)) = tx_formats.get("OfferCreate") {
        let names: Vec<&str> = offer_fields
            .iter()
            .filter_map(|f| match f {
                JsonValue::Object(o) => match o.get("name") {
                    Some(JsonValue::String(s)) => Some(s.as_str()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert!(
            names.contains(&"TakerPays"),
            "OfferCreate should have TakerPays"
        );
        assert!(
            names.contains(&"TakerGets"),
            "OfferCreate should have TakerGets"
        );
    }

    // TrustSet should have LimitAmount
    if let Some(JsonValue::Array(trust_fields)) = tx_formats.get("TrustSet") {
        let names: Vec<&str> = trust_fields
            .iter()
            .filter_map(|f| match f {
                JsonValue::Object(o) => match o.get("name") {
                    Some(JsonValue::String(s)) => Some(s.as_str()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert!(
            names.contains(&"LimitAmount"),
            "TrustSet should have LimitAmount"
        );
    }

    // EscrowCreate should have Destination, Amount
    if let Some(JsonValue::Array(escrow_fields)) = tx_formats.get("EscrowCreate") {
        let names: Vec<&str> = escrow_fields
            .iter()
            .filter_map(|f| match f {
                JsonValue::Object(o) => match o.get("name") {
                    Some(JsonValue::String(s)) => Some(s.as_str()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert!(
            names.contains(&"Destination"),
            "EscrowCreate should have Destination"
        );
        assert!(names.contains(&"Amount"), "EscrowCreate should have Amount");
    }
}

#[test]
fn server_definitions_ledger_entry_flags_all_have_values() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(le_flags) = root.get("LEDGER_ENTRY_FLAGS").unwrap() else {
        panic!("LEDGER_ENTRY_FLAGS must be an object");
    };

    for (le_name, flags) in le_flags {
        let JsonValue::Object(flags) = flags else {
            panic!("{le_name} flags must be an object");
        };
        for (flag_name, flag_value) in flags {
            assert!(
                matches!(flag_value, JsonValue::Unsigned(_) | JsonValue::Signed(_)),
                "{le_name}.{flag_name} should be a number, got {flag_value:?}"
            );
        }
    }
}

#[test]
fn server_definitions_transaction_flags_all_have_values() {
    let response = do_server_definitions(&JsonValue::Object(BTreeMap::new()));
    let root = get_object(&response);
    let JsonValue::Object(tx_flags) = root.get("TRANSACTION_FLAGS").unwrap() else {
        panic!("TRANSACTION_FLAGS must be an object");
    };

    for (tx_name, flags) in tx_flags {
        let JsonValue::Object(flags) = flags else {
            panic!("{tx_name} flags must be an object");
        };
        for (flag_name, flag_value) in flags {
            assert!(
                matches!(flag_value, JsonValue::Unsigned(_) | JsonValue::Signed(_)),
                "{tx_name}.{flag_name} should be a number"
            );
        }
    }
}
