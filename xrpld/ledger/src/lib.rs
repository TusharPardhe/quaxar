//! First `Ledger` caller slice above the landed `SHAMap` owners.
//!
//! This crate starts with the the reference implementation `setFull()` and `walkLedger()`
//! roles. The goal of this slice is to preserve caller-level behavior and
//! quirks while the Rust `SHAMap` boundary is widened underneath it.

mod acquisition;
mod domain;
mod history_runtime;
pub mod sync_config;
pub mod views;

pub use acquisition::delta_acquire;
pub use acquisition::fetch_pack;
pub use acquisition::ledger_fetcher;
// inbound_ledgers module removed — unified into app::ledger::inbound_ledgers
pub use acquisition::inbound_transactions;
pub use acquisition::skip_list_acquire;
pub use acquisition::transaction_acquire;
pub use domain::accepted_ledger;
pub use domain::accepted_ledger_tx;
pub use domain::account_root_helpers;
pub use domain::account_state_sf;
pub use domain::amendment_table;
pub use domain::amm_helpers;
pub use domain::amm_utils;
pub use domain::book_dirs;
pub use domain::book_listeners;
pub use domain::canonical_tx_set;
pub use domain::cleaner;
pub use domain::config;
pub use domain::directory;
pub use domain::fees;
pub use domain::flow_engine;
pub use domain::genesis;
pub use domain::ledger_to_json;
pub use domain::local_txs;
pub use domain::master;
pub use domain::master_sweep;
pub use domain::order_book_db;
pub use domain::pending_saves;
pub use domain::persistence;
pub use domain::ripple_calc;
pub use domain::setup;
pub use domain::timeout_counter;
pub use domain::token_helpers;
pub use domain::transaction_state_sf;
pub use domain::trustline;
pub use flow_sandbox::FlowSandbox;
pub use history_runtime::history;
pub use history_runtime::history_fetch;
pub use history_runtime::history_fill;
pub use history_runtime::history_sync;
pub use history_runtime::replay;
pub use history_runtime::replay_task;
pub use history_runtime::replayer;
pub use views::apply_state_table;
pub use views::apply_view;
pub use views::cached_sles;
pub use views::cached_view;
pub use views::directory as apply_directory;
pub use views::flow_sandbox;
pub use views::holder;
pub use views::open_view;
pub use views::payment_sandbox;
pub use views::raw_state_table;
pub use views::raw_view;
pub use views::read_view;
pub use views::sandbox;
pub use views::state_map;

pub use accepted_ledger::{AcceptedLedger, AcceptedLedgerBuildError};
pub use accepted_ledger_tx::{AcceptedLedgerTx, AcceptedLedgerTxMeta};
pub use account_root_helpers::{
    ACCOUNT_TRANSFER_RATE_PARITY, check_destination_and_tag, create_pseudo_account,
    is_global_frozen, pseudo_account_address, transfer_rate,
};
pub use account_state_sf::AccountStateSF;
pub use amendment_table::{AmendmentTable, FeatureInfo, VoteBehavior};
pub use amm_helpers::{
    IsDeposit, RelativeDistanceAmount, adjust_amounts_by_lp_tokens, adjust_asset_in_by_tokens,
    adjust_asset_out_by_tokens, adjust_frac_by_tokens, adjust_lp_tokens, amm_asset_in,
    amm_asset_out, amm_lp_tokens, get_rounded_asset, get_rounded_asset_with_product,
    get_rounded_lp_tokens, get_rounded_lp_tokens_with_product, lp_tokens_in, lp_tokens_out,
    multiply, solve_quadratic_eq_smallest, within_relative_distance_amount,
    within_relative_distance_quality,
};
pub use amm_utils::{
    amm_account_holds, amm_holds, amm_lp_holds, amm_lp_holds_from_sle, amm_pool_holds,
    get_trading_fee, is_only_liquidity_provider,
};
pub use apply_directory::{dir_append, dir_insert, dir_remove};
pub use apply_state_table::ApplyStateTable;
pub use apply_view::{ApplyView, ApplyViewImpl, adjust_owner_count};
pub use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
pub use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::CacheClock;
pub use book_dirs::{BookDirIter, BookDirs};
pub use book_listeners::{BookListenerSubscriber, BookListeners};
pub use cached_sles::CachedSles;
pub use cached_view::CachedView;
pub use canonical_tx_set::CanonicalTXSet;
pub use cleaner::{
    LEDGER_CLEANER_FAILURE_WAIT, LEDGER_CLEANER_LOAD_WAIT, LEDGER_CLEANER_SUCCESS_WAIT,
    LedgerCleaner, LedgerCleanerJournal, LedgerCleanerLoopAction, LedgerCleanerRangeProvider,
    LedgerCleanerRequest, LedgerCleanerRuntime, LedgerCleanerState, LedgerCleanerStatus,
    NullLedgerCleanerJournal, NullLedgerCleanerRangeProvider, NullLedgerCleanerRuntime,
    configure_ledger_cleaner, ledger_cleaner_status, note_ledger_cleaner_failure,
    note_ledger_cleaner_success, plan_ledger_cleaner_iteration,
};
pub use config::{LedgerConfig, LedgerHistorySyncConfig};
pub use delta_acquire::{LedgerDeltaAcquire, LedgerDeltaBuildError};
pub use directory::{
    Dir, DirIter, dir_is_empty, for_each_item, for_each_item_after, for_each_owner_item,
    for_each_owner_item_after,
};
pub use fees::Fees;
pub use fetch_pack::{FetchPackCache, FetchPackContainer, FetchPackStore, LedgerSyncFilterStore};
pub use genesis::{
    build_genesis_master_account_root_item, genesis_master_account_id, genesis_master_account_key,
};
pub use history::{ConsensusValidatedEntry, LedgerHistory, LedgerHistoryMismatch, fix_gaps};
pub use history_fetch::{
    FetchForHistoryResult, FetchForHistoryState, FetchPackRequest, HistoryHashLookup,
    HistoryInboundAcquire, HistoryLedgerLike, HistoryLedgerLookup, HistorySqlInfo, PrefetchAcquire,
    run_fetch_for_history,
};
pub use history_fill::{
    LedgerFillRange, LedgerHashPair, LedgerHashPairProvider, LedgerHistoryFillPlan,
    LedgerHistoryFillStopReason, LedgerObjectPresence, LedgerPresence, Stopper,
    run_try_fill_backwalk,
};
pub use history_sync::{
    FETCH_PACK_STALE_AFTER, FetchPackSendPlan, HistoryAdvancePlan, HistoryFetchPeer,
    LedgerHistorySyncState, apply_fill_plan, expire_stale_fetch_pack_request,
    make_fetch_pack_request, run_history_advance, select_fetch_pack_peer, should_acquire,
    should_drop_fetch_pack_request,
};
pub use holder::LedgerHolder;
pub use ledger_fetcher::{
    INBOUND_LEDGER_MAX_NEEDED_STATE_HASHES, INBOUND_LEDGER_MAX_NEEDED_TX_HASHES,
    INBOUND_LEDGER_MAX_USEFUL_PEERS, InboundLedgerCompletionDisposition, InboundLedgerDataType,
    InboundLedgerJournal, InboundLedgerLocal, InboundLedgerNodeData, InboundLedgerObjectType,
    InboundLedgerPacket, InboundLedgerPacketDebugStats, InboundLedgerPacketError,
    InboundLedgerPacketShape, InboundLedgerPeerScore, InboundLedgerPlannerState,
    InboundLedgerReason, InboundLedgerReceivedPacket, InboundLedgerRequest,
    InboundLedgerRequestTrigger, InboundLedgerRunDataResult, InboundLedgerStore,
    InboundLedgerTimerResult, NullInboundLedgerJournal, get_needed_hashes_with_family,
    make_inbound_get_ledger_request, make_inbound_needed_by_hash_request,
    needed_hashes_with_family, needed_hashes_with_family_and_first_child,
};
// Removed: InboundLedgersLocal, InboundLedgerRoute, stash_stale_packet
// These will be reimplemented in app::ledger::inbound_ledgers
pub use inbound_transactions::{InboundTransactions, InboundTransactionsDataStatus};
pub use ledger_to_json::{
    DEFAULT_LEDGER_JSON_API_VERSION, LedgerFill, LedgerFillOptions, add_json, add_json_with_family,
    copy_from, fill_json, fill_json_binary, fill_json_header, fill_json_state,
    fill_json_state_with_family, fill_json_with_family, get_json, get_json_with_family,
};
pub use local_txs::LocalTxs;
pub use master::{
    LEDGER_MASTER_DEFAULT_FETCH_PACK_AGE, LEDGER_MASTER_DEFAULT_HISTORY_AGE,
    LEDGER_MASTER_DEFAULT_PATH_FIND_JOB_LIMIT, LedgerMaster, LedgerMasterCaughtUp,
    LedgerMasterConfig, LedgerMasterPathWork,
};
pub use master_sweep::{LedgerMasterSweepTarget, sweep_ledger_master_like};
pub use open_view::OpenView;
pub use order_book_db::{
    NullOrderBookDBJournal, NullOrderBookDBRuntime, OrderBookDB, OrderBookDBConfig,
    OrderBookDBJournal, OrderBookDBRuntime, OrderBookSetupResult, OrderBookUpdateJob,
    OrderBookUpdateResult,
};
pub use payment_sandbox::PaymentSandbox;
pub use pending_saves::PendingSaves;
pub use persistence::{
    LedgerPersistence, LedgerPersistenceJob, LedgerPersistenceJobType, LedgerPersistenceRuntime,
    get_latest_ledger, load_by_hash, load_by_index, load_ledger_helper, pend_save_validated,
};
use protocol::{
    DecodedLedgerHashesEntry, Keylet, STArray, STLedgerEntry, STObject, STTx, SerialIter,
    Serializer, TxMeta, amendments_keylet, build_genesis_state_constructor_entries,
    constructor_ledger_items, decode_ledger_hashes_entry, feature_xrp_fees, fee_settings_keylet,
    get_field_by_symbol, make_rules_given_current, negative_unl_keylet, skip_keylet,
    skip_keylet_for_ledger,
};
pub use protocol::{
    FLAG_LEDGER_INTERVAL, LEDGER_HEADER_WIRE_SIZE, LEDGER_HEADER_WITH_HASH_WIRE_SIZE, LedgerHeader,
    LedgerHeaderCodecError, PREFIXED_LEDGER_HEADER_WIRE_SIZE,
    PREFIXED_LEDGER_HEADER_WITH_HASH_WIRE_SIZE, Rules, SLCF_NO_CONSENSUS_TIME, account_root_key,
    amendments_key, calculate_ledger_hash, deserialize_ledger_header,
    deserialize_prefixed_ledger_header, fees_key, get_close_agree, serialize_ledger_header,
    serialize_prefixed_ledger_header,
};
pub use raw_state_table::RawStateTable;
pub use raw_view::{RawView, TxsRawView, TypedRawViewExt};
pub use read_view::{
    DigestAwareReadView, ReadView, ReadViewTx, TypedLedgerEntryRef, TypedReadViewExt, ViewError,
    after, are_compatible, compatibility_reason, has_expired, make_rules_given_ledger,
    view_get_enabled_amendments, view_get_majority_amendments, view_hash_of_seq,
};
pub use replay::{LedgerReplay, LedgerReplayError};
pub use replay_task::{
    LedgerReplayTask, LedgerReplayTaskParameter, REPLAY_TASK_MAX_TIMEOUTS_MINIMUM,
    REPLAY_TASK_MAX_TIMEOUTS_MULTIPLIER, ReplayTaskError,
};
pub use replayer::{LedgerReplayer, REPLAY_MAX_TASK_SIZE, REPLAY_MAX_TASKS};
pub use sandbox::Sandbox;
pub use setup::{AmendmentsEntry, AmountField, FeeSettingsFields, LedgerSetupEntries, SetupLookup};
use shamap::family::{FullBelowCache, MissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher};
use shamap::fetch::SHAMapSyncFilter;
use shamap::item::SHAMapItem;
use shamap::mutation::{MutableTree, MutationError};
use shamap::search::NodePathEntry;
use shamap::sync::{SHAMapMissingNode, SHAMapType, SyncState, SyncTree};
use shamap::traversal::TraversalError;
use shamap::tree_node::SHAMapNodeType;
pub use skip_list_acquire::{
    REPLAY_MAX_NO_FEATURE_PEER_COUNT, REPLAY_SUB_TASK_MAX_TIMEOUTS, SkipListAcquire, SkipListData,
};
pub use state_map::{LedgerSetupError, encode_amendments_entry, encode_fee_settings_entry};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::hash::BuildHasher;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
pub use timeout_counter::{
    NullTimeoutCounterJournal, NullTimeoutCounterRuntime, TimeoutCounter, TimeoutCounterJob,
    TimeoutCounterJobConfig, TimeoutCounterJournal, TimeoutCounterRuntime, TimeoutCounterSnapshot,
};
pub use token_helpers::{FreezeHandling, account_funds, account_funds_text, xrp_liquid};
pub use transaction_acquire::{
    TX_ACQUIRE_MAX_TIMEOUTS, TX_ACQUIRE_NORM_TIMEOUTS, TransactionAcquire,
    TransactionAcquireDataResult, TransactionAcquireFilterFactory,
};
pub use transaction_state_sf::TransactionStateSF;
pub use trustline::{
    credit_balance, credit_limit, is_deep_frozen, is_frozen, is_individual_frozen,
};

fn full_sync_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("XRPLD_FULL_SYNC_DEBUG")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    })
}

macro_rules! full_sync_debug {
    ($($arg:tt)*) => {
        if crate::full_sync_debug_enabled() {
            tracing::debug!(target: "ledger", $($arg)*);
        }
    };
}

