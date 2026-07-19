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
//! - [`ConsensusRunner`] / [`AppConsensus`]: the dyn-compatible async
//!   wrapper around `Consensus<AppRclConsensusAdaptor>` that
//!   `AppConsensusRuntime` (in `runtime::component_runtime`) drives.
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
//! These are related but distinct views over the same underlying
//! `ledger::Ledger`; `AppRclConsensusAdaptor` converts between them at the
//! two seams where both are needed (`get_prev_ledger`, `on_close`).

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
/// transactions from, and resets once a round is accepted. Implemented for
/// `SharedAppOpenLedger` in `app_registry.rs`.
pub trait RclConsensusOpenLedgerSource {
    fn current_open_transactions(&self) -> Vec<Arc<protocol::STTx>>;
    fn has_open_transactions(&self) -> bool;
    /// Resets the open ledger for the next round, matching the reference's
    /// `app_.getOpenLedger().accept(...)`. `accepted_ids` is the set of
    /// transaction IDs that were just built into the accepted ledger --
    /// anything present in the OLD open ledger's transactions that is NOT
    /// in this set is carried forward into the new (reset) open ledger,
    /// matching the reference's carry-forward of `localTxs_`/leftover
    /// retriable transactions rather than a full destructive reset. This
    /// covers transactions submitted between `on_close`'s capture of
    /// `result.txns` for this round and this reset running (a real, if
    /// narrow, window during every consensus round) -- without carrying
    /// them forward, such a submission would be silently and permanently
    /// lost rather than picked up by the next round, unlike the reference.
    fn accept_consensus_ledger(
        &self,
        next_seq: u32,
        base_fee: u64,
        parent_hash: &Uint256,
        accepted_ids: &std::collections::HashSet<Uint256>,
    );
}

/// The validation-tracking surface the adaptor needs to answer
/// `proposers_validated`/`proposers_finished`/`get_prev_ledger`. Kept as a
/// narrow trait (rather than depending on `SharedAppValidations` directly
/// in the `ConsensusAdaptor` impl) so the adaptor's dependency on
/// validation state stays swappable/testable.
pub trait RclConsensusValidationSource {
    fn num_trusted_for_ledger(&self, ledger_id: Uint256) -> usize;
    fn get_nodes_after(&self, ledger: &crate::consensus::rcl_validation::RclValidatedLedger, ledger_id: &Uint256) -> usize;
    fn preferred_lcl(&self, lcl: &crate::consensus::rcl_validation::RclValidatedLedger, min_seq: u32, peer_counts: &std::collections::BTreeMap<Uint256, u32>) -> Uint256;
    /// The trust-trie-preferred working ledger, derived purely from trusted
    /// validations received so far (no peer input). Matches the reference's
    /// `Validations::getPreferred(Ledger const&, Seq)` overload, which is
    /// what `RCLConsensus::Adaptor::getPrevLedger` actually calls -- NOT
    /// `getPreferredLCL`'s peer-counts-aware overload (that one is reserved
    /// for `NetworkOPsImp::checkLastClosedLedger`/`endConsensus`). This is
    /// the real catch-up mechanism: if this node's validations trie shows a
    /// different (further-ahead) branch than what consensus is currently
    /// building on, `getPrevLedger` detects the mismatch and triggers
    /// `handleWrongLedger`, forcing this node to acquire and switch to the
    /// network's actual preferred ledger instead of blindly continuing to
    /// build its own.
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

/// Timing and mode overrides for [`AppRclConsensusAdaptor`]. Matches the
/// reference's constructor-time configuration (standalone mode disables
/// the multi-proposer wait, matching rippled's `-standalone` flag).
#[derive(Debug, Clone, Copy, Default)]
pub struct AppRclConsensusOptions {
    pub standalone: bool,
    /// Override for the minimum number of ledger-close-agreeing peers
    /// required before adjusting the close-time resolution ladder. `None`
    /// (the default) uses `ConsensusParms`'s reference-matching default.
    /// Reserved for whenever this adaptor needs to tune close-time
    /// convergence behavior independently of `ConsensusParms` (e.g. for
    /// deterministic tests); not yet read anywhere.
    #[allow(dead_code)]
    pub close_time_resolution_override: Option<std::time::Duration>,
}

/// A no-op diagnostics sink for [`AppRclConsensusAdaptor`], used where the
/// caller has no structured logging configured.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullRclConsensusJournal;

/// Diagnostics sink for consensus-round events. Matches the reference's
/// `beast::Journal` usage throughout `RCLConsensus::Adaptor`.
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

/// Peer relay of consensus artifacts: proposals, transaction sets, and
/// disputed transactions. Matches the reference's inline
/// `app_.overlay().relay(...)` calls scattered through
/// `RCLConsensus::Adaptor`.
pub trait RclConsensusRelay: Send + Sync {
    fn relay_proposal(&self, peer_pos: &RclCxPeerPos);
    fn relay_tx_set(&self, set: &consensus::RclTxSet);
    fn relay_disputed_tx(&self, tx: &consensus::RclCxTxRef);
}

/// The concrete peer-relay implementation, broadcasting consensus
/// artifacts over the node's overlay. Constructed from an
/// [`ApplicationRoot`] plus this node's validator identity (needed to
/// sign outgoing proposals).
pub struct AppRclConsensusRelay {
    overlay: Option<Arc<overlay::runtime::overlay_impl::OverlayImpl>>,
    inbound_transactions: AppInboundTransactions,
    validator_keys: ValidatorKeys,
    journal: Arc<dyn RclConsensusJournal>,
}

impl AppRclConsensusRelay {
    /// Construct a relay bound to `root`'s overlay (if attached) and the
    /// given validator identity/journal. Matches the reference's
    /// `RCLConsensus::Adaptor` constructor capturing `app_` for later
    /// `app_.overlay()` access.
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

