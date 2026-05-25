//! server definitions tests part B.

use super::*;

#[test]
fn defs_flags_13() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FLAGS").unwrap() else {
        panic!("")
    };
    assert!(!f.is_empty());
}

#[test]
fn defs_fmt_25() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    assert!(r.len() >= 8);
}

#[test]
fn defs_fmt_26() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Array(f) = r.get("FIELDS").unwrap() else {
        panic!("")
    };
    assert!(f.len() > 100);
}

#[test]
fn defs_fmt_27() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("UInt32"));
}

#[test]
fn defs_fmt_28() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("Amount"));
}

#[test]
fn defs_fmt_29() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("Blob"));
}

#[test]
fn defs_fmt_30() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("EscrowCreate"));
}

#[test]
fn defs_fmt_31() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("CheckCreate"));
}

#[test]
fn defs_fmt_32() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("NFTokenMint"));
}

#[test]
fn defs_fmt_33() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("AccountDelete"));
}

#[test]
fn defs_fmt_34() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("Escrow"));
}

#[test]
fn defs_fmt_35() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("Check"));
}

#[test]
fn defs_fmt_36() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("PayChannel"));
}

#[test]
fn defs_fmt_37() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("Ticket"));
}

#[test]
fn defs_fmt_38() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("SignerList"));
}

#[test]
fn defs_fmt_39() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("DepositPreauth"));
}

#[test]
fn defs_fmt_40() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_RESULTS").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("terRETRY"));
}

#[test]
fn defs_fmt_41() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_RESULTS").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("tefFAILURE"));
}

#[test]
fn defs_fmt_42() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_RESULTS").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("tecPATH_PARTIAL"));
}

#[test]
fn defs_fmt_43() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_RESULTS").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("tecUNFUNDED_ADD"));
}

#[test]
fn defs_fmt_57() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("SignerListSet"));
}

#[test]
fn defs_fmt_58() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("TicketCreate"));
}

#[test]
fn defs_fmt_59() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("DepositPreauth"));
}

#[test]
fn defs_fmt_60() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("AccountSet"));
}

#[test]
fn defs_fmt_61() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("SetRegularKey"));
}

#[test]
fn defs_fmt_62() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("NFTokenOffer"));
}

#[test]
fn defs_fmt_63() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("NFTokenPage"));
}

#[test]
fn defs_fmt_64() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("AMM"));
}

#[test]
fn defs_fmt_65() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::String(h) = r.get("hash").unwrap() else {
        panic!("")
    };
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn defs_fmt_66() {
    let r1 = rpc::do_server_definitions(&JsonValue::Object(Default::default()));
    let JsonValue::Object(r1) = r1 else {
        panic!("")
    };
    let h = r1.get("hash").unwrap().clone();
    let r2 = rpc::do_server_definitions(&json([("hash", h.clone())]));
    let JsonValue::Object(r2) = r2 else {
        panic!("")
    };
    assert_eq!(r2.get("hash"), Some(&h));
    assert!(!r2.contains_key("FIELDS"));
}

#[test]
fn defs_fmt_67() {
    let r = rpc::do_server_definitions(&json([(
        "hash",
        sv("0000000000000000000000000000000000000000000000000000000000000000"),
    )]));
    let JsonValue::Object(r) = r else { panic!("") };
    assert!(r.contains_key("FIELDS"));
}

#[test]
fn defs_fmt_68() {
    let r = rpc::do_server_definitions(&json([("hash", u(42))]));
    let JsonValue::Object(r) = r else { panic!("") };
    assert!(r.contains_key("error"));
}

#[test]
fn defs_fmt_78() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FLAGS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("Payment") || f.contains_key("OfferCreate"));
}

#[test]
fn defs_fmt_79() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FLAGS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("AccountRoot") || f.contains_key("Offer"));
}

#[test]
fn defs_fmt_80() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("Payment"));
    assert!(f.contains_key("OfferCreate"));
    assert!(f.contains_key("TrustSet"));
}

#[test]
fn defs_fmt_81() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("AccountRoot"));
    assert!(f.contains_key("Offer"));
    assert!(f.contains_key("RippleState"));
}

#[test]
fn defs_fmt_99() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.len() > 25);
}

#[test]
fn defs_fmt_100() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_RESULTS").unwrap() else {
        panic!("")
    };
    assert!(t.len() > 50);
}

#[test]
fn defs_types_86_defs_payment_in_tx_types() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("obj")
    };
    assert!(t.contains_key("Payment"));
}

#[test]
fn defs_types_87_defs_offer_in_tx_types() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("obj")
    };
    assert!(t.contains_key("OfferCreate"));
    assert!(t.contains_key("OfferCancel"));
}

#[test]
fn defs_types_88_defs_trust_in_tx_types() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("obj")
    };
    assert!(t.contains_key("TrustSet"));
}

#[test]
fn defs_types_89_defs_account_root_in_le() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("obj")
    };
    assert!(t.contains_key("AccountRoot"));
    assert!(t.contains_key("Offer"));
    assert!(t.contains_key("RippleState"));
}

#[test]
fn defs_types_90_defs_tes_success_in_results() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&JsonValue::Object(Default::default()))
    else {
        panic!("obj")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_RESULTS").unwrap() else {
        panic!("obj")
    };
    assert!(t.contains_key("tesSUCCESS"));
    assert!(t.contains_key("tecCLAIM"));
    assert!(t.contains_key("temMALFORMED"));
}
// More random checks