pub const LEDGER_POSSIBLE_TIME_RESOLUTIONS: [u8; 6] = [10, 20, 30, 60, 90, 120];
pub const LEDGER_GENESIS_TIME_RESOLUTION: u8 = LEDGER_POSSIBLE_TIME_RESOLUTIONS[0];
pub const LEDGER_DEFAULT_TIME_RESOLUTION: u8 = LEDGER_POSSIBLE_TIME_RESOLUTIONS[2];
pub const INCREASE_LEDGER_TIME_RESOLUTION_EVERY: u32 = 8;
pub const DECREASE_LEDGER_TIME_RESOLUTION_EVERY: u32 = 1;
pub const WALK_LEDGER_MAX_MISSING_NODES: i32 = 32;
pub const XRP_LEDGER_EARLIEST_FEES: u32 = 562_177;
pub const INITIAL_XRP_DROPS: u64 = 100_000_000_000_000_000;
pub const CURRENT_DEFAULT_FEES: Fees = Fees {
    base: 10,
    reserve: 10_000_000,  // 10 XRP — matches rippled FeeSetup::accountReserve
    increment: 2_000_000, // 2 XRP — matches rippled FeeSetup::ownerReserve
};

pub type LedgerTxRead = (Arc<STTx>, TxMeta);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerTxReadError {
    Traversal(TraversalError),
    Decode(String),
}

impl From<TraversalError> for LedgerTxReadError {
    fn from(value: TraversalError) -> Self {
        Self::Traversal(value)
    }
}

pub trait LedgerJournal: Send + Sync + std::fmt::Debug + 'static {
    fn debug(&self, _message: &str) {}
    fn info(&self, message: &str);
    fn warn(&self, message: &str);
}

#[derive(Debug, Default)]
pub struct NullLedgerJournal;

impl LedgerJournal for NullLedgerJournal {
    fn info(&self, _message: &str) {}
    fn warn(&self, _message: &str) {}
}

pub trait LedgerInfoProvider {
    fn get_ledger_info_by_index(&self, ledger_index: u32) -> Option<LedgerHeader>;
    fn get_ledger_info_by_hash(&self, ledger_hash: SHAMapHash) -> Option<LedgerHeader>;
    fn get_newest_ledger_info(&self) -> Option<LedgerHeader>;
}

pub fn is_flag_ledger(seq: u32) -> bool {
    seq.is_multiple_of(FLAG_LEDGER_INTERVAL)
}

pub fn is_voting_ledger(seq: u32) -> bool {
    seq.is_multiple_of(FLAG_LEDGER_INTERVAL)
}

pub fn get_enabled_amendments(ledger: &Ledger) -> BTreeSet<Uint256> {
    ledger
        .state_map
        .peek_item_with_hash(amendments_key(), &mut |_| None)
        .ok()
        .flatten()
        .map(|(item, _hash)| item)
        .and_then(|item| {
            state_map::parse_amendments_sle(item.data()).map(|entry| {
                if entry.is_field_present(get_field_by_symbol("sfAmendments")) {
                    entry
                        .get_field_v256(get_field_by_symbol("sfAmendments"))
                        .value()
                        .iter()
                        .copied()
                        .collect()
                } else {
                    BTreeSet::new()
                }
            })
        })
        .or_else(|| {
            ledger
                .state_map
                .peek_item_with_hash(amendments_key(), &mut |_| None)
                .ok()
                .flatten()
                .and_then(|(item, _hash)| protocol::decode_amendments_entry(item.data()).ok())
                .map(|entry| entry.amendments.into_iter().collect())
        })
        .unwrap_or_default()
}

pub fn get_majority_amendments(ledger: &Ledger) -> BTreeMap<Uint256, NetClockTimePoint> {
    ledger
        .state_map
        .peek_item_with_hash(amendments_key(), &mut |_| None)
        .ok()
        .flatten()
        .map(|(item, _hash)| item)
        .and_then(|item| {
            state_map::parse_amendments_sle(item.data()).map(|entry| {
                if !entry.is_field_present(get_field_by_symbol("sfMajorities")) {
                    return BTreeMap::new();
                }

                entry
                    .get_field_array(get_field_by_symbol("sfMajorities"))
                    .iter()
                    .map(|majority| {
                        (
                            majority.get_field_h256(get_field_by_symbol("sfAmendment")),
                            NetClockTimePoint::from(
                                majority.get_field_u32(get_field_by_symbol("sfCloseTime")),
                            ),
                        )
                    })
                    .collect()
            })
        })
        .or_else(|| {
            ledger
                .state_map
                .peek_item_with_hash(amendments_key(), &mut |_| None)
                .ok()
                .flatten()
                .and_then(|(item, _hash)| protocol::decode_amendments_entry(item.data()).ok())
                .map(|entry| {
                    entry
                        .majorities
                        .into_iter()
                        .map(|majority| {
                            (
                                majority.amendment,
                                NetClockTimePoint::from(majority.close_time),
                            )
                        })
                        .collect()
                })
        })
        .unwrap_or_default()
}

fn decode_negative_unl_public_key(bytes: &[u8]) -> Option<[u8; 33]> {
    let key: [u8; 33] = bytes.try_into().ok()?;
    Some(key)
}

fn decode_tx_node(
    ledger_seq: u32,
    node: &shamap::tree_node::SHAMapTreeNode,
) -> Result<Option<LedgerTxRead>, LedgerTxReadError> {
    if !node.is_leaf() {
        return Ok(None);
    }
    if node.get_type() != SHAMapNodeType::TransactionMd {
        return Err(LedgerTxReadError::Decode(
            "closed ledger transaction reads require metadata payloads".to_string(),
        ));
    }
    let Some(item) = node.peek_item() else {
        return Ok(None);
    };
    Ok(Some(decode_tx_item(ledger_seq, &item)?))
}

fn decode_tx_item(ledger_seq: u32, item: &SHAMapItem) -> Result<LedgerTxRead, LedgerTxReadError> {
    let (tx_bytes, meta_bytes) = split_transaction_with_meta(item.data())?;
    let tx = parse_sttx(&tx_bytes)?;
    let meta = parse_tx_meta(item.key(), ledger_seq, &meta_bytes)?;
    Ok((tx, meta))
}

fn split_transaction_with_meta(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>), LedgerTxReadError> {
    catch_unwind(AssertUnwindSafe(|| {
        let mut serial = SerialIter::new(bytes);
        (serial.get_vl(), serial.get_vl())
    }))
    .map_err(|payload| {
        LedgerTxReadError::Decode(
            unwind_message(payload)
                .unwrap_or_else(|| "failed to split transaction-with-meta payload".to_string()),
        )
    })
}

fn parse_sttx(bytes: &[u8]) -> Result<Arc<STTx>, LedgerTxReadError> {
    catch_unwind(AssertUnwindSafe(|| {
        let mut serial = SerialIter::new(bytes);
        Arc::new(STTx::from_serial_iter(&mut serial))
    }))
    .map_err(|payload| {
        LedgerTxReadError::Decode(
            unwind_message(payload).unwrap_or_else(|| "failed to parse STTx".into()),
        )
    })
}

fn parse_tx_meta(
    transaction_id: Uint256,
    ledger_seq: u32,
    bytes: &[u8],
) -> Result<TxMeta, LedgerTxReadError> {
    catch_unwind(AssertUnwindSafe(|| {
        TxMeta::from_raw(transaction_id, ledger_seq, bytes)
    }))
    .map_err(|payload| {
        LedgerTxReadError::Decode(
            unwind_message(payload).unwrap_or_else(|| "failed to parse TxMeta".into()),
        )
    })
}

fn unwind_message(payload: Box<dyn std::any::Any + Send>) -> Option<String> {
    match payload.downcast::<String>() {
        Ok(message) => Some(*message),
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => Some((*message).to_string()),
            Err(_) => None,
        },
    }
}

/// Operation type for batched state map mutations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateBatchOp {
    Insert,
    Update,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerNodeObjectType {
    AccountNode,
    TransactionNode,
}

#[derive(Clone)]
pub struct Ledger {
    header: LedgerHeader,
    state_map: SyncTree,
    tx_map: SyncTree,
    fees: Fees,
    rules: Rules,
    immutable: bool,
    /// Optional node fetcher for backed reads from the node store.
    node_fetcher: Option<
        Arc<
            dyn Fn(
                    basics::sha_map_hash::SHAMapHash,
                ) -> Option<
                    basics::memory::intrusive_pointer::SharedIntrusive<
                        shamap::nodes::tree_node::SHAMapTreeNode,
                    >,
                > + Send
                + Sync,
        >,
    >,
    /// Optional node writer for flushing dirty nodes to the node store.
    node_writer: Option<
        Arc<dyn Fn(LedgerNodeObjectType, basics::base_uint::Uint256, Vec<u8>, u32) + Send + Sync>,
    >,
    /// Persistent mutable tree for the state map — matches reference where stateMap_
    /// is a single persistent SHAMap that all rawInsert/rawErase/rawReplace
    /// operate on directly. Initialized on first mutation, persists across all
    /// operations until set_immutable extracts the final root.
    mutable_state: Option<MutableTree>,
}

impl std::fmt::Debug for Ledger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ledger")
            .field("seq", &self.header.seq)
            .field("immutable", &self.immutable)
            .finish_non_exhaustive()
    }
}

impl Ledger {
    pub fn new(header: LedgerHeader, backed: bool) -> Self {
        Self {
            header,
            state_map: SyncTree::new_with_type(SHAMapType::State, backed, header.seq),
            tx_map: SyncTree::new_with_type(SHAMapType::Transaction, backed, header.seq),
            fees: Fees::default(),
            rules: Rules::default(),
            immutable: false,
            node_fetcher: None,
            node_writer: None,
            mutable_state: None,
        }
    }

    pub fn from_ledger_seq_and_close_time(ledger_seq: u32, close_time: u32, backed: bool) -> Self {
        Self::new(
            LedgerHeader {
                seq: ledger_seq,
                close_time,
                close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
                ..LedgerHeader::default()
            },
            backed,
        )
    }

    pub fn create_genesis_setup_only<I>(
        backed: bool,
        config: &LedgerConfig,
        amendments: I,
    ) -> Result<Self, MutationError>
    where
        I: IntoIterator<Item = Uint256>,
    {
        let items = build_genesis_setup_items(config, amendments);
        let mut tree = MutableTree::new(1);

        for (key, payload) in items {
            tree.add_item(SHAMapNodeType::AccountState, SHAMapItem::new(key, payload))?;
        }

        let state_map = SyncTree::from_root_with_type(
            tree.root(),
            SHAMapType::State,
            backed,
            1,
            SyncState::Modifying,
        );

        let mut ledger = Self::from_maps(
            LedgerHeader {
                seq: 1,
                drops: INITIAL_XRP_DROPS,
                close_time_resolution: LEDGER_GENESIS_TIME_RESOLUTION,
                ..LedgerHeader::default()
            },
            state_map,
            SyncTree::new_with_type(SHAMapType::Transaction, backed, 1),
        );
        ledger.rules = Rules::new(config.features.iter());
        ledger.set_immutable(true);
        Ok(ledger)
    }

    pub fn create_genesis<I>(
        backed: bool,
        config: &LedgerConfig,
        amendments: I,
    ) -> Result<Self, MutationError>
    where
        I: IntoIterator<Item = Uint256>,
    {
        let items = build_genesis_state_items(config, amendments, INITIAL_XRP_DROPS);
        let mut tree = MutableTree::new(1);

        for (key, payload) in items {
            tree.add_item(SHAMapNodeType::AccountState, SHAMapItem::new(key, payload))?;
        }

        let state_map = SyncTree::from_root_with_type(
            tree.root(),
            SHAMapType::State,
            backed,
            1,
            SyncState::Modifying,
        );

        let mut ledger = Self::from_maps(
            LedgerHeader {
                seq: 1,
                drops: INITIAL_XRP_DROPS,
                close_time_resolution: LEDGER_GENESIS_TIME_RESOLUTION,
                ..LedgerHeader::default()
            },
            state_map,
            SyncTree::new_with_type(SHAMapType::Transaction, backed, 1),
        );
        ledger.rules = Rules::new(config.features.iter());
        ledger.set_immutable(true);
        Ok(ledger)
    }

    pub fn from_ledger_seq_and_close_time_with_setup<I>(
        ledger_seq: u32,
        close_time: u32,
        backed: bool,
        default_fees: Fees,
        preset_features: I,
        feature_xrp_fees: &Uint256,
    ) -> Result<Self, LedgerSetupError>
    where
        I: IntoIterator<Item = Uint256>,
    {
        let mut ledger = Self::from_ledger_seq_and_close_time(ledger_seq, close_time, backed);
        ledger.rules = Rules::new(preset_features);
        ledger.apply_default_fees(default_fees);
        let _ = ledger.setup_from_state_map(feature_xrp_fees)?;
        Ok(ledger)
    }

    pub fn from_ledger_seq_and_close_time_with_config(
        ledger_seq: u32,
        close_time: u32,
        backed: bool,
        config: &LedgerConfig,
    ) -> Result<Self, LedgerSetupError> {
        Self::from_ledger_seq_and_close_time_with_setup(
            ledger_seq,
            close_time,
            backed,
            config.fees,
            config.features.iter(),
            &feature_xrp_fees(),
        )
    }

    pub fn from_maps(header: LedgerHeader, mut state_map: SyncTree, mut tx_map: SyncTree) -> Self {
        debug_assert_eq!(state_map.map_type(), SHAMapType::State);
        debug_assert_eq!(tx_map.map_type(), SHAMapType::Transaction);
        state_map.set_ledger_seq(header.seq);
        tx_map.set_ledger_seq(header.seq);
        Self {
            header,
            state_map,
            tx_map,
            fees: Fees::default(),
            rules: Rules::default(),
            immutable: false,
            node_fetcher: None,
            node_writer: None,
            mutable_state: None,
        }
    }

