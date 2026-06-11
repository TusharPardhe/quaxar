use crate::params::{AvalancheState, ConsensusParms, get_needed_weight};
use crate::proposal::ConsensusProposal;
use crate::timing::{get_next_ledger_time_resolution, round_close_time};
use crate::types::{
    ConsensusCloseTimes, ConsensusMode, ConsensusPhase, ConsensusResult, ConsensusState,
    ConsensusTimer, PeerPositions,
};
use basics::chrono::NetClockTimePoint;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::time::Duration;
use time::Duration as TimeDuration;
use tracing;

pub trait ConsensusPeerPosition<NodeId, LedgerId, TxSetId>: Clone {
    fn proposal(&self) -> &ConsensusProposal<NodeId, LedgerId, TxSetId>;
}

pub trait ConsensusAdaptor {
    type LedgerId: Clone + Eq + Hash + ToString;
    type Ledger: Clone;
    type NodeId: Clone + Eq + Hash + Ord + ToString;
    type TxSetId: Clone + Eq + Hash + ToString;
    type TxSet: Clone;
    type Tx: Clone;
    type TxId: Clone + Eq + Hash + Ord + ToString;
    type PeerPosition: ConsensusPeerPosition<Self::NodeId, Self::LedgerId, Self::TxSetId>;

