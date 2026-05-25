use basics::{base_uint::Uint256, basic_config::Section};
use nodestore::{
    Backend, JournalLevel, NodeObject, NodeObjectType, NodeStoreJournal, NullJournal,
    RocksDbBackend, RocksDbConfigSnapshot,
};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[derive(Default)]
struct RecordingJournal {
    entries: Mutex<Vec<(JournalLevel, String)>>,
}

impl RecordingJournal {
    fn entries(&self) -> Vec<(JournalLevel, String)> {
        self.entries
            .lock()
            .expect("recording journal mutex must not be poisoned")
            .clone()
    }
}

impl NodeStoreJournal for RecordingJournal {
    fn log(&self, level: JournalLevel, message: &str) {
        self.entries
            .lock()
            .expect("recording journal mutex must not be poisoned")
            .push((level, message.to_owned()));
    }
}

fn base_section(path: &str) -> Section {
    let mut section = Section::new("node_db");
    section.set("type", "RocksDB");
    section.set("path", path);
    section
}

fn sample_object(fill: u8, payload: &[u8]) -> Arc<NodeObject> {
    NodeObject::create_object(
        NodeObjectType::Ledger,
        payload.to_vec(),
        Uint256::from_array([fill; 32]),
    )
}

#[test]
fn rocksdb_config_snapshot_tuning_rules() {
    let dir = TempDir::new().expect("tempdir");
    let mut section = base_section(&dir.path().join("node_db").to_string_lossy());
    section.set("cache_mb", "256");
    section.set("filter_bits", "12");
    section.set("filter_full", "1");
    section.set("open_files", "2000");
    section.set("file_size_mb", "8");
    section.set("file_size_mult", "3");
    section.set("bg_threads", "4");
    section.set("high_threads", "2");
    section.set("block_size", "4096");
    section.set("universal_compaction", "1");

    let snapshot = RocksDbConfigSnapshot::from_section(&section).expect("snapshot");
    assert_eq!(snapshot.cache_mb, Some(1024));
    assert_eq!(snapshot.filter_bits, Some(12));
    assert!(snapshot.filter_full);
    assert_eq!(snapshot.max_open_files, Some(8000));
    assert_eq!(snapshot.fd_required, 8128);
    assert_eq!(snapshot.target_file_size_base, Some(256 * 1024 * 1024));
    assert_eq!(
        snapshot.max_bytes_for_level_base,
        Some(5 * 256 * 1024 * 1024)
    );
    assert_eq!(snapshot.write_buffer_size, Some(6 * 256 * 1024 * 1024));
    assert_eq!(snapshot.target_file_size_multiplier, Some(3));
    assert_eq!(snapshot.bg_threads, Some(4));
    assert_eq!(snapshot.high_threads, Some(2));
    assert_eq!(snapshot.max_background_flushes, Some(2));
    assert_eq!(snapshot.block_size, Some(4096));
    assert!(snapshot.universal_compaction);
    assert_eq!(snapshot.min_write_buffer_number_to_merge, Some(2));
    assert_eq!(snapshot.max_write_buffer_number, Some(6));
}

#[test]
fn rocksdb_config_snapshot_accepts_cxx_style_numeric_hard_set_flags() {
    let dir = TempDir::new().expect("tempdir");
    let mut section = base_section(&dir.path().join("node_db_hard_set").to_string_lossy());
    section.set("hard_set", "1");
    section.set("cache_mb", "256");
    section.set("open_files", "2000");
    section.set("file_size_mb", "8");

    let snapshot = RocksDbConfigSnapshot::from_section(&section).expect("snapshot");
    assert!(snapshot.hard_set);
    assert_eq!(snapshot.cache_mb, Some(256));
    assert_eq!(snapshot.max_open_files, Some(2000));
    assert_eq!(snapshot.target_file_size_base, Some(8 * 1024 * 1024));
}

