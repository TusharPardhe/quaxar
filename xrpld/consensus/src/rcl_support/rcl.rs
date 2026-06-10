use crate::consensus::{Consensus, ConsensusAdaptor, ConsensusDecision, ConsensusPeerPosition};
use crate::params::ConsensusParms;
use crate::proposal::{ConsensusHashable, ConsensusProposal};
use crate::rcl_hash::rcl_txset_id;
use crate::types::{ConsensusMode, ConsensusPhase, ConsensusState};
use basics::base_uint::Uint256;
use protocol::{PublicKey, encode_node_public_base58, verify};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tracing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RclCxLedger {
    pub id: Uint256,
    pub seq: u32,
    pub parent_id: Uint256,
    pub close_time_resolution: time::Duration,
    pub close_agree: bool,
    pub close_time: basics::chrono::NetClockTimePoint,
    pub parent_close_time: basics::chrono::NetClockTimePoint,
    pub base_fee_req: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RclCxTx {
    pub id: Uint256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RclCxPeerPos {
    pub public_key: PublicKey,
    pub suppression_id: Uint256,
    pub proposal: ConsensusProposal<PublicKey, Uint256, Uint256>,
    signature: Vec<u8>,
}

impl RclCxPeerPos {
    pub fn new(
        public_key: PublicKey,
        signature: impl AsRef<[u8]>,
        suppression_id: Uint256,
        proposal: ConsensusProposal<PublicKey, Uint256, Uint256>,
    ) -> Self {
        let signature = signature.as_ref().to_vec();
        assert!(
            !signature.is_empty() && signature.len() <= 72,
            "RclCxPeerPos signature must be 1..=72 bytes"
        );
        Self {
            public_key,
            suppression_id,
            proposal,
            signature,
        }
    }

    pub fn signature(&self) -> &[u8] {
        &self.signature
    }

    pub fn check_sign(&self) -> bool {
        let valid = verify(
            &self.public_key,
            &self.proposal.signing_data(),
            &self.signature,
        );
        if !valid {
            tracing::warn!(target: "consensus", peer = %encode_node_public_base58(self.public_key.to_bytes()), "Peer proposal signature verification failed");
        }
        valid
    }

    pub fn get_json(&self) -> Value {
        let mut value = self.proposal.get_json();
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "peer_id".to_owned(),
                json!(encode_node_public_base58(self.public_key.to_bytes())),
            );
        }
        value
    }
}

impl ConsensusPeerPosition<PublicKey, Uint256, Uint256> for RclCxPeerPos {
    fn proposal(&self) -> &ConsensusProposal<PublicKey, Uint256, Uint256> {
        &self.proposal
    }
}

pub trait RclConsensusAdapter: Send {
    fn now(&self) -> basics::chrono::NetClockTimePoint;
    fn acquire_ledger(&mut self, ledger_id: &Uint256) -> Option<RclCxLedger>;
    fn acquire_tx_set(&mut self, txset_id: &Uint256) -> Option<Vec<RclCxTx>>;
    fn has_open_transactions(&self) -> bool;
    fn proposers_validated(&self, prev_ledger: &Uint256) -> usize;
    fn proposers_finished(&self, prev_ledger: &RclCxLedger, prev_ledger_id: &Uint256) -> usize;
    fn pre_start_round_for_proposing(&self) {}
    fn should_propose(&self) -> bool;
    fn prev_round_time(&self) -> Duration;
    fn now_close_time(&self) -> basics::chrono::NetClockTimePoint;
    fn get_prev_ledger(
        &mut self,
        prev_ledger_id: &Uint256,
        _prev_ledger: &RclCxLedger,
        _mode: ConsensusMode,
    ) -> Uint256 {
        *prev_ledger_id
    }
    fn on_mode_change(&mut self, _before: ConsensusMode, _after: ConsensusMode) {}
    fn on_accept(
        &mut self,
        result: &crate::types::ConsensusResult<
            Uint256,
            PublicKey,
            Vec<RclCxTx>,
            Uint256,
            RclCxTx,
            Uint256,
        >,
        prev_ledger: &RclCxLedger,
    );
    fn make_txset(&mut self, previous_ledger: &RclCxLedger) -> (Vec<RclCxTx>, Uint256);
    fn propose(&mut self, proposal: &ConsensusProposal<PublicKey, Uint256, Uint256>);
    fn share_peer_position(&mut self, _peer_position: &RclCxPeerPos) {}
    fn share_tx_set(&mut self, _txset: &[RclCxTx]) {}
    fn share_tx(&mut self, tx: &RclCxTx);
    fn node_id(&self) -> PublicKey;
    /// Called after on_accept completes. If end_consensus queued a start_round,
    /// return the parameters here so the caller can start the next round.
    fn take_pending_start_round(
        &self,
    ) -> Option<(basics::chrono::NetClockTimePoint, Uint256, RclCxLedger)> {
        None
    }
}

