use crate::clear_sql_batches;
use crate::tx_queue::transaction::Transaction;
use crate::tx_queue::transaction_master::TransactionMaster;
use ledger::AcceptedLedger;
use protocol::{STTx, to_base58};
use rusqlite::params;
use std::sync::Arc;
use std::time::Duration;
use xrpld_core::DatabaseCon;

pub trait SHAMapStoreRelationalRuntime: Send + Sync {
    fn minimum_sql_seq(&self) -> Option<u32>;
    fn clear_prior(&self, last_rotated: u32, should_stop: &dyn Fn() -> bool) -> Result<(), String>;
}

pub(crate) trait SHAMapStoreRelationalStore {
    fn minimum_ledger_seq(&self) -> Result<Option<u32>, String>;
    fn delete_ledgers_before(&self, ledger_seq: u32) -> Result<(), String>;
    fn transactions_min_ledger_seq(&self) -> Result<Option<u32>, String>;
    fn delete_transactions_before(&self, ledger_seq: u32) -> Result<(), String>;
    fn account_transactions_min_ledger_seq(&self) -> Result<Option<u32>, String>;
    fn delete_account_transactions_before(&self, ledger_seq: u32) -> Result<(), String>;
}

pub(crate) fn clear_relational_prior<S, Sleep>(
    store: &S,
    last_rotated: u32,
    use_tx_tables: bool,
    delete_batch: u32,
    back_off: Duration,
    should_stop: &dyn Fn() -> bool,
    sleep: Sleep,
) -> Result<(), String>
where
    S: SHAMapStoreRelationalStore,
    Sleep: FnMut(Duration) + Copy,
{
    clear_table(
        last_rotated,
        || store.minimum_ledger_seq(),
        |ledger_seq| store.delete_ledgers_before(ledger_seq),
        delete_batch,
        back_off,
        should_stop,
        sleep,
    )?;
    if should_stop() || !use_tx_tables {
        return Ok(());
    }

    clear_table(
        last_rotated,
        || store.transactions_min_ledger_seq(),
        |ledger_seq| store.delete_transactions_before(ledger_seq),
        delete_batch,
        back_off,
        should_stop,
        sleep,
    )?;
    if should_stop() {
        return Ok(());
    }

    clear_table(
        last_rotated,
        || store.account_transactions_min_ledger_seq(),
        |ledger_seq| store.delete_account_transactions_before(ledger_seq),
        delete_batch,
        back_off,
        should_stop,
        sleep,
    )?;

    Ok(())
}

#[derive(Clone)]
pub struct SqliteSHAMapStoreRelational {
    ledger_db: Arc<DatabaseCon>,
    transaction_db: Option<Arc<DatabaseCon>>,
    use_tx_tables: bool,
    delete_batch: u32,
    back_off: Duration,
}

impl SqliteSHAMapStoreRelational {
    pub fn new(
        ledger_db: Arc<DatabaseCon>,
        transaction_db: Option<Arc<DatabaseCon>>,
        use_tx_tables: bool,
        delete_batch: u32,
        back_off: Duration,
    ) -> Self {
        Self {
            ledger_db,
            transaction_db,
            use_tx_tables,
            delete_batch,
            back_off,
        }
    }

    pub fn write_accepted_ledger(
        &self,
        accepted_ledger: &AcceptedLedger,
        transaction_master: &TransactionMaster,
        network_id: u32,
    ) -> Result<(), String> {
        self.write_transactions(accepted_ledger, transaction_master, network_id)?;
        self.write_ledger(accepted_ledger)
    }

