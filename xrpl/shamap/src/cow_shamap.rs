//! Copy-on-write persistent SHAMap foundation.
//!
//! `CowSHAMap` wraps an existing SHAMap root via `Arc` and only clones
//! nodes along the write path, enabling O(1) forking for speculative
//! ledger application.

use std::sync::Arc;

use basics::base_uint::Uint256;
use crate::item::SHAMapItem;

/// A single node in the CoW tree. Inner nodes hold children behind Arc;
/// leaf nodes hold the item data.
#[derive(Debug, Clone)]
pub enum CowNode {
    Inner {
        children: [Option<Arc<CowNode>>; 16],
    },
    Leaf {
        key: Uint256,
        item: SHAMapItem,
    },
}

impl CowNode {
    fn empty_inner() -> Self {
        CowNode::Inner {
            children: Default::default(),
        }
    }
}

/// A copy-on-write SHAMap that shares structure via `Arc` and clones
/// only the path nodes that are modified.
#[derive(Debug, Clone)]
pub struct CowSHAMap {
    root: Arc<CowNode>,
}

impl CowSHAMap {
    /// Create a new empty CowSHAMap.
    pub fn new() -> Self {
        Self {
            root: Arc::new(CowNode::empty_inner()),
        }
    }

    /// O(1) fork via Arc::clone of the root. Both copies share all
    /// unmodified structure.
    pub fn fork(&self) -> Self {
        Self {
            root: Arc::clone(&self.root),
        }
    }

    /// Insert or update a leaf. Clones inner nodes along the path from
    /// root to the target branch (copy-on-write).
    pub fn insert(&mut self, key: Uint256, item: SHAMapItem) {
        let new_root = Self::insert_recursive(&self.root, &key, item, 0);
        self.root = Arc::new(new_root);
    }

    fn insert_recursive(node: &Arc<CowNode>, key: &Uint256, item: SHAMapItem, depth: usize) -> CowNode {
        match node.as_ref() {
            CowNode::Inner { children } => {
                let branch = (key.data()[depth / 2] >> (if depth % 2 == 0 { 4 } else { 0 })) & 0x0F;
                let idx = branch as usize;
                let mut new_children = children.clone();
                let child = &new_children[idx];
                let new_child = match child {
                    Some(existing) => Self::insert_recursive(existing, key, item, depth + 1),
                    None => CowNode::Leaf { key: *key, item },
                };
                new_children[idx] = Some(Arc::new(new_child));
                CowNode::Inner { children: new_children }
            }
            CowNode::Leaf { key: existing_key, item: existing_item } => {
                if existing_key == key {
                    // Replace existing leaf.
                    CowNode::Leaf { key: *key, item }
                } else {
                    // Split: create inner node and re-insert both leaves.
                    let eb = (existing_key.data()[depth / 2] >> (if depth % 2 == 0 { 4 } else { 0 })) & 0x0F;
                    let nb = (key.data()[depth / 2] >> (if depth % 2 == 0 { 4 } else { 0 })) & 0x0F;
                    if eb == nb {
                        // Same branch at this depth — recurse deeper.
                        let sub = Self::insert_recursive(
                            &Arc::new(CowNode::Leaf { key: *existing_key, item: existing_item.clone() }),
                            key, item, depth + 1,
                        );
                        let mut children: [Option<Arc<CowNode>>; 16] = Default::default();
                        children[eb as usize] = Some(Arc::new(sub));
                        CowNode::Inner { children }
                    } else {
                        let mut children: [Option<Arc<CowNode>>; 16] = Default::default();
                        children[eb as usize] = Some(Arc::new(CowNode::Leaf { key: *existing_key, item: existing_item.clone() }));
                        children[nb as usize] = Some(Arc::new(CowNode::Leaf { key: *key, item }));
                        CowNode::Inner { children }
                    }
                }
            }
        }
    }
}

impl Default for CowSHAMap {
    fn default() -> Self {
        Self::new()
    }
}
