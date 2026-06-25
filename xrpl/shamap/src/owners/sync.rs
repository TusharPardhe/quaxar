#![allow(clippy::unnecessary_mut_passed)]
//! Missing-node scan and tree synchronization helpers used by `SHAMap` sync callers.

use crate::compare::{
    DeepCompareEvent, Delta, compare as compare_trees, deep_compare as deep_compare_trees,
    deep_compare_with_events as deep_compare_with_events_impl,
};
use crate::difference::visit_differences as visit_differences_impl;
use crate::family::{
    CachedFetchResult, MissingNodeReporter, OwnerBackedFetchState, SHAMapFamily, SHAMapNodeFetcher,
};
use crate::fetch::{AsyncDescendResult, SHAMapSyncFilter, check_filter, check_filter_with_family};
use crate::item::SHAMapItem;
use crate::iteration::{
    lower_bound as lower_bound_impl, peek_first_item as peek_first_item_impl,
    peek_next_item as peek_next_item_impl, upper_bound as upper_bound_impl,
};
use crate::node_id::SHAMapNodeId;
use crate::proof_path::{get_proof_path_backed, has_inner_node, has_leaf_node_backed};
use crate::read::{
    has_item as has_item_impl, peek_item as peek_item_impl,
    peek_item_with_hash as peek_item_with_hash_impl,
};
use crate::search::{NodePathEntry, find_key as find_key_impl};
use crate::traversal::{TraversalError, descend_no_store, descend_throw};
use crate::tree_node::{BRANCH_FACTOR, SHAMapCodecError, SHAMapTreeNode};
use crate::visitor::{visit_leaves as visit_leaves_impl, visit_nodes as visit_nodes_impl};
use basics::base_uint::Uint256;
use basics::blob::Blob;
use basics::intrusive_pointer::{SharedIntrusive, make_shared_intrusive};
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::CacheClock;
use parking_lot::Mutex;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::hash::BuildHasher;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

pub use crate::family::{FullBelowCache, NullFullBelowCache};

pub const DEFAULT_MAX_DEFERRED_MISSING_NODE_READS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SHAMapType {
    Transaction = 1,
    State = 2,
    Free = 3,
}

impl fmt::Display for SHAMapType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SHAMapType::Transaction => f.write_str("Transaction Tree"),
            SHAMapType::State => f.write_str("State Tree"),
            SHAMapType::Free => f.write_str("Free Tree"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingNodeRef {
    Hash(SHAMapHash),
    Id(Uint256),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SHAMapMissingNode {
    map_type: SHAMapType,
    locator: MissingNodeRef,
}

impl SHAMapMissingNode {
    pub fn from_hash(map_type: SHAMapType, hash: SHAMapHash) -> Self {
        Self {
            map_type,
            locator: MissingNodeRef::Hash(hash),
        }
    }

    pub fn from_id(map_type: SHAMapType, id: Uint256) -> Self {
        Self {
            map_type,
            locator: MissingNodeRef::Id(id),
        }
    }

    pub fn map_type(&self) -> SHAMapType {
        self.map_type
    }

    pub fn locator(&self) -> MissingNodeRef {
        self.locator
    }

    pub fn hash(&self) -> Option<SHAMapHash> {
        match self.locator {
            MissingNodeRef::Hash(hash) => Some(hash),
            MissingNodeRef::Id(_) => None,
        }
    }

    pub fn id(&self) -> Option<Uint256> {
        match self.locator {
            MissingNodeRef::Id(id) => Some(id),
            MissingNodeRef::Hash(_) => None,
        }
    }
}

impl fmt::Display for SHAMapMissingNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.locator {
            MissingNodeRef::Hash(hash) => {
                write!(f, "Missing Node: {}: hash {}", self.map_type, hash)
            }
            MissingNodeRef::Id(id) => write!(f, "Missing Node: {}: id {}", self.map_type, id),
        }
    }
}

