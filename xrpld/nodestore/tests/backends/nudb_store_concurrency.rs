use basics::{base_uint::Uint256, basic_config::Section};
use nodestore::{
    Backend, NUDB_APPNUM, NodeObject, NodeObjectType, NodeStoreJournal, NuDbBackend, Status,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

struct QuietJournal;

impl NodeStoreJournal for QuietJournal {
    fn log(&self, _level: nodestore::JournalLevel, _message: &str) {}
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nodestore-concurrency-tests")
            .join(format!("{name}-pid{}-{id}", std::process::id()));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale test dir");
        }
        fs::create_dir_all(&path).expect("create test dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            panic!("failed to remove test dir {}: {error}", self.path.display());
        }
    }
}

fn nudb_section(path: &Path) -> Section {
    let mut section = Section::new("node_db");
    section.set("path", path.to_string_lossy().into_owned());
    section
}

fn make_object(writer: usize, item: usize) -> Arc<NodeObject> {
    let unique = ((writer as u64) << 32) | item as u64;
    let mut hash = [0u8; 32];
    hash[..8].copy_from_slice(&unique.to_be_bytes());
    hash[8] = writer as u8;
    hash[9] = item as u8;

    NodeObject::create_object(
        NodeObjectType::Ledger,
        format!("nudb-concurrency-{writer}-{item}").into_bytes(),
        Uint256::from_array(hash),
    )
}

fn make_object_matrix(writers: usize, per_writer: usize) -> Vec<Vec<Arc<NodeObject>>> {
    (0..writers)
        .map(|writer| {
            (0..per_writer)
                .map(|item| make_object(writer, item))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn nudb_store_concurrent_writers_preserve_all_objects() {
    const WRITERS: usize = 6;
    const OBJECTS_PER_WRITER: usize = 40;

    let dir = TestDir::new("store-concurrent-writers");
    let backend = Arc::new(
        NuDbBackend::new(
            NodeObject::KEY_BYTES,
            &nudb_section(dir.path()),
            64,
            Arc::new(QuietJournal),
        )
        .expect("create backend"),
    );
    backend
        .open_deterministic(true, NUDB_APPNUM, 8_001, 9_001)
        .expect("open backend");

    let objects = make_object_matrix(WRITERS, OBJECTS_PER_WRITER);
    let start = Arc::new(Barrier::new(WRITERS));
    let mut handles = Vec::with_capacity(WRITERS);

    for writer_objects in &objects {
        let backend = Arc::clone(&backend);
        let start = Arc::clone(&start);
        let writer_objects = writer_objects.clone();
        handles.push(thread::spawn(move || {
            start.wait();
            for (index, object) in writer_objects.iter().enumerate() {
                backend.store(Arc::clone(object));
                if index % 10 == 0 {
                    let (fetched, status) = backend.fetch(object.hash());
                    assert_eq!(status, Status::Ok);
                    assert_eq!(fetched.expect("just stored object").data(), object.data());
                }
            }
        }));
    }

    for handle in handles {
        handle.join().expect("writer thread panic");
    }

    for object in objects.iter().flatten() {
        let (fetched, status) = backend.fetch(object.hash());
        assert_eq!(status, Status::Ok);
        assert_eq!(fetched.expect("stored object").data(), object.data());
    }

    backend.close().expect("close backend");
}

#[test]
fn nudb_store_concurrent_writers_round_trip_after_reopen() {
    const WRITERS: usize = 8;
    const OBJECTS_PER_WRITER: usize = 32;

    let dir = TestDir::new("store-concurrent-reopen");
    let section = nudb_section(dir.path());
    let backend = Arc::new(
        NuDbBackend::new(NodeObject::KEY_BYTES, &section, 64, Arc::new(QuietJournal))
            .expect("create backend"),
    );
    backend
        .open_deterministic(true, NUDB_APPNUM, 8_101, 9_101)
        .expect("open backend");

    let objects = make_object_matrix(WRITERS, OBJECTS_PER_WRITER);
    let start = Arc::new(Barrier::new(WRITERS));
    let mut handles = Vec::with_capacity(WRITERS);

    for writer_objects in &objects {
        let backend = Arc::clone(&backend);
        let start = Arc::clone(&start);
        let writer_objects = writer_objects.clone();
        handles.push(thread::spawn(move || {
            start.wait();
            for object in writer_objects {
                backend.store(object);
            }
        }));
    }

    for handle in handles {
        handle.join().expect("writer thread panic");
    }
    backend.close().expect("close backend after writes");

    let reopened = NuDbBackend::new(NodeObject::KEY_BYTES, &section, 64, Arc::new(QuietJournal))
        .expect("recreate backend");
    reopened
        .open_deterministic(false, NUDB_APPNUM, 1, 1)
        .expect("reopen backend");

    for object in objects.iter().flatten() {
        let (fetched, status) = reopened.fetch(object.hash());
        assert_eq!(status, Status::Ok);
        assert_eq!(
            fetched.expect("stored object after reopen").data(),
            object.data()
        );
    }

    let mut seen = BTreeSet::new();
    reopened.for_each(&mut |object| {
        seen.insert(*object.hash().data());
    });
    assert_eq!(seen.len(), WRITERS * OBJECTS_PER_WRITER);

    reopened.close().expect("close reopened backend");
}
