//! Overlay replay-delta callback implementation.
//!
//! This is the explicit bridge for the parity-flow edge
//! `peer replay response -> LedgerReplayer::got_replay_delta`.

use std::{
    collections::{BTreeMap, btree_map::Entry},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use basics::{base_uint::Uint256, sha_map_hash::SHAMapHash};
use ledger::LedgerHeader;
use overlay::TmReplayDeltaResponse;
use protocol::{STTx, SerialIter, TxMeta, calculate_ledger_hash, deserialize_ledger_header};
use shamap::{item::SHAMapItem, mutation::MutableTree, tree_node::SHAMapNodeType};

use super::ApplicationRoot;

/// Decode and verify `TMReplayDeltaResponse` exactly before it can reach the
/// replay owner. This is the Rust equivalent of
/// `LedgerReplayMsgHandler.cpp::processReplayDeltaResponse` (lines 214-277):
/// validate the header hash, decode TransactionMd payloads, preserve each
/// transaction metadata index, and prove the transaction SHAMap root.
fn decode_replay_delta_response(
    response: &TmReplayDeltaResponse,
) -> Result<(LedgerHeader, BTreeMap<u32, Arc<STTx>>), &'static str> {
    if response.error.is_some() {
        return Err("peer returned replay delta error");
    }
    let reply_hash = Uint256::from_slice(&response.ledger_hash).ok_or("invalid replay hash")?;
    let header_bytes = response
        .ledger_header
        .as_deref()
        .ok_or("missing replay ledger header")?;
    let mut info = deserialize_ledger_header(header_bytes, false)
        .map_err(|_| "invalid replay ledger header")?;
    if calculate_ledger_hash(&info) != SHAMapHash::new(reply_hash) {
        return Err("replay ledger header hash mismatch");
    }
    info.hash = SHAMapHash::new(reply_hash);

    let mut ordered_txs = BTreeMap::new();
    let mut tx_map = MutableTree::new(info.seq.max(1));
    for payload in &response.transaction {
        let (tx, index) = catch_unwind(AssertUnwindSafe(|| {
            let mut serial = SerialIter::new(payload);
            let tx_bytes = serial.get_vl();
            let meta_bytes = serial.get_vl();
            let mut tx_serial = SerialIter::new(&tx_bytes);
            let tx = Arc::new(STTx::from_serial_iter(&mut tx_serial));
            let meta = TxMeta::from_raw(tx.get_transaction_id(), info.seq, &meta_bytes);
            (tx, meta.get_index())
        }))
        .map_err(|_| "invalid replay TransactionMd payload")?;

        if !tx_map
            .add_item(
                SHAMapNodeType::TransactionMd,
                SHAMapItem::new(tx.get_transaction_id(), payload.clone()),
            )
            .map_err(|_| "invalid replay transaction SHAMap item")?
        {
            return Err("duplicate replay transaction SHAMap item");
        }
        match ordered_txs.entry(index) {
            Entry::Vacant(slot) => {
                slot.insert(tx);
            }
            Entry::Occupied(_) => return Err("duplicate replay transaction index"),
        }
    }

    if tx_map.root().get_hash() != info.tx_hash {
        return Err("replay transaction SHAMap root mismatch");
    }

    Ok((info, ordered_txs))
}

impl ApplicationRoot {
    /// Receive a peer `TMReplayDeltaResponse`, validate it, wake its acquired
    /// delta, and synchronously drive the owning replay task. Newly rebuilt
    /// ledgers are stored before publication advancement, matching
    /// `LedgerDeltaAcquire.cpp::onLedgerBuilt` and
    /// `LedgerReplayTask.cpp::deltaReady` / `tryAdvance`.
    pub fn on_replay_delta_response(&self, response: &TmReplayDeltaResponse) -> bool {
        let (info, ordered_txs) = match decode_replay_delta_response(response) {
            Ok(decoded) => decoded,
            Err(reason) => {
                tracing::debug!(target: "ledger", %reason, "discarding invalid replay delta response");
                return false;
            }
        };
        let rules = self
            .closed_ledger()
            .or_else(|| self.validated_ledger())
            .map(|ledger| ledger.rules().clone())
            .unwrap_or_else(|| protocol::Rules::new(std::iter::empty()));

        let completed = match self.registry.ledger_replayer.lock() {
            Ok(mut replayer) => {
                replayer.got_replay_delta_with_rules(info, ordered_txs, rules);
                replayer.advance_ready_tasks(
                    &mut |hash, _seq| self.resolve_ledger_by_hash(SHAMapHash::new(hash)),
                    &mut |replay| crate::build_ledger_from_replay_delta(replay),
                )
            }
            Err(_) => {
                tracing::error!(target: "ledger", "ledger replayer lock poisoned; replay delta discarded");
                return false;
            }
        };
        let completed = match completed {
            Ok(ledgers) => ledgers,
            Err(error) => {
                tracing::warn!(target: "ledger", ?error, "replay task failed while advancing delta");
                return false;
            }
        };

        for ledger in completed {
            let ledger = self.store_consensus_ledger(ledger);
            self.check_accept_hash_seq(*ledger.header().hash.as_uint256(), ledger.header().seq);
        }
        self.try_advance_publication();
        true
    }
}

#[cfg(test)]
mod tests {
    use basics::{base_uint::Uint256, sha_map_hash::SHAMapHash};
    use overlay::TmReplayDeltaResponse;
    use protocol::{LedgerHeader, calculate_ledger_hash, serialize_ledger_header};
    use shamap::mutation::MutableTree;

    use super::decode_replay_delta_response;

    #[test]
    fn replay_response_requires_a_matching_header_and_transaction_root() {
        let tx_hash = MutableTree::new(1).root().get_hash();
        let mut header = LedgerHeader {
            seq: 1,
            tx_hash,
            parent_hash: SHAMapHash::new(Uint256::from_u64(1)),
            account_hash: SHAMapHash::new(Uint256::from_u64(2)),
            ..LedgerHeader::default()
        };
        header.hash = calculate_ledger_hash(&header);
        let response = TmReplayDeltaResponse {
            ledger_hash: header.hash.as_uint256().data().to_vec(),
            ledger_header: Some(serialize_ledger_header(&header, false)),
            transaction: Vec::new(),
            error: None,
        };

        let (decoded, txs) = decode_replay_delta_response(&response).expect("valid empty delta");
        assert_eq!(decoded, header);
        assert!(txs.is_empty());

        let mut mismatched = response;
        mismatched.ledger_hash = Uint256::from_u64(9).data().to_vec();
        assert!(decode_replay_delta_response(&mismatched).is_err());
    }
}