impl Error for SHAMapMissingNode {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SHAMapAddNode {
    good: i32,
    bad: i32,
    duplicate: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddRootNodeEvent {
    DuplicateRoot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddKnownNodeEvent {
    NotSynching,
    EmptyBranch {
        wanted: SHAMapNodeId,
    },
    CorruptNode,
    LeafPositionMismatch {
        expected: Uint256,
        actual: Uint256,
    },
    UnableToHook {
        wanted: SHAMapNodeId,
        stuck: SHAMapNodeId,
    },
    LateDuplicate,
}

impl SHAMapAddNode {
    pub fn inc_invalid(&mut self) {
        self.bad += 1;
    }

    pub fn inc_useful(&mut self) {
        self.good += 1;
    }

    pub fn inc_duplicate(&mut self) {
        self.duplicate += 1;
    }

    pub fn reset(&mut self) {
        self.good = 0;
        self.bad = 0;
        self.duplicate = 0;
    }

    pub fn duplicate() -> Self {
        Self {
            good: 0,
            bad: 0,
            duplicate: 1,
        }
    }

    pub fn useful() -> Self {
        Self {
            good: 1,
            bad: 0,
            duplicate: 0,
        }
    }

    pub fn invalid() -> Self {
        Self {
            good: 0,
            bad: 1,
            duplicate: 0,
        }
    }

    pub fn get_good(&self) -> i32 {
        self.good
    }

    pub fn get_bad(&self) -> i32 {
        self.bad
    }

    pub fn get_duplicate(&self) -> i32 {
        self.duplicate
    }

    pub fn is_good(&self) -> bool {
        (self.good + self.duplicate) > self.bad
    }

    pub fn is_invalid(&self) -> bool {
        self.bad > 0
    }

    pub fn is_useful(&self) -> bool {
        self.good > 0
    }

    pub fn get(&self) -> String {
        let mut parts = Vec::new();
        if self.good > 0 {
            parts.push(format!("good:{}", self.good));
        }
        if self.bad > 0 {
            parts.push(format!("bad:{}", self.bad));
        }
        if self.duplicate > 0 {
            parts.push(format!("dupe:{}", self.duplicate));
        }

        if parts.is_empty() {
            "no nodes processed".to_owned()
        } else {
            parts.join(" ")
        }
    }
}

impl std::ops::AddAssign for SHAMapAddNode {
    fn add_assign(&mut self, rhs: Self) {
        self.good += rhs.good;
        self.bad += rhs.bad;
        self.duplicate += rhs.duplicate;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    Modifying,
    Immutable,
    Synching,
    Invalid,
}

#[derive(Debug, Clone)]
pub struct SyncTree {
    root: SharedIntrusive<SHAMapTreeNode>,
    map_type: SHAMapType,
    backed: bool,
    ledger_seq: u32,
    state: SyncState,
    owner_fetch_state: OwnerBackedFetchState,
}

impl SyncTree {
    pub fn new(backed: bool, ledger_seq: u32) -> Self {
        Self::new_with_type(SHAMapType::Free, backed, ledger_seq)
    }

    pub fn new_with_type(map_type: SHAMapType, backed: bool, ledger_seq: u32) -> Self {
        Self::new_with_type_and_state(map_type, backed, ledger_seq, SyncState::Modifying)
    }

    pub fn new_synching_with_type(map_type: SHAMapType, backed: bool, ledger_seq: u32) -> Self {
        Self::new_with_type_and_state(map_type, backed, ledger_seq, SyncState::Synching)
    }

    fn new_with_type_and_state(
        map_type: SHAMapType,
        backed: bool,
        ledger_seq: u32,
        state: SyncState,
    ) -> Self {
        Self {
            root: make_shared_intrusive(SHAMapTreeNode::new_inner(0)),
            map_type,
            backed,
            ledger_seq,
            state,
            owner_fetch_state: OwnerBackedFetchState::default(),
        }
    }

    pub fn from_root(
        root: SharedIntrusive<SHAMapTreeNode>,
        backed: bool,
        ledger_seq: u32,
        state: SyncState,
    ) -> Self {
        Self::from_root_with_type(root, SHAMapType::Free, backed, ledger_seq, state)
    }

    pub fn from_root_with_type(
        root: SharedIntrusive<SHAMapTreeNode>,
        map_type: SHAMapType,
        backed: bool,
        ledger_seq: u32,
        state: SyncState,
    ) -> Self {
        Self {
            root,
            map_type,
            backed,
            ledger_seq,
            state,
            owner_fetch_state: OwnerBackedFetchState::default(),
        }
    }

    pub fn mutable_snapshot(&self) -> Self {
        Self {
            root: clone_sync_subtree_as_shareable(&self.root, next_snapshot_cowid(&self.root)),
            map_type: self.map_type,
            backed: self.backed,
            ledger_seq: self.ledger_seq,
            state: SyncState::Modifying,
            owner_fetch_state: OwnerBackedFetchState::default(),
        }
    }

    /// Create a mutable snapshot by sharing the root directly.
    /// The MutableTree's copy-on-write (unshare_node) will clone nodes
    /// when they're modified. This avoids the deep clone of
    /// clone_sync_subtree_as_shareable which can lose track of nodes
    /// when the tree is backed by NuDB.
    pub fn share_root_snapshot(&self) -> Self {
        // Ensure root hash is computed before sharing (reference SHAMap copy
        // constructor shares the root which already has its hash set from
        // the sync/load process).
        if self.root.get_hash().is_zero() && self.root.is_inner() {
            self.root.update_hash();
        }
        // The full_ flag indicates sync completeness, not mutability.
        // A new modifying ledger starts as not-full even if the source was full.
        Self {
            root: self.root.clone(),
            map_type: self.map_type,
            backed: self.backed,
            ledger_seq: self.ledger_seq,
            state: SyncState::Modifying,
            owner_fetch_state: OwnerBackedFetchState::default(),
        }
    }

    pub fn root(&self) -> SharedIntrusive<SHAMapTreeNode> {
        self.root.clone()
    }

    pub fn hash(&mut self) -> SHAMapHash {
        let hash = self.root.get_hash();
        if !hash.is_zero() {
            return hash;
        }
        // Since dirtyUp leaves hashes dirty (zero), we must recursively
        // recompute from leaves up before reading the root hash.
        recompute_hashes_recursive(&self.root);
        self.root.get_hash()
    }

    pub fn serialize_root(&self) -> Result<Blob, SHAMapCodecError> {
        self.root.serialize_for_wire()
    }

    pub fn state(&self) -> SyncState {
        self.state
    }

    pub fn map_type(&self) -> SHAMapType {
        self.map_type
    }

    pub fn set_ledger_seq(&mut self, ledger_seq: u32) {
        self.ledger_seq = ledger_seq;
    }

    pub fn leaf_count(&self) -> usize {
        0
    }

    pub fn set_unbacked(&mut self) {
        self.backed = false;
    }

    pub fn backed(&self) -> bool {
        self.backed
    }

    pub fn is_full(&self) -> bool {
        self.owner_fetch_state.is_full()
    }

    pub fn set_full(&self) {
        self.owner_fetch_state.set_full();
    }

    pub fn is_valid(&self) -> bool {
        self.state != SyncState::Invalid
    }

    pub fn is_synching(&self) -> bool {
        self.state == SyncState::Synching
    }

    pub fn is_invalid(&self) -> bool {
        self.state == SyncState::Invalid
    }

    pub fn set_synching(&mut self) {
        self.state = SyncState::Synching;
    }

    pub fn set_immutable(&mut self) {
        assert!(
            self.state != SyncState::Invalid,
            "invalid sync trees cannot become immutable"
        );
        self.state = SyncState::Immutable;
    }

    pub fn clear_synching(&mut self) {
        self.state = SyncState::Modifying;
    }

    fn load_node_with_owner_family<CLOCK, S, FB, F, MR, NS>(
        &self,
        hash: SHAMapHash,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Option<SharedIntrusive<SHAMapTreeNode>>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        match family.fetch_cached_node_result_with_ledger_seq(hash, self.ledger_seq) {
            CachedFetchResult::Found(node) => Some(node),
            CachedFetchResult::Missing => {
                self.owner_fetch_state
                    .report_missing_node_once(self.ledger_seq, hash, family);
                None
            }
            CachedFetchResult::InvalidBlob => None,
        }
    }

    pub fn fetch_node_nt_filtered_with_family<CLOCK, S, FB, F, MR, NS>(
        &self,
        hash: SHAMapHash,
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Option<SharedIntrusive<SHAMapTreeNode>>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        if let Some(node) = family.cache_lookup(hash) {
            return Some(node);
        }

        if self.backed
            && let Some(node) = self.load_node_with_owner_family(hash, family)
        {
            return Some(node);
        }

        check_filter_with_family(hash, self.backed, self.ledger_seq, family, filter)
    }

    pub fn fetch_node_nt_with_family<CLOCK, S, FB, F, MR, NS>(
        &self,
        hash: SHAMapHash,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Option<SharedIntrusive<SHAMapTreeNode>>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        self.fetch_node_nt_filtered_with_family(hash, &mut no_filter, family)
    }

    pub fn fetch_node_with_family<CLOCK, S, FB, F, MR, NS>(
        &self,
        hash: SHAMapHash,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<SharedIntrusive<SHAMapTreeNode>, SHAMapMissingNode>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        self.fetch_node_nt_with_family(hash, family)
            .ok_or_else(|| SHAMapMissingNode::from_hash(self.map_type, hash))
    }

    pub fn descend_with_family<CLOCK, S, FB, F, MR, NS>(
        &self,
        parent: &SharedIntrusive<SHAMapTreeNode>,
        branch: usize,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Option<SharedIntrusive<SHAMapTreeNode>>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        assert!(
            parent.is_inner(),
            "descend_with_family requires an inner parent node"
        );

        let loaded = parent.get_child(branch);
        if loaded.is_some() || !self.backed || parent.is_empty_branch(branch) {
            return loaded;
        }

        let child_hash = parent.get_child_hash(branch);
        let node_hash = child_hash;
        let depth = branch;
        tracing::trace!(target: "shamap", hash = %node_hash, depth, "Tree traversal");
        let child = self.fetch_node_nt_with_family(child_hash, family)?;
        debug_assert_eq!(
            child.get_hash(),
            child_hash,
            "owner-backed descend should preserve the requested child hash"
        );
        Some(parent.canonicalize_child(branch, child))
    }

    pub fn descend_throw_with_family<CLOCK, S, FB, F, MR, NS>(
        &self,
        parent: &SharedIntrusive<SHAMapTreeNode>,
        branch: usize,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, SHAMapMissingNode>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        let child = self.descend_with_family(parent, branch, family);
        if child.is_none() && !parent.is_empty_branch(branch) {
            return Err(SHAMapMissingNode::from_hash(
                self.map_type,
                parent.get_child_hash(branch),
            ));
        }
        Ok(child)
    }

    pub fn descend_no_store_with_family<CLOCK, S, FB, F, MR, NS>(
        &self,
        parent: &SharedIntrusive<SHAMapTreeNode>,
        branch: usize,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, SHAMapMissingNode>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        assert!(
            parent.is_inner(),
            "descend_no_store_with_family requires an inner parent node"
        );

        let loaded = parent.get_child(branch);
        if loaded.is_some() || !self.backed || parent.is_empty_branch(branch) {
            return Ok(loaded);
        }

        self.fetch_node_with_family(parent.get_child_hash(branch), family)
            .map(Some)
    }

    pub fn descend_async_with_family<CLOCK, S, FB, F, MR, NS, REQ>(
        &self,
        parent: &SharedIntrusive<SHAMapTreeNode>,
        branch: usize,
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
        request_async_fetch: &mut REQ,
    ) -> AsyncDescendResult
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        REQ: FnMut(SHAMapHash, u32),
    {
        crate::fetch::descend_async_with_family(
            parent,
            branch,
            self.backed,
            self.ledger_seq,
            family,
            filter,
            request_async_fetch,
        )
    }

    pub fn add_root_node(
        &mut self,
        hash: SHAMapHash,
        root_node: &[u8],
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
    ) -> SHAMapAddNode {
        self.add_root_node_impl(hash, root_node, filter, &mut |_, _| {}, &mut |_| {})
    }

    pub fn add_root_node_with_family<CLOCK, S, FB, F, MR, NS>(
        &mut self,
        hash: SHAMapHash,
        root_node: &[u8],
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> SHAMapAddNode
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
    {
        let backed = self.backed;
        self.add_root_node_impl(
            hash,
            root_node,
            filter,
            &mut |hash, node| {
                if backed {
                    family.canonicalize(hash, node);
                }
            },
            &mut |event| match event {
                AddRootNodeEvent::DuplicateRoot => {
                    family.log_trace("got root node, already have one");
                }
            },
        )
    }

    fn add_root_node_impl<PREPARE, REPORT>(
        &mut self,
        hash: SHAMapHash,
        root_node: &[u8],
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        prepare_node: &mut PREPARE,
        report_event: &mut REPORT,
    ) -> SHAMapAddNode
    where
        PREPARE: FnMut(SHAMapHash, &mut SharedIntrusive<SHAMapTreeNode>),
        REPORT: FnMut(AddRootNodeEvent),
    {
        if self.root.get_hash().is_non_zero() {
            report_event(AddRootNodeEvent::DuplicateRoot);
            debug_assert_eq!(
                self.root.get_hash(),
                hash,
                "existing non-empty sync root should match duplicate add_root_node hash"
            );
            return SHAMapAddNode::duplicate();
        }

        let Ok(Some(mut node)) = SHAMapTreeNode::make_from_wire(root_node) else {
            return SHAMapAddNode::invalid();
        };
        if node.get_hash() != hash {
            return SHAMapAddNode::invalid();
        }

        prepare_node(hash, &mut node);
        self.root = node;
        if self.root.is_leaf() {
            self.clear_synching();
        }

        notify_filter_of_accepted_node(filter, false, hash, self.ledger_seq, &self.root);
        SHAMapAddNode::useful()
    }

    pub fn fetch_root_with_family<CLOCK, S, C, F, MR, NS>(
        &mut self,
        hash: SHAMapHash,
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> bool
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: crate::family::MissingNodeReporter,
    {
        if hash == self.root.get_hash() {
            return true;
        }

        let map_label = match self.map_type {
            SHAMapType::Transaction => "TXN",
            SHAMapType::State => "STATE",
            SHAMapType::Free => "SHAMap",
        };
        family.log_trace(&format!("Fetch root {map_label} node {hash}"));

        let Some(new_root) = self.fetch_node_nt_filtered_with_family(hash, filter, family) else {
            return false;
        };

        debug_assert_eq!(
            new_root.get_hash(),
            hash,
            "fetched root should preserve the requested hash"
        );
        self.root = new_root;
        true
    }

    pub fn add_known_node<F, C>(
        &mut self,
        node_id: SHAMapNodeId,
        raw_node: &[u8],
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        full_below_cache: &C,
        fetch: &mut F,
    ) -> SHAMapAddNode
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
        C: FullBelowCache,
    {
        self.add_known_node_impl(
            node_id,
            raw_node,
            filter,
            full_below_cache,
            fetch,
            &mut |_, _| {},
            &mut |parent, branch, ledger_seq, backed, fetch, filter| {
                resolve_sync_child_for_add_known_node(
                    parent, branch, ledger_seq, backed, fetch, filter,
                )
            },
            &mut |_| {},
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn add_known_node_impl<F, C, PREPARE, RESOLVE, REPORT>(
        &mut self,
        node_id: SHAMapNodeId,
        raw_node: &[u8],
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        full_below_cache: &C,
        fetch: &mut F,
        prepare_node: &mut PREPARE,
        resolve_child: &mut RESOLVE,
        report_event: &mut REPORT,
    ) -> SHAMapAddNode
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
        C: FullBelowCache,
        PREPARE: FnMut(SHAMapHash, &mut SharedIntrusive<SHAMapTreeNode>),
        RESOLVE: FnMut(
            &SHAMapTreeNode,
            usize,
            u32,
            bool,
            &mut F,
            &mut Option<&mut dyn SHAMapSyncFilter>,
        ) -> Option<SharedIntrusive<SHAMapTreeNode>>,
        REPORT: FnMut(AddKnownNodeEvent),
    {
        assert!(
            !node_id.is_root(),
            "add_known_node requires a non-root SHAMap node id"
        );

        if !self.is_synching() {
            report_event(AddKnownNodeEvent::NotSynching);
            return SHAMapAddNode::duplicate();
        }

        let generation = full_below_cache.generation();
        let mut curr_node_id = SHAMapNodeId::default();
        let mut curr_node = &*self.root as *const SHAMapTreeNode;

        while unsafe { &*curr_node }.is_inner()
            && !unsafe { &*curr_node }.is_full_below(generation)
            && curr_node_id.get_depth() < node_id.get_depth()
        {
            let curr_node_ref = unsafe { &*curr_node };
            let branch = crate::node_id::select_branch(curr_node_id, node_id.get_node_id());
            if curr_node_ref.is_empty_branch(branch) {
                report_event(AddKnownNodeEvent::EmptyBranch { wanted: node_id });
                return SHAMapAddNode::invalid();
            }

            let prev_node_id = curr_node_id;
            curr_node_id = prev_node_id
                .get_child_node_id(branch)
                .expect("branch selection must stay within SHAMap depth bounds");

            let child_hash = curr_node_ref.get_child_hash(branch);
            if full_below_cache.touch_if_exists(*child_hash.as_uint256()) {
                return SHAMapAddNode::duplicate();
            }

            if let Some(loaded) = unsafe { curr_node_ref.get_child_ptr(branch) } {
                curr_node = loaded;
                continue;
            }

            let resolved = resolve_child(
                curr_node_ref,
                branch,
                self.ledger_seq,
                self.backed,
                fetch,
                filter,
            );
            if let Some(next_node) = resolved {
                curr_node = &*next_node as *const SHAMapTreeNode;
                continue;
            }

            // Child not found — attach the new node (rare path)

            let Ok(Some(mut new_node)) = SHAMapTreeNode::make_from_wire(raw_node) else {
                report_event(AddKnownNodeEvent::CorruptNode);
                return SHAMapAddNode::invalid();
            };
            if child_hash != new_node.get_hash() {
                tracing::warn!(target: "shamap", expected = %child_hash, "Node hash mismatch — corrupt data");
                report_event(AddKnownNodeEvent::CorruptNode);
                return SHAMapAddNode::invalid();
            }

            if new_node.is_leaf() {
                let actual_key = new_node
                    .peek_item()
                    .expect("decoded SHAMap leaf nodes should carry an item")
                    .key();
                let Ok(expected_node_id) = SHAMapNodeId::create_id(node_id.get_depth(), actual_key)
                else {
                    return SHAMapAddNode::invalid();
                };
                if expected_node_id.get_node_id() != node_id.get_node_id() {
                    report_event(AddKnownNodeEvent::LeafPositionMismatch {
                        expected: expected_node_id.get_node_id(),
                        actual: node_id.get_node_id(),
                    });
                    return SHAMapAddNode::invalid();
                }
            }

            if (curr_node_id.get_depth() > crate::node_id::SHAMAP_LEAF_DEPTH)
                || (new_node.is_inner()
                    && curr_node_id.get_depth() == crate::node_id::SHAMAP_LEAF_DEPTH)
            {
                self.state = SyncState::Invalid;
                return SHAMapAddNode::useful();
            }

            if curr_node_id != node_id {
                report_event(AddKnownNodeEvent::UnableToHook {
                    wanted: node_id,
                    stuck: curr_node_id,
                });
                return SHAMapAddNode::useful();
            }

            prepare_node(child_hash, &mut new_node);
            let attached = curr_node_ref.canonicalize_child(branch, new_node);
            tracing::debug!(target: "shamap", node_hash = %child_hash, "Node added to tree");
            notify_filter_of_accepted_node(filter, false, child_hash, self.ledger_seq, &attached);
            return SHAMapAddNode::useful();
        }

        report_event(AddKnownNodeEvent::LateDuplicate);
        SHAMapAddNode::duplicate()
    }

    pub fn add_known_node_with_family<CLOCK, S, C, F, MR, NS>(
        &mut self,
        node_id: SHAMapNodeId,
        raw_node: &[u8],
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> SHAMapAddNode
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        let backed = self.backed;
        let ledger_seq = self.ledger_seq;
        let owner_fetch_state = &self.owner_fetch_state as *const OwnerBackedFetchState;
        family.with_full_below_cache(|full_below_cache| {
            self.add_known_node_impl(
                node_id,
                raw_node,
                filter,
                full_below_cache,
                &mut |_| None,
                &mut |hash, node| {
                    if backed {
                        family.canonicalize(hash, node);
                    }
                },
                &mut |parent, branch, _ledger_seq, _backed, _fetch, filter| {
                    resolve_sync_child_for_add_known_node_with_family(
                        parent,
                        branch,
                        backed,
                        ledger_seq,
                        // Preserve the real owner-backed fetch state across
                        // addKnownNode descents instead of mutating a clone.
                        unsafe { &*owner_fetch_state },
                        family,
                        filter,
                    )
                },
                &mut |event| match event {
                    AddKnownNodeEvent::NotSynching => {
                        family.log_trace("AddKnownNode while not synching");
                    }
                    AddKnownNodeEvent::EmptyBranch { wanted } => {
                        family.log_warn(&format!("Add known node for empty branch{wanted}"));
                    }
                    AddKnownNodeEvent::CorruptNode => {
                        family.log_warn("Corrupt node received");
                    }
                    AddKnownNodeEvent::LeafPositionMismatch { expected, actual } => {
                        family.log_debug(&format!(
                            "Leaf node position mismatch: expected={expected}, actual={actual}"
                        ));
                    }
                    AddKnownNodeEvent::UnableToHook { wanted, stuck } => {
                        if std::env::var("XRPLD_FULL_SYNC_DEBUG")
                            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
                            .unwrap_or(false)
                        {
                            tracing::debug!(
                                target: "shamap",
                                "[full_debug][add_known_unable_to_hook] ledger_seq={} wanted={} stuck={} wanted_depth={} stuck_depth={}",
                                ledger_seq,
                                wanted,
                                stuck,
                                wanted.get_depth(),
                                stuck.get_depth()
                            );
                        }
                        family.log_warn(&format!("unable to hook node {wanted}"));
                        family.log_info(&format!(" stuck at {stuck}"));
                        family.log_info(&format!(
                            "got depth={}, walked to= {}",
                            wanted.get_depth(),
                            stuck.get_depth()
                        ));
                    }
                    AddKnownNodeEvent::LateDuplicate => {
                        family.log_trace("got node, already had it (late)");
                    }
                },
            )
        })
    }

    pub fn get_missing_nodes<F, R, C>(
        &mut self,
        max: i32,
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        full_below_cache: &C,
        fetch: &mut F,
        next_first_child: &mut R,
    ) -> Vec<(SHAMapNodeId, Uint256)>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
        R: FnMut() -> u8,
        C: FullBelowCache,
    {
        let missing = get_missing_nodes(
            &self.root,
            max,
            self.ledger_seq,
            self.backed,
            fetch,
            filter,
            full_below_cache,
            next_first_child,
        );
        if missing.is_empty() {
            self.clear_synching();
        } else {
            let missing_count = missing.len();
            tracing::debug!(target: "shamap", missing_count, "Missing nodes identified");
        }
        missing
    }

    pub fn get_missing_nodes_with_family<CLOCK, S, C, F, R, MR, NS>(
        &mut self,
        max: i32,
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        next_first_child: &mut R,
    ) -> Vec<(SHAMapNodeId, Uint256)>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        R: FnMut() -> u8,
        MR: MissingNodeReporter,
    {
        let mut scan =
            self.start_deferred_missing_node_scan_with_family(max, family, next_first_child);

        while !scan.is_complete() && scan.remaining() > 0 {
            scan.run_with_family(
                family,
                filter,
                DEFAULT_MAX_DEFERRED_MISSING_NODE_READS,
                next_first_child,
                &mut |_, _| {},
            );

            let pending = scan.pending_requests();
            if pending.is_empty() {
                continue;
            }

            let completions = pending
                .into_iter()
                .map(|request| self.load_node_with_owner_family(request.hash(), family))
                .collect::<Vec<_>>();
            scan.complete_pending_reads(completions);
        }

        let missing = scan.into_missing_nodes();
        if missing.is_empty() {
            self.clear_synching();
        }
        missing
    }

    pub fn get_missing_nodes_with_family_diagnostics<CLOCK, S, C, F, R, MR, NS>(
        &mut self,
        max: i32,
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        next_first_child: &mut R,
    ) -> (Vec<(SHAMapNodeId, Uint256)>, DeferredMissingNodeScanStats)
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        R: FnMut() -> u8,
        MR: MissingNodeReporter,
    {
        let mut scan =
            self.start_deferred_missing_node_scan_with_family(max, family, next_first_child);

        while !scan.is_complete() && scan.remaining() > 0 {
            scan.run_with_family(
                family,
                filter,
                DEFAULT_MAX_DEFERRED_MISSING_NODE_READS,
                next_first_child,
                &mut |_, _| {},
            );

            let pending = scan.pending_requests();
            if pending.is_empty() {
                continue;
            }

            let completions = pending
                .into_iter()
                .map(|request| self.load_node_with_owner_family(request.hash(), family))
                .collect::<Vec<_>>();
            scan.complete_pending_reads(completions);
        }

        let (missing, stats) = scan.into_missing_nodes_and_stats();
        if missing.is_empty() {
            self.clear_synching();
        }
        (missing, stats)
    }

    pub fn start_deferred_missing_node_scan<R, C>(
        &self,
        max: i32,
        full_below_cache: &C,
        next_first_child: &mut R,
    ) -> DeferredMissingNodeScan
    where
        R: FnMut() -> u8,
        C: FullBelowCache,
    {
        DeferredMissingNodeScan::new(
            &self.root,
            max,
            self.backed,
            self.ledger_seq,
            full_below_cache,
            next_first_child,
        )
    }

    pub fn start_deferred_missing_node_scan_with_family<CLOCK, S, C, F, MR, NS, R>(
        &self,
        max: i32,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        next_first_child: &mut R,
    ) -> DeferredMissingNodeScan
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        R: FnMut() -> u8,
    {
        family.with_full_below_cache(|full_below_cache| {
            self.start_deferred_missing_node_scan(max, full_below_cache, next_first_child)
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_missing_nodes_deferred_with_family<CLOCK, S, C, F, R, REQ, COMPLETE, MR, NS>(
        &mut self,
        max: i32,
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        max_deferred: usize,
        next_first_child: &mut R,
        request_async_fetch: &mut REQ,
        complete_async_fetches: &mut COMPLETE,
    ) -> Vec<(SHAMapNodeId, Uint256)>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        R: FnMut() -> u8,
        REQ: FnMut(SHAMapHash, u32),
        COMPLETE:
            FnMut(Vec<DeferredFetchRequestInfo>) -> Vec<Option<SharedIntrusive<SHAMapTreeNode>>>,
    {
        let mut scan =
            self.start_deferred_missing_node_scan_with_family(max, family, next_first_child);

        while !scan.is_complete() && scan.remaining() > 0 {
            scan.run_with_family(
                family,
                filter,
                max_deferred,
                next_first_child,
                request_async_fetch,
            );

            let pending = scan.pending_requests();
            if pending.is_empty() {
                continue;
            }

            let completions = complete_async_fetches(pending);
            scan.complete_pending_reads(completions);
        }

        let missing = scan.into_missing_nodes();
        if missing.is_empty() {
            self.clear_synching();
        }
        missing
    }

    pub fn walk_map<F>(
        &self,
        map_type: SHAMapType,
        missing_nodes: &mut Vec<SHAMapMissingNode>,
        max_missing: i32,
        fetch: &mut F,
    ) where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        walk_map(
            &self.root,
            map_type,
            missing_nodes,
            max_missing,
            self.backed,
            fetch,
        );
    }

    pub fn walk_map_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        map_type: SHAMapType,
        missing_nodes: &mut Vec<SHAMapMissingNode>,
        max_missing: i32,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        self.walk_map(map_type, missing_nodes, max_missing, &mut |hash| {
            self.load_node_with_owner_family(hash, family)
        });
    }

    pub fn walk_map_parallel<F>(
        &self,
        map_type: SHAMapType,
        missing_nodes: &mut Vec<SHAMapMissingNode>,
        max_missing: i32,
        fetch: &F,
    ) -> bool
    where
        F: Fn(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>> + Sync,
    {
        walk_map_parallel(
            &self.root,
            map_type,
            missing_nodes,
            max_missing,
            self.backed,
            fetch,
        )
    }

    pub fn walk_map_parallel_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        map_type: SHAMapType,
        missing_nodes: &mut Vec<SHAMapMissingNode>,
        max_missing: i32,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> bool
    where
        CLOCK: CacheClock + Send + Sync,
        S: BuildHasher + Clone + Send + Sync,
        C: FullBelowCache + Send,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        NS: Send,
    {
        let result = walk_map_parallel_with_observer(
            &self.root,
            map_type,
            missing_nodes,
            max_missing,
            self.backed,
            &|hash| self.load_node_with_owner_family(hash, family),
            &mut |root_child_index| {
                family.log_debug(&format!("starting worker {root_child_index}"))
            },
        );
        if let ParallelWalkResult::WorkerPanics(panics) = &result {
            let mut message = String::from("Exception(s) in ledger load: ");
            for panic in panics {
                message.push_str(panic);
                message.push_str(", ");
            }
            family.log_error(&message);
        }
        result.completed()
    }

    pub fn has_leaf_node<F>(
        &self,
        tag: Uint256,
        target_node_hash: SHAMapHash,
        fetch: &mut F,
    ) -> Result<bool, TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        crate::proof_path::has_leaf_node_backed(
            &self.root,
            tag,
            target_node_hash,
            self.backed,
            fetch,
        )
    }

    pub fn has_leaf_node_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        tag: Uint256,
        target_node_hash: SHAMapHash,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<bool, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: crate::family::MissingNodeReporter,
    {
        has_leaf_node_backed(
            &self.root,
            tag,
            target_node_hash,
            self.backed,
            &mut |hash| self.load_node_with_owner_family(hash, family),
        )
    }

    pub fn has_inner_node<F>(
        &self,
        target_node_id: SHAMapNodeId,
        target_node_hash: SHAMapHash,
        fetch: &mut F,
    ) -> Result<bool, TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        crate::proof_path::has_inner_node(
            &self.root,
            target_node_id,
            target_node_hash,
            self.backed,
            fetch,
        )
    }

    pub fn has_inner_node_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        target_node_id: SHAMapNodeId,
        target_node_hash: SHAMapHash,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<bool, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: crate::family::MissingNodeReporter,
    {
        has_inner_node(
            &self.root,
            target_node_id,
            target_node_hash,
            self.backed,
            &mut |hash| self.load_node_with_owner_family(hash, family),
        )
    }

    pub fn get_proof_path<F>(
        &self,
        key: Uint256,
        fetch: &mut F,
    ) -> Result<Option<Vec<Blob>>, TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        crate::proof_path::get_proof_path_backed(&self.root, key, self.backed, fetch)
    }

    pub fn get_proof_path_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        key: Uint256,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<Option<Vec<Blob>>, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: crate::family::MissingNodeReporter,
    {
        get_proof_path_backed(&self.root, key, self.backed, &mut |hash| {
            self.load_node_with_owner_family(hash, family)
        })
    }

    pub fn has_item<F>(&self, id: Uint256, fetch: &mut F) -> Result<bool, TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        has_item_impl(&self.root, id, self.backed, fetch)
    }

    pub fn has_item_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        id: Uint256,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<bool, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        has_item_impl(&self.root, id, self.backed, &mut |hash| {
            self.load_node_with_owner_family(hash, family)
        })
    }

    pub fn peek_item<F>(
        &self,
        id: Uint256,
        fetch: &mut F,
    ) -> Result<Option<SHAMapItem>, TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        peek_item_impl(&self.root, id, self.backed, fetch)
    }

    pub fn peek_item_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        id: Uint256,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<Option<SHAMapItem>, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        peek_item_impl(&self.root, id, self.backed, &mut |hash| {
            self.load_node_with_owner_family(hash, family)
        })
    }

    pub fn peek_item_with_hash<F>(
        &self,
        id: Uint256,
        fetch: &mut F,
    ) -> Result<Option<(SHAMapItem, SHAMapHash)>, TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        peek_item_with_hash_impl(&self.root, id, self.backed, fetch)
    }

    pub fn peek_item_with_hash_and_family<CLOCK, S, C, F, MR, NS>(
        &self,
        id: Uint256,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<Option<(SHAMapItem, SHAMapHash)>, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        peek_item_with_hash_impl(&self.root, id, self.backed, &mut |hash| {
            self.load_node_with_owner_family(hash, family)
        })
    }

    pub fn find_key<F>(
        &self,
        key: Uint256,
        fetch: &mut F,
    ) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        find_key_impl(&self.root, key, self.backed, fetch)
    }

