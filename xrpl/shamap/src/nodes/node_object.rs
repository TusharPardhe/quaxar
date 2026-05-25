//! `xrpl/nodestore/NodeObject.h` compatibility surface.
//!
//! This keeps the reference shape intentionally small:
//! - object type,
//! - object hash,
//! - raw payload bytes.
//!
//! Like the the reference implementation `NodeObject`, this is just a carrier.

use crate::storage::NodeObjectType;
use basics::base_uint::Uint256;
use basics::blob::Blob;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeObject {
    object_type: NodeObjectType,
    hash: Uint256,
    data: Blob,
}

impl NodeObject {
    pub fn new(object_type: NodeObjectType, data: Blob, hash: Uint256) -> Self {
        Self {
            object_type,
            hash,
            data,
        }
    }

    pub fn object_type(&self) -> NodeObjectType {
        self.object_type
    }

    pub fn hash(&self) -> &Uint256 {
        &self.hash
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}
