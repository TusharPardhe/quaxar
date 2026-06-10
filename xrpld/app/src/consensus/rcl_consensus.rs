//! App-owned `RCLConsensus::Adaptor` bridge over the landed Rust owners.
//!
//! This keeps the generic `consensus::RclConsensus` engine fed by real app
//! owners instead of ad hoc test-only seams:
//! - validator-list / operating-mode round-start gating,
//! - preferred-ledger and proposer-finished queries through app-owned
//!   validations,
//! - inbound-tx-set acquisition and local tx-set publication through the
//!   landed `InboundTransactions` owner,
//! - and transaction relay using the app-owned `TransactionMaster` cache.
//!
//! The current generic consensus core still models tx sets as `Vec<RclCxTx>`
//! rather than a direct `SHAMap` wrapper. To preserve real owner behavior
//! without rewriting that generic layer in this patch, this adaptor caches the
//! authoritative `SyncTree` alongside the generic vector view. The generic
//! engine keeps using the vector, while the app owner still shares and acquires
//! real SHAMap-backed transaction sets through `InboundTransactions`.

use crate::amendments::amendment_status::AmendmentStatus;
use crate::amendments::negative_unl_vote::{
    NegativeUNLVote, NegativeUNLVoteJournal, NegativeUNLVoteValidations,
};
use crate::consensus::rcl_validations::{
    AppRclConsensusValidationBridge, NullRclValidationJournal, SharedAppValidations,
    validated_ledger_from_ledger,
};
use crate::load::fee_vote::{FeeSetup, FeeVote, FeeVoteJournal};
use crate::network::network_ops::{
    AppNetworkOpsModeOwner, NetworkOpsOperatingMode, SharedNetworkOpsState,
};
use crate::state::application_root::ApplicationRoot;
use crate::state::time_keeper::{TimeKeeper, TimeKeeperClock};
use crate::tx_queue::transaction::Transaction;
use crate::tx_queue::transaction_master::TransactionMaster;
use crate::tx_queue::vote_tx_set::ShamapVoteTxSet;
use crate::validator::validator_keys::ValidatorKeys;
use crate::validator::validator_list::ValidatorList;
use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::slice::Slice;
use basics::tagged_cache::MonotonicClock;
use consensus::{
    ConsensusMode, ConsensusProposal, RclConsensusAdapter, RclCxLedger, RclCxPeerPos, RclCxTx,
    RclValidatedLedger, RclValidations, RclValidationsAdapter, proposal_unique_id, rcl_txset_id,
};
use ledger::{InboundTransactions, Ledger, get_close_agree};
use overlay::{OverlayImpl, TmProposeSet, TmTransaction};
use protocol::{
    NodeID, PUBLIC_KEY_LENGTH, PublicKey, STTx, STValidation, calc_node_id, serialize_blob,
    sha512_half, sign_digest,
};
use serde_json::Value;
use shamap::item::SHAMapItem;
use shamap::storage::StorageTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;
use time::Duration;
use xrpl_core::{HashRouter, ServiceRegistry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRclConsensusOptions {
    pub standalone: bool,
    pub max_disallowed_ledger: u32,
    pub txset_cache_size: usize,
    pub txset_cache_ttl: Duration,
}

impl Default for AppRclConsensusOptions {
    fn default() -> Self {
        Self {
            standalone: false,
            max_disallowed_ledger: 0,
            txset_cache_size: 65_536,
            txset_cache_ttl: Duration::minutes(30),
        }
    }
}

pub trait RclConsensusClock: Send + Sync + 'static {
    fn now(&self) -> basics::chrono::NetClockTimePoint;
    fn close_time(&self) -> basics::chrono::NetClockTimePoint;
}

impl<C> RclConsensusClock for Arc<TimeKeeper<C>>
where
    C: TimeKeeperClock,
{
    fn now(&self) -> basics::chrono::NetClockTimePoint {
        TimeKeeper::now(self)
    }

    fn close_time(&self) -> basics::chrono::NetClockTimePoint {
        TimeKeeper::close_time(self)
    }
}

impl<C> RclConsensusClock for TimeKeeper<C>
where
    C: TimeKeeperClock,
{
    fn now(&self) -> basics::chrono::NetClockTimePoint {
        TimeKeeper::now(self)
    }

    fn close_time(&self) -> basics::chrono::NetClockTimePoint {
        TimeKeeper::close_time(self)
    }
}

pub trait RclConsensusLedgerSource: Send + Sync + 'static {
    fn acquire_consensus_ledger(&self, hash: &Uint256) -> Option<Arc<Ledger>>;
    fn get_valid_ledger_index(&self) -> u32;
    fn have_validated(&self) -> bool;
    fn request_consensus_ledger(&self, _hash: &Uint256) {}
}

impl RclConsensusLedgerSource
    for Arc<crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime>
{
    fn acquire_consensus_ledger(&self, hash: &Uint256) -> Option<Arc<Ledger>> {
        self.ledger_master()
            .get_ledger_by_hash(SHAMapHash::new(*hash))
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.ledger_master().valid_ledger_seq()
    }

    fn have_validated(&self) -> bool {
        self.ledger_master().have_validated()
    }
}

pub trait RclConsensusOpenLedgerSource: Send + Sync + 'static {
    fn current_open_transactions(&self) -> Vec<Arc<STTx>>;
    fn has_open_transactions(&self) -> bool;
}

pub trait RclConsensusValidationSource: Send + Sync + 'static {
    fn num_trusted_for_ledger(&self, ledger_id: Uint256) -> usize;
    fn get_preferred_with_min_seq(&self, curr: RclValidatedLedger, min_valid_seq: u32) -> Uint256;
    fn get_nodes_after(&self, ledger: &RclValidatedLedger, ledger_id: Uint256) -> usize;
    fn laggards(&self, seq: u32, trusted_keys: &mut BTreeSet<PublicKey>) -> usize;
    fn current_node_ids(&self) -> BTreeSet<PublicKey>;
    fn get_json_trie(&self) -> Value;
    fn trusted_parent_validations(&self, _ledger_id: Uint256, _seq: u32) -> Vec<STValidation> {
        Vec::new()
    }
    fn set_seq_to_keep(&self, _low: u32, _high: u32) {}
    fn trusted_keys_for_ledger_by_sequence(
        &self,
        _ledger_id: Uint256,
        _seq: u32,
    ) -> Vec<PublicKey> {
        Vec::new()
    }
}

impl<A> RclConsensusValidationSource for Arc<Mutex<RclValidations<A>>>
where
    A: RclValidationsAdapter + Send + 'static,
{
    fn num_trusted_for_ledger(&self, ledger_id: Uint256) -> usize {
        self.lock()
            .expect("validations mutex must not be poisoned")
            .num_trusted_for_ledger(ledger_id)
    }

    fn get_preferred_with_min_seq(&self, curr: RclValidatedLedger, min_valid_seq: u32) -> Uint256 {
        self.lock()
            .expect("validations mutex must not be poisoned")
            .get_preferred_with_min_seq(curr, min_valid_seq)
    }

    fn get_nodes_after(&self, ledger: &RclValidatedLedger, ledger_id: Uint256) -> usize {
        self.lock()
            .expect("validations mutex must not be poisoned")
            .get_nodes_after(ledger, ledger_id)
    }

    fn laggards(&self, seq: u32, trusted_keys: &mut BTreeSet<PublicKey>) -> usize {
        self.lock()
            .expect("validations mutex must not be poisoned")
            .laggards(seq, trusted_keys)
    }

    fn current_node_ids(&self) -> BTreeSet<PublicKey> {
        self.lock()
            .expect("validations mutex must not be poisoned")
            .current_node_ids()
    }

    fn get_json_trie(&self) -> Value {
        self.lock()
            .expect("validations mutex must not be poisoned")
            .get_json_trie()
    }

    fn set_seq_to_keep(&self, low: u32, high: u32) {
        self.lock()
            .expect("validations mutex must not be poisoned")
            .set_seq_to_keep(low, high);
    }

    fn trusted_keys_for_ledger_by_sequence(&self, ledger_id: Uint256, seq: u32) -> Vec<PublicKey> {
        self.lock()
            .expect("validations mutex must not be poisoned")
            .trusted_for_ledger_by_sequence(ledger_id, seq)
    }
}

