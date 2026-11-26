//! Deterministic test accounts.
//!
//! Every key comes from one master tag and an index, so each load host derives
//! the same accounts without sharing files:
//! seed(i) = first 16 bytes of SHA-512-half(tag || i), ed25519.

use protocol::{AccountID, KeyType, PublicKey, SecretKey, Seed, calc_account_id, generate_key_pair};

/// Signs as the forked rich account (its RegularKey).
pub const FUNDER_KEY: u32 = u32::MAX;
pub const ISSUER: u32 = 0;
pub const MARKET_MAKER: u32 = 1;
pub const FIRST_HUB: u32 = 2;
pub const HUBS: u32 = 1000;
/// First test account. Test account `n` (0-based) has index `FIRST_TEST + n`.
pub const FIRST_TEST: u32 = FIRST_HUB + HUBS;

/// Currencies issued by the issuer. Eight, so a path of 8 steps can visit
/// each once (a currency may not repeat within a path).
pub const CURRENCIES: [&str; 8] = ["USD", "EUR", "JPY", "GBP", "CHF", "AUD", "CAD", "SGD"];

pub struct Key {
    pub account: AccountID,
    pub public: PublicKey,
    pub secret: SecretKey,
}

pub fn derive(tag: &str, index: u32) -> Key {
    let mut input = tag.as_bytes().to_vec();
    input.extend_from_slice(&index.to_le_bytes());
    let digest = protocol::sha512_half(&input);
    let seed = Seed::from_slice(&digest.data()[..16]).expect("16 bytes");
    let (public, secret) = generate_key_pair(KeyType::Ed25519, &seed).expect("ed25519 key");
    Key {
        account: calc_account_id(public.as_bytes()),
        public,
        secret,
    }
}

/// Derive `range` on all cores.
pub fn derive_range(tag: &str, range: std::ops::Range<u32>) -> Vec<Key> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let len = range.len();
    let chunk = len.div_ceil(threads).max(1);
    let start = range.start;
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let lo = start + (t * chunk).min(len) as u32;
                let hi = start + ((t + 1) * chunk).min(len) as u32;
                scope.spawn(move || (lo..hi).map(|i| derive(tag, i)).collect::<Vec<_>>())
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("derive thread"))
            .collect()
    })
}

/// Test accounts by role.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub tests: u32,
    /// The first `iou_group` test accounts hold trust lines for all
    /// currencies and a USD balance. They send IOU-direct payments and
    /// receive IOU and path payments.
    pub iou_group: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivation_is_deterministic_and_distinct() {
        let a = derive("t", 5);
        let b = derive("t", 5);
        let c = derive("t", 6);
        let d = derive("u", 5);
        assert_eq!(a.account, b.account);
        assert_ne!(a.account, c.account);
        assert_ne!(a.account, d.account);
        assert_eq!(a.public.as_bytes()[0], 0xED);
    }

    #[test]
    fn derive_range_matches_single_derivation() {
        let keys = derive_range("t", 10..37);
        assert_eq!(keys.len(), 27);
        for (offset, key) in keys.iter().enumerate() {
            assert_eq!(key.account, derive("t", 10 + offset as u32).account);
        }
    }
}