    pub fn from_previous(prev_ledger: &Self, close_time: u32) -> Self {
        let next_seq = prev_ledger.header.seq.wrapping_add(1);
        let close_time_resolution = get_next_ledger_time_resolution(
            prev_ledger.header.close_time_resolution,
            get_close_agree(&prev_ledger.header),
            next_seq,
        );
        let close_time = if prev_ledger.header.close_time == 0 {
            round_close_time(close_time, close_time_resolution)
        } else {
            prev_ledger.header.close_time + u32::from(close_time_resolution)
        };

        Self {
            header: LedgerHeader {
                seq: next_seq,
                drops: prev_ledger.header.drops,
                hash: increment_hash(prev_ledger.header.hash),
                parent_hash: prev_ledger.header.hash,
                parent_close_time: prev_ledger.header.close_time,
                close_time,
                close_time_resolution,
                ..LedgerHeader::default()
            },
            state_map: {
                let sm = prev_ledger.state_map.share_root_snapshot();
                // Inherit the parent's backed flag. When the parent is backed
                // (nodes persisted to NuDB) and has a node_fetcher, the child
                // must also be backed so that read operations (e.g.
                // update_skip_list) can resolve nodes via the fetcher after
                // release_maps_to_disk has evicted in-memory children.
                // Mutations go through MutableTree which always passes
                // backed=true and its own fetch callback independently of
                // this flag, so COW is not affected.
                sm
            },
            tx_map: SyncTree::new_with_type(SHAMapType::Transaction, true, 0),
            fees: prev_ledger.fees,
            rules: prev_ledger.rules.clone(),
            immutable: false,
            node_fetcher: prev_ledger.node_fetcher.clone(),
            node_writer: prev_ledger.node_writer.clone(),
            mutable_state: None,
        }
    }

    pub fn from_header_hashes(mut header: LedgerHeader) -> Self {
        header.hash = calculate_ledger_hash(&header);

        Self {
            state_map: SyncTree::new_synching_with_type(SHAMapType::State, true, header.seq),
            tx_map: SyncTree::new_synching_with_type(SHAMapType::Transaction, true, header.seq),
            fees: Fees::default(),
            rules: Rules::default(),
            header,
            immutable: true,
            node_fetcher: None,
            node_writer: None,
            mutable_state: None,
        }
    }

    pub fn from_header_hashes_with_config(header: LedgerHeader, config: &LedgerConfig) -> Self {
        let mut ledger = Self::from_header_hashes(header);
        ledger.rules = Rules::new(config.features.iter());
        ledger
    }

    pub fn load_immutable_with_family<CLOCK, S, FB, F, MR, NS, J>(
        header: LedgerHeader,
        acquire: bool,
        journal: &J,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> (Self, bool)
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        let mut ledger = Self {
            header,
            state_map: SyncTree::new_with_type(SHAMapType::State, true, header.seq),
            tx_map: SyncTree::new_with_type(SHAMapType::Transaction, true, header.seq),
            fees: Fees::default(),
            rules: Rules::default(),
            immutable: true,
            node_fetcher: None,
            node_writer: None,
            mutable_state: None,
        };
        let mut loaded = true;
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;

        if ledger.header.tx_hash.is_non_zero()
            && !ledger
                .tx_map
                .fetch_root_with_family(ledger.header.tx_hash, &mut no_filter, family)
        {
            loaded = false;
            journal.warn(&format!(
                "Don't have transaction root for ledger{}",
                ledger.header.seq
            ));
        }

        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        if ledger.header.account_hash.is_non_zero()
            && !ledger.state_map.fetch_root_with_family(
                ledger.header.account_hash,
                &mut no_filter,
                family,
            )
        {
            loaded = false;
            journal.warn(&format!(
                "Don't have state data root for ledger{}",
                ledger.header.seq
            ));
        }

        ledger.tx_map.set_immutable();
        ledger.state_map.set_immutable();

        if !loaded {
            ledger.header.hash = calculate_ledger_hash(&ledger.header);
            if acquire {
                family.missing_node_acquire_by_hash(
                    *ledger.header.hash.as_uint256(),
                    ledger.header.seq,
                );
            }
        }

        (ledger, loaded)
    }

    pub fn load_immutable_with_family_and_setup<CLOCK, S, FB, F, MR, NS, J>(
        header: LedgerHeader,
        acquire: bool,
        journal: &J,
        default_fees: Fees,
        feature_xrp_fees: &Uint256,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<(Self, bool), LedgerSetupError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        let (mut ledger, mut loaded) =
            Self::load_immutable_with_family(header, acquire, journal, family);
        let setup_loaded = ledger.setup_from_state_map_with_default_fees_and_family(
            default_fees,
            feature_xrp_fees,
            family,
        )?;

        if loaded && !setup_loaded {
            loaded = false;
            ledger.header.hash = calculate_ledger_hash(&ledger.header);
            if acquire {
                family.missing_node_acquire_by_hash(
                    *ledger.header.hash.as_uint256(),
                    ledger.header.seq,
                );
            }
        }

        Ok((ledger, loaded))
    }

    pub fn load_immutable_with_family_and_config<CLOCK, S, FB, F, MR, NS, J>(
        header: LedgerHeader,
        acquire: bool,
        journal: &J,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<(Self, bool), LedgerSetupError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        let (mut ledger, mut loaded) =
            Self::load_immutable_with_family(header, acquire, journal, family);
        ledger.rules = Rules::new(config.features.iter());
        ledger.apply_default_fees(config.fees);
        let setup_loaded = ledger.setup_from_state_map_with_family(&feature_xrp_fees(), family)?;

        if loaded && !setup_loaded {
            loaded = false;
            ledger.header.hash = calculate_ledger_hash(&ledger.header);
            if acquire {
                family.missing_node_acquire_by_hash(
                    *ledger.header.hash.as_uint256(),
                    ledger.header.seq,
                );
            }
        }

        Ok((ledger, loaded))
    }

    pub fn load_immutable_with_family_and_config_or_none<CLOCK, S, FB, F, MR, NS, J>(
        header: LedgerHeader,
        acquire: bool,
        journal: &J,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<Option<Self>, LedgerSetupError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        let (ledger, loaded) =
            Self::load_immutable_with_family_and_config(header, acquire, journal, config, family)?;
        Ok(loaded.then_some(ledger))
    }

    pub fn load_finished_with_family_and_config_or_none<CLOCK, S, FB, F, MR, NS, J>(
        header: LedgerHeader,
        acquire: bool,
        journal: &J,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<Option<Self>, LedgerSetupError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        let mut ledger = match Self::load_immutable_with_family_and_config_or_none(
            header, acquire, journal, config, family,
        )? {
            Some(ledger) => ledger,
            None => return Ok(None),
        };

        ledger.finish_load_by_index_or_hash(journal)?;
        Ok(Some(ledger))
    }

    pub fn load_finished_by_hash_with_family_and_config_or_none<CLOCK, S, FB, F, MR, NS, J>(
        expected_hash: SHAMapHash,
        header: LedgerHeader,
        acquire: bool,
        journal: &J,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<Option<Self>, LedgerSetupError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        let ledger = Self::load_finished_with_family_and_config_or_none(
            header, acquire, journal, config, family,
        )?;
        assert!(
            ledger
                .as_ref()
                .is_none_or(|ledger| ledger.header.hash == expected_hash),
            "xrpl::loadByHash : ledger hash match if loaded"
        );
        Ok(ledger)
    }

    pub fn load_by_index_with_provider_and_config_or_none<P, CLOCK, S, FB, F, MR, NS, J>(
        ledger_index: u32,
        acquire: bool,
        journal: &J,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
        provider: &P,
    ) -> Result<Option<Self>, LedgerSetupError>
    where
        P: LedgerInfoProvider,
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        let Some(header) = provider.get_ledger_info_by_index(ledger_index) else {
            return Ok(None);
        };

