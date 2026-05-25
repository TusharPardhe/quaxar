use crate::{
    Backend, FetchReport, FetchType, JournalLevel, NodeObject, NodeObjectType, NodeStoreJournal,
    Scheduler, batch_write_preallocation_size,
};
use basics::base_uint::Uint256;
use basics::basic_config::{Section, get};
use basics::blob::Blob;
use protocol::JsonValue;
use std::any::Any;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const XRP_LEDGER_EARLIEST_SEQ: u32 = 32_570;

pub type AsyncFetchCallback = Box<dyn FnOnce(Option<Arc<NodeObject>>) + Send + 'static>;

pub trait DatabaseSource: Send + Sync + 'static {
    fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>));
}

pub trait DatabaseImporter: Send + Sync + 'static {
    fn import_database(&self, source: &dyn DatabaseSource);
}

/// reference-style owner surface shared by the concrete Database wrappers.
pub trait Database: DatabaseSource + DatabaseImporter + Send + Sync + 'static {
    fn get_name(&self) -> String;

    fn get_write_load(&self) -> i32;

    fn store(&self, object_type: NodeObjectType, data: Blob, hash: Uint256, ledger_seq: u32);

    fn is_same_db(&self, first: u32, second: u32) -> bool;

    fn sync(&self);

    fn fetch_node_object(
        &self,
        hash: &Uint256,
        ledger_seq: u32,
        fetch_type: FetchType,
        duplicate: bool,
    ) -> Option<Arc<NodeObject>>;

    fn async_fetch(&self, hash: Uint256, ledger_seq: u32, callback: AsyncFetchCallback);

    fn stop(&self);

    fn is_stopping(&self) -> bool;

    fn earliest_ledger_seq(&self) -> u32;

    fn get_store_count(&self) -> u64;

    fn get_fetch_total_count(&self) -> u64;

    fn get_fetch_hit_count(&self) -> u64;

    fn get_store_size(&self) -> u64;

    fn get_fetch_size(&self) -> u64;

    fn add_counts_json(&self, obj: &mut BTreeMap<String, JsonValue>);

    fn get_counts_json(&self) -> JsonValue;

    fn fd_required(&self) -> i32;
}

/// reference-style rotating owner extension.
pub trait DatabaseRotating: Database {
    fn rotate(&self, new_backend: Box<dyn Backend>, callback: &mut dyn FnMut(&str, &str));
}

/// Backward-compatible alias while older Rust modules migrate to the direct
/// `Database` trait name.
pub trait DatabaseSurface: Database {}

impl<T> DatabaseSurface for T where T: Database + ?Sized {}

pub trait DatabaseDelegate: Send + Sync + 'static {
    fn is_same_db(&self, first: u32, second: u32) -> bool;

    fn fetch_node_object(
        &self,
        hash: &Uint256,
        ledger_seq: u32,
        fetch_report: &mut FetchReport,
        duplicate: bool,
        journal: &dyn NodeStoreJournal,
    ) -> Option<Arc<NodeObject>>;
}

struct AsyncReadRequest {
    ledger_seq: u32,
    callback: AsyncFetchCallback,
}

#[derive(Default)]
struct ReadState {
    queue: BTreeMap<Uint256, Vec<AsyncReadRequest>>,
}

struct DatabaseInner {
    delegate: Arc<dyn DatabaseDelegate>,
    scheduler: Arc<dyn Scheduler>,
    journal: Arc<dyn NodeStoreJournal>,
    earliest_ledger_seq: u32,
    request_bundle: usize,
    read_state: Mutex<ReadState>,
    read_condvar: Condvar,
    read_stopping: AtomicBool,
    live_threads: AtomicUsize,
    running_threads: AtomicUsize,
    store_count: AtomicU64,
    store_size: AtomicU64,
    fetch_total_count: AtomicU64,
    fetch_hit_count: AtomicU64,
    fetch_size: AtomicU64,
    fetch_duration_us: AtomicU64,
    store_duration_us: AtomicU64,
}

struct WorkerExitGuard {
    inner: Arc<DatabaseInner>,
}

