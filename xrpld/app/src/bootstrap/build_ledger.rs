//! App-owned `BuildLedger` orchestration above the landed ledger and tx cores.
//!
//! This ports the deterministic control flow from
//! `xrpld/app/ledger/detail/the reference source`:
//! - parent-ledger follow construction,
//! - consensus `applyTransactions(...)` pass ordering,
//! - replay ordering over pre-sorted transactions,
//! - flag-ledger negative-UNL update,
//! - skip-list, flush, unshare, and accept finalization order.
//!
//! The current Rust substrate still does not expose the exact reference
//! `OpenView`/`StorageTree` owner types, so this module keeps those seams
//! explicit through a view trait plus flush/unshare callbacks while preserving
//! the build ordering literally.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    sync::Arc,
};

use basics::base_uint::Uint256;
use basics::str_hex::str_hex;
use ledger::{CanonicalTXSet, Ledger, LedgerTxReadError, OpenView, XRP_LEDGER_EARLIEST_FEES};
use protocol::{
    JsonOptions, Keylet, LedgerEntryType, STTx, Serializer, StBase, fee_settings_keylet,
    skip_keylet,
};
use shamap::{mutation::MutationError, traversal::TraversalError};
use tx::{ApplyFlags, ApplyTransactionResult};

use crate::{LEDGER_RETRY_PASSES, LEDGER_TOTAL_PASSES};

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
        if crate::bootstrap::build_ledger::full_sync_debug_enabled() {
            tracing::debug!(target: "full_sync", $($arg)*);
        }
    };
}

pub trait BuildLedgerJournal {
    fn debug(&self, message: &str);
    fn warn(&self, message: &str);
}

#[derive(Debug, Default)]
pub struct NullBuildLedgerJournal;

impl BuildLedgerJournal for NullBuildLedgerJournal {
    fn debug(&self, _message: &str) {}

    fn warn(&self, _message: &str) {}
}

pub trait BuildLedgerView {
    fn open(&self) -> bool;
    fn tx_count(&self) -> usize;
    fn apply_to_ledger(self, ledger: &mut Ledger);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildLedgerFlushReport {
    pub account_state_nodes: usize,
    pub transaction_nodes: usize,
}

#[derive(Debug, Clone)]
pub struct LedgerReplay {
    parent: Arc<Ledger>,
    replay: Arc<Ledger>,
    ordered_txs: BTreeMap<u32, Arc<STTx>>,
}

impl LedgerReplay {
    pub fn new(
        parent: Arc<Ledger>,
        replay: Arc<Ledger>,
        ordered_txs: BTreeMap<u32, Arc<STTx>>,
    ) -> Self {
        Self {
            parent,
            replay,
            ordered_txs,
        }
    }

    pub fn parent(&self) -> &Arc<Ledger> {
        &self.parent
    }

    pub fn replay(&self) -> &Arc<Ledger> {
        &self.replay
    }

    pub fn ordered_txs(&self) -> &BTreeMap<u32, Arc<STTx>> {
        &self.ordered_txs
    }

