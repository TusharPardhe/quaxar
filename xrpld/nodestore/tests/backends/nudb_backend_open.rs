use basics::base_uint::Uint256;
use basics::basic_config::Section;
use nodestore::{
    Backend, JournalLevel, NUDB_APPNUM, NUDB_CURRENT_VERSION, NodeObject, NodeObjectType,
    NodeStoreJournal, NuDbBackend, NuDbDataFileHeader, NuDbKeyFileHeader, NuDbLogFileHeader,
    NuDbOpenState, Status, encode_nudb_data_file_header, encode_nudb_key_file_header,
    encode_nudb_log_file_header, nodeobject_compress, nudb_bucket_capacity,
    nudb_encode_load_factor, nudb_pepper, read_nudb_data_file_header, read_nudb_key_file_header,
};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use xxhash_rust::xxh64::xxh64;

#[derive(Default)]
struct RecordingJournal {
    entries: Mutex<Vec<(JournalLevel, String)>>,
}

impl NodeStoreJournal for RecordingJournal {
    fn log(&self, level: JournalLevel, message: &str) {
        self.entries
            .lock()
            .expect("recording journal mutex must not be poisoned")
            .push((level, message.to_owned()));
    }
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

fn create_complete_file_set(dir: &std::path::Path, key_header: &NuDbKeyFileHeader) {
    let data_header =
        NuDbDataFileHeader::from_metadata(key_header.metadata_header()).expect("data header");
    std::fs::create_dir_all(dir).expect("dir");
    let mut key_file = encode_nudb_key_file_header(key_header).expect("key header bytes");
    key_file.extend(vec![0u8; usize::from(key_header.block_size)]);
    std::fs::write(dir.join("nudb.key"), key_file).expect("write key file");
    std::fs::write(
        dir.join("nudb.dat"),
        encode_nudb_data_file_header(&data_header).expect("data header bytes"),
    )
    .expect("write data file");
    std::fs::write(dir.join("nudb.log"), []).expect("write log file");
}

fn append_orphan_data_record(dir: &std::path::Path, object: &Arc<NodeObject>) {
    let compressed = nodeobject_compress(object.data()).expect("compress");
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
    file.write_all(object.hash().data()).expect("write key");
    file.write_all(&compressed).expect("write value");
    file.flush().expect("flush orphan record");
}

fn append_trailing_garbage(dir: &std::path::Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .append(true)
        .open(dir.join("nudb.dat"))
        .expect("open data file");
    file.write_all(bytes).expect("append garbage");
    file.flush().expect("flush garbage");
}

fn read_bucket_compact_bytes(dir: &std::path::Path, block_size: u16, bucket_index: u32) -> Vec<u8> {
    let mut file = OpenOptions::new()
        .read(true)
        .open(dir.join("nudb.key"))
        .expect("open key file");
    let offset = u64::from(bucket_index + 1) * u64::from(block_size);
    file.seek(SeekFrom::Start(offset)).expect("seek bucket");
    let mut block = vec![0u8; usize::from(block_size)];
    file.read_exact(&mut block).expect("read bucket block");
    let count = usize::from(u16::from_be_bytes([block[0], block[1]]));
    let compact_len = 8 + count * 18;
    block[..compact_len].to_vec()
}

fn write_log_recovery_snapshot(
    dir: &std::path::Path,
    key_header: &NuDbKeyFileHeader,
    key_file_size: u64,
    dat_file_size: u64,
    bucket_index: u32,
    bucket_compact: &[u8],
) {
    let log_header = NuDbLogFileHeader {
        version: key_header.version,
        uid: key_header.uid,
        appnum: key_header.appnum,
        key_size: key_header.key_size,
        salt: key_header.salt,
        pepper: key_header.pepper,
        block_size: key_header.block_size,
        key_file_size,
        dat_file_size,
    };
    let mut bytes = encode_nudb_log_file_header(&log_header).expect("log header bytes");
    bytes.extend_from_slice(&u64::from(bucket_index).to_be_bytes());
    bytes.extend_from_slice(bucket_compact);
    std::fs::write(dir.join("nudb.log"), bytes).expect("write recovery log");
}

fn append_real_data_record(dir: &std::path::Path, object: &Arc<NodeObject>) -> (u64, u64) {
    let encoded = nodestore::EncodedBlob::new(object.as_ref());
    let compressed = nodeobject_compress(encoded.get_data()).expect("compress encoded blob");
    let mut file = OpenOptions::new()
        .append(true)
        .open(dir.join("nudb.dat"))
        .expect("open data file");
    let offset = file.seek(SeekFrom::End(0)).expect("seek data end");
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
    file.write_all(object.hash().data()).expect("write key");
    file.write_all(&compressed).expect("write value");
    file.flush().expect("flush data record");
    (offset, len)
}

fn encode_bucket_block(entries: &[(u64, u64, u64)], spill: u64, block_size: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; block_size];
    bytes[0..2].copy_from_slice(&(entries.len() as u16).to_be_bytes());
    bytes[2..8].copy_from_slice(&[
        ((spill >> 40) & 0xff) as u8,
        ((spill >> 32) & 0xff) as u8,
        ((spill >> 24) & 0xff) as u8,
        ((spill >> 16) & 0xff) as u8,
        ((spill >> 8) & 0xff) as u8,
        (spill & 0xff) as u8,
    ]);
    let mut offset = 8usize;
    for (record_offset, size, hash_prefix) in entries {
        for value in [*record_offset, *size, *hash_prefix] {
            bytes[offset..offset + 6].copy_from_slice(&[
                ((value >> 40) & 0xff) as u8,
                ((value >> 32) & 0xff) as u8,
                ((value >> 24) & 0xff) as u8,
                ((value >> 16) & 0xff) as u8,
                ((value >> 8) & 0xff) as u8,
                (value & 0xff) as u8,
            ]);
            offset += 6;
        }
    }
    bytes
}

