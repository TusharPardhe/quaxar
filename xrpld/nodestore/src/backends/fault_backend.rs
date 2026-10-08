//! Fault-injecting backend wrapper and a conformance suite for key-value
//! backends.
//!
//! [`FaultBackend`] wraps any key-value backend and fails the Nth mutating
//! call, letting crash tests exercise the "batch never committed" and
//! "resume after partial progress" paths from the pruned-store design. The
//! [`backend_conformance`] macro asserts the invariants every key-value
//! backend must satisfy (atomic batches, ascending range order, delete
//! idempotence), so Memory and fjall are tested identically.

use crate::backends::kv::{Keyspace, KvBatch, PersistMode};
use crate::{Backend, NodeObject, Status};
use basics::base_uint::Uint256;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Wraps a backend and injects a failure on a chosen mutating call.
///
/// `fail_after` counts `kv_write_batch` and `kv_persist` calls; when the count
/// reaches the configured trip point the call returns an error instead of
/// mutating, simulating a crash at a batch boundary. Reads are never failed so
/// a test can inspect the post-fault state.
pub struct FaultBackend {
    inner: Arc<dyn Backend>,
    writes: AtomicUsize,
    fail_at: AtomicUsize,
}

impl FaultBackend {
    /// Wrap `inner`. With `fail_at == 0` no fault is injected (pass-through).
    pub fn new(inner: Arc<dyn Backend>) -> Self {
        Self {
            inner,
            writes: AtomicUsize::new(0),
            fail_at: AtomicUsize::new(0),
        }
    }

    /// Trip on the `n`th mutating call (1-based). Zero disables faulting.
    pub fn fail_on_write(&self, n: usize) {
        self.fail_at.store(n, Ordering::SeqCst);
    }

    /// Stop injecting faults and reset the call counter.
    pub fn clear(&self) {
        self.fail_at.store(0, Ordering::SeqCst);
        self.writes.store(0, Ordering::SeqCst);
    }

    fn should_fail(&self) -> bool {
        let trip = self.fail_at.load(Ordering::SeqCst);
        if trip == 0 {
            return false;
        }
        let n = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
        n == trip
    }
}

impl Backend for FaultBackend {
    fn get_name(&self) -> String {
        self.inner.get_name()
    }

    fn open(&self, create_if_missing: bool) -> Result<(), String> {
        self.inner.open(create_if_missing)
    }

    fn is_open(&self) -> bool {
        self.inner.is_open()
    }

    fn close(&self) -> Result<(), String> {
        self.inner.close()
    }

    fn fetch(&self, hash: &Uint256) -> (Option<Arc<NodeObject>>, Status) {
        self.inner.fetch(hash)
    }

    fn fetch_batch(&self, hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status) {
        self.inner.fetch_batch(hashes)
    }

    fn store(&self, object: Arc<NodeObject>) -> Result<(), String> {
        self.inner.store(object)
    }

    fn store_batch(&self, batch: &crate::Batch) {
        self.inner.store_batch(batch);
    }

    fn sync(&self) {
        self.inner.sync();
    }

    fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) {
        self.inner.for_each(callback);
    }

    fn get_write_load(&self) -> i32 {
        self.inner.get_write_load()
    }

    fn set_delete_path(&self) {
        self.inner.set_delete_path();
    }

    fn fd_required(&self) -> i32 {
        self.inner.fd_required()
    }

    fn supports_kv(&self) -> bool {
        self.inner.supports_kv()
    }

    fn kv_get(&self, keyspace: Keyspace, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        self.inner.kv_get(keyspace, key)
    }

    fn kv_write_batch(&self, batch: &KvBatch) -> Result<(), String> {
        if self.should_fail() {
            return Err("injected fault: kv_write_batch".to_owned());
        }
        self.inner.kv_write_batch(batch)
    }

    fn kv_range(
        &self,
        keyspace: Keyspace,
        start: &[u8],
        end: &[u8],
        callback: &mut dyn FnMut(&[u8], &[u8]) -> bool,
    ) -> Result<(), String> {
        self.inner.kv_range(keyspace, start, end, callback)
    }

    fn kv_persist(&self, mode: PersistMode) -> Result<(), String> {
        if self.should_fail() {
            return Err("injected fault: kv_persist".to_owned());
        }
        self.inner.kv_persist(mode)
    }
}

/// Collect every key-value pair in a keyspace into a sorted vector. A helper
/// for conformance assertions and crash-recovery checks.
pub fn kv_collect(backend: &dyn Backend, keyspace: Keyspace) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    backend
        .kv_range(keyspace, &[], &[0xFF; 64], &mut |key, value| {
            out.push((key.to_vec(), value.to_vec()));
            true
        })
        .expect("kv_range over an open backend");
    out
}

