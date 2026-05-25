//! publish receive tests part B.

use super::*;

#[test]
fn defs_flags_18() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::BookChanges);
    m.publish_json(
        StreamKind::BookChanges,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("bookChanges")),
            ("ledger_index".to_owned(), u(99)),
            ("changes".to_owned(), JsonValue::Array(vec![])),
        ])),
    );
    let e = r.try_recv().unwrap();
    let JsonValue::Object(p) = e.payload else {
        panic!("")
    };
    assert_eq!(p.get("type"), Some(&sv("bookChanges")));
    assert_eq!(p.get("ledger_index"), Some(&u(99)));
}

#[test]
fn defs_flags_19() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Consensus);
    m.publish_json(
        StreamKind::Consensus,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("consensusPhase")),
            ("consensus".to_owned(), sv("accepted")),
        ])),
    );
    let e = r.try_recv().unwrap();
    let JsonValue::Object(p) = e.payload else {
        panic!("")
    };
    assert_eq!(p.get("type"), Some(&sv("consensusPhase")));
    assert_eq!(p.get("consensus"), Some(&sv("accepted")));
}

#[test]
fn defs_flags_20() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::PeerStatus);
    m.publish_json(
        StreamKind::PeerStatus,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("peerStatusChange")),
            ("action".to_owned(), sv("connect")),
        ])),
    );
    let e = r.try_recv().unwrap();
    let JsonValue::Object(p) = e.payload else {
        panic!("")
    };
    assert_eq!(p.get("type"), Some(&sv("peerStatusChange")));
    assert_eq!(p.get("action"), Some(&sv("connect")));
}

#[test]
fn defs_flags_21() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Manifests);
    m.publish_json(
        StreamKind::Manifests,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("manifestReceived")),
            ("master_key".to_owned(), sv("nHB")),
        ])),
    );
    let e = r.try_recv().unwrap();
    let JsonValue::Object(p) = e.payload else {
        panic!("")
    };
    assert_eq!(p.get("type"), Some(&sv("manifestReceived")));
}

#[test]
fn defs_flags_22() {
    let m = SubscriptionManager::new(8);
    let mut r1 = m.subscribe(StreamKind::Ledger);
    let mut r2 = m.subscribe(StreamKind::Transactions);
    let mut r3 = m.subscribe(StreamKind::Server);
    m.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(1))])),
    );
    assert!(r1.try_recv().is_ok());
    assert!(r2.try_recv().is_err());
    assert!(r3.try_recv().is_err());
}

#[test]
fn defs_flags_23() {
    let m = SubscriptionManager::new(8);
    let mut r1 = m.subscribe(StreamKind::Ledger);
    let mut r2 = m.subscribe(StreamKind::Transactions);
    let mut r3 = m.subscribe(StreamKind::Server);
    m.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(2))])),
    );
    assert!(r1.try_recv().is_err());
    assert!(r2.try_recv().is_ok());
    assert!(r3.try_recv().is_err());
}

#[test]
fn defs_flags_24() {
    let m = SubscriptionManager::new(8);
    let mut r1 = m.subscribe(StreamKind::Ledger);
    let mut r2 = m.subscribe(StreamKind::Transactions);
    let mut r3 = m.subscribe(StreamKind::Server);
    m.publish_json(
        StreamKind::Server,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(3))])),
    );
    assert!(r1.try_recv().is_err());
    assert!(r2.try_recv().is_err());
    assert!(r3.try_recv().is_ok());
}

#[test]
fn defs_flags_25() {
    let m = SubscriptionManager::new(64);
    let mut r = m.subscribe(StreamKind::Ledger);
    for i in 0..50 {
        m.publish_json(
            StreamKind::Ledger,
            JsonValue::Object(BTreeMap::from([("n".to_owned(), u(i))])),
        );
    }
    let mut c = 0;
    while r.try_recv().is_ok() {
        c += 1;
    }
    assert_eq!(c, 50);
}

#[test]
fn defs_flags_26() {
    let m = SubscriptionManager::new(64);
    let mut r = m.subscribe(StreamKind::Transactions);
    for i in 0..40 {
        m.publish_json(
            StreamKind::Transactions,
            JsonValue::Object(BTreeMap::from([("n".to_owned(), u(i))])),
        );
    }
    let mut c = 0;
    while r.try_recv().is_ok() {
        c += 1;
    }
    assert_eq!(c, 40);
}

#[test]
fn defs_fmt_1() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Ledger);
    m.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([("a".to_owned(), u(1))])),
    );
    let e = r.try_recv().unwrap();
    assert_eq!(e.stream, StreamKind::Ledger);
}