    pub fn from_replay_ledger(
        parent: Arc<Ledger>,
        replay: Arc<Ledger>,
    ) -> Result<Self, LedgerReplayError> {
        let mut ordered_txs = BTreeMap::new();

        for (tx, meta) in replay.tx_snapshot()? {
            ordered_txs.entry(meta.get_index()).or_insert(tx);
        }

        Ok(Self::new(parent, replay, ordered_txs))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerReplayError {
    TxRead(LedgerTxReadError),
}

impl From<LedgerTxReadError> for LedgerReplayError {
    fn from(value: LedgerTxReadError) -> Self {
        Self::TxRead(value)
    }
}

#[derive(Debug)]
pub enum BuildLedgerError {
    Mutation(MutationError),
    Traversal(TraversalError),
}

impl From<MutationError> for BuildLedgerError {
    fn from(value: MutationError) -> Self {
        Self::Mutation(value)
    }
}

impl From<TraversalError> for BuildLedgerError {
    fn from(value: TraversalError) -> Self {
        Self::Traversal(value)
    }
}

pub fn apply_transactions<V, J, ApplyE>(
    built: &Ledger,
    txns: &mut CanonicalTXSet,
    failed: &mut BTreeSet<Uint256>,
    view: &mut V,
    journal: &J,
    apply_transaction: &mut impl FnMut(
        &mut V,
        &Arc<STTx>,
        bool,
        ApplyFlags,
    ) -> Result<ApplyTransactionResult, ApplyE>,
) -> usize
where
    J: BuildLedgerJournal,
    ApplyE: Display,
{
    let mut certain_retry = true;
    let mut count = 0usize;

    for pass in 0..LEDGER_TOTAL_PASSES {
        let pass_label = if certain_retry {
            "Pass: "
        } else {
            "Final pass: "
        };
        journal.debug(&format!(
            "{pass_label}{pass} begins ({} transactions)",
            txns.len()
        ));

        let mut changes = 0usize;
        let pending = txns.drain_ordered();

        for tx in pending {
            let txid = tx.get_transaction_id();

            if pass == 0 && built.tx_exists(txid) {
                continue;
            }

            match apply_transaction(view, &tx, certain_retry, ApplyFlags::NONE) {
                Ok(ApplyTransactionResult::Success) => {
                    changes += 1;
                }
                Ok(ApplyTransactionResult::Fail) => {
                    failed.insert(txid);
                }
                Ok(ApplyTransactionResult::Retry) => {
                    txns.insert(tx);
                }
                Err(error) => {
                    journal.warn(&format!("Transaction {txid} throws: {error}"));
                    failed.insert(txid);
                }
            }
        }

        journal.debug(&format!("{pass_label}{pass} completed ({changes} changes)"));

        count += changes;

        if changes == 0 && !certain_retry {
            break;
        }

        if changes == 0 || pass >= LEDGER_RETRY_PASSES {
            certain_retry = false;
        }
    }

    assert!(
        txns.is_empty() || !certain_retry,
        "xrpl::applyTransactions : retry transactions"
    );
    count
}

pub fn build_ledger<V, J, CreateView, Apply, FlushState, FlushTx, Unshare>(
    parent: Arc<Ledger>,
    close_time: u32,
    close_time_correct: bool,
    close_resolution: u8,
    txns: &mut CanonicalTXSet,
    failed_txns: &mut BTreeSet<Uint256>,
    journal: &J,
    create_view: CreateView,
    mut apply_transaction: Apply,
    flush_state: FlushState,
    flush_tx: FlushTx,
    unshare: Unshare,
) -> Result<Arc<Ledger>, BuildLedgerError>
where
    V: BuildLedgerView,
    J: BuildLedgerJournal,
    CreateView: FnOnce(&Ledger) -> V,
    Apply: FnMut(&mut V, &Arc<STTx>, bool, ApplyFlags) -> Result<ApplyTransactionResult, String>,
    FlushState: FnMut(&mut Ledger) -> usize,
    FlushTx: FnMut(&mut Ledger) -> usize,
    Unshare: FnMut(&mut Ledger),
{
    journal.debug(&format!(
        "Report: Transaction Set = {}, close {}{}",
        txns.key(),
        close_time,
        if close_time_correct {
            ""
        } else {
            " (incorrect)"
        }
    ));

    build_ledger_impl(
        parent,
        close_time,
        close_time_correct,
        close_resolution,
        journal,
        create_view,
        flush_state,
        flush_tx,
        unshare,
        |view, built, ledger_journal| {
            ledger_journal.debug(&format!("Attempting to apply {} transactions", txns.len()));

            let applied = apply_transactions(
                built,
                txns,
                failed_txns,
                view,
                ledger_journal,
                &mut apply_transaction,
            );

            if !txns.is_empty() || !failed_txns.is_empty() {
                ledger_journal.debug(&format!(
                    "Applied {applied} transactions; {} failed and {} will be retried. Total transactions in ledger (including Inner Batch): {}",
                    failed_txns.len(),
                    txns.len(),
                    view.tx_count()
                ));
            } else {
                ledger_journal.debug(&format!(
                    "Applied {applied} transactions. Total transactions in ledger (including Inner Batch): {}",
                    view.tx_count()
                ));
            }
        },
    )
}

pub fn build_ledger_from_view<V, J, CreateView, FlushState, FlushTx, Unshare>(
    parent: Arc<Ledger>,
    close_time: u32,
    close_time_correct: bool,
    close_resolution: u8,
    journal: &J,
    create_view: CreateView,
    flush_state: FlushState,
    flush_tx: FlushTx,
    unshare: Unshare,
) -> Result<Arc<Ledger>, BuildLedgerError>
where
    V: BuildLedgerView,
    J: BuildLedgerJournal,
    CreateView: FnOnce(&Ledger) -> V,
    FlushState: FnMut(&mut Ledger) -> usize,
    FlushTx: FnMut(&mut Ledger) -> usize,
    Unshare: FnMut(&mut Ledger),
{
    build_ledger_impl(
        parent,
        close_time,
        close_time_correct,
        close_resolution,
        journal,
        create_view,
        flush_state,
        flush_tx,
        unshare,
        |_view, _built, _journal| {},
    )
}

pub fn build_ledger_replay<V, J, CreateView, Apply, FlushState, FlushTx, Unshare>(
    replay_data: &LedgerReplay,
    apply_flags: ApplyFlags,
    journal: &J,
    create_view: CreateView,
    mut apply_transaction: Apply,
    flush_state: FlushState,
    flush_tx: FlushTx,
    unshare: Unshare,
) -> Result<Arc<Ledger>, BuildLedgerError>
where
    V: BuildLedgerView,
    J: BuildLedgerJournal,
    CreateView: FnOnce(&Ledger) -> V,
    Apply: FnMut(&mut V, &Arc<STTx>, ApplyFlags),
    FlushState: FnMut(&mut Ledger) -> usize,
    FlushTx: FnMut(&mut Ledger) -> usize,
    Unshare: FnMut(&mut Ledger),
{
    let replay_ledger = replay_data.replay();
    journal.debug(&format!(
        "Report: Replay Ledger {}",
        replay_ledger.header().hash
    ));

    build_ledger_impl(
        Arc::clone(replay_data.parent()),
        replay_ledger.header().close_time,
        (replay_ledger.header().close_flags & ledger::SLCF_NO_CONSENSUS_TIME) == 0,
        replay_ledger.header().close_time_resolution,
        journal,
        create_view,
        flush_state,
        flush_tx,
        unshare,
        |view, _built, _ledger_journal| {
            for tx in replay_data.ordered_txs().values() {
                apply_transaction(view, tx, apply_flags);
            }
        },
    )
}

fn build_ledger_impl<V, J, CreateView, FlushState, FlushTx, Unshare, ApplyTxs>(
    parent: Arc<Ledger>,
    close_time: u32,
    close_time_correct: bool,
    close_resolution: u8,
    journal: &J,
    create_view: CreateView,
    mut flush_state: FlushState,
    mut flush_tx: FlushTx,
    mut unshare: Unshare,
    apply_txs: ApplyTxs,
) -> Result<Arc<Ledger>, BuildLedgerError>
where
    V: BuildLedgerView,
    J: BuildLedgerJournal,
    CreateView: FnOnce(&Ledger) -> V,
    FlushState: FnMut(&mut Ledger) -> usize,
    FlushTx: FnMut(&mut Ledger) -> usize,
    Unshare: FnMut(&mut Ledger),
    ApplyTxs: FnOnce(&mut V, &Ledger, &J),
{
    let mut built = Ledger::from_previous(&parent, close_time);

    if built.is_flag_ledger() {
        built.update_negative_unl()?;
    }

    let mut accum = create_view(&built);
    assert!(!accum.open(), "xrpl::buildLedgerImpl : valid ledger state");
    apply_txs(&mut accum, &built, journal);
    accum.apply_to_ledger(&mut built);

    built.update_skip_list()?;
    let flush_report = BuildLedgerFlushReport {
        account_state_nodes: flush_state(&mut built),
        transaction_nodes: flush_tx(&mut built),
    };
    journal.debug(&format!(
        "Flushed {} accounts and {} transaction nodes",
        flush_report.account_state_nodes, flush_report.transaction_nodes
    ));
    unshare(&mut built);

    assert!(
        built.header().seq < XRP_LEDGER_EARLIEST_FEES
            || built.read(fee_settings_keylet())?.is_some(),
        "xrpl::buildLedgerImpl : valid ledger fees"
    );
    built.set_accepted(close_time, close_resolution, close_time_correct);

    Ok(Arc::new(built))
}

/// Build a complete ledger locally from parent state + acquired TX map.
///
/// This matches reference consensus behavior: instead of acquiring the full state
/// tree from peers (25M nodes), we apply the transaction set to the parent
/// state to produce the new state locally. This guarantees a complete state
/// tree with no missing nodes.
///
/// Returns None if the resulting account_hash doesn't match the acquired
/// header (transactor produced different state — likely an unimplemented
/// transaction type).

/// Global counter: only log detailed state_map/sandbox mutations for first 2 builds
static BUILD_DETAIL_LOG_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Check if detailed logging is enabled for this build
pub fn should_log_build_details() -> bool {
    BUILD_DETAIL_LOG_COUNT.load(std::sync::atomic::Ordering::Relaxed) < 2
}

/// Decode acquired transaction-map payloads into reference canonical apply order.
///
/// `CanonicalTXSet`, then applies from that canonical set. Applying raw SHAMap
/// leaf order can differ from reference when one account has multiple transactions.
pub fn decode_acquired_tx_set(
    tx_items: &[(Vec<u8>, basics::base_uint::Uint256)],
    salt: basics::base_uint::Uint256,
) -> Vec<Arc<STTx>> {
    use protocol::SerialIter;

    let mut txns = CanonicalTXSet::new(salt);
    for (tx_data, _tx_id) in tx_items {
        let mut outer = SerialIter::new(tx_data);
        let tx_bytes = outer.get_vl();
        let mut sit = SerialIter::new(&tx_bytes);
        txns.insert(Arc::new(STTx::from_serial_iter(&mut sit)));
    }
    txns.drain_ordered()
}

pub fn build_ledger_from_acquired_tx(
    parent: &ledger::Ledger,
    acquired_header: protocol::LedgerHeader,
    tx_items: &[(Vec<u8>, basics::base_uint::Uint256)],
) -> Option<ledger::Ledger> {
    use crate::state::application_root::apply_submit_transactor_shell;
    use std::sync::Arc;

    let build_num = BUILD_DETAIL_LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let _detail_log = build_num < 2; // Only log detailed mutations for first 2 builds

    // Create new ledger from parent with mutable state snapshot.
    let mut built = ledger::Ledger::from_previous(parent, acquired_header.close_time);

    full_sync_debug!(
        "[full_debug][build_start] seq={} parent_seq={} parent_hash={} parent_account_hash={} parent_tx_hash={} parent_drops={} parent_fetcher={} parent_writer={} parent_state_full={} parent_tx_full={} parent_fees_base={} parent_fees_reserve={} parent_fees_inc={} target_hash={} target_account_hash={} target_tx_hash={} target_drops={} tx_count={}",
        acquired_header.seq,
        parent.header().seq,
        parent.header().hash,
        parent.header().account_hash,
        parent.header().tx_hash,
        parent.header().drops,
        parent.has_node_fetcher(),
        parent.has_node_writer(),
        parent.state_map().is_full(),
        parent.tx_map().is_full(),
        parent.fees().base,
        parent.fees().reserve,
        parent.fees().increment,
        acquired_header.hash,
        acquired_header.account_hash,
        acquired_header.tx_hash,
        acquired_header.drops,
        tx_items.len()
    );

    // Override header fields from the acquired header.
    let mut header = built.header();
    header.close_time = acquired_header.close_time;
    header.parent_close_time = acquired_header.parent_close_time;
    header.close_time_resolution = acquired_header.close_time_resolution;
    header.close_flags = acquired_header.close_flags;
    header.tx_hash = acquired_header.tx_hash;
    built.set_ledger_info(header);

    // Apply transactions from the acquired TX map in the reference CanonicalTXSet order.
    let ordered_txs = decode_acquired_tx_set(tx_items, *acquired_header.tx_hash.as_uint256());
    let tx_count = ordered_txs.len();
    let mut tx_type_counts: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();

    // Note: fees are already destroyed via view.destroy_xrp() inside
    // apply_submit_transactor_shell, which reduces header.drops when
    // the sandbox is applied. No need to manually adjust drops here.

    // reference: updateNegativeUNL on flag ledgers (every 256 ledgers)
    if ledger::is_flag_ledger(acquired_header.seq) {
        tracing::debug!(target: "ledger",
            "[build] FLAG_LEDGER seq={} — calling update_negative_unl",
            acquired_header.seq
        );
        let _ = built.update_negative_unl();
    }

    // Each tx gets a Sandbox wrapping a snapshot of the accumulator (cheap:
    // clones only the BTreeMap of SLE Arc pointers, not the SHAMap).
    // After all txs, accum.apply(&mut built) commits once to the SHAMap.
    let mut accum = OpenView::new_closed(Arc::new(built.clone()));

    for (tx_index, sttx) in ordered_txs.iter().enumerate() {
        let txn_type = sttx.get_txn_type();
        let tx_id = sttx.get_transaction_id();
        let tx_flags = sttx.get_flags();
        let tx_fee_drops = if sttx.is_field_present(protocol::get_field_by_symbol("sfFee")) {
            sttx.get_field_amount(protocol::get_field_by_symbol("sfFee"))
                .xrp()
                .drops()
        } else {
            0
        };

        // Sandbox wraps a snapshot of the accumulator — reads see all prior
        // tx changes via the accumulator's delta table, then fall through to
        // the original built ledger. No SHAMap clone needed.
        let base = Arc::new(accum.clone());
        let mut view = ledger::Sandbox::new(base, protocol::ApplyFlags::default());

        // Wrap individual tx application in catch_unwind for diagnostics
        let tx_type_name = format!("{:?}", txn_type);
        *tx_type_counts.entry(tx_type_name.clone()).or_insert(0) += 1;
        let tx_account = sttx.get_account_id(protocol::get_field_by_symbol("sfAccount"));
        full_sync_debug!(
            "[full_debug][tx_pre] seq={} tx_index={} tx_count={} txid={} type={} account={} flags=0x{:08x} fee_drops={} ledger_drops_before={}",
            acquired_header.seq,
            tx_index,
            tx_count,
            tx_id,
            tx_type_name,
            tx_account,
            tx_flags,
            tx_fee_drops,
            built.header().drops
        );
        let apply_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apply_submit_transactor_shell(&mut view, sttx, txn_type)
        }));

