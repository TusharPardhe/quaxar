//! In-memory cache of recent ledgers by hash and sequence.
//! Handles mismatch detection between built vs validated ledgers.
//!

use basics::base_uint::Uint256;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value as JsonValue;

/// Tracks built vs validated ledger info for mismatch detection.
#[derive(Debug, Clone, Default)]
pub struct CvEntry {
    pub built: Option<Uint256>,
    pub validated: Option<Uint256>,
    pub built_consensus_hash: Option<Uint256>,
    pub validated_consensus_hash: Option<Uint256>,
    pub consensus: Option<JsonValue>,
}

/// Cached ledger entry (hash + seq for lookup).
#[derive(Debug, Clone)]
pub struct CachedLedger {
    pub hash: Uint256,
    pub seq: u32,
}

#[derive(Debug, Default)]
struct Inner {
    /// hash -> cached ledger data
    ledgers_by_hash: HashMap<Uint256, Arc<CachedLedger>>,
    /// seq -> hash (validated ledgers only)
    ledgers_by_index: BTreeMap<u32, Uint256>,
    /// seq -> built/validated tracking
    consensus_validated: HashMap<u32, CvEntry>,
    hits: u64,
    misses: u64,
}

/// Retains historical ledgers for fast lookup and mismatch detection.
pub struct LedgerHistory {
    inner: Mutex<Inner>,
    mismatch_counter: AtomicU64,
}

impl Default for LedgerHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl LedgerHistory {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            mismatch_counter: AtomicU64::new(0),
        }
    }

    /// Track a ledger. Returns `true` if the ledger was already tracked.
    pub fn insert(&self, hash: Uint256, seq: u32, validated: bool) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let existed = inner.ledgers_by_hash.contains_key(&hash);
        inner
            .ledgers_by_hash
            .insert(hash, Arc::new(CachedLedger { hash, seq }));
        if validated {
            inner.ledgers_by_index.insert(seq, hash);
        }
        existed
    }

    pub fn get_cache_hit_rate(&self) -> f32 {
        let inner = self.inner.lock().unwrap();
        let total = inner.hits + inner.misses;
        if total == 0 {
            return 0.0;
        }
        inner.hits as f32 / total as f32
    }

    pub fn get_ledger_by_seq(&self, index: u32) -> Option<Arc<CachedLedger>> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(hash) = inner.ledgers_by_index.get(&index).copied() {
            if let Some(entry) = inner.ledgers_by_hash.get(&hash).cloned() {
                inner.hits += 1;
                return Some(entry);
            }
        }
        inner.misses += 1;
        None
    }

    pub fn get_ledger_by_hash(&self, hash: &Uint256) -> Option<Arc<CachedLedger>> {
        let mut inner = self.inner.lock().unwrap();
        match inner.ledgers_by_hash.get(hash).cloned() {
            Some(h) => {
                inner.hits += 1;
                Some(h)
            }
            None => {
                inner.misses += 1;
                None
            }
        }
    }

    pub fn get_ledger_hash(&self, index: u32) -> Option<Uint256> {
        self.inner
            .lock()
            .unwrap()
            .ledgers_by_index
            .get(&index)
            .copied()
    }

    /// Remove stale cache entries.
    pub fn sweep(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.consensus_validated.len() > 256 {
            let keys: Vec<u32> = inner.consensus_validated.keys().copied().collect();
            if let Some(&max) = keys.iter().max() {
                let cutoff = max.saturating_sub(256);
                inner.consensus_validated.retain(|&k, _| k > cutoff);
            }
        }
    }

    /// Report that we have locally built a particular ledger.
    pub fn built_ledger(
        &self,
        hash: Uint256,
        seq: u32,
        consensus_hash: Uint256,
        consensus: JsonValue,
    ) {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.consensus_validated.entry(seq).or_default();

        if let Some(validated) = entry.validated {
            if validated != hash && entry.built.is_none() {
                self.mismatch_counter.fetch_add(1, Ordering::Relaxed);
            }
        }

        entry.built = Some(hash);
        entry.built_consensus_hash = Some(consensus_hash);
        entry.consensus = Some(consensus);
    }

    /// Report that we have validated a particular ledger.
    pub fn validated_ledger(&self, hash: Uint256, seq: u32, consensus_hash: Option<Uint256>) {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.consensus_validated.entry(seq).or_default();

        if let Some(built) = entry.built {
            if built != hash && entry.validated.is_none() {
                self.mismatch_counter.fetch_add(1, Ordering::Relaxed);
            }
        }

        entry.validated = Some(hash);
        entry.validated_consensus_hash = consensus_hash;
    }

    /// Repair a hash to index mapping. Returns `false` if the mapping was repaired.
    pub fn fix_index(&self, index: u32, hash: Uint256) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if let Some(existing) = inner.ledgers_by_index.get(&index) {
            if *existing != hash {
                inner.ledgers_by_index.insert(index, hash);
                return false;
            }
        }
        true
    }

    /// Clear cached ledgers with sequence numbers prior to `seq`.
    pub fn clear_ledger_cache_prior(&self, seq: u32) {
        let mut inner = self.inner.lock().unwrap();
        let to_remove: Vec<Uint256> = inner
            .ledgers_by_hash
            .values()
            .filter(|l| l.seq < seq)
            .map(|l| l.hash)
            .collect();
        for hash in &to_remove {
            inner.ledgers_by_hash.remove(hash);
        }
        inner.ledgers_by_index.retain(|&k, _| k >= seq);
    }

    pub fn mismatch_count(&self) -> u64 {
        self.mismatch_counter.load(Ordering::Relaxed)
    }
}
