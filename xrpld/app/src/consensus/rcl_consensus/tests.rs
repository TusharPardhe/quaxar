use super::{
    AppRclConsensusAdaptor, AppRclConsensusOptions, FeeSetup, NullRclConsensusJournal,
    RclConsensusClock, RclConsensusJournal, RclConsensusLedgerSource, RclConsensusMessageSink,
    RclConsensusModeSource, RclConsensusOpenLedgerSource, RclConsensusValidationSource,
    RclConsensusValidatorSource, consensus_ledger_from_ledger,
};
use crate::network::network_ops::NetworkOpsOperatingMode;
use crate::tx_queue::transaction_master::TransactionMaster;
use crate::validator::validator_keys::ValidatorKeys;
use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::MonotonicClock;
use consensus::{
    ConsensusMode, ConsensusProposal, RclConsensusAdapter, RclCxPeerPos, RclCxTx,
    RclValidatedLedger, rcl_txset_id,
};
use ledger::{Fees, InboundTransactions, Ledger};
use overlay::SimplePeerSetBuilder;
use protocol::{
    AccountID, KeyType, PublicKey, STAmount, STTx, STValidation, SecretKey, TxType,
    VF_FULL_VALIDATION, calc_node_id, derive_public_key, get_field_by_symbol, serialize_blob,
};
use serde_json::{Value, json};
use shamap::item::SHAMapItem;
use shamap::storage::StorageTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use time::Duration;

#[derive(Clone)]
struct FixedClock {
    now: NetClockTimePoint,
    close_time: NetClockTimePoint,
}

impl RclConsensusClock for FixedClock {
    fn now(&self) -> NetClockTimePoint {
        self.now
    }

    fn close_time(&self) -> NetClockTimePoint {
        self.close_time
    }
}

#[derive(Default)]
struct TestLedgerSource {
    ledgers: HashMap<Uint256, Arc<Ledger>>,
    valid_ledger_index: u32,
    have_validated: bool,
}

impl RclConsensusLedgerSource for TestLedgerSource {
    fn acquire_consensus_ledger(&self, hash: &Uint256) -> Option<Arc<Ledger>> {
        self.ledgers.get(hash).cloned()
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.valid_ledger_index
    }

    fn have_validated(&self) -> bool {
        self.have_validated
    }
}

#[derive(Default)]
struct TestOpenLedgerSource {
    txs: Vec<Arc<STTx>>,
}

impl RclConsensusOpenLedgerSource for TestOpenLedgerSource {
    fn current_open_transactions(&self) -> Vec<Arc<STTx>> {
        self.txs.clone()
    }

    fn has_open_transactions(&self) -> bool {
        !self.txs.is_empty()
    }
}

#[derive(Default)]
struct TestValidationSource {
    trusted_per_ledger: HashMap<Uint256, usize>,
    preferred: Option<Uint256>,
    nodes_after: usize,
    node_ids: BTreeSet<PublicKey>,
    json_trie: Value,
    parent_validations: HashMap<(Uint256, u32), Vec<STValidation>>,
    trusted_by_sequence: HashMap<(Uint256, u32), Vec<PublicKey>>,
    keep_ranges: Arc<Mutex<Vec<(u32, u32)>>>,
}

impl RclConsensusValidationSource for TestValidationSource {
    fn num_trusted_for_ledger(&self, ledger_id: Uint256) -> usize {
        self.trusted_per_ledger
            .get(&ledger_id)
            .copied()
            .unwrap_or(0)
    }

    fn get_preferred_with_min_seq(&self, curr: RclValidatedLedger, _min_valid_seq: u32) -> Uint256 {
        self.preferred.unwrap_or(curr.ledger_id)
    }

    fn get_nodes_after(&self, _ledger: &RclValidatedLedger, _ledger_id: Uint256) -> usize {
        self.nodes_after
    }

    fn laggards(&self, _seq: u32, trusted_keys: &mut BTreeSet<PublicKey>) -> usize {
        let laggards = trusted_keys.len().saturating_sub(self.node_ids.len());
        trusted_keys.retain(|key| self.node_ids.contains(key));
        laggards
    }

    fn current_node_ids(&self) -> BTreeSet<PublicKey> {
        self.node_ids.clone()
    }

    fn get_json_trie(&self) -> Value {
        self.json_trie.clone()
    }