        // Matches the reference's `app_.overlay().relay(peerPos, suppression, validatorKeys)`:
        // squelch-aware relay keyed by this proposal's suppression id, so
        // peers that already saw the same proposal (via a different path)
        // don't get it re-forwarded. Both our own freshly-signed proposals
        // (from `AppRclConsensusAdaptor::propose`) and peer proposals we're
        // forwarding (from `share_peer_position`) go through this same
        // suppression-aware path, matching the reference's unified
        // `relay()` call for both cases.
        let _ = overlay.relay_proposal(message, peer_pos.suppression_id(), *peer_pos.public_key());
    }

    fn relay_tx_set(&self, set: &consensus::RclTxSet) {
        // Store in InboundTransactions so peers can pull-acquire this set
        // via TMGetLedger(itype=3). Matches rippled where every tx-set
        // position change (from update_our_positions/dispute resolution)
        // makes the set available for serving. Without this, peers request
        // our updated set hash but get "set not found" because only the
        // INITIAL on_close set was stored — updated positions from dispute
        // resolution were broadcast but never stored for serving.
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

        // Matches the reference's `app_.overlay().relay(TMHaveTransactionSet)`:
        // announce that we now have this tx-set (peers that want its
        // contents will pull it via `TMGetLedger`/`TMLedgerData`, matching
        // the pull-based tx-set acquisition `acquire_tx_set` implements on
        // the receiving side), rather than eagerly pushing the full set.
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

/// Bridges Phase 3's generic `Consensus<Adaptor>` state machine to real
/// `Ledger`/`SHAMap`/`ValidatorList` types. Matches the reference's
/// `RCLConsensus::Adaptor`.
/// Work captured by `on_accept` that must execute synchronously while the
/// consensus state mutex is still held, matching rippled's single-threaded
/// `doAccept → endConsensus → beginConsensus` call chain. Stored in the
/// adaptor's `pending_accept` field and drained by `AppConsensus::timer_tick`
/// immediately after `timer_entry` returns (before releasing the mutex).
pub(crate) struct PendingAcceptWork {
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
    /// ApplicationRoot reference for calling apply_network_ops_pending_to_open_ledger
    /// in on_close (matching rippled's applyHeldTransactions before capturing the open ledger).
    app_root: crate::state::application_root::ApplicationRoot,
    // The following fields are accepted from the constructor call site
    // (`application_root.rs`'s `attach_default_consensus_runtime`, which
    // passes them in exactly this order) and retained for the follow-up
    // work documented in this module's honest-limitations notes:
    // trusted-proposer-set lookups for negative-UNL voting eligibility
    // (`validators`), operating-mode-aware proposing gates
    // (`network_ops_mode_owner`), the negative-UNL vote itself
    // (`negative_unl_vote`), amendment-majority bookkeeping
    // (`amendment_status`), and direct overlay access for future
    // wire-format relay (`overlay`, currently only reached indirectly via
    // `AppRclConsensusRelay`). None of these are read yet because
    // `on_close`/`on_accept` do not yet perform negative-UNL voting or
    // amendment-vote injection, and `propose`/`share_*` relay through
    // `AppRclConsensusRelay` rather than this field directly. Kept as
    // named struct fields (not dropped) so the constructor's shape stays
    // stable for `application_root.rs` while that follow-up work lands.
    #[allow(dead_code)]
    validators: Arc<ValidatorList>,
    #[allow(dead_code)]
    network_ops_mode_owner: AppNetworkOpsModeOwner,
    /// Tracks the number of peer proposers seen in the LAST completed round.
    /// Used by start_round to determine if we should propose: if zero peers
    /// proposed last round (we can't see the network), we must not propose
    /// this round either (matching rippled's updateOperatingMode demotion).
    /// Atomic to avoid racing between on_accept (writer) and start_round (reader).
    ledger_acceptor: Arc<dyn LedgerAcceptor>,
    inbound_transactions: AppInboundTransactions,
    transaction_master: Arc<TransactionMaster>,
    relay: AppRclConsensusRelay,
    journal: Arc<dyn RclConsensusJournal>,
    validator_keys: ValidatorKeys,
    #[allow(dead_code)]
    negative_unl_vote: Option<Arc<crate::amendments::negative_unl_vote::NegativeUNLVote>>,
    #[allow(dead_code)]
    amendment_status: Option<Arc<crate::amendments::amendment_status::AmendmentStatus>>,
    #[allow(dead_code)]
    overlay: Option<Arc<overlay::runtime::overlay_impl::OverlayImpl>>,
    parms: ConsensusParms,
    /// A single, long-lived tree-node cache shared by every `RclTxSet`
    /// this adaptor builds or adopts (`on_close`'s initial position,
    /// `acquire_tx_set`'s adopted peer sets, and `AppConsensus::got_tx_set`'s
    /// reconstructed sets). Matches the reference's single shared
    /// `TreeNodeCache` for the transaction-tree family: reusing one cache
    /// (rather than a fresh, empty one per call) means a tx-set adopted
    /// from an acquired `SyncTree` and a tx-set rebuilt independently from
    /// the same transactions' blobs can share cached nodes, and matters
    /// for correctness (not just performance) when comparing two `RclTxSet`s
    /// built through different paths, since `RclTxSet::compare` fetches
    /// through this cache on cache misses.
    tx_set_cache: consensus::rcl::RclTxSetSharedCache,
    /// Accept-work captured by `on_accept` for synchronous execution in
    /// `timer_tick` while the consensus mutex is still held. This is the
    /// key mechanism that eliminates the thread-scheduling gap between
    /// `on_accept` and `start_next_round`, matching rippled's single-threaded
    /// `doAccept → endConsensus → beginConsensus` flow.
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

    /// Whether this node is configured to propose (has validator keys
    /// loaded). Matches the reference's `validating_` flag, derived from
    /// whether `ValidatorKeys::from_sources` produced usable keys.
    pub fn is_validator(&self) -> bool {
        self.validator_keys.keys.is_some()
    }

    /// Convert a `RclCxLedger` (the `ConsensusLedger`-bound type) into the
    /// ancestor-trie-carrying `RclValidatedLedger` Phase 5's validations
    /// tracker needs. See the module doc comment for why these are two
    /// distinct types.
    fn validated_view(&self, ledger: &RclCxLedger) -> crate::consensus::rcl_validation::RclValidatedLedger {
        crate::consensus::rcl_validation::RclValidatedLedger::from_ledger(&ledger.ledger())
    }

    /// Adopt a completed `shamap::sync::SyncTree` (as delivered by
    /// `InboundTransactions::get_set` once a peer tx-set finishes
    /// acquiring) as an `RclTxSet`, sharing this adaptor's persistent
    /// tree-node cache. Matches the reference's implicit construction of
    /// an `RCLTxSet` directly from the acquired `SHAMap`.
    ///
    /// `SyncTree` does not expose a public `ledger_seq()` getter (only a
    /// setter), so this uses `0` for the resulting `RclTxSet`'s ledger-seq
    /// bookkeeping field. That field only affects the SHAMap storage
    /// tree's internal write-versioning; since the adopted tree is only
    /// ever read/compared afterward (a peer's tx-set position is never
    /// mutated once acquired), its value has no bearing on correctness
    /// here -- matching the same reasoning already applied to
    /// `AppConsensus::got_tx_set`'s own reconstruction.
    fn sync_tree_to_rcl_tx_set(&self, sync_tree: &shamap::sync::SyncTree) -> consensus::RclTxSet {
        sync_tree_to_rcl_tx_set(sync_tree, &self.tx_set_cache)
    }
}

/// Adopt a completed `shamap::sync::SyncTree` as an `RclTxSet` sharing the
/// given tree-node cache. Extracted as a free function (rather than an
/// `AppRclConsensusAdaptor` method only) so it can be unit-tested directly
/// against a real `SyncTree` without needing to construct a full adaptor.
/// See [`AppRclConsensusAdaptor::sync_tree_to_rcl_tx_set`] for the full
/// rationale on the `ledger_seq` placeholder.
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

        // Matches the reference's `RCLConsensus::Adaptor::acquireLedger`:
        // when the cache lookup misses, ACTIVELY dispatch a fetch for the
        // consensus ledger (`app.getInboundLedgers().acquireAsync(id, 0,
        // Reason::CONSENSUS)`), rather than passively waiting for it to
        // arrive via some unrelated path. `SharedInboundLedgers::acquire`
        // dedupes internally (a repeated call for the same hash while one
        // is already in flight just touches its last-seen time), so this
        // is safe to call unconditionally on every tick that still can't
        // find the ledger cached -- matching the reference's own
        // `acquiringLedger_ != hash` guard, which exists purely to avoid
        // re-logging/re-dispatching every single tick, not for
        // correctness (a duplicate `acquireAsync` for the same hash is
        // itself a no-op in the reference's `InboundLedgers::acquire`).
        if let Some(guard) = self.ledger_master_runtime.shared_inbound_ledgers.lock().ok()
            && let Some(shared) = guard.as_ref()
        {
            shared.acquire(*ledger_id, 0);
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
        // Matches rippled's `proposersFinished` calling
        // `vals.getNodesAfter(ledger, prevLedgerID)` — counts validators
        // that have validated a ledger AFTER (higher sequence than) the
        // one we're currently building on. This is NOT the same as
        // counting validations FOR prev_ledger_id (which would always
        // return the quorum count for an already-validated parent, causing
        // checkConsensus's MovedOn fallback to fire immediately on every
        // round and force-close before dispute resolution can complete).
        let wrapped = self.validated_view(prev_ledger);
        RclConsensusValidationSource::get_nodes_after(&self.validations, &wrapped, prev_ledger_id)
    }

    fn get_prev_ledger(&self, _prev_ledger_id: &Uint256, prev_ledger: &Self::Ledger, _mode: ConsensusMode) -> Uint256 {
        // Matches rippled's `RCLConsensus::Adaptor::getPrevLedger`:
        // uses the validation trie's preferred branch.
        let min_valid_seq = self.ledger_master_runtime.ledger_master().valid_ledger_seq();
        let wrapped = self.validated_view(prev_ledger);
        RclConsensusValidationSource::preferred_min_seq(&self.validations, &wrapped, min_valid_seq)
    }

    fn on_mode_change(&self, before: ConsensusMode, after: ConsensusMode) {
        self.journal.info(&format!("Consensus mode change {before:?} -> {after:?}"));
    }

    fn on_close(&self, prev_ledger: &Self::Ledger, now: NetClockTimePoint, _mode: ConsensusMode) -> consensus::algorithm::consensus::ConsensusResultOf<Self> {
        // Apply any remaining pending transactions. The dedicated batch-apply
        // thread has been continuously applying relay for the past ~995ms.
        // This final apply catches any stragglers from the last few ms.
        // Matches rippled's applyHeldTransactions() in onClose.
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

        // Store our own tx-set in InboundTransactions so peers that
        // request it via TMGetLedger(liTS_CANDIDATE) can be served.
        // Matches rippled's InboundTransactions always having the local
        // node's own set available for peer pull-requests.
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

        // Matches the reference's `doAccept` building `retriableTxs` from
        // `result.txns` (the transaction set `onClose` captured earlier
        // from the open ledger, now consensus-agreed) -- NOT by re-reading
        // the open ledger's current contents, which is about to be reset
        // for the next round below. Deserialize failures are skipped
        // (matching the reference's own try/catch around
        // `std::make_shared<STTx const>(SerialIter{item.slice()})`, which
        // tracks failed items separately but does not abort the round).
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

        // Reset the open ledger for the next round, carrying forward
        // anything submitted between `on_close`'s capture (above) and this
        // point that did NOT make it into the just-accepted set -- see
        // `RclConsensusOpenLedgerSource::accept_consensus_ledger`'s doc
        // comment for why this carry-forward matters.
        RclConsensusOpenLedgerSource::accept_consensus_ledger(&self.open_ledger, next_seq, base_fee, &prev_ledger.id(), &accepted_ids);

        // Matches the reference's `doAccept`:
        //
        //   if (consensusCloseTime == NetClock::time_point{}) {
        //       // We agreed to disagree on the close time
        //       consensusCloseTime = prevLedger.closeTime() + 1s;
        //       closeTimeCorrect = false;
        //   } else {
        //       consensusCloseTime =
        //           effCloseTime(consensusCloseTime, closeResolution, prevLedger.closeTime());
        //       closeTimeCorrect = true;
        //   }
        //
        // `result.position.close_time()` is each node's OWN final position
        // out of `updateOurPositions` -- close-time agreement
        // (`haveCloseTimeConsensus_`) is tracked completely separately
        // from transaction-set agreement (`checkConsensus`'s
        // proposers/agreement-percentage threshold, verified faithful
        // elsewhere), so a round can legitimately reach
        // `ConsensusState::Yes` (and thus `on_accept`) while every node's
        // own `result.position.close_time()` differs, or while it never
        // crossed ANY vote threshold at all -- in which case
        // `update_our_positions` leaves it at `NetClockTimePoint::default()`
        // (the zero/epoch sentinel; see
        // `consensus::algorithm::consensus::Consensus::update_our_positions`).
        // Using that raw, per-node value VERBATIM as the new ledger's
        // `close_time` (as this function used to do) bakes a
        // node-divergent value directly into the built ledger's header --
        // a hash-affecting field -- producing a different ledger hash on
        // every node despite identical parent, identical (possibly empty)
        // transaction set, and correct algorithm-layer consensus. Once
        // that happens once, every subsequent round is built on an
        // already-diverged parent and the fork is permanent: this is the
        // root cause of the 5-way genesis-cluster consensus fork this
        // function's fix addresses (see
        // `CONSENSUS_FORK_INVESTIGATION.md`). The zero-sentinel branch
        // derives a value purely from `prev_ledger.close_time()`
        // (identical on every node by construction), and the non-zero
        // branch rounds to `close_resolution` and floors at
        // `prev_ledger.close_time() + 1s` via `effective_close_time` --
        // both `NetClockTimePoint`s here, so no `.as_seconds()` overflow
        // concerns. `close_resolution` (renamed from the previously
        // unused, underscore-prefixed `_close_resolution`) is exactly
        // `Consensus::phaseEstablish`'s own `closeResolution_`, passed
        // down by the algorithm layer to this call unchanged.
        let raw_close_time = result.position.close_time();
        let close_time_correct = raw_close_time != NetClockTimePoint::default();
        let effective_close_time = if !close_time_correct {
            NetClockTimePoint::new(prev_ledger.close_time().as_seconds().saturating_add(1))
        } else {
            let resolution = time::Duration::seconds(close_resolution.as_secs() as i64);
            consensus::algorithm::timing::effective_close_time(raw_close_time, resolution, prev_ledger.close_time())
        };
        let close_time = effective_close_time.as_seconds();
        // Stored verbatim into the built ledger's header (`LedgerInfo::
        // closeTimeResolution`), matching the reference's `buildLedger`
        // stashing `closeResolution` (this round's `Consensus::
        // closeResolution_`, i.e. this exact `close_resolution` parameter)
        // onto the new ledger via `Ledger::setAccepted`. Consumed by the
        // NEXT round's `next_ledger_time_resolution` call (see
        // `Consensus::start_round_internal`) as `previous_resolution` --
        // if this were hardcoded instead of the real value (as it
        // previously was, always writing `0`, a value absent from
        // `LEDGER_POSSIBLE_TIME_RESOLUTIONS`), every future round's
        // resolution lookup permanently fails to find a match and freezes
        // at `0` forever, which in turn makes `effective_close_time`'s own
        // rounding step above a complete no-op (rounding to a 0-second
        // bucket rounds to itself) -- silently defeating the very
        // determinism this function's zero-sentinel handling exists to
        // provide. `close_resolution.as_secs()` fits in a `u8` for every
        // entry in `LEDGER_POSSIBLE_TIME_RESOLUTIONS` (max 120s).
        let close_resolution_secs = close_resolution.as_secs().min(u8::MAX as u64) as u8;
        // `ApplicationRoot::accept_ledger`'s `closed_seq` parameter names the
        // sequence of the ledger being built (checked against
        // `parent.seq() + 1` internally), not the previous/parent ledger's
        // own sequence -- so this must be `next_seq` directly, not
        // `next_seq - 1` (which would just be `prev_ledger.seq()` again and
        // fail that internal guard, silently erroring out of every accept
        // past the first ledger).
        let closed_seq = next_seq;

        // Matches the reference's `RCLConsensus::Adaptor::doAccept` calling
        // `validate(built, result.txns, proposing)` when this node is a
        // configured validator. This is not merely a diagnostic artifact:
        // without publishing a validation for every accepted ledger, no
        // node's trust trie (`Validations`) ever accumulates real
        // cross-node data, so `getPrevLedger`'s `Validations::getPreferred`
        // lookup can never detect that a node has fallen behind the
        // network's actual validated chain -- the exact catch-up mechanism
        // `checkLedger`/`handleWrongLedger` depends on.
        //
        // Signing happens INSIDE the spawned job, after `accept_ledger`
        // succeeds and the real built ledger's hash is known -- NOT here.
        // `STValidation::new_signed` computes and embeds the signature
        // over the validation's fields as they exist at signing time;
        // mutating any field afterward (e.g. setting `sfLedgerHash` post
        // hoc once the async build finishes) invalidates that signature,
        // which is exactly the bug an earlier version of this code had
        // (every peer's `from_serial_iter` signature check failed with
        // `InvalidSignature` because `sfLedgerHash` was being set after
        // `new_signed` had already signed the object without it).
        //
        // Even after fixing that, a SECOND, subtler bug remained: signing
        // right after `ledger_acceptor.accept_ledger(...)` returns (as this
        // code used to do) races `ConsensusLedgerAcceptor::accept_ledger`'s
        // async, fire-and-forget wrapper, which enqueues the real ledger
        // build and returns `Ok` immediately WITHOUT waiting for it. Reading
        // `closed_ledger()` immediately after that `Ok` can observe the
        // PREVIOUS ledger, producing a validation whose `sfLedgerHash`
        // doesn't match its claimed `sfLedgerSequence` -- which corrupted
        // the trust trie with an internally inconsistent `(seq, id)` pair,
        // making `Validations::getPreferred` return nonsense and causing
        // `Consensus::checkLedger` to reset back to genesis every round.
        // Fixed by moving signing INSIDE `ConsensusLedgerAcceptor::
        // accept_ledger`'s own inner job (see `application_root.rs`), which
        // runs synchronously right after the real, just-built ledger's hash
        // is known -- passed down here as a `PendingValidation` rather than
        // signed at this call site.
        let pending_validation = self.validator_keys.keys.as_ref().map(|keys| crate::state::application_root::PendingValidation {
            public_key: keys.public_key,
            secret_key: keys.secret_key.clone(),
            node_id: protocol::calc_node_id(&keys.public_key),
            consensus_hash: result.txns.id(),
            proposing: mode == ConsensusMode::Proposing,
        });

        // Matches the reference's `doAccept`'s clock-adjustment block: once
        // we've completed a round with the network (not after a
        // `WrongLedger`/`SwitchedLedger` round, and not after a round that
        // ended in `ConsensusState::MovedOn`), compare our own close time
        // to the weighted average of every peer close-time vote received
        // this round (`rawCloseTimes.peers`, vote count per distinct close
        // time) and nudge `TimeKeeper`'s `closeOffset_` a quarter-step
        // toward that average via `adjust_close_time`. This is the
        // mechanism that lets each node's own future `close_time()`/`now()`
        // calls -- which is what `on_close` uses to build ITS OWN close
        // time proposal for the NEXT round -- gradually converge toward
        // the network's actual observed consensus time, round over round,
        // even if this node's raw local clock started with meaningful
        // jitter relative to its peers (e.g. at genesis/startup, before
        // any convergence has had a chance to happen). Without this call,
        // `close_offset_` never moves, so a node's own close-time votes
        // stay permanently anchored to whatever its local wall clock
        // happened to be at process start, with no self-correction --
        // this was previously a silently dropped parameter
        // (`_raw_close_times`).
        let consensus_fail = result.state == consensus::algorithm::types::ConsensusState::MovedOn;
        if (mode == ConsensusMode::Proposing || mode == ConsensusMode::Observing) && !consensus_fail {
            let close_time = raw_close_times.self_;
            let mut close_total: i64 = i64::from(close_time.as_seconds());
            let mut close_count: i64 = 1;
            for (t, v) in &raw_close_times.peers {
                close_count += i64::from(*v);
                close_total += i64::from(t.as_seconds()) * i64::from(*v);
            }
            close_total += close_count / 2;
            close_total /= close_count;

            let offset_seconds = close_total - i64::from(close_time.as_seconds());
            let new_offset = self.time_keeper.adjust_close_time(time::Duration::seconds(offset_seconds));
            tracing::warn!(
                target: "consensus",
                self_close_time = close_time.as_seconds(),
                peer_vote_count = raw_close_times.peers.len(),
                peer_votes = ?raw_close_times.peers.iter().map(|(t, v)| (t.as_seconds(), *v)).collect::<Vec<_>>(),
                computed_offset_seconds = offset_seconds,
                new_close_offset_seconds = new_offset.whole_seconds(),
                "DIAG close_time_adjust"
            );
        }

        // Store the accept work for synchronous execution in timer_tick,
        // while the consensus state mutex is still held. This eliminates
        // the thread-scheduling gap between on_accept and start_next_round
        // that caused proposals to be missed during the Accepted→Open
        // transition. Matches rippled's single-threaded doAccept →
        // endConsensus → beginConsensus flow.
        {
            let mut pending = self.pending_accept.lock().expect("pending_accept mutex must not be poisoned");
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

/// A dyn-compatible async facade over `Consensus<AppRclConsensusAdaptor>`,
/// matching the four operations `AppConsensusRuntime`
/// (`runtime::component_runtime`) drives from its own async methods.
/// Since Rust does not support `async fn` in trait objects directly, each
/// method returns a boxed future.
pub trait ConsensusRunner: Send + Sync {
    fn start_round<'a>(&'a self, now: NetClockTimePoint, prev_ledger_id: Uint256, prev_ledger: RclCxLedger) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;

    fn peer_proposal<'a>(
        &'a self,
        now: NetClockTimePoint,
        public_key: PublicKey,
        signature: Vec<u8>,
        suppression_id: Uint256,
        proposal: consensus::ConsensusProposal<PublicKey, Uint256, Uint256>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>>;

    fn timer_tick<'a>(&'a self, now: NetClockTimePoint) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;

    fn got_tx_set<'a>(&'a self, now: NetClockTimePoint, txs: Vec<RclCxTx>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;

    /// The current round's phase. Matches the reference's
    /// `Consensus::phase()`, used by `NetworkOPsImp::processHeartbeatTimer`
    /// to detect phase transitions and by `endConsensus`/`beginConsensus`'s
    /// caller to know when a round has finished (`Accepted`) and a new one
    /// needs to be started.
    fn phase(&self) -> consensus::algorithm::ConsensusPhase;

    /// The previous ledger hash the current consensus round is building on.
    /// Used to avoid redundant start_round calls when the round is already
    /// on the correct ledger.
    fn prev_ledger_id(&self) -> Uint256;
}

/// Concrete [`ConsensusRunner`] wrapping Phase 3's
/// `Consensus<AppRclConsensusAdaptor>` state machine behind a blocking
/// mutex (the state machine itself is not thread-safe; the reference
/// documents the same constraint and relies on its single-threaded
/// `io_context` strand for serialization -- this port uses an explicit
/// mutex since there is no equivalent strand here).
pub struct AppConsensus {
    adaptor: AppRclConsensusAdaptor,
    state: StdMutex<Consensus<AppRclConsensusAdaptor>>,
}

impl AppConsensus {
    pub fn new(adaptor: AppRclConsensusAdaptor, _parms: ConsensusParms) -> Self {
        Self { adaptor, state: StdMutex::new(Consensus::new()) }
    }

    /// Execute the accept-ledger work and start the next consensus round
    /// synchronously, while the consensus state mutex is still held. This
    /// matches rippled's single-threaded flow:
    ///   doAccept (build ledger) → endConsensus (checkLastClosedLedger) →
    ///   beginConsensus (start_round)
    /// all happening atomically before any proposal can be processed.
    fn execute_accept_and_start_next_round(
        &self,
        state: &mut Consensus<AppRclConsensusAdaptor>,
        now: NetClockTimePoint,
        work: PendingAcceptWork,
    ) {
        let closed_seq = work.closed_seq;

        // Phase 1: Build the accepted ledger (matches doAccept → buildLCL).
        // This is the heavy work: apply transactions, build SHAMap, persist
        // to NuDB. It runs synchronously here (blocking the consensus
        // timer), matching rippled where the JtAccept job blocks its own
        // strand for the same duration.
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
                // Phase 2: Broadcast StatusChange (neACCEPTED_LEDGER) and
                // sign/publish validation. Matches doAccept tail.
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
                            }
                            Err(err) => {
                                tracing::error!(target: "consensus", closed_seq, ?err, "synchronous accept: validation signing failed");
                            }
                        }
                    }
                }

                // Phase 3: checkLastClosedLedger → beginConsensus (matches
                // endConsensus). Determine if peers prefer a different
                // closed ledger, then start the next round on the consensus
                // state machine INLINE (the mutex is still held, so no
                // proposal can slip in between acceptance and round start).
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
                            "synchronous accept: checkLastClosedLedger: peers prefer different chain"
                        );
                        if let Some(lm_rt) = root.ledger_master_runtime() {
                            if let Some(network_ledger) = lm_rt.ledger_master().get_ledger_by_hash(
                                basics::sha_map_hash::SHAMapHash::new(network_closed)
                            ) {
                                tracing::info!(
                                    target: "consensus",
                                    seq = network_ledger.header().seq,
                                    "synchronous accept: switching to network chain"
                                );
                                root.on_closed_ledger(Arc::clone(&network_ledger));
                                Some(network_ledger)
                            } else {
                                // Ledger not locally available — acquire from peers.
                                // Do NOT start next round on wrong ledger.
                                if let Ok(guard) = lm_rt.shared_inbound_ledgers.lock() {
                                    if let Some(shared) = guard.as_ref() {
                                        shared.acquire(network_closed, 0);
                                        tracing::info!(
                                            target: "consensus",
                                            %network_closed,
                                            "synchronous accept: acquiring network ledger, suppressing round start"
                                        );
                                    }
                                }
                                let _ = root.set_network_ops_operating_mode(
                                    crate::network::network_ops::NetworkOpsOperatingMode::Connected
                                );
                                root.set_need_network_ledger(true);
                                None // Suppress round start
                            }
                        } else {
                            Some(Arc::clone(&closed))
                        }
                    } else {
                        Some(Arc::clone(&closed))
                    };

                    // beginConsensus: start the next round INLINE on the
                    // already-held consensus state. No mutex re-acquisition
                    // needed — we have &mut state from the caller.
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

                        state.start_round(
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
                            "synchronous accept: started next round inline (no scheduling gap)"
                        );
                    }
                } else {
                    // No closed ledger available — wake bootstrap loop
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
    fn start_round<'a>(&'a self, now: NetClockTimePoint, prev_ledger_id: Uint256, prev_ledger: RclCxLedger) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            // Matches rippled's RCLConsensus::Adaptor::preStartRound
            // (RCLConsensus.cpp:998): propose only when operating mode is FULL
            // (synced with network). Without this, the node proposes with
            // potentially wrong close_time while catching up, wasting bandwidth.
            // With the close_time parity fix (skip validated rounds + peer time),
            // this is safe: the node reaches FULL quickly via
            // promote_operating_mode_after_accepted_ledger, and once FULL its
            // close_time is correct (from peer votes in SwitchedLedger mode).
            let proposing = self.adaptor.is_validator()
                && !self.adaptor.options.standalone
                && self.adaptor.network_ops_mode_owner.operating_mode()
                    == crate::network::network_ops::NetworkOpsOperatingMode::Full;
            let mut state = self.state.lock().expect("consensus state mutex must not be poisoned");
            if self.adaptor.validators.count() > 0 && self.adaptor.validators.unl_size() == 0 {
                self.adaptor.network_ops_mode_owner.set_unl_blocked(true);
            } else {
                self.adaptor.network_ops_mode_owner.set_unl_blocked(false);
            }
            state.start_round(&self.adaptor, now, prev_ledger_id, prev_ledger, &HashSet::default(), proposing);
        })
    }

    fn peer_proposal<'a>(
        &'a self,
        now: NetClockTimePoint,
        public_key: PublicKey,
        signature: Vec<u8>,
        suppression_id: Uint256,
        proposal: consensus::ConsensusProposal<PublicKey, Uint256, Uint256>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>> {
        Box::pin(async move {
            let peer_pos = RclCxPeerPos::new(public_key, signature, suppression_id, proposal);
            if !peer_pos.check_sign() {
                return false;
            }
            let mut state = self.state.lock().expect("consensus state mutex must not be poisoned");
            let our_prev = *state.prev_ledger_id();
            let their_prev = *peer_pos.proposal().prev_ledger();
            let accepted = state.peer_proposal(&self.adaptor, now, &peer_pos);
            if !accepted && our_prev != their_prev {
                tracing::info!(target: "consensus",
                    %our_prev, %their_prev,
                    phase = ?state.phase(),
                    "peer_proposal REJECTED: prev_ledger mismatch"
                );
            } else if !accepted {
                tracing::info!(target: "consensus",
                    %our_prev,
                    phase = ?state.phase(),
                    "peer_proposal REJECTED: prev_ledger MATCHES but still rejected (other reason)"
                );
            }
            accepted
        })
    }

    fn timer_tick<'a>(&'a self, now: NetClockTimePoint) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("consensus state mutex must not be poisoned");
            state.timer_entry(&self.adaptor, now);

            // Drain pending accept work synchronously while the consensus
            // mutex is still held. This is the critical parity fix: in
            // rippled, doAccept → endConsensus → beginConsensus all execute
            // atomically on the same strand, so no proposal can arrive
            // between round acceptance and next round start. Previously,
            // accept was dispatched to a job queue thread, creating a
            // 1-100ms gap where proposals were dropped (consensus saw
            // Accepted phase). Now the full sequence runs inline.
            let pending = {
                self.adaptor.pending_accept.lock()
                    .expect("pending_accept mutex must not be poisoned")
                    .take()
            };
            if let Some(work) = pending {
                self.execute_accept_and_start_next_round(&mut state, now, work);
            }
        })
    }

    fn got_tx_set<'a>(&'a self, now: NetClockTimePoint, txs: Vec<RclCxTx>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            // `txs` carries only the transaction ids that make up the
            // completed acquired tx-set (matches `network_ops_runtime.rs`'s
            // `handle_map_complete`, which visits the acquired `SyncTree`'s
            // leaves for ids but does not forward the tree itself). Rebuild
            // an `RclTxSet` from those ids' full transaction blobs, sourced
            // from the transaction master cache -- this reconstruction is
            // deterministic (SHAMap hashing depends only on the tx set's
            // contents), so the resulting `RclTxSet::id()` will match the
            // originally-acquired set's hash as long as every transaction
            // is present in the cache. Transactions this node has already
            // seen (via ordinary relay, which normally arrives at or
            // before tx-set acquisition completes) will be present; any
            // that are not will simply be missing from the reconstructed
            // set, which would produce a hash mismatch `Consensus::got_tx_set`
            // itself cannot detect (it trusts the id it's given) -- this
            // matches the reference's own trust model, which likewise
            // assumes a fully hydrated `SHAMap` from the acquisition layer.
            let mut set = consensus::RclTxSet::new(Arc::clone(&self.adaptor.tx_set_cache), 0);
            {
                let mut editable = set.mutable_view();
                for tx in &txs {
                    if let Some(shared) = self.adaptor.transaction_master.fetch_from_cache(&tx.id) {
                        let guard = shared.lock().expect("transaction master entry mutex must not be poisoned");
                        editable.insert(&consensus::RclCxTxRef::from_transaction(guard.get_s_transaction()));
                    } else {
                        self.adaptor.journal.warn(&format!("got_tx_set: transaction {} not found in local cache; reconstructed tx-set may not match the acquired hash", tx.id));
                    }
                }
                set = editable.freeze();
            }
            let mut state = self.state.lock().expect("consensus state mutex must not be poisoned");
            state.got_tx_set(&self.adaptor, now, &set);
        })
    }

    fn phase(&self) -> consensus::algorithm::ConsensusPhase {
        self.state.lock().expect("consensus state mutex must not be poisoned").phase()
    }

    fn prev_ledger_id(&self) -> Uint256 {
        *self.state.lock().expect("consensus state mutex must not be poisoned").prev_ledger_id()
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

    /// Build a real, completed `SyncTree` containing the given transactions,
    /// matching the shape `InboundTransactions::get_set` hands back once a
    /// peer tx-set finishes acquiring (an unbacked, non-synching tree with
    /// every leaf already attached). Uses `StorageTree` directly (the same
    /// underlying primitive `consensus::RclTxSet` builds on) since
    /// `RclTxSet`'s own root is private to its crate.
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
        // The whole point of the SyncTree->RclTxSet conversion is that an
        // acquired peer tx-set and a locally-reconstructed one containing
        // the exact same transactions must compare equal (same root hash),
        // since that's what lets `Consensus::got_tx_set`'s `id()` check
        // succeed for a set rebuilt from cached transaction blobs (see
        // `AppConsensus::got_tx_set`'s doc comment).
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