#[test]
fn rocksdb_config_snapshot_rejects_malformed_numeric_values() {
    let dir = TempDir::new().expect("tempdir");
    let mut section = base_section(&dir.path().join("node_db_invalid").to_string_lossy());
    section.set("bg_threads", "not-a-number");

    let error = match RocksDbConfigSnapshot::from_section(&section) {
        Ok(_) => panic!("malformed numeric config should fail"),
        Err(error) => error,
    };

    assert!(error.contains("bg_threads"), "unexpected error: {error}");
}

#[test]
fn rocksdb_backend_emits_option_summary_logs_on_construction() {
    let dir = TempDir::new().expect("tempdir");
    let mut section = base_section(&dir.path().join("logged_rocks").to_string_lossy());
    section.set("options", "disable_auto_compactions=true");
    section.set("bbt_options", "block_size=4096;index_type=hash_search");

    let journal = Arc::new(RecordingJournal::default());
    let journal_for_backend: Arc<dyn NodeStoreJournal> = journal.clone();
    let _backend = RocksDbBackend::new(
        NodeObject::KEY_BYTES,
        &section,
        Arc::new(nodestore::DummyScheduler),
        journal_for_backend,
    )
    .expect("rocksdb backend");

    let entries = journal.entries();
    assert_eq!(entries.len(), 2, "expected DBOptions and CFOptions logs");
    assert_eq!(entries[0].0, JournalLevel::Debug);
    assert_eq!(entries[1].0, JournalLevel::Debug);
    assert!(
        entries[0].1.starts_with("RocksDB DBOptions: "),
        "unexpected first log: {:?}",
        entries[0]
    );
    assert!(
        entries[1].1.starts_with("RocksDB CFOptions: "),
        "unexpected second log: {:?}",
        entries[1]
    );
}

#[test]
fn rocksdb_backend_writes_and_reads_disk_objects() {
    let dir = TempDir::new().expect("tempdir");
    let section = base_section(&dir.path().join("ledger_rocks").to_string_lossy());
    let backend = RocksDbBackend::new(
        NodeObject::KEY_BYTES,
        &section,
        Arc::new(nodestore::DummyScheduler),
        Arc::new(NullJournal),
    )
    .expect("rocksdb backend");

    backend.open(true).expect("open");
    let object = sample_object(0xAB, &[1, 2, 3, 4]);
    backend.store(Arc::clone(&object));

    let (fetched, status) = backend.fetch(object.hash());
    assert_eq!(status, nodestore::Status::Ok);
    let fetched = fetched.expect("object should exist");
    assert_eq!(fetched.hash(), object.hash());
    assert_eq!(fetched.data(), object.data());
    assert!(backend.is_open());
    assert!(dir.path().join("ledger_rocks").exists());

    backend.close().expect("close");
}

#[test]
fn rocksdb_backend_applies_cpp_style_option_strings() {
    let dir = TempDir::new().expect("tempdir");
    let mut section = base_section(&dir.path().join("style_options").to_string_lossy());
    section.set("options", "disable_auto_compactions=true");
    section.set(
        "bbt_options",
        "block_size=4096;block_cache=1M;cache_index_and_filter_blocks=true",
    );

    let backend = RocksDbBackend::new(
        NodeObject::KEY_BYTES,
        &section,
        Arc::new(nodestore::DummyScheduler),
        Arc::new(NullJournal),
    )
    .expect("rocksdb backend");

    assert_eq!(
        backend.config_snapshot().options.as_deref(),
        Some("disable_auto_compactions=true")
    );
    assert_eq!(
        backend.config_snapshot().bbt_options.as_deref(),
        Some("block_size=4096;block_cache=1M;cache_index_and_filter_blocks=true")
    );

    backend
        .open(true)
        .expect("open with cpp-style option strings");
    let object = sample_object(0xEF, &[4, 5, 6, 7]);
    backend.store(Arc::clone(&object));

    let (fetched, status) = backend.fetch(object.hash());
    assert_eq!(status, nodestore::Status::Ok);
    assert_eq!(fetched.expect("object should exist").data(), object.data());

    backend.close().expect("close");
}