        Self::load_finished_with_family_and_config_or_none(header, acquire, journal, config, family)
    }

    pub fn load_by_hash_with_provider_and_config_or_none<P, CLOCK, S, FB, F, MR, NS, J>(
        expected_hash: SHAMapHash,
        acquire: bool,
        journal: &J,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
        provider: &P,
    ) -> Result<Option<Self>, LedgerSetupError>
    where
        P: LedgerInfoProvider,
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        let Some(header) = provider.get_ledger_info_by_hash(expected_hash) else {
            return Ok(None);
        };

        Self::load_finished_by_hash_with_family_and_config_or_none(
            expected_hash,
            header,
            acquire,
            journal,
            config,
            family,
        )
    }

    pub fn get_latest_ledger_with_provider_and_config<P, CLOCK, S, FB, F, MR, NS, J>(
        journal: &J,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
        provider: &P,
    ) -> Result<(Option<Self>, u32, SHAMapHash), LedgerSetupError>
    where
        P: LedgerInfoProvider,
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        let Some(header) = provider.get_newest_ledger_info() else {
            return Ok((None, 0, SHAMapHash::default()));
        };

        let ledger = Self::load_immutable_with_family_and_config_or_none(
            header, true, journal, config, family,
        )?;
        Ok((ledger, header.seq, header.hash))
    }

    pub fn header(&self) -> LedgerHeader {
        self.header
    }

    pub fn set_ledger_info(&mut self, header: LedgerHeader) {
        self.header = header;
    }

    pub fn set_total_drops(&mut self, drops: u64) {
        self.header.drops = drops;
    }

    pub fn state_map(&self) -> &SyncTree {
        &self.state_map
    }

    pub fn state_map_mut(&mut self) -> &mut SyncTree {
        &mut self.state_map
    }

    pub fn tx_map(&self) -> &SyncTree {
        &self.tx_map
    }

    pub fn tx_map_mut(&mut self) -> &mut SyncTree {
        &mut self.tx_map
    }

    /// Release all in-memory tree nodes from both state and transaction maps.
    ///
    /// Takes `&self` — operates via interior mutability (per-branch spinlocks
    /// on SHAMapTreeNode). This means it works on `Arc<Ledger>` directly,
    /// so ALL holders of this ledger (closed/validated/published slots, history
    /// cache, consensus state) automatically see the released tree without any
    /// slot-swapping or cloning.
    ///
    /// After this call, all SHAMap reads go through the `node_fetcher` to NuDB
    /// on demand. The tree's identity (root hash, branch topology) is preserved.
    ///
    /// Call AFTER the ledger's nodes are confirmed durable in NuDB (either via
    /// the acquisition worker's store_object path, or persist_dirty_nodes_to_store).
    pub fn release_maps_to_disk(&self) {
        self.state_map.release_to_disk();
        self.tx_map.release_to_disk();
    }

    pub fn needed_tx_hashes_with_family<CLOCK, S, C, F, MR, NS>(
        &mut self,
        max: i32,
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Vec<Uint256>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        ledger_fetcher::needed_hashes_with_family(
            self.header.tx_hash,
            &mut self.tx_map,
            max,
            filter,
            family,
        )
    }

    pub fn needed_state_hashes_with_family<CLOCK, S, C, F, MR, NS>(
        &mut self,
        max: i32,
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Vec<Uint256>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        ledger_fetcher::needed_hashes_with_family(
            self.header.account_hash,
            &mut self.state_map,
            max,
            filter,
            family,
        )
    }

    pub fn fees(&self) -> Fees {
        self.fees
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    pub fn set_fees(&mut self, fees: Fees) {
        self.fees = fees;
    }

    pub fn set_rules(&mut self, rules: Rules) {
        self.rules = rules;
    }

    /// Set the node fetcher for backed reads from the node store.
    pub fn set_node_fetcher(
        &mut self,
        fetcher: Arc<
            dyn Fn(
                    basics::sha_map_hash::SHAMapHash,
                ) -> Option<
                    basics::memory::intrusive_pointer::SharedIntrusive<
                        shamap::nodes::tree_node::SHAMapTreeNode,
                    >,
                > + Send
                + Sync,
        >,
    ) {
        self.node_fetcher = Some(fetcher);
        let _ = if self.fees.base == 0 && self.fees.reserve == 0 && self.fees.increment == 0 {
            self.setup_from_state_map_with_default_fees(CURRENT_DEFAULT_FEES, &feature_xrp_fees())
        } else {
            self.setup_from_state_map(&feature_xrp_fees())
        };
    }

    pub fn has_node_fetcher(&self) -> bool {
        self.node_fetcher.is_some()
    }

    /// Returns a clone of the node fetcher closure (if attached).
    /// Used by consensus to pin state map nodes in memory before building.
    pub fn node_fetcher_closure(
        &self,
    ) -> Option<
        Arc<
            dyn Fn(
                    basics::sha_map_hash::SHAMapHash,
                ) -> Option<
                    basics::memory::intrusive_pointer::SharedIntrusive<
                        shamap::nodes::tree_node::SHAMapTreeNode,
                    >,
                > + Send
                + Sync,
        >,
    > {
        self.node_fetcher.clone()
    }

    pub fn has_node_writer(&self) -> bool {
        self.node_writer.is_some()
    }

    /// Returns true if the state map root is zero (empty state map).
    /// In Rust, this can happen when a consensus-built ledger is used as
    /// parent but its state map was never populated with real account state.
    pub fn state_map_root_is_zero(&self) -> bool {
        self.state_map.root().get_hash().as_uint256().is_zero()
    }

    /// Load the real state root from NuDB using the given account_hash.
    /// accessible via the Family. This method recovers that invariant.
    pub fn try_load_state_root_from_fetcher(
        &mut self,
        account_hash: basics::sha_map_hash::SHAMapHash,
    ) {
        let Some(fetcher) = &self.node_fetcher else {
            return;
        };
        let Some(root_node) = fetcher(account_hash) else {
            return;
        };
        let seq = self.header.seq;
        let backed = self.state_map.backed();
        let new_map = shamap::sync::SyncTree::from_root_with_type(
            root_node,
            shamap::sync::SHAMapType::State,
            backed,
            seq,
            shamap::sync::SyncState::Immutable,
        );
        self.state_map = new_map;
    }

    pub fn is_immutable(&self) -> bool {
        self.immutable
    }

    pub fn apply_default_fees(&mut self, defaults: Fees) {
        // Only apply defaults if fees haven't been set yet (e.g. before setup() reads FeeSettings SLE)
        if self.fees.base == 0 {
            self.fees.base = defaults.base;
        }
        if self.fees.reserve == 0 {
            self.fees.reserve = defaults.reserve;
        }
        if self.fees.increment == 0 {
            self.fees.increment = defaults.increment;
        }
    }

    pub fn exists_keylet(&self, keylet: Keylet) -> Result<bool, TraversalError> {
        let mut fetch_fn = |hash: basics::sha_map_hash::SHAMapHash| -> Option<
            basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >,
        > {
            let fetcher = self.node_fetcher.as_ref()?;
            fetcher(hash)
        };
        self.state_map.has_item(keylet.key, &mut fetch_fn)
    }

    pub fn exists(&self, key: Uint256) -> Result<bool, TraversalError> {
        let mut fetch_fn = |hash: basics::sha_map_hash::SHAMapHash| -> Option<
            basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >,
        > {
            let fetcher = self.node_fetcher.as_ref()?;
            fetcher(hash)
        };
        self.state_map.has_item(key, &mut fetch_fn)
    }

    pub fn tx_exists(&self, key: Uint256) -> bool {
        self.tx_map.has_item(key, &mut |_| None).unwrap_or(false)
    }

    pub fn tx_read(&self, key: Uint256) -> Result<Option<LedgerTxRead>, LedgerTxReadError> {
        let Some(node) = self.tx_map.find_key(key, &mut |_| None)? else {
            return Ok(None);
        };
        let Some(decoded) = decode_tx_node(self.header.seq, &node)? else {
            return Ok(None);
        };

        Ok((decoded.0.get_transaction_id() == key).then_some(decoded))
    }

    pub fn tx_snapshot(&self) -> Result<Vec<LedgerTxRead>, LedgerTxReadError> {
        let mut stack: Vec<NodePathEntry> = Vec::new();
        let mut snapshot = Vec::new();
        let mut current = self.tx_map.peek_first_item(&mut stack, &mut |_| None)?;

        while let Some(node) = current {
            if !node.is_leaf() {
                break;
            }

            let item = node
                .peek_item()
                .expect("ledger tx snapshot leaf nodes should carry an item");
            if let Some(decoded) = decode_tx_node(self.header.seq, &node)? {
                snapshot.push(decoded);
            }
            current = self
                .tx_map
                .peek_next_item(item.key(), &mut stack, &mut |_| None)?;
        }

        Ok(snapshot)
    }

    pub fn tx_snapshot_with_family<CLOCK, S, FB, F, MR, NS>(
        &self,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<Vec<LedgerTxRead>, LedgerTxReadError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        let mut stack: Vec<NodePathEntry> = Vec::new();
        let mut snapshot = Vec::new();
        let mut current = self
            .tx_map
            .peek_first_item_with_family(&mut stack, family)?;

        while let Some(node) = current {
            if !node.is_leaf() {
                break;
            }

            let item = node
                .peek_item()
                .expect("ledger tx snapshot leaf nodes should carry an item");
            snapshot.push(decode_tx_item(self.header.seq, &item)?);
            current = self
                .tx_map
                .peek_next_item_with_family(item.key(), &mut stack, family)?;
        }

        Ok(snapshot)
    }

    pub fn succ(
        &self,
        key: Uint256,
        last: Option<Uint256>,
    ) -> Result<Option<Uint256>, TraversalError> {
        let mut fetch_fn = |hash: basics::sha_map_hash::SHAMapHash| -> Option<
            basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >,
        > {
            let fetcher = self.node_fetcher.as_ref()?;
            fetcher(hash)
        };
        let Some(leaf) = self.state_map.upper_bound(key, &mut fetch_fn)? else {
            return Ok(None);
        };
        let Some(item) = leaf.peek_item() else {
            return Ok(None);
        };
        if last.is_some_and(|last| item.key() >= last) {
            return Ok(None);
        }
        Ok(Some(item.key()))
    }

    pub fn succ_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        key: Uint256,
        last: Option<Uint256>,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<Option<Uint256>, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        let Some(leaf) = self.state_map.upper_bound_with_family(key, family)? else {
            return Ok(None);
        };
        let Some(item) = leaf.peek_item() else {
            return Ok(None);
        };
        if last.is_some_and(|last| item.key() >= last) {
            return Ok(None);
        }
        Ok(Some(item.key()))
    }

    pub fn read(&self, keylet: Keylet) -> Result<Option<STLedgerEntry>, TraversalError> {
        if keylet.key == Uint256::zero() {
            return Ok(None);
        }

        let mut fetch_fn = |hash: basics::sha_map_hash::SHAMapHash| -> Option<
            basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >,
        > {
            let fetcher = self.node_fetcher.as_ref()?;
            let fetched = fetcher(hash);
            full_sync_debug!(
                "[full_debug][ledger_node_fetch] seq={} map=state hash={} result={}",
                self.header.seq,
                hash,
                if fetched.is_some() { "hit" } else { "miss" }
            );
            fetched
        };

        full_sync_debug!(
            "[full_debug][ledger_read] start seq={} key={} type={:?} backed={} full={} fetcher={} writer={} fees_base={} reserve={} inc={}",
            self.header.seq,
            keylet.key,
            keylet.entry_type,
            self.state_map.backed(),
            self.state_map.is_full(),
            self.node_fetcher.is_some(),
            self.node_writer.is_some(),
            self.fees.base,
            self.fees.reserve,
            self.fees.increment
        );
        let result = self.state_map.peek_item(keylet.key, &mut fetch_fn);
        match &result {
            Err(e) => {
                let seq = self.header.seq;
                tracing::debug!(target: "ledger", seq, "Failed to read ledger entry from DB");
                static READ_ERR_LOG: std::sync::atomic::AtomicU32 =
                    std::sync::atomic::AtomicU32::new(0);
                if READ_ERR_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 8 {
                    let thread = std::thread::current();
                    let thread_name = thread.name().unwrap_or("<unnamed>");
                    let self_ptr = self as *const Self as usize;
                    // Capture a short backtrace to identify the calling code path.
                    // Requires RUST_BACKTRACE=1 (or `=full`) in the environment.
                    let bt = std::backtrace::Backtrace::capture();
                    tracing::error!(
                        target: "ledger",
                        "[ledger_read] ERROR seq={} ledger_ptr=0x{:x} thread={:?}({}) key={:02x}{:02x}{:02x}{:02x} entry_type={:?} err={:?} backed={} has_fetcher={} state_is_full={} state_state={:?} mutable={}\n  backtrace:\n{}",
                        self.header.seq,
                        self_ptr,
                        thread.id(),
                        thread_name,
                        keylet.key.data()[0],
                        keylet.key.data()[1],
                        keylet.key.data()[2],
                        keylet.key.data()[3],
                        keylet.entry_type,
                        e,
                        self.state_map.backed(),
                        self.node_fetcher.is_some(),
                        self.state_map.is_full(),
                        self.state_map.state(),
                        self.mutable_state.is_some(),
                        bt,
                    );
                }
            }
            Ok(None) => {
                // Log misses for AccountRoot and DirectoryNode (key types we care about for parity)
                let log_this = keylet.entry_type == protocol::LedgerEntryType::AccountRoot
                    || keylet.entry_type == protocol::LedgerEntryType::DirectoryNode;
                if log_this {
                    static READ_MISS_LOG: std::sync::atomic::AtomicU32 =
                        std::sync::atomic::AtomicU32::new(0);
                    if READ_MISS_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 20 {
                        let thread = std::thread::current();
                        let thread_name = thread.name().unwrap_or("<unnamed>");
                        let self_ptr = self as *const Self as usize;
                        tracing::debug!(
                            target: "ledger",
                            "[ledger_read] MISS {:?} seq={} ledger_ptr=0x{:x} thread={:?}({}) key={:02x}{:02x}{:02x}{:02x} backed={} has_fetcher={}",
                            keylet.entry_type,
                            self.header.seq,
                            self_ptr,
                            thread.id(),
                            thread_name,
                            keylet.key.data()[0],
                            keylet.key.data()[1],
                            keylet.key.data()[2],
                            keylet.key.data()[3],
                            self.state_map.backed(),
                            self.node_fetcher.is_some(),
                        );
                    }
                }
            }
            _ => {}
        }
        full_sync_debug!(
            "[full_debug][ledger_read] done seq={} key={} type={:?} result={}",
            self.header.seq,
            keylet.key,
            keylet.entry_type,
            match &result {
                Ok(Some(_)) => "hit",
                Ok(None) => "miss",
                Err(_) => "error",
            }
        );

        let Some(item) = result? else {
            return Ok(None);
        };

        Ok(parse_state_sle(item.data(), keylet))
    }

    pub fn read_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        keylet: Keylet,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<Option<STLedgerEntry>, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        assert!(
            keylet.key != Uint256::zero(),
            "xrpl::Ledger::read_with_family : zero key"
        );

        let Some(item) = self.state_map.peek_item_with_family(keylet.key, family)? else {
            return Ok(None);
        };

        Ok(parse_state_sle(item.data(), keylet))
    }

    pub fn peek(&self, keylet: Keylet) -> Result<Option<STLedgerEntry>, TraversalError> {
        self.read(keylet)
    }

    pub fn digest(&self, key: Uint256) -> Result<Option<Uint256>, TraversalError> {
        Ok(self
            .state_map
            .peek_item_with_hash(key, &mut |_| None)?
            .map(|(_item, hash)| *hash.as_uint256()))
    }

    pub fn visit_state_sles<V>(&self, visit: &mut V) -> Result<(), TraversalError>
    where
        V: FnMut(&STLedgerEntry),
    {
        self.visit_state_sles_while(&mut |sle| {
            visit(sle);
            true
        })
    }

    pub fn visit_state_sles_while<V>(&self, visit: &mut V) -> Result<(), TraversalError>
    where
        V: FnMut(&STLedgerEntry) -> bool,
    {
        self.state_map.visit_nodes(&mut |_| None, &mut |node| {
            if !node.is_leaf() {
                return true;
            }

            let item = node.peek_item().expect("leaf nodes should carry an item");
            match parse_state_sle_any(item.data(), item.key()) {
                Some(sle) => visit(&sle),
                None => true,
            }
        })
    }

    pub fn visit_state_sles_with_family<CLOCK, S, C, F, MR, NS, V>(
        &self,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        visit: &mut V,
    ) -> Result<(), TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        V: FnMut(&STLedgerEntry),
    {
        self.visit_state_sles_with_family_while(family, &mut |sle| {
            visit(sle);
            true
        })
    }

    pub fn visit_state_sles_with_family_while<CLOCK, S, C, F, MR, NS, V>(
        &self,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        visit: &mut V,
    ) -> Result<(), TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        V: FnMut(&STLedgerEntry) -> bool,
    {
        self.state_map.visit_nodes_with_family(family, &mut |node| {
            if !node.is_leaf() {
                return true;
            }

            let item = node.peek_item().expect("leaf nodes should carry an item");
            match parse_state_sle_any(item.data(), item.key()) {
                Some(sle) => visit(&sle),
                None => true,
            }
        })
    }

    pub fn setup_with_entries(
        &mut self,
        entries: &LedgerSetupEntries,
        feature_xrp_fees: &Uint256,
    ) -> bool {
        let mut ret = true;

        match &entries.amendments {
            SetupLookup::MissingNode => ret = false,
            SetupLookup::MissingObject => {
                self.rules =
                    make_rules_given_current(&self.rules, None, None::<std::iter::Empty<Uint256>>);
            }
            SetupLookup::Present(entry) => {
                self.rules = make_rules_given_current(
                    &self.rules,
                    Some(entry.digest),
                    Some(entry.amendments.iter().copied()),
                );
            }
        }

        match &entries.fees {
            SetupLookup::MissingNode => ret = false,
            SetupLookup::MissingObject => {}
            SetupLookup::Present(fees) => {
                let mut old_fees = false;

                if let Some(base_fee) = fees.base_fee {
                    self.fees.base = base_fee;
                    old_fees = true;
                }
                if let Some(reserve_base) = fees.reserve_base {
                    self.fees.reserve = u64::from(reserve_base);
                    old_fees = true;
                }
                if let Some(reserve_increment) = fees.reserve_increment {
                    self.fees.increment = u64::from(reserve_increment);
                    old_fees = true;
                }

                let mut assign_amount = |dest: &mut u64, src: Option<AmountField>| {
                    if let Some(src) = src {
                        if src.native && !src.negative {
                            *dest = src.drops;
                        } else {
                            ret = false;
                        }
                    }
                };

                assign_amount(&mut self.fees.base, fees.base_fee_drops);
                assign_amount(&mut self.fees.reserve, fees.reserve_base_drops);
                assign_amount(&mut self.fees.increment, fees.reserve_increment_drops);

                let new_fees = fees.base_fee_drops.is_some()
                    || fees.reserve_base_drops.is_some()
                    || fees.reserve_increment_drops.is_some();

                if old_fees && new_fees {
                    ret = false;
                }
                if !self.rules.enabled(feature_xrp_fees) && new_fees {
                    ret = false;
                }
            }
        }

        ret
    }

    pub fn read_setup_entries_from_state_map(
        &self,
    ) -> Result<LedgerSetupEntries, LedgerSetupError> {
        self.read_setup_entries_from_lookup(|key| {
            let fetcher = self.node_fetcher.clone();
            self.state_map.peek_item_with_hash(key, &mut |hash| {
                fetcher.as_ref().and_then(|fetch| fetch(hash))
            })
        })
    }

    pub fn read_setup_entries_from_state_map_with_family<CLOCK, S, FB, F, MR, NS>(
        &self,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<LedgerSetupEntries, LedgerSetupError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        self.read_setup_entries_from_lookup(|key| {
            self.state_map.peek_item_with_hash_and_family(key, family)
        })
    }

    pub fn setup_from_state_map(
        &mut self,
        feature_xrp_fees: &Uint256,
    ) -> Result<bool, LedgerSetupError> {
        let entries = self.read_setup_entries_from_state_map()?;
        Ok(self.setup_with_entries(&entries, feature_xrp_fees))
    }

    pub fn setup_from_state_map_with_default_fees(
        &mut self,
        default_fees: Fees,
        feature_xrp_fees: &Uint256,
    ) -> Result<bool, LedgerSetupError> {
        self.apply_default_fees(default_fees);
        self.setup_from_state_map(feature_xrp_fees)
    }

    pub fn setup_from_state_map_with_config(
        &mut self,
        config: &LedgerConfig,
    ) -> Result<bool, LedgerSetupError> {
        self.rules = Rules::new(config.features.iter());
        self.setup_from_state_map_with_default_fees(config.fees, &feature_xrp_fees())
    }

    pub fn setup_from_state_map_with_family<CLOCK, S, FB, F, MR, NS>(
        &mut self,
        feature_xrp_fees: &Uint256,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<bool, LedgerSetupError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        let entries = self.read_setup_entries_from_state_map_with_family(family)?;
        Ok(self.setup_with_entries(&entries, feature_xrp_fees))
    }

    pub fn setup_from_state_map_with_default_fees_and_family<CLOCK, S, FB, F, MR, NS>(
        &mut self,
        default_fees: Fees,
        feature_xrp_fees: &Uint256,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<bool, LedgerSetupError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        self.apply_default_fees(default_fees);
        self.setup_from_state_map_with_family(feature_xrp_fees, family)
    }

    pub fn setup_from_state_map_with_config_and_family<CLOCK, S, FB, F, MR, NS>(
        &mut self,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<bool, LedgerSetupError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        self.rules = Rules::new(config.features.iter());
        self.setup_from_state_map_with_default_fees_and_family(
            config.fees,
            &feature_xrp_fees(),
            family,
        )
    }

    pub fn set_immutable_and_setup_from_state_map(
        &mut self,
        rehash: bool,
        feature_xrp_fees: &Uint256,
    ) -> Result<bool, LedgerSetupError> {
        self.finalize_immutable(rehash);
        self.setup_from_state_map(feature_xrp_fees)
    }

    pub fn set_immutable_and_setup_from_config(
        &mut self,
        rehash: bool,
        config: &LedgerConfig,
    ) -> Result<bool, LedgerSetupError> {
        self.finalize_immutable(rehash);
        self.setup_from_state_map_with_config(config)
    }

    pub fn set_full(&mut self) {
        self.tx_map.set_full();
        self.tx_map.set_ledger_seq(self.header.seq);
        self.state_map.set_full();
        self.state_map.set_ledger_seq(self.header.seq);
        let seq = self.header.seq;
        let state_entries = self.state_map.leaf_count() as u64;
        tracing::debug!(target: "ledger", seq, state_entries, "Ledger state size");
    }

    fn finalize_immutable(&mut self, rehash: bool) {
        if !self.immutable && rehash {
            // Only recompute tx_hash if the tx map has content
            let tx_hash = self.tx_map.hash();
            if !tx_hash.is_zero() || self.header.tx_hash.is_zero() {
                self.header.tx_hash = tx_hash;
            }
            // Only recompute account_hash from state map if the state map
            // is actually populated. For skip_state acquisitions, the state
            // map is empty but the header already has the correct account_hash
            // from the peer data.
            let state_hash = self.state_map.hash();
            if !state_hash.is_zero() || self.header.account_hash.is_zero() {
                self.header.account_hash = state_hash;
            }
        }

        if rehash {
            self.header.hash = calculate_ledger_hash(&self.header);
        }

        self.immutable = true;
        self.tx_map.set_immutable();
        self.state_map.set_immutable();

        {
            let seq = self.header.seq;
            let hash = self.header.account_hash;
            tracing::debug!(target: "ledger", seq, account_hash = %hash, tx_hash = %self.header.tx_hash, "Ledger immutable — hashes computed");
        }
    }

    /// Mark the ledger as immutable without rehashing or running
    /// setup_from_state_map. Used during catchup when the state map
    /// may not be fully traversable.
    pub fn finalize_immutable_no_setup(&mut self) {
        self.immutable = true;
        self.tx_map.set_immutable();
        self.state_map.set_immutable();
    }

    pub fn set_immutable(&mut self, rehash: bool) {
        self.finalize_immutable(rehash);

        if let Err(error) = self.setup_from_state_map(&feature_xrp_fees())
            && !matches!(error, LedgerSetupError::Traversal(_))
        {
            panic!("xrpl::Ledger::setImmutable : setup failed: {error:?}");
        }
    }

    /// Walks the loaded subtree bottom-up, computes hashes, marks nodes
    /// shareable (cowid=0), and writes them to the node store.
    /// Must be called after set_immutable and before the ledger is used
    /// as a parent for the next build.
    pub fn flush_state_map_to_store(&mut self) {
        let ledger_seq = self.header.seq;

        // Use the persistent mutable_state if available (has all loaded nodes),
        // otherwise create from state_map root.
        let mut tree = self.mutable_state.take().unwrap_or_else(|| {
            MutableTree::from_loaded_root(self.state_map.root(), ledger_seq.max(1))
        });

        let writer = self.node_writer.clone();
        tree.flush_dirty(
            &mut |node: basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >|
             -> basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            > {
                if let Some(ref write_fn) = writer {
                    let hash = node.get_hash();
                    if let Ok(data) = node.serialize_with_prefix() {
                        write_fn(
                            LedgerNodeObjectType::AccountNode,
                            *hash.as_uint256(),
                            data,
                            ledger_seq,
                        );
                    }
                }
                node
            },
        );

        // Update state_map with the flushed root (all nodes now have cowid=0)
        let map_type = self.state_map.map_type();
        let backed = self.state_map.backed();
        let state = self.state_map.state();
        let was_full = self.state_map.is_full();
        let next = SyncTree::from_root_with_type(tree.root(), map_type, backed, ledger_seq, state);
        if was_full {
            next.set_full();
        }
        self.state_map = next;
    }

    pub fn flush_tx_map_to_store(&mut self) {
        let ledger_seq = self.header.seq;
        let mut tree = MutableTree::from_loaded_root(self.tx_map.root(), ledger_seq.max(1));

        let writer = self.node_writer.clone();
        tree.flush_dirty(
            &mut |node: basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >|
             -> basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            > {
                if let Some(ref write_fn) = writer {
                    let hash = node.get_hash();
                    if let Ok(data) = node.serialize_with_prefix() {
                        write_fn(
                            LedgerNodeObjectType::TransactionNode,
                            *hash.as_uint256(),
                            data,
                            ledger_seq,
                        );
                    }
                }
                node
            },
        );

        let map_type = self.tx_map.map_type();
        let backed = self.tx_map.backed();
        let state = self.tx_map.state();
        let was_full = self.tx_map.is_full();
        let next = SyncTree::from_root_with_type(tree.root(), map_type, backed, ledger_seq, state);
        if was_full {
            next.set_full();
        }
        self.tx_map = next;
    }

    /// Persist dirty SHAMap nodes to the node store WITHOUT rebuilding
    /// the in-memory tree. Matches rippled's `BuildLedger.cpp` lines 69-73:
    ///   built->stateMap().flushDirty(AccountNode)
    ///   built->txMap().flushDirty(TransactionNode)
    ///
    /// For each flushed node this performs the two-step write that rippled's
    /// `SHAMap::writeNode` (SHAMap.cpp:935-947) performs:
    ///   1. **Canonicalize into the shared tree-node cache** — matches
    ///      `canonicalize(node->getHash(), node)` which calls
    ///      `f_.getTreeNodeCache()->canonicalizeReplaceClient()`. This ensures
    ///      subsequent `cacheLookup()` calls return a hit instead of falling
    ///      through to a NuDB round-trip (fixes Issue B: tree cache = 0).
    ///   2. **Persist to NuDB** — matches `f_.db().store(t, data, hash, seq)`.
    ///
    /// # Parameters
    /// - `tree_cache`: Shared tree-node cache (the single `TreeNodeCache`
    ///   instance shared across all SHAMaps via `SHAMapFamily`). When `Some`,
    ///   each flushed node is canonicalized into the cache before NuDB
    ///   persistence. When `None`, only NuDB persistence occurs (backward
    ///   compatible with callers that don't have a cache reference).
    pub fn persist_dirty_nodes_to_store(
        &mut self,
        tree_cache: Option<
            &shamap::tree_node_cache::TreeNodeCache<
                basics::tagged_cache::MonotonicClock,
                basics::hardened_hash::HardenedHashBuilder,
            >,
        >,
    ) {
        let ledger_seq = self.header.seq;
        let writer = self.node_writer.clone();
        if writer.is_none() {
            return;
        }

        // Flush state map dirty nodes
        let mut state_tree = self.mutable_state.take().unwrap_or_else(|| {
            MutableTree::from_loaded_root(self.state_map.root(), ledger_seq.max(1))
        });
        state_tree.flush_dirty(
            &mut |node: basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >|
             -> basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            > {
                // Step 1: Canonicalize into the shared tree-node cache.
                // Matches rippled SHAMap::writeNode (SHAMap.cpp:941):
                //   canonicalize(node->getHash(), node);
                if let Some(cache) = tree_cache {
                    let mut node_ref = node.clone();
                    let key = *node.get_hash().as_uint256();
                    cache.canonicalize_replace_client(&key, &mut node_ref);
                }

                // Step 2: Persist to NuDB.
                // Matches rippled SHAMap::writeNode (SHAMap.cpp:944-945):
                //   Serializer s; node->serializeWithPrefix(s);
                //   f_.db().store(t, std::move(s.modData()), node->getHash().asUInt256(), ledgerSeq_);
                if let Some(ref write_fn) = writer {
                    let hash = node.get_hash();
                    if let Ok(data) = node.serialize_with_prefix() {
                        write_fn(
                            LedgerNodeObjectType::AccountNode,
                            *hash.as_uint256(),
                            data,
                            ledger_seq,
                        );
                    }
                }
                node
            },
        );
        // Keep mutable_state so the tree stays in memory
        self.mutable_state = Some(state_tree);

        // Flush tx map dirty nodes
        let mut tx_tree = MutableTree::from_loaded_root(self.tx_map.root(), ledger_seq.max(1));
        tx_tree.flush_dirty(
            &mut |node: basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >|
             -> basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            > {
                // Step 1: Canonicalize into the shared tree-node cache (same as above).
                if let Some(cache) = tree_cache {
                    let mut node_ref = node.clone();
                    let key = *node.get_hash().as_uint256();
                    cache.canonicalize_replace_client(&key, &mut node_ref);
                }

                // Step 2: Persist to NuDB (same as above).
                if let Some(ref write_fn) = writer {
                    let hash = node.get_hash();
                    if let Ok(data) = node.serialize_with_prefix() {
                        write_fn(
                            LedgerNodeObjectType::TransactionNode,
                            *hash.as_uint256(),
                            data,
                            ledger_seq,
                        );
                    }
                }
                node
            },
        );
    }

    /// Set the node writer closure for flushing dirty nodes to the store.
    pub fn set_node_writer(
        &mut self,
        writer: Arc<
            dyn Fn(LedgerNodeObjectType, basics::base_uint::Uint256, Vec<u8>, u32) + Send + Sync,
        >,
    ) {
        self.node_writer = Some(writer);
    }

    pub fn set_accepted(
        &mut self,
        close_time: u32,
        close_resolution: u8,
        correct_close_time: bool,
    ) {
        self.header.close_time = close_time;
        self.header.close_time_resolution = close_resolution;
        self.header.close_flags = if correct_close_time {
            0
        } else {
            SLCF_NO_CONSENSUS_TIME
        };
        self.set_immutable(true);
    }

    pub fn set_accepted_and_setup_from_state_map(
        &mut self,
        close_time: u32,
        close_resolution: u8,
        correct_close_time: bool,
        feature_xrp_fees: &Uint256,
    ) -> Result<bool, LedgerSetupError> {
        self.header.close_time = close_time;
        self.header.close_time_resolution = close_resolution;
        self.header.close_flags = if correct_close_time {
            0
        } else {
            SLCF_NO_CONSENSUS_TIME
        };
        self.finalize_immutable(true);
        self.setup_from_state_map(feature_xrp_fees)
    }

    pub fn set_accepted_and_setup_from_config(
        &mut self,
        close_time: u32,
        close_resolution: u8,
        correct_close_time: bool,
        config: &LedgerConfig,
    ) -> Result<bool, LedgerSetupError> {
        self.header.close_time = close_time;
        self.header.close_time_resolution = close_resolution;
        self.header.close_flags = if correct_close_time {
            0
        } else {
            SLCF_NO_CONSENSUS_TIME
        };
        self.finalize_immutable(true);
        self.setup_from_state_map_with_config(config)
    }

    pub fn set_validated(&mut self) {
        self.header.validated = true;
    }

    pub fn get_enabled_amendments(&self) -> BTreeSet<Uint256> {
        get_enabled_amendments(self)
    }

    pub fn get_majority_amendments(&self) -> BTreeMap<Uint256, NetClockTimePoint> {
        get_majority_amendments(self)
    }

    pub fn is_flag_ledger(&self) -> bool {
        is_flag_ledger(self.header.seq)
    }

    pub fn is_voting_ledger(&self) -> bool {
        is_voting_ledger(self.header.seq.wrapping_add(1))
    }

    pub fn hash_of_seq<J: LedgerJournal>(&self, seq: u32, journal: &J) -> Option<SHAMapHash> {
        if seq > self.header.seq {
            journal.warn(&format!(
                "Can't get seq {seq} from {} future",
                self.header.seq
            ));
            return None;
        }

        if seq == self.header.seq {
            return Some(self.header.hash);
        }

        if seq.wrapping_add(1) == self.header.seq {
            return Some(self.header.parent_hash);
        }

        let diff = self.header.seq - seq;
        if diff <= 256 {
            match self.read(skip_keylet()) {
                Ok(Some(hashes)) => {
                    assert_eq!(
                        hashes.get_field_u32(get_field_by_symbol("sfLastLedgerSequence")),
                        self.header.seq - 1,
                        "xrpl::hashOfSeq : matching ledger sequence"
                    );
                    let hashes = hashes.get_field_v256(get_field_by_symbol("sfHashes"));
                    if hashes.value().len() >= diff as usize {
                        return Some(SHAMapHash::new(
                            hashes.value()[hashes.value().len() - diff as usize],
                        ));
                    }
                    journal.warn(&format!(
                        "Ledger {} missing hash for {} ({},{})",
                        self.header.seq,
                        seq,
                        hashes.value().len(),
                        diff
                    ));
                }
                Ok(None) => match self
                    .state_map
                    .peek_item(skip_keylet().key, &mut |_| None)
                    .map_err(MutationError::from)
                {
                    Ok(Some(item)) => {
                        let hashes = decode_ledger_hashes_entry(item.data())
                            .expect("current ledger skip-list objects must decode");
                        assert_eq!(
                            hashes.last_ledger_sequence,
                            Some(self.header.seq - 1),
                            "xrpl::hashOfSeq : matching ledger sequence"
                        );
                        if hashes.hashes.len() >= diff as usize {
                            return Some(SHAMapHash::new(
                                hashes.hashes[hashes.hashes.len() - diff as usize],
                            ));
                        }
                        journal.warn(&format!(
                            "Ledger {} missing hash for {} ({},{})",
                            self.header.seq,
                            seq,
                            hashes.hashes.len(),
                            diff
                        ));
                    }
                    Ok(None) => {
                        journal.warn(&format!(
                            "Ledger {}:{} missing normal list",
                            self.header.seq, self.header.hash
                        ));
                    }
                    Err(_) => return None,
                },
                Err(_) => return None,
            }
        }

        if (seq & 0xff) != 0 {
            return None;
        }

        if let Ok(Some(hashes)) = self.read(skip_keylet_for_ledger(seq)) {
            let last_seq = hashes.get_field_u32(get_field_by_symbol("sfLastLedgerSequence"));
            assert!(last_seq >= seq, "xrpl::hashOfSeq : minimum last ledger");
            assert!(
                (last_seq & 0xff) == 0,
                "xrpl::hashOfSeq : valid last ledger"
            );
            let diff = ((last_seq - seq) >> 8) as usize;
            let hashes = hashes.get_field_v256(get_field_by_symbol("sfHashes"));
            if hashes.value().len() > diff {
                return Some(SHAMapHash::new(
                    hashes.value()[hashes.value().len() - diff - 1],
                ));
            }
        } else if let Some(item) = self
            .state_map
            .peek_item(skip_keylet_for_ledger(seq).key, &mut |_| None)
            .ok()
            .flatten()
        {
            let hashes = decode_ledger_hashes_entry(item.data())
                .expect("current ledger skip-list objects must decode");
            let last_seq = hashes
                .last_ledger_sequence
                .expect("long skip-list entries must track the last ledger sequence");
            assert!(last_seq >= seq, "xrpl::hashOfSeq : minimum last ledger");
            assert!(
                (last_seq & 0xff) == 0,
                "xrpl::hashOfSeq : valid last ledger"
            );
            let diff = ((last_seq - seq) >> 8) as usize;
            if hashes.hashes.len() > diff {
                return Some(SHAMapHash::new(
                    hashes.hashes[hashes.hashes.len() - diff - 1],
                ));
            }
        }

        journal.warn(&format!(
            "Can't get seq {seq} from {} error",
            self.header.seq
        ));
        None
    }

    pub fn update_skip_list(&mut self) -> Result<(), MutationError> {
        use crate::views::raw_view::RawView;

        if self.header.seq == 0 {
            return Ok(());
        }

        let seq = self.header.seq;
        let backed = self.state_map.backed();
        let has_fetcher = self.node_fetcher.is_some();
        let has_mutable = self.mutable_state.is_some();
        tracing::debug!(
            target: "ledger",
            seq,
            backed,
            has_fetcher,
            has_mutable,
            "[skip_list] update_skip_list enter"
        );

        let prev_index = self.header.seq - 1;

        // Per-256 skip list (one per 256-ledger range)
        if (prev_index & 0xff) == 0 {
            let keylet = skip_keylet_for_ledger(prev_index);
            let peek_result = self.peek(keylet);
            tracing::debug!(
                target: "ledger",
                seq,
                key = %keylet.key,
                found = peek_result.as_ref().map(|r| r.is_some()).unwrap_or(false),
                err = peek_result.is_err(),
                "[skip_list] per-256 skip list peek"
            );
            if let Ok(Some(sle)) = peek_result {
                let mut hashes = sle.get_field_v256(get_field_by_symbol("sfHashes"));
                assert!(hashes.value().len() <= 256);
                hashes.push_back(*self.header.parent_hash.as_uint256());
                let mut obj = sle.clone_as_object();
                obj.set_field_v256(get_field_by_symbol("sfHashes"), hashes);
                obj.set_field_u32(get_field_by_symbol("sfLastLedgerSequence"), prev_index);
                let updated = Arc::new(STLedgerEntry::from_stobject(obj, keylet.key));
                tracing::debug!(target: "ledger", seq, key = %keylet.key, "[skip_list] per-256: raw_replace");
                let r = self.raw_replace(updated)
                    .map_err(|e| { let te = e.into(); tracing::error!(target: "ledger", seq, ?te, "[skip_list] per-256 raw_replace failed: ViewError→TraversalError"); MutationError::Traversal(te) });
                r?
            } else {
                let mut sle = STLedgerEntry::new(keylet);
                let mut hashes = protocol::STVector256::new();
                hashes.push_back(*self.header.parent_hash.as_uint256());
                sle.set_field_v256(get_field_by_symbol("sfHashes"), hashes);
                sle.set_field_u32(get_field_by_symbol("sfLastLedgerSequence"), prev_index);
                tracing::debug!(target: "ledger", seq, key = %keylet.key, "[skip_list] per-256: raw_insert");
                let r = self.raw_insert(Arc::new(sle))
                    .map_err(|e| { let te = e.into(); tracing::error!(target: "ledger", seq, ?te, "[skip_list] per-256 raw_insert failed: ViewError→TraversalError"); MutationError::Traversal(te) });
                r?
            }
        }

        // Global skip list (last 256 hashes)
        let keylet = skip_keylet();
        let peek_result = self.peek(keylet);
        tracing::debug!(
            target: "ledger",
            seq,
            key = %keylet.key,
            found = peek_result.as_ref().map(|r| r.is_some()).unwrap_or(false),
            err = peek_result.is_err(),
            "[skip_list] global skip list peek"
        );
        if let Ok(Some(sle)) = peek_result {
            let old_hashes = sle.get_field_v256(get_field_by_symbol("sfHashes"));
            let mut new_values: Vec<Uint256> = old_hashes.value().to_vec();
            assert!(new_values.len() <= 256);
            if new_values.len() == 256 {
                new_values.remove(0);
            }
            new_values.push(*self.header.parent_hash.as_uint256());
            let hashes =
                protocol::STVector256::from_values(get_field_by_symbol("sfHashes"), new_values);
            let mut obj = sle.clone_as_object();
            obj.set_field_v256(get_field_by_symbol("sfHashes"), hashes);
            obj.set_field_u32(get_field_by_symbol("sfLastLedgerSequence"), prev_index);
            let updated = Arc::new(STLedgerEntry::from_stobject(obj, keylet.key));
            tracing::debug!(target: "ledger", seq, key = %keylet.key, "[skip_list] global: raw_replace");
            let r = self.raw_replace(updated)
                .map_err(|e| { let te = e.into(); tracing::error!(target: "ledger", seq, ?te, "[skip_list] global raw_replace failed: ViewError→TraversalError"); MutationError::Traversal(te) });
            r?
        } else {
            let mut sle = STLedgerEntry::new(keylet);
            let mut hashes = protocol::STVector256::new();
            hashes.push_back(*self.header.parent_hash.as_uint256());
            sle.set_field_v256(get_field_by_symbol("sfHashes"), hashes);
            sle.set_field_u32(get_field_by_symbol("sfLastLedgerSequence"), prev_index);
            tracing::debug!(target: "ledger", seq, key = %keylet.key, "[skip_list] global: raw_insert");
            let r = self.raw_insert(Arc::new(sle))
                .map_err(|e| { let te = e.into(); tracing::error!(target: "ledger", seq, ?te, "[skip_list] global raw_insert failed: ViewError→TraversalError"); MutationError::Traversal(te) });
            r?
        }

        tracing::debug!(target: "ledger", seq, "[skip_list] update_skip_list OK");
        Ok(())
    }

    pub fn update_negative_unl(&mut self) -> Result<(), MutationError> {
        let Some(mut sle) = self.peek(negative_unl_keylet())? else {
            return Ok(());
        };

        let sf_disabled_validators = get_field_by_symbol("sfDisabledValidators");
        let sf_disabled_validator = get_field_by_symbol("sfDisabledValidator");
        let sf_first_ledger_sequence = get_field_by_symbol("sfFirstLedgerSequence");
        let sf_public_key = get_field_by_symbol("sfPublicKey");
        let sf_validator_to_disable = get_field_by_symbol("sfValidatorToDisable");
        let sf_validator_to_re_enable = get_field_by_symbol("sfValidatorToReEnable");
        let has_to_disable = sle.is_field_present(sf_validator_to_disable);
        let has_to_re_enable = sle.is_field_present(sf_validator_to_re_enable);

        if !has_to_disable && !has_to_re_enable {
            return Ok(());
        }

        let mut new_negative_unl = STArray::new(sf_disabled_validators);
        if sle.is_field_present(sf_disabled_validators) {
            let disabled_validators = sle.get_field_array(sf_disabled_validators);
            for validator in disabled_validators.iter() {
                if has_to_re_enable
                    && validator.is_field_present(sf_public_key)
                    && validator.get_field_vl(sf_public_key)
                        == sle.get_field_vl(sf_validator_to_re_enable)
                {
                    continue;
                }
                new_negative_unl.push_back(validator.clone());
            }
        }

        if has_to_disable {
            let mut validator = STObject::make_inner_object(sf_disabled_validator);
            validator.set_field_vl(sf_public_key, &sle.get_field_vl(sf_validator_to_disable));
            validator.set_field_u32(sf_first_ledger_sequence, self.header.seq);
            new_negative_unl.push_back(validator);
        }

        if new_negative_unl.iter().next().is_some() {
            sle.set_field_array(sf_disabled_validators, new_negative_unl);
            if has_to_re_enable {
                sle.make_field_absent(sf_validator_to_re_enable);
            }
            if has_to_disable {
                sle.make_field_absent(sf_validator_to_disable);
            }
            self.replace_state_map_item(*sle.key(), sle.get_serializer().data().to_vec())?;
        } else {
            self.delete_state_map_item(*sle.key())?;
        }

        Ok(())
    }

    pub fn negative_unl(&self) -> HashSet<[u8; 33]> {
        let sf_disabled_validators = get_field_by_symbol("sfDisabledValidators");
        let sf_public_key = get_field_by_symbol("sfPublicKey");

        self.read(negative_unl_keylet())
            .ok()
            .flatten()
            .filter(|sle| sle.is_field_present(sf_disabled_validators))
            .map(|sle| {
                sle.get_field_array(sf_disabled_validators)
                    .iter()
                    .filter(|validator| validator.is_field_present(sf_public_key))
                    .filter_map(|validator| {
                        decode_negative_unl_public_key(&validator.get_field_vl(sf_public_key))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn validator_to_disable(&self) -> Option<[u8; 33]> {
        let field = get_field_by_symbol("sfValidatorToDisable");

        self.read(negative_unl_keylet())
            .ok()
            .flatten()
            .filter(|sle| sle.is_field_present(field))
            .and_then(|sle| decode_negative_unl_public_key(&sle.get_field_vl(field)))
    }

    pub fn validator_to_re_enable(&self) -> Option<[u8; 33]> {
        let field = get_field_by_symbol("sfValidatorToReEnable");

        self.read(negative_unl_keylet())
            .ok()
            .flatten()
            .filter(|sle| sle.is_field_present(field))
            .and_then(|sle| decode_negative_unl_public_key(&sle.get_field_vl(field)))
    }

    pub fn finish_load_by_index_or_hash<J: LedgerJournal>(
        &mut self,
        journal: &J,
    ) -> Result<(), LedgerSetupError> {
        if self.header.seq >= XRP_LEDGER_EARLIEST_FEES {
            assert!(
                self.has_fee_settings_object()?,
                "xrpl::finishLoadByIndexOrHash : valid ledger fees"
            );
        }

        let _ = self.set_immutable_and_setup_from_state_map(true, &feature_xrp_fees())?;
        journal.info(&format!("Loaded ledger: {}", self.header.hash));
        self.set_full();
        Ok(())
    }

    pub fn assert_sensible(&mut self) -> bool {
        let sensible = self.header.hash.is_non_zero()
            && self.header.account_hash.is_non_zero()
            && self.header.account_hash == self.state_map.hash()
            && self.header.tx_hash == self.tx_map.hash();

        assert!(sensible, "ledger is not sensible");
        true
    }

    pub fn walk_ledger_with_family<CLOCK, S, FB, F, MR, NS, J>(
        &mut self,
        journal: &J,
        parallel: bool,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> bool
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone + Send + Sync,
        FB: FullBelowCache + Send,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        NS: Send,
        J: LedgerJournal,
    {
        let mut missing_nodes1 = Vec::new();
        let mut missing_nodes2 = Vec::new();

        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        if self.state_map.hash().is_zero()
            && self.header.account_hash.is_non_zero()
            && !self.state_map.fetch_root_with_family(
                self.header.account_hash,
                &mut no_filter,
                family,
            )
        {
            missing_nodes1.push(SHAMapMissingNode::from_hash(
                SHAMapType::State,
                self.header.account_hash,
            ));
        } else if parallel {
            return self.state_map.walk_map_parallel_with_family(
                SHAMapType::State,
                &mut missing_nodes1,
                WALK_LEDGER_MAX_MISSING_NODES,
                family,
            );
        } else {
            self.state_map.walk_map_with_family(
                SHAMapType::State,
                &mut missing_nodes1,
                WALK_LEDGER_MAX_MISSING_NODES,
                family,
            );
        }

        log_missing_nodes(journal, "account", &missing_nodes1);

        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        if self.tx_map.hash().is_zero()
            && self.header.tx_hash.is_non_zero()
            && !self
                .tx_map
                .fetch_root_with_family(self.header.tx_hash, &mut no_filter, family)
        {
            missing_nodes2.push(SHAMapMissingNode::from_hash(
                SHAMapType::Transaction,
                self.header.tx_hash,
            ));
        } else {
            self.tx_map.walk_map_with_family(
                SHAMapType::Transaction,
                &mut missing_nodes2,
                WALK_LEDGER_MAX_MISSING_NODES,
                family,
            );
        }

        log_missing_nodes(journal, "transaction", &missing_nodes2);

        missing_nodes1.is_empty() && missing_nodes2.is_empty()
    }
}

fn parse_state_sle(payload: &[u8], keylet: Keylet) -> Option<STLedgerEntry> {
    let mut iter = SerialIter::new(payload);
    let entry = catch_unwind(AssertUnwindSafe(|| {
        STLedgerEntry::from_serial_iter(&mut iter, keylet.key)
    }))
    .ok()?;
    if !iter.empty() || !keylet.check_ledger_entry(entry.get_type(), *entry.key()) {
        return None;
    }
    Some(entry)
}

fn parse_state_sle_any(payload: &[u8], key: Uint256) -> Option<STLedgerEntry> {
    parse_state_sle(payload, Keylet::new(protocol::LedgerEntryType::Any, key))
}

impl Ledger {
    fn has_fee_settings_object(&self) -> Result<bool, LedgerSetupError> {
        Ok(matches!(
            self.read_setup_entries_from_state_map()?.fees,
            SetupLookup::Present(_)
        ))
    }

    fn read_setup_entries_from_lookup<F>(
        &self,
        mut lookup: F,
    ) -> Result<LedgerSetupEntries, LedgerSetupError>
    where
        F: FnMut(
            Uint256,
        ) -> Result<
            Option<(shamap::item::SHAMapItem, SHAMapHash)>,
            shamap::traversal::TraversalError,
        >,
    {
        let amendments = match lookup(amendments_keylet().key) {
            Ok(Some((item, hash))) => SetupLookup::Present(
                if let Some(decoded) =
                    state_map::decode_amendments_entry_from_sle(item.data(), *hash.as_uint256())
                {
                    decoded
                } else {
                    state_map::decode_amendments_entry(item.data(), *hash.as_uint256())?
                },
            ),
            Ok(None) => SetupLookup::MissingObject,
            Err(shamap::traversal::TraversalError::MissingNode(_)) => SetupLookup::MissingNode,
            Err(_) => SetupLookup::MissingNode, // Treat other traversal errors as missing node for setup
        };

        let fees = match lookup(fee_settings_keylet().key) {
            Ok(Some((item, _hash))) => SetupLookup::Present(
                if let Some(decoded) = state_map::decode_fee_settings_fields_from_sle(item.data()) {
                    decoded
                } else {
                    state_map::decode_fee_settings_fields(item.data())?
                },
            ),
            Ok(None) => SetupLookup::MissingObject,
            Err(shamap::traversal::TraversalError::MissingNode(_)) => SetupLookup::MissingNode,
            Err(_) => SetupLookup::MissingNode,
        };

        Ok(LedgerSetupEntries { amendments, fees })
    }

    #[allow(dead_code)]
    fn read_ledger_hashes_entry(
        &self,
        key: Uint256,
    ) -> Result<DecodedLedgerHashesEntry, MutationError> {
        match self.state_map.peek_item(key, &mut |_| None)? {
            Some(item) => Ok(decode_ledger_hashes_entry(item.data())
                .expect("current ledger skip-list objects must decode")),
            None => Ok(DecodedLedgerHashesEntry::default()),
        }
    }

    fn replace_state_map_item(
        &mut self,
        key: Uint256,
        payload: Vec<u8>,
    ) -> Result<(), MutationError> {
        // Route through apply_state_batch to keep mutable_state in sync.
        // Use the ledger's node_fetcher when checking whether the key already
        // exists in the state map. Without this, nodes that were evicted from
        // memory by `release_maps_to_disk` (backed=true but no in-memory
        // children) cannot be resolved, causing TraversalError::MissingNode.
        // That error propagates through ViewError::Mutation and then through
        // the `From<ViewError> for TraversalError` catch-all arm into
        // TraversalError::View, producing the `Mutation(Traversal(View))`
        // error seen when building ledger N+1 from a released ledger N state.
        let seq = self.header.seq;
        let backed = self.state_map.backed();
        let has_fetcher = self.node_fetcher.is_some();
        let mut fetch_fn = |hash: basics::sha_map_hash::SHAMapHash| -> Option<
            basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >,
        > {
            let fetcher = self.node_fetcher.as_ref()?;
            fetcher(hash)
        };
        let peek_result = self.state_map.peek_item(key, &mut fetch_fn);
        tracing::debug!(
            target: "ledger",
            seq,
            %key,
            backed,
            has_fetcher,
            exists = peek_result.as_ref().map(|r| r.is_some()).unwrap_or(false),
            err = peek_result.is_err(),
            "[replace_state_map] peek result"
        );
        if let Err(ref e) = peek_result {
            tracing::error!(
                target: "ledger",
                seq,
                %key,
                backed,
                has_fetcher,
                ?e,
                "[replace_state_map] peek FAILED — this is the Mutation(Traversal(View)) source"
            );
        }
        let exists = peek_result?.is_some();
        let op = if exists {
            StateBatchOp::Update
        } else {
            StateBatchOp::Insert
        };
        tracing::debug!(target: "ledger", seq, %key, ?op, "[replace_state_map] applying batch");
        self.apply_state_batch(&[(op, key, payload)])
    }

    fn insert_state_map_item(
        &mut self,
        key: Uint256,
        payload: Vec<u8>,
    ) -> Result<(), MutationError> {
        // Route through apply_state_batch to keep mutable_state in sync.
        self.apply_state_batch(&[(StateBatchOp::Insert, key, payload)])
    }

    fn update_state_map_item(
        &mut self,
        key: Uint256,
        payload: Vec<u8>,
    ) -> Result<(), MutationError> {
        // Route through apply_state_batch to keep mutable_state in sync.
        self.apply_state_batch(&[(StateBatchOp::Update, key, payload)])
    }

    fn delete_state_map_item(&mut self, key: Uint256) -> Result<(), MutationError> {
        // Route through apply_state_batch to keep mutable_state in sync.
        self.apply_state_batch(&[(StateBatchOp::Delete, key, Vec::new())])
    }

    /// Apply a batch of state map operations using a SINGLE MutableTree.
    /// This prevents MissingNode errors that occur when sequential individual
    /// mutations create new inner nodes that subsequent mutations can't find
    /// in NuDB (because they only exist in memory from the previous mutation).
    ///
    /// This matches reference behavior where OpenView::apply writes all changes to
    /// the same underlying SHAMap in one session.
    pub fn apply_state_batch(
        &mut self,
        ops: &[(StateBatchOp, Uint256, Vec<u8>)],
    ) -> Result<(), MutationError> {
        if ops.is_empty() {
            return Ok(());
        }

        let ledger_seq = self.header.seq;

        // Use a globally unique cowid to prevent in-place mutation of shared nodes.
        // In C++ rippled, each SHAMap has its own cowid_ incremented from the copy
        // constructor. Using ledger_seq alone can collide when multiple MutableTrees
        // are created from roots with the same cowid (e.g., submit-time sandbox and
        // accept-time build both operating on the same closed ledger's state).
        static NEXT_COWID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(100);

        // across ALL transactions, matching reference where stateMap_ is a persistent
        // SHAMap that rawInsert/rawErase/rawReplace all operate on directly.
        if self.mutable_state.is_none() {
            let cowid = NEXT_COWID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.mutable_state = Some(MutableTree::from_loaded_root(
                self.state_map.root(),
                cowid.max(1),
            ));
        }
        let tree = self.mutable_state.as_mut().unwrap();

        let mut fetch_fn = |hash: basics::sha_map_hash::SHAMapHash| -> Option<
            basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >,
        > {
            let fetcher = self.node_fetcher.as_ref()?;
            fetcher(hash)
        };

        for (op, key, payload) in ops {
            match op {
                StateBatchOp::Insert => {
                    let inserted = tree.add_item_with_fetch(
                        shamap::tree_node::SHAMapNodeType::AccountState,
                        SHAMapItem::new(*key, payload.clone()),
                        &mut fetch_fn,
                    )?;
                    // If the item already exists (add_item returns Ok(false)), fall back
                    // to update. This handles the case where a sandbox tracks a modification
                    // as Insert (because the item was created within the sandbox's scope)
                    // but the item already exists in the underlying state map from a
                    // previous ledger.
                    if !inserted {
                        tree.update_item_with_fetch(
                            shamap::tree_node::SHAMapNodeType::AccountState,
                            SHAMapItem::new(*key, payload.clone()),
                            &mut fetch_fn,
                        )?;
                    }
                }
                StateBatchOp::Update => {
                    tree.update_item_with_fetch(
                        shamap::tree_node::SHAMapNodeType::AccountState,
                        SHAMapItem::new(*key, payload.clone()),
                        &mut fetch_fn,
                    )?;
                }
                StateBatchOp::Delete => {
                    tree.delete_item_with_fetch(*key, &mut fetch_fn)?;
                }
            }
        }

        // Update state_map root from the persistent tree (for reads between TXs)
        let map_type = self.state_map.map_type();
        let backed = self.state_map.backed();
        let state = self.state_map.state();
        let was_full = self.state_map.is_full();
        let next_state_map =
            SyncTree::from_root_with_type(tree.root(), map_type, backed, ledger_seq, state);
        if was_full {
            next_state_map.set_full();
        }
        self.state_map = next_state_map;
        let seq = self.header.seq;
        let ops_count = ops.len();
        tracing::debug!(target: "ledger", seq, ops_count, "State batch applied");
        Ok(())
    }

    fn insert_tx_map_item(
        &mut self,
        key: Uint256,
        txn_bytes: Vec<u8>,
        metadata_bytes: Vec<u8>,
    ) -> Result<(), MutationError> {
        let map_type = self.tx_map.map_type();
        let backed = self.tx_map.backed();
        let ledger_seq = self.header.seq;
        let state = self.tx_map.state();
        let was_full = self.tx_map.is_full();
        let mut payload = Serializer::new(txn_bytes.len() + metadata_bytes.len() + 16);
        payload.add_vl(&txn_bytes);
        payload.add_vl(&metadata_bytes);

        let mut tree = MutableTree::from_loaded_root(self.tx_map.root(), ledger_seq.max(1));
        tree.add_item(
            SHAMapNodeType::TransactionMd,
            SHAMapItem::new(key, payload.data().to_vec()),
        )?;

        let next_tx_map =
            SyncTree::from_root_with_type(tree.root(), map_type, backed, ledger_seq, state);
        if was_full {
            next_tx_map.set_full();
        }
        self.tx_map = next_tx_map;
        Ok(())
    }
}

pub fn build_genesis_setup_items<I>(config: &LedgerConfig, amendments: I) -> Vec<(Uint256, Vec<u8>)>
where
    I: IntoIterator<Item = Uint256>,
{
    let amendments: Vec<_> = amendments.into_iter().collect();
    genesis::build_genesis_setup_items(
        config.fees.base,
        config.fees.reserve,
        config.fees.increment,
        &amendments,
    )
}

pub fn build_genesis_state_items<I>(
    config: &LedgerConfig,
    amendments: I,
    total_drops: u64,
) -> Vec<(Uint256, Vec<u8>)>
where
    I: IntoIterator<Item = Uint256>,
{
    let amendments: Vec<_> = amendments.into_iter().collect();
    constructor_ledger_items(&build_genesis_state_constructor_entries(
        total_drops,
        config.fees.base,
        config.fees.reserve,
        config.fees.increment,
        &amendments,
    ))
}

pub fn get_next_ledger_time_resolution(
    previous_resolution: u8,
    previous_agree: bool,
    ledger_seq: u32,
) -> u8 {
    let Some(index) = LEDGER_POSSIBLE_TIME_RESOLUTIONS
        .iter()
        .position(|&resolution| resolution == previous_resolution)
    else {
        return previous_resolution;
    };

    if !previous_agree && ledger_seq.is_multiple_of(DECREASE_LEDGER_TIME_RESOLUTION_EVERY) {
        return LEDGER_POSSIBLE_TIME_RESOLUTIONS
            .get(index + 1)
            .copied()
            .unwrap_or(previous_resolution);
    }

    if previous_agree && ledger_seq.is_multiple_of(INCREASE_LEDGER_TIME_RESOLUTION_EVERY) {
        return index
            .checked_sub(1)
            .and_then(|index| LEDGER_POSSIBLE_TIME_RESOLUTIONS.get(index))
            .copied()
            .unwrap_or(previous_resolution);
    }

    previous_resolution
}

pub fn round_close_time(close_time: u32, close_resolution: u8) -> u32 {
    if close_time == 0 || close_resolution == 0 {
        return close_time;
    }

    let close_resolution = u32::from(close_resolution);
    let rounded = close_time + (close_resolution / 2);
    rounded - (rounded % close_resolution)
}

fn log_missing_nodes<J: LedgerJournal>(
    journal: &J,
    label: &str,
    missing_nodes: &[SHAMapMissingNode],
) {
    if missing_nodes.is_empty() {
        return;
    }

    journal.info(&format!(
        "{} missing {label} node(s)First: {}",
        missing_nodes.len(),
        missing_nodes[0]
    ));
}

fn increment_hash(hash: SHAMapHash) -> SHAMapHash {
    SHAMapHash::new((*hash.as_uint256()).next())
}

#[cfg(test)]
mod tests {
    use super::*;
    use basics::base_uint::Uint256;
    use protocol::{ApplyFlags, Keylet, LedgerEntryType, STLedgerEntry, Serializer, XRPAmount};
    use std::sync::Arc;

    #[derive(Debug, Default)]
    struct MockBaseView {
        items: BTreeMap<Uint256, Arc<STLedgerEntry>>,
    }

    impl ReadView for MockBaseView {
        fn open(&self) -> bool {
            true
        }
        fn header(&self) -> LedgerHeader {
            LedgerHeader::default()
        }
        fn fees(&self) -> Fees {
            Fees::default()
        }
        fn rules(&self) -> Rules {
            Rules::default()
        }
        fn exists(&self, k: Keylet) -> Result<bool, ViewError> {
            Ok(self.items.contains_key(&k.key))
        }
        fn succ(&self, key: Uint256, _last: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
            Ok(self
                .items
                .range((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
                .next()
                .map(|(k, _)| *k))
        }
        fn read(&self, k: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.items.get(&k.key).cloned())
        }
        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.items.values().cloned().collect())
        }
        fn tx_exists(&self, _key: Uint256) -> Result<bool, ViewError> {
            Ok(false)
        }
        fn tx_read(&self, _key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            Ok(None)
        }
        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            Ok(vec![])
        }
    }

    #[derive(Debug, Default)]
    struct RecordingRawView {
        calls: Vec<String>,
        destroyed: XRPAmount,
    }

    impl RawView for RecordingRawView {
        fn raw_erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
            self.calls.push(format!("erase:{}", sle.key()));
            Ok(())
        }

        fn raw_insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
            self.calls.push(format!("insert:{}", sle.key()));
            Ok(())
        }

        fn raw_replace(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
            self.calls.push(format!("replace:{}", sle.key()));
            Ok(())
        }

        fn raw_destroy_xrp(&mut self, fee: XRPAmount) -> Result<(), ViewError> {
            self.calls.push("destroy_xrp".to_string());
            self.destroyed += fee;
            Ok(())
        }

        fn raw_apply_batch(
            &mut self,
            ops: &[(StateBatchOp, Uint256, Vec<u8>)],
        ) -> Result<(), ViewError> {
            for (op, key, _) in ops {
                match op {
                    StateBatchOp::Insert => self.calls.push(format!("insert:{}", key)),
                    StateBatchOp::Update => self.calls.push(format!("replace:{}", key)),
                    StateBatchOp::Delete => self.calls.push(format!("erase:{}", key)),
                }
            }
            Ok(())
        }
    }

    #[test]
    fn test_apply_view_succ_parity() {
        let mut base = MockBaseView::default();
        let k1 = Uint256::from_u64(10);
        let k2 = Uint256::from_u64(20);
        let k3 = Uint256::from_u64(30);

        let sle1 = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            k1,
        )));
        let sle2 = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            k2,
        )));
        let sle3 = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            k3,
        )));

        base.items.insert(k1, sle1.clone());
        base.items.insert(k2, sle2.clone());
        base.items.insert(k3, sle3.clone());

        let base_arc = Arc::new(base);
        let mut view = ApplyViewImpl::new(base_arc.clone(), ApplyFlags::default());

        // 1. Initial succ should match base
        assert_eq!(view.succ(Uint256::zero(), None).unwrap(), Some(k1));
        assert_eq!(view.succ(k1, None).unwrap(), Some(k2));

        // 2. Erase k2 in view (must peek first to load into state table)
        let _ = view.peek(Keylet::new(LedgerEntryType::AccountRoot, k2));
        view.raw_erase(sle2).unwrap();
        assert_eq!(view.succ(k1, None).unwrap(), Some(k3)); // Should skip erased k2

        // 3. Insert k1.5 (key 15)
        let k15 = Uint256::from_u64(15);
        let sle15 = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            k15,
        )));
        view.raw_insert(sle15).unwrap();
        assert_eq!(view.succ(k1, None).unwrap(), Some(k15));
        assert_eq!(view.succ(k15, None).unwrap(), Some(k3));
    }

    #[test]
    fn test_sandbox_commit_parity() {
        let base = Arc::new(MockBaseView::default());
        let view = ApplyViewImpl::new(base, ApplyFlags::default());

        let k1 = Uint256::from_u64(100);
        let sle1 = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            k1,
        )));

        let mut sb = Sandbox::new(Arc::new(view), ApplyFlags::default());
        sb.raw_insert(sle1.clone()).unwrap();

        assert!(
            sb.exists(Keylet::new(LedgerEntryType::AccountRoot, k1))
                .unwrap()
        );
    }

    #[test]
    fn test_payment_sandbox_merge_parity() {
        let base = Arc::new(MockBaseView::default());
        let view = Arc::new(ApplyViewImpl::new(base, ApplyFlags::default()));

        let k1 = Uint256::from_u64(100);
        let sle1 = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            k1,
        )));

        let mut ps_target = PaymentSandbox::new(view.clone(), ApplyFlags::default());
        let mut ps_source = PaymentSandbox::new(view.clone(), ApplyFlags::default());
        ps_source.raw_insert(sle1.clone()).unwrap();

        ps_source.apply_to_sandbox(&mut ps_target).unwrap();
        assert!(
            ps_target
                .exists(Keylet::new(LedgerEntryType::AccountRoot, k1))
                .unwrap()
        );
    }

    #[test]
    fn test_apply_view_raw_replace_missing_key_behavior() {
        let base = Arc::new(MockBaseView::default());
        let mut view = ApplyViewImpl::new(base, ApplyFlags::default());

        let key = Uint256::from_u64(55);
        let sle = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            key,
        )));

        view.raw_replace(sle.clone())
            .expect("raw_replace should behave like reference replace on missing key");

        assert!(
            view.exists(Keylet::new(LedgerEntryType::AccountRoot, key))
                .expect("existence check should succeed")
        );
        assert_eq!(
            view.read(Keylet::new(LedgerEntryType::AccountRoot, key))
                .expect("read should succeed"),
            Some(sle)
        );
    }

    #[test]
    fn test_apply_view_update_requires_existing_entry() {
        let base = Arc::new(MockBaseView::default());
        let mut view = ApplyViewImpl::new(base, ApplyFlags::default());

        let key = Uint256::from_u64(77);
        let sle = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            key,
        )));

        let error = view
            .update(sle)
            .expect_err("update should reject missing-key writes");
        assert!(
            matches!(error, ViewError::Conversion(message) if message == "ApplyStateTable::update: missing key")
        );
    }

    #[test]
    fn test_apply_view_insert_rejects_cached_entry() {
        let key = Uint256::from_u64(88);
        let sle = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            key,
        )));

        let mut base = MockBaseView::default();
        base.items.insert(key, sle.clone());

        let base = Arc::new(base);
        let mut view = ApplyViewImpl::new(base, ApplyFlags::default());

        view.peek(Keylet::new(LedgerEntryType::AccountRoot, key))
            .expect("peek should succeed")
            .expect("base entry should exist");

        let error = view
            .insert(sle)
            .expect_err("insert should reject already cached entries");
        assert!(
            matches!(error, ViewError::Conversion(message) if message == "ApplyStateTable::insert: already cached")
        );
    }

    #[test]
    fn test_raw_state_table_exists_and_read_check_keylet_type() {
        let key = Uint256::from_u64(501);
        let sle = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            key,
        )));

        let mut table = RawStateTable::new();
        table
            .insert(sle.clone())
            .expect("overlay insert should succeed");

        let wrong_keylet = Keylet::new(LedgerEntryType::Offer, key);
        assert!(
            !table
                .exists(&MockBaseView::default(), wrong_keylet)
                .expect("exists should succeed"),
            "overlay exists should reject mismatched keylet types"
        );
        assert_eq!(
            table
                .read(&MockBaseView::default(), wrong_keylet)
                .expect("read should succeed"),
            None,
            "overlay read should reject mismatched keylet types"
        );
    }

    #[test]
    fn test_raw_state_table_succ_skips_erased_base_and_includes_overlay_insert() {
        let key10 = Uint256::from_u64(10);
        let key20 = Uint256::from_u64(20);
        let key15 = Uint256::from_u64(15);

        let sle10 = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            key10,
        )));
        let sle20 = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            key20,
        )));
        let sle15 = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            key15,
        )));

        let mut base = MockBaseView::default();
        base.items.insert(key10, sle10);
        base.items.insert(key20, sle20.clone());

        let mut table = RawStateTable::new();
        table.erase(sle20).expect("erase overlay should succeed");
        table.insert(sle15).expect("insert overlay should succeed");

        let base = Arc::new(base);
        assert_eq!(
            table
                .succ(base.as_ref(), key10, None)
                .expect("succ should succeed"),
            Some(key15)
        );
        assert_eq!(
            table
                .succ(base.as_ref(), key15, None)
                .expect("succ should succeed"),
            None
        );
    }

    #[test]
    fn test_raw_state_table_apply_destroys_xrp_before_mutations() {
        let key = Uint256::from_u64(601);
        let sle = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            key,
        )));

        let mut table = RawStateTable::new();
        table.insert(sle).expect("insert overlay should succeed");
        table.destroy_xrp(XRPAmount::from_drops(10));

        let mut sink = RecordingRawView::default();
        table.apply(&mut sink).expect("apply should succeed");

        assert_eq!(
            sink.calls,
            vec!["destroy_xrp".to_string(), format!("insert:{key}")]
        );
        assert_eq!(sink.destroyed, XRPAmount::from_drops(10));
    }

    #[test]
    fn test_raw_state_table_rejects_invalid_transition_sequences() {
        let key = Uint256::from_u64(701);
        let sle = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            key,
        )));

        let mut table = RawStateTable::new();
        table
            .insert(sle.clone())
            .expect("initial insert should succeed");

        let duplicate_insert = table
            .insert(sle.clone())
            .expect_err("duplicate insert should fail");
        assert!(
            matches!(duplicate_insert, ViewError::Conversion(message) if message == "RawStateTable::insert: already inserted")
        );

        table
            .erase(sle.clone())
            .expect("erase after insert should cancel");
        assert!(
            !table
                .exists(
                    &MockBaseView::default(),
                    Keylet::new(LedgerEntryType::AccountRoot, key)
                )
                .expect("exists should succeed")
        );

        table
            .erase(sle.clone())
            .expect("first erase should succeed");
        let double_erase = table
            .erase(sle.clone())
            .expect_err("double erase should fail");
        assert!(
            matches!(double_erase, ViewError::Conversion(message) if message == "RawStateTable::erase: already erased")
        );

        let replace_after_erase = table
            .replace(sle)
            .expect_err("replace after erase should fail");
        assert!(
            matches!(replace_after_erase, ViewError::Conversion(message) if message == "RawStateTable::replace: was erased")
        );
    }

    #[test]
    fn test_open_view_new_closed_preserves_base_open_flag() {
        let base = Arc::new(MockBaseView::default());
        let view = OpenView::new_closed(base);

        assert!(view.open(), "OpenView copies the base open flag");
    }

    #[test]
    fn test_open_view_batch_clone_resets_base_tx_count() {
        let base = Arc::new(MockBaseView::default());
        let mut parent = OpenView::new_closed(base);

        parent
            .raw_tx_insert(Uint256::from_u64(901), Arc::new(Serializer::new(0)), None)
            .expect("parent overlay tx insert should succeed");

        let batch = OpenView::batch_from(Arc::new(parent));
        assert_eq!(
            batch.tx_count(),
            1,
            "batch view should inherit base tx count"
        );

        let cloned = batch.clone();
        assert_eq!(
            cloned.tx_count(),
            0,
            "OpenView copy constructor leaves baseTxCount_ at zero"
        );
    }

    #[test]
    fn test_open_view_exists_and_read_follow_raw_state_table_keylet_checks() {
        let base = Arc::new(MockBaseView::default());
        let mut view = OpenView::new_closed(base);

        let key = Uint256::from_u64(902);
        let sle = Arc::new(STLedgerEntry::new(Keylet::new(
            LedgerEntryType::AccountRoot,
            key,
        )));
        view.raw_insert(sle).expect("overlay insert should succeed");

        let wrong_keylet = Keylet::new(LedgerEntryType::Offer, key);
        assert!(
            !view.exists(wrong_keylet).expect("exists should succeed"),
            "OpenView should reject mismatched keylet types through RawStateTable"
        );
        assert_eq!(
            view.read(wrong_keylet).expect("read should succeed"),
            None,
            "OpenView should reject mismatched keylet reads through RawStateTable"
        );
    }

    #[test]
    fn setup_from_state_map_fetches_fee_settings_through_node_fetcher() {
        let fees = Fees {
            base: 12,
            reserve: 1_000_000,
            increment: 200_000,
        };
        let fee_payload = encode_fee_settings_entry(fees, false);
        let fee_key = fee_settings_keylet().key;
        let fee_leaf = basics::intrusive_pointer::make_shared_intrusive(
            shamap::tree_node::SHAMapTreeNode::new_leaf(
                SHAMapNodeType::AccountState,
                SHAMapItem::new(fee_key, fee_payload),
                0,
            ),
        );
        let fee_leaf_hash = fee_leaf.get_hash();

        let root = basics::intrusive_pointer::make_shared_intrusive(
            shamap::tree_node::SHAMapTreeNode::new_inner(0),
        );
        let branch =
            shamap::node_id::select_branch(shamap::node_id::SHAMapNodeId::default(), fee_key);
        root.set_child_hash(branch, fee_leaf_hash);

        let seq = XRP_LEDGER_EARLIEST_FEES;
        let state_map =
            SyncTree::from_root_with_type(root, SHAMapType::State, true, seq, SyncState::Immutable);
        let mut ledger = Ledger::from_maps(
            LedgerHeader {
                seq,
                ..LedgerHeader::default()
            },
            state_map,
            SyncTree::new_with_type(SHAMapType::Transaction, true, seq),
        );

        let setup_without_fetcher = ledger
            .setup_from_state_map(&feature_xrp_fees())
            .expect("setup should report missing-node state without failing");
        assert!(!setup_without_fetcher);
        assert_eq!(ledger.fees(), Fees::default());

        ledger.set_node_fetcher(Arc::new(move |hash| {
            if hash == fee_leaf_hash {
                Some(fee_leaf.clone())
            } else {
                None
            }
        }));

        let setup_with_fetcher = ledger
            .setup_from_state_map(&feature_xrp_fees())
            .expect("fetch-backed setup should succeed");
        assert!(setup_with_fetcher);
        assert_eq!(ledger.fees(), fees);
    }
}
pub use domain::credential_helpers;
pub use domain::lending_helpers;
pub use domain::mptoken_helpers;
pub use domain::mul_ratio;
pub use domain::nftoken_helpers;
pub use domain::offer_helpers;
pub use domain::payment_channel_helpers;
pub use domain::permissioned_dex_helpers;
pub use domain::ripple_state_helpers;
pub use domain::token_helpers::{add_empty_holding, can_add_holding, remove_empty_holding};
pub use domain::vault_helpers;