    fn trusted_parent_validations(&self, ledger_id: Uint256, seq: u32) -> Vec<STValidation> {
        self.parent_validations
            .get(&(ledger_id, seq))
            .cloned()
            .unwrap_or_default()
    }

    fn set_seq_to_keep(&self, low: u32, high: u32) {
        self.keep_ranges
            .lock()
            .expect("keep ranges")
            .push((low, high));
    }

    fn trusted_keys_for_ledger_by_sequence(&self, ledger_id: Uint256, seq: u32) -> Vec<PublicKey> {
        self.trusted_by_sequence
            .get(&(ledger_id, seq))
            .cloned()
            .unwrap_or_default()
    }

    fn add_trusted_validation(&self, _node_id: PublicKey, _validation: &STValidation) {}
}

#[derive(Default)]
struct TestValidatorSource {
    count: usize,
    expires: Option<u32>,
    quorum: usize,
    quorum_keys: HashSet<PublicKey>,
    trusted_master_keys: HashSet<PublicKey>,
}

impl RclConsensusValidatorSource for TestValidatorSource {
    fn count(&self) -> usize {
        self.count
    }

    fn expires(&self) -> Option<u32> {
        self.expires
    }

    fn get_quorum_keys(&self) -> (usize, HashSet<PublicKey>) {
        (self.quorum, self.quorum_keys.clone())
    }

    fn get_trusted_master_keys(&self) -> HashSet<PublicKey> {
        self.trusted_master_keys.clone()
    }
}

#[derive(Default)]
struct TestModeSource {
    mode: AtomicU8,
    blocked: AtomicBool,
    need_network_ledger: AtomicBool,
}

impl TestModeSource {
    fn new(mode: NetworkOpsOperatingMode) -> Self {
        Self {
            mode: AtomicU8::new(match mode {
                NetworkOpsOperatingMode::Disconnected => 0,
                NetworkOpsOperatingMode::Connected => 1,
                NetworkOpsOperatingMode::Syncing => 2,
                NetworkOpsOperatingMode::Tracking => 3,
                NetworkOpsOperatingMode::Full => 4,
            }),
            blocked: AtomicBool::new(false),
            need_network_ledger: AtomicBool::new(false),
        }
    }

    fn with_need_network_ledger(self, need_network_ledger: bool) -> Self {
        self.need_network_ledger
            .store(need_network_ledger, Ordering::Release);
        self
    }
}

impl RclConsensusModeSource for TestModeSource {
    fn operating_mode(&self) -> NetworkOpsOperatingMode {
        match self.mode.load(Ordering::Acquire) {
            1 => NetworkOpsOperatingMode::Connected,
            2 => NetworkOpsOperatingMode::Syncing,
            3 => NetworkOpsOperatingMode::Tracking,
            4 => NetworkOpsOperatingMode::Full,
            _ => NetworkOpsOperatingMode::Disconnected,
        }
    }

    fn set_operating_mode(&self, mode: NetworkOpsOperatingMode) {
        self.mode.store(
            match mode {
                NetworkOpsOperatingMode::Disconnected => 0,
                NetworkOpsOperatingMode::Connected => 1,
                NetworkOpsOperatingMode::Syncing => 2,
                NetworkOpsOperatingMode::Tracking => 3,
                NetworkOpsOperatingMode::Full => 4,
            },
            Ordering::Release,
        );
    }

    fn is_blocked(&self) -> bool {
        self.blocked.load(Ordering::Acquire)
    }

    fn need_network_ledger(&self) -> bool {
        self.need_network_ledger.load(Ordering::Acquire)
    }
}

#[derive(Default)]
struct RecordingSink {
    proposed: Mutex<Vec<Uint256>>,
    shared_txset_ids: Mutex<Vec<Uint256>>,
    shared_tx_ids: Mutex<Vec<Uint256>>,
    shared_transactions: Mutex<Vec<Arc<STTx>>>,
    peer_positions: Mutex<Vec<Uint256>>,
    view_changes: AtomicUsize,
}

impl RclConsensusMessageSink for Arc<RecordingSink> {
    fn share_peer_position(&self, peer_position: &RclCxPeerPos) {
        self.peer_positions
            .lock()
            .expect("peer positions mutex")
            .push(*peer_position.proposal.prev_ledger());
    }

    fn propose(&self, proposal: &ConsensusProposal<PublicKey, Uint256, Uint256>) {
        self.proposed
            .lock()
            .expect("proposed mutex")
            .push(*proposal.position());
    }

