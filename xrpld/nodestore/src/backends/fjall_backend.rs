//! Fjall-backed node store: a pure-Rust LSM engine that provides the key-value
//! v2 surface the pruned store needs (per-key deletes, multiple keyspaces,
//! atomic cross-keyspace batches, ordered range scans, an explicit durability
//! barrier). It also serves the legacy hash -> NodeObject API from its `nodes`
//! keyspace using the same `EncodedBlob`/`DecodedBlob` codec as the legacy
//! node store, so snapshot export/import and db-stats see an identical byte
//! format.

use crate::backends::kv::{Keyspace, KvBatch, KvOp, PersistMode};
use crate::{
    Backend, DecodedBlob, EncodedBlob, Factory, NodeObject, NodeStoreJournal, Scheduler, Status,
};
use basics::{base_uint::Uint256, basic_config::Section};
use fjall::{
    Database, Keyspace as FjallKeyspace, KeyspaceCreateOptions, PersistMode as FjallPersistMode,
};
use std::sync::{Arc, Mutex};

/// Default block-cache budget when the operator does not size it explicitly.
/// fjall recommends roughly 20-25% of available memory; the node layer passes
/// a concrete value derived from `node_size`, so this is only a floor.
const DEFAULT_CACHE_BYTES: u64 = 256 * 1024 * 1024;

fn read_string(section: &Section, key: &str) -> String {
    section
        .get::<String>(key)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn read_cache_bytes(section: &Section) -> u64 {
    section
        .get::<u64>("cache_mb")
        .ok()
        .flatten()
        .map(|mb| mb.saturating_mul(1024 * 1024))
        .filter(|&bytes| bytes > 0)
        .unwrap_or(DEFAULT_CACHE_BYTES)
}

/// Default disk-full ballast, in MB, when the operator does not set
/// `reserve_mb`. One GiB gives LSM compaction and tombstone deletes room to
/// run once the ballast is released under disk pressure (design Case 10).
const DEFAULT_RESERVE_MB: u64 = 1024;

/// Resolve the ballast size in bytes. `reserve_mb = 0` disables it; absence
/// uses the default.
fn read_reserve_bytes(section: &Section) -> u64 {
    section
        .get::<u64>("reserve_mb")
        .ok()
        .flatten()
        .unwrap_or(DEFAULT_RESERVE_MB)
        .saturating_mul(1024 * 1024)
}

/// Default size of the first leveled-compaction level of the `nodes`
/// keyspace, in MB. Sized for a pruned mainnet store (`online_delete = 512`,
/// live node set ~8 GB in 2026-11): half the live data. See
/// docs/CONFIGURATION.md for the formula; full-history stores should use 256.
const DEFAULT_COMPACTION_BASE_MB: u64 = 4096;
const COMPACTION_BASE_MB_RANGE: std::ops::RangeInclusive<u64> = 16..=65536;
/// Pruned stores (`online_delete > 0`) delete every node that leaves the
/// window, and fjall drops those tombstones only when they are merged into
/// the bottom level. A first level that is small next to the live node set
/// makes a deep tree whose penultimate level never reaches its target, so
/// tombstones accumulate and compaction stalls L0 writes (mainnet validator
/// 2026-10-31: base 64 MB, live ~8 GB, 575M tombstones, 30-minute stalls).
/// Measured on a write-once/delete-once workload: healthy when the base is
/// at least a quarter of the live data, failing at 1/31 and below. The
/// watchdog warns below this ratio so operators rebuild before stalls start.
const MIN_BASE_TO_LIVE_RATIO: u64 = 4;
/// How often the watchdog re-checks the base-to-live ratio.
const SIZING_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);
/// L0 tables merged at once; the base level is `L0_THRESHOLD` target tables.
const L0_THRESHOLD: u8 = 4;

/// Resolve `compaction_base_mb` into the per-table target size in bytes.
/// Rejects values outside the supported range instead of silently clamping.
fn read_table_target_bytes(section: &Section) -> Result<u64, String> {
    let base_mb = section
        .get::<u64>("compaction_base_mb")
        .map_err(|error| format!("invalid compaction_base_mb: {error}"))?
        .unwrap_or(DEFAULT_COMPACTION_BASE_MB);
    if !COMPACTION_BASE_MB_RANGE.contains(&base_mb) {
        return Err(format!(
            "compaction_base_mb must be between {} and {}, got {base_mb}",
            COMPACTION_BASE_MB_RANGE.start(),
            COMPACTION_BASE_MB_RANGE.end()
        ));
    }
    Ok(base_mb * 1024 * 1024 / u64::from(L0_THRESHOLD))
}

/// Create options for a keyspace. Only `nodes` is large enough for level
/// sizing to matter; the index keyspaces keep fjall defaults. fjall persists
/// these at creation, so a changed value applies only to a new store.
fn keyspace_options(keyspace: Keyspace, table_target_bytes: u64) -> KeyspaceCreateOptions {
    let options = KeyspaceCreateOptions::default();
    match keyspace {
        Keyspace::Nodes => options.compaction_strategy(Arc::new(
            fjall::compaction::Leveled::default()
                .with_l0_threshold(L0_THRESHOLD)
                .with_table_target_size(table_target_bytes),
        )),
        _ => options,
    }
}

