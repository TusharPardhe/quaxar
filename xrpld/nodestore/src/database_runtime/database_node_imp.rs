use crate::database::{
    Database as DatabaseTrait, DatabaseDelegate, DatabaseImporter, DatabaseRuntime, DatabaseSource,
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
use std::sync::Arc;
use std::time::Instant;

struct DatabaseNodeImpCore {
    backend: Arc<dyn Backend>,
}

impl DatabaseDelegate for DatabaseNodeImpCore {
    fn is_same_db(&self, _first: u32, _second: u32) -> bool {
        true
    }

    fn fetch_node_object(
        &self,
        hash: &Uint256,
        _ledger_seq: u32,
        fetch_report: &mut FetchReport,
        _duplicate: bool,
        journal: &dyn NodeStoreJournal,
    ) -> Option<Arc<NodeObject>> {
        let hash_hex = str_hex(hash.data());
        let (node_object, status) =
            match catch_unwind(AssertUnwindSafe(|| self.backend.fetch(hash))) {
                Ok(result) => result,
                Err(payload) => {
                    journal.log(
                        JournalLevel::Fatal,
                        &format!(
                            "fetchNodeObject {hash_hex}: Exception fetching from backend: {}",
                            panic_message(payload.as_ref())
                        ),
                    );
                    resume_unwind(payload);
                }
            };

        match status {
            Status::Ok | Status::NotFound => {}
            Status::DataCorrupt => {
                journal.log(
                    JournalLevel::Fatal,
                    &format!("fetchNodeObject {hash_hex}: nodestore data is corrupted"),
                );
            }
            other => {
                journal.log(
                    JournalLevel::Warn,
                    &format!(
                        "fetchNodeObject {hash_hex}: backend returns unknown result {other:?}"
                    ),
                );
            }
        }

        if node_object.is_some() {
            fetch_report.was_found = true;
        }
        node_object
    }
}

pub struct DatabaseNodeImp {
    database: DatabaseRuntime,
    backend: Arc<dyn Backend>,
}

impl Drop for DatabaseNodeImp {
    fn drop(&mut self) {
        self.stop();
    }
}

impl DatabaseNodeImp {
    pub fn new(
        scheduler: Arc<dyn Scheduler>,
        read_threads: usize,
        backend: Arc<dyn Backend>,
        config: &Section,
        journal: Arc<dyn NodeStoreJournal>,
    ) -> Result<Arc<Self>, String> {
        let database = DatabaseRuntime::new(
            Arc::new(DatabaseNodeImpCore {
                backend: backend.clone(),
            }),
            scheduler,
            read_threads,
            config,
            journal,
        )?;

        Ok(Arc::new(Self { database, backend }))
    }

    pub fn get_name(&self) -> String {
        self.backend.get_name()
    }

    pub fn get_write_load(&self) -> i32 {
        self.backend.get_write_load()
    }

    pub fn import_database(&self, source: &dyn DatabaseSource) {
        self.database.import_internal(self.backend.as_ref(), source);
    }

    pub fn store(&self, object_type: NodeObjectType, data: Blob, hash: Uint256, _ledger_seq: u32) {
        self.database.store_stats(1, data.len() as u64);
        let object = NodeObject::create_object(object_type, data, hash);
        self.backend.store(object);
    }

    pub fn is_same_db(&self, _first: u32, _second: u32) -> bool {
        true
    }

    pub fn sync(&self) {
        self.backend.sync();
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

    pub fn fetch_batch(&self, hashes: &[Uint256]) -> Vec<Option<Arc<NodeObject>>> {
        let before = Instant::now();
        let (mut results, _status) = self.backend.fetch_batch(hashes);
        assert!(
            results.len() == hashes.len() || results.is_empty(),
            "number of output objects either matches number of input hashes or is empty"
        );
        results.resize(hashes.len(), None);

        for (index, result) in results.iter().enumerate() {
            if result.is_none() {
                self.database.journal().log(
                    JournalLevel::Error,
                    &format!(
                        "fetchBatch - record not found in db. hash = {}",
                        str_hex(hashes[index].data())
                    ),
                );
            }
        }

        self.database.update_fetch_metrics(
            hashes.len() as u64,
            0,
            before.elapsed().as_micros() as u64,
        );
        results
    }

    pub fn stop(&self) {
        self.database.stop();
        if let Err(error) = self.backend.close() {
            tracing::error!(target: "nodestore", %error, "NodeStore backend close failed");
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
        self.backend.fd_required()
    }
}

impl DatabaseSource for DatabaseNodeImp {
    fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) {
        self.backend.for_each(callback);
    }
}

impl DatabaseImporter for DatabaseNodeImp {
    fn import_database(&self, source: &dyn DatabaseSource) {
        DatabaseNodeImp::import_database(self, source);
    }
}

impl DatabaseTrait for DatabaseNodeImp {
    fn get_name(&self) -> String {
        DatabaseNodeImp::get_name(self)
    }

    fn get_write_load(&self) -> i32 {
        DatabaseNodeImp::get_write_load(self)
    }

    fn store(&self, object_type: NodeObjectType, data: Blob, hash: Uint256, ledger_seq: u32) {
        DatabaseNodeImp::store(self, object_type, data, hash, ledger_seq);
    }

    fn is_same_db(&self, first: u32, second: u32) -> bool {
        let _ = (first, second);
        true
    }

    fn sync(&self) {
        DatabaseNodeImp::sync(self);
    }

    fn fetch_node_object(
        &self,
        hash: &Uint256,
        ledger_seq: u32,
        fetch_type: FetchType,
        duplicate: bool,
    ) -> Option<Arc<NodeObject>> {
        DatabaseNodeImp::fetch_node_object(self, hash, ledger_seq, fetch_type, duplicate)
    }

    fn async_fetch(&self, hash: Uint256, ledger_seq: u32, callback: AsyncFetchCallback) {
        DatabaseNodeImp::async_fetch(self, hash, ledger_seq, callback);
    }

    fn stop(&self) {
        DatabaseNodeImp::stop(self);
    }

    fn is_stopping(&self) -> bool {
        DatabaseNodeImp::is_stopping(self)
    }

    fn earliest_ledger_seq(&self) -> u32 {
        DatabaseNodeImp::earliest_ledger_seq(self)
    }

    fn get_store_count(&self) -> u64 {
        DatabaseNodeImp::get_store_count(self)
    }

    fn get_fetch_total_count(&self) -> u64 {
        DatabaseNodeImp::get_fetch_total_count(self)
    }

    fn get_fetch_hit_count(&self) -> u64 {
        DatabaseNodeImp::get_fetch_hit_count(self)
    }

    fn get_store_size(&self) -> u64 {
        DatabaseNodeImp::get_store_size(self)
    }

    fn get_fetch_size(&self) -> u64 {
        DatabaseNodeImp::get_fetch_size(self)
    }

    fn add_counts_json(&self, obj: &mut BTreeMap<String, JsonValue>) {
        DatabaseNodeImp::add_counts_json(self, obj);
    }

    fn get_counts_json(&self) -> JsonValue {
        DatabaseNodeImp::get_counts_json(self)
    }

    fn fd_required(&self) -> i32 {
        DatabaseNodeImp::fd_required(self)
    }

    fn export_backend(&self) -> Option<Arc<dyn Backend>> {
        Some(Arc::clone(&self.backend))
    }

    fn get_fetch_latency_histogram(&self) -> [u64; 6] {
        self.database.get_fetch_latency_histogram()
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
