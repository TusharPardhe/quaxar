#![allow(clippy::large_enum_variant)]
use basics::base_uint::Uint256;
use basics::basic_config::Section;
use nodestore::{
    Backend, JournalLevel, NUDB_APPNUM, NodeObject, NodeObjectType, NodeStoreJournal, NuDbBackend,
    NuDbKeyFileHeader, NuDbLogFileHeader, Status, encode_nudb_key_file_header,
    encode_nudb_log_file_header, nodeobject_compress, read_nudb_key_file_header,
};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

struct QuietJournal;

impl NodeStoreJournal for QuietJournal {
    fn log(&self, _level: JournalLevel, _message: &str) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutcomeClass {
    Recovers,
    Rejects,
    Truncates,
}

struct CrashFixture {
    _temp: TempDir,
    path: PathBuf,
    committed: Arc<NodeObject>,
    key_header: NuDbKeyFileHeader,
    key_file_size: u64,
    dat_file_size: u64,
    bucket_zero_compact: Vec<u8>,
}

fn nudb_section(path: &Path) -> Section {
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

fn malformed_over_capacity_bucket_compact(capacity: usize) -> Vec<u8> {
    let count = u16::try_from(capacity + 1).expect("capacity+1 must fit into u16");
    let compact_size = 8 + usize::from(count) * 18;
    let mut compact = vec![0u8; compact_size];
    compact[0..2].copy_from_slice(&count.to_be_bytes());
    compact
}

fn rewrite_key_file_with_zeroed_buckets(path: &Path, key_header: &NuDbKeyFileHeader) {
    let mut bytes = encode_nudb_key_file_header(key_header).expect("key header bytes");
    for _ in 0..key_header.buckets.max(1) {
        bytes.extend(vec![0u8; usize::from(key_header.block_size)]);
    }
    std::fs::write(path.join("nudb.key"), bytes).expect("rewrite key file with zeroed buckets");
}

fn log_header_bytes(
    key_header: &NuDbKeyFileHeader,
    key_file_size: u64,
    dat_file_size: u64,
) -> Vec<u8> {
    encode_nudb_log_file_header(&NuDbLogFileHeader {
        version: key_header.version,
        uid: key_header.uid,
        appnum: key_header.appnum,
        key_size: key_header.key_size,
        salt: key_header.salt,
        pepper: key_header.pepper,
        block_size: key_header.block_size,
        key_file_size,
        dat_file_size,
    })
    .expect("log header bytes")
}

fn write_log_bytes(path: &Path, bytes: &[u8]) {
    std::fs::write(path.join("nudb.log"), bytes).expect("write log bytes");
}

fn write_single_bucket_checkpoint_log(
    path: &Path,
    key_header: &NuDbKeyFileHeader,
    key_file_size: u64,
    dat_file_size: u64,
    bucket_index: u32,
    bucket_compact: &[u8],
) {
    let mut bytes = log_header_bytes(key_header, key_file_size, dat_file_size);
    bytes.extend_from_slice(&u64::from(bucket_index).to_be_bytes());
    bytes.extend_from_slice(bucket_compact);
    write_log_bytes(path, &bytes);
}

fn build_fixture(uid: u64, salt: u64, committed_fill: u8, payload: &[u8]) -> CrashFixture {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().to_path_buf();
    let committed = object(committed_fill, payload);
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(&path),
        64,
        Arc::new(QuietJournal),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, uid, salt)
        .expect("open deterministic");
    backend.store(Arc::clone(&committed));
    backend.close().expect("close");

    let key_header = read_nudb_key_file_header(&path.join("nudb.key")).expect("key header");
    let key_file_size = std::fs::metadata(path.join("nudb.key"))
        .expect("key metadata")
        .len();
    let dat_file_size = std::fs::metadata(path.join("nudb.dat"))
        .expect("data metadata")
        .len();
    let bucket_zero_compact = read_bucket_compact_bytes(&path, key_header.block_size, 0);
    CrashFixture {
        _temp: temp,
        path,
        committed,
        key_header,
        key_file_size,
        dat_file_size,
        bucket_zero_compact,
    }
}

fn assert_fetch(
    backend: &NuDbBackend,
    object: &Arc<NodeObject>,
    expected_status: Status,
    expect_present: bool,
) {
    let (fetched, status) = backend.fetch(object.hash());
    assert_eq!(status, expected_status);
    if expect_present {
        assert_eq!(fetched.expect("fetched value").data(), object.data());
    } else {
        assert!(fetched.is_none());
    }
}

enum ReopenAttempt {
    Opened {
        backend: NuDbBackend,
        class: OutcomeClass,
        data_size_after: u64,
    },
    Rejected(String),
}

fn reopen_with_class(path: &Path, data_size_before: u64) -> ReopenAttempt {
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(path),
        64,
        Arc::new(QuietJournal),
    )
    .expect("reopen backend");
    match backend.open_deterministic(false, NUDB_APPNUM, 1, 1) {
        Ok(()) => {
            let data_size_after = std::fs::metadata(path.join("nudb.dat"))
                .expect("data metadata after reopen")
                .len();
            let class = if data_size_after < data_size_before {
                OutcomeClass::Truncates
            } else {
                OutcomeClass::Recovers
            };
            ReopenAttempt::Opened {
                backend,
                class,
                data_size_after,
            }
        }
        Err(error) => ReopenAttempt::Rejected(error),
    }
}

