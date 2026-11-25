//! Reachable-only copy of a stopped quaxar fjall store.
//!
//! The pruned store leaked nodes it never tracked
//! (docs/design/2026-11-09-pruned-store-leak.md): they are in no retained
//! ledger and in none of the pruning bookkeeping, so quaxar never deletes or
//! reads them again. This copy keeps a `nodes` entry when any of these holds:
//!
//! - it is reachable from a root of a ledger at or above the retained floor
//!   (`meta/pruned_to + 1`): the state, transaction and header roots of every
//!   such row in `ledger.db` and `ledger_headers.db`, plus the claimed and
//!   completed state roots in `meta`;
//! - the pruning bookkeeping names it: a `counts` row, a `notebook` death
//!   record or an `unclaimed` row;
//! - it is a ledger header object (type byte 1).
//!
//! Every other keyspace is copied whole. The result is then re-walked from
//! every root, and the copy fails unless every node found in the old store is
//! found in the new one.

use fjall::{Database, Keyspace, KeyspaceCreateOptions};
use rusqlite::{Connection, OpenFlags};
use std::collections::HashSet;
use std::hash::{BuildHasherDefault, Hasher};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

pub type Hash = [u8; 32];

/// Stored value layout (`EncodedBlob`): 8 zero bytes, the type byte, then the
/// node in prefix format.
const TYPE_OFFSET: usize = 8;
const TYPE_LEDGER: u8 = 1;
/// Inner node: `MIN\0` then 16 child hashes, zero for an empty branch.
const INNER_PREFIX: &[u8; 4] = b"MIN\0";
const INNER_LEN: usize = 4 + 16 * 32;

/// Node keys are SHA-512-half digests, so their first 8 bytes are already a
/// uniform hash.
#[derive(Default)]
struct FirstBytes(u64);

impl Hasher for FirstBytes {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        // `[u8; 32]` hashes its length (8 bytes) and then its 32 bytes.
        if bytes.len() == 32 {
            self.0 = u64::from_le_bytes(bytes[..8].try_into().expect("8 bytes"));
        }
    }
}

const SHARDS: usize = 256;

/// Set of node hashes shared by the walk threads.
pub struct Seen {
    shards: Vec<Mutex<HashSet<Hash, BuildHasherDefault<FirstBytes>>>>,
}

impl Seen {
    pub fn new() -> Self {
        Self {
            shards: (0..SHARDS).map(|_| Mutex::default()).collect(),
        }
    }
    /// Shard on the last byte, which the in-shard hasher does not use.
    fn shard(&self, h: &Hash) -> &Mutex<HashSet<Hash, BuildHasherDefault<FirstBytes>>> {
        &self.shards[h[31] as usize]
    }
    pub fn insert(&self, h: Hash) -> bool {
        self.shard(&h).lock().expect("seen lock").insert(h)
    }
    pub fn contains(&self, h: &Hash) -> bool {
        self.shard(h).lock().expect("seen lock").contains(h)
    }
    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|s| s.lock().expect("seen lock").len())
            .sum()
    }
}

