//! server definitions tests part A.

use super::*;

#[test]
fn server_definitions_hash_is_64_hex() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    let JsonValue::String(h) = r.get("hash").unwrap() else {
        panic!("str")
    };
    assert_eq!(h.len(), 64);
}

#[test]
fn server_definitions_has_fields_section() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    assert!(r.contains_key("FIELDS"));
}

#[test]
fn server_definitions_has_types_section() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    assert!(r.contains_key("TYPES"));
}

#[test]
fn server_definitions_has_transaction_types() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    assert!(r.contains_key("TRANSACTION_TYPES"));
}

#[test]
fn server_definitions_has_ledger_entry_types() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    assert!(r.contains_key("LEDGER_ENTRY_TYPES"));
}

#[test]
fn server_definitions_has_transaction_results() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    assert!(r.contains_key("TRANSACTION_RESULTS"));
}

#[test]
fn server_definitions_has_transaction_flags() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    assert!(r.contains_key("TRANSACTION_FLAGS"));
}

#[test]
fn server_definitions_has_ledger_entry_flags() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    assert!(r.contains_key("LEDGER_ENTRY_FLAGS"));
}

#[test]
fn server_definitions_has_transaction_formats() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    assert!(r.contains_key("TRANSACTION_FORMATS"));
}

#[test]
fn server_definitions_has_ledger_entry_formats() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    assert!(r.contains_key("LEDGER_ENTRY_FORMATS"));
}

#[test]
fn defs_server_defs_idempotent() {
    let r1 = rpc::do_server_definitions(&JsonValue::Object(Default::default()));
    let r2 = rpc::do_server_definitions(&JsonValue::Object(Default::default()));
    let JsonValue::Object(r1) = r1 else {
        panic!("obj")
    };
    let JsonValue::Object(r2) = r2 else {
        panic!("obj")
    };
    assert_eq!(r1.get("hash"), r2.get("hash"));
}

#[test]
fn defs_server_defs_cached() {
    let r = rpc::do_server_definitions(&JsonValue::Object(Default::default()));
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    let h = r.get("hash").unwrap().clone();
    let r2 = rpc::do_server_definitions(&json([("hash", h.clone())]));
    let JsonValue::Object(r2) = r2 else {
        panic!("obj")
    };
    assert!(!r2.contains_key("FIELDS"));
}

#[test]
fn defs_server_defs_wrong_hash() {
    let r = rpc::do_server_definitions(&json([(
        "hash",
        sv("0000000000000000000000000000000000000000000000000000000000000000"),
    )]));
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert!(r.contains_key("FIELDS"));
}

#[test]
fn defs_ext_24() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    assert!(r.contains_key("FIELDS"));
    assert!(r.contains_key("TYPES"));
    assert!(r.contains_key("TRANSACTION_TYPES"));
    assert!(r.contains_key("LEDGER_ENTRY_TYPES"));
    assert!(r.contains_key("TRANSACTION_RESULTS"));
    assert!(r.contains_key("hash"));
    assert!(r.contains_key("TRANSACTION_FLAGS"));
    assert!(r.contains_key("LEDGER_ENTRY_FLAGS"));
    assert!(r.contains_key("TRANSACTION_FORMATS"));
    assert!(r.contains_key("LEDGER_ENTRY_FORMATS"));
}

#[test]
fn defs_ext_25() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::String(h) = r.get("hash").unwrap() else {
        panic!("")
    };
    assert_eq!(h.len(), 64);
}

#[test]
fn defs_ext_26() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Array(f) = r.get("FIELDS").unwrap() else {
        panic!("")
    };
    assert!(f.len() > 200);
}

#[test]
fn defs_ext_27() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.len() > 15);
}

#[test]
fn defs_ext_28() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.len() > 25);
}

#[test]
fn defs_ext_29() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.len() > 15);
}

#[test]
fn defs_ext_30() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_RESULTS").unwrap() else {
        panic!("")
    };
    assert!(t.len() > 50);
}

#[test]
fn defs_ext_82() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("AccountID"));
    assert!(t.contains_key("Amount"));
    assert!(t.contains_key("Hash256"));
    assert!(t.contains_key("UInt32"));
    assert!(t.contains_key("Blob"));
}

