//! `SkipListAcquire` owner port above the landed ledger and overlay seams.

use crate::{InboundLedgerReason, Ledger};
use basics::base_uint::Uint256;
use overlay::{PeerSet, ProtocolFeature, ProtocolMessage, ProtocolPayload, TmProofPathRequest};
use protocol::{decode_ledger_hashes_entry, keylet};
use shamap::item::SHAMapItem;
use std::sync::Arc;

pub const REPLAY_SUB_TASK_MAX_TIMEOUTS: i32 = 10;
pub const REPLAY_MAX_NO_FEATURE_PEER_COUNT: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipListData {
    pub ledger_seq: u32,
    pub skip_list: Vec<Uint256>,
}

pub struct SkipListAcquire {
    hash: Uint256,
    peer_set: Arc<dyn PeerSet>,
    data: Option<SkipListData>,
    no_feature_peer_count: u32,
    fall_back: bool,
    complete: bool,
    failed: bool,
    stopping: bool,
    progress: bool,
    timeouts: i32,
}

impl SkipListAcquire {
    pub fn new(hash: Uint256, peer_set: Arc<dyn PeerSet>) -> Self {
        Self {
            hash,
            peer_set,
            data: None,
            no_feature_peer_count: 0,
            fall_back: false,
            complete: false,
            failed: false,
            stopping: false,
            progress: false,
            timeouts: 0,
        }
    }

    pub fn hash(&self) -> Uint256 {
        self.hash
    }

    pub fn get_data(&self) -> Option<&SkipListData> {
        self.data.as_ref()
    }

    pub fn is_done(&self) -> bool {
        self.complete || self.failed || self.stopping
    }

    pub fn is_failed(&self) -> bool {
        self.failed
    }

    pub fn is_stopped(&self) -> bool {
        self.stopping
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn stop(&mut self) {
        self.stopping = true;
    }

    pub fn is_fallback(&self) -> bool {
        self.fall_back
    }

    pub fn init<LOOKUP, FALLBACK>(
        &mut self,
        num_peers: usize,
        lookup_ledger: &mut LOOKUP,
        fallback_acquire: &mut FALLBACK,
    ) where
        LOOKUP: FnMut(Uint256) -> Option<Arc<Ledger>>,
        FALLBACK: FnMut(Uint256, u32, InboundLedgerReason),
    {
        if !self.is_done() {
            self.trigger(num_peers, lookup_ledger, fallback_acquire);
        }
    }

    pub fn invoke_on_timer<LOOKUP, FALLBACK>(
        &mut self,
        lookup_ledger: &mut LOOKUP,
        fallback_acquire: &mut FALLBACK,
    ) where
        LOOKUP: FnMut(Uint256) -> Option<Arc<Ledger>>,
        FALLBACK: FnMut(Uint256, u32, InboundLedgerReason),
    {
        if self.is_done() {
            return;
        }

        if !self.progress {
            self.timeouts += 1;
            if self.timeouts > REPLAY_SUB_TASK_MAX_TIMEOUTS {
                self.failed = true;
                return;
            }
        } else {
            self.progress = false;
        }

        self.trigger(1, lookup_ledger, fallback_acquire);
    }

    pub fn process_data(&mut self, ledger_seq: u32, item: &SHAMapItem) {
        if ledger_seq == 0 || self.is_done() {
            return;
        }

        if let Ok(entry) = decode_ledger_hashes_entry(item.data()) {
            let hashes = entry.hashes;
            if !hashes.is_empty() {
                self.complete = true;
                self.data = Some(SkipListData {
                    ledger_seq,
                    skip_list: hashes,
                });
                self.progress = true;
                return;
            }
        }

        self.failed = true;
    }

    fn trigger<LOOKUP, FALLBACK>(
        &mut self,
        limit: usize,
        lookup_ledger: &mut LOOKUP,
        fallback_acquire: &mut FALLBACK,
    ) where
        LOOKUP: FnMut(Uint256) -> Option<Arc<Ledger>>,
        FALLBACK: FnMut(Uint256, u32, InboundLedgerReason),
    {
        if let Some(ledger) = lookup_ledger(self.hash) {
            if let Ok(Some((skip, _))) = ledger
                .state_map()
                .peek_item_with_hash(keylet::skip().key, &mut |_| None)
                && let Ok(entry) = decode_ledger_hashes_entry(skip.data())
            {
                let hashes = entry.hashes;
                if !hashes.is_empty() {
                    self.complete = true;
                    self.data = Some(SkipListData {
                        ledger_seq: ledger.header().seq,
                        skip_list: hashes,
                    });
                    self.progress = true;
                    return;
                }
            }

            self.failed = true;
            return;
        }

        if !self.fall_back {
            let request =
                ProtocolMessage::new(ProtocolPayload::ProofPathRequest(TmProofPathRequest {
                    key: keylet::skip().key.data().to_vec(),
                    ledger_hash: self.hash.data().to_vec(),
                    r#type: 2,
                }));

            self.peer_set.add_peers(
                limit,
                &mut |peer| {
                    peer.supports_feature(ProtocolFeature::LedgerReplay)
                        && peer.has_ledger(self.hash, 0)
                },
                &mut |peer| {
                    if peer.supports_feature(ProtocolFeature::LedgerReplay) {
                        self.peer_set.send_request(&request, Some(peer));
                    } else {
                        self.no_feature_peer_count += 1;
                        if self.no_feature_peer_count >= REPLAY_MAX_NO_FEATURE_PEER_COUNT {
                            self.fall_back = true;
                        }
                    }
                },
            );
        }

        if self.fall_back {
            fallback_acquire(self.hash, 0, InboundLedgerReason::Generic);
        }
    }
}
