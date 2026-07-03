//! publish receive tests part A.

use super::*;

#[test]
fn server_definitions_sub_ledger() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Ledger);
    m.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(1))])),
    );
    assert!(rx.try_recv().is_ok());
}

#[test]
fn server_definitions_sub_tx() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Transactions);
    m.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(Default::default()),
    );
    assert!(rx.try_recv().is_ok());
}

#[test]
fn server_definitions_sub_server() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Server);
    m.publish_json(StreamKind::Server, JsonValue::Object(Default::default()));
    assert!(rx.try_recv().is_ok());
}

#[test]
fn server_definitions_sub_validations() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Validations);
    m.publish_json(
        StreamKind::Validations,
        JsonValue::Object(Default::default()),
    );
    assert!(rx.try_recv().is_ok());
}

#[test]
fn server_definitions_sub_manifests() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Manifests);
    m.publish_json(StreamKind::Manifests, JsonValue::Object(Default::default()));
    assert!(rx.try_recv().is_ok());
}

#[test]
fn server_definitions_sub_consensus() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Consensus);
    m.publish_json(StreamKind::Consensus, JsonValue::Object(Default::default()));
    assert!(rx.try_recv().is_ok());
}

#[test]
fn server_definitions_sub_peer() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::PeerStatus);
    m.publish_json(
        StreamKind::PeerStatus,
        JsonValue::Object(Default::default()),
    );
    assert!(rx.try_recv().is_ok());
}

#[test]
fn server_definitions_sub_book() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::BookChanges);
    m.publish_json(
        StreamKind::BookChanges,
        JsonValue::Object(Default::default()),
    );
    assert!(rx.try_recv().is_ok());
}

#[test]
fn server_definitions_sub_no_cross() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Ledger);
    m.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(Default::default()),
    );
    assert!(rx.try_recv().is_err());
}

#[test]
fn server_definitions_sub_drop() {
    let m = SubscriptionManager::new(8);
    let rx = m.subscribe(StreamKind::Ledger);
    drop(rx);
    assert_eq!(
        m.publish_json(StreamKind::Ledger, JsonValue::Object(Default::default())),
        0
    );
}

#[test]
fn defs_sub_10_events() {
    let m = SubscriptionManager::new(16);
    let mut rx = m.subscribe(StreamKind::Ledger);
    for i in 0..10 {
        m.publish_json(
            StreamKind::Ledger,
            JsonValue::Object(BTreeMap::from([("i".to_owned(), u(i))])),
        );
    }
    let mut c = 0;
    while rx.try_recv().is_ok() {
        c += 1;
    }
    assert_eq!(c, 10);
}

#[test]
fn defs_sub_payload_preserved() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Server);
    m.publish_json(
        StreamKind::Server,
        JsonValue::Object(BTreeMap::from([("key".to_owned(), sv("value"))])),
    );
    let e = rx.try_recv().unwrap();
    let _payload_json: JsonValue = serde_json::from_slice(&e.payload).unwrap();
    let JsonValue::Object(p) = _payload_json else {
        panic!("obj")
    };
    assert_eq!(p.get("key"), Some(&sv("value")));
}

#[test]
fn defs_ext_31() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Ledger);
    m.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(1))])),
    );
    assert!(r.try_recv().is_ok());
}

#[test]
fn defs_ext_32() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Transactions);
    m.publish_json(StreamKind::Ledger, obj());
    assert!(r.try_recv().is_err());
}

#[test]
fn defs_ext_33() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Server);
    m.publish_json(StreamKind::Ledger, obj());
    assert!(r.try_recv().is_err());
}

#[test]
fn defs_ext_34() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Validations);
    m.publish_json(StreamKind::Ledger, obj());
    assert!(r.try_recv().is_err());
}

#[test]
fn defs_ext_35() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Manifests);
    m.publish_json(StreamKind::Ledger, obj());
    assert!(r.try_recv().is_err());
}

#[test]
fn defs_ext_36() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Consensus);
    m.publish_json(StreamKind::Ledger, obj());
    assert!(r.try_recv().is_err());
}

#[test]
fn defs_ext_37() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::PeerStatus);
    m.publish_json(StreamKind::Ledger, obj());
    assert!(r.try_recv().is_err());
}

#[test]
fn defs_ext_38() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::BookChanges);
    m.publish_json(StreamKind::Ledger, obj());
    assert!(r.try_recv().is_err());
}

#[test]
fn defs_ext_39() {
    let m = SubscriptionManager::new(8);
    let r = m.subscribe(StreamKind::Ledger);
    drop(r);
    assert_eq!(m.publish_json(StreamKind::Ledger, obj()), 0);
}

#[test]
fn defs_ext_40() {
    let m = SubscriptionManager::new(8);
    let _r = m.subscribe(StreamKind::Ledger);
    assert_eq!(m.publish_json(StreamKind::Ledger, obj()), 1);
}