fn ensure_ballast(path: &std::path::Path, len: u64) {
    if len == 0 || super::ballast::is_allocated(path, len) {
        return;
    }
    let started = std::time::Instant::now();
    match super::ballast::create(path, len) {
        Ok(method) => tracing::info!(
            target: "nodestore",
            path = %path.display(),
            reserve_bytes = len,
            method = ?method,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "fjall disk-full ballast reserved"
        ),
        Err(error) => {
            let _ = std::fs::remove_file(path);
            tracing::warn!(
                target: "nodestore",
                path = %path.display(),
                %error,
                "could not create fjall disk-full ballast (continuing without reserve)"
            );
        }
    }
}

fn release_ballast(path: &std::path::Path) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => {
            tracing::error!(
                target: "nodestore",
                path = %path.display(),
                "disk nearly full: released the fjall disk-full ballast so compaction and pruning can run"
            );
            true
        }
        Err(_) => false,
    }
}

/// Release the ballast when free space falls below its size, and re-create it
/// once free space is back above three times its size (hysteresis, so it is
/// not created and released in a loop).
const BALLAST_RECREATE_FACTOR: u64 = 3;
/// How often the watchdog checks free space.
const DISK_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

fn maintain_ballast(dir: &std::path::Path, ballast: &std::path::Path, len: u64) {
    if len == 0 {
        return;
    }
    let Some(available) = super::ballast::available_bytes(dir) else {
        return;
    };
    match ballast_action(ballast.exists(), available, len) {
        BallastAction::Release => {
            release_ballast(ballast);
        }
        BallastAction::Recreate => ensure_ballast(ballast, len),
        BallastAction::Keep => {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BallastAction {
    Keep,
    Release,
    Recreate,
}

/// Release below `len` free; re-create at or above `3 * len` free.
fn ballast_action(present: bool, available: u64, len: u64) -> BallastAction {
    if present && available < len {
        BallastAction::Release
    } else if !present && available >= len.saturating_mul(BALLAST_RECREATE_FACTOR) {
        BallastAction::Recreate
    } else {
        BallastAction::Keep
    }
}

/// fjall starts `min(cores, 4)` workers and worker 0 only flushes, so on a
/// 2-vCPU host a single worker does every compaction. One long merge into the
/// bottom levels then lets L0 reach fjall's 30-run write halt (testnet cold
/// sync, 2026-10-15). Keep at least three compaction workers.
const MIN_FJALL_WORKERS: usize = 4;

fn fjall_worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .max(MIN_FJALL_WORKERS)
}

/// fjall's L0 write-stall threshold (`Keyspace::local_backpressure`): from 20
/// runs writers are throttled, from 30 they are halted until compaction
/// reduces L0.
const L0_STALL_RUNS: usize = 20;
/// How long L0 may stay at the stall threshold without shrinking before the
/// watchdog forces a flush.
const L0_STALL_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
/// How long writes may stay halted with nothing to rotate before the watchdog
/// runs a compaction itself.
const L0_FORCE_COMPACT_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// What the watchdog does after `halted_for` at the L0 stall limit, given
/// whether its memtable rotation actually sealed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StallAction {
    Wait,
    Compact,
}

/// lsm-tree's `rotate_memtable` is a no-op on an empty active memtable. Once
/// the only writer is parked in fjall's L0 halt, the memtable stays empty,
/// rotation schedules no flush, and therefore no compaction: the halt is
/// permanent (mainnet snapshot import, 2026-10-22, stuck at 8 GB). Compact
/// directly in that case.
fn stall_action(halted_for: std::time::Duration, rotated: bool) -> StallAction {
    if !rotated && halted_for >= L0_FORCE_COMPACT_GRACE {
        StallAction::Compact
    } else {
        StallAction::Wait
    }
}

/// fjall schedules compaction only after a flush. When writes are halted at
/// the L0 limit nothing flushes, so if the queued compaction requests were
/// consumed before L0 could be compacted the halt is permanent: writers sleep
/// in `local_backpressure`, the workers wait for messages, and node-store
/// writes never complete. Rotating the active memtable starts a flush, which
/// queues a compaction request for every worker.
fn spawn_compaction_watchdog(
    open: std::sync::Weak<OpenDb>,
    dir: std::path::PathBuf,
    reserve_bytes: u64,
    pruned: bool,
) {
    let ballast = dir.join(".reserve_ballast");
    let spawned = std::thread::Builder::new()
        .name("fjall-watchdog".to_owned())
        .spawn(move || {
            let mut stuck_since: Option<(std::time::Instant, usize)> = None;
            let mut halted_since: Option<std::time::Instant> = None;
            let mut last_disk_check = std::time::Instant::now();
            // First check shortly after open, then hourly.
            let mut last_sizing_check = std::time::Instant::now()
                .checked_sub(SIZING_CHECK_INTERVAL)
                .unwrap_or_else(std::time::Instant::now);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let Some(open) = open.upgrade() else {
                    return;
                };
                if last_disk_check.elapsed() >= DISK_CHECK_INTERVAL {
                    last_disk_check = std::time::Instant::now();
                    maintain_ballast(&dir, &ballast, reserve_bytes);
                }
                if last_sizing_check.elapsed() >= SIZING_CHECK_INTERVAL {
                    last_sizing_check = std::time::Instant::now();
                    let live = bottom_level_bytes(&open.nodes);
                    if let Some(base) = effective_base_bytes(&open.nodes)
                        && sizing_warning_applies(pruned, base, live)
                    {
                        tracing::warn!(
                            target: "nodestore",
                            effective_base_mb = base / (1024 * 1024),
                            live_mb = live / (1024 * 1024),
                            recommended_base_mb = recommended_base_mb(live),
                            "fjall compaction_base_mb is too small for this store's live data; \
                             tombstones will accumulate and stall writes. Set the recommended \
                             compaction_base_mb and rebuild the store (the value is fixed at creation)"
                        );
                    }
                }
                let l0 = open.nodes.l0_table_count();
                halted_since = if l0 < L0_STALL_RUNS {
                    None
                } else {
                    halted_since.or_else(|| Some(std::time::Instant::now()))
                };
                stuck_since = match stuck_since {
                    _ if l0 < L0_STALL_RUNS => None,
                    Some((since, at)) if l0 >= at => Some((since, at)),
                    _ => Some((std::time::Instant::now(), l0)),
                };
                if let Some((since, _)) = stuck_since
                    && since.elapsed() >= L0_STALL_GRACE
                {
                    let rotated = open.nodes.rotate_memtable().unwrap_or(false);
                    tracing::warn!(
                        target: "nodestore",
                        l0_tables = l0,
                        sealed_memtables = open.nodes.sealed_memtable_count(),
                        rotated,
                        "fjall L0 at the write-stall limit without compaction progress; forced a flush to schedule compaction"
                    );
                    stuck_since = Some((std::time::Instant::now(), l0));
                    let halted_for = halted_since.map_or(std::time::Duration::ZERO, |at| at.elapsed());
                    if stall_action(halted_for, rotated) == StallAction::Compact {
                        let started = std::time::Instant::now();
                        let result = open.nodes.major_compact();
                        tracing::warn!(
                            target: "nodestore",
                            l0_before = l0,
                            l0_after = open.nodes.l0_table_count(),
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            ok = result.is_ok(),
                            "fjall writes halted with nothing to rotate; ran a compaction to release them"
                        );
                        halted_since = Some(std::time::Instant::now());
                    }
                }
            }
        });
    if let Err(error) = spawned {
        tracing::error!(target: "nodestore", %error, "could not start the fjall compaction watchdog");
    }
}


