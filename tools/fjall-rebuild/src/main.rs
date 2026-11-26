//! Offline store rebuild: copy every live key of a stopped quaxar fjall store
//! into a NEW store created with quaxar's current keyspace options
//! (fjall_backend.rs `keyspace_options`, `compaction_base_mb` from
//! `FJALL_REBUILD_BASE_MB`, default 2048), so the
//! nodes keyspace gets partitioned, pinned indexes and filters (#135).
//! Keys are read in order and written with fjall's bulk ingestion; deleted
//! keys (tombstones) are not copied. Prints progress every 5 s.
//! Usage:
//!   rebuild <old-store> <new-store>
//!   rebuild --compact <store>
//!   rebuild --reachable <old-store> <new-store> <db-dir>
//!   rebuild --reachable-dry-run <old-store> <db-dir>
//! `--reachable` keeps only the nodes a retained ledger or the pruning
//! bookkeeping needs (see reachable.rs); `<db-dir>` holds `ledger.db` and
//! `ledger_headers.db`.
use fjall::config::{PartitioningPolicy, PinningPolicy};
use fjall::{Database, KeyspaceCreateOptions};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

mod reachable;

/// Walk threads: point reads wait on the disk, so use more than the cores.
/// `FJALL_REBUILD_THREADS` and `FJALL_REBUILD_CACHE_MB` override the walk
/// threads and the old store's block cache, for a dry run beside a live node.
const WALK_THREADS: usize = 16;

fn env_or(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The store's `compaction_base_mb` (`FJALL_REBUILD_BASE_MB`, default 2048,
/// quaxar's default); fjall's table target is a quarter of it.
fn table_target_bytes() -> u64 {
    env_or("FJALL_REBUILD_BASE_MB", 2048) as u64 * 1024 * 1024 / 4
}

fn options(name: &str) -> KeyspaceCreateOptions {
    let o = KeyspaceCreateOptions::default();
    if name != "nodes" {
        return o;
    }
    // Must match quaxar keyspace_options(Keyspace::Nodes, base / 4).
    o.compaction_strategy(Arc::new(
        fjall::compaction::Leveled::default()
            .with_l0_threshold(4)
            .with_table_target_size(table_target_bytes()),
    ))
    .index_block_partitioning_policy(PartitioningPolicy::all(true))
    .filter_block_partitioning_policy(PartitioningPolicy::all(true))
    .index_block_pinning_policy(PinningPolicy::all(true))
    .filter_block_pinning_policy(PinningPolicy::all(true))
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a[1] == "--compact" {
        let db = Database::builder(&a[2]).worker_threads(2).open().expect("open");
        compact(&db, Instant::now());
        return;
    }
    if a[1] == "--reachable" || a[1] == "--reachable-dry-run" {
        let dry = a[1] == "--reachable-dry-run";
        let (src, dst, db_dir) = if dry {
            (&a[2], None, &a[3])
        } else {
            (&a[2], Some(&a[3]), &a[4])
        };
        if let Some(dst) = dst {
            assert!(!Path::new(dst).exists(), "destination must not exist");
        }
        let t0 = Instant::now();
        let threads = env_or("FJALL_REBUILD_THREADS", WALK_THREADS);
        // quaxar is stopped, so the walk may use a large block cache.
        let cache_mb = env_or("FJALL_REBUILD_CACHE_MB", 2048) as u64;
        let old = Database::builder(src)
            .worker_threads(1)
            .cache_size(cache_mb << 20)
            .open()
            .expect("open old");
        let plan =
            reachable::plan(&old, Path::new(db_dir), threads).unwrap_or_else(|e| panic!("{e}"));
        println!("plan: floor {}, claimed {}", plan.floor, plan.claimed_seq);
        let Some(dst) = dst else {
            let c = reachable::dry_run(&old, &plan);
            println!(
                "dry run: would keep {} nodes ({:.2} GB), drop {} ({:.2} GB), {:.0} s",
                c.kept,
                c.kept_bytes as f64 / 1e9,
                c.dropped,
                c.dropped_bytes as f64 / 1e9,
                t0.elapsed().as_secs_f64()
            );
            return;
        };
        let new = Database::builder(dst)
            .worker_threads(2)
            .cache_size(1 << 30)
            .open()
            .expect("create new");
        let (_, check) = reachable::copy(&old, &new, plan).unwrap_or_else(|e| panic!("{e}"));
        drop(old);
        compact(&new, t0);
        reachable::verify(&new, &check, threads).unwrap_or_else(|e| panic!("{e}"));
        println!("verified, total {:.0} s", t0.elapsed().as_secs_f64());
        return;
    }
    let (src, dst) = (&a[1], &a[2]);
    assert!(!std::path::Path::new(dst).exists(), "destination must not exist");
    let old = Database::builder(src).worker_threads(1).cache_size(256 << 20).open().expect("open old");
    let new = Database::builder(dst).worker_threads(2).open().expect("create new");
    let t0 = Instant::now();
    copy_all(&old, &new, t0);
    compact(&new, t0);
}

/// Copy every key of every keyspace, including `unclaimed` (#156).
fn copy_all(old: &Database, new: &Database, t0: Instant) {
    let names: Vec<String> = old
        .list_keyspace_names()
        .iter()
        .map(|n| n.to_string())
        .collect();
    for name in &names {
        let name = name.as_str();
        let from = old.keyspace(name, KeyspaceCreateOptions::default).expect("old keyspace");
        let to = new.keyspace(name, || options(name)).expect("new keyspace");
        let mut ing = to.start_ingestion().expect("ingestion");
        let (mut n, mut bytes, mut last) = (0u64, 0u64, Instant::now());
        for kv in from.iter() {
            let (k, v) = kv.into_inner().expect("read");
            bytes += (k.len() + v.len()) as u64;
            ing.write(k, v).expect("write");
            n += 1;
            if last.elapsed().as_secs() >= 5 {
                eprintln!("{name}: {n} keys, {:.2} GB, {:.0} s", bytes as f64 / 1e9, t0.elapsed().as_secs_f64());
                last = Instant::now();
            }
        }
        ing.finish().expect("finish");
        println!("{name}: copied {n} keys, {:.2} GB in {:.0} s", bytes as f64 / 1e9, t0.elapsed().as_secs_f64());
    }
    new.persist(fjall::PersistMode::SyncAll).expect("persist");
}

/// Ingestion writes canonical-level-1 tables (no data compression, 64 MB).
/// Rewrite nodes into the last level with 1 GiB tables, as quaxar compaction
/// would, so data blocks use the last level LZ4 policy.
fn compact(db: &Database, t0: Instant) {
    use fjall::AbstractTree;
    let nodes = db.keyspace("nodes", KeyspaceCreateOptions::default).expect("nodes");
    nodes.tree.major_compact(table_target_bytes(), u64::MAX >> 1).expect("major compaction");
    db.persist(fjall::PersistMode::SyncAll).expect("persist");
    println!("major compaction done, nodes disk {:.2} GB, total {:.0} s", nodes.disk_space() as f64 / 1e9, t0.elapsed().as_secs_f64());
}

#[cfg(test)]
mod tests;