    fn write_transactions(
        &self,
        accepted_ledger: &AcceptedLedger,
        transaction_master: &TransactionMaster,
        network_id: u32,
    ) -> Result<(), String> {
        let Some(transaction_db) = &self.transaction_db else {
            return Ok(());
        };

        let ledger_seq = accepted_ledger.get_ledger().header().seq;
        let mut connection = transaction_db.checkout_db();
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;

        for accepted_ledger_tx in accepted_ledger {
            let transaction_id = accepted_ledger_tx.get_transaction_id().to_string();
            transaction
                .execute(
                    "DELETE FROM AccountTransactions WHERE TransID = ?1",
                    params![transaction_id],
                )
                .map_err(|error| error.to_string())?;

            // Use affected accounts from metadata; fall back to accounts
            // mentioned in the transaction itself (for standalone mode where
            // metadata may not have proper AffectedNodes).
            let affected: std::collections::BTreeSet<_> = if accepted_ledger_tx.get_affected().is_empty() {
                accepted_ledger_tx.get_txn().get_mentioned_accounts()
            } else {
                accepted_ledger_tx.get_affected().clone()
            };

            if !affected.is_empty() {
                let mut sql = String::from(
                    "INSERT INTO AccountTransactions (TransID, Account, LedgerSeq, TxnSeq) VALUES ",
                );

                for (index, account) in affected.iter().enumerate() {
                    if index != 0 {
                        sql.push_str(", ");
                    }
                    sql.push_str("('");
                    sql.push_str(&accepted_ledger_tx.get_transaction_id().to_string());
                    sql.push_str("','");
                    sql.push_str(&to_base58(*account));
                    sql.push_str("',");
                    sql.push_str(&ledger_seq.to_string());
                    sql.push(',');
                    sql.push_str(&accepted_ledger_tx.get_txn_seq().to_string());
                    sql.push(')');
                }
                sql.push(';');

                transaction
                    .execute_batch(&sql)
                    .map_err(|error| error.to_string())?;
            }

            let sql = format!(
                "{}{};",
                STTx::get_meta_sql_insert_replace_header(),
                accepted_ledger_tx
                    .get_txn()
                    .get_meta_sql(ledger_seq, &accepted_ledger_tx.get_esc_meta())
            );
            transaction
                .execute_batch(&sql)
                .map_err(|error| error.to_string())?;

            let _ = transaction_master.in_ledger(
                accepted_ledger_tx.get_transaction_id(),
                ledger_seq,
                Some(accepted_ledger_tx.get_txn_seq()),
                Some(network_id),
            );
        }

        transaction.commit().map_err(|error| error.to_string())
    }

    fn write_ledger(&self, accepted_ledger: &AcceptedLedger) -> Result<(), String> {
        let header = accepted_ledger.get_ledger().header();
        let mut connection = self.ledger_db.checkout_db();
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO Ledgers (LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    header.hash.as_uint256().to_string(),
                    i64::from(header.seq),
                    header.parent_hash.as_uint256().to_string(),
                    i64::try_from(header.drops).map_err(|_| "ledger drops exceed sqlite i64 range".to_owned())?,
                    i64::from(header.close_time),
                    i64::from(header.parent_close_time),
                    i64::from(header.close_time_resolution),
                    i64::from(header.close_flags),
                    header.account_hash.as_uint256().to_string(),
                    header.tx_hash.as_uint256().to_string(),
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn ledger_db(&self) -> Arc<DatabaseCon> {
        Arc::clone(&self.ledger_db)
    }

    pub fn transaction_db(&self) -> Option<Arc<DatabaseCon>> {
        self.transaction_db.as_ref().map(Arc::clone)
    }

    pub fn get_tx_history(&self, start_index: u32) -> Vec<Arc<Transaction>> {
        let Some(transaction_db) = &self.transaction_db else {
            return Vec::new();
        };

        let connection = transaction_db.get_session();
        let Ok(mut statement) = connection.prepare(
            "SELECT LedgerSeq, Status, RawTxn \
             FROM Transactions ORDER BY LedgerSeq DESC LIMIT ?1, ?2",
        ) else {
            return Vec::new();
        };

        let Ok(rows) = statement.query_map(params![i64::from(start_index), 20_i64], |row| {
            Ok((
                row.get::<_, Option<u64>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
            ))
        }) else {
            return Vec::new();
        };

        rows.filter_map(|row| {
            let (ledger_seq, status, raw_txn) = row.ok()?;
            Transaction::transaction_from_sql(
                ledger_seq,
                status.as_deref(),
                raw_txn.as_deref().unwrap_or(&[]),
            )
            .ok()
            .map(Arc::new)
        })
        .collect()
    }
}

impl std::fmt::Debug for SqliteSHAMapStoreRelational {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteSHAMapStoreRelational")
            .field("has_transaction_db", &self.transaction_db.is_some())
            .field("use_tx_tables", &self.use_tx_tables)
            .field("delete_batch", &self.delete_batch)
            .field("back_off", &self.back_off)
            .finish()
    }
}

impl SHAMapStoreRelationalRuntime for SqliteSHAMapStoreRelational {
    fn minimum_sql_seq(&self) -> Option<u32> {
        self.minimum_ledger_seq().ok().flatten()
    }

