use basics::basic_config::Section;
use nodestore::{DummyScheduler, Manager, ManagerImp, NodeObject, NodeObjectType, NullJournal};
use protocol::JsonValue;
use std::sync::Arc;
use tempfile::TempDir;

fn memory_section(path: &str) -> Section {
    let mut section = Section::new("node_db");
    section.set("type", "Memory");
    section.set("path", path);
    section
}

#[test]
fn manager_clamps_non_positive_read_threads_across_all_constructors() {
    let manager = ManagerImp::new();
    let scheduler: Arc<dyn nodestore::Scheduler> = Arc::new(DummyScheduler);
    let journal: Arc<dyn nodestore::NodeStoreJournal> = Arc::new(NullJournal);
    let mut context = false;

    let zero_config = memory_section("validation-zero");
    let zero_database = manager
        .make_database(
            0,
            Arc::clone(&scheduler),
            0,
            &zero_config,
            Arc::clone(&journal),
        )
        .expect("zero read threads should clamp to one");
    let JsonValue::Object(zero_counts) = zero_database.get_counts_json() else {
        panic!("database counts should be a JSON object");
    };
    assert_eq!(
        zero_counts.get("read_threads_total"),
        Some(&JsonValue::Signed(1))
    );
    zero_database.stop();

    let negative_config = memory_section("validation-negative");
    let negative_database = manager
        .make_database(
            0,
            Arc::clone(&scheduler),
            -1,
            &negative_config,
            Arc::clone(&journal),
        )
        .expect("negative read threads should clamp to one");
    let JsonValue::Object(negative_counts) = negative_database.get_counts_json() else {
        panic!("database counts should be a JSON object");
    };
    assert_eq!(
        negative_counts.get("read_threads_total"),
        Some(&JsonValue::Signed(1))
    );
    negative_database.stop();

    let deterministic_dir = TempDir::new().expect("tempdir");
    let deterministic_path = deterministic_dir.path().join("validation-deterministic");
    let mut deterministic_config = Section::new("node_db");
    deterministic_config.set("type", "NuDB");
    deterministic_config.set("path", deterministic_path.to_string_lossy().into_owned());
    let deterministic_database = manager
        .make_database_deterministic(
            0,
            Arc::clone(&scheduler),
            0,
            &deterministic_config,
            1,
            2,
            3,
            Arc::clone(&journal),
        )
        .expect("deterministic constructor should clamp zero threads to one");
    let JsonValue::Object(deterministic_counts) = deterministic_database.get_counts_json() else {
        panic!("database counts should be a JSON object");
    };
    assert_eq!(
        deterministic_counts.get("read_threads_total"),
        Some(&JsonValue::Signed(1))
    );
    deterministic_database.stop();

    let context_config = memory_section("validation-context");
    let context_database = manager
        .make_database_with_context(
            0,
            Arc::clone(&scheduler),
            -4,
            &context_config,
            &mut context,
            Arc::clone(&journal),
        )
        .expect("context constructor should clamp negative threads to one");
    let JsonValue::Object(context_counts) = context_database.get_counts_json() else {
        panic!("database counts should be a JSON object");
    };
    assert_eq!(
        context_counts.get("read_threads_total"),
        Some(&JsonValue::Signed(1))
    );
    context_database.stop();

    let mut writable = memory_section("writable");
    let mut archive = memory_section("archive");
    writable.set("type", "Memory");
    archive.set("type", "Memory");
    let rotating_database = manager
        .make_rotating_database(0, scheduler, 0, &writable, &archive, &writable, journal)
        .expect("rotating constructor should clamp zero threads to one");
    let JsonValue::Object(rotating_counts) = rotating_database.get_counts_json() else {
        panic!("database counts should be a JSON object");
    };
    assert_eq!(
        rotating_counts.get("read_threads_total"),
        Some(&JsonValue::Signed(1))
    );
    rotating_database.stop();
}

