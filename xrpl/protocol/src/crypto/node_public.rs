//! `NodePublic` base58 helpers matching the XRPL token-prefix and checksum
//! rules used by reference `parseBase58<PublicKey>(TokenType::NodePublic, ...)`.

use bs58::Alphabet;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

const XRPL_BASE58_ALPHABET: &[u8; 58] =
    b"rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz";
const NODE_PUBLIC_TOKEN_TYPE: u8 = 28;
pub const NODE_PUBLIC_KEY_LEN: usize = 33;

pub type NodePublicKey = [u8; NODE_PUBLIC_KEY_LEN];

pub fn encode_node_public_base58(public_key: NodePublicKey) -> String {
    let mut payload = Vec::with_capacity(1 + public_key.len() + 4);
    payload.push(NODE_PUBLIC_TOKEN_TYPE);
    payload.extend_from_slice(&public_key);
    payload.extend_from_slice(&checksum(&payload));
    bs58::encode(payload)
        .with_alphabet(xrpl_base58_alphabet())
        .into_string()
}

pub fn parse_base58_node_public(value: &str) -> Option<NodePublicKey> {
    let decoded = bs58::decode(value)
        .with_alphabet(xrpl_base58_alphabet())
        .into_vec()
        .ok()?;
    if decoded.len() != 1 + NODE_PUBLIC_KEY_LEN + 4 {
        return None;
    }
    if decoded[0] != NODE_PUBLIC_TOKEN_TYPE {
        return None;
    }

    let checksum_offset = 1 + NODE_PUBLIC_KEY_LEN;
    let expected = checksum(&decoded[..checksum_offset]);
    if decoded[checksum_offset..] != expected {
        return None;
    }

    decoded[1..checksum_offset].try_into().ok()
}

fn checksum(message: &[u8]) -> [u8; 4] {
    let first = Sha256::digest(message);
    let second = Sha256::digest(first);
    let mut checksum = [0u8; 4];
    checksum.copy_from_slice(&second[..4]);
    checksum
}

fn xrpl_base58_alphabet() -> &'static Alphabet {
    static ALPHABET: OnceLock<Alphabet> = OnceLock::new();
    ALPHABET.get_or_init(|| {
        Alphabet::new(XRPL_BASE58_ALPHABET).expect("XRPL base58 alphabet should remain valid")
    })
}

#[cfg(test)]
mod tests {
    use super::{NodePublicKey, encode_node_public_base58, parse_base58_node_public};

    const CPP_NODE_PUBLIC_KEY: NodePublicKey = [
        0x03, 0xD4, 0x9C, 0x56, 0xE1, 0xB1, 0x85, 0xF1, 0xBE, 0x89, 0x9A, 0xE6, 0x6A, 0x02, 0xEF,
        0xC1, 0x7F, 0x78, 0xEA, 0x6F, 0xC5, 0x3A, 0xF8, 0x5E, 0x0F, 0xE5, 0x4C, 0x6E, 0x8B, 0x7F,
        0x8C, 0x71, 0xA8,
    ];

    #[test]
    fn node_public_base58_vector() {
        assert_eq!(
            encode_node_public_base58(CPP_NODE_PUBLIC_KEY),
            "n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9"
        );
    }

    #[test]
    fn node_public_base58_round_trips() {
        let encoded = encode_node_public_base58(CPP_NODE_PUBLIC_KEY);
        assert_eq!(
            parse_base58_node_public(&encoded),
            Some(CPP_NODE_PUBLIC_KEY)
        );
        assert_eq!(parse_base58_node_public("abcdef12345"), None);
    }
}