        match apply_result {
            Ok(ter) => {
                // === DEBUG: Log sandbox modifications before apply ===
                let mods = view.modification_summary();
                tracing::debug!(target: "ledger",
                    "[build] TX_PRE_APPLY seq={} tx={}/{} type={} ter={:?} txid={:02x}{:02x}{:02x}{:02x} account={:02x}{:02x}{:02x}{:02x} mods={}",
                    acquired_header.seq,
                    tx_index,
                    tx_count,
                    tx_type_name,
                    ter,
                    tx_id.data()[0],
                    tx_id.data()[1],
                    tx_id.data()[2],
                    tx_id.data()[3],
                    tx_account.data()[0],
                    tx_account.data()[1],
                    tx_account.data()[2],
                    tx_account.data()[3],
                    mods,
                );
                for line in view.modification_debug_lines() {
                    full_sync_debug!(
                        "[full_debug][tx_touch] seq={} tx_index={} txid={} ter={:?} {}",
                        acquired_header.seq,
                        tx_index,
                        tx_id,
                        ter,
                        line
                    );
                }

                let rules = built.rules().clone();
                if let Err(e) =
                    view.apply_with_tx_thread(&mut accum, tx_id, acquired_header.seq, &rules)
                {
                    tracing::debug!(target: "ledger",
                        "[build] APPLY ERROR seq={} tx={}/{} type={} error={:?}",
                        acquired_header.seq, tx_index, tx_count, tx_type_name, e
                    );
                }

                // Log TX result (without computing state hash — that's destructive
                // because recompute_hashes_recursive calls update_hash_deep which
                // stores new hashes in child_hashes, corrupting the tree for
                // subsequent operations).
                full_sync_debug!(
                    "[full_debug][tx_post] seq={} tx_index={} txid={} type={} ter={:?} drops_after={} drops_delta={} mods={}",
                    acquired_header.seq,
                    tx_index,
                    tx_id,
                    tx_type_name,
                    ter,
                    built.header().drops,
                    built.header().drops as i128 - parent.header().drops as i128,
                    mods
                );

                if full_sync_debug_enabled() {
                    let mut hash_probe = built.clone();
                    hash_probe.set_immutable(true);
                    tracing::debug!(target: "ledger",
                        "[full_debug][tx_hash_probe] seq={} tx_index={} txid={} account_hash={} tx_hash={} ledger_hash={} drops={}",
                        acquired_header.seq,
                        tx_index,
                        tx_id,
                        hash_probe.header().account_hash,
                        hash_probe.header().tx_hash,
                        protocol::calculate_ledger_hash(&hash_probe.header()),
                        hash_probe.header().drops
                    );
                }
            }
            Err(panic_info) => {
                let panic_msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "unknown panic".to_string()
                };
                tracing::debug!(target: "ledger",
                    "[build] TX PANIC seq={} tx_index={}/{} type={} account={:02x}{:02x}{:02x}{:02x} panic={}",
                    acquired_header.seq,
                    tx_index,
                    tx_count,
                    tx_type_name,
                    tx_account.data()[0],
                    tx_account.data()[1],
                    tx_account.data()[2],
                    tx_account.data()[3],
                    panic_msg
                );
                full_sync_debug!(
                    "[full_debug][tx_panic] seq={} tx_index={} txid={} type={} account={} panic={}",
                    acquired_header.seq,
                    tx_index,
                    tx_id,
                    tx_type_name,
                    tx_account,
                    panic_msg
                );
                // Skip this tx — don't apply its changes
            }
        }
    }

    // tx changes from the OpenView into the built ledger's SHAMap at once.
    if let Err(e) = accum.apply_state_only(&mut built) {
        tracing::debug!(target: "ledger",
            "[build] ACCUM APPLY ERROR seq={} error={:?}",
            acquired_header.seq, e
        );
    }

    // Update skip list (reference does this after applying all txs)
    let _ = built.update_skip_list();

    // Must flush BEFORE set_immutable so state_map.hash() reflects all mutations.
    // apply_state_batch writes to mutable_state; flush_state_map_to_store
    // propagates mutable_state → state_map so finalize_immutable gets the right hash.
    built.flush_state_map_to_store();
    built.flush_tx_map_to_store();

    // Finalize: compute state hash (now reads from updated state_map)
    built.set_immutable(true);
    debug_dump_build_state_bytes(&built, acquired_header.seq);
    let drops_match = built.header().drops == acquired_header.drops;
    full_sync_debug!(
        "[full_debug][build_final] seq={} hash={} expected_hash={} account_hash={} expected_account_hash={} tx_hash={} expected_tx_hash={} drops={} expected_drops={} drops_match={}",
        acquired_header.seq,
        protocol::calculate_ledger_hash(&built.header()),
        acquired_header.hash,
        built.header().account_hash,
        acquired_header.account_hash,
        built.header().tx_hash,
        acquired_header.tx_hash,
        built.header().drops,
        acquired_header.drops,
        drops_match
    );

    // Verify account_hash matches what the network validated
    if built.header().account_hash != acquired_header.account_hash {
        tracing::debug!(target: "ledger",
            "[build] HASH MISMATCH seq={} tx_count={} expected_prefix={:02x}{:02x}{:02x}{:02x} got_prefix={:02x}{:02x}{:02x}{:02x}",
            acquired_header.seq,
            tx_count,
            acquired_header.account_hash.as_uint256().data()[0],
            acquired_header.account_hash.as_uint256().data()[1],
            acquired_header.account_hash.as_uint256().data()[2],
            acquired_header.account_hash.as_uint256().data()[3],
            built.header().account_hash.as_uint256().data()[0],
            built.header().account_hash.as_uint256().data()[1],
            built.header().account_hash.as_uint256().data()[2],
            built.header().account_hash.as_uint256().data()[3],
        );
        // Log tx type breakdown for diagnosis
        let mut types_vec: Vec<_> = tx_type_counts.iter().collect();
        types_vec.sort_by(|a, b| b.1.cmp(a.1));
        let types_str: String = types_vec
            .iter()
            .map(|(k, v)| format!("{}:{}", k, v))
            .collect::<Vec<_>>()
            .join(" ");
        tracing::debug!(target: "ledger", "[build] TX TYPES seq={} {}", acquired_header.seq, types_str);
        full_sync_debug!(
            "[full_debug][build_mismatch] seq={} reason=account_hash expected={} got={} tx_count={} drops={} expected_drops={} tx_types={}",
            acquired_header.seq,
            acquired_header.account_hash,
            built.header().account_hash,
            tx_count,
            built.header().drops,
            acquired_header.drops,
            types_str
        );
        // The next build uses this ledger as parent — it needs the state nodes
        // in NuDB to read account/directory state via the fetcher.
        built.flush_state_map_to_store();
        built.flush_tx_map_to_store();
        return None;
    }

    // Verify full ledger hash
    let computed_hash = protocol::calculate_ledger_hash(&built.header());
    if computed_hash != acquired_header.hash {
        full_sync_debug!(
            "[full_debug][build_mismatch] seq={} reason=ledger_hash expected={} got={} account_hash={} tx_hash={}",
            acquired_header.seq,
            acquired_header.hash,
            computed_hash,
            built.header().account_hash,
            built.header().tx_hash
        );
        built.flush_state_map_to_store(); // flush even on full-hash mismatch
        built.flush_tx_map_to_store();
        return None;
    }

    // State already flushed before set_immutable above.
    // No additional flush needed here.

    Some(built)
}