pub struct AdapterBox<A: RclConsensusAdapter> {
    pub inner: A,
}

impl<A: RclConsensusAdapter> ConsensusAdaptor for AdapterBox<A> {
    type LedgerId = Uint256;
    type Ledger = RclCxLedger;
    type NodeId = PublicKey;
    type TxSetId = Uint256;
    type TxSet = Vec<RclCxTx>;
    type Tx = RclCxTx;
    type TxId = Uint256;
    type PeerPosition = RclCxPeerPos;

    fn now(&self) -> basics::chrono::NetClockTimePoint {
        self.inner.now()
    }
    fn acquire_ledger(&mut self, ledger_id: &Self::LedgerId) -> Option<Self::Ledger> {
        self.inner.acquire_ledger(ledger_id)
    }
    fn acquire_tx_set(&mut self, txset_id: &Self::TxSetId) -> Option<Self::TxSet> {
        self.inner.acquire_tx_set(txset_id)
    }
    fn has_open_transactions(&self) -> bool {
        self.inner.has_open_transactions()
    }
    fn proposers_validated(&self, prev_ledger: &Self::LedgerId) -> usize {
        self.inner.proposers_validated(prev_ledger)
    }
    fn proposers_finished(
        &self,
        prev_ledger: &Self::Ledger,
        prev_ledger_id: &Self::LedgerId,
    ) -> usize {
        self.inner.proposers_finished(prev_ledger, prev_ledger_id)
    }
    fn should_propose(&self) -> bool {
        self.inner.should_propose()
    }
    fn prev_round_time(&self) -> Duration {
        self.inner.prev_round_time()
    }
    fn now_close_time(&self) -> basics::chrono::NetClockTimePoint {
        self.inner.now_close_time()
    }
    fn close_time_resolution(&self, ledger: &Self::Ledger) -> time::Duration {
        ledger.close_time_resolution
    }
    fn close_agree(&self, ledger: &Self::Ledger) -> bool {
        ledger.close_agree
    }
    fn close_time(&self, ledger: &Self::Ledger) -> basics::chrono::NetClockTimePoint {
        ledger.close_time
    }
    fn parent_close_time(&self, ledger: &Self::Ledger) -> basics::chrono::NetClockTimePoint {
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
        result: &crate::types::ConsensusResult<
            Self::LedgerId,
            Self::NodeId,
            Self::TxSet,
            Self::TxSetId,
            Self::Tx,
            Self::TxId,
        >,
        prev_ledger: &Self::Ledger,
    ) {
        tracing::info!(target: "consensus", prev_seq = prev_ledger.seq, disputes = result.disputes.len(), "RCL on_accept");
        self.inner.on_accept(result, prev_ledger)
    }
    fn make_txset(&mut self, previous_ledger: &Self::Ledger) -> (Self::TxSet, Self::TxSetId) {
        self.inner.make_txset(previous_ledger)
    }
    fn get_prev_ledger(
        &mut self,
        prev_ledger_id: &Self::LedgerId,
        prev_ledger: &Self::Ledger,
        mode: ConsensusMode,
    ) -> Self::LedgerId {
        self.inner
            .get_prev_ledger(prev_ledger_id, prev_ledger, mode)
    }
    fn on_mode_change(&mut self, before: ConsensusMode, after: ConsensusMode) {
        tracing::info!(target: "consensus", ?before, ?after, "RCL consensus mode change");
        self.inner.on_mode_change(before, after);
    }
    fn propose(
        &mut self,
        proposal: &ConsensusProposal<Self::NodeId, Self::LedgerId, Self::TxSetId>,
    ) {
        tracing::debug!(target: "consensus", seq = proposal.propose_seq(), position = %proposal.position(), "RCL proposing position");
        self.inner.propose(proposal)
    }
    fn share_peer_position(&mut self, peer_position: &Self::PeerPosition) {
        self.inner.share_peer_position(peer_position)
    }
    fn share_tx_set(&mut self, txset: &Self::TxSet) {
        self.inner.share_tx_set(txset)
    }
    fn share_tx(&mut self, tx: &Self::Tx) {
        self.inner.share_tx(tx)
    }
    fn node_id(&self) -> Self::NodeId {
        self.inner.node_id()
    }
    fn txset_id(&self, txset: &Self::TxSet) -> Self::TxSetId {
        let tx_ids = txset.iter().map(|tx| tx.id).collect::<Vec<_>>();
        rcl_txset_id(&tx_ids)
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
        txset.retain(|candidate| candidate.id != *txid);
    }
}

