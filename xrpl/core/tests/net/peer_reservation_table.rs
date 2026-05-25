use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use protocol::PublicKey;
use serde_json::json;
use tempfile::TempDir;
use xrpl_core::{
    PeerReservation, PeerReservationJournal, PeerReservationStore, PeerReservationTable,
    ServiceRegistry, load_peer_reservations_from_registry,
};
use xrpld_core::{DatabaseCon, WALLET_DB_INIT};

fn hash_value<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn peer_reservation_identity_key_semantics() {
    let first = PeerReservation::new(vec![1, 2, 3], "first");
    let second = PeerReservation::new(vec![1, 2, 3], "replacement");
    let different = PeerReservation::new(vec![1, 2, 4], "first");

    assert_eq!(first, second);
    assert!(first < different);
    assert_eq!(hash_value(&first), hash_value(&second));
}

#[test]
fn peer_reservation_table_replace_list_and_erase_shape() {
    let table = PeerReservationTable::new();
    let first = PeerReservation::new(vec![9, 9, 9], "first");
    let replacement = PeerReservation::new(vec![9, 9, 9], "replacement");
    let other = PeerReservation::new(vec![1, 2, 3], "other");

    assert!(!table.contains(&first.node_id));
    assert!(table.insert_or_assign(first.clone()).is_none());
    assert!(table.contains(&first.node_id));
    assert_eq!(table.list(), vec![first.clone()]);

    assert_eq!(table.insert_or_assign(replacement.clone()), Some(first));
    assert_eq!(table.insert_or_assign(other.clone()), None);
    assert_eq!(table.list(), vec![other.clone(), replacement.clone()]);
    assert_eq!(table.erase(&replacement.node_id), Some(replacement));
    assert_eq!(table.erase(&vec![7]), None);
    assert_eq!(table.list(), vec![other]);
}

#[test]
fn peer_reservation_table_mutates_through_shared_owner() {
    let table = PeerReservationTable::new();
    let reservation = PeerReservation::new(vec![7, 7, 7], "shared-owner");

    assert!(table.insert_or_assign(reservation.clone()).is_none());
    assert!(table.contains(&reservation.node_id));
    assert_eq!(table.erase(&reservation.node_id), Some(reservation));
}

#[test]
fn peer_reservation_to_json_node_public_shape() {
    let node_id = PublicKey::from_bytes([
        0x03, 0xD4, 0x9C, 0x56, 0xE1, 0xB1, 0x85, 0xF1, 0xBE, 0x89, 0x9A, 0xE6, 0x6A, 0x02, 0xEF,
        0xC1, 0x7F, 0x78, 0xEA, 0x6F, 0xC5, 0x3A, 0xF8, 0x5E, 0x0F, 0xE5, 0x4C, 0x6E, 0x8B, 0x7F,
        0x8C, 0x71, 0xA8,
    ]);
    let reservation = PeerReservation::new(node_id, "trusted peer");

    assert_eq!(
        reservation.to_json(),
        json!({
            "node": "n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9",
            "description": "trusted peer",
        })
    );
}

#[test]
fn peer_reservation_to_json_omits_empty_description() {
    let node_id = PublicKey::from_bytes([
        0x03, 0xD4, 0x9C, 0x56, 0xE1, 0xB1, 0x85, 0xF1, 0xBE, 0x89, 0x9A, 0xE6, 0x6A, 0x02, 0xEF,
        0xC1, 0x7F, 0x78, 0xEA, 0x6F, 0xC5, 0x3A, 0xF8, 0x5E, 0x0F, 0xE5, 0x4C, 0x6E, 0x8B, 0x7F,
        0x8C, 0x71, 0xA8,
    ]);
    let reservation = PeerReservation::new(node_id, "");

    assert_eq!(
        reservation.to_json(),
        json!({
            "node": "n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9",
        })
    );
}

