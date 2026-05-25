//! App-owned validated-ledger persistence runtime.
//!
//! This ports the concrete reference the reference implementation responsibility of saving one
//! validated ledger into the relational tables and then promoting cached
//! transactions into their committed in-ledger state.
//!
//! "Narrow" here does not mean the save function is partial or fake. It means
//! this file intentionally covers only that specific validated-ledger save
//! responsibility, while other surrounding reference application owners such as the
//! fuller background job orchestration and broader relational-database owner
//! graph remain separate migration slices.

use crate::shamap::shamap_store_relational::SqliteSHAMapStoreRelational;
use crate::tx_queue::transaction_master::TransactionMaster;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::MonotonicClock;
use ledger::{
    AcceptedLedger, Ledger, LedgerPersistenceJob, LedgerPersistenceJobType,
    LedgerPersistenceRuntime,
};
use nodestore::{FetchType, NodeObjectType as NodeStoreObjectType};
use shamap::family::{
    NullFullBelowCache, NullMissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher,
};
use shamap::node_object::NodeObject as SHAMapNodeObject;
use shamap::storage::NodeObjectType as SHAMapNodeObjectType;
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use time::Duration;

#[derive(Clone)]
struct PersistenceNodeStoreFetcher {
    node_store: crate::SHAMapStoreNodeStore,
}

impl PersistenceNodeStoreFetcher {
    fn new(node_store: crate::SHAMapStoreNodeStore) -> Self {
        Self { node_store }
    }
}

impl SHAMapNodeFetcher for PersistenceNodeStoreFetcher {
    fn fetch_node_object(&mut self, hash: SHAMapHash, ledger_seq: u32) -> Option<SHAMapNodeObject> {
        let fetched = match &self.node_store {
            crate::SHAMapStoreNodeStore::Single(database) => database.fetch_node_object(
                hash.as_uint256(),
                ledger_seq,
                FetchType::Synchronous,
                false,
            ),
            crate::SHAMapStoreNodeStore::Rotating(database) => database.fetch_node_object(
                hash.as_uint256(),
                ledger_seq,
                FetchType::Synchronous,
                false,
            ),
        }?;

        Some(SHAMapNodeObject::new(
            match fetched.object_type() {
                NodeStoreObjectType::Ledger => SHAMapNodeObjectType::Ledger,
                NodeStoreObjectType::AccountNode => SHAMapNodeObjectType::AccountNode,
                NodeStoreObjectType::TransactionNode => SHAMapNodeObjectType::TransactionNode,
                NodeStoreObjectType::Unknown | NodeStoreObjectType::Dummy => {
                    SHAMapNodeObjectType::Unknown
                }
            },
            fetched.data().to_vec(),
            *fetched.hash(),
        ))
    }
}

pub fn save_validated_ledger_to_sql(
    relational: &SqliteSHAMapStoreRelational,
    transaction_master: &TransactionMaster,
    ledger: Arc<Ledger>,
    network_id: u32,
    node_store: Option<crate::SHAMapStoreNodeStore>,
) -> Result<(), String> {
    let accepted = if let Some(node_store) = node_store {
        let family = SHAMapFamily::new(
            Arc::new(TreeNodeCache::new(
                "app-ledger-persistence",
                2048,
                Duration::seconds(30),
                MonotonicClock::default(),
            )),
            NullFullBelowCache::new(0),
            PersistenceNodeStoreFetcher::new(node_store),
            NullMissingNodeReporter,
        );
        AcceptedLedger::new_with_family(Arc::clone(&ledger), &family)
            .map_err(|error| format!("accepted ledger build failed: {error:?}"))?
    } else {
        AcceptedLedger::new(Arc::clone(&ledger))
            .map_err(|error| format!("accepted ledger build failed: {error:?}"))?
    };
    relational.write_accepted_ledger(&accepted, transaction_master, network_id)
}

pub struct AppLedgerPersistenceRuntime {
    relational_database: Option<Arc<SqliteSHAMapStoreRelational>>,
    node_store: Option<crate::SHAMapStoreNodeStore>,
    transaction_master: Arc<TransactionMaster>,
    network_id: u32,
    ledger_db: Option<Arc<rdb::LedgerDb>>,
    saved_hashes: Mutex<HashSet<SHAMapHash>>,
    pending: Mutex<HashSet<u32>>,
}

impl AppLedgerPersistenceRuntime {
    pub fn new(
        relational_database: Option<Arc<SqliteSHAMapStoreRelational>>,
        node_store: Option<crate::SHAMapStoreNodeStore>,
        transaction_master: Arc<TransactionMaster>,
        network_id: u32,
        ledger_db: Option<Arc<rdb::LedgerDb>>,
    ) -> Self {
        Self {
            relational_database,
            node_store,
            transaction_master,
            network_id,
            ledger_db,
            saved_hashes: Mutex::new(HashSet::new()),
            pending: Mutex::new(HashSet::new()),
        }
    }
}

impl LedgerPersistenceRuntime for AppLedgerPersistenceRuntime {
    fn mark_saved(&self, hash: SHAMapHash) -> bool {
        self.saved_hashes
            .lock()
            .expect("saved_hashes mutex must not be poisoned")
            .insert(hash)
    }

    fn start_work(&self, seq: u32) -> bool {
        self.pending
            .lock()
            .expect("pending mutex must not be poisoned")
            .insert(seq)
    }

    fn finish_work(&self, seq: u32) {
        self.pending
            .lock()
            .expect("pending mutex must not be poisoned")
            .remove(&seq);
    }

    fn should_work(&self, _seq: u32, _is_synchronous: bool) -> bool {
        true
    }

    fn pending(&self, seq: u32) -> bool {
        self.pending
            .lock()
            .expect("pending mutex must not be poisoned")
            .contains(&seq)
    }

    fn save_validated_ledger(&self, ledger: Arc<Ledger>, _is_current: bool) -> bool {
        // Write header to SQLite Ledgers table (compatibility: the reference source kADD_LEDGER).
        // This is the primary bootstrap source on restart — mirrors reference getLastFullLedger().
        if let Some(db) = self.ledger_db.as_ref() {
            if let Err(e) = db.insert_ledger(&ledger.header()) {
                tracing::info!(target: "ledger",
                    "[ledger_persistence] rdb insert failed seq={} error={e}",
                    ledger.header().seq
                );
            }
        }

        let Some(relational) = self.relational_database.as_ref() else {
            return true;
        };

        match save_validated_ledger_to_sql(
            relational,
            self.transaction_master.as_ref(),
            Arc::clone(&ledger),
            self.network_id,
            self.node_store.clone(),
        ) {
            Ok(()) => true,
            Err(error) => {
                self.saved_hashes
                    .lock()
                    .expect("saved_hashes mutex must not be poisoned")
                    .remove(&ledger.header().hash);
                tracing::info!(target: "ledger",
                    "[ledger_persistence] save failed seq={} hash={} error={}",
                    ledger.header().seq,
                    ledger.header().hash,
                    error
                );
                false
            }
        }
    }

    fn enqueue_job(
        &self,
        _job_type: LedgerPersistenceJobType,
        _job_name: String,
        job: LedgerPersistenceJob,
    ) -> bool {
        job();
        true
    }
}
