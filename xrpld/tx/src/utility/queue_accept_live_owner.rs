//! Live-owner wrapper for the current `xrpld` `TxQ::accept(...)` boundary.
//!
//! This layer moves the actual TxQ-owned accept state into one Rust owner:
//! - fee metrics,
//! - current queue max size,
//! - parent-hash comparator state,
//! - synchronized queue/account views.

use std::{collections::BTreeSet, fmt::Display};

use crate::{
    ApplyResult, ClosedLedgerCandidate, ClosedLedgerMaintenanceWithMetrics, FeeQueueKey, MaybeTx,
    QueueAcceptEntryResult, QueueAcceptLedgerViewSource, QueueAcceptOwnerState,
    QueueAcceptPreparedCallStep, QueueAcceptRuntimeSnapshot, QueueFeeMetricsState, QueueViews,
    TxQSetup, prepare_queue_accept_top_with_runtime_source, process_closed_ledger_with_metrics,
    run_queue_accept_top_with_runtime_source,
    run_queue_accept_top_with_runtime_source_with_log_sinks,
};

pub trait QueueAcceptLiveApplyRuntime<Account, Tx, Journal, ParentBatchId> {
    fn apply_queued(
        &mut self,
        queued: &mut MaybeTx<Tx, Account, Journal, ParentBatchId>,
    ) -> ApplyResult;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptLiveOwner<Account, Tx, Journal, ParentBatchId> {
    metrics: QueueFeeMetricsState,
    current_max_size: Option<usize>,
    owner_state: QueueAcceptOwnerState,
    views: QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
}

impl<Account, Tx, Journal, ParentBatchId>
    QueueAcceptLiveOwner<Account, Tx, Journal, ParentBatchId>
{
    pub fn new(
        metrics: QueueFeeMetricsState,
        current_max_size: Option<usize>,
        owner_state: QueueAcceptOwnerState,
        views: QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> Self {
        Self {
            metrics,
            current_max_size,
            owner_state,
            views,
        }
    }

    pub fn new_from_setup(
        setup: &TxQSetup,
        current_max_size: Option<usize>,
        owner_state: QueueAcceptOwnerState,
        views: QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> Self {
        Self::new(
            setup.fee_metrics_state(),
            current_max_size,
            owner_state,
            views,
        )
    }

    pub fn metrics(&self) -> &QueueFeeMetricsState {
        &self.metrics
    }

    pub fn metrics_mut(&mut self) -> &mut QueueFeeMetricsState {
        &mut self.metrics
    }

    pub const fn current_max_size(&self) -> Option<usize> {
        self.current_max_size
    }

    pub fn set_current_max_size(&mut self, current_max_size: Option<usize>) {
        self.current_max_size = current_max_size;
    }

    pub const fn owner_state(&self) -> QueueAcceptOwnerState {
        self.owner_state
    }

    pub fn owner_state_mut(&mut self) -> &mut QueueAcceptOwnerState {
        &mut self.owner_state
    }

    pub fn views(&self) -> &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>> {
        &self.views
    }

    pub fn views_mut(
        &mut self,
    ) -> &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>> {
        &mut self.views
    }

    pub fn process_closed_ledger<I>(
        &mut self,
        validated_fee_levels: &[crate::FeeLevel64],
        by_fee_candidates: I,
        ledger_seq: u32,
        time_leap: bool,
    ) -> ClosedLedgerMaintenanceWithMetrics<Account>
    where
        Account: Clone + Ord,
        I: IntoIterator<Item = ClosedLedgerCandidate<Account>>,
    {
        let had_current_max_size = self.current_max_size.is_some();
        let current_max_size = self
            .current_max_size
            .unwrap_or_else(|| self.metrics.queue_size_min());
        let result = process_closed_ledger_with_metrics(
            &mut self.metrics,
            validated_fee_levels,
            &mut self.views.accounts,
            by_fee_candidates,
            ledger_seq,
            current_max_size,
            time_leap,
        );
        let expired = result
            .maintenance
            .expired_candidates
            .iter()
            .map(|candidate| FeeQueueKey::new(candidate.account.clone(), candidate.seq_proxy))
            .collect::<BTreeSet<_>>();

        self.views
            .fee_order
            .retain(|entry| !expired.contains(&entry.key));
        self.current_max_size = if !had_current_max_size && time_leap {
            None
        } else {
            Some(result.maintenance.next_max_size)
        };

        result
    }
}

pub fn snapshot_queue_accept_live_owner<Account, Tx, Journal, ParentBatchId, View>(
    owner: &QueueAcceptLiveOwner<Account, Tx, Journal, ParentBatchId>,
    view: &View,
) -> QueueAcceptRuntimeSnapshot
where
    View: QueueAcceptLedgerViewSource,
{
    QueueAcceptRuntimeSnapshot::new(
        owner.metrics().clone(),
        view.open_ledger_tx_count(),
        view.parent_hash(),
        owner.current_max_size(),
    )
}

pub fn run_queue_accept_with_live_owner<Account, Tx, Journal, ParentBatchId, App, View>(
    owner: &mut QueueAcceptLiveOwner<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
) -> QueueAcceptEntryResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    App: QueueAcceptLiveApplyRuntime<Account, Tx, Journal, ParentBatchId>,
    View: QueueAcceptLedgerViewSource,
{
    let snapshot = snapshot_queue_accept_live_owner(&*owner, view);
    let accept = run_queue_accept_top_with_runtime_source(
        &mut owner.views,
        &mut owner.owner_state,
        &snapshot,
        |queued| app.apply_queued(queued),
    );

    QueueAcceptEntryResult {
        ledger_changed: accept.ledger_changed(),
        accept,
    }
}

pub fn prepare_queue_accept_with_live_owner<Account, Tx, Journal, ParentBatchId, View>(
    owner: &mut QueueAcceptLiveOwner<Account, Tx, Journal, ParentBatchId>,
    view: &View,
) -> QueueAcceptPreparedCallStep<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    View: QueueAcceptLedgerViewSource,
{
    let snapshot = snapshot_queue_accept_live_owner(&*owner, view);
    prepare_queue_accept_top_with_runtime_source(
        &mut owner.views,
        &mut owner.owner_state,
        &snapshot,
    )
}

pub fn run_queue_accept_with_live_owner_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    App,
    View,
    TraceFn,
    DebugFn,
    InfoFn,
    WarnFn,
>(
    owner: &mut QueueAcceptLiveOwner<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    warn: WarnFn,
) -> QueueAcceptEntryResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    App: QueueAcceptLiveApplyRuntime<Account, Tx, Journal, ParentBatchId>,
    View: QueueAcceptLedgerViewSource,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    let snapshot = snapshot_queue_accept_live_owner(&*owner, view);
    let accept = run_queue_accept_top_with_runtime_source_with_log_sinks(
        &mut owner.views,
        &mut owner.owner_state,
        &snapshot,
        trace,
        debug,
        info,
        warn,
        |queued| app.apply_queued(queued),
    );

    QueueAcceptEntryResult {
        ledger_changed: accept.ledger_changed(),
        accept,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use basics::base_uint::Uint256;
    use protocol::SeqProxy;

    use super::QueueAcceptLiveOwner;
    use crate::{
        ApplyFlags, ClosedLedgerCandidate, MaybeTx, MaybeTxCore, PreflightResult,
        QueueAcceptOwnerState, QueueViews, TxConsequences, TxQAccount, TxQSetup,
    };

    fn queued_owner(
        current_max_size: Option<usize>,
    ) -> QueueAcceptLiveOwner<&'static str, &'static str, &'static str, &'static str> {
        let seq_proxy = SeqProxy::sequence(5);
        let consequences = TxConsequences::new(1, seq_proxy);
        let mut account = TxQAccount::new("acct");
        account.add(
            seq_proxy,
            MaybeTxCore::new(
                MaybeTx::new(
                    Uint256::from_u64(7),
                    256,
                    "acct",
                    Some(20),
                    seq_proxy,
                    ApplyFlags::NONE,
                    PreflightResult::new(
                        "tx",
                        None::<&str>,
                        protocol::Rules::new(std::iter::empty()),
                        consequences,
                        ApplyFlags::NONE,
                        "journal",
                        protocol::Ter::TES_SUCCESS,
                    ),
                ),
                consequences,
            ),
        );

        QueueAcceptLiveOwner::new_from_setup(
            &TxQSetup::default(),
            current_max_size,
            QueueAcceptOwnerState::new(Uint256::from_u64(0)),
            QueueViews::new(BTreeMap::from([("acct", account)]), vec![]),
        )
    }

    #[test]
    fn process_closed_ledger_initializes_max_size_on_first_non_time_leap() {
        let mut owner = queued_owner(None);

        let result = owner.process_closed_ledger(
            &[],
            std::iter::empty::<ClosedLedgerCandidate<_>>(),
            20,
            false,
        );

        assert_eq!(
            owner.current_max_size(),
            Some(result.maintenance.next_max_size)
        );
        assert!(owner.current_max_size().is_some());
    }

    #[test]
    fn process_closed_ledger_preserves_unset_max_size_on_first_time_leap() {
        let mut owner = queued_owner(None);

        let result = owner.process_closed_ledger(
            &[],
            std::iter::empty::<ClosedLedgerCandidate<_>>(),
            20,
            true,
        );

        assert_eq!(
            result.maintenance.next_max_size,
            owner.metrics().queue_size_min()
        );
        assert_eq!(owner.current_max_size(), None);
    }
}
