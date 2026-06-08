use crate::database::{
    Database as DatabaseTrait, DatabaseDelegate, DatabaseImporter,
    DatabaseRotating as DatabaseRotatingTrait, DatabaseRuntime, DatabaseSource,
};
use crate::{
    AsyncFetchCallback, Backend, FetchReport, FetchType, JournalLevel, NodeObject, NodeObjectType,
    NodeStoreJournal, Scheduler, Status,
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

struct DatabaseRotatingCore {
    state: Arc<Mutex<RotatingState>>,
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

                if duplicate {
                    writable.store(Arc::clone(node_object_ref));
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
}

impl Drop for DatabaseRotatingImp {
    fn drop(&mut self) {
        self.database.stop();
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
        let database = DatabaseRuntime::new(
            Arc::new(DatabaseRotatingCore {
                state: Arc::clone(&state),
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
        }))
    }

    fn rotate_impl(&self, new_backend: Box<dyn Backend>, callback: &mut dyn FnMut(&str, &str)) {
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

    pub fn store(&self, object_type: NodeObjectType, data: Blob, hash: Uint256, _ledger_seq: u32) {
        let node_object = NodeObject::create_object(object_type, data, hash);
        let backend = {
            let state = self
                .state
                .lock()
                .expect("rotating backend mutex must not be poisoned");
            Arc::clone(&state.writable_backend)
        };

        backend.store(Arc::clone(&node_object));
        self.database
            .store_stats(1, node_object.data().len() as u64);
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

    pub fn async_fetch(&self, hash: Uint256, ledger_seq: u32, callback: AsyncFetchCallback) {
        self.database.async_fetch(hash, ledger_seq, callback);
    }

    pub fn stop(&self) {
        self.database.stop();
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

    fn store(&self, object_type: NodeObjectType, data: Blob, hash: Uint256, ledger_seq: u32) {
        DatabaseRotatingImp::store(self, object_type, data, hash, ledger_seq);
    }

    fn is_same_db(&self, first: u32, second: u32) -> bool {
        let _ = (first, second);
        true
    }

    fn sync(&self) {
        DatabaseRotatingImp::sync(self);
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

    fn async_fetch(&self, hash: Uint256, ledger_seq: u32, callback: AsyncFetchCallback) {
        DatabaseRotatingImp::async_fetch(self, hash, ledger_seq, callback);
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
        let state = self.state.lock().expect("rotating backend mutex must not be poisoned");
        Some(Arc::clone(&state.writable_backend))
    }
}

impl DatabaseRotatingTrait for DatabaseRotatingImp {
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
        Backend, DummyScheduler, FetchType, NodeObject, NodeObjectType, NullJournal, Status,
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

        fn store(&self, object: Arc<NodeObject>) {
            self.store_count.fetch_add(1, Ordering::Relaxed);
            self.objects
                .lock()
                .expect("objects mutex")
                .insert(*object.hash(), object);
        }

        fn store_batch(&self, batch: &crate::Batch) {
            for object in batch {
                self.store(Arc::clone(object));
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
        archive.store(Arc::clone(&object));

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