/// The first-level size this store was created with, in bytes. fjall
/// persists the leveled strategy at keyspace creation and ignores the
/// configured value on reopen, so this is the effective base.
fn effective_base_bytes(nodes: &FjallKeyspace) -> Option<u64> {
    let config = nodes.config.compaction_strategy.get_config();
    let target = config
        .iter()
        .find(|(key, _)| &key[..] == b"leveled_target_size")
        .and_then(|(_, value)| <[u8; 8]>::try_from(&value[..]).ok())
        .map(u64::from_le_bytes)?;
    Some(target.saturating_mul(u64::from(L0_THRESHOLD)))
}

/// Size of the deepest non-empty level of `nodes`: the live node set once
/// tombstones have been merged out.
fn bottom_level_bytes(nodes: &FjallKeyspace) -> u64 {
    use fjall::AbstractTree;
    let version = nodes.tree.current_version();
    version
        .iter_levels()
        .filter(|level| !level.is_empty())
        .last()
        .map_or(0, |level| level.size())
}

/// Whether the base is too small for the live data (see
/// `MIN_BASE_TO_LIVE_RATIO`). An empty or tiny store is never undersized.
fn base_undersized(base_bytes: u64, live_bytes: u64) -> bool {
    base_bytes.saturating_mul(MIN_BASE_TO_LIVE_RATIO) < live_bytes
}

/// The sizing warning applies only to pruned stores: with `online_delete = 0`
/// nothing is deleted, no tombstones build up, and a small base is correct.
fn sizing_warning_applies(pruned: bool, base_bytes: u64, live_bytes: u64) -> bool {
    pruned && base_undersized(base_bytes, live_bytes)
}

/// `compaction_base_mb` the docs recommend for `live_bytes`: half the live
/// node set, rounded up to a power of two, within the accepted range.
fn recommended_base_mb(live_bytes: u64) -> u64 {
    let half_mb = (live_bytes / 2).div_ceil(1024 * 1024).max(1);
    half_mb
        .next_power_of_two()
        .clamp(*COMPACTION_BASE_MB_RANGE.start(), *COMPACTION_BASE_MB_RANGE.end())
}

/// The open fjall database and its four keyspace handles, resolved once at
/// open so every operation is a direct handle call.
struct OpenDb {
    db: Database,
    nodes: FjallKeyspace,
    notebook: FjallKeyspace,
    counts: FjallKeyspace,
    meta: FjallKeyspace,
}

impl OpenDb {
    fn handle(&self, keyspace: Keyspace) -> &FjallKeyspace {
        match keyspace {
            Keyspace::Nodes => &self.nodes,
            Keyspace::Notebook => &self.notebook,
            Keyspace::Counts => &self.counts,
            Keyspace::Meta => &self.meta,
        }
    }
}

pub struct FjallBackend {
    name: String,
    path: String,
    cache_bytes: u64,
    /// Disk-full ballast size in bytes (design Case 10). A file of this size is
    /// created at open so that, when free space runs out, deleting it frees
    /// enough room for LSM compaction and tombstone-driven deletes to proceed.
    /// Zero disables the ballast.
    reserve_bytes: u64,
    /// Target table size for the `nodes` keyspace (`compaction_base_mb / 4`).
    table_target_bytes: u64,
    /// `online_delete > 0`: the store deletes nodes, so tombstones must reach
    /// the bottom level and the base-to-live ratio matters.
    pruned: bool,
    journal: Arc<dyn NodeStoreJournal>,
    db: Mutex<Option<Arc<OpenDb>>>,
    // When set (via set_delete_path, used by rotation to retire an archive),
    // close removes the database directory from disk after dropping it.
    delete_on_close: std::sync::atomic::AtomicBool,
    /// Dead-pending node hashes the pruned index asked us to watch, mapped to
    /// whether they were written again since. Bounded by the index's
    /// dead-pending set. Prune keeps any node marked `true` (design R3/R5).
    restore_watch: super::restore_watch::RestoreWatch,
}