impl<A> RclConsensusValidationSource for AppRclConsensusValidationBridge<A>
where
    A: RclValidationsAdapter + Send + 'static,
{
    fn num_trusted_for_ledger(&self, ledger_id: Uint256) -> usize {
        self.validations()
            .lock()
            .expect("validations mutex must not be poisoned")
            .num_trusted_for_ledger(ledger_id)
    }

    fn get_preferred_with_min_seq(&self, curr: RclValidatedLedger, min_valid_seq: u32) -> Uint256 {
        self.validations()
            .lock()
            .expect("validations mutex must not be poisoned")
            .get_preferred_with_min_seq(curr, min_valid_seq)
    }

    fn get_nodes_after(&self, ledger: &RclValidatedLedger, ledger_id: Uint256) -> usize {
        self.validations()
            .lock()
            .expect("validations mutex must not be poisoned")
            .get_nodes_after(ledger, ledger_id)
    }

    fn laggards(&self, seq: u32, trusted_keys: &mut BTreeSet<PublicKey>) -> usize {
        self.validations()
            .lock()
            .expect("validations mutex must not be poisoned")
            .laggards(seq, trusted_keys)
    }

    fn current_node_ids(&self) -> BTreeSet<PublicKey> {
        self.validations()
            .lock()
            .expect("validations mutex must not be poisoned")
            .current_node_ids()
    }

    fn get_json_trie(&self) -> Value {
        self.validations()
            .lock()
            .expect("validations mutex must not be poisoned")
            .get_json_trie()
    }

    fn trusted_parent_validations(&self, ledger_id: Uint256, seq: u32) -> Vec<STValidation> {
        self.store().trusted_for_ledger_by_sequence(ledger_id, seq)
    }

    fn set_seq_to_keep(&self, low: u32, high: u32) {
        self.validations()
            .lock()
            .expect("validations mutex must not be poisoned")
            .set_seq_to_keep(low, high);
    }

    fn trusted_keys_for_ledger_by_sequence(&self, ledger_id: Uint256, seq: u32) -> Vec<PublicKey> {
        self.validations()
            .lock()
            .expect("validations mutex must not be poisoned")
            .trusted_for_ledger_by_sequence(ledger_id, seq)
    }
}

impl<C> RclConsensusValidationSource for SharedAppValidations<C>
where
    C: crate::state::time_keeper::TimeKeeperClock,
{
    fn num_trusted_for_ledger(&self, ledger_id: Uint256) -> usize {
        self.bridge().num_trusted_for_ledger(ledger_id)
    }

    fn get_preferred_with_min_seq(&self, curr: RclValidatedLedger, min_valid_seq: u32) -> Uint256 {
        self.bridge()
            .get_preferred_with_min_seq(curr, min_valid_seq)
    }

    fn get_nodes_after(&self, ledger: &RclValidatedLedger, ledger_id: Uint256) -> usize {
        self.bridge().get_nodes_after(ledger, ledger_id)
    }

    fn laggards(&self, seq: u32, trusted_keys: &mut BTreeSet<PublicKey>) -> usize {
        self.bridge().laggards(seq, trusted_keys)
    }

    fn current_node_ids(&self) -> BTreeSet<PublicKey> {
        self.bridge().current_node_ids()
    }

    fn get_json_trie(&self) -> Value {
        self.bridge().get_json_trie()
    }

    fn trusted_parent_validations(&self, ledger_id: Uint256, seq: u32) -> Vec<STValidation> {
        self.bridge().trusted_parent_validations(ledger_id, seq)
    }

    fn set_seq_to_keep(&self, low: u32, high: u32) {
        self.bridge().set_seq_to_keep(low, high);
    }

    fn trusted_keys_for_ledger_by_sequence(&self, ledger_id: Uint256, seq: u32) -> Vec<PublicKey> {
        self.bridge()
            .trusted_keys_for_ledger_by_sequence(ledger_id, seq)
    }
}

pub trait RclConsensusValidatorSource: Send + Sync + 'static {
    fn count(&self) -> usize;
    fn expires(&self) -> Option<u32>;
    fn get_quorum_keys(&self) -> (usize, HashSet<PublicKey>);
    fn get_trusted_master_keys(&self) -> HashSet<PublicKey>;
}

impl<C> RclConsensusValidatorSource for ValidatorList<C>
where
    C: crate::validator::validator_list::ValidatorListClock,
{
    fn count(&self) -> usize {
        ValidatorList::count(self)
    }

    fn expires(&self) -> Option<u32> {
        ValidatorList::expires(self)
    }

    fn get_quorum_keys(&self) -> (usize, HashSet<PublicKey>) {
        ValidatorList::get_quorum_keys(self)
    }

    fn get_trusted_master_keys(&self) -> HashSet<PublicKey> {
        ValidatorList::get_trusted_master_keys(self)
    }
}

impl RclConsensusValidatorSource for Arc<ValidatorList> {
    fn count(&self) -> usize {
        ValidatorList::count(self)
    }

    fn expires(&self) -> Option<u32> {
        ValidatorList::expires(self)
    }

    fn get_quorum_keys(&self) -> (usize, HashSet<PublicKey>) {
        ValidatorList::get_quorum_keys(self)
    }

    fn get_trusted_master_keys(&self) -> HashSet<PublicKey> {
        ValidatorList::get_trusted_master_keys(self)
    }
}

pub trait RclConsensusModeSource: Send + Sync + 'static {
    fn operating_mode(&self) -> NetworkOpsOperatingMode;
    fn set_operating_mode(&self, mode: NetworkOpsOperatingMode);
    fn is_blocked(&self) -> bool;
    fn need_network_ledger(&self) -> bool;
}

impl RclConsensusModeSource for SharedNetworkOpsState {
    fn operating_mode(&self) -> NetworkOpsOperatingMode {
        SharedNetworkOpsState::operating_mode(self)
    }

    fn set_operating_mode(&self, mode: NetworkOpsOperatingMode) {
        SharedNetworkOpsState::set_operating_mode(self, mode);
    }

    fn is_blocked(&self) -> bool {
        SharedNetworkOpsState::is_blocked(self)
    }

    fn need_network_ledger(&self) -> bool {
        SharedNetworkOpsState::need_network_ledger(self)
    }
}

impl RclConsensusModeSource for Arc<SharedNetworkOpsState> {
    fn operating_mode(&self) -> NetworkOpsOperatingMode {
        SharedNetworkOpsState::operating_mode(self)
    }

    fn set_operating_mode(&self, mode: NetworkOpsOperatingMode) {
        SharedNetworkOpsState::set_operating_mode(self, mode);
    }

    fn is_blocked(&self) -> bool {
        SharedNetworkOpsState::is_blocked(self)
    }

    fn need_network_ledger(&self) -> bool {
        SharedNetworkOpsState::need_network_ledger(self)
    }
}

