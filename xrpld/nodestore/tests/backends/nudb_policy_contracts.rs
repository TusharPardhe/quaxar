use basics::{base_uint::Uint256, basic_config::Section};
use nodestore::{
    Backend, JournalLevel, NUDB_APPNUM, NodeObject, NodeObjectType, NodeStoreJournal, NuDbBackend,
    NuDbKeyFileHeader, NuDbLogFileHeader, Status, encode_nudb_key_file_header,
    encode_nudb_log_file_header, nodeobject_compress, read_nudb_key_file_header,
};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

struct QuietJournal;

impl NodeStoreJournal for QuietJournal {
    fn log(&self, _level: JournalLevel, _message: &str) {}
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nodestore-policy-contracts")
            .join(format!("{name}-pid{}-{id}", std::process::id()));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale policy test dir");
        }
        fs::create_dir_all(&path).expect("create policy test dir");
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
            panic!(
                "failed to remove policy test dir {}: {error}",
                self.path.display()
            );
        }
    }
}

fn nudb_section(path: &Path) -> Section {
    let mut section = Section::new("node_db");
    section.set("path", path.to_string_lossy().into_owned());
    section
}

fn make_object(seed: u64, payload: &[u8]) -> Arc<NodeObject> {
    let mut hash = [0u8; 32];
    hash[..8].copy_from_slice(&seed.to_be_bytes());
    hash[8..16].copy_from_slice(&seed.rotate_left(19).to_be_bytes());
    Arc::new(NodeObject::new(
        NodeObjectType::Ledger,
        payload.to_vec(),
        Uint256::from_array(hash),
    ))
}

fn ceil_pow2_u32(mut x: u32) -> u32 {
    if x <= 1 {
        return 1;
    }
    x -= 1;
    x |= x >> 1;
    x |= x >> 2;
    x |= x >> 4;
    x |= x >> 8;
    x |= x >> 16;
    x + 1
}

fn minimum_cpp_import_buckets(item_count: u32, capacity: u16) -> u32 {
    let denominator = f64::from(capacity) * 0.50;
    ((f64::from(item_count) / denominator).ceil() as u32).max(1)
}

