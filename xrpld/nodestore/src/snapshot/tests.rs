use protocol::{LedgerHeader, calculate_ledger_hash};
use sha2::{Digest, Sha256};
use shamap::nodes::item::SHAMapItem;
use shamap::nodes::tree_node::{BRANCH_FACTOR, SHAMapNodeType, SHAMapTreeNode};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::sync::Arc;

use basics::base_uint::Uint256;
use basics::basic_config::Section;
use basics::intrusive_pointer::SharedIntrusive;

use crate::database_runtime::scheduler::DummyScheduler;
use crate::snapshot::manifest::*;
use crate::snapshot::{
    SnapshotError, SnapshotExportCancellation, SnapshotManifest, export_snapshot,
    export_snapshot_with_cancellation, load_snapshot,
};
use crate::{Backend, Factory, MemoryFactory, NodeObject, NodeObjectType, NullJournal, Status};

// ─── Fixtures ────────────────────────────────────────────────────────────────

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

/// Deterministic, well-spread distinct keys.
fn key(seed: u64) -> Uint256 {
    let mut bytes = [0u8; 32];
    let mut z = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x5DEE_CE66;
    for chunk in bytes.chunks_mut(8) {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut x = z;
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        chunk.copy_from_slice(&(x ^ (x >> 31)).to_be_bytes());
    }
    Uint256::from_array(bytes)
}

fn leaves(seed: u64, n: u64) -> Vec<(Uint256, Vec<u8>)> {
    (0..n)
        .map(|i| (key(seed * 1_000_003 + i), vec![(i % 251) as u8; 16 + (i % 9) as usize]))
        .collect()
}

fn rehash(node: &SharedIntrusive<SHAMapTreeNode>) {
    if !node.is_inner() {
        return;
    }
    for branch in 0..BRANCH_FACTOR {
        if let Some(child) = node.get_child(branch) {
            rehash(&child);
        }
    }
    node.update_hash_deep();
}

fn collect(node: &SharedIntrusive<SHAMapTreeNode>, out: &mut BTreeMap<Uint256, Vec<u8>>) {
    out.insert(
        *node.get_hash().as_uint256(),
        node.serialize_with_prefix().expect("serialize"),
    );
    if node.is_inner() {
        for branch in 0..BRANCH_FACTOR {
            if let Some(child) = node.get_child(branch) {
                collect(&child, out);
            }
        }
    }
}

/// Build a SHAMap with quaxar's normal insert path (independent of the
/// snapshot builder), store every node in `backend`, and return the root and
/// all stored nodes.
fn store_tree(
    backend: &dyn Backend,
    items: &[(Uint256, Vec<u8>)],
    node_type: SHAMapNodeType,
    object_type: NodeObjectType,
) -> ([u8; 32], BTreeMap<Uint256, Vec<u8>>) {
    let mut nodes = BTreeMap::new();
    if items.is_empty() {
        return ([0; 32], nodes);
    }
    let root = SHAMapTreeNode::new_inner(1);
    for (k, v) in items {
        shamap::operations::mutation::add_item(&root, node_type, SHAMapItem::new(*k, v.clone()))
            .expect("insert");
    }
    rehash(&root);
    collect(&root, &mut nodes);
    for (hash, data) in &nodes {
        backend
            .store(Arc::new(NodeObject::new(object_type, data.clone(), *hash)))
            .expect("store");
    }
    (*root.get_hash().as_uint256().data(), nodes)
}

/// A source store holding a state map and a tx map, plus a manifest for them.
struct Ledger {
    manifest: SnapshotManifest,
    nodes: BTreeMap<Uint256, Vec<u8>>,
    state: Vec<(Uint256, Vec<u8>)>,
    txs: Vec<(Uint256, Vec<u8>)>,
}

fn ledger(backend: &dyn Backend, n_state: u64, n_tx: u64) -> Ledger {
    let state = leaves(1, n_state);
    let txs = leaves(2, n_tx);
    let (account_hash, mut nodes) = store_tree(
        backend,
        &state,
        SHAMapNodeType::AccountState,
        NodeObjectType::AccountNode,
    );
    let (tx_hash, tx_nodes) = store_tree(
        backend,
        &txs,
        SHAMapNodeType::TransactionMd,
        NodeObjectType::TransactionNode,
    );
    nodes.extend(tx_nodes);
    let mut manifest = test_manifest();
    manifest.account_hash = account_hash;
    manifest.tx_hash = tx_hash;
    refresh_manifest_ledger_hash(&mut manifest);
    Ledger {
        manifest,
        nodes,
        state,
        txs,
    }
}

