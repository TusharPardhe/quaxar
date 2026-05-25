//! `LedgerReplayTask` owner port.

use crate::{
    InboundLedgerReason, Ledger, LedgerDeltaAcquire, LedgerDeltaBuildError, SkipListAcquire,
};
use basics::base_uint::Uint256;
use std::sync::{Arc, Mutex};

pub const REPLAY_TASK_MAX_TIMEOUTS_MULTIPLIER: u32 = 2;
pub const REPLAY_TASK_MAX_TIMEOUTS_MINIMUM: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerReplayTaskParameter {
    pub reason: InboundLedgerReason,
    pub finish_hash: Uint256,
    pub total_ledgers: u32,
    pub finish_seq: u32,
    pub skip_list: Vec<Uint256>,
    pub start_hash: Uint256,
    pub start_seq: u32,
    pub full: bool,
}

impl LedgerReplayTaskParameter {
    pub fn new(reason: InboundLedgerReason, finish_hash: Uint256, total_ledgers: u32) -> Self {
        assert!(
            finish_hash.is_non_zero() && total_ledgers > 0,
            "xrpl::LedgerReplayTask::TaskParameter::TaskParameter : valid inputs"
        );
        Self {
            reason,
            finish_hash,
            total_ledgers,
            finish_seq: 0,
            skip_list: Vec::new(),
            start_hash: Uint256::zero(),
            start_seq: 0,
            full: false,
        }
    }

    pub fn update(&mut self, hash: Uint256, seq: u32, skip_list: &[Uint256]) -> bool {
        if self.finish_hash != hash
            || skip_list.len() + 1 < self.total_ledgers as usize
            || self.full
        {
            return false;
        }

        self.finish_seq = seq;
        self.skip_list = skip_list.to_vec();
        self.skip_list.push(self.finish_hash);
        self.start_hash = self.skip_list[self.skip_list.len() - self.total_ledgers as usize];
        assert!(
            self.start_hash.is_non_zero(),
            "xrpl::LedgerReplayTask::TaskParameter::update : nonzero start hash"
        );
        self.start_seq = self.finish_seq - self.total_ledgers + 1;
        self.full = true;
        true
    }

    pub fn can_merge_into(&self, existing: &Self) -> bool {
        if self.reason != existing.reason {
            return false;
        }

        if self.finish_hash == existing.finish_hash && self.total_ledgers <= existing.total_ledgers
        {
            return true;
        }

        if existing.full
            && let Some(index) = existing
                .skip_list
                .iter()
                .position(|hash| *hash == self.finish_hash)
        {
            return existing.total_ledgers as usize
                >= self.total_ledgers as usize + (existing.skip_list.len() - index) - 1;
        }

        false
    }
}

pub struct LedgerReplayTask {
    parameter: LedgerReplayTaskParameter,
    max_timeouts: u32,
    _skip_list: Arc<Mutex<SkipListAcquire>>,
    parent: Option<Arc<Ledger>>,
    delta_to_build: usize,
    deltas: Vec<Arc<Mutex<LedgerDeltaAcquire>>>,
    complete: bool,
    failed: bool,
    stopping: bool,
    progress: bool,
    timeouts: u32,
}

impl LedgerReplayTask {
    pub fn new(
        parameter: LedgerReplayTaskParameter,
        skip_list: Arc<Mutex<SkipListAcquire>>,
    ) -> Self {
        let max_timeouts = REPLAY_TASK_MAX_TIMEOUTS_MINIMUM.max(
            parameter
                .total_ledgers
                .saturating_mul(REPLAY_TASK_MAX_TIMEOUTS_MULTIPLIER),
        );
        Self {
            parameter,
            max_timeouts,
            _skip_list: skip_list,
            parent: None,
            delta_to_build: 0,
            deltas: Vec::new(),
            complete: false,
            failed: false,
            stopping: false,
            progress: false,
            timeouts: 0,
        }
    }

    pub fn parameter(&self) -> &LedgerReplayTaskParameter {
        &self.parameter
    }

