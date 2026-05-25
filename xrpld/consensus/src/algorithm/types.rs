use crate::disputed_tx::DisputedTx;
use crate::proposal::ConsensusProposal;
use basics::chrono::NetClockTimePoint;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::Hash;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusMode {
    Proposing,
    Observing,
    WrongLedger,
    SwitchedLedger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusPhase {
    Open,
    Establish,
    Accepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusState {
    No,
    MovedOn,
    Expired,
    Yes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConsensusTimer {
    dur: Duration,
}

impl ConsensusTimer {
    pub fn read(&self) -> Duration {
        self.dur
    }

    pub fn tick_fixed(&mut self, fixed: Duration) {
        self.dur += fixed;
    }

    pub fn reset(&mut self) {
        self.dur = Duration::ZERO;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConsensusCloseTimes {
    pub peers: BTreeMap<NetClockTimePoint, i32>,
    pub self_close_time: NetClockTimePoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusResult<LedgerId, NodeId, TxSet, TxSetId, Tx, TxId>
where
    TxSetId: Eq + Hash,
    TxId: Eq + Hash + Ord,
    NodeId: Ord,
{
    pub txns: TxSet,
    pub position: ConsensusProposal<NodeId, LedgerId, TxSetId>,
    pub disputes: HashMap<TxId, DisputedTx<Tx, TxId, NodeId>>,
    pub compares: HashSet<TxSetId>,
    pub round_time: ConsensusTimer,
    pub state: ConsensusState,
    pub proposers: usize,
}

impl<LedgerId, NodeId, TxSet, TxSetId, Tx, TxId>
    ConsensusResult<LedgerId, NodeId, TxSet, TxSetId, Tx, TxId>
where
    TxSet: Clone,
    TxSetId: Eq + Hash + Clone,
    TxId: Eq + Hash + Ord,
    NodeId: Ord,
{
    pub fn new(txns: TxSet, position: ConsensusProposal<NodeId, LedgerId, TxSetId>) -> Self {
        Self {
            txns,
            position,
            disputes: HashMap::new(),
            compares: HashSet::new(),
            round_time: ConsensusTimer::default(),
            state: ConsensusState::No,
            proposers: 0,
        }
    }
}

pub(crate) type PeerPositions<NodeId, PeerPosition> = HashMap<NodeId, PeerPosition>;
