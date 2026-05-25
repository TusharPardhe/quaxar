use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use basics::slice::Slice;
use consensus::{
    Consensus, ConsensusAdaptor, ConsensusDecision, ConsensusParms, ConsensusPeerPosition,
    ConsensusProposal, ConsensusResult, ConsensusState, RclCxPeerPos, RclValidatedLedger,
    RclValidation, RclValidations, RclValidationsAdapter, ValidationStatus, proposal_unique_id,
};
use protocol::{KeyType, PublicKey, SecretKey, derive_public_key, sign};
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct MockTx {
    id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MockLedger {
    id: u32,
    seq: u32,
    close_time_resolution: time::Duration,
    close_agree: bool,
    close_time: NetClockTimePoint,
    parent_close_time: NetClockTimePoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MockPeerPosition {
    proposal: ConsensusProposal<u32, u32, u32>,
}

impl ConsensusPeerPosition<u32, u32, u32> for MockPeerPosition {
    fn proposal(&self) -> &ConsensusProposal<u32, u32, u32> {
        &self.proposal
    }
}

#[derive(Clone)]
struct MockAdaptor {
    now: NetClockTimePoint,
    open_transactions: bool,
    prev_round_time: Duration,
    txsets: HashMap<u32, Vec<MockTx>>,
    proposed: Vec<u32>,
    shared_sets: Vec<u32>,
    shared: Vec<u32>,
    accepted_states: Vec<ConsensusState>,
}

impl MockAdaptor {
    fn new() -> Self {
        let txsets = HashMap::from([
            (100, vec![MockTx { id: 1 }]),
            (200, vec![MockTx { id: 1 }, MockTx { id: 2 }]),
        ]);
        Self {
            now: NetClockTimePoint::new(100),
            open_transactions: true,
            prev_round_time: Duration::from_secs(10),
            txsets,
            proposed: Vec::new(),
            shared_sets: Vec::new(),
            shared: Vec::new(),
            accepted_states: Vec::new(),
        }
    }
}

impl ConsensusAdaptor for MockAdaptor {
    type LedgerId = u32;
    type Ledger = MockLedger;
    type NodeId = u32;
    type TxSetId = u32;
    type TxSet = Vec<MockTx>;
    type Tx = MockTx;
    type TxId = u32;
    type PeerPosition = MockPeerPosition;

    fn now(&self) -> NetClockTimePoint {
        self.now
    }
    fn acquire_ledger(&mut self, _ledger_id: &Self::LedgerId) -> Option<Self::Ledger> {
        None
    }
    fn acquire_tx_set(&mut self, txset_id: &Self::TxSetId) -> Option<Self::TxSet> {
        self.txsets.get(txset_id).cloned()
    }
    fn has_open_transactions(&self) -> bool {
        self.open_transactions
    }
    fn proposers_validated(&self, _prev_ledger: &Self::LedgerId) -> usize {
        2
    }
    fn proposers_finished(
        &self,
        _prev_ledger: &Self::Ledger,
        _prev_ledger_id: &Self::LedgerId,
    ) -> usize {
        0
    }
    fn should_propose(&self) -> bool {
        true
    }
    fn prev_round_time(&self) -> Duration {
        self.prev_round_time
    }
    fn now_close_time(&self) -> NetClockTimePoint {
        self.now
    }
    fn get_prev_ledger(
        &mut self,
        prev_ledger_id: &Self::LedgerId,
        _prev_ledger: &Self::Ledger,
        _mode: consensus::ConsensusMode,
    ) -> Self::LedgerId {
        *prev_ledger_id
    }
    fn on_mode_change(
        &mut self,
        _before: consensus::ConsensusMode,
        _after: consensus::ConsensusMode,
    ) {
    }
    fn close_time_resolution(&self, ledger: &Self::Ledger) -> time::Duration {
        ledger.close_time_resolution
    }
    fn close_agree(&self, ledger: &Self::Ledger) -> bool {
        ledger.close_agree
    }
    fn close_time(&self, ledger: &Self::Ledger) -> NetClockTimePoint {
        ledger.close_time
    }
    fn parent_close_time(&self, ledger: &Self::Ledger) -> NetClockTimePoint {
        ledger.parent_close_time
    }
    fn seq(&self, ledger: &Self::Ledger) -> u32 {
        ledger.seq
    }
    fn id(&self, ledger: &Self::Ledger) -> Self::LedgerId {
        ledger.id
    }
    fn on_accept(
        &mut self,
        result: &ConsensusResult<
            Self::LedgerId,
            Self::NodeId,
            Self::TxSet,
            Self::TxSetId,
            Self::Tx,
            Self::TxId,
        >,
        _prev_ledger: &Self::Ledger,
    ) {
        self.accepted_states.push(result.state);
    }
    fn make_txset(&mut self, _previous_ledger: &Self::Ledger) -> (Self::TxSet, Self::TxSetId) {
        (self.txsets.get(&100).cloned().expect("known txset"), 100)
    }
    fn propose(
        &mut self,
        proposal: &ConsensusProposal<Self::NodeId, Self::LedgerId, Self::TxSetId>,
    ) {
        self.proposed.push(*proposal.position());
    }
    fn share_peer_position(&mut self, _peer_position: &Self::PeerPosition) {}
    fn share_tx_set(&mut self, txset: &Self::TxSet) {
        self.shared_sets.push(self.txset_id(txset));
    }
    fn share_tx(&mut self, tx: &Self::Tx) {
        self.shared.push(tx.id);
    }
    fn node_id(&self) -> Self::NodeId {
        7
    }
    fn txset_id(&self, txset: &Self::TxSet) -> Self::TxSetId {
        if txset.len() == 1 { 100 } else { 200 }
    }
    fn tx_id(&self, tx: &Self::Tx) -> Self::TxId {
        tx.id
    }
    fn txset_find(&self, txset: &Self::TxSet, txid: &Self::TxId) -> Option<Self::Tx> {
        txset.iter().find(|tx| tx.id == *txid).cloned()
    }
    fn txset_exists(&self, txset: &Self::TxSet, txid: &Self::TxId) -> bool {
        txset.iter().any(|tx| tx.id == *txid)
    }
    fn txset_compare(&self, ours: &Self::TxSet, other: &Self::TxSet) -> Vec<(Self::TxId, bool)> {
        let mut out = Vec::new();
        for tx in ours {
            if !other.iter().any(|candidate| candidate.id == tx.id) {
                out.push((tx.id, true));
            }
        }
        for tx in other {
            if !ours.iter().any(|candidate| candidate.id == tx.id) {
                out.push((tx.id, false));
            }
        }
        out
    }
    fn txset_insert(&self, txset: &mut Self::TxSet, tx: Self::Tx) {
        if !txset.iter().any(|candidate| candidate.id == tx.id) {
            txset.push(tx);
            txset.sort_by_key(|candidate| candidate.id);
        }
    }
    fn txset_erase(&self, txset: &mut Self::TxSet, txid: &Self::TxId) {
        txset.retain(|tx| tx.id != *txid);
    }
}

#[test]
fn consensus_creates_disputes_updates_position_and_accepts() {
    let mut consensus = Consensus::new(MockAdaptor::new(), ConsensusParms::default());
    let prev_ledger = MockLedger {
        id: 1,
        seq: 10,
        close_time_resolution: time::Duration::seconds(30),
        close_agree: true,
        close_time: NetClockTimePoint::new(90),
        parent_close_time: NetClockTimePoint::new(60),
    };

    consensus.start_round(NetClockTimePoint::new(100), 1, prev_ledger, true);
    assert!(consensus.peer_proposal(
        NetClockTimePoint::new(100),
        MockPeerPosition {
            proposal: ConsensusProposal::new(
                1,
                ConsensusProposal::<u32, u32, u32>::SEQ_JOIN,
                200,
                NetClockTimePoint::new(100),
                NetClockTimePoint::new(100),
                9,
            ),
        },
    ));
    assert!(consensus.peer_proposal(
        NetClockTimePoint::new(100),
        MockPeerPosition {
            proposal: ConsensusProposal::new(
                1,
                ConsensusProposal::<u32, u32, u32>::SEQ_JOIN,
                200,
                NetClockTimePoint::new(100),
                NetClockTimePoint::new(100),
                10,
            ),
        },
    ));

    assert_eq!(
        consensus.timer_entry(NetClockTimePoint::new(101)),
        ConsensusDecision::CloseLedger
    );

    let result = consensus.result().expect("consensus result should exist");
    assert_eq!(result.disputes.len(), 1);
    assert!(result.disputes.contains_key(&2));

    assert_eq!(
        consensus.timer_entry(NetClockTimePoint::new(103)),
        ConsensusDecision::StayOpen
    );
    let accepted = consensus.timer_entry(NetClockTimePoint::new(104));
    assert_eq!(accepted, ConsensusDecision::Accepted(ConsensusState::Yes));

    let result = consensus.result().expect("accepted result should exist");
    assert_eq!(*result.position.position(), 200);
    assert_eq!(result.txns.len(), 2);
}

#[test]
fn rcl_peer_position_renders_base58_and_verifies_signature() {
    let secret = SecretKey::from_bytes([7u8; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("valid public key");
    let proposal = ConsensusProposal::new(
        Uint256::from_u64(1),
        ConsensusProposal::<PublicKey, Uint256, Uint256>::SEQ_JOIN,
        Uint256::from_u64(2),
        NetClockTimePoint::new(90),
        NetClockTimePoint::new(90),
        public,
    );
    let signing_bytes = {
        let mut serializer = protocol::Serializer::new(128);
        serializer.add32(protocol::HashPrefix::Proposal as u32);
        serializer.add32(proposal.propose_seq());
        serializer.add32(proposal.close_time().as_seconds());
        serializer.add_bit_string(*proposal.prev_ledger());
        serializer.add_bit_string(*proposal.position());
        serializer.data().to_vec()
    };
    let signature = sign(&public, &secret, &signing_bytes).expect("signature");
    let peer = RclCxPeerPos::new(public, &signature, Uint256::from_u64(3), proposal.clone());

    assert!(peer.check_sign());
    assert_eq!(
        peer.get_json()["peer_id"].as_str(),
        Some(public.to_node_public_base58().as_str())
    );

    let unique = proposal_unique_id(
        *proposal.position(),
        *proposal.prev_ledger(),
        proposal.propose_seq(),
        proposal.close_time(),
        Slice::new(public.as_ref()),
        Slice::new(&signature),
    );
    assert_ne!(unique, Uint256::default());
}

#[derive(Debug, Clone)]
struct MockValidationsAdapter {
    now: NetClockTimePoint,
    ledgers: HashMap<Uint256, RclValidatedLedger>,
}

impl RclValidationsAdapter for MockValidationsAdapter {
    fn now(&self) -> NetClockTimePoint {
        self.now
    }

    fn acquire(&mut self, ledger_id: &Uint256) -> Option<RclValidatedLedger> {
        self.ledgers.get(ledger_id).cloned()
    }
}

#[test]
fn rcl_validations_tracks_current_trusted_and_acquisition() {
    let public_a = PublicKey::from_bytes([0x02; 33]);
    let public_b = PublicKey::from_bytes([0x03; 33]);
    let ledger_id = Uint256::from_u64(55);
    let mut validations = RclValidations::new(
        MockValidationsAdapter {
            now: NetClockTimePoint::new(200),
            ledgers: HashMap::from([(
                ledger_id,
                RclValidatedLedger {
                    ledger_id,
                    ledger_seq: 22,
                    ancestors: vec![Uint256::from_u64(11)],
                },
            )]),
        },
        ConsensusParms::default(),
    );

    let current = RclValidation {
        ledger_id,
        seq: 22,
        sign_time: NetClockTimePoint::new(199),
        seen_time: NetClockTimePoint::new(199),
        key: public_a,
        trusted: true,
        full: true,
        load_fee: Some(10),
        cookie: 1,
    };
    assert_eq!(
        validations.add(public_a, current),
        ValidationStatus::Current
    );

    let stale = RclValidation {
        ledger_id,
        seq: 20,
        sign_time: NetClockTimePoint::new(1),
        seen_time: NetClockTimePoint::new(1),
        key: public_b,
        trusted: true,
        full: true,
        load_fee: None,
        cookie: 2,
    };
    assert_eq!(validations.add(public_b, stale), ValidationStatus::Stale);

    let partial = RclValidation {
        ledger_id,
        seq: 23,
        sign_time: NetClockTimePoint::new(199),
        seen_time: NetClockTimePoint::new(199),
        key: public_b,
        trusted: true,
        full: false,
        load_fee: None,
        cookie: 3,
    };
    assert_eq!(
        validations.add(public_b, partial),
        ValidationStatus::Current
    );

    let bad_seq = RclValidation {
        ledger_id,
        seq: 21,
        sign_time: NetClockTimePoint::new(199),
        seen_time: NetClockTimePoint::new(199),
        key: public_a,
        trusted: true,
        full: true,
        load_fee: Some(10),
        cookie: 4,
    };
    assert_eq!(validations.add(public_a, bad_seq), ValidationStatus::BadSeq);

    assert_eq!(validations.num_trusted_for_ledger(ledger_id), 1);
    assert_eq!(validations.current_trusted().len(), 1);
    let mut trusted_keys = BTreeSet::from([public_a]);
    assert_eq!(validations.laggards(22, &mut trusted_keys), 0);
    assert!(trusted_keys.is_empty());
    assert!(validations.can_validate_seq(23));
    assert!(!validations.can_validate_seq(23));
    assert_eq!(
        validations
            .last_ledger(public_a)
            .expect("trusted ledger should be acquired")
            .ledger_seq,
        22
    );
}

#[test]
fn rcl_validations_seq_enforcer_expires_from_adapter_time() {
    let public = PublicKey::from_bytes([0x04; 33]);
    let mut validations = RclValidations::new(
        MockValidationsAdapter {
            now: NetClockTimePoint::new(100),
            ledgers: HashMap::new(),
        },
        ConsensusParms {
            validation_set_expires: Duration::from_secs(1),
            ..ConsensusParms::default()
        },
    );

    let current = RclValidation {
        ledger_id: Uint256::from_u64(77),
        seq: 5,
        sign_time: NetClockTimePoint::new(99),
        seen_time: NetClockTimePoint::new(99),
        key: public,
        trusted: false,
        full: false,
        load_fee: None,
        cookie: 8,
    };
    assert_eq!(validations.add(public, current), ValidationStatus::Current);
    assert!(validations.can_validate_seq(23));
    assert!(!validations.can_validate_seq(23));

    validations.adaptor_mut().now = NetClockTimePoint::new(102);
    assert!(validations.can_validate_seq(23));
}