    pub fn find_key_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        key: Uint256,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        find_key_impl(&self.root, key, self.backed, &mut |hash| {
            self.load_node_with_owner_family(hash, family)
        })
    }

    pub fn peek_first_item<F>(
        &self,
        stack: &mut Vec<NodePathEntry>,
        fetch: &mut F,
    ) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        peek_first_item_impl(&self.root, stack, self.backed, fetch)
    }

    pub fn peek_first_item_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        stack: &mut Vec<NodePathEntry>,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        peek_first_item_impl(&self.root, stack, self.backed, &mut |hash| {
            self.load_node_with_owner_family(hash, family)
        })
    }

    pub fn peek_next_item<F>(
        &self,
        id: Uint256,
        stack: &mut Vec<NodePathEntry>,
        fetch: &mut F,
    ) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        peek_next_item_impl(id, stack, self.backed, fetch)
    }

    pub fn peek_next_item_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        id: Uint256,
        stack: &mut Vec<NodePathEntry>,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        peek_next_item_impl(id, stack, self.backed, &mut |hash| {
            self.load_node_with_owner_family(hash, family)
        })
    }

    pub fn upper_bound<F>(
        &self,
        id: Uint256,
        fetch: &mut F,
    ) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        upper_bound_impl(&self.root, id, self.backed, fetch)
    }

    pub fn upper_bound_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        id: Uint256,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        upper_bound_impl(&self.root, id, self.backed, &mut |hash| {
            self.load_node_with_owner_family(hash, family)
        })
    }

    pub fn lower_bound<F>(
        &self,
        id: Uint256,
        fetch: &mut F,
    ) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        lower_bound_impl(&self.root, id, self.backed, fetch)
    }

    pub fn lower_bound_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        id: Uint256,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        lower_bound_impl(&self.root, id, self.backed, &mut |hash| {
            self.load_node_with_owner_family(hash, family)
        })
    }

    pub fn visit_nodes<F, V>(&self, fetch: &mut F, visit: &mut V) -> Result<(), TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
        V: FnMut(&SharedIntrusive<SHAMapTreeNode>) -> bool,
    {
        visit_nodes_impl(&self.root, self.backed, fetch, visit)
    }

    pub fn visit_nodes_with_family<CLOCK, S, C, F, MR, NS, V>(
        &self,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        visit: &mut V,
    ) -> Result<(), TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        V: FnMut(&SharedIntrusive<SHAMapTreeNode>) -> bool,
    {
        visit_nodes_impl(
            &self.root,
            self.backed,
            &mut |hash| self.load_node_with_owner_family(hash, family),
            visit,
        )
    }

    pub fn visit_leaves<F, V>(&self, fetch: &mut F, visit: &mut V) -> Result<(), TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
        V: FnMut(&SHAMapItem),
    {
        visit_leaves_impl(&self.root, self.backed, fetch, visit)
    }

    pub fn visit_leaves_with_family<CLOCK, S, C, F, MR, NS, V>(
        &self,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
        visit: &mut V,
    ) -> Result<(), TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        V: FnMut(&SHAMapItem),
    {
        visit_leaves_impl(
            &self.root,
            self.backed,
            &mut |hash| self.load_node_with_owner_family(hash, family),
            visit,
        )
    }


    /// Collect strong references to ALL nodes in this tree, loading from the
    /// family's fetcher if needed. Returns a Vec of SharedIntrusive refs that
    /// pin every node in memory — as long as the Vec is alive, no node can be
    /// evicted by the TreeNodeCache sweep.
    ///
    /// Used by consensus to guarantee zero-I/O traversal of the validated
    /// ledger's state map. Cost: O(N) where N = total nodes in tree.
    pub fn collect_strong_refs_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Vec<SharedIntrusive<SHAMapTreeNode>>
    where
        CLOCK: basics::tagged_cache::CacheClock,
        S: std::hash::BuildHasher + Clone,
        C: crate::family::FullBelowCache,
        F: crate::family::SHAMapNodeFetcher,
        MR: crate::family::MissingNodeReporter,
    {
        let mut refs = Vec::new();
        refs.push(self.root.clone());

        if !self.root.is_inner() {
            return refs;
        }

        let mut stack: Vec<SharedIntrusive<SHAMapTreeNode>> = vec![self.root.clone()];

        while let Some(node) = stack.pop() {
            for branch in 0..16 {
                if node.is_empty_branch(branch) {
                    continue;
                }
                let child = if let Some(c) = node.get_child(branch) {
                    c
                } else if self.backed {
                    let hash = node.get_child_hash(branch);
                    if let Some(c) = self.load_node_with_owner_family(hash, family) {
                        c
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };

                refs.push(child.clone());
                if child.is_inner() {
                    stack.push(child);
                }
            }
        }

        refs
    }

    pub fn get_node_fat<F>(
        &self,
        wanted: SHAMapNodeId,
        data: &mut Vec<(SHAMapNodeId, Blob)>,
        fat_leaves: bool,
        depth: u32,
        fetch: &mut F,
    ) -> Result<bool, TraversalError>
    where
        F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        get_node_fat(
            &self.root,
            wanted,
            data,
            fat_leaves,
            depth,
            self.backed,
            fetch,
        )
    }

    pub fn get_node_fat_with_family<CLOCK, S, C, F, MR, NS>(
        &self,
        wanted: SHAMapNodeId,
        data: &mut Vec<(SHAMapNodeId, Blob)>,
        fat_leaves: bool,
        depth: u32,
        family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    ) -> Result<bool, TraversalError>
    where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        C: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        get_node_fat_with_family(
            &self.root,
            wanted,
            data,
            fat_leaves,
            depth,
            self.backed,
            family,
        )
    }

    pub fn compare_with_families<CLOCKL, SL, CL, FL, MRL, NSL, CLOCKR, SR, CR, FR, MRR, NSR>(
        &self,
        other: &SyncTree,
        differences: &mut Delta,
        max_count: i32,
        left_family: &SHAMapFamily<CLOCKL, SL, CL, FL, MRL, NSL>,
        right_family: &SHAMapFamily<CLOCKR, SR, CR, FR, MRR, NSR>,
    ) -> Result<bool, TraversalError>
    where
        CLOCKL: CacheClock,
        SL: BuildHasher + Clone,
        CL: FullBelowCache,
        FL: SHAMapNodeFetcher,
        MRL: MissingNodeReporter,
        CLOCKR: CacheClock,
        SR: BuildHasher + Clone,
        CR: FullBelowCache,
        FR: SHAMapNodeFetcher,
        MRR: MissingNodeReporter,
    {
        compare_trees(
            &self.root,
            &other.root,
            self.backed,
            &mut |hash| self.load_node_with_owner_family(hash, left_family),
            other.backed,
            &mut |hash| other.load_node_with_owner_family(hash, right_family),
            differences,
            max_count,
        )
    }

    pub fn deep_compare_with_families<CLOCKL, SL, CL, FL, MRL, NSL, CLOCKR, SR, CR, FR, MRR, NSR>(
        &self,
        other: &SyncTree,
        left_family: &SHAMapFamily<CLOCKL, SL, CL, FL, MRL, NSL>,
        right_family: &SHAMapFamily<CLOCKR, SR, CR, FR, MRR, NSR>,
    ) -> bool
    where
        CLOCKL: CacheClock,
        SL: BuildHasher + Clone,
        CL: FullBelowCache,
        FL: SHAMapNodeFetcher,
        MRL: MissingNodeReporter,
        CLOCKR: CacheClock,
        SR: BuildHasher + Clone,
        CR: FullBelowCache,
        FR: SHAMapNodeFetcher,
        MRR: MissingNodeReporter,
    {
        deep_compare_with_events_impl(
            &self.root,
            &other.root,
            self.backed,
            &mut |hash| self.load_node_with_owner_family(hash, left_family),
            other.backed,
            &mut |hash| other.load_node_with_owner_family(hash, right_family),
            &mut |event| match event {
                DeepCompareEvent::HashMismatch => {
                    left_family.log_warn("node hash mismatch");
                }
                DeepCompareEvent::UnableToFetchInnerNode => {
                    left_family.log_warn("unable to fetch inner node");
                }
            },
        )
    }

    pub fn compare<FL, FR>(
        &self,
        other: &SyncTree,
        differences: &mut Delta,
        max_count: i32,
        left_fetch: &mut FL,
        right_fetch: &mut FR,
    ) -> Result<bool, TraversalError>
    where
        FL: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
        FR: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        assert!(
            self.is_valid() && other.is_valid(),
            "compare requires valid sync trees"
        );

        compare_trees(
            &self.root,
            &other.root,
            self.backed,
            left_fetch,
            other.backed,
            right_fetch,
            differences,
            max_count,
        )
    }

    pub fn deep_compare<FL, FR>(
        &self,
        other: &SyncTree,
        left_fetch: &mut FL,
        right_fetch: &mut FR,
    ) -> bool
    where
        FL: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
        FR: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    {
        deep_compare_trees(
            &self.root,
            &other.root,
            self.backed,
            left_fetch,
            other.backed,
            right_fetch,
        )
    }

    pub fn visit_differences<SF, HF, V>(
        &self,
        have: Option<&SyncTree>,
        self_fetch: &mut SF,
        have_fetch: &mut HF,
        visit: &mut V,
    ) -> Result<(), TraversalError>
    where
        SF: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
        HF: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
        V: FnMut(&SharedIntrusive<SHAMapTreeNode>) -> bool,
    {
        let have_root = have.map(|tree| tree.root.clone());
        let have_ref = have_root.as_ref();
        let have_backed = have.map(|tree| tree.backed).unwrap_or(false);

        visit_differences_impl(
            &self.root,
            have_ref,
            self.backed,
            self_fetch,
            have_backed,
            have_fetch,
            visit,
        )
    }

    pub fn visit_differences_with_families<
        CLOCKS,
        SS,
        CS,
        FS,
        MRS,
        NSS,
        CLOCKH,
        SH,
        CH,
        FH,
        MRH,
        NSH,
        V,
    >(
        &self,
        have: Option<&SyncTree>,
        self_family: &SHAMapFamily<CLOCKS, SS, CS, FS, MRS, NSS>,
        have_family: Option<&SHAMapFamily<CLOCKH, SH, CH, FH, MRH, NSH>>,
        visit: &mut V,
    ) -> Result<(), TraversalError>
    where
        CLOCKS: CacheClock,
        SS: BuildHasher + Clone,
        CS: FullBelowCache,
        FS: SHAMapNodeFetcher,
        MRS: MissingNodeReporter,
        CLOCKH: CacheClock,
        SH: BuildHasher + Clone,
        CH: FullBelowCache,
        FH: SHAMapNodeFetcher,
        MRH: MissingNodeReporter,
        V: FnMut(&SharedIntrusive<SHAMapTreeNode>) -> bool,
    {
        let have_root = have.map(|tree| tree.root.clone());
        let have_ref = have_root.as_ref();
        let have_backed = have.map(|tree| tree.backed).unwrap_or(false);

        visit_differences_impl(
            &self.root,
            have_ref,
            self.backed,
            &mut |hash| self.load_node_with_owner_family(hash, self_family),
            have_backed,
            &mut |hash| {
                have.and_then(|tree| {
                    have_family.and_then(|family| tree.load_node_with_owner_family(hash, family))
                })
            },
            visit,
        )
    }
}