    fn share_tx_set(&self, txset_id: Uint256, _tx_count: usize) {
        self.shared_txset_ids
            .lock()
            .expect("shared txset mutex")
            .push(txset_id);
    }

    fn share_transaction(&self, tx: Arc<STTx>) {
        self.shared_tx_ids
            .lock()
            .expect("shared tx mutex")
            .push(tx.get_transaction_id());
        self.shared_transactions
            .lock()
            .expect("shared transactions mutex")
            .push(tx);
    }

    fn consensus_view_change(&self) {
        self.view_changes.fetch_add(1, Ordering::AcqRel);
    }

    fn broadcast_validation(&self, _validation_bytes: Vec<u8>, _validator: PublicKey) {}
}

#[derive(Default, Clone)]
struct RecordingJournal {
    entries: Arc<Mutex<Vec<(String, String)>>>,
}

impl RclConsensusJournal for RecordingJournal {
    fn trace(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal mutex")
            .push(("trace".to_owned(), message.to_owned()));
    }

    fn debug(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal mutex")
            .push(("debug".to_owned(), message.to_owned()));
    }

    fn info(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal mutex")
            .push(("info".to_owned(), message.to_owned()));
    }

    fn warn(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal mutex")
            .push(("warn".to_owned(), message.to_owned()));
    }

    fn error(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal mutex")
            .push(("error".to_owned(), message.to_owned()));
    }
}

fn signed_validation(
    seed: u8,
    ledger_id: Uint256,
    seq: u32,
    fill: impl FnOnce(&mut STValidation),
) -> STValidation {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("validator public key");
    let mut validation = STValidation::new_signed(
        1_000,
        &public,
        calc_node_id(&public),
        &secret,
        |validation| {
            validation.set_field_h256(get_field_by_symbol("sfLedgerHash"), ledger_id);
            validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), seq);
            validation.set_flag(VF_FULL_VALIDATION);
            fill(validation);
        },
    )
    .expect("signed validation");
    validation.set_trusted();
    validation
}

fn payment_tx(sequence: u32, fill: u8) -> Arc<STTx> {
    Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(
            get_field_by_symbol("sfAccount"),
            AccountID::from_array([fill; 20]),
        );
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            AccountID::from_array([fill.wrapping_add(1); 20]),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(u64::from(sequence) + 10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
    }))
}

fn build_sync_tree(ledger_seq: u32, txs: &[Arc<STTx>]) -> Arc<SyncTree> {
    let cache = Arc::new(TreeNodeCache::<MonotonicClock>::new(
        "TestConsensusTxSet",
        32,
        Duration::minutes(5),
        MonotonicClock::default(),
    ));
    let mut map = StorageTree::new(1, false, ledger_seq, cache);
    for tx in txs {
        let _ = map.add_item(
            SHAMapNodeType::TransactionNm,
            SHAMapItem::new(tx.get_transaction_id(), serialize_blob(tx.as_ref())),
        );
    }
    Arc::new(SyncTree::from_root_with_type(
        map.root(),
        SHAMapType::Transaction,
        false,
        ledger_seq,
        SyncState::Modifying,
    ))
}

