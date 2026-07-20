//! App-level wiring of Phase 3's generic `Consensus<Adaptor>` state machine
//! against real `Ledger`/`SHAMap`/`STValidation`/`ValidatorList` types.
//! Ported from `RCLConsensus.h`/`RCLConsensus.cpp`.
//!
//! This module defines:
//! - [`RclConsensusOpenLedgerSource`]: the open-ledger view the adaptor
//!   reads current transactions from and resets on ledger acceptance.
//! - [`RclConsensusValidationSource`]: the validation-tracking surface the
//!   adaptor needs (trusted proposer counts, preferred ledger).
//! - [`AppRclConsensusOptions`]: standalone/timing overrides.
//! - [`AppRclConsensusRelay`]: peer broadcast of proposals/tx-sets/etc.
//! - [`NullRclConsensusJournal`]: a no-op diagnostics sink.
//! - [`AppRclConsensusAdaptor`]: the `ConsensusAdaptor` implementation.
//! - [`ConsensusRunner`] / [`AppConsensus`]: the single-strand consensus
//!   driver that owns `Consensus<AppRclConsensusAdaptor>` directly on the
//!   strand thread with NO mutex (matching rippled's single-strand model).
//!
//! ## Single-Strand Model
//!
//! In rippled, consensus runs on a single strand: "In general, the idea is
//! that there is only ONE thread that is running consensus code at anytime."
//! (RCLConsensus.h:168-170). This port matches that exactly:
//! - `AppConsensus` is owned by value on the strand thread
//! - All methods take `&mut self` (no interior mutability needed)
//! - No mutex protects the `Consensus` state machine
//! - Proposals, timer_entry, and accept all run on the same thread in FIFO order
//!
//! ## `Ledger` associated type: `RclCxLedger` vs `RclValidatedLedger`
//!
//! Phase 3's `Consensus<Adaptor>::Ledger` associated type must implement
//! `consensus::ConsensusLedger` (id/seq/close-time accessors only) --
//! that's [`consensus::RclCxLedger`], a thin wrapper over `Arc<Ledger>`.
//! Phase 5's validation tracker instead needs `ValidationsLedger`
//! (ancestor-trie lookups for Byzantine-safe preference resolution) --
//! that's [`crate::consensus::rcl_validation::RclValidatedLedger`], a
//! *different* concrete type with its own eagerly-cached ancestor vector.

use basics::unordered_containers::HashSet;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration as StdDuration;

use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use consensus::algorithm::types::{ConsensusCloseTimes, ConsensusMode};
use consensus::{Consensus, ConsensusParms};
use protocol::PublicKey;

use crate::consensus::rcl_cx_peer_pos::{Proposal, RclCxPeerPos, sign_proposal};
use crate::consensus::rcl_validations::SharedAppValidations;
use crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime;
use crate::network::network_ops::AppNetworkOpsModeOwner;
use crate::state::app_registry::{AppInboundTransactions, SharedAppOpenLedger};
use crate::state::application_root::{ApplicationRoot, LedgerAcceptor};
use crate::state::time_keeper::{SystemTimeKeeperClock, TimeKeeper};
use crate::tx_queue::transaction_master::TransactionMaster;
use crate::validator::validator_keys::ValidatorKeys;
use crate::validator::validator_list::ValidatorList;
use overlay::Overlay;

pub type RclCxTx = consensus::RclCxTx;
pub type RclCxLedger = consensus::RclCxLedger;

/// The open-ledger view consensus reads current (not-yet-consensus-agreed)
/// transactions from, and resets once a round is accepted.
pub trait RclConsensusOpenLedgerSource {
    fn current_open_transactions(&self) -> Vec<Arc<protocol::STTx>>;
    fn has_open_transactions(&self) -> bool;
    fn accept_consensus_ledger(
        &self,
        next_seq: u32,
        base_fee: u64,
        parent_hash: &Uint256,
        accepted_ids: &std::collections::HashSet<Uint256>,
    );
}

/// The validation-tracking surface the adaptor needs to answer
/// `proposers_validated`/`proposers_finished`/`get_prev_ledger`.
pub trait RclConsensusValidationSource {
    fn num_trusted_for_ledger(&self, ledger_id: Uint256) -> usize;
    fn get_nodes_after(&self, ledger: &crate::consensus::rcl_validation::RclValidatedLedger, ledger_id: &Uint256) -> usize;
    fn preferred_lcl(&self, lcl: &crate::consensus::rcl_validation::RclValidatedLedger, min_seq: u32, peer_counts: &std::collections::BTreeMap<Uint256, u32>) -> Uint256;
    fn preferred_min_seq(&self, curr: &crate::consensus::rcl_validation::RclValidatedLedger, min_valid_seq: u32) -> Uint256;
}

impl RclConsensusValidationSource for SharedAppValidations<SystemTimeKeeperClock> {
    fn num_trusted_for_ledger(&self, ledger_id: Uint256) -> usize {
        SharedAppValidations::num_trusted_for_ledger(self, ledger_id)
    }

