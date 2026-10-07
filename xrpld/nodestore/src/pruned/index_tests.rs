//! Tests for the pruned index core: deterministic case coverage plus a
//! randomized equivalence check against [`ModelStore`](super::model::ModelStore).
//!
//! The harness drives the real [`IndexWriter`](super::index::IndexWriter) over
//! a MemoryBackend from a sequence of full ledger snapshots: it diffs each
//! snapshot's state set against the previous to form a [`ClaimDelta`], stores
//! the node bytes, claims, then prunes to the window floor. After every step
//! it asserts the invariant that matters: no node the model still requires is
//! missing from the backend.

use crate::backends::kv::Keyspace;
use crate::pruned::index::{ClaimDelta, IndexWriter};
use crate::pruned::model::{LedgerSnapshot, ModelStore};
use crate::{Backend, Factory, MemoryFactory, NodeObject, NullJournal};
use basics::base_uint::Uint256;
use basics::basic_config::Section;
use std::collections::BTreeSet;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

fn h(bytes: [u8; 32]) -> Uint256 {
    Uint256::from_array(bytes)
}

fn hid(n: u64) -> Uint256 {
    let mut b = [0u8; 32];
    b[24..].copy_from_slice(&n.to_be_bytes());
    h(b)
}

fn open_memory() -> Arc<dyn Backend> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut section = Section::new("node_db");
    section.set("type", "Memory");
    section.set("path", format!("index-test/{n}"));
    let backend = MemoryFactory::new()
        .create_instance(
            NodeObject::KEY_BYTES,
            &section,
            0,
            Arc::new(crate::DummyScheduler),
            Arc::new(NullJournal),
        )
        .expect("memory backend");
    let backend: Arc<dyn Backend> = Arc::from(backend);
    backend.open(true).expect("open");
    backend
}

fn open_fjall() -> Arc<dyn Backend> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut dir = std::env::temp_dir();
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    dir.push(format!("quaxar-index-fjall-{t}-{n}"));
    let mut section = Section::new("node_db");
    section.set("type", "fjall");
    section.set("path", dir.to_string_lossy().into_owned());
    let backend = crate::FjallFactory::new()
        .create_instance(
            NodeObject::KEY_BYTES,
            &section,
            0,
            Arc::new(crate::DummyScheduler),
            Arc::new(NullJournal),
        )
        .expect("fjall backend");
    let backend: Arc<dyn Backend> = Arc::from(backend);
    backend.open(true).expect("open");
    backend
}

/// Store a node's bytes into the backend's nodes keyspace (what the pruned
/// store's flush path does: nodes live in the `nodes` keyspace, which is where
/// prune deletes from).
fn store_node(backend: &dyn Backend, hash: &Uint256) {
    let mut batch = crate::backends::kv::KvBatch::new();
    batch.put(Keyspace::Nodes, hash.as_slice().to_vec(), vec![1, 2, 3]);
    backend.kv_write_batch(&batch).expect("store node");
}

fn delta_between(
    prev: Option<&LedgerSnapshot>,
    next: &LedgerSnapshot,
    state_root: Uint256,
) -> ClaimDelta {
    let empty = BTreeSet::new();
    let prev_state = prev.map(|s| &s.state).unwrap_or(&empty);
    ClaimDelta {
        seq: next.seq,
        state_root,
        new_state: next.state.difference(prev_state).copied().collect(),
        dead_state: prev_state.difference(&next.state).copied().collect(),
        owned: next.owned.iter().copied().collect(),
    }
}

/// Node hashes physically present in the backend's nodes keyspace.
fn present_nodes(backend: &dyn Backend) -> BTreeSet<Uint256> {
    let mut set = BTreeSet::new();
    backend
        .kv_range(Keyspace::Nodes, &[], &[0xFF; 64], &mut |key, _| {
            if let Some(hash) = Uint256::from_slice(key) {
                set.insert(hash);
            }
            true
        })
        .expect("range nodes");
    set
}

