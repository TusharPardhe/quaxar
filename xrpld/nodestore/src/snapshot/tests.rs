use basics::intrusive_pointer::make_shared_intrusive;
use protocol::{LedgerHeader, calculate_ledger_hash};
use shamap::nodes::item::SHAMapItem;
use shamap::nodes::tree_node::{SHAMapNodeType, SHAMapTreeNode};
use std::sync::Arc;

use basics::base_uint::Uint256;
use basics::basic_config::Section;

use crate::database_runtime::scheduler::DummyScheduler;
use crate::snapshot::manifest::*;
use crate::snapshot::{SnapshotError, SnapshotManifest, export_snapshot, load_snapshot};
use crate::{Backend, Factory, MemoryFactory, NodeObject, NodeObjectType, NullJournal};

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
    let mut manifest = SnapshotManifest {
        version: SNAPSHOT_VERSION,
        ledger_seq: 100,
        ledger_hash: [0; 32],
        account_hash: [0; 32],
        tx_hash: [0; 32],
        parent_hash: [0xDD; 32],
        drops: 100_000_000_000,
        close_time: 750_000_000,
        parent_close_time: 749_999_990,
        close_time_res: 10,
        close_flags: 0,
        chunks: Vec::new(),
    };
    refresh_manifest_ledger_hash(&mut manifest);
    manifest
}

fn refresh_manifest_ledger_hash(manifest: &mut SnapshotManifest) {
    let header = LedgerHeader {
        seq: manifest.ledger_seq,
        drops: manifest.drops,
        parent_hash: basics::sha_map_hash::SHAMapHash::new(Uint256::from_array(
            manifest.parent_hash,
        )),
        tx_hash: basics::sha_map_hash::SHAMapHash::new(Uint256::from_array(manifest.tx_hash)),
        account_hash: basics::sha_map_hash::SHAMapHash::new(Uint256::from_array(
            manifest.account_hash,
        )),
        parent_close_time: manifest.parent_close_time,
        close_time: manifest.close_time,
        close_time_resolution: manifest.close_time_res,
        close_flags: manifest.close_flags,
        ..LedgerHeader::default()
    };
    manifest.ledger_hash = *calculate_ledger_hash(&header).as_uint256().data();
}

fn shamap_leaf(
    object_type: NodeObjectType,
    node_type: SHAMapNodeType,
    marker: u8,
) -> (Arc<NodeObject>, [u8; 32]) {
    let node = make_shared_intrusive(SHAMapTreeNode::new_leaf(
        node_type,
        SHAMapItem::new(Uint256::from_array([marker; 32]), vec![marker; 12]),
        0,
    ));
    let hash = *node.get_hash().as_uint256();
    let hash_bytes = *hash.data();
    let data = node
        .serialize_with_prefix()
        .expect("test SHAMap leaf must serialize");
    (
        Arc::new(NodeObject::new(object_type, data, hash)),
        hash_bytes,
    )
}

fn shamap_inner_with_child(child_hash: [u8; 32]) -> (Arc<NodeObject>, [u8; 32]) {
    let node = make_shared_intrusive(SHAMapTreeNode::new_inner(0));
    node.set_child_hash(
        0,
        basics::sha_map_hash::SHAMapHash::new(Uint256::from_array(child_hash)),
    );
    node.update_hash();
    let hash = *node.get_hash().as_uint256();
    let hash_bytes = *hash.data();
    let data = node
        .serialize_with_prefix()
        .expect("test SHAMap inner node must serialize");
    (
        Arc::new(NodeObject::new(NodeObjectType::AccountNode, data, hash)),
        hash_bytes,
    )
}

#[test]
fn post_import_verifies_account_and_transaction_shamap_roots() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("roots.xrpls");
    let src = make_backend("src-roots");
    let (account_node, account_hash) = shamap_leaf(
        NodeObjectType::AccountNode,
        SHAMapNodeType::AccountState,
        0x81,
    );
    let (transaction_node, tx_hash) = shamap_leaf(
        NodeObjectType::TransactionNode,
        SHAMapNodeType::TransactionMd,
        0x82,
    );
    src.store(account_node);
    src.store(transaction_node);

    let mut manifest = test_manifest();
    manifest.account_hash = account_hash;
    manifest.tx_hash = tx_hash;
    refresh_manifest_ledger_hash(&mut manifest);
    export_snapshot(src.as_ref(), &manifest, &snap_path).expect("export must succeed");

    let dst = make_backend("dst-roots");
    let loaded = load_snapshot(dst.as_ref(), &snap_path).expect("roots must verify");
    assert_eq!(loaded.account_hash, account_hash);
    assert_eq!(loaded.tx_hash, tx_hash);
}

