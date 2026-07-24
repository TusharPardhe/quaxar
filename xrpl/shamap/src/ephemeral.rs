use crate::item::SHAMapItem;
use crate::nodes::tree_node::SHAMapNodeType;
use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use std::collections::BTreeMap;

/// Phase 4: The Ephemeral Working Set.
/// In the Flat State Architecture, we do not maintain a persistent COW tree in RAM.
/// During a ledger tick, all inserts, updates, and deletes are collected here.
/// At ledger close, we compute the state root hash by bubbling these changes up
/// alongside sibling hashes fetched directly from the Trie DB, and then we instantly
/// drop this structure, keeping RAM perfectly flat.
#[derive(Debug, Clone)]
pub enum EphemeralOp {
    Insert(SHAMapItem, SHAMapHash),
    Update(SHAMapItem, SHAMapHash),
    Delete,
}

#[derive(Debug, Clone)]
pub struct EphemeralWorkingSet {
    pub modifications: BTreeMap<Uint256, EphemeralOp>,
    node_type: SHAMapNodeType,
}

impl Default for EphemeralWorkingSet {
    fn default() -> Self {
        Self::new(SHAMapNodeType::AccountState)
    }
}

impl EphemeralWorkingSet {
    pub fn new(node_type: SHAMapNodeType) -> Self {
        Self {
            modifications: BTreeMap::new(),
            node_type,
        }
    }

    pub fn insert(&mut self, key: Uint256, item: SHAMapItem) {
        let leaf =
            crate::nodes::tree_node::SHAMapTreeNode::new_leaf(self.node_type, item.clone(), 0);
        self.modifications
            .insert(key, EphemeralOp::Insert(item, leaf.get_hash()));
    }

    pub fn update(&mut self, key: Uint256, item: SHAMapItem) {
        let leaf =
            crate::nodes::tree_node::SHAMapTreeNode::new_leaf(self.node_type, item.clone(), 0);
        let hash = leaf.get_hash();
        if let Some(existing) = self.modifications.get(&key) {
            match existing {
                EphemeralOp::Insert(_, _) => {
                    self.modifications
                        .insert(key, EphemeralOp::Insert(item, hash));
                    return;
                }
                _ => {}
            }
        }
        self.modifications
            .insert(key, EphemeralOp::Update(item, hash));
    }

    pub fn remove(&mut self, key: Uint256) {
        self.modifications.insert(key, EphemeralOp::Delete);
    }

    pub fn get(&self, key: &Uint256) -> Option<&EphemeralOp> {
        self.modifications.get(key)
    }
}
