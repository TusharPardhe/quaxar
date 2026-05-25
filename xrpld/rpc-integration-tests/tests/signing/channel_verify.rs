//! Integration tests for channel verify operations.

use protocol::JsonValue;
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;

#[test]
fn defs_channel_verify_missing() {
    let r = rpc::do_channel_verify(&JsonValue::Object(Default::default()));
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert!(r.contains_key("error"));
}

#[test]
fn defs_channel_verify_bad_key() {
    let r = rpc::do_channel_verify(&json([
        ("public_key", sv("bad")),
        ("channel_id", sv(&"AA".repeat(32))),
        ("amount", sv("1")),
        ("signature", sv("BB")),
    ]));
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert_eq!(r.get("error"), Some(&sv("publicMalformed")));
}

#[test]
fn defs_channel_verify_bad_chan() {
    let r = rpc::do_channel_verify(&json([
        (
            "public_key",
            sv("aB4BXXLuPu8DpVuyq1DBiu3SrPdtK9AYZisKhu8mvkoiUD8J9Gov"),
        ),
        ("channel_id", sv("short")),
        ("amount", sv("1")),
        ("signature", sv("CC")),
    ]));
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert_eq!(r.get("error"), Some(&sv("channelMalformed")));
}

#[test]
fn defs_channel_verify_bad_amt() {
    let r = rpc::do_channel_verify(&json([
        (
            "public_key",
            sv("aB4BXXLuPu8DpVuyq1DBiu3SrPdtK9AYZisKhu8mvkoiUD8J9Gov"),
        ),
        ("channel_id", sv(&"DD".repeat(32))),
        ("amount", sv("99999999999999999999")),
        ("signature", sv("EE")),
    ]));
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert_eq!(r.get("error"), Some(&sv("channelAmtMalformed")));
}

#[test]
fn defs_channel_verify_false() {
    let r = rpc::do_channel_verify(&json([
        (
            "public_key",
            sv("aB4BXXLuPu8DpVuyq1DBiu3SrPdtK9AYZisKhu8mvkoiUD8J9Gov"),
        ),
        ("channel_id", sv(&"FF".repeat(32))),
        ("amount", sv("100")),
        ("signature", sv(&"AB".repeat(64))),
    ]));
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert_eq!(r.get("signature_verified"), Some(&b(false)));
}

#[test]
fn defs_ext_74() {
    let r = rpc::do_channel_verify(&obj());
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(r.get("error"), Some(&sv("invalidParams")));
}

#[test]
fn defs_ext_75() {
    let r = rpc::do_channel_verify(&json([
        ("public_key", sv("bad")),
        ("channel_id", JsonValue::String("AA".repeat(32))),
        ("amount", sv("1")),
        ("signature", sv("BB")),
    ]));
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(r.get("error"), Some(&sv("publicMalformed")));
}

#[test]
fn defs_ext_76() {
    let r = rpc::do_channel_verify(&json([
        (
            "public_key",
            sv("aB4BXXLuPu8DpVuyq1DBiu3SrPdtK9AYZisKhu8mvkoiUD8J9Gov"),
        ),
        ("channel_id", sv("short")),
        ("amount", sv("1")),
        ("signature", sv("CC")),
    ]));
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(r.get("error"), Some(&sv("channelMalformed")));
}

#[test]
fn defs_ext_77() {
    let r = rpc::do_channel_verify(&json([
        (
            "public_key",
            sv("aB4BXXLuPu8DpVuyq1DBiu3SrPdtK9AYZisKhu8mvkoiUD8J9Gov"),
        ),
        ("channel_id", JsonValue::String("DD".repeat(32))),
        ("amount", sv("99999999999999999999")),
        ("signature", sv("EE")),
    ]));
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(r.get("error"), Some(&sv("channelAmtMalformed")));
}

#[test]
fn defs_ext_78() {
    let r = rpc::do_channel_verify(&json([
        (
            "public_key",
            sv("aB4BXXLuPu8DpVuyq1DBiu3SrPdtK9AYZisKhu8mvkoiUD8J9Gov"),
        ),
        ("channel_id", JsonValue::String("FF".repeat(32))),
        ("amount", sv("100")),
        ("signature", JsonValue::String("AB".repeat(64))),
    ]));
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(r.get("signature_verified"), Some(&b(false)));
}

#[test]
fn defs_fmt_48() {
    let r = rpc::do_channel_verify(&JsonValue::Object(Default::default()));
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(r.get("error"), Some(&sv("invalidParams")));
}

#[test]
fn defs_types_99_channel_verify_structure() {
    let r = rpc::do_channel_verify(&json([
        (
            "public_key",
            sv("aB4BXXLuPu8DpVuyq1DBiu3SrPdtK9AYZisKhu8mvkoiUD8J9Gov"),
        ),
        ("channel_id", JsonValue::String("AA".repeat(32))),
        ("amount", sv("500")),
        ("signature", JsonValue::String("BB".repeat(64))),
    ]));
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert!(r.contains_key("signature_verified"));
    assert_eq!(r.get("signature_verified"), Some(&b(false)));
}
