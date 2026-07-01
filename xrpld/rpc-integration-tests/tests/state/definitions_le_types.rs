//! Tests for definitions le types.

use protocol::{get_field_by_symbol, to_base58, JsonValue, STAmount, STTx, TxType};
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;
use server::{StreamKind, SubscriptionManager};
use std::collections::BTreeMap;
#[test]
fn ledger_entry_type_1() {
    let a = TestAccount::new("i01a");
    let b2 = TestAccount::new("i01b");
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
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 99);
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
            if let Some(JsonValue::String(er)) = o.get("engine_result") {
                assert!(er.starts_with("te") || er.starts_with("tef"));
            }
        }
        Err(_) => {}
        _ => {}
    }
}
#[test]
fn ledger_entry_type_2() {
    let mut a = TestAccount::new("i02a");
    let b2 = TestAccount::new("i02b");
    let e = RpcTestEnv::new(&[(&a, 100_000), (&b2, 100_000)]);
    let mut p = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), b2.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(999_999_999, false),
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
            if let Some(JsonValue::String(er)) = o.get("engine_result") {
                assert!(er.starts_with("te") || er.starts_with("tec"));
            }
        }
        Err(_) => {}
        _ => {}
    }
}
#[test]
fn ledger_entry_type_3() {
    let m = SubscriptionManager::new(16);
    let mut r = m.subscribe(StreamKind::Ledger);
    let payload = JsonValue::Object(BTreeMap::from([
        ("type".to_owned(), sv("ledgerClosed")),
        ("ledger_index".to_owned(), u(999)),
        ("ledger_hash".to_owned(), sv("ABCDEF")),
        ("txn_count".to_owned(), u(42)),
        ("fee_base".to_owned(), u(10)),
        ("fee_ref".to_owned(), u(10)),
        ("reserve_base".to_owned(), u(10000000)),
        ("reserve_inc".to_owned(), u(2000000)),
        ("validated_ledgers".to_owned(), sv("1-999")),
        ("ledger_time".to_owned(), u(750000000)),
    ]));
    m.publish_json(StreamKind::Ledger, payload.clone());
    let e = r.try_recv().unwrap();
    { let parsed: JsonValue = serde_json::from_slice(&e.payload).unwrap(); assert_eq!(parsed, payload); }
}
#[test]
fn ledger_entry_type_4() {
    let m = SubscriptionManager::new(16);
    let mut r = m.subscribe(StreamKind::Transactions);
    let payload = JsonValue::Object(BTreeMap::from([
        ("type".to_owned(), sv("transaction")),
        ("validated".to_owned(), b(true)),
        ("engine_result".to_owned(), sv("tesSUCCESS")),
        ("engine_result_code".to_owned(), si(0)),
        (
            "engine_result_message".to_owned(),
            sv("The transaction was applied."),
        ),
        ("ledger_index".to_owned(), u(50)),
        ("ledger_hash".to_owned(), sv("AABB")),
        ("close_time_iso".to_owned(), sv("2024-01-01T00:00:00Z")),
    ]));
    m.publish_json(StreamKind::Transactions, payload.clone());
    let e = r.try_recv().unwrap();
    { let parsed: JsonValue = serde_json::from_slice(&e.payload).unwrap(); assert_eq!(parsed, payload); }
}
#[test]
fn ledger_entry_type_5() {
    let m = SubscriptionManager::new(16);
    let mut r = m.subscribe(StreamKind::Server);
    let payload = JsonValue::Object(BTreeMap::from([
        ("type".to_owned(), sv("serverStatus")),
        ("server_status".to_owned(), sv("full")),
        ("load_base".to_owned(), u(256)),
        ("load_factor".to_owned(), u(256)),
        ("base_fee".to_owned(), u(10)),
        ("reserve_base".to_owned(), u(10000000)),
        ("reserve_inc".to_owned(), u(2000000)),
        ("validated_ledgers".to_owned(), sv("1-100")),
    ]));
    m.publish_json(StreamKind::Server, payload.clone());
    let e = r.try_recv().unwrap();
    { let parsed: JsonValue = serde_json::from_slice(&e.payload).unwrap(); assert_eq!(parsed, payload); }
}
#[test]
fn ledger_entry_type_6() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    for (name, fields) in f {
        let JsonValue::Array(fields) = fields else {
            continue;
        };
        for field in fields {
            let JsonValue::Object(fo) = field else {
                panic!("{name} bad field")
            };
            assert!(fo.contains_key("name"), "{name} field missing name");
            assert!(
                fo.contains_key("optionality"),
                "{name} field missing optionality"
            );
        }
    }
}
#[test]
fn ledger_entry_type_7() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    for (name, fields) in f {
        let JsonValue::Array(fields) = fields else {
            continue;
        };
        for field in fields {
            let JsonValue::Object(fo) = field else {
                panic!("{name} bad field")
            };
            assert!(fo.contains_key("name"), "{name} field missing name");
            assert!(
                fo.contains_key("optionality"),
                "{name} field missing optionality"
            );
        }
    }
}
#[test]
fn ledger_entry_type_8() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Array(f) = r.get("FIELDS").unwrap() else {
        panic!("")
    };
    for entry in f.iter().take(50) {
        let JsonValue::Array(e) = entry else {
            panic!("")
        };
        assert_eq!(e.len(), 2);
        let JsonValue::String(_) = &e[0] else {
            panic!("")
        };
        let JsonValue::Object(props) = &e[1] else {
            panic!("")
        };
        assert!(props.contains_key("nth"));
        assert!(props.contains_key("isVLEncoded"));
        assert!(props.contains_key("isSerialized"));
        assert!(props.contains_key("isSigningField"));
        assert!(props.contains_key("type"));
    }
}
#[test]
fn ledger_entry_type_9() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FLAGS").unwrap() else {
        panic!("")
    };
    for (le_name, flags) in f {
        let JsonValue::Object(flags) = flags else {
            panic!("{le_name}")
        };
        for (flag_name, val) in flags {
            assert!(
                matches!(val, JsonValue::Unsigned(_) | JsonValue::Signed(_)),
                "{le_name}.{flag_name} not number"
            );
        }
    }
}
#[test]
fn ledger_entry_type_10() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FLAGS").unwrap() else {
        panic!("")
    };
    for (tx_name, flags) in f {
        let JsonValue::Object(flags) = flags else {
            panic!("{tx_name}")
        };
        for (flag_name, val) in flags {
            assert!(
                matches!(val, JsonValue::Unsigned(_) | JsonValue::Signed(_)),
                "{tx_name}.{flag_name} not number"
            );
        }
    }
}
#[test]
fn ledger_entry_type_11() {
    let a = TestAccount::new("i11");
    let e = RpcTestEnv::new(&[(&a, 9_876_543_210)]);
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
            assert_eq!(bal, "9876543210");
        }
    }
}
#[test]
fn ledger_entry_type_12() {
    let a = TestAccount::new("i12");
    let e = RpcTestEnv::new(&[(&a, 1_000_000)]);
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
            assert_eq!(bal, "1000000");
        }
    }
}
#[test]
fn ledger_entry_type_13() {
    let a = TestAccount::new("i13");
    let e = RpcTestEnv::new(&[(&a, 5_555_555_555)]);
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
        if let Some(JsonValue::String(bal)) = n.get("Balance") {
            assert_eq!(bal, "5555555555");
        }
    }
}
#[test]
fn ledger_entry_type_14() {
    let a = TestAccount::new("i14");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_lines(
        &rpc::AccountLinesRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("account"), Some(&JsonValue::String(to_base58(a.id))));
}
#[test]
fn ledger_entry_type_15() {
    let a = TestAccount::new("i15");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
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
    assert_eq!(r.get("account"), Some(&JsonValue::String(to_base58(a.id))));
}
#[test]
fn ledger_entry_type_16() {
    let a = TestAccount::new("i16");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_offers(
        &rpc::AccountOffersRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("account"), Some(&JsonValue::String(to_base58(a.id))));
}
#[test]
fn ledger_entry_type_17() {
    let a = TestAccount::new("i17");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_gateway_balances(
        &rpc::GatewayBalancesRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 2,
            role: rpc::RpcRole::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert_eq!(r.get("account"), Some(&JsonValue::String(to_base58(a.id))));
    }
}
#[test]
fn ledger_entry_type_18() {
    let a = TestAccount::new("i18a");
    let b2 = TestAccount::new("i18b");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000), (&b2, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_deposit_authorized(
        &rpc::DepositAuthorizedRequest {
            params: &json([
                ("source_account", JsonValue::String(to_base58(a.id))),
                ("destination_account", JsonValue::String(to_base58(b2.id))),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert_eq!(r.get("deposit_authorized"), Some(&b(true)));
        assert_eq!(
            r.get("source_account"),
            Some(&JsonValue::String(to_base58(a.id)))
        );
        assert_eq!(
            r.get("destination_account"),
            Some(&JsonValue::String(to_base58(b2.id)))
        );
    }
}
#[test]
fn ledger_entry_type_19() {
    let a = TestAccount::new("i19");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_no_ripple_check(
        &rpc::NoRippleCheckRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("role", sv("gateway")),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("problems"));
        let JsonValue::Array(p) = r.get("problems").unwrap() else {
            panic!("")
        };
        assert!(!p.is_empty());
    }
}
#[test]
fn ledger_entry_type_20() {
    let a = TestAccount::new("i20");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_no_ripple_check(
        &rpc::NoRippleCheckRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("role", sv("user")),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if r.get("error").is_none() {
        assert!(r.contains_key("problems"));
    }
}
// More random
#[test]
fn ledger_entry_type_21() {
    let mut prev = String::new();
    for _ in 0..25 {
        let JsonValue::Object(r) = rpc::do_random() else {
            panic!("")
        };
        let JsonValue::String(v) = r.get("random").unwrap() else {
            panic!("")
        };
        assert_ne!(*v, prev);
        prev = v.clone();
    }
}
// More wallet
#[test]
fn ledger_entry_type_22() {
    for _ in 0..5 {
        let r = rpc::wallet_propose(&json([])).unwrap();
        let JsonValue::Object(r) = r else { panic!("") };
        assert!(r.contains_key("account_id"));
        assert!(r.contains_key("master_seed"));
        assert!(r.contains_key("public_key"));
        assert!(!r.contains_key("error"));
    }
}
#[test]
fn ledger_entry_type_23() {
    let a = TestAccount::new("i23");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::feature::do_feature(
        &rpc::feature::FeatureRequest {
            params: &obj(),
            role: rpc::RpcRole::Admin,
        },
        &s,
    );
    let JsonValue::Object(r) = r else { panic!("") };
    let JsonValue::Object(feats) = r.get("features").unwrap() else {
        panic!("")
    };
    assert!(!feats.is_empty());
    for (_k, v) in feats.iter().take(5) {
        let JsonValue::Object(f) = v else { continue };
        assert!(f.contains_key("name"));
        assert!(f.contains_key("enabled"));
        assert!(f.contains_key("supported"));
    }
}
#[test]
fn ledger_entry_type_24() {
    let a = TestAccount::new("i24");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_closed(&s) else {
        panic!("")
    };
    let JsonValue::String(h) = r.get("ledger_hash").unwrap() else {
        panic!("")
    };
    assert_eq!(h.len(), 64);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
}
#[test]
fn ledger_entry_type_25() {
    let a = TestAccount::new("i25");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_ledger_current(&s) else {
        panic!("")
    };
    let JsonValue::Unsigned(i) = r.get("ledger_current_index").unwrap() else {
        panic!("")
    };
    assert!(*i >= 2);
}
