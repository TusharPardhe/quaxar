use crate::fetch_pack::{FetchPackContainer, LedgerSyncFilterStore};
use basics::blob::Blob;
use basics::sha_map_hash::SHAMapHash;
use shamap::fetch::SHAMapSyncFilter;
use shamap::storage::NodeObjectType;
use shamap::tree_node::SHAMapNodeType;

#[derive(Debug)]
pub struct AccountStateSF<DB, FP> {
    db: DB,
    fp: FP,
}

impl<DB, FP> AccountStateSF<DB, FP> {
    pub fn new(db: DB, fp: FP) -> Self {
        Self { db, fp }
    }
}

impl<DB, FP> SHAMapSyncFilter for AccountStateSF<DB, FP>
where
    DB: LedgerSyncFilterStore,
    FP: FetchPackContainer,
{
    fn got_node(
        &mut self,
        _from_filter: bool,
        node_hash: SHAMapHash,
        ledger_seq: u32,
        node_data: Blob,
        _node_type: SHAMapNodeType,
    ) {
        self.db.store_shamap_node(
            NodeObjectType::AccountNode,
            node_data,
            *node_hash.as_uint256(),
            ledger_seq,
        );
    }

    fn get_node(&mut self, node_hash: SHAMapHash) -> Option<Blob> {
        // NodeStore reads are handled by SHAMap::descendAsync/fetchNodeNT.
        self.fp.get_fetch_pack(*node_hash.as_uint256())
    }

    fn should_store(&mut self, node_hash: SHAMapHash) -> bool {
        self.db.should_store_hash(*node_hash.as_uint256())
    }
}