/// Write a v2 container around arbitrary leaf records (for adversarial input).
fn write_raw_v2(path: &std::path::Path, manifest: &SnapshotManifest, records: &[(u8, Uint256, Vec<u8>)]) {
    let mut payload = Vec::new();
    for (tag, k, data) in records {
        encode_node_record(*tag, k.data(), data, &mut payload);
    }
    let mut m = manifest.clone();
    let mut file = Vec::new();
    let mut chunk = Vec::new();
    if !payload.is_empty() {
        chunk = lz4_flex::block::compress_prepend_size(&payload);
        m.chunks = vec![ChunkMeta {
            compressed_len: chunk.len() as u32,
            sha256: Sha256::digest(&chunk).into(),
        }];
    }
    file.extend_from_slice(&m.serialize_header());
    for meta in &m.chunks {
        file.extend_from_slice(&SnapshotManifest::serialize_chunk_meta(meta));
    }
    file.extend_from_slice(&chunk);
    let footer: [u8; 32] = Sha256::digest(&file).into();
    file.extend_from_slice(&footer);
    std::fs::write(path, file).expect("write snapshot");
}

fn sorted_records(l: &Ledger) -> Vec<(u8, Uint256, Vec<u8>)> {
    let mut state = l.state.clone();
    state.sort_by(|a, b| a.0.cmp(&b.0));
    let mut txs = l.txs.clone();
    txs.sort_by(|a, b| a.0.cmp(&b.0));
    state
        .into_iter()
        .map(|(k, v)| (LEAF_TAG_ACCOUNT_STATE, k, v))
        .chain(txs.into_iter().map(|(k, v)| (LEAF_TAG_TRANSACTION, k, v)))
        .collect()
}

fn assert_shamap_failure(result: Result<crate::SnapshotLoadOutcome, SnapshotError>, map: &str) {
    match result {
        Err(SnapshotError::ShamapVerificationFailed { map: got, .. }) => assert_eq!(got, map),
        Err(other) => panic!("expected {map} verification failure, got {other:?}"),
        Ok(_) => panic!("expected {map} verification failure, load succeeded"),
    }
}

// ─── Round trip and contents ─────────────────────────────────────────────────

#[test]
fn round_trip_rebuilds_every_node_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("rt.xrpls");
    let src = make_backend("src-rt");
    let l = ledger(src.as_ref(), 700, 40);
    // Unrelated objects in the store (other ledgers, headers) are not exported.
    src.store(Arc::new(NodeObject::new(
        NodeObjectType::Ledger,
        vec![9; 40],
        Uint256::from_array([0x11; 32]),
    )))
    .unwrap();

    export_snapshot(src.as_ref(), &l.manifest, &snap_path).expect("export");
    let dst = make_backend("dst-rt");
    let outcome = load_snapshot(dst.as_ref(), &snap_path).expect("load");

    assert_eq!(outcome.manifest.account_hash, l.manifest.account_hash);
    assert_eq!(outcome.manifest.tx_hash, l.manifest.tx_hash);
    assert!(outcome.store_was_empty);
    assert_eq!(outcome.nodes_written, l.nodes.len() as u64, "exactly the two trees");
    for (hash, data) in &l.nodes {
        let (object, status) = dst.fetch(hash);
        assert_eq!(status, Status::Ok, "node {hash} must be written");
        assert_eq!(object.unwrap().data(), data, "node {hash} bytes must match");
    }
    assert_eq!(
        dst.fetch(&Uint256::from_array([0x11; 32])).1,
        Status::NotFound,
        "objects outside the two trees are not part of the snapshot"
    );
}

