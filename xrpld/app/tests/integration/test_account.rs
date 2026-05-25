//! Test account — mirrors C++ `test::jtx::Account`.

use basics::base_uint::{Uint160, Uint256};
use protocol::AccountID;

/// A named test account with a deterministic keypair derived from the name.
#[derive(Debug, Clone)]
pub struct TestAccount {
    pub name: String,
    pub id: AccountID,
    pub sequence: u32,
}

impl TestAccount {
    /// Create a test account from a human-readable name.
    /// The account ID is derived deterministically from the name (same as C++ Account("alice")).
    pub fn new(name: &str) -> Self {
        // Deterministic account ID from name — use SHA-256 of name truncated to 20 bytes
        let hash = sha2_hash(name.as_bytes());
        let id = AccountID::from_array(hash);
        Self {
            name: name.to_owned(),
            id,
            sequence: 1,
        }
    }

    pub fn id_160(&self) -> Uint160 {
        Uint160::from_slice(self.id.data()).expect("account width")
    }

    pub fn next_seq(&mut self) -> u32 {
        let seq = self.sequence;
        self.sequence += 1;
        seq
    }
}

fn sha2_hash(data: &[u8]) -> [u8; 20] {
    use sha2::{Digest, Sha256};
    let full = Sha256::digest(data);
    let mut result = [0u8; 20];
    result.copy_from_slice(&full[..20]);
    result
}
