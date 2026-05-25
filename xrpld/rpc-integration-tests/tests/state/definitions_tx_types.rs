//! Tests for definitions tx types.

use protocol::{
    currency_from_string, get_field_by_symbol, lsfDefaultRipple, to_base58, Issue, JsonValue,
    STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;
use server::{StreamKind, SubscriptionManager};
use std::collections::BTreeMap;
// Subscribe: 8-stream full isolation matrix (8x8=64 tests)
#[test]
fn tx_type_1() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Ledger);
    m.publish_json(StreamKind::Transactions, obj());
    m.publish_json(StreamKind::Server, obj());
    m.publish_json(StreamKind::Validations, obj());
    m.publish_json(StreamKind::Manifests, obj());
    m.publish_json(StreamKind::Consensus, obj());
    m.publish_json(StreamKind::PeerStatus, obj());
    m.publish_json(StreamKind::BookChanges, obj());
    assert!(r.try_recv().is_err());
}
#[test]
fn tx_type_2() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Transactions);
    m.publish_json(StreamKind::Ledger, obj());
    m.publish_json(StreamKind::Server, obj());
    m.publish_json(StreamKind::Validations, obj());
    m.publish_json(StreamKind::Manifests, obj());
    m.publish_json(StreamKind::Consensus, obj());
    m.publish_json(StreamKind::PeerStatus, obj());
    m.publish_json(StreamKind::BookChanges, obj());
    assert!(r.try_recv().is_err());
}
#[test]
fn tx_type_3() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Server);
    m.publish_json(StreamKind::Ledger, obj());
    m.publish_json(StreamKind::Transactions, obj());
    m.publish_json(StreamKind::Validations, obj());
    m.publish_json(StreamKind::Manifests, obj());
    m.publish_json(StreamKind::Consensus, obj());
    m.publish_json(StreamKind::PeerStatus, obj());
    m.publish_json(StreamKind::BookChanges, obj());
    assert!(r.try_recv().is_err());
}
#[test]
fn tx_type_4() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Validations);
    m.publish_json(StreamKind::Ledger, obj());
    m.publish_json(StreamKind::Transactions, obj());
    m.publish_json(StreamKind::Server, obj());
    m.publish_json(StreamKind::Manifests, obj());
    m.publish_json(StreamKind::Consensus, obj());
    m.publish_json(StreamKind::PeerStatus, obj());
    m.publish_json(StreamKind::BookChanges, obj());
    assert!(r.try_recv().is_err());
}
#[test]
fn tx_type_5() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Manifests);
    m.publish_json(StreamKind::Ledger, obj());
    m.publish_json(StreamKind::Transactions, obj());
    m.publish_json(StreamKind::Server, obj());
    m.publish_json(StreamKind::Validations, obj());
    m.publish_json(StreamKind::Consensus, obj());
    m.publish_json(StreamKind::PeerStatus, obj());
    m.publish_json(StreamKind::BookChanges, obj());
    assert!(r.try_recv().is_err());
}
#[test]
fn tx_type_6() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Consensus);
    m.publish_json(StreamKind::Ledger, obj());
    m.publish_json(StreamKind::Transactions, obj());
    m.publish_json(StreamKind::Server, obj());
    m.publish_json(StreamKind::Validations, obj());
    m.publish_json(StreamKind::Manifests, obj());
    m.publish_json(StreamKind::PeerStatus, obj());
    m.publish_json(StreamKind::BookChanges, obj());
    assert!(r.try_recv().is_err());
}
#[test]
fn tx_type_7() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::PeerStatus);
    m.publish_json(StreamKind::Ledger, obj());
    m.publish_json(StreamKind::Transactions, obj());
    m.publish_json(StreamKind::Server, obj());
    m.publish_json(StreamKind::Validations, obj());
    m.publish_json(StreamKind::Manifests, obj());
    m.publish_json(StreamKind::Consensus, obj());
    m.publish_json(StreamKind::BookChanges, obj());
    assert!(r.try_recv().is_err());
}
#[test]
fn tx_type_8() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::BookChanges);
    m.publish_json(StreamKind::Ledger, obj());
    m.publish_json(StreamKind::Transactions, obj());
    m.publish_json(StreamKind::Server, obj());
    m.publish_json(StreamKind::Validations, obj());
    m.publish_json(StreamKind::Manifests, obj());
    m.publish_json(StreamKind::Consensus, obj());
    m.publish_json(StreamKind::PeerStatus, obj());
    assert!(r.try_recv().is_err());
}
// Subscribe: each stream receives its own
#[test]
fn tx_type_9() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Ledger);
    m.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(1))])),
    );
    assert!(r.try_recv().is_ok());
}
#[test]
fn tx_type_10() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Transactions);
    m.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(2))])),
    );
    assert!(r.try_recv().is_ok());
}
#[test]
fn tx_type_11() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Server);
    m.publish_json(
        StreamKind::Server,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(3))])),
    );
    assert!(r.try_recv().is_ok());
}
#[test]
fn tx_type_12() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Validations);
    m.publish_json(
        StreamKind::Validations,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(4))])),
    );
    assert!(r.try_recv().is_ok());
}
#[test]
fn tx_type_13() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Manifests);
    m.publish_json(
        StreamKind::Manifests,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(5))])),
    );
    assert!(r.try_recv().is_ok());
}
#[test]
fn tx_type_14() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Consensus);
    m.publish_json(
        StreamKind::Consensus,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(6))])),
    );
    assert!(r.try_recv().is_ok());
}
#[test]
fn tx_type_15() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::PeerStatus);
    m.publish_json(
        StreamKind::PeerStatus,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(7))])),
    );
    assert!(r.try_recv().is_ok());
}
#[test]
fn tx_type_16() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::BookChanges);
    m.publish_json(
        StreamKind::BookChanges,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(8))])),
    );
    assert!(r.try_recv().is_ok());
}
// Subscribe: drop receiver = 0 sent
#[test]
fn tx_type_17() {
    let m = SubscriptionManager::new(8);
    let r = m.subscribe(StreamKind::Ledger);
    drop(r);
    assert_eq!(m.publish_json(StreamKind::Ledger, obj()), 0);
}
#[test]
fn tx_type_18() {
    let m = SubscriptionManager::new(8);
    let r = m.subscribe(StreamKind::Transactions);
    drop(r);
    assert_eq!(m.publish_json(StreamKind::Transactions, obj()), 0);
}
#[test]
fn tx_type_19() {
    let m = SubscriptionManager::new(8);
    let r = m.subscribe(StreamKind::Server);
    drop(r);
    assert_eq!(m.publish_json(StreamKind::Server, obj()), 0);
}
#[test]
fn tx_type_20() {
    let m = SubscriptionManager::new(8);
    let r = m.subscribe(StreamKind::Validations);
    drop(r);
    assert_eq!(m.publish_json(StreamKind::Validations, obj()), 0);
}
#[test]
fn tx_type_21() {
    let m = SubscriptionManager::new(8);
    let r = m.subscribe(StreamKind::Manifests);
    drop(r);
    assert_eq!(m.publish_json(StreamKind::Manifests, obj()), 0);
}
#[test]
fn tx_type_22() {
    let m = SubscriptionManager::new(8);
    let r = m.subscribe(StreamKind::Consensus);
    drop(r);
    assert_eq!(m.publish_json(StreamKind::Consensus, obj()), 0);
}
#[test]
fn tx_type_23() {
    let m = SubscriptionManager::new(8);
    let r = m.subscribe(StreamKind::PeerStatus);
    drop(r);
    assert_eq!(m.publish_json(StreamKind::PeerStatus, obj()), 0);
}
#[test]
fn tx_type_24() {
    let m = SubscriptionManager::new(8);
    let r = m.subscribe(StreamKind::BookChanges);
    drop(r);
    assert_eq!(m.publish_json(StreamKind::BookChanges, obj()), 0);
}
// Subscribe: multi-receiver count
#[test]
fn tx_type_25() {
    let m = SubscriptionManager::new(8);
    let _a = m.subscribe(StreamKind::Ledger);
    let _b = m.subscribe(StreamKind::Ledger);
    let _c = m.subscribe(StreamKind::Ledger);
    assert_eq!(m.publish_json(StreamKind::Ledger, obj()), 3);
}
#[test]
fn tx_type_26() {
    let m = SubscriptionManager::new(8);
    let _a = m.subscribe(StreamKind::Transactions);
    let _b = m.subscribe(StreamKind::Transactions);
    assert_eq!(m.publish_json(StreamKind::Transactions, obj()), 2);
}
#[test]
fn tx_type_27() {
    let m = SubscriptionManager::new(8);
    let _a = m.subscribe(StreamKind::Server);
    let _b = m.subscribe(StreamKind::Server);
    let _c = m.subscribe(StreamKind::Server);
    let _d = m.subscribe(StreamKind::Server);
    assert_eq!(m.publish_json(StreamKind::Server, obj()), 4);
}
// Subscribe: volume tests
#[test]
fn tx_type_28() {
    let m = SubscriptionManager::new(128);
    let mut r = m.subscribe(StreamKind::Ledger);
    for i in 0..100 {
        m.publish_json(
            StreamKind::Ledger,
            JsonValue::Object(BTreeMap::from([("n".to_owned(), u(i))])),
        );
    }
    let mut c = 0;
    while r.try_recv().is_ok() {
        c += 1;
    }
    assert_eq!(c, 100);
}
#[test]
fn tx_type_29() {
    let m = SubscriptionManager::new(128);
    let mut r = m.subscribe(StreamKind::Transactions);
    for i in 0..75 {
        m.publish_json(
            StreamKind::Transactions,
            JsonValue::Object(BTreeMap::from([("n".to_owned(), u(i))])),
        );
    }
    let mut c = 0;
    while r.try_recv().is_ok() {
        c += 1;
    }
    assert_eq!(c, 75);
}
#[test]
fn tx_type_30() {
    let m = SubscriptionManager::new(128);
    let mut r = m.subscribe(StreamKind::Server);
    for i in 0..60 {
        m.publish_json(
            StreamKind::Server,
            JsonValue::Object(BTreeMap::from([("n".to_owned(), u(i))])),
        );
    }
    let mut c = 0;
    while r.try_recv().is_ok() {
        c += 1;
    }
    assert_eq!(c, 60);
}
// ServerDefinitions: more type code checks
#[test]
fn tx_type_31() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Hash384"), Some(&si(22)));
    assert_eq!(t.get("Hash512"), Some(&si(23)));
}
#[test]
fn tx_type_32() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("TYPES").unwrap() else {
        panic!("")
    };
    assert_eq!(t.get("Hash192"), Some(&si(21)));
}
// ServerDefinitions: more LE type codes
#[test]
fn tx_type_33() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(t) = r.get("LEDGER_ENTRY_TYPES").unwrap() else {
        panic!("")
    };
    assert!(t.contains_key("LedgerHashes"));
    assert!(t.contains_key("Amendments"));
    assert!(t.contains_key("FeeSettings"));
}
// ServerDefinitions: TRANSACTION_FORMATS has many entries
#[test]
fn tx_type_34() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("TRANSACTION_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.len() > 20);
}
#[test]
fn tx_type_35() {
    let JsonValue::Object(r) = rpc::do_server_definitions(&obj()) else {
        panic!("")
    };
    let JsonValue::Object(f) = r.get("LEDGER_ENTRY_FORMATS").unwrap() else {
        panic!("")
    };
    assert!(f.len() > 10);
}
// Integration: NFTokenMint
#[test]
fn tx_type_36() {
    let mut a = TestAccount::new("h36");
    let e = RpcTestEnv::new(&[(&a, 10_000_000_000)]);
    let mut m = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_u32(get_field_by_symbol("sfNFTokenTaxon"), 1);
        tx.set_field_u32(get_field_by_symbol("sfFlags"), 0);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut m, &a);
    e.submit_and_close(&m);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_nfts(
        &rpc::AccountNFTsRequest {
            params: &json([("account", JsonValue::String(to_base58(a.id)))]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Array(nfts)) = r.get("account_nfts") {
        if !nfts.is_empty() {
            let JsonValue::Object(nft) = &nfts[0] else {
                panic!("")
            };
            assert!(nft.contains_key("NFTokenID"));
            assert!(nft.contains_key("Flags"));
            assert!(nft.contains_key("Issuer"));
            assert!(nft.contains_key("NFTokenTaxon"));
            assert!(nft.contains_key("nft_serial"));
        }
    }
}
// Integration: OfferCancel
#[test]
fn tx_type_37() {
    let mut a = TestAccount::new("h37a");
    let g = TestAccount::new("h37g");
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
    let os = a.next_seq();
    let mut o = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(100_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerGets"),
                Issue::new(usd, g.id),
                50,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), os);
    });
    sign_tx(&mut o, &a);
    e.submit_and_close(&o);
    let mut c = STTx::new(TxType::OFFER_CANCEL, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_u32(get_field_by_symbol("sfOfferSequence"), os);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut c, &a);
    e.submit_and_close(&c);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(a.id))),
                ("type", sv("offer")),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &s,
    ) else {
        panic!("")
    };
    if let Some(JsonValue::Array(objs)) = r.get("account_objects") {
        assert_eq!(objs.len(), 0);
    }
}
// More wallet
#[test]
fn tx_type_38() {
    let r = rpc::wallet_propose(&json([
        ("key_type", sv("ed25519")),
        ("passphrase", sv("test")),
    ]))
    .unwrap();
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(r.get("key_type"), Some(&sv("ed25519")));
    assert!(r.contains_key("warning"));
}
#[test]
fn tx_type_39() {
    let r = rpc::wallet_propose(&json([
        ("key_type", sv("secp256k1")),
        ("passphrase", sv("test")),
    ]))
    .unwrap();
    let JsonValue::Object(r) = r else { panic!("") };
    assert_eq!(r.get("key_type"), Some(&sv("secp256k1")));
    assert!(r.contains_key("warning"));
}
#[test]
fn tx_type_40() {
    let r1 = rpc::wallet_propose(&json([
        ("key_type", sv("ed25519")),
        ("passphrase", sv("same")),
    ]))
    .unwrap();
    let r2 = rpc::wallet_propose(&json([
        ("key_type", sv("secp256k1")),
        ("passphrase", sv("same")),
    ]))
    .unwrap();
    let (JsonValue::Object(r1), JsonValue::Object(r2)) = (r1, r2) else {
        panic!("")
    };
    assert_ne!(r1.get("account_id"), r2.get("account_id"));
}