impl WorkerExitGuard {
    fn new(inner: &Arc<DatabaseInner>) -> Self {
        Self {
            inner: Arc::clone(inner),
        }
    }
}

impl Drop for WorkerExitGuard {
    fn drop(&mut self) {
        self.inner.live_threads.fetch_sub(1, Ordering::Relaxed);
    }
}

impl DatabaseInner {
    fn is_stopping(&self) -> bool {
        self.read_stopping.load(Ordering::Relaxed)
    }

    fn store_stats(&self, count: u64, size: u64) {
        assert!(
            count <= size,
            "xrpl::NodeStore::Database::store_stats : valid inputs"
        );
        self.store_count.fetch_add(count, Ordering::Relaxed);
        self.store_size.fetch_add(size, Ordering::Relaxed);
    }

    fn update_fetch_metrics(&self, fetches: u64, hits: u64, duration_us: u64) {
        self.fetch_total_count.fetch_add(fetches, Ordering::Relaxed);
        self.fetch_hit_count.fetch_add(hits, Ordering::Relaxed);
        self.fetch_duration_us
            .fetch_add(duration_us, Ordering::Relaxed);
    }

    fn fetch_node_object(
        &self,
        hash: &Uint256,
        ledger_seq: u32,
        fetch_type: FetchType,
        duplicate: bool,
    ) -> Option<Arc<NodeObject>> {
        let mut fetch_report = FetchReport::new(fetch_type);
        let begin = Instant::now();
        let node_object = self.delegate.fetch_node_object(
            hash,
            ledger_seq,
            &mut fetch_report,
            duplicate,
            self.journal.as_ref(),
        );
        let elapsed = begin.elapsed();
        let elapsed_us = elapsed.as_micros() as u64;

        self.fetch_duration_us
            .fetch_add(elapsed_us, Ordering::Relaxed);
        if let Some(node_object) = &node_object {
            self.fetch_hit_count.fetch_add(1, Ordering::Relaxed);
            self.fetch_size
                .fetch_add(node_object.data().len() as u64, Ordering::Relaxed);
            fetch_report.was_found = true;
        }
        self.fetch_total_count.fetch_add(1, Ordering::Relaxed);
        fetch_report.elapsed = Duration::from_millis(elapsed.as_millis() as u64);
        self.scheduler.on_fetch(fetch_report);
        node_object
    }
}

/// Shared async-read runtime used by the concrete public database owners.
pub struct DatabaseRuntime {
    inner: Arc<DatabaseInner>,
    read_handles: Mutex<Vec<JoinHandle<()>>>,
}

impl DatabaseRuntime {
    pub fn new(
        delegate: Arc<dyn DatabaseDelegate>,
        scheduler: Arc<dyn Scheduler>,
        read_threads: usize,
        config: &Section,
        journal: Arc<dyn NodeStoreJournal>,
    ) -> Result<Self, String> {
        assert!(
            read_threads != 0,
            "xrpl::NodeStore::Database::new : nonzero threads input"
        );

        let earliest_ledger_seq = get(config, "earliest_seq", XRP_LEDGER_EARLIEST_SEQ);
        if earliest_ledger_seq < 1 {
            return Err("Invalid earliest_seq".to_owned());
        }

        let request_bundle = get(config, "rq_bundle", 4i32);
        if !(1..=64).contains(&request_bundle) {
            return Err("Invalid rq_bundle".to_owned());
        }

        let inner = Arc::new(DatabaseInner {
            delegate,
            scheduler,
            journal,
            earliest_ledger_seq,
            request_bundle: request_bundle as usize,
            read_state: Mutex::new(ReadState::default()),
            read_condvar: Condvar::new(),
            read_stopping: AtomicBool::new(false),
            live_threads: AtomicUsize::new(read_threads.max(1)),
            running_threads: AtomicUsize::new(0),
            store_count: AtomicU64::new(0),
            store_size: AtomicU64::new(0),
            fetch_total_count: AtomicU64::new(0),
            fetch_hit_count: AtomicU64::new(0),
            fetch_size: AtomicU64::new(0),
            fetch_duration_us: AtomicU64::new(0),
            store_duration_us: AtomicU64::new(0),
        });

        let mut read_handles: Vec<JoinHandle<()>> = Vec::with_capacity(read_threads.max(1));
        for index in 1..=read_threads.max(1) {
            let worker_inner = inner.clone();
            let handle = match thread::Builder::new()
                .name(format!("db prefetch #{index}"))
                .spawn(move || worker_loop(worker_inner))
            {
                Ok(handle) => handle,
                Err(error) => {
                    inner.read_stopping.store(true, Ordering::Relaxed);
                    inner.read_condvar.notify_all();
                    for handle in read_handles.drain(..) {
                        let _ = handle.join();
                    }
                    return Err(format!("failed to spawn NodeStore read thread: {error}"));
                }
            };
            read_handles.push(handle);
        }

        Ok(Self {
            inner,
            read_handles: Mutex::new(read_handles),
        })
    }

