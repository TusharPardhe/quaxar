//! Arena allocation for short-lived SHAMap tree nodes.
//!
//! `TreeNodeArena` owns the backing storage for arena-backed [`SHAMapTreeNode`]
//! values.  Nodes contain heap-owned members, so the arena records every
//! allocation and explicitly runs its destructor before releasing the bump
//! allocator's raw memory.

use crate::tree_node::SHAMapTreeNode;
use basics::intrusive_pointer::IntrusiveObject;
use bumpalo::Bump;
use parking_lot::Mutex;
use std::fmt;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

/// Thread-safe backing arena for SHAMap nodes.
///
/// All allocation and allocation tracking is protected by mutexes. Arena node
/// pointers remain valid until this arena is dropped; callers must retain an
/// `Arc<TreeNodeArena>` for every tree that contains [`crate::node_ref::NodeRef::Arena`]
/// children.
pub struct TreeNodeArena {
    bump: Mutex<Bump>,
    allocations: Mutex<Vec<*mut SHAMapTreeNode>>,
    generation: u64,
    allocated_count: AtomicU64,
}

impl fmt::Debug for TreeNodeArena {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeNodeArena")
            .field("generation", &self.generation)
            .field("allocated_count", &self.allocated_count())
            .finish_non_exhaustive()
    }
}

impl TreeNodeArena {
    pub fn new(generation: u64) -> Self {
        Self {
            bump: Mutex::new(Bump::new()),
            allocations: Mutex::new(Vec::new()),
            generation,
            allocated_count: AtomicU64::new(0),
        }
    }

    #[inline]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[inline]
    pub fn allocated_count(&self) -> u64 {
        self.allocated_count.load(Ordering::Acquire)
    }

    /// Allocate a node whose storage remains valid for the arena lifetime.
    ///
    /// The returned pointer is not an intrusive owner. It must only be used
    /// while an `Arc<TreeNodeArena>` retaining this arena is alive.
    pub fn alloc(&self, node: SHAMapTreeNode) -> *const SHAMapTreeNode {
        let ptr = {
            let bump = self.bump.lock();
            bump.alloc(node) as *mut SHAMapTreeNode
        };
        self.allocations.lock().push(ptr);
        self.allocated_count.fetch_add(1, Ordering::Release);
        ptr.cast_const()
    }

    /// Clone a fetched node's data directly into arena-owned storage.
    ///
    /// This deliberately clones the node value rather than retaining its
    /// `SharedIntrusive` wrapper, so the attached child is owned by this arena.
    pub fn alloc_clone(&self, node: &SHAMapTreeNode) -> *const SHAMapTreeNode {
        self.alloc(node.clone_value_with_cowid(node.cowid()))
    }
}

impl Drop for TreeNodeArena {
    fn drop(&mut self) {
        // `allocations` contains pointers into `bump`. Explicitly destroy every
        // node first so fields such as `Box<InnerNodeArrays>` release their own
        // resources before `Bump` frees the raw arena storage.
        for &node in self.allocations.get_mut().iter() {
            // Arena nodes are never adopted as intrusive owners. Their initial
            // construction strong count must be released before the ordinary
            // value destructor validates the terminal refcount state.
            let action = unsafe { (&*node).intrusive_ref_counts().release_strong_ref() };
            debug_assert!(matches!(
                action,
                basics::intrusive_ref_counts::ReleaseStrongRefAction::Destroy
            ));
            unsafe { ptr::drop_in_place(node) };
        }
    }
}

// `Bump` itself is not Sync. Both it and the raw-pointer allocation registry
// are accessed only through their mutexes. The raw pointers are valid until
// this arena is dropped, which SyncTree enforces by retaining Arc<TreeNodeArena>.
unsafe impl Send for TreeNodeArena {}
unsafe impl Sync for TreeNodeArena {}

#[cfg(test)]
mod tests {
    use super::TreeNodeArena;
    use crate::tree_node::SHAMapTreeNode;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn tracks_allocations_and_is_send_sync() {
        assert_send_sync::<TreeNodeArena>();

        let arena = TreeNodeArena::new(42);
        let node = arena.alloc(SHAMapTreeNode::new_inner(1));

        assert!(!node.is_null());
        assert_eq!(arena.generation(), 42);
        assert_eq!(arena.allocated_count(), 1);
    }
}
