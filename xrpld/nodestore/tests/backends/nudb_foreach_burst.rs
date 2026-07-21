use basics::base_uint::Uint256;
use basics::basic_config::Section;
use nodestore::{
    Backend, EncodedBlob, JournalLevel, NUDB_APPNUM, NodeObject, NodeObjectType, NodeStoreJournal,
    NuDbBackend, Status, nodeobject_compress,
};
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;
use tempfile::TempDir;

struct QuietJournal;

impl NodeStoreJournal for QuietJournal {
    fn log(&self, _level: JournalLevel, _message: &str) {}
}

fn nudb_section(path: &std::path::Path) -> Section {
    let mut section = Section::new("node_db");
    section.set("path", path.to_string_lossy().into_owned());
    section
}

fn object(fill: u8, payload: &[u8]) -> Arc<NodeObject> {
    Arc::new(NodeObject::new(
        NodeObjectType::Ledger,
        payload.to_vec(),
        Uint256::from_array([fill; 32]),
    ))
}

fn append_encoded_orphan_data_record(dir: &std::path::Path, object: &Arc<NodeObject>) {
    let encoded = EncodedBlob::new(object.as_ref());
    let compressed = nodeobject_compress(encoded.get_data()).expect("compress encoded blob");
    let mut file = OpenOptions::new()
        .append(true)
        .open(dir.join("nudb.dat"))
        .expect("open data file");
    let len = compressed.len() as u64;
    file.write_all(&[
        ((len >> 40) & 0xff) as u8,
        ((len >> 32) & 0xff) as u8,
        ((len >> 24) & 0xff) as u8,
        ((len >> 16) & 0xff) as u8,
        ((len >> 8) & 0xff) as u8,
        (len & 0xff) as u8,
    ])
    .expect("write size");
    file.write_all(encoded.get_key()).expect("write orphan key");
    file.write_all(&compressed).expect("write orphan value");
    file.flush().expect("flush orphan record");
}

fn log_size(dir: &std::path::Path) -> u64 {
    std::fs::metadata(dir.join("nudb.log"))
        .expect("log metadata")
        .len()
}

#[test]
fn nudb_for_each_visits_indexed_records_and_skips_orphan_data_file_rows() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        8,
        Arc::new(QuietJournal),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 7001, 8001)
        .expect("open");

    let first = object(0x11, b"indexed-first");
    let second = object(0x22, b"indexed-second");
    let orphan = object(0x33, b"orphan-only-in-dat");
    backend.store(Arc::clone(&first));
    backend.store(Arc::clone(&second));
    append_encoded_orphan_data_record(temp.path(), &orphan);

    let mut seen = Vec::new();
    backend.for_each(&mut |entry| seen.push(*entry.hash()));
    let seen_set: BTreeSet<_> = seen.into_iter().collect();

    let expected_set: BTreeSet<_> = [*first.hash(), *second.hash()].into_iter().collect();
    assert_eq!(seen_set, expected_set);
    assert!(!seen_set.contains(orphan.hash()));
}

#[test]
fn nudb_bulk_import_flushes_deferred_buckets_before_reopen() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(QuietJournal),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 7301, 8301)
        .expect("open");

    let first = object(0x71, b"bulk-first");
    let second = object(0x72, b"bulk-second");
    backend.bulk_import_start(2).expect("start bulk import");
    backend.store(Arc::clone(&first));
    backend.store(Arc::clone(&second));
    backend.bulk_import_finish().expect("finish bulk import");
    backend.close().expect("close");

    let reopened = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(QuietJournal),
    )
    .expect("reopen backend");
    reopened
        .open_deterministic(false, NUDB_APPNUM, 1, 1)
        .expect("reopen");
    for item in [&first, &second] {
        let (fetched, status) = reopened.fetch(item.hash());
        assert_eq!(status, Status::Ok);
        assert_eq!(fetched.expect("bulk item").data(), item.data());
    }
}

#[test]
fn nudb_burst_size_changes_checkpoint_commit_policy_without_claiming_exact_cpp_crash_semantics() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        4,
        Arc::new(QuietJournal),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 7101, 8101)
        .expect("open");

    let mut stored = Vec::new();
    for fill in [0x41, 0x42, 0x43] {
        let item = object(fill, &[fill, fill ^ 0x55]);
        backend.store(Arc::clone(&item));
        stored.push(item);
    }
    assert!(
        log_size(temp.path()) > 0,
        "burst_size=4 should keep a pending checkpoint before the fourth write commits"
    );

    let fourth = object(0x44, b"burst-commit-edge");
    backend.store(Arc::clone(&fourth));
    stored.push(fourth);
    assert_eq!(
        log_size(temp.path()),
        0,
        "fourth write should commit and clear the pending checkpoint"
    );

    let fifth = object(0x45, b"burst-close-commit");
    backend.store(Arc::clone(&fifth));
    stored.push(fifth);
    assert!(
        log_size(temp.path()) > 0,
        "a new burst should start after the previous one commits"
    );

    backend.close().expect("close flushes pending burst");
    assert_eq!(
        log_size(temp.path()),
        0,
        "close should commit pending burst"
    );

    let reopened = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        4,
        Arc::new(QuietJournal),
    )
    .expect("reopen backend");
    reopened
        .open_deterministic(false, NUDB_APPNUM, 1, 1)
        .expect("reopen");
    for item in &stored {
        let (fetched, status) = reopened.fetch(item.hash());
        assert_eq!(status, Status::Ok);
        assert_eq!(fetched.expect("reopened item").data(), item.data());
    }

    let immediate_dir = TempDir::new().expect("tempdir");
    let immediate = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(immediate_dir.path()),
        1,
        Arc::new(QuietJournal),
    )
    .expect("immediate backend");
    immediate
        .open_deterministic(true, NUDB_APPNUM, 7201, 8201)
        .expect("open immediate");
    immediate.store(object(0x99, b"immediate-commit"));
    assert_eq!(
        log_size(immediate_dir.path()),
        0,
        "burst_size=1 keeps per-write commit behavior"
    );
}
