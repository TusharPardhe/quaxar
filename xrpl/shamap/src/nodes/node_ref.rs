//! Child ownership for SHAMap tree nodes.

use super::tree_node::SHAMapTreeNode;
use basics::intrusive_pointer::SharedIntrusive;

/// A loaded SHAMap child node owned either by an intrusive pointer or a tree
/// arena. `Arena` pointers are valid while the owning `SyncTree` retains its
/// `Arc<TreeNodeArena>`.
#[derive(Debug)]
pub enum NodeRef {
    Arena(*const SHAMapTreeNode),
    Shared(SharedIntrusive<SHAMapTreeNode>),
}

impl NodeRef {
    #[inline]
    pub fn as_ptr(&self) -> *const SHAMapTreeNode {
        match self {
            Self::Arena(node) => *node,
            Self::Shared(node) => &**node as *const SHAMapTreeNode,
        }
    }

    /// Return a shared owner for compatibility and copy-on-write paths.
    ///
    /// Arena nodes cannot be adopted into an intrusive owner because their
    /// allocation belongs to the arena. Instead, create an independently owned
    /// node clone. Raw-pointer traversal should use [`Self::as_ptr`] and avoid
    /// this conversion.
    pub fn into_shared(self) -> SharedIntrusive<SHAMapTreeNode> {
        match self {
            Self::Arena(node) => {
                let node = unsafe {
                    node.as_ref()
                        .expect("arena-backed SHAMap node pointers must be non-null")
                };
                node.clone_with_cowid(node.cowid())
            }
            Self::Shared(node) => node,
        }
    }

    /// Obtain a shared child for legacy callers. This returns an intrusive
    /// clone for shared storage and an owned clone for arena storage.
    #[inline]
    pub fn get_child(&self) -> SharedIntrusive<SHAMapTreeNode> {
        self.clone().into_shared()
    }
}

impl Clone for NodeRef {
    fn clone(&self) -> Self {
        match self {
            Self::Arena(node) => Self::Arena(*node),
            Self::Shared(node) => Self::Shared(node.clone()),
        }
    }
}

// Arena pointers are valid while their SyncTree retains Arc<TreeNodeArena>.
// SharedIntrusive already supplies thread-safe ownership for its variant.
unsafe impl Send for NodeRef {}
unsafe impl Sync for NodeRef {}

#[cfg(test)]
mod tests {
    use super::NodeRef;
    use crate::arena::TreeNodeArena;
    use crate::item::SHAMapItem;
    use crate::tree_node::{SHAMapNodeType, SHAMapTreeNode};
    use basics::base_uint::Uint256;
    use basics::intrusive_pointer::make_shared_intrusive;
    use std::sync::Arc;

    #[test]
    fn arena_children_are_visible_to_raw_and_legacy_accessors() {
        let arena = Arc::new(TreeNodeArena::new(9));
        let arena_child = arena.alloc(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(Uint256::from_array([0xA5; 32]), vec![0x5A; 12]),
            1,
        ));
        let parent = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        parent.set_child_ref(3, Some(NodeRef::Arena(arena_child)));

        assert!(
            matches!(parent.get_child_ref(3), Some(NodeRef::Arena(ptr)) if *ptr == arena_child)
        );
        assert_eq!(unsafe { parent.get_child_ptr(3) }, Some(arena_child));

        let child = parent
            .get_child(3)
            .expect("arena-backed children must be visible through get_child");
        assert_eq!(child.get_hash(), unsafe { (&*arena_child).get_hash() });
        assert!(!std::ptr::eq(&*child, unsafe { &*arena_child }));
    }
}