    fn now(&self) -> NetClockTimePoint;
    fn acquire_ledger(&mut self, ledger_id: &Self::LedgerId) -> Option<Self::Ledger>;
    fn acquire_tx_set(&mut self, txset_id: &Self::TxSetId) -> Option<Self::TxSet>;
    fn has_open_transactions(&self) -> bool;
    fn proposers_validated(&self, prev_ledger: &Self::LedgerId) -> usize;
    fn proposers_finished(
        &self,
        prev_ledger: &Self::Ledger,
        prev_ledger_id: &Self::LedgerId,
    ) -> usize;
    fn should_propose(&self) -> bool;
    fn prev_round_time(&self) -> Duration;
    fn now_close_time(&self) -> NetClockTimePoint;
    fn get_prev_ledger(
        &mut self,
        prev_ledger_id: &Self::LedgerId,
        prev_ledger: &Self::Ledger,
        mode: ConsensusMode,
    ) -> Self::LedgerId;
    fn on_mode_change(&mut self, before: ConsensusMode, after: ConsensusMode);
    fn close_time_resolution(&self, ledger: &Self::Ledger) -> TimeDuration;
    fn close_agree(&self, ledger: &Self::Ledger) -> bool;
    fn close_time(&self, ledger: &Self::Ledger) -> NetClockTimePoint;
    fn parent_close_time(&self, ledger: &Self::Ledger) -> NetClockTimePoint;
    fn seq(&self, ledger: &Self::Ledger) -> u32;
    fn id(&self, ledger: &Self::Ledger) -> Self::LedgerId;
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
        prev_ledger: &Self::Ledger,
    );
    fn make_txset(&mut self, previous_ledger: &Self::Ledger) -> (Self::TxSet, Self::TxSetId);
    fn propose(
        &mut self,
        proposal: &ConsensusProposal<Self::NodeId, Self::LedgerId, Self::TxSetId>,
    );
    fn share_peer_position(&mut self, peer_position: &Self::PeerPosition);
    fn share_tx_set(&mut self, txset: &Self::TxSet);
    fn share_tx(&mut self, tx: &Self::Tx);
    fn node_id(&self) -> Self::NodeId;
    fn txset_id(&self, txset: &Self::TxSet) -> Self::TxSetId;
    fn tx_id(&self, tx: &Self::Tx) -> Self::TxId;
    fn txset_find(&self, txset: &Self::TxSet, txid: &Self::TxId) -> Option<Self::Tx>;
    fn txset_exists(&self, txset: &Self::TxSet, txid: &Self::TxId) -> bool;
    fn txset_compare(&self, ours: &Self::TxSet, other: &Self::TxSet) -> Vec<(Self::TxId, bool)>;
    fn txset_insert(&self, txset: &mut Self::TxSet, tx: Self::Tx);
    fn txset_erase(&self, txset: &mut Self::TxSet, txid: &Self::TxId);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusDecision {
    StayOpen,
    CloseLedger,
    Accepted(ConsensusState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsensusEvent<LedgerId> {
    WrongLedger { expected: LedgerId },
    SwitchedLedger,
    Accepted,
}

type ResultState<A> = ConsensusResult<
    <A as ConsensusAdaptor>::LedgerId,
    <A as ConsensusAdaptor>::NodeId,
    <A as ConsensusAdaptor>::TxSet,
    <A as ConsensusAdaptor>::TxSetId,
    <A as ConsensusAdaptor>::Tx,
    <A as ConsensusAdaptor>::TxId,
>;

#[allow(clippy::too_many_arguments)]
pub fn should_close_ledger(
    any_transactions: bool,
    prev_proposers: usize,
    proposers_closed: usize,
    proposers_validated: usize,
    prev_round_time: Duration,
    time_since_prev_close: Duration,
    open_time: Duration,
    idle_interval: Duration,
    parms: &ConsensusParms,
) -> bool {
    let ten_minutes = Duration::from_secs(10 * 60);
    if prev_round_time > ten_minutes || time_since_prev_close > ten_minutes {
        tracing::debug!(target: "consensus", "Closing ledger: exceeded 10 minute threshold");
        return true;
    }
    if proposers_closed + proposers_validated > (prev_proposers / 2) {
        tracing::debug!(target: "consensus", proposers_closed, proposers_validated, prev_proposers, "Closing ledger: majority of proposers closed");
        return true;
    }
    if !any_transactions {
        return time_since_prev_close >= idle_interval;
    }
    if open_time < parms.ledger_min_close {
        return false;
    }
    if open_time < prev_round_time / 2 {
        return false;
    }
    true
}

fn check_consensus_reached(
    mut agreeing: usize,
    mut total: usize,
    count_self: bool,
    min_consensus_pct: usize,
    reached_max: bool,
    stalled: bool,
) -> bool {
    if total == 0 {
        return reached_max;
    }
    if stalled {
        return true;
    }
    if count_self {
        agreeing += 1;
        total += 1;
    }
    ((agreeing * 100) / total) >= min_consensus_pct
}

#[allow(clippy::too_many_arguments)]
pub fn check_consensus(
    prev_proposers: usize,
    current_proposers: usize,
    current_agree: usize,
    current_finished: usize,
    previous_agree_time: Duration,
    current_agree_time: Duration,
    stalled: bool,
    parms: &ConsensusParms,
    proposing: bool,
) -> ConsensusState {
    if current_agree_time <= parms.ledger_min_consensus {
        return ConsensusState::No;
    }
    if current_proposers < (prev_proposers * 3 / 4)
        && current_agree_time < (previous_agree_time + parms.ledger_min_consensus)
    {
        tracing::debug!(target: "consensus", current_proposers, prev_proposers, "Waiting for more proposers");
        return ConsensusState::No;
    }
    if check_consensus_reached(
        current_agree,
        current_proposers,
        proposing,
        parms.min_consensus_pct,
        current_agree_time > parms.ledger_max_consensus,
        stalled,
    ) {
        return ConsensusState::Yes;
    }
    if check_consensus_reached(
        current_finished,
        current_proposers,
        false,
        parms.min_consensus_pct,
        current_agree_time > parms.ledger_max_consensus,
        false,
    ) {
        return ConsensusState::MovedOn;
    }
    let max_agree_time = duration_to_millis(previous_agree_time).saturating_mul(
        u32::try_from(parms.ledger_abandon_consensus_factor)
            .expect("abandon consensus factor must fit in u32"),
    );
    let expiry = max_agree_time.clamp(parms.ledger_max_consensus, parms.ledger_abandon_consensus);
    if current_agree_time > expiry {
        tracing::warn!(target: "consensus", elapsed_ms = current_agree_time.as_millis() as u64, "Consensus expired");
        return ConsensusState::Expired;
    }
    ConsensusState::No
}

pub struct Consensus<A: ConsensusAdaptor> {
    adaptor: A,
    parms: ConsensusParms,
    phase: ConsensusPhase,
    mode: ConsensusMode,
    first_round: bool,
    have_close_time_consensus: bool,
    converge_percent: i32,
    open_time: ConsensusTimer,
    close_resolution: TimeDuration,
    prev_round_time: Duration,
    now: NetClockTimePoint,
    prev_close_time: NetClockTimePoint,
    prev_ledger_id: Option<A::LedgerId>,
    previous_ledger: Option<A::Ledger>,
    result: Option<ResultState<A>>,
    raw_close_times: ConsensusCloseTimes,
    curr_peer_positions: PeerPositions<A::NodeId, A::PeerPosition>,
    recent_peer_positions: HashMap<A::NodeId, VecDeque<A::PeerPosition>>,
    prev_proposers: usize,
    acquired: HashMap<A::TxSetId, A::TxSet>,
    dead_nodes: HashSet<A::NodeId>,
    peer_unchanged_counter: usize,
    establish_counter: usize,
    close_time_avalanche_state: AvalancheState,
}

impl<A: ConsensusAdaptor> Consensus<A> {
    pub fn new(adaptor: A, parms: ConsensusParms) -> Self {
        let now = adaptor.now();
        Self {
            adaptor,
            parms,
            phase: ConsensusPhase::Accepted,
            mode: ConsensusMode::Observing,
            first_round: true,
            have_close_time_consensus: false,
            converge_percent: 0,
            open_time: ConsensusTimer::default(),
            close_resolution: crate::timing::LEDGER_DEFAULT_TIME_RESOLUTION,
            prev_round_time: Duration::ZERO,
            now,
            prev_close_time: now,
            prev_ledger_id: None,
            previous_ledger: None,
            result: None,
            raw_close_times: ConsensusCloseTimes::default(),
            curr_peer_positions: HashMap::new(),
            recent_peer_positions: HashMap::new(),
            prev_proposers: 0,
            acquired: HashMap::new(),
            dead_nodes: HashSet::new(),
            peer_unchanged_counter: 0,
            establish_counter: 0,
            close_time_avalanche_state: AvalancheState::Init,
        }
    }

    pub fn phase(&self) -> ConsensusPhase {
        self.phase
    }

    pub fn mode(&self) -> ConsensusMode {
        self.mode
    }

    pub fn close_resolution(&self) -> TimeDuration {
        self.close_resolution
    }

    pub fn result(&self) -> Option<&ResultState<A>> {
        self.result.as_ref()
    }

    pub fn adaptor(&self) -> &A {
        &self.adaptor
    }

    pub fn adaptor_mut(&mut self) -> &mut A {
        &mut self.adaptor
    }

    fn set_mode(&mut self, mode: ConsensusMode) {
        if self.mode != mode {
            let before = self.mode;
            self.mode = mode;
            tracing::info!(target: "consensus", ?before, ?mode, "Consensus mode changed");
            self.adaptor.on_mode_change(before, mode);
        }
    }

    pub fn start_round(
        &mut self,
        now: NetClockTimePoint,
        prev_ledger_id: A::LedgerId,
        prev_ledger: A::Ledger,
        proposing: bool,
    ) -> Vec<ConsensusEvent<A::LedgerId>> {
        tracing::info!(target: "consensus", prev_ledger = %prev_ledger_id.to_string(), proposing, "Starting consensus round");
        if self.first_round {
            self.prev_round_time = self.parms.ledger_idle_interval;
            let ledger_close = self.adaptor.close_time(&prev_ledger);
            if ledger_close.as_seconds() != 0 {
                self.prev_close_time = ledger_close;
            }
            // If ledger_close == 0 (genesis), keep prev_close_time from
            // constructor initialization (clock.now()), matching rippled.
            self.first_round = false;
        } else {
            self.prev_close_time = self.raw_close_times.self_close_time;
        }

        if self.prev_proposers == 0 {
            self.prev_proposers = self.adaptor.proposers_validated(&prev_ledger_id);
        }

        let mut start_mode = if proposing {
            ConsensusMode::Proposing
        } else {
            ConsensusMode::Observing
        };
        let mut working_ledger = prev_ledger;

        if self.adaptor.id(&working_ledger) != prev_ledger_id {
            if let Some(acquired_ledger) = self.adaptor.acquire_ledger(&prev_ledger_id) {
                working_ledger = acquired_ledger;
            } else {
                start_mode = ConsensusMode::WrongLedger;
            }
        }

        self.start_round_internal(now, prev_ledger_id, working_ledger, start_mode);
        Vec::new()
    }

    fn start_round_internal(
        &mut self,
        now: NetClockTimePoint,
        prev_ledger_id: A::LedgerId,
        prev_ledger: A::Ledger,
        mode: ConsensusMode,
    ) {
        let seq = self.adaptor.seq(&prev_ledger) + 1;
        tracing::info!(target: "consensus", phase = "open", seq, "Consensus phase: open");
        self.phase = ConsensusPhase::Open;
        self.set_mode(mode);
        self.now = now;
        self.prev_ledger_id = Some(prev_ledger_id.clone());
        self.previous_ledger = Some(prev_ledger.clone());
        self.result = None;
        self.acquired.clear();
        self.converge_percent = 0;
        self.close_time_avalanche_state = AvalancheState::Init;
        self.have_close_time_consensus = false;
        self.open_time.reset();
        self.curr_peer_positions.clear();
        self.raw_close_times.peers.clear();
        self.raw_close_times.self_close_time = NetClockTimePoint::default();
        self.dead_nodes.clear();
        self.close_resolution = get_next_ledger_time_resolution(
            self.adaptor.close_time_resolution(&prev_ledger),
            self.adaptor.close_agree(&prev_ledger),
            self.adaptor.seq(&prev_ledger) + 1,
        );

        self.playback_proposals();
        if self.curr_peer_positions.len() > (self.prev_proposers / 2) {
            let _ = self.timer_entry(self.now);
        }
    }

    pub fn peer_proposal(&mut self, now: NetClockTimePoint, new_peer_pos: A::PeerPosition) -> bool {
        let proposal = new_peer_pos.proposal().clone();
        let peer_id = proposal.node_id().clone();
        let recent = self.recent_peer_positions.entry(peer_id).or_default();
        if recent.len() >= 10 {
            recent.pop_front();
        }
        recent.push_back(new_peer_pos.clone());
        self.peer_proposal_internal(now, new_peer_pos)
    }

    fn peer_proposal_internal(
        &mut self,
        now: NetClockTimePoint,
        new_peer_pos: A::PeerPosition,
    ) -> bool {
        let proposal = new_peer_pos.proposal().clone();
        let peer_id = proposal.node_id().clone();

        self.now = now;
        if self.phase == ConsensusPhase::Accepted {
            return false;
        }
        if self.dead_nodes.contains(&peer_id) {
            return false;
        }

        let Some(prev_ledger_id) = self.prev_ledger_id.as_ref() else {
            return false;
        };
        if proposal.prev_ledger() != prev_ledger_id {
            return false;
        }

        if let Some(existing) = self.curr_peer_positions.get(&peer_id)
            && proposal.propose_seq() <= existing.proposal().propose_seq()
        {
            return false;
        }

        if proposal.is_bow_out() {
            tracing::debug!(target: "consensus", peer = %peer_id.to_string(), "Peer bowed out");
            if let Some(result) = &mut self.result {
                for dispute in result.disputes.values_mut() {
                    dispute.unvote(&peer_id);
                }
            }
            self.curr_peer_positions.remove(&peer_id);
            self.dead_nodes.insert(peer_id);
            return true;
        }

        self.curr_peer_positions
            .insert(peer_id.clone(), new_peer_pos.clone());

        tracing::debug!(target: "consensus", peer = %peer_id.to_string(), their_position = %proposal.position().to_string(), "Peer position received");

        if proposal.is_initial() {
            *self
                .raw_close_times
                .peers
                .entry(proposal.close_time())
                .or_default() += 1;
        }

        let txset_id = proposal.position().clone();
        if let Some(txset) = self.acquired.get(&txset_id).cloned() {
            if self.result.is_some() {
                self.update_disputes(&peer_id, &txset);
            }
        } else if let Some(txset) = self.adaptor.acquire_tx_set(&txset_id) {
            self.got_tx_set(now, txset);
        }

        true
    }

    pub fn got_tx_set(&mut self, now: NetClockTimePoint, txset: A::TxSet) {
        if self.phase == ConsensusPhase::Accepted {
            return;
        }
        self.now = now;
        let id = self.adaptor.txset_id(&txset);
        tracing::debug!(target: "consensus", txset_id = %id.to_string(), "Transaction set acquired");
        if self.acquired.insert(id.clone(), txset.clone()).is_some() {
            return;
        }
        if self.result.is_none() {
            return;
        }

        let peers = self
            .curr_peer_positions
            .iter()
            .filter_map(|(node_id, peer_pos)| {
                (peer_pos.proposal().position() == &id).then_some(node_id.clone())
            })
            .collect::<Vec<_>>();
        for node_id in peers {
            self.update_disputes(&node_id, &txset);
        }
    }

    pub fn timer_entry(&mut self, now: NetClockTimePoint) -> ConsensusDecision {
        if self.phase == ConsensusPhase::Accepted {
            return ConsensusDecision::Accepted(
                self.result
                    .as_ref()
                    .map_or(ConsensusState::Yes, |r| r.state),
            );
        }
        self.now = now;
        self.check_ledger();
        match self.phase {
            ConsensusPhase::Open => self.phase_open(),
            ConsensusPhase::Establish => self.phase_establish(),
            ConsensusPhase::Accepted => ConsensusDecision::Accepted(
                self.result
                    .as_ref()
                    .map_or(ConsensusState::Yes, |r| r.state),
            ),
        }
    }

    fn check_ledger(&mut self) {
        let (Some(prev_ledger_id), Some(previous_ledger)) =
            (self.prev_ledger_id.clone(), self.previous_ledger.clone())
        else {
            return;
        };

        let network_ledger =
            self.adaptor
                .get_prev_ledger(&prev_ledger_id, &previous_ledger, self.mode);
        if network_ledger != prev_ledger_id || self.adaptor.id(&previous_ledger) != prev_ledger_id {
            self.handle_wrong_ledger(network_ledger);
        }
    }

    fn handle_wrong_ledger(&mut self, ledger_id: A::LedgerId) {
        tracing::warn!(target: "consensus", new_ledger = %ledger_id.to_string(), "Switching to different ledger");
        self.leave_consensus();

        if self.prev_ledger_id.as_ref() != Some(&ledger_id) {
            self.prev_ledger_id = Some(ledger_id.clone());
            if let Some(result) = &mut self.result {
                result.disputes.clear();
                result.compares.clear();
            }
            self.curr_peer_positions.clear();
            self.raw_close_times.peers.clear();
            self.dead_nodes.clear();
            self.playback_proposals();
        }

        let Some(previous_ledger) = self.previous_ledger.clone() else {
            return;
        };
        if self.adaptor.id(&previous_ledger) == ledger_id {
            return;
        }

        if let Some(new_ledger) = self.adaptor.acquire_ledger(&ledger_id) {
            self.start_round_internal(
                self.now,
                ledger_id,
                new_ledger,
                ConsensusMode::SwitchedLedger,
            );
        } else {
            self.set_mode(ConsensusMode::WrongLedger);
        }
    }

    fn playback_proposals(&mut self) {
        let Some(prev_ledger_id) = self.prev_ledger_id.as_ref() else {
            return;
        };

        let replay = self
            .recent_peer_positions
            .values()
            .flat_map(|positions| positions.iter())
            .filter(|peer_pos| peer_pos.proposal().prev_ledger() == prev_ledger_id)
            .cloned()
            .collect::<Vec<_>>();

        if !replay.is_empty() {
            tracing::debug!(target: "consensus", count = replay.len(), "Replaying recent peer positions");
        }
        for peer_pos in replay {
            if self.peer_proposal_internal(self.now, peer_pos.clone()) {
                self.adaptor.share_peer_position(&peer_pos);
            }
        }
    }

    fn phase_open(&mut self) -> ConsensusDecision {
        let previous_ledger = self
            .previous_ledger
            .as_ref()
            .expect("consensus must have previous ledger");
        self.open_time.tick_fixed(self.parms.ledger_granularity);

        let previous_close_correct = self.mode != ConsensusMode::WrongLedger
            && self.adaptor.close_agree(previous_ledger)
            && self.adaptor.close_time(previous_ledger)
                != (self.adaptor.parent_close_time(previous_ledger) + TimeDuration::seconds(1));
        let last_close_time = if previous_close_correct {
            self.adaptor.close_time(previous_ledger)
        } else {
            self.prev_close_time
        };

        let idle_interval = self.parms.ledger_idle_interval.max(time_duration_to_std(
            self.adaptor.close_time_resolution(previous_ledger) * 2,
        ));
        let should_close = should_close_ledger(
            self.adaptor.has_open_transactions(),
            self.prev_proposers,
            self.curr_peer_positions.len(),
            self.adaptor
                .proposers_validated(self.prev_ledger_id.as_ref().expect("previous ledger id")),
            self.prev_round_time,
            duration_between(last_close_time, self.now),
            self.open_time.read(),
            idle_interval,
            &self.parms,
        );
        if !should_close {
            return ConsensusDecision::StayOpen;
        }
        // Don't close when we know we're on the wrong ledger — wait for
        // acquisition to switch us to the correct one.
        if matches!(self.mode, ConsensusMode::WrongLedger) {
            return ConsensusDecision::StayOpen;
        }
        // Don't close on idle when observing and no peers are on our ledger.
        // This prevents solo-closing while waiting to catch up to the network.
        if self.mode == ConsensusMode::Observing
            && self.curr_peer_positions.is_empty()
            && self.adaptor.proposers_validated(
                self.prev_ledger_id.as_ref().expect("prev id"),
            ) == 0
        {
            return ConsensusDecision::StayOpen;
        }

        self.close_ledger();
        ConsensusDecision::CloseLedger
    }

    fn phase_establish(&mut self) -> ConsensusDecision {
        self.peer_unchanged_counter += 1;
        self.establish_counter += 1;

        if let Some(result) = &mut self.result {
            result.round_time.tick_fixed(self.parms.ledger_granularity);
            result.proposers = self.curr_peer_positions.len();
            self.converge_percent = calc_converge_percent(
                result.round_time.read(),
                self.prev_round_time,
                self.parms.av_min_consensus_time,
            );
            let elapsed_ms = result.round_time.read().as_millis() as u64;
            tracing::debug!(target: "consensus", elapsed_ms, converge_pct = self.converge_percent, proposers = result.proposers, "Establish round progress");
            if result.round_time.read() < self.parms.ledger_min_consensus {
                return ConsensusDecision::StayOpen;
            }
        }

        self.update_our_positions();
        let state = self.have_consensus();
        if state == ConsensusState::No || !self.have_close_time_consensus {
            return ConsensusDecision::StayOpen;
        }

        self.phase = ConsensusPhase::Accepted;
        if let Some(result) = &mut self.result {
            result.state = state;
            result.proposers = self.curr_peer_positions.len();
            self.prev_proposers = self.curr_peer_positions.len();
            self.prev_round_time = result.round_time.read();
            let tx_count = result.disputes.len();
            tracing::info!(target: "consensus", phase = "accept", tx_count, "Consensus phase: accept");
        }
        let previous_ledger = self
            .previous_ledger
            .as_ref()
            .expect("consensus must have previous ledger");

        if let Some(result) = &self.result {
            tracing::info!(target: "consensus", state = ?state, proposers = result.proposers, round_time_ms = result.round_time.read().as_millis() as u64, "Consensus accepted, calling on_accept");
            self.adaptor.on_accept(result, previous_ledger);
        }
        ConsensusDecision::Accepted(state)
    }

    fn close_ledger(&mut self) {
        let previous_ledger = self
            .previous_ledger
            .as_ref()
            .expect("consensus must have previous ledger");
        let (txns, txset_id) = self.adaptor.make_txset(previous_ledger);
        let proposal = ConsensusProposal::new(
            self.prev_ledger_id.clone().expect("previous ledger id"),
            ConsensusProposal::<A::NodeId, A::LedgerId, A::TxSetId>::SEQ_JOIN,
            txset_id.clone(),
            self.adaptor.now_close_time(),
            self.now,
            self.adaptor.node_id(),
        );

        self.phase = ConsensusPhase::Establish;
        let proposers = self.curr_peer_positions.len();
        tracing::info!(target: "consensus", phase = "establish", proposers, "Consensus phase: establish");
        self.raw_close_times.self_close_time = self.now;
        self.peer_unchanged_counter = 0;
        self.establish_counter = 0;
        self.result = Some(ConsensusResult::new(txns, proposal));

        if let Some(result) = &self.result
            && self
                .acquired
                .insert(txset_id.clone(), result.txns.clone())
                .is_none()
        {
            self.adaptor.share_tx_set(&result.txns);
        }

        if self.mode == ConsensusMode::Proposing
            && let Some(result) = &self.result
        {
            tracing::debug!(target: "consensus", our_position = %result.position.position().to_string(), "Our position set");
            self.adaptor.propose(&result.position);
        }

        let peer_sets = self
            .curr_peer_positions
            .values()
            .map(|peer| peer.proposal().position().clone())
            .collect::<Vec<_>>();
        for txset_id in peer_sets {
            if let Some(txset) = self.acquired.get(&txset_id).cloned() {
                self.create_disputes(&txset);
            }
        }
    }

    fn update_our_positions(&mut self) {
        let peer_cutoff = subtract_duration(self.now, self.parms.propose_freshness);
        let our_cutoff = subtract_duration(self.now, self.parms.propose_interval);
        let mut close_time_votes: HashMap<NetClockTimePoint, i32> = HashMap::new();
        let mut stale_nodes = Vec::new();

        for (node_id, peer_pos) in &self.curr_peer_positions {
            let peer_prop = peer_pos.proposal();
            if peer_prop.is_stale(peer_cutoff) {
                stale_nodes.push(node_id.clone());
            } else {
                *close_time_votes
                    .entry(self.as_close_time(peer_prop.close_time()))
                    .or_default() += 1;
            }
        }

        if let Some(result) = &mut self.result {
            for node_id in stale_nodes {
                tracing::debug!(target: "consensus", peer = %node_id.to_string(), "Removing stale peer proposal");
                for dispute in result.disputes.values_mut() {
                    dispute.unvote(&node_id);
                }
                self.curr_peer_positions.remove(&node_id);
            }
        }

        let mut our_new_set = None;
        if let Some(result) = &mut self.result {
            let mut mutable_set = None;
            for dispute in result.disputes.values_mut() {
                if dispute.update_vote(
                    self.converge_percent,
                    self.mode == ConsensusMode::Proposing,
                    &self.parms,
                ) {
                    let set = mutable_set.get_or_insert_with(|| result.txns.clone());
                    if dispute.get_our_vote() {
                        self.adaptor.txset_insert(set, dispute.tx().clone());
                    } else {
                        let tx_id = self.adaptor.tx_id(dispute.tx());
                        self.adaptor.txset_erase(set, &tx_id);
                    }
                }
            }
            our_new_set = mutable_set;
        }

        let mut consensus_close_time = NetClockTimePoint::default();
        self.have_close_time_consensus = false;

        if self.curr_peer_positions.is_empty() {
            self.have_close_time_consensus = true;
            consensus_close_time = self.as_close_time(
                self.result
                    .as_ref()
                    .expect("consensus result must exist")
                    .position
                    .close_time(),
            );
        } else {
            let (needed_weight, new_state) = get_needed_weight(
                &self.parms,
                self.close_time_avalanche_state,
                self.converge_percent,
                0,
                0,
            );
            if let Some(new_state) = new_state {
                self.close_time_avalanche_state = new_state;
            }

            let mut participants = self.curr_peer_positions.len();
            if self.mode == ConsensusMode::Proposing
                && let Some(result) = &self.result
            {
                *close_time_votes
                    .entry(self.as_close_time(result.position.close_time()))
                    .or_default() += 1;
                participants += 1;
            }

            let mut thresh_vote = participants_needed(participants, needed_weight);
            let thresh_consensus =
                participants_needed(participants, self.parms.av_ct_consensus_pct);

            for (time, votes) in close_time_votes {
                if usize::try_from(votes).unwrap_or_default() >= thresh_vote {
                    consensus_close_time = time;
                    thresh_vote = usize::try_from(votes).unwrap_or_default();
                    if thresh_vote >= thresh_consensus {
                        self.have_close_time_consensus = true;
                        tracing::debug!(target: "consensus", "Close time consensus reached");
                    }
                }
            }
        }

        let change_due_to_time = self.result.as_ref().is_some_and(|result| {
            consensus_close_time != self.as_close_time(result.position.close_time())
                || result.position.is_stale(our_cutoff)
        });
        if our_new_set.is_none() && change_due_to_time {
            our_new_set = self.result.as_ref().map(|result| result.txns.clone());
        }

        if let Some(new_set) = our_new_set {
            let new_id = self.adaptor.txset_id(&new_set);
            let should_update_peers = if let Some(result) = &mut self.result {
                result.txns = new_set;
                result
                    .position
                    .change_position(new_id.clone(), consensus_close_time, self.now);
                self.acquired
                    .insert(new_id.clone(), result.txns.clone())
                    .is_none()
            } else {
                false
            };

            if should_update_peers {
                let txset = self
                    .result
                    .as_ref()
                    .expect("updated result must exist")
                    .txns
                    .clone();
                self.adaptor.share_tx_set(&txset);
                let peers = self
                    .curr_peer_positions
                    .iter()
                    .filter_map(|(node_id, peer_pos)| {
                        (peer_pos.proposal().position() == &new_id).then_some(node_id.clone())
                    })
                    .collect::<Vec<_>>();
                for node_id in peers {
                    self.update_disputes(&node_id, &txset);
                }
            }

            if self.mode == ConsensusMode::Proposing
                && let Some(result) = &self.result
                && !result.position.is_bow_out()
            {
                tracing::debug!(target: "consensus", our_position = %result.position.position().to_string(), "Our position updated");
                self.adaptor.propose(&result.position);
            }
        }
    }

    fn have_consensus(&mut self) -> ConsensusState {
        let Some(result) = &self.result else {
            return ConsensusState::No;
        };
        let previous_ledger = self
            .previous_ledger
            .as_ref()
            .expect("consensus must have previous ledger");

        let our_position = result.position.position().clone();
        let mut agree = 0usize;
        let mut disagree = 0usize;
        for peer_pos in self.curr_peer_positions.values() {
            if peer_pos.proposal().position() == &our_position {
                agree += 1;
            } else {
                disagree += 1;
            }
        }

        let stalled = self.have_close_time_consensus
            && !result.disputes.is_empty()
            && result.disputes.values().all(|dispute| {
                dispute.stalled(
                    &self.parms,
                    self.mode == ConsensusMode::Proposing,
                    self.peer_unchanged_counter,
                )
            });

        if stalled {
            tracing::debug!(target: "consensus", disputes = result.disputes.len(), "All disputes stalled");
        }

        let state = check_consensus(
            self.prev_proposers,
            agree + disagree,
            agree,
            self.adaptor.proposers_finished(
                previous_ledger,
                self.prev_ledger_id.as_ref().expect("previous ledger id"),
            ),
            self.prev_round_time,
            result.round_time.read(),
            stalled,
            &self.parms,
            self.mode == ConsensusMode::Proposing,
        );

        if state == ConsensusState::Yes || state == ConsensusState::MovedOn {
            let total = agree + disagree;
            let threshold_pct = if total > 0 {
                (agree * 100) / total
            } else {
                100
            };
            tracing::info!(target: "consensus", threshold_pct, agree, total, "Consensus threshold met");
        }

        if state == ConsensusState::Expired {
            let elapsed_ms = result.round_time.read().as_millis() as u64;
            tracing::warn!(target: "consensus", elapsed_ms, "Consensus round taking too long");
            let minimum_counter = self.parms.avalanche_cutoffs.len() * self.parms.av_min_rounds;
            if self.establish_counter < minimum_counter {
                return ConsensusState::No;
            }
            self.leave_consensus();
        }

        state
    }

    fn leave_consensus(&mut self) {
        if self.mode == ConsensusMode::Proposing {
            tracing::info!(target: "consensus", "Bowing out of consensus");
            if let Some(result) = &mut self.result
                && !result.position.is_bow_out()
            {
                result.position.bow_out(self.now);
                self.adaptor.propose(&result.position);
            }
            self.set_mode(ConsensusMode::Observing);
        }
    }

    fn create_disputes(&mut self, other: &A::TxSet) {
        let Some(result) = &mut self.result else {
            return;
        };

        let other_id = self.adaptor.txset_id(other);
        if !result.compares.insert(other_id.clone()) {
            return;
        }
        if self.adaptor.txset_id(&result.txns) == other_id {
            return;
        }

        for (tx_id, in_this_set) in self.adaptor.txset_compare(&result.txns, other) {
            if result.disputes.contains_key(&tx_id) {
                continue;
            }
            let tx = if in_this_set {
                self.adaptor
                    .txset_find(&result.txns, &tx_id)
                    .expect("differing transaction must exist in local set")
            } else {
                self.adaptor
                    .txset_find(other, &tx_id)
                    .expect("differing transaction must exist in peer set")
            };
            let mut dispute =
                crate::disputed_tx::DisputedTx::new(tx.clone(), tx_id.clone(), in_this_set);

            for (node_id, peer_pos) in &self.curr_peer_positions {
                if let Some(peer_set) = self.acquired.get(peer_pos.proposal().position())
                    && dispute
                        .set_vote(node_id.clone(), self.adaptor.txset_exists(peer_set, &tx_id))
                {
                    self.peer_unchanged_counter = 0;
                }
            }

            self.adaptor.share_tx(&tx);
            tracing::debug!(target: "consensus", tx_id = %tx_id.to_string(), in_our_set = in_this_set, "New transaction dispute created");
            result.disputes.insert(tx_id, dispute);
        }
    }

    fn update_disputes(&mut self, node_id: &A::NodeId, other: &A::TxSet) {
        if let Some(result) = &self.result {
            let other_id = self.adaptor.txset_id(other);
            if !result.compares.contains(&other_id) {
                let other_owned = other.clone();
                let _ = result;
                self.create_disputes(&other_owned);
            }
        }

        if let Some(result) = &mut self.result {
            for dispute in result.disputes.values_mut() {
                let tx_id = self.adaptor.tx_id(dispute.tx());
                if dispute.set_vote(node_id.clone(), self.adaptor.txset_exists(other, &tx_id)) {
                    self.peer_unchanged_counter = 0;
                }
            }
        }
    }

    fn as_close_time(&self, raw: NetClockTimePoint) -> NetClockTimePoint {
        round_close_time(raw, self.close_resolution)
    }
}

fn duration_between(start: NetClockTimePoint, end: NetClockTimePoint) -> Duration {
    let diff = end - start;
    Duration::from_secs(u64::try_from(diff.whole_seconds().max(0)).expect("seconds must fit u64"))
}

fn duration_to_millis(duration: Duration) -> Duration {
    Duration::from_millis(
        u64::try_from(duration.as_millis()).expect("duration milliseconds must fit u64"),
    )
}

fn calc_converge_percent(
    current_round_time: Duration,
    previous_round_time: Duration,
    minimum_previous_round: Duration,
) -> i32 {
    let denominator = previous_round_time.max(minimum_previous_round).as_millis();
    let numerator = current_round_time.as_millis().saturating_mul(100);
    i32::try_from(numerator / denominator).expect("converge percent must fit in i32")
}

fn participants_needed(participants: usize, percent: usize) -> usize {
    let result = ((participants * percent) + (percent / 2)) / 100;
    result.max(1)
}

fn subtract_duration(point: NetClockTimePoint, duration: Duration) -> NetClockTimePoint {
    point
        .checked_sub(TimeDuration::seconds(
            i64::try_from(duration.as_secs()).expect("duration seconds must fit i64"),
        ))
        .unwrap_or_default()
}

fn time_duration_to_std(duration: TimeDuration) -> Duration {
    Duration::from_millis(
        u64::try_from(duration.whole_milliseconds().max(0))
            .expect("duration milliseconds must fit u64"),
    )
}
