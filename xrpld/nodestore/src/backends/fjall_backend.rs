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
    journal: Arc<dyn NodeStoreJournal>,
    db: Mutex<Option<Arc<OpenDb>>>,
    // When set (via set_delete_path, used by rotation to retire an archive),
    // close removes the database directory from disk after dropping it.
    delete_on_close: std::sync::atomic::AtomicBool,
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
            journal,
            db: Mutex::new(None),
            delete_on_close: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Absolute path of the disk-full ballast file for this store.
    fn reserve_path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.path).join(".reserve_ballast")
    }

    /// Create the ballast file if a reserve is configured and it is absent.
    /// Best effort: a filesystem that cannot allocate it (e.g. already full)
    /// is logged, not fatal, so the node still opens and can prune.
    fn ensure_reserve(&self) {
        if self.reserve_bytes == 0 {
            return;
        }
        let path = self.reserve_path();
        let present = std::fs::metadata(&path)
            .map(|m| m.len() >= self.reserve_bytes)
            .unwrap_or(false);
        if present {
            return;
        }
        match std::fs::File::create(&path).and_then(|file| {
            file.set_len(self.reserve_bytes)?;
            Ok(())
        }) {
            Ok(()) => tracing::info!(
                target: "nodestore",
                path = %path.display(),
                reserve_bytes = self.reserve_bytes,
                "fjall disk-full ballast reserved"
            ),
            Err(error) => tracing::warn!(
                target: "nodestore",
                path = %path.display(),
                %error,
                "could not create fjall disk-full ballast (continuing without reserve)"
            ),
        }
    }

    /// Release the ballast so freed space is available for compaction and
    /// deletes under disk pressure (design Case 10). Returns true if a ballast
    /// file was removed. Idempotent.
    pub fn release_reserve(&self) -> bool {
        let path = self.reserve_path();
        match std::fs::remove_file(&path) {
            Ok(()) => {
                tracing::warn!(
                    target: "nodestore",
                    path = %path.display(),
                    "released fjall disk-full ballast under space pressure"
                );
                true
            }
            Err(_) => false,
        }
    }

    /// Whether the ballast file is currently present at its configured size.
    pub fn reserve_present(&self) -> bool {
        if self.reserve_bytes == 0 {
            return false;
        }
        std::fs::metadata(self.reserve_path())
            .map(|m| m.len() >= self.reserve_bytes)
            .unwrap_or(false)
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
            .open()
            .map_err(|error| Self::fjall_err("open", error))?;
        // Each keyspace is its own LSM-tree. Defaults keep bloom filters and
        // LZ4 on; the index layer tunes per-keyspace policy later if needed.
        let mut handles = Vec::with_capacity(Keyspace::ALL.len());
        for keyspace in Keyspace::ALL {
            let handle = db
                .keyspace(keyspace.name(), KeyspaceCreateOptions::default)
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
        *guard = Some(Arc::new(open));
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
        match open.nodes.get(hash.as_slice()) {
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
        open.nodes
            .insert(object.hash().as_slice(), encoded.get_data())
            .map_err(|error| Self::fjall_err("store", error))
    }

    fn store_batch(&self, batch: &crate::Batch) {
        if let Err(error) = self.store_batch_result(batch) {
            self.journal.log(crate::JournalLevel::Error, &error);
        }
    }

    fn store_batch_result(&self, batch: &crate::Batch) -> Result<(), String> {
        let open = self.open_db()?;
        let mut write = open.db.batch();
        for object in batch {
            let encoded = EncodedBlob::new(object);
            write.insert(&open.nodes, object.hash().as_slice(), encoded.get_data());
        }
        write
            .commit()
            .map_err(|error| Self::fjall_err("store_batch", error))
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
    use super::{FjallBackend, FjallFactory};
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
