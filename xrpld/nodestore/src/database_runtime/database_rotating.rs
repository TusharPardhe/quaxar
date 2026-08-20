use crate::database::{
    Database as DatabaseTrait, DatabaseDelegate, DatabaseImporter,
    DatabaseRotating as DatabaseRotatingTrait, DatabaseRuntime, DatabaseSource,
};
use crate::{
    AsyncReadWork, Backend, FetchReport, FetchType, JournalLevel, NodeObject, NodeObjectType,
    NodeStoreJournal, ScheduledWrite, Scheduler, Status,
};
use basics::base_uint::Uint256;
use basics::basic_config::Section;
use basics::blob::Blob;
use basics::str_hex::str_hex;
use protocol::JsonValue;
use std::any::Any;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::{Arc, Mutex};

struct RotatingState {
    writable_backend: Arc<dyn Backend>,
    archive_backend: Arc<dyn Backend>,
}

/// Read-only snapshot view captured from a rotating database. Holding both
/// backend Arcs prevents the retired archive from being dropped while a
/// background snapshot export is still traversing it.
struct RotatingSnapshotBackend {
    writable_backend: Arc<dyn Backend>,
    archive_backend: Arc<dyn Backend>,
}

impl Backend for RotatingSnapshotBackend {
    fn get_name(&self) -> String {
        self.writable_backend.get_name()
    }

    fn get_block_size(&self) -> Option<usize> {
        self.writable_backend.get_block_size()
    }

    fn open(&self, _create_if_missing: bool) -> Result<(), String> {
        Err("rotating snapshot backend is an already-open read-only view".to_owned())
    }

    fn is_open(&self) -> bool {
        self.writable_backend.is_open() && self.archive_backend.is_open()
    }

    fn close(&self) -> Result<(), String> {
        // The database owner, not the export view, owns backend lifecycle.
        Ok(())
    }

    fn fetch(&self, hash: &Uint256) -> (Option<Arc<NodeObject>>, Status) {
        let (object, status) = self.writable_backend.fetch(hash);
        if object.is_some() || status != Status::NotFound {
            return (object, status);
        }
        self.archive_backend.fetch(hash)
    }

    fn fetch_batch(&self, hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status) {
        let mut results = Vec::with_capacity(hashes.len());
        let mut status = Status::Ok;
        for hash in hashes {
            let (object, result_status) = self.fetch(hash);
            if !matches!(result_status, Status::Ok | Status::NotFound) && status == Status::Ok {
                status = result_status;
            }
            results.push(object);
        }
        (results, status)
    }

    fn store(&self, _object: Arc<NodeObject>) -> Result<(), String> {
        Err("rotating snapshot backend is read-only".to_owned())
    }

    fn store_batch(&self, _batch: &crate::Batch) {}

    fn store_batch_result(&self, _batch: &crate::Batch) -> Result<(), String> {
        Err("rotating snapshot backend is read-only".to_owned())
    }

    fn sync(&self) {}

    fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) {
        self.writable_backend.for_each(callback);
        self.archive_backend.for_each(callback);
    }

    fn for_each_result(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) -> Result<(), String> {
        self.writable_backend.for_each_result(callback)?;
        self.archive_backend.for_each_result(callback)
    }

    fn get_write_load(&self) -> i32 {
        0
    }

    fn set_delete_path(&self) {}

    fn fd_required(&self) -> i32 {
        0
    }
}

struct DatabaseRotatingCore {
    state: Arc<Mutex<RotatingState>>,
    rotation_in_flight: Arc<std::sync::atomic::AtomicBool>,
    // Shared handle written by copy-forward reads during rotation; the
    // DatabaseRotatingImp retains its own clone of the same atomic for
    // take_copy_forward_count(). The field exists to keep the handle alive.
    #[allow(dead_code)]
    copy_forward_count: Arc<std::sync::atomic::AtomicU64>,
}

impl DatabaseRotatingCore {
    fn backends(&self) -> (Arc<dyn Backend>, Arc<dyn Backend>) {
        let state = self
            .state
            .lock()
            .expect("rotating backend mutex must not be poisoned");
        (
            Arc::clone(&state.writable_backend),
            Arc::clone(&state.archive_backend),
        )
    }
}