fn append_orphan_data_record(dir: &Path, object: &Arc<NodeObject>) {
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

fn read_bucket_compact_bytes(dir: &Path, block_size: u16, bucket_index: u32) -> Vec<u8> {
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
    dir: &Path,
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
    fs::write(dir.join("nudb.log"), bytes).expect("write recovery log");
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
fn nudb_split_policy_formulas_and_bounds() {
    const OBJECT_COUNT: u32 = 320;

    let dir = TestDir::new("split-policy");
    let backend = NuDbBackend::new(
        NodeObject::KEY_BYTES,
        &nudb_section(dir.path()),
        64,
        Arc::new(QuietJournal),
    )
    .expect("create backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 11_001, 22_001)
        .expect("open backend");

    backend.store(make_object(1, b"policy-split-1"));
    let first = backend.key_file_header().expect("header after first store");
    assert_eq!(first.buckets, 1);
    assert_eq!(first.modulus, 1);
    assert!(first.capacity > 0, "bucket capacity must be non-zero");

    let capacity = first.capacity;
    let mut previous_buckets = first.buckets;
    let mut previous_modulus = first.modulus;
    let mut split_events = 0u32;

    for index in 2..=OBJECT_COUNT {
        let payload = format!("policy-split-{index}");
        backend.store(make_object(u64::from(index), payload.as_bytes()));
        let header = backend
            .key_file_header()
            .expect("header after split-policy store");

        assert_eq!(
            header.modulus,
            ceil_pow2_u32(header.buckets.max(1)),
            "modulus should track ceil_pow2(buckets) after store {index}"
        );

        let minimum = minimum_cpp_import_buckets(index, capacity);
        assert!(
            header.buckets >= minimum,
            "bucket count {} fell below C++ import lower bound {} at store {index}",
            header.buckets,
            minimum
        );

        if header.buckets == previous_buckets + 1 {
            split_events += 1;
            if previous_buckets == previous_modulus {
                assert_eq!(
                    header.modulus,
                    previous_modulus * 2,
                    "modulus should double when split crosses a power-of-two boundary"
                );
            } else {
                assert_eq!(
                    header.modulus, previous_modulus,
                    "modulus should stay stable when split does not cross a power-of-two boundary"
                );
            }
        } else {
            assert_eq!(
                header.buckets, previous_buckets,
                "bucket count should stay stable or grow by exactly one per write"
            );
            assert_eq!(
                header.modulus, previous_modulus,
                "modulus should not change without a split"
            );
        }

        previous_buckets = header.buckets;
        previous_modulus = header.modulus;
    }

    assert!(
        split_events >= 3,
        "expected repeated split events, got {split_events}"
    );
    backend
        .verify_backend()
        .expect("verify backend after split-policy run");
    backend.close().expect("close backend");
}

#[test]
fn nudb_recovery_policy_recovery_contract() {
    let dir = TestDir::new("recovery-policy");
    let backend = NuDbBackend::new(
        NodeObject::KEY_BYTES,
        &nudb_section(dir.path()),
        64,
        Arc::new(QuietJournal),
    )
    .expect("create backend");

    let committed = make_object(0x1001, b"policy-committed");
    let uncommitted = make_object(0x2002, b"policy-uncommitted");

    backend
        .open_deterministic(true, NUDB_APPNUM, 33_001, 44_001)
        .expect("open");
    backend.store(Arc::clone(&committed));
    backend.close().expect("close after committed write");

    let key_header = read_nudb_key_file_header(&dir.path().join("nudb.key")).expect("key header");
    let key_file_size = fs::metadata(dir.path().join("nudb.key"))
        .expect("key metadata")
        .len();
    let dat_file_size = fs::metadata(dir.path().join("nudb.dat"))
        .expect("data metadata")
        .len();
    let bucket_zero = read_bucket_compact_bytes(dir.path(), key_header.block_size, 0);

    write_log_recovery_snapshot(
        dir.path(),
        &key_header,
        key_file_size,
        dat_file_size,
        0,
        &bucket_zero,
    );

    fs::write(dir.path().join("nudb.key"), {
        let mut bytes = encode_nudb_key_file_header(&key_header).expect("key header bytes");
        bytes.extend(vec![0u8; usize::from(key_header.block_size)]);
        bytes
    })
    .expect("rewrite key file");
    append_orphan_data_record(dir.path(), &uncommitted);

    backend
        .open_deterministic(false, NUDB_APPNUM, 1, 1)
        .expect("checkpoint recovery should reopen cleanly");
    let (fetched, status) = backend.fetch(committed.hash());
    assert_eq!(status, Status::Ok);
    assert_eq!(fetched.expect("committed object").data(), committed.data());
    let (missing, missing_status) = backend.fetch(uncommitted.hash());
    assert_eq!(missing_status, Status::NotFound);
    assert!(missing.is_none());
    assert_eq!(
        fs::metadata(dir.path().join("nudb.dat"))
            .expect("data metadata")
            .len(),
        dat_file_size,
        "recovery should truncate uncommitted data tail"
    );
    assert_eq!(
        fs::metadata(dir.path().join("nudb.log"))
            .expect("log metadata")
            .len(),
        0,
        "recovery should clear the log once replay finishes"
    );
    backend.close().expect("close after first recovery check");

    let malformed_header = read_nudb_key_file_header(&dir.path().join("nudb.key"))
        .expect("header after first recovery");
    let malformed_log_header = NuDbLogFileHeader {
        version: malformed_header.version,
        uid: malformed_header.uid,
        appnum: malformed_header.appnum,
        key_size: malformed_header.key_size,
        salt: malformed_header.salt,
        pepper: malformed_header.pepper,
        block_size: malformed_header.block_size,
        key_file_size: fs::metadata(dir.path().join("nudb.key"))
            .expect("key metadata")
            .len(),
        dat_file_size: fs::metadata(dir.path().join("nudb.dat"))
            .expect("data metadata")
            .len(),
    };
    let mut malformed_log =
        encode_nudb_log_file_header(&malformed_log_header).expect("malformed log header bytes");
    malformed_log.extend_from_slice(&0u64.to_be_bytes());
    malformed_log.extend_from_slice(&malformed_unsorted_bucket_compact());
    fs::write(dir.path().join("nudb.log"), malformed_log).expect("write malformed log");

    assert_eq!(
        backend
            .open_deterministic(false, NUDB_APPNUM, 1, 1)
            .expect_err("unsorted compact should be rejected"),
        "NuDB bucket entries are not sorted by hash prefix"
    );
}