#[derive(Debug, Clone)]
/// Scan state for getMissingNodes — uses raw pointer matching reference stack.
/// Safety: parent node on the stack holds SharedIntrusive to children,
/// keeping them alive for the scan duration. Single-threaded access.
struct MissingNodeScanState {
    node: *const SHAMapTreeNode,
    node_id: SHAMapNodeId,
    first_child: usize,
    current_child: usize,
    full_below: bool,
}

unsafe impl Send for MissingNodeScanState {}

impl MissingNodeScanState {
    #[inline(always)]
    fn node(&self) -> &SHAMapTreeNode {
        unsafe { &*self.node }
    }
}

#[derive(Debug, Clone)]
struct DeferredSyncRead {
    parent: *const SHAMapTreeNode,
    parent_id: SHAMapNodeId,
    branch: usize,
    node: Option<SharedIntrusive<SHAMapTreeNode>>,
}

unsafe impl Send for DeferredSyncRead {}

#[derive(Debug, Clone)]
struct DeferredResume {
    node: *const SHAMapTreeNode,
    node_id: SHAMapNodeId,
}

unsafe impl Send for DeferredResume {}

#[derive(Debug, Clone)]
/// Deferred fetch state — uses raw pointer for parent (parent is on scan stack).
struct PendingDeferredFetch {
    parent: *const SHAMapTreeNode,
    parent_id: SHAMapNodeId,
    branch: usize,
    hash: SHAMapHash,
    ledger_seq: u32,
}

unsafe impl Send for PendingDeferredFetch {}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeferredMissingNodeScanStats {
    pub branches_seen: u64,
    pub duplicate_missing_hashes: u64,
    pub full_below_hits: u64,
    pub loaded_or_cached_children: u64,
    pub leaf_children: u64,
    pub inner_children: u64,
    pub full_below_inner_children: u64,
    pub pending_reads: u64,
    pub completed_pending_reads: u64,
    pub completed_pending_misses: u64,
    pub missing_recorded: u64,
    pub full_below_marked: u64,
    pub deferred_resumes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeferredFetchRequestInfo {
    hash: SHAMapHash,
    ledger_seq: u32,
}

impl DeferredFetchRequestInfo {
    pub fn hash(&self) -> SHAMapHash {
        self.hash
    }

    pub fn ledger_seq(&self) -> u32 {
        self.ledger_seq
    }
}

#[derive(Debug, Clone)]
pub struct DeferredMissingNodeScan {
    generation: u32,
    ledger_seq: u32,
    backed: bool,
    remaining: i32,
    missing_hashes: BTreeSet<SHAMapHash>,
    missing_nodes: Vec<(SHAMapNodeId, Uint256)>,
    stack: Vec<MissingNodeScanState>,
    pending_reads: Vec<PendingDeferredFetch>,
    deferred_resumes: BTreeMap<usize, DeferredResume>,
    stats: DeferredMissingNodeScanStats,
}

impl DeferredMissingNodeScan {
    pub fn new<R, C>(
        root: &SharedIntrusive<SHAMapTreeNode>,
        max: i32,
        backed: bool,
        ledger_seq: u32,
        full_below_cache: &C,
        next_first_child: &mut R,
    ) -> Self
    where
        R: FnMut() -> u8,
        C: FullBelowCache,
    {
        assert!(
            root.get_hash().is_non_zero(),
            "deferred missing-node scans require a non-zero root hash"
        );
        assert!(
            max > 0,
            "deferred missing-node scans require a positive max bound"
        );

        let generation = full_below_cache.generation();
        let mut stack = Vec::new();
        if root.is_inner() && !root.is_full_below(generation) {
            push_scan_state(
                &mut stack,
                root.clone(),
                SHAMapNodeId::default(),
                next_first_child,
            );
        }

        Self {
            generation,
            ledger_seq,
            backed,
            remaining: max,
            missing_hashes: BTreeSet::new(),
            missing_nodes: Vec::with_capacity(max as usize),
            stack,
            pending_reads: Vec::new(),
            deferred_resumes: BTreeMap::new(),
            stats: DeferredMissingNodeScanStats::default(),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.stack.is_empty() && self.pending_reads.is_empty() && self.deferred_resumes.is_empty()
    }

    pub fn remaining(&self) -> i32 {
        self.remaining
    }

    pub fn missing_nodes(&self) -> &[(SHAMapNodeId, Uint256)] {
        &self.missing_nodes
    }

    pub fn pending_requests(&self) -> Vec<DeferredFetchRequestInfo> {
        self.pending_reads
            .iter()
            .map(|pending| DeferredFetchRequestInfo {
                hash: pending.hash,
                ledger_seq: pending.ledger_seq,
            })
            .collect()
    }

    pub fn run_with_family<CLOCK, S, FB, F, MR, NS, R, REQ>(
        &mut self,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
        filter: &mut Option<&mut dyn SHAMapSyncFilter>,
        max_deferred: usize,
        next_first_child: &mut R,
        request_async_fetch: &mut REQ,
    ) where
        CLOCK: CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        R: FnMut() -> u8,
        REQ: FnMut(SHAMapHash, u32),
    {
        if self.stack.is_empty() && !self.deferred_resumes.is_empty() {
            activate_deferred_resumes(
                &mut self.stack,
                &mut self.deferred_resumes,
                self.generation,
                next_first_child,
            );
        }

        while let Some(state) = self.stack.last_mut() {
            // so a pass is allowed to post one more deferred read after reaching
            // the threshold. Keep the same boundary here.
            if self.remaining <= 0 || self.pending_reads.len() > max_deferred {
                break;
            }

            if state.current_child >= BRANCH_FACTOR {
                let state_full_below = state.full_below;
                let completed_hash =
                    state_full_below.then(|| *state.node().get_hash().as_uint256());
                if let Some(hash) = completed_hash {
                    state.node().set_full_below_gen(self.generation);
                    if self.backed {
                        family.with_full_below_cache(|full_below_cache| {
                            full_below_cache.insert(hash);
                        });
                    }
                    self.stats.full_below_marked += 1;
                }
                self.stack.pop();
                if !state_full_below && let Some(parent) = self.stack.last_mut() {
                    parent.full_below = false;
                }
                continue;
            }

            let branch = (state.first_child + state.current_child) % BRANCH_FACTOR;
            state.current_child += 1;
            if state.node().is_empty_branch(branch) {
                continue;
            }
            self.stats.branches_seen += 1;

            let child_hash = state.node().get_child_hash(branch);
            if self.missing_hashes.contains(&child_hash) {
                state.full_below = false;
                self.stats.duplicate_missing_hashes += 1;
                continue;
            }

            if self.backed
                && family.with_full_below_cache(|full_below_cache| {
                    full_below_cache.touch_if_exists(*child_hash.as_uint256())
                })
            {
                self.stats.full_below_hits += 1;
                continue;
            }

            match crate::fetch::descend_async_raw_nocopy(
                state.node(),
                branch,
                self.backed,
                self.ledger_seq,
                family,
                filter,
                request_async_fetch,
            ) {
                crate::fetch::AsyncDescendResultRaw::Ready(None) => {
                    state.full_below = false;
                    let exhausted = record_missing_child(
                        &mut self.missing_hashes,
                        &mut self.missing_nodes,
                        &mut self.remaining,
                        state.node_id,
                        branch,
                        child_hash,
                    );
                    self.stats.missing_recorded += 1;
                    if exhausted {
                        break;
                    }
                }
                crate::fetch::AsyncDescendResultRaw::Ready(Some(child_ptr)) => {
                    self.stats.loaded_or_cached_children += 1;
                    let child = unsafe { &*child_ptr };
                    if child.is_inner() {
                        self.stats.inner_children += 1;
                        if child.is_full_below(self.generation) {
                            self.stats.full_below_inner_children += 1;
                        } else {
                            let child_id = state
                                .node_id
                                .get_child_node_id(branch)
                                .expect("branch selection must stay within SHAMap depth bounds");
                            push_scan_state_raw(
                                &mut self.stack,
                                child_ptr,
                                child_id,
                                next_first_child,
                            );
                        }
                    } else {
                        self.stats.leaf_children += 1;
                    }
                }
                crate::fetch::AsyncDescendResultRaw::Pending(hash) => {
                    state.full_below = false;
                    self.stats.pending_reads += 1;
                    self.pending_reads.push(PendingDeferredFetch {
                        parent: state.node,
                        parent_id: state.node_id,
                        branch,
                        hash,
                        ledger_seq: self.ledger_seq,
                    });
                }
            }
        }
    }

    pub fn complete_pending_reads<I>(&mut self, completions: I)
    where
        I: IntoIterator<Item = Option<SharedIntrusive<SHAMapTreeNode>>>,
    {
        let completions: Vec<_> = completions.into_iter().collect();
        let pending_reads = std::mem::take(&mut self.pending_reads);
        assert_eq!(
            pending_reads.len(),
            completions.len(),
            "complete_pending_reads requires one completion per queued deferred fetch"
        );

        let deferred_reads = pending_reads
            .into_iter()
            .zip(completions)
            .map(|(pending, node)| DeferredSyncRead {
                parent: pending.parent,
                parent_id: pending.parent_id,
                branch: pending.branch,
                node,
            });

        for (parent_key, resume) in process_deferred_sync_reads(
            deferred_reads,
            &mut self.missing_hashes,
            &mut self.missing_nodes,
            &mut self.remaining,
            &mut self.stats,
        ) {
            self.deferred_resumes.insert(parent_key, resume);
        }
    }

    pub fn into_missing_nodes(self) -> Vec<(SHAMapNodeId, Uint256)> {
        self.missing_nodes
    }

    pub fn into_missing_nodes_and_stats(
        self,
    ) -> (Vec<(SHAMapNodeId, Uint256)>, DeferredMissingNodeScanStats) {
        (self.missing_nodes, self.stats)
    }
}

fn push_scan_state<R>(
    stack: &mut Vec<MissingNodeScanState>,
    node: SharedIntrusive<SHAMapTreeNode>,
    node_id: SHAMapNodeId,
    next_first_child: &mut R,
) where
    R: FnMut() -> u8,
{
    let ptr: *const SHAMapTreeNode = &*node;
    stack.push(MissingNodeScanState {
        node: ptr,
        node_id,
        first_child: next_first_child() as usize,
        current_child: 0,
        full_below: true,
    });
}

fn push_scan_state_raw<R>(
    stack: &mut Vec<MissingNodeScanState>,
    node: *const SHAMapTreeNode,
    node_id: SHAMapNodeId,
    next_first_child: &mut R,
) where
    R: FnMut() -> u8,
{
    stack.push(MissingNodeScanState {
        node,
        node_id,
        first_child: next_first_child() as usize,
        current_child: 0,
        full_below: true,
    });
}

fn record_missing_child(
    missing_hashes: &mut BTreeSet<SHAMapHash>,
    missing_nodes: &mut Vec<(SHAMapNodeId, Uint256)>,
    remaining: &mut i32,
    parent_id: SHAMapNodeId,
    branch: usize,
    child_hash: SHAMapHash,
) -> bool {
    if *remaining <= 0 || !missing_hashes.insert(child_hash) {
        return false;
    }

    let child_id = parent_id
        .get_child_node_id(branch)
        .expect("branch selection must stay within SHAMap depth bounds");
    missing_nodes.push((child_id, *child_hash.as_uint256()));
    *remaining -= 1;
    *remaining <= 0
}

#[cfg_attr(not(test), allow(dead_code))]
fn process_deferred_sync_reads(
    deferred_reads: impl IntoIterator<Item = DeferredSyncRead>,
    missing_hashes: &mut BTreeSet<SHAMapHash>,
    missing_nodes: &mut Vec<(SHAMapNodeId, Uint256)>,
    remaining: &mut i32,
    stats: &mut DeferredMissingNodeScanStats,
) -> BTreeMap<usize, DeferredResume> {
    let mut resumes = BTreeMap::<usize, DeferredResume>::new();

    for deferred in deferred_reads {
        let parent = unsafe { &*deferred.parent };
        assert!(
            parent.is_inner(),
            "process_deferred_sync_reads requires inner parent nodes"
        );

        let child_hash = parent.get_child_hash(deferred.branch);
        if let Some(node) = deferred.node {
            parent.canonicalize_child(deferred.branch, node);
            stats.completed_pending_reads += 1;
            let parent_key = deferred.parent as usize;
            resumes.insert(
                parent_key,
                DeferredResume {
                    node: deferred.parent,
                    node_id: deferred.parent_id,
                },
            );
        } else if *remaining > 0 {
            stats.completed_pending_misses += 1;
            record_missing_child(
                missing_hashes,
                missing_nodes,
                remaining,
                deferred.parent_id,
                deferred.branch,
                child_hash,
            );
            stats.missing_recorded += 1;
        }
    }

    stats.deferred_resumes += resumes.len() as u64;

    resumes
}

#[cfg_attr(not(test), allow(dead_code))]
fn enqueue_deferred_resumes<R>(
    stack: &mut Vec<MissingNodeScanState>,
    resumes: impl IntoIterator<Item = DeferredResume>,
    generation: u32,
    next_first_child: &mut R,
) where
    R: FnMut() -> u8,
{
    for resume in resumes {
        if !unsafe { &*resume.node }.is_full_below(generation) {
            push_scan_state_raw(stack, resume.node, resume.node_id, next_first_child);
        }
    }
}

fn activate_deferred_resumes<R>(
    stack: &mut Vec<MissingNodeScanState>,
    deferred_resumes: &mut BTreeMap<usize, DeferredResume>,
    generation: u32,
    next_first_child: &mut R,
) where
    R: FnMut() -> u8,
{
    if deferred_resumes.is_empty() {
        return;
    }

    enqueue_deferred_resumes(
        stack,
        std::mem::take(deferred_resumes).into_values(),
        generation,
        next_first_child,
    );
}

pub fn walk_map<F>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    map_type: SHAMapType,
    missing_nodes: &mut Vec<SHAMapMissingNode>,
    mut max_missing: i32,
    backed: bool,
    fetch: &mut F,
) where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    if !root.is_inner() {
        return;
    }

    let mut stack = vec![root.clone()];
    while let Some(node) = stack.pop() {
        for branch in 0..BRANCH_FACTOR {
            if node.is_empty_branch(branch) {
                continue;
            }

            let next = descend_no_store(&node, branch, backed, fetch);
            if let Some(next_node) = next {
                if next_node.is_inner() {
                    stack.push(next_node);
                }
            } else {
                missing_nodes.push(SHAMapMissingNode::from_hash(
                    map_type,
                    node.get_child_hash(branch),
                ));
                max_missing -= 1;
                if max_missing <= 0 {
                    return;
                }
            }
        }
    }
}

struct ParallelWalkState {
    missing_nodes: Vec<SHAMapMissingNode>,
    remaining: i32,
}

pub(crate) enum ParallelWalkResult {
    Completed,
    RootNotInner,
    WorkerPanics(Vec<String>),
}