#[test]
fn manager_nudb_entrypoint_uses_native_nudb_storage_policy() {
    let manager = ManagerImp::new();
    let scheduler: Arc<dyn nodestore::Scheduler> = Arc::new(DummyScheduler);
    let journal: Arc<dyn nodestore::NodeStoreJournal> = Arc::new(NullJournal);
    let dir = TempDir::new().expect("tempdir");
    let backend_path = dir.path().join("nudb");

    let mut config = Section::new("node_db");
    config.set("type", "NuDB");
    config.set("path", backend_path.to_string_lossy().into_owned());

    let backend = manager
        .make_backend(&config, 0, Arc::clone(&scheduler), Arc::clone(&journal))
        .expect("NuDB config should resolve to the native Rust NuDB backend");
    assert_eq!(
        manager
            .find("NuDB")
            .expect("NuDB factory should stay registered")
            .get_name(),
        "NuDB"
    );
    assert_eq!(
        manager
            .find("RocksDB")
            .expect("rocksdb backend should stay registered")
            .get_name(),
        "RocksDB"
    );
    backend.open(true).expect("open");
    let object = NodeObject::create_object(
        NodeObjectType::Ledger,
        vec![1, 3, 5, 7],
        basics::base_uint::Uint256::from_array([0xCC; 32]),
    );
    backend.store(Arc::clone(&object));

    let (fetched, status) = backend.fetch(object.hash());
    assert_eq!(status, nodestore::Status::Ok);
    assert_eq!(fetched.expect("stored object").data(), object.data());
    assert!(backend_path.join("nudb.dat").exists());
    assert!(backend_path.join("nudb.key").exists());
    assert!(backend_path.join("nudb.log").exists());

    backend.close().expect("close");
}

#[test]
fn manager_nudb_deterministic_entrypoint_uses_native_nudb_storage_policy() {
    let manager = ManagerImp::new();
    let scheduler: Arc<dyn nodestore::Scheduler> = Arc::new(DummyScheduler);
    let journal: Arc<dyn nodestore::NodeStoreJournal> = Arc::new(NullJournal);
    let dir = TempDir::new().expect("tempdir");
    let backend_path = dir.path().join("nudb-deterministic");

    let mut config = Section::new("node_db");
    config.set("type", "NuDB");
    config.set("path", backend_path.to_string_lossy().into_owned());

    let database = manager
        .make_database_deterministic(
            0,
            Arc::clone(&scheduler),
            1,
            &config,
            nodestore::NUDB_APPNUM,
            0x1234,
            0x5678,
            Arc::clone(&journal),
        )
        .expect("NuDB deterministic config should resolve to the native Rust NuDB backend");

    let hash = basics::base_uint::Uint256::from_array([0xDD; 32]);
    database.store(NodeObjectType::Ledger, vec![1, 3, 5, 7], hash, 0);
    database.sync();
    assert_eq!(
        std::fs::metadata(backend_path.join("nudb.log"))
            .expect("NuDB log metadata after sync")
            .len(),
        0,
        "sync must commit the active NuDB burst before an acquired ledger is published"
    );

    let fetched = database.fetch_batch(&[hash]);
    assert_eq!(fetched.len(), 1);
    assert_eq!(
        fetched[0].as_ref().expect("stored object").data(),
        &[1, 3, 5, 7]
    );

    database.stop();
    drop(database);

    let reopened = manager
        .make_database_deterministic(
            0,
            scheduler,
            1,
            &config,
            nodestore::NUDB_APPNUM,
            0x1234,
            0x5678,
            journal,
        )
        .expect("stopped NuDB database should reopen");
    let restored = reopened
        .fetch_node_object(&hash, 0, nodestore::FetchType::Synchronous, false)
        .expect("single buffered NuDB object must survive stop and reopen");
    assert_eq!(restored.data(), &[1, 3, 5, 7]);
    reopened.stop();

    assert!(backend_path.join("nudb.dat").exists());
    assert!(backend_path.join("nudb.key").exists());
    assert!(backend_path.join("nudb.log").exists());
}