pub struct RclRoundTimer {
    next_tick: Option<Instant>,
    period: Duration,
}

impl RclRoundTimer {
    pub fn new(period: Duration) -> Self {
        Self {
            next_tick: None,
            period,
        }
    }

    pub fn new_at(period: Duration, first_tick_at: Instant) -> Self {
        Self {
            next_tick: Some(first_tick_at),
            period,
        }
    }

    pub const fn period(&self) -> Duration {
        self.period
    }

    pub async fn tick(&mut self) {
        let next = self
            .next_tick
            .get_or_insert_with(|| Instant::now() + self.period);
        let now = Instant::now();
        if now < *next {
            tokio::time::sleep(*next - now).await;
        }
        *next += self.period;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RclConsensusState {
    pub phase: ConsensusPhase,
    pub mode: ConsensusMode,
    pub close_resolution: time::Duration,
    pub result_state: Option<ConsensusState>,
}

pub struct RclConsensus<A: RclConsensusAdapter> {
    consensus: Consensus<AdapterBox<A>>,
    timer: RclRoundTimer,
}

impl<A: RclConsensusAdapter> RclConsensus<A> {
    pub fn new(adapter: A, parms: ConsensusParms) -> Self {
        let timer = RclRoundTimer::new(parms.ledger_granularity);
        Self::with_round_timer(adapter, parms, timer)
    }

    pub fn with_round_timer(adapter: A, parms: ConsensusParms, timer: RclRoundTimer) -> Self {
        Self {
            consensus: Consensus::new(AdapterBox { inner: adapter }, parms),
            timer,
        }
    }

    pub fn start_round(
        &mut self,
        now: basics::chrono::NetClockTimePoint,
        prev_ledger_id: Uint256,
        prev_ledger: RclCxLedger,
    ) {
        tracing::info!(target: "consensus", seq = prev_ledger.seq + 1, prev_ledger = %prev_ledger_id, "RCL consensus starting round");
        self.consensus.adaptor_mut().inner.pre_start_round_for_proposing(); let proposing = self.consensus.adaptor().inner.should_propose();
        let _ = self
            .consensus
            .start_round(now, prev_ledger_id, prev_ledger, proposing);
    }

    pub fn peer_proposal(
        &mut self,
        now: basics::chrono::NetClockTimePoint,
        peer_position: RclCxPeerPos,
    ) -> bool {
        tracing::debug!(target: "consensus", peer = %encode_node_public_base58(peer_position.public_key.to_bytes()), "RCL peer proposal received");
        self.consensus.peer_proposal(now, peer_position)
    }

    pub fn got_tx_set(&mut self, now: basics::chrono::NetClockTimePoint, txset: Vec<RclCxTx>) {
        tracing::debug!(target: "consensus", tx_count = txset.len(), "RCL got transaction set");
        self.consensus.got_tx_set(now, txset);
    }

    pub async fn timer_tick(
        &mut self,
        now: basics::chrono::NetClockTimePoint,
    ) -> ConsensusDecision {
        self.timer.tick().await;
        let decision = self.consensus.timer_entry(now);
        match &decision {
            ConsensusDecision::CloseLedger => {
                tracing::info!(target: "consensus", "RCL timer: ledger closed");
            }
            ConsensusDecision::Accepted(state) => {
                tracing::info!(target: "consensus", ?state, "RCL timer: consensus accepted");
            }
            ConsensusDecision::StayOpen => {}
        }
        decision
    }

    pub fn state(&self) -> RclConsensusState {
        RclConsensusState {
            phase: self.consensus.phase(),
            mode: self.consensus.mode(),
            close_resolution: self.consensus.close_resolution(),
            result_state: self.consensus.result().map(|result| result.state),
        }
    }

    pub fn result(
        &self,
    ) -> Option<
        &crate::types::ConsensusResult<Uint256, PublicKey, Vec<RclCxTx>, Uint256, RclCxTx, Uint256>,
    > {
        self.consensus.result()
    }

    /// Access the inner adaptor (for consuming pending state from on_accept).
    pub fn adaptor(&self) -> &AdapterBox<A> {
        self.consensus.adaptor()
    }
}

impl ConsensusHashable for PublicKey {
    fn append_consensus_bytes(&self, serializer: &mut protocol::Serializer) {
        serializer.add_vl(self.as_ref());
    }
}