#[test]
fn snapshot_contains_only_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("leaves.xrpls");
    let src = make_backend("src-leaves");
    let l = ledger(src.as_ref(), 300, 20);
    export_snapshot(src.as_ref(), &l.manifest, &snap_path).expect("export");

    let bytes = std::fs::read(&snap_path).unwrap();
    let manifest = SnapshotManifest::deserialize_header(&bytes[..SNAPSHOT_HEADER_SIZE]).unwrap();
    assert_eq!(manifest.version, 2);
    let mut offset = SNAPSHOT_HEADER_SIZE;
    let count = u32::from_be_bytes(
        bytes[SNAPSHOT_HEADER_SIZE - 10..SNAPSHOT_HEADER_SIZE - 6]
            .try_into()
            .unwrap(),
    ) as usize;
    let metas: Vec<ChunkMeta> = (0..count)
        .map(|i| {
            SnapshotManifest::deserialize_chunk_meta(
                &bytes[offset + i * CHUNK_META_SIZE..offset + (i + 1) * CHUNK_META_SIZE],
            )
        })
        .collect();
    offset += count * CHUNK_META_SIZE;
    let mut records = Vec::new();
    for meta in metas {
        let chunk = &bytes[offset..offset + meta.compressed_len as usize];
        offset += meta.compressed_len as usize;
        let raw = lz4_flex::block::decompress_size_prepended(chunk).unwrap();
        let mut pos = 0;
        while pos < raw.len() {
            let (tag, k, range, used) = decode_node_record(&raw, pos, 0).unwrap();
            records.push((tag, Uint256::from_array(k), raw[range].to_vec()));
            pos += used;
        }
    }
    assert_eq!(records, sorted_records(&l), "sorted leaves, state then tx, nothing else");
}

#[test]
fn single_leaf_and_empty_transaction_map_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("one.xrpls");
    let src = make_backend("src-one");
    let l = ledger(src.as_ref(), 1, 0);
    assert_eq!(l.manifest.tx_hash, [0; 32]);
    export_snapshot(src.as_ref(), &l.manifest, &snap_path).expect("export");
    let dst = make_backend("dst-one");
    let outcome = load_snapshot(dst.as_ref(), &snap_path).expect("load");
    assert_eq!(outcome.nodes_written, l.nodes.len() as u64);
}

#[test]
fn non_empty_store_load_collects_exact_retained_set() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("retained.xrpls");
    let src = make_backend("src-retained");
    let l = ledger(src.as_ref(), 400, 12);
    export_snapshot(src.as_ref(), &l.manifest, &snap_path).expect("export");

    let dst = make_backend("dst-retained");
    let leftover = Uint256::from_array([0x5A; 32]);
    dst.store(Arc::new(NodeObject::new(NodeObjectType::AccountNode, vec![1; 40], leftover)))
        .unwrap();
    let outcome = load_snapshot(dst.as_ref(), &snap_path).expect("load");
    assert!(!outcome.store_was_empty);
    let want: BTreeSet<Uint256> = l.nodes.keys().copied().collect();
    let got: BTreeSet<Uint256> = outcome.required_nodes.iter().copied().collect();
    assert_eq!(got, want, "retained set is exactly the two trees");
    assert!(!outcome.required_nodes.contains(&leftover));
}

// ─── Verification failures (crafted input) ───────────────────────────────────

#[test]
fn tampered_leaf_data_fails_root_check() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tampered.xrpls");
    let src = make_backend("src-tamper");
    let l = ledger(src.as_ref(), 50, 5);
    let mut records = sorted_records(&l);
    records[17].2[0] ^= 0x01;
    write_raw_v2(&path, &l.manifest, &records);
    assert_shamap_failure(load_snapshot(make_backend("dst-tamper").as_ref(), &path), "account-state");
}

#[test]
fn dropped_leaf_fails_root_check() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dropped.xrpls");
    let src = make_backend("src-drop");
    let l = ledger(src.as_ref(), 50, 5);
    let mut records = sorted_records(&l);
    records.remove(51); // a transaction leaf
    write_raw_v2(&path, &l.manifest, &records);
    assert_shamap_failure(load_snapshot(make_backend("dst-drop").as_ref(), &path), "transaction");
}

#[test]
fn unsorted_leaves_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unsorted.xrpls");
    let src = make_backend("src-unsorted");
    let l = ledger(src.as_ref(), 20, 0);
    let mut records = sorted_records(&l);
    records.swap(3, 4);
    write_raw_v2(&path, &l.manifest, &records);
    assert!(matches!(
        load_snapshot(make_backend("dst-unsorted").as_ref(), &path),
        Err(SnapshotError::MalformedNodeRecord { ref reason, .. }) if reason.contains("ascending")
    ));
}