/// Drive a snapshot sequence through writer + model, pruning each step, and
/// assert the writer never drops a node the model still requires.
fn run_sequence(snapshots: &[LedgerSnapshot], online_delete: u32) {
    run_sequence_on(&open_memory(), snapshots, online_delete);
}

fn run_sequence_on(backend: &Arc<dyn Backend>, snapshots: &[LedgerSnapshot], online_delete: u32) {
    let mut writer = IndexWriter::open(Arc::clone(backend)).expect("open writer");
    let mut model = ModelStore::new();
    let mut prev: Option<LedgerSnapshot> = None;

    for snap in snapshots {
        // Flush this ledger's nodes to the backend before claiming (store
        // path). The state root is a deterministic function of seq here.
        for hash in snap.state.iter().chain(snap.owned.iter()) {
            store_node(backend.as_ref(), hash);
            writer.note_stored(*hash, snap.seq);
        }
        let delta = delta_between(prev.as_ref(), snap, hid(0xF00D + snap.seq as u64));
        writer.claim(&delta).expect("claim");
        model.claim(snap.clone());

        let floor = model
            .retained_floor(online_delete, u32::MAX)
            .expect("floor");
        // K is one below the retained floor: the oldest ledger still dropped.
        let k = floor.saturating_sub(1);
        writer.prune(k, 4).expect("prune");

        // Invariant: every required node is physically present.
        let required = model.required_nodes(online_delete, u32::MAX);
        let present = present_nodes(backend.as_ref());
        for node in &required {
            assert!(
                present.contains(node),
                "required node missing after seq {} (floor {floor}, k {k}): {node:?}",
                snap.seq
            );
        }
        prev = Some(snap.clone());
    }
}

#[test]
fn index_core_is_engine_agnostic_on_fjall() {
    // The same claim/prune logic must hold on the fjall backend, proving the
    // index core is engine-agnostic over the key-value trait.
    let backend = open_fjall();
    let carol = hid(0xC0);
    let snaps: Vec<LedgerSnapshot> = (1..=8)
        .map(|seq| {
            LedgerSnapshot::new(seq)
                .with_state([hid(seq as u64), hid(0xA000 + seq as u64), carol])
                .with_owned([hid(0x7000 + seq as u64)])
        })
        .collect();
    run_sequence_on(&backend, &snaps, 3);
    let _ = backend.close();
}

#[test]
fn alice_carol_window_keeps_shared_node() {
    let carol = hid(0xC0);
    let snaps: Vec<LedgerSnapshot> = (1..=8)
        .map(|seq| {
            LedgerSnapshot::new(seq)
                .with_state([hid(seq as u64), hid(0xA000 + seq as u64), carol])
                .with_owned([hid(0x7000 + seq as u64)])
        })
        .collect();
    run_sequence(&snaps, 3);
}

#[test]
fn resurrection_keeps_node_live() {
    // A node A100 dies then returns; it must never be pruned while live.
    let a100 = hid(100);
    let other = hid(0xBEEF);
    let snaps = vec![
        LedgerSnapshot::new(1).with_state([hid(1), a100]),
        LedgerSnapshot::new(2).with_state([hid(2), other]), // a100 dies @2
        LedgerSnapshot::new(3).with_state([hid(3), a100]),  // a100 reborn
        LedgerSnapshot::new(4).with_state([hid(4), a100]),
        LedgerSnapshot::new(5).with_state([hid(5), a100]),
    ];
    run_sequence(&snaps, 2);
}

#[test]
fn gap_claim_diffs_against_last_state() {
    // Jump from seq 100 to 150; the writer diffs across the gap.
    let shared = hid(0x5EED);
    let snaps = vec![
        LedgerSnapshot::new(100).with_state([hid(1), shared]),
        LedgerSnapshot::new(150).with_state([hid(2), shared]),
        LedgerSnapshot::new(151).with_state([hid(3), shared]),
    ];
    run_sequence(&snaps, 5);
}