fn append_spill_bucket_record(
    dir: &std::path::Path,
    entries: &[(u64, u64, u64)],
    spill: u64,
) -> u64 {
    let compact = encode_bucket_block(entries, spill, 8 + 18 * entries.len());
    let mut file = OpenOptions::new()
        .append(true)
        .open(dir.join("nudb.dat"))
        .expect("open data file");
    let record_offset = file.seek(SeekFrom::End(0)).expect("seek spill end");
    file.write_all(&[0, 0, 0, 0, 0, 0]).expect("spill marker");
    file.write_all(&(compact.len() as u16).to_be_bytes())
        .expect("spill size");
    file.write_all(&compact).expect("spill bucket");
    file.flush().expect("flush spill");
    record_offset + 8
}

fn hash_prefix(hash: &Uint256, salt: u64) -> u64 {
    (xxh64(hash.data(), salt) >> 16) & 0x0000_FFFF_FFFF_FFFF
}

#[test]
fn nudb_backend_deterministic_create_writes_cpp_shaped_key_header_and_opens() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");

    backend
        .open_deterministic(true, NUDB_APPNUM, 41, 99)
        .expect("deterministic create");

    assert!(backend.is_open());
    assert_eq!(
        backend.get_name(),
        temp.path().to_string_lossy().into_owned()
    );
    assert_eq!(backend.get_block_size(), Some(4096));

    let data_header =
        read_nudb_data_file_header(&temp.path().join("nudb.dat")).expect("data header");
    let on_disk = read_nudb_key_file_header(&temp.path().join("nudb.key")).expect("disk header");
    assert_eq!(data_header.uid, 41);
    assert_eq!(data_header.appnum, NUDB_APPNUM);
    assert_eq!(on_disk.uid, 41);
    assert_eq!(on_disk.salt, 99);
    assert_eq!(on_disk.appnum, NUDB_APPNUM);
    assert_eq!(on_disk.buckets, 1);
    assert_eq!(backend.key_file_header(), Some(on_disk));

    let open_state: NuDbOpenState = backend.open_state();
    assert!(open_state.is_open());
    assert_eq!(open_state.header().uid, 41);
    assert_eq!(open_state.header().salt, 99);

    backend.close().expect("close");
    assert!(!backend.is_open());
}

