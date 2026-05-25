//! AssetCache ported from `xrpld/rpc/detail/AssetCache.h/the reference source`.
//!
//! Caches trust lines per account for pathfinding. Uses a hardened hash and
//! direction-aware caching to minimize memory usage. Thread-safe via Mutex.

#![allow(dead_code)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use protocol::AccountID;

use super::path_find_mpt::PathFindMPT;
use super::trust_line::{LineDirection, PathFindTrustLine};

/// Trait representing a read-only ledger view for the cache.
pub trait ReadView: Send + Sync {
    fn ledger_seq(&self) -> u32;
}

/// Key for the trust-line cache: (account, direction).
#[derive(Debug, Clone, PartialEq, Eq)]
struct AccountKey {
    account: AccountID,
    direction: LineDirection,
    hash_value: u64,
}

impl Hash for AccountKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash_value);
    }
}

/// Trait for fetching trust lines and MPTs from the ledger.
/// The real implementation reads from the SHAMap; this trait allows testing.
pub trait AssetCacheLedger: Send + Sync {
    fn ledger_seq(&self) -> u32;
    fn get_trust_lines(
        &self,
        account: &AccountID,
        direction: LineDirection,
    ) -> Vec<PathFindTrustLine>;
    fn get_mpts(&self, account: &AccountID) -> Vec<PathFindMPT>;
}

/// AssetCache — caches trust lines and MPTs per account for pathfinding.
///
/// Preserves the reference behavior:
/// - If an `Incoming` request finds existing `Outgoing` lines, returns the
///   outgoing superset.
/// - If an `Outgoing` request finds existing `Incoming` lines, erases the
///   subset and rebuilds with the full outgoing set.
pub struct AssetCache {
    inner: Mutex<AssetCacheInner>,
    ledger: Arc<dyn AssetCacheLedger>,
}

struct AssetCacheInner {
    lines: HashMap<AccountKey, Option<Arc<Vec<PathFindTrustLine>>>>,
    total_line_count: usize,
    mpts: HashMap<AccountID, Option<Arc<Vec<PathFindMPT>>>>,
}

impl AssetCache {
    pub fn new(ledger: Arc<dyn AssetCacheLedger>) -> Self {
        Self {
            inner: Mutex::new(AssetCacheInner {
                lines: HashMap::new(),
                total_line_count: 0,
                mpts: HashMap::new(),
            }),
            ledger,
        }
    }

    pub fn ledger_seq(&self) -> u32 {
        self.ledger.ledger_seq()
    }

    /// Get trust lines for an account in the given direction.
    /// Mirrors the reference `getRippleLines` with its direction-aware caching logic.
    pub fn get_ripple_lines(
        &self,
        account_id: &AccountID,
        direction: LineDirection,
    ) -> Option<Arc<Vec<PathFindTrustLine>>> {
        let hash_value = hash_account(account_id);
        let key = AccountKey {
            account: account_id.clone(),
            direction,
            hash_value,
        };
        let other_direction = match direction {
            LineDirection::Outgoing => LineDirection::Incoming,
            LineDirection::Incoming => LineDirection::Outgoing,
        };
        let other_key = AccountKey {
            account: account_id.clone(),
            direction: other_direction,
            hash_value,
        };

        let mut inner = self.inner.lock().unwrap();

        // Check if the other direction already exists
        if let Some(other_entry) = inner.lines.get(&other_key).cloned() {
            let other_size = other_entry.as_ref().map_or(0, |v| v.len());

            if direction == LineDirection::Outgoing {
                // Outgoing request but incoming subset exists — erase it and rebuild
                inner.total_line_count -= other_size;
                inner.lines.remove(&other_key);
            } else {
                // Incoming request but outgoing superset exists — return it
                return other_entry;
            }
        }

        // Check if we already have this exact key
        if let Some(entry) = inner.lines.get(&key) {
            return entry.clone();
        }

        // Build from ledger
        let lines = self.ledger.get_trust_lines(account_id, direction);

        if lines.is_empty() {
            inner.lines.insert(key, None);
            None
        } else {
            let count = lines.len();
            let arc = Arc::new(lines);
            inner.lines.insert(key, Some(arc.clone()));
            inner.total_line_count += count;
            Some(arc)
        }
    }

    /// Get MPTs for an account. Caches the result.
    pub fn get_mpts(&self, account: &AccountID) -> Option<Arc<Vec<PathFindMPT>>> {
        let mut inner = self.inner.lock().unwrap();

        if let Some(entry) = inner.mpts.get(account) {
            return entry.clone();
        }

        let mpts = self.ledger.get_mpts(account);

        if mpts.is_empty() {
            inner.mpts.insert(account.clone(), None);
            None
        } else {
            let arc = Arc::new(mpts);
            inner.mpts.insert(account.clone(), Some(arc.clone()));
            Some(arc)
        }
    }
}

/// Simple hash function for AccountID (mirrors hardened hash in the reference).
fn hash_account(account: &AccountID) -> u64 {
    use std::hash::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    account.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_direction_values() {
        assert_eq!(LineDirection::Incoming as u8, 0);
        assert_eq!(LineDirection::Outgoing as u8, 1);
    }
}