/// Child hashes of a stored inner node; empty for a leaf or header.
pub fn children(value: &[u8]) -> Vec<Hash> {
    let Some(node) = value.get(TYPE_OFFSET + 1..) else {
        return Vec::new();
    };
    if node.len() != INNER_LEN || &node[..4] != INNER_PREFIX {
        return Vec::new();
    }
    node[4..]
        .chunks_exact(32)
        .map(|c| <Hash>::try_from(c).expect("32 bytes"))
        .filter(|h| *h != [0u8; 32])
        .collect()
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WalkCount {
    pub found: u64,
    pub missing: u64,
}

/// Result of one walk: the counts, and one missing node if any.
pub struct Walk {
    pub count: WalkCount,
    pub first_missing: Option<Hash>,
}

/// Walk the trees under `roots` with `threads` workers, skipping nodes
/// already in `seen`, and count the nodes found and missing.
pub fn walk(nodes: &Keyspace, roots: &[Hash], seen: &Seen, threads: usize) -> Walk {
    struct Queue {
        stack: Vec<Hash>,
        /// Queued plus in-progress nodes; the walk ends at zero.
        pending: usize,
    }
    let mut start = Vec::new();
    for r in roots {
        if *r != [0u8; 32] && seen.insert(*r) {
            start.push(*r);
        }
    }
    let queue = Mutex::new(Queue {
        pending: start.len(),
        stack: start,
    });
    let ready = Condvar::new();
    let (found, missing) = (AtomicU64::new(0), AtomicU64::new(0));
    let first_missing = Mutex::new(None);
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| loop {
                let batch: Vec<Hash> = {
                    let mut q = queue.lock().expect("queue lock");
                    while q.stack.is_empty() {
                        if q.pending == 0 {
                            return;
                        }
                        q = ready.wait(q).expect("queue lock");
                    }
                    let at = q.stack.len().saturating_sub(64);
                    q.stack.split_off(at)
                };
                let mut next = Vec::new();
                for h in &batch {
                    match nodes.get(h).expect("read nodes") {
                        Some(v) => {
                            let n = found.fetch_add(1, Ordering::Relaxed) + 1;
                            if n % 2_000_000 == 0 {
                                eprintln!("  walked {n} nodes");
                            }
                            next.extend(children(&v).into_iter().filter(|c| seen.insert(*c)));
                        }
                        None => {
                            if missing.fetch_add(1, Ordering::Relaxed) == 0 {
                                *first_missing.lock().expect("missing lock") = Some(*h);
                            }
                        }
                    }
                }
                let mut q = queue.lock().expect("queue lock");
                q.pending = q.pending + next.len() - batch.len();
                q.stack.extend(next);
                ready.notify_all();
            });
        }
    });
    Walk {
        count: WalkCount {
            found: found.into_inner(),
            missing: missing.into_inner(),
        },
        first_missing: first_missing.into_inner().expect("missing lock"),
    }
}

fn hex(h: &[u8]) -> String {
    h.iter().map(|b| format!("{b:02X}")).collect()
}

fn unhex(s: &str) -> Option<Hash> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// One `Ledgers` row: sequence, ledger (header) hash, state root, tx root.
pub type LedgerRoots = (u32, Hash, Hash, Hash);

/// Rows at or above `floor` from a `Ledgers` table; none if the file is absent.
pub fn sql_roots(path: &Path, floor: u32) -> Result<Vec<LedgerRoots>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut stmt = conn
        .prepare(
            "SELECT LedgerSeq, LedgerHash, AccountSetHash, TransSetHash FROM Ledgers \
             WHERE LedgerSeq >= ?1 ORDER BY LedgerSeq",
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let rows = stmt
        .query_map([floor], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (seq, l, a, t) = row.map_err(|e| e.to_string())?;
        match (unhex(&l), unhex(&a), unhex(&t)) {
            (Some(l), Some(a), Some(t)) => out.push((seq, l, a, t)),
            _ => return Err(format!("{}: bad hash in ledger {seq}", path.display())),
        }
    }
    Ok(out)
}

fn meta_u32(meta: &Keyspace, key: &str) -> Option<u32> {
    let v = meta.get(key).expect("read meta")?;
    Some(u32::from_le_bytes(v.as_ref().try_into().ok()?))
}

fn meta_hash(meta: &Keyspace, key: &str) -> Option<Hash> {
    let v = meta.get(key).expect("read meta")?;
    v.as_ref().try_into().ok()
}

/// Roots walked together: one ledger's state, transaction and header roots,
/// or a single tree named in `meta`.
pub struct Group {
    pub label: String,
    pub roots: Vec<Hash>,
}

/// What the old store needs kept.
pub struct Plan {
    pub floor: u32,
    pub claimed_seq: u32,
    /// The claimed state tree first; it must be complete.
    pub groups: Vec<Group>,
    /// Walk counts per group, in order.
    pub walked: Vec<WalkCount>,
    pub seen: Seen,
}

fn keyspace(db: &Database, name: &str) -> Keyspace {
    assert!(db.keyspace_exists(name), "keyspace {name} missing");
    db.keyspace(name, KeyspaceCreateOptions::default)
        .expect("open keyspace")
}