impl FjallBackend {
    pub(crate) fn new(
        key_values: &Section,
        journal: Arc<dyn NodeStoreJournal>,
    ) -> Result<Self, String> {
        let path = read_string(key_values, "path");
        if path.is_empty() {
            return Err("Missing path in fjall backend".to_owned());
        }
        Ok(Self {
            name: path.clone(),
            path,
            cache_bytes: read_cache_bytes(key_values),
            reserve_bytes: read_reserve_bytes(key_values),
            table_target_bytes: read_table_target_bytes(key_values)?,
            pruned: key_values
                .get::<u64>("online_delete")
                .ok()
                .flatten()
                .is_some_and(|window| window > 0),
            journal,
            db: Mutex::new(None),
            delete_on_close: std::sync::atomic::AtomicBool::new(false),
            restore_watch: super::restore_watch::RestoreWatch::new(),
        })
    }

    /// Absolute path of the disk-full ballast file for this store.
    fn reserve_path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.path).join(".reserve_ballast")
    }

    /// Create the ballast if a reserve is configured and it is not already
    /// fully allocated (an older sparse ballast is replaced). Best effort: a
    /// filesystem that cannot allocate it (e.g. already full) is logged, not
    /// fatal, so the node still opens and can prune.
    fn ensure_reserve(&self) {
        ensure_ballast(&self.reserve_path(), self.reserve_bytes);
    }

    /// Release the ballast so freed space is available for compaction and
    /// deletes under disk pressure (design Case 10). Returns true if a ballast
    /// file was removed. Idempotent.
    pub fn release_reserve(&self) -> bool {
        release_ballast(&self.reserve_path())
    }

    /// Whether the ballast is present at its configured size and really
    /// occupies disk.
    pub fn reserve_present(&self) -> bool {
        self.reserve_bytes != 0 && super::ballast::is_allocated(&self.reserve_path(), self.reserve_bytes)
    }

    fn open_db(&self) -> Result<Arc<OpenDb>, String> {
        self.db
            .lock()
            .expect("fjall backend db mutex must not be poisoned")
            .clone()
            .ok_or_else(|| "fjall backend is not open".to_owned())
    }

    fn fjall_err(context: &str, error: impl std::fmt::Display) -> String {
        format!("fjall {context}: {error}")
    }
}

fn to_fjall_persist(mode: PersistMode) -> FjallPersistMode {
    match mode {
        PersistMode::Buffer => FjallPersistMode::Buffer,
        PersistMode::SyncAll => FjallPersistMode::SyncAll,
    }
}

impl Backend for FjallBackend {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn open(&self, create_if_missing: bool) -> Result<(), String> {
        let mut guard = self
            .db
            .lock()
            .expect("fjall backend db mutex must not be poisoned");
        if guard.is_some() {
            return Err("already open".to_owned());
        }
        if !create_if_missing && !std::path::Path::new(&self.path).exists() {
            return Err(format!("fjall database at {} does not exist", self.path));
        }
        let db = Database::builder(&self.path)
            .cache_size(self.cache_bytes)
            .worker_threads(fjall_worker_threads())
            .open()
            .map_err(|error| Self::fjall_err("open", error))?;
        // Each keyspace is its own LSM-tree with bloom filters and LZ4 on.
        let mut handles = Vec::with_capacity(Keyspace::ALL.len());
        for keyspace in Keyspace::ALL {
            let handle = db
                .keyspace(keyspace.name(), || {
                    keyspace_options(keyspace, self.table_target_bytes)
                })
                .map_err(|error| Self::fjall_err("keyspace", error))?;
            handles.push(handle);
        }
        let mut handles = handles.into_iter();
        let open = OpenDb {
            nodes: handles.next().expect("nodes keyspace"),
            notebook: handles.next().expect("notebook keyspace"),
            counts: handles.next().expect("counts keyspace"),
            meta: handles.next().expect("meta keyspace"),
            db,
        };
        let open = Arc::new(open);
        spawn_compaction_watchdog(
            Arc::downgrade(&open),
            std::path::PathBuf::from(&self.path),
            self.reserve_bytes,
            self.pruned,
        );
        *guard = Some(open);
        drop(guard);
        // Reserve the disk-full ballast once the store is open (design Case 10).
        self.ensure_reserve();
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.db
            .lock()
            .expect("fjall backend db mutex must not be poisoned")
            .is_some()
    }