fn sample_node_public() -> PublicKey {
    PublicKey::from_bytes([
        0x03, 0xD4, 0x9C, 0x56, 0xE1, 0xB1, 0x85, 0xF1, 0xBE, 0x89, 0x9A, 0xE6, 0x6A, 0x02, 0xEF,
        0xC1, 0x7F, 0x78, 0xEA, 0x6F, 0xC5, 0x3A, 0xF8, 0x5E, 0x0F, 0xE5, 0x4C, 0x6E, 0x8B, 0x7F,
        0x8C, 0x71, 0xA8,
    ])
}

fn other_node_public() -> PublicKey {
    PublicKey::from_bytes([
        0x02, 0x7A, 0x2F, 0x76, 0x83, 0x7B, 0x5E, 0x44, 0x11, 0x29, 0xCF, 0xD3, 0x7B, 0x8F, 0x21,
        0x74, 0x2A, 0xBC, 0xD5, 0x0D, 0x48, 0x36, 0x41, 0x99, 0xAD, 0xD7, 0xB0, 0x1E, 0xCB, 0xD8,
        0x51, 0x5B, 0x73,
    ])
}

fn wallet_db() -> (TempDir, Arc<DatabaseCon>) {
    let temp = TempDir::new().expect("tempdir");
    let db = Arc::new(
        DatabaseCon::new_at_path(temp.path(), "wallet.db", &[], WALLET_DB_INIT).expect("wallet db"),
    );
    (temp, db)
}

fn persisted_reservations(db: &DatabaseCon) -> Vec<(String, String)> {
    let connection = db.get_session();
    let mut statement = connection
        .prepare("SELECT PublicKey, Description FROM PeerReservations ORDER BY PublicKey")
        .expect("statement");
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .expect("query");

    rows.map(|row| row.expect("row")).collect()
}

#[derive(Default)]
struct RecordingJournal {
    entries: Mutex<Vec<String>>,
}

impl PeerReservationJournal for RecordingJournal {
    fn warn(&self, message: &str) {
        self.entries
            .lock()
            .expect("recording journal poisoned")
            .push(message.to_owned());
    }
}

#[test]
fn peer_reservation_table_load_reads_wallet_db_and_skips_invalid_keys() {
    let (_temp, db) = wallet_db();
    {
        let connection = db.checkout_db();
        connection
            .execute(
                "INSERT INTO PeerReservations (PublicKey, Description) VALUES (?1, ?2)",
                [
                    sample_node_public().to_node_public_base58(),
                    "valid".to_owned(),
                ],
            )
            .expect("insert valid row");
        connection
            .execute(
                "INSERT INTO PeerReservations (PublicKey, Description) VALUES (?1, ?2)",
                ["not-a-node-public".to_owned(), "invalid".to_owned()],
            )
            .expect("insert invalid row");
    }

    let table = PeerReservationTable::new();
    assert!(table.load_from_database(Arc::clone(&db)).expect("load"));
    assert_eq!(
        table.list(),
        vec![PeerReservation::new(sample_node_public(), "valid")]
    );
}

#[test]
fn peer_reservation_table_load_warns_when_persisted_node_public_is_invalid() {
    let (_temp, db) = wallet_db();
    {
        let connection = db.checkout_db();
        connection
            .execute(
                "INSERT INTO PeerReservations (PublicKey, Description) VALUES (?1, ?2)",
                ["not-a-node-public".to_owned(), "invalid".to_owned()],
            )
            .expect("insert invalid row");
    }

    let journal = Arc::new(RecordingJournal::default());
    let table = PeerReservationTable::new();
    assert!(
        table
            .load_from_database_with_journal(Arc::clone(&db), journal.clone())
            .expect("load")
    );
    assert!(table.list().is_empty());
    assert_eq!(
        journal
            .entries
            .lock()
            .expect("recording journal poisoned")
            .as_slice(),
        ["load: not a public key: not-a-node-public"]
    );
}

