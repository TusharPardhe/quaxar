//! Integration tests for wallet operations.

use protocol::JsonValue;
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;
use server::{StreamKind, SubscriptionManager};

#[test]
fn wallet_propose_returns_account_and_keys() {
    let r = rpc::wallet_propose(&json([])).unwrap();
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert!(r.contains_key("account_id"));
    assert!(r.contains_key("master_seed"));
    assert!(r.contains_key("public_key"));
}

#[test]
fn wallet_propose_ed25519_key_type() {
    let r = rpc::wallet_propose(&json([("key_type", sv("ed25519"))])).unwrap();
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert_eq!(r.get("key_type"), Some(&sv("ed25519")));
}

#[test]
fn wallet_propose_secp256k1_default() {
    let r = rpc::wallet_propose(&json([])).unwrap();
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert_eq!(r.get("key_type"), Some(&sv("secp256k1")));
}

#[test]
fn wallet_propose_seed_starts_with_s() {
    let r = rpc::wallet_propose(&json([])).unwrap();
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    let JsonValue::String(seed) = r.get("master_seed").unwrap() else {
        panic!("str")
    };
    assert!(seed.starts_with('s'));
}

#[test]
fn server_definitions_wallet_acct_format() {
    let r = rpc::wallet_propose(&json([])).unwrap();
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    let JsonValue::String(a) = r.get("account_id").unwrap() else {
        panic!("str")
    };
    assert!(a.starts_with('r'));
}

#[test]
fn server_definitions_wallet_bad_type() {
    assert!(rpc::wallet_propose(&json([("key_type", sv("bad"))])).is_err());
}

#[test]
fn server_definitions_wallet_passphrase() {
    let r = rpc::wallet_propose(&json([("passphrase", sv("test"))])).unwrap();
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert!(r.contains_key("warning"));
}

#[test]
fn defs_wallet_deterministic() {
    let r1 = rpc::wallet_propose(&json([("passphrase", sv("hello"))])).unwrap();
    let r2 = rpc::wallet_propose(&json([("passphrase", sv("hello"))])).unwrap();
    let JsonValue::Object(r1) = r1 else {
        panic!("obj")
    };
    let JsonValue::Object(r2) = r2 else {
        panic!("obj")
    };
    assert_eq!(r1.get("account_id"), r2.get("account_id"));
}

#[test]
fn defs_wallet_different_seeds() {
    let r1 = rpc::wallet_propose(&json([])).unwrap();
    let r2 = rpc::wallet_propose(&json([])).unwrap();
    let JsonValue::Object(r1) = r1 else {
        panic!("obj")
    };
    let JsonValue::Object(r2) = r2 else {
        panic!("obj")
    };
    assert_ne!(r1.get("master_seed"), r2.get("master_seed"));
}

#[test]
fn defs_ext_7() {
    let r = rpc::wallet_propose(&json([])).unwrap();
    assert!(matches!(r, JsonValue::Object(_)));
}

#[test]
fn defs_ext_8() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    assert!(r.len() >= 6);
}

#[test]
fn defs_ext_9() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    let JsonValue::String(a) = r.get("account_id").unwrap() else {
        panic!("")
    };
    assert!(a.starts_with('r'));
    assert!(a.len() >= 25);
}

#[test]
fn defs_ext_10() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    let JsonValue::String(s) = r.get("master_seed").unwrap() else {
        panic!("")
    };
    assert!(s.starts_with('s'));
}

#[test]
fn defs_ext_11() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    let JsonValue::String(h) = r.get("master_seed_hex").unwrap() else {
        panic!("")
    };
    assert_eq!(h.len(), 32);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn defs_ext_12() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    let JsonValue::String(p) = r.get("public_key").unwrap() else {
        panic!("")
    };
    assert!(p.starts_with('a'));
}

#[test]
fn defs_ext_13() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    let JsonValue::String(h) = r.get("public_key_hex").unwrap() else {
        panic!("")
    };
    assert!(h.len() >= 66);
}

#[test]
fn defs_ext_14() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    assert_eq!(r.get("key_type"), Some(&sv("secp256k1")));
}

#[test]
fn defs_ext_15() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([("key_type", sv("ed25519"))])).unwrap()
    else {
        panic!("")
    };
    assert_eq!(r.get("key_type"), Some(&sv("ed25519")));
}

#[test]
fn defs_ext_16() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([("key_type", sv("ed25519"))])).unwrap()
    else {
        panic!("")
    };
    let JsonValue::String(h) = r.get("public_key_hex").unwrap() else {
        panic!("")
    };
    assert!(h.to_uppercase().starts_with("ED"));
}

#[test]
fn defs_ext_17() {
    assert!(rpc::wallet_propose(&json([("key_type", sv("invalid"))])).is_err());
}

#[test]
fn defs_ext_18() {
    assert!(rpc::wallet_propose(&json([("key_type", u(1))])).is_err());
}

#[test]
fn defs_ext_19() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([("passphrase", sv("test"))])).unwrap()
    else {
        panic!("")
    };
    assert!(r.contains_key("warning"));
}

#[test]
fn defs_ext_20() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    assert!(!r.contains_key("warning"));
}

#[test]
fn defs_ext_21() {
    let r1 = rpc::wallet_propose(&json([("passphrase", sv("same"))])).unwrap();
    let r2 = rpc::wallet_propose(&json([("passphrase", sv("same"))])).unwrap();
    let (JsonValue::Object(r1), JsonValue::Object(r2)) = (r1, r2) else {
        panic!("")
    };
    assert_eq!(r1.get("account_id"), r2.get("account_id"));
}

