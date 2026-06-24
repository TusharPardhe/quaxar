//! App-owned loaded-ledger serving seam for `GetLedger`.
//!
//! This keeps the reference `PeerImp::getLedger` shape out of `main.rs`: resolve an
//! immutable ledger through the app-owned `LedgerMaster` plus `LedgerHistory`
//! storage path, then let callers serve base or SHAMap node replies from that
//! loaded ledger.

use crate::{
    AppLedgerMaster, AppLedgerMasterRuntime, ApplicationRoot, SHAMapStoreNodeStore,
    SqliteSHAMapStoreRelational,
};
use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::MonotonicClock;
use ledger::{
    Ledger, LedgerConfig, LedgerHashPair, LedgerInfoProvider, LedgerSetupError, NullLedgerJournal,
};
use nodestore::{FetchType, NodeObjectType as NodeStoreObjectType};
use overlay::TmGetLedger;
use overlay::message::wire::TmLedgerNode;
use protocol::{LedgerHeader, serialize_ledger_header};
use rusqlite::{OptionalExtension, params};
use shamap::family::{
    NullFullBelowCache, NullMissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher,
};
use shamap::node_id::deserialize_shamap_node_id;
use shamap::node_object::NodeObject as SHAMapNodeObject;
use shamap::storage::NodeObjectType as SHAMapNodeObjectType;
use shamap::traversal::TraversalError;
use shamap::tree_node_cache::TreeNodeCache;
use std::sync::Arc;
use time::Duration;

const LI_TX_NODE: i32 = 1;
const LI_AS_NODE: i32 = 2;
const LT_CLOSED: i32 = 0;
const DEFAULT_TREE_CACHE_SIZE: usize = 8;
const DEFAULT_TREE_CACHE_AGE: Duration = Duration::seconds(1);
const SOFT_MAX_REPLY_NODES: usize = 8_192;
const HARD_MAX_REPLY_NODES: usize = 12_288;

type LoadedLedgerFamily = SHAMapFamily<
    MonotonicClock,
    HardenedHashBuilder,
    NullFullBelowCache,
    LoadedLedgerNodeFetcher,
    NullMissingNodeReporter,
>;

#[derive(Clone)]
struct LoadedLedgerDbProvider {
    relational: Option<Arc<SqliteSHAMapStoreRelational>>,
}

impl LoadedLedgerDbProvider {
    fn new(relational: Option<Arc<SqliteSHAMapStoreRelational>>) -> Self {
        Self { relational }
    }

    fn query_one(&self, sql: &str, bind: impl rusqlite::Params) -> Option<LedgerHeader> {
        let relational = self.relational.as_ref()?;
        let ledger_db = relational.ledger_db();
        let connection = ledger_db.get_session();
        connection
            .query_row(sql, bind, |row| {
                let close_time_resolution = row.get::<_, u32>(6)?;
                let close_flags = row.get::<_, u32>(7)?;
                Ok(LedgerHeader {
                    hash: parse_sql_hash(row.get::<_, String>(0)?)?,
                    seq: row.get::<_, u32>(1)?,
                    parent_hash: parse_sql_hash(row.get::<_, String>(2)?)?,
                    drops: row.get::<_, u64>(3)?,
                    close_time: row.get::<_, u32>(4)?,
                    parent_close_time: row.get::<_, u32>(5)?,
                    close_time_resolution: u8::try_from(close_time_resolution).map_err(|_| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Integer,
                            Box::new(std::io::Error::other("invalid close time resolution")),
                        )
                    })?,
                    close_flags: u8::try_from(close_flags).map_err(|_| {
                        rusqlite::Error::FromSqlConversionFailure(
                            7,
                            rusqlite::types::Type::Integer,
                            Box::new(std::io::Error::other("invalid close flags")),
                        )
                    })?,
                    account_hash: parse_sql_hash(row.get::<_, String>(8)?)?,
                    tx_hash: parse_sql_hash(row.get::<_, String>(9)?)?,
                    ..LedgerHeader::default()
                })
            })
            .optional()
            .ok()
            .flatten()
    }
}

