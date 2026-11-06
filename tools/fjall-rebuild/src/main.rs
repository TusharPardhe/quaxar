//! Offline store rebuild: copy every live key of a stopped quaxar fjall store
//! into a NEW store created with quaxar's current keyspace options
//! (fjall_backend.rs `keyspace_options`, compaction_base_mb = 4096), so the
//! nodes keyspace gets partitioned, pinned indexes and filters (#135).
//! Keys are read in order and written with fjall's bulk ingestion; deleted
//! keys (tombstones) are not copied. Prints progress every 5 s.
//! Usage: rebuild <old-store> <new-store>
use fjall::config::{PartitioningPolicy, PinningPolicy};
use fjall::{Database, KeyspaceCreateOptions};
use std::sync::Arc;
use std::time::Instant;

fn options(name: &str) -> KeyspaceCreateOptions {
    let o = KeyspaceCreateOptions::default();
    if name != "nodes" {
        return o;
    }
    // Must match quaxar keyspace_options(Keyspace::Nodes, 4096 MB / 4).
    o.compaction_strategy(Arc::new(
        fjall::compaction::Leveled::default()
            .with_l0_threshold(4)
            .with_table_target_size(4096 * 1024 * 1024 / 4),
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
    let (src, dst) = (&a[1], &a[2]);
    assert!(!std::path::Path::new(dst).exists(), "destination must not exist");
    let old = Database::builder(src).worker_threads(1).cache_size(256 << 20).open().expect("open old");
    let new = Database::builder(dst).worker_threads(2).open().expect("create new");
    let t0 = Instant::now();
    for name in ["nodes", "notebook", "counts", "meta"] {
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
    compact(&new, t0);
}

/// Ingestion writes canonical-level-1 tables (no data compression, 64 MB).
/// Rewrite nodes into the last level with 1 GiB tables, as quaxar compaction
/// would, so data blocks use the last level LZ4 policy.
fn compact(db: &Database, t0: Instant) {
    use fjall::AbstractTree;
    let nodes = db.keyspace("nodes", KeyspaceCreateOptions::default).expect("nodes");
    nodes.tree.major_compact(4096 * 1024 * 1024 / 4, u64::MAX >> 1).expect("major compaction");
    db.persist(fjall::PersistMode::SyncAll).expect("persist");
    println!("major compaction done, nodes disk {:.2} GB, total {:.0} s", nodes.disk_space() as f64 / 1e9, t0.elapsed().as_secs_f64());
}