/// Walk `groups` in order over one node set and print a summary. Counts
/// depend only on the store contents and the order, so two stores holding the
/// same retained nodes give equal counts.
fn walk_groups(nodes: &Keyspace, groups: &[Group], threads: usize) -> (Vec<WalkCount>, Seen) {
    let t0 = Instant::now();
    let seen = Seen::new();
    let mut counts = Vec::with_capacity(groups.len());
    let mut incomplete = Vec::new();
    for g in groups {
        let w = walk(nodes, &g.roots, &seen, threads);
        if counts.is_empty() {
            println!(
                "{}: {} found, {} missing, {:.0} s",
                g.label,
                w.count.found,
                w.count.missing,
                t0.elapsed().as_secs_f64()
            );
        }
        if let Some(h) = w.first_missing {
            incomplete.push(format!(
                "{} ({} missing, e.g. {})",
                g.label,
                w.count.missing,
                hex(&h)
            ));
        }
        counts.push(w.count);
    }
    let found: u64 = counts.iter().map(|c| c.found).sum();
    let missing: u64 = counts.iter().map(|c| c.missing).sum();
    println!(
        "{} groups: {found} found, {missing} missing in {} groups, {:.0} s",
        groups.len(),
        incomplete.len(),
        t0.elapsed().as_secs_f64()
    );
    for line in incomplete.iter().take(10) {
        println!("  incomplete: {line}");
    }
    (counts, seen)
}

/// Walk every retained root of `old` and add the bookkeeping-named nodes.
pub fn plan(old: &Database, db_dir: &Path, threads: usize) -> Result<Plan, String> {
    let t0 = Instant::now();
    let meta = keyspace(old, "meta");
    let claimed_seq =
        meta_u32(&meta, "claimed_seq").ok_or("store has no claimed_seq: not anchored")?;
    let claimed_root =
        meta_hash(&meta, "claimed_state_root").ok_or("store has no claimed_state_root")?;
    let floor = meta_u32(&meta, "pruned_to").unwrap_or(0).saturating_add(1);
    let mut rows = sql_roots(&db_dir.join("ledger_headers.db"), floor)?;
    rows.extend(sql_roots(&db_dir.join("ledger.db"), floor)?);
    rows.sort();
    rows.dedup();
    let (lo, hi) = (rows.first().map(|r| r.0), rows.last().map(|r| r.0));
    println!(
        "floor {floor}, claimed {claimed_seq}, {} SQL rows, seq {lo:?}..={hi:?}, claimed row present: {}",
        rows.len(),
        rows.iter().any(|r| r.0 == claimed_seq)
    );
    if rows.is_empty() {
        return Err("no SQL ledger rows at or above the floor".into());
    }
    let mut groups = vec![Group {
        label: "claimed state tree".into(),
        roots: vec![claimed_root],
    }];
    // The pre-claim tree matters only while it is inside the window.
    if let (Some(seq), Some(root)) = (
        meta_u32(&meta, "completed_seq"),
        meta_hash(&meta, "completed_state_root"),
    ) {
        if seq >= floor {
            groups.push(Group {
                label: format!("completed state tree {seq}"),
                roots: vec![root],
            });
        }
    }
    for (seq, l, a, t) in rows.iter().rev() {
        groups.push(Group {
            label: format!("ledger {seq}"),
            roots: vec![*a, *t, *l],
        });
    }
    let (walked, seen) = walk_groups(&keyspace(old, "nodes"), &groups, threads);
    if walked[0].missing > 0 {
        return Err("claimed state tree is incomplete in the old store".into());
    }
    // Added only after the walks: a node seen early would not be descended.
    let mut named = 0u64;
    for (name, at) in [("counts", 0usize), ("unclaimed", 0), ("notebook", 4)] {
        if !old.keyspace_exists(name) {
            continue;
        }
        for kv in keyspace(old, name).iter() {
            let k = kv.key().expect("read key");
            if let Some(h) = k.get(at..at + 32) {
                named += u64::from(seen.insert(h.try_into().expect("32 bytes")));
            }
        }
    }
    println!(
        "bookkeeping names {named} more nodes, {} to keep, {:.0} s",
        seen.len(),
        t0.elapsed().as_secs_f64()
    );
    Ok(Plan {
        floor,
        claimed_seq,
        groups,
        walked,
        seen,
    })
}