#[test]
fn rocksdb_close_flushes_batch_writer_persistence() {
    let dir = TempDir::new().expect("tempdir");
    let section = base_section(&dir.path().join("queued_rocks").to_string_lossy());
    let backend = RocksDbBackend::new(
        NodeObject::KEY_BYTES,
        &section,
        Arc::new(nodestore::DummyScheduler),
        Arc::new(NullJournal),
    )
    .expect("rocksdb backend");

    backend.open(true).expect("open");
    let object = sample_object(0xCD, &[9, 9, 9]);
    backend.store(Arc::clone(&object));
    backend.close().expect("close should flush");

    let reopened = RocksDbBackend::new(
        NodeObject::KEY_BYTES,
        &section,
        Arc::new(nodestore::DummyScheduler),
        Arc::new(NullJournal),
    )
    .expect("rocksdb backend");
    reopened.open(false).expect("reopen");
    let (fetched, status) = reopened.fetch(object.hash());
    assert_eq!(status, nodestore::Status::Ok);
    assert_eq!(fetched.expect("persisted object").data(), object.data());
    reopened.close().expect("close");
}

#[test]
fn rocksdb_fetch_batch_order_for_present_and_missing_keys() {
    let dir = TempDir::new().expect("tempdir");
    let section = base_section(&dir.path().join("batch_fetch").to_string_lossy());
    let backend = RocksDbBackend::new(
        NodeObject::KEY_BYTES,
        &section,
        Arc::new(nodestore::DummyScheduler),
        Arc::new(NullJournal),
    )
    .expect("rocksdb backend");

    backend.open(true).expect("open");
    let first = sample_object(0x11, &[1, 2, 3]);
    let second = sample_object(0x22, &[4, 5, 6]);
    backend.store(Arc::clone(&first));
    backend.store(Arc::clone(&second));
    backend.close().expect("close should flush");

    let reopened = RocksDbBackend::new(
        NodeObject::KEY_BYTES,
        &section,
        Arc::new(nodestore::DummyScheduler),
        Arc::new(NullJournal),
    )
    .expect("rocksdb backend");
    reopened.open(false).expect("reopen");

    let missing = Uint256::from_array([0x33; 32]);
    let (results, status) = reopened.fetch_batch(&[*second.hash(), missing, *first.hash()]);
    assert_eq!(status, nodestore::Status::Ok);
    assert_eq!(results.len(), 3);
    assert_eq!(
        results[0].as_ref().expect("second object").data(),
        second.data()
    );
    assert!(results[1].is_none());
    assert_eq!(
        results[2].as_ref().expect("first object").data(),
        first.data()
    );

    reopened.close().expect("close");
}

#[test]
fn rocksdb_delete_path_flag_stays_sticky_across_close_and_reopen() {
    let dir = TempDir::new().expect("tempdir");
    let section = base_section(&dir.path().join("sticky_delete").to_string_lossy());
    let backend = RocksDbBackend::new(
        NodeObject::KEY_BYTES,
        &section,
        Arc::new(nodestore::DummyScheduler),
        Arc::new(NullJournal),
    )
    .expect("rocksdb backend");

    backend.open(true).expect("open");
    backend.set_delete_path();
    backend
        .close()
        .expect("first close should remove the directory");
    assert!(
        !dir.path().join("sticky_delete").exists(),
        "delete-path should remove the directory on close"
    );

    backend
        .open(true)
        .expect("reopen should recreate the directory");
    assert!(dir.path().join("sticky_delete").exists());
    backend
        .close()
        .expect("sticky delete-path should remove the directory again");
    assert!(
        !dir.path().join("sticky_delete").exists(),
        "delete-path flag should stay sticky like the C++ backend"
    );
}