impl RclConsensusModeSource for AppNetworkOpsModeOwner {
    fn operating_mode(&self) -> NetworkOpsOperatingMode {
        AppNetworkOpsModeOwner::operating_mode(self)
    }

    fn set_operating_mode(&self, mode: NetworkOpsOperatingMode) {
        let _ = AppNetworkOpsModeOwner::set_operating_mode(self, mode);
    }

    fn is_blocked(&self) -> bool {
        AppNetworkOpsModeOwner::is_blocked(self)
    }

    fn need_network_ledger(&self) -> bool {
        AppNetworkOpsModeOwner::need_network_ledger(self)
    }
}

pub trait RclConsensusMessageSink: Send + Sync + 'static {
    fn share_peer_position(&self, peer_position: &RclCxPeerPos);
    fn propose(&self, proposal: &ConsensusProposal<PublicKey, Uint256, Uint256>);
    fn share_tx_set(&self, txset_id: Uint256, tx_count: usize);
    fn share_transaction(&self, tx: Arc<STTx>);

    fn consensus_view_change(&self) {}
}

pub trait RclConsensusJournal: Send + Sync + Clone + 'static {
    fn trace(&self, message: &str);
    fn debug(&self, message: &str);
    fn info(&self, message: &str);
    fn warn(&self, message: &str);
    fn error(&self, message: &str);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NullRclConsensusJournal;

impl RclConsensusJournal for NullRclConsensusJournal {
    fn trace(&self, _message: &str) {}
    fn debug(&self, _message: &str) {}
    fn info(&self, _message: &str) {}
    fn warn(&self, _message: &str) {}
    fn error(&self, _message: &str) {}
}

impl<T> NegativeUNLVoteJournal for T
where
    T: RclConsensusJournal,
{
    fn trace(&self, message: &str) {
        RclConsensusJournal::trace(self, message);
    }

    fn debug(&self, message: &str) {
        RclConsensusJournal::debug(self, message);
    }

    fn warn(&self, message: &str) {
        RclConsensusJournal::warn(self, message);
    }

    fn error(&self, message: &str) {
        RclConsensusJournal::error(self, message);
    }
}

impl<T> FeeVoteJournal for T
where
    T: RclConsensusJournal,
{
    fn info(&self, message: &str) {
        RclConsensusJournal::info(self, message);
    }

    fn warn(&self, message: &str) {
        RclConsensusJournal::warn(self, message);
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NullRclConsensusMessageSink;

impl RclConsensusMessageSink for NullRclConsensusMessageSink {
    fn share_peer_position(&self, _peer_position: &RclCxPeerPos) {}

    fn propose(&self, _proposal: &ConsensusProposal<PublicKey, Uint256, Uint256>) {}

    fn share_tx_set(&self, _txset_id: Uint256, _tx_count: usize) {}

    fn share_transaction(&self, _tx: Arc<STTx>) {}
}

#[derive(Clone)]
pub struct AppRclConsensusRelay<J = NullRclConsensusJournal>
where
    J: RclConsensusJournal,
{
    clock: Arc<dyn RclConsensusClock>,
    hash_router: Arc<HashRouter>,
    overlay: Option<Arc<OverlayImpl>>,
    mode_source: AppNetworkOpsModeOwner,
    validator_keys: ValidatorKeys,
    journal: J,
}

impl<J> std::fmt::Debug for AppRclConsensusRelay<J>
where
    J: RclConsensusJournal,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppRclConsensusRelay")
            .field("has_overlay", &self.overlay.is_some())
            .field("hash_router_entries", &self.hash_router.entry_count())
            .field("operating_mode", &self.mode_source.operating_mode())
            .field("validator", &self.validator_keys.keys.is_some())
            .finish()
    }
}

impl<J> AppRclConsensusRelay<J>
where
    J: RclConsensusJournal,
{
    pub fn new(
        clock: Arc<dyn RclConsensusClock>,
        hash_router: Arc<HashRouter>,
        overlay: Option<Arc<OverlayImpl>>,
        mode_source: AppNetworkOpsModeOwner,
        validator_keys: ValidatorKeys,
        journal: J,
    ) -> Self {
        Self {
            clock,
            hash_router,
            overlay,
            mode_source,
            validator_keys,
            journal,
        }
    }

    pub fn from_application_root(
        root: &ApplicationRoot,
        validator_keys: ValidatorKeys,
        journal: J,
    ) -> Self {
        let clock: Arc<dyn RclConsensusClock> = root.shared_time_keeper();
        let hash_router = Arc::clone(ServiceRegistry::get_hash_router(root));
        let overlay = root.overlay_runtime().map(|runtime| runtime.overlay());
        Self::new(
            clock,
            hash_router,
            overlay,
            root.network_ops_mode_owner(),
            validator_keys,
            journal,
        )
    }

    fn proposal_message(
        public_key: PublicKey,
        signature: Vec<u8>,
        proposal: &ConsensusProposal<PublicKey, Uint256, Uint256>,
    ) -> TmProposeSet {
        TmProposeSet {
            propose_seq: proposal.propose_seq(),
            current_tx_hash: proposal.position().data().to_vec(),
            node_pub_key: public_key.as_bytes().to_vec(),
            close_time: proposal.close_time().as_seconds(),
            signature,
            previousledger: proposal.prev_ledger().data().to_vec(),
            added_transactions: Vec::new(),
            removed_transactions: Vec::new(),
            ..Default::default()
        }
    }

    fn transaction_message(&self, tx: &STTx) -> TmTransaction {
        TmTransaction {
            raw_transaction: serialize_blob(tx),
            status: 1,
            receive_timestamp: Some(u64::from(self.clock.now().as_seconds())),
            deferred: None,
        }
    }
}

impl<J> RclConsensusMessageSink for AppRclConsensusRelay<J>
where
    J: RclConsensusJournal,
{
    fn share_peer_position(&self, peer_position: &RclCxPeerPos) {
        let Some(overlay) = &self.overlay else {
            return;
        };

        let _ = overlay.relay_proposal(
            Self::proposal_message(
                peer_position.public_key,
                peer_position.signature().to_vec(),
                &peer_position.proposal,
            ),
            peer_position.suppression_id,
            peer_position.public_key,
        );
    }

    fn propose(&self, proposal: &ConsensusProposal<PublicKey, Uint256, Uint256>) {
        let Some(keys) = self.validator_keys.keys.as_ref() else {
            self.journal
                .warn("RCLConsensus::Adaptor::propose: ValidatorKeys not set");
            return;
        };

        let signing_hash = sha512_half(proposal.signing_data());
        let Ok(signature) = sign_digest(&keys.public_key, &keys.secret_key, signing_hash) else {
            self.journal
                .warn("RCLConsensus::Adaptor::propose: failed to sign proposal");
            return;
        };

        let suppression = proposal_unique_id(
            *proposal.position(),
            *proposal.prev_ledger(),
            proposal.propose_seq(),
            proposal.close_time(),
            Slice::new(keys.public_key.as_bytes()),
            Slice::new(signature.as_slice()),
        );
        self.hash_router.add_suppression(suppression);

        if let Some(overlay) = &self.overlay {
            overlay.broadcast_proposal(
                Self::proposal_message(keys.public_key, signature, proposal),
                keys.public_key,
            );
        }
    }

    fn share_tx_set(&self, _txset_id: Uint256, _tx_count: usize) {}

    fn share_transaction(&self, tx: Arc<STTx>) {
        let Some(to_skip) = self.hash_router.should_relay(tx.get_transaction_id()) else {
            self.journal.debug(&format!(
                "Not relaying disputed tx {}",
                tx.get_transaction_id()
            ));
            return;
        };

        self.journal
            .debug(&format!("Relaying disputed tx {}", tx.get_transaction_id()));
        if let Some(overlay) = &self.overlay {
            overlay.relay_transaction(
                tx.get_transaction_id(),
                Some(self.transaction_message(tx.as_ref())),
                &to_skip,
            );
        }
    }

    fn consensus_view_change(&self) {
        // fires frequently during normal tip-tracking and causes cascading
        // panics from mutex poisoning. Guard same as update_operating_mode.
        if self.validator_keys.keys.is_some()
            && matches!(
                self.mode_source.operating_mode(),
                NetworkOpsOperatingMode::Full | NetworkOpsOperatingMode::Tracking
            )
        {
            self.mode_source
                .set_operating_mode_direct(NetworkOpsOperatingMode::Connected);
        }
    }
}

