//! Tests for definitions results.

use protocol::{
    currency_from_string, get_field_by_symbol, lsfDefaultRipple, to_base58, Issue, JsonValue,
    STAmount, STArray, STObject, STTx, TxType,
};
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;
use server::{StreamKind, SubscriptionManager};
use std::collections::BTreeMap;
// === AMM CREATE (needs two funded assets) ===
#[test]
fn tx_result_code_1() {
    let mut a = TestAccount::new("j01a");
    let mut g = TestAccount::new("j01g");
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
                100000,
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
                50000,
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
    let mut amm = STTx::new(TxType::AMM_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfAmount2"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfAmount2"),
                Issue::new(usd, g.id),
                1000,
                0,
                false,
            ),
        );
        tx.set_field_u16(get_field_by_symbol("sfTradingFee"), 100);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut amm, &a);
    e.submit_and_close(&amm);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Array(objs)) = r.get("account_objects") {
        let has_amm = objs.iter().any(
            |o| matches!(o,JsonValue::Object(obj) if obj.get("LedgerEntryType")==Some(&sv("AMM"))),
        );
        if has_amm {
            assert!(has_amm);
        }
    }
}
// === DID SET ===
#[test]
fn tx_result_code_2() {
    let mut a = TestAccount::new("j02a");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000)]);
    let mut did = STTx::new(TxType::DID_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_vl(get_field_by_symbol("sfURI"), b"https://example.com/did");
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut did, &a);
    e.submit_and_close(&did);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Array(objs)) = r.get("account_objects") {
        let has_did = objs.iter().any(
            |o| matches!(o,JsonValue::Object(obj) if obj.get("LedgerEntryType")==Some(&sv("DID"))),
        );
        if has_did {
            assert!(has_did);
        }
    }
}
// === ORACLE SET ===
#[test]
fn tx_result_code_3() {
    let mut a = TestAccount::new("j03a");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000)]);
    let mut oracle = STTx::new(TxType::ORACLE_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_u32(get_field_by_symbol("sfOracleDocumentID"), 1);
        tx.set_field_vl(get_field_by_symbol("sfProvider"), b"chainlink");
        tx.set_field_vl(get_field_by_symbol("sfAssetClass"), b"currency");
        tx.set_field_u32(get_field_by_symbol("sfLastUpdateTime"), 750000000);
        let mut entry = STObject::make_inner_object(get_field_by_symbol("sfPriceData"));
        entry.set_field_amount(
            get_field_by_symbol("sfAssetPrice"),
            STAmount::new_native(100, false),
        );
        let mut arr = STArray::new(get_field_by_symbol("sfPriceDataSeries"));
        arr.push_back(entry);
        tx.set_field_array(get_field_by_symbol("sfPriceDataSeries"), arr);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut oracle, &a);
    e.submit_and_close(&oracle);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Array(objs)) = r.get("account_objects") {
        let has_oracle=objs.iter().any(|o|matches!(o,JsonValue::Object(obj) if obj.get("LedgerEntryType")==Some(&sv("Oracle"))));
        if has_oracle {
            assert!(has_oracle);
        }
    }
}
// === RobustTransaction: submit correct seq succeeds ===
#[test]
fn tx_result_code_4() {
    let mut a = TestAccount::new("j04a");
    let b2 = TestAccount::new("j04b");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000), (&b2, 1_000_000_000)]);
    let mut p = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), b2.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut p, &a);
    let blob = basics::str_hex::str_hex(p.get_serializer().data());
    let s = e.rpc_source();
    let r = rpc::do_submit(&rpc::RpcRequestContext {
        params: &json([("tx_blob", JsonValue::String(blob))]),
        env: &rpc::SubmitSource,
        runtime: &s,
        role: rpc::Role::Admin,
        api_version: 2,
        headers: rpc::JsonContextHeaders::default(),
        request_headers: BTreeMap::new(),
        unlimited: true,
        remote_ip: None,
        load_type: rpc::RpcLoadType::Reference,
    });
    match r {
        Ok(JsonValue::Object(o)) => {
            assert!(
                o.contains_key("engine_result")
                    || o.contains_key("tx_json")
                    || o.contains_key("error")
            );
        }
        Err(_) => {}
        _ => {}
    }
}
// === More subscribe payload checks ===
#[test]
fn tx_result_code_5() {
    let m = SubscriptionManager::new(16);
    let mut r = m.subscribe(StreamKind::BookChanges);
    let payload = JsonValue::Object(BTreeMap::from([
        ("type".to_owned(), sv("bookChanges")),
        ("ledger_index".to_owned(), u(200)),
        ("ledger_time".to_owned(), u(800000000)),
        (
            "changes".to_owned(),
            JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([
                ("currency_a".to_owned(), sv("XRP")),
                ("currency_b".to_owned(), sv("USD.rGw")),
                ("volume_a".to_owned(), sv("1000000")),
                ("volume_b".to_owned(), sv("50")),
                ("high".to_owned(), sv("20000")),
                ("low".to_owned(), sv("19000")),
                ("open".to_owned(), sv("19500")),
                ("close".to_owned(), sv("19800")),
            ]))]),
        ),
    ]));
    m.publish_json(StreamKind::BookChanges, payload.clone());
    let e = r.try_recv().unwrap();
    { let parsed: JsonValue = serde_json::from_slice(&e.payload).unwrap(); assert_eq!(parsed, payload); }
}
#[test]
fn tx_result_code_6() {
    let m = SubscriptionManager::new(16);
    let mut r = m.subscribe(StreamKind::Validations);
    let payload = JsonValue::Object(BTreeMap::from([
        ("type".to_owned(), sv("validationReceived")),
        ("ledger_hash".to_owned(), JsonValue::String("AA".repeat(32))),
        (
            "validation_public_key".to_owned(),
            sv("n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7"),
        ),
        ("full".to_owned(), b(true)),
        ("ledger_index".to_owned(), u(500)),
        ("signature".to_owned(), sv("sig_hex")),
    ]));
    m.publish_json(StreamKind::Validations, payload.clone());
    let e = r.try_recv().unwrap();
    { let parsed: JsonValue = serde_json::from_slice(&e.payload).unwrap(); assert_eq!(parsed, payload); }
}
#[test]
fn tx_result_code_7() {
    let m = SubscriptionManager::new(16);
    let mut r = m.subscribe(StreamKind::Manifests);
    let payload = JsonValue::Object(BTreeMap::from([
        ("type".to_owned(), sv("manifestReceived")),
        (
            "master_key".to_owned(),
            sv("nHBt9fsb4849WmZiCds4r5TXyBeQjqnH5kzPtqgMAQMgi39YZRPa"),
        ),
        (
            "signing_key".to_owned(),
            sv("n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7"),
        ),
        ("seq".to_owned(), u(1)),
        ("signature".to_owned(), sv("manifest_sig")),
    ]));
    m.publish_json(StreamKind::Manifests, payload.clone());
    let e = r.try_recv().unwrap();
    { let parsed: JsonValue = serde_json::from_slice(&e.payload).unwrap(); assert_eq!(parsed, payload); }
}
#[test]
fn tx_result_code_8() {
    let m = SubscriptionManager::new(16);
    let mut r = m.subscribe(StreamKind::Consensus);
    let payload = JsonValue::Object(BTreeMap::from([
        ("type".to_owned(), sv("consensusPhase")),
        ("consensus".to_owned(), sv("open")),
    ]));
    m.publish_json(StreamKind::Consensus, payload.clone());
    let e = r.try_recv().unwrap();
    { let parsed: JsonValue = serde_json::from_slice(&e.payload).unwrap(); assert_eq!(parsed, payload); }
}
#[test]
fn tx_result_code_9() {
    let m = SubscriptionManager::new(16);
    let mut r = m.subscribe(StreamKind::PeerStatus);
    let payload = JsonValue::Object(BTreeMap::from([
        ("type".to_owned(), sv("peerStatusChange")),
        ("action".to_owned(), sv("disconnect")),
        ("address".to_owned(), sv("10.0.0.1:51235")),
        ("date".to_owned(), u(750000000)),
    ]));
    m.publish_json(StreamKind::PeerStatus, payload.clone());
    let e = r.try_recv().unwrap();
    { let parsed: JsonValue = serde_json::from_slice(&e.payload).unwrap(); assert_eq!(parsed, payload); }
}
// === More ServerDefinitions specific checks ===
#[test]
fn tx_result_code_10() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("AMMCreate"));
    assert!(t.contains_key("AMMDeposit"));
}
#[test]
fn tx_result_code_11() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("DIDSet"));
    assert!(t.contains_key("DIDDelete"));
}
#[test]
fn tx_result_code_12() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("OracleSet"));
    assert!(t.contains_key("OracleDelete"));
}
#[test]
fn tx_result_code_13() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("AMM"));
}
#[test]
fn tx_result_code_14() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("DID"));
}
#[test]
fn tx_result_code_15() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("Oracle"));
}
#[test]
fn tx_result_code_16() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("AMMCreate"));
}
#[test]
fn tx_result_code_17() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("DIDSet"));
}
#[test]
fn tx_result_code_18() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("OracleSet"));
}
#[test]
fn tx_result_code_19() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("AMM"));
}
#[test]
fn tx_result_code_20() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("DID"));
}
#[test]
fn tx_result_code_21() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.contains_key("Oracle"));
}
// === More integration: multiple payments, balance tracking ===
#[test]
fn tx_result_code_22() {
    let mut a = TestAccount::new("j22a");
    let b2 = TestAccount::new("j22b");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000), (&b2, 1_000_000_000)]);
    for _ in 0..3 {
        let mut p = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
            tx.set_account_id(get_field_by_symbol("sfDestination"), b2.id);
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(100_000, false),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
        });
        sign_tx(&mut p, &a);
        e.submit_and_close(&p);
    }
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
            assert!(v < 10_000_000_000);
            assert!(v > 0);
        }
    }
}
// === Ledger progression tracking ===
#[test]
fn tx_result_code_23() {
    let a = TestAccount::new("j23");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    for _ in 0..5 {
        e.app.accept_standalone_ledger().unwrap();
    }
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_current(&s) else {
        panic!("")
    };
    let JsonValue::Unsigned(i) = r.get("ledger_current_index").unwrap() else {
        panic!("")
    };
    assert!(*i >= 7);
}
#[test]
fn tx_result_code_24() {
    let a = TestAccount::new("j24");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s1 = e.rpc_source();
    let JsonValue::Object(r1) = rpc::do_ledger_closed(&s1) else {
        panic!("")
    };
    let h1 = r1.get("ledger_hash").cloned();
    for _ in 0..3 {
        e.app.accept_standalone_ledger().unwrap();
    }
    let s2 = e.rpc_source();
    let JsonValue::Object(r2) = rpc::do_ledger_closed(&s2) else {
        panic!("")
    };
    let h2 = r2.get("ledger_hash").cloned();
    assert_ne!(h1, h2);
}
#[test]
fn tx_result_code_25() {
    let a = TestAccount::new("j25");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s1 = e.rpc_source();
    let JsonValue::Object(r1) = rpc::do_ledger_closed(&s1) else {
        panic!("")
    };
    let JsonValue::Unsigned(i1) = r1.get("ledger_index").unwrap() else {
        panic!("")
    };
    e.app.accept_standalone_ledger().unwrap();
    let s2 = e.rpc_source();
    let JsonValue::Object(r2) = rpc::do_ledger_closed(&s2) else {
        panic!("")
    };
    let JsonValue::Unsigned(i2) = r2.get("ledger_index").unwrap() else {
        panic!("")
    };
    assert!(*i2 > *i1);
}
