//! Rust port of `xrpl/basics/partitioned_unordered_map.h`.
//!
//! The reference type is not "just another hash map." It stores several internal
//! maps, routes keys to partitions, and exposes the partitions directly for
//! callers like `TaggedCache`.

use crate::hardened_hash::{HashBuilder, hash_one};
use std::borrow::Borrow;
use std::collections::HashMap as StdHashMap;
use std::hash::{BuildHasher, Hash};

/// Extract the partition key the same way the reference helper does.
///
/// The default reference template uses the key value directly for integer-like keys.
/// It specializes `std::string` to hash the string with `uhash`.
/// Future ports such as `uint256` or `SHAMapHash` can implement this trait
/// explicitly once those types exist on the Rust side.
pub trait PartitionKey {
    fn partition_key(&self) -> usize;
}

macro_rules! impl_partition_key_unsigned {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl PartitionKey for $ty {
                fn partition_key(&self) -> usize {
                    *self as usize
                }
            }
        )+
    };
}

macro_rules! impl_partition_key_signed {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl PartitionKey for $ty {
                fn partition_key(&self) -> usize {
                    *self as usize
                }
            }
        )+
    };
}

impl_partition_key_unsigned!(u8, u16, u32, u64, u128, usize);
impl_partition_key_signed!(i8, i16, i32, i64, i128, isize);

impl PartitionKey for str {
    fn partition_key(&self) -> usize {
        hash_one(self) as usize
    }
}

impl PartitionKey for String {
    fn partition_key(&self) -> usize {
        self.as_str().partition_key()
    }
}

impl<T> PartitionKey for &T
where
    T: PartitionKey + ?Sized,
{
    fn partition_key(&self) -> usize {
        (*self).partition_key()
    }
}

#[derive(Clone, Debug)]
pub struct PartitionedUnorderedMap<K, V, S = HashBuilder> {
    partitions: usize,
    maps: Vec<StdHashMap<K, V, S>>,
}

impl<K, V> PartitionedUnorderedMap<K, V, HashBuilder>
where
    K: Eq + Hash + PartitionKey,
{
    pub fn new(partitions: Option<usize>) -> Self {
        Self::with_hasher(partitions, HashBuilder::default())
    }
}

impl<K, V, S> PartitionedUnorderedMap<K, V, S>
where
    K: Eq + Hash + PartitionKey,
    S: BuildHasher + Clone,
{
    pub fn with_hasher(partitions: Option<usize>, hasher: S) -> Self {
        let partitions = normalize_partitions(partitions);
        let maps = (0..partitions)
            .map(|_| StdHashMap::with_hasher(hasher.clone()))
            .collect();

        Self { partitions, maps }
    }

    pub fn partitions(&self) -> usize {
        self.partitions
    }

    pub fn map(&self) -> &[StdHashMap<K, V, S>] {
        &self.maps
    }

    pub fn map_mut(&mut self) -> &mut [StdHashMap<K, V, S>] {
        &mut self.maps
    }

    pub fn len(&self) -> usize {
        self.maps.iter().map(StdHashMap::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&mut self) {
        for partition in &mut self.maps {
            partition.clear();
        }
    }

    pub fn partition_for<Q>(&self, key: &Q) -> usize
    where
        K: Borrow<Q>,
        Q: Eq + Hash + PartitionKey + ?Sized,
    {
        key.partition_key() % self.partitions
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Eq + Hash + PartitionKey + ?Sized,
    {
        self.get(key).is_some()
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + PartitionKey + ?Sized,
    {
        self.maps[self.partition_for(key)].get(key)
    }

    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + PartitionKey + ?Sized,
    {
        let partition = self.partition_for(key);
        self.maps[partition].get_mut(key)
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let partition = self.partition_for(&key);
        self.maps[partition].insert(key, value)
    }

    pub fn get_or_insert_with(&mut self, key: K, make_value: impl FnOnce() -> V) -> &mut V {
        let partition = self.partition_for(&key);
        self.maps[partition].entry(key).or_insert_with(make_value)
    }

    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + PartitionKey + ?Sized,
    {
        let partition = self.partition_for(key);
        self.maps[partition].remove(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.maps.iter().flat_map(StdHashMap::iter)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        self.maps.iter_mut().flat_map(StdHashMap::iter_mut)
    }
}

impl<K, V, S> Default for PartitionedUnorderedMap<K, V, S>
where
    K: Eq + Hash + PartitionKey,
    S: BuildHasher + Clone + Default,
{
    fn default() -> Self {
        Self::with_hasher(None, S::default())
    }
}

fn normalize_partitions(partitions: Option<usize>) -> usize {
    match partitions {
        Some(value) if value > 0 => value,
        _ => std::thread::available_parallelism()
            .map(|threads| threads.get())
            .unwrap_or(1),
    }
}

#[cfg(test)]
mod tests {
    use super::PartitionedUnorderedMap;
    use crate::hardened_hash::HardenedHashBuilder;

    #[test]
    fn defaults_to_a_non_zero_partition_count() {
        let map = PartitionedUnorderedMap::<u32, u32>::new(Some(0));
        assert!(map.partitions() > 0);
    }

    #[test]
    fn integer_keys_route_by_modulo() {
        let mut map = PartitionedUnorderedMap::<u32, &'static str>::new(Some(4));
        map.insert(6, "ledger");

        assert_eq!(map.partition_for(&6), 2);
        assert_eq!(map.map()[2].get(&6), Some(&"ledger"));
        assert_eq!(map.get(&6), Some(&"ledger"));
    }

    #[test]
    fn string_keys_route_by_plain_hash_extract_rule() {
        let mut map = PartitionedUnorderedMap::<String, usize>::new(Some(8));
        let key = String::from("validators");
        let partition = map.partition_for(key.as_str());
        map.insert(key.clone(), 1);

        assert_eq!(map.map()[partition].get("validators"), Some(&1));
        assert_eq!(map.get("validators"), Some(&1));
    }

    #[test]
    fn hardened_hasher_can_back_partition_maps_without_changing_partition_api() {
        let mut map = PartitionedUnorderedMap::<String, usize, HardenedHashBuilder>::with_hasher(
            Some(4),
            HardenedHashBuilder::from_seed(9),
        );

        assert_eq!(map.insert(String::from("cache"), 2), None);
        assert_eq!(map.insert(String::from("cache"), 3), Some(2));
        assert_eq!(map.get("cache"), Some(&3));
    }

    #[test]
    fn iteration_visits_all_partitions() {
        let mut map = PartitionedUnorderedMap::<u32, u32>::new(Some(3));
        map.insert(0, 10);
        map.insert(1, 11);
        map.insert(2, 12);

        let mut values = map.iter().map(|(_, value)| *value).collect::<Vec<_>>();
        values.sort_unstable();

        assert_eq!(values, vec![10, 11, 12]);
        assert_eq!(map.len(), 3);
    }
}