#[test]
fn post_import_rejects_manifest_root_missing_from_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("missing-root.xrpls");
    let src = make_backend("src-missing-root");
    let (_missing_child, child_hash) = shamap_leaf(
        NodeObjectType::AccountNode,
        SHAMapNodeType::AccountState,
        0x91,
    );
    let (root_node, root_hash) = shamap_inner_with_child(child_hash);
    src.store(root_node);

    let mut manifest = test_manifest();
    manifest.account_hash = root_hash;
    refresh_manifest_ledger_hash(&mut manifest);
    export_snapshot(src.as_ref(), &manifest, &snap_path).expect("export must succeed");

    let dst = make_backend("dst-missing-root");
    assert!(matches!(
        load_snapshot(dst.as_ref(), &snap_path),
        Err(SnapshotError::ShamapVerificationFailed {
            map: "account-state",
            ..
        })
    ));
}

#[test]
fn post_import_rejects_shamap_body_under_forged_content_key() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("forged-key.xrpls");
    let src = make_backend("src-forged-key");
    let (valid_node, _valid_hash) = shamap_leaf(
        NodeObjectType::AccountNode,
        SHAMapNodeType::AccountState,
        0xA1,
    );
    let forged_hash = Uint256::from_array([0xE1; 32]);
    src.store(Arc::new(NodeObject::new(
        NodeObjectType::AccountNode,
        valid_node.data().to_vec(),
        forged_hash,
    )));

    let mut manifest = test_manifest();
    manifest.account_hash = *forged_hash.data();
    refresh_manifest_ledger_hash(&mut manifest);
    export_snapshot(src.as_ref(), &manifest, &snap_path).expect("export must succeed");

    let dst = make_backend("dst-forged-key");
    assert!(matches!(
        load_snapshot(dst.as_ref(), &snap_path),
        Err(SnapshotError::ShamapVerificationFailed {
            map: "account-state",
            ..
        })
    ));
}

#[test]
fn post_import_rejects_transaction_leaf_as_account_state_root() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("cross-map.xrpls");
    let src = make_backend("src-cross-map");
    let (transaction_node, tx_hash) = shamap_leaf(
        NodeObjectType::TransactionNode,
        SHAMapNodeType::TransactionMd,
        0xA2,
    );
    src.store(transaction_node);

    let mut manifest = test_manifest();
    manifest.account_hash = tx_hash;
    refresh_manifest_ledger_hash(&mut manifest);
    export_snapshot(src.as_ref(), &manifest, &snap_path).expect("export must succeed");

    let dst = make_backend("dst-cross-map");
    assert!(matches!(
        load_snapshot(dst.as_ref(), &snap_path),
        Err(SnapshotError::ShamapVerificationFailed {
            map: "account-state",
            ..
        })
    ));
}

#[test]
fn post_import_rejects_inconsistent_manifest_ledger_hash() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("bad-ledger-hash.xrpls");
    let src = make_backend("src-bad-ledger-hash");
    let mut manifest = test_manifest();
    manifest.ledger_hash = [0xFF; 32];
    export_snapshot(src.as_ref(), &manifest, &snap_path).expect("export must succeed");

    let dst = make_backend("dst-bad-ledger-hash");
    assert!(matches!(
        load_snapshot(dst.as_ref(), &snap_path),
        Err(SnapshotError::LedgerHashMismatch { .. })
    ));
}

#[test]
fn snapshot_loader_rejects_excessive_chunk_count_before_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("excessive-chunks.xrpls");
    let manifest = test_manifest();
    let mut header = manifest.serialize_header();
    let excessive = u32::try_from(SNAPSHOT_MAX_CHUNKS + 1).expect("test limit fits u32");
    header[SNAPSHOT_HEADER_SIZE - 10..SNAPSHOT_HEADER_SIZE - 6]
        .copy_from_slice(&excessive.to_be_bytes());
    std::fs::write(&snap_path, header).expect("header write");

    let dst = make_backend("dst-excessive-chunks");
    assert!(matches!(
        load_snapshot(dst.as_ref(), &snap_path),
        Err(SnapshotError::ResourceLimitExceeded {
            resource: "chunk count",
            ..
        })
    ));
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
    assert_eq!(loaded_manifest.ledger_hash, test_manifest().ledger_hash);
    assert_eq!(loaded_manifest.account_hash, [0; 32]);

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
    assert!(matches!(
        result,
        Err(SnapshotError::ChunkHashMismatch { .. })
    ));
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
