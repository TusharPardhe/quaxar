//! Trusted committed baseline for restart catch-up.
//!
//! See `docs/design/2026-10-11-trusted-baseline-restart.md`.
//!
//! The pruned node store durably records the last claimed (validated) ledger's
//! state root. A claim restores every node of that tree that is missing and is
//! persisted in one synced batch, and prune only deletes nodes whose reference
//! count is zero, so every node of the claimed tree, and therefore its whole
//! subtree, is on disk at startup. This is the shadow-paging / copy-on-write
//! commit guarantee (Rodeh, ACM TOS 2008; Ganger et al., ACM TOCS 2000): a
//! committed root implies a complete tree.
//!
//! Catch-up acquisitions compare each target inner node against the baseline
//! node at the same position. A child whose hash equals the baseline child is
//! the same content-addressed subtree, already complete on disk, so the
//! traversal skips it without reading it (Merkle anti-entropy, as in Dynamo,
//! SOSP 2007). Only diverging paths are read, so restart cost is proportional
//! to the change since the baseline instead of to the state size.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use basics::base_uint::Uint256;
use basics::intrusive_pointer::SharedIntrusive;
use basics::sha_map_hash::SHAMapHash;
use shamap::node_id::{SHAMapNodeId, select_branch};
use shamap::nodes::tree_node::SHAMapTreeNode;

/// Synchronous node reader for baseline nodes: returns the NodeStore payload
/// (prefix form) for a hash.
pub(crate) type BaselineFetch = Arc<dyn Fn(&Uint256) -> Option<Vec<u8>> + Send + Sync>;

/// Upper bound on resident baseline inner nodes. The memo only holds nodes on
/// paths that diverge from the baseline, which is proportional to the change
/// since the restart; the bound only guards against a pathological gap.
const MAX_MEMO_NODES: usize = 2_000_000;

pub(crate) struct TrustedBaseline {
    seq: u32,
    root_hash: Uint256,
    fetch: BaselineFetch,
    retired: AtomicBool,
    /// Baseline nodes by position, `None` when the baseline has no inner node
    /// there (empty branch, leaf, or unreadable).
    memo: Mutex<HashMap<SHAMapNodeId, Option<SharedIntrusive<SHAMapTreeNode>>>>,
    skips: AtomicU64,
    loads: AtomicU64,
    load_misses: AtomicU64,
}

impl std::fmt::Debug for TrustedBaseline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustedBaseline")
            .field("seq", &self.seq)
            .field("root_hash", &self.root_hash)
            .field("retired", &self.retired.load(Ordering::Relaxed))
            .finish()
    }
}

/// Snapshot of baseline counters for logs and `get_counts`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrustedBaselineStats {
    pub seq: u32,
    pub active: bool,
    pub skips: u64,
    pub loads: u64,
    pub load_misses: u64,
    pub memo_nodes: u64,
}

impl TrustedBaseline {
    pub(crate) fn new(seq: u32, root_hash: Uint256, fetch: BaselineFetch) -> Self {
        Self {
            seq,
            root_hash,
            fetch,
            retired: AtomicBool::new(false),
            memo: Mutex::new(HashMap::new()),
            skips: AtomicU64::new(0),
            loads: AtomicU64::new(0),
            load_misses: AtomicU64::new(0),
        }
    }

    pub(crate) fn seq(&self) -> u32 {
        self.seq
    }

    pub(crate) fn is_retired(&self) -> bool {
        self.retired.load(Ordering::Acquire)
    }

    pub(crate) fn stats(&self) -> TrustedBaselineStats {
        TrustedBaselineStats {
            seq: self.seq,
            active: !self.is_retired(),
            skips: self.skips.load(Ordering::Relaxed),
            loads: self.loads.load(Ordering::Relaxed),
            load_misses: self.load_misses.load(Ordering::Relaxed),
            memo_nodes: self.memo.lock().map(|memo| memo.len() as u64).unwrap_or(0),
        }
    }

    /// Stop trusting the baseline and release its resident nodes.
    pub(crate) fn retire(&self) {
        if !self.retired.swap(true, Ordering::AcqRel)
            && let Ok(mut memo) = self.memo.lock()
        {
            memo.clear();
            memo.shrink_to_fit();
        }
    }