pub struct AppRclConsensusAdaptor<CLOCK, LEDGERS, OPEN, VALIDATIONS, VALIDATORS, MODE, SINK, J>
where
    CLOCK: RclConsensusClock,
    LEDGERS: RclConsensusLedgerSource,
    OPEN: RclConsensusOpenLedgerSource,
    VALIDATIONS: RclConsensusValidationSource,
    VALIDATORS: RclConsensusValidatorSource,
    MODE: RclConsensusModeSource,
    SINK: RclConsensusMessageSink,
    J: RclConsensusJournal,
{
    options: AppRclConsensusOptions,
    clock: CLOCK,
    ledgers: LEDGERS,
    open_ledger: OPEN,
    validations: VALIDATIONS,
    validators: VALIDATORS,
    mode_source: MODE,
    ledger_acceptor: Arc<dyn crate::state::application_root::LedgerAcceptor>,
    inbound_transactions: Arc<Mutex<InboundTransactions>>,
    transaction_master: Arc<TransactionMaster>,
    message_sink: SINK,
    journal: J,
    validator_keys: ValidatorKeys,
    fee_vote: Option<FeeVote<J>>,
    amendment_table: Option<Arc<AmendmentStatus>>,
    negative_unl_vote: NegativeUNLVote<J>,
    txsets: Mutex<HashMap<Uint256, Arc<SyncTree>>>,
    known_ledgers: Mutex<HashMap<Uint256, Arc<Ledger>>>,
    txset_tree_cache: Arc<TreeNodeCache<MonotonicClock, HardenedHashBuilder>>,
    next_cowid: AtomicU32,
    validating: AtomicBool,
    consensus_cookie: u64,
    acquiring_ledger: Mutex<Option<Uint256>>,
    prev_proposers: AtomicUsize,
    prev_round_time_millis: AtomicU64,
    mode: AtomicU8,
    /// Pending start_round parameters queued by end_consensus.
    /// Consumed by the next timer_tick to start the next round.
    pending_start_round: Mutex<Option<(basics::chrono::NetClockTimePoint, Uint256, RclCxLedger)>>,
}

impl RclConsensusLedgerSource for crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime {
    fn acquire_consensus_ledger(&self, hash: &Uint256) -> Option<Arc<Ledger>> {
        self.ledger_master()
            .get_ledger_by_hash(basics::sha_map_hash::SHAMapHash::new(*hash))
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.ledger_master().valid_ledger_seq()
    }

    fn have_validated(&self) -> bool {
        self.ledger_master().have_validated()
    }
}

impl<CLOCK, LEDGERS, OPEN, VALIDATIONS, VALIDATORS, MODE, SINK, J>
    AppRclConsensusAdaptor<CLOCK, LEDGERS, OPEN, VALIDATIONS, VALIDATORS, MODE, SINK, J>