#[test]
fn nudb_crash_recovery_matrix_checkpoint_replay_is_idempotent_across_reopen_cycles() {
    let fixture = build_fixture(9_101, 9_201, 0x11, b"checkpoint-replay-committed");
    let orphan = object(0x22, b"checkpoint-replay-orphan");
    write_single_bucket_checkpoint_log(
        &fixture.path,
        &fixture.key_header,
        fixture.key_file_size,
        fixture.dat_file_size,
        0,
        &fixture.bucket_zero_compact,
    );
    rewrite_key_file_with_zeroed_buckets(&fixture.path, &fixture.key_header);
    append_orphan_data_record(&fixture.path, &orphan);

    let mut data_size_before = std::fs::metadata(fixture.path.join("nudb.dat"))
        .expect("data metadata before first reopen")
        .len();

    for iteration in 0..4 {
        match reopen_with_class(&fixture.path, data_size_before) {
            ReopenAttempt::Opened {
                backend,
                class,
                data_size_after,
            } => {
                let expected_class = if iteration == 0 {
                    OutcomeClass::Truncates
                } else {
                    OutcomeClass::Recovers
                };
                assert_eq!(
                    class, expected_class,
                    "checkpoint replay iteration {iteration}"
                );
                assert_fetch(&backend, &fixture.committed, Status::Ok, true);
                assert_fetch(&backend, &orphan, Status::NotFound, false);
                assert_eq!(
                    std::fs::metadata(fixture.path.join("nudb.log"))
                        .expect("log metadata")
                        .len(),
                    0
                );
                assert_eq!(data_size_after, fixture.dat_file_size);
                backend.close().expect("close reopened backend");
                data_size_before = data_size_after;
            }
            ReopenAttempt::Rejected(error) => {
                panic!("checkpoint replay should recover, got rejection: {error}");
            }
        }
    }
}