impl DatabaseDelegate for DatabaseRotatingCore {
    fn is_same_db(&self, _first: u32, _second: u32) -> bool {
        true
    }

    fn cache_read_allowed(&self, duplicate: bool) -> bool {
        !duplicate
            && !self
                .rotation_in_flight
                .load(std::sync::atomic::Ordering::Acquire)
    }

    fn fetch_node_object(
        &self,
        hash: &Uint256,
        _ledger_seq: u32,
        fetch_report: &mut FetchReport,
        duplicate: bool,
        journal: &dyn NodeStoreJournal,
    ) -> Option<Arc<NodeObject>> {
        let fetch = |backend: &Arc<dyn Backend>| {
            let (node_object, status) = match catch_unwind(AssertUnwindSafe(|| backend.fetch(hash)))
            {
                Ok(result) => result,
                Err(payload) => {
                    journal.log(
                        JournalLevel::Fatal,
                        &format!("Exception, {}", panic_message(payload.as_ref())),
                    );
                    resume_unwind(payload);
                }
            };

            match status {
                Status::Ok | Status::NotFound => {}
                Status::DataCorrupt => {
                    journal.log(
                        JournalLevel::Fatal,
                        &format!("Corrupt NodeObject #{}", str_hex(hash.data())),
                    );
                }
                other => {
                    journal.log(JournalLevel::Warn, &format!("Unknown status={other:?}"));
                }
            }

            node_object
        };

        let (mut writable, archive) = self.backends();
        let mut node_object = fetch(&writable);
        if node_object.is_none() {
            node_object = fetch(&archive);
            if let Some(node_object_ref) = &node_object {
                let state = self
                    .state
                    .lock()
                    .expect("rotating backend mutex must not be poisoned");
                writable = Arc::clone(&state.writable_backend);
                drop(state);

                if duplicate && let Err(error) = writable.store(Arc::clone(node_object_ref)) {
                    tracing::error!(target: "nodestore", %error, hash = %hash, "Failed to copy archive node into writable backend");
                    journal.log(JournalLevel::Error, &error);
                    panic!("failed to copy archive node into writable backend: {error}");
                }
                // While rotation is in flight, copy archive-served reads
                // forward into the writable backend. The archive is about
                // to be deleted; without this, a node canonicalized into
                // the cache after the freshen snapshot would survive only
                // in RAM once the archive is dropped.
                if !duplicate
                    && self
                        .rotation_in_flight
                        .load(std::sync::atomic::Ordering::Acquire)
                {
                    if let Err(error) = writable.store(Arc::clone(node_object_ref)) {
                        tracing::error!(target: "nodestore", %error, hash = %hash, "Failed to copy archive node into writable backend");
                        journal.log(JournalLevel::Error, &error);
                        panic!("failed to copy archive node into writable backend: {error}");
                    }
                    self.copy_forward_count
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }

        if node_object.is_some() {
            fetch_report.was_found = true;
        }
        node_object
    }
}

pub struct DatabaseRotatingImp {
    database: DatabaseRuntime,
    state: Arc<Mutex<RotatingState>>,
    journal: Arc<dyn NodeStoreJournal>,
    fd_required: i32,
    rotation_in_flight: Arc<std::sync::atomic::AtomicBool>,
    copy_forward_count: Arc<std::sync::atomic::AtomicU64>,
}

impl Drop for DatabaseRotatingImp {
    fn drop(&mut self) {
        self.stop();
    }
}

impl DatabaseRotatingImp {
    pub fn new(
        scheduler: Arc<dyn Scheduler>,
        read_threads: usize,
        writable_backend: Arc<dyn Backend>,
        archive_backend: Arc<dyn Backend>,
        config: &Section,
        journal: Arc<dyn NodeStoreJournal>,
    ) -> Result<Arc<Self>, String> {
        let fd_required = writable_backend.fd_required() + archive_backend.fd_required();
        let state = Arc::new(Mutex::new(RotatingState {
            writable_backend,
            archive_backend,
        }));
        let rotation_in_flight = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let copy_forward_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let database = DatabaseRuntime::new(
            Arc::new(DatabaseRotatingCore {
                state: Arc::clone(&state),
                rotation_in_flight: Arc::clone(&rotation_in_flight),
                copy_forward_count: Arc::clone(&copy_forward_count),
            }),
            scheduler,
            read_threads,
            config,
            Arc::clone(&journal),
        )?;

        Ok(Arc::new(Self {
            database,
            state,
            journal,
            fd_required,
            rotation_in_flight,
            copy_forward_count,
        }))
    }

    /// Enable/disable copy-forward of archive reads during rotation.
    /// Call with `true` before starting the copy phase, `false` after
    /// rotate() completes. Matches rippled's setRotationInFlight().
    pub fn set_rotation_in_flight(&self, in_flight: bool) {
        let was_in_flight = self
            .rotation_in_flight
            .swap(in_flight, std::sync::atomic::Ordering::AcqRel);
        if in_flight && !was_in_flight {
            // Publish the new identity before cache invalidation or any
            // archive read can be admitted into a rotating exposure window.
            self.database.advance_store_generation();
            // Archive-resident entries must not hide copy-forward reads while
            // the old archive is being retired.
            self.database.invalidate_node_object_cache();
        }
    }

    /// Returns and resets the count of nodes copied forward during rotation.
    pub fn take_copy_forward_count(&self) -> u64 {
        self.copy_forward_count
            .swap(0, std::sync::atomic::Ordering::Relaxed)
    }

    fn rotate_impl(&self, new_backend: Box<dyn Backend>, callback: &mut dyn FnMut(&str, &str)) {
        // A caller normally enables rotation_in_flight before copying. Set it
        // defensively here as well so a direct rotate cannot serve entries
        // sourced solely from the outgoing archive.
        let already_in_flight = self
            .rotation_in_flight
            .swap(true, std::sync::atomic::Ordering::AcqRel);
        if !already_in_flight {
            self.database.advance_store_generation();
        }
        self.database.invalidate_node_object_cache();

        let new_writable_backend_name = new_backend.get_name();
        let new_writable_backend: Arc<dyn Backend> = Arc::from(new_backend);

        let (old_archive_backend, new_archive_backend_name) = {
            let mut state = self
                .state
                .lock()
                .expect("rotating backend mutex must not be poisoned");

            state.archive_backend.set_delete_path();
            let old_archive_backend = Arc::clone(&state.archive_backend);
            state.archive_backend = Arc::clone(&state.writable_backend);
            let archive_name = state.archive_backend.get_name();
            state.writable_backend = new_writable_backend;
            (old_archive_backend, archive_name)
        };

        callback(&new_writable_backend_name, &new_archive_backend_name);
        drop(old_archive_backend);
        if !already_in_flight {
            self.rotation_in_flight
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }

    pub fn rotate<F>(&self, new_backend: Box<dyn Backend>, callback: F)
    where
        F: FnOnce(&str, &str),
    {
        let mut callback = Some(callback);
        self.rotate_impl(new_backend, &mut |writable_name, archive_name| {
            if let Some(callback) = callback.take() {
                callback(writable_name, archive_name);
            }
        });
    }

    pub fn get_name(&self) -> String {
        let state = self
            .state
            .lock()
            .expect("rotating backend mutex must not be poisoned");
        state.writable_backend.get_name()
    }

    pub fn get_write_load(&self) -> i32 {
        let state = self
            .state
            .lock()
            .expect("rotating backend mutex must not be poisoned");
        state.writable_backend.get_write_load()
    }

    pub fn journal(&self) -> Arc<dyn NodeStoreJournal> {
        Arc::clone(&self.journal)
    }

    pub fn import_database(&self, source: &dyn DatabaseSource) {
        let backend = {
            let state = self
                .state
                .lock()
                .expect("rotating backend mutex must not be poisoned");
            Arc::clone(&state.writable_backend)
        };
        self.database.import_internal(backend.as_ref(), source);
    }

    pub fn sync(&self) {
        let state = self
            .state
            .lock()
            .expect("rotating backend mutex must not be poisoned");
        state.writable_backend.sync();
    }

    pub fn sync_result(&self) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .expect("rotating backend mutex must not be poisoned");
        state.writable_backend.sync_result()
    }

    pub fn store(
        &self,
        object_type: NodeObjectType,
        data: Blob,
        hash: Uint256,
        _ledger_seq: u32,
    ) -> Result<(), String> {
        let node_object = NodeObject::create_object(object_type, data, hash);
        let backend = {
            let state = self
                .state
                .lock()
                .expect("rotating backend mutex must not be poisoned");
            Arc::clone(&state.writable_backend)
        };

        backend.store(Arc::clone(&node_object)).map_err(|error| {
            tracing::error!(target: "nodestore", %error, hash = %node_object.hash(), "Rotating NodeStore backend write failed");
            error
        })?;
        self.database
            .store_stats(1, node_object.data().len() as u64);
        self.database.promote_node_object(node_object);
        Ok(())
    }

    pub fn fetch_node_object(
        &self,
        hash: &Uint256,
        ledger_seq: u32,
        fetch_type: FetchType,
        duplicate: bool,
    ) -> Option<Arc<NodeObject>> {
        self.database
            .fetch_node_object(hash, ledger_seq, fetch_type, duplicate)
    }

    pub fn async_fetch(&self, hash: Uint256, ledger_seq: u32, work: Box<dyn AsyncReadWork>) {
        self.database.async_fetch(hash, ledger_seq, work);
    }

    pub fn stop(&self) {
        self.database.stop();
        let (writable, archive) = {
            let state = self
                .state
                .lock()
                .expect("rotating backend mutex must not be poisoned");
            (
                Arc::clone(&state.writable_backend),
                Arc::clone(&state.archive_backend),
            )
        };
        for (role, backend) in [("writable", writable), ("archive", archive)] {
            if let Err(error) = backend.close() {
                tracing::error!(target: "nodestore", %error, role, "Rotating NodeStore backend close failed");
            }
        }
    }

    pub fn is_stopping(&self) -> bool {
        self.database.is_stopping()
    }

    pub fn earliest_ledger_seq(&self) -> u32 {
        self.database.earliest_ledger_seq()
    }

    pub fn get_store_count(&self) -> u64 {
        self.database.get_store_count()
    }

    pub fn get_fetch_total_count(&self) -> u64 {
        self.database.get_fetch_total_count()
    }

    pub fn get_fetch_hit_count(&self) -> u64 {
        self.database.get_fetch_hit_count()
    }

    pub fn get_store_size(&self) -> u64 {
        self.database.get_store_size()
    }

    pub fn get_fetch_size(&self) -> u64 {
        self.database.get_fetch_size()
    }

    pub fn add_counts_json(&self, obj: &mut BTreeMap<String, JsonValue>) {
        self.database.add_counts_json(obj);
    }

    pub fn get_counts_json(&self) -> JsonValue {
        self.database.get_counts_json()
    }

    pub fn fd_required(&self) -> i32 {
        self.fd_required
    }

    pub fn store_generation(&self) -> u64 {
        self.database.store_generation()
    }
}

impl DatabaseSource for DatabaseRotatingImp {
    fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) {
        let (writable, archive) = {
            let state = self
                .state
                .lock()
                .expect("rotating backend mutex must not be poisoned");
            (
                Arc::clone(&state.writable_backend),
                Arc::clone(&state.archive_backend),
            )
        };

        writable.for_each(callback);
        archive.for_each(callback);
    }
}

impl DatabaseImporter for DatabaseRotatingImp {
    fn import_database(&self, source: &dyn DatabaseSource) {
        DatabaseRotatingImp::import_database(self, source);
    }
}

impl DatabaseTrait for DatabaseRotatingImp {
    fn get_name(&self) -> String {
        DatabaseRotatingImp::get_name(self)
    }