    fn get_nodes_after(&self, ledger: &crate::consensus::rcl_validation::RclValidatedLedger, ledger_id: &Uint256) -> usize {
        self.validations().lock().expect("shared app validations mutex must not be poisoned").get_nodes_after(ledger, ledger_id)
    }

    fn preferred_lcl(&self, lcl: &crate::consensus::rcl_validation::RclValidatedLedger, min_seq: u32, peer_counts: &std::collections::BTreeMap<Uint256, u32>) -> Uint256 {
        self.validations().lock().expect("shared app validations mutex must not be poisoned").get_preferred_lcl(lcl, min_seq, peer_counts)
    }

    fn preferred_min_seq(&self, curr: &crate::consensus::rcl_validation::RclValidatedLedger, min_valid_seq: u32) -> Uint256 {
        self.validations().lock().expect("shared app validations mutex must not be poisoned").get_preferred_min_seq(curr, min_valid_seq)
    }
}

/// Timing and mode overrides for [`AppRclConsensusAdaptor`].
#[derive(Debug, Clone, Copy, Default)]
pub struct AppRclConsensusOptions {
    pub standalone: bool,
    #[allow(dead_code)]
    pub close_time_resolution_override: Option<std::time::Duration>,
}

/// A no-op diagnostics sink for [`AppRclConsensusAdaptor`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NullRclConsensusJournal;

/// Diagnostics sink for consensus-round events.
pub trait RclConsensusJournal: Send + Sync {
    fn trace(&self, message: &str);
    fn info(&self, message: &str);
    fn warn(&self, message: &str);
    fn error(&self, message: &str);
}

impl RclConsensusJournal for NullRclConsensusJournal {
    fn trace(&self, _message: &str) {}
    fn info(&self, _message: &str) {}
    fn warn(&self, _message: &str) {}
    fn error(&self, _message: &str) {}
}

/// Peer relay of consensus artifacts.
pub trait RclConsensusRelay: Send + Sync {
    fn relay_proposal(&self, peer_pos: &RclCxPeerPos);
    fn relay_tx_set(&self, set: &consensus::RclTxSet);
    fn relay_disputed_tx(&self, tx: &consensus::RclCxTxRef);
}

/// The concrete peer-relay implementation.
pub struct AppRclConsensusRelay {
    overlay: Option<Arc<overlay::runtime::overlay_impl::OverlayImpl>>,
    inbound_transactions: AppInboundTransactions,
    validator_keys: ValidatorKeys,
    journal: Arc<dyn RclConsensusJournal>,
}

impl AppRclConsensusRelay {
    pub fn from_application_root(root: &ApplicationRoot, inbound_transactions: AppInboundTransactions, validator_keys: ValidatorKeys, journal: impl RclConsensusJournal + 'static) -> Self {
        Self { overlay: root.overlay_runtime().map(|rt| rt.overlay()), inbound_transactions, validator_keys, journal: Arc::new(journal) }
    }

    pub fn validator_keys(&self) -> &ValidatorKeys {
        &self.validator_keys
    }
}

impl RclConsensusRelay for AppRclConsensusRelay {
    fn relay_proposal(&self, peer_pos: &RclCxPeerPos) {
        let Some(overlay) = self.overlay.as_ref() else {
            self.journal.trace("relay_proposal: no overlay attached, skipping broadcast");
            return;
        };

        let proposal = peer_pos.proposal();
        let message = overlay::TmProposeSet {
            propose_seq: proposal.propose_seq(),
            current_tx_hash: proposal.position().data().to_vec(),
            node_pub_key: peer_pos.public_key().as_bytes().to_vec(),
            close_time: proposal.close_time().as_seconds(),
            signature: peer_pos.signature().to_vec(),
            previousledger: proposal.prev_ledger().data().to_vec(),
            added_transactions: Vec::new(),
            removed_transactions: Vec::new(),
            ..Default::default()
        };

        let _ = overlay.relay_proposal(message, peer_pos.suppression_id(), *peer_pos.public_key());
    }

    fn relay_tx_set(&self, set: &consensus::RclTxSet) {
        let set_id = consensus::ConsensusTxSet::id(set);
        {
            let sync_tree = set.to_sync_tree();
            let mut guard = self.inbound_transactions.lock().expect("inbound_transactions mutex");
            guard.give_set(set_id, std::sync::Arc::new(sync_tree), false);
        }

        let Some(overlay) = self.overlay.as_ref() else {
            self.journal.trace("relay_tx_set: no overlay attached, skipping broadcast");
            return;
        };

        let message = overlay::ProtocolMessage::new(overlay::ProtocolPayload::HaveSet(overlay::TmHaveTransactionSet {
            status: 1, // tsHAVE
            hash: set_id.data().to_vec(),
        }));
        overlay.broadcast(&message);
    }

