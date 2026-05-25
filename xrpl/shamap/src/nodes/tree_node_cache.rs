//! Rust migration boundary for `xrpl/shamap/TreeNodeCache.h`.

use crate::tree_node::SHAMapTreeNode;
use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::intrusive_pointer::{SharedIntrusive, SharedWeakUnion};
use basics::tagged_cache::{MonotonicClock, TaggedCache};

pub type TreeNodeCache<C = MonotonicClock, S = HardenedHashBuilder> = TaggedCache<
    Uint256,
    SHAMapTreeNode,
    C,
    S,
    SharedWeakUnion<SHAMapTreeNode>,
    SharedIntrusive<SHAMapTreeNode>,
>;
