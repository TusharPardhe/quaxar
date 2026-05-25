use app::{
    NullSHAMapStoreCopyRuntime, SHAMapStore, SHAMapStoreAppRuntime, SHAMapStoreCopyRuntime,
    SHAMapStoreLedgerRuntime, SHAMapStoreNodeFamilyCacheRuntime, SHAMapStoreNodeStoreRuntime,
    SHAMapStoreOperatingMode, SHAMapStoreRelationalRuntime, SHAMapStoreRotatingBackendFactory,
    SHAMapStoreSavedState, SHAMapStoreSavedStateDb, SHAMapStoreTransactionCacheRuntime,
    SqliteSHAMapStoreRelational, run_shamap_store_worker_step,
};
use basics::base_uint::Uint256;
use basics::basic_config::BasicConfig;
use ledger::Ledger;
use nodestore::{Backend, Batch, NodeObject, Status};
use shamap::traversal::TraversalError;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use xrpld_core::{DatabaseCon, LEDGER_DB_INIT, TRANSACTION_DB_INIT};

#[derive(Default)]
struct RecordingLedgerRuntime {
    clear_prior: Mutex<Vec<u32>>,
    clear_caches: Mutex<Vec<u32>>,
}

impl RecordingLedgerRuntime {
    fn clear_prior_calls(&self) -> Vec<u32> {
        self.clear_prior
            .lock()
            .expect("clear_prior mutex must not be poisoned")
            .clone()
    }

    fn clear_cache_calls(&self) -> Vec<u32> {
        self.clear_caches
            .lock()
            .expect("clear_caches mutex must not be poisoned")
            .clone()
    }
}

impl SHAMapStoreLedgerRuntime for RecordingLedgerRuntime {
    fn clear_prior_ledgers(&self, last_rotated: u32) {
        self.clear_prior
            .lock()
            .expect("clear_prior mutex must not be poisoned")
            .push(last_rotated);
    }

    fn clear_online_delete_caches(&self, validated_seq: u32) {
        self.clear_caches
            .lock()
            .expect("clear_caches mutex must not be poisoned")
            .push(validated_seq);
    }
}

#[derive(Default)]
struct EmptyNodeFamilyRuntime {
    clear_full_below_calls: Mutex<u32>,
}

impl EmptyNodeFamilyRuntime {
    fn clear_full_below_calls(&self) -> u32 {
        *self
            .clear_full_below_calls
            .lock()
            .expect("full-below mutex must not be poisoned")
    }
}

impl SHAMapStoreNodeFamilyCacheRuntime for EmptyNodeFamilyRuntime {
    fn tree_node_cache_keys(&self) -> Vec<Uint256> {
        Vec::new()
    }

    fn clear_full_below_cache(&self) {
        *self
            .clear_full_below_calls
            .lock()
            .expect("full-below mutex must not be poisoned") += 1;
    }

    fn visit_state_map_hashes(
        &self,
        _ledger: &Ledger,
        _visit: &mut dyn FnMut(Uint256) -> bool,
    ) -> Result<(), TraversalError> {
        Ok(())
    }
}

#[derive(Default)]
struct EmptyTransactionCacheRuntime;

impl SHAMapStoreTransactionCacheRuntime for EmptyTransactionCacheRuntime {
    fn cache_keys(&self) -> Vec<Uint256> {
        Vec::new()
    }
}

#[derive(Default)]
struct RotatingNodeStoreState {
    current_writable: Mutex<String>,
    current_archive: Mutex<String>,
    rotations: Mutex<Vec<(String, String)>>,
}

impl RotatingNodeStoreState {
    fn seed(&self, writable: impl Into<String>, archive: impl Into<String>) {
        *self
            .current_writable
            .lock()
            .expect("current writable mutex must not be poisoned") = writable.into();
        *self
            .current_archive
            .lock()
            .expect("current archive mutex must not be poisoned") = archive.into();
    }

    fn rotations(&self) -> Vec<(String, String)> {
        self.rotations
            .lock()
            .expect("rotations mutex must not be poisoned")
            .clone()
    }

    fn current_names(&self) -> (String, String) {
        (
            self.current_writable
                .lock()
                .expect("current writable mutex must not be poisoned")
                .clone(),
            self.current_archive
                .lock()
                .expect("current archive mutex must not be poisoned")
                .clone(),
        )
    }
}