#[test]
fn state_leaf_after_transaction_leaves_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("order.xrpls");
    let src = make_backend("src-order");
    let l = ledger(src.as_ref(), 10, 3);
    let mut records = sorted_records(&l);
    let first_state = records.remove(0);
    records.push(first_state);
    write_raw_v2(&path, &l.manifest, &records);
    assert!(matches!(
        load_snapshot(make_backend("dst-order").as_ref(), &path),
        Err(SnapshotError::MalformedNodeRecord { ref reason, .. })
            if reason.contains("after transaction")
    ));
}

#[test]
fn unknown_record_tag_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tag.xrpls");
    write_raw_v2(&path, &test_manifest(), &[(1, key(1), vec![0; 16])]);
    assert!(matches!(
        load_snapshot(make_backend("dst-tag").as_ref(), &path),
        Err(SnapshotError::MalformedNodeRecord { ref reason, .. }) if reason.contains("tag")
    ));
}

#[test]
fn leaves_for_an_empty_header_root_are_rejected() {
    // Header says the state map is empty, but the file carries a leaf.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extra.xrpls");
    write_raw_v2(
        &path,
        &test_manifest(),
        &[(LEAF_TAG_ACCOUNT_STATE, key(9), vec![1; 16])],
    );
    assert_shamap_failure(load_snapshot(make_backend("dst-extra").as_ref(), &path), "account-state");
}

#[test]
fn v1_snapshot_is_rejected_by_version() {
    let mut header = test_manifest().serialize_header();
    header[8..10].copy_from_slice(&1_u16.to_be_bytes());
    assert!(matches!(
        SnapshotManifest::deserialize_header(&header),
        Err(SnapshotError::UnsupportedVersion { found: 1, .. })
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
    assert!(matches!(
        load_snapshot(make_backend("dst-bad-ledger-hash").as_ref(), &snap_path),
        Err(SnapshotError::LedgerHashMismatch { .. })
    ));
}

// ─── Export failure handling ─────────────────────────────────────────────────

#[test]
fn export_fails_atomically_when_a_tree_node_is_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output_path = dir.path().join("failed.xrpls");
    let tmp_chunks_path = output_path.with_extension("xrpls.chunks.tmp");
    let tmp_final_path = output_path.with_extension("xrpls.tmp");
    std::fs::write(&tmp_final_path, b"stale temporary output").expect("stale temp file");

    // The root is known but the store does not hold it.
    let mut manifest = test_manifest();
    manifest.account_hash = [0xAB; 32];
    refresh_manifest_ledger_hash(&mut manifest);
    let empty = make_backend("src-missing");
    let error = export_snapshot(empty.as_ref(), &manifest, &output_path)
        .expect_err("missing tree node must fail export");

    assert!(matches!(error, SnapshotError::BackendTraversalFailed { .. }));
    assert!(!output_path.exists(), "failed export must not create output");
    assert!(!tmp_chunks_path.exists(), "chunk temporary file must be cleaned");
    assert!(!tmp_final_path.exists(), "snapshot temporary file must be cleaned");
}

/// Delegates to a memory backend and cancels the export on the first fetch.
struct CancelOnFetch {
    inner: Box<dyn Backend>,
    cancellation: SnapshotExportCancellation,
}

impl Backend for CancelOnFetch {
    fn get_name(&self) -> String {
        "cancel-on-fetch".to_owned()
    }
    fn open(&self, _create_if_missing: bool) -> Result<(), String> {
        Ok(())
    }
    fn is_open(&self) -> bool {
        true
    }
    fn close(&self) -> Result<(), String> {
        Ok(())
    }
    fn fetch(&self, hash: &Uint256) -> (Option<Arc<NodeObject>>, Status) {
        self.cancellation.cancel();
        self.inner.fetch(hash)
    }
    fn fetch_batch(&self, hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status) {
        self.inner.fetch_batch(hashes)
    }
    fn store(&self, object: Arc<NodeObject>) -> Result<(), String> {
        self.inner.store(object)
    }
    fn store_batch(&self, _batch: &crate::Batch) {}
    fn sync(&self) {}
    fn for_each(&self, _callback: &mut dyn FnMut(Arc<NodeObject>)) {}
    fn get_write_load(&self) -> i32 {
        0
    }
    fn set_delete_path(&self) {}
    fn fd_required(&self) -> i32 {
        0
    }
}

#[test]
fn export_cancellation_cleans_all_temporary_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output_path = dir.path().join("cancelled.xrpls");
    let tmp_chunks_path = output_path.with_extension("xrpls.chunks.tmp");
    let tmp_final_path = output_path.with_extension("xrpls.tmp");
    let cancellation = SnapshotExportCancellation::new();
    let inner = make_backend("src-cancel");
    let l = ledger(inner.as_ref(), 30, 2);
    let backend = CancelOnFetch {
        inner,
        cancellation: cancellation.clone(),
    };

    let error = export_snapshot_with_cancellation(&backend, &l.manifest, &output_path, &cancellation)
        .expect_err("cancellation must stop export");

    assert!(matches!(error, SnapshotError::ExportCancelled));
    assert!(!output_path.exists());
    assert!(!tmp_chunks_path.exists());
    assert!(!tmp_final_path.exists());
}