#[test]
fn defs_ext_22() {
    let r1 = rpc::wallet_propose(&json([("passphrase", sv("a"))])).unwrap();
    let r2 = rpc::wallet_propose(&json([("passphrase", sv("b"))])).unwrap();
    let (JsonValue::Object(r1), JsonValue::Object(r2)) = (r1, r2) else {
        panic!("")
    };
    assert_ne!(r1.get("account_id"), r2.get("account_id"));
}

#[test]
fn defs_ext_23() {
    let r1 = rpc::wallet_propose(&json([])).unwrap();
    let r2 = rpc::wallet_propose(&json([])).unwrap();
    let (JsonValue::Object(r1), JsonValue::Object(r2)) = (r1, r2) else {
        panic!("")
    };
    assert_ne!(r1.get("master_seed"), r2.get("master_seed"));
}

#[test]
fn defs_fmt_11() {
    let r = rpc::wallet_propose(&json([])).unwrap();
    assert!(matches!(r, JsonValue::Object(_)));
}

#[test]
fn defs_fmt_12() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    assert!(r.contains_key("account_id"));
}

#[test]
fn defs_fmt_13() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    assert!(r.contains_key("master_seed"));
}

#[test]
fn defs_fmt_14() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    assert!(r.contains_key("master_seed_hex"));
}

#[test]
fn defs_fmt_15() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    assert!(r.contains_key("public_key"));
}

#[test]
fn defs_fmt_16() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    assert!(r.contains_key("public_key_hex"));
}

#[test]
fn defs_fmt_17() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    assert!(r.contains_key("key_type"));
}

#[test]
fn defs_fmt_18() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    assert!(!r.contains_key("error"));
}

#[test]
fn defs_fmt_19() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([])).unwrap() else {
        panic!("")
    };
    assert!(!r.contains_key("warning"));
}

#[test]
fn defs_fmt_20() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([("passphrase", sv("x"))])).unwrap()
    else {
        panic!("")
    };
    assert!(r.contains_key("warning"));
}

#[test]
fn defs_fmt_21() {
    assert!(rpc::wallet_propose(&json([("key_type", sv("bad"))])).is_err());
}

#[test]
fn defs_fmt_22() {
    assert!(rpc::wallet_propose(&json([("key_type", u(1))])).is_err());
}

#[test]
fn defs_fmt_23() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([("key_type", sv("ed25519"))])).unwrap()
    else {
        panic!("")
    };
    assert_eq!(r.get("key_type"), Some(&sv("ed25519")));
}

#[test]
fn defs_fmt_24() {
    let JsonValue::Object(r) = rpc::wallet_propose(&json([("key_type", sv("secp256k1"))])).unwrap()
    else {
        panic!("")
    };
    assert_eq!(r.get("key_type"), Some(&sv("secp256k1")));
}

#[test]
fn defs_fmt_87() {
    let r = rpc::wallet_propose(&json([("passphrase", sv("a"))])).unwrap();
    let JsonValue::Object(r) = r else { panic!("") };
    let JsonValue::String(w) = r.get("warning").unwrap() else {
        panic!("")
    };
    assert!(w.contains("passphrase"));
}

#[test]
fn defs_fmt_88() {
    let r1 = rpc::wallet_propose(&json([("passphrase", sv("same"))])).unwrap();
    let r2 = rpc::wallet_propose(&json([("passphrase", sv("same"))])).unwrap();
    let JsonValue::Object(r1) = r1 else {
        panic!("")
    };
    let JsonValue::Object(r2) = r2 else {
        panic!("")
    };
    assert_eq!(r1.get("master_seed"), r2.get("master_seed"));
}

#[test]
fn defs_fmt_89() {
    let r1 = rpc::wallet_propose(&json([("passphrase", sv("one"))])).unwrap();
    let r2 = rpc::wallet_propose(&json([("passphrase", sv("two"))])).unwrap();
    let JsonValue::Object(r1) = r1 else {
        panic!("")
    };
    let JsonValue::Object(r2) = r2 else {
        panic!("")
    };
    assert_ne!(r1.get("account_id"), r2.get("account_id"));
}

#[test]
fn defs_types_80_sub_manifest_no_peer() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Manifests);
    m.publish_json(
        StreamKind::PeerStatus,
        JsonValue::Object(Default::default()),
    );
    assert!(rx.try_recv().is_err());
}
// More wallet_propose checks

#[test]
fn defs_types_81_wallet_seed_hex_len() {
    let r = rpc::wallet_propose(&json([])).unwrap();
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    let JsonValue::String(h) = r.get("master_seed_hex").unwrap() else {
        panic!("str")
    };
    assert_eq!(h.len(), 32);
}

#[test]
fn defs_types_82_wallet_pub_key_hex_len() {
    let r = rpc::wallet_propose(&json([])).unwrap();
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    let JsonValue::String(h) = r.get("public_key_hex").unwrap() else {
        panic!("str")
    };
    assert!(h.len() >= 66);
}

#[test]
fn defs_types_83_wallet_pub_key_base58() {
    let r = rpc::wallet_propose(&json([])).unwrap();
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    let JsonValue::String(pk) = r.get("public_key").unwrap() else {
        panic!("str")
    };
    assert!(pk.starts_with('a'));
}

#[test]
fn defs_types_84_wallet_no_warning_random() {
    let r = rpc::wallet_propose(&json([])).unwrap();
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert!(!r.contains_key("warning"));
}

#[test]
fn defs_types_85_wallet_warning_passphrase() {
    let r = rpc::wallet_propose(&json([("passphrase", sv("weak"))])).unwrap();
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert!(r.contains_key("warning"));
}
// More server_definitions checks