#[test]
fn peer_reservation_table_persists_replace_and_erase_after_load() {
    let (_temp, db) = wallet_db();
    let table = PeerReservationTable::new();
    assert!(table.load_from_database(Arc::clone(&db)).expect("load"));

    let first = PeerReservation::new(sample_node_public(), "first");
    let replacement = PeerReservation::new(sample_node_public(), "replacement");
    let other = PeerReservation::new(other_node_public(), "other");

    assert_eq!(table.insert_or_assign(first), None);
    assert_eq!(
        table
            .insert_or_assign(replacement.clone())
            .map(|p| p.description),
        Some("first".to_owned())
    );
    assert_eq!(table.insert_or_assign(other.clone()), None);
    assert_eq!(table.erase(&replacement.node_id), Some(replacement));

    assert_eq!(
        persisted_reservations(&db),
        vec![(
            other.node_id.to_node_public_base58(),
            other.description.clone()
        )]
    );
}

#[test]
fn peer_reservation_table_public_key_insert_requires_load_two_phase_owner() {
    let table = PeerReservationTable::new();
    let error = table
        .try_insert_or_assign(PeerReservation::new(sample_node_public(), "pre-load"))
        .expect_err("wallet-backed mutation should require load first");

    assert_eq!(
        error,
        "peer reservation table must be loaded before wallet-backed mutation"
    );
}

#[test]
fn peer_reservation_table_public_key_erase_requires_load_two_phase_owner() {
    let table = PeerReservationTable::new();
    let error = table
        .try_erase(&sample_node_public())
        .expect_err("wallet-backed erase should require load first");

    assert_eq!(
        error,
        "peer reservation table must be loaded before wallet-backed mutation"
    );
}

struct BlockingStore {
    started_tx: Mutex<Option<mpsc::Sender<()>>>,
    resume_rx: Mutex<mpsc::Receiver<()>>,
    loaded: Vec<PeerReservation<PublicKey>>,
    inserts: Mutex<Vec<PeerReservation<PublicKey>>>,
}

impl PeerReservationStore<PublicKey> for BlockingStore {
    fn load(&self) -> Result<Vec<PeerReservation<PublicKey>>, String> {
        if let Some(started_tx) = self
            .started_tx
            .lock()
            .expect("blocking store sender poisoned")
            .take()
        {
            started_tx.send(()).expect("send load started");
        }
        self.resume_rx
            .lock()
            .expect("blocking store receiver poisoned")
            .recv()
            .expect("resume load");
        Ok(self.loaded.clone())
    }

    fn insert_or_assign(&self, reservation: &PeerReservation<PublicKey>) -> Result<(), String> {
        self.inserts
            .lock()
            .expect("blocking store inserts poisoned")
            .push(reservation.clone());
        Ok(())
    }