fn debug_dump_build_state_bytes(built: &ledger::Ledger, seq: u32) {
    let Ok(target_seq) = std::env::var("XRPL_DEBUG_BUILD_SEQ") else {
        return;
    };
    if target_seq.parse::<u32>().ok() != Some(seq) {
        return;
    }

    let default_keys = if seq == 17_254_684 {
        format!(
            "AccountRoot:BD90E00001AB09941B2814A1F2B441C7661B889F39236B19EE12CCDA1EE8D03E,\
             AccountRoot:55FC1B596115716D5DC58F4B443E5CC4D0988C381B2E8F3225F34865493893F3,\
             RippleState:7200F873DB6454148CD4013BC32640AE6E8C14E893EA3396D6253768DE3A1434,\
             LedgerHashes:{}",
            skip_keylet().key
        )
    } else {
        String::new()
    };
    let keys = std::env::var("XRPL_DEBUG_BUILD_STATE_KEYS").unwrap_or(default_keys);
    for item in keys
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let Some((entry_type_name, key_hex)) = item.split_once(':') else {
            tracing::debug!(target: "ledger", "[state_diff] seq={} invalid_key_spec={}", seq, item);
            continue;
        };
        let Some(entry_type) = parse_debug_ledger_entry_type(entry_type_name) else {
            tracing::debug!(target: "ledger",
                "[state_diff] seq={} unsupported_entry_type={}",
                seq, entry_type_name
            );
            continue;
        };
        let Ok(key) = Uint256::from_hex(key_hex) else {
            tracing::debug!(target: "ledger", "[state_diff] seq={} invalid_key_hex={}", seq, key_hex);
            continue;
        };
        match built.read(Keylet::new(entry_type, key)) {
            Ok(Some(sle)) => {
                let mut serializer = Serializer::new(256);
                sle.add(&mut serializer);
                tracing::debug!(target: "ledger",
                    "[state_diff] seq={} type={:?} key={} hex={} json={:?}",
                    seq,
                    entry_type,
                    key,
                    str_hex(serializer.data()),
                    sle.json(JsonOptions::NONE)
                );
            }
            Ok(None) => tracing::debug!(target: "ledger",
                "[state_diff] seq={} type={:?} key={} missing=true",
                seq, entry_type, key
            ),
            Err(error) => tracing::debug!(target: "ledger",
                "[state_diff] seq={} type={:?} key={} error={:?}",
                seq, entry_type, key, error
            ),
        }
    }
}