impl LedgerInfoProvider for LoadedLedgerDbProvider {
    fn get_ledger_info_by_index(&self, ledger_index: u32) -> Option<LedgerHeader> {
        self.query_one(
            "SELECT LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash FROM Ledgers WHERE LedgerSeq = ?1 ORDER BY LedgerSeq DESC LIMIT 1",
            params![i64::from(ledger_index)],
        )
    }

    fn get_ledger_info_by_hash(&self, ledger_hash: SHAMapHash) -> Option<LedgerHeader> {
        self.query_one(
            "SELECT LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash FROM Ledgers WHERE LedgerHash = ?1 LIMIT 1",
            params![ledger_hash.as_uint256().to_string()],
        )
    }

    fn get_newest_ledger_info(&self) -> Option<LedgerHeader> {
        self.query_one(
            "SELECT LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash FROM Ledgers ORDER BY LedgerSeq DESC LIMIT 1",
            [],
        )
    }
}

#[derive(Clone)]
struct LoadedLedgerNodeFetcher {
    node_store: Option<SHAMapStoreNodeStore>,
}

impl LoadedLedgerNodeFetcher {
    fn new(node_store: Option<SHAMapStoreNodeStore>) -> Self {
        Self { node_store }
    }
}

impl SHAMapNodeFetcher for LoadedLedgerNodeFetcher {
    fn fetch_node_object(&self, hash: SHAMapHash, ledger_seq: u32) -> Option<SHAMapNodeObject> {
        let node_store = self.node_store.as_ref()?;
        let fetched = match node_store {
            SHAMapStoreNodeStore::Single(database) => database.fetch_node_object(
                hash.as_uint256(),
                ledger_seq,
                FetchType::Synchronous,
                false,
            ),
            SHAMapStoreNodeStore::Rotating(database) => database.fetch_node_object(
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

#[derive(Clone)]
pub struct AppLoadedLedgerRuntime {
    ledger_master: Arc<AppLedgerMaster>,
    config: LedgerConfig,
    provider: LoadedLedgerDbProvider,
    family: Arc<LoadedLedgerFamily>,
    relational: Option<Arc<SqliteSHAMapStoreRelational>>,
    node_store: Option<SHAMapStoreNodeStore>,
    storage_enabled: bool,
}

impl AppLoadedLedgerRuntime {
    pub fn with_ledger_master(ledger_master: Arc<AppLedgerMaster>) -> Self {
        Self::with_sources(ledger_master, None, None)
    }

    pub fn with_runtime_and_sources(
        ledger_master_runtime: Arc<AppLedgerMasterRuntime>,
        relational: Option<Arc<SqliteSHAMapStoreRelational>>,
        node_store: Option<SHAMapStoreNodeStore>,
    ) -> Self {
        Self::with_sources(
            ledger_master_runtime.ledger_master(),
            relational,
            node_store,
        )
    }

    pub fn with_sources(
        ledger_master: Arc<AppLedgerMaster>,
        relational: Option<Arc<SqliteSHAMapStoreRelational>>,
        node_store: Option<SHAMapStoreNodeStore>,
    ) -> Self {
        let storage_enabled = relational.is_some() && node_store.is_some();
        Self {
            ledger_master,
            config: LedgerConfig::default(),
            provider: LoadedLedgerDbProvider::new(relational.clone()),
            family: Arc::new(SHAMapFamily::new(
                Arc::new(TreeNodeCache::new(
                    "app-loaded-ledger-runtime",
                    DEFAULT_TREE_CACHE_SIZE,
                    DEFAULT_TREE_CACHE_AGE,
                    MonotonicClock::default(),
                )),
                NullFullBelowCache::new(0),
                LoadedLedgerNodeFetcher::new(node_store.clone()),
                NullMissingNodeReporter,
            )),
            relational,
            node_store,
            storage_enabled,
        }
    }

    pub fn from_root(root: &ApplicationRoot) -> Option<Self> {
        let ledger_master_runtime = root.ledger_master_runtime()?;
        Some(Self::with_runtime_and_sources(
            ledger_master_runtime,
            root.relational_database().clone(),
            root.node_store().clone(),
        ))
    }

    pub fn resolve_request_ledger(
        &self,
        request: &TmGetLedger,
    ) -> Result<Option<Arc<Ledger>>, LedgerSetupError> {
        let journal = NullLedgerJournal;

        if let Some(hash) = request
            .ledger_hash
            .as_deref()
            .and_then(Uint256::from_slice)
            .map(SHAMapHash::new)
        {
            let mut ledger = self.ledger_master.get_ledger_by_hash(hash);
            if ledger.is_none() && self.storage_enabled {
                ledger = self.ledger_master.ledger_history().get_ledger_by_hash(
                    hash,
                    &journal,
                    &self.config,
                    self.family.as_ref(),
                    &self.provider,
                )?;
            }
            return Ok(filter_requested_ledger_seq(request, ledger));
        }

        if let Some(seq) = request.ledger_seq {
            let mut ledger = self.ledger_master.get_ledger_by_seq(seq, &journal);
            if ledger.is_none() && self.storage_enabled {
                ledger = self.ledger_master.ledger_history().get_ledger_by_seq(
                    seq,
                    &journal,
                    &self.config,
                    self.family.as_ref(),
                    &self.provider,
                )?;
            }
            return Ok(filter_requested_ledger_seq(request, ledger));
        }

        if request.ltype == Some(LT_CLOSED) {
            return Ok(filter_requested_ledger_seq(
                request,
                self.ledger_master.closed_ledger(),
            ));
        }

        Ok(None)
    }

    pub fn get_history_ledger_by_hash(
        &self,
        hash: SHAMapHash,
    ) -> Result<Option<Arc<Ledger>>, LedgerSetupError> {
        let journal = NullLedgerJournal;

        let mut ledger = self.ledger_master.get_ledger_by_hash(hash);
        if ledger.is_none() && self.storage_enabled {
            ledger = self.ledger_master.ledger_history().get_ledger_by_hash(
                hash,
                &journal,
                &self.config,
                self.family.as_ref(),
                &self.provider,
            )?;
        }

        Ok(ledger)
    }

    pub fn get_history_ledger_by_seq(
        &self,
        seq: u32,
    ) -> Result<Option<Arc<Ledger>>, LedgerSetupError> {
        let journal = NullLedgerJournal;

        let mut ledger = self.ledger_master.get_ledger_by_seq(seq, &journal);
        if ledger.is_none() && self.storage_enabled {
            ledger = self.ledger_master.ledger_history().get_ledger_by_seq(
                seq,
                &journal,
                &self.config,
                self.family.as_ref(),
                &self.provider,
            )?;
        }

        Ok(ledger)
    }

    pub fn earliest_ledger_seq(&self) -> u32 {
        match self.node_store.as_ref() {
            Some(SHAMapStoreNodeStore::Single(database)) => database.earliest_ledger_seq(),
            Some(SHAMapStoreNodeStore::Rotating(database)) => database.earliest_ledger_seq(),
            None => self.minimum_sql_ledger_seq().unwrap_or(1),
        }
    }

    pub fn get_hash_by_index(&self, ledger_index: u32) -> Option<SHAMapHash> {
        let journal = NullLedgerJournal;
        if let Some(ledger) = self.ledger_master.get_ledger_by_seq(ledger_index, &journal) {
            return Some(ledger.header().hash);
        }

        let hash = self
            .ledger_master
            .ledger_history()
            .get_ledger_hash(ledger_index);
        if hash.is_non_zero() {
            return Some(hash);
        }

        self.provider
            .get_ledger_info_by_index(ledger_index)
            .map(|header| header.hash)
    }

    pub fn get_hash_pairs_by_index(
        &self,
        min_seq: u32,
        max_seq: u32,
    ) -> Vec<(u32, LedgerHashPair)> {
        let Some(relational) = self.relational.as_ref() else {
            return Vec::new();
        };
        let ledger_db = relational.ledger_db();
        let connection = ledger_db.get_session();
        let mut statement = match connection.prepare(
            "SELECT LedgerSeq, LedgerHash, PrevHash \
             FROM Ledgers WHERE LedgerSeq >= ?1 AND LedgerSeq <= ?2 ORDER BY LedgerSeq ASC",
        ) {
            Ok(statement) => statement,
            Err(_) => return Vec::new(),
        };
        let rows =
            match statement.query_map(params![i64::from(min_seq), i64::from(max_seq)], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    LedgerHashPair {
                        ledger_hash: parse_sql_hash(row.get::<_, String>(1)?)?,
                        parent_hash: parse_sql_hash(row.get::<_, String>(2)?)?,
                    },
                ))
            }) {
                Ok(rows) => rows,
                Err(_) => return Vec::new(),
            };

        rows.filter_map(Result::ok).collect()
    }

    pub fn has_ledger_object(&self, ledger_hash: SHAMapHash, ledger_seq: u32) -> bool {
        match self.node_store.as_ref() {
            Some(SHAMapStoreNodeStore::Single(database)) => database
                .fetch_node_object(
                    ledger_hash.as_uint256(),
                    ledger_seq,
                    FetchType::Synchronous,
                    false,
                )
                .is_some(),
            Some(SHAMapStoreNodeStore::Rotating(database)) => database
                .fetch_node_object(
                    ledger_hash.as_uint256(),
                    ledger_seq,
                    FetchType::Synchronous,
                    false,
                )
                .is_some(),
            None => false,
        }
    }

    pub fn build_base_reply_nodes(&self, ledger: &Ledger) -> Vec<TmLedgerNode> {
        let mut nodes = vec![TmLedgerNode {
            nodeid: None,
            nodedata: serialize_ledger_header(&ledger.header(), false),
        }];

        let state_map = ledger.state_map();
        if state_map.root().get_hash().is_non_zero()
            && let Ok(root) = state_map.serialize_root()
        {
            nodes.push(TmLedgerNode {
                nodeid: None,
                nodedata: root,
            });

            let tx_map = ledger.tx_map();
            if ledger.header().tx_hash.is_non_zero()
                && tx_map.root().get_hash().is_non_zero()
                && let Ok(root) = tx_map.serialize_root()
            {
                nodes.push(TmLedgerNode {
                    nodeid: None,
                    nodedata: root,
                });
            }
        }

        nodes
    }

    pub fn build_shamap_reply_nodes(
        &self,
        ledger: &Ledger,
        request: &TmGetLedger,
        peer_high_latency: bool,
    ) -> Result<Vec<TmLedgerNode>, TraversalError> {
        let map = match request.itype {
            LI_TX_NODE => ledger.tx_map(),
            LI_AS_NODE => ledger.state_map(),
            _ => return Ok(Vec::new()),
        };

        let default_depth = if peer_high_latency { 2 } else { 1 };
        let query_depth = request.query_depth.unwrap_or(default_depth);
        let mut reply_nodes = Vec::new();

        for node_id_bytes in &request.node_i_ds {
            if reply_nodes.len() >= SOFT_MAX_REPLY_NODES {
                break;
            }

            let Some(node_id) = deserialize_shamap_node_id(node_id_bytes) else {
                continue;
            };

            let mut data = Vec::new();
            if !map.get_node_fat_with_family(
                node_id,
                &mut data,
                true,
                query_depth,
                self.family.as_ref(),
            )? {
                continue;
            }

            for (node_id, blob) in data {
                if reply_nodes.len() >= HARD_MAX_REPLY_NODES {
                    break;
                }
                reply_nodes.push(TmLedgerNode {
                    nodeid: Some(node_id.get_raw_string()),
                    nodedata: blob,
                });
            }
        }

        Ok(reply_nodes)
    }

    fn minimum_sql_ledger_seq(&self) -> Option<u32> {
        let relational = self.relational.as_ref()?;
        let ledger_db = relational.ledger_db();
        let connection = ledger_db.get_session();
        connection
            .query_row("SELECT MIN(LedgerSeq) FROM Ledgers", [], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }
}

fn filter_requested_ledger_seq(
    request: &TmGetLedger,
    ledger: Option<Arc<Ledger>>,
) -> Option<Arc<Ledger>> {
    ledger.filter(|ledger| {
        request
            .ledger_seq
            .is_none_or(|requested| ledger.header().seq == requested)
    })
}

fn parse_sql_hash(value: String) -> rusqlite::Result<SHAMapHash> {
    Uint256::from_hex(&value).map(SHAMapHash::new).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            value.len(),
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other("invalid ledger hash")),
        )
    })
}
