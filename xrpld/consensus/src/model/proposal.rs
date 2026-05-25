use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use protocol::{HashPrefix, Serializer};
use serde_json::{Value, json};

pub trait ConsensusHashable {
    fn append_consensus_bytes(&self, serializer: &mut Serializer);
}

impl ConsensusHashable for u32 {
    fn append_consensus_bytes(&self, serializer: &mut Serializer) {
        serializer.add32(*self);
    }
}

impl ConsensusHashable for Uint256 {
    fn append_consensus_bytes(&self, serializer: &mut Serializer) {
        serializer.add_bit_string(*self);
    }
}

impl ConsensusHashable for NetClockTimePoint {
    fn append_consensus_bytes(&self, serializer: &mut Serializer) {
        serializer.add32(self.as_seconds());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusProposal<NodeId, LedgerId, Position> {
    previous_ledger: LedgerId,
    position: Position,
    close_time: NetClockTimePoint,
    seen_time: NetClockTimePoint,
    propose_seq: u32,
    node_id: NodeId,
    signing_hash: Option<Uint256>,
}

impl<NodeId, LedgerId, Position> ConsensusProposal<NodeId, LedgerId, Position> {
    pub const SEQ_JOIN: u32 = 0;
    pub const SEQ_LEAVE: u32 = u32::MAX;

    pub fn new(
        previous_ledger: LedgerId,
        propose_seq: u32,
        position: Position,
        close_time: NetClockTimePoint,
        now: NetClockTimePoint,
        node_id: NodeId,
    ) -> Self {
        Self {
            previous_ledger,
            position,
            close_time,
            seen_time: now,
            propose_seq,
            node_id,
            signing_hash: None,
        }
    }

    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    pub fn position(&self) -> &Position {
        &self.position
    }

    pub fn prev_ledger(&self) -> &LedgerId {
        &self.previous_ledger
    }

    pub fn propose_seq(&self) -> u32 {
        self.propose_seq
    }

    pub fn close_time(&self) -> NetClockTimePoint {
        self.close_time
    }

    pub fn seen_time(&self) -> NetClockTimePoint {
        self.seen_time
    }

    pub fn is_initial(&self) -> bool {
        self.propose_seq == Self::SEQ_JOIN
    }

    pub fn is_bow_out(&self) -> bool {
        self.propose_seq == Self::SEQ_LEAVE
    }

    pub fn is_stale(&self, cutoff: NetClockTimePoint) -> bool {
        self.seen_time <= cutoff
    }

    #[allow(clippy::implicit_saturating_add)]
    pub fn change_position(
        &mut self,
        new_position: Position,
        new_close_time: NetClockTimePoint,
        now: NetClockTimePoint,
    ) {
        self.signing_hash = None;
        self.position = new_position;
        self.close_time = new_close_time;
        self.seen_time = now;
        if self.propose_seq != Self::SEQ_LEAVE {
            self.propose_seq += 1;
        }
    }

    pub fn bow_out(&mut self, now: NetClockTimePoint) {
        self.signing_hash = None;
        self.seen_time = now;
        self.propose_seq = Self::SEQ_LEAVE;
    }
}

impl<NodeId, LedgerId, Position> ConsensusProposal<NodeId, LedgerId, Position>
where
    LedgerId: ToString,
    Position: ToString,
    NodeId: ToString,
{
    pub fn render(&self) -> String {
        format!(
            "proposal: previous_ledger: {} proposal_seq: {} position: {} close_time: {} now: {} is_bow_out:{} node_id: {}",
            self.previous_ledger.to_string(),
            self.propose_seq,
            self.position.to_string(),
            self.close_time.as_seconds(),
            self.seen_time.as_seconds(),
            self.is_bow_out(),
            self.node_id.to_string()
        )
    }

    pub fn get_json(&self) -> Value {
        if self.is_bow_out() {
            json!({
                "previous_ledger": self.prev_ledger().to_string(),
                "close_time": self.close_time.as_seconds().to_string(),
            })
        } else {
            json!({
                "previous_ledger": self.prev_ledger().to_string(),
                "transaction_hash": self.position().to_string(),
                "propose_seq": self.propose_seq(),
                "close_time": self.close_time().as_seconds().to_string(),
            })
        }
    }
}

impl<NodeId, LedgerId, Position> ConsensusProposal<NodeId, LedgerId, Position>
where
    LedgerId: ConsensusHashable,
    Position: ConsensusHashable,
{
    pub fn append_signing_data(&self, serializer: &mut Serializer) {
        serializer.add32(HashPrefix::Proposal as u32);
        serializer.add32(self.propose_seq());
        self.close_time().append_consensus_bytes(serializer);
        self.prev_ledger().append_consensus_bytes(serializer);
        self.position().append_consensus_bytes(serializer);
    }

    pub fn signing_data(&self) -> Vec<u8> {
        let mut serializer = Serializer::new(128);
        self.append_signing_data(&mut serializer);
        serializer.data().to_vec()
    }

    pub fn signing_hash(&mut self) -> Uint256 {
        if let Some(hash) = self.signing_hash {
            return hash;
        }
        let mut serializer = Serializer::new(128);
        self.append_signing_data(&mut serializer);
        let hash = serializer.get_sha512_half();
        self.signing_hash = Some(hash);
        hash
    }
}

#[cfg(test)]
mod tests {
    use basics::{base_uint::Uint256, chrono::NetClockTimePoint};
    use protocol::sha512_half;

    use super::ConsensusProposal;

    #[test]
    fn signing_data_hash_matches_cached_signing_hash() {
        let mut proposal = ConsensusProposal::new(
            Uint256::from_u64(11),
            3,
            Uint256::from_u64(22),
            NetClockTimePoint::new(44),
            NetClockTimePoint::new(55),
            7u32,
        );

        let data = proposal.signing_data();
        assert_eq!(proposal.signing_hash(), sha512_half(&data));
    }
}
