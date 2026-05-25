//! Tests for definitions nft types.

use protocol::{to_base58, JsonValue};
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;
use server::{StreamKind, SubscriptionManager};
use std::collections::BTreeMap;
#[test]
fn nft_type_code_1() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TRANSACTION_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("AccountDelete"), Some(&si(21)));
    assert_eq!(t.get("EnableAmendment"), Some(&si(100)));
    assert_eq!(t.get("SetFee"), Some(&si(101)));
}
#[test]
fn nft_type_code_2() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Amendments"), Some(&si(102)));
    assert_eq!(t.get("FeeSettings"), Some(&si(115)));
}
#[test]
fn nft_type_code_3() {
    let a = TestAccount::new("n03");
    let e = RpcTestEnv::new(&[(&a, 3_141_592_653)]);
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
        assert_eq!(d.get("Balance"), Some(&sv("3141592653")));
        assert_eq!(d.get("Sequence"), Some(&u(1)));
        assert_eq!(d.get("Flags"), Some(&u(0)));
    }
}
#[test]
fn nft_type_code_4() {
    let a = TestAccount::new("n04");
    let e = RpcTestEnv::new(&[(&a, 2_718_281_828)]);
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
        assert_eq!(n.get("Balance"), Some(&sv("2718281828")));
        assert_eq!(n.get("Sequence"), Some(&u(1)));
    }
}
#[test]
fn nft_type_code_5() {
    let m = SubscriptionManager::new(16);
    let mut r1 = m.subscribe(StreamKind::Ledger);
    let mut r2 = m.subscribe(StreamKind::Ledger);
    let mut r3 = m.subscribe(StreamKind::Ledger);
    let p = JsonValue::Object(BTreeMap::from([("v".to_owned(), u(42))]));
    m.publish_json(StreamKind::Ledger, p.clone());
    assert_eq!(r1.try_recv().unwrap().payload, p);
    assert_eq!(r2.try_recv().unwrap().payload, p);
    assert_eq!(r3.try_recv().unwrap().payload, p);
}
#[test]
fn nft_type_code_6() {
    let m = SubscriptionManager::new(16);
    let mut r = m.subscribe(StreamKind::Transactions);
    for i in 0..8u64 {
        m.publish_json(
            StreamKind::Transactions,
            JsonValue::Object(BTreeMap::from([("i".to_owned(), u(i))])),
        );
    }
    let mut c = 0;
    while let Ok(e) = r.try_recv() {
        let JsonValue::Object(p) = e.payload else {
            panic!("")
        };
        assert_eq!(p.get("i"), Some(&u(c)));
        c += 1;
    }
    assert_eq!(c, 8);
}
#[test]
fn nft_type_code_7() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Array(f) = r.get("FIELDS").unwrap() else {
        panic!("")
    };
    let names: Vec<&str> = f
        .iter()
        .filter_map(|e| {
            if let JsonValue::Array(a) = e {
                if let JsonValue::String(n) = &a[0] {
                    Some(n.as_str())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();
    assert!(names.contains(&"Account"));
    assert!(names.contains(&"Destination"));
    assert!(names.contains(&"Amount"));
    assert!(names.contains(&"Fee"));
    assert!(names.contains(&"Sequence"));
    assert!(names.contains(&"SigningPubKey"));
    assert!(names.contains(&"TxnSignature"));
}