impl ParallelWalkResult {
    pub(crate) fn completed(&self) -> bool {
        matches!(self, Self::Completed)
    }
}

fn panic_payload_to_string(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = panic.downcast_ref::<&'static str>() {
        return (*message).to_owned();
    }
    "unknown panic".to_owned()
}

pub(crate) fn walk_map_parallel_with_observer<F, O>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    map_type: SHAMapType,
    missing_nodes: &mut Vec<SHAMapMissingNode>,
    max_missing: i32,
    backed: bool,
    fetch: &F,
    on_worker_start: &mut O,
) -> ParallelWalkResult
where
    F: Fn(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>> + Sync,
    O: FnMut(usize),
{
    if !root.is_inner() {
        return ParallelWalkResult::RootNotInner;
    }

    let mut top_children: [Option<SharedIntrusive<SHAMapTreeNode>>; BRANCH_FACTOR] =
        std::array::from_fn(|_| None);
    for (branch, slot) in top_children.iter_mut().enumerate().take(BRANCH_FACTOR) {
        if root.is_empty_branch(branch) {
            continue;
        }

        *slot = descend_no_store(root, branch, backed, &mut |hash| fetch(hash));
    }

    let shared_state = Mutex::new(ParallelWalkState {
        missing_nodes: Vec::new(),
        remaining: max_missing,
    });
    let stop = AtomicBool::new(false);
    let mut panics = Vec::new();

    thread::scope(|scope| {
        let mut workers = Vec::new();
        for (root_child_index, child) in top_children.into_iter().enumerate() {
            if stop.load(Ordering::Acquire) {
                break;
            }
            let Some(child) = child else {
                continue;
            };
            if !child.is_inner() {
                continue;
            }

            on_worker_start(root_child_index);
            let shared_state = &shared_state;
            let stop = &stop;
            workers.push(scope.spawn(move || {
                catch_unwind(AssertUnwindSafe(|| {
                    let mut node_stack = vec![child];
                    while let Some(node) = node_stack.pop() {
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        for branch in 0..BRANCH_FACTOR {
                            if stop.load(Ordering::Acquire) {
                                return;
                            }
                            if node.is_empty_branch(branch) {
                                continue;
                            }

                            let next =
                                descend_no_store(&node, branch, backed, &mut |hash| fetch(hash));
                            if stop.load(Ordering::Acquire) {
                                return;
                            }
                            if let Some(next_node) = next {
                                if next_node.is_inner() {
                                    node_stack.push(next_node);
                                }
                            } else {
                                let mut state = shared_state.lock();
                                if stop.load(Ordering::Acquire) || state.remaining <= 0 {
                                    return;
                                }
                                state.missing_nodes.push(SHAMapMissingNode::from_hash(
                                    map_type,
                                    node.get_child_hash(branch),
                                ));
                                state.remaining -= 1;
                                if state.remaining <= 0 {
                                    stop.store(true, Ordering::Release);
                                    return;
                                }
                            }
                        }
                    }
                }))
                .err()
                .map(|panic| panic_payload_to_string(panic.as_ref()))
            }));
        }

        for worker in workers {
            match worker.join() {
                Ok(Some(panic)) => panics.push(panic),
                Ok(None) => {}
                Err(panic) => panics.push(panic_payload_to_string(panic.as_ref())),
            }
        }
    });

    let mut state = shared_state.lock();
    missing_nodes.append(&mut state.missing_nodes);
    if panics.is_empty() {
        ParallelWalkResult::Completed
    } else {
        ParallelWalkResult::WorkerPanics(panics)
    }
}

pub fn walk_map_parallel<F>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    map_type: SHAMapType,
    missing_nodes: &mut Vec<SHAMapMissingNode>,
    max_missing: i32,
    backed: bool,
    fetch: &F,
) -> bool
where
    F: Fn(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>> + Sync,
{
    walk_map_parallel_with_observer(
        root,
        map_type,
        missing_nodes,
        max_missing,
        backed,
        fetch,
        &mut |_| {},
    )
    .completed()
}

#[allow(clippy::too_many_arguments)]
pub fn get_missing_nodes<F, R, C>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    max: i32,
    ledger_seq: u32,
    backed: bool,
    fetch: &mut F,
    filter: &mut Option<&mut dyn SHAMapSyncFilter>,
    full_below_cache: &C,
    next_first_child: &mut R,
) -> Vec<(SHAMapNodeId, Uint256)>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
    R: FnMut() -> u8,
    C: FullBelowCache,
{
    assert!(
        root.get_hash().is_non_zero(),
        "get_missing_nodes requires a non-zero root hash"
    );
    assert!(max > 0, "get_missing_nodes requires a positive max bound");

    let generation = full_below_cache.generation();
    if !root.is_inner() || root.is_full_below(generation) {
        return Vec::new();
    }

    let mut missing_hashes = BTreeSet::new();
    let mut missing_nodes = Vec::with_capacity(max as usize);
    let mut remaining = max;
    let mut stack = Vec::new();
    push_scan_state(
        &mut stack,
        root.clone(),
        SHAMapNodeId::default(),
        next_first_child,
    );

    while let Some(state) = stack.last_mut() {
        if state.current_child >= BRANCH_FACTOR {
            let state_full_below = state.full_below;
            let completed_hash = state_full_below.then(|| *state.node().get_hash().as_uint256());
            if let Some(hash) = completed_hash {
                state.node().set_full_below_gen(generation);
                full_below_cache.insert(hash);
            }
            stack.pop();
            if !state_full_below && let Some(parent) = stack.last_mut() {
                parent.full_below = false;
            }
            continue;
        }

        let branch = (state.first_child + state.current_child) % BRANCH_FACTOR;
        state.current_child += 1;
        if state.node().is_empty_branch(branch) {
            continue;
        }

        let child_hash = state.node().get_child_hash(branch);
        if missing_hashes.contains(&child_hash) {
            state.full_below = false;
            continue;
        }

        if backed && full_below_cache.touch_if_exists(*child_hash.as_uint256()) {
            continue;
        }

        let Some(child) =
            resolve_sync_child(state.node(), branch, ledger_seq, backed, fetch, filter)
        else {
            state.full_below = false;
            if record_missing_child(
                &mut missing_hashes,
                &mut missing_nodes,
                &mut remaining,
                state.node_id,
                branch,
                child_hash,
            ) {
                return missing_nodes;
            }
            continue;
        };

        if child.is_inner() && !child.is_full_below(generation) {
            let child_id = state
                .node_id
                .get_child_node_id(branch)
                .expect("branch selection must stay within SHAMap depth bounds");
            push_scan_state(&mut stack, child, child_id, next_first_child);
        }
    }

    missing_nodes
}

#[allow(clippy::too_many_arguments)]
pub fn get_missing_nodes_with_family<CLOCK, S, C, F, R, MR, NS>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    max: i32,
    ledger_seq: u32,
    backed: bool,
    filter: &mut Option<&mut dyn SHAMapSyncFilter>,
    full_below_cache: &C,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
    next_first_child: &mut R,
) -> Vec<(SHAMapNodeId, Uint256)>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    C: FullBelowCache,
    F: SHAMapNodeFetcher,
    R: FnMut() -> u8,
{
    assert!(
        root.get_hash().is_non_zero(),
        "get_missing_nodes_with_family requires a non-zero root hash"
    );
    assert!(
        max > 0,
        "get_missing_nodes_with_family requires a positive max bound"
    );

    let generation = full_below_cache.generation();
    if !root.is_inner() || root.is_full_below(generation) {
        return Vec::new();
    }

    let mut missing_hashes = BTreeSet::new();
    let mut missing_nodes = Vec::with_capacity(max as usize);
    let mut remaining = max;
    let mut stack = Vec::new();
    push_scan_state(
        &mut stack,
        root.clone(),
        SHAMapNodeId::default(),
        next_first_child,
    );

    while let Some(state) = stack.last_mut() {
        if state.current_child >= BRANCH_FACTOR {
            let state_full_below = state.full_below;
            let completed_hash = state_full_below.then(|| *state.node().get_hash().as_uint256());
            if let Some(hash) = completed_hash {
                state.node().set_full_below_gen(generation);
                full_below_cache.insert(hash);
            }
            stack.pop();
            if !state_full_below && let Some(parent) = stack.last_mut() {
                parent.full_below = false;
            }
            continue;
        }

        let branch = (state.first_child + state.current_child) % BRANCH_FACTOR;
        state.current_child += 1;
        if state.node().is_empty_branch(branch) {
            continue;
        }

        let child_hash = state.node().get_child_hash(branch);
        if missing_hashes.contains(&child_hash) {
            state.full_below = false;
            continue;
        }

        if backed && full_below_cache.touch_if_exists(*child_hash.as_uint256()) {
            continue;
        }

        let Some(child) = resolve_sync_child_with_family(
            state.node(),
            branch,
            ledger_seq,
            backed,
            family,
            filter,
        ) else {
            state.full_below = false;
            if record_missing_child(
                &mut missing_hashes,
                &mut missing_nodes,
                &mut remaining,
                state.node_id,
                branch,
                child_hash,
            ) {
                return missing_nodes;
            }
            continue;
        };

        if child.is_inner() && !child.is_full_below(generation) {
            let child_id = state
                .node_id
                .get_child_node_id(branch)
                .expect("branch selection must stay within SHAMap depth bounds");
            push_scan_state(&mut stack, child, child_id, next_first_child);
        }
    }

    missing_nodes
}

pub fn get_node_fat<F>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    wanted: SHAMapNodeId,
    data: &mut Vec<(SHAMapNodeId, Blob)>,
    fat_leaves: bool,
    depth: u32,
    backed: bool,
    fetch: &mut F,
) -> Result<bool, TraversalError>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    let mut node = root.clone();
    let mut node_id = SHAMapNodeId::default();

    while node.is_inner() && node_id.get_depth() < wanted.get_depth() {
        let branch = crate::node_id::select_branch(node_id, wanted.get_node_id());
        if node.is_empty_branch(branch) {
            return Ok(false);
        }

        node = descend_throw(&node, branch, backed, fetch)?
            .expect("non-empty branches should resolve to a child or error");
        node_id = node_id
            .get_child_node_id(branch)
            .expect("branch selection must stay within SHAMap depth bounds");
    }

    if wanted != node_id {
        return Ok(false);
    }

    if node.is_inner() && node.is_empty() {
        return Ok(false);
    }

    let mut stack = vec![(node, node_id, depth)];
    while let Some((node, node_id, depth)) = stack.pop() {
        data.push((
            node_id,
            node.serialize_for_wire()
                .expect("sync replies should only serialize non-empty valid nodes"),
        ));

        if !node.is_inner() {
            continue;
        }

        let branch_count = node.branch_count() as u32;
        if depth == 0 && branch_count != 1 {
            continue;
        }

        for branch in 0..BRANCH_FACTOR {
            if node.is_empty_branch(branch) {
                continue;
            }

            let child = descend_throw(&node, branch, backed, fetch)?
                .expect("non-empty branches should resolve to a child or error");
            let child_id = node_id
                .get_child_node_id(branch)
                .expect("branch selection must stay within SHAMap depth bounds");

            if child.is_inner() && ((depth > 1) || (branch_count == 1)) {
                let next_depth = if branch_count > 1 { depth - 1 } else { depth };
                stack.push((child, child_id, next_depth));
            } else if child.is_inner() || fat_leaves {
                data.push((
                    child_id,
                    child
                        .serialize_for_wire()
                        .expect("sync replies should only serialize non-empty valid nodes"),
                ));
            }
        }
    }

    Ok(true)
}

pub fn get_node_fat_with_family<CLOCK, S, C, F, MR, NS>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    wanted: SHAMapNodeId,
    data: &mut Vec<(SHAMapNodeId, Blob)>,
    fat_leaves: bool,
    depth: u32,
    backed: bool,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
) -> Result<bool, TraversalError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    F: SHAMapNodeFetcher,
{
    let mut node = root.clone();
    let mut node_id = SHAMapNodeId::default();

    while node.is_inner() && node_id.get_depth() < wanted.get_depth() {
        let branch = crate::node_id::select_branch(node_id, wanted.get_node_id());
        if node.is_empty_branch(branch) {
            return Ok(false);
        }

        node = descend_throw(&node, branch, backed, &mut |hash| {
            family.fetch_cached_node(hash)
        })?
        .expect("non-empty branches should resolve to a child or error");
        node_id = node_id
            .get_child_node_id(branch)
            .expect("branch selection must stay within SHAMap depth bounds");
    }

    if wanted != node_id {
        family.log_info(&format!(
            "peer requested node that is not in the map: {wanted} but found {node_id}"
        ));
        return Ok(false);
    }

    if node.is_inner() && node.is_empty() {
        family.log_warn("peer requests empty node");
        return Ok(false);
    }

    let mut stack = vec![(node, node_id, depth)];
    while let Some((node, node_id, depth)) = stack.pop() {
        data.push((
            node_id,
            node.serialize_for_wire()
                .expect("sync replies should only serialize non-empty valid nodes"),
        ));

        if !node.is_inner() {
            continue;
        }

        let branch_count = node.branch_count() as u32;
        if depth == 0 && branch_count != 1 {
            continue;
        }

        for branch in 0..BRANCH_FACTOR {
            if node.is_empty_branch(branch) {
                continue;
            }

            let child = descend_throw(&node, branch, backed, &mut |hash| {
                family.fetch_cached_node(hash)
            })?
            .expect("non-empty branches should resolve to a child or error");
            let child_id = node_id
                .get_child_node_id(branch)
                .expect("branch selection must stay within SHAMap depth bounds");

            if child.is_inner() && ((depth > 1) || (branch_count == 1)) {
                let next_depth = if branch_count > 1 { depth - 1 } else { depth };
                stack.push((child, child_id, next_depth));
            } else if child.is_inner() || fat_leaves {
                data.push((
                    child_id,
                    child
                        .serialize_for_wire()
                        .expect("sync replies should only serialize non-empty valid nodes"),
                ));
            }
        }
    }

    Ok(true)
}

fn resolve_sync_child<F>(
    parent: &SHAMapTreeNode,
    branch: usize,
    ledger_seq: u32,
    backed: bool,
    fetch: &mut F,
    filter: &mut Option<&mut dyn SHAMapSyncFilter>,
) -> Option<SharedIntrusive<SHAMapTreeNode>>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    if let Some(loaded) = parent.get_child(branch) {
        return Some(loaded);
    }
    if parent.is_empty_branch(branch) {
        return None;
    }

    let child_hash = parent.get_child_hash(branch);
    if let Some(from_filter) = check_filter(child_hash, ledger_seq, filter) {
        return Some(parent.canonicalize_child(branch, from_filter));
    }

    if !backed {
        return None;
    }

    fetch(child_hash).map(|node| parent.canonicalize_child(branch, node))
}

fn resolve_sync_child_with_family<CLOCK, S, FB, F, MR, NS>(
    parent: &SHAMapTreeNode,
    branch: usize,
    ledger_seq: u32,
    backed: bool,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    filter: &mut Option<&mut dyn SHAMapSyncFilter>,
) -> Option<SharedIntrusive<SHAMapTreeNode>>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    FB: FullBelowCache,
    F: SHAMapNodeFetcher,
{
    if let Some(loaded) = parent.get_child(branch) {
        return Some(loaded);
    }
    if parent.is_empty_branch(branch) {
        return None;
    }

    let child_hash = parent.get_child_hash(branch);
    if let Some(node) = family.cache_lookup(child_hash) {
        return Some(parent.canonicalize_child(branch, node));
    }

    if let Some(from_filter) =
        check_filter_with_family(child_hash, backed, ledger_seq, family, filter)
    {
        return Some(parent.canonicalize_child(branch, from_filter));
    }

    if !backed {
        return None;
    }

    match family.fetch_cached_node_result_with_ledger_seq(child_hash, ledger_seq) {
        CachedFetchResult::Found(node) => Some(parent.canonicalize_child(branch, node)),
        CachedFetchResult::Missing | CachedFetchResult::InvalidBlob => None,
    }
}