    fn close(&self) -> Result<(), String> {
        // Dropping the OpenDb drops the Database, which persists the journal
        // with SyncAll (fjall's documented drop behavior).
        let open = self
            .db
            .lock()
            .expect("fjall backend db mutex must not be poisoned")
            .take();
        drop(open);
        // Honor a pending delete-path request (rotation retiring this store):
        // remove the whole database directory now that it is closed.
        if self
            .delete_on_close
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            let _ = std::fs::remove_dir_all(&self.path);
        }
        Ok(())
    }

    fn fetch(&self, hash: &Uint256) -> (Option<Arc<NodeObject>>, Status) {
        let Ok(open) = self.open_db() else {
            return (None, Status::NotFound);
        };
        // A read of a watched (dead-pending) node pins it so prune cannot
        // delete it under the ledger being built from it (restore_watch).
        let read = self.restore_watch.read(hash, || {
            let read = open.nodes.get(hash.as_slice());
            let bytes = match &read {
                Ok(Some(value)) => Some(value.to_vec()),
                _ => None,
            };
            (read, bytes)
        });
        match read {
            Ok(Some(value)) => {
                let decoded = DecodedBlob::new(hash.data(), &value);
                if decoded.was_ok() {
                    (Some(decoded.create_object()), Status::Ok)
                } else {
                    (None, Status::DataCorrupt)
                }
            }
            Ok(None) => (None, Status::NotFound),
            Err(error) => {
                self.journal
                    .log(crate::JournalLevel::Error, &Self::fjall_err("fetch", error));
                (None, Status::Unknown)
            }
        }
    }

    fn fetch_batch(&self, hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status) {
        let mut results = Vec::with_capacity(hashes.len());
        for hash in hashes {
            let (object, status) = self.fetch(hash);
            results.push(if status == Status::Ok { object } else { None });
        }
        (results, Status::Ok)
    }

    fn store(&self, object: Arc<NodeObject>) -> Result<(), String> {
        let open = self.open_db()?;
        let encoded = EncodedBlob::new(&object);
        self.restore_watch
            .before_write(object.hash(), || encoded.get_data().to_vec());
        open.nodes
            .insert(object.hash().as_slice(), encoded.get_data())
            .map_err(|error| Self::fjall_err("store", error))?;
        Ok(())
    }

    fn store_batch(&self, batch: &crate::Batch) {
        if let Err(error) = self.store_batch_result(batch) {
            self.journal.log(crate::JournalLevel::Error, &error);
        }
    }

    fn nodes_is_empty(&self) -> Option<bool> {
        // `store` and the KV `Nodes` keyspace share this partition.
        let open = self.open_db().ok()?;
        match open.nodes.iter().next() {
            None => Some(true),
            Some(entry) => entry.into_inner().ok().map(|_| false),
        }
    }

    fn store_batch_result(&self, batch: &crate::Batch) -> Result<(), String> {
        let open = self.open_db()?;
        let mut write = open.db.batch();
        for object in batch {
            let encoded = EncodedBlob::new(object);
            self.restore_watch
                .before_write(object.hash(), || encoded.get_data().to_vec());
            write.insert(&open.nodes, object.hash().as_slice(), encoded.get_data());
        }
        write
            .commit()
            .map_err(|error| Self::fjall_err("store_batch", error))?;
        Ok(())
    }

    fn sync(&self) {
        let _ = self.sync_result();
    }

    fn sync_result(&self) -> Result<(), String> {
        let open = self.open_db()?;
        open.db
            .persist(FjallPersistMode::SyncAll)
            .map_err(|error| Self::fjall_err("sync", error))
    }

    fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) {
        let _ = self.for_each_result(callback);
    }

    fn for_each_result(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) -> Result<(), String> {
        let open = self.open_db()?;
        for pair in open.nodes.iter() {
            let (key, value) = pair
                .into_inner()
                .map_err(|error| Self::fjall_err("for_each", error))?;
            let Some(hash) = Uint256::from_slice(&key) else {
                continue;
            };
            let decoded = DecodedBlob::new(hash.data(), &value);
            if decoded.was_ok() {
                callback(decoded.create_object());
            }
        }
        Ok(())
    }

    fn get_write_load(&self) -> i32 {
        0
    }

    fn set_delete_path(&self) {
        self.delete_on_close
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn fd_required(&self) -> i32 {
        // fjall manages its own file descriptors through a cached table; the
        // node layer does not need to reserve a fixed count for it.
        0
    }

    fn supports_kv(&self) -> bool {
        true
    }

    fn counters(&self) -> Vec<(&'static str, u64)> {
        let Ok(open) = self.open_db() else {
            return Vec::new();
        };
        let live = bottom_level_bytes(&open.nodes);
        let mut counters = vec![
            ("fjall_l0_tables", open.nodes.l0_table_count() as u64),
            ("fjall_live_mb", live / (1024 * 1024)),
            ("fjall_disk_mb", open.nodes.disk_space() / (1024 * 1024)),
            ("fjall_recommended_base_mb", recommended_base_mb(live)),
        ];
        if let Some(base) = effective_base_bytes(&open.nodes) {
            counters.push(("fjall_effective_base_mb", base / (1024 * 1024)));
            // Full-history stores never delete, so the ratio does not apply.
            let undersized = sizing_warning_applies(self.pruned, base, live);
            counters.push(("fjall_base_undersized", u64::from(undersized)));
        }
        counters
    }

    fn kv_get(&self, keyspace: Keyspace, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let open = self.open_db()?;
        open.handle(keyspace)
            .get(key)
            .map(|value| value.map(|bytes| bytes.to_vec()))
            .map_err(|error| Self::fjall_err("kv_get", error))
    }

    fn kv_write_batch(&self, batch: &KvBatch) -> Result<(), String> {
        let open = self.open_db()?;
        let mut write = open.db.batch();
        for op in batch.ops() {
            match op {
                KvOp::Put {
                    keyspace,
                    key,
                    value,
                } => {
                    write.insert(open.handle(*keyspace), key.as_slice(), value.as_slice());
                }
                KvOp::Delete { keyspace, key } => {
                    write.remove(open.handle(*keyspace), key.as_slice());
                }
            }
        }
        write
            .commit()
            .map_err(|error| Self::fjall_err("kv_write_batch", error))
    }

    fn kv_range(
        &self,
        keyspace: Keyspace,
        start: &[u8],
        end: &[u8],
        callback: &mut dyn FnMut(&[u8], &[u8]) -> bool,
    ) -> Result<(), String> {
        let open = self.open_db()?;
        // Half-open [start, end) to match the KvBatch/notebook contract.
        let range = start.to_vec()..end.to_vec();
        for pair in open.handle(keyspace).range(range) {
            let (key, value) = pair
                .into_inner()
                .map_err(|error| Self::fjall_err("kv_range", error))?;
            if !callback(&key, &value) {
                break;
            }
        }
        Ok(())
    }

    fn watch_restores(&self, hashes: &[Uint256]) {
        self.restore_watch.watch(hashes);
    }

    fn unwatch_restores(&self, hashes: &[Uint256]) {
        self.restore_watch.unwatch(hashes);
    }

    fn was_restored(&self, hash: &Uint256) -> bool {
        self.restore_watch.was_restored(hash)
    }

    fn begin_delete(&self, hashes: &[Uint256]) -> Vec<bool> {
        self.restore_watch.begin_delete(hashes)
    }

    fn finish_delete(&self, hashes: &[Uint256]) -> Vec<(Uint256, Vec<u8>)> {
        self.restore_watch.finish_delete(hashes)
    }

    fn kv_persist(&self, mode: PersistMode) -> Result<(), String> {
        let open = self.open_db()?;
        open.db
            .persist(to_fjall_persist(mode))
            .map_err(|error| Self::fjall_err("kv_persist", error))
    }
}

