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

#[test]
fn crash_during_claim_leaves_no_partial_state() {
    use crate::FaultBackend;
    let inner = open_memory();
    // Pre-store the node so a replayed claim can rewrite nothing new.
    store_node(inner.as_ref(), &hid(1));
    // Open once cleanly so the schema is stamped on the inner backend; the
    // fault we arm next should only trip the claim batch, not open().
    IndexWriter::open(Arc::clone(&inner)).expect("initial open stamps schema");
    let fault = Arc::new(FaultBackend::new(Arc::clone(&inner)));
    fault.fail_on_write(1); // trip the next write batch (the claim)
    let fault_dyn: Arc<dyn Backend> = fault.clone();
    {
        let mut writer = IndexWriter::open(Arc::clone(&fault_dyn)).expect("writer");
        let err = writer.claim(&ClaimDelta {
            seq: 7,
            state_root: hid(7),
            new_state: vec![hid(1)],
            dead_state: vec![],
            owned: vec![],
        });
        assert!(err.is_err(), "the injected fault must fail the claim");
    }
    // The inner backend never saw the batch: a fresh writer sees no claim.
    fault.clear();
    let writer = IndexWriter::open(Arc::clone(&inner)).expect("reopen");
    assert_eq!(writer.claimed_seq(), None, "failed claim did not persist");
    // Retrying the claim now succeeds (R3: idempotent re-claim).
    let mut writer = writer;
    writer
        .claim(&ClaimDelta {
            seq: 7,
            state_root: hid(7),
            new_state: vec![hid(1)],
            dead_state: vec![],
            owned: vec![],
        })
        .expect("re-claim succeeds");
    assert_eq!(writer.claimed_seq(), Some(7));
}

#[test]
fn crash_during_prune_resumes_from_pruned_to() {
    use crate::FaultBackend;
    let inner = open_memory();
    let mut setup = IndexWriter::open(Arc::clone(&inner)).expect("writer");
    // Build several dead nodes across sequences so prune has multiple chunks.
    for seq in 1..=6u32 {
        let node = hid(1000 + seq as u64);
        store_node(inner.as_ref(), &node);
        setup.note_stored(node, seq);
        let prev = if seq == 1 {
            vec![]
        } else {
            vec![hid(1000 + seq as u64 - 1)]
        };
        setup
            .claim(&ClaimDelta {
                seq,
                state_root: hid(seq as u64),
                new_state: vec![node],
                dead_state: prev,
                owned: vec![],
            })
            .expect("claim");
    }
    // Now wrap in a fault backend and fail partway through a chunked prune.
    let fault = Arc::new(FaultBackend::new(Arc::clone(&inner)));
    fault.fail_on_write(2); // fail the second prune batch
    let fault_dyn: Arc<dyn Backend> = fault.clone();
    let mut writer = IndexWriter::open(Arc::clone(&fault_dyn)).expect("reopen on fault");
    let before = writer.pruned_to();
    let _ = writer.prune(5, 1); // chunk=1 forces several batches; one fails
    // A fresh writer on the inner backend resumes from the persisted cursor
    // and completes the prune; the invariant is that pruning is idempotent and
    // never loses a required node.
    fault.clear();
    let mut resumed = IndexWriter::open(Arc::clone(&inner)).expect("resume");
    assert!(
        resumed.pruned_to() >= before,
        "pruned_to never goes backwards"
    );
    resumed.prune(5, 16).expect("prune completes on resume");
    assert_eq!(resumed.pruned_to(), 5, "prune reaches k after resume");
}

#[test]
fn concurrent_readers_never_see_a_required_node_vanish() {
    use crate::backends::kv::Keyspace;
    use std::sync::atomic::{AtomicBool, Ordering as AOrd};
    use std::thread;
    // A node that is live for the whole run (shared by every ledger) must be
    // readable at every instant, even while prune deletes dead nodes.
    let backend = open_memory();
    let mut writer = IndexWriter::open(Arc::clone(&backend)).expect("writer");
    let pinned = hid(0xA11CE);
    // Seed a window of ledgers: each adds a unique node and retires the
    // previous unique node; `pinned` stays live throughout.
    store_node(backend.as_ref(), &pinned);
    for seq in 1..=40u32 {
        let unique = hid(10_000 + seq as u64);
        store_node(backend.as_ref(), &unique);
        writer.note_stored(unique, seq);
        let dead = if seq == 1 {
            vec![]
        } else {
            vec![hid(10_000 + seq as u64 - 1)]
        };
        let new_state = if seq == 1 {
            vec![unique, pinned]
        } else {
            vec![unique]
        };
        writer
            .claim(&ClaimDelta {
                seq,
                state_root: hid(seq as u64),
                new_state,
                dead_state: dead,
                owned: vec![],
            })
            .expect("claim");
    }

    let stop = Arc::new(AtomicBool::new(false));
    let reader_backend = Arc::clone(&backend);
    let reader_stop = Arc::clone(&stop);
    let reader = thread::spawn(move || {
        // Continuously read the pinned node; it must always be present.
        while !reader_stop.load(AOrd::Relaxed) {
            let got = reader_backend
                .kv_get(Keyspace::Nodes, pinned.as_slice())
                .expect("read");
            assert!(got.is_some(), "pinned live node vanished during prune");
        }
    });

    // Prune hard while the reader runs. The pinned node is referenced by the
    // latest claim (count >= 1), so prune must never delete it.
    for k in 1..=38u32 {
        writer.prune(k, 2).expect("prune");
    }
    stop.store(true, AOrd::Relaxed);
    reader.join().expect("reader thread");

    // Final check: the pinned node survived every prune.
    assert!(
        backend
            .kv_get(Keyspace::Nodes, pinned.as_slice())
            .expect("read")
            .is_some(),
        "pinned node present after all prunes"
    );
}