// ─── Container integrity (format-independent) ────────────────────────────────

fn exported(tag: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{tag}.xrpls"));
    let src = make_backend(&format!("src-{tag}"));
    let l = ledger(src.as_ref(), 40, 4);
    export_snapshot(src.as_ref(), &l.manifest, &path).expect("export");
    (dir, path)
}

#[test]
fn corrupt_chunk_hash_detected() {
    let (_dir, path) = exported("corrupt");
    let mut data = std::fs::read(&path).unwrap();
    let chunks = u32::from_be_bytes(
        data[SNAPSHOT_HEADER_SIZE - 10..SNAPSHOT_HEADER_SIZE - 6]
            .try_into()
            .unwrap(),
    ) as usize;
    assert!(chunks >= 1);
    data[SNAPSHOT_HEADER_SIZE + chunks * CHUNK_META_SIZE] ^= 0xFF; // first chunk byte
    std::fs::write(&path, &data).unwrap();
    assert!(matches!(
        load_snapshot(make_backend("dst-corrupt").as_ref(), &path),
        Err(SnapshotError::ChunkHashMismatch { .. })
    ));
}

#[test]
fn bad_magic_detected() {
    let (_dir, path) = exported("magic");
    let mut data = std::fs::read(&path).unwrap();
    data[0] = b'Z';
    std::fs::write(&path, &data).unwrap();
    assert!(matches!(
        load_snapshot(make_backend("dst-magic").as_ref(), &path),
        Err(SnapshotError::BadMagic { .. })
    ));
}

#[test]
fn truncated_file_detected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("truncated.xrpls");
    std::fs::write(&path, [0u8; 10]).unwrap();
    assert!(load_snapshot(make_backend("dst-trunc").as_ref(), &path).is_err());
}

#[test]
fn snapshot_loader_rejects_bytes_after_authenticated_footer() {
    let (_dir, path) = exported("trailing");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&[0xA5])
        .unwrap();
    assert!(matches!(
        load_snapshot(make_backend("dst-trailing").as_ref(), &path),
        Err(SnapshotError::TrailingData)
    ));
}

#[test]
fn snapshot_loader_rejects_excessive_chunk_count_before_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("excessive-chunks.xrpls");
    let mut header = test_manifest().serialize_header();
    let excessive = u32::try_from(SNAPSHOT_MAX_CHUNKS + 1).expect("test limit fits u32");
    header[SNAPSHOT_HEADER_SIZE - 10..SNAPSHOT_HEADER_SIZE - 6]
        .copy_from_slice(&excessive.to_be_bytes());
    std::fs::write(&snap_path, header).expect("header write");
    assert!(matches!(
        load_snapshot(make_backend("dst-excessive-chunks").as_ref(), &snap_path),
        Err(SnapshotError::ResourceLimitExceeded {
            resource: "chunk count",
            ..
        })
    ));
}

#[test]
fn snapshot_manifest_rejects_noncanonical_header_fields() {
    let manifest = test_manifest();

    let mut legacy_version = manifest.serialize_header();
    legacy_version[8..10].copy_from_slice(&0_u16.to_be_bytes());
    assert!(matches!(
        SnapshotManifest::deserialize_header(&legacy_version),
        Err(SnapshotError::UnsupportedVersion { found: 0, .. })
    ));

    let mut reserved = manifest.serialize_header();
    reserved[SNAPSHOT_HEADER_SIZE - 1] = 1;
    assert!(matches!(
        SnapshotManifest::deserialize_header(&reserved),
        Err(SnapshotError::MalformedHeader { .. })
    ));

    let mut zero_sequence = manifest.serialize_header();
    zero_sequence[10..14].copy_from_slice(&0_u32.to_be_bytes());
    assert!(matches!(
        SnapshotManifest::deserialize_header(&zero_sequence),
        Err(SnapshotError::MalformedHeader { .. })
    ));
}

