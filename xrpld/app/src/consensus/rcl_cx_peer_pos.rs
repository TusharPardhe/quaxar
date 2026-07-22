//! A peer's signed consensus proposal. Ported from `RCLCxPeerPos.h`.

use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use consensus::model::ConsensusProposal;
use protocol::{PublicKey, sha512_half, sign_digest, verify_digest};

/// The RCL consensus proposal type, keyed by the proposer's `PublicKey`
/// (matching `component_runtime.rs`'s `PendingProposal` and
/// `network_ops_runtime.rs`'s `peer_proposal` contract -- the reference's
/// `NodeID` template parameter is instantiated with the full public key
/// here rather than the 160-bit node id hash).
pub type Proposal = ConsensusProposal<PublicKey, Uint256, Uint256>;

const MAX_SIGNATURE_SIZE: usize = 72;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RclCxPeerPos {
    public_key: PublicKey,
    suppression: Uint256,
    proposal: Proposal,
    signature: Vec<u8>,
}

impl RclCxPeerPos {
    pub fn new(
        public_key: PublicKey,
        signature: Vec<u8>,
        suppression: Uint256,
        proposal: Proposal,
    ) -> Self {
        assert!(
            !signature.is_empty() && signature.len() <= MAX_SIGNATURE_SIZE,
            "RclCxPeerPos::new: invalid signature length"
        );
        Self {
            public_key,
            suppression,
            proposal,
            signature,
        }
    }

    pub fn signature(&self) -> &[u8] {
        &self.signature
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.public_key
    }

    pub fn suppression_id(&self) -> Uint256 {
        self.suppression
    }

    pub fn proposal(&self) -> &Proposal {
        &self.proposal
    }

    pub fn check_sign(&self) -> bool {
        verify_digest(&self.public_key, self.signing_hash(), &self.signature, true)
    }

    fn signing_hash(&self) -> Uint256 {
        proposal_signing_hash(
            self.proposal.propose_seq(),
            self.proposal.close_time(),
            self.proposal.prev_ledger(),
            self.proposal.position(),
        )
    }
}

/// The signing hash for a proposal's `(seq, close_time, prev_ledger,
/// position)` tuple, matching `RCLCxPeerPos::hash_append`'s field order
/// (prefixed with the `HashPrefix::Proposal` equivalent).
fn proposal_signing_hash(
    propose_seq: u32,
    close_time: NetClockTimePoint,
    prev_ledger: &Uint256,
    position: &Uint256,
) -> Uint256 {
    let mut data = Vec::with_capacity(4 + 4 + 4 + 32 + 32);
    data.extend_from_slice(&0x5052_4F50u32.to_be_bytes());
    data.extend_from_slice(&propose_seq.to_be_bytes());
    data.extend_from_slice(&close_time.as_seconds().to_be_bytes());
    data.extend_from_slice(prev_ledger.data());
    data.extend_from_slice(position.data());
    sha512_half(&data)
}

/// Sign a new proposal, producing the `(signature, suppression_id)` pair
/// needed to construct an [`RclCxPeerPos`] via [`RclCxPeerPos::new`].
/// Matches the reference's `proposalUniqueId` (used as the suppression id)
/// plus the actual signing step performed at the call site in the C++
/// consensus adaptor.
pub fn sign_proposal(
    secret_key: &protocol::SecretKey,
    public_key: &PublicKey,
    proposal: &Proposal,
) -> Result<(Vec<u8>, Uint256), protocol::SignError> {
    let signing_hash = proposal_signing_hash(
        proposal.propose_seq(),
        proposal.close_time(),
        proposal.prev_ledger(),
        proposal.position(),
    );
    let signature = sign_digest(public_key, secret_key, signing_hash)?;
    let suppression = proposal_unique_id(
        proposal.position(),
        proposal.prev_ledger(),
        proposal.propose_seq(),
        proposal.close_time(),
        public_key.as_bytes(),
        &signature,
    );
    Ok((signature, suppression))
}

/// Calculate a unique identifier for a signed proposal, used for hash
/// router suppression of duplicate relays. Matches `proposalUniqueId`.
pub fn proposal_unique_id(
    propose_hash: &Uint256,
    previous_ledger: &Uint256,
    propose_seq: u32,
    close_time: NetClockTimePoint,
    public_key: &[u8],
    signature: &[u8],
) -> Uint256 {
    let mut data = Vec::with_capacity(32 + 32 + 4 + 4 + 1 + public_key.len() + 1 + signature.len());
    data.extend_from_slice(propose_hash.data());
    data.extend_from_slice(previous_ledger.data());
    data.extend_from_slice(&propose_seq.to_be_bytes());
    data.extend_from_slice(&close_time.as_seconds().to_be_bytes());
    data.push(public_key.len() as u8);
    data.extend_from_slice(public_key);
    data.push(signature.len() as u8);
    data.extend_from_slice(signature);
    sha512_half(&data)
}

impl consensus::algorithm::PeerPosition<PublicKey, Uint256, Uint256> for RclCxPeerPos {
    fn proposal(&self) -> &Proposal {
        &self.proposal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{KeyType, derive_public_key, generate_secret_key, random_seed};

    fn keypair() -> (protocol::SecretKey, PublicKey) {
        let seed = random_seed();
        let secret_key = generate_secret_key(KeyType::Secp256k1, &seed)
            .expect("secret key generation should succeed");
        let public_key = derive_public_key(KeyType::Secp256k1, &secret_key)
            .expect("public key derivation should succeed");
        (secret_key, public_key)
    }

    fn sample_proposal(node_id: PublicKey) -> Proposal {
        let now = NetClockTimePoint::new(1000);
        Proposal::new(
            Uint256::from_slice(&[1; 32]).unwrap(),
            1,
            Uint256::from_slice(&[2; 32]).unwrap(),
            now,
            now,
            node_id,
        )
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let (secret_key, public_key) = keypair();
        let proposal = sample_proposal(public_key);

        let (signature, suppression) =
            sign_proposal(&secret_key, &public_key, &proposal).expect("signing should succeed");
        let peer_pos = RclCxPeerPos::new(public_key, signature, suppression, proposal);

        assert!(peer_pos.check_sign());
    }

    #[test]
    fn tampered_signature_fails_verification() {
        let (secret_key, public_key) = keypair();
        let proposal = sample_proposal(public_key);

        let (mut signature, suppression) =
            sign_proposal(&secret_key, &public_key, &proposal).expect("signing should succeed");
        *signature.last_mut().unwrap() ^= 0xFF;
        let peer_pos = RclCxPeerPos::new(public_key, signature, suppression, proposal);

        assert!(!peer_pos.check_sign());
    }

    #[test]
    fn suppression_id_changes_with_signature() {
        let (secret_key, public_key) = keypair();
        let proposal_a = sample_proposal(public_key);
        let mut proposal_b = sample_proposal(public_key);
        proposal_b.change_position(
            Uint256::from_slice(&[9; 32]).unwrap(),
            NetClockTimePoint::new(1001),
            NetClockTimePoint::new(1001),
        );

        let (_, suppression_a) = sign_proposal(&secret_key, &public_key, &proposal_a).unwrap();
        let (_, suppression_b) = sign_proposal(&secret_key, &public_key, &proposal_b).unwrap();

        assert_ne!(suppression_a, suppression_b);
    }
}
