use basics::base_uint::Uint256;
use basics::basic_config::Section;
use nodestore::{
    Backend, JournalLevel, NUDB_APPNUM, NodeObject, NodeObjectType, NodeStoreJournal, NuDbBackend,
    NuDbKeyFileHeader, NuDbLogFileHeader, Status, encode_nudb_log_file_header, nodeobject_compress,
    read_nudb_key_file_header,
};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
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

fn write_u48_be(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&[
        ((value >> 40) & 0xff) as u8,
        ((value >> 32) & 0xff) as u8,
        ((value >> 24) & 0xff) as u8,
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    ]);
}

fn malformed_unsorted_bucket_compact() -> Vec<u8> {
    let mut compact = Vec::with_capacity(8 + (2 * 18));
    compact.extend_from_slice(&2u16.to_be_bytes());
    write_u48_be(&mut compact, 0);
    write_u48_be(&mut compact, 10);
    write_u48_be(&mut compact, 20);
    write_u48_be(&mut compact, 9);
    write_u48_be(&mut compact, 30);
    write_u48_be(&mut compact, 40);
    write_u48_be(&mut compact, 3);
    compact
}

#[test]
fn nudb_recovery_checkpoint_is_idempotent_across_repeated_reopens() {
    let temp = TempDir::new().expect("tempdir");
    let committed = object(0x11, b"committed-recovery");
    let uncommitted = object(0x22, b"uncommitted-recovery");

    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(QuietJournal),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 3001, 4001)
        .expect("open");
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
        let mut bytes = nodestore::encode_nudb_key_file_header(&key_header).expect("key bytes");
        bytes.extend(vec![0u8; usize::from(key_header.block_size)]);
        bytes
    })
    .expect("rewrite key");
    append_orphan_data_record(temp.path(), &uncommitted);

    for _ in 0..3 {
        let reopened = NuDbBackend::new(
            nodestore::NodeObject::KEY_BYTES,
            &nudb_section(temp.path()),
            64,
            Arc::new(QuietJournal),
        )
        .expect("reopened backend");
        reopened
            .open_deterministic(false, NUDB_APPNUM, 1, 1)
            .expect("reopen");

        let (fetched, status) = reopened.fetch(committed.hash());
        assert_eq!(status, Status::Ok);
        assert_eq!(fetched.expect("committed").data(), committed.data());

        let (missing, missing_status) = reopened.fetch(uncommitted.hash());
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
        reopened.close().expect("close reopened");
    }
}

#[test]
fn nudb_recovery_truncated_log_tail_is_fail_open_and_clears_log() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(QuietJournal),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 3101, 4101)
        .expect("open");

    let committed = object(0x33, b"truncated-tail-committed");
    let uncommitted = object(0x44, b"truncated-tail-uncommitted");
    backend.store(Arc::clone(&committed));
    backend.close().expect("close");

    let key_header = read_nudb_key_file_header(&temp.path().join("nudb.key")).expect("key header");
    let key_file_size = std::fs::metadata(temp.path().join("nudb.key"))
        .expect("key metadata")
        .len();
    let dat_file_size = std::fs::metadata(temp.path().join("nudb.dat"))
        .expect("data metadata")
        .len();
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
    bytes.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
    std::fs::write(temp.path().join("nudb.log"), bytes).expect("write malformed log tail");

    append_orphan_data_record(temp.path(), &uncommitted);

    backend
        .open_deterministic(false, NUDB_APPNUM, 1, 1)
        .expect("reopen must fail-open");

    let (fetched, status) = backend.fetch(committed.hash());
    assert_eq!(status, Status::Ok);
    assert_eq!(fetched.expect("committed").data(), committed.data());

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
}

#[test]
fn nudb_recovery_rejects_unsupported_unsorted_bucket_entries() {
    let temp = TempDir::new().expect("tempdir");
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        Arc::new(QuietJournal),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 3201, 4201)
        .expect("open");
    backend.close().expect("close");

    let key_header = read_nudb_key_file_header(&temp.path().join("nudb.key")).expect("key header");
    let key_file_size = std::fs::metadata(temp.path().join("nudb.key"))
        .expect("key metadata")
        .len();
    let dat_file_size = std::fs::metadata(temp.path().join("nudb.dat"))
        .expect("data metadata")
        .len();

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
    bytes.extend_from_slice(&0u64.to_be_bytes());
    bytes.extend_from_slice(&malformed_unsorted_bucket_compact());
    std::fs::write(temp.path().join("nudb.log"), bytes).expect("write malformed recovery log");

    assert_eq!(
        backend
            .open_deterministic(false, NUDB_APPNUM, 1, 1)
            .expect_err("unsupported recovery compact must reject open"),
        "NuDB bucket entries are not sorted by hash prefix"
    );
}
