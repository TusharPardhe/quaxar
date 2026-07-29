//! App-level wiring of the generic `Consensus<Adaptor>` state machine
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
//! ## Two ledger types: `RclCxLedger` vs `RclValidatedLedger`
//!
//! The `Consensus<Adaptor>::Ledger` associated type must implement
//! `consensus::ConsensusLedger` (id/seq/close-time accessors only) —
//! that's [`consensus::RclCxLedger`], a thin wrapper over `Arc<Ledger>`.
//! The validation tracker instead needs `ValidationsLedger`
//! (ancestor-trie lookups for Byzantine-safe preference resolution) —
//! that's [`crate::consensus::rcl_validation::RclValidatedLedger`], a
//! *different* concrete type with its own eagerly-cached ancestor vector.

use basics::unordered_containers::HashSet;
use std::collections::HashSet as StdHashSet;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration as StdDuration;

use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use basics::sha_map_hash::SHAMapHash;
use consensus::algorithm::types::{ConsensusCloseTimes, ConsensusMode};
use consensus::{Consensus, ConsensusParms};
use protocol::PublicKey;

use crate::consensus::rcl_cx_peer_pos::{Proposal, RclCxPeerPos, sign_proposal};
use crate::consensus::rcl_validations::SharedAppValidations;
use crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime;
use crate::load::fee_vote::FeeVote;
use crate::network::network_ops::{AppNetworkOpsModeOwner, NetworkOpsOperatingMode};
use crate::state::app_registry::{AppInboundTransactions, SharedAppOpenLedger};
use crate::state::application_root::{ApplicationRoot, LedgerAcceptor};
use crate::state::time_keeper::{SystemTimeKeeperClock, TimeKeeper};
use crate::tx_queue::transaction_master::TransactionMaster;
use crate::validator::validator_keys::ValidatorKeys;
use crate::validator::validator_list::ValidatorList;
use ledger::CanonicalTXSet;
use overlay::Overlay;

pub type RclCxTx = consensus::RclCxTx;
pub type RclCxLedger = consensus::RclCxLedger;

fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
        })
        .unwrap_or_else(|| "non-string panic payload".to_owned())
}

/// Decode peer-supplied consensus transaction bytes at the same boundary where
/// rippled constructs its canonical consensus transaction set. A malformed
/// entry is reported but does not prevent valid entries from being applied.
fn decode_consensus_accept_transactions<'a>(
    tx_set_id: Uint256,
    items: impl IntoIterator<Item = (usize, &'a [u8])>,
) -> (Vec<Arc<protocol::STTx>>, Vec<(usize, String)>) {
    let mut txns = CanonicalTXSet::new(tx_set_id);
    let mut malformed = Vec::new();

    for (payload_len, bytes) in items {
        if !(protocol::TX_MIN_SIZE_BYTES..=protocol::TX_MAX_SIZE_BYTES).contains(&payload_len) {
            malformed.push((
                payload_len,
                format!("transaction payload length {payload_len} is outside the legal range"),
            ));
            continue;
        }

        let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut sit: protocol::SerialIter<'_> = bytes.into();
            let tx = protocol::STTx::from_serial_iter(&mut sit);
            if !sit.empty() {
                return Err("transaction payload has trailing bytes".to_owned());
            }
            Ok(tx)
        }));
        match parsed {
            Ok(Ok(tx)) => txns.insert(Arc::new(tx)),
            Ok(Err(message)) => malformed.push((payload_len, message)),
            Err(payload) => malformed.push((payload_len, panic_payload_message(payload))),
        }
    }

    (txns.drain_ordered(), malformed)
}

fn pseudo_transaction_voting_enabled(options: AppRclConsensusOptions, mode: ConsensusMode) -> bool {
    options.standalone || mode == ConsensusMode::Proposing
}

fn trusted_validation_quorum_reached(validations: usize, quorum: usize) -> bool {
    validations >= quorum
}

fn update_operating_mode_after_accept(
    network_ops_mode_owner: &AppNetworkOpsModeOwner,
    positions: usize,
) {
    if positions == 0 && network_ops_mode_owner.operating_mode() == NetworkOpsOperatingMode::Full {
        network_ops_mode_owner.set_operating_mode_direct(NetworkOpsOperatingMode::Connected);
    }
}

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
        completed_transaction_ids: &std::collections::HashSet<Uint256>,
        retry_transactions: &[Arc<protocol::STTx>],
    );
}

/// The validation-tracking surface the adaptor needs to answer
/// `proposers_validated`/`proposers_finished`/`get_prev_ledger`.
pub trait RclConsensusValidationSource {
    fn num_trusted_for_ledger(&self, ledger_id: Uint256) -> usize;
    fn get_nodes_after(
        &self,
        ledger: &crate::consensus::rcl_validation::RclValidatedLedger,
        ledger_id: &Uint256,
    ) -> usize;
    fn preferred_lcl(
        &self,
        lcl: &crate::consensus::rcl_validation::RclValidatedLedger,
        min_seq: u32,
        peer_counts: &std::collections::BTreeMap<Uint256, u32>,
    ) -> Uint256;
    fn preferred_min_seq(
        &self,
        curr: &crate::consensus::rcl_validation::RclValidatedLedger,
        min_valid_seq: u32,
    ) -> Uint256;
}

impl RclConsensusValidationSource for SharedAppValidations<SystemTimeKeeperClock> {
    fn num_trusted_for_ledger(&self, ledger_id: Uint256) -> usize {
        SharedAppValidations::num_trusted_for_ledger(self, ledger_id)
    }

    fn get_nodes_after(
        &self,
        ledger: &crate::consensus::rcl_validation::RclValidatedLedger,
        ledger_id: &Uint256,
    ) -> usize {
        self.validations()
            .lock()
            .expect("shared app validations mutex must not be poisoned")
            .get_nodes_after(ledger, ledger_id)
    }

    fn preferred_lcl(
        &self,
        lcl: &crate::consensus::rcl_validation::RclValidatedLedger,
        min_seq: u32,
        peer_counts: &std::collections::BTreeMap<Uint256, u32>,
    ) -> Uint256 {
        self.validations()
            .lock()
            .expect("shared app validations mutex must not be poisoned")
            .get_preferred_lcl(lcl, min_seq, peer_counts)
    }

