//! Tests for definitions fields.

use protocol::{to_base58, JsonValue};
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;
use server::{StreamKind, SubscriptionManager};
use std::collections::BTreeMap;
// Exact balance assertions after funding
#[test]
fn definitions_field_1() {
    let a = TestAccount::new("k01");
    let e = RpcTestEnv::new(&[(&a, 1_234_567_890)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("ledger_index", sv("current")),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Object(d)) = r.get("account_data") {
        assert_eq!(d.get("Balance"), Some(&sv("1234567890")));
        assert_eq!(d.get("Sequence"), Some(&u(1)));
        assert_eq!(d.get("Flags"), Some(&u(0)));
    }
}
#[test]
fn definitions_field_2() {
    let a = TestAccount::new("k02");
    let e = RpcTestEnv::new(&[(&a, 999_999_999)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("ledger_index", sv("current")),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Object(d)) = r.get("account_data") {
        assert_eq!(d.get("Balance"), Some(&sv("999999999")));
    }
}
#[test]
fn definitions_field_3() {
    let a = TestAccount::new("k03");
    let e = RpcTestEnv::new(&[(&a, 100_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("ledger_index", sv("current")),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Object(d)) = r.get("account_data") {
        assert_eq!(d.get("Balance"), Some(&sv("100000000000")));
    }
}
// Exact ledger_entry balance
#[test]
fn definitions_field_4() {
    let a = TestAccount::new("k04");
    let e = RpcTestEnv::new(&[(&a, 2_222_222_222)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                ("account_root", JsonValue::String(to_base58(a.id))),
                ("ledger_index", sv("current")),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Object(n)) = r.get("node") {
        assert_eq!(n.get("Balance"), Some(&sv("2222222222")));
        assert_eq!(n.get("Sequence"), Some(&u(1)));
    }
}
#[test]
fn definitions_field_5() {
    let a = TestAccount::new("k05");
    let e = RpcTestEnv::new(&[(&a, 8_888_888_888)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                ("account_root", JsonValue::String(to_base58(a.id))),
                ("ledger_index", sv("current")),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Object(n)) = r.get("node") {
        assert_eq!(n.get("Balance"), Some(&sv("8888888888")));
    }
}
// Subscribe exact payload round-trip (10 tests)
#[test]
fn definitions_field_6() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Ledger);
    let p = JsonValue::Object(BTreeMap::from([
        ("a".to_owned(), u(1)),
        ("b".to_owned(), sv("x")),
    ]));
    m.publish_json(StreamKind::Ledger, p.clone());
    assert_eq!(r.try_recv().unwrap().payload, p);
}
#[test]
fn definitions_field_7() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Transactions);
    let p = JsonValue::Object(BTreeMap::from([
        ("c".to_owned(), b(true)),
        ("d".to_owned(), si(-5)),
    ]));
    m.publish_json(StreamKind::Transactions, p.clone());
    assert_eq!(r.try_recv().unwrap().payload, p);
}
#[test]
fn definitions_field_8() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Server);
    let p = JsonValue::Object(BTreeMap::from([("e".to_owned(), u(999))]));
    m.publish_json(StreamKind::Server, p.clone());
    assert_eq!(r.try_recv().unwrap().payload, p);
}
#[test]
fn definitions_field_9() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Validations);
    let p = JsonValue::Object(BTreeMap::from([("f".to_owned(), sv("val"))]));
    m.publish_json(StreamKind::Validations, p.clone());
    assert_eq!(r.try_recv().unwrap().payload, p);
}
#[test]
fn definitions_field_10() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Manifests);
    let p = JsonValue::Object(BTreeMap::from([("g".to_owned(), u(0))]));
    m.publish_json(StreamKind::Manifests, p.clone());
    assert_eq!(r.try_recv().unwrap().payload, p);
}
#[test]
fn definitions_field_11() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Consensus);
    let p = JsonValue::Object(BTreeMap::from([("h".to_owned(), b(false))]));
    m.publish_json(StreamKind::Consensus, p.clone());
    assert_eq!(r.try_recv().unwrap().payload, p);
}
#[test]
fn definitions_field_12() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::PeerStatus);
    let p = JsonValue::Object(BTreeMap::from([("i".to_owned(), sv("peer"))]));
    m.publish_json(StreamKind::PeerStatus, p.clone());
    assert_eq!(r.try_recv().unwrap().payload, p);
}
#[test]
fn definitions_field_13() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::BookChanges);
    let p = JsonValue::Object(BTreeMap::from([("j".to_owned(), u(42))]));
    m.publish_json(StreamKind::BookChanges, p.clone());
    assert_eq!(r.try_recv().unwrap().payload, p);
}
// ServerDefinitions exact type codes
#[test]
fn definitions_field_14() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Unknown"), Some(&si(-2)));
    assert_eq!(t.get("Done"), Some(&si(-1)));
    assert_eq!(t.get("NotPresent"), Some(&si(0)));
}
#[test]
fn definitions_field_15() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("UInt16"), Some(&si(1)));
    assert_eq!(t.get("UInt32"), Some(&si(2)));
    assert_eq!(t.get("UInt64"), Some(&si(3)));
    assert_eq!(t.get("Hash128"), Some(&si(4)));
    assert_eq!(t.get("Hash256"), Some(&si(5)));
    assert_eq!(t.get("Amount"), Some(&si(6)));
    assert_eq!(t.get("Blob"), Some(&si(7)));
    assert_eq!(t.get("AccountID"), Some(&si(8)));
}
// ServerDefinitions: LEDGER_ENTRY_TYPES exact codes
#[test]
fn definitions_field_16() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("AccountRoot"), Some(&si(97)));
    assert_eq!(t.get("DirectoryNode"), Some(&si(100)));
    assert_eq!(t.get("RippleState"), Some(&si(114)));
    assert_eq!(t.get("Offer"), Some(&si(111)));
    assert_eq!(t.get("LedgerHashes"), Some(&si(104)));
}
// Wallet: seed_hex round-trip
#[test]
fn definitions_field_17() {
    let r = rpc::wallet_propose(&json([(
        "seed_hex",
        sv("AAAABBBBCCCCDDDDEEEEFFFFAAAABBBB"),
    )]))
    .unwrap();
    let JsonValue::Object(r) = r else { panic!("") };
    let JsonValue::String(h) = r.get("master_seed_hex").unwrap() else {
        panic!("")
    };
    assert_eq!(h.to_uppercase(), "AAAABBBBCCCCDDDDEEEEFFFFAAAABBBB");
}
#[test]
fn definitions_field_18() {
    let r = rpc::wallet_propose(&json([(
        "seed_hex",
        sv("00000000000000000000000000000001"),
    )]))
    .unwrap();
    let JsonValue::Object(r) = r else { panic!("") };
    let JsonValue::String(h) = r.get("master_seed_hex").unwrap() else {
        panic!("")
    };
    assert_eq!(h.to_uppercase(), "00000000000000000000000000000001");
}
// More ledger progression
#[test]
fn definitions_field_19() {
    let a = TestAccount::new("k19");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_closed(&s) else {
        panic!("")
    };
    let JsonValue::Unsigned(i) = r.get("ledger_index").unwrap() else {
        panic!("")
    };
    assert!(*i >= 1);
    e.app.accept_standalone_ledger().unwrap();
    let s2 = e.rpc_source();
    let JsonValue::Object(r2) = rpc::do_ledger_closed(&s2) else {
        panic!("")
    };
    let JsonValue::Unsigned(i2) = r2.get("ledger_index").unwrap() else {
        panic!("")
    };
    assert!(*i2 >= *i + 1);
}
#[test]
fn definitions_field_20() {
    let a = TestAccount::new("k20");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    for _ in 0..10 {
        e.app.accept_standalone_ledger().unwrap();
    }
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_current(&s) else {
        panic!("")
    };
    let JsonValue::Unsigned(i) = r.get("ledger_current_index").unwrap() else {
        panic!("")
    };
    assert!(*i >= 12);
}
