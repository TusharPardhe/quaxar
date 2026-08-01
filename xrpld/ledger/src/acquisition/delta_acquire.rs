//! `LedgerDeltaAcquire` owner port above the landed replay-data and overlay
//! seams.

use crate::{
    InboundLedgerReason, Ledger, LedgerConfig, LedgerHeader, LedgerReplay,
    REPLAY_SUB_TASK_FALLBACK_TIMEOUT, REPLAY_SUB_TASK_MAX_TIMEOUTS, REPLAY_SUB_TASK_TIMEOUT,
    timeout_counter::{
        TimeoutCounter, TimeoutCounterJobConfig, TimeoutCounterJournal, TimeoutCounterRuntime,
    },
};
use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use overlay::{PeerSet, ProtocolFeature, ProtocolMessage, ProtocolPayload, TmReplayDeltaRequest};
use protocol::STTx;
use shamap::sync::{SHAMapType, SyncTree};
use std::collections::BTreeMap;
use std::sync::Arc;
use time::Duration;

pub struct LedgerDeltaAcquire {
    hash: Uint256,
    ledger_seq: u32,
    peer_set: Arc<dyn PeerSet>,
    replay_temp: Option<Arc<Ledger>>,
    full_ledger: Option<Arc<Ledger>>,
    ordered_txs: BTreeMap<u32, Arc<STTx>>,
    reasons: Vec<InboundLedgerReason>,
    ready_reasons: Vec<InboundLedgerReason>,
    no_feature_peer_count: u32,
    fall_back: bool,
    complete: bool,
    failed: bool,
    stopping: bool,
    progress: bool,
    timeouts: i32,
    timer_interval: Duration,
    timeout_counter: Option<TimeoutCounter>,
}

impl LedgerDeltaAcquire {
    pub fn new(hash: Uint256, ledger_seq: u32, peer_set: Arc<dyn PeerSet>) -> Self {
        Self {
            hash,
            ledger_seq,
            peer_set,
            replay_temp: None,
            full_ledger: None,
            ordered_txs: BTreeMap::new(),
            reasons: Vec::new(),
            ready_reasons: Vec::new(),
            no_feature_peer_count: 0,
            fall_back: false,
            complete: false,
            failed: false,
            stopping: false,
            progress: false,
            timeouts: 0,
            timer_interval: REPLAY_SUB_TASK_TIMEOUT,
            timeout_counter: None,
        }
    }

    pub fn hash(&self) -> Uint256 {
        self.hash
    }

    pub fn ledger_seq(&self) -> u32 {
        self.ledger_seq
    }

