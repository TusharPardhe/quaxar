use basics::chrono::NetClockTimePoint;
use consensus::{
    Consensus, ConsensusAdaptor, ConsensusDecision, ConsensusMode, ConsensusParms,
    ConsensusPeerPosition, ConsensusProposal, ConsensusResult,
};
use std::collections::HashMap;
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
    txsets: HashMap<u32, Vec<MockTx>>,
    ledgers: HashMap<u32, MockLedger>,
    ledger_available: bool,
    shared_positions: Vec<u32>,
    shared_sets: Vec<u32>,
}

impl MockAdaptor {
    fn new() -> Self {
        Self {
            txsets: HashMap::from([
                (100, vec![MockTx { id: 1 }]),
                (200, vec![MockTx { id: 1 }, MockTx { id: 2 }]),
            ]),
            ledgers: HashMap::from([(
                99,
                MockLedger {
                    id: 99,
                    seq: 10,
                    close_time_resolution: time::Duration::seconds(30),
                    close_agree: true,
                    close_time: NetClockTimePoint::new(90),
                    parent_close_time: NetClockTimePoint::new(60),
                },
            )]),
            ledger_available: false,
            shared_positions: Vec::new(),
            shared_sets: Vec::new(),
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
        NetClockTimePoint::new(100)
    }
    fn acquire_ledger(&mut self, ledger_id: &Self::LedgerId) -> Option<Self::Ledger> {
        self.ledger_available
            .then(|| self.ledgers.get(ledger_id).cloned())
            .flatten()
    }
    fn acquire_tx_set(&mut self, txset_id: &Self::TxSetId) -> Option<Self::TxSet> {
        self.txsets.get(txset_id).cloned()
    }
    fn has_open_transactions(&self) -> bool {
        true
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
        Duration::from_secs(10)
    }
    fn now_close_time(&self) -> NetClockTimePoint {
        NetClockTimePoint::new(100)
    }
    fn get_prev_ledger(
        &mut self,
        prev_ledger_id: &Self::LedgerId,
        _prev_ledger: &Self::Ledger,
        _mode: ConsensusMode,
    ) -> Self::LedgerId {
        *prev_ledger_id
    }
    fn on_mode_change(&mut self, _before: ConsensusMode, _after: ConsensusMode) {}
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
        _result: &ConsensusResult<
            Self::LedgerId,
            Self::NodeId,
            Self::TxSet,
            Self::TxSetId,
            Self::Tx,
            Self::TxId,
        >,
        _prev_ledger: &Self::Ledger,
    ) {
    }
    fn make_txset(&mut self, _previous_ledger: &Self::Ledger) -> (Self::TxSet, Self::TxSetId) {
        (self.txsets.get(&100).cloned().expect("known txset"), 100)
    }
    fn propose(
        &mut self,
        _proposal: &ConsensusProposal<Self::NodeId, Self::LedgerId, Self::TxSetId>,
    ) {
    }
    fn share_peer_position(&mut self, peer_position: &Self::PeerPosition) {
        self.shared_positions
            .push(*peer_position.proposal.position());
    }
    fn share_tx_set(&mut self, txset: &Self::TxSet) {
        self.shared_sets.push(self.txset_id(txset));
    }
    fn share_tx(&mut self, _tx: &Self::Tx) {}
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
fn wrong_ledger_recovery_replays_recent_proposals_after_switch() {
    let mut consensus = Consensus::new(MockAdaptor::new(), ConsensusParms::default());
    let wrong_ledger = MockLedger {
        id: 1,
        seq: 10,
        close_time_resolution: time::Duration::seconds(30),
        close_agree: true,
        close_time: NetClockTimePoint::new(90),
        parent_close_time: NetClockTimePoint::new(60),
    };

    consensus.start_round(NetClockTimePoint::new(100), 99, wrong_ledger, true);
    assert_eq!(consensus.mode(), ConsensusMode::WrongLedger);

    assert!(consensus.peer_proposal(
        NetClockTimePoint::new(100),
        MockPeerPosition {
            proposal: ConsensusProposal::new(
                99,
                ConsensusProposal::<u32, u32, u32>::SEQ_JOIN,
                200,
                NetClockTimePoint::new(100),
                NetClockTimePoint::new(100),
                9,
            ),
        },
    ));

    consensus.adaptor_mut().ledger_available = true;

    assert_eq!(
        consensus.timer_entry(NetClockTimePoint::new(101)),
        ConsensusDecision::CloseLedger
    );
    assert_eq!(consensus.mode(), ConsensusMode::SwitchedLedger);
    assert_eq!(consensus.adaptor().shared_positions, vec![200]);

    let result = consensus.result().expect("result should exist after close");
    assert_eq!(result.disputes.len(), 1);
    assert!(result.disputes.contains_key(&2));
}
