use basics::base_uint::Uint256;
use basics::basic_config::Section;
use nodestore::{
    Backend, JournalLevel, NUDB_APPNUM, NodeObject, NodeObjectType, NodeStoreJournal, NuDbBackend,
    Status,
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

fn nudb_section(path: &Path) -> Section {
    let mut section = Section::new("node_db");
    section.set("path", path.to_string_lossy().into_owned());
    section
}

fn hash_prefix(hash: &Uint256, salt: u64) -> u64 {
    (xxh64(hash.data(), salt) >> 16) & 0x0000_FFFF_FFFF
}

fn bucket_zero_collision_objects(count: usize, salt: u64, modulus: u64) -> Vec<Arc<NodeObject>> {
    let mut objects = Vec::with_capacity(count);
    let mut candidate = 0u64;
    while objects.len() < count {
        let mut hash = [0u8; 32];
        hash[..8].copy_from_slice(&candidate.to_be_bytes());
        hash[8..16].copy_from_slice(&candidate.wrapping_mul(0x9E37_79B1_85EB_CA87).to_be_bytes());

        let key = Uint256::from_array(hash);
        if hash_prefix(&key, salt).is_multiple_of(modulus) {
            let mut payload = Vec::with_capacity(24);
            payload.extend_from_slice(&candidate.to_be_bytes());
            payload.extend_from_slice(&candidate.rotate_left(13).to_be_bytes());
            payload.extend_from_slice(&candidate.rotate_right(7).to_be_bytes());
            objects.push(Arc::new(NodeObject::new(
                NodeObjectType::Ledger,
                payload,
                key,
            )));
        }

        candidate = candidate
            .checked_add(1)
            .expect("candidate counter must not overflow");
        assert!(
            candidate < 3_000_000,
            "unable to build deterministic collision corpus"
        );
    }
    objects
}

fn read_u48_be(bytes: &[u8]) -> u64 {
    ((bytes[0] as u64) << 40)
        | ((bytes[1] as u64) << 32)
        | ((bytes[2] as u64) << 24)
        | ((bytes[3] as u64) << 16)
        | ((bytes[4] as u64) << 8)
        | (bytes[5] as u64)
}

fn read_key_bucket_spill(dir: &Path, block_size: u16, bucket_index: u32) -> u64 {
    let mut file = OpenOptions::new()
        .read(true)
        .open(dir.join("nudb.key"))
        .expect("open key file");
    let offset = (u64::from(bucket_index) + 1) * u64::from(block_size) + 2;
    file.seek(SeekFrom::Start(offset))
        .expect("seek bucket spill");
    let mut spill = [0u8; 6];
    file.read_exact(&mut spill).expect("read spill pointer");
    read_u48_be(&spill)
}

fn read_spill_next(dir: &Path, spill_offset: u64) -> u64 {
    let mut file = OpenOptions::new()
        .read(true)
        .open(dir.join("nudb.dat"))
        .expect("open data file");
    file.seek(SeekFrom::Start(spill_offset - 8))
        .expect("seek spill record");

    let mut marker = [0u8; 6];
    file.read_exact(&mut marker).expect("read spill marker");
    assert_eq!(marker, [0; 6], "spill marker must remain zeroed");

    let mut size = [0u8; 2];
    file.read_exact(&mut size).expect("read spill size");
    let compact_size = usize::from(u16::from_be_bytes(size));
    let mut compact = vec![0u8; compact_size];
    file.read_exact(&mut compact)
        .expect("read spill compact bytes");
    read_u48_be(&compact[2..8])
}

fn spill_chain_depth(dir: &Path, spill_head: u64) -> usize {
    let mut depth = 0usize;
    let mut seen = BTreeSet::new();
    let mut cursor = spill_head;
    while cursor != 0 {
        assert!(seen.insert(cursor), "spill chain must not loop");
        depth += 1;
        cursor = read_spill_next(dir, cursor);
    }
    depth
}

#[test]
fn nudb_backend_repeated_splits_and_spill_chain_survive_reopen() {
    const UID: u64 = 9_801;
    const SALT: u64 = 3_421;
    const COLLISION_MODULUS: u64 = 256;
    const OBJECT_COUNT: usize = 300;

    let temp = TempDir::new().expect("tempdir");
    let journal = Arc::new(RecordingJournal::default());
    let backend = NuDbBackend::new(
        NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        journal.clone(),
    )
    .expect("nudb backend");

    backend
        .open_deterministic(true, NUDB_APPNUM, UID, SALT)
        .expect("open deterministic");

    let objects = bucket_zero_collision_objects(OBJECT_COUNT, SALT, COLLISION_MODULUS);
    let mut observed_bucket_counts = Vec::new();
    for object in &objects {
        backend.store(Arc::clone(object));
        let buckets = backend
            .key_file_header()
            .expect("header after store")
            .buckets;
        if observed_bucket_counts.last().copied() != Some(buckets) {
            observed_bucket_counts.push(buckets);
        }
    }
    backend.sync();

    let grown_header = backend.key_file_header().expect("grown header");
    assert!(
        observed_bucket_counts.len() >= 3,
        "expected repeated bucket growth, observed counts: {observed_bucket_counts:?}"
    );
    assert!(
        grown_header.buckets > 2,
        "expected bucket count to grow more than once, got {}",
        grown_header.buckets
    );

    let spill_head = read_key_bucket_spill(temp.path(), grown_header.block_size, 0);
    assert!(spill_head != 0, "expected a spill chain head on bucket 0");
    let spill_depth = spill_chain_depth(temp.path(), spill_head);
    assert!(
        spill_depth >= 1,
        "expected at least one spill record, got depth {spill_depth}"
    );

    for object in &objects {
        let (fetched, status) = backend.fetch(object.hash());
        assert_eq!(status, Status::Ok);
        assert_eq!(fetched.expect("fetched object").data(), object.data());
    }
    backend.verify_backend().expect("verify before close");

    backend.close().expect("close backend");

    let reopened_journal = Arc::new(RecordingJournal::default());
    let reopened = NuDbBackend::new(
        NodeObject::KEY_BYTES,
        &nudb_section(temp.path()),
        64,
        reopened_journal.clone(),
    )
    .expect("reopen backend");
    reopened.open(false).expect("reopen");

    let reopened_header = reopened.key_file_header().expect("reopened header");
    assert_eq!(reopened_header.buckets, grown_header.buckets);
    assert_eq!(reopened_header.modulus, grown_header.modulus);
    let reopened_spill_head = read_key_bucket_spill(temp.path(), reopened_header.block_size, 0);
    assert_eq!(reopened_spill_head, spill_head);

    for object in &objects {
        let (fetched, status) = reopened.fetch(object.hash());
        assert_eq!(status, Status::Ok);
        assert_eq!(fetched.expect("reopened object").data(), object.data());
    }
    reopened.verify_backend().expect("verify after reopen");

    assert!(
        journal
            .entries
            .lock()
            .expect("recording journal mutex must not be poisoned")
            .is_empty(),
        "store/fetch/verify path should not emit journal errors"
    );
    assert!(
        reopened_journal
            .entries
            .lock()
            .expect("recording journal mutex must not be poisoned")
            .is_empty(),
        "reopened fetch/verify path should not emit journal errors"
    );
}