    fn relay_disputed_tx(&self, tx: &consensus::RclCxTxRef) {
        let Some(overlay) = self.overlay.as_ref() else {
            self.journal.trace("relay_disputed_tx: no overlay attached, skipping broadcast");
            return;
        };

        let raw_transaction = tx.item().data().to_vec();
        let message = overlay::TmTransaction {
            raw_transaction,
            status: 2, // tsCURRENT
            receive_timestamp: None,
            deferred: None,
        };
        overlay.relay_transaction(tx.id(), Some(message), &std::collections::BTreeSet::new());
    }
}

/// Work captured by `on_accept` that must execute synchronously on the
/// consensus strand, matching rippled's single-threaded
/// `doAccept → endConsensus → beginConsensus` call chain.
pub struct PendingAcceptWork {
    pub closed_seq: u32,
    pub close_time: u32,
    pub close_resolution: u8,
    pub correct_close_time: bool,
    pub base_fee_drops: u64,
    pub txns: Vec<Arc<protocol::STTx>>,
    pub validation: Option<crate::state::application_root::PendingValidation>,
}

pub struct AppRclConsensusAdaptor {
    options: AppRclConsensusOptions,
    time_keeper: Arc<TimeKeeper<SystemTimeKeeperClock>>,
    ledger_master_runtime: Arc<AppLedgerMasterRuntime>,
    open_ledger: SharedAppOpenLedger,
    validations: SharedAppValidations<SystemTimeKeeperClock>,
    app_root: crate::state::application_root::ApplicationRoot,
    #[allow(dead_code)]
    pub(crate) validators: Arc<ValidatorList>,
    #[allow(dead_code)]
    pub(crate) network_ops_mode_owner: AppNetworkOpsModeOwner,
    ledger_acceptor: Arc<dyn LedgerAcceptor>,
    inbound_transactions: AppInboundTransactions,
    transaction_master: Arc<TransactionMaster>,
    relay: AppRclConsensusRelay,
    journal: Arc<dyn RclConsensusJournal>,
    pub(crate) validator_keys: ValidatorKeys,
    #[allow(dead_code)]
    negative_unl_vote: Option<Arc<crate::amendments::negative_unl_vote::NegativeUNLVote>>,
    #[allow(dead_code)]
    amendment_status: Option<Arc<crate::amendments::amendment_status::AmendmentStatus>>,
    #[allow(dead_code)]
    overlay: Option<Arc<overlay::runtime::overlay_impl::OverlayImpl>>,
    parms: ConsensusParms,
    tx_set_cache: consensus::rcl::RclTxSetSharedCache,
    /// Accept-work captured by `on_accept` for synchronous execution by
    /// the strand thread. In the single-strand model, this is read
    /// immediately after `timer_entry` returns on the same thread.
    /// Uses a Mutex to satisfy the type system (ConsensusAdaptor::on_accept
    /// takes &self), but only one thread ever accesses this.
    pub(crate) pending_accept: StdMutex<Option<PendingAcceptWork>>,
}

#[allow(clippy::too_many_arguments)]
impl AppRclConsensusAdaptor {
    pub fn new(
        options: AppRclConsensusOptions,
        time_keeper: Arc<TimeKeeper<SystemTimeKeeperClock>>,
        ledger_master_runtime: Arc<AppLedgerMasterRuntime>,
        open_ledger: SharedAppOpenLedger,
        validations: SharedAppValidations<SystemTimeKeeperClock>,
        validators: Arc<ValidatorList>,
        network_ops_mode_owner: AppNetworkOpsModeOwner,
        ledger_acceptor: Arc<dyn LedgerAcceptor>,
        inbound_transactions: AppInboundTransactions,
        transaction_master: Arc<TransactionMaster>,
        relay: AppRclConsensusRelay,
        journal: impl RclConsensusJournal + 'static,
        validator_keys: ValidatorKeys,
        negative_unl_vote: Option<Arc<crate::amendments::negative_unl_vote::NegativeUNLVote>>,
        amendment_status: Option<Arc<crate::amendments::amendment_status::AmendmentStatus>>,
        overlay: Option<Arc<overlay::runtime::overlay_impl::OverlayImpl>>,
        app_root: crate::state::application_root::ApplicationRoot,
    ) -> Self {
        let tx_set_cache: consensus::rcl::RclTxSetSharedCache = Arc::new(shamap::tree_node_cache::TreeNodeCache::new(
            "consensus-tx-set-cache",
            256,
            time::Duration::minutes(5),
            basics::tagged_cache::MonotonicClock::default(),
        ));
        Self {
            options,
            time_keeper,
            ledger_master_runtime,
            open_ledger,
            validations,
            app_root,
            validators,
            network_ops_mode_owner,
            ledger_acceptor,
            inbound_transactions,
            transaction_master,
            relay,
            journal: Arc::new(journal),
            validator_keys,
            negative_unl_vote,
            amendment_status,
            overlay,
            parms: ConsensusParms::default(),
            tx_set_cache,
            pending_accept: StdMutex::new(None),
        }
    }

    #[allow(dead_code)]
    fn now(&self) -> NetClockTimePoint {
        self.time_keeper.close_time()
    }

    pub fn is_validator(&self) -> bool {
        self.validator_keys.keys.is_some()
    }