#[test]
fn owned_tx_nodes_prune_with_their_ledger() {
    let backend = open_memory();
    let mut writer = IndexWriter::open(Arc::clone(&backend)).expect("writer");
    let tx = hid(0x7A1);
    // Ledger 1 owns tx node; ledger 2 arrives, so tx dies @2.
    store_node(backend.as_ref(), &tx);
    writer.note_stored(tx, 1);
    writer
        .claim(&ClaimDelta {
            seq: 1,
            state_root: hid(1),
            new_state: vec![hid(0xA1)],
            dead_state: vec![],
            owned: vec![tx],
        })
        .expect("claim 1");
    store_node(backend.as_ref(), &hid(0xA2));
    writer
        .claim(&ClaimDelta {
            seq: 2,
            state_root: hid(2),
            new_state: vec![hid(0xA2)],
            dead_state: vec![],
            owned: vec![],
        })
        .expect("claim 2");
    // Keep window 1 (only seq 2 retained): prune up to k=1 removes the owned
    // tx node, which died @2 (<= ... actually owned dies at seq+1=2).
    writer.prune(1, 16).expect("prune");
    let present = present_nodes(backend.as_ref());
    // tx owned node died @2; k=1 does not reach it yet, so it is still here.
    assert!(present.contains(&tx), "owned node not yet past the window");
    // Advance: claim seq 3, then prune to k=2, which reaches the owned record.
    store_node(backend.as_ref(), &hid(0xA3));
    writer
        .claim(&ClaimDelta {
            seq: 3,
            state_root: hid(3),
            new_state: vec![hid(0xA3)],
            dead_state: vec![],
            owned: vec![],
        })
        .expect("claim 3");
    writer.prune(2, 16).expect("prune 2");
    let present = present_nodes(backend.as_ref());
    assert!(
        !present.contains(&tx),
        "owned node pruned once past the window"
    );
}

#[test]
fn restart_resumes_from_meta_cursors() {
    let backend = open_memory();
    {
        let mut writer = IndexWriter::open(Arc::clone(&backend)).expect("writer");
        store_node(backend.as_ref(), &hid(1));
        writer.note_stored(hid(1), 1);
        writer
            .claim(&ClaimDelta {
                seq: 10,
                state_root: hid(10),
                new_state: vec![hid(1)],
                dead_state: vec![],
                owned: vec![],
            })
            .expect("claim");
    }
    // Reopen on the same backend: the writer must recover claimed_seq.
    let writer = IndexWriter::open(Arc::clone(&backend)).expect("reopen writer");
    assert_eq!(writer.claimed_seq(), Some(10));
}

/// Randomized equivalence: random edits, reversions and a window, checked
/// against the model after every step. Deterministic LCG so failures are
/// reproducible without a proptest dependency.
#[test]
fn randomized_matches_model_oracle() {
    let mut lcg: u64 = 0x1234_5678_9abc_def0;
    let mut next = || {
        lcg = lcg
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        lcg >> 33
    };

    for trial in 0..300 {
        let window = 2 + (next() % 5) as u32;
        // A small universe of state nodes so reversions and sharing happen.
        let universe: Vec<Uint256> = (0..12).map(|i| hid(1000 + i)).collect();
        let mut snaps = Vec::new();
        let mut live: BTreeSet<Uint256> = BTreeSet::new();
        let ledger_count = 6 + (next() % 12) as u32;
        for seq in 1..=ledger_count {
            // Randomly add/remove a few nodes from the live set.
            let churn = 1 + (next() % 4);
            for _ in 0..churn {
                let node = universe[(next() as usize) % universe.len()];
                if live.contains(&node) && next() % 2 == 0 {
                    live.remove(&node);
                } else {
                    live.insert(node);
                }
            }
            // Always include a per-seq unique node so the root changes.
            let unique = hid(50_000 + trial * 1000 + seq as u64);
            let mut state = live.clone();
            state.insert(unique);
            snaps.push(
                LedgerSnapshot::new(seq)
                    .with_state(state)
                    .with_owned([hid(90_000 + trial * 1000 + seq as u64)]),
            );
        }
        run_sequence(&snaps, window);
    }
}
