//! A proposed position taken during a round of consensus.
//!
//! Ported from rippled's `ConsensusProposal.h`. During consensus, peers seek
//! agreement on a set of transactions to apply to the prior ledger. Each
//! peer's position on that set is communicated to its peers as an instance
//! of [`ConsensusProposal`]. An instance can represent either our own
//! proposal or one received from a peer.
//!
//! As consensus proceeds, a peer may change its position, or choose to
//! abstain ("bow out"). Each successive proposal from a given peer carries a
//! strictly monotonically increasing sequence number, or the special
//! sentinel [`ConsensusProposal::SEQ_LEAVE`] once that peer has bowed out.

use basics::chrono::NetClockTimePoint;

/// A proposed position taken during a round of consensus.
///
/// Type parameters mirror the reference's template parameters:
/// - `NodeId`: uniquely identifies the peer/node taking this position.
/// - `LedgerId`: uniquely identifies a ledger.
/// - `Position`: the type used to represent the position taken on the
///   transaction set under consideration (typically a tx-set digest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusProposal<NodeId, LedgerId, Position> {
    /// Unique identifier of the prior ledger this proposal builds on.
    /// `previousLedger_` in the reference.
    previous_ledger: LedgerId,
    /// The position taken on the transaction set. `position_`.
    position: Position,
    /// The close-time position for this proposal. `closeTime_`.
    close_time: NetClockTimePoint,
    /// When this position was last updated. `time_`.
    seen_time: NetClockTimePoint,
    /// The monotonically increasing sequence number of this proposal, or
    /// `SEQ_LEAVE` if bowed out. `proposeSeq_`.
    propose_seq: u32,
    /// The peer taking this position. `nodeID_`.
    node_id: NodeId,
}

impl<NodeId, LedgerId, Position> ConsensusProposal<NodeId, LedgerId, Position> {
    /// Sequence value when a peer initially joins consensus. `kSeqJoin`.
    pub const SEQ_JOIN: u32 = 0;

    /// Sequence number signaling a peer has bowed out of consensus.
    /// `kSeqLeave`.
    pub const SEQ_LEAVE: u32 = u32::MAX;

    /// Construct a new proposal.
    ///
    /// Matches the reference constructor's parameter order exactly:
    /// previous ledger, sequence, position, close time, seen time, node id.
    pub fn new(
        previous_ledger: LedgerId,
        propose_seq: u32,
        position: Position,
        close_time: NetClockTimePoint,
        seen_time: NetClockTimePoint,
        node_id: NodeId,
    ) -> Self {
        Self {
            previous_ledger,
            position,
            close_time,
            seen_time,
            propose_seq,
            node_id,
        }
    }

    /// The peer that took this position. `nodeID()`.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// The proposed position. `position()`.
    pub fn position(&self) -> &Position {
        &self.position
    }

    /// The prior accepted ledger this position builds on. `prevLedger()`.
    pub fn prev_ledger(&self) -> &LedgerId {
        &self.previous_ledger
    }

    /// The sequence number of this proposal. `proposeSeq()`.
    pub fn propose_seq(&self) -> u32 {
        self.propose_seq
    }

    /// The current close-time position. `closeTime()`.
    pub fn close_time(&self) -> NetClockTimePoint {
        self.close_time
    }

    /// When this position was taken. `seenTime()`.
    pub fn seen_time(&self) -> NetClockTimePoint {
        self.seen_time
    }

    /// Whether this is the first position taken this round. `isInitial()`.
    pub fn is_initial(&self) -> bool {
        self.propose_seq == Self::SEQ_JOIN
    }

    /// Whether this node left consensus. `isBowOut()`.
    pub fn is_bow_out(&self) -> bool {
        self.propose_seq == Self::SEQ_LEAVE
    }

    /// Whether this position is stale relative to `cutoff`. `isStale()`.
    pub fn is_stale(&self, cutoff: NetClockTimePoint) -> bool {
        self.seen_time <= cutoff
    }