    fn validated_view(&self, ledger: &RclCxLedger) -> crate::consensus::rcl_validation::RclValidatedLedger {
        crate::consensus::rcl_validation::RclValidatedLedger::from_ledger(&ledger.ledger())
    }

    fn sync_tree_to_rcl_tx_set(&self, sync_tree: &shamap::sync::SyncTree) -> consensus::RclTxSet {
        sync_tree_to_rcl_tx_set(sync_tree, &self.tx_set_cache)
    }

    pub fn tx_set_cache(&self) -> &consensus::rcl::RclTxSetSharedCache {
        &self.tx_set_cache
    }
}

fn sync_tree_to_rcl_tx_set(sync_tree: &shamap::sync::SyncTree, cache: &consensus::rcl::RclTxSetSharedCache) -> consensus::RclTxSet {
    consensus::RclTxSet::from_parts(sync_tree.root(), Arc::clone(cache), sync_tree.backed(), 0)
}

impl consensus::algorithm::ConsensusAdaptor for AppRclConsensusAdaptor {
    type Ledger = RclCxLedger;
    type NodeId = PublicKey;
    type TxSet = consensus::RclTxSet;
    type PeerPos = RclCxPeerPos;

    fn acquire_ledger(&self, ledger_id: &Uint256) -> Option<Self::Ledger> {
        let hash = basics::sha_map_hash::SHAMapHash::new(*ledger_id);
        if let Some(ledger) = self.ledger_master_runtime.ledger_master().ledger_history().get_cached_ledger_by_hash(hash) {
            return Some(RclCxLedger::new(ledger));
        }

        if let Some(guard) = self.ledger_master_runtime.shared_inbound_ledgers.lock().ok()
            && let Some(shared) = guard.as_ref()
        {
            shared.acquire_for_consensus(*ledger_id, 0);
        }
        None
    }

    fn acquire_tx_set(&self, set_id: &Uint256) -> Option<Self::TxSet> {
        let sync_tree = {
            let mut guard = self.inbound_transactions.lock().expect("inbound transactions mutex must not be poisoned");
            guard.get_set(*set_id, true)?
        };
        Some(self.sync_tree_to_rcl_tx_set(&sync_tree))
    }

    fn has_open_transactions(&self) -> bool {
        RclConsensusOpenLedgerSource::has_open_transactions(&self.open_ledger)
    }

    fn proposers_validated(&self, prev_ledger: &Uint256) -> usize {
        RclConsensusValidationSource::num_trusted_for_ledger(&self.validations, *prev_ledger)
    }

    fn proposers_finished(&self, prev_ledger: &Self::Ledger, prev_ledger_id: &Uint256) -> usize {
        let wrapped = self.validated_view(prev_ledger);
        RclConsensusValidationSource::get_nodes_after(&self.validations, &wrapped, prev_ledger_id)
    }

    fn get_prev_ledger(&self, _prev_ledger_id: &Uint256, prev_ledger: &Self::Ledger, _mode: ConsensusMode) -> Uint256 {
        let min_valid_seq = self.ledger_master_runtime.ledger_master().valid_ledger_seq();
        let wrapped = self.validated_view(prev_ledger);
        RclConsensusValidationSource::preferred_min_seq(&self.validations, &wrapped, min_valid_seq)
    }

    fn on_mode_change(&self, before: ConsensusMode, after: ConsensusMode) {
        self.journal.info(&format!("Consensus mode change {before:?} -> {after:?}"));
    }

    fn on_close(&self, prev_ledger: &Self::Ledger, now: NetClockTimePoint, _mode: ConsensusMode) -> consensus::algorithm::consensus::ConsensusResultOf<Self> {
        let _ = self.app_root.apply_network_ops_pending_to_open_ledger();

        let txs = RclConsensusOpenLedgerSource::current_open_transactions(&self.open_ledger);
        let mut set = consensus::RclTxSet::new(Arc::clone(&self.tx_set_cache), prev_ledger.seq() + 1);
        {
            let mut editable = set.mutable_view();
            for tx in &txs {
                editable.insert(&consensus::RclCxTxRef::from_transaction(tx));
            }
            set = editable.freeze();
        }

        let position_id = consensus::ConsensusTxSet::id(&set);
        let node_id = self.validator_keys.keys.as_ref().map(|k| k.public_key).unwrap_or_else(|| PublicKey::from_bytes([0u8; 33]));
        let position = Proposal::new(prev_ledger.id(), 0, position_id, now, now, node_id);

        {
            let sync_tree = set.to_sync_tree();
            let mut guard = self.inbound_transactions.lock().expect("inbound_transactions mutex");
            guard.give_set(position_id, Arc::new(sync_tree), false);
            tracing::info!(target: "consensus", %position_id, "on_close: stored tx-set");
        }

        consensus::algorithm::types::ConsensusResult::new(set, position, position_id)
    }