    pub fn fetch_node_object(
        &self,
        hash: &Uint256,
        ledger_seq: u32,
        fetch_type: FetchType,
        duplicate: bool,
    ) -> Option<Arc<NodeObject>> {
        self.inner
            .fetch_node_object(hash, ledger_seq, fetch_type, duplicate)
    }

    pub fn async_fetch(&self, hash: Uint256, ledger_seq: u32, callback: AsyncFetchCallback) {
        let mut read_state = self
            .inner
            .read_state
            .lock()
            .expect("nodestore read queue mutex must not be poisoned");
        if !self.inner.is_stopping() {
            read_state
                .queue
                .entry(hash)
                .or_default()
                .push(AsyncReadRequest {
                    ledger_seq,
                    callback,
                });
            self.inner.read_condvar.notify_one();
        }
    }

    pub fn import_internal(&self, dst_backend: &dyn Backend, src_db: &dyn DatabaseSource) {
        let mut batch = Vec::with_capacity(batch_write_preallocation_size);
        let store_batch = |batch: &mut Vec<Arc<NodeObject>>| {
            let begin = Instant::now();
            let result = catch_unwind(AssertUnwindSafe(|| dst_backend.store_batch(batch)));
            match result {
                Ok(()) => {
                    let size = batch
                        .iter()
                        .map(|node_object| node_object.data().len() as u64)
                        .sum();
                    self.inner.store_stats(batch.len() as u64, size);
                    self.inner
                        .store_duration_us
                        .fetch_add(begin.elapsed().as_micros() as u64, Ordering::Relaxed);
                    batch.clear();
                }
                Err(payload) => {
                    // Mirror reference importInternal: keep the batch intact when the
                    // destination write fails so a later retry still sees the
                    // full accumulated batch.
                    self.inner.journal.log(
                        JournalLevel::Error,
                        &format!(
                            "Exception caught in function import_internal. Error: {}",
                            panic_message(payload.as_ref())
                        ),
                    );
                }
            }
        };

        src_db.for_each(&mut |node_object| {
            batch.push(node_object);
            if batch.len() >= batch_write_preallocation_size {
                store_batch(&mut batch);
            }
        });

        if !batch.is_empty() {
            store_batch(&mut batch);
        }
    }

    pub fn stop(&self) {
        let first_stop = {
            let mut read_state = self
                .inner
                .read_state
                .lock()
                .expect("nodestore read queue mutex must not be poisoned");
            let first_stop = !self.inner.read_stopping.swap(true, Ordering::Relaxed);
            if first_stop {
                self.inner.journal.log(
                    JournalLevel::Debug,
                    "Clearing read queue because of stop request",
                );
                read_state.queue.clear();
            }
            first_stop
        };

        if first_stop {
            self.inner.read_condvar.notify_all();
        }

        self.inner.journal.log(
            JournalLevel::Debug,
            "Waiting for stop request to complete...",
        );

        let start = Instant::now();
        while self.inner.live_threads.load(Ordering::Acquire) != 0 {
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "xrpl::NodeStore::Database::stop : maximum stop duration"
            );
            thread::yield_now();
        }