    fn clear_prior(&self, last_rotated: u32, should_stop: &dyn Fn() -> bool) -> Result<(), String> {
        clear_relational_prior(
            self,
            last_rotated,
            self.use_tx_tables,
            self.delete_batch,
            self.back_off,
            should_stop,
            std::thread::sleep,
        )
    }
}

impl SHAMapStoreRelationalStore for SqliteSHAMapStoreRelational {
    fn minimum_ledger_seq(&self) -> Result<Option<u32>, String> {
        minimum_seq(&self.ledger_db, "Ledgers")
    }

    fn delete_ledgers_before(&self, ledger_seq: u32) -> Result<(), String> {
        delete_before_ledger_seq(&self.ledger_db, "Ledgers", ledger_seq)
    }

    fn transactions_min_ledger_seq(&self) -> Result<Option<u32>, String> {
        match &self.transaction_db {
            Some(db) => minimum_seq(db, "Transactions"),
            None => Ok(None),
        }
    }

    fn delete_transactions_before(&self, ledger_seq: u32) -> Result<(), String> {
        match &self.transaction_db {
            Some(db) => delete_before_ledger_seq(db, "Transactions", ledger_seq),
            None => Ok(()),
        }
    }

    fn account_transactions_min_ledger_seq(&self) -> Result<Option<u32>, String> {
        match &self.transaction_db {
            Some(db) => minimum_seq(db, "AccountTransactions"),
            None => Ok(None),
        }
    }

