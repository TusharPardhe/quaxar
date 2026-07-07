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
    fn accept_consensus_ledger(&self, next_seq: u32, base_fee: u64, parent_hash: &Uint256);
}

/// The validation-tracking surface the adaptor needs to answer
/// `proposers_validated`/`proposers_finished`/`get_prev_ledger`. Kept as a
/// narrow trait (rather than depending on `SharedAppValidations` directly
/// in the `ConsensusAdaptor` impl) so the adaptor's dependency on
/// validation state stays swappable/testable.
pub trait RclConsensusValidationSource {
    fn num_trusted_for_ledger(&self, ledger_id: Uint256) -> usize;
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
    validator_keys: ValidatorKeys,
    journal: Arc<dyn RclConsensusJournal>,
}

impl AppRclConsensusRelay {
    /// Construct a relay bound to `root`'s overlay (if attached) and the
    /// given validator identity/journal. Matches the reference's
    /// `RCLConsensus::Adaptor` constructor capturing `app_` for later
    /// `app_.overlay()` access.
    pub fn from_application_root(root: &ApplicationRoot, validator_keys: ValidatorKeys, journal: impl RclConsensusJournal + 'static) -> Self {
        Self { overlay: root.overlay_runtime().map(|rt| rt.overlay()), validator_keys, journal: Arc::new(journal) }
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
            hash: set.id().data().to_vec(),
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
pub struct AppRclConsensusAdaptor {
    options: AppRclConsensusOptions,
    time_keeper: Arc<TimeKeeper<SystemTimeKeeperClock>>,
    ledger_master_runtime: Arc<AppLedgerMasterRuntime>,
    open_ledger: SharedAppOpenLedger,
    validations: SharedAppValidations<SystemTimeKeeperClock>,
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

    fn proposers_finished(&self, _prev_ledger: &Self::Ledger, prev_ledger_id: &Uint256) -> usize {
        RclConsensusValidationSource::num_trusted_for_ledger(&self.validations, *prev_ledger_id)
    }

    fn get_prev_ledger(&self, _prev_ledger_id: &Uint256, prev_ledger: &Self::Ledger, _mode: ConsensusMode) -> Uint256 {
        // Matches the reference's `RCLConsensus::Adaptor::getPrevLedger`:
        // ask the trust trie (built purely from received trusted
        // validations, independent of what any single peer reports) which
        // branch it prefers, using `ledgerMaster_.getValidLedgerIndex()` as
        // the minimum sequence floor. This is the real catch-up trigger --
        // if this node's own view has fallen behind what the network's
        // trusted validators have actually validated, the trie will prefer
        // a different (further-ahead) ledger id than `prev_ledger_id`, and
        // `Consensus::checkLedger` will detect that mismatch and call
        // `handleWrongLedger` to force this node to acquire and switch to
        // it -- rather than blindly continuing to build on its own stale
        // view, which is what happens if this always echoes back
        // `prev_ledger_id` unconditionally (the bug this replaces: seeding
        // `getPreferredLCL`'s peer-counts map with only our own id can
        // never disagree with ourselves, permanently defeating catch-up).
        let min_valid_seq = self.ledger_master_runtime.ledger_master().valid_ledger_seq();
        let wrapped = self.validated_view(prev_ledger);
        RclConsensusValidationSource::preferred_min_seq(&self.validations, &wrapped, min_valid_seq)
    }

    fn on_mode_change(&self, before: ConsensusMode, after: ConsensusMode) {
        self.journal.info(&format!("Consensus mode change {before:?} -> {after:?}"));
    }

    fn on_close(&self, prev_ledger: &Self::Ledger, now: NetClockTimePoint, _mode: ConsensusMode) -> consensus::algorithm::consensus::ConsensusResultOf<Self> {
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

        consensus::algorithm::types::ConsensusResult::new(set, position, position_id)
    }

    fn on_accept(
        &self,
        result: &consensus::algorithm::consensus::ConsensusResultOf<Self>,
        prev_ledger: &Self::Ledger,
        _close_resolution: StdDuration,
        _raw_close_times: &ConsensusCloseTimes,
        mode: ConsensusMode,
    ) {
        let next_seq = prev_ledger.seq() + 1;
        let base_fee = self.ledger_master_runtime.ledger_master().closed_ledger().map(|l| l.fees().base).unwrap_or(10);

        RclConsensusOpenLedgerSource::accept_consensus_ledger(&self.open_ledger, next_seq, base_fee, &prev_ledger.id());

        let close_time = result.position.close_time().as_seconds();
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

        let ledger_acceptor = Arc::clone(&self.ledger_acceptor);
        let journal = Arc::clone(&self.journal);
        if let Err(err) = ledger_acceptor.accept_ledger(closed_seq, close_time, base_fee, pending_validation) {
            journal.error(&format!("on_accept: accept_ledger failed: {err}"));
        }
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
}

impl ConsensusRunner for AppConsensus {
    fn start_round<'a>(&'a self, now: NetClockTimePoint, prev_ledger_id: Uint256, prev_ledger: RclCxLedger) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let proposing = self.adaptor.is_validator() && !self.adaptor.options.standalone;
            let mut state = self.state.lock().expect("consensus state mutex must not be poisoned");
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
            state.peer_proposal(&self.adaptor, now, &peer_pos)
        })
    }

    fn timer_tick<'a>(&'a self, now: NetClockTimePoint) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("consensus state mutex must not be poisoned");
            state.timer_entry(&self.adaptor, now);
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
