use crate::backends::kv::{Keyspace, KvBatch, PersistMode};
use crate::{Batch, NodeObject, Status};
use basics::base_uint::Uint256;
use std::sync::Arc;

pub trait Backend: Send + Sync + 'static {
    fn get_name(&self) -> String;

    fn get_block_size(&self) -> Option<usize> {
        None
    }

    fn open(&self, create_if_missing: bool) -> Result<(), String>;

    fn open_deterministic(
        &self,
        _create_if_missing: bool,
        _app_type: u64,
        _uid: u64,
        _salt: u64,
    ) -> Result<(), String> {
        Err(format!(
            "Deterministic appType/uid/salt not supported by backend {}",
            self.get_name()
        ))
    }

    fn is_open(&self) -> bool;

    fn close(&self) -> Result<(), String>;

    fn fetch(&self, hash: &Uint256) -> (Option<Arc<NodeObject>>, Status);

    fn fetch_batch(&self, hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status);

    /// Stores one object, reporting backend failures to the caller so it does
    /// not treat an unpersisted object as cacheable.
    fn store(&self, object: Arc<NodeObject>) -> Result<(), String>;

    fn store_batch(&self, batch: &Batch);

    /// Write a complete batch and report backend failures to the caller.
    ///
    /// `store_batch` remains for asynchronous legacy paths that cannot return
    /// a result. Snapshot and import paths must use this checked form so they
    /// never report a successful import after a failed durable write.
    fn store_batch_result(&self, batch: &Batch) -> Result<(), String> {
        for object in batch {
            self.store(Arc::clone(object))?;
        }
        Ok(())
    }

    fn sync(&self);

    /// Checked durability barrier. Backends without a fallible checkpoint keep
    /// the historical no-op/default behavior; durable backends override this
    /// to expose commit and fsync failures to lifecycle owners.
    fn sync_result(&self) -> Result<(), String> {
        self.sync();
        Ok(())
    }

    /// Begin bulk import mode. Optimized for loading millions of nodes sequentially.
    /// Skips existence checks, disables burst checkpoints, pre-allocates structures.
    fn bulk_import_start(&self, _estimated_nodes: u64) -> Result<(), String> {
        Ok(())
    }

    /// Finish bulk import. Flushes all data to disk, builds indexes.
    fn bulk_import_finish(&self) -> Result<(), String> {
        Ok(())
    }

    /// Mark an import as failed after it has started. Backends with an
    /// incomplete-import marker must keep or restore that marker so an invalid
    /// post-finalization snapshot cannot be reopened as a successful import.
    fn bulk_import_abort(&self) {}

    fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>));

    /// Traverse all objects, reporting backend traversal failures to callers
    /// that need a complete view. The default preserves historical behavior
    /// for existing backends that only implement `for_each`.
    fn for_each_result(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) -> Result<(), String> {
        self.for_each(callback);
        Ok(())
    }

    fn get_write_load(&self) -> i32;

    fn set_delete_path(&self);

    fn verify(&self) {}

    fn fd_required(&self) -> i32;

    // --- Key-value v2 surface -------------------------------------------
    //
    // The pruned node store needs per-key deletes, several keyspaces, atomic
    // cross-keyspace writes, ordered range scans, and an explicit durability
    // barrier. Backends without that surface (the null backend) keep the
    // defaults, which report that the backend is not key-value capable; MemoryBackend and the fjall
    // backend override them. `supports_kv` lets callers select a path without
    // probing for errors.

    /// Whether this backend implements the key-value v2 methods below.
    fn supports_kv(&self) -> bool {
        false
    }

    /// Read one key from a keyspace. `Ok(None)` is a definite miss.
    fn kv_get(&self, _keyspace: Keyspace, _key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        Err(self.kv_unsupported())
    }

    /// Apply a batch atomically across keyspaces: all ops land or none do.
    fn kv_write_batch(&self, _batch: &KvBatch) -> Result<(), String> {
        Err(self.kv_unsupported())
    }

    /// Scan a keyspace over `[start, end)` in ascending key order, invoking the
    /// callback for each pair. The callback returns `false` to stop early.
    fn kv_range(
        &self,
        _keyspace: Keyspace,
        _start: &[u8],
        _end: &[u8],
        _callback: &mut dyn FnMut(&[u8], &[u8]) -> bool,
    ) -> Result<(), String> {
        Err(self.kv_unsupported())
    }

    /// Flush the backend journal to the requested durability level.
    fn kv_persist(&self, _mode: PersistMode) -> Result<(), String> {
        Err(self.kv_unsupported())
    }

    #[doc(hidden)]
    fn kv_unsupported(&self) -> String {
        format!("backend {} is not key-value capable", self.get_name())
    }
}
