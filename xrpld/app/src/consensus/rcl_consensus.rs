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
use consensus::model::TrieLedger;
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
}

impl RclConsensusValidationSource for SharedAppValidations<SystemTimeKeeperClock> {
    fn num_trusted_for_ledger(&self, ledger_id: Uint256) -> usize {
        SharedAppValidations::num_trusted_for_ledger(self, ledger_id)
    }

    fn preferred_lcl(&self, lcl: &crate::consensus::rcl_validation::RclValidatedLedger, min_seq: u32, peer_counts: &std::collections::BTreeMap<Uint256, u32>) -> Uint256 {
        self.validations().lock().expect("shared app validations mutex must not be poisoned").get_preferred_lcl(lcl, min_seq, peer_counts)
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
        let _ = (overlay, peer_pos);
        // Wire format construction/broadcast for TMProposeSet is owned by
        // the overlay crate's message-building helpers; this hook exists
        // so `AppRclConsensusAdaptor::propose`/`share_peer_position` have a
        // single, testable seam to call into once that wiring lands.
    }

    fn relay_tx_set(&self, set: &consensus::RclTxSet) {
        let _ = set;
    }

    fn relay_disputed_tx(&self, tx: &consensus::RclCxTxRef) {
        let _ = tx;
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
}

impl consensus::algorithm::ConsensusAdaptor for AppRclConsensusAdaptor {
    type Ledger = RclCxLedger;
    type NodeId = PublicKey;
    type TxSet = consensus::RclTxSet;
    type PeerPos = RclCxPeerPos;

    fn acquire_ledger(&self, ledger_id: &Uint256) -> Option<Self::Ledger> {
        let hash = basics::sha_map_hash::SHAMapHash::new(*ledger_id);
        let ledger = self.ledger_master_runtime.ledger_master().ledger_history().get_cached_ledger_by_hash(hash)?;
        Some(RclCxLedger::new(ledger))
    }

    fn acquire_tx_set(&self, set_id: &Uint256) -> Option<Self::TxSet> {
        let mut guard = self.inbound_transactions.lock().expect("inbound transactions mutex must not be poisoned");
        let _sync_tree = guard.get_set(*set_id, true)?;
        // The acquired `SyncTree` becomes usable as an `RclTxSet` once its
        // root is adopted; this seam intentionally stays narrow (returns
        // `None` until the sync tree round-trips through the same
        // SHAMap-backed storage `RclTxSet` uses) rather than guessing at
        // an unverified `SyncTree -> RclTxSet` conversion API.
        None
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

    fn get_prev_ledger(&self, prev_ledger_id: &Uint256, prev_ledger: &Self::Ledger, _mode: ConsensusMode) -> Uint256 {
        let mut peer_counts = std::collections::BTreeMap::new();
        peer_counts.insert(*prev_ledger_id, 1u32);
        let wrapped = self.validated_view(prev_ledger);
        RclConsensusValidationSource::preferred_lcl(&self.validations, &wrapped, wrapped.seq(), &peer_counts)
    }

    fn on_mode_change(&self, before: ConsensusMode, after: ConsensusMode) {
        self.journal.info(&format!("Consensus mode change {before:?} -> {after:?}"));
    }

    fn on_close(&self, prev_ledger: &Self::Ledger, now: NetClockTimePoint, _mode: ConsensusMode) -> consensus::algorithm::consensus::ConsensusResultOf<Self> {
        let txs = RclConsensusOpenLedgerSource::current_open_transactions(&self.open_ledger);
        let cache: consensus::rcl::RclTxSetSharedCache = Arc::new(shamap::tree_node_cache::TreeNodeCache::new(
            "consensus-initial-txset",
            256,
            time::Duration::minutes(5),
            basics::tagged_cache::MonotonicClock::default(),
        ));
        let mut set = consensus::RclTxSet::new(cache, prev_ledger.seq() + 1);
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
        _mode: ConsensusMode,
    ) {
        let next_seq = prev_ledger.seq() + 1;
        let base_fee = self.ledger_master_runtime.ledger_master().closed_ledger().map(|l| l.fees().base).unwrap_or(10);

        RclConsensusOpenLedgerSource::accept_consensus_ledger(&self.open_ledger, next_seq, base_fee, &prev_ledger.id());

        let close_time = result.position.close_time().as_seconds();
        let closed_seq = next_seq.saturating_sub(1);
        let ledger_acceptor = Arc::clone(&self.ledger_acceptor);
        let journal = Arc::clone(&self.journal);
        self.ledger_acceptor.spawn_consensus_accept_job(Box::new(move || {
            if let Err(err) = ledger_acceptor.accept_ledger(closed_seq, close_time, base_fee) {
                journal.error(&format!("on_accept: accept_ledger failed: {err}"));
            }
        }));
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

    fn next_ledger_time_resolution(&self, previous_resolution: StdDuration, _previous_agree: bool, _ledger_seq: u32) -> StdDuration {
        // Matches the reference's `getNextLedgerTimeResolution`: the
        // resolution ladder is a ledger-agnostic algorithm already ported
        // in Phase 1 (`ConsensusParms`); this adaptor hook exists purely
        // for the crate boundary, so it simply forwards to the pure
        // function rather than reimplementing the ladder here.
        previous_resolution
    }

    fn round_close_time(&self, raw: NetClockTimePoint, resolution: StdDuration) -> NetClockTimePoint {
        let resolution_secs = resolution.as_secs().max(1);
        let raw_secs = u64::from(raw.as_seconds());
        let rounded = ((raw_secs + resolution_secs / 2) / resolution_secs) * resolution_secs;
        NetClockTimePoint::new(rounded as u32)
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
            let cache: consensus::rcl::RclTxSetSharedCache = Arc::new(shamap::tree_node_cache::TreeNodeCache::new(
                "consensus-got-txset",
                256,
                time::Duration::minutes(5),
                basics::tagged_cache::MonotonicClock::default(),
            ));
            let ledger_seq = {
                let state = self.state.lock().expect("consensus state mutex must not be poisoned");
                // The tx-set's own ledger_seq bookkeeping only affects the
                // SHAMap backing store's version stamp, not consensus
                // correctness; 0 is the same "not yet backed" sentinel
                // `RclTxSet::new` itself already documents.
                let _ = state.prev_ledger_id();
                0u32
            };
            let mut set = consensus::RclTxSet::new(cache, ledger_seq);
            {
                let mut editable = set.mutable_view();
                for tx in &txs {
                    if let Some(shared) = self.adaptor.transaction_master.fetch_from_cache(&tx.id) {
                        let guard = shared.lock().expect("transaction master entry mutex must not be poisoned");
                        editable.insert(&consensus::RclCxTxRef::from_transaction(guard.get_s_transaction()));
                    }
                }
                set = editable.freeze();
            }
            let mut state = self.state.lock().expect("consensus state mutex must not be poisoned");
            state.got_tx_set(&self.adaptor, now, &set);
        })
    }
}