impl Drop for FjallBackend {
    fn drop(&mut self) {
        let _ = <Self as Backend>::close(self);
    }
}

pub struct FjallFactory;

impl FjallFactory {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FjallFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl Factory for FjallFactory {
    fn get_name(&self) -> String {
        "fjall".to_owned()
    }

    fn create_instance(
        &self,
        key_bytes: usize,
        parameters: &Section,
        burst_size: usize,
        scheduler: Arc<dyn Scheduler>,
        journal: Arc<dyn NodeStoreJournal>,
    ) -> crate::factory::BackendResult {
        let _ = (key_bytes, burst_size, scheduler);
        Ok(Box::new(FjallBackend::new(parameters, journal)?))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FjallBackend, FjallFactory, base_undersized, effective_base_bytes, read_table_target_bytes,
        recommended_base_mb, sizing_warning_applies,
    };
    use crate::{Backend, Factory, NodeObject, NodeObjectType, NullJournal};
    use basics::{base_uint::Uint256, basic_config::Section};
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    fn section(path: &str) -> Section {
        let mut section = Section::new("node_db");
        section.set("type", "fjall");
        section.set("path", path);
        section
    }

    fn temp_path(tag: &str) -> String {
        let mut dir = std::env::temp_dir();
        let n: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        dir.push(format!("quaxar-fjall-{tag}-{n}"));
        dir.to_string_lossy().into_owned()
    }

    fn open_backend(path: &str) -> Box<dyn Backend> {
        let backend = FjallFactory::new()
            .create_instance(
                NodeObject::KEY_BYTES,
                &section(path),
                0,
                Arc::new(crate::DummyScheduler),
                Arc::new(NullJournal),
            )
            .expect("fjall backend should construct");
        backend.open(true).expect("open should succeed");
        backend
    }

    #[test]
    fn fjall_backend_requires_path() {
        let result = FjallBackend::new(&Section::new("node_db"), Arc::new(NullJournal));
        match result {
            Ok(_) => panic!("backend should require a path"),
            Err(error) => assert_eq!(error, "Missing path in fjall backend"),
        }
    }

    #[test]
    fn watchdog_compacts_only_when_rotation_is_impossible_for_the_grace() {
        use super::{L0_FORCE_COMPACT_GRACE, StallAction, stall_action};
        use std::time::Duration;
        assert_eq!(stall_action(Duration::from_secs(5), false), StallAction::Wait);
        assert_eq!(stall_action(L0_FORCE_COMPACT_GRACE, true), StallAction::Wait);
        assert_eq!(stall_action(L0_FORCE_COMPACT_GRACE, false), StallAction::Compact);
        assert_eq!(stall_action(Duration::from_secs(600), false), StallAction::Compact);
    }

    #[test]
    fn compaction_base_mb_defaults_and_rejects_out_of_range() {
        let path = temp_path("base-parse");
        assert_eq!(
            read_table_target_bytes(&section(&path)).expect("default"),
            1024 * 1024 * 1024
        );
        let mut cfg = section(&path);
        cfg.set("compaction_base_mb", "64");
        assert_eq!(
            read_table_target_bytes(&cfg).expect("64"),
            16 * 1024 * 1024
        );
        let mut cfg = section(&path);
        cfg.set("compaction_base_mb", "8192");
        assert_eq!(
            read_table_target_bytes(&cfg).expect("8192 is accepted for large windows"),
            2048 * 1024 * 1024
        );
        for bad in ["8", "131072", "abc"] {
            let mut cfg = section(&path);
            cfg.set("compaction_base_mb", bad);
            assert!(
                FjallBackend::new(&cfg, Arc::new(NullJournal)).is_err(),
                "compaction_base_mb = {bad} must be rejected"
            );
        }
    }