#[test]
fn snapshot_record_decoder_rejects_overlong_length_varints() {
    let mut record = vec![LEAF_TAG_ACCOUNT_STATE];
    record.extend_from_slice(&[0_u8; 32]);
    record.extend_from_slice(&[0x80, 0x00]);
    assert!(matches!(
        decode_node_record(&record, 0, 0),
        Err(SnapshotError::MalformedNodeRecord { ref reason, .. })
            if reason == "data_len varint is not canonical"
    ));
}

struct RejectingBatchBackend {
    aborted: std::sync::atomic::AtomicBool,
}

impl Backend for RejectingBatchBackend {
    fn get_name(&self) -> String {
        "rejecting-batch".to_owned()
    }
    fn open(&self, _create_if_missing: bool) -> Result<(), String> {
        Ok(())
    }
    fn is_open(&self) -> bool {
        true
    }
    fn close(&self) -> Result<(), String> {
        Ok(())
    }
    fn fetch(&self, _hash: &Uint256) -> (Option<Arc<NodeObject>>, Status) {
        (None, Status::NotFound)
    }
    fn fetch_batch(&self, hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status) {
        (vec![None; hashes.len()], Status::Ok)
    }
    fn store(&self, _object: Arc<NodeObject>) -> Result<(), String> {
        Ok(())
    }
    fn store_batch(&self, _batch: &crate::Batch) {}
    fn store_batch_result(&self, _batch: &crate::Batch) -> Result<(), String> {
        Err("injected batch write failure".to_owned())
    }
    fn sync(&self) {}
    fn for_each(&self, _callback: &mut dyn FnMut(Arc<NodeObject>)) {}
    fn get_write_load(&self) -> i32 {
        0
    }
    fn set_delete_path(&self) {}
    fn fd_required(&self) -> i32 {
        0
    }
    fn bulk_import_abort(&self) {
        self.aborted
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

#[test]
fn snapshot_loader_propagates_batch_write_failure_and_aborts_import() {
    let (_dir, path) = exported("batch-failure");
    let destination = RejectingBatchBackend {
        aborted: std::sync::atomic::AtomicBool::new(false),
    };
    assert!(matches!(
        load_snapshot(&destination, &path),
        Err(SnapshotError::BackendWriteFailed { ref reason })
            if reason.contains("injected batch write failure")
    ));
    assert!(destination.aborted.load(std::sync::atomic::Ordering::Acquire));
}

// ─── Retained-set collection ─────────────────────────────────────────────────

fn fjall_store(tag: &str) -> (Arc<dyn Backend>, String) {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("quaxar-snap-{tag}-{t}"));
    let mut section = Section::new("node_db");
    section.set("type", "fjall");
    section.set("path", path.to_string_lossy().into_owned());
    let backend = crate::FjallFactory::new()
        .create_instance(
            NodeObject::KEY_BYTES,
            &section,
            0,
            Arc::new(DummyScheduler),
            Arc::new(NullJournal),
        )
        .expect("fjall backend");
    let backend: Arc<dyn Backend> = Arc::from(backend);
    backend.open(true).expect("open");
    (backend, path.to_string_lossy().into_owned())
}

#[test]
fn empty_store_load_skips_retained_set_and_adopts_anchor_only() {
    use crate::{PrunedConfig, PrunedStore};
    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("empty.xrpls");
    let src = make_backend("src-empty-store");
    let l = ledger(src.as_ref(), 120, 4);
    export_snapshot(src.as_ref(), &l.manifest, &snap_path).expect("export");

    let (dst, dst_path) = fjall_store("empty");
    let outcome = load_snapshot(dst.as_ref(), &snap_path).expect("load");
    assert!(outcome.store_was_empty);
    assert!(outcome.required_nodes.is_empty(), "nothing to reconcile");
    assert_eq!(outcome.nodes_written, l.nodes.len() as u64);
    for hash in l.nodes.keys() {
        assert_eq!(dst.fetch(hash).1, Status::Ok);
    }

    let store = PrunedStore::open(Arc::clone(&dst), PrunedConfig::default()).expect("open pruned");
    store
        .adopt_snapshot_into_empty_store(outcome.manifest.ledger_seq, Uint256::from_array(l.manifest.account_hash))
        .expect("adopt");
    assert_eq!(store.metrics().claimed_seq, Some(outcome.manifest.ledger_seq));
    // A second adoption on a claimed store is refused.
    assert!(store.adopt_snapshot_into_empty_store(1, Uint256::default()).is_err());
    drop(store);
    drop(dst);
    let _ = std::fs::remove_dir_all(&dst_path);
}

// ─── Operator migration path ─────────────────────────────────────────────────

// Export from an old fjall store, load into a fresh one, adopt the snapshot as
// the pruned anchor, and keep claiming (design Case 7 / Stage 6).
#[test]
fn migration_snapshot_load_then_adopt_and_claim() {
    use crate::backends::kv::{Keyspace, KvBatch};
    use crate::{ClaimDelta, FjallFactory, PrunedConfig, PrunedStore};

    let make_fjall = |tag: &str| -> (Arc<dyn Backend>, String) {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("quaxar-migrate-{tag}-{t}"));
        let mut section = Section::new("node_db");
        section.set("type", "fjall");
        section.set("path", path.to_string_lossy().into_owned());
        let backend = FjallFactory::new()
            .create_instance(
                NodeObject::KEY_BYTES,
                &section,
                0,
                Arc::new(DummyScheduler),
                Arc::new(NullJournal),
            )
            .expect("fjall backend");
        let backend: Arc<dyn Backend> = Arc::from(backend);
        backend.open(true).expect("open");
        (backend, path.to_string_lossy().into_owned())
    };

    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("migrate.xrpls");
    let (src, src_path) = make_fjall("src");
    let mut l = ledger(src.as_ref(), 25, 3);
    l.manifest.ledger_seq = 7000;
    refresh_manifest_ledger_hash(&mut l.manifest);
    export_snapshot(src.as_ref(), &l.manifest, &snap_path).expect("export");