    fn get_write_load(&self) -> i32 {
        DatabaseRotatingImp::get_write_load(self)
    }

    fn store(
        &self,
        object_type: NodeObjectType,
        data: Blob,
        hash: Uint256,
        ledger_seq: u32,
    ) -> Result<(), String> {
        DatabaseRotatingImp::store(self, object_type, data, hash, ledger_seq)
    }

    fn is_same_db(&self, first: u32, second: u32) -> bool {
        let _ = (first, second);
        true
    }

    fn sync(&self) {
        DatabaseRotatingImp::sync(self);
    }

    fn sync_result(&self) -> Result<(), String> {
        DatabaseRotatingImp::sync_result(self)
    }

    fn schedule_write(&self, write: ScheduledWrite) {
        self.database.schedule_write(write);
    }

    fn store_generation(&self) -> u64 {
        DatabaseRotatingImp::store_generation(self)
    }

    fn fetch_node_object(
        &self,
        hash: &Uint256,
        ledger_seq: u32,
        fetch_type: FetchType,
        duplicate: bool,
    ) -> Option<Arc<NodeObject>> {
        DatabaseRotatingImp::fetch_node_object(self, hash, ledger_seq, fetch_type, duplicate)
    }

    fn async_fetch(&self, hash: Uint256, ledger_seq: u32, work: Box<dyn AsyncReadWork>) {
        DatabaseRotatingImp::async_fetch(self, hash, ledger_seq, work);
    }