where
    CLOCK: RclConsensusClock,
    LEDGERS: RclConsensusLedgerSource,
    OPEN: RclConsensusOpenLedgerSource,
    VALIDATIONS: RclConsensusValidationSource,
    VALIDATORS: RclConsensusValidatorSource,
    MODE: RclConsensusModeSource,
    SINK: RclConsensusMessageSink,
    J: RclConsensusJournal,
{
    pub fn new(
        options: AppRclConsensusOptions,
        clock: CLOCK,
        ledgers: LEDGERS,
        open_ledger: OPEN,
        validations: VALIDATIONS,
        validators: VALIDATORS,
        mode_source: MODE,
        ledger_acceptor: Arc<dyn crate::state::application_root::LedgerAcceptor>,
        inbound_transactions: Arc<Mutex<InboundTransactions>>,
        transaction_master: Arc<TransactionMaster>,
        message_sink: SINK,
        journal: J,
        validator_keys: ValidatorKeys,
        fee_setup: Option<FeeSetup>,
        amendment_table: Option<Arc<AmendmentStatus>>,
    ) -> Self {
        static NEXT_CONSENSUS_COOKIE: AtomicU64 = AtomicU64::new(1);

        let consensus_cookie = NEXT_CONSENSUS_COOKIE.fetch_add(1, Ordering::AcqRel).max(1);
        let my_node_id = validator_keys
            .keys
            .as_ref()
            .map(|keys| calc_node_id(&keys.public_key))
            .unwrap_or_else(|| calc_node_id(&placeholder_public_key()));
        let negative_unl_vote = NegativeUNLVote::new(my_node_id, journal.clone());
        journal.info(&format!(
            "Consensus engine started (cookie: {consensus_cookie})"
        ));
        if !validator_keys.node_id.is_zero()
            && let Some(keys) = validator_keys.keys.as_ref()
        {
            journal.info(&format!(
                "Validator identity: {}",
                keys.master_public_key.to_node_public_base58()
            ));
            if keys.master_public_key != keys.public_key {
                journal.debug(&format!(
                    "Validator ephemeral signing key: {} (seq: {})",
                    keys.public_key.to_node_public_base58(),
                    validator_keys.sequence
                ));
            }
        }

        Self {
            txset_tree_cache: Arc::new(TreeNodeCache::new(
                "RclConsensusTxSetTreeCache",
                options.txset_cache_size,
                options.txset_cache_ttl,
                MonotonicClock::default(),
            )),
            options,
            clock,
            ledgers,
            open_ledger,
            validations,
            validators,
            mode_source,
            ledger_acceptor,
            inbound_transactions,
            transaction_master,
            message_sink,
            validator_keys,
            fee_vote: fee_setup.map(|setup| FeeVote::new(setup, journal.clone())),
            journal,
            amendment_table,
            negative_unl_vote,
            txsets: Mutex::new(HashMap::new()),
            known_ledgers: Mutex::new(HashMap::new()),
            next_cowid: AtomicU32::new(1),
            validating: AtomicBool::new(false),
            consensus_cookie,
            acquiring_ledger: Mutex::new(None),
            prev_proposers: AtomicUsize::new(0),
            prev_round_time_millis: AtomicU64::new(0),
            mode: AtomicU8::new(encode_consensus_mode(ConsensusMode::Observing)),
            pending_start_round: Mutex::new(None),
        }
    }

    pub fn remember_ledger(&self, ledger: Arc<Ledger>) -> RclCxLedger {
        let converted = consensus_ledger_from_ledger(ledger.as_ref());
        let ledger = if !ledger.has_node_fetcher() {
            if let Some(fetcher) = self.ledger_acceptor.node_fetcher() {
                let mut l = ledger.as_ref().clone();
                l.set_node_fetcher(fetcher);
                Arc::new(l)
            } else {
                ledger
            }
        } else {
            ledger
        };
        self.known_ledgers
            .lock()
            .expect("known ledgers mutex must not be poisoned")
            .insert(converted.id, ledger);
        converted
    }

    pub fn validating(&self) -> bool {
        self.validating.load(Ordering::Acquire)
    }

    /// Enable validating/proposing if keys are configured. Called before
    /// start_round so the first round can propose immediately.
    pub fn pre_start_round_for_proposing(&self) {
        if self.validator_keys.keys.is_some() && !self.mode_source.is_blocked() {
            self.validating.store(true, Ordering::Release);
        }
    }

    pub const fn consensus_cookie(&self) -> u64 {
        self.consensus_cookie
    }

    pub fn set_fee_vote(&mut self, target: FeeSetup) {
        self.fee_vote = Some(FeeVote::new(target, self.journal.clone()));
    }

    pub fn set_amendment_table(&mut self, amendment_table: Arc<AmendmentStatus>) {
        self.amendment_table = Some(amendment_table);
    }

    pub fn prev_proposers(&self) -> usize {
        self.prev_proposers.load(Ordering::Acquire)
    }

    pub fn prev_round_time(&self) -> StdDuration {
        StdDuration::from_millis(self.prev_round_time_millis.load(Ordering::Acquire))
    }

    pub fn mode(&self) -> ConsensusMode {
        decode_consensus_mode(self.mode.load(Ordering::Acquire))
    }

    pub fn validator(&self) -> bool {
        self.validator_keys.keys.is_some()
    }

    pub fn have_validated(&self) -> bool {
        self.ledgers.have_validated()
    }

    pub fn get_valid_ledger_index(&self) -> u32 {
        self.ledgers.get_valid_ledger_index()
    }

    pub fn get_quorum_keys(&self) -> (usize, HashSet<PublicKey>) {
        self.validators.get_quorum_keys()
    }

    pub fn laggards(&self, seq: u32, trusted_keys: &mut BTreeSet<PublicKey>) -> usize {
        self.validations.laggards(seq, trusted_keys)
    }

    pub fn current_node_ids(&self) -> BTreeSet<PublicKey> {
        self.validations.current_node_ids()
    }

    pub fn pre_start_round(&self, prev_ledger: &Ledger, now_trusted: &HashSet<NodeID>) -> bool {
        let mut validating = self.validator_keys.keys.is_some()
            && prev_ledger.header().seq >= self.options.max_disallowed_ledger
            && !self.mode_source.is_blocked();

        if validating && !self.options.standalone && self.validators.count() != 0 {
            let expired = self
                .validators
                .expires()
                .is_some_and(|when| when < self.clock.now().as_seconds());
            if expired {
                self.journal.error(
                    "Voluntarily bowing out of consensus process because of an expired validator list.",
                );
                validating = false;
            }
        }

        self.validating.store(validating, Ordering::Release);

        let synced = self.mode_source.operating_mode() == NetworkOpsOperatingMode::Full;
        if validating {
            self.journal.info(&format!(
                "Entering consensus process, validating, synced={}",
                if synced { "yes" } else { "no" }
            ));
        } else {
            self.journal.info(&format!(
                "Entering consensus process, watching, synced={}",
                if synced { "yes" } else { "no" }
            ));
        }

        self.inbound_transactions
            .lock()
            .expect("inbound transactions mutex must not be poisoned")
            .new_round(prev_ledger.header().seq);

        if !now_trusted.is_empty() {
            self.negative_unl_vote
                .new_validators(prev_ledger.header().seq + 1, now_trusted);
        }

        validating && synced
    }

    pub fn update_operating_mode(&self, positions: usize) {
        // Downgrades Full→Connected when no proposals were seen this round,
        // indicating the node lost touch with the network consensus.
        if positions == 0 && self.mode_source.operating_mode() == NetworkOpsOperatingMode::Full {
            // Only downgrade if we're a validator. Non-validating nodes that
            // don't participate in consensus shouldn't be penalized for not
            // seeing proposals in their local consensus state machine.
            if self.validator_keys.keys.is_some() {
                self.mode_source
                    .set_operating_mode(NetworkOpsOperatingMode::Connected);
            }
        }
    }

    fn store_txset(&self, txset_id: Uint256, txset: Arc<SyncTree>) {
        self.txsets
            .lock()
            .expect("txsets mutex must not be poisoned")
            .insert(txset_id, txset);
    }

    fn lookup_ledger(&self, ledger_id: &Uint256) -> Option<Arc<Ledger>> {
        self.known_ledgers
            .lock()
            .expect("known ledgers mutex must not be poisoned")
            .get(ledger_id)
            .cloned()
    }

    fn cache_transaction(&self, tx: Arc<STTx>) {
        let mut shared = Arc::new(Mutex::new(Transaction::new(tx)));
        self.transaction_master.canonicalize(&mut shared);
    }

    fn decode_txset(&self, txset: &SyncTree) -> Option<Vec<RclCxTx>> {
        let mut decoded = Vec::new();
        let mut failed = None::<String>;
        let mut no_fetch = |_hash| None;
        let visit = txset.visit_leaves(&mut no_fetch, &mut |item| match self
            .transaction_master
            .fetch_from_shamap_item(item, SHAMapNodeType::TransactionNm, 0)
        {
            Ok(Some(tx)) => {
                self.cache_transaction(Arc::clone(&tx));
                decoded.push(RclCxTx {
                    id: tx.get_transaction_id(),
                });
            }
            Ok(None) => {
                failed = Some("transaction set leaf was not a transaction".to_owned());
            }
            Err(error) => {
                failed = Some(error);
            }
        });

        if let Err(error) = visit {
            self.journal
                .warn(&format!("failed to traverse consensus tx set: {error:?}"));
            return None;
        }

        if let Some(error) = failed {
            self.journal
                .warn(&format!("failed to decode consensus tx set leaf: {error}"));
            return None;
        }

        decoded.sort_by_key(|tx| tx.id);
        Some(decoded)
    }

    fn build_txset(
        &self,
        previous_ledger: Option<&Ledger>,
        ledger_seq: u32,
        txs: &[Arc<STTx>],
    ) -> (Vec<RclCxTx>, Arc<SyncTree>) {
        let cowid = self.next_cowid.fetch_add(1, Ordering::AcqRel).max(1);
        let mut map =
            StorageTree::new(cowid, false, ledger_seq, Arc::clone(&self.txset_tree_cache));

        for tx in txs {
            let inserted = map
                .add_item(
                    SHAMapNodeType::TransactionNm,
                    SHAMapItem::new(tx.get_transaction_id(), serialize_blob(tx.as_ref())),
                )
                .unwrap_or(false);
            if inserted {
                self.cache_transaction(Arc::clone(tx));
            }
        }

        if let Some(previous_ledger) = previous_ledger {
            let mut vote_tx_set = ShamapVoteTxSet::new(&mut map);

            if previous_ledger.is_flag_ledger() {
                let parent_validations = self.validations.trusted_parent_validations(
                    *previous_ledger.header().hash.as_uint256(),
                    previous_ledger.header().seq,
                );
                if let Some(fee_vote) = &self.fee_vote {
                    fee_vote.do_voting(previous_ledger, &parent_validations, &mut vote_tx_set);
                }
                if let Some(amendment_table) = &self.amendment_table {
                    amendment_table
                        .set_trusted_validators(self.validators.get_trusted_master_keys());
                    let _ = amendment_table.do_voting_for_ledger(
                        previous_ledger,
                        &parent_validations,
                        &mut vote_tx_set,
                    );
                }
            }

            if previous_ledger.is_voting_ledger() {
                let unl_keys = self.validators.get_trusted_master_keys();
                let mut validations = ConsensusNegativeUnlValidations {
                    source: &self.validations,
                };
                self.negative_unl_vote.do_voting(
                    previous_ledger,
                    &unl_keys,
                    &mut validations,
                    &mut vote_tx_set,
                );
            }
        }

        let set = Arc::new(SyncTree::from_root_with_type(
            map.root(),
            SHAMapType::Transaction,
            false,
            ledger_seq,
            SyncState::Modifying,
        ));
        let result = self
            .decode_txset(set.as_ref())
            .expect("locally built consensus tx set should decode");
        (result, set)
    }

    /// Extracts TXs from the agreed set, builds the ledger, stores it.
    fn do_accept(
        &self,
        result: &consensus::ConsensusResult<
            Uint256,
            PublicKey,
            Vec<RclCxTx>,
            Uint256,
            RclCxTx,
            Uint256,
        >,
        prev_ledger: &RclCxLedger,
        seq: u32,
    ) -> Option<Arc<Ledger>> {
        // Look up the TX set SHAMap from the agreed position
        let txset_id = result.position.position();
        let txset_map = self
            .txsets
            .lock()
            .expect("txsets mutex")
            .get(txset_id)
            .cloned();
        let parent = self.ledgers.acquire_consensus_ledger(&prev_ledger.id);

        let (txset, parent) = match (txset_map, parent) {
            (Some(t), Some(p)) => (t, p),
            (ref t, ref p) => {
                tracing::info!(target: "consensus",
                    "[consensus] on_accept seq={} — missing txset={} parent={} prev_id={:x?}",
                    seq,
                    t.is_none(),
                    p.is_none(),
                    &prev_ledger.id.data()[..4]
                );
                return None;
            }
        };

        // Set node_fetcher on parent so it can read state from NuDB
        let parent = if !parent.has_node_fetcher() {
            if let Some(fetcher) = self.ledger_acceptor.node_fetcher() {
                let mut p = parent.as_ref().clone();
                p.set_node_fetcher(fetcher);
                Arc::new(p)
            } else {
                parent
            }
        } else {
            parent
        };

        // Extract TX blobs from the SHAMap leaves (reference iterates result.txns.map)
        let mut tx_items: Vec<(Vec<u8>, Uint256)> = Vec::new();
        let mut fetch = |_hash: basics::sha_map_hash::SHAMapHash| -> Option<
            basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >,
        > { None };
        let _ = txset.visit_leaves(&mut fetch, &mut |item: &SHAMapItem| {
            tx_items.push((item.data().to_vec(), item.key()));
        });

        tracing::info!(target: "consensus",
            "[consensus] doAccept seq={} tx_count={} parent={:02x}{:02x}{:02x}{:02x}",
            seq,
            tx_items.len(),
            prev_ledger.id.data()[0],
            prev_ledger.id.data()[1],
            prev_ledger.id.data()[2],
            prev_ledger.id.data()[3],
        );

        // Build header from consensus result (reference buildLedgerImpl)
        let close_time = result.position.close_time().as_seconds();
        let close_time_resolution = prev_ledger.close_time_resolution.whole_seconds() as u32;
        let close_flags = if close_time == 0 { 1u8 } else { 0u8 };

        let mut acquired_header = parent.header();
        acquired_header.seq = seq;
        acquired_header.close_time = close_time;
        acquired_header.parent_close_time = prev_ledger.close_time.as_seconds();
        acquired_header.close_time_resolution = close_time_resolution as u8;
        acquired_header.close_flags = close_flags;
        acquired_header.parent_hash = SHAMapHash::new(prev_ledger.id);
        acquired_header.account_hash = SHAMapHash::default();
        acquired_header.tx_hash = txset.root().get_hash();
        acquired_header.hash = SHAMapHash::default();

        // Build the ledger
        let consensus_fetcher = self.ledger_acceptor.node_fetcher();
        let build_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::build_ledger_from_consensus(
                parent.as_ref(),
                acquired_header,
                &tx_items,
                consensus_fetcher,
            )
        }));

        match build_result {
            Ok(Some(built)) => {
                let built = Arc::new(built);
                tracing::info!(target: "consensus",
                    "[consensus] BUILT seq={} hash={:02x}{:02x}{:02x}{:02x} drops={}",
                    seq,
                    built.header().hash.as_uint256().data()[0],
                    built.header().hash.as_uint256().data()[1],
                    built.header().hash.as_uint256().data()[2],
                    built.header().hash.as_uint256().data()[3],
                    built.header().drops,
                );
                let _ = self.ledger_acceptor.consensus_built(Arc::clone(&built));
                Some(built)
            }
            Ok(None) => {
                tracing::info!(target: "consensus", "BUILD FAILED seq={}", seq);
                None
            }
            Err(panic) => {
                let msg = panic
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown");
                tracing::info!(target: "consensus", "BUILD PANIC seq={} — {}", seq, msg);
                None
            }
        }
    }

    /// Performs state transitions and starts the next consensus round.
    fn end_consensus(&self, built_ledger: Option<&Arc<Ledger>>, prev_ledger: &RclCxLedger) {
        let network_closed = self
            .ledger_acceptor
            .consensus_closed_ledger()
            .map(|ledger| *ledger.header().hash.as_uint256())
            .unwrap_or(prev_ledger.id);

        if network_closed == Uint256::zero() {
            tracing::info!(target: "consensus", "endConsensus: network closed is zero, skipping");
            return;
        }

        let current_mode = self.mode_source.operating_mode();
        let need_network_ledger = self.mode_source.need_network_ledger();
        let ledger_change = built_ledger
            .map(|built| network_closed != *built.header().hash.as_uint256())
            .unwrap_or(false);

        // CONNECTED/SYNCING + no ledger change → TRACKING
        if (current_mode == NetworkOpsOperatingMode::Connected
            || current_mode == NetworkOpsOperatingMode::Syncing)
            && !need_network_ledger
            && !ledger_change
        {
            self.mode_source
                .set_operating_mode(NetworkOpsOperatingMode::Tracking);
            tracing::info!(target: "consensus",
                "[consensus] endConsensus: {} → TRACKING",
                current_mode.as_str()
            );
        }

        // CONNECTED/TRACKING + ledger is fresh → FULL
        if (current_mode == NetworkOpsOperatingMode::Connected
            || current_mode == NetworkOpsOperatingMode::Tracking
            || self.mode_source.operating_mode() == NetworkOpsOperatingMode::Tracking)
            && !need_network_ledger
            && !ledger_change
            && built_ledger.is_some()
        {
            let built = built_ledger.unwrap();
            let now_seconds = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .saturating_sub(946684800)) as u32;
            let freshness_limit = built.header().parent_close_time
                + 2 * (built.header().close_time_resolution as u32);
            if now_seconds < freshness_limit {
                self.mode_source
                    .set_operating_mode(NetworkOpsOperatingMode::Full);
                tracing::info!(target: "consensus", "endConsensus: → FULL (ledger is fresh)");
            }
        }

        // owner-selected current ledger parent, not from a built-ledger
        // shortcut inside consensus.
        let next_prev = self
            .ledger_acceptor
            .consensus_previous_ledger()
            .or_else(|| self.ledgers.acquire_consensus_ledger(&network_closed));

        if let Some(ledger) = next_prev {
            let now = self.clock.close_time();
            let prev_cx = consensus_ledger_from_ledger(&ledger);
            let next_seq = prev_cx.seq;
            // Store in known_ledgers so consensus can look it up
            self.known_ledgers
                .lock()
                .expect("known_ledgers mutex")
                .insert(*ledger.header().hash.as_uint256(), ledger);

            // We can't call start_round here because we're inside the adaptor
            // which is behind the consensus mutex. Instead, store the next round
            // info and let the caller (timer_tick) pick it up.
            // Actually in the reference, endConsensus is called OUTSIDE the consensus lock
            // (it's in a separate job). So we need a different approach.
            //
            // For now, store the pending start_round parameters.
            self.pending_start_round
                .lock()
                .expect("pending_start_round mutex")
                .replace((now, network_closed, prev_cx));

            tracing::info!(target: "consensus",
                "[consensus] endConsensus: queued start_round for seq={}",
                next_seq
            );
        } else {
            tracing::info!(target: "consensus", "endConsensus: no ledger for next round");
        }
    }
}

