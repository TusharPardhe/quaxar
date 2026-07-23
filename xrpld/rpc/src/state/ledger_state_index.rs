//! Materialized sorted snapshot of all state map entries for fast binary-search ledger_data.
//!
//! # Design
//!
//! `LedgerStateIndex` is built once per ledger by consuming an iterator of
//! [`StateIndexEntry`] values, sorting them by key, and storing them in a
//! `Vec`.  Subsequent `ledger_data` requests perform a `partition_point`
//! (binary-search) seek and then walk forward, which avoids repeated
//! SHAMap traversals and is O(log N + page_size) instead of O(N) per page.
//!
//! The `LedgerStateIndexCache` wraps the latest built index behind an
//! `RwLock` so multiple reader threads can query concurrently while a
//! background writer replaces the index after each ledger close.
//!
//! # Marker semantics
//!
//! To match rippled exactly, the marker returned to the caller is
//! `next_key - 1` (via `Uint256::decrement()`), so that
//! `succ(marker) == next_key` and the next page resumes without overlap.

use std::sync::{Arc, OnceLock, RwLock};

use basics::base_uint::Uint256;
use protocol::LedgerEntryType;

// ---------------------------------------------------------------------------
// StateIndexEntry
// ---------------------------------------------------------------------------

/// A single entry in the sorted state-map snapshot.
///
/// JSON and binary hex representations are computed lazily and cached in
/// `OnceLock` fields so the first request pays the serialization cost once,
/// and all subsequent requests (within the same ledger lifetime) read the
/// cached value at zero extra cost.
pub struct StateIndexEntry {
    /// The 256-bit ledger-object key (used for sort order and marker math).
    pub key: Uint256,
    /// Raw serialized bytes of the ledger entry (as returned by the node
    /// store).  Wrapped in an `Arc<[u8]>` to allow cheap cloning.
    pub raw_data: Arc<[u8]>,
    /// Parsed ledger entry type — used by the `type_filter` predicate.
    pub entry_type: LedgerEntryType,
    /// Lazily computed JSON string cache.
    pub json_cache: OnceLock<Arc<str>>,
    /// Lazily computed hex-encoded binary string cache.
    pub binary_hex_cache: OnceLock<Arc<str>>,
}

// Manual `Clone` because `OnceLock` is not `Clone`.
impl Clone for StateIndexEntry {
    fn clone(&self) -> Self {
        Self {
            key: self.key,
            raw_data: Arc::clone(&self.raw_data),
            entry_type: self.entry_type,
            // Propagate cached values if already computed.
            json_cache: self
                .json_cache
                .get()
                .map(|v| {
                    let lock = OnceLock::new();
                    let _ = lock.set(Arc::clone(v));
                    lock
                })
                .unwrap_or_default(),
            binary_hex_cache: self
                .binary_hex_cache
                .get()
                .map(|v| {
                    let lock = OnceLock::new();
                    let _ = lock.set(Arc::clone(v));
                    lock
                })
                .unwrap_or_default(),
        }
    }
}

impl StateIndexEntry {
    /// Construct a new entry with empty caches.
    pub fn new(key: Uint256, raw_data: Arc<[u8]>, entry_type: LedgerEntryType) -> Self {
        Self {
            key,
            raw_data,
            entry_type,
            json_cache: OnceLock::new(),
            binary_hex_cache: OnceLock::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// LedgerStateIndex
// ---------------------------------------------------------------------------

/// Immutable, sorted snapshot of a single ledger's state map.
///
/// Once built via [`LedgerStateIndex::build_from_iter`] the entries are
/// sorted by key and never mutated.  All read operations are therefore
/// lock-free — the only synchronisation is the outer `RwLock` in
/// [`LedgerStateIndexCache`] that guards the `Arc<LedgerStateIndex>`.
pub struct LedgerStateIndex {
    entries: Vec<StateIndexEntry>,
    type_indices: std::collections::HashMap<LedgerEntryType, Vec<usize>>,
    /// The validated ledger sequence this snapshot was built from.
    pub ledger_seq: u32,
    /// The hash of the validated ledger this snapshot was built from.
    pub ledger_hash: Uint256,
}

impl LedgerStateIndex {
    /// Build a sorted index from an arbitrary iterator of [`StateIndexEntry`].
    ///
    /// Sorting is `unstable` (faster; entries with duplicate keys are
    /// pathological and not expected on a well-formed ledger).
    pub fn build_from_iter<I>(ledger_seq: u32, ledger_hash: Uint256, iter: I) -> Self
    where
        I: Iterator<Item = StateIndexEntry>,
    {
        let mut entries: Vec<StateIndexEntry> = iter.collect();
        use rayon::prelude::*;
        entries.par_sort_unstable_by_key(|e| e.key);
        let mut type_indices = std::collections::HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            type_indices
                .entry(e.entry_type)
                .or_insert_with(Vec::new)
                .push(i);
        }
        Self {
            entries,
            type_indices,
            ledger_seq,
            ledger_hash,
        }
    }

    /// Prepare a [`LedgerStateQuery`] starting after `marker` (or from the
    /// beginning of the index when `marker` is `None`).
    ///
    /// The query is a lightweight view; it does not copy any entries.
    pub fn query(
        &self,
        marker: Option<Uint256>,
        limit: usize,
        type_filter: LedgerEntryType,
    ) -> LedgerStateQuery<'_> {
        let start_key = marker.unwrap_or_default();
        if type_filter == LedgerEntryType::Any {
            let start = self.entries.partition_point(|e| e.key <= start_key);
            LedgerStateQuery {
                index: self,
                type_filter_indices: None,
                start,
                limit,
            }
        } else {
            let indices = self
                .type_indices
                .get(&type_filter)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let start = indices.partition_point(|&i| self.entries[i].key <= start_key);
            LedgerStateQuery {
                index: self,
                type_filter_indices: Some(indices),
                start,
                limit,
            }
        }
    }

    /// Total number of entries in the snapshot.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the snapshot contains no entries.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Read-only access to the underlying sorted entry slice.
    #[inline]
    pub fn entries(&self) -> &[StateIndexEntry] {
        &self.entries
    }
}

// ---------------------------------------------------------------------------
// LedgerStateQuery
// ---------------------------------------------------------------------------

/// A lazy, forward-scanning view over a [`LedgerStateIndex`].
///
/// Obtain one via [`LedgerStateIndex::query`] and then call
/// [`LedgerStateQuery::collect_entries`] to materialise the page.
pub struct LedgerStateQuery<'a> {
    index: &'a LedgerStateIndex,
    /// Offset into `index.entries` (or `type_filter_indices`) at which to start scanning.
    start: usize,
    /// Maximum number of entries to return in this page.
    limit: usize,
    /// The secondary index array if a type filter is active.
    type_filter_indices: Option<&'a [usize]>,
}