    fn preferred_min_seq(
        &self,
        curr: &crate::consensus::rcl_validation::RclValidatedLedger,
        min_valid_seq: u32,
    ) -> Uint256 {
        self.validations()
            .lock()
            .expect("shared app validations mutex must not be poisoned")
            .get_preferred_min_seq(curr, min_valid_seq)
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
    pub fn from_application_root(
        root: &ApplicationRoot,
        inbound_transactions: AppInboundTransactions,
        validator_keys: ValidatorKeys,
        journal: impl RclConsensusJournal + 'static,
    ) -> Self {
        Self {
            overlay: root.overlay_runtime().map(|rt| rt.overlay()),
            inbound_transactions,
            validator_keys,
            journal: Arc::new(journal),
        }
    }

    pub fn validator_keys(&self) -> &ValidatorKeys {
        &self.validator_keys
    }
}

impl RclConsensusRelay for AppRclConsensusRelay {
    fn relay_proposal(&self, peer_pos: &RclCxPeerPos) {
        let Some(overlay) = self.overlay.as_ref() else {
            self.journal
                .trace("relay_proposal: no overlay attached, skipping broadcast");
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
            let mut guard = self
                .inbound_transactions
                .lock()
                .expect("inbound_transactions mutex");
            guard.give_set(set_id, std::sync::Arc::new(sync_tree), false);
        }

        let Some(overlay) = self.overlay.as_ref() else {
            self.journal
                .trace("relay_tx_set: no overlay attached, skipping broadcast");
            return;
        };

        let message = overlay::ProtocolMessage::new(overlay::ProtocolPayload::HaveSet(
            overlay::TmHaveTransactionSet {
                status: 1, // tsHAVE
                hash: set_id.data().to_vec(),
            },
        ));
        overlay.broadcast(&message);
    }

    fn relay_disputed_tx(&self, tx: &consensus::RclCxTxRef) {
        let Some(overlay) = self.overlay.as_ref() else {
            self.journal
                .trace("relay_disputed_tx: no overlay attached, skipping broadcast");
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
    /// Hash of the consensus transaction set that built this ledger.
    pub consensus_hash: Uint256,
    /// `false` means consensus accepted while on a wrong LCL, so the
    /// outward status event must be `neLOST_SYNC`.
    pub have_correct_lcl: bool,
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
    pub(crate) validators: Arc<ValidatorList>,
    #[allow(dead_code)]
    pub(crate) network_ops_mode_owner: AppNetworkOpsModeOwner,
    ledger_acceptor: Arc<dyn LedgerAcceptor>,
    inbound_transactions: AppInboundTransactions,
    transaction_master: Arc<TransactionMaster>,
    relay: AppRclConsensusRelay,
    journal: Arc<dyn RclConsensusJournal>,
    pub(crate) validator_keys: ValidatorKeys,
    fee_vote: Option<Arc<FeeVote>>,
    negative_unl_vote: Option<Arc<crate::amendments::negative_unl_vote::NegativeUNLVote>>,
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
    /// Ledger most recently requested by consensus after a cache miss.
    ///
    /// This mirrors rippled's `acquiringLedger_`: consensus may ask for the
    /// same unavailable LCL on every timer/proposal pass, but only the first
    /// miss should begin its potentially expensive inbound acquisition.
    acquiring_ledger: StdMutex<Option<Uint256>>,
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
        fee_vote: Option<Arc<FeeVote>>,
        negative_unl_vote: Option<Arc<crate::amendments::negative_unl_vote::NegativeUNLVote>>,
        amendment_status: Option<Arc<crate::amendments::amendment_status::AmendmentStatus>>,
        overlay: Option<Arc<overlay::runtime::overlay_impl::OverlayImpl>>,
        app_root: crate::state::application_root::ApplicationRoot,
    ) -> Self {
        let tx_set_cache: consensus::rcl::RclTxSetSharedCache =
            Arc::new(shamap::tree_node_cache::TreeNodeCache::new(
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
            fee_vote,
            negative_unl_vote,
            amendment_status,
            overlay,
            parms: ConsensusParms::default(),
            tx_set_cache,
            pending_accept: StdMutex::new(None),
            acquiring_ledger: StdMutex::new(None),
        }
    }

    #[allow(dead_code)]
    fn now(&self) -> NetClockTimePoint {
        self.time_keeper.close_time()
    }

    pub fn is_validator(&self) -> bool {
        self.validator_keys.keys.is_some()
    }

    fn validated_view(
        &self,
        ledger: &RclCxLedger,
    ) -> crate::consensus::rcl_validation::RclValidatedLedger {
        crate::consensus::rcl_validation::RclValidatedLedger::from_ledger(&ledger.ledger())
    }

    fn sync_tree_to_rcl_tx_set(&self, sync_tree: &shamap::sync::SyncTree) -> consensus::RclTxSet {
        sync_tree_to_rcl_tx_set(sync_tree, &self.tx_set_cache)
    }

    pub fn tx_set_cache(&self) -> &consensus::rcl::RclTxSetSharedCache {
        &self.tx_set_cache
    }
}

fn sync_tree_to_rcl_tx_set(
    sync_tree: &shamap::sync::SyncTree,
    cache: &consensus::rcl::RclTxSetSharedCache,
) -> consensus::RclTxSet {
    consensus::RclTxSet::from_parts(sync_tree.root(), Arc::clone(cache), sync_tree.backed(), 0)
}

fn should_acquire_consensus_ledger(
    acquiring_ledger: &mut Option<Uint256>,
    ledger_id: Uint256,
) -> bool {
    if *acquiring_ledger == Some(ledger_id) {
        return false;
    }
    *acquiring_ledger = Some(ledger_id);
    true
}

/// Thin adapter bridging `Validations<RclValidationsAdaptor>` (our inner type)
/// to the `NegativeUNLVoteValidations` trait expected by
/// `NegativeUNLVote::do_voting`. Acquires no additional locks — the caller
/// must already hold the validations mutex.
struct NegativeUNLValidationsAdapter<'a>(
    &'a mut crate::consensus::rcl_validations::RclValidationsInner,
);

impl crate::amendments::negative_unl_vote::NegativeUNLVoteValidations
    for NegativeUNLValidationsAdapter<'_>
{
    fn set_seq_to_keep(&mut self, low: u32, high: u32) {
        self.0.set_seq_to_keep(low, high);
    }

    fn trusted_keys_for_ledger(&mut self, ledger_id: Uint256, seq: u32) -> Vec<PublicKey> {
        self.0
            .get_trusted_for_ledger(&ledger_id, seq)
            .into_iter()
            .map(|wrapped| *wrapped.get_signer_public())
            .collect()
    }
}

impl consensus::algorithm::ConsensusAdaptor for AppRclConsensusAdaptor {
    type Ledger = RclCxLedger;
    type NodeId = PublicKey;
    type TxSet = consensus::RclTxSet;
    type PeerPos = RclCxPeerPos;

