//! Rust port of `xrpl/basics/SHAMapHash.h`.
//!
//! This stays intentionally thin: the reference type is mostly a domain wrapper
//! around `uint256` with a custom partition extractor.

use crate::base_uint::{Uint256, to_string as uint256_to_string};
use crate::partitioned_unordered_map::PartitionKey;
use std::fmt;
use std::hash::{Hash, Hasher};

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct SHAMapHash {
    hash: Uint256,
}

impl SHAMapHash {
    pub fn new(hash: Uint256) -> Self {
        Self { hash }
    }

    pub fn as_uint256(&self) -> &Uint256 {
        &self.hash
    }

    pub fn as_uint256_mut(&mut self) -> &mut Uint256 {
        &mut self.hash
    }

    pub fn is_zero(&self) -> bool {
        self.hash.is_zero()
    }

    pub fn is_non_zero(&self) -> bool {
        self.hash.is_non_zero()
    }

    pub fn signum(&self) -> i32 {
        self.hash.signum()
    }

    pub fn zero(&mut self) {
        self.hash = Uint256::zero();
    }
}

impl Hash for SHAMapHash {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl fmt::Display for SHAMapHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&uint256_to_string(&self.hash))
    }
}

impl fmt::Debug for SHAMapHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SHAMapHash")
            .field(&self.to_string())
            .finish()
    }
}

impl PartitionKey for SHAMapHash {
    fn partition_key(&self) -> usize {
        self.hash.partition_key()
    }
}

#[cfg(test)]
mod tests {
    use super::SHAMapHash;
    use crate::base_uint::Uint256;
    use crate::partitioned_unordered_map::PartitionKey;

    #[test]
    fn wrapper_behavior_role() {
        let hash =
            Uint256::from_hex("0102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F20")
                .expect("hex should parse");
        let wrapped = SHAMapHash::new(hash);

        assert_eq!(wrapped.as_uint256(), &hash);
        assert!(wrapped.is_non_zero());
        assert!(!wrapped.is_zero());
        assert_eq!(wrapped.signum(), 1);
        assert_eq!(
            wrapped.to_string(),
            "0102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F20"
        );
        assert_eq!(wrapped.partition_key(), hash.partition_key());
    }
}