    /// Update the position during consensus, incrementing the sequence
    /// number unless already bowed out. `changePosition()`.
    pub fn change_position(
        &mut self,
        new_position: Position,
        new_close_time: NetClockTimePoint,
        now: NetClockTimePoint,
    ) {
        self.position = new_position;
        self.close_time = new_close_time;
        self.seen_time = now;
        // Written as an explicit guard (rather than `saturating_add`) to
        // match the reference's `if (proposeSeq_ != kSeqLeave)
        // ++proposeSeq_;` literally, since this method's exact bow-out
        // guard behavior is part of what Phase 2's tests verify parity on.
        #[allow(clippy::implicit_saturating_add)]
        if self.propose_seq != Self::SEQ_LEAVE {
            self.propose_seq += 1;
        }
    }

    /// Leave consensus: mark this position as bowed out. `bowOut()`.
    pub fn bow_out(&mut self, now: NetClockTimePoint) {
        self.seen_time = now;
        self.propose_seq = Self::SEQ_LEAVE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(seconds: u32) -> NetClockTimePoint {
        NetClockTimePoint::new(seconds)
    }

    fn sample() -> ConsensusProposal<u32, u32, u32> {
        ConsensusProposal::new(100, ConsensusProposal::<u32, u32, u32>::SEQ_JOIN, 200, t(10), t(10), 7)
    }

    #[test]
    fn constructor_and_accessors_match_reference_field_order() {
        let p = sample();
        assert_eq!(*p.prev_ledger(), 100);
        assert_eq!(p.propose_seq(), 0);
        assert_eq!(*p.position(), 200);
        assert_eq!(p.close_time(), t(10));
        assert_eq!(p.seen_time(), t(10));
        assert_eq!(*p.node_id(), 7);
    }

    #[test]
    fn is_initial_true_only_at_seq_join() {
        let mut p = sample();
        assert!(p.is_initial());
        p.change_position(201, t(11), t(11));
        assert!(!p.is_initial());
    }

    #[test]
    fn is_bow_out_true_only_at_seq_leave() {
        let mut p = sample();
        assert!(!p.is_bow_out());
        p.bow_out(t(20));
        assert!(p.is_bow_out());
        assert_eq!(p.propose_seq(), ConsensusProposal::<u32, u32, u32>::SEQ_LEAVE);
    }

    #[test]
    fn is_stale_uses_seen_time_less_or_equal_cutoff() {
        let p = sample();
        assert!(p.is_stale(t(10))); // seen_time == cutoff -> stale
        assert!(p.is_stale(t(11))); // seen_time < cutoff -> stale
        assert!(!p.is_stale(t(9))); // seen_time > cutoff -> fresh
    }

    #[test]
    fn change_position_increments_sequence_and_updates_fields() {
        let mut p = sample();
        p.change_position(300, t(15), t(16));
        assert_eq!(p.propose_seq(), 1);
        assert_eq!(*p.position(), 300);
        assert_eq!(p.close_time(), t(15));
        assert_eq!(p.seen_time(), t(16));

        p.change_position(301, t(17), t(18));
        assert_eq!(p.propose_seq(), 2);
    }

    #[test]
    fn change_position_after_bow_out_does_not_resume_sequence() {
        let mut p = sample();
        p.bow_out(t(20));
        assert_eq!(p.propose_seq(), ConsensusProposal::<u32, u32, u32>::SEQ_LEAVE);
        // Reference: changePosition only increments "if not already bowed
        // out" -- once at kSeqLeave, further changePosition calls leave the
        // sequence at kSeqLeave (still bowed out), matching the C++ guard
        // `if (proposeSeq_ != kSeqLeave) ++proposeSeq_;`.
        p.change_position(400, t(21), t(21));
        assert_eq!(p.propose_seq(), ConsensusProposal::<u32, u32, u32>::SEQ_LEAVE);
        assert!(p.is_bow_out());
    }

    #[test]
    fn bow_out_sets_seq_leave_and_updates_seen_time() {
        let mut p = sample();
        p.bow_out(t(99));
        assert_eq!(p.propose_seq(), ConsensusProposal::<u32, u32, u32>::SEQ_LEAVE);
        assert_eq!(p.seen_time(), t(99));
    }

    #[test]
    fn seq_join_and_seq_leave_sentinel_values_match_reference() {
        assert_eq!(ConsensusProposal::<u32, u32, u32>::SEQ_JOIN, 0);
        assert_eq!(ConsensusProposal::<u32, u32, u32>::SEQ_LEAVE, 0xFFFF_FFFF);
    }
}