    /// Flush eight ~3 MB L0 tables of random values into `nodes`, wait for the
    /// leveled strategy to merge L0 below its threshold, and return the number
    /// of tables outside L0.
    /// (`major_compact` is not used: fjall pins its output size at 64 MB.)
    fn leveled_nodes_table_count(backend: &FjallBackend, seed: u64) -> usize {
        let open = backend.open_db().expect("open");
        // splitmix64 gives well-spread, incompressible keys and values.
        let mut state = seed;
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        for _ in 0..8 {
            for _ in 0..6_000 {
                let bytes: Vec<u8> = (0..(32 + 512) / 8)
                    .flat_map(|_| next().to_le_bytes())
                    .collect();
                open.nodes.insert(&bytes[..32], &bytes[32..]).expect("insert");
            }
            open.nodes.rotate_memtable_and_wait().expect("flush");
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while open.nodes.l0_table_count() >= 4 {
            assert!(std::time::Instant::now() < deadline, "L0 never compacted");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        open.nodes.table_count() - open.nodes.l0_table_count()
    }

    #[test]
    fn compaction_base_mb_sizes_node_tables_and_persists_across_reopen() {
        // Default base (1 GiB tables): the merged L0 fits in one table.
        let default_path = temp_path("base-default");
        let mut cfg = section(&default_path);
        cfg.set("reserve_mb", "0");
        let backend = FjallBackend::new(&cfg, Arc::new(NullJournal)).expect("construct");
        backend.open(true).expect("open");
        let default_tables = leveled_nodes_table_count(&backend, 1);
        assert!(default_tables <= 2, "expected one merged table, got {default_tables}");
        let _ = backend.close();
        let _ = std::fs::remove_dir_all(&default_path);

        // Minimum base (4 MiB tables): the same volume splits into several.
        let small_path = temp_path("base-small");
        let mut cfg = section(&small_path);
        cfg.set("reserve_mb", "0");
        cfg.set("compaction_base_mb", "16");
        let backend = FjallBackend::new(&cfg, Arc::new(NullJournal)).expect("construct");
        backend.open(true).expect("open");
        let small_tables = leveled_nodes_table_count(&backend, 1);
        assert!(small_tables >= 3, "expected split tables, got {small_tables}");
        let _ = backend.close();

        // Reopening without the key keeps the size the store was created with.
        let mut cfg = section(&small_path);
        cfg.set("reserve_mb", "0");
        let backend = FjallBackend::new(&cfg, Arc::new(NullJournal)).expect("construct");
        backend.open(true).expect("reopen");
        let tables = leveled_nodes_table_count(&backend, 2);
        assert!(tables > small_tables, "persisted table size must win, got {tables}");
        let _ = backend.close();
        let _ = std::fs::remove_dir_all(&small_path);
    }

    #[test]
    fn base_to_live_ratio_flags_undersized_pruned_stores() {
        const MB: u64 = 1024 * 1024;
        // Mainnet validator 2026-10-31: base 64 MB, live ~7.5 GB -> undersized.
        assert!(base_undersized(64 * MB, 7_500 * MB));
        // The documented fix: base 4096 MB for that store is healthy.
        assert!(!base_undersized(4096 * MB, 7_500 * MB));
        // Boundary: exactly a quarter of the live data is still healthy.
        assert!(!base_undersized(2000 * MB, 8000 * MB));
        assert!(base_undersized(1999 * MB, 8000 * MB));
        // An empty store is never flagged.
        assert!(!base_undersized(256 * MB, 0));
    }

    #[test]
    fn recommended_base_is_half_live_rounded_to_a_power_of_two() {
        const MB: u64 = 1024 * 1024;
        assert_eq!(recommended_base_mb(7_700 * MB), 4096); // mainnet, window 512
        assert_eq!(recommended_base_mb(11_100 * MB), 8192); // window 2000
        assert_eq!(recommended_base_mb(0), 16); // floor of the accepted range
        assert_eq!(recommended_base_mb(500_000 * MB), 65536); // ceiling
    }

    #[test]
    fn effective_base_is_read_from_the_store_not_the_config() {
        let path = temp_path("base-effective");
        let mut cfg = section(&path);
        cfg.set("reserve_mb", "0");
        cfg.set("compaction_base_mb", "16");
        let backend = FjallBackend::new(&cfg, Arc::new(NullJournal)).expect("construct");
        backend.open(true).expect("open");
        let open = backend.open_db().expect("open db");
        assert_eq!(effective_base_bytes(&open.nodes), Some(16 * 1024 * 1024));
        drop(open);
        let _ = backend.close();

        // Reopen asking for 4096: fjall keeps the persisted 16 MB, and the
        // counters report what the store really uses.
        let mut cfg = section(&path);
        cfg.set("reserve_mb", "0");
        cfg.set("compaction_base_mb", "4096");
        let backend = FjallBackend::new(&cfg, Arc::new(NullJournal)).expect("construct");
        backend.open(true).expect("reopen");
        let counters: std::collections::BTreeMap<_, _> = backend.counters().into_iter().collect();
        assert_eq!(counters.get("fjall_effective_base_mb"), Some(&16));
        assert_eq!(counters.get("fjall_base_undersized"), Some(&0));
        assert!(counters.contains_key("fjall_live_mb"));
        assert!(counters.contains_key("fjall_l0_tables"));
        let _ = backend.close();
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn full_history_store_is_never_flagged_undersized() {
        const MB: u64 = 1024 * 1024;
        // online_delete = 0 never deletes nodes: a 256 MB base over 4 TB of
        // history is correct and must not warn.
        assert!(!sizing_warning_applies(false, 256 * MB, 4_000_000 * MB));
        // The same sizes on a pruned store do warn.
        assert!(sizing_warning_applies(true, 256 * MB, 4_000_000 * MB));
        assert!(!sizing_warning_applies(true, 4096 * MB, 7_500 * MB));
    }

    #[test]
    fn pruned_flag_follows_online_delete() {
        for (window, pruned) in [(None, false), (Some("0"), false), (Some("512"), true)] {
            let path = temp_path("pruned-flag");
            let mut cfg = section(&path);
            if let Some(window) = window {
                cfg.set("online_delete", window);
            }
            let backend = FjallBackend::new(&cfg, Arc::new(NullJournal)).expect("construct");
            assert_eq!(backend.pruned, pruned, "online_delete = {window:?}");
        }
    }

    #[test]
    fn disk_full_ballast_is_reserved_and_releasable() {
        let path = temp_path("reserve");
        let mut cfg = section(&path);
        // Keep the test cheap: a 1 MiB ballast.
        cfg.set("reserve_mb", "1");
        let backend = FjallBackend::new(&cfg, Arc::new(NullJournal)).expect("construct");
        backend.open(true).expect("open");

        // The ballast exists at the configured size after open.
        assert!(
            backend.reserve_present(),
            "ballast should be reserved at open"
        );
        let ballast = std::path::Path::new(&path).join(".reserve_ballast");
        assert_eq!(
            std::fs::metadata(&ballast).expect("ballast metadata").len(),
            1024 * 1024
        );

        // Releasing it frees the space and is idempotent.
        assert!(
            backend.release_reserve(),
            "first release removes the ballast"
        );
        assert!(!backend.reserve_present());
        assert!(!backend.release_reserve(), "second release is a no-op");

        let _ = backend.close();
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn ballast_is_released_when_free_space_drops_and_recreated_with_hysteresis() {
        use super::{BallastAction, ballast_action};
        let gb = 1u64 << 30;
        assert_eq!(ballast_action(true, 5 * gb, gb), BallastAction::Keep);
        assert_eq!(ballast_action(true, gb - 1, gb), BallastAction::Release);
        // Just released: 1-3 GB free is not enough to take it back.
        assert_eq!(ballast_action(false, gb, gb), BallastAction::Keep);
        assert_eq!(ballast_action(false, 3 * gb - 1, gb), BallastAction::Keep);
        assert_eq!(ballast_action(false, 3 * gb, gb), BallastAction::Recreate);
    }

    /// A ballast left by an older build (a sparse `set_len` file) is replaced
    /// by a really allocated one at open.
    #[cfg(unix)]
    #[test]
    fn sparse_ballast_from_an_older_build_is_replaced_at_open() {
        let path = temp_path("reserve-sparse");
        std::fs::create_dir_all(&path).expect("dir");
        let ballast = std::path::Path::new(&path).join(".reserve_ballast");
        std::fs::File::create(&ballast)
            .expect("create")
            .set_len(1024 * 1024)
            .expect("set_len");
        let mut cfg = section(&path);
        cfg.set("reserve_mb", "1");
        let backend = FjallBackend::new(&cfg, Arc::new(NullJournal)).expect("construct");
        backend.open(true).expect("open");
        assert!(backend.reserve_present(), "allocated ballast after open");
        assert!(
            super::super::ballast::allocated_bytes(&ballast).is_some_and(|b| b >= 1024 * 1024),
            "ballast blocks are really allocated"
        );
        let _ = backend.close();
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn reserve_mb_zero_disables_the_ballast() {
        let path = temp_path("reserve-off");
        let mut cfg = section(&path);
        cfg.set("reserve_mb", "0");
        let backend = FjallBackend::new(&cfg, Arc::new(NullJournal)).expect("construct");
        backend.open(true).expect("open");
        assert!(
            !backend.reserve_present(),
            "zero reserve creates no ballast"
        );
        assert!(
            !std::path::Path::new(&path)
                .join(".reserve_ballast")
                .exists(),
            "no ballast file should exist"
        );
        let _ = backend.close();
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn fjall_backend_round_trips_node_objects() {
        let path = temp_path("nodes");
        {
            let backend = open_backend(&path);
            let object = NodeObject::create_object(
                NodeObjectType::Ledger,
                vec![1, 2, 3, 4],
                Uint256::from_array([7; 32]),
            );
            backend.store(Arc::clone(&object)).expect("store");
            backend.sync_result().expect("sync");
            let (fetched, status) = backend.fetch(object.hash());
            assert_eq!(status, crate::Status::Ok);
            let fetched = fetched.expect("object present");
            assert_eq!(fetched.data(), object.data());
            assert_eq!(fetched.hash(), object.hash());
        }
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn fjall_backend_persists_nodes_across_reopen() {
        let path = temp_path("reopen");
        let hash = Uint256::from_array([9; 32]);
        {
            let backend = open_backend(&path);
            backend
                .store(NodeObject::create_object(
                    NodeObjectType::Ledger,
                    vec![5, 6, 7],
                    hash,
                ))
                .expect("store");
            backend.close().expect("close");
        }
        {
            let backend = FjallFactory::new()
                .create_instance(
                    NodeObject::KEY_BYTES,
                    &section(&path),
                    0,
                    Arc::new(crate::DummyScheduler),
                    Arc::new(NullJournal),
                )
                .expect("construct");
            backend.open(false).expect("reopen existing");
            let (fetched, status) = backend.fetch(&hash);
            assert_eq!(status, crate::Status::Ok);
            assert_eq!(fetched.expect("persisted node").data(), &[5, 6, 7]);
        }
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn fjall_backend_satisfies_kv_conformance() {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let base = temp_path("conformance");
        let open = || {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            open_backend(&format!("{base}-{n}"))
        };
        crate::backend_conformance!(open());
        // Best-effort cleanup of the per-iteration directories.
        for n in 0..COUNTER.load(Ordering::SeqCst) {
            let _ = std::fs::remove_dir_all(format!("{base}-{n}"));
        }
    }
}
