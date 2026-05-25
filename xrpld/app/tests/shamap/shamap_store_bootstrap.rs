use app::{
    ApplicationRoot, SHAMapStoreNodeStore, SHAMapStoreSavedState, SHAMapStoreSavedStateDb,
    bootstrap_shamap_store,
};
use basics::basic_config::BasicConfig;
use nodestore::{DummyScheduler, ManagerImp, NullJournal, Scheduler};
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

fn online_delete_config(
    database_path: &std::path::Path,
    node_db_path: &std::path::Path,
    backend_type: &str,
) -> BasicConfig {
    let mut config = BasicConfig::new();
    config.set_legacy("database_path", database_path.to_string_lossy());
    let node_db = config.section_mut("node_db");
    node_db.set("type", backend_type);
    node_db.set("path", node_db_path.to_string_lossy());
    node_db.set("online_delete", "256");
    config
}

#[test]
fn shamap_store_bootstrap_persists_first_rotating_backend_names() {
    let dir = TempDir::new().expect("tempdir");
    let mut config = BasicConfig::new();
    config.set_legacy("database_path", dir.path().join("sql").to_string_lossy());
    let node_db = config.section_mut("node_db");
    node_db.set("type", "RocksDB");
    node_db.set("path", dir.path().join("node").to_string_lossy());
    node_db.set("online_delete", "256");

    let bootstrap = bootstrap_shamap_store(
        &config,
        false,
        128,
        2,
        8,
        64,
        2,
        &ManagerImp::new(),
        Arc::new(DummyScheduler) as Arc<dyn Scheduler>,
        Arc::new(NullJournal),
    )
    .expect("bootstrap");

    match bootstrap.node_store {
        SHAMapStoreNodeStore::Rotating(_) => {}
        SHAMapStoreNodeStore::Single(_) => panic!("online delete should create rotating store"),
    }

    let state_db = bootstrap.state_db.expect("state db");
    let state = state_db.get_state().expect("state");
    assert!(!state.writable_db.is_empty());
    assert!(!state.archive_db.is_empty());
    assert!(bootstrap.effective_node_db_config.exists("cache_mb"));
    assert!(bootstrap.effective_node_db_config.exists("filter_bits"));
}

#[test]
fn shamap_store_bootstrap_can_attach_node_store_to_application_root() {
    let dir = TempDir::new().expect("tempdir");
    let mut config = BasicConfig::new();
    config.set_legacy("database_path", dir.path().join("sql").to_string_lossy());
    let node_db = config.section_mut("node_db");
    node_db.set("type", "Memory");
    node_db.set("path", dir.path().join("node").to_string_lossy());

    let bootstrap = bootstrap_shamap_store(
        &config,
        false,
        128,
        1,
        8,
        64,
        2,
        &ManagerImp::new(),
        Arc::new(DummyScheduler) as Arc<dyn Scheduler>,
        Arc::new(NullJournal),
    )
    .expect("bootstrap");

    let mut root = ApplicationRoot::new(0).expect("root");
    let previous = bootstrap.attach_node_store(&mut root);
    assert!(previous.is_none());
    assert!(root.node_store().is_some());
    assert_eq!(
        root.node_store().as_ref().expect("node store").kind(),
        bootstrap.node_store_kind()
    );
}

#[test]
fn shamap_store_bootstrap_deletes_stale_rotating_backend_paths_like_dbpaths() {
    let dir = TempDir::new().expect("tempdir");
    let database_path = dir.path().join("sql");
    let node_db_path = dir.path().join("node");
    fs::create_dir_all(&node_db_path).expect("node db dir");
    fs::create_dir(node_db_path.join("xrpldb.0000")).expect("stale dir");
    fs::write(node_db_path.join("xrpldb.0001"), b"stale").expect("stale file");

    let config = online_delete_config(&database_path, &node_db_path, "NuDB");
    let bootstrap = bootstrap_shamap_store(
        &config,
        false,
        128,
        2,
        8,
        64,
        2,
        &ManagerImp::new(),
        Arc::new(DummyScheduler) as Arc<dyn Scheduler>,
        Arc::new(NullJournal),
    )
    .expect("bootstrap");

    let state = bootstrap
        .state_db
        .expect("state db")
        .get_state()
        .expect("state");
    assert!(!state.writable_db.is_empty());
    assert!(!state.archive_db.is_empty());
    assert!(!node_db_path.join("xrpldb.0000").exists());
    assert!(!node_db_path.join("xrpldb.0001").exists());
}

