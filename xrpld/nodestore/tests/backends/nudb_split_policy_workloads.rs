use basics::base_uint::Uint256;
use basics::basic_config::Section;
use nodestore::{
    Backend, JournalLevel, NUDB_APPNUM, NodeObject, NodeObjectType, NodeStoreJournal, NuDbBackend,
    NuDbKeyFileHeader, Status,
};
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SpillMetrics {
    total_links: usize,
    max_depth: usize,
    buckets_with_spill: usize,
    total_entries: usize,
    non_empty_buckets: usize,
    max_bucket_entries: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeSnapshot {
    header: NuDbKeyFileHeader,
    spill: SpillMetrics,
}

#[derive(Debug, Clone, Copy)]
enum WorkloadPhase {
    Collision {
        count: usize,
        modulus: u64,
        stream: u64,
    },
    NearUniform {
        count: usize,
        modulus: u64,
        stream: u64,
    },
}

fn nudb_section(path: &Path) -> Section {
    let mut section = Section::new("node_db");
    section.set("path", path.to_string_lossy().into_owned());
    section
}

fn hash_prefix(hash: &Uint256, salt: u64) -> u64 {
    (xxh64(hash.data(), salt) >> 16) & 0x0000_FFFF_FFFF
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn object_from_id(object_id: u64, stream: u64) -> Arc<NodeObject> {
    let mut hash = [0u8; 32];
    hash[..8].copy_from_slice(&object_id.to_be_bytes());

    let mixed0 = splitmix64(object_id ^ stream.rotate_left(5));
    let mixed1 = splitmix64(object_id.wrapping_add(0x9E37_79B1_85EB_CA87) ^ stream.rotate_left(17));
    let mixed2 = splitmix64(object_id.wrapping_add(0xC2B2_AE3D_27D4_EB4F) ^ stream.rotate_left(33));
    hash[8..16].copy_from_slice(&mixed0.to_be_bytes());
    hash[16..24].copy_from_slice(&mixed1.to_be_bytes());
    hash[24..32].copy_from_slice(&mixed2.to_be_bytes());

    let mut payload = Vec::with_capacity(32);
    payload.extend_from_slice(&object_id.to_be_bytes());
    payload.extend_from_slice(&stream.to_be_bytes());
    payload.extend_from_slice(&mixed0.to_be_bytes());
    payload.extend_from_slice(&mixed1.to_be_bytes());

    Arc::new(NodeObject::new(
        NodeObjectType::Ledger,
        payload,
        Uint256::from_array(hash),
    ))
}

fn collision_workload(
    count: usize,
    salt: u64,
    modulus: u64,
    stream: u64,
    next_id: &mut u64,
) -> Vec<Arc<NodeObject>> {
    let mut objects = Vec::with_capacity(count);
    let mut attempts = 0usize;
    while objects.len() < count {
        let object = object_from_id(*next_id, stream);
        *next_id = next_id.checked_add(1).expect("object id must not overflow");
        if hash_prefix(object.hash(), salt).is_multiple_of(modulus) {
            objects.push(object);
        }
        attempts = attempts
            .checked_add(1)
            .expect("attempt counter must not overflow");
        assert!(
            attempts < count.saturating_mul(20_000),
            "unable to build deterministic collision workload"
        );
    }
    objects
}

fn near_uniform_workload(
    count: usize,
    salt: u64,
    modulus: u64,
    stream: u64,
    next_id: &mut u64,
) -> Vec<Arc<NodeObject>> {
    let modulus_usize = usize::try_from(modulus).expect("modulus must fit usize");
    assert!(modulus_usize > 0, "modulus must be non-zero");

    let mut quota = vec![count / modulus_usize; modulus_usize];
    for q in quota.iter_mut().take(count % modulus_usize) {
        *q += 1;
    }

    let mut accepted = vec![0usize; modulus_usize];
    let mut objects = Vec::with_capacity(count);
    let mut attempts = 0usize;

    while objects.len() < count {
        let object = object_from_id(*next_id, stream);
        *next_id = next_id.checked_add(1).expect("object id must not overflow");
        let bucket = usize::try_from(hash_prefix(object.hash(), salt) % modulus)
            .expect("bucket index must fit usize");
        if accepted[bucket] < quota[bucket] {
            accepted[bucket] += 1;
            objects.push(object);
        }

        attempts = attempts
            .checked_add(1)
            .expect("attempt counter must not overflow");
        assert!(
            attempts < count.saturating_mul(8_000),
            "unable to build deterministic near-uniform workload"
        );
    }

    objects
}

fn open_backend(
    path: &Path,
    create_if_missing: bool,
    uid: u64,
    salt: u64,
    journal: Arc<RecordingJournal>,
) -> NuDbBackend {
    let backend = NuDbBackend::new(NodeObject::KEY_BYTES, &nudb_section(path), 64, journal)
        .expect("nudb backend");
    backend
        .open_deterministic(create_if_missing, NUDB_APPNUM, uid, salt)
        .expect("open deterministic");
    backend
}

fn read_u48_be(bytes: &[u8]) -> u64 {
    ((bytes[0] as u64) << 40)
        | ((bytes[1] as u64) << 32)
        | ((bytes[2] as u64) << 24)
        | ((bytes[3] as u64) << 16)
        | ((bytes[4] as u64) << 8)
        | (bytes[5] as u64)
}

fn read_key_bucket_head(path: &Path, block_size: u16, bucket_index: u32) -> (u16, u64) {
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .expect("open key file");
    let offset = (u64::from(bucket_index) + 1) * u64::from(block_size);
    file.seek(SeekFrom::Start(offset)).expect("seek key bucket");

    let mut head = [0u8; 8];
    file.read_exact(&mut head).expect("read key bucket head");
    (
        u16::from_be_bytes([head[0], head[1]]),
        read_u48_be(&head[2..8]),
    )
}

fn read_spill_head(path: &Path, spill_offset: u64) -> (u16, u64) {
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .expect("open data file");
    file.seek(SeekFrom::Start(spill_offset - 8))
        .expect("seek spill record");

    let mut marker = [0u8; 6];
    file.read_exact(&mut marker).expect("read spill marker");
    assert_eq!(marker, [0; 6], "spill marker must remain zeroed");

    let mut size = [0u8; 2];
    file.read_exact(&mut size).expect("read spill size");
    let compact_size = usize::from(u16::from_be_bytes(size));
    assert!(
        compact_size >= 8,
        "spill compact block must include count and next"
    );

    let mut compact = vec![0u8; compact_size];
    file.read_exact(&mut compact)
        .expect("read spill compact bytes");
    (
        u16::from_be_bytes([compact[0], compact[1]]),
        read_u48_be(&compact[2..8]),
    )
}

fn collect_spill_metrics(dir: &Path, header: NuDbKeyFileHeader) -> SpillMetrics {
    let key_path = dir.join("nudb.key");
    let dat_path = dir.join("nudb.dat");

    let mut total_links = 0usize;
    let mut max_depth = 0usize;
    let mut buckets_with_spill = 0usize;
    let mut total_entries = 0usize;
    let mut non_empty_buckets = 0usize;
    let mut max_bucket_entries = 0usize;

    for bucket_index in 0..header.buckets {
        let (count, mut spill) = read_key_bucket_head(&key_path, header.block_size, bucket_index);
        let mut bucket_entries = usize::from(count);
        let mut depth = 0usize;
        let mut seen_spills = BTreeSet::new();

        while spill != 0 {
            assert!(
                seen_spills.insert(spill),
                "spill chain must not loop for bucket {bucket_index}"
            );
            depth += 1;
            let (spill_count, next_spill) = read_spill_head(&dat_path, spill);
            bucket_entries += usize::from(spill_count);
            spill = next_spill;
        }

        total_links += depth;
        max_depth = max_depth.max(depth);
        if depth > 0 {
            buckets_with_spill += 1;
        }
        total_entries += bucket_entries;
        if bucket_entries > 0 {
            non_empty_buckets += 1;
        }
        max_bucket_entries = max_bucket_entries.max(bucket_entries);
    }

    SpillMetrics {
        total_links,
        max_depth,
        buckets_with_spill,
        total_entries,
        non_empty_buckets,
        max_bucket_entries,
    }
}

fn runtime_snapshot(backend: &NuDbBackend, dir: &Path) -> RuntimeSnapshot {
    let header = backend
        .key_file_header()
        .expect("key header must be present");
    RuntimeSnapshot {
        header,
        spill: collect_spill_metrics(dir, header),
    }
}

fn record_split_transition(trace: &mut Vec<NuDbKeyFileHeader>, header: NuDbKeyFileHeader) {
    if trace.last().copied() != Some(header) {
        trace.push(header);
    }
}

fn assert_split_progression(trace: &[NuDbKeyFileHeader]) {
    assert!(!trace.is_empty(), "split trace must not be empty");
    for headers in trace.windows(2) {
        let previous = headers[0];
        let next = headers[1];

        assert!(next.modulus >= 1, "modulus must remain positive");
        assert!(
            next.modulus.is_power_of_two(),
            "modulus must stay power-of-two"
        );
        assert!(
            next.buckets >= previous.buckets,
            "bucket count must not regress"
        );
        assert!(next.modulus >= previous.modulus, "modulus must not regress");

        if next.buckets > previous.buckets {
            assert_eq!(
                next.buckets,
                previous.buckets + 1,
                "split must add exactly one bucket"
            );
            if previous.buckets == previous.modulus {
                assert_eq!(
                    next.modulus,
                    previous.modulus.saturating_mul(2),
                    "modulus must double when split reaches current modulus"
                );
            } else {
                assert_eq!(
                    next.modulus, previous.modulus,
                    "modulus must stay stable between doubling points"
                );
            }
        } else {
            assert_eq!(
                next.modulus, previous.modulus,
                "modulus must not change without a split"
            );
        }
    }
}

fn assert_fetch_integrity(backend: &NuDbBackend, objects: &[Arc<NodeObject>]) {
    for object in objects {
        let (fetched, status) = backend.fetch(object.hash());
        assert_eq!(status, Status::Ok);
        assert_eq!(fetched.expect("fetched object").data(), object.data());
    }
}

fn assert_no_error_logs(journal: &Arc<RecordingJournal>) {
    let entries = journal
        .entries
        .lock()
        .expect("recording journal mutex must not be poisoned");
    assert!(
        entries
            .iter()
            .all(|(level, _)| !matches!(level, JournalLevel::Error | JournalLevel::Fatal)),
        "journal contains error-level entries: {entries:?}"
    );
}

#[test]
fn nudb_split_policy_heavy_collision_workload_equivalence() {
    const UID: u64 = 55_001;
    const SALT: u64 = 77_001;

    let temp = TempDir::new().expect("tempdir");
    let journal = Arc::new(RecordingJournal::default());
    let backend = open_backend(temp.path(), true, UID, SALT, Arc::clone(&journal));

    let mut next_id = 1u64;
    let objects = collision_workload(320, SALT, 256, 0xC011_1DE0, &mut next_id);

    let mut split_trace = vec![backend.key_file_header().expect("header after open")];
    let mut saw_spill_transition = false;

    for object in &objects {
        backend.store(Arc::clone(object));
        record_split_transition(
            &mut split_trace,
            backend.key_file_header().expect("header after store"),
        );
        if !saw_spill_transition {
            saw_spill_transition = runtime_snapshot(&backend, temp.path()).spill.total_links > 0;
        }
    }

    backend.sync();
    let before_close = runtime_snapshot(&backend, temp.path());

    assert_split_progression(&split_trace);
    assert!(
        before_close.header.buckets > 2,
        "expected repeated split growth"
    );
    assert!(
        saw_spill_transition,
        "heavy collision workload must trigger spill transition"
    );
    assert!(
        before_close.spill.max_depth >= 1,
        "expected at least one spill record"
    );
    assert_eq!(before_close.spill.total_entries, objects.len());

    assert_fetch_integrity(&backend, &objects);
    backend.verify_backend().expect("verify before close");

    backend.close().expect("close backend");
    assert_no_error_logs(&journal);

    let reopened_journal = Arc::new(RecordingJournal::default());
    let reopened = open_backend(temp.path(), false, UID, SALT, Arc::clone(&reopened_journal));

    let reopened_snapshot = runtime_snapshot(&reopened, temp.path());
    assert_eq!(reopened_snapshot, before_close);
    assert_fetch_integrity(&reopened, &objects);
    reopened.verify_backend().expect("verify after reopen");
    reopened.close().expect("close reopened backend");
    assert_no_error_logs(&reopened_journal);
}

#[test]
fn nudb_split_policy_near_uniform_workload_equivalence() {
    const UID: u64 = 55_002;
    const SALT: u64 = 77_002;

    let temp = TempDir::new().expect("tempdir");
    let journal = Arc::new(RecordingJournal::default());
    let backend = open_backend(temp.path(), true, UID, SALT, Arc::clone(&journal));

    let mut next_id = 10_000u64;
    let objects = near_uniform_workload(192, SALT, 16, 0xB0AD_5EED, &mut next_id);

    let mut split_trace = vec![backend.key_file_header().expect("header after open")];
    for object in &objects {
        backend.store(Arc::clone(object));
        record_split_transition(
            &mut split_trace,
            backend.key_file_header().expect("header after store"),
        );
    }

    backend.sync();
    let before_close = runtime_snapshot(&backend, temp.path());

    assert_split_progression(&split_trace);
    assert!(
        before_close.header.buckets >= 3,
        "near-uniform workload should still progress split schedule"
    );
    assert_eq!(before_close.spill.total_links, 0);
    assert_eq!(before_close.spill.max_depth, 0);
    assert_eq!(before_close.spill.total_entries, objects.len());
    assert!(before_close.spill.non_empty_buckets >= 2);
    assert!(
        before_close.spill.max_bucket_entries <= 120,
        "near-uniform workload should remain broadly distributed"
    );

    assert_fetch_integrity(&backend, &objects);
    backend.verify_backend().expect("verify before close");

    backend.close().expect("close backend");
    assert_no_error_logs(&journal);

    let reopened_journal = Arc::new(RecordingJournal::default());
    let reopened = open_backend(temp.path(), false, UID, SALT, Arc::clone(&reopened_journal));

    let reopened_snapshot = runtime_snapshot(&reopened, temp.path());
    assert_eq!(reopened_snapshot, before_close);
    assert_fetch_integrity(&reopened, &objects);
    reopened.verify_backend().expect("verify after reopen");
    reopened.close().expect("close reopened backend");
    assert_no_error_logs(&reopened_journal);
}

#[test]
fn nudb_split_policy_phased_mixed_workload_reopen_equivalence() {
    const UID: u64 = 55_003;
    const SALT: u64 = 77_003;

    let phases = [
        WorkloadPhase::Collision {
            count: 150,
            modulus: 128,
            stream: 0xABCD_0001,
        },
        WorkloadPhase::NearUniform {
            count: 96,
            modulus: 16,
            stream: 0xABCD_0002,
        },
        WorkloadPhase::Collision {
            count: 170,
            modulus: 256,
            stream: 0xABCD_0003,
        },
    ];

    let temp = TempDir::new().expect("tempdir");
    let mut current_journal = Arc::new(RecordingJournal::default());
    let mut backend = open_backend(temp.path(), true, UID, SALT, Arc::clone(&current_journal));

    let mut next_id = 100_000u64;
    let mut seen_hashes = BTreeSet::new();
    let mut all_objects = Vec::new();
    let mut split_trace = vec![backend.key_file_header().expect("header after open")];
    let mut saw_spill_transition = false;

    for (phase_index, phase) in phases.iter().enumerate() {
        let phase_objects = match *phase {
            WorkloadPhase::Collision {
                count,
                modulus,
                stream,
            } => collision_workload(count, SALT, modulus, stream, &mut next_id),
            WorkloadPhase::NearUniform {
                count,
                modulus,
                stream,
            } => near_uniform_workload(count, SALT, modulus, stream, &mut next_id),
        };

        for object in &phase_objects {
            let hash = *object.hash().data();
            assert!(
                seen_hashes.insert(hash),
                "workload hashes must remain unique"
            );
            backend.store(Arc::clone(object));
            record_split_transition(
                &mut split_trace,
                backend.key_file_header().expect("header after store"),
            );
        }

        all_objects.extend(phase_objects);
        backend.sync();
        let phase_snapshot = runtime_snapshot(&backend, temp.path());
        saw_spill_transition |= phase_snapshot.spill.total_links > 0;

        assert_eq!(phase_snapshot.spill.total_entries, all_objects.len());
        assert_fetch_integrity(&backend, &all_objects);
        backend.verify_backend().expect("verify within phase");

        if phase_index + 1 < phases.len() {
            backend.close().expect("close between phases");
            assert_no_error_logs(&current_journal);

            current_journal = Arc::new(RecordingJournal::default());
            backend = open_backend(temp.path(), false, UID, SALT, Arc::clone(&current_journal));

            let reopened_snapshot = runtime_snapshot(&backend, temp.path());
            assert_eq!(reopened_snapshot, phase_snapshot);
            assert_fetch_integrity(&backend, &all_objects);
            backend
                .verify_backend()
                .expect("verify after phased reopen");
        }
    }

    let before_final_close = runtime_snapshot(&backend, temp.path());
    assert_split_progression(&split_trace);
    assert!(before_final_close.header.buckets > 3);
    assert!(
        saw_spill_transition,
        "mixed phased workload must eventually transition into spill usage"
    );

    backend.close().expect("close final phased backend");
    assert_no_error_logs(&current_journal);

    let final_journal = Arc::new(RecordingJournal::default());
    let reopened = open_backend(temp.path(), false, UID, SALT, Arc::clone(&final_journal));
    let reopened_snapshot = runtime_snapshot(&reopened, temp.path());

    assert_eq!(reopened_snapshot, before_final_close);
    assert_fetch_integrity(&reopened, &all_objects);
    reopened.verify_backend().expect("verify final reopen");
    reopened.close().expect("close final reopened backend");
    assert_no_error_logs(&final_journal);
}