    fn on_accept(
        &self,
        result: &consensus::algorithm::consensus::ConsensusResultOf<Self>,
        prev_ledger: &Self::Ledger,
        close_resolution: StdDuration,
        raw_close_times: &ConsensusCloseTimes,
        mode: ConsensusMode,
    ) {
        let next_seq = prev_ledger.seq() + 1;
        let base_fee = self.ledger_master_runtime.ledger_master().closed_ledger().map(|l| l.fees().base).unwrap_or(10);

        let txns: Vec<Arc<protocol::STTx>> = result
            .txns
            .all_items()
            .into_iter()
            .map(|item| {
                let mut sit: protocol::SerialIter<'_> = item.data().into();
                Arc::new(protocol::STTx::from_serial_iter(&mut sit))
            })
            .collect();
        let accepted_ids: std::collections::HashSet<Uint256> =
            txns.iter().map(|tx| tx.get_transaction_id()).collect();

        RclConsensusOpenLedgerSource::accept_consensus_ledger(&self.open_ledger, next_seq, base_fee, &prev_ledger.id(), &accepted_ids);

        let raw_close_time = result.position.close_time();
        let close_time_correct = raw_close_time != NetClockTimePoint::default();
        let effective_close_time = if !close_time_correct {
            NetClockTimePoint::new(prev_ledger.close_time().as_seconds().saturating_add(1))
        } else {
            let resolution = time::Duration::seconds(close_resolution.as_secs() as i64);
            consensus::algorithm::timing::effective_close_time(raw_close_time, resolution, prev_ledger.close_time())
        };
        let close_time = effective_close_time.as_seconds();
        let close_resolution_secs = close_resolution.as_secs().min(u8::MAX as u64) as u8;
        let closed_seq = next_seq;

        let pending_validation = self.validator_keys.keys.as_ref().map(|keys| crate::state::application_root::PendingValidation {
            public_key: keys.public_key,
            secret_key: keys.secret_key.clone(),
            node_id: protocol::calc_node_id(&keys.public_key),
            consensus_hash: result.txns.id(),
            proposing: mode == ConsensusMode::Proposing,
        });

        // Clock adjustment: converge toward network's observed close time
        let consensus_fail = result.state == consensus::algorithm::types::ConsensusState::MovedOn;
        if (mode == ConsensusMode::Proposing || mode == ConsensusMode::Observing) && !consensus_fail {
            let close_time_val = raw_close_times.self_;
            let mut close_total: i64 = i64::from(close_time_val.as_seconds());
            let mut close_count: i64 = 1;
            for (t, v) in &raw_close_times.peers {
                close_count += i64::from(*v);
                close_total += i64::from(t.as_seconds()) * i64::from(*v);
            }
            close_total += close_count / 2;
            close_total /= close_count;

            let offset_seconds = close_total - i64::from(close_time_val.as_seconds());
            let new_offset = self.time_keeper.adjust_close_time(time::Duration::seconds(offset_seconds));
            tracing::debug!(
                target: "consensus",
                self_close_time = close_time_val.as_seconds(),
                peer_vote_count = raw_close_times.peers.len(),
                computed_offset_seconds = offset_seconds,
                new_close_offset_seconds = new_offset.whole_seconds(),
                "close_time_adjust"
            );
        }

        // Store the accept work in the adaptor's pending_accept field.
        // In the single-strand model, on_accept is called from timer_entry
        // on the strand thread. The strand thread reads pending_accept
        // immediately after timer_entry returns.
        {
            let mut pending = self.pending_accept.lock()
                .expect("pending_accept mutex must not be poisoned");
            *pending = Some(PendingAcceptWork {
                closed_seq,
                close_time,
                close_resolution: close_resolution_secs,
                correct_close_time: close_time_correct,
                base_fee_drops: base_fee,
                txns,
                validation: pending_validation,
            });
        }
        tracing::debug!(target: "consensus", closed_seq, "on_accept: stored pending accept work for synchronous execution");
    }

    fn propose(&self, pos: &consensus::ConsensusProposal<PublicKey, Uint256, Uint256>) {
        let Some(keys) = self.validator_keys.keys.as_ref() else {
            return;
        };
        match sign_proposal(&keys.secret_key, &keys.public_key, pos) {
            Ok((signature, suppression)) => {
                let peer_pos = RclCxPeerPos::new(keys.public_key, signature, suppression, pos.clone());
                self.relay.relay_proposal(&peer_pos);
            }
            Err(err) => self.journal.error(&format!("propose: signing failed: {err:?}")),
        }
    }

    fn share_peer_position(&self, prop: &Self::PeerPos) {
        self.relay.relay_proposal(prop);
    }

    fn share_tx(&self, tx: &consensus::RclCxTxRef) {
        self.relay.relay_disputed_tx(tx);
    }

    fn share_tx_set(&self, set: &Self::TxSet) {
        self.relay.relay_tx_set(set);
    }

    fn parms(&self) -> &ConsensusParms {
        &self.parms
    }

    fn next_ledger_time_resolution(&self, previous_resolution: StdDuration, previous_agree: bool, ledger_seq: u32) -> StdDuration {
        let previous = time::Duration::seconds(previous_resolution.as_secs() as i64);
        let next = consensus::algorithm::timing::get_next_ledger_time_resolution(previous, previous_agree, ledger_seq);
        StdDuration::from_secs(next.whole_seconds().max(0) as u64)
    }

