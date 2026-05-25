//! Rust port of `xrpld/app/ledger/TransactionStateSF.*`.

use crate::fetch_pack::{FetchPackContainer, LedgerSyncFilterStore};
use basics::blob::Blob;
use basics::sha_map_hash::SHAMapHash;
use shamap::fetch::SHAMapSyncFilter;
use shamap::storage::NodeObjectType;
use shamap::tree_node::SHAMapNodeType;

#[derive(Debug)]
pub struct TransactionStateSF<DB, FP> {
    db: DB,
    fp: FP,
}

impl<DB, FP> TransactionStateSF<DB, FP> {
    pub fn new(db: DB, fp: FP) -> Self {
        Self { db, fp }
    }
}

impl<DB, FP> SHAMapSyncFilter for TransactionStateSF<DB, FP>
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
        node_type: SHAMapNodeType,
    ) {
        assert_ne!(
            node_type,
            SHAMapNodeType::TransactionNm,
            "ledger::TransactionStateSF::got_node requires a valid transaction sync node type"
        );
        self.db.store_shamap_node(
            NodeObjectType::TransactionNode,
            node_data,
            *node_hash.as_uint256(),
            ledger_seq,
        );
    }

    fn get_node(&mut self, node_hash: SHAMapHash) -> Option<Blob> {
        // NodeStore reads are handled by SHAMap::descendAsync/fetchNodeNT.
        self.fp.get_fetch_pack(*node_hash.as_uint256())
    }
}
