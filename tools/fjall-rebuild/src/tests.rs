use crate::reachable::{self, Hash};
use fjall::{Database, KeyspaceCreateOptions};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::time::Instant;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("fjall-rebuild-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("tempdir");
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn h(n: u8) -> Hash {
    let mut x = [n; 32];
    x[0] = 0xA0;
    x
}

/// `EncodedBlob` layout: 8 zero bytes, type byte, prefixed node.
fn value(kind: u8, data: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 8];
    v.push(kind);
    v.extend_from_slice(data);
    v
}

fn inner(children: &[(usize, Hash)]) -> Vec<u8> {
    let mut d = b"MIN\0".to_vec();
    let mut slots = [[0u8; 32]; 16];
    for (i, c) in children {
        slots[*i] = *c;
    }
    for s in slots {
        d.extend_from_slice(&s);
    }
    value(3, &d)
}

fn leaf(n: u8) -> Vec<u8> {
    value(3, &[b'M', b'L', b'N', 0, n, n, n])
}

// Ledger 10 (claimed): state S10 -> {L1, I1 -> {L2}}, tx T10 -> {TX1}, header H10.
// Ledger 9: state S9 -> {L1, L3}, empty tx tree, header H9.
// Ledger 5 (below the floor, pruned_to = 8): state S5 -> {L4}, header-less.
// Leaked and untracked: X. Old header H3 (not in SQL). Named by bookkeeping:
// N1 (notebook), U1 (unclaimed), C1 and I1 (counts).
const S10: u8 = 1;
const I1: u8 = 2;
const L1: u8 = 3;
const L2: u8 = 4;
const T10: u8 = 5;
const TX1: u8 = 6;
const H10: u8 = 7;
const S9: u8 = 8;
const L3: u8 = 9;
const H9: u8 = 10;
const S5: u8 = 11;
const L4: u8 = 12;
const X: u8 = 13;
const H3: u8 = 14;
const N1: u8 = 15;
const U1: u8 = 16;
const C1: u8 = 17;
const H5: u8 = 18;

fn build_old(dir: &Path, skip: &[u8]) {
    let db = Database::builder(dir.join("fjall")).open().expect("open");
    let nodes = db
        .keyspace("nodes", KeyspaceCreateOptions::default)
        .expect("nodes");
    let all: Vec<(u8, Vec<u8>)> = vec![
        (S10, inner(&[(0, h(L1)), (5, h(I1))])),
        (I1, inner(&[(2, h(L2))])),
        (L1, leaf(L1)),
        (L2, leaf(L2)),
        (T10, inner(&[(1, h(TX1))])),
        (TX1, value(4, b"SND\0tx")),
        (H10, value(1, b"LWR\0h10")),
        (S9, inner(&[(0, h(L1)), (9, h(L3))])),
        (L3, leaf(L3)),
        (H9, value(1, b"LWR\0h9")),
        (S5, inner(&[(3, h(L4))])),
        (L4, leaf(L4)),
        (X, leaf(X)),
        (H3, value(1, b"LWR\0h3")),
        (N1, leaf(N1)),
        (U1, leaf(U1)),
        (C1, leaf(C1)),
    ];
    for (n, v) in all {
        if !skip.contains(&n) {
            nodes.insert(h(n), v).expect("insert");
        }
    }
    let meta = db
        .keyspace("meta", KeyspaceCreateOptions::default)
        .expect("meta");
    meta.insert("claimed_seq", 10u32.to_le_bytes())
        .expect("meta");
    meta.insert("claimed_state_root", h(S10)).expect("meta");
    meta.insert("pruned_to", 8u32.to_le_bytes()).expect("meta");
    // The pre-claim tree is below the floor, so it must not be walked.
    meta.insert("completed_seq", 5u32.to_le_bytes())
        .expect("meta");
    meta.insert("completed_state_root", h(S5)).expect("meta");
    let notebook = db
        .keyspace("notebook", KeyspaceCreateOptions::default)
        .expect("notebook");
    let mut nk = 11u32.to_be_bytes().to_vec();
    nk.extend_from_slice(&h(N1));
    notebook.insert(nk, [1u8]).expect("notebook");
    let counts = db
        .keyspace("counts", KeyspaceCreateOptions::default)
        .expect("counts");
    counts.insert(h(C1), 0u32.to_le_bytes()).expect("counts");
    counts.insert(h(I1), 2u32.to_le_bytes()).expect("counts");
    let unclaimed = db
        .keyspace("unclaimed", KeyspaceCreateOptions::default)
        .expect("unclaimed");
    unclaimed
        .insert(h(U1), 10u32.to_le_bytes())
        .expect("unclaimed");
    db.persist(fjall::PersistMode::SyncAll).expect("persist");

    let hex = |n: u8| h(n).iter().map(|b| format!("{b:02X}")).collect::<String>();
    let zero = "0".repeat(64);
    let rows = [
        (5u32, hex(H5), hex(S5), zero.clone()),
        (9, hex(H9), hex(S9), zero.clone()),
        (10, hex(H10), hex(S10), hex(T10)),
    ];
    for (file, keep) in [("ledger_headers.db", &rows[..]), ("ledger.db", &rows[2..])] {
        let conn = Connection::open(dir.join(file)).expect("sql");
        conn.execute_batch(
            "CREATE TABLE Ledgers (LedgerHash TEXT, LedgerSeq INTEGER, AccountSetHash TEXT, TransSetHash TEXT);",
        )
        .expect("table");
        for (seq, l, a, t) in keep {
            conn.execute(
                "INSERT INTO Ledgers VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![l, seq, a, t],
            )
            .expect("row");
        }
    }
}