#[test]
fn nudb_backend_open_existing_validates_and_adopts_on_disk_header() {
    let temp = TempDir::new().expect("tempdir");
    let key_header = NuDbKeyFileHeader {
        version: NUDB_CURRENT_VERSION,
        uid: 501,
        appnum: NUDB_APPNUM,
        key_size: 32,
        salt: 777,
        pepper: nudb_pepper(777),
        block_size: 8192,
        load_factor: nudb_encode_load_factor(0.5).expect("load factor"),
        capacity: nudb_bucket_capacity(8192),
        buckets: 1,
        modulus: 1,
    };
    create_complete_file_set(temp.path(), &key_header);

    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");

    backend
        .open_deterministic(false, NUDB_APPNUM, 1, 2)
        .expect("open existing");

    assert!(backend.is_open());
    assert_eq!(backend.get_block_size(), Some(8192));
    assert_eq!(backend.key_file_header(), Some(key_header));
    assert_eq!(backend.open_state().header().uid, 501);
    assert_eq!(backend.open_state().header().salt, 777);
}

#[test]
fn nudb_backend_rejects_existing_file_set_with_wrong_appnum() {
    let temp = TempDir::new().expect("tempdir");
    let key_header = NuDbKeyFileHeader {
        version: NUDB_CURRENT_VERSION,
        uid: 11,
        appnum: NUDB_APPNUM,
        key_size: 32,
        salt: 22,
        pepper: nudb_pepper(22),
        block_size: 4096,
        load_factor: nudb_encode_load_factor(0.5).expect("load factor"),
        capacity: nudb_bucket_capacity(4096),
        buckets: 1,
        modulus: 1,
    };
    create_complete_file_set(temp.path(), &key_header);

    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");

    assert_eq!(
        backend
            .open_deterministic(false, 9, 1, 2)
            .expect_err("wrong appnum must fail"),
        "nodestore: unknown appnum"
    );
    assert!(!backend.is_open());
}

#[test]
fn nudb_backend_close_deletes_path_when_requested() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("delete-me");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(&path),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");

    backend
        .open_deterministic(true, NUDB_APPNUM, 70, 80)
        .expect("deterministic create");
    backend.set_delete_path();
    backend.close().expect("close");

    assert!(!path.exists());
}

#[test]
fn nudb_backend_rejects_existing_file_set_with_mismatched_data_or_log_headers() {
    let temp = TempDir::new().expect("tempdir");
    let key_header = NuDbKeyFileHeader {
        version: NUDB_CURRENT_VERSION,
        uid: 11,
        appnum: NUDB_APPNUM,
        key_size: 32,
        salt: 22,
        pepper: nudb_pepper(22),
        block_size: 4096,
        load_factor: nudb_encode_load_factor(0.5).expect("load factor"),
        capacity: nudb_bucket_capacity(4096),
        buckets: 1,
        modulus: 1,
    };
    create_complete_file_set(temp.path(), &key_header);
    let mismatched_data = NuDbDataFileHeader {
        version: NUDB_CURRENT_VERSION,
        uid: 99,
        appnum: NUDB_APPNUM,
        key_size: 32,
    };
    std::fs::write(
        temp.path().join("nudb.dat"),
        encode_nudb_data_file_header(&mismatched_data).expect("data bytes"),
    )
    .expect("rewrite data header");

    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    assert_eq!(
        backend
            .open_deterministic(false, NUDB_APPNUM, 1, 2)
            .expect_err("mismatched data header must fail"),
        "NuDB data header uid mismatch"
    );

    let valid_log = NuDbLogFileHeader {
        version: NUDB_CURRENT_VERSION,
        uid: key_header.uid,
        appnum: key_header.appnum,
        key_size: key_header.key_size,
        salt: key_header.salt,
        pepper: key_header.pepper,
        block_size: key_header.block_size,
        key_file_size: 4096,
        dat_file_size: 92,
    };
    std::fs::write(
        temp.path().join("nudb.dat"),
        encode_nudb_data_file_header(
            &NuDbDataFileHeader::from_metadata(key_header.metadata_header()).expect("data header"),
        )
        .expect("data bytes"),
    )
    .expect("restore data header");
    let mismatched_log = NuDbLogFileHeader {
        uid: 999,
        ..valid_log
    };
    std::fs::write(
        temp.path().join("nudb.log"),
        encode_nudb_log_file_header(&mismatched_log).expect("log bytes"),
    )
    .expect("write log header");
    assert_eq!(
        backend
            .open_deterministic(false, NUDB_APPNUM, 1, 2)
            .expect_err("mismatched log header must fail"),
        "NuDB log header uid mismatch"
    );
}