fn sample_validator_keys() -> ValidatorKeys {
    let secret = SecretKey::from_bytes([9; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("validator key");
    let mut keys = ValidatorKeys::default();
    keys.keys = Some(crate::validator::validator_keys::Keys {
        master_public_key: public,
        public_key: public,
        secret_key: secret,
    });
    keys.node_id = crate::validator::validator_keys::calc_node_id(&public);
    keys
}

#[derive(Default)]
struct MockLedgerAcceptor;
impl crate::state::application_root::LedgerAcceptor for MockLedgerAcceptor {
    fn accept_ledger(
        &self,
        _closed_seq: u32,
        _close_time: u32,
        _base_fee_drops: u64,
    ) -> Result<u32, String> {
        Ok(0)
    }

    fn consensus_built(&self, _ledger: Arc<Ledger>) -> Result<(), String> {
        Ok(())
    }

    fn consensus_closed_ledger(&self) -> Option<Arc<Ledger>> {
        None
    }

    fn consensus_previous_ledger(&self) -> Option<Arc<Ledger>> {
        None
    }
}

#[derive(Default)]
struct OwnerLedgerAcceptor {
    closed: Option<Arc<Ledger>>,
    previous: Option<Arc<Ledger>>,
}

impl crate::state::application_root::LedgerAcceptor for OwnerLedgerAcceptor {
    fn accept_ledger(
        &self,
        _closed_seq: u32,
        _close_time: u32,
        _base_fee_drops: u64,
    ) -> Result<u32, String> {
        Ok(0)
    }

    fn consensus_built(&self, _ledger: Arc<Ledger>) -> Result<(), String> {
        Ok(())
    }

    fn consensus_closed_ledger(&self) -> Option<Arc<Ledger>> {
        self.closed.clone()
    }

    fn consensus_previous_ledger(&self) -> Option<Arc<Ledger>> {
        self.previous.clone()
    }
}

#[test]
fn app_rcl_consensus_pre_start_round_matches_current_cpp_gates() {
    let ledger = Arc::new(Ledger::from_ledger_seq_and_close_time(400, 900, false));
    let inbound = Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
        SimplePeerSetBuilder::new(Vec::new()),
    ))));
    let mode = TestModeSource::new(NetworkOpsOperatingMode::Full);
    let journal = RecordingJournal::default();
    let adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_000),
            close_time: NetClockTimePoint::new(1_001),
        },
        TestLedgerSource::default(),
        TestOpenLedgerSource::default(),
        TestValidationSource::default(),
        TestValidatorSource {
            count: 1,
            expires: Some(2_000),
            quorum: 1,
            quorum_keys: HashSet::new(),
            trusted_master_keys: HashSet::new(),
        },
        mode,
        Arc::new(MockLedgerAcceptor::default()),
        Arc::clone(&inbound),
        Arc::new(TransactionMaster::new()),
        Arc::new(RecordingSink::default()),
        journal.clone(),
        sample_validator_keys(),
        None,
        None,
    );

    assert!(adaptor.pre_start_round(ledger.as_ref(), &HashSet::new()));
    assert!(adaptor.validating());

    let entries = journal.entries.lock().expect("journal entries");
    assert!(entries.iter().any(|(level, message)| level == "info"
        && message.contains("Entering consensus process, validating, synced=yes")));
}

#[test]
fn app_rcl_consensus_bows_out_when_validator_list_is_expired() {
    let ledger = Arc::new(Ledger::from_ledger_seq_and_close_time(400, 900, false));
    let journal = RecordingJournal::default();
    let adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_000),
            close_time: NetClockTimePoint::new(1_001),
        },
        TestLedgerSource::default(),
        TestOpenLedgerSource::default(),
        TestValidationSource::default(),
        TestValidatorSource {
            count: 2,
            expires: Some(999),
            quorum: 1,
            quorum_keys: HashSet::new(),
            trusted_master_keys: HashSet::new(),
        },
        TestModeSource::new(NetworkOpsOperatingMode::Full),
        Arc::new(MockLedgerAcceptor::default()),
        Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        )))),
        Arc::new(TransactionMaster::new()),
        Arc::new(RecordingSink::default()),
        journal.clone(),
        sample_validator_keys(),
        None,
        None,
    );

    assert!(!adaptor.pre_start_round(ledger.as_ref(), &HashSet::new()));
    assert!(!adaptor.validating());
    assert!(
        journal
            .entries
            .lock()
            .expect("journal entries")
            .iter()
            .any(|(level, message)| level == "error" && message.contains("expired validator list"))
    );
}

#[test]
fn app_rcl_consensus_acquires_and_re_shares_real_sync_txsets() {
    let tx_one = payment_tx(1, 0x11);
    let tx_two = payment_tx(2, 0x22);
    let txs = vec![Arc::clone(&tx_one), Arc::clone(&tx_two)];
    let set = build_sync_tree(600, &txs);
    let txset_id = {
        let mut tx_ids = txs
            .iter()
            .map(|tx| RclCxTx {
                id: tx.get_transaction_id(),
            })
            .collect::<Vec<_>>();
        tx_ids.sort_by_key(|tx| tx.id);
        let ids = tx_ids.iter().map(|tx| tx.id).collect::<Vec<_>>();
        rcl_txset_id(&ids)
    };

    let inbound = Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
        SimplePeerSetBuilder::new(Vec::new()),
    ))));
    {
        let _ =
            inbound
                .lock()
                .expect("inbound tx mutex")
                .give_set(txset_id, Arc::clone(&set), false);
    }

    let sink = Arc::new(RecordingSink::default());
    let mut adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_000),
            close_time: NetClockTimePoint::new(1_001),
        },
        TestLedgerSource::default(),
        TestOpenLedgerSource::default(),
        TestValidationSource::default(),
        TestValidatorSource::default(),
        TestModeSource::new(NetworkOpsOperatingMode::Connected),
        Arc::new(MockLedgerAcceptor::default()),
        Arc::clone(&inbound),
        Arc::new(TransactionMaster::new()),
        Arc::clone(&sink),
        NullRclConsensusJournal,
        sample_validator_keys(),
        None,
        None,
    );

    let decoded = adaptor
        .acquire_tx_set(&txset_id)
        .expect("consensus tx set should decode");
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].id, tx_one.get_transaction_id());
    assert_eq!(decoded[1].id, tx_two.get_transaction_id());

    adaptor.share_tx_set(&decoded);
    assert_eq!(
        sink.shared_txset_ids
            .lock()
            .expect("shared txset ids")
            .as_slice(),
        &[txset_id]
    );
}

