//! Tests for definitions amm types.

use protocol::{to_base58, JsonValue};
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;
use server::{StreamKind, SubscriptionManager};
use std::collections::BTreeMap;
#[test]
fn tx_type_code_1() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("EscrowCancel"), Some(&si(4)));
    assert_eq!(t.get("TicketCreate"), Some(&si(10)));
    assert_eq!(t.get("DepositPreauth"), Some(&si(19)));
}
#[test]
fn tx_type_code_2() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("CheckCreate"), Some(&si(16)));
    assert_eq!(t.get("CheckCash"), Some(&si(17)));
    assert_eq!(t.get("CheckCancel"), Some(&si(18)));
}
#[test]
fn tx_type_code_3() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("NFTokenCreateOffer"), Some(&si(27)));
    assert_eq!(t.get("NFTokenCancelOffer"), Some(&si(28)));
    assert_eq!(t.get("NFTokenAcceptOffer"), Some(&si(29)));
}
#[test]
fn tx_type_code_4() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("DIDSet"), Some(&si(49)));
    assert_eq!(t.get("DIDDelete"), Some(&si(50)));
    assert_eq!(t.get("OracleSet"), Some(&si(51)));
    assert_eq!(t.get("OracleDelete"), Some(&si(52)));
}
#[test]
fn tx_type_code_5() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("NFTokenPage"), Some(&si(80)));
    assert_eq!(t.get("NFTokenOffer"), Some(&si(55)));
}
#[test]
fn tx_type_code_6() {
    let a = TestAccount::new("m06");
    let e = RpcTestEnv::new(&[(&a, 4_444_444_444)]);
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
        assert_eq!(d.get("Balance"), Some(&sv("4444444444")));
        assert_eq!(d.get("Sequence"), Some(&u(1)));
        assert_eq!(d.get("Flags"), Some(&u(0)));
        assert_eq!(d.get("Account"), Some(&JsonValue::String(to_base58(a.id))));
    }
}
#[test]
fn tx_type_code_7() {
    let a = TestAccount::new("m07");
    let e = RpcTestEnv::new(&[(&a, 6_666_666_666)]);
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
        assert_eq!(n.get("Balance"), Some(&sv("6666666666")));
        assert_eq!(n.get("Sequence"), Some(&u(1)));
        assert_eq!(n.get("LedgerEntryType"), Some(&sv("AccountRoot")));
        assert_eq!(n.get("Account"), Some(&JsonValue::String(to_base58(a.id))));
    }
}
#[test]
fn tx_type_code_8() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Ledger);
    let p = JsonValue::Object(BTreeMap::from([
        ("type".to_owned(), sv("ledgerClosed")),
        ("ledger_index".to_owned(), u(1000)),
        ("txn_count".to_owned(), u(0)),
        ("fee_base".to_owned(), u(10)),
        ("reserve_base".to_owned(), u(10000000)),
        ("reserve_inc".to_owned(), u(2000000)),
    ]));
    m.publish_json(StreamKind::Ledger, p.clone());
    let e = r.try_recv().unwrap();
    assert_eq!(e.payload, p);
    assert_eq!(e.stream, StreamKind::Ledger);
}
#[test]
fn tx_type_code_9() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Transactions);
    let p = JsonValue::Object(BTreeMap::from([
        ("type".to_owned(), sv("transaction")),
        ("validated".to_owned(), b(true)),
        ("engine_result".to_owned(), sv("tesSUCCESS")),
        ("engine_result_code".to_owned(), si(0)),
    ]));
    m.publish_json(StreamKind::Transactions, p.clone());
    let e = r.try_recv().unwrap();
    assert_eq!(e.payload, p);
    assert_eq!(e.stream, StreamKind::Transactions);
}
#[test]
fn tx_type_code_10() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Server);
    let p = JsonValue::Object(BTreeMap::from([
        ("type".to_owned(), sv("serverStatus")),
        ("server_status".to_owned(), sv("full")),
        ("load_base".to_owned(), u(256)),
        ("load_factor".to_owned(), u(256)),
    ]));
    m.publish_json(StreamKind::Server, p.clone());
    let e = r.try_recv().unwrap();
    assert_eq!(e.payload, p);
    assert_eq!(e.stream, StreamKind::Server);
}