struct ConsensusNegativeUnlValidations<'a, V> {
    source: &'a V,
}

impl<V> NegativeUNLVoteValidations for ConsensusNegativeUnlValidations<'_, V>
where
    V: RclConsensusValidationSource,
{
    fn set_seq_to_keep(&mut self, low: u32, high: u32) {
        self.source.set_seq_to_keep(low, high);
    }

    fn trusted_keys_for_ledger(&mut self, ledger_id: Uint256, seq: u32) -> Vec<PublicKey> {
        self.source
            .trusted_keys_for_ledger_by_sequence(ledger_id, seq)
    }
}

impl<CLOCK, LEDGERS, OPEN, VALIDATIONS, VALIDATORS, MODE, SINK, J> RclConsensusAdapter
    for AppRclConsensusAdaptor<CLOCK, LEDGERS, OPEN, VALIDATIONS, VALIDATORS, MODE, SINK, J>
where
    CLOCK: RclConsensusClock,
    LEDGERS: RclConsensusLedgerSource,
    OPEN: RclConsensusOpenLedgerSource,
    VALIDATIONS: RclConsensusValidationSource,
    VALIDATORS: RclConsensusValidatorSource,
    MODE: RclConsensusModeSource,
    SINK: RclConsensusMessageSink,
    J: RclConsensusJournal,
{
    fn now(&self) -> basics::chrono::NetClockTimePoint {
        self.clock.now()
    }

    fn acquire_ledger(&mut self, ledger_id: &Uint256) -> Option<RclCxLedger> {
        let Some(ledger) = self.ledgers.acquire_consensus_ledger(ledger_id) else {
            let mut acquiring = self
                .acquiring_ledger
                .lock()
                .expect("acquiring ledger mutex must not be poisoned");
            if acquiring.as_ref() != Some(ledger_id) {
                self.journal
                    .warn(&format!("Need consensus ledger {ledger_id}"));
                *acquiring = Some(*ledger_id);
                self.ledgers.request_consensus_ledger(ledger_id);
            }
            return None;
        };
        if !ledger.is_immutable() || *ledger.header().hash.as_uint256() != *ledger_id {
            self.journal.warn(&format!(
                "rejected consensus ledger {ledger_id} because it was not immutable or its hash mismatched"
            ));
            return None;
        }
        *self
            .acquiring_ledger
            .lock()
            .expect("acquiring ledger mutex must not be poisoned") = None;

        self.inbound_transactions
            .lock()
            .expect("inbound transactions mutex must not be poisoned")
            .new_round(ledger.header().seq);

        Some(self.remember_ledger(ledger))
    }

    fn acquire_tx_set(&mut self, txset_id: &Uint256) -> Option<Vec<RclCxTx>> {
        let set = self
            .inbound_transactions
            .lock()
            .expect("inbound transactions mutex must not be poisoned")
            .get_set(*txset_id, true)?;
        let txs = self.decode_txset(set.as_ref())?;
        self.store_txset(*txset_id, set);
        Some(txs)
    }

    fn has_open_transactions(&self) -> bool {
        self.open_ledger.has_open_transactions()
    }

    fn proposers_validated(&self, prev_ledger: &Uint256) -> usize {
        self.validations.num_trusted_for_ledger(*prev_ledger)
    }

    fn proposers_finished(&self, prev_ledger: &RclCxLedger, prev_ledger_id: &Uint256) -> usize {
        let Some(ledger) = self.lookup_ledger(&prev_ledger.id) else {
            return 0;
        };
        let validated = validated_ledger_from_ledger(ledger.as_ref(), &NullRclValidationJournal);
        self.validations
            .get_nodes_after(&validated, *prev_ledger_id)
    }


    fn pre_start_round_for_proposing(&self) {
        self.pre_start_round_for_proposing();
    }
    fn should_propose(&self) -> bool {
        self.validating() && self.mode_source.operating_mode() == NetworkOpsOperatingMode::Full
    }

    fn prev_round_time(&self) -> StdDuration {
        AppRclConsensusAdaptor::prev_round_time(self)
    }

    fn now_close_time(&self) -> basics::chrono::NetClockTimePoint {
        self.clock.close_time()
    }

    fn get_prev_ledger(
        &mut self,
        prev_ledger_id: &Uint256,
        prev_ledger: &RclCxLedger,
        mode: ConsensusMode,
    ) -> Uint256 {
        let Some(ledger) = self.lookup_ledger(&prev_ledger.id) else {
            return *prev_ledger_id;
        };
        let preferred = self.validations.get_preferred_with_min_seq(
            validated_ledger_from_ledger(ledger.as_ref(), &NullRclValidationJournal),
            self.ledgers.get_valid_ledger_index(),
        );

        if preferred != *prev_ledger_id {
            if mode != ConsensusMode::WrongLedger {
                self.message_sink.consensus_view_change();
            }
            self.journal
                .debug(&self.validations.get_json_trie().to_string());
        }

        preferred
    }

    fn on_mode_change(&mut self, before: ConsensusMode, after: ConsensusMode) {
        let _ = before;
        self.mode
            .store(encode_consensus_mode(after), Ordering::Release);
    }

    fn on_accept(
        &mut self,
        result: &consensus::ConsensusResult<
            Uint256,
            PublicKey,
            Vec<RclCxTx>,
            Uint256,
            RclCxTx,
            Uint256,
        >,
        prev_ledger: &RclCxLedger,
    ) {
        self.prev_proposers
            .store(result.proposers, Ordering::Release);
        let millis = u64::try_from(result.round_time.read().as_millis()).unwrap_or(u64::MAX);
        self.prev_round_time_millis.store(millis, Ordering::Release);

        let proposers = result.proposers;
        if proposers == 0 {
            tracing::warn!(target: "consensus", "No proposals received this round");
        }
        tracing::info!(target: "consensus", round = prev_ledger.seq + 1, duration_ms = millis, proposers, "Consensus round complete");

        self.update_operating_mode(result.proposers);

        let seq = prev_ledger.seq + 1;

        // === reference doAccept equivalent ===
        let built_ledger = self.do_accept(result, prev_ledger, seq);

        // === reference endConsensus equivalent ===
        self.end_consensus(built_ledger.as_ref(), prev_ledger);
    }

    fn make_txset(&mut self, previous_ledger: &RclCxLedger) -> (Vec<RclCxTx>, Uint256) {
        let txs = self.open_ledger.current_open_transactions();
        let previous = self.lookup_ledger(&previous_ledger.id);
        let (result, sync_tree) =
            self.build_txset(previous.as_deref(), previous_ledger.seq + 1, &txs);
        let txset_ids = result.iter().map(|tx| tx.id).collect::<Vec<_>>();
        let txset_id = rcl_txset_id(&txset_ids);
        self.store_txset(txset_id, sync_tree);
        (result, txset_id)
    }

    fn propose(&mut self, proposal: &ConsensusProposal<PublicKey, Uint256, Uint256>) {
        self.message_sink.propose(proposal);
    }

    fn share_peer_position(&mut self, peer_position: &RclCxPeerPos) {
        self.message_sink.share_peer_position(peer_position);
    }

    fn share_tx_set(&mut self, txset: &[RclCxTx]) {
        let txset_ids = txset.iter().map(|tx| tx.id).collect::<Vec<_>>();
        let txset_id = rcl_txset_id(&txset_ids);
        if let Some(set) = self
            .txsets
            .lock()
            .expect("txsets mutex must not be poisoned")
            .get(&txset_id)
            .cloned()
        {
            self.inbound_transactions
                .lock()
                .expect("inbound transactions mutex must not be poisoned")
                .give_set(txset_id, set, false);
            self.message_sink.share_tx_set(txset_id, txset.len());
        } else {
            self.journal.warn(&format!(
                "consensus tx set {txset_id} was not cached with an authoritative SyncTree"
            ));
        }
    }

    fn share_tx(&mut self, tx: &RclCxTx) {
        let Some(shared) = self.transaction_master.fetch_from_cache(&tx.id) else {
            self.journal.warn(&format!(
                "disputed tx {} missing from transaction cache",
                tx.id
            ));
            return;
        };

        let tx = Arc::clone(
            shared
                .lock()
                .expect("transaction mutex must not be poisoned")
                .get_s_transaction(),
        );
        self.message_sink.share_transaction(tx);
    }

    fn node_id(&self) -> PublicKey {
        self.validator_keys
            .keys
            .as_ref()
            .map(|keys| keys.public_key)
            .unwrap_or_else(placeholder_public_key)
    }

    fn take_pending_start_round(
        &self,
    ) -> Option<(basics::chrono::NetClockTimePoint, Uint256, RclCxLedger)> {
        self.pending_start_round
            .lock()
            .expect("pending_start_round mutex")
            .take()
    }
}