    fn erase(&self, _node_id: &PublicKey) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn peer_reservation_table_public_key_insert_waits_for_in_flight_load() {
    let table = Arc::new(PeerReservationTable::new());
    let (started_tx, started_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let loaded = PeerReservation::new(sample_node_public(), "loaded");
    let inserted = PeerReservation::new(other_node_public(), "inserted");
    let store = Arc::new(BlockingStore {
        started_tx: Mutex::new(Some(started_tx)),
        resume_rx: Mutex::new(resume_rx),
        loaded: vec![loaded.clone()],
        inserts: Mutex::new(Vec::new()),
    });

    let load_table = Arc::clone(&table);
    let load_store: Arc<dyn PeerReservationStore<PublicKey>> = store.clone();
    let load_thread = std::thread::spawn(move || load_table.load(load_store));

    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("load should start");

    let insert_table = Arc::clone(&table);
    let insert_reservation = inserted.clone();
    let (insert_done_tx, insert_done_rx) = mpsc::channel();
    let insert_thread = std::thread::spawn(move || {
        let result = insert_table.try_insert_or_assign(insert_reservation);
        insert_done_tx.send(result).expect("send insert result");
    });

    assert!(
        insert_done_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err(),
        "insert should wait for load to release the table mutex"
    );

    resume_tx.send(()).expect("resume load");

    assert!(load_thread.join().expect("join load thread").expect("load"));
    assert!(
        insert_done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("insert should complete after load")
            .expect("insert should succeed")
            .is_none()
    );
    insert_thread.join().expect("join insert thread");

    assert_eq!(table.list(), vec![inserted.clone(), loaded]);
    assert_eq!(
        store
            .inserts
            .lock()
            .expect("blocking store inserts poisoned")
            .as_slice(),
        &[inserted]
    );
}

#[derive(Clone)]
struct RegistryJournal {
    warnings: Arc<Mutex<Vec<String>>>,
}

impl PeerReservationJournal for RegistryJournal {
    fn warn(&self, message: &str) {
        self.warnings
            .lock()
            .expect("registry journal warnings poisoned")
            .push(message.to_owned());
    }
}

struct PeerReservationRegistry {
    peer_reservations: PeerReservationTable<PublicKey>,
    wallet_db: Arc<DatabaseCon>,
    journal_names: Arc<Mutex<Vec<String>>>,
    journal_warnings: Arc<Mutex<Vec<String>>>,
}

static UNIT: () = ();
static REGISTRY_OPEN_LEDGER: usize = 0;
static REGISTRY_CONFIG: usize = 0;
static REGISTRY_NO_TRAP_TX_ID: Option<u64> = None;

impl ServiceRegistry for PeerReservationRegistry {
    type CollectorManager = ();
    type NodeFamily = ();
    type TimeKeeper = ();
    type JobQueue = ();
    type TempNodeCache = ();
    type CachedSles = ();
    type NetworkIdService = ();
    type AmendmentTable = ();
    type HashRouter = ();
    type LoadFeeTrack = ();
    type LoadManager = ();
    type Validations = ();
    type ValidatorList = ();
    type ValidatorSite = ();
    type ManifestCache = ();
    type Overlay = ();
    type Cluster = ();
    type PeerReservationTable = PeerReservationTable<PublicKey>;
    type ResourceManager = ();
    type NodeStore = ();
    type ShamapStore = ();
    type RelationalDatabase = ();
    type InboundLedgers = ();
    type InboundTransactions = ();
    type AcceptedLedgerCache = ();
    type LedgerMaster = ();
    type LedgerCleaner = ();
    type LedgerReplayer = ();
    type PendingSaves = ();
    type OpenLedger = usize;
    type NetworkOps = ();
    type OrderBookDb = ();
    type TransactionMaster = ();
    type TxQ = ();
    type PathRequestManager = ();
    type ServerHandler = ();
    type PerfLog = ();
    type Journal = RegistryJournal;
    type IoContext = ();
    type Config = usize;
    type Logs = ();
    type TrapTxId = u64;
    type WalletDb = Arc<DatabaseCon>;
    type Application = ();

    fn get_collector_manager(&self) -> &Self::CollectorManager {
        &UNIT
    }
    fn get_node_family(&self) -> &Self::NodeFamily {
        &UNIT
    }
    fn get_time_keeper(&self) -> &Self::TimeKeeper {
        &UNIT
    }
    fn get_job_queue(&self) -> &Self::JobQueue {
        &UNIT
    }
    fn get_temp_node_cache(&self) -> &Self::TempNodeCache {
        &UNIT
    }
    fn get_cached_sles(&self) -> &Self::CachedSles {
        &UNIT
    }
    fn get_network_id_service(&self) -> &Self::NetworkIdService {
        &UNIT
    }
    fn get_amendment_table(&self) -> &Self::AmendmentTable {
        &UNIT
    }
    fn get_hash_router(&self) -> &Self::HashRouter {
        &UNIT
    }
    fn get_fee_track(&self) -> &Self::LoadFeeTrack {
        &UNIT
    }
    fn get_load_manager(&self) -> &Self::LoadManager {
        &UNIT
    }
    fn get_validations(&self) -> &Self::Validations {
        &UNIT
    }
    fn get_validators(&self) -> &Self::ValidatorList {
        &UNIT
    }
    fn get_validator_sites(&self) -> &Self::ValidatorSite {
        &UNIT
    }
    fn get_validator_manifests(&self) -> &Self::ManifestCache {
        &UNIT
    }
    fn get_publisher_manifests(&self) -> &Self::ManifestCache {
        &UNIT
    }
    fn get_overlay(&self) -> &Self::Overlay {
        &UNIT
    }
    fn get_cluster(&self) -> &Self::Cluster {
        &UNIT
    }
    fn get_peer_reservations(&self) -> &Self::PeerReservationTable {
        &self.peer_reservations
    }
    fn get_resource_manager(&self) -> &Self::ResourceManager {
        &UNIT
    }
    fn get_node_store(&self) -> &Self::NodeStore {
        &UNIT
    }
    fn get_shamap_store(&self) -> &Self::ShamapStore {
        &UNIT
    }
    fn get_relational_database(&self) -> &Self::RelationalDatabase {
        &UNIT
    }
    fn get_inbound_ledgers(&self) -> &Self::InboundLedgers {
        &UNIT
    }
    fn get_inbound_transactions(&self) -> &Self::InboundTransactions {
        &UNIT
    }
    fn get_accepted_ledger_cache(&self) -> &Self::AcceptedLedgerCache {
        &UNIT
    }
    fn get_ledger_master(&self) -> &Self::LedgerMaster {
        &UNIT
    }
    fn get_ledger_cleaner(&self) -> &Self::LedgerCleaner {
        &UNIT
    }
    fn get_ledger_replayer(&self) -> &Self::LedgerReplayer {
        &UNIT
    }
    fn get_pending_saves(&self) -> &Self::PendingSaves {
        &UNIT
    }
    fn get_open_ledger(&self) -> &Self::OpenLedger {
        &REGISTRY_OPEN_LEDGER
    }
    fn get_ops(&self) -> &Self::NetworkOps {
        &UNIT
    }
    fn get_order_book_db(&self) -> &Self::OrderBookDb {
        &UNIT
    }
    fn get_master_transaction(&self) -> &Self::TransactionMaster {
        &UNIT
    }
    fn get_tx_q(&self) -> &Self::TxQ {
        &UNIT
    }
    fn get_path_request_manager(&self) -> &Self::PathRequestManager {
        &UNIT
    }
    fn get_server_handler(&self) -> &Self::ServerHandler {
        &UNIT
    }
    fn get_perf_log(&self) -> &Self::PerfLog {
        &UNIT
    }
    fn is_stopping(&self) -> bool {
        false
    }
    fn get_journal(&self, name: &str) -> Self::Journal {
        self.journal_names
            .lock()
            .expect("registry journal names poisoned")
            .push(name.to_owned());
        RegistryJournal {
            warnings: Arc::clone(&self.journal_warnings),
        }
    }
    fn get_io_context(&self) -> &Self::IoContext {
        &UNIT
    }
    fn get_config(&self) -> &Self::Config {
        &REGISTRY_CONFIG
    }
    fn get_logs(&self) -> &Self::Logs {
        &UNIT
    }
    fn get_trap_tx_id(&self) -> &Option<Self::TrapTxId> {
        &REGISTRY_NO_TRAP_TX_ID
    }
    fn get_wallet_db(&self) -> &Self::WalletDb {
        &self.wallet_db
    }
    fn get_app(&self) -> &Self::Application {
        &UNIT
    }
}

#[test]
fn peer_reservation_table_registry_load_application_setup_shape() {
    let (_temp, _db, registry, journal_names, journal_warnings) =
        peer_reservation_registry_fixture();

    assert!(load_peer_reservations_from_registry(&registry).expect("registry load"));
    assert_eq!(
        registry.get_peer_reservations().list(),
        vec![PeerReservation::new(sample_node_public(), "valid")]
    );
    assert_eq!(
        journal_names
            .lock()
            .expect("registry journal names poisoned")
            .as_slice(),
        &[] as &[String]
    );
    assert_eq!(
        journal_warnings
            .lock()
            .expect("registry journal warnings poisoned")
            .as_slice(),
        ["load: not a public key: not-a-node-public"]
    );
}

type PeerReservationRegistryFixture = (
    TempDir,
    Arc<DatabaseCon>,
    PeerReservationRegistry,
    Arc<Mutex<Vec<String>>>,
    Arc<Mutex<Vec<String>>>,
);

fn peer_reservation_registry_fixture() -> PeerReservationRegistryFixture {
    let (temp, db) = wallet_db();
    {
        let connection = db.checkout_db();
        connection
            .execute(
                "INSERT INTO PeerReservations (PublicKey, Description) VALUES (?1, ?2)",
                [
                    sample_node_public().to_node_public_base58(),
                    "valid".to_owned(),
                ],
            )
            .expect("insert valid row");
        connection
            .execute(
                "INSERT INTO PeerReservations (PublicKey, Description) VALUES (?1, ?2)",
                ["not-a-node-public".to_owned(), "invalid".to_owned()],
            )
            .expect("insert invalid row");
    }

    let journal_names = Arc::new(Mutex::new(Vec::new()));
    let journal_warnings = Arc::new(Mutex::new(Vec::new()));
    let registry = PeerReservationRegistry {
        peer_reservations: PeerReservationTable::new_with_journal(Arc::new(RegistryJournal {
            warnings: Arc::clone(&journal_warnings),
        })),
        wallet_db: Arc::clone(&db),
        journal_names: Arc::clone(&journal_names),
        journal_warnings: Arc::clone(&journal_warnings),
    };

    (temp, db, registry, journal_names, journal_warnings)
}

fn assert_registry_holder_load<R>(
    holder: &R,
    registry: &PeerReservationRegistry,
    journal_names: &Arc<Mutex<Vec<String>>>,
    journal_warnings: &Arc<Mutex<Vec<String>>>,
) where
    R: ServiceRegistry<
            PeerReservationTable = PeerReservationTable<PublicKey>,
            WalletDb = Arc<DatabaseCon>,
        >,
{
    assert!(load_peer_reservations_from_registry(holder).expect("registry load"));
    assert_eq!(
        registry.get_peer_reservations().list(),
        vec![PeerReservation::new(sample_node_public(), "valid")]
    );
    assert_eq!(
        journal_names
            .lock()
            .expect("registry journal names poisoned")
            .as_slice(),
        &[] as &[String]
    );
    assert_eq!(
        journal_warnings
            .lock()
            .expect("registry journal warnings poisoned")
            .as_slice(),
        ["load: not a public key: not-a-node-public"]
    );
}

#[test]
fn peer_reservation_registry_load_helper_forwards_through_box_arc_rc_and_mut_holders() {
    {
        let (_temp, _db, registry, journal_names, journal_warnings) =
            peer_reservation_registry_fixture();
        let holder = Box::new(registry);
        assert_registry_holder_load(&holder, holder.as_ref(), &journal_names, &journal_warnings);
    }

    {
        let (_temp, _db, registry, journal_names, journal_warnings) =
            peer_reservation_registry_fixture();
        let holder = Arc::new(registry);
        assert_registry_holder_load(&holder, holder.as_ref(), &journal_names, &journal_warnings);
    }

    {
        let (_temp, _db, registry, journal_names, journal_warnings) =
            peer_reservation_registry_fixture();
        let holder = Rc::new(registry);
        assert_registry_holder_load(&holder, holder.as_ref(), &journal_names, &journal_warnings);
    }

    {
        let (_temp, _db, mut registry, journal_names, journal_warnings) =
            peer_reservation_registry_fixture();
        let holder = &mut registry;
        let registry_view: &PeerReservationRegistry = holder;
        assert_registry_holder_load(&holder, registry_view, &journal_names, &journal_warnings);
    }
}