#[test]
fn reconcile_reclaims_orphans_the_index_missed() {
    use crate::{reconcile, verify_present};
    use std::collections::BTreeSet;
    let backend = open_memory();
    let mut writer = IndexWriter::open(Arc::clone(&backend)).expect("writer");
    // Claim one ledger with a live node.
    let live = hid(1);
    store_node(backend.as_ref(), &live);
    writer.note_stored(live, 1);
    writer
        .claim(&ClaimDelta {
            seq: 1,
            state_root: hid(1),
            new_state: vec![live],
            dead_state: vec![],
            owned: vec![],
        })
        .expect("claim");
    // Simulate a leaked orphan: a node stored but never claimed and never
    // swept (e.g. from an abandoned close the writer forgot to track).
    store_node(backend.as_ref(), &hid(999));

    let required: BTreeSet<Uint256> = [live].into_iter().collect();
    let swept = reconcile(backend.as_ref(), &required, 16).expect("reconcile");
    assert_eq!(swept, 1, "the orphan is reclaimed");
    let report = verify_present(backend.as_ref(), &required, 1).expect("verify");
    assert!(report.is_ok(), "the live node is still present");
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

/// Count rows currently present in a keyspace.
fn keyspace_rows(backend: &dyn Backend, keyspace: Keyspace) -> usize {
    let mut n = 0usize;
    backend
        .kv_range(keyspace, &[], &[0xFF; 64], &mut |_, _| {
            n += 1;
            true
        })
        .expect("range");
    n
}

// T-ANCH-1: the anchor claim (first validated ledger, empty previous state)
// writes no explicit count rows — every node is implicitly live-once (count 1,
// represented by the absence of a row) — and records the anchor sequence.
#[test]
fn t_anch_1_first_claim_writes_no_count_rows_and_sets_anchor() {
    let backend = open_memory();
    let mut writer = IndexWriter::open(Arc::clone(&backend)).expect("open");
    // Anchor ledger A=100: whole tree is new, nothing dies.
    writer
        .claim(&ClaimDelta {
            seq: 100,
            state_root: hid(1),
            new_state: vec![hid(10), hid(11), hid(12)],
            dead_state: vec![],
            owned: vec![hid(20)],
        })
        .expect("anchor claim");

    assert_eq!(writer.claimed_seq(), Some(100));
    // No count rows: all new state nodes are implicit count-1.
    assert_eq!(
        keyspace_rows(backend.as_ref(), Keyspace::Counts),
        0,
        "anchor claim must write zero explicit count rows"
    );
    // Reopening resumes from the persisted anchor/claimed cursor.
    let reopened = IndexWriter::open(Arc::clone(&backend)).expect("reopen");
    assert_eq!(reopened.claimed_seq(), Some(100));
}

// T-GATE-1 (core): prune is not gated on any operating mode — it advances
// whenever called, deleting dead nodes up to K. (The app-level test that prune
// keeps running while the node is `syncing` builds on this unconditional core.)
#[test]
fn t_gate_1_prune_advances_unconditionally() {
    let backend = open_memory();
    let mut writer = IndexWriter::open(Arc::clone(&backend)).expect("open");
    // Ledger 1 stores node A; ledger 2 retires it (A dies at seq 2).
    store_node(backend.as_ref(), &hid(10));
    writer
        .claim(&ClaimDelta {
            seq: 1,
            state_root: hid(1),
            new_state: vec![hid(10)],
            dead_state: vec![],
            owned: vec![],
        })
        .expect("claim 1");
    writer
        .claim(&ClaimDelta {
            seq: 2,
            state_root: hid(2),
            new_state: vec![hid(11)],
            dead_state: vec![hid(10)],
            owned: vec![],
        })
        .expect("claim 2");
    store_node(backend.as_ref(), &hid(11));

    // Prune to K=2 deletes the dead node with no mode/health gate.
    let pruned = writer.prune(2, 10_000).expect("prune");
    assert!(pruned >= 1, "the dead node must be pruned");
    assert_eq!(writer.pruned_to(), 2);
    assert!(
        backend
            .kv_get(Keyspace::Nodes, hid(10).as_slice())
            .unwrap()
            .is_none(),
        "dead node A is gone after prune"
    );
}
