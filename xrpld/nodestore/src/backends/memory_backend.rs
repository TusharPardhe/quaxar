use crate::{Backend, Factory, NodeObject, NodeStoreJournal, Scheduler, Status};
use basics::{base_uint::Uint256, basic_config::Section};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::{Arc, Mutex},
};

fn read_string(section: &Section, key: &str) -> String {
    section
        .get::<String>(key)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn normalize_case_fold(value: &str) -> String {
    value.to_ascii_lowercase()
}

#[derive(Debug, Default)]
struct MemoryDb {
    open: Mutex<bool>,
    table: Mutex<BTreeMap<Uint256, Arc<NodeObject>>>,
}

#[derive(Debug, Default)]
pub(crate) struct MemoryFactoryState {
    databases: Mutex<BTreeMap<String, Arc<MemoryDb>>>,
}

impl MemoryFactoryState {
    fn open(&self, path: &str) -> Result<Arc<MemoryDb>, String> {
        let key = normalize_case_fold(path);
        let mut databases = self
            .databases
            .lock()
            .expect("memory factory databases mutex must not be poisoned");
        let database = databases
            .entry(key)
            .or_insert_with(|| Arc::new(MemoryDb::default()))
            .clone();
        let mut is_open = database
            .open
            .lock()
            .expect("memory database open mutex must not be poisoned");
        if *is_open {
            return Err("already open".to_owned());
        }
        *is_open = true;
        drop(is_open);
        Ok(database)
    }

    fn close(&self, database: &Arc<MemoryDb>) {
        let mut is_open = database
            .open
            .lock()
            .expect("memory database open mutex must not be poisoned");
        *is_open = false;
    }
}

pub struct MemoryFactory {
    state: Arc<MemoryFactoryState>,
}

impl MemoryFactory {
    pub fn new() -> Self {
        Self {
            state: Arc::new(MemoryFactoryState::default()),
        }
    }
}

impl Default for MemoryFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl Factory for MemoryFactory {
    fn get_name(&self) -> String {
        "Memory".to_owned()
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
        Ok(Box::new(MemoryBackend::new(
            parameters,
            journal,
            Arc::clone(&self.state),
        )?))
    }
}

pub struct MemoryBackend {
    name: String,
    journal: Arc<dyn NodeStoreJournal>,
    factory_state: Arc<MemoryFactoryState>,
    database: Mutex<Option<Arc<MemoryDb>>>,
}

impl MemoryBackend {
    pub(crate) fn new(
        key_values: &Section,
        journal: Arc<dyn NodeStoreJournal>,
        factory_state: Arc<MemoryFactoryState>,
    ) -> Result<Self, String> {
        let name = read_string(key_values, "path");
        if name.is_empty() {
            return Err("Missing path in Memory backend".to_owned());
        }

        Ok(Self {
            name,
            journal,
            factory_state,
            database: Mutex::new(None),
        })
    }

    fn open_database(&self) -> Arc<MemoryDb> {
        self.database
            .lock()
            .expect("memory backend database mutex must not be poisoned")
            .clone()
            .expect("xrpl::NodeStore::MemoryBackend requires an open database")
    }
}

impl Backend for MemoryBackend {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn open(&self, _create_if_missing: bool) -> Result<(), String> {
        let db = self.factory_state.open(&self.name)?;
        *self
            .database
            .lock()
            .expect("memory backend database mutex must not be poisoned") = Some(db);
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.database
            .lock()
            .expect("memory backend database mutex must not be poisoned")
            .is_some()
    }

    fn close(&self) -> Result<(), String> {
        let database = self
            .database
            .lock()
            .expect("memory backend database mutex must not be poisoned")
            .take();
        if let Some(database) = database {
            self.factory_state.close(&database);
        }
        Ok(())
    }

    fn fetch(&self, hash: &Uint256) -> (Option<Arc<NodeObject>>, Status) {
        let db = self.open_database();
        let table = db
            .table
            .lock()
            .expect("memory database table mutex must not be poisoned");
        match table.get(hash) {
            Some(object) => (Some(Arc::clone(object)), Status::Ok),
            None => (None, Status::NotFound),
        }
    }

    fn fetch_batch(&self, hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status) {
        let mut results = Vec::with_capacity(hashes.len());
        for hash in hashes {
            let (object, status) = self.fetch(hash);
            if status == Status::Ok {
                results.push(object);
            } else {
                results.push(None);
            }
        }
        (results, Status::Ok)
    }

    fn store(&self, object: Arc<NodeObject>) {
        let db = self.open_database();
        let mut table = db
            .table
            .lock()
            .expect("memory database table mutex must not be poisoned");
        match table.entry(*object.hash()) {
            Entry::Vacant(entry) => {
                entry.insert(object);
            }
            Entry::Occupied(_) => {}
        }
    }

    fn store_batch(&self, batch: &crate::Batch) {
        for object in batch {
            self.store(Arc::clone(object));
        }
    }

    fn sync(&self) {}

    fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) {
        let _ = &self.journal;
        let db = self.open_database();
        let snapshot = db
            .table
            .lock()
            .expect("memory database table mutex must not be poisoned")
            .iter()
            .map(|(hash, object)| (*hash, Arc::clone(object)))
            .collect::<Vec<_>>();
        for (_hash, object) in snapshot {
            callback(object);
        }
    }

    fn get_write_load(&self) -> i32 {
        0
    }

    fn set_delete_path(&self) {}

    fn fd_required(&self) -> i32 {
        0
    }
}

