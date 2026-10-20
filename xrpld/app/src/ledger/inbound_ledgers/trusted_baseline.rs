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

/// Age limit for a store pruned `online_delete` ledgers after death: half the
/// window, so a baseline is retired long before any of its nodes is deletable.
pub(crate) fn max_age_for(online_delete: u32) -> u32 {
    (online_delete / 2).max(1)
}

pub(crate) struct TrustedBaseline {
    seq: u32,
    /// Ledgers past `seq` for which the baseline stays trusted. Its nodes die
    /// no earlier than `seq + 1` and are deleted `online_delete` ledgers after
    /// they die, so any value below `online_delete` keeps every use safe.
    max_age: u32,
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
    #[cfg(test)]
    pub(crate) fn new(seq: u32, root_hash: Uint256, fetch: BaselineFetch) -> Self {
        Self::with_max_age(seq, root_hash, fetch, 256)
    }

    pub(crate) fn with_max_age(
        seq: u32,
        root_hash: Uint256,
        fetch: BaselineFetch,
        max_age: u32,
    ) -> Self {
        Self {
            seq,
            max_age,
            root_hash,
            fetch,
            retired: AtomicBool::new(false),
            memo: Mutex::new(HashMap::new()),
            skips: AtomicU64::new(0),
            loads: AtomicU64::new(0),
            load_misses: AtomicU64::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn seq(&self) -> u32 {
        self.seq
    }

    /// Whether the baseline is too old to trust at validated ledger
    /// `validated` (some of its nodes may become deletable).
    pub(crate) fn is_too_old(&self, validated: u32) -> bool {
        validated > self.seq.saturating_add(self.max_age)
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

/// Source for completion-installed baselines: the store reader and age limit
/// registered at startup, plus whether the store has ever claimed a ledger.
struct CompletionSource {
    fetch: BaselineFetch,
    max_age: u32,
    /// Durably records the installed completion so a restart before the
    /// first claim can reinstall it.
    record: Option<CompletionRecord>,
}

/// Persists `(seq, state_root)` of a completion-installed baseline.
pub(crate) type CompletionRecord = Arc<dyn Fn(u32, Uint256) + Send + Sync>;

fn completion_source() -> &'static RwLock<Option<CompletionSource>> {
    static SOURCE: OnceLock<RwLock<Option<CompletionSource>>> = OnceLock::new();
    SOURCE.get_or_init(|| RwLock::new(None))
}

fn claimed_any() -> &'static AtomicBool {
    static CLAIMED: AtomicBool = AtomicBool::new(false);
    &CLAIMED
}

/// Register the reader used by [`install_from_completed_acquisition`].
/// `claimed_at_start` is true when the store already holds a claimed ledger;
/// claims then own the baseline exclusively.
pub(crate) fn register_completion_source(
    fetch: BaselineFetch,
    max_age: u32,
    claimed_at_start: bool,
    record: Option<CompletionRecord>,
) {
    claimed_any().store(claimed_at_start, Ordering::Relaxed);
    if let Ok(mut guard) = completion_source().write() {
        *guard = Some(CompletionSource {
            fetch,
            max_age,
            record,
        });
    }
}

/// Install a baseline from a completed, durable acquisition when the store
/// has never claimed a ledger.
///
/// A node that cold-syncs has no claimed ledger, so without this the state
/// tree it just downloaded is never used as a baseline: every later catch-up
/// acquisition walks the whole tree again. On mainnet (2026-10-20) each tip
/// acquisition needed far longer than its idle lifetime, so the node never
/// reached the tip after a 90-minute initial sync. The completed ledger
/// passed the durability fence (its whole tree is on disk) and nothing is
/// deleted before the first claim, so the commit guarantee of a claimed root
/// holds. The first claim replaces this baseline through [`advance`].
pub(crate) fn install_from_completed_acquisition(seq: u32, root_hash: Uint256) -> bool {
    if claimed_any().load(Ordering::Relaxed) || root_hash.is_zero() {
        return false;
    }
    if active().is_some_and(|current| current.seq >= seq) {
        return false;
    }
    let Some((fetch, max_age, record)) = completion_source().read().ok().and_then(|guard| {
        guard.as_ref().map(|source| {
            (
                Arc::clone(&source.fetch),
                source.max_age,
                source.record.clone(),
            )
        })
    }) else {
        return false;
    };
    if fetch(&root_hash).is_none() {
        return false;
    }
    if let Some(record) = record {
        record(seq, root_hash);
    }
    let baseline = Arc::new(TrustedBaseline::with_max_age(seq, root_hash, fetch, max_age));
    let previous = slot()
        .write()
        .ok()
        .and_then(|mut guard| guard.replace(baseline));
    if let Some(previous) = previous {
        previous.retire();
    }
    true
}

/// Roll the baseline forward to a newly claimed ledger. The claim is synced
/// before this runs, so the new root's tree is complete on disk; the previous
/// baseline is retired (its memo is released once in-flight users drop it).
pub(crate) fn advance(baseline: Arc<TrustedBaseline>) {
    claimed_any().store(true, Ordering::Relaxed);
    let previous = slot()
        .write()
        .ok()
        .and_then(|mut guard| guard.replace(baseline));
    if let Some(previous) = previous {
        previous.retire();
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

    /// Mainnet cold sync 2026-10-20: a node with no claimed ledger must seed
    /// the baseline from its first completed durable acquisition; a newer
    /// completion replaces it; once any claim exists claims own it.
    #[test]
    fn completed_acquisition_seeds_baseline_until_the_first_claim() {
        clear_for_test();
        let root = SHAMapTreeNode::new_inner(0);
        root.set_child_hash(1, hash(0x11));
        root.update_hash();
        let root_hash = *root.get_hash().as_uint256();
        let blob = root.serialize_with_prefix().unwrap();
        let fetch: BaselineFetch =
            Arc::new(move |key| (*key == root_hash).then(|| blob.clone()));

        register_completion_source(Arc::clone(&fetch), 256, false, None);
        assert!(!install_from_completed_acquisition(10, Uint256::from_array([0xEE; 32])),
            "a root that is not on disk is never trusted");
        assert!(install_from_completed_acquisition(10, root_hash));
        assert_eq!(active().map(|b| b.seq()), Some(10));
        assert!(!install_from_completed_acquisition(9, root_hash), "older completion ignored");
        assert!(install_from_completed_acquisition(12, root_hash));
        assert_eq!(active().map(|b| b.seq()), Some(12));

        advance(Arc::new(TrustedBaseline::new(13, root_hash, Arc::clone(&fetch))));
        assert!(!install_from_completed_acquisition(20, root_hash),
            "claims own the baseline after the first claim");
        assert_eq!(active().map(|b| b.seq()), Some(13));

        register_completion_source(fetch, 256, true, None);
        clear_for_test();
        assert!(!install_from_completed_acquisition(30, root_hash),
            "a store with a claimed ledger at startup never uses completions");
        clear_for_test();
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
    fn age_limit_is_half_the_prune_window() {
        assert_eq!(max_age_for(512), 256);
        assert_eq!(max_age_for(256), 128);
        assert_eq!(max_age_for(1), 1);
        let (_, x_hash, root, _) = baseline();
        let fetch: BaselineFetch = Arc::new(|_| None);
        let b = TrustedBaseline::with_max_age(100, *root.as_uint256(), fetch, 10);
        assert!(!b.is_too_old(110));
        assert!(b.is_too_old(111));
        let _ = x_hash;
    }

    /// Rolling forward replaces the active baseline and retires the old one,
    /// so an in-flight holder of the previous Arc stops trusting it.
    #[test]
    fn advance_replaces_and_retires_previous_baseline() {
        clear_for_test();
        let (first, x_hash, _, _) = baseline();
        let first = Arc::new(first);
        install(Arc::clone(&first));
        assert!(first.has_complete_child(SHAMapNodeId::default(), 3, x_hash));
        let (second, _, _, _) = baseline();
        let second = Arc::new(TrustedBaseline::with_max_age(
            8,
            second.root_hash,
            Arc::clone(&second.fetch),
            256,
        ));
        advance(Arc::clone(&second));
        assert!(first.is_retired(), "previous baseline retired");
        assert!(!first.has_complete_child(SHAMapNodeId::default(), 3, x_hash));
        assert_eq!(active().map(|b| b.seq()), Some(8));
        clear_for_test();
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
