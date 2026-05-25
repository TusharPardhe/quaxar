//! Narrow fetch-pack and node-store seams for ledger sync filters.
//!
//! The reference `AccountStateSF` and `TransactionStateSF` classes sit at the edge
//! between `SHAMapSyncFilter`, a fetch-pack source, and NodeStore writes. This
//! file keeps that adapter boundary explicit without forcing the wider
//! application `NodeStore::Database` graph into Rust yet.

use basics::base_uint::Uint256;
use basics::blob::Blob;
use basics::hardened_hash::HardenedHashBuilder;
use basics::tagged_cache::{CacheClock, MonotonicClock, TaggedCache};
use protocol::sha512_half;
use shamap::storage::{NodeObjectType, NodeStoreSink, StoredNode};
use std::hash::BuildHasher;
use time::Duration;

#[derive(Debug)]
pub struct FetchPackCache<C = MonotonicClock, S = HardenedHashBuilder> {
    cache: TaggedCache<Uint256, Blob, C, S>,
}

impl<C> FetchPackCache<C, HardenedHashBuilder>
where
    C: CacheClock,
{
    pub fn new(size: usize, age: Duration, clock: C) -> Self {
        Self::with_hasher(size, age, clock, HardenedHashBuilder::default())
    }
}

impl<C, S> FetchPackCache<C, S>
where
    C: CacheClock,
    S: BuildHasher + Clone,
{
    pub fn with_hasher(size: usize, age: Duration, clock: C, hasher: S) -> Self {
        Self {
            cache: TaggedCache::with_hasher("FetchPack", size, age, clock, hasher),
        }
    }

    pub fn add_fetch_pack(&self, hash: Uint256, data: Blob) {
        self.cache.insert(hash, data);
    }

    pub fn get_fetch_pack(&self, hash: Uint256) -> Option<Blob> {
        let data = self.cache.retrieve(&hash)?;
        self.cache.del(&hash, false);
        (sha512_half(&data) == hash).then_some(data)
    }

    pub fn sweep(&self) {
        self.cache.sweep();
    }

    pub fn get_cache_size(&self) -> usize {
        self.cache.get_cache_size()
    }
}

impl<C, S> FetchPackContainer for FetchPackCache<C, S>
where
    C: CacheClock,
    S: BuildHasher + Clone,
{
    fn get_fetch_pack(&mut self, hash: Uint256) -> Option<Blob> {
        FetchPackCache::get_fetch_pack(self, hash)
    }
}

impl<C, S> FetchPackStore for FetchPackCache<C, S>
where
    C: CacheClock,
    S: BuildHasher + Clone,
{
    fn add_fetch_pack(&mut self, hash: Uint256, data: Blob) {
        FetchPackCache::add_fetch_pack(self, hash, data);
    }
}

pub trait FetchPackContainer {
    fn get_fetch_pack(&mut self, hash: Uint256) -> Option<Blob>;
}

pub trait FetchPackStore {
    fn add_fetch_pack(&mut self, hash: Uint256, data: Blob);
}

impl<T> FetchPackContainer for &mut T
where
    T: FetchPackContainer + ?Sized,
{
    fn get_fetch_pack(&mut self, hash: Uint256) -> Option<Blob> {
        (**self).get_fetch_pack(hash)
    }
}

impl<T> FetchPackStore for &mut T
where
    T: FetchPackStore + ?Sized,
{
    fn add_fetch_pack(&mut self, hash: Uint256, data: Blob) {
        (**self).add_fetch_pack(hash, data);
    }
}

pub trait LedgerSyncFilterStore {
    fn store_shamap_node(
        &mut self,
        object_type: NodeObjectType,
        data: Blob,
        hash: Uint256,
        ledger_seq: u32,
    );

    /// Check if this hash should be stored. Returns false for duplicates.
    /// Called before serialization to avoid wasted work.
    fn should_store_hash(&mut self, hash: Uint256) -> bool {
        let _ = hash;
        true
    }

    /// Fetch node data from local persistent storage (reference getNode fallback).
    /// Used to avoid re-requesting nodes that already exist in the store.
    fn fetch_node_data(&self, hash: Uint256) -> Option<Blob> {
        let _ = hash;
        None
    }
}

impl<T> LedgerSyncFilterStore for T
where
    T: NodeStoreSink,
{
    fn store_shamap_node(
        &mut self,
        object_type: NodeObjectType,
        data: Blob,
        hash: Uint256,
        ledger_seq: u32,
    ) {
        self.store(StoredNode::new(object_type, data, hash, ledger_seq));
    }
}