fn has(db: &Database, ks: &str, key: &[u8]) -> bool {
    db.keyspace(ks, KeyspaceCreateOptions::default)
        .expect("ks")
        .contains_key(key)
        .expect("read")
}

#[test]
fn reachable_copy_keeps_retained_and_named_nodes_and_drops_the_rest() {
    let t = TempDir::new();
    build_old(&t.0, &[]);
    let old = Database::builder(t.0.join("fjall")).open().expect("old");
    let plan = reachable::plan(&old, &t.0, 4).expect("plan");
    assert_eq!(plan.floor, 9);
    let new = Database::builder(t.0.join("fjall.new"))
        .open()
        .expect("new");
    let (count, check) = reachable::copy(&old, &new, plan).expect("copy");
    reachable::verify(&new, &check, 4).expect("verify");

    for n in [S10, I1, L1, L2, T10, TX1, H10, S9, L3, H9, H3, N1, U1, C1] {
        assert!(has(&new, "nodes", &h(n)), "node {n} must be kept");
    }
    for n in [S5, L4, X] {
        assert!(!has(&new, "nodes", &h(n)), "node {n} must be dropped");
    }
    assert_eq!((count.kept, count.dropped), (14, 3));
    assert!(has(&new, "unclaimed", &h(U1)));
    assert!(has(&new, "counts", &h(C1)));
    assert!(has(&new, "meta", b"claimed_seq"));
    let mut nk = 11u32.to_be_bytes().to_vec();
    nk.extend_from_slice(&h(N1));
    assert!(has(&new, "notebook", &nk));
}

#[test]
fn incomplete_claimed_tree_stops_the_copy() {
    let t = TempDir::new();
    build_old(&t.0, &[L2]);
    let old = Database::builder(t.0.join("fjall")).open().expect("old");
    assert!(reachable::plan(&old, &t.0, 4).is_err());
}

#[test]
fn verify_fails_when_the_new_store_lost_a_retained_node() {
    let t = TempDir::new();
    build_old(&t.0, &[]);
    let old = Database::builder(t.0.join("fjall")).open().expect("old");
    let plan = reachable::plan(&old, &t.0, 4).expect("plan");
    let new = Database::builder(t.0.join("fjall.new"))
        .open()
        .expect("new");
    let (_, check) = reachable::copy(&old, &new, plan).expect("copy");
    new.keyspace("nodes", KeyspaceCreateOptions::default)
        .expect("nodes")
        .remove(h(L3).to_vec())
        .expect("rm");
    assert!(reachable::verify(&new, &check, 4).is_err());
}

#[test]
fn plain_copy_keeps_the_unclaimed_keyspace() {
    let t = TempDir::new();
    build_old(&t.0, &[]);
    let old = Database::builder(t.0.join("fjall")).open().expect("old");
    let new = Database::builder(t.0.join("fjall.new"))
        .open()
        .expect("new");
    crate::copy_all(&old, &new, Instant::now());
    assert!(has(&new, "unclaimed", &h(U1)));
    assert!(has(&new, "nodes", &h(X)), "plain copy keeps every node");
}

#[test]
fn node_already_missing_from_an_older_window_tree_does_not_stop_the_copy() {
    let t = TempDir::new();
    build_old(&t.0, &[L3]);
    let old = Database::builder(t.0.join("fjall")).open().expect("old");
    let plan = reachable::plan(&old, &t.0, 4).expect("plan");
    let new = Database::builder(t.0.join("fjall.new"))
        .open()
        .expect("new");
    let (count, check) = reachable::copy(&old, &new, plan).expect("copy");
    reachable::verify(&new, &check, 4).expect("same retained nodes as the old store");
    assert_eq!((count.kept, count.dropped), (13, 3));
}