#[test]
fn nudb_crash_recovery_matrix_torn_log_payload_variants_have_stable_outcomes() {
    let variants = [
        "short-header-clears-log",
        "tail-after-header-truncates",
        "torn-bucket-prefix-truncates",
        "torn-bucket-entry-truncates",
    ];

    for (index, variant) in variants.into_iter().enumerate() {
        let fixture = build_fixture(
            9_300 + index as u64,
            9_400 + index as u64,
            0x30 + index as u8,
            b"torn-log-committed",
        );
        let orphan = object(0x70 + index as u8, b"torn-log-orphan");

        let header = log_header_bytes(
            &fixture.key_header,
            fixture.key_file_size,
            fixture.dat_file_size,
        );
        let mut bytes = Vec::new();
        let mut data_size_before = fixture.dat_file_size;
        let expected_class = match variant {
            "short-header-clears-log" => {
                bytes.extend_from_slice(&header[..16]);
                OutcomeClass::Recovers
            }
            "tail-after-header-truncates" => {
                bytes.extend_from_slice(&header);
                bytes.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
                append_orphan_data_record(&fixture.path, &orphan);
                data_size_before = std::fs::metadata(fixture.path.join("nudb.dat"))
                    .expect("data metadata before reopen")
                    .len();
                OutcomeClass::Truncates
            }
            "torn-bucket-prefix-truncates" => {
                bytes.extend_from_slice(&header);
                bytes.extend_from_slice(&0u64.to_be_bytes());
                bytes.extend_from_slice(&fixture.bucket_zero_compact[..4]);
                append_orphan_data_record(&fixture.path, &orphan);
                data_size_before = std::fs::metadata(fixture.path.join("nudb.dat"))
                    .expect("data metadata before reopen")
                    .len();
                OutcomeClass::Truncates
            }
            "torn-bucket-entry-truncates" => {
                bytes.extend_from_slice(&header);
                bytes.extend_from_slice(&0u64.to_be_bytes());
                bytes.extend_from_slice(&fixture.bucket_zero_compact[..10]);
                append_orphan_data_record(&fixture.path, &orphan);
                data_size_before = std::fs::metadata(fixture.path.join("nudb.dat"))
                    .expect("data metadata before reopen")
                    .len();
                OutcomeClass::Truncates
            }
            _ => unreachable!("unknown variant"),
        };

        write_log_bytes(&fixture.path, &bytes);
        match reopen_with_class(&fixture.path, data_size_before) {
            ReopenAttempt::Opened {
                backend,
                class,
                data_size_after,
            } => {
                assert_eq!(class, expected_class, "variant={variant}");
                assert_fetch(&backend, &fixture.committed, Status::Ok, true);
                if expected_class == OutcomeClass::Truncates {
                    assert_fetch(&backend, &orphan, Status::NotFound, false);
                    assert_eq!(data_size_after, fixture.dat_file_size, "variant={variant}");
                } else {
                    assert_eq!(data_size_after, fixture.dat_file_size, "variant={variant}");
                }
                assert_eq!(
                    std::fs::metadata(fixture.path.join("nudb.log"))
                        .expect("log metadata")
                        .len(),
                    0,
                    "variant={variant}"
                );
                backend.close().expect("close backend");
            }
            ReopenAttempt::Rejected(error) => {
                panic!("variant {variant} should not reject, got: {error}");
            }
        }
    }
}

#[test]
fn nudb_crash_recovery_matrix_malformed_bucket_compact_variants_reject() {
    let variants = ["unsorted-hash-prefix", "entry-count-exceeds-capacity"];
    for (index, variant) in variants.into_iter().enumerate() {
        let fixture = build_fixture(
            9_500 + index as u64,
            9_600 + index as u64,
            0x41 + index as u8,
            b"malformed-compact-committed",
        );
        let malformed = match variant {
            "unsorted-hash-prefix" => malformed_unsorted_bucket_compact(),
            "entry-count-exceeds-capacity" => {
                malformed_over_capacity_bucket_compact(usize::from(fixture.key_header.capacity))
            }
            _ => unreachable!("unknown variant"),
        };
        write_single_bucket_checkpoint_log(
            &fixture.path,
            &fixture.key_header,
            fixture.key_file_size,
            fixture.dat_file_size,
            0,
            &malformed,
        );

        let class = match reopen_with_class(&fixture.path, fixture.dat_file_size) {
            ReopenAttempt::Opened { .. } => {
                panic!("variant {variant} must reject malformed compact");
            }
            ReopenAttempt::Rejected(error) => {
                let expected_error = match variant {
                    "unsorted-hash-prefix" => "NuDB bucket entries are not sorted by hash prefix",
                    "entry-count-exceeds-capacity" => "NuDB bucket entry count exceeds capacity",
                    _ => unreachable!("unknown variant"),
                };
                assert_eq!(error, expected_error, "variant={variant}");
                OutcomeClass::Rejects
            }
        };
        assert_eq!(class, OutcomeClass::Rejects, "variant={variant}");
    }
}