    fn round_close_time(&self, raw: NetClockTimePoint, resolution: StdDuration) -> NetClockTimePoint {
        let resolution = time::Duration::seconds(resolution.as_secs() as i64);
        consensus::algorithm::timing::round_close_time(raw, resolution)
    }
}

// ---------------------------------------------------------------------------
// Single-Strand ConsensusRunner
// ---------------------------------------------------------------------------

/// The consensus runner trait for the single-strand model. All methods take
/// `&mut self` because only the strand thread ever calls them — no locks needed.
///
/// This trait exists so `AppConsensus` can be constructed in `application_root.rs`
/// and then moved to the strand thread in `bootstrap.rs`.
pub trait ConsensusRunner: Send {
    /// Process a proposal. Called on the consensus strand.
    fn peer_proposal(&mut self, now: NetClockTimePoint, peer_pos: &RclCxPeerPos) -> bool;

    /// Run the 1s timer tick. Returns PendingAcceptWork if on_accept fired.
    fn timer_tick(&mut self, now: NetClockTimePoint) -> Option<PendingAcceptWork>;

    /// Start a round. Called after ledger build or by initial bootstrap.
    fn start_round(&mut self, now: NetClockTimePoint, prev_ledger_id: Uint256, prev_ledger: RclCxLedger, proposing: bool);

    /// Notify tx-set acquired.
    fn got_tx_set(&mut self, now: NetClockTimePoint, tx_set: consensus::RclTxSet);

    /// Build the accepted ledger and start the next round.
    /// Called on the strand after timer_tick returns Some(work).
    fn execute_accept(&mut self, now: NetClockTimePoint, work: PendingAcceptWork);

    /// Phase accessor.
    fn phase(&self) -> consensus::algorithm::ConsensusPhase;

    /// Prev ledger id accessor.
    fn prev_ledger_id(&self) -> Uint256;
}

/// Concrete single-strand consensus driver. Owns `Consensus<AppRclConsensusAdaptor>`
/// directly — NO mutex, NO Arc. Lives on the strand thread's stack.
pub struct AppConsensus {
    pub(crate) adaptor: AppRclConsensusAdaptor,
    state: Consensus<AppRclConsensusAdaptor>,
}

impl AppConsensus {
    pub fn new(adaptor: AppRclConsensusAdaptor, _parms: ConsensusParms) -> Self {
        Self { adaptor, state: Consensus::new() }
    }

