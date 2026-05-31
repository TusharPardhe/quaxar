//! tx tests part 2.

use super::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use app::SqliteSHAMapStoreRelational;
use xrpld_core::{DatabaseCon, LEDGER_DB_INIT, TRANSACTION_DB_INIT};

fn attach_test_relational(app: &mut ApplicationRoot, label: &str) -> Arc<DatabaseCon> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "xrpld-rpc-tx-{label}-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("test db dir should be created");

    let ledger_db = Arc::new(
        DatabaseCon::new_at_path(&dir, "ledger.db", &[], LEDGER_DB_INIT).expect("ledger db"),
    );
    let transaction_db = Arc::new(
        DatabaseCon::new_at_path(&dir, "transaction.db", &[], TRANSACTION_DB_INIT)
            .expect("transaction db"),
    );
    let relational = Arc::new(SqliteSHAMapStoreRelational::new(
        ledger_db,
        Some(Arc::clone(&transaction_db)),
        true,
        100,
        Duration::from_secs(0),
    ));
    app.attach_relational_database(Some(relational));
    transaction_db
}

fn insert_sql_transaction(
    transaction_db: &DatabaseCon,
    tx: &STTx,
    meta: &TxMeta,
    ledger_seq: u32,
    txn_seq: u32,
) {
    let connection = transaction_db.get_session();
    let tx_id = tx.get_transaction_id().to_string();
    connection
        .execute(
            "INSERT OR REPLACE INTO Transactions (TransID, TransType, FromAcct, FromSeq, LedgerSeq, Status, RawTxn, TxnMeta) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                tx_id.as_str(),
                "Payment",
                "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh",
                5_i64,
                i64::from(ledger_seq),
                "V",
                tx.get_serializer().data(),
                meta.get_as_object().get_serializer().data(),
            ),
        )
        .expect("transaction row should insert");
    connection
        .execute(
            "INSERT INTO AccountTransactions (TransID, Account, LedgerSeq, TxnSeq) VALUES (?1, ?2, ?3, ?4)",
            (
                tx_id.as_str(),
                "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh",
                i64::from(ledger_seq),
                i64::from(txn_seq),
            ),
        )
        .expect("account transaction row should insert");
}