#[test]
fn nudb_crash_recovery_matrix_recover_store_recover_sequence_preserves_commits() {
    let fixture = build_fixture(9_701, 9_801, 0x51, b"recover-store-recover-committed-a");
    let orphan_a = object(0x52, b"recover-store-recover-orphan-a");
    write_single_bucket_checkpoint_log(
        &fixture.path,
        &fixture.key_header,
        fixture.key_file_size,
        fixture.dat_file_size,
        0,
        &fixture.bucket_zero_compact,
    );
    rewrite_key_file_with_zeroed_buckets(&fixture.path, &fixture.key_header);
    append_orphan_data_record(&fixture.path, &orphan_a);
    let data_before_first_reopen = std::fs::metadata(fixture.path.join("nudb.dat"))
        .expect("data metadata before first reopen")
        .len();

    let recovered = match reopen_with_class(&fixture.path, data_before_first_reopen) {
        ReopenAttempt::Opened {
            backend,
            class,
            data_size_after,
        } => {
            assert_eq!(class, OutcomeClass::Truncates);
            assert_eq!(data_size_after, fixture.dat_file_size);
            backend
        }
        ReopenAttempt::Rejected(error) => {
            panic!("first recovery must succeed, got: {error}");
        }
    };
    assert_fetch(&recovered, &fixture.committed, Status::Ok, true);
    assert_fetch(&recovered, &orphan_a, Status::NotFound, false);

    let post_recovery_committed = object(0x53, b"recover-store-recover-committed-b");
    recovered.store(Arc::clone(&post_recovery_committed));
    recovered.close().expect("close after post-recovery store");

    let second_header = read_nudb_key_file_header(&fixture.path.join("nudb.key")).expect("header");
    let second_key_file_size = std::fs::metadata(fixture.path.join("nudb.key"))
        .expect("key metadata")
        .len();
    let second_dat_file_size = std::fs::metadata(fixture.path.join("nudb.dat"))
        .expect("data metadata")
        .len();
    let second_bucket_zero = read_bucket_compact_bytes(&fixture.path, second_header.block_size, 0);
    write_single_bucket_checkpoint_log(
        &fixture.path,
        &second_header,
        second_key_file_size,
        second_dat_file_size,
        0,
        &second_bucket_zero,
    );
    rewrite_key_file_with_zeroed_buckets(&fixture.path, &second_header);
    let orphan_b = object(0x54, b"recover-store-recover-orphan-b");
    append_orphan_data_record(&fixture.path, &orphan_b);

    let data_before_second_reopen = std::fs::metadata(fixture.path.join("nudb.dat"))
        .expect("data metadata before second reopen")
        .len();
    let recovered_again = match reopen_with_class(&fixture.path, data_before_second_reopen) {
        ReopenAttempt::Opened {
            backend,
            class,
            data_size_after,
        } => {
            assert_eq!(class, OutcomeClass::Truncates);
            assert_eq!(data_size_after, second_dat_file_size);
            backend
        }
        ReopenAttempt::Rejected(error) => {
            panic!("second recovery must succeed, got: {error}");
        }
    };
    assert_fetch(&recovered_again, &fixture.committed, Status::Ok, true);
    assert_fetch(&recovered_again, &post_recovery_committed, Status::Ok, true);
    assert_fetch(&recovered_again, &orphan_b, Status::NotFound, false);
    recovered_again
        .close()
        .expect("close second recovery backend");

    let data_before_third_reopen = std::fs::metadata(fixture.path.join("nudb.dat"))
        .expect("data metadata before third reopen")
        .len();
    match reopen_with_class(&fixture.path, data_before_third_reopen) {
        ReopenAttempt::Opened {
            backend,
            class,
            data_size_after,
        } => {
            assert_eq!(class, OutcomeClass::Recovers);
            assert_eq!(data_size_after, second_dat_file_size);
            assert_fetch(&backend, &fixture.committed, Status::Ok, true);
            assert_fetch(&backend, &post_recovery_committed, Status::Ok, true);
            backend.close().expect("close idempotent reopen backend");
        }
        ReopenAttempt::Rejected(error) => {
            panic!("third reopen should recover, got: {error}");
        }
    }
}