    // A leftover a prior partial sync could have left behind is reclaimed.
    let (dst, dst_path) = make_fjall("dst");
    let leftover = Uint256::from_array([0xEE; 32]);
    let mut batch = KvBatch::new();
    batch.put(Keyspace::Nodes, leftover.as_slice().to_vec(), vec![0xEE; 8]);
    dst.kv_write_batch(&batch).expect("seed leftover");

    let outcome = load_snapshot(dst.as_ref(), &snap_path).expect("load");
    assert!(!outcome.store_was_empty, "the leftover makes the store non-empty");
    let account_root = Uint256::from_array(l.manifest.account_hash);
    assert!(outcome.required_nodes.contains(&account_root));

    let store = PrunedStore::open(Arc::clone(&dst), PrunedConfig::default()).expect("open pruned");
    let required: BTreeSet<Uint256> = outcome.required_nodes.iter().copied().collect();
    let swept = store
        .adopt_snapshot(&required, outcome.manifest.ledger_seq, account_root)
        .expect("adopt");
    assert_eq!(swept, 1, "the pre-sync leftover is reconciled away");
    assert_eq!(store.fetch(&account_root).1, Status::Ok);
    assert_eq!(store.fetch(&leftover).1, Status::NotFound);
    assert_eq!(store.metrics().claimed_seq, Some(7000));

    let next_leaf = Uint256::from_array([0x93; 32]);
    let mut batch = KvBatch::new();
    batch.put(Keyspace::Nodes, next_leaf.as_slice().to_vec(), vec![0x93; 8]);
    dst.kv_write_batch(&batch).expect("store next node");
    store
        .claim(&ClaimDelta {
            seq: 7001,
            state_root: next_leaf,
            new_state: vec![next_leaf],
            dead_state: vec![account_root],
            owned: vec![],
        })
        .expect("claim 7001");
    assert_eq!(store.metrics().claimed_seq, Some(7001));

    drop(store);
    drop(src);
    drop(dst);
    let _ = std::fs::remove_dir_all(&src_path);
    let _ = std::fs::remove_dir_all(&dst_path);
}
