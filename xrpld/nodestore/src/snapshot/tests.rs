use std::sync::Arc;

use basics::base_uint::Uint256;
use basics::basic_config::Section;

use crate::{Backend, Factory, MemoryFactory, NodeObject, NodeObjectType, NullJournal};
use crate::database_runtime::scheduler::DummyScheduler;
use crate::snapshot::{SnapshotError, SnapshotManifest, export_snapshot, load_snapshot};
use crate::snapshot::manifest::*;

fn config(path: &str) -> Section {
    let mut section = Section::new("node_db");
    section.set("type", "Memory");
    section.set("path", path);
    section
}

fn make_backend(path: &str) -> Box<dyn Backend> {
    let factory = MemoryFactory::new();
    let scheduler: Arc<dyn crate::Scheduler> = Arc::new(DummyScheduler);
    let journal: Arc<dyn crate::NodeStoreJournal> = Arc::new(NullJournal);
    let backend = factory
        .create_instance(NodeObject::KEY_BYTES, &config(path), 0, scheduler, journal)
        .expect("memory backend must be created");
    backend.open(true).expect("backend must open");
    backend
}

fn test_manifest() -> SnapshotManifest {
    SnapshotManifest {
        version: SNAPSHOT_VERSION,
        ledger_seq: 100,
        ledger_hash: [0xAA; 32],
        account_hash: [0xBB; 32],
        tx_hash: [0xCC; 32],
        parent_hash: [0xDD; 32],
        drops: 100_000_000_000,
        close_time: 750_000_000,
        parent_close_time: 749_999_990,
        close_time_res: 10,
        close_flags: 0,
        chunks: Vec::new(),
    }
}

#[test]
fn round_trip_export_load() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("test.xrpls");

    let src = make_backend("src-rt");
    // Store some nodes
    src.store(Arc::new(NodeObject::new(
        NodeObjectType::Ledger,
        vec![1, 2, 3, 4],
        Uint256::from_array([0x11; 32]),
    )));
    src.store(Arc::new(NodeObject::new(
        NodeObjectType::AccountNode,
        vec![5, 6, 7],
        Uint256::from_array([0x22; 32]),
    )));
    src.store(Arc::new(NodeObject::new(
        NodeObjectType::TransactionNode,
        vec![8, 9],
        Uint256::from_array([0x33; 32]),
    )));

    // Export
    let manifest = test_manifest();
    export_snapshot(src.as_ref(), &manifest, &snap_path).expect("export must succeed");

    // Load into fresh backend
    let dst = make_backend("dst-rt");
    let loaded_manifest = load_snapshot(dst.as_ref(), &snap_path).expect("load must succeed");

    // Verify manifest fields
    assert_eq!(loaded_manifest.ledger_seq, 100);
    assert_eq!(loaded_manifest.ledger_hash, [0xAA; 32]);
    assert_eq!(loaded_manifest.account_hash, [0xBB; 32]);

    // Verify all nodes are present
    let (obj, _) = dst.fetch(&Uint256::from_array([0x11; 32]));
    let obj = obj.expect("node 0x11 must exist");
    assert_eq!(obj.data().as_slice(), &[1, 2, 3, 4]);
    assert_eq!(obj.object_type(), NodeObjectType::Ledger);

    let (obj, _) = dst.fetch(&Uint256::from_array([0x22; 32]));
    let obj = obj.expect("node 0x22 must exist");
    assert_eq!(obj.data().as_slice(), &[5, 6, 7]);

    let (obj, _) = dst.fetch(&Uint256::from_array([0x33; 32]));
    let obj = obj.expect("node 0x33 must exist");
    assert_eq!(obj.data().as_slice(), &[8, 9]);
}

#[test]
fn corrupt_chunk_hash_detected() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("corrupt.xrpls");

    let src = make_backend("src-corrupt");
    src.store(Arc::new(NodeObject::new(
        NodeObjectType::Ledger,
        vec![1, 2, 3],
        Uint256::from_array([0x44; 32]),
    )));

    export_snapshot(src.as_ref(), &test_manifest(), &snap_path).expect("export must succeed");

    // Corrupt the chunk data (after header + chunk table)
    let mut data = std::fs::read(&snap_path).unwrap();
    let chunk_data_offset = SNAPSHOT_HEADER_SIZE + CHUNK_META_SIZE; // 1 chunk
    if chunk_data_offset < data.len() {
        data[chunk_data_offset] ^= 0xFF;
    }
    std::fs::write(&snap_path, &data).unwrap();

    let dst = make_backend("dst-corrupt");
    let result = load_snapshot(dst.as_ref(), &snap_path);
    assert!(matches!(result, Err(SnapshotError::ChunkHashMismatch { .. })));
}

#[test]
fn bad_magic_detected() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("badmagic.xrpls");

    let src = make_backend("src-magic");
    src.store(Arc::new(NodeObject::new(
        NodeObjectType::Ledger,
        vec![1],
        Uint256::from_array([0x55; 32]),
    )));

    export_snapshot(src.as_ref(), &test_manifest(), &snap_path).expect("export must succeed");

    // Corrupt magic bytes
    let mut data = std::fs::read(&snap_path).unwrap();
    data[0] = b'Z';
    std::fs::write(&snap_path, &data).unwrap();

    let dst = make_backend("dst-magic");
    let result = load_snapshot(dst.as_ref(), &snap_path);
    assert!(matches!(result, Err(SnapshotError::BadMagic { .. })));
}

#[test]
fn truncated_file_detected() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("truncated.xrpls");

    // Write a file that's too short to even contain a header
    std::fs::write(&snap_path, &[0u8; 10]).unwrap();

    let dst = make_backend("dst-trunc");
    let result = load_snapshot(dst.as_ref(), &snap_path);
    assert!(result.is_err());
}
