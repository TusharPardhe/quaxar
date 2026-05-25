//! Peer's signed consensus proposal wrapper.
//!

use basics::base_uint::Uint256;
use serde_json::{Value as JsonValue, json};
use sha2::{Digest, Sha512};

/// A consensus proposal's core fields.
#[derive(Clone, Debug)]
pub struct Proposal {
    pub propose_seq: u32,
    pub close_time: u32,
    pub prev_ledger: Uint256,
    pub position: Uint256,
}

/// A peer's signed, proposed position for use in RCLConsensus.
#[derive(Clone, Debug)]
pub struct RCLCxPeerPos {
    pub public_key: Vec<u8>,
    pub suppression: Uint256,
    pub proposal: Proposal,
    signature: Vec<u8>,
}

impl RCLCxPeerPos {
    /// Maximum signature size (72 bytes for DER-encoded secp256k1).
    const MAX_SIGNATURE_SIZE: usize = 72;

    pub fn new(
        public_key: Vec<u8>,
        signature: &[u8],
        suppression: Uint256,
        proposal: Proposal,
    ) -> Self {
        assert!(
            !signature.is_empty() && signature.len() <= Self::MAX_SIGNATURE_SIZE,
            "invalid signature size"
        );
        Self {
            public_key,
            suppression,
            proposal,
            signature: signature.to_vec(),
        }
    }

    pub fn signature(&self) -> &[u8] {
        &self.signature
    }

    pub fn suppression_id(&self) -> &Uint256 {
        &self.suppression
    }

    /// Verify the signing hash of the proposal.
    pub fn check_sign(&self) -> bool {
        if self.public_key.is_empty() || self.signature.is_empty() {
            return false;
        }
        // Build the signing data: the suppression hash is the signing hash
        let Ok(pk) = protocol::PublicKey::from_slice(&self.public_key) else {
            return false;
        };
        protocol::verify(&pk, self.suppression.data(), &self.signature)
    }

    pub fn get_json(&self) -> JsonValue {
        json!({
            "propose_seq": self.proposal.propose_seq,
            "close_time": self.proposal.close_time,
        })
    }
}

/// Calculate a unique identifier for a signed proposal.
///
pub fn proposal_unique_id(
    propose_hash: &Uint256,
    previous_ledger: &Uint256,
    propose_seq: u32,
    close_time: u32,
    public_key: &[u8],
    signature: &[u8],
) -> Uint256 {
    let mut hasher = Sha512::new();
    hasher.update(propose_hash.data());
    hasher.update(previous_ledger.data());
    hasher.update(propose_seq.to_be_bytes());
    hasher.update(close_time.to_be_bytes());
    // VL-encoded: length prefix + data
    let pk_len = public_key.len() as u8;
    hasher.update([pk_len]);
    hasher.update(public_key);
    let sig_len = signature.len() as u8;
    hasher.update([sig_len]);
    hasher.update(signature);

    let result = hasher.finalize();
    Uint256::from_slice(&result[..32]).expect("SHA-512 half should fit in 32 bytes")
}