    fn stop(&self) {
        DatabaseRotatingImp::stop(self);
    }

    fn is_stopping(&self) -> bool {
        DatabaseRotatingImp::is_stopping(self)
    }

    fn earliest_ledger_seq(&self) -> u32 {
        DatabaseRotatingImp::earliest_ledger_seq(self)
    }

    fn get_store_count(&self) -> u64 {
        DatabaseRotatingImp::get_store_count(self)
    }

    fn get_fetch_total_count(&self) -> u64 {
        DatabaseRotatingImp::get_fetch_total_count(self)
    }

    fn get_fetch_hit_count(&self) -> u64 {
        DatabaseRotatingImp::get_fetch_hit_count(self)
    }

    fn get_store_size(&self) -> u64 {
        DatabaseRotatingImp::get_store_size(self)
    }

    fn get_fetch_size(&self) -> u64 {
        DatabaseRotatingImp::get_fetch_size(self)
    }

    fn add_counts_json(&self, obj: &mut BTreeMap<String, JsonValue>) {
        DatabaseRotatingImp::add_counts_json(self, obj);
    }

    fn get_counts_json(&self) -> JsonValue {
        DatabaseRotatingImp::get_counts_json(self)
    }

    fn fd_required(&self) -> i32 {
        DatabaseRotatingImp::fd_required(self)
    }