#[test]
fn defs_ext_41() {
    let m = SubscriptionManager::new(8);
    let _a = m.subscribe(StreamKind::Ledger);
    let _b = m.subscribe(StreamKind::Ledger);
    assert_eq!(m.publish_json(StreamKind::Ledger, obj()), 2);
}

#[test]
fn defs_ext_42() {
    let m = SubscriptionManager::new(32);
    let mut r = m.subscribe(StreamKind::Transactions);
    for i in 0..30 {
        m.publish_json(
            StreamKind::Transactions,
            JsonValue::Object(BTreeMap::from([("n".to_owned(), u(i))])),
        );
    }
    let mut c = 0;
    while r.try_recv().is_ok() {
        c += 1;
    }
    assert_eq!(c, 30);
}

#[test]
fn defs_ext_43() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Ledger);
    m.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("ledgerClosed")),
            ("ledger_index".to_owned(), u(77)),
        ])),
    );
    let e = r.try_recv().unwrap();
    let _payload_json: JsonValue = serde_json::from_slice(&e.payload).unwrap();
    let JsonValue::Object(p) = _payload_json else {
        panic!("")
    };
    assert_eq!(p.get("type"), Some(&sv("ledgerClosed")));
    assert_eq!(p.get("ledger_index"), Some(&u(77)));
}

#[test]
fn defs_ext_44() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Transactions);
    m.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("transaction")),
            ("validated".to_owned(), b(true)),
        ])),
    );
    let e = r.try_recv().unwrap();
    let _payload_json: JsonValue = serde_json::from_slice(&e.payload).unwrap();
    let JsonValue::Object(p) = _payload_json else {
        panic!("")
    };
    assert_eq!(p.get("type"), Some(&sv("transaction")));
    assert_eq!(p.get("validated"), Some(&b(true)));
}

#[test]
fn defs_ext_45() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Server);
    m.publish_json(
        StreamKind::Server,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("serverStatus")),
            ("load_base".to_owned(), u(256)),
        ])),
    );
    let e = r.try_recv().unwrap();
    let _payload_json: JsonValue = serde_json::from_slice(&e.payload).unwrap();
    let JsonValue::Object(p) = _payload_json else {
        panic!("")
    };
    assert_eq!(p.get("type"), Some(&sv("serverStatus")));
    assert_eq!(p.get("load_base"), Some(&u(256)));
}

#[test]
fn defs_flags_14() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Ledger);
    m.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("ledgerClosed")),
            ("ledger_index".to_owned(), u(200)),
            ("txn_count".to_owned(), u(10)),
            ("fee_base".to_owned(), u(10)),
            ("reserve_base".to_owned(), u(10000000)),
            ("reserve_inc".to_owned(), u(2000000)),
        ])),
    );
    let e = r.try_recv().unwrap();
    let _payload_json: JsonValue = serde_json::from_slice(&e.payload).unwrap();
    let JsonValue::Object(p) = _payload_json else {
        panic!("")
    };
    assert_eq!(p.get("type"), Some(&sv("ledgerClosed")));
    assert_eq!(p.get("ledger_index"), Some(&u(200)));
    assert_eq!(p.get("txn_count"), Some(&u(10)));
    assert_eq!(p.get("fee_base"), Some(&u(10)));
    assert_eq!(p.get("reserve_base"), Some(&u(10000000)));
    assert_eq!(p.get("reserve_inc"), Some(&u(2000000)));
}

#[test]
fn defs_flags_15() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Transactions);
    m.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("transaction")),
            ("validated".to_owned(), b(true)),
            ("engine_result".to_owned(), sv("tesSUCCESS")),
            ("engine_result_code".to_owned(), si(0)),
            ("ledger_index".to_owned(), u(55)),
            ("ledger_hash".to_owned(), sv("AABB")),
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
    assert_eq!(p.get("engine_result_code"), Some(&u(0)));
    assert_eq!(p.get("ledger_index"), Some(&u(55)));
    assert_eq!(p.get("ledger_hash"), Some(&sv("AABB")));
}

#[test]
fn defs_flags_16() {
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
}

#[test]
fn defs_flags_17() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Validations);
    m.publish_json(
        StreamKind::Validations,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("validationReceived")),
            ("ledger_hash".to_owned(), sv("CCDD")),
            ("full".to_owned(), b(true)),
        ])),
    );
    let e = r.try_recv().unwrap();
    let _payload_json: JsonValue = serde_json::from_slice(&e.payload).unwrap();
    let JsonValue::Object(p) = _payload_json else {
        panic!("")
    };
    assert_eq!(p.get("type"), Some(&sv("validationReceived")));
    assert_eq!(p.get("full"), Some(&b(true)));
}