impl Drop for MemoryBackend {
    fn drop(&mut self) {
        let _ = <Self as Backend>::close(self);
    }
}

#[cfg(test)]
mod tests {
    use super::{MemoryBackend, MemoryFactory};
    use crate::{Factory, NodeObject, NodeObjectType, NullJournal};
    use basics::{base_uint::Uint256, basic_config::Section};
    use std::sync::Arc;

    fn section(path: &str) -> Section {
        let mut section = Section::new("node_db");
        section.set("type", "Memory");
        section.set("path", path);
        section
    }

    fn sample_object(fill: u8, payload: &[u8]) -> Arc<NodeObject> {
        NodeObject::create_object(
            NodeObjectType::Ledger,
            payload.to_vec(),
            Uint256::from_array([fill; 32]),
        )
    }

    #[test]
    fn memory_backend_requires_path() {
        let section = Section::new("node_db");
        let result = MemoryBackend::new(&section, Arc::new(NullJournal), Arc::default());
        match result {
            Ok(_) => panic!("backend should require a path"),
            Err(error) => assert_eq!(error, "Missing path in Memory backend"),
        }
    }

    #[test]
    fn memory_factory_requires_exclusive_open_and_preserves_table_after_reopen() {
        let factory = MemoryFactory::new();
        let scheduler = Arc::new(crate::DummyScheduler);
        let journal = Arc::new(NullJournal);

        let backend_a = factory
            .create_instance(
                NodeObject::KEY_BYTES,
                &section("Cache/Path"),
                0,
                scheduler.clone(),
                journal.clone(),
            )
            .expect("memory backend should construct");
        let backend_b = factory
            .create_instance(
                NodeObject::KEY_BYTES,
                &section("cache/path"),
                0,
                scheduler,
                journal,
            )
            .expect("memory backend should construct");

        backend_a.open(true).expect("open should succeed");
        match backend_b.open(true) {
            Ok(()) => panic!("second open should fail while the first backend owns the path"),
            Err(error) => assert_eq!(error, "already open"),
        }

        let first = sample_object(0x11, &[1, 2, 3]);
        backend_a.store(Arc::clone(&first));
        backend_a.close().expect("close should succeed");

        backend_b
            .open(true)
            .expect("reopen should succeed after close");
        let replacement =
            NodeObject::create_object(NodeObjectType::Ledger, vec![9, 9, 9], *first.hash());
        backend_b.store(replacement);

        let (fetched, status) = backend_b.fetch(first.hash());
        assert_eq!(status, crate::Status::Ok);
        let fetched = fetched.expect("stored object should be found");
        assert_eq!(fetched.data(), first.data());
    }

    #[test]
    fn memory_backend_drop_releases_named_database_ownership() {
        let factory = MemoryFactory::new();
        let scheduler = Arc::new(crate::DummyScheduler);
        let journal = Arc::new(NullJournal);

        let backend_a = factory
            .create_instance(
                NodeObject::KEY_BYTES,
                &section("drop/path"),
                0,
                Arc::clone(&scheduler) as Arc<dyn crate::Scheduler>,
                Arc::clone(&journal) as Arc<dyn crate::NodeStoreJournal>,
            )
            .expect("memory backend should construct");
        backend_a.open(true).expect("open should succeed");
        drop(backend_a);

        let backend_b = factory
            .create_instance(
                NodeObject::KEY_BYTES,
                &section("drop/path"),
                0,
                scheduler,
                journal,
            )
            .expect("memory backend should construct");
        backend_b
            .open(true)
            .expect("drop should release ownership for a later open");
    }

    #[test]
    fn memory_backend_fetch_batch_tracks_missing_entries() {
        let factory = MemoryFactory::new();
        let backend = factory
            .create_instance(
                NodeObject::KEY_BYTES,
                &section("batch/path"),
                0,
                Arc::new(crate::DummyScheduler),
                Arc::new(NullJournal),
            )
            .expect("memory backend should construct");
        backend.open(true).expect("open should succeed");

        let present = sample_object(0x21, &[4, 5, 6]);
        backend.store(Arc::clone(&present));

        let missing = Uint256::from_array([0x22; 32]);
        let (results, status) = backend.fetch_batch(&[*present.hash(), missing]);
        assert_eq!(status, crate::Status::Ok);
        assert!(results[0].is_some());
        assert!(results[1].is_none());
    }

    #[test]
    fn memory_backend_for_each_visits_each_live_object_once_in_key_order() {
        let factory = MemoryFactory::new();
        let backend = factory
            .create_instance(
                NodeObject::KEY_BYTES,
                &section("foreach/path"),
                0,
                Arc::new(crate::DummyScheduler),
                Arc::new(NullJournal),
            )
            .expect("memory backend should construct");
        backend.open(true).expect("open should succeed");

        let first = sample_object(0x11, &[1]);
        let second = sample_object(0x22, &[2]);
        let duplicate = NodeObject::create_object(NodeObjectType::Ledger, vec![9], *first.hash());

        backend.store(Arc::clone(&second));
        backend.store(Arc::clone(&first));
        backend.store(duplicate);

        let mut seen = Vec::new();
        backend.for_each(&mut |object| {
            seen.push((*object.hash(), object.data().clone()));
        });

        assert_eq!(
            seen,
            vec![
                (*first.hash(), first.data().clone()),
                (*second.hash(), second.data().clone()),
            ]
        );
    }
}