impl<'a> LedgerStateQuery<'a> {
    /// Collect at most `limit` entries (respecting `type_filter`) and return
    /// a pagination marker if more entries remain.
    ///
    /// # Marker semantics
    ///
    /// When the page is full and more entries exist, the marker is set to
    /// `next_entry.key - 1` via [`Uint256::decrement`].  This mirrors the
    /// C++ rippled implementation: `succ(marker) == next_key` so the
    /// following request resumes without a gap or overlap.
    ///
    /// # Returns
    ///
    /// `(entries, next_marker)` where `next_marker` is `None` when the page
    /// covers all remaining entries.
    pub fn collect_entries(&self) -> (Vec<&'a StateIndexEntry>, Option<Uint256>) {
        let mut results: Vec<&'a StateIndexEntry> = Vec::new();
        let mut next_marker: Option<Uint256> = None;
        let mut pos = self.start;

        let len = self
            .type_filter_indices
            .map_or(self.index.entries.len(), |indices| indices.len());

        while pos < len {
            let entry_idx = self.type_filter_indices.map_or(pos, |indices| indices[pos]);
            let entry = &self.index.entries[entry_idx];

            if results.len() >= self.limit {
                // We have a full page. Compute marker = next_key - 1 to
                // match rippled's C++ convention exactly.
                let mut marker_key = entry.key;
                marker_key.decrement();
                next_marker = Some(marker_key);
                break;
            }

            results.push(entry);
            pos += 1;
        }

        (results, next_marker)
    }
}

// ---------------------------------------------------------------------------
// LedgerStateIndexCache
// ---------------------------------------------------------------------------

/// Thread-safe, single-slot cache holding the most recently built
/// [`LedgerStateIndex`].
///
/// The cache stores at most one index (the latest validated ledger).
/// [`LedgerStateIndexCache::get`] returns the cached index only when the
/// requested `ledger_seq` matches the cached ledger sequence, avoiding stale
/// reads.
///
/// # Concurrency
///
/// * Many reader threads can call [`get`](LedgerStateIndexCache::get)
///   simultaneously without blocking each other.
/// * A single writer thread calls [`insert`](LedgerStateIndexCache::insert)
///   after each ledger close; readers are blocked only for the duration of the
///   pointer swap (no index rebuilding happens under the lock).
pub struct LedgerStateIndexCacheEntry {
    pub ledger_seq: u32,
    pub index: OnceLock<Arc<LedgerStateIndex>>,
}

pub struct LedgerStateIndexCache {
    inner: RwLock<Arc<LedgerStateIndexCacheEntry>>,
}

pub static GLOBAL_STATE_INDEX_CACHE: OnceLock<LedgerStateIndexCache> = OnceLock::new();

pub fn get_global_state_index_cache() -> &'static LedgerStateIndexCache {
    GLOBAL_STATE_INDEX_CACHE.get_or_init(LedgerStateIndexCache::default)
}