        let mut handles = self
            .read_handles
            .lock()
            .expect("nodestore thread handle mutex must not be poisoned");
        for handle in handles.drain(..) {
            let _ = handle.join();
        }

        self.inner.journal.log(
            JournalLevel::Debug,
            &format!(
                "Stop request completed in {} milliseconds",
                start.elapsed().as_millis()
            ),
        );
    }

    pub fn is_stopping(&self) -> bool {
        self.inner.is_stopping()
    }

    pub fn earliest_ledger_seq(&self) -> u32 {
        self.inner.earliest_ledger_seq
    }

    pub fn get_store_count(&self) -> u64 {
        self.inner.store_count.load(Ordering::Relaxed)
    }

    pub fn get_fetch_total_count(&self) -> u64 {
        self.inner.fetch_total_count.load(Ordering::Relaxed)
    }

    pub fn get_fetch_hit_count(&self) -> u64 {
        self.inner.fetch_hit_count.load(Ordering::Relaxed)
    }

    pub fn get_store_size(&self) -> u64 {
        self.inner.store_size.load(Ordering::Relaxed)
    }

    pub fn get_fetch_size(&self) -> u64 {
        self.inner.fetch_size.load(Ordering::Relaxed)
    }

    pub fn add_counts_json(&self, obj: &mut BTreeMap<String, JsonValue>) {
        let read_queue = self
            .inner
            .read_state
            .lock()
            .expect("nodestore read queue mutex must not be poisoned")
            .queue
            .len() as u64;

        obj.insert("read_queue".to_owned(), JsonValue::Unsigned(read_queue));
        obj.insert(
            "read_threads_total".to_owned(),
            JsonValue::Signed(self.inner.live_threads.load(Ordering::Relaxed) as i64),
        );
        obj.insert(
            "read_threads_running".to_owned(),
            JsonValue::Signed(self.inner.running_threads.load(Ordering::Relaxed) as i64),
        );
        obj.insert(
            "read_request_bundle".to_owned(),
            JsonValue::Signed(self.inner.request_bundle as i64),
        );
        obj.insert(
            "node_writes".to_owned(),
            JsonValue::String(self.get_store_count().to_string()),
        );
        obj.insert(
            "node_reads_total".to_owned(),
            JsonValue::String(self.get_fetch_total_count().to_string()),
        );
        obj.insert(
            "node_reads_hit".to_owned(),
            JsonValue::String(self.get_fetch_hit_count().to_string()),
        );
        obj.insert(
            "node_written_bytes".to_owned(),
            JsonValue::String(self.get_store_size().to_string()),
        );
        obj.insert(
            "node_read_bytes".to_owned(),
            JsonValue::String(self.get_fetch_size().to_string()),
        );
        obj.insert(
            "node_reads_duration_us".to_owned(),
            JsonValue::String(
                self.inner
                    .fetch_duration_us
                    .load(Ordering::Relaxed)
                    .to_string(),
            ),
        );
    }

    pub fn get_counts_json(&self) -> JsonValue {
        let mut obj = BTreeMap::new();
        self.add_counts_json(&mut obj);
        JsonValue::Object(obj)
    }

    pub(crate) fn store_stats(&self, count: u64, size: u64) {
        self.inner.store_stats(count, size);
    }

    pub(crate) fn update_fetch_metrics(&self, fetches: u64, hits: u64, duration_us: u64) {
        self.inner.update_fetch_metrics(fetches, hits, duration_us);
    }

    pub fn journal(&self) -> Arc<dyn NodeStoreJournal> {
        Arc::clone(&self.inner.journal)
    }
}

impl Drop for DatabaseRuntime {
    fn drop(&mut self) {
        self.stop();
    }
}