impl SHAMapStoreNodeStoreRuntime for RotatingNodeStoreState {
    fn fetch_node_object(&self, _hash: &Uint256, _ledger_seq: u32) -> bool {
        true
    }

    fn rotate_with(&self, new_backend: Box<dyn Backend>) -> (String, String) {
        let new_writable = new_backend.get_name();
        let mut current_writable = self
            .current_writable
            .lock()
            .expect("current writable mutex must not be poisoned");
        let previous_writable = std::mem::replace(&mut *current_writable, new_writable.clone());
        *self
            .current_archive
            .lock()
            .expect("current archive mutex must not be poisoned") = previous_writable.clone();
        self.rotations
            .lock()
            .expect("rotations mutex must not be poisoned")
            .push((new_writable.clone(), previous_writable.clone()));
        (new_writable, previous_writable)
    }
}

struct NamedBackend {
    name: String,
}

impl NamedBackend {
    fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Backend for NamedBackend {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn open(&self, _create_if_missing: bool) -> Result<(), String> {
        Ok(())
    }

    fn is_open(&self) -> bool {
        true
    }

    fn close(&self) -> Result<(), String> {
        Ok(())
    }

    fn fetch(&self, _hash: &Uint256) -> (Option<Arc<NodeObject>>, Status) {
        (None, Status::NotFound)
    }

    fn fetch_batch(&self, _hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status) {
        (Vec::new(), Status::NotFound)
    }

    fn store(&self, _object: Arc<NodeObject>) {}

    fn store_batch(&self, _batch: &Batch) {}

    fn sync(&self) {}

    fn for_each(&self, _callback: &mut dyn FnMut(Arc<NodeObject>)) {}

    fn get_write_load(&self) -> i32 {
        0
    }

    fn set_delete_path(&self) {}

    fn fd_required(&self) -> i32 {
        1
    }
}

#[derive(Debug)]
struct QueueBackendFactory {
    names: Mutex<VecDeque<String>>,
}

impl QueueBackendFactory {
    fn new(names: impl IntoIterator<Item = String>) -> Self {
        Self {
            names: Mutex::new(names.into_iter().collect()),
        }
    }
}

impl SHAMapStoreRotatingBackendFactory for QueueBackendFactory {
    fn make_backend(&self) -> Result<Box<dyn Backend>, String> {
        let Some(name) = self
            .names
            .lock()
            .expect("backend names mutex must not be poisoned")
            .pop_front()
        else {
            return Err("no prepared backend names remain".to_owned());
        };
        Ok(Box::new(NamedBackend::new(name)))
    }
}

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

fn insert_transaction_seq(db: &DatabaseCon, seq: u32) {
    let connection = db.get_session();
    let mut hash = vec![0_u8; 32];
    hash[0..4].copy_from_slice(&seq.to_be_bytes());
    connection
        .execute(
            "INSERT INTO Transactions (TransID, TransType, FromAcct, FromSeq, LedgerSeq, Status, RawTxn, TxnMeta) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![hash.clone(), "Payment", vec![0_u8; 20], 1_i64, seq, "tesSUCCESS", vec![0_u8; 1], vec![0_u8; 1]],
        )
        .expect("transaction insert");
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

fn minimum_seq(db: &DatabaseCon, table_name: &str) -> Option<u32> {
    let connection = db.get_session();
    connection
        .query_row(
            &format!("SELECT MIN(LedgerSeq) FROM {table_name}"),
            [],
            |row| row.get(0),
        )
        .expect("min query")
}

struct OnlineDeleteHarness {
    _temp: TempDir,
    store: SHAMapStore,
    runtime: SHAMapStoreAppRuntime,
    state_db: Arc<SHAMapStoreSavedStateDb>,
    ledger_db: Arc<DatabaseCon>,
    transaction_db: Arc<DatabaseCon>,
    ledger_runtime: Arc<RecordingLedgerRuntime>,
    node_family: Arc<EmptyNodeFamilyRuntime>,
    node_store: Arc<RotatingNodeStoreState>,
}

impl OnlineDeleteHarness {
    fn new(
        delete_interval: u32,
        advisory_delete: bool,
        use_tx_tables: bool,
        initial_can_delete: u32,
    ) -> Self {
        let temp = TempDir::new().expect("tempdir");
        let sql_dir = temp.path().join("sql");
        std::fs::create_dir_all(&sql_dir).expect("sql dir");

        let mut config = BasicConfig::new();
        config.set_legacy("database_path", sql_dir.to_string_lossy());
        let state_db = Arc::new(
            SHAMapStoreSavedStateDb::open(&config, "state").expect("state db should initialize"),
        );
        state_db
            .set_state(&SHAMapStoreSavedState {
                writable_db: "xrpldb.0000".to_owned(),
                archive_db: "xrpldb.0001".to_owned(),
                last_rotated: 0,
            })
            .expect("initial saved state");
        state_db
            .set_can_delete(initial_can_delete)
            .expect("initial can_delete");

        let ledger_db = Arc::new(
            DatabaseCon::new_at_path(&sql_dir, "ledger.db", &[], LEDGER_DB_INIT)
                .expect("ledger db"),
        );
        let transaction_db = Arc::new(
            DatabaseCon::new_at_path(&sql_dir, "transaction.db", &[], TRANSACTION_DB_INIT)
                .expect("transaction db"),
        );

        let relational = Arc::new(SqliteSHAMapStoreRelational::new(
            Arc::clone(&ledger_db),
            Some(Arc::clone(&transaction_db)),
            use_tx_tables,
            100,
            Duration::from_secs(0),
        ));

        let ledger_runtime: Arc<RecordingLedgerRuntime> = Arc::default();
        let node_family: Arc<EmptyNodeFamilyRuntime> = Arc::default();
        let node_store: Arc<RotatingNodeStoreState> = Arc::default();
        node_store.seed("xrpldb.0000", "xrpldb.0001");

        let mut store = SHAMapStore::new(delete_interval, advisory_delete, 9);
        store.set_saved_state(state_db.get_state().expect("saved state should load"));
        if advisory_delete {
            store.set_can_delete(
                state_db
                    .get_can_delete()
                    .expect("can_delete state should load"),
            );
        }

        let mut runtime = SHAMapStoreAppRuntime::new(
            Arc::clone(&ledger_runtime) as Arc<dyn SHAMapStoreLedgerRuntime>,
            Arc::clone(&node_family) as Arc<dyn SHAMapStoreNodeFamilyCacheRuntime>,
            Arc::new(EmptyTransactionCacheRuntime) as Arc<dyn SHAMapStoreTransactionCacheRuntime>,
            Arc::clone(&node_store) as Arc<dyn SHAMapStoreNodeStoreRuntime>,
            Arc::new(QueueBackendFactory::new(
                (2..32).map(|index| format!("xrpldb.{index:04}")),
            )) as Arc<dyn SHAMapStoreRotatingBackendFactory>,
            Some(relational as Arc<dyn SHAMapStoreRelationalRuntime>),
            Arc::new(NullSHAMapStoreCopyRuntime) as Arc<dyn SHAMapStoreCopyRuntime>,
        );
        runtime.set_operating_mode(SHAMapStoreOperatingMode::Full);
        runtime.set_validated_ledger_age(Duration::from_secs(1));

        Self {
            _temp: temp,
            store,
            runtime,
            state_db,
            ledger_db,
            transaction_db,
            ledger_runtime,
            node_family,
            node_store,
        }
    }

    fn queue_ledger(&mut self, seq: u32) {
        insert_ledger_seq(&self.ledger_db, seq);
        insert_transaction_seq(&self.transaction_db, seq);
        self.store
            .on_ledger_closed(Arc::new(Ledger::from_ledger_seq_and_close_time(
                seq, 0, false,
            )));
    }

    fn step(&mut self) -> app::SHAMapStoreWorkerStep {
        run_shamap_store_worker_step(&mut self.store, &mut self.runtime, Some(&self.state_db))
            .expect("worker step should succeed")
            .expect("queued ledger should produce a step")
    }

    fn queue_and_step(&mut self, seq: u32) -> app::SHAMapStoreWorkerStep {
        self.queue_ledger(seq);
        self.step()
    }

    fn set_can_delete(&mut self, can_delete: u32) {
        self.state_db
            .set_can_delete(can_delete)
            .expect("set can_delete");
        self.store.set_can_delete(can_delete);
    }

    fn saved_state(&self) -> SHAMapStoreSavedState {
        self.state_db.get_state().expect("saved state")
    }

    fn can_delete(&self) -> u32 {
        self.state_db.get_can_delete().expect("can_delete")
    }
}

#[test]
fn shamap_store_online_delete_automatic_rotation_prunes_sql_windows() {
    let mut harness = OnlineDeleteHarness::new(8, false, true, 0);

    harness.queue_ledger(2);
    let initialized = harness.queue_and_step(3);
    assert_eq!(initialized.runloop.decision.last_rotated, 3);
    assert!(!initialized.rotated);
    assert_eq!(initialized.minimum_online, Some(2));
    assert_eq!(harness.saved_state().last_rotated, 3);

    for seq in 4..11 {
        let step = harness.queue_and_step(seq);
        assert!(
            !step.rotated,
            "automatic online delete should not rotate before seq {seq}"
        );
    }

    assert_eq!(count_rows(&harness.ledger_db, "Ledgers"), 9);
    assert_eq!(count_rows(&harness.transaction_db, "Transactions"), 9);
    assert_eq!(
        count_rows(&harness.transaction_db, "AccountTransactions"),
        9
    );

    let first_rotation = harness.queue_and_step(11);
    assert!(first_rotation.rotated);
    assert_eq!(first_rotation.minimum_online, Some(4));
    assert_eq!(harness.saved_state().last_rotated, 11);
    assert_eq!(count_rows(&harness.ledger_db, "Ledgers"), 9);
    assert_eq!(count_rows(&harness.transaction_db, "Transactions"), 9);
    assert_eq!(
        count_rows(&harness.transaction_db, "AccountTransactions"),
        9
    );
    assert_eq!(minimum_seq(&harness.ledger_db, "Ledgers"), Some(3));

    for seq in 12..19 {
        let step = harness.queue_and_step(seq);
        assert!(
            !step.rotated,
            "second automatic rotation should not fire before seq {seq}"
        );
    }

    let second_rotation = harness.queue_and_step(19);
    assert!(second_rotation.rotated);
    assert_eq!(second_rotation.minimum_online, Some(12));
    assert_eq!(harness.saved_state().last_rotated, 19);
    assert_eq!(count_rows(&harness.ledger_db, "Ledgers"), 9);
    assert_eq!(count_rows(&harness.transaction_db, "Transactions"), 9);
    assert_eq!(
        count_rows(&harness.transaction_db, "AccountTransactions"),
        9
    );
    assert_eq!(minimum_seq(&harness.ledger_db, "Ledgers"), Some(11));
    assert_eq!(
        harness.ledger_runtime.clear_prior_calls(),
        vec![3, 11],
        "clearPrior should run with the previous rotation boundary"
    );
    assert_eq!(
        harness.ledger_runtime.clear_cache_calls(),
        vec![11, 11, 19, 19],
        "clearCaches should run before and after backend swap"
    );
    assert_eq!(harness.node_family.clear_full_below_calls(), 4);
    assert_eq!(
        harness.node_store.rotations(),
        vec![
            ("xrpldb.0002".to_owned(), "xrpldb.0000".to_owned()),
            ("xrpldb.0003".to_owned(), "xrpldb.0002".to_owned()),
        ]
    );
    assert_eq!(
        harness.node_store.current_names(),
        ("xrpldb.0003".to_owned(), "xrpldb.0002".to_owned())
    );
    assert_eq!(
        harness.saved_state(),
        SHAMapStoreSavedState {
            writable_db: "xrpldb.0003".to_owned(),
            archive_db: "xrpldb.0002".to_owned(),
            last_rotated: 19,
        }
    );
}

#[test]
fn shamap_store_online_delete_advisory_gate_blocks_and_releases_rotation() {
    let mut harness = OnlineDeleteHarness::new(8, true, true, 0);

    harness.queue_ledger(2);
    let initialized = harness.queue_and_step(3);
    assert_eq!(initialized.runloop.decision.last_rotated, 3);
    assert_eq!(harness.saved_state().last_rotated, 3);
    assert_eq!(harness.can_delete(), 0);

    for seq in 4..12 {
        let step = harness.queue_and_step(seq);
        assert!(
            !step.rotated,
            "advisory delete should block the first rotation at seq {seq}"
        );
    }
    assert_eq!(harness.saved_state().last_rotated, 3);
    assert_eq!(count_rows(&harness.ledger_db, "Ledgers"), 10);

    harness.set_can_delete(2);
    let first_rotation = harness.queue_and_step(12);
    assert!(first_rotation.rotated);
    assert_eq!(first_rotation.minimum_online, Some(4));
    assert_eq!(harness.saved_state().last_rotated, 12);
    assert_eq!(harness.can_delete(), 2);
    assert_eq!(count_rows(&harness.ledger_db, "Ledgers"), 10);
    assert_eq!(minimum_seq(&harness.ledger_db, "Ledgers"), Some(3));

    for seq in 13..21 {
        let step = harness.queue_and_step(seq);
        assert!(
            !step.rotated,
            "stale advisory can_delete should block the second rotation at seq {seq}"
        );
    }
    assert_eq!(harness.saved_state().last_rotated, 12);
    assert_eq!(count_rows(&harness.ledger_db, "Ledgers"), 18);

    harness.set_can_delete(11);
    let second_rotation = harness.queue_and_step(21);
    assert!(second_rotation.rotated);
    assert_eq!(second_rotation.minimum_online, Some(13));
    assert_eq!(harness.saved_state().last_rotated, 21);
    assert_eq!(harness.can_delete(), 11);
    assert_eq!(count_rows(&harness.ledger_db, "Ledgers"), 10);
    assert_eq!(count_rows(&harness.transaction_db, "Transactions"), 10);
    assert_eq!(
        count_rows(&harness.transaction_db, "AccountTransactions"),
        10
    );
    assert_eq!(minimum_seq(&harness.ledger_db, "Ledgers"), Some(12));
    assert_eq!(harness.ledger_runtime.clear_prior_calls(), vec![3, 12]);
    assert_eq!(
        harness.node_store.rotations(),
        vec![
            ("xrpldb.0002".to_owned(), "xrpldb.0000".to_owned()),
            ("xrpldb.0003".to_owned(), "xrpldb.0002".to_owned()),
        ]
    );
}

#[test]
fn shamap_store_online_delete_clear_prior_skips_tx_tables_when_disabled() {
    let mut harness = OnlineDeleteHarness::new(8, false, false, 0);

    harness.queue_ledger(2);
    let _ = harness.queue_and_step(3);
    for seq in 4..11 {
        let _ = harness.queue_and_step(seq);
    }

    let rotation = harness.queue_and_step(11);
    assert!(rotation.rotated);
    assert_eq!(harness.saved_state().last_rotated, 11);
    assert_eq!(count_rows(&harness.ledger_db, "Ledgers"), 9);
    assert_eq!(
        count_rows(&harness.transaction_db, "Transactions"),
        10,
        "Transactions should remain untouched when use_tx_tables is disabled"
    );
    assert_eq!(
        count_rows(&harness.transaction_db, "AccountTransactions"),
        10,
        "AccountTransactions should remain untouched when use_tx_tables is disabled"
    );
}

#[test]
fn shamap_store_online_delete_persists_first_and_rotated_state_boundaries() {
    let mut harness = OnlineDeleteHarness::new(8, false, true, 0);

    harness.queue_ledger(2);
    let first = harness.queue_and_step(3);
    assert!(!first.rotated);
    assert_eq!(
        harness.saved_state(),
        SHAMapStoreSavedState {
            writable_db: "xrpldb.0000".to_owned(),
            archive_db: "xrpldb.0001".to_owned(),
            last_rotated: 3,
        }
    );

    for seq in 4..11 {
        let _ = harness.queue_and_step(seq);
    }

    let rotated = harness.queue_and_step(11);
    assert!(rotated.rotated);
    assert_eq!(
        harness.saved_state(),
        SHAMapStoreSavedState {
            writable_db: "xrpldb.0002".to_owned(),
            archive_db: "xrpldb.0000".to_owned(),
            last_rotated: 11,
        }
    );
}