pub fn consensus_ledger_from_ledger(ledger: &Ledger) -> RclCxLedger {
    RclCxLedger {
        id: *ledger.header().hash.as_uint256(),
        seq: ledger.header().seq,
        parent_id: *ledger.header().parent_hash.as_uint256(),
        close_time_resolution: Duration::seconds(i64::from(ledger.header().close_time_resolution)),
        close_agree: get_close_agree(&ledger.header()),
        close_time: basics::chrono::NetClockTimePoint::new(ledger.header().close_time),
        parent_close_time: basics::chrono::NetClockTimePoint::new(
            ledger.header().parent_close_time,
        ),
        base_fee_req: ledger.fees().base, // or .base
    }
}

fn placeholder_public_key() -> PublicKey {
    let mut bytes = [0u8; PUBLIC_KEY_LENGTH];
    bytes[0] = 0x02;
    PublicKey::from_bytes(bytes)
}

const fn encode_consensus_mode(mode: ConsensusMode) -> u8 {
    match mode {
        ConsensusMode::Proposing => 0,
        ConsensusMode::Observing => 1,
        ConsensusMode::WrongLedger => 2,
        ConsensusMode::SwitchedLedger => 3,
    }
}

const fn decode_consensus_mode(mode: u8) -> ConsensusMode {
    match mode {
        0 => ConsensusMode::Proposing,
        2 => ConsensusMode::WrongLedger,
        3 => ConsensusMode::SwitchedLedger,
        _ => ConsensusMode::Observing,
    }
}