    fn load_inner(&self, hash: &Uint256) -> Option<SharedIntrusive<SHAMapTreeNode>> {
        self.loads.fetch_add(1, Ordering::Relaxed);
        let Some(blob) = (self.fetch)(hash) else {
            self.load_misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        let node = SHAMapTreeNode::make_from_prefix(&blob, SHAMapHash::new(*hash)).ok()?;
        (node.is_inner() && *node.get_hash().as_uint256() == *hash).then_some(node)
    }

    /// The baseline inner node at `id`, walking from the root and memoizing
    /// every level on the way.
    fn inner_at(&self, id: SHAMapNodeId) -> Option<SharedIntrusive<SHAMapTreeNode>> {
        let mut memo = self.memo.lock().ok()?;
        if let Some(found) = memo.get(&id) {
            return found.clone();
        }
        let root_id = SHAMapNodeId::default();
        let mut current = match memo.get(&root_id) {
            Some(found) => found.clone(),
            None => {
                let root = self.load_inner(&self.root_hash);
                memo.insert(root_id, root.clone());
                root
            }
        };
        let key = id.get_node_id();
        for depth in 0..id.get_depth() {
            let node = current?;
            let here = SHAMapNodeId::create_id(depth, key).ok()?;
            let child_id = SHAMapNodeId::create_id(depth + 1, key).ok()?;
            if let Some(found) = memo.get(&child_id) {
                current = found.clone();
                continue;
            }
            let branch = select_branch(here, key);
            let child = if node.is_empty_branch(branch) {
                None
            } else {
                self.load_inner(node.get_child_hash(branch).as_uint256())
            };
            if memo.len() >= MAX_MEMO_NODES {
                return None;
            }
            memo.insert(child_id, child.clone());
            current = child;
        }
        current
    }

    /// Whether `hash` under `branch` of the target inner node at `parent` is a
    /// subtree of the baseline tree at the same position.
    pub(crate) fn has_complete_child(
        &self,
        parent: SHAMapNodeId,
        branch: usize,
        hash: SHAMapHash,
    ) -> bool {
        if self.is_retired() {
            return false;
        }
        let Some(baseline_parent) = self.inner_at(parent) else {
            return false;
        };
        let matched = !baseline_parent.is_empty_branch(branch)
            && baseline_parent.get_child_hash(branch) == hash;
        if matched {
            self.skips.fetch_add(1, Ordering::Relaxed);
        }
        matched
    }
}

fn slot() -> &'static RwLock<Option<Arc<TrustedBaseline>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<TrustedBaseline>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Install the startup baseline (once per process).
pub(crate) fn install(baseline: Arc<TrustedBaseline>) {
    if let Ok(mut guard) = slot().write() {
        *guard = Some(baseline);
    }
}

/// The active baseline, if any.
pub(crate) fn active() -> Option<Arc<TrustedBaseline>> {
    slot()
        .read()
        .ok()
        .and_then(|guard| guard.clone())
        .filter(|baseline| !baseline.is_retired())
}

/// Retire the baseline and log its counters. Returns the final stats when a
/// baseline was active.
pub(crate) fn retire(reason: &'static str) -> Option<TrustedBaselineStats> {
    let baseline = slot().write().ok().and_then(|mut guard| guard.take())?;
    let stats = baseline.stats();
    baseline.retire();
    tracing::info!(
        target: "acquisition_trace",
        event = "trusted_baseline_retired",
        reason,
        baseline_seq = stats.seq,
        skips = stats.skips,
        loads = stats.loads,
        load_misses = stats.load_misses,
        memo_nodes = stats.memo_nodes,
        "trusted committed baseline retired"
    );
    Some(stats)
}

/// Current baseline counters (also after retirement is not retained).
pub fn stats() -> Option<TrustedBaselineStats> {
    slot().read().ok().and_then(|guard| guard.as_ref().map(|b| b.stats()))
}

#[cfg(test)]
pub(crate) fn clear_for_test() {
    if let Ok(mut guard) = slot().write() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(fill: u8) -> SHAMapHash {
        SHAMapHash::new(Uint256::from_array([fill; 32]))
    }

    /// Baseline: root -(3)-> inner X -(5)-> hash L.
    fn baseline() -> (TrustedBaseline, SHAMapHash, SHAMapHash, Arc<Mutex<u64>>) {
        let x = SHAMapTreeNode::new_inner(0);
        x.set_child_hash(5, hash(0x55));
        x.update_hash();
        let root = SHAMapTreeNode::new_inner(0);
        root.set_child_hash(3, x.get_hash());
        root.set_child_hash(9, hash(0x99));
        root.update_hash();
        let mut blobs = HashMap::new();
        for node in [&root, &x] {
            blobs.insert(*node.get_hash().as_uint256(), node.serialize_with_prefix().unwrap());
        }
        let reads = Arc::new(Mutex::new(0u64));
        let counter = Arc::clone(&reads);
        let fetch: BaselineFetch = Arc::new(move |key| {
            *counter.lock().unwrap() += 1;
            blobs.get(key).cloned()
        });
        (
            TrustedBaseline::new(7, *root.get_hash().as_uint256(), fetch),
            x.get_hash(),
            root.get_hash(),
            reads,
        )
    }

    #[test]
    fn matching_child_at_same_position_is_complete() {
        let (baseline, x_hash, _, reads) = baseline();
        let root_id = SHAMapNodeId::default();
        assert!(baseline.has_complete_child(root_id, 3, x_hash));
        assert!(baseline.has_complete_child(root_id, 9, hash(0x99)));
        // Different hash, empty branch, or another position: not trusted.
        assert!(!baseline.has_complete_child(root_id, 3, hash(0x01)));
        assert!(!baseline.has_complete_child(root_id, 4, x_hash));
        let x_id = root_id.get_child_node_id(3).unwrap();
        assert!(baseline.has_complete_child(x_id, 5, hash(0x55)));
        assert!(!baseline.has_complete_child(x_id, 6, hash(0x55)));
        // Under a baseline leaf/absent branch there is no inner node.
        let missing_id = root_id.get_child_node_id(9).unwrap();
        assert!(!baseline.has_complete_child(missing_id, 0, hash(0x01)));
        // Root, X and the unreadable 0x99 were each read once (memoized).
        assert_eq!(*reads.lock().unwrap(), 3);
        assert_eq!(baseline.stats().skips, 3);
    }

    #[test]
    fn retired_baseline_trusts_nothing_and_drops_memo() {
        let (baseline, x_hash, _, _) = baseline();
        assert!(baseline.has_complete_child(SHAMapNodeId::default(), 3, x_hash));
        baseline.retire();
        assert!(!baseline.has_complete_child(SHAMapNodeId::default(), 3, x_hash));
        assert_eq!(baseline.stats().memo_nodes, 0);
    }
}
