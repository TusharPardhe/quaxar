use basics::base_uint::Uint256;
use basics::basic_config::Section;
use nodestore::{
    Backend, EncodedBlob, JournalLevel, NUDB_APPNUM, NodeObject, NodeObjectType, NodeStoreJournal,
    NuDbBackend, nodeobject_compress, read_nudb_key_file_header,
};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

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

fn read_u48(bytes: &[u8]) -> u64 {
    ((bytes[0] as u64) << 40)
        | ((bytes[1] as u64) << 32)
        | ((bytes[2] as u64) << 24)
        | ((bytes[3] as u64) << 16)
        | ((bytes[4] as u64) << 8)
        | bytes[5] as u64
}

fn write_u48(bytes: &mut [u8], value: u64) {
    bytes[..6].copy_from_slice(&[
        ((value >> 40) & 0xff) as u8,
        ((value >> 32) & 0xff) as u8,
        ((value >> 24) & 0xff) as u8,
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    ]);
}

fn read_bucket_block(dir: &std::path::Path, block_size: u16, bucket_index: u32) -> Vec<u8> {
    let mut file = OpenOptions::new()
        .read(true)
        .open(dir.join("nudb.key"))
        .expect("open key file");
    let offset = u64::from(bucket_index + 1) * u64::from(block_size);
    file.seek(SeekFrom::Start(offset)).expect("seek key bucket");
    let mut bytes = vec![0u8; usize::from(block_size)];
    file.read_exact(&mut bytes).expect("read key bucket");
    bytes
}

fn write_bucket_block(dir: &std::path::Path, block_size: u16, bucket_index: u32, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.join("nudb.key"))
        .expect("open key file");
    let offset = u64::from(bucket_index + 1) * u64::from(block_size);
    file.seek(SeekFrom::Start(offset)).expect("seek key bucket");
    file.write_all(bytes).expect("write key bucket");
    file.flush().expect("flush key bucket");
}

fn append_orphan_data_record(dir: &std::path::Path, object: &Arc<NodeObject>) {
    let encoded = EncodedBlob::new(object.as_ref());
    let compressed = nodeobject_compress(encoded.get_data()).expect("compress encoded blob");
    let mut file = OpenOptions::new()
        .append(true)
        .open(dir.join("nudb.dat"))
        .expect("open data file");
    write_u48_record(&mut file, compressed.len() as u64);
    file.write_all(object.hash().data())
        .expect("write orphan key");
    file.write_all(&compressed).expect("write orphan value");
    file.flush().expect("flush orphan record");
}

fn write_u48_record(file: &mut std::fs::File, value: u64) {
    file.write_all(&[
        ((value >> 40) & 0xff) as u8,
        ((value >> 32) & 0xff) as u8,
        ((value >> 24) & 0xff) as u8,
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    ])
    .expect("write u48");
}

fn append_empty_spill_bucket(dir: &std::path::Path) -> u64 {
    let mut file = OpenOptions::new()
        .append(true)
        .open(dir.join("nudb.dat"))
        .expect("open data file");
    let record_offset = file.seek(SeekFrom::End(0)).expect("seek spill end");
    file.write_all(&[0, 0, 0, 0, 0, 0]).expect("spill marker");
    file.write_all(&8u16.to_be_bytes()).expect("spill size");
    file.write_all(&[0u8; 8]).expect("spill compact bucket");
    file.flush().expect("flush spill record");
    record_offset + 8
}

fn write_spill_compact_next_pointer(dir: &std::path::Path, spill_offset: u64, next_spill: u64) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.join("nudb.dat"))
        .expect("open data file");
    file.seek(SeekFrom::Start(spill_offset + 2))
        .expect("seek spill compact pointer");
    write_u48_record(&mut file, next_spill);
    file.flush().expect("flush spill pointer update");
}

#[test]
fn nudb_verify_clean_backend_succeeds() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 11, 22)
        .expect("open");

    backend.store(object(0x11, b"verify-clean-a"));
    backend.store(object(0x22, b"verify-clean-b"));

    backend.verify_backend().expect("verify clean backend");
}

#[test]
fn nudb_verify_requires_empty_log_file() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 33, 44)
        .expect("open");

    std::fs::write(temp.path().join("nudb.log"), [0xff]).expect("write log corruption");

    assert_eq!(
        backend.verify_backend().expect_err("verify must fail"),
        "NuDB verify requires an empty log file"
    );
}

#[test]
fn nudb_verify_detects_orphan_data_records() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 55, 66)
        .expect("open");

    backend.store(object(0x31, b"verify-indexed"));
    let orphan = object(0x32, b"verify-orphan");
    append_orphan_data_record(temp.path(), &orphan);

    assert_eq!(
        backend.verify_backend().expect_err("verify must fail"),
        "NuDB data file contains an orphan value record"
    );
}

#[test]
fn nudb_verify_detects_inconsistent_key_metadata_size() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 77, 88)
        .expect("open");

    backend.store(object(0x41, b"verify-size-mismatch"));

    let header = read_nudb_key_file_header(&temp.path().join("nudb.key")).expect("key header");
    let mut bucket = read_bucket_block(temp.path(), header.block_size, 0);
    let count = u16::from_be_bytes([bucket[0], bucket[1]]);
    assert!(count >= 1, "expected at least one key entry");
    let size_offset = 8 + 6;
    let size = read_u48(&bucket[size_offset..size_offset + 6]);
    write_u48(
        &mut bucket[size_offset..size_offset + 6],
        size.checked_add(1).expect("size increment"),
    );
    write_bucket_block(temp.path(), header.block_size, 0, &bucket);

    assert_eq!(
        backend.verify_backend().expect_err("verify must fail"),
        "NuDB data record size does not match key bucket metadata"
    );
}

#[test]
fn nudb_verify_detects_spill_chain_cycles() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(RecordingJournal::default()),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 99, 111)
        .expect("open");

    backend.store(object(0x51, b"verify-cycle"));

    let spill_offset = append_empty_spill_bucket(temp.path());
    write_spill_compact_next_pointer(temp.path(), spill_offset, spill_offset);

    let header = read_nudb_key_file_header(&temp.path().join("nudb.key")).expect("key header");
    let mut bucket = read_bucket_block(temp.path(), header.block_size, 0);
    write_u48(&mut bucket[2..8], spill_offset);
    write_bucket_block(temp.path(), header.block_size, 0, &bucket);

    assert_eq!(
        backend.verify_backend().expect_err("verify must fail"),
        "NuDB spill chain contains a cycle"
    );
}