#[test]
fn defs_ext_83() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("Payment"));
    assert!(t.contains_key("OfferCreate"));
    assert!(t.contains_key("OfferCancel"));
    assert!(t.contains_key("TrustSet"));
    assert!(t.contains_key("AccountSet"));
}

#[test]
fn defs_ext_84() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("EscrowCreate"));
    assert!(t.contains_key("EscrowFinish"));
    assert!(t.contains_key("EscrowCancel"));
}

#[test]
fn defs_ext_85() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("CheckCreate"));
    assert!(t.contains_key("CheckCash"));
    assert!(t.contains_key("CheckCancel"));
}

#[test]
fn defs_ext_86() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("NFTokenMint"));
    assert!(t.contains_key("NFTokenBurn"));
    assert!(t.contains_key("NFTokenCreateOffer"));
}

#[test]
fn defs_ext_87() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("AccountRoot"));
    assert!(t.contains_key("DirectoryNode"));
    assert!(t.contains_key("RippleState"));
    assert!(t.contains_key("Offer"));
}

#[test]
fn defs_ext_88() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("Escrow"));
    assert!(t.contains_key("PayChannel"));
    assert!(t.contains_key("Check"));
    assert!(t.contains_key("Ticket"));
}

#[test]
fn defs_ext_89() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("SignerList"));
    assert!(t.contains_key("DepositPreauth"));
    assert!(t.contains_key("NFTokenPage"));
    assert!(t.contains_key("NFTokenOffer"));
}

#[test]
fn defs_ext_90() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_RESULTS").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("tesSUCCESS"));
    assert!(t.contains_key("tecCLAIM"));
    assert!(t.contains_key("tecPATH_PARTIAL"));
    assert!(t.contains_key("temMALFORMED"));
    assert!(t.contains_key("tefFAILURE"));
    assert!(t.contains_key("terRETRY"));
}

#[test]
fn defs_ext_99() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&json([("hash", u(42))])) else {
        panic!("")
    };
    assert!(r.contains_key("error"));
}

#[test]
fn defs_ext_100() {
    let JsonValue::Object(r1) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let h = r1.get("hash").unwrap().clone();
    let JsonValue::Object(r2) = rpc::do_server_definitions(&json([("hash", h.clone())])) else {
        panic!("")
    };
    assert_eq!(r2.get("hash"), Some(&h));
    assert!(!r2.contains_key("FIELDS"));
}

#[test]
fn defs_flags_1() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("Payment"));
    let JsonValue::Array(pf) = f.get("Payment").unwrap() else {
        panic!("")
    };
    assert!(!pf.is_empty());
}

#[test]
fn defs_flags_2() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("OfferCreate"));
    let JsonValue::Array(of) = f.get("OfferCreate").unwrap() else {
        panic!("")
    };
    assert!(!of.is_empty());
}

#[test]
fn defs_flags_3() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("TrustSet"));
    let JsonValue::Array(tf) = f.get("TrustSet").unwrap() else {
        panic!("")
    };
    assert!(!tf.is_empty());
}

#[test]
fn defs_flags_4() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("EscrowCreate"));
}

#[test]
fn defs_flags_5() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("CheckCreate"));
}

#[test]
fn defs_flags_6() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("AccountRoot"));
    let JsonValue::Array(af) = f.get("AccountRoot").unwrap() else {
        panic!("")
    };
    assert!(!af.is_empty());
}

#[test]
fn defs_flags_7() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("Offer"));
}

#[test]
fn defs_flags_8() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("RippleState"));
}

#[test]
fn defs_flags_9() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("DirectoryNode"));
}

#[test]
fn defs_flags_10() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("Escrow"));
}

#[test]
fn defs_flags_11() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FLAGS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("AccountRoot"));
}

#[test]
fn defs_flags_12() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FLAGS").unwrap() else {
        panic!("")
    };
    if let Some(JsonValue::Object(ar)) = f.get("AccountRoot") {
        assert!(ar.contains_key("lsfDisallowXRP"));
    }
}
