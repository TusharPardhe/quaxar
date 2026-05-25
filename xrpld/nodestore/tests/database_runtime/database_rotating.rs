use basics::basic_config::Section;
use nodestore::{
    Backend, DatabaseRotatingImp, JournalLevel, NodeObject, NodeStoreJournal, NullJournal,
    RocksDbBackend, Scheduler,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[derive(Default)]
struct RecordingJournal {
    entries: Mutex<Vec<(String, String)>>,
}

impl NodeStoreJournal for RecordingJournal {
    fn log(&self, level: JournalLevel, message: &str) {
        self.entries
            .lock()
            .expect("journal mutex")
            .push((level.to_string(), message.to_owned()));
    }
}

fn section(path: &Path) -> Section {
    let mut section = Section::new("node_db");
    section.set("type", "RocksDB");
    section.set("path", path.to_string_lossy().as_ref());
    section
}

fn open_backend(path: &Path, scheduler: Arc<dyn Scheduler>) -> Arc<dyn Backend> {
    let backend: Arc<dyn Backend> = Arc::new(
        RocksDbBackend::new(
            NodeObject::KEY_BYTES,
            &section(path),
            scheduler,
            Arc::new(NullJournal),
        )
        .expect("rocksdb backend"),
    );
    backend.open(true).expect("open backend");
    backend
}

fn open_boxed_backend(path: &Path, scheduler: Arc<dyn Scheduler>) -> Box<dyn Backend> {
    let backend = RocksDbBackend::new(
        NodeObject::KEY_BYTES,
        &section(path),
        scheduler,
        Arc::new(NullJournal),
    )
    .expect("rocksdb backend");
    backend.open(true).expect("open backend");
    Box::new(backend)
}

#[test]
fn rotating_rocksdb_deletes_retired_archive_only_after_callback_returns() {
    let dir = TempDir::new().expect("tempdir");
    let writable_path = dir.path().join("writable");
    let archive_path = dir.path().join("archive");
    let next_path = dir.path().join("next");
    let scheduler: Arc<dyn Scheduler> = Arc::new(nodestore::DummyScheduler);

    let writable_backend = open_backend(&writable_path, Arc::clone(&scheduler));
    let archive_backend = open_backend(&archive_path, Arc::clone(&scheduler));
    let rotating = DatabaseRotatingImp::new(
        Arc::clone(&scheduler),
        1,
        writable_backend,
        archive_backend,
        &section(&writable_path),
        Arc::new(NullJournal),
    )
    .expect("rotating database");

    let new_backend = open_boxed_backend(&next_path, Arc::clone(&scheduler));

    let seen = Arc::new(Mutex::new(None::<(String, String, PathBuf)>));
    let callback_seen = Arc::clone(&seen);
    let callback_archive_path = archive_path.clone();
    rotating.rotate(new_backend, move |writable_name, archive_name| {
        assert!(
            callback_archive_path.exists(),
            "old archive path must still exist while the rotate callback runs"
        );
        *callback_seen.lock().expect("callback mutex") = Some((
            writable_name.to_owned(),
            archive_name.to_owned(),
            callback_archive_path.clone(),
        ));
    });

    assert!(
        !dir.path().join("archive").exists(),
        "retired archive path should be deleted once the callback returns"
    );
    assert!(
        writable_path.exists(),
        "previous writable becomes the archive"
    );
    assert!(next_path.exists(), "new writable remains live");
    assert_eq!(
        seen.lock().expect("callback mutex").clone(),
        Some((
            next_path.to_string_lossy().into_owned(),
            writable_path.to_string_lossy().into_owned(),
            dir.path().join("archive"),
        ))
    );

    rotating.stop();
}

#[test]
fn rotating_database_retains_the_journal_owner_handle() {
    let dir = TempDir::new().expect("tempdir");
    let writable_path = dir.path().join("writable");
    let archive_path = dir.path().join("archive");
    let scheduler: Arc<dyn Scheduler> = Arc::new(nodestore::DummyScheduler);
    let writable_backend = open_backend(&writable_path, Arc::clone(&scheduler));
    let archive_backend = open_backend(&archive_path, Arc::clone(&scheduler));
    let journal: Arc<RecordingJournal> = Arc::new(RecordingJournal::default());
    let journal_iface: Arc<dyn NodeStoreJournal> = journal.clone();
    let rotating = DatabaseRotatingImp::new(
        Arc::clone(&scheduler),
        1,
        writable_backend,
        archive_backend,
        &section(&writable_path),
        Arc::clone(&journal_iface),
    )
    .expect("rotating database");

    assert!(Arc::ptr_eq(&journal_iface, &rotating.journal()));
    rotating.stop();
    assert!(!journal.entries.lock().expect("journal mutex").is_empty());
}