    fn export_backend(&self) -> Option<Arc<dyn Backend>> {
        let state = self
            .state
            .lock()
            .expect("rotating backend mutex must not be poisoned");
        Some(Arc::new(RotatingSnapshotBackend {
            writable_backend: Arc::clone(&state.writable_backend),
            archive_backend: Arc::clone(&state.archive_backend),
        }))
    }
}

impl DatabaseRotatingTrait for DatabaseRotatingImp {
    fn set_rotation_in_flight(&self, in_flight: bool) {
        DatabaseRotatingImp::set_rotation_in_flight(self, in_flight);
    }

    fn rotate(&self, new_backend: Box<dyn Backend>, callback: &mut dyn FnMut(&str, &str)) {
        self.rotate_impl(new_backend, callback);
    }
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
    use super::DatabaseRotatingImp;
    use crate::{
        Backend, Database, DummyScheduler, FetchType, NodeObject, NodeObjectType, NullJournal,
        Status,
    };
    use basics::{base_uint::Uint256, basic_config::Section};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct TestBackend {
        name: String,
        delete_path_called: AtomicBool,
        store_count: AtomicUsize,
        objects: Mutex<BTreeMap<Uint256, Arc<NodeObject>>>,
    }

    use std::collections::BTreeMap;

    impl TestBackend {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_owned(),
                delete_path_called: AtomicBool::new(false),
                store_count: AtomicUsize::new(0),
                objects: Mutex::new(BTreeMap::new()),
            }
        }
    }

    impl Backend for TestBackend {
        fn get_name(&self) -> String {
            self.name.clone()
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

        fn fetch(&self, hash: &Uint256) -> (Option<Arc<NodeObject>>, Status) {
            let object = self
                .objects
                .lock()
                .expect("objects mutex")
                .get(hash)
                .cloned();
            match object {
                Some(object) => (Some(object), Status::Ok),
                None => (None, Status::NotFound),
            }
        }

        fn fetch_batch(&self, hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status) {
            (
                hashes.iter().map(|hash| self.fetch(hash).0).collect(),
                Status::Ok,
            )
        }

        fn store(&self, object: Arc<NodeObject>) -> Result<(), String> {
            self.store_count.fetch_add(1, Ordering::Relaxed);
            self.objects
                .lock()
                .expect("objects mutex")
                .insert(*object.hash(), object);
            Ok(())
        }

        fn store_batch(&self, batch: &crate::Batch) {
            for object in batch {
                self.store(Arc::clone(object))
                    .expect("test backend store must succeed");
            }
        }

        fn sync(&self) {}

        fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) {
            for object in self
                .objects
                .lock()
                .expect("objects mutex")
                .values()
                .cloned()
                .collect::<Vec<_>>()
            {
                callback(object);
            }
        }

        fn get_write_load(&self) -> i32 {
            self.store_count.load(Ordering::Relaxed) as i32
        }

        fn set_delete_path(&self) {
            self.delete_path_called.store(true, Ordering::Relaxed);
        }

        fn fd_required(&self) -> i32 {
            7
        }
    }

    fn config() -> Section {
        let mut section = Section::new("node_db");
        section.set("type", "memory");
        section.set("path", "rotating");
        section
    }

    fn sample_object(fill: u8) -> Arc<NodeObject> {
        NodeObject::create_object(
            NodeObjectType::Ledger,
            vec![fill, fill + 1],
            Uint256::from_array([fill; 32]),
        )
    }

    #[test]
    fn rotating_fetch_can_duplicate_archive_hit_into_writable_backend() {
        let writable = Arc::new(TestBackend::new("writable"));
        let archive = Arc::new(TestBackend::new("archive"));
        let object = sample_object(0x44);
        archive
            .store(Arc::clone(&object))
            .expect("archive store should succeed");

        let database = DatabaseRotatingImp::new(
            Arc::new(DummyScheduler),
            1,
            Arc::clone(&writable) as Arc<dyn Backend>,
            Arc::clone(&archive) as Arc<dyn Backend>,
            &config(),
            Arc::new(NullJournal),
        )
        .expect("rotating database");

        let fetched = database
            .fetch_node_object(object.hash(), 0, FetchType::Synchronous, true)
            .expect("archive hit");
        assert_eq!(fetched.data(), object.data());
        assert_eq!(writable.store_count.load(Ordering::Relaxed), 1);
        assert_eq!(
            writable
                .fetch(object.hash())
                .0
                .expect("duplicated object")
                .data(),
            object.data()
        );
        assert_eq!(database.fd_required(), 14);

        database.stop();
    }

    #[test]
    fn rotating_duplicate_fetch_bypasses_cached_archive_hit_and_copies_forward() {
        let writable = Arc::new(TestBackend::new("writable"));
        let archive = Arc::new(TestBackend::new("archive"));
        let object = sample_object(0x45);
        archive
            .store(Arc::clone(&object))
            .expect("archive store should succeed");

        let database = DatabaseRotatingImp::new(
            Arc::new(DummyScheduler),
            1,
            Arc::clone(&writable) as Arc<dyn Backend>,
            Arc::clone(&archive) as Arc<dyn Backend>,
            &config(),
            Arc::new(NullJournal),
        )
        .expect("rotating database");

        // Populate Moka from the archive without the duplicate side effect.
        assert!(
            database
                .fetch_node_object(object.hash(), 0, FetchType::Synchronous, false)
                .is_some()
        );
        assert_eq!(writable.store_count.load(Ordering::Relaxed), 0);

        // A duplicate request must still reach the rotating delegate and copy
        // the archive value into writable storage.
        assert!(
            database
                .fetch_node_object(object.hash(), 0, FetchType::Synchronous, true)
                .is_some()
        );
        assert_eq!(writable.store_count.load(Ordering::Relaxed), 1);

        database.stop();
    }

    #[test]
    fn rotation_fences_late_callback_identity_invalidates_cache_and_copies_archive_reads_forward() {
        let writable = Arc::new(TestBackend::new("writable"));
        let archive = Arc::new(TestBackend::new("archive"));
        let object = sample_object(0x46);
        archive
            .store(Arc::clone(&object))
            .expect("archive store should succeed");

        let database = DatabaseRotatingImp::new(
            Arc::new(DummyScheduler),
            1,
            Arc::clone(&writable) as Arc<dyn Backend>,
            Arc::clone(&archive) as Arc<dyn Backend>,
            &config(),
            Arc::new(NullJournal),
        )
        .expect("rotating database");

        assert!(
            database
                .fetch_node_object(object.hash(), 0, FetchType::Synchronous, false)
                .is_some()
        );
        assert_eq!(writable.store_count.load(Ordering::Relaxed), 0);
        assert_eq!(database.store_generation(), 1);

        let pre_rotation_generation = database.store_generation();
        let before_identity = crate::database::PersistenceIdentity {
            acquisition_id: 99,
            persistence_generation: 7,
            store_generation: pre_rotation_generation,
        };
        database.set_rotation_in_flight(true);
        let rotation_generation = database.store_generation();
        let after_identity = crate::database::PersistenceIdentity {
            acquisition_id: 99,
            persistence_generation: 7,
            store_generation: rotation_generation,
        };
        assert_ne!(
            before_identity, after_identity,
            "a late persistence callback stamped before rotation cannot settle the replacement owner"
        );
        assert_ne!(
            pre_rotation_generation, rotation_generation,
            "a callback stamped before rotation cannot observe the current store identity"
        );
        assert!(rotation_generation > 1);
        database.set_rotation_in_flight(true);
        assert_eq!(
            database.store_generation(),
            rotation_generation,
            "one rotation window advances generation once"
        );
        assert!(
            database
                .fetch_node_object(object.hash(), 0, FetchType::Synchronous, false)
                .is_some()
        );
        assert_eq!(writable.store_count.load(Ordering::Relaxed), 1);
        assert_eq!(database.take_copy_forward_count(), 1);

        database.set_rotation_in_flight(false);
        database.rotate(Box::new(TestBackend::new("next")), |_, _| {});
        assert!(
            database.store_generation() > rotation_generation,
            "a direct rotation must also fence pre-rotation callbacks"
        );
        database.stop();
    }

    #[test]
    fn rotating_snapshot_export_view_traverses_writable_and_archive_backends() {
        let writable = Arc::new(TestBackend::new("writable"));
        let archive = Arc::new(TestBackend::new("archive"));
        let writable_object = sample_object(0x51);
        let archive_object = sample_object(0x52);
        writable
            .store(Arc::clone(&writable_object))
            .expect("writable store should succeed");
        archive
            .store(Arc::clone(&archive_object))
            .expect("archive store should succeed");
        let database = DatabaseRotatingImp::new(
            Arc::new(DummyScheduler),
            1,
            Arc::clone(&writable) as Arc<dyn Backend>,
            Arc::clone(&archive) as Arc<dyn Backend>,
            &config(),
            Arc::new(NullJournal),
        )
        .expect("rotating database");

        let export = Database::export_backend(database.as_ref()).expect("export backend");
        let mut hashes = Vec::new();
        export
            .for_each_result(&mut |object| hashes.push(*object.hash()))
            .expect("snapshot traversal");

        assert_eq!(hashes.len(), 2);
        assert!(hashes.contains(writable_object.hash()));
        assert!(hashes.contains(archive_object.hash()));
        database.stop();
    }

    #[test]
    fn rotating_rotate_marks_old_archive_for_delete_and_swaps_names() {
        let writable = Arc::new(TestBackend::new("writable"));
        let archive = Arc::new(TestBackend::new("archive"));
        let database = DatabaseRotatingImp::new(
            Arc::new(DummyScheduler),
            1,
            Arc::clone(&writable) as Arc<dyn Backend>,
            Arc::clone(&archive) as Arc<dyn Backend>,
            &config(),
            Arc::new(NullJournal),
        )
        .expect("rotating database");

        let callback_args = Arc::new(Mutex::new(None));
        let seen = Arc::clone(&callback_args);
        database.rotate(
            Box::new(TestBackend::new("new")),
            move |writable_name, archive_name| {
                *seen.lock().expect("callback mutex") =
                    Some((writable_name.to_owned(), archive_name.to_owned()));
            },
        );

        assert!(archive.delete_path_called.load(Ordering::Relaxed));
        assert_eq!(database.get_name(), "new");
        assert_eq!(
            callback_args.lock().expect("callback mutex").clone(),
            Some(("new".to_owned(), "writable".to_owned()))
        );

        database.stop();
    }
}