pub struct AppConsensus<A: RclConsensusAdapter> {
    consensus: tokio::sync::Mutex<consensus::RclConsensus<A>>,
}

impl<A: RclConsensusAdapter> AppConsensus<A> {
    pub fn new(adapter: A, parms: consensus::ConsensusParms) -> Self {
        Self {
            consensus: tokio::sync::Mutex::new(consensus::RclConsensus::new(adapter, parms)),
        }
    }

    pub async fn timer_tick(&self, now: basics::chrono::NetClockTimePoint) {
        let mut consensus = self.consensus.lock().await;
        let _decision = consensus.timer_tick(now).await;

        // Consume them on every tick — the round may have been queued
        // during on_accept (which returns Accepted) but by the time
        // timer_tick runs again the state machine has already moved on.
        if let Some((round_now, prev_id, prev_cx)) =
            consensus.adaptor().inner.take_pending_start_round()
        {
            consensus.start_round(round_now, prev_id, prev_cx);
            tracing::info!(target: "consensus", "timer_tick: started next round");
        }
    }

    pub async fn start_round(
        &self,
        now: basics::chrono::NetClockTimePoint,
        prev_ledger_id: Uint256,
        prev_ledger: RclCxLedger,
    ) {
        let round = prev_ledger.seq + 1;
        tracing::info!(target: "consensus", round, "Consensus round started");
        let mut consensus = self.consensus.lock().await;
        consensus.start_round(now, prev_ledger_id, prev_ledger);
    }

    pub async fn got_tx_set(&self, now: basics::chrono::NetClockTimePoint, txset: Vec<RclCxTx>) {
        tracing::debug!(target: "consensus", tx_count = txset.len(), "Transaction set received");
        let mut consensus = self.consensus.lock().await;
        consensus.got_tx_set(now, txset);
    }
}

pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

pub trait ConsensusRunner: Send + Sync + 'static {
    fn timer_tick(&self, now: basics::chrono::NetClockTimePoint) -> BoxFuture<'_, ()>;
    fn start_round(
        &self,
        now: basics::chrono::NetClockTimePoint,
        prev_ledger_id: Uint256,
        prev_ledger: RclCxLedger,
    ) -> BoxFuture<'_, ()>;
    /// has been acquired from peers.
    fn got_tx_set(
        &self,
        now: basics::chrono::NetClockTimePoint,
        txset: Vec<RclCxTx>,
    ) -> BoxFuture<'_, ()>;
    fn peer_proposal(
        &self,
        now: basics::chrono::NetClockTimePoint,
        public_key: protocol::PublicKey,
        signature: Vec<u8>,
        suppression_id: basics::base_uint::Uint256,
        proposal: consensus::ConsensusProposal<
            protocol::PublicKey,
            basics::base_uint::Uint256,
            basics::base_uint::Uint256,
        >,
    ) -> BoxFuture<'_, bool>;
}

impl<A: RclConsensusAdapter + Send + Sync + 'static> ConsensusRunner for AppConsensus<A> {
    fn timer_tick(&self, now: basics::chrono::NetClockTimePoint) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.timer_tick(now).await;
        })
    }

    fn start_round(
        &self,
        now: basics::chrono::NetClockTimePoint,
        prev_ledger_id: Uint256,
        prev_ledger: RclCxLedger,
    ) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.start_round(now, prev_ledger_id, prev_ledger).await;
        })
    }

    fn got_tx_set(
        &self,
        now: basics::chrono::NetClockTimePoint,
        txset: Vec<RclCxTx>,
    ) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.got_tx_set(now, txset).await;
        })
    }

    fn peer_proposal(
        &self,
        now: basics::chrono::NetClockTimePoint,
        public_key: protocol::PublicKey,
        signature: Vec<u8>,
        suppression_id: basics::base_uint::Uint256,
        proposal: consensus::ConsensusProposal<
            protocol::PublicKey,
            basics::base_uint::Uint256,
            basics::base_uint::Uint256,
        >,
    ) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            tracing::debug!(target: "consensus", peer_id = ?&suppression_id.data()[..4], position = ?proposal.position().data()[..4], "Proposal received");
            let peer_pos =
                consensus::RclCxPeerPos::new(public_key, &signature, suppression_id, proposal);
            let mut consensus = self.consensus.lock().await;
            consensus.peer_proposal(now, peer_pos)
        })
    }
}

#[cfg(test)]
mod tests;