    pub fn finished(&self) -> bool {
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

    pub fn add_delta(&mut self, delta: Arc<Mutex<LedgerDeltaAcquire>>) {
        if self.finished() {
            return;
        }

        if let Some(last) = self.deltas.last() {
            let last_seq = last.lock().expect("last delta lock").ledger_seq();
            let next_seq = delta.lock().expect("delta lock").ledger_seq();
            assert_eq!(
                last_seq + 1,
                next_seq,
                "xrpl::LedgerReplayTask::addDelta : no deltas or consecutive sequence"
            );
        }

        self.deltas.push(delta);
    }

    pub fn update_skip_list(&mut self, hash: Uint256, seq: u32, skip_list: &[Uint256]) -> bool {
        if self.finished() {
            return false;
        }
        self.parameter.update(hash, seq, skip_list)
    }

    pub fn stop(&mut self) {
        self.stopping = true;
        if let Ok(mut skip_list) = self._skip_list.lock() {
            skip_list.stop();
        }
        for delta in &self.deltas {
            if let Ok(mut delta) = delta.lock() {
                delta.stop();
            }
        }
    }

    pub fn invoke_on_timer<LOOKUP, BUILD, E>(
        &mut self,
        lookup_parent: &mut LOOKUP,
        build_replay: &mut BUILD,
    ) -> Result<(), ReplayTaskError<E>>
    where
        LOOKUP: FnMut(Uint256, u32) -> Option<Arc<Ledger>>,
        BUILD: FnMut(
            &mut LedgerDeltaAcquire,
            &Arc<Ledger>,
        ) -> Result<Option<Arc<Ledger>>, LedgerDeltaBuildError<E>>,
    {
        if self.finished() {
            return Ok(());
        }

        if !self.progress {
            self.timeouts += 1;
            if self.timeouts > self.max_timeouts {
                self.failed = true;
                return Ok(());
            }
        } else {
            self.progress = false;
        }

        self.trigger(lookup_parent, build_replay)
    }

    pub fn trigger<LOOKUP, BUILD, E>(
        &mut self,
        lookup_parent: &mut LOOKUP,
        build_replay: &mut BUILD,
    ) -> Result<(), ReplayTaskError<E>>
    where
        LOOKUP: FnMut(Uint256, u32) -> Option<Arc<Ledger>>,
        BUILD: FnMut(
            &mut LedgerDeltaAcquire,
            &Arc<Ledger>,
        ) -> Result<Option<Arc<Ledger>>, LedgerDeltaBuildError<E>>,
    {
        if !self.parameter.full {
            return Ok(());
        }

        if self.parent.is_none() {
            self.parent = lookup_parent(self.parameter.start_hash, self.parameter.start_seq);
        }

        self.try_advance(build_replay)
    }

    fn try_advance<BUILD, E>(&mut self, build_replay: &mut BUILD) -> Result<(), ReplayTaskError<E>>
    where
        BUILD: FnMut(
            &mut LedgerDeltaAcquire,
            &Arc<Ledger>,
        ) -> Result<Option<Arc<Ledger>>, LedgerDeltaBuildError<E>>,
    {
        let should_try = self.parent.is_some()
            && self.parameter.full
            && self.parameter.total_ledgers.saturating_sub(1) as usize == self.deltas.len();
        if !should_try {
            return Ok(());
        }

        while self.delta_to_build < self.deltas.len() {
            let parent = self.parent.as_ref().expect("parent should exist").clone();
            let mut delta = self.deltas[self.delta_to_build].lock().expect("delta lock");
            assert_eq!(
                parent.header().seq + 1,
                delta.ledger_seq(),
                "xrpl::LedgerReplayTask::tryAdvance : consecutive sequence"
            );
            match build_replay(&mut delta, &parent).map_err(ReplayTaskError::Delta)? {
                Some(ledger) => {
                    self.parent = Some(ledger);
                    self.delta_to_build += 1;
                    self.progress = true;
                }
                None => return Ok(()),
            }
        }

        self.complete = true;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayTaskError<E> {
    Delta(LedgerDeltaBuildError<E>),
}