/// Should the `nodes` entry `key` with `value` be kept?
fn keep(seen: &Seen, key: &[u8], value: &[u8]) -> bool {
    value.get(TYPE_OFFSET) == Some(&TYPE_LEDGER)
        || key.try_into().is_ok_and(|h: Hash| seen.contains(&h))
}

#[derive(Debug, Default)]
pub struct CopyCount {
    pub kept: u64,
    pub kept_bytes: u64,
    pub dropped: u64,
    pub dropped_bytes: u64,
}

impl CopyCount {
    fn add(&mut self, kept: bool, bytes: u64) {
        if kept {
            self.kept += 1;
            self.kept_bytes += bytes;
        } else {
            self.dropped += 1;
            self.dropped_bytes += bytes;
        }
    }
}

/// Count what a copy would keep, without writing.
pub fn dry_run(old: &Database, plan: &Plan) -> CopyCount {
    let mut c = CopyCount::default();
    for kv in keyspace(old, "nodes").iter() {
        let (k, v) = kv.into_inner().expect("read");
        c.add(keep(&plan.seen, &k, &v), (k.len() + v.len()) as u64);
    }
    c
}

/// What `verify` needs once the plan's node set is freed.
pub struct Check {
    pub groups: Vec<Group>,
    pub walked: Vec<WalkCount>,
}

/// Copy `old` into the empty `new`, keeping only planned nodes.
pub fn copy(old: &Database, new: &Database, plan: Plan) -> Result<(CopyCount, Check), String> {
    let t0 = Instant::now();
    let mut nodes_count = CopyCount::default();
    let mut names: Vec<String> = old
        .list_keyspace_names()
        .iter()
        .map(|n| n.to_string())
        .collect();
    names.sort();
    for name in &names {
        let from = keyspace(old, name);
        let to = new
            .keyspace(name, || crate::options(name))
            .expect("new keyspace");
        let mut ing = to.start_ingestion().expect("ingestion");
        let mut c = CopyCount::default();
        for kv in from.iter() {
            let (k, v) = kv.into_inner().expect("read");
            let bytes = (k.len() + v.len()) as u64;
            let kept = name != "nodes" || keep(&plan.seen, &k, &v);
            c.add(kept, bytes);
            if kept {
                ing.write(k, v).expect("write");
            }
        }
        ing.finish().expect("finish");
        println!(
            "{name}: kept {} ({:.2} GB), dropped {} ({:.2} GB), {:.0} s",
            c.kept,
            c.kept_bytes as f64 / 1e9,
            c.dropped,
            c.dropped_bytes as f64 / 1e9,
            t0.elapsed().as_secs_f64()
        );
        if name == "nodes" {
            nodes_count = c;
        }
    }
    new.persist(fjall::PersistMode::SyncAll)
        .map_err(|e| e.to_string())?;
    let check = Check {
        groups: plan.groups,
        walked: plan.walked,
    };
    Ok((nodes_count, check))
}

/// Re-walk every group in `new`: each must find exactly the nodes, and miss
/// exactly the nodes, that the old store's walk did.
pub fn verify(new: &Database, check: &Check, threads: usize) -> Result<(), String> {
    println!("verify:");
    let (got, _) = walk_groups(&keyspace(new, "nodes"), &check.groups, threads);
    let differ: Vec<String> = check
        .groups
        .iter()
        .zip(got.iter().zip(&check.walked))
        .filter(|(_, (new, old))| new != old)
        .map(|(g, (new, old))| format!("{}: new {new:?}, old {old:?}", g.label))
        .collect();
    if differ.is_empty() {
        return Ok(());
    }
    for line in differ.iter().take(10) {
        println!("  differs: {line}");
    }
    Err(format!(
        "new store differs from the old store in {} retained groups",
        differ.len()
    ))
}