    /// Execute the accept-ledger work and start the next consensus round,
    /// matching rippled's single-threaded flow:
    ///   doAccept (build ledger) → endConsensus (checkLastClosedLedger) →
    ///   beginConsensus (start_round)
    fn do_accept_and_start_next_round(&mut self, now: NetClockTimePoint, work: PendingAcceptWork) {
        let closed_seq = work.closed_seq;
        let root = self.adaptor.app_root.clone();

        match root.accept_ledger_with_txns(
            work.closed_seq,
            work.close_time,
            work.close_resolution,
            work.correct_close_time,
            work.base_fee_drops,
            work.txns,
        ) {
            Ok(_) => {
                // Broadcast StatusChange (neACCEPTED_LEDGER) and sign/publish validation.
                if let Some(closed) = root.closed_ledger() {
                    let hdr = closed.header();
                    if let Some(overlay_rt) = root.overlay_runtime() {
                        use overlay::Overlay;
                        let status = overlay::ProtocolMessage::new(overlay::ProtocolPayload::StatusChange(overlay::message::wire::TmStatusChange {
                            new_status: None,
                            new_event: Some(2), // neACCEPTED_LEDGER
                            ledger_seq: Some(hdr.seq),
                            ledger_hash: Some(hdr.hash.as_uint256().data().to_vec()),
                            ledger_hash_previous: Some(hdr.parent_hash.as_uint256().data().to_vec()),
                            network_time: None,
                            first_seq: Some(0),
                            last_seq: Some(0),
                        }));
                        overlay_rt.overlay().broadcast(&status);
                    }

                    if let Some(pending) = work.validation {
                        tracing::info!(target: "consensus", closed_seq, proposing = pending.proposing, "execute_accept: signing validation");
                        let ledger_hash = *hdr.hash.as_uint256();
                        match protocol::STValidation::new_signed(work.close_time.max(1), &pending.public_key, pending.node_id, &pending.secret_key, |v| {
                            v.set_field_h256(protocol::get_field_by_symbol("sfLedgerHash"), ledger_hash);
                            v.set_field_h256(protocol::get_field_by_symbol("sfConsensusHash"), pending.consensus_hash);
                            v.set_field_u32(protocol::get_field_by_symbol("sfLedgerSequence"), closed_seq);
                            if pending.proposing {
                                v.set_flag(protocol::VF_FULL_VALIDATION);
                            }
                        }) {
                            Ok(built_validation) => {
                                self.adaptor.ledger_acceptor.publish_validation(Arc::new(built_validation));
                                tracing::info!(target: "consensus", closed_seq, "execute_accept: validation SIGNED and PUBLISHED");
                            }
                            Err(err) => {
                                tracing::error!(target: "consensus", closed_seq, ?err, "synchronous accept: validation signing failed");
                            }
                        }
                    }
                }

                // checkLastClosedLedger → beginConsensus
                let closed = root.closed_ledger();
                if let Some(closed) = closed {
                    let closed_id = *closed.header().hash.as_uint256();
                    let network_closed = if let Some(ort) = root.overlay_runtime() {
                        use overlay::Overlay;
                        let peers = ort.overlay().active_peers();
                        if peers.len() >= 3 {
                            let mut counts = std::collections::HashMap::<Uint256, u32>::new();
                            *counts.entry(closed_id).or_insert(0) += 1;
                            for peer in &peers {
                                let h = peer.closed_ledger_hash();
                                if !h.is_zero() {
                                    *counts.entry(h).or_insert(0) += 1;
                                }
                            }
                            let preferred = counts.iter()
                                .max_by_key(|(_, c)| *c)
                                .map(|(h, _)| *h)
                                .unwrap_or(closed_id);
                            if preferred != closed_id
                                && preferred != *closed.header().parent_hash.as_uint256()
                            {
                                preferred
                            } else {
                                closed_id
                            }
                        } else {
                            closed_id
                        }
                    } else {
                        closed_id
                    };

                    let round_ledger = if network_closed != closed_id {
                        tracing::info!(
                            target: "consensus",
                            %closed_id, %network_closed,
                            "synchronous accept: peers prefer different chain"
                        );
                        if let Some(lm_rt) = root.ledger_master_runtime() {
                            if let Some(network_ledger) = lm_rt.ledger_master().get_ledger_by_hash(
                                basics::sha_map_hash::SHAMapHash::new(network_closed)
                            ) {
                                root.on_closed_ledger(Arc::clone(&network_ledger));
                                Some(network_ledger)
                            } else {
                                // Network ledger not locally available — acquire from peers.
                                // Do NOT demote operating mode or set need_network_ledger.
                                // The consensus timer's checkLedger (inside timer_entry) will
                                // detect the mismatch via handleWrongLedger on the next tick
                                // and advance via SwitchedLedger mode. This matches rippled
                                // where doAccept never demotes the operating mode.
                                if let Ok(guard) = lm_rt.shared_inbound_ledgers.lock() {
                                    if let Some(shared) = guard.as_ref() {
                                        shared.acquire_for_consensus(network_closed, 0);
                                    }
                                }
                                None
                            }
                        } else {
                            Some(Arc::clone(&closed))
                        }
                    } else {
                        Some(Arc::clone(&closed))
                    };

                    if let Some(round_ledger) = round_ledger {
                        let proposing = self.adaptor.is_validator()
                            && !self.adaptor.options.standalone
                            && self.adaptor.network_ops_mode_owner.operating_mode()
                                == crate::network::network_ops::NetworkOpsOperatingMode::Full;
                        let prev_id = *round_ledger.header().hash.as_uint256();
                        let prev_cx = crate::consensus_ledger_from_ledger(&round_ledger);

                        if self.adaptor.validators.count() > 0 && self.adaptor.validators.unl_size() == 0 {
                            self.adaptor.network_ops_mode_owner.set_unl_blocked(true);
                        } else {
                            self.adaptor.network_ops_mode_owner.set_unl_blocked(false);
                        }

                        self.state.start_round(
                            &self.adaptor,
                            now,
                            prev_id,
                            prev_cx,
                            &HashSet::default(),
                            proposing,
                        );
                        tracing::info!(
                            target: "consensus",
                            closed_seq,
                            next_prev_ledger = %prev_id,
                            "synchronous accept: started next round inline"
                        );
                    }
                } else {
                    root.notify_tx_pending();
                }
            }
            Err(err) => {
                tracing::error!(target: "consensus", closed_seq, %err, "synchronous accept: accept_ledger_with_txns failed");
            }
        }
    }
}

impl ConsensusRunner for AppConsensus {
    fn peer_proposal(&mut self, now: NetClockTimePoint, peer_pos: &RclCxPeerPos) -> bool {
        // Signature already verified by the overlay layer before queueing.
        // Matches rippled where processTrustedProposal trusts the overlay's
        // prior validation.
        let our_prev = *self.state.prev_ledger_id();
        let their_prev = *peer_pos.proposal().prev_ledger();
        let accepted = self.state.peer_proposal(&self.adaptor, now, peer_pos);
        if !accepted && our_prev != their_prev {
            tracing::info!(target: "consensus",
                %our_prev, %their_prev,
                phase = ?self.state.phase(),
                "peer_proposal REJECTED: prev_ledger mismatch"
            );
        }
        accepted
    }

    fn timer_tick(&mut self, now: NetClockTimePoint) -> Option<PendingAcceptWork> {
        self.state.timer_entry(&self.adaptor, now);
        // If on_accept fired during timer_entry, pending_accept will be Some.
        self.adaptor.pending_accept.lock()
            .expect("pending_accept mutex must not be poisoned")
            .take()
    }