#[test]
fn shamap_store_bootstrap_rewrites_moved_backend_paths_and_persists_them() {
    let dir = TempDir::new().expect("tempdir");
    let database_path = dir.path().join("sql");
    let old_node_db_path = dir.path().join("node-old");
    let new_node_db_path = dir.path().join("node-new");
    fs::create_dir_all(&old_node_db_path).expect("old node dir");
    fs::create_dir_all(&new_node_db_path).expect("new node dir");

    let config = online_delete_config(&database_path, &new_node_db_path, "NuDB");
    let state_db = SHAMapStoreSavedStateDb::open(&config, "state").expect("state db");
    state_db
        .set_state(&SHAMapStoreSavedState {
            writable_db: old_node_db_path
                .join("xrpldb.0000")
                .to_string_lossy()
                .into_owned(),
            archive_db: old_node_db_path
                .join("xrpldb.0001")
                .to_string_lossy()
                .into_owned(),
            last_rotated: 700,
        })
        .expect("saved state");

    fs::create_dir(new_node_db_path.join("xrpldb.0000")).expect("writable");
    fs::create_dir(new_node_db_path.join("xrpldb.0001")).expect("archive");
    fs::create_dir(new_node_db_path.join("xrpldb.0002")).expect("stale");

    let bootstrap = bootstrap_shamap_store(
        &config,
        false,
        128,
        2,
        8,
        64,
        2,
        &ManagerImp::new(),
        Arc::new(DummyScheduler) as Arc<dyn Scheduler>,
        Arc::new(NullJournal),
    )
    .expect("bootstrap");

    let state = bootstrap
        .state_db
        .expect("state db")
        .get_state()
        .expect("state");
    assert_eq!(
        state.writable_db,
        new_node_db_path
            .join("xrpldb.0000")
            .to_string_lossy()
            .into_owned()
    );
    assert_eq!(
        state.archive_db,
        new_node_db_path
            .join("xrpldb.0001")
            .to_string_lossy()
            .into_owned()
    );
    assert_eq!(state.last_rotated, 700);
    assert!(!new_node_db_path.join("xrpldb.0002").exists());
}

#[test]
fn shamap_store_bootstrap_rejects_cpp_corruption_when_only_one_saved_backend_exists() {
    let dir = TempDir::new().expect("tempdir");
    let database_path = dir.path().join("sql");
    let node_db_path = dir.path().join("node");
    fs::create_dir_all(&node_db_path).expect("node dir");

    let config = online_delete_config(&database_path, &node_db_path, "NuDB");
    let state_db = SHAMapStoreSavedStateDb::open(&config, "state").expect("state db");
    let writable = node_db_path.join("xrpldb.0000");
    let archive = node_db_path.join("xrpldb.0001");
    state_db
        .set_state(&SHAMapStoreSavedState {
            writable_db: writable.to_string_lossy().into_owned(),
            archive_db: archive.to_string_lossy().into_owned(),
            last_rotated: 42,
        })
        .expect("saved state");
    fs::create_dir(&writable).expect("writable");

    let error = match bootstrap_shamap_store(
        &config,
        false,
        128,
        2,
        8,
        64,
        2,
        &ManagerImp::new(),
        Arc::new(DummyScheduler) as Arc<dyn Scheduler>,
        Arc::new(NullJournal),
    ) {
        Ok(_) => panic!("bootstrap should reject mismatched saved backends"),
        Err(error) => error,
    };

    assert_eq!(error, "state db error");
}