#[test]
fn nudb_crash_recovery_matrix_truncates_key_buckets_created_after_checkpoint_split() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().to_path_buf();
    let backend = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(&path),
        10_000,
        Arc::new(QuietJournal),
    )
    .expect("nudb backend");
    backend
        .open_deterministic(true, NUDB_APPNUM, 10_101, 10_201)
        .expect("open deterministic");

    let checkpoint_key_size = std::fs::metadata(path.join("nudb.key"))
        .expect("initial key metadata")
        .len();
    let checkpoint_data_size = std::fs::metadata(path.join("nudb.dat"))
        .expect("initial data metadata")
        .len();
    let uncommitted = object(0x61, b"uncommitted-after-split");
    let mut stored = Vec::new();

    for index in 0..300u16 {
        let fill = 0x80u8.wrapping_add(index as u8);
        let item = Arc::new(NodeObject::new(
            NodeObjectType::Ledger,
            format!("checkpoint-split-uncommitted-{index:03}").into_bytes(),
            Uint256::from_array([fill; 32]),
        ));
        backend.store(Arc::clone(&item));
        stored.push(item);
    }
    backend.store(Arc::clone(&uncommitted));

    assert!(
        std::fs::metadata(path.join("nudb.log"))
            .expect("log metadata")
            .len()
            > 0,
        "large burst size should leave an active checkpoint"
    );
    assert!(
        std::fs::metadata(path.join("nudb.key"))
            .expect("key metadata after split")
            .len()
            > checkpoint_key_size,
        "workload should create key buckets after the checkpoint"
    );
    let current_key_size = std::fs::metadata(path.join("nudb.key"))
        .expect("key metadata after split")
        .len();
    let mut log = OpenOptions::new()
        .write(true)
        .open(path.join("nudb.log"))
        .expect("open log for legacy key size mutation");
    log.seek(SeekFrom::Start(46))
        .expect("seek log key_file_size");
    log.write_all(&current_key_size.to_be_bytes())
        .expect("write legacy enlarged key_file_size");
    log.sync_all().expect("sync mutated log");
    drop(backend);

    let reopened = NuDbBackend::new(
        nodestore::NodeObject::KEY_BYTES,
        &nudb_section(&path),
        10_000,
        Arc::new(QuietJournal),
    )
    .expect("reopen backend");
    reopened
        .open_deterministic(false, NUDB_APPNUM, 1, 1)
        .expect("reopen should recover checkpoint");

    assert_eq!(
        std::fs::metadata(path.join("nudb.key"))
            .expect("key metadata after recovery")
            .len(),
        checkpoint_key_size,
        "recovery must discard key buckets created after the active checkpoint"
    );
    assert_eq!(
        std::fs::metadata(path.join("nudb.dat"))
            .expect("data metadata after recovery")
            .len(),
        checkpoint_data_size,
        "recovery must discard data records created after the active checkpoint"
    );
    assert_eq!(
        std::fs::metadata(path.join("nudb.log"))
            .expect("log metadata after recovery")
            .len(),
        0,
        "recovery should clear the active checkpoint log"
    );
    for item in stored.iter().chain(std::iter::once(&uncommitted)) {
        assert_fetch(&reopened, item, Status::NotFound, false);
    }
    reopened
        .verify_backend()
        .expect("recovered backend must not contain key entries past the data file");
    reopened.close().expect("close recovered backend");
}
