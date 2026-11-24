//! account_tx against a real SQLite transaction database: marker semantics
//! and entry shapes (rippled AccountTx.cpp / `accountTxPage`).

use super::formatting::{attach_test_relational, insert_sql_transaction};
use super::*;

use quaxar_core::DatabaseCon;
use rpc::{RpcRole, do_account_tx};

/// The account `insert_sql_transaction` files its rows under.
const SQL_ACCOUNT: &str = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh";

fn add_account_row(transaction_db: &DatabaseCon, tx_id: Uint256, ledger_seq: u32, txn_seq: u32) {
    transaction_db
        .get_session()
        .execute(
            "INSERT INTO AccountTransactions (TransID, Account, LedgerSeq, TxnSeq) VALUES (?1, ?2, ?3, ?4)",
            (
                tx_id.to_string().as_str(),
                SQL_ACCOUNT,
                i64::from(ledger_seq),
                i64::from(txn_seq),
            ),
        )
        .expect("account transaction row should insert");
}

/// A standalone app with ledger 2 validated and the payment filed for
/// `SQL_ACCOUNT` at (2, 1), (2, 2) and (2, 3).
fn app_with_three_rows() -> ApplicationRoot {
    let mut app = ApplicationRoot::with_options(app::ApplicationRootOptions {
        standalone: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("standalone root should build");
    let transaction_db = attach_test_relational(&mut app, "account-tx");
    let _ = app.attach_default_network_ops_runtime();
    let (source, tx) = signed_payment_tx(0x44, account(2), 5, 10);
    let mut parent = funded_parent_ledger(1, source, 5);
    parent.set_accepted(1_111, LEDGER_DEFAULT_TIME_RESOLUTION, true);
    app.on_closed_ledger(Arc::new(parent));
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
    insert_sql_transaction(&transaction_db, &tx, &payment_meta(tx_id), 2, 1);
    add_account_row(&transaction_db, tx_id, 2, 2);
    add_account_row(&transaction_db, tx_id, 2, 3);
    app
}

fn account_tx(
    app: &ApplicationRoot,
    extra: Vec<(&str, JsonValue)>,
    api_version: u32,
) -> BTreeMap<String, JsonValue> {
    let mut params = BTreeMap::from([(
        "account".to_owned(),
        JsonValue::String(SQL_ACCOUNT.to_owned()),
    )]);
    params.extend(extra.into_iter().map(|(key, value)| (key.to_owned(), value)));
    let JsonValue::Object(result) = do_account_tx(
        &JsonValue::Object(params),
        RpcRole::Admin,
        api_version,
        &rpc::ApplicationServerInfo::new(app),
    ) else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None, "account_tx failed: {result:?}");
    result
}

fn marker(ledger: u64, seq: u64) -> JsonValue {
    JsonValue::Object(BTreeMap::from([
        ("ledger".to_owned(), JsonValue::Unsigned(ledger)),
        ("seq".to_owned(), JsonValue::Unsigned(seq)),
    ]))
}

fn entries(result: &BTreeMap<String, JsonValue>) -> &Vec<JsonValue> {
    let Some(JsonValue::Array(entries)) = result.get("transactions") else {
        panic!("transactions must be an array: {result:?}");
    };
    entries
}

fn sorted_keys(value: &JsonValue) -> Vec<&str> {
    let JsonValue::Object(object) = value else {
        panic!("entry must be an object");
    };
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys
}

/// rippled `accountTxPage`: the marker names the first row of the next page
/// (the limit+1-th row) and the next request starts at it, inclusive. Before
/// the fix quaxar returned the last row of the current page and resumed after
/// it, so pagination worked but every marker differed from rippled's.
#[test]
fn account_tx_marker_is_first_row_of_next_page() {
    let app = app_with_three_rows();
    for (forward, second, third) in [(true, 2, 3), (false, 2, 1)] {
        let page = account_tx(
            &app,
            vec![
                ("forward", JsonValue::Bool(forward)),
                ("limit", JsonValue::Unsigned(1)),
            ],
            2,
        );
        assert_eq!(entries(&page).len(), 1);
        assert_eq!(page.get("marker"), Some(&marker(2, second)), "forward={forward}");

        let page = account_tx(
            &app,
            vec![
                ("forward", JsonValue::Bool(forward)),
                ("limit", JsonValue::Unsigned(1)),
                ("marker", marker(2, second)),
            ],
            2,
        );
        assert_eq!(entries(&page).len(), 1, "resume includes the marker row");
        assert_eq!(page.get("marker"), Some(&marker(2, third)), "forward={forward}");

        let page = account_tx(
            &app,
            vec![
                ("forward", JsonValue::Bool(forward)),
                ("limit", JsonValue::Unsigned(1)),
                ("marker", marker(2, third)),
            ],
            2,
        );
        assert_eq!(entries(&page).len(), 1);
        assert_eq!(page.get("marker"), None, "last page has no marker");
    }
}

/// rippled AccountTx `populateJsonResponse` entry shapes, key for key. Before
/// the fix v1 flattened the transaction instead of nesting it under `tx`,
/// binary v1 carried the v1 JSON keys, and binary v2 added `hash` and
/// `ledger_hash`.
#[test]
fn account_tx_entry_shapes_match_rippled() {
    let app = app_with_three_rows();
    let first = |api_version: u32, binary: bool| {
        let result = account_tx(
            &app,
            vec![
                ("binary", JsonValue::Bool(binary)),
                ("limit", JsonValue::Unsigned(1)),
            ],
            api_version,
        );
        entries(&result)[0].clone()
    };

    assert_eq!(sorted_keys(&first(1, false)), ["meta", "tx", "validated"]);
    let v2 = first(2, false);
    assert_eq!(
        sorted_keys(&v2),
        [
            "close_time_iso",
            "hash",
            "ledger_hash",
            "ledger_index",
            "meta",
            "tx_json",
            "validated"
        ]
    );
    let JsonValue::Object(v2) = v2 else {
        unreachable!("checked above")
    };
    assert_eq!(v2.get("ledger_index"), Some(&JsonValue::Unsigned(2)));
    assert_ne!(
        v2.get("ledger_hash"),
        Some(&JsonValue::String("0".repeat(64))),
        "the held ledger's hash is known"
    );
    assert_eq!(
        sorted_keys(&first(1, true)),
        ["ledger_index", "meta", "tx_blob", "validated"]
    );
    assert_eq!(
        sorted_keys(&first(2, true)),
        ["ledger_index", "meta_blob", "tx_blob", "validated"]
    );
}