#[test]
fn defs_fmt_2() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Transactions);
    m.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(BTreeMap::from([("b".to_owned(), u(2))])),
    );
    let e = r.try_recv().unwrap();
    assert_eq!(e.stream, StreamKind::Transactions);
}

#[test]
fn defs_fmt_3() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Server);
    m.publish_json(
        StreamKind::Server,
        JsonValue::Object(BTreeMap::from([("c".to_owned(), u(3))])),
    );
    let e = r.try_recv().unwrap();
    assert_eq!(e.stream, StreamKind::Server);
}

#[test]
fn defs_fmt_4() {
    let m = SubscriptionManager::new(8);
    let r = m.subscribe(StreamKind::Ledger);
    drop(r);
    let n = m.publish_json(StreamKind::Ledger, JsonValue::Object(Default::default()));
    assert_eq!(n, 0);
}

#[test]
fn defs_fmt_5() {
    let m = SubscriptionManager::new(8);
    let _r1 = m.subscribe(StreamKind::Ledger);
    let _r2 = m.subscribe(StreamKind::Ledger);
    let n = m.publish_json(StreamKind::Ledger, JsonValue::Object(Default::default()));
    assert_eq!(n, 2);
}

#[test]
fn defs_fmt_6() {
    let m = SubscriptionManager::new(8);
    let mut r = m.subscribe(StreamKind::Ledger);
    for i in 0..5 {
        m.publish_json(
            StreamKind::Ledger,
            JsonValue::Object(BTreeMap::from([("i".to_owned(), u(i))])),
        );
    }
    let mut c = 0;
    while r.try_recv().is_ok() {
        c += 1;
    }
    assert_eq!(c, 5);
}

#[test]
fn defs_fmt_51() {
    let m = SubscriptionManager::new(16);
    let mut r = m.subscribe(StreamKind::Ledger);
    m.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("ledgerClosed")),
            ("ledger_index".to_owned(), u(100)),
            ("txn_count".to_owned(), u(7)),
        ])),
    );
    let e = r.try_recv().unwrap();
    let JsonValue::Object(p) = e.payload else {
        panic!("")
    };
    assert_eq!(p.get("ledger_index"), Some(&u(100)));
    assert_eq!(p.get("txn_count"), Some(&u(7)));
}

#[test]
fn defs_fmt_52() {
    let m = SubscriptionManager::new(16);
    let mut r = m.subscribe(StreamKind::Transactions);
    m.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("transaction")),
            ("validated".to_owned(), b(true)),
            ("ledger_index".to_owned(), u(50)),
        ])),
    );
    let e = r.try_recv().unwrap();
    let JsonValue::Object(p) = e.payload else {
        panic!("")
    };
    assert_eq!(p.get("validated"), Some(&b(true)));
    assert_eq!(p.get("ledger_index"), Some(&u(50)));
}

#[test]
fn defs_fmt_53() {
    let m = SubscriptionManager::new(16);
    let mut r = m.subscribe(StreamKind::Server);
    m.publish_json(
        StreamKind::Server,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("serverStatus")),
            ("load_base".to_owned(), u(256)),
            ("load_factor".to_owned(), u(512)),
        ])),
    );
    let e = r.try_recv().unwrap();
    let JsonValue::Object(p) = e.payload else {
        panic!("")
    };
    assert_eq!(p.get("load_base"), Some(&u(256)));
    assert_eq!(p.get("load_factor"), Some(&u(512)));
}

#[test]
fn defs_fmt_54() {
    let m = SubscriptionManager::new(8);
    let mut r1 = m.subscribe(StreamKind::Ledger);
    let mut r2 = m.subscribe(StreamKind::Transactions);
    m.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(1))])),
    );
    assert!(r1.try_recv().is_ok());
    assert!(r2.try_recv().is_err());
}

#[test]
fn defs_fmt_55() {
    let m = SubscriptionManager::new(8);
    let mut r1 = m.subscribe(StreamKind::Ledger);
    let mut r2 = m.subscribe(StreamKind::Transactions);
    m.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(BTreeMap::from([("x".to_owned(), u(2))])),
    );
    assert!(r1.try_recv().is_err());
    assert!(r2.try_recv().is_ok());
}

#[test]
fn defs_fmt_56() {
    let m = SubscriptionManager::new(32);
    let mut r = m.subscribe(StreamKind::Ledger);
    for i in 0..20 {
        m.publish_json(
            StreamKind::Ledger,
            JsonValue::Object(BTreeMap::from([("n".to_owned(), u(i))])),
        );
    }
    let mut c = 0;
    while r.try_recv().is_ok() {
        c += 1;
    }
    assert_eq!(c, 20);
}

