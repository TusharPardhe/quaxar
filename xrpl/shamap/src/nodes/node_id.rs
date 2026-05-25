//! `xrpl/shamap/SHAMapNodeID.h` compatibility surface.

use basics::base_uint::Uint256;
use std::fmt;

pub const SHAMAP_BRANCH_FACTOR: usize = 16;
pub const SHAMAP_LEAF_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SHAMapNodeIdError {
    InvalidDepth,
    HashDoesNotMatchDepth,
    InvalidBranch,
    LeafHasNoChildren,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SHAMapNodeId {
    id: Uint256,
    depth: u8,
}

impl SHAMapNodeId {
    pub fn new(depth: usize, hash: Uint256) -> Result<Self, SHAMapNodeIdError> {
        if depth > SHAMAP_LEAF_DEPTH {
            return Err(SHAMapNodeIdError::InvalidDepth);
        }

        if hash != (hash & depth_mask(depth)) {
            return Err(SHAMapNodeIdError::HashDoesNotMatchDepth);
        }

        Ok(Self {
            id: hash,
            depth: depth as u8,
        })
    }

    pub fn is_root(&self) -> bool {
        self.depth == 0
    }

    pub fn get_depth(&self) -> usize {
        self.depth as usize
    }

    pub fn get_node_id(&self) -> Uint256 {
        self.id
    }

    pub fn get_raw_string(&self) -> Vec<u8> {
        let mut result = self.id.data().to_vec();
        result.push(self.depth);
        result
    }

    pub fn get_child_node_id(&self, branch: usize) -> Result<Self, SHAMapNodeIdError> {
        if branch >= SHAMAP_BRANCH_FACTOR {
            return Err(SHAMapNodeIdError::InvalidBranch);
        }
        if self.get_depth() >= SHAMAP_LEAF_DEPTH {
            return Err(SHAMapNodeIdError::LeafHasNoChildren);
        }
        if self.id != (self.id & depth_mask(self.get_depth())) {
            return Err(SHAMapNodeIdError::HashDoesNotMatchDepth);
        }

        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(self.id.data());
        let depth = self.get_depth();
        let byte_index = depth / 2;
        if depth & 1 == 1 {
            bytes[byte_index] |= branch as u8;
        } else {
            bytes[byte_index] |= (branch as u8) << 4;
        }

        SHAMapNodeId::new(depth + 1, Uint256::from_array(bytes))
    }

    pub fn create_id(depth: usize, key: Uint256) -> Result<Self, SHAMapNodeIdError> {
        if depth > SHAMAP_LEAF_DEPTH {
            return Err(SHAMapNodeIdError::InvalidDepth);
        }

        SHAMapNodeId::new(depth, key & depth_mask(depth))
    }
}

impl fmt::Display for SHAMapNodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_root() {
            formatter.write_str("NodeID(root)")
        } else {
            write!(
                formatter,
                "NodeID({},{})",
                self.get_depth(),
                self.get_node_id()
            )
        }
    }
}

pub fn deserialize_shamap_node_id(data: &[u8]) -> Option<SHAMapNodeId> {
    if data.len() != 33 {
        return None;
    }

    let depth = data[32] as usize;
    if depth > SHAMAP_LEAF_DEPTH {
        return None;
    }

    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&data[..32]);
    SHAMapNodeId::new(depth, Uint256::from_array(bytes)).ok()
}

pub fn select_branch(id: SHAMapNodeId, hash: Uint256) -> usize {
    let depth = id.get_depth();
    let mut branch = hash.data()[depth / 2] as usize;
    if depth & 1 == 1 {
        branch &= 0x0F;
    } else {
        branch >>= 4;
    }
    branch
}

fn depth_mask(depth: usize) -> Uint256 {
    let mut bytes = [0u8; 32];
    for nibble in 0..depth {
        let byte_index = nibble / 2;
        if nibble & 1 == 0 {
            bytes[byte_index] |= 0xF0;
        } else {
            bytes[byte_index] |= 0x0F;
        }
    }
    Uint256::from_array(bytes)
}

#[cfg(test)]
mod tests {
    use super::{
        SHAMAP_LEAF_DEPTH, SHAMapNodeId, SHAMapNodeIdError, depth_mask, deserialize_shamap_node_id,
        select_branch,
    };
    use basics::base_uint::Uint256;

    fn sample(fill: u8) -> Uint256 {
        Uint256::from_array([fill; 32])
    }

    #[test]
    fn create_id_masks_keys_to_requested_depth() {
        let key = sample(0xAB);
        let root = SHAMapNodeId::create_id(0, key).expect("root id should be valid");
        assert!(root.is_root());
        assert_eq!(root.get_node_id(), Uint256::zero());

        let depth_three = SHAMapNodeId::create_id(3, key).expect("masked id should be valid");
        assert_eq!(depth_three.get_node_id(), key & depth_mask(3));
        assert_eq!(
            depth_three.to_string(),
            format!("NodeID(3,{})", key & depth_mask(3))
        );
    }

    #[test]
    fn child_ids_and_branch_selection_match_cpp_roles() {
        let key =
            Uint256::from_hex("1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF")
                .expect("hex should parse");
        let root = SHAMapNodeId::default();
        let child = root.get_child_node_id(1).expect("child id should exist");
        assert_eq!(child.get_depth(), 1);
        assert_eq!(select_branch(root, key), 0x1);
        assert_eq!(select_branch(child, key), 0x2);
    }

    #[test]
    fn serialization_and_deserialization_match_cpp_roles() {
        let id = SHAMapNodeId::create_id(5, sample(0xBC)).expect("id should be valid");
        let raw = id.get_raw_string();
        let parsed = deserialize_shamap_node_id(&raw).expect("serialized form should parse");
        assert_eq!(parsed, id);
    }

    #[test]
    fn invalid_depths_and_leaf_children_are_rejected() {
        assert!(matches!(
            SHAMapNodeId::new(SHAMAP_LEAF_DEPTH + 1, Uint256::zero()),
            Err(SHAMapNodeIdError::InvalidDepth)
        ));

        let leaf = SHAMapNodeId::create_id(SHAMAP_LEAF_DEPTH, Uint256::zero())
            .expect("leaf id should be valid");
        assert!(matches!(
            leaf.get_child_node_id(0),
            Err(SHAMapNodeIdError::LeafHasNoChildren)
        ));
    }
}