#[test]
fn nudb_backend_store_fetch_batch_and_for_each_use_real_data_records() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 71, 81)
        .expect("open");

    let first = object(0x11, b"first-record");
    let second = object(0x22, b"second-record");
    backend.store(Arc::clone(&first));
    backend.store(Arc::clone(&second));
    backend.store(Arc::clone(&first));

    let (fetched_first, status) = backend.fetch(first.hash());
    assert_eq!(status, Status::Ok);
    assert_eq!(fetched_first.expect("first object").data(), first.data());

    let (batch, batch_status) = backend.fetch_batch(&[*first.hash(), *second.hash()]);
    assert_eq!(batch_status, Status::Ok);
    assert_eq!(
        batch[0].as_ref().expect("first batch object").data(),
        first.data()
    );
    assert_eq!(
        batch[1].as_ref().expect("second batch object").data(),
        second.data()
    );

    let mut seen = Vec::new();
    backend.for_each(&mut |object| seen.push(*object.hash()));
    seen.sort();
    let mut expected = vec![*first.hash(), *second.hash()];
    expected.sort();
    assert_eq!(seen, expected);
}

#[test]
fn nudb_backend_fetch_does_not_scan_orphan_data_records_without_key_entries() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 90, 91)
        .expect("open");

    let orphan = object(0x33, b"orphan-record");
    append_orphan_data_record(temp.path(), &orphan);

    let (fetched, status) = backend.fetch(orphan.hash());
    assert_eq!(status, Status::NotFound);
    assert!(fetched.is_none());
}

#[test]
fn nudb_backend_duplicate_store_does_not_append_extra_data_records() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 92, 93)
        .expect("open");

    let item = object(0x44, b"dup-record");
    backend.store(Arc::clone(&item));
    let first_size = std::fs::metadata(temp.path().join("nudb.dat"))
        .expect("data metadata")
        .len();
    backend.store(Arc::clone(&item));
    let second_size = std::fs::metadata(temp.path().join("nudb.dat"))
        .expect("data metadata")
        .len();

    assert_eq!(first_size, second_size);
}

#[test]
fn nudb_backend_fetch_uses_bucket_entries_even_with_trailing_data_garbage() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 96, 97)
        .expect("open");

    let item = object(0x55, b"direct-bucket-fetch");
    backend.store(Arc::clone(&item));
    append_trailing_garbage(temp.path(), &[0xff]);

    let (fetched, status) = backend.fetch(item.hash());
    assert_eq!(status, Status::Ok);
    assert_eq!(fetched.expect("fetched item").data(), item.data());
}

#[test]
fn nudb_backend_duplicate_store_uses_key_entries_even_with_trailing_data_garbage() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 98, 99)
        .expect("open");

    let item = object(0x66, b"duplicate-with-garbage");
    backend.store(Arc::clone(&item));
    append_trailing_garbage(temp.path(), &[0xaa, 0xbb, 0xcc]);
    let corrupted_size = std::fs::metadata(temp.path().join("nudb.dat"))
        .expect("data metadata")
        .len();

    backend.store(Arc::clone(&item));

    let final_size = std::fs::metadata(temp.path().join("nudb.dat"))
        .expect("data metadata")
        .len();
    assert_eq!(final_size, corrupted_size);
}

#[test]
fn nudb_backend_reopen_recovers_from_log_checkpoint_and_truncates_uncommitted_data() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 100, 101)
        .expect("open");

    let committed = object(0x77, b"committed-record");
    backend.store(Arc::clone(&committed));
    backend.close().expect("close");

    let key_header = read_nudb_key_file_header(&temp.path().join("nudb.key")).expect("key header");
    let key_file_size = std::fs::metadata(temp.path().join("nudb.key"))
        .expect("key metadata")
        .len();
    let dat_file_size = std::fs::metadata(temp.path().join("nudb.dat"))
        .expect("data metadata")
        .len();
    let bucket_zero = read_bucket_compact_bytes(temp.path(), key_header.block_size, 0);
    write_log_recovery_snapshot(
        temp.path(),
        &key_header,
        key_file_size,
        dat_file_size,
        0,
        &bucket_zero,
    );

    std::fs::write(temp.path().join("nudb.key"), {
        let mut bytes = encode_nudb_key_file_header(&key_header).expect("key header bytes");
        bytes.extend(vec![0u8; usize::from(key_header.block_size)]);
        bytes
    })
    .expect("rewrite key file");
    let uncommitted = object(0x88, b"uncommitted-record");
    append_orphan_data_record(temp.path(), &uncommitted);

    backend
        .open_deterministic(false, NUDB_APPNUM, 1, 1)
        .expect("reopen with recovery");

    let (fetched, status) = backend.fetch(committed.hash());
    assert_eq!(status, Status::Ok);
    assert_eq!(fetched.expect("committed fetch").data(), committed.data());
    let (missing, missing_status) = backend.fetch(uncommitted.hash());
    assert_eq!(missing_status, Status::NotFound);
    assert!(missing.is_none());
    assert_eq!(
        std::fs::metadata(temp.path().join("nudb.dat"))
            .expect("data metadata")
            .len(),
        dat_file_size
    );
    assert_eq!(
        std::fs::metadata(temp.path().join("nudb.log"))
            .expect("log metadata")
            .len(),
        0
    );
    backend.verify_backend().expect("verify after recovery");
}