    pub fn is_done(&self) -> bool {
        self.complete || self.failed || self.stopping
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn is_failed(&self) -> bool {
        self.failed
    }

    pub fn is_stopped(&self) -> bool {
        self.stopping
    }

    pub fn full_ledger(&self) -> Option<&Arc<Ledger>> {
        self.full_ledger.as_ref()
    }

    pub fn stop(&mut self) {
        self.stopping = true;
    }

    pub fn add_data_reason(&mut self, reason: InboundLedgerReason) {
        if self.is_done() {
            return;
        }
        if !self.reasons.contains(&reason) {
            self.reasons.push(reason);
        }
        if self.full_ledger.is_some() && !self.ready_reasons.contains(&reason) {
            self.ready_reasons.push(reason);
        }
    }

    pub fn drain_ready_reasons(&mut self) -> Vec<InboundLedgerReason> {
        std::mem::take(&mut self.ready_reasons)
    }

    pub fn clear_ready_reasons(&mut self) {
        self.ready_reasons.clear();
    }

    pub fn is_fallback(&self) -> bool {
        self.fall_back
    }

    pub fn timer_interval(&self) -> Duration {
        self.timer_interval
    }

    /// Construct and retain the TimeoutCounter that drives this replay
    /// subtask. It starts at 250 ms and changes to one second on fallback.
    pub fn create_timeout_counter(
        &mut self,
        runtime: Arc<dyn TimeoutCounterRuntime>,
        journal: Arc<dyn TimeoutCounterJournal>,
        job: TimeoutCounterJobConfig,
        on_timer: impl Fn(bool, TimeoutCounter) + Send + Sync + 'static,
    ) -> TimeoutCounter {
        let counter = TimeoutCounter::new(
            runtime,
            journal,
            self.hash,
            self.timer_interval,
            job,
            on_timer,
        );
        self.timeout_counter = Some(counter.clone());
        counter
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

    pub fn process_data(
        &mut self,
        info: LedgerHeader,
        ordered_txs: BTreeMap<u32, Arc<STTx>>,
        config: &LedgerConfig,
    ) {
        if self.is_done() {
            return;
        }

        if info.seq == self.ledger_seq {
            let mut replay_temp = Ledger::from_maps(
                info,
                SyncTree::new_synching_with_type(SHAMapType::State, true, info.seq),
                SyncTree::new_synching_with_type(SHAMapType::Transaction, true, info.seq),
            );
            replay_temp.set_rules(crate::Rules::new(config.features.iter()));
            self.replay_temp = Some(Arc::new(replay_temp));
            self.ordered_txs = ordered_txs;
            self.complete = true;
            self.progress = true;
            return;
        }

        self.failed = true;
    }

    pub fn try_build<E, BUILD>(
        &mut self,
        parent: &Arc<Ledger>,
        build_replay: &mut BUILD,
    ) -> Result<Option<Arc<Ledger>>, LedgerDeltaBuildError<E>>
    where
        BUILD: FnMut(&LedgerReplay) -> Result<Arc<Ledger>, E>,
    {
        if let Some(full) = &self.full_ledger {
            return Ok(Some(Arc::clone(full)));
        }

        let Some(replay_temp) = self.replay_temp.as_ref() else {
            return Ok(None);
        };
        if self.failed || !self.complete {
            return Ok(None);
        }

        assert_eq!(
            parent.header().seq + 1,
            replay_temp.header().seq,
            "xrpl::LedgerDeltaAcquire::tryBuild : parent sequence match"
        );
        assert_eq!(
            parent.header().hash,
            replay_temp.header().parent_hash,
            "xrpl::LedgerDeltaAcquire::tryBuild : parent hash match"
        );

        let replay = LedgerReplay::new(
            Arc::clone(parent),
            Arc::clone(replay_temp),
            self.ordered_txs.clone(),
        );
        let built = build_replay(&replay).map_err(LedgerDeltaBuildError::Builder)?;

        if built.header().hash == SHAMapHash::new(self.hash) {
            self.full_ledger = Some(Arc::clone(&built));
            for reason in &self.reasons {
                if !self.ready_reasons.contains(reason) {
                    self.ready_reasons.push(*reason);
                }
            }
            return Ok(Some(built));
        }

        self.failed = true;
        self.complete = false;
        Err(LedgerDeltaBuildError::HashMismatch {
            expected: self.hash,
            actual: *built.header().hash.as_uint256(),
        })
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
        if let Some(full) = lookup_ledger(self.hash) {
            self.full_ledger = Some(full);
            self.complete = true;
            for reason in &self.reasons {
                if !self.ready_reasons.contains(reason) {
                    self.ready_reasons.push(*reason);
                }
            }
            self.progress = true;
            return;
        }

        if !self.fall_back {
            let request =
                ProtocolMessage::new(ProtocolPayload::ReplayDeltaRequest(TmReplayDeltaRequest {
                    ledger_hash: self.hash.data().to_vec(),
                }));

            self.peer_set.add_peers(
                limit,
                &mut |peer| peer.has_ledger(self.hash, self.ledger_seq),
                &mut |peer| {
                    if peer.supports_feature(ProtocolFeature::LedgerReplay) {
                        self.peer_set.send_request(&request, Some(peer));
                    } else {
                        self.no_feature_peer_count += 1;
                        if self.no_feature_peer_count >= crate::REPLAY_MAX_NO_FEATURE_PEER_COUNT {
                            self.fall_back = true;
                            self.timer_interval = REPLAY_SUB_TASK_FALLBACK_TIMEOUT;
                            if let Some(counter) = self.timeout_counter.as_ref() {
                                counter.set_timer_interval(self.timer_interval);
                            }
                        }
                    }
                },
            );
        }

        if self.fall_back {
            fallback_acquire(self.hash, self.ledger_seq, InboundLedgerReason::Generic);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerDeltaBuildError<E> {
    Builder(E),
    HashMismatch { expected: Uint256, actual: Uint256 },
}