#[test]
fn app_rcl_consensus_make_txset_builds_sync_tree_and_relays_cached_disputed_tx() {
    let tx_one = payment_tx(10, 0x33);
    let tx_two = payment_tx(11, 0x44);
    let sink = Arc::new(RecordingSink::default());
    let mut adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_000),
            close_time: NetClockTimePoint::new(1_001),
        },
        TestLedgerSource::default(),
        TestOpenLedgerSource {
            txs: vec![Arc::clone(&tx_two), Arc::clone(&tx_one)],
        },
        TestValidationSource::default(),
        TestValidatorSource::default(),
        TestModeSource::new(NetworkOpsOperatingMode::Connected),
        Arc::new(MockLedgerAcceptor::default()),
        Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        )))),
        Arc::new(TransactionMaster::new()),
        Arc::clone(&sink),
        NullRclConsensusJournal,
        sample_validator_keys(),
        None,
        None,
    );

    let prev =
        consensus_ledger_from_ledger(&Ledger::from_ledger_seq_and_close_time(700, 1_200, false));
    let (txset, txset_id) = adaptor.make_txset(&prev);

    assert_eq!(txset.len(), 2);
    let actual_ids = txset.iter().map(|tx| tx.id).collect::<BTreeSet<_>>();
    let expected_ids = [tx_one.get_transaction_id(), tx_two.get_transaction_id()]
        .into_iter()
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_ids, expected_ids);

    adaptor.share_tx_set(&txset);
    assert_eq!(
        sink.shared_txset_ids
            .lock()
            .expect("shared txset ids")
            .as_slice(),
        &[txset_id]
    );

    adaptor.share_tx(&txset[0]);
    assert_eq!(
        sink.shared_tx_ids
            .lock()
            .expect("shared tx ids")
            .first()
            .copied(),
        Some(txset[0].id)
    );
}

#[test]
fn app_rcl_consensus_make_txset_injects_fee_vote_pseudotx_into_authoritative_shamap() {
    let mut previous = Ledger::from_ledger_seq_and_close_time(256, 1_200, false);
    previous.apply_default_fees(Fees {
        base: 10,
        reserve: 200_000,
        increment: 50_000,
    });
    let previous = Arc::new(previous);
    let previous_id = *previous.header().hash.as_uint256();
    let fee_setup = FeeSetup {
        reference_fee: protocol::XRPAmount::from_drops(42),
        account_reserve: protocol::XRPAmount::from_drops(1_234_567),
        owner_reserve: protocol::XRPAmount::from_drops(7_654_321),
    };
    let validations = vec![signed_validation(1, previous_id, 256, |validation| {
        validation.set_field_u64(
            get_field_by_symbol("sfBaseFee"),
            fee_setup
                .reference_fee
                .drops_as::<u64>()
                .expect("base fee fits u64"),
        );
        validation.set_field_u32(
            get_field_by_symbol("sfReserveBase"),
            fee_setup
                .account_reserve
                .drops_as::<u32>()
                .expect("reserve base fits u32"),
        );
        validation.set_field_u32(
            get_field_by_symbol("sfReserveIncrement"),
            fee_setup
                .owner_reserve
                .drops_as::<u32>()
                .expect("reserve increment fits u32"),
        );
    })];
    let sink = Arc::new(RecordingSink::default());
    let mut adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_000),
            close_time: NetClockTimePoint::new(1_001),
        },
        TestLedgerSource::default(),
        TestOpenLedgerSource::default(),
        TestValidationSource {
            parent_validations: HashMap::from([((previous_id, 256), validations)]),
            ..TestValidationSource::default()
        },
        TestValidatorSource::default(),
        TestModeSource::new(NetworkOpsOperatingMode::Connected),
        Arc::new(MockLedgerAcceptor::default()),
        Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        )))),
        Arc::new(TransactionMaster::new()),
        Arc::clone(&sink),
        NullRclConsensusJournal,
        sample_validator_keys(),
        None,
        None,
    );
    adaptor.set_fee_vote(fee_setup);

    let prev = adaptor.remember_ledger(previous);
    let (txset, _) = adaptor.make_txset(&prev);

    assert_eq!(txset.len(), 1);
    adaptor.share_tx(&txset[0]);

    let shared = sink
        .shared_transactions
        .lock()
        .expect("shared transactions")
        .clone();
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].get_txn_type(), TxType::FEE);
    assert_eq!(
        shared[0].get_field_u32(get_field_by_symbol("sfLedgerSequence")),
        257
    );
    assert_eq!(
        shared[0].get_field_u64(get_field_by_symbol("sfBaseFee")),
        fee_setup
            .reference_fee
            .drops_as::<u64>()
            .expect("base fee fits u64")
    );
}

