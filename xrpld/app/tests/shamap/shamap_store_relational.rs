use app::{SHAMapStoreRelationalRuntime, SqliteSHAMapStoreRelational};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use xrpld_core::{DatabaseCon, LEDGER_DB_INIT, TRANSACTION_DB_INIT};

fn insert_ledger_seq(db: &DatabaseCon, seq: u32) {
    let connection = db.get_session();
    let mut hash = vec![0_u8; 32];
    hash[0..4].copy_from_slice(&seq.to_be_bytes());
    connection
        .execute(
            "INSERT INTO Ledgers (LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![hash, seq, vec![0_u8; 32], "0", 0_i64, 0_i64, 0_i64, 0_i64, vec![0_u8; 32], vec![0_u8; 32]],
        )
        .expect("ledger insert");
}

fn insert_tx_seq(db: &DatabaseCon, seq: u32) {
    let connection = db.get_session();
    let mut hash = vec![0_u8; 32];
    hash[0..4].copy_from_slice(&seq.to_be_bytes());
    connection
        .execute(
            "INSERT INTO Transactions (TransID, TransType, FromAcct, FromSeq, LedgerSeq, Status, RawTxn, TxnMeta) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![hash, "Payment", vec![0_u8; 20], 1_i64, seq, "tesSUCCESS", vec![0_u8; 1], vec![0_u8; 1]],
        )
        .expect("transaction insert");
}

fn insert_account_tx_seq(db: &DatabaseCon, seq: u32) {
    let connection = db.get_session();
    let mut hash = vec![0_u8; 32];
    hash[0..4].copy_from_slice(&seq.to_be_bytes());
    connection
        .execute(
            "INSERT INTO AccountTransactions (TransID, Account, LedgerSeq, TxnSeq) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![hash, vec![0_u8; 20], seq, 1_i64],
        )
        .expect("account transaction insert");
}

fn count_rows(db: &DatabaseCon, table_name: &str) -> i64 {
    let connection = db.get_session();
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table_name}"), [], |row| {
            row.get(0)
        })
        .expect("count query")
}

#[test]
fn shamap_store_relational_uses_ledgers_minimum_for_minimum_online_fallback() {
    let temp = TempDir::new().expect("tempdir");
    let ledger_db = Arc::new(
        DatabaseCon::new_at_path(temp.path(), "ledger.db", &[], LEDGER_DB_INIT).expect("ledger db"),
    );
    insert_ledger_seq(&ledger_db, 800);
    insert_ledger_seq(&ledger_db, 900);

    let relational =
        SqliteSHAMapStoreRelational::new(ledger_db, None, false, 100, Duration::from_secs(0));

    assert_eq!(relational.minimum_sql_seq(), Some(800));
}

#[test]
fn shamap_store_relational_clears_ledgers_transactions_and_account_transactions_before_last_rotated()
 {
    let temp = TempDir::new().expect("tempdir");
    let ledger_db = Arc::new(
        DatabaseCon::new_at_path(temp.path(), "ledger.db", &[], LEDGER_DB_INIT).expect("ledger db"),
    );
    let transaction_db = Arc::new(
        DatabaseCon::new_at_path(temp.path(), "tx.db", &[], TRANSACTION_DB_INIT)
            .expect("transaction db"),
    );

    insert_ledger_seq(&ledger_db, 800);
    insert_ledger_seq(&ledger_db, 950);
    insert_tx_seq(&transaction_db, 810);
    insert_tx_seq(&transaction_db, 920);
    insert_account_tx_seq(&transaction_db, 820);
    insert_account_tx_seq(&transaction_db, 930);

    let relational = SqliteSHAMapStoreRelational::new(
        Arc::clone(&ledger_db),
        Some(Arc::clone(&transaction_db)),
        true,
        50,
        Duration::from_secs(0),
    );

    relational
        .clear_prior(900, &|| false)
        .expect("clear should succeed");

    assert_eq!(count_rows(&ledger_db, "Ledgers"), 1);
    assert_eq!(count_rows(&transaction_db, "Transactions"), 1);
    assert_eq!(count_rows(&transaction_db, "AccountTransactions"), 1);
    assert_eq!(relational.minimum_sql_seq(), Some(950));
}
