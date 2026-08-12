//! Overlay replay callback implementation.
//!
//! This is the explicit bridge for the parity-flow edges
//! `peer replay response -> LedgerReplayer::got_replay_delta` and
//! `peer skip-list proof -> LedgerReplayer::got_skip_list`.

use std::{
    collections::{BTreeMap, btree_map::Entry},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use basics::{base_uint::Uint256, sha_map_hash::SHAMapHash};
use ledger::LedgerHeader;
use overlay::{
    TmProofPathRequest, TmProofPathResponse, TmReplayDeltaRequest, TmReplayDeltaResponse,
};
use protocol::{
    STTx, SerialIter, Serializer, TxMeta, calculate_ledger_hash, deserialize_ledger_header,
    serialize_ledger_header, skip_keylet,
};
use shamap::{
    item::SHAMapItem,
    mutation::MutableTree,
    proof_path::verify_proof_path,
    tree_node::{SHAMapNodeType, SHAMapTreeNode},
};

use super::ApplicationRoot;

const RE_NO_LEDGER: i32 = 1;
const RE_NO_NODE: i32 = 2;
const RE_BAD_REQUEST: i32 = 3;
const LM_TRANSACTION: i32 = 1;
const LM_ACCOUNT_STATE: i32 = 2;

/// Decode and verify `TMReplayDeltaResponse` exactly before it can reach the
/// replay owner. This is the Rust equivalent of
/// `LedgerReplayMsgHandler.cpp::processReplayDeltaResponse` (lines 214-282):
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

fn replay_error_response(ledger_hash: Vec<u8>, error: i32) -> TmReplayDeltaResponse {
    TmReplayDeltaResponse {
        ledger_hash,
        ledger_header: None,
        transaction: Vec::new(),
        error: Some(error),
    }
}

fn proof_error_response(request: &TmProofPathRequest, error: i32) -> TmProofPathResponse {
    TmProofPathResponse {
        key: request.key.clone(),
        ledger_hash: request.ledger_hash.clone(),
        r#type: request.r#type,
        ledger_header: None,
        path: Vec::new(),
        error: Some(error),
    }
}

impl ApplicationRoot {
    /// Serve `TMReplayDeltaRequest` only from an immutable, exact-hash ledger.
    /// This mirrors `LedgerReplayMsgHandler.cpp::processReplayDeltaRequest`:
    /// each TransactionMd payload is rebuilt as `STTx VL | TxMeta VL`, so the
    /// receiver can recover `sfTransactionIndex` before `BuildLedger.cpp`
    /// replays dependent transactions.
    pub fn replay_delta_response_for(
        &self,
        request: &TmReplayDeltaRequest,
    ) -> TmReplayDeltaResponse {
        let Some(hash) = Uint256::from_slice(&request.ledger_hash) else {
            return replay_error_response(request.ledger_hash.clone(), RE_BAD_REQUEST);
        };
        let ledger = self
            .resolve_ledger_by_hash(SHAMapHash::new(hash))
            .or_else(|| {
                self.closed_ledger()
                    .filter(|ledger| *ledger.header().hash.as_uint256() == hash)
            })
            .or_else(|| {
                self.validated_ledger()
                    .filter(|ledger| *ledger.header().hash.as_uint256() == hash)
            });
        let Some(ledger) = ledger.filter(|ledger| ledger.is_immutable()) else {
            return replay_error_response(request.ledger_hash.clone(), RE_NO_LEDGER);
        };

        let transaction = match ledger.tx_snapshot() {
            Ok(entries) => entries
                .into_iter()
                .map(|(tx, mut meta)| {
                    let mut payload = Serializer::default();
                    payload.add_vl(tx.get_serializer().data());
                    let mut raw_meta = Serializer::default();
                    meta.add_raw(&mut raw_meta, meta.get_result_ter(), meta.get_index());
                    payload.add_vl(raw_meta.data());
                    payload.data().to_vec()
                })
                .collect(),
            Err(_) => return replay_error_response(request.ledger_hash.clone(), RE_NO_NODE),
        };
        TmReplayDeltaResponse {
            ledger_hash: request.ledger_hash.clone(),
            ledger_header: Some(serialize_ledger_header(&ledger.header(), false)),
            transaction,
            error: None,
        }
    }

    /// Serve a proof from the exact requested ledger. Error categories and
    /// response shape follow `LedgerReplayMsgHandler.cpp::processProofPathRequest`.
    pub fn proof_path_response_for(&self, request: &TmProofPathRequest) -> TmProofPathResponse {
        let Some(key) = Uint256::from_slice(&request.key) else {
            return proof_error_response(request, RE_BAD_REQUEST);
        };
        let Some(hash) = Uint256::from_slice(&request.ledger_hash) else {
            return proof_error_response(request, RE_BAD_REQUEST);
        };
        if !matches!(request.r#type, LM_TRANSACTION | LM_ACCOUNT_STATE) {
            return proof_error_response(request, RE_BAD_REQUEST);
        }
        let ledger = self
            .resolve_ledger_by_hash(SHAMapHash::new(hash))
            .or_else(|| {
                self.closed_ledger()
                    .filter(|ledger| *ledger.header().hash.as_uint256() == hash)
            })
            .or_else(|| {
                self.validated_ledger()
                    .filter(|ledger| *ledger.header().hash.as_uint256() == hash)
            });
        let Some(ledger) = ledger else {
            return proof_error_response(request, RE_NO_LEDGER);
        };
        let path = match request.r#type {
            LM_ACCOUNT_STATE => ledger.state_map().get_proof_path(key, &mut |_| None),
            LM_TRANSACTION => ledger.tx_map().get_proof_path(key, &mut |_| None),
            _ => unreachable!("validated ledger-map type"),
        };
        let Ok(Some(path)) = path else {
            return proof_error_response(request, RE_NO_NODE);
        };
        TmProofPathResponse {
            key: request.key.clone(),
            ledger_hash: request.ledger_hash.clone(),
            r#type: request.r#type,
            ledger_header: Some(serialize_ledger_header(&ledger.header(), false)),
            path,
            error: None,
        }
    }

    /// Validate a short-skip-list proof and route the resulting `SHAMapItem`
    /// to `LedgerReplayer::got_skip_list`. This is the proof counterpart of
    /// `LedgerReplayMsgHandler.cpp::processProofPathResponse`; `got_skip_list`
    /// starts newly-created deltas and wakes `LedgerReplayTask` progress.
    pub fn on_proof_path_response(&self, response: &TmProofPathResponse) -> bool {
        if response.error.is_some()
            || response.r#type != LM_ACCOUNT_STATE
            || response.path.is_empty()
        {
            return false;
        }
        let (Some(key), Some(reply_hash), Some(header_bytes)) = (
            Uint256::from_slice(&response.key),
            Uint256::from_slice(&response.ledger_hash),
            response.ledger_header.as_deref(),
        ) else {
            return false;
        };
        if key != skip_keylet().key {
            return false;
        }
        let Ok(mut info) = deserialize_ledger_header(header_bytes, false) else {
            return false;
        };
        if calculate_ledger_hash(&info) != SHAMapHash::new(reply_hash)
            || !verify_proof_path(*info.account_hash.as_uint256(), key, &response.path)
        {
            return false;
        }
        info.hash = SHAMapHash::new(reply_hash);
        let Ok(Some(node)) = SHAMapTreeNode::make_from_wire(&response.path[0]) else {
            return false;
        };
        let Some(item) = node.is_leaf().then(|| node.peek_item()).flatten() else {
            return false;
        };
        if item.key() != key {
            return false;
        }

        let Some(runtime) = self.ledger_master_runtime() else {
            return false;
        };
        let root = self.clone();
        let inbound_ledgers = Arc::clone(&runtime.inbound_ledgers);
        let Ok(mut replayer) = self.registry.ledger_replayer.lock() else {
            return false;
        };
        replayer.got_skip_list(
            info,
            &item,
            1,
            move |hash| root.resolve_ledger_by_hash(SHAMapHash::new(hash)),
            move |hash, seq, _reason| {
                if let Ok(guard) = inbound_ledgers.lock()
                    && let Some(shared) = guard.as_ref()
                {
                    shared.acquire_async(
                        hash,
                        seq,
                        crate::ledger::inbound_ledgers::AcquireReason::Generic,
                    );
                }
            },
        );
        true
    }

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
                    &mut |hash, seq, _reason| {
                        if let Some(runtime) = self.ledger_master_runtime()
                            && let Ok(guard) = runtime.inbound_ledgers.lock()
                            && let Some(shared) = guard.as_ref()
                        {
                            shared.acquire_async(
                                hash,
                                seq,
                                crate::ledger::inbound_ledgers::AcquireReason::Generic,
                            );
                        }
                    },
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
        // A verified replay response can advance its owned task before a
        // validated/published head changes; request one serialized replan.
        self.request_publication_advance();
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
        // LedgerReplayMsgHandler.cpp::processReplayDeltaResponse rejects a
        // response before LedgerReplayer sees it unless both commitments hold.
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