#[test]
fn app_rcl_consensus_get_prev_ledger_uses_validations_and_records_view_change() {
    let ledger = Arc::new(Ledger::from_ledger_seq_and_close_time(801, 1_500, false));
    let preferred = Uint256::from_u64(999);
    let mut ledgers = HashMap::new();
    ledgers.insert(*ledger.header().hash.as_uint256(), Arc::clone(&ledger));
    let sink = Arc::new(RecordingSink::default());
    let mut adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_000),
            close_time: NetClockTimePoint::new(1_001),
        },
        TestLedgerSource {
            ledgers,
            valid_ledger_index: 801,
            have_validated: true,
        },
        TestOpenLedgerSource::default(),
        TestValidationSource {
            preferred: Some(preferred),
            json_trie: json!({"preferred": preferred.to_string()}),
            ..TestValidationSource::default()
        },
        TestValidatorSource::default(),
        TestModeSource::new(NetworkOpsOperatingMode::Connected),
        Arc::new(MockLedgerAcceptor::default()),
        Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        )))),
        Arc::new(TransactionMaster::new()),
        Arc::clone(&sink),
        RecordingJournal::default(),
        sample_validator_keys(),
        None,
        None,
    );

    let consensus_ledger = adaptor.remember_ledger(Arc::clone(&ledger));
    let selected = adaptor.get_prev_ledger(
        ledger.header().hash.as_uint256(),
        &consensus_ledger,
        ConsensusMode::Proposing,
    );

    assert_eq!(selected, preferred);
    assert_eq!(sink.view_changes.load(Ordering::Acquire), 1);
}

#[test]
fn app_rcl_consensus_end_consensus_uses_owner_closed_and_parent_ledgers() {
    let previous = Arc::new(Ledger::from_ledger_seq_and_close_time(900, 1_500, false));
    let mut built = Ledger::from_ledger_seq_and_close_time(901, 1_510, false);
    let mut built_header = built.header();
    built_header.hash = SHAMapHash::new(Uint256::from_u64(0xB001));
    built.set_ledger_info(built_header);
    let built = Arc::new(built);
    let mut owner_closed = Ledger::from_ledger_seq_and_close_time(902, 1_520, false);
    let mut owner_closed_header = owner_closed.header();
    owner_closed_header.hash = SHAMapHash::new(Uint256::from_u64(0xC105ED));
    owner_closed.set_ledger_info(owner_closed_header);
    let owner_closed = Arc::new(owner_closed);
    let mut owner_prev = Ledger::from_ledger_seq_and_close_time(902, 1_520, false);
    let mut owner_prev_header = owner_prev.header();
    owner_prev_header.hash = SHAMapHash::new(Uint256::from_u64(0xC0FFEE));
    owner_prev.set_ledger_info(owner_prev_header);
    let owner_prev = Arc::new(owner_prev);

    let adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_600),
            close_time: NetClockTimePoint::new(1_601),
        },
        TestLedgerSource::default(),
        TestOpenLedgerSource::default(),
        TestValidationSource::default(),
        TestValidatorSource::default(),
        TestModeSource::new(NetworkOpsOperatingMode::Connected),
        Arc::new(OwnerLedgerAcceptor {
            closed: Some(Arc::clone(&owner_closed)),
            previous: Some(Arc::clone(&owner_prev)),
        }),
        Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        )))),
        Arc::new(TransactionMaster::new()),
        Arc::new(RecordingSink::default()),
        RecordingJournal::default(),
        sample_validator_keys(),
        None,
        None,
    );

    adaptor.end_consensus(
        Some(&built),
        &consensus_ledger_from_ledger(previous.as_ref()),
    );

    let (queued_now, queued_network_closed, queued_prev) = adaptor
        .take_pending_start_round()
        .expect("endConsensus should queue a next round");
    assert_eq!(queued_now, NetClockTimePoint::new(1_601));
    assert_eq!(
        queued_network_closed,
        *owner_closed.header().hash.as_uint256()
    );
    assert_eq!(queued_prev.id, *owner_prev.header().hash.as_uint256());
    assert_eq!(queued_prev.seq, owner_prev.header().seq);
}

