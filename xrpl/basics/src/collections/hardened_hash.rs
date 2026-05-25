//! Rust port of `xrpl/basics/hardened_hash.h`.
//!
//! The reference code has two distinct hashing policies:
//! - plain `uhash`, which is unseeded `xxh3`,
//! - `hardened_hash`, which seeds the hasher once per container instance.
//!
//! In Rust that split belongs in explicit `BuildHasher` types so callers can
//! see which policy they are choosing.

use rand::RngCore;
use rand::rngs::OsRng;
use std::hash::{BuildHasher, Hash};
use xxhash_rust::xxh3::{Xxh3, Xxh3Builder};

/// Mirrors the plain `beast::uhash<>` policy used by `hash_map` and `hash_set`.
pub type HashBuilder = Xxh3Builder;

/// Mirrors the strong hashing algorithm chosen by reference for hardened containers.
pub type StrongHash = Xxh3;

/// Hash one value with the plain unseeded hash policy.
pub fn hash_one<T>(value: T) -> u64
where
    T: Hash,
{
    HashBuilder::default().hash_one(value)
}

/// Seeded hasher builder used by hardened unordered containers.
///
/// Each builder captures one seed. That matches the reference shape where a
/// `hardened_hash` functor seeds itself once at construction time and then
/// reuses that seed for each hash operation performed by the container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HardenedHashBuilder {
    seed: u64,
}

impl HardenedHashBuilder {
    /// Create a builder with an OS-random seed.
    pub fn new() -> Self {
        Self::from_seed(random_seed())
    }

    /// Create a builder with a known seed.
    ///
    /// This is mainly useful for tests, fixtures, and compatibility probes.
    pub const fn from_seed(seed: u64) -> Self {
        Self { seed }
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    pub fn hash_one<T>(self, value: T) -> u64
    where
        T: Hash,
    {
        BuildHasher::hash_one(&self, value)
    }
}

impl Default for HardenedHashBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildHasher for HardenedHashBuilder {
    type Hasher = StrongHash;

    fn build_hasher(&self) -> Self::Hasher {
        StrongHash::with_seed(self.seed)
    }
}

fn random_seed() -> u64 {
    let mut rng = OsRng;
    rng.next_u64()
}

#[cfg(test)]
mod tests {
    use super::{HardenedHashBuilder, hash_one};

    #[test]
    fn plain_hash_builder_is_deterministic_for_the_same_input() {
        let first = hash_one("ledger");
        let second = hash_one("ledger");
        assert_eq!(first, second);
    }

    #[test]
    fn hardened_builder_preserves_its_seed() {
        let builder = HardenedHashBuilder::from_seed(7);
        assert_eq!(builder.seed(), 7);
        assert_eq!(builder.hash_one("ledger"), builder.hash_one("ledger"));
    }

    #[test]
    fn hardened_hashing_changes_when_the_seed_changes() {
        let first = HardenedHashBuilder::from_seed(7).hash_one("ledger");
        let second = HardenedHashBuilder::from_seed(11).hash_one("ledger");
        assert_ne!(first, second);
    }
}