fn parse_debug_ledger_entry_type(value: &str) -> Option<LedgerEntryType> {
    match value {
        "AccountRoot" => Some(LedgerEntryType::AccountRoot),
        "RippleState" => Some(LedgerEntryType::RippleState),
        "LedgerHashes" => Some(LedgerEntryType::LedgerHashes),
        "DirectoryNode" => Some(LedgerEntryType::DirectoryNode),
        "Offer" => Some(LedgerEntryType::Offer),
        "Ticket" => Some(LedgerEntryType::Ticket),
        _ => None,
    }
}

/// Build a ledger from consensus TX set — no hash verification.
/// This matches reference buildLedgerImpl: apply TXs to parent, compute hashes,
/// return the built ledger. The caller (consensus) accepts it as the new LCL.
pub fn build_ledger_from_consensus(
    parent: &ledger::Ledger,
    header: protocol::LedgerHeader,
    tx_items: &[(Vec<u8>, basics::base_uint::Uint256)],
    node_fetcher: Option<
        std::sync::Arc<
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
) -> Option<ledger::Ledger> {
    use crate::state::application_root::apply_submit_transactor_shell;
    use std::sync::Arc;

    let mut built = ledger::Ledger::from_previous(parent, header.close_time);
    // Ensure built ledger has node_fetcher for state map reads
    if !built.has_node_fetcher() {
        if let Some(fetcher) = node_fetcher {
            built.set_node_fetcher(fetcher);
        }
    }

    // Override header fields from consensus
    let mut h = built.header();
    h.close_time = header.close_time;
    h.parent_close_time = header.parent_close_time;
    h.close_time_resolution = header.close_time_resolution;
    h.close_flags = header.close_flags;
    built.set_ledger_info(h);

    // reference: updateNegativeUNL on flag ledgers
    if ledger::is_flag_ledger(header.seq) {
        let _ = built.update_negative_unl();
    }

    // Apply transactions using OpenView accumulator (reference buildLedgerImpl parity)
    let mut accum = OpenView::new_closed(Arc::new(built.clone()));

    let ordered_txs = decode_acquired_tx_set(tx_items, *header.tx_hash.as_uint256());
    for sttx in ordered_txs {
        let tx_id = sttx.get_transaction_id();
        let txn_type = sttx.get_txn_type();

        let base = Arc::new(accum.clone());
        let mut view = ledger::Sandbox::new(base, protocol::ApplyFlags::default());

        let apply_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apply_submit_transactor_shell(&mut view, &sttx, txn_type)
        }));

        match apply_result {
            Ok(_ter) => {
                let rules = built.rules().clone();
                if let Err(e) = view.apply_with_tx_thread(&mut accum, tx_id, header.seq, &rules) {
                    tracing::info!(target: "consensus", "APPLY ERROR tx={} error={:?}", tx_id, e);
                }
            }
            Err(_) => {
                // Skip panicking transactions (reference catches exceptions)
            }
        }
    }

    if let Err(e) = accum.apply_state_only(&mut built) {
        tracing::info!(target: "consensus", "ACCUM APPLY ERROR error={:?}", e);
    }

    // Update skip list and finalize
    let _ = built.update_skip_list();
    built.flush_state_map_to_store();
    built.flush_tx_map_to_store();
    built.set_immutable(true);

    Some(built)
}