#[test]
fn tx_reads_committed_live_transactions_from_application_server_info() {
    let mut app = ApplicationRoot::with_options(app::ApplicationRootOptions {
        standalone: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("standalone root should build");
    let _ = app.attach_default_network_ops_runtime();
    let (source, tx) = signed_payment_tx(0x21, account(2), 5, 10);
    let mut parent = funded_parent_ledger(1, source, 5);
    parent.set_accepted(1_111, LEDGER_DEFAULT_TIME_RESOLUTION, true);
    let parent = Arc::new(parent);
    app.on_closed_ledger(Arc::clone(&parent));
    let _ = app.open_ledger().modify(|view| {
        *view = AppOpenLedgerView::new(2, 10);
        true
    });
    let tx_id = tx.get_transaction_id();
    let mut cached = Arc::new(std::sync::Mutex::new(Transaction::new(Arc::clone(&tx))));
    app.canonicalize_transaction(&mut cached);
    assert!(app.add_held_transaction(&Transaction::new(Arc::clone(&tx))));
    app.accept_standalone_ledger()
        .expect("standalone ledger accept should succeed");

    let response = do_tx(
        &TxRequest {
            params: &object([("transaction", JsonValue::String(tx_id.to_string()))]),
            api_version: 2,
        },
        &rpc::ApplicationServerInfo::new(&app),
    );
    let JsonValue::Object(object) = response else {
        panic!("response must be an object");
    };

    assert_eq!(object.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(object.get("ledger_index"), Some(&JsonValue::Unsigned(2)));
    assert_eq!(
        object.get("hash"),
        Some(&JsonValue::String(tx_id.to_string()))
    );
}

#[test]
fn tx_reads_unvalidated_cached_transactions_as_proposed() {
    let app = ApplicationRoot::with_options(app::ApplicationRootOptions {
        standalone: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("standalone root should build");
    let (_, tx) = signed_payment_tx(0x20, account(2), 5, 10);
    let tx_id = tx.get_transaction_id();
    let mut cached = Arc::new(std::sync::Mutex::new(Transaction::new(Arc::clone(&tx))));
    app.canonicalize_transaction(&mut cached);

    let response = do_tx(
        &TxRequest {
            params: &object([("transaction", JsonValue::String(tx_id.to_string()))]),
            api_version: 2,
        },
        &rpc::ApplicationServerInfo::new(&app),
    );
    let JsonValue::Object(object) = response else {
        panic!("response must be an object");
    };

    assert_eq!(object.get("validated"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        object.get("hash"),
        Some(&JsonValue::String(tx_id.to_string()))
    );
    assert!(object.contains_key("tx_json"));
    assert!(!object.contains_key("ledger_index"));
    assert!(!object.contains_key("ledger_hash"));
    assert!(!object.contains_key("meta"));
    assert!(!object.contains_key("ctid"));
}

#[test]
fn tx_prefers_sql_metadata_for_validated_cached_transactions() {
    let mut app = ApplicationRoot::with_options(app::ApplicationRootOptions {
        standalone: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("standalone root should build");
    let transaction_db = attach_test_relational(&mut app, "validated-cache");
    let _ = app.attach_default_network_ops_runtime();
    let (source, tx) = signed_payment_tx(0x22, account(2), 5, 10);
    let mut parent = funded_parent_ledger(1, source, 5);
    parent.set_accepted(1_111, LEDGER_DEFAULT_TIME_RESOLUTION, true);
    let parent = Arc::new(parent);
    app.on_closed_ledger(Arc::clone(&parent));
    let _ = app.open_ledger().modify(|view| {
        *view = AppOpenLedgerView::new(2, 10);
        true
    });
    let tx_id = tx.get_transaction_id();
    let mut cached = Arc::new(std::sync::Mutex::new(Transaction::new(Arc::clone(&tx))));
    app.canonicalize_transaction(&mut cached);
    assert!(app.add_held_transaction(&Transaction::new(Arc::clone(&tx))));
    app.accept_standalone_ledger()
        .expect("standalone ledger accept should succeed");
    insert_sql_transaction(&transaction_db, &tx, &payment_meta(tx_id), 2, 3);

    let response = do_tx(
        &TxRequest {
            params: &object([("transaction", JsonValue::String(tx_id.to_string()))]),
            api_version: 2,
        },
        &rpc::ApplicationServerInfo::new(&app),
    );
    let JsonValue::Object(object) = response else {
        panic!("response must be an object");
    };

    assert_eq!(object.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(object.get("ledger_index"), Some(&JsonValue::Unsigned(2)));
    let JsonValue::Object(meta) = object.get("meta").expect("sql metadata must be returned") else {
        panic!("metadata must be an object");
    };
    assert_eq!(
        meta.get("TransactionResult"),
        Some(&JsonValue::String("tesSUCCESS".to_owned()))
    );
    assert!(object.contains_key("ctid"));
}

#[test]
fn tx_requires_network_sync() {
    let source = FakeTxSource {
        enabled: true,
        synced: false,
        network_id: 0,
        by_hash: BTreeMap::new(),
        by_ctid: BTreeMap::new(),
    };

    let JsonValue::Object(result) = do_tx(
        &TxRequest {
            params: &object([(
                "transaction",
                JsonValue::String(Uint256::from_array([0xFF; 32]).to_string()),
            )]),
            api_version: 1,
        },
        &source,
    ) else {
        panic!("response must be object");
    };

    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("noNetwork".to_owned()))
    );
    assert_eq!(result.get("error_code"), Some(&JsonValue::Signed(17)));
}

#[test]
fn tx_missing_transaction_field() {
    let source = FakeTxSource {
        enabled: true,
        synced: true,
        network_id: 0,
        by_hash: BTreeMap::new(),
        by_ctid: BTreeMap::new(),
    };

    let result = do_tx(
        &TxRequest {
            params: &object([]),
            api_version: 2,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("error"));
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
}

#[test]
fn tx_invalid_transaction_hash() {
    let source = FakeTxSource {
        enabled: true,
        synced: true,
        network_id: 0,
        by_hash: BTreeMap::new(),
        by_ctid: BTreeMap::new(),
    };

    let result = do_tx(
        &TxRequest {
            params: &object([("transaction", JsonValue::String("not_a_hash".to_owned()))]),
            api_version: 2,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("error"));
}

#[test]
fn tx_not_found() {
    let source = FakeTxSource {
        enabled: true,
        synced: true,
        network_id: 0,
        by_hash: BTreeMap::new(),
        by_ctid: BTreeMap::new(),
    };

    let result = do_tx(
        &TxRequest {
            params: &object([(
                "transaction",
                JsonValue::String(Uint256::from_array([0xAA; 32]).to_string()),
            )]),
            api_version: 2,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("txnNotFound".to_owned()))
    );
    assert_eq!(result.get("error_code"), Some(&JsonValue::Signed(29)));
}

#[test]
fn tx_non_string_transaction_field() {
    let source = FakeTxSource {
        enabled: true,
        synced: true,
        network_id: 0,
        by_hash: BTreeMap::new(),
        by_ctid: BTreeMap::new(),
    };

    for param in [
        JsonValue::Unsigned(42),
        JsonValue::Bool(true),
        JsonValue::Null,
    ] {
        let result = do_tx(
            &TxRequest {
                params: &object([("transaction", param)]),
                api_version: 2,
            },
            &source,
        );
        let JsonValue::Object(result) = result else {
            panic!("result must be an object");
        };
        assert!(
            result.contains_key("error"),
            "non-string transaction should produce error"
        );
    }
}

#[test]
fn tx_v1_response_includes_inledger_and_ledger_index() {
    let mut app = ApplicationRoot::with_options(app::ApplicationRootOptions {
        standalone: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("standalone root should build");
    let _ = app.attach_default_network_ops_runtime();
    let (source, tx) = signed_payment_tx(0x31, account(2), 5, 10);
    let mut parent = funded_parent_ledger(1, source, 5);
    parent.set_accepted(1_111, LEDGER_DEFAULT_TIME_RESOLUTION, true);
    let parent = Arc::new(parent);
    app.on_closed_ledger(Arc::clone(&parent));
    let _ = app.open_ledger().modify(|view| {
        *view = AppOpenLedgerView::new(2, 10);
        true
    });
    let tx_id = tx.get_transaction_id();
    let mut cached = Arc::new(std::sync::Mutex::new(Transaction::new(Arc::clone(&tx))));
    app.canonicalize_transaction(&mut cached);
    assert!(app.add_held_transaction(&Transaction::new(Arc::clone(&tx))));
    app.accept_standalone_ledger()
        .expect("standalone ledger accept should succeed");

    // v1 response
    let response = do_tx(
        &TxRequest {
            params: &object([("transaction", JsonValue::String(tx_id.to_string()))]),
            api_version: 1,
        },
        &rpc::ApplicationServerInfo::new(&app),
    );
    let JsonValue::Object(object) = response else {
        panic!("response must be an object");
    };

    assert_eq!(object.get("validated"), Some(&JsonValue::Bool(true)));
    // v1 should have inLedger (deprecated) and ledger_index
    assert_eq!(object.get("ledger_index"), Some(&JsonValue::Unsigned(2)));
    assert_eq!(object.get("inLedger"), Some(&JsonValue::Unsigned(2)));
    assert!(object.contains_key("hash"));
    // v1 has tx fields at top level
    assert!(object.contains_key("TransactionType"));
    assert!(object.contains_key("Account"));
}

#[test]
fn tx_v2_response_uses_tx_json_wrapper() {
    let mut app = ApplicationRoot::with_options(app::ApplicationRootOptions {
        standalone: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("standalone root should build");
    let _ = app.attach_default_network_ops_runtime();
    let (source, tx) = signed_payment_tx(0x32, account(2), 5, 10);
    let mut parent = funded_parent_ledger(1, source, 5);
    parent.set_accepted(1_111, LEDGER_DEFAULT_TIME_RESOLUTION, true);
    let parent = Arc::new(parent);
    app.on_closed_ledger(Arc::clone(&parent));
    let _ = app.open_ledger().modify(|view| {
        *view = AppOpenLedgerView::new(2, 10);
        true
    });
    let tx_id = tx.get_transaction_id();
    let mut cached = Arc::new(std::sync::Mutex::new(Transaction::new(Arc::clone(&tx))));
    app.canonicalize_transaction(&mut cached);
    assert!(app.add_held_transaction(&Transaction::new(Arc::clone(&tx))));
    app.accept_standalone_ledger()
        .expect("standalone ledger accept should succeed");

    // v2 response
    let response = do_tx(
        &TxRequest {
            params: &object([("transaction", JsonValue::String(tx_id.to_string()))]),
            api_version: 2,
        },
        &rpc::ApplicationServerInfo::new(&app),
    );
    let JsonValue::Object(object) = response else {
        panic!("response must be an object");
    };

    assert_eq!(object.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(object.get("ledger_index"), Some(&JsonValue::Unsigned(2)));
    assert!(object.contains_key("hash"));
    // v2 should have tx_json wrapper, not top-level tx fields
    assert!(object.contains_key("tx_json"));
    assert!(!object.contains_key("TransactionType"));
    // v2 should have close_time_iso
    assert!(object.contains_key("close_time_iso"));
    // v2 should NOT have inLedger (deprecated)
    assert!(!object.contains_key("inLedger"));
}

#[test]
fn tx_not_found_returns_correct_error() {
    let app = ApplicationRoot::with_options(app::ApplicationRootOptions {
        standalone: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("standalone root should build");

    let fake_hash = Uint256::from_array([0xEE; 32]);
    let response = do_tx(
        &TxRequest {
            params: &object([("transaction", JsonValue::String(fake_hash.to_string()))]),
            api_version: 2,
        },
        &rpc::ApplicationServerInfo::new(&app),
    );
    let JsonValue::Object(object) = response else {
        panic!("response must be an object");
    };

    assert_eq!(
        object.get("error"),
        Some(&JsonValue::String("txnNotFound".to_owned()))
    );
    assert_eq!(object.get("error_code"), Some(&JsonValue::Signed(29)));
}