    fn acquire_ledger(&self, ledger_id: &Uint256) -> Option<Self::Ledger> {
        let hash = basics::sha_map_hash::SHAMapHash::new(*ledger_id);
        if let Some(ledger) = self
            .ledger_master_runtime
            .ledger_master()
            .ledger_history()
            .get_cached_ledger_by_hash(hash)
        {
            return Some(RclCxLedger::new(ledger));
        }

        let shared = self
            .ledger_master_runtime
            .inbound_ledgers
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned());
        if let Some(shared) = shared {
            let should_acquire = should_acquire_consensus_ledger(
                &mut self
                    .acquiring_ledger
                    .lock()
                    .expect("acquiring_ledger mutex must not be poisoned"),
                *ledger_id,
            );
            if should_acquire {
                // A peer's current closed-ledger hash is not sequence-bound to
                // the separately advertised history range. Acquire by hash and
                // learn the authoritative sequence from the returned header.
                shared.acquire_closed_ledger_async(
                    *ledger_id,
                    crate::ledger::inbound_ledgers::AcquireReason::Consensus,
                );
            }
        }
        None
    }

    fn acquire_tx_set(&self, set_id: &Uint256) -> Option<Self::TxSet> {
        let sync_tree = {
            let mut guard = self
                .inbound_transactions
                .lock()
                .expect("inbound transactions mutex must not be poisoned");
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

    fn get_prev_ledger(
        &self,
        prev_ledger_id: &Uint256,
        prev_ledger: &Self::Ledger,
        mode: ConsensusMode,
    ) -> Uint256 {
        let min_valid_seq = self
            .ledger_master_runtime
            .ledger_master()
            .valid_ledger_seq();
        let wrapped = self.validated_view(prev_ledger);
        let preferred = RclConsensusValidationSource::preferred_min_seq(
            &self.validations,
            &wrapped,
            min_valid_seq,
        );
        if mode != ConsensusMode::WrongLedger && preferred != *prev_ledger_id {
            tracing::info!(
                target: "consensus",
                requested = %prev_ledger_id,
                preferred = %preferred,
                "Consensus view change — preferred ledger differs from current"
            );
        }
        preferred
    }

    fn on_mode_change(&self, before: ConsensusMode, after: ConsensusMode) {
        self.journal
            .info(&format!("Consensus mode change {before:?} -> {after:?}"));
    }

    fn on_close(
        &self,
        prev_ledger: &Self::Ledger,
        now: NetClockTimePoint,
        mode: ConsensusMode,
    ) -> consensus::algorithm::consensus::ConsensusResultOf<Self> {
        // Match RCLConsensus::Adaptor::onClose ordering: advertise that we
        // are closing, re-inject held transactions, mark the ledger being
        // built, then take one immutable transaction snapshot under the close
        // gate. This prevents the submission path from interleaving with the
        // snapshot.
        let txs = {
            let _close_guard = self
                .app_root
                .close_gate()
                .lock()
                .expect("close_gate mutex must not be poisoned");
            self.app_root.broadcast_consensus_status_change(
                prev_ledger.ledger().as_ref(),
                1, // neCLOSING_LEDGER
                mode != ConsensusMode::WrongLedger,
            );
            let next_seq = prev_ledger.seq().saturating_add(1);
            self.ledger_master_runtime.set_building_ledger(next_seq);
            let mut run_sync_batch = false;
            let _ = self
                .app_root
                .apply_held_transactions_to_network_ops(SHAMapHash::new(prev_ledger.id()), |_| {
                    run_sync_batch = true
                });
            // rippled's LedgerMaster::applyHeldTransactions calls
            // processTransactionSet only for a non-empty held set. Its sync
            // batch may also consume previously queued work; do the same, but
            // never unconditionally flush normal async peer ingress here.
            if run_sync_batch {
                let _ = self.app_root.apply_network_ops_pending_to_open_ledger();
            }
            RclConsensusOpenLedgerSource::current_open_transactions(&self.open_ledger)
        };
        // close_gate released — batch-apply can resume while we build the set.
        let mut set =
            consensus::RclTxSet::new(Arc::clone(&self.tx_set_cache), prev_ledger.seq() + 1);
        {
            let mut editable = set.mutable_view();
            for tx in &txs {
                editable.insert(&consensus::RclCxTxRef::from_transaction(tx));
            }

            // Rippled injects amendment and fee pseudo-transactions after a
            // flag LCL. Negative-UNL voting runs after the preceding voting
            // LCL, for the consensus session that produces the flag ledger.
            if pseudo_transaction_voting_enabled(self.options, mode)
                && prev_ledger.ledger().is_flag_ledger()
            {
                // Amendment voting — creates ttAMENDMENT pseudo-txs for
                // amendments gaining/losing majority or activating.
                if let Some(ref amendment_status) = self.amendment_status {
                    let prev_id = prev_ledger.ledger().header().parent_hash;
                    let parent_validations =
                        self.validations.store().trusted_for_ledger_by_sequence(
                            *prev_id.as_uint256(),
                            prev_ledger.seq() - 1,
                        );
                    let parent_validations = self
                        .validators
                        .negative_unl_filter_validations(parent_validations);
                    let mut vote_set: Vec<protocol::STTx> = Vec::new();
                    if trusted_validation_quorum_reached(
                        parent_validations.len(),
                        self.validators.quorum(),
                    ) {
                        amendment_status.do_voting_for_ledger(
                            &prev_ledger.ledger(),
                            &parent_validations,
                            &mut vote_set,
                        );
                    }
                    for pseudo_tx in &vote_set {
                        editable.insert(&consensus::RclCxTxRef::from_transaction(pseudo_tx));
                    }
                    if !vote_set.is_empty() {
                        tracing::info!(
                            target: "consensus",
                            count = vote_set.len(),
                            "on_close: injected amendment pseudo-transactions"
                        );
                    }
                }

                // FeeVote — injects a ttFEE pseudo-tx when the network
                // should move to new fee parameters.
                if let Some(ref fee_vote) = self.fee_vote {
                    let prev_id = prev_ledger.ledger().header().parent_hash;
                    let parent_validations =
                        self.validations.store().trusted_for_ledger_by_sequence(
                            *prev_id.as_uint256(),
                            prev_ledger.seq() - 1,
                        );
                    let parent_validations = self
                        .validators
                        .negative_unl_filter_validations(parent_validations);
                    let mut fee_vote_set: Vec<protocol::STTx> = Vec::new();
                    let ledger_ref = prev_ledger.ledger();
                    if trusted_validation_quorum_reached(
                        parent_validations.len(),
                        self.validators.quorum(),
                    ) {
                        fee_vote.do_voting(&*ledger_ref, &parent_validations, &mut fee_vote_set);
                    }
                    for pseudo_tx in &fee_vote_set {
                        editable.insert(&consensus::RclCxTxRef::from_transaction(pseudo_tx));
                    }
                    if !fee_vote_set.is_empty() {
                        tracing::info!(
                            target: "consensus",
                            count = fee_vote_set.len(),
                            "on_close: injected fee pseudo-transactions"
                        );
                    }
                }
            } else if pseudo_transaction_voting_enabled(self.options, mode)
                && prev_ledger.ledger().is_voting_ledger()
            {
                // NegativeUNLVote — injects ttUNL_MODIFY pseudo-txs for
                // validator disable/re-enable when reliability thresholds
                // are crossed.
                if let Some(ref neg_unl_vote) = self.negative_unl_vote {
                    let unl_keys = self.validators.get_trusted_master_keys();
                    let mut nunl_vote_set: Vec<protocol::STTx> = Vec::new();
                    // Acquire the validations lock to get mutable access for
                    // NegativeUNLVoteValidations::set_seq_to_keep and
                    // trusted_keys_for_ledger, then release it immediately.
                    {
                        let mut validations_guard = self
                            .validations
                            .validations()
                            .lock()
                            .expect("shared app validations mutex must not be poisoned");
                        let mut adapter = NegativeUNLValidationsAdapter(&mut *validations_guard);
                        let ledger_ref = prev_ledger.ledger();
                        neg_unl_vote.do_voting(
                            &*ledger_ref,
                            &unl_keys,
                            &mut adapter,
                            &mut nunl_vote_set,
                        );
                    }
                    for pseudo_tx in &nunl_vote_set {
                        editable.insert(&consensus::RclCxTxRef::from_transaction(pseudo_tx));
                    }
                    if !nunl_vote_set.is_empty() {
                        tracing::info!(
                            target: "consensus",
                            count = nunl_vote_set.len(),
                            "on_close: injected negative-UNL pseudo-transactions"
                        );
                    }
                }
            }

            set = editable.freeze();
        }

        let position_id = consensus::ConsensusTxSet::id(&set);
        let node_id = self
            .validator_keys
            .keys
            .as_ref()
            .map(|k| k.public_key)
            .unwrap_or_else(|| PublicKey::from_bytes([0u8; 33]));
        let position = Proposal::new(prev_ledger.id(), 0, position_id, now, now, node_id);

        {
            let sync_tree = set.to_sync_tree();
            let mut guard = self
                .inbound_transactions
                .lock()
                .expect("inbound_transactions mutex");
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
        let base_fee = self
            .ledger_master_runtime
            .ledger_master()
            .closed_ledger()
            .map(|l| l.fees().base)
            .unwrap_or(10);

        // Consensus transaction-set bytes originate from peers. rippled catches
        // STTx construction failures here, marks that entry failed, and keeps
        // building the remaining canonical set. Do the same before handing
        // acceptance work to the dedicated consensus strand.
        let items = result.txns.all_items();
        let (txns, malformed_txns) = decode_consensus_accept_transactions(
            result.txns.id(),
            items.iter().map(|item| (item.data().len(), item.data())),
        );
        let malformed_tx_count = malformed_txns.len();
        for (payload_len, message) in malformed_txns {
            tracing::warn!(
                target: "consensus",
                tx_set = %result.txns.id(),
                payload_len,
                %message,
                "discarded malformed consensus transaction"
            );
        }
        if malformed_tx_count > 0 {
            tracing::warn!(
                target: "consensus",
                tx_set = %result.txns.id(),
                malformed_tx_count,
                "consensus transaction set contains malformed entries"
            );
        }
        let raw_close_time = result.position.close_time();
        let close_time_correct = raw_close_time != NetClockTimePoint::default();
        let effective_close_time = if !close_time_correct {
            NetClockTimePoint::new(prev_ledger.close_time().as_seconds().saturating_add(1))
        } else {
            let resolution = time::Duration::seconds(close_resolution.as_secs() as i64);
            consensus::algorithm::timing::effective_close_time(
                raw_close_time,
                resolution,
                prev_ledger.close_time(),
            )
        };
        let close_time = effective_close_time.as_seconds();
        let close_resolution_secs = close_resolution.as_secs().min(u8::MAX as u64) as u8;
        let closed_seq = next_seq;

        let pending_validation = self.validator_keys.keys.as_ref().map(|keys| {
            crate::state::application_root::PendingValidation {
                public_key: keys.public_key,
                secret_key: keys.secret_key.clone(),
                node_id: protocol::calc_node_id(&keys.public_key),
                consensus_hash: result.txns.id(),
                proposing: mode == ConsensusMode::Proposing,
            }
        });

        // Clock adjustment: converge toward network's observed close time
        let consensus_fail = result.state == consensus::algorithm::types::ConsensusState::MovedOn;
        if (mode == ConsensusMode::Proposing || mode == ConsensusMode::Observing) && !consensus_fail
        {
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
            let new_offset = self
                .time_keeper
                .adjust_close_time(time::Duration::seconds(offset_seconds));
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
            let mut pending = self
                .pending_accept
                .lock()
                .expect("pending_accept mutex must not be poisoned");
            *pending = Some(PendingAcceptWork {
                closed_seq,
                close_time,
                close_resolution: close_resolution_secs,
                correct_close_time: close_time_correct,
                consensus_hash: result.txns.id(),
                have_correct_lcl: mode != ConsensusMode::WrongLedger,
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
                let peer_pos =
                    RclCxPeerPos::new(keys.public_key, signature, suppression, pos.clone());
                self.relay.relay_proposal(&peer_pos);
            }
            Err(err) => self
                .journal
                .error(&format!("propose: signing failed: {err:?}")),
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

    fn next_ledger_time_resolution(
        &self,
        previous_resolution: StdDuration,
        previous_agree: bool,
        ledger_seq: u32,
    ) -> StdDuration {
        let previous = time::Duration::seconds(previous_resolution.as_secs() as i64);
        let next = consensus::algorithm::timing::get_next_ledger_time_resolution(
            previous,
            previous_agree,
            ledger_seq,
        );
        StdDuration::from_secs(next.whole_seconds().max(0) as u64)
    }

    fn round_close_time(
        &self,
        raw: NetClockTimePoint,
        resolution: StdDuration,
    ) -> NetClockTimePoint {
        let resolution = time::Duration::seconds(resolution.as_secs() as i64);
        consensus::algorithm::timing::round_close_time(raw, resolution)
    }

    fn valid_ledger_seq(&self) -> u32 {
        self.ledger_master_runtime
            .ledger_master()
            .valid_ledger_seq()
    }

    fn quorum_keys(&self) -> (usize, HashSet<PublicKey>) {
        let (quorum, keys) = self.validators.get_quorum_keys();
        (quorum, keys.into_iter().collect())
    }

    fn laggards(&self, seq: u32, trusted_keys: &mut HashSet<PublicKey>) -> usize {
        // The generic consensus layer deliberately uses its hardened HashSet,
        // while the existing validation tracker predates that boundary and
        // owns a standard HashSet. Preserve rippled's mutating contract by
        // moving all keys into the tracker, then restoring only its remaining
        // (offline) keys to the generic set.
        let mut validation_keys: StdHashSet<_> = trusted_keys.drain().collect();
        let laggards = self
            .validations
            .validations()
            .lock()
            .expect("shared app validations mutex must not be poisoned")
            .laggards(seq, &mut validation_keys);
        trusted_keys.extend(validation_keys);
        laggards
    }

    fn validator(&self) -> bool {
        self.is_validator()
    }

    fn have_validated(&self) -> bool {
        self.ledger_master_runtime.ledger_master().have_validated()
    }

    fn update_operating_mode(&self, positions: usize) {
        update_operating_mode_after_accept(&self.network_ops_mode_owner, positions);
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
    fn start_round(
        &mut self,
        now: NetClockTimePoint,
        prev_ledger_id: Uint256,
        prev_ledger: RclCxLedger,
        proposing: bool,
    );

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
    /// Tracks the trusted validator keys from the previous round so we can
    /// compute `now_untrusted` (keys removed since last round) for start_round.
    /// Matches rippled's dynamic UNL update behavior.
    last_trusted_keys: HashSet<PublicKey>,
}

impl AppConsensus {
    pub fn new(adaptor: AppRclConsensusAdaptor, _parms: ConsensusParms) -> Self {
        Self {
            adaptor,
            state: Consensus::new(),
            last_trusted_keys: HashSet::default(),
        }
    }

    /// Compute the set of validator keys that were trusted in the previous
    /// round but are no longer trusted (i.e. removed from UNL or added to
    /// the negative UNL). Updates `last_trusted_keys` for the next call.
    fn publish_consensus_mode(&self) {
        use crate::network::network_ops::NetworkOpsConsensusMode;

        let mode = match self.state.mode() {
            ConsensusMode::Observing => NetworkOpsConsensusMode::Observing,
            ConsensusMode::Proposing => NetworkOpsConsensusMode::Proposing,
            ConsensusMode::WrongLedger => NetworkOpsConsensusMode::WrongLedger,
            ConsensusMode::SwitchedLedger => NetworkOpsConsensusMode::SwitchedLedger,
        };
        self.adaptor.network_ops_mode_owner.set_consensus_mode(mode);
    }

    fn compute_now_untrusted(&mut self) -> HashSet<PublicKey> {
        let current_trusted: HashSet<PublicKey> = self
            .adaptor
            .validators
            .get_trusted_master_keys()
            .into_iter()
            .collect();
        // Keys that were in last_trusted but not in current are now untrusted
        let now_untrusted: HashSet<PublicKey> = self
            .last_trusted_keys
            .difference(&current_trusted)
            .copied()
            .collect();
        // Also include keys on the negative UNL
        let negative_unl = self.adaptor.validators.get_negative_unl();
        let mut combined = now_untrusted;
        for key in negative_unl {
            combined.insert(key);
        }
        self.last_trusted_keys = current_trusted;
        combined
    }

    /// Execute the accept-ledger work and start the next consensus round,
    /// matching rippled's single-threaded flow:
    ///   doAccept (build ledger) → endConsensus (checkLastClosedLedger) →
    ///   beginConsensus (start_round)
    fn do_accept_and_start_next_round(&mut self, now: NetClockTimePoint, work: PendingAcceptWork) {
        let closed_seq = work.closed_seq;
        let root = self.adaptor.app_root.clone();

        match root.accept_ledger_with_txns_outcome(
            work.closed_seq,
            work.close_time,
            work.close_resolution,
            work.correct_close_time,
            work.base_fee_drops,
            work.txns,
        ) {
            Ok(outcome) => {
                // This is the live `doAccept` handoff. Run the exact
                // LedgerMaster::consensusBuilt lifecycle once, after the
                // ledger exists and before open-ledger or next-round work.
                // The helper clears building state, records built-history
                // metadata, checks this ledger, then scans alternate trusted
                // candidates.
                if let Some(closed) = root.closed_ledger() {
                    root.record_consensus_built_ledger(Arc::clone(&closed), work.consensus_hash);
                }
                if let Some(closed) = root.closed_ledger() {
                    RclConsensusOpenLedgerSource::accept_consensus_ledger(
                        &self.adaptor.open_ledger,
                        outcome.next_open_index,
                        work.base_fee_drops,
                        closed.header().hash.as_uint256(),
                        &outcome.completed_transaction_ids,
                        &outcome.retry_transactions,
                    );
                    root.set_status_rpc_current_ledger_index(Some(outcome.next_open_index));
                    root.set_status_rpc_queue_report(Some(root.tx_q_rpc_report()));
                }
                // Clear pending transactions from the network ops queue to
                // prevent stale txns from contaminating the next consensus
                // round. Matches rippled's endConsensus which clears the
                // submission queue after a successful accept.
                if let Some(network_ops_rt) = root.network_ops_runtime() {
                    network_ops_rt.clear_pending_transactions();
                }

                // Broadcast StatusChange (neACCEPTED_LEDGER) and sign/publish validation.
                if let Some(closed) = root.closed_ledger() {
                    let hdr = closed.header();
                    root.broadcast_consensus_status_change(
                        closed.as_ref(),
                        2, // neACCEPTED_LEDGER
                        work.have_correct_lcl,
                    );

                    if let Some(pending) = work.validation {
                        tracing::info!(target: "consensus", closed_seq, proposing = pending.proposing, "execute_accept: signing validation");
                        let ledger_hash = *hdr.hash.as_uint256();
                        match protocol::STValidation::new_signed(
                            work.close_time.max(1),
                            &pending.public_key,
                            pending.node_id,
                            &pending.secret_key,
                            |v| {
                                v.set_field_h256(
                                    protocol::get_field_by_symbol("sfLedgerHash"),
                                    ledger_hash,
                                );
                                v.set_field_h256(
                                    protocol::get_field_by_symbol("sfConsensusHash"),
                                    pending.consensus_hash,
                                );
                                v.set_field_u32(
                                    protocol::get_field_by_symbol("sfLedgerSequence"),
                                    closed_seq,
                                );
                                if pending.proposing {
                                    v.set_flag(protocol::VF_FULL_VALIDATION);
                                }

                                // sfValidatedHash — hash of the last fully
                                // validated ledger (may be the one we just
                                // accepted or an earlier one).
                                if let Some(validated) = root.validated_ledger() {
                                    v.set_field_h256(
                                        protocol::get_field_by_symbol("sfValidatedHash"),
                                        *validated.header().hash.as_uint256(),
                                    );
                                }

                                // sfCookie — random nonce for replay protection.
                                v.set_field_u64(
                                    protocol::get_field_by_symbol("sfCookie"),
                                    basics::random::rand_int_full::<u64>(),
                                );

                                // sfServerVersion — report on voting ledgers
                                // (the ledger just before a flag ledger).
                                if closed.is_voting_ledger() {
                                    v.set_field_u64(
                                        protocol::get_field_by_symbol("sfServerVersion"),
                                        protocol::get_encoded_version(),
                                    );
                                }

                                // sfLoadFee — report when load factor exceeds
                                // base (matching rippled's FeeTrack logic).
                                {
                                    let load_fee_track = root.load_fee_track();
                                    let fee = std::cmp::max(
                                        load_fee_track.local_fee(),
                                        load_fee_track.cluster_fee(),
                                    );
                                    if fee > load_fee_track.load_base() {
                                        v.set_field_u32(
                                            protocol::get_field_by_symbol("sfLoadFee"),
                                            fee,
                                        );
                                    }
                                }

                                // Fee vote and amendments — only on voting
                                // ledgers (the ledger just before a flag ledger).
                                if closed.is_voting_ledger() {
                                    if let Some(ref fee_vote) = self.adaptor.fee_vote {
                                        fee_vote.do_validation(closed.fees(), closed.rules(), v);
                                    }
                                    if let Some(ref amendment_status) =
                                        self.adaptor.amendment_status
                                    {
                                        amendment_status
                                            .do_validation_for_ledger(closed.as_ref(), v);
                                    }
                                }
                            },
                        ) {
                            Ok(built_validation) => {
                                self.adaptor
                                    .ledger_acceptor
                                    .publish_validation(Arc::new(built_validation));
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
                    let network_closed = {
                        // Build peer_counts from overlay peers (used as tie-breaker input).
                        let mut peer_counts = std::collections::BTreeMap::<Uint256, u32>::new();
                        if let Some(ort) = root.overlay_runtime() {
                            use overlay::Overlay;
                            let peers = ort.overlay().active_peers();
                            for peer in &peers {
                                let h = peer.closed_ledger_hash();
                                if !h.is_zero() {
                                    *peer_counts.entry(h).or_insert(0) += 1;
                                }
                            }
                        }

                        // H3: Include our own closed ledger in the tally so
                        // getPreferredLCL accounts for our view.  Matches
                        // rippled where checkLastClosedLedger counts the node's
                        // own closed ledger hash among peers.
                        if let Some(our_closed_lcl) = root.closed_ledger() {
                            let our_h = *our_closed_lcl.header().hash.as_uint256();
                            if !our_h.is_zero() {
                                *peer_counts.entry(our_h).or_insert(0) += 1;
                            }
                        }

                        // Use the validation trie to determine the preferred LCL.
                        // This consults trusted validations first, using peer_counts
                        // only as a tie-breaker — matching rippled's getPreferredLCL.
                        let wrapped_lcl =
                            crate::consensus::rcl_validation::RclValidatedLedger::from_ledger(
                                &closed,
                            );
                        let min_seq = self
                            .adaptor
                            .ledger_master_runtime
                            .ledger_master()
                            .valid_ledger_seq();
                        let preferred = RclConsensusValidationSource::preferred_lcl(
                            &self.adaptor.validations,
                            &wrapped_lcl,
                            min_seq,
                            &peer_counts,
                        );

                        if preferred != closed_id
                            && preferred != *closed.header().parent_hash.as_uint256()
                        {
                            preferred
                        } else {
                            closed_id
                        }
                    };

                    let round_ledger = if network_closed != closed_id {
                        tracing::info!(
                            target: "consensus",
                            %closed_id, %network_closed,
                            "synchronous accept: peers prefer different chain"
                        );
                        if let Some(lm_rt) = root.ledger_master_runtime() {
                            if lm_rt
                                .ledger_master()
                                .get_ledger_by_hash(basics::sha_map_hash::SHAMapHash::new(
                                    network_closed,
                                ))
                                .is_some()
                            {
                                // Cached and acquired preferred LCLs must take
                                // the same strand-owned switchLastClosedLedger
                                // path. Do not partially install a cached LCL
                                // here and skip open-ledger/TxQ/trust/status
                                // work that an acquired LCL receives.
                                root.set_need_network_ledger(true);
                                self.adaptor
                                    .network_ops_mode_owner
                                    .set_operating_mode_direct(
                                    crate::network::network_ops::NetworkOpsOperatingMode::Connected,
                                );
                                None
                            } else {
                                // Network ledger not locally available — acquire from peers.
                                // Set need_network_ledger so the strand's switchLastClosedLedger
                                // block handles the acquired ledger when it arrives, and
                                // strand step 5 won't start a new round on our wrong ledger.
                                // Demote to Connected matching rippled NetworkOPs.cpp:1997.
                                if let Ok(guard) = lm_rt.inbound_ledgers.lock() {
                                    if let Some(shared) = guard.as_ref() {
                                        // The peer's history range does not authoritatively
                                        // bind a sequence to its closed-ledger hash.
                                        shared.acquire_closed_ledger_async(
                                            network_closed,
                                            crate::ledger::inbound_ledgers::AcquireReason::Consensus,
                                        );
                                    }
                                }
                                root.set_need_network_ledger(true);
                                self.adaptor
                                    .network_ops_mode_owner
                                    .set_operating_mode_direct(
                                    crate::network::network_ops::NetworkOpsOperatingMode::Connected,
                                );
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

                        self.start_round(now, prev_id, prev_cx, proposing);
                        tracing::info!(
                            target: "consensus",
                            closed_seq,
                            next_prev_ledger = %prev_id,
                            "synchronous accept: started next round inline"
                        );
                    } else if root.need_network_ledger() {
                        // Network ledger not available locally AND we already
                        // set need_network_ledger — do NOT start a round on the
                        // wrong local ledger. This breaks the acrolytic loop:
                        //   accept wrong ledger → detect peers disagree →
                        //   acquire → still not found → start round on wrong
                        //   ledger again → accept → detect → acquire → …
                        //
                        // The strand's switchLastClosedLedger block (step 6)
                        // handles starting a round on the correct chain once
                        // the acquisition completes and the ledger enters
                        // ledger_history.
                        tracing::info!(
                            target: "consensus",
                            closed_seq,
                            "synchronous accept: network ledger pending — deferring round start to strand"
                        );
                    } else {
                        // Network ledger not available locally but
                        // need_network_ledger is false (shouldn't normally
                        // happen). Start round on our ledger as a fallback.
                        let prev_id = *closed.header().hash.as_uint256();
                        let prev_cx = crate::consensus_ledger_from_ledger(&closed);
                        self.start_round(now, prev_id, prev_cx, false);
                        tracing::warn!(
                            target: "consensus",
                            closed_seq,
                            next_prev_ledger = %prev_id,
                            "synchronous accept: started round on own LCL (fallback, need_network_ledger=false)"
                        );
                    }
                } else {
                    let _ = root.schedule_network_ops_transaction_batch();
                }
            }
            Err(err) => {
                tracing::error!(target: "consensus", closed_seq, %err, "synchronous accept: accept_ledger_with_txns failed");
                // Don't leave consensus stuck in Accepted phase — start a
                // round on whatever closed ledger we have. timer_entry is a
                // no-op in Accepted, so without this the node never recovers.
                if let Some(closed) = root.closed_ledger() {
                    let prev_id = *closed.header().hash.as_uint256();
                    let prev_cx = crate::consensus_ledger_from_ledger(&closed);
                    self.start_round(now, prev_id, prev_cx, false);
                    tracing::warn!(
                        target: "consensus",
                        closed_seq,
                        next_prev_ledger = %prev_id,
                        "synchronous accept: accept failed, started recovery round on LCL"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppRclConsensusOptions, decode_consensus_accept_transactions,
        pseudo_transaction_voting_enabled, trusted_validation_quorum_reached,
        update_operating_mode_after_accept,
    };
    use crate::network::network_ops::{
        AppNetworkOpsModeOwner, NetworkOpsOperatingMode, SharedNetworkOpsState,
    };
    use basics::base_uint::Uint256;
    use consensus::algorithm::types::ConsensusMode;
    use protocol::{AccountID, STAmount, STTx, TxType, get_field_by_symbol};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn consensus_pseudo_transaction_voting_requires_proposing_or_standalone_and_quorum() {
        assert!(pseudo_transaction_voting_enabled(
            AppRclConsensusOptions::default(),
            ConsensusMode::Proposing,
        ));
        assert!(!pseudo_transaction_voting_enabled(
            AppRclConsensusOptions::default(),
            ConsensusMode::Observing,
        ));
        assert!(pseudo_transaction_voting_enabled(
            AppRclConsensusOptions {
                standalone: true,
                ..Default::default()
            },
            ConsensusMode::WrongLedger,
        ));
        assert!(trusted_validation_quorum_reached(3, 3));
        assert!(!trusted_validation_quorum_reached(2, 3));
    }

    #[test]
    fn consensus_accept_with_no_positions_downgrades_full_operating_mode() {
        let state = Arc::new(SharedNetworkOpsState::new(NetworkOpsOperatingMode::Full));
        let owner = AppNetworkOpsModeOwner::new(Arc::clone(&state), Arc::new(|| Duration::ZERO));

        update_operating_mode_after_accept(&owner, 0);

        assert_eq!(state.operating_mode(), NetworkOpsOperatingMode::Connected);
    }

    #[test]
    fn consensus_decode_discards_malformed_entries_and_keeps_valid_transactions() {
        let source = AccountID::from_hex("1111111111111111111111111111111111111111")
            .expect("source account");
        let destination = AccountID::from_hex("2222222222222222222222222222222222222222")
            .expect("destination account");
        let valid = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), source);
            tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1_000_000, false),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        });
        let valid_bytes = valid.get_serializer().data().to_vec();
        let malformed_bytes = [0; protocol::TX_MIN_SIZE_BYTES - 1];

        let (transactions, malformed) = decode_consensus_accept_transactions(
            Uint256::zero(),
            [
                (valid_bytes.len(), valid_bytes.as_slice()),
                (malformed_bytes.len(), &malformed_bytes),
            ],
        );

        assert_eq!(transactions.len(), 1);
        assert_eq!(
            transactions[0].get_transaction_id(),
            valid.get_transaction_id()
        );
        assert_eq!(malformed.len(), 1);
        assert_eq!(malformed[0].0, malformed_bytes.len());
    }
}

impl ConsensusRunner for AppConsensus {
    fn peer_proposal(&mut self, now: NetClockTimePoint, peer_pos: &RclCxPeerPos) -> bool {
        // Signature was verified by `overlay_impl.rs::on_propose_ledger` before
        // this proposal was queued — matching rippled's `checkPropose` which
        // calls `peerPos.checkSign()` and drops invalid proposals before they
        // reach `processTrustedProposal` / `peer_proposal`.
        let our_prev = *self.state.prev_ledger_id();
        let their_prev = *peer_pos.proposal().prev_ledger();
        let accepted = self.state.peer_proposal(&self.adaptor, now, peer_pos);
        self.publish_consensus_mode();
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
        self.publish_consensus_mode();
        // If on_accept fired during timer_entry, pending_accept will be Some.
        self.adaptor
            .pending_accept
            .lock()
            .expect("pending_accept mutex must not be poisoned")
            .take()
    }

    fn start_round(
        &mut self,
        now: NetClockTimePoint,
        prev_ledger_id: Uint256,
        prev_ledger: RclCxLedger,
        proposing: bool,
    ) {
        // Every AppConsensus start path, including initial, external, and
        // accepted-phase recovery starts, must run the beginConsensus trust
        // prelude against the LCL actually selected for this round. Rippled
        // refuses beginConsensus when that LCL cannot be loaded, rather than
        // starting a round with stale NegativeUNL/trusted state.
        let lcl = self
            .adaptor
            .ledger_master_runtime
            .ledger_master()
            .get_ledger_by_hash(basics::sha_map_hash::SHAMapHash::new(prev_ledger_id))
            .or_else(|| {
                self.adaptor
                    .app_root
                    .closed_ledger()
                    .filter(|ledger| *ledger.header().hash.as_uint256() == prev_ledger_id)
            });
        let Some(lcl) = lcl else {
            self.adaptor
                .network_ops_mode_owner
                .set_operating_mode_direct(
                    crate::network::network_ops::NetworkOpsOperatingMode::Connected,
                );
            tracing::warn!(target: "consensus", %prev_ledger_id,
                "refusing consensus round because selected LCL is unavailable");
            return;
        };
        self.adaptor
            .app_root
            .refresh_validator_trust_for_consensus(lcl.as_ref());

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
        let now_untrusted = self.compute_now_untrusted();
        self.state.start_round(
            &self.adaptor,
            now,
            prev_ledger_id,
            prev_ledger,
            &now_untrusted,
            actual_proposing,
        );
        self.publish_consensus_mode();
    }

    fn got_tx_set(&mut self, now: NetClockTimePoint, tx_set: consensus::RclTxSet) {
        self.state.got_tx_set(&self.adaptor, now, &tx_set);
        self.publish_consensus_mode();
    }

    fn execute_accept(&mut self, now: NetClockTimePoint, work: PendingAcceptWork) {
        self.do_accept_and_start_next_round(now, work);
        self.publish_consensus_mode();
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
    use super::{should_acquire_consensus_ledger, sync_tree_to_rcl_tx_set};
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
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(u64::from(fill), false),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        })
    }

    fn completed_sync_tree(txs: &[STTx], cache: consensus::rcl::RclTxSetSharedCache) -> SyncTree {
        let cache: Arc<
            shamap::tree_node_cache::TreeNodeCache<MonotonicClock, HardenedHashBuilder>,
        > = cache;
        let mut map = StorageTree::new(1, false, 1, cache);
        for tx in txs {
            let item = SHAMapItem::new(tx.get_transaction_id(), serialize_blob(tx));
            map.add_item(SHAMapNodeType::TransactionNm, item)
                .expect("insert into a fresh test tree should not need fetches");
        }
        map.unshare();

        let tree = SyncTree::from_root_with_type(
            map.root(),
            SHAMapType::Transaction,
            false,
            1,
            SyncState::Modifying,
        );
        tree.set_full();
        tree
    }

    #[test]
    fn consensus_ledger_acquisition_is_coalesced_by_hash() {
        let first = basics::base_uint::Uint256::from_u64(1);
        let second = basics::base_uint::Uint256::from_u64(2);
        let mut acquiring = None;

        assert!(should_acquire_consensus_ledger(&mut acquiring, first));
        assert!(!should_acquire_consensus_ledger(&mut acquiring, first));
        assert!(should_acquire_consensus_ledger(&mut acquiring, second));
        assert!(!should_acquire_consensus_ledger(&mut acquiring, second));
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