    fn start_round(&mut self, now: NetClockTimePoint, prev_ledger_id: Uint256, prev_ledger: RclCxLedger, proposing: bool) {
        if self.adaptor.validators.count() > 0 && self.adaptor.validators.unl_size() == 0 {
            self.adaptor.network_ops_mode_owner.set_unl_blocked(true);
        } else {
            self.adaptor.network_ops_mode_owner.set_unl_blocked(false);
        }
        let actual_proposing = proposing
            && self.adaptor.is_validator()
            && !self.adaptor.options.standalone
            && self.adaptor.network_ops_mode_owner.operating_mode()
                == crate::network::network_ops::NetworkOpsOperatingMode::Full;
        self.state.start_round(&self.adaptor, now, prev_ledger_id, prev_ledger, &HashSet::default(), actual_proposing);
    }

    fn got_tx_set(&mut self, now: NetClockTimePoint, tx_set: consensus::RclTxSet) {
        self.state.got_tx_set(&self.adaptor, now, &tx_set);
    }

    fn execute_accept(&mut self, now: NetClockTimePoint, work: PendingAcceptWork) {
        self.do_accept_and_start_next_round(now, work);
    }

    fn phase(&self) -> consensus::algorithm::ConsensusPhase {
        self.state.phase()
    }

    fn prev_ledger_id(&self) -> Uint256 {
        *self.state.prev_ledger_id()
    }
}

#[cfg(test)]
mod sync_tree_conversion_tests {
    use super::sync_tree_to_rcl_tx_set;
    use basics::hardened_hash::HardenedHashBuilder;
    use basics::tagged_cache::MonotonicClock;
    use protocol::{STAmount, STTx, TxType, get_field_by_symbol, serialize_blob};
    use shamap::item::SHAMapItem;
    use shamap::storage::StorageTree;
    use shamap::sync::{SHAMapType, SyncState, SyncTree};
    use shamap::tree_node::SHAMapNodeType;
    use std::sync::Arc;

    fn cache() -> consensus::rcl::RclTxSetSharedCache {
        Arc::new(shamap::tree_node_cache::TreeNodeCache::new(
            "sync-tree-conversion-test",
            32,
            time::Duration::minutes(5),
            basics::tagged_cache::MonotonicClock::default(),
        ))
    }

    fn payment(fill: u8) -> STTx {
        STTx::new(TxType::PAYMENT, |tx| {
            tx.set_field_u32(get_field_by_symbol("sfSequence"), u32::from(fill));
            tx.set_field_amount(get_field_by_symbol("sfAmount"), STAmount::new_native(u64::from(fill), false));
            tx.set_field_amount(get_field_by_symbol("sfFee"), STAmount::new_native(10, false));
        })
    }

    fn completed_sync_tree(txs: &[STTx], cache: consensus::rcl::RclTxSetSharedCache) -> SyncTree {
        let cache: Arc<shamap::tree_node_cache::TreeNodeCache<MonotonicClock, HardenedHashBuilder>> = cache;
        let mut map = StorageTree::new(1, false, 1, cache);
        for tx in txs {
            let item = SHAMapItem::new(tx.get_transaction_id(), serialize_blob(tx));
            map.add_item(SHAMapNodeType::TransactionNm, item).expect("insert into a fresh test tree should not need fetches");
        }
        map.unshare();

        let tree = SyncTree::from_root_with_type(map.root(), SHAMapType::Transaction, false, 1, SyncState::Modifying);
        tree.set_full();
        tree
    }

    #[test]
    fn sync_tree_to_rcl_tx_set_preserves_id_and_membership() {
        let tx1 = payment(1);
        let tx2 = payment(2);
        let tx1_id = tx1.get_transaction_id();
        let tx2_id = tx2.get_transaction_id();

        let shared_cache = cache();
        let tree = completed_sync_tree(&[tx1, tx2], cache());
        let adopted = sync_tree_to_rcl_tx_set(&tree, &shared_cache);

        assert!(adopted.exists(tx1_id));
        assert!(adopted.exists(tx2_id));
        assert_eq!(adopted.id(), *tree.root().get_hash().as_uint256());
    }

    #[test]
    fn sync_tree_to_rcl_tx_set_matches_hash_of_independently_built_equivalent_set() {
        let tx1 = payment(3);
        let tx2 = payment(4);

        let tree = completed_sync_tree(&[tx1.clone(), tx2.clone()], cache());
        let adopted = sync_tree_to_rcl_tx_set(&tree, &cache());

        let mut rebuilt = consensus::RclTxSet::new(cache(), 1);
        {
            let mut editable = rebuilt.mutable_view();
            editable.insert(&consensus::RclCxTxRef::from_transaction(&tx1));
            editable.insert(&consensus::RclCxTxRef::from_transaction(&tx2));
            rebuilt = editable.freeze();
        }

        assert_eq!(adopted.id(), rebuilt.id());
    }
}