fn resolve_sync_child_for_add_known_node<F>(
    parent: &SHAMapTreeNode,
    branch: usize,
    ledger_seq: u32,
    backed: bool,
    fetch: &mut F,
    filter: &mut Option<&mut dyn SHAMapSyncFilter>,
) -> Option<SharedIntrusive<SHAMapTreeNode>>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    if let Some(loaded) = parent.get_child(branch) {
        return Some(loaded);
    }
    if parent.is_empty_branch(branch) {
        return None;
    }

    let child_hash = parent.get_child_hash(branch);
    if backed && let Some(node) = fetch(child_hash) {
        return Some(parent.canonicalize_child(branch, node));
    }

    if let Some(from_filter) = check_filter(child_hash, ledger_seq, filter) {
        return Some(parent.canonicalize_child(branch, from_filter));
    }

    None
}

fn resolve_sync_child_for_add_known_node_with_family<CLOCK, S, FB, F, MR, NS>(
    parent: &SHAMapTreeNode,
    branch: usize,
    backed: bool,
    ledger_seq: u32,
    owner_fetch_state: &OwnerBackedFetchState,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    filter: &mut Option<&mut dyn SHAMapSyncFilter>,
) -> Option<SharedIntrusive<SHAMapTreeNode>>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    FB: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    if let Some(loaded) = parent.get_child(branch) {
        return Some(loaded);
    }
    if parent.is_empty_branch(branch) {
        return None;
    }

    let child_hash = parent.get_child_hash(branch);
    if let Some(node) = family.cache_lookup(child_hash) {
        return Some(parent.canonicalize_child(branch, node));
    }

    if backed {
        match family.fetch_cached_node_result_with_ledger_seq(child_hash, ledger_seq) {
            CachedFetchResult::Found(node) => {
                return Some(parent.canonicalize_child(branch, node));
            }
            CachedFetchResult::Missing => {
                owner_fetch_state.report_missing_node_once(ledger_seq, child_hash, family);
            }
            CachedFetchResult::InvalidBlob => {}
        }
    }

    if let Some(node) = check_filter_with_family(child_hash, backed, ledger_seq, family, filter) {
        return Some(parent.canonicalize_child(branch, node));
    }

    None
}

fn notify_filter_of_accepted_node(
    filter: &mut Option<&mut dyn SHAMapSyncFilter>,
    from_filter: bool,
    node_hash: SHAMapHash,
    ledger_seq: u32,
    node: &SharedIntrusive<SHAMapTreeNode>,
) {
    let Some(filter) = filter.as_deref_mut() else {
        return;
    };
    let Ok(node_data) = node.serialize_with_prefix() else {
        return;
    };
    filter.got_node(
        from_filter,
        node_hash,
        ledger_seq,
        node_data,
        node.get_type(),
    );
}

/// Recursively recompute hashes bottom-up for all dirty (hash=0) nodes.
/// This matches reference walkSubTree behavior: process children first, then parent.
/// Only visits loaded children — unloaded branches keep their stored hashes.
fn recompute_hashes_recursive(node: &SharedIntrusive<SHAMapTreeNode>) {
    if !node.is_inner() {
        // Leaf: compute hash if dirty
        if node.get_hash().is_zero() {
            node.update_hash();
        }
        return;
    }

    // Inner node: first recursively process all loaded children
    for branch in 0..16 {
        if node.is_empty_branch(branch) {
            continue;
        }
        if let Some(child) = node.get_child(branch)
            && child.get_hash().is_zero()
        {
            recompute_hashes_recursive(&child);
        }
    }

    // Now all loaded children have correct hashes — refresh and compute
    node.update_hash_deep();
}

fn next_snapshot_cowid(root: &SharedIntrusive<SHAMapTreeNode>) -> u32 {
    root.cowid().max(1) + 1
}

fn clone_sync_subtree_as_shareable(
    node: &SharedIntrusive<SHAMapTreeNode>,
    cowid: u32,
) -> SharedIntrusive<SHAMapTreeNode> {
    let cloned = node.clone_with_cowid(cowid);

    if cloned.is_inner() {
        for branch in 0..BRANCH_FACTOR {
            if cloned.is_empty_branch(branch) {
                continue;
            }

            let Some(child) = node.get_child(branch) else {
                continue;
            };
            let cloned_child = clone_sync_subtree_as_shareable(&child, cowid);
            cloned.set_child(branch, Some(cloned_child));
        }
        cloned.update_hash_deep();
    } else {
        cloned.update_hash();
    }

    cloned.unshare();
    cloned
}

#[cfg(test)]
mod tests {
    use super::{
        DeferredSyncRead, MissingNodeRef, SHAMapAddNode, SHAMapMissingNode, SHAMapSyncFilter,
        SHAMapType, SyncState, SyncTree, enqueue_deferred_resumes, get_missing_nodes, get_node_fat,
        process_deferred_sync_reads, walk_map, walk_map_parallel,
    };
    use crate::family::{
        FullBelowCache, MissingNodeReporter, NullFullBelowCache, NullMissingNodeReporter,
        NullNodeFetcher, SHAMapFamily,
    };
    use crate::item::SHAMapItem;
    use crate::node_id::SHAMapNodeId;
    use crate::tree_node::{SHAMapNodeType, SHAMapTreeNode};
    use crate::tree_node_cache::TreeNodeCache;
    use basics::base_uint::Uint256;
    use basics::blob::Blob;
    use basics::intrusive_pointer::make_shared_intrusive;
    use basics::sha_map_hash::SHAMapHash;
    use basics::tagged_cache::ManualClock;
    use parking_lot::Mutex;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;
    use time::Duration;

    fn sample_uint256(fill: u8) -> Uint256 {
        Uint256::from_array([fill; 32])
    }

    fn sample_uint256_with_prefix(first_byte: u8, fill: u8) -> Uint256 {
        let mut bytes = [fill; 32];
        bytes[0] = first_byte;
        Uint256::from_array(bytes)
    }

    fn sample_hash(fill: u8) -> SHAMapHash {
        SHAMapHash::new(sample_uint256(fill))
    }

    #[derive(Default)]
    struct RecordingFullBelowCache {
        generation: u32,
        known: parking_lot::Mutex<BTreeSet<Uint256>>,
        inserted: parking_lot::Mutex<Vec<Uint256>>,
    }

    impl RecordingFullBelowCache {
        fn new(generation: u32) -> Self {
            Self {
                generation,
                known: parking_lot::Mutex::new(BTreeSet::new()),
                inserted: parking_lot::Mutex::new(Vec::new()),
            }
        }
    }

    impl FullBelowCache for RecordingFullBelowCache {
        fn generation(&self) -> u32 {
            self.generation
        }

        fn touch_if_exists(&self, hash: Uint256) -> bool {
            self.known.lock().contains(&hash)
        }

        fn insert(&self, hash: Uint256) {
            self.known.lock().insert(hash);
            self.inserted.lock().push(hash);
        }
    }

    #[derive(Default)]
    struct RecordingFilter {
        next_node: Option<Blob>,
        got_nodes: Vec<(bool, SHAMapHash, u32, SHAMapNodeType)>,
    }

    impl SHAMapSyncFilter for RecordingFilter {
        fn got_node(
            &mut self,
            from_filter: bool,
            node_hash: SHAMapHash,
            ledger_seq: u32,
            _node_data: Blob,
            node_type: SHAMapNodeType,
        ) {
            self.got_nodes
                .push((from_filter, node_hash, ledger_seq, node_type));
        }

        fn get_node(&mut self, _node_hash: SHAMapHash) -> Option<Blob> {
            self.next_node.take()
        }
    }

    #[derive(Debug, Clone, Default)]
    struct RecordingMissingNodeReporter {
        by_seq: Arc<Mutex<Vec<(u32, Uint256)>>>,
    }

    impl RecordingMissingNodeReporter {
        fn recorded(&self) -> Vec<(u32, Uint256)> {
            self.by_seq.lock().clone()
        }
    }

    impl MissingNodeReporter for RecordingMissingNodeReporter {
        fn missing_node_acquire_by_seq(&self, ref_num: u32, node_hash: Uint256) {
            self.by_seq.lock().push((ref_num, node_hash));
        }

        fn missing_node_acquire_by_hash(&self, _ref_hash: Uint256, _ref_num: u32) {}
    }

    #[test]
    fn missing_node_formats_hashes_and_ids() {
        let hash_node = SHAMapMissingNode::from_hash(SHAMapType::State, sample_hash(0xAA));
        let id_node = SHAMapMissingNode::from_id(SHAMapType::Transaction, sample_uint256(0xBB));

        assert_eq!(hash_node.map_type(), SHAMapType::State);
        assert_eq!(hash_node.hash(), Some(sample_hash(0xAA)));
        assert_eq!(hash_node.id(), None);
        assert_eq!(
            hash_node.to_string(),
            format!("Missing Node: State Tree: hash {}", sample_hash(0xAA))
        );

        assert_eq!(id_node.locator(), MissingNodeRef::Id(sample_uint256(0xBB)));
        assert_eq!(id_node.hash(), None);
        assert_eq!(id_node.id(), Some(sample_uint256(0xBB)));
        assert_eq!(
            id_node.to_string(),
            format!(
                "Missing Node: Transaction Tree: id {}",
                sample_uint256(0xBB)
            )
        );
    }