impl LedgerStateIndexCache {
    /// Create a new, empty cache.
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Arc::new(LedgerStateIndexCacheEntry {
                ledger_seq: 0,
                index: OnceLock::new(),
            })),
        }
    }

    /// Return the cached index if it matches `ledger_seq`, or `None`
    /// if the cache is empty or stale.
    pub fn get(&self, ledger_seq: u32) -> Option<Arc<LedgerStateIndex>> {
        let guard = self.inner.read().unwrap();
        if guard.ledger_seq == ledger_seq {
            guard.index.get().cloned()
        } else {
            None
        }
    }

    /// Get the cached index if it matches `ledger_seq`, or build it using `builder`.
    /// Only one thread will execute `builder` per `ledger_seq`.
    pub fn get_or_build<F>(&self, ledger_seq: u32, builder: F) -> Arc<LedgerStateIndex>
    where
        F: FnOnce() -> Arc<LedgerStateIndex>,
    {
        let entry = {
            let guard = self.inner.read().unwrap();
            if guard.ledger_seq == ledger_seq {
                guard.clone()
            } else {
                drop(guard);
                let mut guard = self.inner.write().unwrap();
                if guard.ledger_seq != ledger_seq {
                    *guard = Arc::new(LedgerStateIndexCacheEntry {
                        ledger_seq,
                        index: OnceLock::new(),
                    });
                }
                guard.clone()
            }
        };

        entry.index.get_or_init(builder).clone()
    }

    /// Store a newly built index, replacing any previously cached value.
    pub fn insert(&self, index: Arc<LedgerStateIndex>) {
        let mut guard = self.inner.write().unwrap();
        let entry = LedgerStateIndexCacheEntry {
            ledger_seq: index.ledger_seq,
            index: OnceLock::new(),
        };
        let _ = entry.index.set(index);
        *guard = Arc::new(entry);
    }

    /// Discard the cached index, for example when the node falls behind and
    /// the stale snapshot should not be served.
    pub fn clear(&self) {
        let mut guard = self.inner.write().unwrap();
        *guard = Arc::new(LedgerStateIndexCacheEntry {
            ledger_seq: 0,
            index: OnceLock::new(),
        });
    }
}

impl Default for LedgerStateIndexCache {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(key_val: u64, entry_type: LedgerEntryType) -> StateIndexEntry {
        let mut key = Uint256::default();
        // Place the value in the lowest 8 bytes of the 256-bit key.
        let bytes = key.as_mut_slice();
        let offset = bytes.len() - 8;
        bytes[offset..].copy_from_slice(&key_val.to_be_bytes());

        StateIndexEntry::new(key, Arc::from(vec![0u8; 4].as_slice()), entry_type)
    }

    #[test]
    fn build_sorts_entries() {
        let entries = vec![
            make_entry(3, LedgerEntryType::AccountRoot),
            make_entry(1, LedgerEntryType::Offer),
            make_entry(2, LedgerEntryType::AccountRoot),
        ];
        let index = LedgerStateIndex::build_from_iter(1, Uint256::default(), entries.into_iter());

        assert_eq!(index.len(), 3);
        let keys: Vec<u64> = index
            .entries()
            .iter()
            .map(|e| {
                let b = e.key.as_slice();
                let offset = b.len() - 8;
                u64::from_be_bytes(b[offset..].try_into().unwrap())
            })
            .collect();
        assert_eq!(keys, vec![1, 2, 3]);
    }

    #[test]
    fn query_no_marker_no_filter() {
        let entries = (1u64..=5).map(|i| make_entry(i, LedgerEntryType::AccountRoot));
        let index = LedgerStateIndex::build_from_iter(1, Uint256::default(), entries);

        let query = index.query(None, 3, LedgerEntryType::Any);
        let (page, marker) = query.collect_entries();
        assert_eq!(page.len(), 3);
        assert!(marker.is_some(), "expect a marker when more entries remain");
    }

    #[test]
    fn query_respects_type_filter() {
        let entries = vec![
            make_entry(1, LedgerEntryType::AccountRoot),
            make_entry(2, LedgerEntryType::Offer),
            make_entry(3, LedgerEntryType::AccountRoot),
            make_entry(4, LedgerEntryType::Offer),
        ];
        let index = LedgerStateIndex::build_from_iter(1, Uint256::default(), entries.into_iter());

        let query = index.query(None, 10, LedgerEntryType::Offer);
        let (page, marker) = query.collect_entries();
        assert_eq!(page.len(), 2);
        assert!(marker.is_none());
        assert!(page.iter().all(|e| e.entry_type == LedgerEntryType::Offer));
    }

    #[test]
    fn cache_returns_none_on_stale_seq() {
        let cache = LedgerStateIndexCache::new();
        let index = Arc::new(LedgerStateIndex::build_from_iter(
            5,
            Uint256::default(),
            std::iter::empty(),
        ));
        cache.insert(Arc::clone(&index));

        assert!(cache.get(5).is_some());
        assert!(cache.get(6).is_none(), "stale seq should return None");
    }

    #[test]
    fn cache_clear_removes_entry() {
        let cache = LedgerStateIndexCache::new();
        let index = Arc::new(LedgerStateIndex::build_from_iter(
            1,
            Uint256::default(),
            std::iter::empty(),
        ));
        cache.insert(index);
        cache.clear();
        assert!(cache.get(1).is_none());
    }
}