#[test]
fn nudb_backend_verify_detects_orphan_data_records() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 102, 103)
        .expect("open");

    let committed = object(0x99, b"verify-record");
    backend.store(Arc::clone(&committed));
    let orphan = object(0xaa, b"verify-orphan");
    append_orphan_data_record(temp.path(), &orphan);

    assert_eq!(
        backend.verify_backend().expect_err("verify should fail"),
        "NuDB data file contains an orphan value record"
    );
}

#[test]
fn nudb_backend_spill_chain_survives_reopen_and_fetches_late_entries() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 94, 95)
        .expect("open");

    backend.close().expect("close");

    let late = object(0x77, b"spill-record");
    let (record_offset, record_size) = append_real_data_record(temp.path(), &late);
    let spill_offset = append_spill_bucket_record(
        temp.path(),
        &[(record_offset, record_size, hash_prefix(late.hash(), 95))],
        0,
    );
    let header = read_nudb_key_file_header(&temp.path().join("nudb.key")).expect("disk header");
    let mut key_file = encode_nudb_key_file_header(&header).expect("header bytes");
    key_file.extend(encode_bucket_block(
        &[],
        spill_offset,
        usize::from(header.block_size),
    ));
    std::fs::write(temp.path().join("nudb.key"), key_file).expect("rewrite key file");

    backend
        .open_deterministic(false, NUDB_APPNUM, 1, 1)
        .expect("reopen");

    let (fetched, status) = backend.fetch(late.hash());
    assert_eq!(status, Status::Ok);
    assert_eq!(fetched.expect("late spill fetch").data(), late.data());
}

#[test]
fn nudb_backend_grows_key_buckets_and_preserves_entries_after_reopen() {
    let temp = TempDir::new().expect("tempdir");
    let journal = Arc::new(RecordingJournal::default());
    let journal_sink: Arc<dyn NodeStoreJournal> = journal.clone();
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        journal_sink,
    )
    .expect("nudb backend");

    backend
        .open_deterministic(true, NUDB_APPNUM, 5001, 7001)
        .expect("open");

    let mut objects = Vec::new();
    for index in 0u8..120 {
        let object = object(index, &[index, index.wrapping_add(1), index ^ 0x5A]);
        backend.store(Arc::clone(&object));
        objects.push(object);
    }
    backend.sync();

    let grown_header = backend.key_file_header().expect("grown key header");
    assert!(
        grown_header.buckets > 1,
        "expected bucket growth after enough inserts, got {} bucket(s)",
        grown_header.buckets
    );

    for object in &objects {
        let (fetched, status) = backend.fetch(object.hash());
        assert_eq!(status, Status::Ok);
        assert_eq!(fetched.expect("fetched object").data(), object.data());
    }
    backend.verify();
    assert!(
        journal
            .entries
            .lock()
            .expect("recording journal mutex must not be poisoned")
            .is_empty(),
        "verify/fetch should not log errors during bucket-growth path"
    );

    backend.close().expect("close");

    let reopened = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("reopen backend");
    reopened.open(false).expect("reopen");

    let reopened_header = reopened.key_file_header().expect("reopened header");
    assert_eq!(reopened_header.buckets, grown_header.buckets);
    assert_eq!(reopened_header.modulus, grown_header.modulus);

    for object in &objects {
        let (fetched, status) = reopened.fetch(object.hash());
        assert_eq!(status, Status::Ok);
        assert_eq!(fetched.expect("reopened object").data(), object.data());
    }
    reopened.verify();
}
