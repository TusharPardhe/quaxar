//! Tests for definitions validation.

use protocol::{to_base58, JsonValue};
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;
use server::{StreamKind, SubscriptionManager};
// Exact numeric balance assertions with different amounts
#[test]
fn validation_1() {
    let a = TestAccount::new("l01");
    let e = RpcTestEnv::new(&[(&a, 50_000_000)]);
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
        assert_eq!(d.get("Balance"), Some(&sv("50000000")));
    }
}
#[test]
fn validation_2() {
    let a = TestAccount::new("l02");
    let e = RpcTestEnv::new(&[(&a, 1)]);
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
        assert_eq!(d.get("Balance"), Some(&sv("1")));
    }
}
#[test]
fn validation_3() {
    let a = TestAccount::new("l03");
    let e = RpcTestEnv::new(&[(&a, 999_999_999_999)]);
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
        assert_eq!(d.get("Balance"), Some(&sv("999999999999")));
    }
}
// ledger_entry exact balance
#[test]
fn validation_4() {
    let a = TestAccount::new("l04");
    let e = RpcTestEnv::new(&[(&a, 77_777_777)]);
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
        assert_eq!(n.get("Balance"), Some(&sv("77777777")));
        assert_eq!(n.get("LedgerEntryType"), Some(&sv("AccountRoot")));
    }
}
#[test]
fn validation_5() {
    let a = TestAccount::new("l05");
    let e = RpcTestEnv::new(&[(&a, 12_345)]);
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
        assert_eq!(n.get("Balance"), Some(&sv("12345")));
    }
}
// ServerDefinitions: exact TRANSACTION_TYPES codes
#[test]
fn validation_6() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Payment"), Some(&si(0)));
    assert_eq!(t.get("EscrowCreate"), Some(&si(1)));
    assert_eq!(t.get("EscrowFinish"), Some(&si(2)));
    assert_eq!(t.get("AccountSet"), Some(&si(3)));
}
#[test]
fn validation_7() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("SetRegularKey"), Some(&si(5)));
    assert_eq!(t.get("OfferCreate"), Some(&si(7)));
    assert_eq!(t.get("OfferCancel"), Some(&si(8)));
}
#[test]
fn validation_8() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("SignerListSet"), Some(&si(12)));
    assert_eq!(t.get("TrustSet"), Some(&si(20)));
}
#[test]
fn validation_9() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("NFTokenMint"), Some(&si(25)));
    assert_eq!(t.get("NFTokenBurn"), Some(&si(26)));
}
#[test]
fn validation_10() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("AMMCreate"), Some(&si(35)));
    assert_eq!(t.get("AMMDeposit"), Some(&si(36)));
}
// More LEDGER_ENTRY_TYPES exact codes
#[test]
fn validation_11() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Ticket"), Some(&si(84)));
    assert_eq!(t.get("SignerList"), Some(&si(83)));
}
#[test]
fn validation_12() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Escrow"), Some(&si(117)));
    assert_eq!(t.get("PayChannel"), Some(&si(120)));
}
#[test]
fn validation_13() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Check"), Some(&si(67)));
    assert_eq!(t.get("DepositPreauth"), Some(&si(112)));
}
// Subscribe: exact stream kind matching
#[test]
fn validation_14() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Ledger);
    m.publish_json(StreamKind::Ledger, obj());
    let e = r.try_recv().unwrap();
    assert_eq!(e.stream, StreamKind::Ledger);
}
#[test]
fn validation_15() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Transactions);
    m.publish_json(StreamKind::Transactions, obj());
    let e = r.try_recv().unwrap();
    assert_eq!(e.stream, StreamKind::Transactions);
}
#[test]
fn validation_16() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Server);
    m.publish_json(StreamKind::Server, obj());
    let e = r.try_recv().unwrap();
    assert_eq!(e.stream, StreamKind::Server);
}
#[test]
fn validation_17() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Validations);
    m.publish_json(StreamKind::Validations, obj());
    let e = r.try_recv().unwrap();
    assert_eq!(e.stream, StreamKind::Validations);
}
#[test]
fn validation_18() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Manifests);
    m.publish_json(StreamKind::Manifests, obj());
    let e = r.try_recv().unwrap();
    assert_eq!(e.stream, StreamKind::Manifests);
}
#[test]
fn validation_19() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Consensus);
    m.publish_json(StreamKind::Consensus, obj());
    let e = r.try_recv().unwrap();
    assert_eq!(e.stream, StreamKind::Consensus);
}
#[test]
fn validation_20() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::PeerStatus);
    m.publish_json(StreamKind::PeerStatus, obj());
    let e = r.try_recv().unwrap();
    assert_eq!(e.stream, StreamKind::PeerStatus);
}
#[test]
fn validation_21() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::BookChanges);
    m.publish_json(StreamKind::BookChanges, obj());
    let e = r.try_recv().unwrap();
    assert_eq!(e.stream, StreamKind::BookChanges);
}
// Wallet: exact seed round-trip
#[test]
fn validation_22() {
    let r = rpc::wallet_propose(&json([(
        "seed_hex",
        sv("11111111111111111111111111111111"),
    )]))
    .unwrap();
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(
        r.get("master_seed_hex"),
        Some(&sv("11111111111111111111111111111111"))
    );
}
#[test]
fn validation_23() {
    let r = rpc::wallet_propose(&json([(
        "seed_hex",
        sv("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
    )]))
    .unwrap();
    let JsonValue::Object(r) = r else { panic!("") };
    let JsonValue::String(h) = r.get("master_seed_hex").unwrap() else {
        panic!("")
    };
    assert_eq!(h.to_uppercase(), "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF");
}
// Random: 64 hex chars, all unique
#[test]
fn validation_24() {
    let mut s = std::collections::HashSet::new();
    for _ in 0..30 {
        let JsonValue::Object(r) = rpc::do_random() else {
            panic!("")
        };
        let JsonValue::String(v) = r.get("random").unwrap() else {
            panic!("")
        };
        assert_eq!(v.len(), 64);
        assert!(v.chars().all(|c| c.is_ascii_hexdigit()));
        s.insert(v.clone());
    }
    assert_eq!(s.len(), 30);
}
// Ledger: exact index progression
#[test]
fn validation_25() {
    let a = TestAccount::new("l25");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_closed(&s) else {
        panic!("")
    };
    let JsonValue::Unsigned(i1) = r.get("ledger_index").unwrap() else {
        panic!("")
    };
    assert_eq!(*i1, 1);
    e.app.accept_standalone_ledger().unwrap();
    let s2 = e.rpc_source();
    let JsonValue::Object(r2) = rpc::do_ledger_closed(&s2) else {
        panic!("")
    };
    let JsonValue::Unsigned(i2) = r2.get("ledger_index").unwrap() else {
        panic!("")
    };
    assert_eq!(*i2, 2);
}
