use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use consensus::{
    ConsensusDecision, ConsensusParms, ConsensusProposal, ConsensusResult, ConsensusState,
    RclConsensus, RclConsensusAdapter, RclCxLedger, RclCxPeerPos, RclCxTx, RclRoundTimer,
};
use protocol::PublicKey;
use std::collections::HashMap;
use std::time::Duration;
use tokio::runtime::Builder;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MockRclAdaptor {
    now: NetClockTimePoint,
    open_transactions: bool,
    prev_round_time: Duration,
    txsets: HashMap<Uint256, Vec<RclCxTx>>,
    proposed: Vec<Uint256>,
    shared_sets: Vec<Uint256>,
    accepted_states: Vec<ConsensusState>,
}

impl MockRclAdaptor {
    fn new() -> Self {
        Self {
            now: NetClockTimePoint::new(100),
            open_transactions: true,
            prev_round_time: Duration::from_secs(10),
            txsets: HashMap::from([
                (
                    Uint256::from_u64(100),
                    vec![RclCxTx {
                        id: Uint256::from_u64(1),
                    }],
                ),
                (
                    Uint256::from_u64(200),
                    vec![
                        RclCxTx {
                            id: Uint256::from_u64(1),
                        },
                        RclCxTx {
                            id: Uint256::from_u64(2),
                        },
                    ],
                ),
            ]),
            proposed: Vec::new(),
            shared_sets: Vec::new(),
            accepted_states: Vec::new(),
        }
    }
}

impl RclConsensusAdapter for MockRclAdaptor {
    fn now(&self) -> NetClockTimePoint {
        self.now
    }

    fn acquire_ledger(&mut self, _ledger_id: &Uint256) -> Option<RclCxLedger> {
        None
    }

    fn acquire_tx_set(&mut self, txset_id: &Uint256) -> Option<Vec<RclCxTx>> {
        self.txsets.get(txset_id).cloned()
    }

    fn has_open_transactions(&self) -> bool {
        self.open_transactions
    }

    fn proposers_validated(&self, _prev_ledger: &Uint256) -> usize {
        2
    }

    fn proposers_finished(&self, _prev_ledger: &RclCxLedger, _prev_ledger_id: &Uint256) -> usize {
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
        prev_ledger_id: &Uint256,
        _prev_ledger: &RclCxLedger,
        _mode: consensus::ConsensusMode,
    ) -> Uint256 {
        *prev_ledger_id
    }

    fn on_mode_change(
        &mut self,
        _before: consensus::ConsensusMode,
        _after: consensus::ConsensusMode,
    ) {
    }

    fn on_accept(
        &mut self,
        result: &ConsensusResult<Uint256, PublicKey, Vec<RclCxTx>, Uint256, RclCxTx, Uint256>,
        _prev_ledger: &RclCxLedger,
    ) {
        self.accepted_states.push(result.state);
    }

    fn make_txset(&mut self, _previous_ledger: &RclCxLedger) -> (Vec<RclCxTx>, Uint256) {
        (
            self.txsets
                .get(&Uint256::from_u64(100))
                .cloned()
                .expect("known txset"),
            Uint256::from_u64(100),
        )
    }

    fn propose(&mut self, proposal: &ConsensusProposal<PublicKey, Uint256, Uint256>) {
        self.proposed.push(*proposal.position());
    }

    fn share_peer_position(&mut self, _peer_position: &RclCxPeerPos) {}

    fn share_tx_set(&mut self, txset: &[RclCxTx]) {
        self.shared_sets.push(if txset.len() == 1 {
            Uint256::from_u64(100)
        } else {
            Uint256::from_u64(200)
        });
    }

    fn share_tx(&mut self, _tx: &RclCxTx) {}

    fn node_id(&self) -> PublicKey {
        PublicKey::from_bytes([0x07; 33])
    }
}

#[test]
fn rcl_consensus_uses_an_injected_round_timer() {
    let runtime = Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("tokio runtime");

    runtime.block_on(async {
        let timer = RclRoundTimer::new_at(Duration::from_millis(1), std::time::Instant::now());
        assert_eq!(timer.period(), Duration::from_millis(1));

        let mut consensus =
            RclConsensus::with_round_timer(MockRclAdaptor::new(), ConsensusParms::default(), timer);
        let prev_ledger = RclCxLedger {
            id: Uint256::from_u64(1),
            seq: 10,
            parent_id: Uint256::from_u64(0),
            close_time_resolution: time::Duration::seconds(30),
            close_agree: true,
            close_time: NetClockTimePoint::new(90),
            parent_close_time: NetClockTimePoint::new(60),
            base_fee_req: 10,
        };

        consensus.start_round(
            NetClockTimePoint::new(100),
            Uint256::from_u64(1),
            prev_ledger,
        );

        let signing_public = PublicKey::from_bytes([0x02; 33]);
        let proposal = ConsensusProposal::new(
            Uint256::from_u64(1),
            ConsensusProposal::<PublicKey, Uint256, Uint256>::SEQ_JOIN,
            Uint256::from_u64(200),
            NetClockTimePoint::new(100),
            NetClockTimePoint::new(100),
            signing_public,
        );
        assert!(consensus.peer_proposal(
            NetClockTimePoint::new(100),
            RclCxPeerPos::new(signing_public, [1u8; 65], Uint256::from_u64(9), proposal),
        ));

        let second_public = PublicKey::from_bytes([0x03; 33]);
        let second_proposal = ConsensusProposal::new(
            Uint256::from_u64(1),
            ConsensusProposal::<PublicKey, Uint256, Uint256>::SEQ_JOIN,
            Uint256::from_u64(200),
            NetClockTimePoint::new(100),
            NetClockTimePoint::new(100),
            second_public,
        );
        assert!(consensus.peer_proposal(
            NetClockTimePoint::new(100),
            RclCxPeerPos::new(
                second_public,
                [2u8; 65],
                Uint256::from_u64(10),
                second_proposal
            ),
        ));

        assert_eq!(
            consensus.timer_tick(NetClockTimePoint::new(101)).await,
            ConsensusDecision::CloseLedger
        );
        assert!(consensus.result().is_some());
    });
}