    fn delete_account_transactions_before(&self, ledger_seq: u32) -> Result<(), String> {
        match &self.transaction_db {
            Some(db) => delete_before_ledger_seq(db, "AccountTransactions", ledger_seq),
            None => Ok(()),
        }
    }
}

fn clear_table<GetMinSeq, DeleteBeforeSeq, Sleep>(
    last_rotated: u32,
    get_min_seq: GetMinSeq,
    delete_before_seq: DeleteBeforeSeq,
    delete_batch: u32,
    back_off: Duration,
    should_stop: &dyn Fn() -> bool,
    sleep: Sleep,
) -> Result<(), String>
where
    GetMinSeq: Fn() -> Result<Option<u32>, String>,
    DeleteBeforeSeq: Fn(u32) -> Result<(), String>,
    Sleep: FnMut(Duration) + Copy,
{
    let error = std::cell::RefCell::new(None);
    let should_stop_or_error = || should_stop() || error.borrow().is_some();
    clear_sql_batches(
        last_rotated,
        delete_batch,
        back_off,
        || match get_min_seq() {
            Ok(value) => value,
            Err(next_error) => {
                *error.borrow_mut() = Some(next_error);
                None
            }
        },
        |ledger_seq| {
            if error.borrow().is_none() {
                if let Err(next_error) = delete_before_seq(ledger_seq) {
                    *error.borrow_mut() = Some(next_error);
                }
            }
        },
        should_stop_or_error,
        sleep,
    );

    match error.into_inner() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn minimum_seq(db: &DatabaseCon, table_name: &str) -> Result<Option<u32>, String> {
    let connection = db.get_session();
    connection
        .query_row(
            &format!("SELECT MIN(LedgerSeq) FROM {table_name}"),
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn delete_before_ledger_seq(
    db: &DatabaseCon,
    table_name: &str,
    ledger_seq: u32,
) -> Result<(), String> {
    let connection = db.get_session();
    connection
        .execute(
            &format!("DELETE FROM {table_name} WHERE LedgerSeq < ?1"),
            params![ledger_seq],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        SHAMapStoreRelationalRuntime, SqliteSHAMapStoreRelational, clear_relational_prior,
    };
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;
    use xrpld_core::{DatabaseCon, LEDGER_DB_INIT, TRANSACTION_DB_INIT};

    #[derive(Default)]
    struct RecordingStore {
        events: Mutex<Vec<String>>,
        ledger_min: Option<u32>,
        transactions_min: Option<u32>,
        account_transactions_min: Option<u32>,
    }

    impl super::SHAMapStoreRelationalStore for RecordingStore {
        fn minimum_ledger_seq(&self) -> Result<Option<u32>, String> {
            self.events
                .lock()
                .expect("events mutex must not be poisoned")
                .push("min:Ledgers".to_owned());
            Ok(self.ledger_min)
        }

        fn delete_ledgers_before(&self, ledger_seq: u32) -> Result<(), String> {
            self.events
                .lock()
                .expect("events mutex must not be poisoned")
                .push(format!("delete:Ledgers:{ledger_seq}"));
            Ok(())
        }

        fn transactions_min_ledger_seq(&self) -> Result<Option<u32>, String> {
            self.events
                .lock()
                .expect("events mutex must not be poisoned")
                .push("min:Transactions".to_owned());
            Ok(self.transactions_min)
        }

        fn delete_transactions_before(&self, ledger_seq: u32) -> Result<(), String> {
            self.events
                .lock()
                .expect("events mutex must not be poisoned")
                .push(format!("delete:Transactions:{ledger_seq}"));
            Ok(())
        }

        fn account_transactions_min_ledger_seq(&self) -> Result<Option<u32>, String> {
            self.events
                .lock()
                .expect("events mutex must not be poisoned")
                .push("min:AccountTransactions".to_owned());
            Ok(self.account_transactions_min)
        }

        fn delete_account_transactions_before(&self, ledger_seq: u32) -> Result<(), String> {
            self.events
                .lock()
                .expect("events mutex must not be poisoned")
                .push(format!("delete:AccountTransactions:{ledger_seq}"));
            Ok(())
        }
    }

    fn insert_seq(db: &DatabaseCon, table_name: &str, seq: u32) {
        let connection = db.get_session();
        let mut hash = vec![0_u8; 32];
        hash[0..4].copy_from_slice(&seq.to_be_bytes());
        match table_name {
            "Ledgers" => {
                connection
                    .execute(
                        "INSERT INTO Ledgers (LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        rusqlite::params![hash.clone(), seq, vec![0_u8; 32], "0", 0_i64, 0_i64, 0_i64, 0_i64, vec![0_u8; 32], vec![0_u8; 32]],
                    )
                    .expect("ledger insert");
            }
            "Transactions" => {
                connection
                    .execute(
                        "INSERT INTO Transactions (TransID, TransType, FromAcct, FromSeq, LedgerSeq, Status, RawTxn, TxnMeta) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        rusqlite::params![hash.clone(), "Payment", vec![0_u8; 20], 1_i64, seq, "tesSUCCESS", vec![0_u8; 1], vec![0_u8; 1]],
                    )
                    .expect("transaction insert");
            }
            "AccountTransactions" => {
                connection
                    .execute(
                        "INSERT INTO AccountTransactions (TransID, Account, LedgerSeq, TxnSeq) VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![hash, vec![0_u8; 20], seq, 1_i64],
                    )
                    .expect("account transaction insert");
            }
            _ => panic!("unexpected table"),
        }
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
    fn clear_relational_prior_table_order_and_tx_table_gate() {
        let store = RecordingStore {
            ledger_min: Some(800),
            transactions_min: Some(810),
            account_transactions_min: Some(820),
            ..RecordingStore::default()
        };

        clear_relational_prior(
            &store,
            900,
            false,
            100,
            Duration::from_secs(0),
            &|| false,
            |_| {},
        )
        .expect("clear should succeed");
        assert_eq!(
            *store
                .events
                .lock()
                .expect("events mutex must not be poisoned"),
            vec!["min:Ledgers".to_owned(), "delete:Ledgers:900".to_owned(),]
        );
    }

    #[test]
    fn clear_relational_prior_visits_transactions_before_account_transactions() {
        let store = RecordingStore {
            ledger_min: Some(800),
            transactions_min: Some(810),
            account_transactions_min: Some(820),
            ..RecordingStore::default()
        };

        clear_relational_prior(
            &store,
            900,
            true,
            100,
            Duration::from_secs(0),
            &|| false,
            |_| {},
        )
        .expect("clear should succeed");
        assert_eq!(
            *store
                .events
                .lock()
                .expect("events mutex must not be poisoned"),
            vec![
                "min:Ledgers".to_owned(),
                "delete:Ledgers:900".to_owned(),
                "min:Transactions".to_owned(),
                "delete:Transactions:900".to_owned(),
                "min:AccountTransactions".to_owned(),
                "delete:AccountTransactions:900".to_owned(),
            ]
        );
    }

    #[test]
    fn clear_relational_prior_stops_on_first_delete_error_throw_path() {
        struct FailingDeleteStore;

        impl super::SHAMapStoreRelationalStore for FailingDeleteStore {
            fn minimum_ledger_seq(&self) -> Result<Option<u32>, String> {
                Ok(Some(800))
            }

            fn delete_ledgers_before(&self, _ledger_seq: u32) -> Result<(), String> {
                Err("delete failed".to_owned())
            }

            fn transactions_min_ledger_seq(&self) -> Result<Option<u32>, String> {
                panic!("transaction table should not be queried after a fatal delete error")
            }

            fn delete_transactions_before(&self, _ledger_seq: u32) -> Result<(), String> {
                panic!("transaction table should not be deleted after a fatal delete error")
            }

            fn account_transactions_min_ledger_seq(&self) -> Result<Option<u32>, String> {
                panic!("account transaction table should not be queried after a fatal delete error")
            }

            fn delete_account_transactions_before(&self, _ledger_seq: u32) -> Result<(), String> {
                panic!("account transaction table should not be deleted after a fatal delete error")
            }
        }

        let error = clear_relational_prior(
            &FailingDeleteStore,
            900,
            true,
            100,
            Duration::from_secs(0),
            &|| false,
            |_| {},
        )
        .expect_err("delete failure should surface immediately");

        assert_eq!(error, "delete failed");
    }

    #[test]
    fn sqlite_relational_runtime_uses_real_minimum_sql_seq_and_deletes_in_batches() {
        let temp = TempDir::new().expect("tempdir");
        let ledger_db = Arc::new(
            DatabaseCon::new_at_path(temp.path(), "ledger.db", &[], LEDGER_DB_INIT)
                .expect("ledger db"),
        );
        let transaction_db = Arc::new(
            DatabaseCon::new_at_path(temp.path(), "transaction.db", &[], TRANSACTION_DB_INIT)
                .expect("transaction db"),
        );
        insert_seq(&ledger_db, "Ledgers", 800);
        insert_seq(&ledger_db, "Ledgers", 850);
        insert_seq(&ledger_db, "Ledgers", 950);
        insert_seq(&transaction_db, "Transactions", 810);
        insert_seq(&transaction_db, "Transactions", 910);
        insert_seq(&transaction_db, "AccountTransactions", 820);
        insert_seq(&transaction_db, "AccountTransactions", 920);

        let relational = SqliteSHAMapStoreRelational::new(
            Arc::clone(&ledger_db),
            Some(Arc::clone(&transaction_db)),
            true,
            50,
            Duration::from_secs(0),
        );

        assert_eq!(relational.minimum_sql_seq(), Some(800));
        relational
            .clear_prior(900, &|| false)
            .expect("clear should succeed");

        assert_eq!(count_rows(&ledger_db, "Ledgers"), 1);
        assert_eq!(count_rows(&transaction_db, "Transactions"), 1);
        assert_eq!(count_rows(&transaction_db, "AccountTransactions"), 1);
        assert_eq!(relational.minimum_sql_seq(), Some(950));
    }
}
