//! Tests for signing and streams.

use protocol::{
    currency_from_string, get_field_by_symbol, lsfDefaultRipple, to_base58, Issue, JsonValue,
    STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;
use server::{StreamKind, SubscriptionManager};
use std::collections::BTreeMap;
#[test]
fn sub_sign_1() {
    let mut a = TestAccount::new("g01a");
    let mut g = TestAccount::new("g01g");
    let e = RpcTestEnv::with_flags(
        &[(&a, 10_000_000_000), (&g, 10_000_000_000)],
        &[(&g, lsfDefaultRipple)],
    );
    let usd = currency_from_string("USD");
    let mut t = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(usd, g.id),
                10000,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut t, &a);
    e.submit_and_close(&t);
    let mut p = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), g.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfAmount"),
                Issue::new(usd, g.id),
                1000,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), g.next_seq());
    });
    sign_tx(&mut p, &g);
    e.submit_and_close(&p);
    let mut o = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(400_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerGets"),
                Issue::new(usd, g.id),
                100,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut o, &a);
    e.submit_and_close(&o);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_book_offers(
        &rpc::BookOffersRequest {
            params: &json([
                ("taker_pays", json([("currency", sv("XRP"))])),
                (
                    "taker_gets",
                    json([
                        ("currency", sv("USD")),
                        ("issuer", JsonValue::String(to_base58(g.id))),
                    ]),
                ),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Array(offers)) = r.get("offers") {
        if !offers.is_empty() {
            let JsonValue::Object(of) = &offers[0] else {
                panic!("")
            };
            assert!(of.contains_key("Account"));
            assert!(of.contains_key("TakerPays"));
            assert!(of.contains_key("TakerGets"));
            assert!(of.contains_key("Sequence"));
            assert!(of.contains_key("BookDirectory"));
            assert!(of.contains_key("quality"));
            assert!(of.contains_key("owner_funds"));
            assert!(of.contains_key("index"));
            if let Some(JsonValue::String(q)) = of.get("quality") {
                let qv: u64 = q.parse().unwrap_or(0);
                assert!(qv > 0);
            }
            if let Some(JsonValue::String(f)) = of.get("owner_funds") {
                assert!(!f.is_empty());
            }
        }
    }
}
#[test]
fn sub_sign_2() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Ledger);
    m.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("ledgerClosed")),
            ("ledger_index".to_owned(), u(42)),
            ("ledger_hash".to_owned(), sv("AABB")),
            ("txn_count".to_owned(), u(5)),
            ("fee_base".to_owned(), u(10)),
            ("fee_ref".to_owned(), u(10)),
            ("reserve_base".to_owned(), u(10000000)),
            ("reserve_inc".to_owned(), u(2000000)),
            ("validated_ledgers".to_owned(), sv("1-42")),
        ])),
    );
    let e = r.try_recv().unwrap();
    let _payload_json: JsonValue = serde_json::from_slice(&e.payload).unwrap();
    let JsonValue::Object(p) = _payload_json else {
        panic!("")
    };
    assert_eq!(p.get("type"), Some(&sv("ledgerClosed")));
    assert_eq!(p.get("ledger_index"), Some(&u(42)));
    assert_eq!(p.get("txn_count"), Some(&u(5)));
    assert_eq!(p.get("fee_base"), Some(&u(10)));
    assert_eq!(p.get("fee_ref"), Some(&u(10)));
    assert_eq!(p.get("reserve_base"), Some(&u(10000000)));
    assert_eq!(p.get("reserve_inc"), Some(&u(2000000)));
    assert_eq!(p.get("validated_ledgers"), Some(&sv("1-42")));
}
#[test]
fn sub_sign_3() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Transactions);
    m.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("transaction")),
            ("validated".to_owned(), b(true)),
            ("engine_result".to_owned(), sv("tesSUCCESS")),
            ("engine_result_code".to_owned(), si(0)),
            (
                "engine_result_message".to_owned(),
                sv("The transaction was applied."),
            ),
            ("ledger_index".to_owned(), u(10)),
            ("ledger_hash".to_owned(), sv("CCDD")),
        ])),
    );
    let e = r.try_recv().unwrap();
    let _payload_json: JsonValue = serde_json::from_slice(&e.payload).unwrap();
    let JsonValue::Object(p) = _payload_json else {
        panic!("")
    };
    assert_eq!(p.get("type"), Some(&sv("transaction")));
    assert_eq!(p.get("validated"), Some(&b(true)));
    assert_eq!(p.get("engine_result"), Some(&sv("tesSUCCESS")));
    assert_eq!(p.get("engine_result_code"), Some(&si(0)));
    assert_eq!(
        p.get("engine_result_message"),
        Some(&sv("The transaction was applied."))
    );
    assert_eq!(p.get("ledger_index"), Some(&u(10)));
    assert_eq!(p.get("ledger_hash"), Some(&sv("CCDD")));
}
#[test]
fn sub_sign_4() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Server);
    m.publish_json(
        StreamKind::Server,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("serverStatus")),
            ("server_status".to_owned(), sv("full")),
            ("load_base".to_owned(), u(256)),
            ("load_factor".to_owned(), u(256)),
            ("base_fee".to_owned(), u(10)),
            ("reserve_base".to_owned(), u(10000000)),
            ("reserve_inc".to_owned(), u(2000000)),
        ])),
    );
    let e = r.try_recv().unwrap();
    let _payload_json: JsonValue = serde_json::from_slice(&e.payload).unwrap();
    let JsonValue::Object(p) = _payload_json else {
        panic!("")
    };
    assert_eq!(p.get("type"), Some(&sv("serverStatus")));
    assert_eq!(p.get("server_status"), Some(&sv("full")));
    assert_eq!(p.get("load_base"), Some(&u(256)));
    assert_eq!(p.get("load_factor"), Some(&u(256)));
    assert_eq!(p.get("base_fee"), Some(&u(10)));
    assert_eq!(p.get("reserve_base"), Some(&u(10000000)));
    assert_eq!(p.get("reserve_inc"), Some(&u(2000000)));
}
#[test]
fn sub_sign_5() {
    let m = SubscriptionManager::new(8);
    let mut r1 = m.subscribe(StreamKind::Ledger);
    let mut r2 = m.subscribe(StreamKind::Transactions);
    let mut r3 = m.subscribe(StreamKind::Server);
    let mut r4 = m.subscribe(StreamKind::Validations);
    let mut r5 = m.subscribe(StreamKind::Manifests);
    let mut r6 = m.subscribe(StreamKind::Consensus);
    let mut r7 = m.subscribe(StreamKind::PeerStatus);
    let mut r8 = m.subscribe(StreamKind::BookChanges);
    m.publish_json(StreamKind::Ledger, obj());
    assert!(r1.try_recv().is_ok());
    assert!(r2.try_recv().is_err());
    assert!(r3.try_recv().is_err());
    assert!(r4.try_recv().is_err());
    assert!(r5.try_recv().is_err());
    assert!(r6.try_recv().is_err());
    assert!(r7.try_recv().is_err());
    assert!(r8.try_recv().is_err());
}
#[test]
fn sub_sign_6() {
    let m = SubscriptionManager::new(8);
    let mut r1 = m.subscribe(StreamKind::Ledger);
    let mut r2 = m.subscribe(StreamKind::Transactions);
    let mut r3 = m.subscribe(StreamKind::Server);
    let mut r4 = m.subscribe(StreamKind::Validations);
    let mut r5 = m.subscribe(StreamKind::Manifests);
    let mut r6 = m.subscribe(StreamKind::Consensus);
    let mut r7 = m.subscribe(StreamKind::PeerStatus);
    let mut r8 = m.subscribe(StreamKind::BookChanges);
    m.publish_json(StreamKind::Transactions, obj());
    assert!(r1.try_recv().is_err());
    assert!(r2.try_recv().is_ok());
    assert!(r3.try_recv().is_err());
    assert!(r4.try_recv().is_err());
    assert!(r5.try_recv().is_err());
    assert!(r6.try_recv().is_err());
    assert!(r7.try_recv().is_err());
    assert!(r8.try_recv().is_err());
}
#[test]
fn sub_sign_7() {
    let m = SubscriptionManager::new(8);
    let mut r1 = m.subscribe(StreamKind::Ledger);
    let mut r2 = m.subscribe(StreamKind::Transactions);
    let mut r3 = m.subscribe(StreamKind::Server);
    let mut r4 = m.subscribe(StreamKind::Validations);
    let mut r5 = m.subscribe(StreamKind::Manifests);
    let mut r6 = m.subscribe(StreamKind::Consensus);
    let mut r7 = m.subscribe(StreamKind::PeerStatus);
    let mut r8 = m.subscribe(StreamKind::BookChanges);
    m.publish_json(StreamKind::Server, obj());
    assert!(r1.try_recv().is_err());
    assert!(r2.try_recv().is_err());
    assert!(r3.try_recv().is_ok());
    assert!(r4.try_recv().is_err());
    assert!(r5.try_recv().is_err());
    assert!(r6.try_recv().is_err());
    assert!(r7.try_recv().is_err());
    assert!(r8.try_recv().is_err());
}
#[test]
fn sub_sign_8() {
    let m = SubscriptionManager::new(8);
    let mut r1 = m.subscribe(StreamKind::Ledger);
    let mut r2 = m.subscribe(StreamKind::Transactions);
    let mut r3 = m.subscribe(StreamKind::Server);
    let mut r4 = m.subscribe(StreamKind::Validations);
    m.publish_json(StreamKind::Validations, obj());
    assert!(r1.try_recv().is_err());
    assert!(r2.try_recv().is_err());
    assert!(r3.try_recv().is_err());
    assert!(r4.try_recv().is_ok());
}
#[test]
fn sub_sign_9() {
    let m = SubscriptionManager::new(8);
    let mut r5 = m.subscribe(StreamKind::Manifests);
    let mut r6 = m.subscribe(StreamKind::Consensus);
    let mut r7 = m.subscribe(StreamKind::PeerStatus);
    let mut r8 = m.subscribe(StreamKind::BookChanges);
    m.publish_json(StreamKind::Manifests, obj());
    assert!(r5.try_recv().is_ok());
    assert!(r6.try_recv().is_err());
    assert!(r7.try_recv().is_err());
    assert!(r8.try_recv().is_err());
}
#[test]
fn sub_sign_10() {
    let m = SubscriptionManager::new(8);
    let mut r5 = m.subscribe(StreamKind::Manifests);
    let mut r6 = m.subscribe(StreamKind::Consensus);
    let mut r7 = m.subscribe(StreamKind::PeerStatus);
    let mut r8 = m.subscribe(StreamKind::BookChanges);
    m.publish_json(StreamKind::Consensus, obj());
    assert!(r5.try_recv().is_err());
    assert!(r6.try_recv().is_ok());
    assert!(r7.try_recv().is_err());
    assert!(r8.try_recv().is_err());
}
#[test]
fn sub_sign_11() {
    let m = SubscriptionManager::new(8);
    let mut r5 = m.subscribe(StreamKind::Manifests);
    let mut r6 = m.subscribe(StreamKind::Consensus);
    let mut r7 = m.subscribe(StreamKind::PeerStatus);
    let mut r8 = m.subscribe(StreamKind::BookChanges);
    m.publish_json(StreamKind::PeerStatus, obj());
    assert!(r5.try_recv().is_err());
    assert!(r6.try_recv().is_err());
    assert!(r7.try_recv().is_ok());
    assert!(r8.try_recv().is_err());
}
#[test]
fn sub_sign_12() {
    let m = SubscriptionManager::new(8);
    let mut r5 = m.subscribe(StreamKind::Manifests);
    let mut r6 = m.subscribe(StreamKind::Consensus);
    let mut r7 = m.subscribe(StreamKind::PeerStatus);
    let mut r8 = m.subscribe(StreamKind::BookChanges);
    m.publish_json(StreamKind::BookChanges, obj());
    assert!(r5.try_recv().is_err());
    assert!(r6.try_recv().is_err());
    assert!(r7.try_recv().is_err());
    assert!(r8.try_recv().is_ok());
}
#[test]
fn sub_sign_13() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Payment"), Some(&si(0)));
}
#[test]
fn sub_sign_14() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("AccountRoot"), Some(&si(97)));
}
#[test]
fn sub_sign_15() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Offer"), Some(&si(111)));
}
#[test]
fn sub_sign_16() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("RippleState"), Some(&si(114)));
}
#[test]
fn sub_sign_17() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("DirectoryNode"), Some(&si(100)));
}
// More wallet checks
#[test]
fn sub_sign_18() {
    let r = rpc::wallet_propose(&json([(
        "seed_hex",
        sv("BE6A670A19B209E112146D0A7ED2AAD7"),
    )]))
    .unwrap();
    let JsonValue::Object(r) = r else { panic!("") };
    assert!(r.contains_key("account_id"));
    assert!(r.contains_key("master_seed"));
    let JsonValue::String(h) = r.get("master_seed_hex").unwrap() else {
        panic!("")
    };
    assert_eq!(h.to_uppercase(), "BE6A670A19B209E112146D0A7ED2AAD7");
}
#[test]
fn sub_sign_19() {
    let r = rpc::wallet_propose(&json([("seed", sv("snMwVWs2hZzfDUF3p2tHZ3EgmyhFs"))])).unwrap();
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(
        r.get("master_seed"),
        Some(&sv("snMwVWs2hZzfDUF3p2tHZ3EgmyhFs"))
    );
}
#[test]
fn sub_sign_20() {
    let r1 = rpc::wallet_propose(&json([("seed", sv("snMwVWs2hZzfDUF3p2tHZ3EgmyhFs"))])).unwrap();
    let r2 = rpc::wallet_propose(&json([("seed", sv("snMwVWs2hZzfDUF3p2tHZ3EgmyhFs"))])).unwrap();
    let (JsonValue::Object(r1), JsonValue::Object(r2)) = (r1, r2) else {
        panic!("")
    };
    assert_eq!(r1.get("account_id"), r2.get("account_id"));
    assert_eq!(r1.get("public_key"), r2.get("public_key"));
}
// More integration: account_info balance field
#[test]
fn sub_sign_21() {
    let a = TestAccount::new("g21");
    let e = RpcTestEnv::new(&[(&a, 7_777_777_777)]);
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
        if let Some(JsonValue::String(bal)) = d.get("Balance") {
            let v: i64 = bal.parse().unwrap_or(0);
            assert_eq!(v, 7_777_777_777);
        }
    }
}
// More integration: ledger_entry account_root has correct balance
#[test]
fn sub_sign_22() {
    let a = TestAccount::new("g22");
    let e = RpcTestEnv::new(&[(&a, 3_000_000_000)]);
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
    if let Some(JsonValue::Object(node)) = r.get("node") {
        if let Some(JsonValue::String(bal)) = node.get("Balance") {
            let v: i64 = bal.parse().unwrap_or(0);
            assert_eq!(v, 3_000_000_000);
        }
    }
}
// More random uniqueness
#[test]
fn sub_sign_23() {
    let mut set = std::collections::HashSet::new();
    for _ in 0..20 {
        let JsonValue::Object(r) = rpc::do_random() else {
            panic!("")
        };
        let JsonValue::String(v) = r.get("random").unwrap() else {
            panic!("")
        };
        assert!(set.insert(v.clone()));
    }
    assert_eq!(set.len(), 20);
}
// More channel_verify
#[test]
fn sub_sign_24() {
    let r = rpc::do_channel_verify(&json([
        (
            "public_key",
            sv("aB4BXXLuPu8DpVuyq1DBiu3SrPdtK9AYZisKhu8mvkoiUD8J9Gov"),
        ),
        ("channel_id", JsonValue::String("11".repeat(32))),
        ("amount", sv("999")),
        ("signature", JsonValue::String("CC".repeat(64))),
    ]));
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(r.get("signature_verified"), Some(&b(false)));
}
#[test]
fn sub_sign_25() {
    let r = rpc::do_channel_verify(&json([
        ("public_key", u(123)),
        ("channel_id", JsonValue::String("22".repeat(32))),
        ("amount", sv("1")),
        ("signature", sv("DD")),
    ]));
    let JsonValue::Object(r) = r else { panic!("") };
    assert!(r.contains_key("error"));
}