#[test]
fn app_rcl_consensus_end_consensus_does_not_promote_when_need_network_ledger_is_set() {
    let mut built = Ledger::from_ledger_seq_and_close_time(901, 1_510, false);
    let mut built_header = built.header();
    built_header.hash = SHAMapHash::new(Uint256::from_u64(0xB001));
    built.set_ledger_info(built_header);
    let built = Arc::new(built);

    let adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_600),
            close_time: NetClockTimePoint::new(1_601),
        },
        TestLedgerSource::default(),
        TestOpenLedgerSource::default(),
        TestValidationSource::default(),
        TestValidatorSource::default(),
        TestModeSource::new(NetworkOpsOperatingMode::Syncing).with_need_network_ledger(true),
        Arc::new(OwnerLedgerAcceptor {
            closed: Some(Arc::clone(&built)),
            previous: Some(Arc::clone(&built)),
        }),
        Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        )))),
        Arc::new(TransactionMaster::new()),
        Arc::new(RecordingSink::default()),
        RecordingJournal::default(),
        sample_validator_keys(),
        None,
        None,
    );

    adaptor.end_consensus(Some(&built), &consensus_ledger_from_ledger(built.as_ref()));

    assert_eq!(
        adaptor.mode_source.operating_mode(),
        NetworkOpsOperatingMode::Syncing
    );
}

#[test]
fn app_rcl_consensus_end_consensus_does_not_promote_on_owner_ledger_change() {
    let mut built = Ledger::from_ledger_seq_and_close_time(901, 1_510, false);
    let mut built_header = built.header();
    built_header.hash = SHAMapHash::new(Uint256::from_u64(0xB001));
    built.set_ledger_info(built_header);
    let built = Arc::new(built);

    let mut owner_closed = Ledger::from_ledger_seq_and_close_time(901, 1_510, false);
    let mut owner_closed_header = owner_closed.header();
    owner_closed_header.hash = SHAMapHash::new(Uint256::from_u64(0xC105ED));
    owner_closed.set_ledger_info(owner_closed_header);
    let owner_closed = Arc::new(owner_closed);

    let adaptor = AppRclConsensusAdaptor::new(
        AppRclConsensusOptions::default(),
        FixedClock {
            now: NetClockTimePoint::new(1_600),
            close_time: NetClockTimePoint::new(1_601),
        },
        TestLedgerSource::default(),
        TestOpenLedgerSource::default(),
        TestValidationSource::default(),
        TestValidatorSource::default(),
        TestModeSource::new(NetworkOpsOperatingMode::Connected),
        Arc::new(OwnerLedgerAcceptor {
            closed: Some(owner_closed),
            previous: Some(Arc::clone(&built)),
        }),
        Arc::new(Mutex::new(InboundTransactions::new(Arc::new(
            SimplePeerSetBuilder::new(Vec::new()),
        )))),
        Arc::new(TransactionMaster::new()),
        Arc::new(RecordingSink::default()),
        RecordingJournal::default(),
        sample_validator_keys(),
        None,
        None,
    );

    adaptor.end_consensus(Some(&built), &consensus_ledger_from_ledger(built.as_ref()));

    assert_eq!(
        adaptor.mode_source.operating_mode(),
        NetworkOpsOperatingMode::Connected
    );
}