fn worker_loop(inner: Arc<DatabaseInner>) {
    let _exit_guard = WorkerExitGuard::new(&inner);
    inner.running_threads.fetch_add(1, Ordering::Relaxed);
    let mut read = BTreeMap::<Uint256, Vec<AsyncReadRequest>>::new();

    loop {
        {
            let mut read_state = inner
                .read_state
                .lock()
                .expect("nodestore read queue mutex must not be poisoned");

            if inner.is_stopping() {
                break;
            }

            if read_state.queue.is_empty() {
                inner.running_threads.fetch_sub(1, Ordering::Relaxed);
                while read_state.queue.is_empty() && !inner.is_stopping() {
                    read_state = inner
                        .read_condvar
                        .wait(read_state)
                        .expect("nodestore read queue mutex must not be poisoned");
                }
                inner.running_threads.fetch_add(1, Ordering::Relaxed);
            }

            if inner.is_stopping() {
                break;
            }

            for _ in 0..inner.request_bundle {
                let Some((hash, requests)) = read_state.queue.pop_first() else {
                    break;
                };
                read.insert(hash, requests);
            }
        }

        for (hash, requests) in std::mem::take(&mut read) {
            if requests.is_empty() {
                continue;
            }

            let seqn = requests[0].ledger_seq;
            let first = inner.fetch_node_object(&hash, seqn, FetchType::Async, false);
            for request in requests {
                let result = if request.ledger_seq == seqn
                    || inner.delegate.is_same_db(request.ledger_seq, seqn)
                {
                    first.clone()
                } else {
                    inner.fetch_node_object(&hash, request.ledger_seq, FetchType::Async, false)
                };
                (request.callback)(result);
            }
        }
    }

    inner.running_threads.fetch_sub(1, Ordering::Relaxed);
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{DatabaseDelegate, DatabaseRuntime, DatabaseSource, FetchType};
    use crate::{
        DummyScheduler, FetchReport, NodeObject, NodeObjectType, NodeStoreJournal, NullJournal,
    };
    use basics::{base_uint::Uint256, basic_config::Section};
    use protocol::JsonValue;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    struct TestDelegate {
        objects: BTreeMap<Uint256, Arc<NodeObject>>,
    }

    impl DatabaseDelegate for TestDelegate {
        fn is_same_db(&self, _first: u32, _second: u32) -> bool {
            true
        }

        fn fetch_node_object(
            &self,
            hash: &Uint256,
            _ledger_seq: u32,
            fetch_report: &mut FetchReport,
            _duplicate: bool,
            _journal: &dyn NodeStoreJournal,
        ) -> Option<Arc<NodeObject>> {
            let object = self.objects.get(hash).cloned();
            if object.is_some() {
                fetch_report.was_found = true;
            }
            object
        }
    }

    #[test]
    fn get_counts_json_matches_current_cpp_field_set_and_types() {
        let hash = Uint256::from_array([0xA5; 32]);
        let object = Arc::new(NodeObject::new(NodeObjectType::Ledger, vec![1, 2, 3], hash));
        let delegate = Arc::new(TestDelegate {
            objects: BTreeMap::from([(hash, Arc::clone(&object))]),
        });
        let scheduler = Arc::new(DummyScheduler);
        let journal: Arc<dyn NodeStoreJournal> = Arc::new(NullJournal);
        let config = Section::new("node_db");
        let database =
            DatabaseRuntime::new(delegate, scheduler, 1, &config, journal).expect("database");

        database.store_stats(1, object.data().len() as u64);
        let fetched = database.fetch_node_object(&hash, 1, FetchType::Synchronous, false);
        assert_eq!(fetched.expect("stored object").data(), object.data());

        let JsonValue::Object(counts) = database.get_counts_json() else {
            panic!("counts json should be an object");
        };

        let keys: Vec<_> = counts.keys().cloned().collect();
        assert_eq!(
            keys,
            vec![
                "node_read_bytes",
                "node_reads_duration_us",
                "node_reads_hit",
                "node_reads_total",
                "node_writes",
                "node_written_bytes",
                "read_queue",
                "read_request_bundle",
                "read_threads_running",
                "read_threads_total",
            ]
        );
        assert_eq!(counts.get("read_queue"), Some(&JsonValue::Unsigned(0)));
        assert!(matches!(
            counts.get("read_threads_total"),
            Some(JsonValue::Signed(value)) if *value >= 1
        ));
        assert!(matches!(
            counts.get("read_threads_running"),
            Some(JsonValue::Signed(value)) if *value >= 0
        ));
        assert_eq!(
            counts.get("read_request_bundle"),
            Some(&JsonValue::Signed(4))
        );
        assert_eq!(
            counts.get("node_writes"),
            Some(&JsonValue::String("1".to_owned()))
        );
        assert_eq!(
            counts.get("node_reads_total"),
            Some(&JsonValue::String("1".to_owned()))
        );
        assert_eq!(
            counts.get("node_reads_hit"),
            Some(&JsonValue::String("1".to_owned()))
        );
        assert_eq!(
            counts.get("node_written_bytes"),
            Some(&JsonValue::String("3".to_owned()))
        );
        assert_eq!(
            counts.get("node_read_bytes"),
            Some(&JsonValue::String("3".to_owned()))
        );
        assert!(matches!(
            counts.get("node_reads_duration_us"),
            Some(JsonValue::String(value)) if value.parse::<u64>().is_ok()
        ));

        database.stop();
    }

    struct VecSource(Vec<Arc<NodeObject>>);

    impl DatabaseSource for VecSource {
        fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) {
            for object in &self.0 {
                callback(Arc::clone(object));
            }
        }
    }

    #[test]
    fn import_internal_batches_every_source_object() {
        use crate::Backend;
        use crate::Status;
        use std::sync::Mutex;

        struct RecordingBackend {
            imported: Mutex<Vec<Arc<NodeObject>>>,
        }

        impl Backend for RecordingBackend {
            fn get_name(&self) -> String {
                "recording".to_owned()
            }

            fn open(&self, _create_if_missing: bool) -> Result<(), String> {
                Ok(())
            }

            fn is_open(&self) -> bool {
                true
            }

            fn close(&self) -> Result<(), String> {
                Ok(())
            }

            fn fetch(&self, _hash: &Uint256) -> (Option<Arc<NodeObject>>, Status) {
                (None, Status::NotFound)
            }

            fn fetch_batch(&self, _hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status) {
                (Vec::new(), Status::Ok)
            }

            fn store(&self, object: Arc<NodeObject>) {
                self.imported
                    .lock()
                    .expect("recording backend mutex must not be poisoned")
                    .push(object);
            }

            fn store_batch(&self, batch: &crate::Batch) {
                self.imported
                    .lock()
                    .expect("recording backend mutex must not be poisoned")
                    .extend(batch.iter().cloned());
            }

            fn sync(&self) {}

            fn for_each(&self, _callback: &mut dyn FnMut(Arc<NodeObject>)) {}

            fn get_write_load(&self) -> i32 {
                0
            }

            fn set_delete_path(&self) {}

            fn fd_required(&self) -> i32 {
                0
            }
        }

        let scheduler = Arc::new(DummyScheduler);
        let journal: Arc<dyn NodeStoreJournal> = Arc::new(NullJournal);
        let database = DatabaseRuntime::new(
            Arc::new(TestDelegate {
                objects: BTreeMap::new(),
            }),
            scheduler,
            1,
            &Section::new("node_db"),
            journal,
        )
        .expect("database");
        let backend = RecordingBackend {
            imported: Mutex::new(Vec::new()),
        };
        let source = VecSource(vec![
            Arc::new(NodeObject::new(
                NodeObjectType::Ledger,
                vec![1],
                Uint256::from_array([0x01; 32]),
            )),
            Arc::new(NodeObject::new(
                NodeObjectType::TransactionNode,
                vec![2, 3],
                Uint256::from_array([0x02; 32]),
            )),
        ]);

        database.import_internal(&backend, &source);

        let imported = backend
            .imported
            .lock()
            .expect("recording backend mutex must not be poisoned");
        assert_eq!(imported.len(), 2);
        assert_eq!(database.get_store_count(), 2);
        assert_eq!(database.get_store_size(), 3);

        drop(imported);
        database.stop();
    }
}
