//! Rust migration boundary for `xrpl/basics/UnorderedContainers.h`.
//!
//! We mirror the reference split explicitly:
//! - plain unordered aliases,
//! - hardened seeded aliases,
//! - partitioned hardened aliases for cache-oriented callers,
//! - duplicate-friendly multimap and multiset wrappers.

use crate::hardened_hash::{HardenedHashBuilder, HashBuilder};
use crate::partitioned_unordered_map::PartitionedUnorderedMap;
use std::borrow::Borrow;
use std::collections::{HashMap as StdHashMap, HashSet as StdHashSet};
use std::hash::{BuildHasher, Hash};

pub type HashMap<K, V> = StdHashMap<K, V, HashBuilder>;
pub type HashSet<T> = StdHashSet<T, HashBuilder>;

pub type HardenedHashMap<K, V> = StdHashMap<K, V, HardenedHashBuilder>;
pub type HardenedHashSet<T> = StdHashSet<T, HardenedHashBuilder>;

pub type HardenedPartitionedHashMap<K, V> = PartitionedUnorderedMap<K, V, HardenedHashBuilder>;

#[derive(Clone, Debug)]
pub struct UnorderedMultimap<K, V, S = HashBuilder> {
    entries: StdHashMap<K, Vec<V>, S>,
}

pub type HashMultimap<K, V> = UnorderedMultimap<K, V, HashBuilder>;
pub type HardenedHashMultimap<K, V> = UnorderedMultimap<K, V, HardenedHashBuilder>;

impl<K, V> UnorderedMultimap<K, V, HashBuilder>
where
    K: Eq + Hash,
{
    pub fn new() -> Self {
        Self::default()
    }
}

impl<K, V, S> UnorderedMultimap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher + Clone,
{
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            entries: StdHashMap::with_hasher(hasher),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn count<Q>(&self, key: &Q) -> usize
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.entries.get(key).map_or(0, Vec::len)
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.count(key) > 0
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&[V]>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.entries.get(key).map(Vec::as_slice)
    }

    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut [V]>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.entries.get_mut(key).map(Vec::as_mut_slice)
    }

    pub fn insert(&mut self, key: K, value: V) {
        self.entries.entry(key).or_default().push(value);
    }

    pub fn remove_one<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        let (value, remove_key) = {
            let values = self.entries.get_mut(key)?;
            let value = values.pop()?;
            (value, values.is_empty())
        };

        if remove_key {
            let _ = self.entries.remove(key);
        }

        Some(value)
    }

    pub fn remove_all<Q>(&mut self, key: &Q) -> usize
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.entries.remove(key).map_or(0, |values| values.len())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries
            .iter()
            .flat_map(|(key, values)| values.iter().map(move |value| (key, value)))
    }
}

impl<K, V, S> Default for UnorderedMultimap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher + Clone + Default,
{
    fn default() -> Self {
        Self::with_hasher(S::default())
    }
}

#[derive(Clone, Debug)]
pub struct UnorderedMultiset<T, S = HashBuilder> {
    entries: StdHashMap<T, usize, S>,
}

pub type HashMultiset<T> = UnorderedMultiset<T, HashBuilder>;
pub type HardenedHashMultiset<T> = UnorderedMultiset<T, HardenedHashBuilder>;

impl<T> UnorderedMultiset<T, HashBuilder>
where
    T: Eq + Hash,
{
    pub fn new() -> Self {
        Self::default()
    }
}

impl<T, S> UnorderedMultiset<T, S>
where
    T: Eq + Hash,
    S: BuildHasher + Clone,
{
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            entries: StdHashMap::with_hasher(hasher),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.values().sum()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn count<Q>(&self, key: &Q) -> usize
    where
        T: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.entries.get(key).copied().unwrap_or(0)
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.count(key) > 0
    }

    pub fn insert(&mut self, value: T) {
        *self.entries.entry(value).or_insert(0) += 1;
    }

    pub fn remove_one<Q>(&mut self, key: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        let remove_key = {
            let count = match self.entries.get_mut(key) {
                Some(count) => count,
                None => return false,
            };
            *count -= 1;
            *count == 0
        };

        if remove_key {
            let _ = self.entries.remove(key);
        }

        true
    }

    pub fn remove_all<Q>(&mut self, key: &Q) -> usize
    where
        T: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.entries.remove(key).unwrap_or(0)
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries
            .iter()
            .flat_map(|(value, count)| std::iter::repeat_n(value, *count))
    }
}

impl<T, S> Default for UnorderedMultiset<T, S>
where
    T: Eq + Hash,
    S: BuildHasher + Clone + Default,
{
    fn default() -> Self {
        Self::with_hasher(S::default())
    }
}

#[cfg(test)]
mod tests {
    use super::{HardenedHashMap, HardenedHashSet, HardenedPartitionedHashMap, HashMap, HashSet};
    use crate::hardened_hash::HardenedHashBuilder;

    #[test]
    fn plain_aliases_behave_like_regular_maps_and_sets() {
        let mut map = HashMap::<String, usize>::default();
        let mut set = HashSet::<String>::default();

        assert_eq!(map.insert(String::from("fee"), 10), None);
        assert_eq!(map.insert(String::from("fee"), 12), Some(10));
        assert!(set.insert(String::from("ledger")));
        assert!(!set.insert(String::from("ledger")));
        assert_eq!(map.get("fee"), Some(&12));
        assert!(set.contains("ledger"));
    }

    #[test]
    fn hardened_aliases_accept_explicit_seeded_hashers() {
        let mut map = HardenedHashMap::with_hasher(HardenedHashBuilder::from_seed(13));
        let mut set = HardenedHashSet::with_hasher(HardenedHashBuilder::from_seed(13));

        assert_eq!(map.insert(String::from("owner"), 1), None);
        assert!(set.insert(String::from("owner")));
        assert_eq!(map.get("owner"), Some(&1));
        assert!(set.contains("owner"));
    }

    #[test]
    fn hardened_partitioned_alias_keeps_partition_access_visible() {
        let mut map =
            HardenedPartitionedHashMap::with_hasher(Some(4), HardenedHashBuilder::from_seed(21));
        let key = String::from("txn");
        let partition = map.partition_for(key.as_str());

        map.insert(key, 7);

        assert_eq!(map.partitions(), 4);
        assert_eq!(map.map()[partition].get("txn"), Some(&7));
    }
}