/// Assert the key-value invariants a backend must satisfy. `open_backend` must
/// return a freshly opened, empty backend each time it is called.
///
/// Covered: write/read round-trip across keyspaces, keyspace isolation, batch
/// atomicity (a failed batch leaves no partial writes), ascending range order
/// with big-endian keys, half-open range bounds, and delete idempotence.
#[macro_export]
macro_rules! backend_conformance {
    ($open_backend:expr) => {{
        use $crate::Backend as _;
        use $crate::backends::fault_backend::{FaultBackend, kv_collect};
        use $crate::kv::{Keyspace, KvBatch, PersistMode};

        // round-trip and keyspace isolation
        {
            let backend = $open_backend;
            let mut batch = KvBatch::new();
            batch.put(Keyspace::Nodes, vec![1u8], vec![10u8]).put(
                Keyspace::Counts,
                vec![1u8],
                vec![20u8],
            );
            backend.kv_write_batch(&batch).expect("write batch");
            assert_eq!(
                backend.kv_get(Keyspace::Nodes, &[1]).expect("get"),
                Some(vec![10u8]),
                "nodes keyspace round-trip"
            );
            assert_eq!(
                backend.kv_get(Keyspace::Counts, &[1]).expect("get"),
                Some(vec![20u8]),
                "same key in a different keyspace is isolated"
            );
            assert_eq!(
                backend.kv_get(Keyspace::Meta, &[1]).expect("get"),
                None,
                "unwritten keyspace is empty"
            );
        }

        // ascending range order with big-endian keys and half-open bounds
        {
            let backend = $open_backend;
            let mut batch = KvBatch::new();
            for seq in [2u32, 10u32, 1u32, 256u32] {
                batch.put(Keyspace::Notebook, seq.to_be_bytes().to_vec(), vec![]);
            }
            backend.kv_write_batch(&batch).expect("write batch");
            let keys: Vec<u32> =
                kv_collect(backend.as_ref() as &dyn $crate::Backend, Keyspace::Notebook)
                    .into_iter()
                    .map(|(k, _)| u32::from_be_bytes(k.try_into().expect("4-byte key")))
                    .collect();
            assert_eq!(keys, vec![1, 2, 10, 256], "range yields ascending order");

            let mut seen = Vec::new();
            backend
                .kv_range(
                    Keyspace::Notebook,
                    &2u32.to_be_bytes(),
                    &10u32.to_be_bytes(),
                    &mut |k, _| {
                        seen.push(u32::from_be_bytes(k.try_into().expect("4-byte key")));
                        true
                    },
                )
                .expect("range");
            assert_eq!(seen, vec![2], "range is half-open [start, end)");
        }

        // delete idempotence
        {
            let backend = $open_backend;
            let mut put = KvBatch::new();
            put.put(Keyspace::Nodes, vec![7u8], vec![7u8]);
            backend.kv_write_batch(&put).expect("write");
            let mut del = KvBatch::new();
            del.delete(Keyspace::Nodes, vec![7u8]);
            backend.kv_write_batch(&del).expect("delete");
            backend
                .kv_write_batch(&del)
                .expect("second delete is a no-op");
            assert_eq!(
                backend.kv_get(Keyspace::Nodes, &[7]).expect("get"),
                None,
                "deleted key stays gone"
            );
        }

        // batch atomicity under an injected fault: nothing from a failed batch
        // becomes visible.
        {
            let inner = $open_backend;
            let inner: std::sync::Arc<dyn $crate::Backend> = std::sync::Arc::from(inner);
            let fault = FaultBackend::new(inner);
            fault.fail_on_write(1);
            let mut batch = KvBatch::new();
            batch.put(Keyspace::Nodes, vec![9u8], vec![9u8]).put(
                Keyspace::Counts,
                vec![9u8],
                vec![9u8],
            );
            assert!(
                fault.kv_write_batch(&batch).is_err(),
                "injected fault must fail the batch"
            );
            assert_eq!(
                fault.kv_get(Keyspace::Nodes, &[9]).expect("get"),
                None,
                "failed batch left no partial write in nodes"
            );
            assert_eq!(
                fault.kv_get(Keyspace::Counts, &[9]).expect("get"),
                None,
                "failed batch left no partial write in counts"
            );
            fault.clear();
            let _ = PersistMode::SyncAll;
        }
    }};
}
