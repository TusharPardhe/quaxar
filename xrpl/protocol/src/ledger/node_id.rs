//! `NodeID` helpers ported from `xrpl/protocol/PublicKey.*`.

use basics::base_uint::BaseUInt;

use crate::{PublicKey, ripesha};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeIdTag;

pub type NodeId = BaseUInt<20, NodeIdTag>;
pub type NodeID = NodeId;

pub fn calc_node_id(public_key: &PublicKey) -> NodeId {
    let digest = ripesha(public_key.as_bytes());
    NodeId::from_slice(&digest).expect("ripemd160 width should match NodeId")
}

#[cfg(test)]
mod tests {
    use super::{NodeId, calc_node_id};
    use crate::PublicKey;

    #[test]
    fn calc_node_id_vector() {
        let public_key = PublicKey::from_bytes([
            0x03, 0xD4, 0x9C, 0x56, 0xE1, 0xB1, 0x85, 0xF1, 0xBE, 0x89, 0x9A, 0xE6, 0x6A, 0x02,
            0xEF, 0xC1, 0x7F, 0x78, 0xEA, 0x6F, 0xC5, 0x3A, 0xF8, 0x5E, 0x0F, 0xE5, 0x4C, 0x6E,
            0x8B, 0x7F, 0x8C, 0x71, 0xA8,
        ]);

        assert_eq!(
            calc_node_id(&public_key),
            NodeId::from_hex("7E59C17D50F5959C7B158FEC95C8F815BF653DC8")
                .expect("hex node id should parse")
        );
    }
}
