//! ConsensusTransSetSF — SHAMapSyncFilter for tx-set acquisition.
//!
//! Matches rippled's `ConsensusTransSetSF`: when downloading a peer's tx-set
//! via SHAMap sync, this filter checks if we already have each transaction
//! locally (in TransactionMaster's cache). If yes, it returns the serialized
//! node data directly — avoiding a network round-trip for that node.
//!
//! This is the KEY optimization for fast dispute resolution: when two tx-sets
//! differ by only 3 transactions out of 75, the filter supplies 72 of them
//! locally, reducing the download from 10+ round-trips to 1-2.

use std::sync::Arc;

use basics::sha_map_hash::SHAMapHash;
use shamap::tree_node::SHAMapNodeType;
use shamap::fetch::SHAMapSyncFilter;

use crate::tx_queue::transaction_master::TransactionMaster;
use ledger::transaction_acquire::TransactionAcquireFilterFactory;

/// The hash prefix for TransactionNm leaves: 'T','X','N',0 = 0x54584E00
const HASH_PREFIX_TRANSACTION_ID: u32 = 0x54584E00;

/// Blob is Vec<u8> in the shamap crate.
type Blob = Vec<u8>;

/// Factory that creates ConsensusTransSetSF instances.
/// Stored in InboundTransactions and cloned for each new acquisition.
pub struct ConsensusTransSetSFFactory {
    transaction_master: Arc<TransactionMaster>,
}

impl ConsensusTransSetSFFactory {
    pub fn new(transaction_master: Arc<TransactionMaster>) -> Self {
        Self { transaction_master }
    }
}

impl TransactionAcquireFilterFactory for ConsensusTransSetSFFactory {
    fn build_filter(&self) -> Box<dyn SHAMapSyncFilter> {
        Box::new(ConsensusTransSetSF {
            transaction_master: Arc::clone(&self.transaction_master),
        })
    }
}

/// The actual filter used during SHAMap sync for tx-set acquisition.
struct ConsensusTransSetSF {
    transaction_master: Arc<TransactionMaster>,
}

impl SHAMapSyncFilter for ConsensusTransSetSF {
    fn got_node(
        &mut self,
        _from_filter: bool,
        _node_hash: SHAMapHash,
        _ledger_seq: u32,
        _node_data: Blob,
        _node_type: SHAMapNodeType,
    ) {
        // In rippled, this caches the node in TempNodeCache and submits
        // unknown transactions to the local queue. For now we just accept
        // the data — the SHAMap sync engine handles storage internally.
    }

    fn get_node(&mut self, node_hash: SHAMapHash) -> Option<Blob> {
        // The node_hash for TransactionNm leaves equals the transaction ID.
        // Check if we already have this transaction in our local cache.
        let tx_id = node_hash.as_uint256();
        let tx = self.transaction_master.fetch_from_cache(tx_id)?;
        let tx_guard = tx.lock().expect("transaction lock");
        let st_tx = tx_guard.get_s_transaction();

        // Serialize in "prefix format" matching SHAMap's serialize_with_prefix
        // for TransactionNm leaves: HASH_PREFIX_TRANSACTION_ID || serialized_stx
        let serialized = protocol::serialize_blob(st_tx.as_ref());
        let mut bytes = Vec::with_capacity(4 + serialized.len());
        bytes.extend_from_slice(&HASH_PREFIX_TRANSACTION_ID.to_be_bytes());
        bytes.extend_from_slice(&serialized);
        Some(bytes)
    }
}