    #[test]
    fn mutable_snapshot_resets_full_and_returns_modifying_tree() {
        let root = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(0x35), vec![0x41; 20]),
            7,
        ));
        let tree = SyncTree::from_root_with_type(
            root.clone(),
            SHAMapType::State,
            true,
            77,
            SyncState::Immutable,
        );
        tree.set_full();

        let snapshot = tree.mutable_snapshot();

        assert_eq!(snapshot.map_type(), SHAMapType::State);
        assert_eq!(snapshot.state(), SyncState::Modifying);
        assert!(!snapshot.is_full());
        assert_eq!(snapshot.root().get_hash(), root.get_hash());
        assert_eq!(snapshot.root().cowid(), 0);
    }

    #[test]
    fn new_synching_with_type_starts_in_synching_state() {
        let tree = SyncTree::new_synching_with_type(SHAMapType::Transaction, true, 88);

        assert_eq!(tree.map_type(), SHAMapType::Transaction);
        assert_eq!(tree.state(), SyncState::Synching);
        assert!(tree.root().is_inner());
        assert!(tree.root().is_empty());
    }

    #[test]
    fn walk_map_ignores_leaf_roots() {
        let leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(1), vec![1; 12]),
            0,
        ));
        let mut missing = Vec::new();

        walk_map(
            &leaf,
            SHAMapType::State,
            &mut missing,
            32,
            false,
            &mut |_| None,
        );

        assert!(missing.is_empty());
    }

    #[test]
    fn walk_map_reports_missing_branch_hashes_and_stops_at_max() {
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(1, sample_hash(0x11));
        root.set_child_hash(2, sample_hash(0x22));

        let mut missing = Vec::new();
        walk_map(&root, SHAMapType::State, &mut missing, 1, true, &mut |_| {
            None
        });

        assert_eq!(
            missing,
            vec![SHAMapMissingNode::from_hash(
                SHAMapType::State,
                sample_hash(0x11)
            )]
        );
    }

    #[test]
    fn walk_map_fetches_inner_children_without_attaching_them() {
        let nested_inner = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        nested_inner.set_child_hash(4, sample_hash(0x44));

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, sample_hash(0x33));

        let mut missing = Vec::new();
        let mut fetch_calls = Vec::new();
        walk_map(
            &root,
            SHAMapType::Transaction,
            &mut missing,
            32,
            true,
            &mut |hash| {
                fetch_calls.push(hash);
                if hash == sample_hash(0x33) {
                    Some(nested_inner.clone())
                } else {
                    None
                }
            },
        );

        assert_eq!(fetch_calls, vec![sample_hash(0x33), sample_hash(0x44)]);
        assert_eq!(
            missing,
            vec![SHAMapMissingNode::from_hash(
                SHAMapType::Transaction,
                sample_hash(0x44)
            )]
        );
        assert!(root.get_child(3).is_none());
    }

    #[test]
    fn walk_map_parallel_returns_false_for_leaf_roots() {
        let leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(2), vec![2; 12]),
            0,
        ));
        let mut missing = Vec::new();

        assert!(!walk_map_parallel(
            &leaf,
            SHAMapType::State,
            &mut missing,
            32,
            false,
            &|_| None,
        ));
        assert!(missing.is_empty());
    }

    #[test]
    fn walk_map_parallel_skips_missing_top_level_children_like_current_cpp_role() {
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(5, sample_hash(0x55));
        root.update_hash();

        let mut missing = Vec::new();
        assert!(walk_map_parallel(
            &root,
            SHAMapType::State,
            &mut missing,
            32,
            true,
            &|_| None,
        ));
        assert!(missing.is_empty());
    }

    #[test]
    fn walk_map_parallel_fetches_inner_children_without_attaching_them() {
        let nested_inner = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        nested_inner.set_child_hash(4, sample_hash(0x64));

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, sample_hash(0x63));

        let fetch_calls = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let fetch_calls_for_closure = fetch_calls.clone();
        let mut missing = Vec::new();

        assert!(walk_map_parallel(
            &root,
            SHAMapType::Transaction,
            &mut missing,
            32,
            true,
            &move |hash| {
                fetch_calls_for_closure.lock().push(hash);
                if hash == sample_hash(0x63) {
                    Some(nested_inner.clone())
                } else {
                    None
                }
            },
        ));

        assert_eq!(
            *fetch_calls.lock(),
            vec![sample_hash(0x63), sample_hash(0x64)]
        );
        assert_eq!(
            missing,
            vec![SHAMapMissingNode::from_hash(
                SHAMapType::Transaction,
                sample_hash(0x64)
            )]
        );
        assert!(root.get_child(3).is_none());
    }

    #[test]
    fn walk_map_parallel_respects_the_shared_missing_limit_across_workers() {
        let left_inner = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        left_inner.set_child_hash(1, sample_hash(0x71));
        left_inner.update_hash();

        let right_inner = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        right_inner.set_child_hash(2, sample_hash(0x72));
        right_inner.update_hash();

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, left_inner.get_hash());
        root.share_child(3, &left_inner);
        root.set_child_hash(4, right_inner.get_hash());
        root.share_child(4, &right_inner);

        let first_fetch_started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let first_fetch_started_for_closure = first_fetch_started.clone();
        let mut missing = Vec::new();

        assert!(walk_map_parallel(
            &root,
            SHAMapType::State,
            &mut missing,
            1,
            true,
            &move |hash| {
                if hash == sample_hash(0x71) {
                    first_fetch_started_for_closure
                        .store(true, std::sync::atomic::Ordering::Release);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    None
                } else if hash == sample_hash(0x72) {
                    while !first_fetch_started.load(std::sync::atomic::Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                    None
                } else {
                    None
                }
            },
        ));

        assert_eq!(missing.len(), 1);
    }

    #[test]
    fn get_missing_nodes_returns_node_ids_and_dedupes_hashes() {
        let shared_hash = sample_hash(0x55);
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(1, shared_hash);
        root.set_child_hash(2, shared_hash);
        root.update_hash();

        let mut full_below = NullFullBelowCache::new(7);
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        let missing = get_missing_nodes(
            &root,
            8,
            0,
            true,
            &mut |_| None,
            &mut no_filter,
            &mut full_below,
            &mut || 0,
        );

        assert_eq!(
            missing,
            vec![(
                SHAMapNodeId::default()
                    .get_child_node_id(1)
                    .expect("child id should exist"),
                *shared_hash.as_uint256(),
            )]
        );
    }

    #[test]
    fn get_missing_nodes_uses_filter_blobs_and_canonicalizes_children() {
        let child = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        child.set_child_hash(4, sample_hash(0x44));
        child.update_hash_deep();
        let child_blob = child
            .serialize_with_prefix()
            .expect("inner prefix serialization should succeed");

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, child.get_hash());
        root.update_hash_deep();

        let mut filter = RecordingFilter {
            next_node: Some(child_blob),
            got_nodes: Vec::new(),
        };
        let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
        let mut full_below = NullFullBelowCache::new(8);
        let missing = get_missing_nodes(
            &root,
            8,
            55,
            true,
            &mut |_| None,
            &mut filter_ref,
            &mut full_below,
            &mut || 0,
        );

        assert_eq!(
            missing,
            vec![(
                SHAMapNodeId::default()
                    .get_child_node_id(3)
                    .expect("child id should exist")
                    .get_child_node_id(4)
                    .expect("grandchild id should exist"),
                *sample_hash(0x44).as_uint256(),
            )]
        );
        assert!(root.get_child(3).is_some());
        assert_eq!(
            filter.got_nodes,
            vec![(true, child.get_hash(), 55, SHAMapNodeType::Inner)]
        );
    }

    #[test]
    fn get_missing_nodes_with_family_canonicalizes_filter_hits_into_shared_cache() {
        let child = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        child.set_child_hash(4, sample_hash(0x46));
        child.update_hash_deep();
        let child_blob = child
            .serialize_with_prefix()
            .expect("inner prefix serialization should succeed");

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, child.get_hash());
        root.update_hash();

        let cache = Arc::new(TreeNodeCache::new(
            "family-filter-scan",
            8,
            Duration::seconds(1),
            ManualClock::new(0),
        ));
        let family = SHAMapFamily::new(
            cache.clone(),
            NullFullBelowCache::new(17),
            NullNodeFetcher,
            NullMissingNodeReporter,
        );
        let mut tree = SyncTree::from_root(root.clone(), true, 55, SyncState::Synching);
        let mut filter = RecordingFilter {
            next_node: Some(child_blob),
            got_nodes: Vec::new(),
        };
        let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
        let missing = tree.get_missing_nodes_with_family(8, &mut filter_ref, &family, &mut || 0);

        assert_eq!(
            missing,
            vec![(
                SHAMapNodeId::default()
                    .get_child_node_id(3)
                    .expect("child id should exist")
                    .get_child_node_id(4)
                    .expect("grandchild id should exist"),
                *sample_hash(0x46).as_uint256(),
            )]
        );
        assert!(root.get_child(3).is_some());
        assert!(family.cache_lookup(child.get_hash()).is_some());
        assert_eq!(
            filter.got_nodes,
            vec![(true, child.get_hash(), 55, SHAMapNodeType::Inner)]
        );
    }

    #[test]
    fn get_missing_nodes_with_family_prefers_shared_cache_before_filter_descend_async() {
        let child = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(0x5A), vec![6; 12]),
            0,
        ));
        let child_prefix = child
            .serialize_with_prefix()
            .expect("leaf prefix serialization should succeed");

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, child.get_hash());
        root.update_hash();

        let cache = Arc::new(TreeNodeCache::new(
            "family-cache-before-filter",
            8,
            Duration::seconds(1),
            ManualClock::new(0),
        ));
        let family = SHAMapFamily::new(
            cache.clone(),
            NullFullBelowCache::new(19),
            NullNodeFetcher,
            NullMissingNodeReporter,
        );
        let mut cached = child.clone();
        assert!(!cache.canonicalize_replace_client(child.get_hash().as_uint256(), &mut cached));

        let mut tree = SyncTree::from_root(root.clone(), true, 55, SyncState::Synching);
        let mut filter = RecordingFilter {
            next_node: Some(child_prefix),
            got_nodes: Vec::new(),
        };
        let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
        let missing = tree.get_missing_nodes_with_family(8, &mut filter_ref, &family, &mut || 0);

        assert!(missing.is_empty());
        assert!(root.get_child(3).is_some());
        assert!(filter.next_node.is_some());
        assert!(filter.got_nodes.is_empty());
    }

    #[test]
    fn get_missing_nodes_with_family_does_not_report_owner_missing_acquire_on_deferred_db_miss() {
        let child_hash = sample_hash(0x6B);
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, child_hash);
        root.update_hash();

        let reporter = RecordingMissingNodeReporter::default();
        let family = SHAMapFamily::new(
            Arc::new(TreeNodeCache::new(
                "family-deferred-miss-no-owner-report",
                8,
                Duration::seconds(1),
                ManualClock::new(0),
            )),
            NullFullBelowCache::new(23),
            NullNodeFetcher,
            reporter.clone(),
        );
        let mut tree = SyncTree::from_root(root, true, 55, SyncState::Synching);
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;

        let missing = tree.get_missing_nodes_with_family(8, &mut no_filter, &family, &mut || 0);

        assert_eq!(
            missing,
            vec![(
                SHAMapNodeId::default()
                    .get_child_node_id(3)
                    .expect("child node id should exist"),
                *child_hash.as_uint256(),
            )]
        );
        assert!(reporter.recorded().is_empty());
    }

    #[test]
    fn add_known_node_prefers_backed_fetch_before_filter_descend() {
        let child = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(0x45), vec![3; 12]),
            0,
        ));
        let child_hash = child.get_hash();
        let raw_child = child
            .serialize_for_wire()
            .expect("leaf wire serialization should succeed");
        let child_prefix = child
            .serialize_with_prefix()
            .expect("leaf prefix serialization should succeed");

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, child_hash);
        root.update_hash();

        let mut tree = SyncTree::from_root(root, true, 55, SyncState::Synching);
        let mut fetch_calls = Vec::new();
        let mut filter = RecordingFilter {
            next_node: Some(child_prefix),
            got_nodes: Vec::new(),
        };
        let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
        let mut full_below = NullFullBelowCache::new(8);
        let node_id = SHAMapNodeId::default()
            .get_child_node_id(3)
            .expect("child node id should exist");

        let result = tree.add_known_node(
            node_id,
            &raw_child,
            &mut filter_ref,
            &mut full_below,
            &mut |hash| {
                fetch_calls.push(hash);
                if hash == child_hash {
                    Some(child.clone())
                } else {
                    None
                }
            },
        );

        assert_eq!(result, SHAMapAddNode::duplicate());
        assert_eq!(fetch_calls, vec![child_hash]);
        assert!(filter.next_node.is_some());
        assert!(filter.got_nodes.is_empty());
    }

    #[test]
    fn add_known_node_with_family_clears_full_on_backed_miss_before_filter_fallback() {
        let child = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(0x46), vec![4; 12]),
            0,
        ));
        let child_hash = child.get_hash();
        let raw_child = child
            .serialize_for_wire()
            .expect("leaf wire serialization should succeed");
        let child_prefix = child
            .serialize_with_prefix()
            .expect("leaf prefix serialization should succeed");

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, child_hash);
        root.update_hash();

        let cache = Arc::new(TreeNodeCache::new(
            "family-add-known-owner-fetch",
            8,
            Duration::seconds(1),
            ManualClock::new(0),
        ));
        let family = SHAMapFamily::new(
            cache,
            NullFullBelowCache::new(23),
            NullNodeFetcher,
            NullMissingNodeReporter,
        );
        let mut tree = SyncTree::from_root(root, true, 55, SyncState::Synching);
        tree.set_full();

        let mut filter = RecordingFilter {
            next_node: Some(child_prefix),
            got_nodes: Vec::new(),
        };
        let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
        let node_id = SHAMapNodeId::default()
            .get_child_node_id(3)
            .expect("child node id should exist");

        let result = tree.add_known_node_with_family(node_id, &raw_child, &mut filter_ref, &family);

        assert_eq!(result, SHAMapAddNode::duplicate());
        assert!(!tree.is_full());
        assert!(filter.next_node.is_none());
    }

    #[test]
    fn add_known_node_treats_full_below_hash_hits_as_duplicates() {
        let child = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(0x5B), vec![7; 12]),
            0,
        ));
        let child_hash = child.get_hash();
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, child_hash);
        root.update_hash();

        let mut tree = SyncTree::from_root(root, true, 55, SyncState::Synching);
        let mut full_below = RecordingFullBelowCache::new(20);
        full_below.known.lock().insert(*child_hash.as_uint256());
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        let node_id = SHAMapNodeId::default()
            .get_child_node_id(3)
            .expect("child node id should exist");

        let result =
            tree.add_known_node(node_id, &[], &mut no_filter, &mut full_below, &mut |_| {
                panic!("full-below hits should short-circuit before fetch")
            });

        assert_eq!(result, SHAMapAddNode::duplicate());
    }

    #[test]
    fn add_known_node_checks_full_below_before_loaded_child_descent() {
        let child = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        child.set_child_hash(4, sample_hash(0x44));
        child.update_hash();
        let child_hash = child.get_hash();
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, child_hash);
        root.canonicalize_child(3, child);
        root.update_hash();

        let mut tree = SyncTree::from_root(root, true, 55, SyncState::Synching);
        let mut full_below = RecordingFullBelowCache::new(20);
        full_below.known.lock().insert(*child_hash.as_uint256());
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        let node_id = SHAMapNodeId::default()
            .get_child_node_id(3)
            .and_then(|id| id.get_child_node_id(4))
            .expect("grandchild node id should exist");

        let result =
            tree.add_known_node(node_id, &[], &mut no_filter, &mut full_below, &mut |_| {
                panic!("full-below hits should short-circuit before loaded-child descent")
            });

        assert_eq!(result, SHAMapAddNode::duplicate());
    }

    #[test]
    fn add_known_node_with_family_canonicalizes_filter_path_hits_into_shared_cache() {
        let intermediate = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        let leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256_with_prefix(0x36, 0x47), vec![5; 12]),
            0,
        ));
        intermediate.set_child_hash(6, leaf.get_hash());
        intermediate.update_hash_deep();
        let intermediate_blob = intermediate
            .serialize_with_prefix()
            .expect("inner prefix serialization should succeed");
        let raw_leaf = leaf
            .serialize_for_wire()
            .expect("leaf wire serialization should succeed");

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, intermediate.get_hash());
        root.update_hash();

        let cache = Arc::new(TreeNodeCache::new(
            "family-filter-add-known",
            8,
            Duration::seconds(1),
            ManualClock::new(0),
        ));
        let family = SHAMapFamily::new(
            cache.clone(),
            NullFullBelowCache::new(18),
            NullNodeFetcher,
            NullMissingNodeReporter,
        );
        let mut tree = SyncTree::from_root(root.clone(), true, 60, SyncState::Synching);
        let mut filter = RecordingFilter {
            next_node: Some(intermediate_blob),
            got_nodes: Vec::new(),
        };
        let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
        let node_id = SHAMapNodeId::default()
            .get_child_node_id(3)
            .expect("child node id should exist")
            .get_child_node_id(6)
            .expect("grandchild node id should exist");

        let result = tree.add_known_node_with_family(node_id, &raw_leaf, &mut filter_ref, &family);

        assert_eq!(result, SHAMapAddNode::useful());
        assert!(family.cache_lookup(intermediate.get_hash()).is_some());
        let attached = root
            .get_child(3)
            .expect("filter path child should be attached to the root");
        assert!(attached.get_child(6).is_some());
        assert_eq!(
            filter.got_nodes,
            vec![
                (true, intermediate.get_hash(), 60, SHAMapNodeType::Inner),
                (false, leaf.get_hash(), 60, SHAMapNodeType::AccountState),
            ]
        );
    }

    #[test]
    fn add_known_node_with_family_persists_accepted_node_even_when_should_store_is_false() {
        struct SkippingFilter {
            got_nodes: Vec<(bool, SHAMapHash, u32, SHAMapNodeType)>,
        }

        impl SHAMapSyncFilter for SkippingFilter {
            fn got_node(
                &mut self,
                from_filter: bool,
                node_hash: SHAMapHash,
                ledger_seq: u32,
                _node_data: Blob,
                node_type: SHAMapNodeType,
            ) {
                self.got_nodes
                    .push((from_filter, node_hash, ledger_seq, node_type));
            }

            fn get_node(&mut self, _node_hash: SHAMapHash) -> Option<Blob> {
                None
            }

            fn should_store(&mut self, _node_hash: SHAMapHash) -> bool {
                false
            }
        }

        let leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256_with_prefix(0x30, 0x72), vec![8; 12]),
            0,
        ));
        let raw_leaf = leaf
            .serialize_for_wire()
            .expect("leaf wire serialization should succeed");

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, leaf.get_hash());
        root.update_hash();

        let family = SHAMapFamily::new(
            Arc::new(TreeNodeCache::new(
                "family-add-known-persist-even-skip",
                8,
                Duration::seconds(1),
                ManualClock::new(0),
            )),
            NullFullBelowCache::new(19),
            NullNodeFetcher,
            NullMissingNodeReporter,
        );
        let mut tree = SyncTree::from_root(root, true, 61, SyncState::Synching);
        let mut filter = SkippingFilter {
            got_nodes: Vec::new(),
        };
        let mut filter_ref: Option<&mut dyn SHAMapSyncFilter> = Some(&mut filter);
        let node_id = SHAMapNodeId::default()
            .get_child_node_id(3)
            .expect("child node id should exist");

        let result = tree.add_known_node_with_family(node_id, &raw_leaf, &mut filter_ref, &family);

        assert_eq!(result, SHAMapAddNode::useful());
        assert_eq!(
            filter.got_nodes,
            vec![(false, leaf.get_hash(), 61, SHAMapNodeType::AccountState)]
        );
    }

    #[test]
    fn get_missing_nodes_marks_full_subtrees_for_the_generation() {
        let leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(0x11), vec![1; 12]),
            0,
        ));
        let inner = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        inner.set_child_hash(1, leaf.get_hash());
        inner.share_child(1, &leaf);
        inner.update_hash_deep();

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(2, inner.get_hash());
        root.share_child(2, &inner);
        root.update_hash_deep();

        let mut full_below = RecordingFullBelowCache::new(9);
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        let first = get_missing_nodes(
            &root,
            8,
            0,
            false,
            &mut |_| None,
            &mut no_filter,
            &mut full_below,
            &mut || 0,
        );
        assert!(first.is_empty());
        assert!(root.is_full_below(9));
        assert!(inner.is_full_below(9));
        assert_eq!(
            full_below.inserted.lock().clone(),
            vec![
                *inner.get_hash().as_uint256(),
                *root.get_hash().as_uint256()
            ]
        );

        let second = get_missing_nodes(
            &root,
            8,
            0,
            false,
            &mut |_| None,
            &mut no_filter,
            &mut full_below,
            &mut || 0,
        );
        assert!(second.is_empty());
    }

    #[test]
    fn get_missing_nodes_respects_full_below_hash_cache_without_loading_children() {
        let child_hash = sample_hash(0x66);
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(6, child_hash);
        root.update_hash();

        let mut full_below = RecordingFullBelowCache::new(10);
        full_below.known.lock().insert(*child_hash.as_uint256());
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        let missing = get_missing_nodes(
            &root,
            8,
            0,
            true,
            &mut |_| panic!("full-below hash should skip child resolution"),
            &mut no_filter,
            &mut full_below,
            &mut || 0,
        );

        assert!(missing.is_empty());
        assert!(root.is_full_below(10));
    }

    #[test]
    fn get_missing_nodes_keeps_parent_not_full_below_when_descendant_is_missing() {
        let missing_hash = sample_hash(0x77);
        let inner = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        inner.set_child_hash(7, missing_hash);
        inner.update_hash();

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, inner.get_hash());
        root.share_child(3, &inner);
        root.update_hash_deep();

        let mut full_below = RecordingFullBelowCache::new(11);
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        let missing = get_missing_nodes(
            &root,
            8,
            0,
            true,
            &mut |_| None,
            &mut no_filter,
            &mut full_below,
            &mut || 0,
        );

        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].1, *missing_hash.as_uint256());
        assert!(!inner.is_full_below(11));
        assert!(!root.is_full_below(11));
    }

    #[test]
    fn process_deferred_sync_reads_canonicalizes_children_and_dedupes_resume_parents() {
        let left_child = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(0x71), vec![1; 12]),
            0,
        ));
        let right_child = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(0x72), vec![2; 12]),
            0,
        ));
        let parent = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        parent.set_child_hash(1, left_child.get_hash());
        parent.set_child_hash(2, right_child.get_hash());

        let mut missing_hashes = BTreeSet::new();
        let mut missing_nodes = Vec::new();
        let mut remaining = 8;
        let mut stats = super::DeferredMissingNodeScanStats::default();
        let resumes = process_deferred_sync_reads(
            [
                DeferredSyncRead {
                    parent: &*parent as *const SHAMapTreeNode,
                    parent_id: SHAMapNodeId::default(),
                    branch: 1,
                    node: Some(left_child.clone()),
                },
                DeferredSyncRead {
                    parent: &*parent as *const SHAMapTreeNode,
                    parent_id: SHAMapNodeId::default(),
                    branch: 2,
                    node: Some(right_child.clone()),
                },
            ],
            &mut missing_hashes,
            &mut missing_nodes,
            &mut remaining,
            &mut stats,
        );

        assert!(parent.get_child(1).is_some());
        assert!(parent.get_child(2).is_some());
        assert!(missing_nodes.is_empty());
        assert!(missing_hashes.is_empty());
        assert_eq!(remaining, 8);
        assert_eq!(resumes.len(), 1);
        assert_eq!(stats.completed_pending_reads, 2);
        assert_eq!(stats.deferred_resumes, 1);
        let resume = resumes
            .values()
            .next()
            .expect("one deferred resume should be recorded");
        assert!(std::ptr::eq(unsafe { &*resume.node }, &*parent));
        assert_eq!(resume.node_id, SHAMapNodeId::default());
    }

    #[test]
    fn process_deferred_sync_reads_records_missing_children_once() {
        let shared_hash = sample_hash(0x73);
        let parent = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        parent.set_child_hash(3, shared_hash);
        parent.set_child_hash(4, shared_hash);

        let mut missing_hashes = BTreeSet::new();
        let mut missing_nodes = Vec::new();
        let mut remaining = 8;
        let mut stats = super::DeferredMissingNodeScanStats::default();
        let resumes = process_deferred_sync_reads(
            [
                DeferredSyncRead {
                    parent: &*parent as *const SHAMapTreeNode,
                    parent_id: SHAMapNodeId::default(),
                    branch: 3,
                    node: None,
                },
                DeferredSyncRead {
                    parent: &*parent as *const SHAMapTreeNode,
                    parent_id: SHAMapNodeId::default(),
                    branch: 4,
                    node: None,
                },
            ],
            &mut missing_hashes,
            &mut missing_nodes,
            &mut remaining,
            &mut stats,
        );

        assert!(resumes.is_empty());
        assert_eq!(remaining, 7);
        assert_eq!(stats.completed_pending_misses, 2);
        assert_eq!(stats.missing_recorded, 2);
        assert_eq!(
            missing_nodes,
            vec![(
                SHAMapNodeId::default()
                    .get_child_node_id(3)
                    .expect("child id should exist"),
                *shared_hash.as_uint256(),
            )]
        );
    }

    #[test]
    fn enqueue_deferred_resumes_skips_nodes_already_marked_full_below() {
        let queued_parent = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        let skipped_parent = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        skipped_parent.set_full_below_gen(12);

        let mut stack = Vec::new();
        let mut first_children = [5_u8, 9_u8].into_iter();
        enqueue_deferred_resumes(
            &mut stack,
            vec![
                super::DeferredResume {
                    node: &*queued_parent as *const SHAMapTreeNode,
                    node_id: SHAMapNodeId::default(),
                },
                super::DeferredResume {
                    node: &*skipped_parent as *const SHAMapTreeNode,
                    node_id: SHAMapNodeId::default()
                        .get_child_node_id(1)
                        .expect("child id should exist"),
                },
            ],
            12,
            &mut || {
                first_children
                    .next()
                    .expect("test should provide enough branch seeds")
            },
        );

        assert_eq!(stack.len(), 1);
        assert!(std::ptr::eq(unsafe { &*stack[0].node }, &*queued_parent));
        assert_eq!(stack[0].node_id, SHAMapNodeId::default());
        assert_eq!(stack[0].first_child, 5);
        assert_eq!(stack[0].current_child, 0);
        assert!(stack[0].full_below);
    }

    #[test]
    fn deferred_missing_node_scan_resumes_after_completed_reads() {
        let missing_leaf_hash = sample_hash(0x74);
        let fetched_inner = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        fetched_inner.set_child_hash(7, missing_leaf_hash);
        fetched_inner.update_hash_deep();

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, fetched_inner.get_hash());
        root.update_hash();

        let tree = SyncTree::from_root(root, true, 55, SyncState::Synching);
        let full_below = NullFullBelowCache::new(13);
        let family = SHAMapFamily::new(
            Arc::new(TreeNodeCache::new(
                "deferred-scan",
                8,
                Duration::seconds(1),
                ManualClock::new(0),
            )),
            NullFullBelowCache::new(13),
            NullNodeFetcher,
            NullMissingNodeReporter,
        );
        let mut scan = tree.start_deferred_missing_node_scan(8, &full_below, &mut || 0);
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        let mut requests = Vec::new();

        scan.run_with_family(
            &family,
            &mut no_filter,
            8,
            &mut || 0,
            &mut |hash, ledger_seq| requests.push((hash, ledger_seq)),
        );

        assert_eq!(requests, vec![(fetched_inner.get_hash(), 55)]);
        assert_eq!(
            scan.pending_requests()
                .iter()
                .map(|request| (request.hash(), request.ledger_seq()))
                .collect::<Vec<_>>(),
            vec![(fetched_inner.get_hash(), 55)]
        );

        scan.complete_pending_reads(vec![Some(fetched_inner)]);
        requests.clear();

        scan.run_with_family(
            &family,
            &mut no_filter,
            8,
            &mut || 0,
            &mut |hash, ledger_seq| requests.push((hash, ledger_seq)),
        );

        assert_eq!(requests, vec![(missing_leaf_hash, 55)]);
        scan.complete_pending_reads(vec![None]);

        assert!(scan.is_complete());
        assert_eq!(
            scan.missing_nodes(),
            &[(
                SHAMapNodeId::default()
                    .get_child_node_id(3)
                    .expect("child id should exist")
                    .get_child_node_id(7)
                    .expect("grandchild id should exist"),
                *missing_leaf_hash.as_uint256(),
            )]
        );
        assert_eq!(scan.remaining(), 7);
    }

    #[test]
    fn deferred_missing_node_scan_defers_resume_activation_until_stack_drains() {
        let active_hash = sample_hash(0x77);
        let resumed_hash = sample_hash(0x78);
        let active_parent = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        active_parent.set_child_hash(0, active_hash);
        active_parent.update_hash();

        let resumed_parent = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        resumed_parent.set_child_hash(1, resumed_hash);
        resumed_parent.update_hash();

        let family = SHAMapFamily::new(
            Arc::new(TreeNodeCache::new(
                "deferred-resume-order",
                8,
                Duration::seconds(1),
                ManualClock::new(0),
            )),
            NullFullBelowCache::new(16),
            NullNodeFetcher,
            NullMissingNodeReporter,
        );
        let mut scan = super::DeferredMissingNodeScan {
            generation: 16,
            ledger_seq: 91,
            backed: true,
            remaining: 8,
            missing_hashes: BTreeSet::new(),
            missing_nodes: Vec::new(),
            stack: vec![super::MissingNodeScanState {
                node: &*active_parent as *const SHAMapTreeNode,
                node_id: SHAMapNodeId::default(),
                first_child: 0,
                current_child: 0,
                full_below: true,
            }],
            pending_reads: Vec::new(),
            deferred_resumes: BTreeMap::from([(
                (&*resumed_parent as *const SHAMapTreeNode) as usize,
                super::DeferredResume {
                    node: &*resumed_parent as *const SHAMapTreeNode,
                    node_id: SHAMapNodeId::default()
                        .get_child_node_id(2)
                        .expect("child id should exist"),
                },
            )]),
            stats: super::DeferredMissingNodeScanStats::default(),
        };
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        let mut requests = Vec::new();

        scan.run_with_family(
            &family,
            &mut no_filter,
            8,
            &mut || 9,
            &mut |hash, ledger_seq| requests.push((hash, ledger_seq)),
        );

        assert_eq!(requests, vec![(active_hash, 91)]);
        assert!(scan.stack.is_empty());
        assert_eq!(scan.deferred_resumes.len(), 1);

        scan.complete_pending_reads(vec![None]);
        requests.clear();

        scan.run_with_family(
            &family,
            &mut no_filter,
            8,
            &mut || 11,
            &mut |hash, ledger_seq| requests.push((hash, ledger_seq)),
        );

        assert_eq!(requests, vec![(resumed_hash, 91)]);
    }

    #[test]
    fn deferred_missing_node_scan_deferred_threshold_boundary() {
        let child_hash = sample_hash(0x7d);
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(1, child_hash);
        root.update_hash();

        let tree = SyncTree::from_root(root, true, 90, SyncState::Synching);
        let family = SHAMapFamily::new(
            Arc::new(TreeNodeCache::new(
                "deferred-threshold",
                8,
                Duration::seconds(1),
                ManualClock::new(0),
            )),
            NullFullBelowCache::new(16),
            NullNodeFetcher,
            NullMissingNodeReporter,
        );
        let mut scan = tree.start_deferred_missing_node_scan_with_family(8, &family, &mut || 0);
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        let mut requests = Vec::new();

        scan.run_with_family(
            &family,
            &mut no_filter,
            0,
            &mut || 0,
            &mut |hash, ledger_seq| requests.push((hash, ledger_seq)),
        );

        assert_eq!(requests, vec![(child_hash, 90)]);
        assert_eq!(
            scan.pending_requests()
                .iter()
                .map(|request| (request.hash(), request.ledger_seq()))
                .collect::<Vec<_>>(),
            vec![(child_hash, 90)]
        );
    }

    #[test]
    fn deferred_missing_node_scan_with_family_skips_known_full_below_hashes() {
        let child_hash = sample_hash(0x75);
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(4, child_hash);
        root.update_hash();

        let cache = Arc::new(TreeNodeCache::new(
            "deferred-skip-full-below",
            8,
            Duration::seconds(1),
            ManualClock::new(0),
        ));
        let family = SHAMapFamily::new(
            cache,
            RecordingFullBelowCache {
                generation: 14,
                known: parking_lot::Mutex::new(BTreeSet::from([*child_hash.as_uint256()])),
                inserted: parking_lot::Mutex::new(Vec::new()),
            },
            NullNodeFetcher,
            NullMissingNodeReporter,
        );
        let tree = SyncTree::from_root(root.clone(), true, 66, SyncState::Synching);
        let mut scan = tree.start_deferred_missing_node_scan_with_family(8, &family, &mut || 0);
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        let mut requests = Vec::new();

        scan.run_with_family(
            &family,
            &mut no_filter,
            8,
            &mut || 0,
            &mut |hash, ledger_seq| requests.push((hash, ledger_seq)),
        );

        assert!(scan.is_complete());
        assert!(scan.missing_nodes().is_empty());
        assert!(requests.is_empty());
        assert!(root.is_full_below(14));
    }

    #[test]
    fn deferred_missing_node_scan_with_family_inserts_completed_full_below_nodes() {
        let leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(0x76), vec![9; 12]),
            0,
        ));
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(2, leaf.get_hash());
        root.share_child(2, &leaf);
        root.update_hash_deep();

        let cache = Arc::new(TreeNodeCache::new(
            "deferred-insert-full-below",
            8,
            Duration::seconds(1),
            ManualClock::new(0),
        ));
        let family = SHAMapFamily::new(
            cache,
            RecordingFullBelowCache::new(15),
            NullNodeFetcher,
            NullMissingNodeReporter,
        );
        let tree = SyncTree::from_root(root.clone(), true, 67, SyncState::Synching);
        let mut scan = tree.start_deferred_missing_node_scan_with_family(8, &family, &mut || 0);
        let mut no_filter: Option<&mut dyn SHAMapSyncFilter> = None;
        let mut requests = Vec::new();

        scan.run_with_family(
            &family,
            &mut no_filter,
            8,
            &mut || 0,
            &mut |hash, ledger_seq| requests.push((hash, ledger_seq)),
        );

        assert!(scan.is_complete());
        assert!(scan.missing_nodes().is_empty());
        assert!(requests.is_empty());
        assert!(root.is_full_below(15));
        family.with_full_below_cache(|full_below_cache| {
            assert_eq!(
                full_below_cache.inserted.lock().clone(),
                vec![*root.get_hash().as_uint256()]
            );
        });
    }

    #[test]
    fn get_node_fat_follows_single_child_inner_chains_without_spending_depth() {
        let leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(0xAA), vec![1; 12]),
            0,
        ));
        let child_inner = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        child_inner.set_child_hash(2, leaf.get_hash());
        child_inner.share_child(2, &leaf);
        child_inner.update_hash_deep();

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(3, child_inner.get_hash());
        root.share_child(3, &child_inner);
        root.update_hash_deep();

        let mut data = Vec::new();
        let found = get_node_fat(
            &root,
            SHAMapNodeId::default(),
            &mut data,
            true,
            0,
            false,
            &mut |_| None,
        )
        .expect("loaded get_node_fat should succeed");

        assert!(found);
        assert_eq!(data.len(), 3);
        assert_eq!(data[0].0, SHAMapNodeId::default());
        assert_eq!(
            data[1].0,
            SHAMapNodeId::default()
                .get_child_node_id(3)
                .expect("child id should exist")
        );
        assert_eq!(
            data[2].0,
            SHAMapNodeId::default()
                .get_child_node_id(3)
                .expect("child id should exist")
                .get_child_node_id(2)
                .expect("grandchild id should exist")
        );
    }

    #[test]
    fn get_node_fat_can_include_immediate_inner_children_without_fat_leaves() {
        let left_leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(0x10), vec![1; 12]),
            0,
        ));
        let deep_leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(sample_uint256(0x40), vec![2; 12]),
            0,
        ));
        let inner_child = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        inner_child.set_child_hash(4, deep_leaf.get_hash());
        inner_child.share_child(4, &deep_leaf);
        inner_child.update_hash_deep();

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(1, left_leaf.get_hash());
        root.share_child(1, &left_leaf);
        root.set_child_hash(4, inner_child.get_hash());
        root.share_child(4, &inner_child);
        root.update_hash_deep();

        let mut data = Vec::new();
        let found = get_node_fat(
            &root,
            SHAMapNodeId::default(),
            &mut data,
            false,
            1,
            false,
            &mut |_| None,
        )
        .expect("loaded get_node_fat should succeed");

        assert!(found);
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].0, SHAMapNodeId::default());
        assert_eq!(
            data[1].0,
            SHAMapNodeId::default()
                .get_child_node_id(4)
                .expect("child id should exist")
        );
    }

    #[test]
    fn get_node_fat_returns_false_for_missing_requested_branches() {
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(1, sample_hash(0x11));
        root.update_hash();

        let mut data = Vec::new();
        let found = get_node_fat(
            &root,
            SHAMapNodeId::default()
                .get_child_node_id(2)
                .expect("child id should exist"),
            &mut data,
            true,
            1,
            true,
            &mut |_| None,
        )
        .expect("missing branch checks should not error");

        assert!(!found);
        assert!(data.is_empty());
    }
}