#[test]
fn defs_fmt_84() {
    let m = SubscriptionManager::new(8);
    let n = m.publish_json(StreamKind::Ledger, JsonValue::Object(Default::default()));
    assert_eq!(n, 0);
}

#[test]
fn defs_fmt_85() {
    let m = SubscriptionManager::new(8);
    let _r = m.subscribe(StreamKind::Ledger);
    let n = m.publish_json(StreamKind::Ledger, JsonValue::Object(Default::default()));
    assert_eq!(n, 1);
}

#[test]
fn defs_fmt_86() {
    let m = SubscriptionManager::new(8);
    let _r1 = m.subscribe(StreamKind::Ledger);
    let _r2 = m.subscribe(StreamKind::Ledger);
    let _r3 = m.subscribe(StreamKind::Ledger);
    let n = m.publish_json(StreamKind::Ledger, JsonValue::Object(Default::default()));
    assert_eq!(n, 3);
}

#[test]
fn defs_types_63_sub_ledger_fields() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Ledger);
    m.publish_json(
        StreamKind::Ledger,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("ledgerClosed")),
            ("ledger_index".to_owned(), u(55)),
            ("txn_count".to_owned(), u(3)),
            ("fee_base".to_owned(), u(10)),
        ])),
    );
    let e = rx.try_recv().unwrap();
    let JsonValue::Object(p) = e.payload else {
        panic!("obj")
    };
    assert_eq!(p.get("type"), Some(&sv("ledgerClosed")));
    assert_eq!(p.get("ledger_index"), Some(&u(55)));
    assert_eq!(p.get("txn_count"), Some(&u(3)));
    assert_eq!(p.get("fee_base"), Some(&u(10)));
}

#[test]
fn defs_types_64_sub_tx_fields() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Transactions);
    m.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("transaction")),
            ("validated".to_owned(), b(true)),
            ("engine_result".to_owned(), sv("tesSUCCESS")),
            ("engine_result_code".to_owned(), si(0)),
            ("ledger_index".to_owned(), u(10)),
        ])),
    );
    let e = rx.try_recv().unwrap();
    let JsonValue::Object(p) = e.payload else {
        panic!("obj")
    };
    assert_eq!(p.get("type"), Some(&sv("transaction")));
    assert_eq!(p.get("validated"), Some(&b(true)));
    assert_eq!(p.get("engine_result"), Some(&sv("tesSUCCESS")));
    assert_eq!(p.get("engine_result_code"), Some(&si(0)));
    assert_eq!(p.get("ledger_index"), Some(&u(10)));
}

#[test]
fn defs_types_65_sub_server_fields() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Server);
    m.publish_json(
        StreamKind::Server,
        JsonValue::Object(BTreeMap::from([
            ("type".to_owned(), sv("serverStatus")),
            ("server_status".to_owned(), sv("full")),
            ("load_base".to_owned(), u(256)),
            ("load_factor".to_owned(), u(256)),
        ])),
    );
    let e = rx.try_recv().unwrap();
    let JsonValue::Object(p) = e.payload else {
        panic!("obj")
    };
    assert_eq!(p.get("type"), Some(&sv("serverStatus")));
    assert_eq!(p.get("server_status"), Some(&sv("full")));
    assert_eq!(p.get("load_base"), Some(&u(256)));
    assert_eq!(p.get("load_factor"), Some(&u(256)));
}
// Feature: field checks

#[test]
fn defs_types_75_sub_ledger_no_tx() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Ledger);
    m.publish_json(
        StreamKind::Transactions,
        JsonValue::Object(Default::default()),
    );
    assert!(rx.try_recv().is_err());
}

#[test]
fn defs_types_76_sub_tx_no_server() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Transactions);
    m.publish_json(StreamKind::Server, JsonValue::Object(Default::default()));
    assert!(rx.try_recv().is_err());
}

#[test]
fn defs_types_77_sub_server_no_ledger() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Server);
    m.publish_json(StreamKind::Ledger, JsonValue::Object(Default::default()));
    assert!(rx.try_recv().is_err());
}

#[test]
fn defs_types_78_sub_val_no_consensus() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Validations);
    m.publish_json(StreamKind::Consensus, JsonValue::Object(Default::default()));
    assert!(rx.try_recv().is_err());
}

#[test]
fn defs_types_79_sub_consensus_no_val() {
    let m = SubscriptionManager::new(8);
    let mut rx = m.subscribe(StreamKind::Consensus);
    m.publish_json(
        StreamKind::Validations,
        JsonValue::Object(Default::default()),
    );
    assert!(rx.try_recv().is_err());
}
