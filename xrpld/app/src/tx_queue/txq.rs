//! App-owned `TxQ` runtime shell above the landed `xrpld/tx` internals.
//!
//! The `tx` crate already owns the deterministic queue mechanics. This app
//! layer owns the runtime state that the reference `TxQ` class keeps together:
//! - one shared queue/account view,
//! - one shared fee-metrics state,
//! - one shared parent-hash ordering seed,
//! - one shared current max-size snapshot,
//! - and method-style `accept(...)`, `apply(...)`, query, and maintenance
//!   surfaces over those shared owner values.
//!
//! The important Rust design point for a JS/TS reader is ownership: instead of
//! keeping separate mutable `accept` and `apply` objects that can drift apart,
//! this owner keeps one canonical state and reconstructs the narrower tx-layer
//! facades only for the duration of each call.

use std::{collections::BTreeMap, fmt::Display, mem};

use basics::mul_div::mul_div;
use protocol::Ter;
use tx::{
    ApplyFlags, ClosedLedgerMaintenanceWithMetrics, MaybeTx, PreflightResult,
    QueueAcceptEntryResult, QueueAcceptJournalOwner, QueueAcceptJournalSink,
    QueueAcceptLedgerViewSource, QueueAcceptLiveApplyRuntime, QueueAcceptLiveOwner,
    QueueAcceptLockScope, QueueAcceptLockScopeOwner, QueueAcceptOwnerShell, QueueAcceptOwnerState,
    QueueAcceptPreparedCallStep, QueueAcceptTxQ, QueueApplyAppRuntime,
    QueueApplyHoldPreflightTxSource, QueueApplyJournalSink, QueueApplyJournalTxQ,
    QueueApplyLedgerViewSource, QueueApplyLiveOwner, QueueApplyLockScope, QueueApplyLockScopeOwner,
    QueueApplyObservedTxSource, QueueApplyOwnerShell, QueueApplyPreflightStage,
    QueueApplyTopWithLogMessagesResult, QueueApplyTxQ, QueueFeeMetricsSnapshot,
    QueueFeeMetricsState, QueueHoldPreflight, QueueTxQAccountState, QueueTxQClosedLedgerAppSource,
    QueueTxQClosedLedgerView, QueueTxQMetrics, QueueTxQRequiredFeeAndSeq,
    QueueTxQRequiredFeeTxSource, QueueTxQRequiredFeeViewSource, QueueTxQRpcAppSource,
    QueueTxQRpcReport, QueueTxQRpcView, QueueViews, TxConsequences, TxDetails, TxQSetup,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct NullTxQJournal;

impl QueueAcceptJournalSink for NullTxQJournal {
    fn trace(&mut self, _message: &str) {}

    fn debug(&mut self, _message: &str) {}

    fn info(&mut self, _message: &str) {}

    fn warn(&mut self, _message: &str) {}
}

impl QueueApplyJournalSink for NullTxQJournal {
    fn trace(&mut self, _message: &str) {}

    fn debug(&mut self, _message: &str) {}

    fn info(&mut self, _message: &str) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxQ<Account, Tx, Journal, ParentBatchId> {
    setup: TxQSetup,
    metrics: QueueFeeMetricsState,
    current_max_size: Option<usize>,
    owner_state: QueueAcceptOwnerState,
    views: QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
}

impl<Account, Tx, Journal, ParentBatchId> TxQ<Account, Tx, Journal, ParentBatchId> {
    fn empty_views() -> QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>> {
        QueueViews::new(BTreeMap::new(), Vec::new())
    }

    pub fn new(
        setup: TxQSetup,
        metrics: QueueFeeMetricsState,
        current_max_size: Option<usize>,
        owner_state: QueueAcceptOwnerState,
        views: QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> Self {
        Self {
            setup,
            metrics,
            current_max_size,
            owner_state,
            views,
        }
    }

    pub fn new_from_setup(
        setup: TxQSetup,
        current_max_size: Option<usize>,
        owner_state: QueueAcceptOwnerState,
        views: QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> Self {
        let metrics = setup.fee_metrics_state();
        Self::new(setup, metrics, current_max_size, owner_state, views)
    }

    pub fn setup(&self) -> &TxQSetup {
        &self.setup
    }

    pub fn set_standalone(&mut self, standalone: bool) {
        self.setup.standalone = standalone;
        self.metrics = self.setup.fee_metrics_state();
    }

    /// Reconfigure the TxQ setup from a parsed config section.
    /// Matches rippled's `setupTxQ(Config const& config)`.
    pub fn reconfigure_setup(&mut self, new_setup: TxQSetup) {
        let standalone = self.setup.standalone;
        self.setup = new_setup;
        self.setup.standalone = standalone;
        self.metrics = self.setup.fee_metrics_state();
    }

    pub fn metrics(&self) -> &QueueFeeMetricsState {
        &self.metrics
    }

    pub const fn current_max_size(&self) -> Option<usize> {
        self.current_max_size
    }

    pub const fn owner_state(&self) -> QueueAcceptOwnerState {
        self.owner_state
    }

    pub fn views(&self) -> &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>> {
        &self.views
    }

    pub fn views_mut(
        &mut self,
    ) -> &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>> {
        &mut self.views
    }

    fn take_accept_txq(
        &mut self,
    ) -> QueueAcceptTxQ<Account, Tx, Journal, ParentBatchId, NullTxQJournal> {
        let views = mem::replace(&mut self.views, Self::empty_views());
        QueueAcceptTxQ::new(QueueAcceptLockScopeOwner::new(
            QueueAcceptJournalOwner::new(
                QueueAcceptOwnerShell::new(QueueAcceptLiveOwner::new(
                    self.metrics.clone(),
                    self.current_max_size,
                    self.owner_state,
                    views,
                )),
                NullTxQJournal,
            ),
        ))
    }

    fn restore_from_accept_txq(
        &mut self,
        txq: &mut QueueAcceptTxQ<Account, Tx, Journal, ParentBatchId, NullTxQJournal>,
    ) {
        let live_owner = txq
            .lock_scope_owner_mut()
            .owner_mut()
            .owner_mut()
            .owner_mut();
        self.metrics = live_owner.metrics().clone();
        self.current_max_size = live_owner.current_max_size();
        self.owner_state = live_owner.owner_state();
        self.views = mem::replace(live_owner.views_mut(), Self::empty_views());
    }

    fn with_accept_txq<R>(
        &mut self,
        run: impl FnOnce(&mut QueueAcceptTxQ<Account, Tx, Journal, ParentBatchId, NullTxQJournal>) -> R,
    ) -> R {
        let mut txq = self.take_accept_txq();
        let result = run(&mut txq);
        self.restore_from_accept_txq(&mut txq);
        result
    }

    fn take_apply_txq(&mut self) -> QueueApplyTxQ<Account, Tx, Journal, ParentBatchId> {
        let views = mem::replace(&mut self.views, Self::empty_views());
        let maximum_txn_per_account =
            usize::try_from(self.setup.maximum_txn_per_account).unwrap_or(usize::MAX);

        QueueApplyTxQ::new(QueueApplyLockScopeOwner::new(QueueApplyOwnerShell::new(
            QueueApplyLiveOwner::new_with_metrics(
                self.metrics.clone(),
                self.setup.minimum_last_ledger_buffer,
                maximum_txn_per_account,
                self.setup.retry_sequence_percent,
                self.current_max_size,
                self.owner_state.current_order(),
                views,
            ),
        )))
    }

    fn restore_from_apply_txq(
        &mut self,
        txq: &mut QueueApplyTxQ<Account, Tx, Journal, ParentBatchId>,
    ) {
        let live_owner = txq.lock_scope_owner_mut().owner_mut().owner_mut();
        self.metrics = live_owner.metrics().clone();
        self.current_max_size = live_owner.current_max_size();
        self.views = mem::replace(live_owner.views_mut(), Self::empty_views());
    }

    fn with_apply_txq<R>(
        &mut self,
        run: impl FnOnce(&mut QueueApplyTxQ<Account, Tx, Journal, ParentBatchId>) -> R,
    ) -> R {
        let mut txq = self.take_apply_txq();
        let result = run(&mut txq);
        self.restore_from_apply_txq(&mut txq);
        result
    }

    fn take_apply_journal_txq(
        &mut self,
    ) -> QueueApplyJournalTxQ<Account, Tx, Journal, ParentBatchId, NullTxQJournal> {
        self.take_apply_txq().into_journal_txq(NullTxQJournal)
    }

    fn restore_from_apply_journal_txq(
        &mut self,
        txq: &mut QueueApplyJournalTxQ<Account, Tx, Journal, ParentBatchId, NullTxQJournal>,
    ) {
        let live_owner = txq
            .lock_scope_owner_mut()
            .owner_mut()
            .owner_mut()
            .owner_mut();
        self.metrics = live_owner.metrics().clone();
        self.current_max_size = live_owner.current_max_size();
        self.views = mem::replace(live_owner.views_mut(), Self::empty_views());
    }

    fn with_apply_journal_txq<R>(
        &mut self,
        run: impl FnOnce(
            &mut QueueApplyJournalTxQ<Account, Tx, Journal, ParentBatchId, NullTxQJournal>,
        ) -> R,
    ) -> R {
        let mut txq = self.take_apply_journal_txq();
        let result = run(&mut txq);
        self.restore_from_apply_journal_txq(&mut txq);
        result
    }
}

impl<Account, Tx, Journal, ParentBatchId> TxQ<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    fn build_metrics(&self, tx_in_ledger: usize) -> QueueTxQMetrics {
        let snapshot = self.metrics.snapshot();
        let is_full = self
            .current_max_size
            .is_some_and(|max_size| self.views.fee_order.len() >= max_size);

        QueueTxQMetrics {
            tx_count: self.views.fee_order.len(),
            tx_q_max_size: self.current_max_size,
            tx_in_ledger,
            tx_per_ledger: snapshot.txns_expected,
            reference_fee_level: tx::TXQ_BASE_LEVEL,
            min_processing_fee_level: if is_full {
                self.views
                    .fee_order
                    .last()
                    .map(|entry| entry.candidate.fee_level + 1)
                    .expect("xrpl::TxQ::getMetrics : full queue must have a trailing fee entry")
            } else {
                tx::TXQ_BASE_LEVEL
            },
            med_fee_level: snapshot.escalation_multiplier,
            open_ledger_fee_level: tx::evaluate_required_fee_level(
                snapshot,
                tx_in_ledger,
                ApplyFlags::NONE,
            ),
        }
    }

    pub fn accept<Lock, App, View>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
    ) -> QueueAcceptEntryResult<Account>
    where
        Lock: QueueAcceptLockScope,
        App: QueueAcceptLiveApplyRuntime<Account, Tx, Journal, ParentBatchId>,
        View: QueueAcceptLedgerViewSource,
    {
        self.with_accept_txq(|txq| txq.accept(lock, app, view))
    }

    pub fn prepare_accept<Lock, View>(
        &mut self,
        lock: &mut Lock,
        view: &View,
    ) -> QueueAcceptPreparedCallStep<Account>
    where
        Lock: QueueAcceptLockScope,
        View: QueueAcceptLedgerViewSource,
    {
        self.with_accept_txq(|txq| txq.prepare_accept(lock, view))
    }

    pub fn next_queuable_seq<Lock>(
        &self,
        _lock: &mut Lock,
        account_state: QueueTxQAccountState<'_, Account>,
    ) -> protocol::SeqProxy
    where
        Lock: QueueAcceptLockScope,
    {
        let QueueTxQAccountState::Present { account, seq_proxy } = account_state else {
            return protocol::SeqProxy::sequence(0);
        };

        self.views
            .accounts
            .get(account)
            .filter(|queued_account| !queued_account.transactions.is_empty())
            .map_or(seq_proxy, |queued_account| {
                queued_account.next_queuable_seq(seq_proxy)
            })
    }

    pub fn get_account_txs<Lock>(
        &self,
        _lock: &mut Lock,
        account: &Account,
    ) -> Vec<TxDetails<Tx, Account>>
    where
        Lock: QueueAcceptLockScope,
    {
        self.views
            .accounts
            .get(account)
            .map_or_else(Vec::new, |queued_account| {
                queued_account
                    .transactions
                    .values()
                    .map(|queued| queued.payload.get_tx_details())
                    .collect()
            })
    }

    pub fn get_txs<Lock>(&self, _lock: &mut Lock) -> Vec<TxDetails<Tx, Account>>
    where
        Lock: QueueAcceptLockScope,
    {
        self.views
            .fee_order
            .iter()
            .map(|entry| {
                self.views
                    .accounts
                    .get(&entry.key.account)
                    .and_then(|queued_account| {
                        queued_account.transactions.get(&entry.key.seq_proxy)
                    })
                    .map(|queued| queued.payload.get_tx_details())
                    .expect(
                        "xrpl::TxQ::getTxs : fee-order entry must reference a queued transaction",
                    )
            })
            .collect()
    }

    pub fn get_tx_required_fee_and_seq<Lock, View, TxSource>(
        &self,
        _lock: &mut Lock,
        view: &View,
        tx: &TxSource,
    ) -> QueueTxQRequiredFeeAndSeq
    where
        Lock: QueueAcceptLockScope,
        View: QueueTxQRequiredFeeViewSource<Account, TxSource>,
        TxSource: QueueTxQRequiredFeeTxSource<Account>,
    {
        let snapshot = self.metrics.snapshot();
        let required_fee_level = tx::evaluate_required_fee_level(
            snapshot,
            view.open_ledger_tx_count(),
            ApplyFlags::NONE,
        );
        let base_fee_drops = view.calculate_base_fee_drops(tx);
        let account_seq = view.account_sequence(tx.account()).unwrap_or(0);
        let available_seq = if account_seq == 0 {
            0
        } else {
            self.views
                .accounts
                .get(tx.account())
                .filter(|queued_account| !queued_account.transactions.is_empty())
                .map_or(account_seq, |queued_account| {
                    queued_account
                        .next_queuable_seq(protocol::SeqProxy::sequence(account_seq))
                        .value()
                })
        };

        QueueTxQRequiredFeeAndSeq {
            required_fee_drops: mul_div(required_fee_level, base_fee_drops, tx::TXQ_BASE_LEVEL)
                .and_then(|fee| i64::try_from(fee).ok())
                .unwrap_or(i64::MAX),
            account_seq,
            available_seq,
        }
    }

    pub fn get_metrics<Lock, View>(&self, _lock: &mut Lock, view: &View) -> QueueTxQMetrics
    where
        Lock: QueueAcceptLockScope,
        View: tx::QueueTxQMetricsView,
    {
        self.build_metrics(view.open_ledger_tx_count())
    }

    pub fn get_rpc_fee_report<Lock, View>(&self, _lock: &mut Lock, view: &View) -> QueueTxQRpcReport
    where
        Lock: QueueAcceptLockScope,
        View: QueueTxQRpcView,
    {
        let metrics = self.build_metrics(view.open_ledger_tx_count());
        let base_fee_drops = view.base_fee_drops();
        let effective_base_fee_drops = if base_fee_drops == 0
            && metrics.open_ledger_fee_level != metrics.reference_fee_level
        {
            1
        } else {
            base_fee_drops
        };
        let minimum_fee_base_drops = if metrics
            .tx_q_max_size
            .is_some_and(|max_size| metrics.tx_count >= max_size)
        {
            effective_base_fee_drops
        } else {
            base_fee_drops
        };
        let mut open_ledger_fee = mul_div(
            metrics.open_ledger_fee_level,
            effective_base_fee_drops,
            tx::TXQ_BASE_LEVEL,
        )
        .unwrap_or(u64::MAX);

        if effective_base_fee_drops != 0
            && mul_div(
                open_ledger_fee,
                tx::TXQ_BASE_LEVEL,
                effective_base_fee_drops,
            )
            .unwrap_or(tx::FeeLevel64::MAX)
                < metrics.open_ledger_fee_level
        {
            open_ledger_fee = open_ledger_fee.saturating_add(1);
        }

        QueueTxQRpcReport {
            ledger_current_index: view.ledger_current_index(),
            expected_ledger_size: metrics.tx_per_ledger.to_string(),
            current_ledger_size: metrics.tx_in_ledger.to_string(),
            current_queue_size: metrics.tx_count.to_string(),
            max_queue_size: metrics.tx_q_max_size.map(|size| size.to_string()),
            levels: tx::QueueTxQRpcLevels {
                reference_level: metrics.reference_fee_level.to_string(),
                minimum_level: metrics.min_processing_fee_level.to_string(),
                median_level: metrics.med_fee_level.to_string(),
                open_ledger_level: metrics.open_ledger_fee_level.to_string(),
            },
            drops: tx::QueueTxQRpcDrops {
                base_fee: base_fee_drops.to_string(),
                median_fee: mul_div(metrics.med_fee_level, base_fee_drops, tx::TXQ_BASE_LEVEL)
                    .unwrap_or(u64::MAX)
                    .to_string(),
                minimum_fee: mul_div(
                    metrics.min_processing_fee_level,
                    minimum_fee_base_drops,
                    tx::TXQ_BASE_LEVEL,
                )
                .unwrap_or(u64::MAX)
                .to_string(),
                open_ledger_fee: open_ledger_fee.to_string(),
            },
        }
    }

    pub fn get_rpc_fee_report_from_app<Lock, App>(
        &self,
        lock: &mut Lock,
        app: &App,
    ) -> Option<QueueTxQRpcReport>
    where
        Lock: QueueAcceptLockScope,
        App: QueueTxQRpcAppSource,
    {
        app.current_rpc_view()
            .map(|view| self.get_rpc_fee_report(lock, &view))
    }

    pub fn do_rpc<Lock, App>(&self, lock: &mut Lock, app: &App) -> Option<QueueTxQRpcReport>
    where
        Lock: QueueAcceptLockScope,
        App: QueueTxQRpcAppSource,
    {
        self.get_rpc_fee_report_from_app(lock, app)
    }

    pub fn process_closed_ledger<Lock, App, View>(
        &mut self,
        lock: &mut Lock,
        app: &App,
        view: &View,
        time_leap: bool,
    ) -> ClosedLedgerMaintenanceWithMetrics<Account>
    where
        Lock: QueueAcceptLockScope,
        App: QueueTxQClosedLedgerAppSource<View>,
        View: QueueTxQClosedLedgerView,
    {
        self.with_accept_txq(|txq| txq.process_closed_ledger(lock, app, view, time_leap))
    }

    pub fn metrics_snapshot(&self) -> QueueFeeMetricsSnapshot {
        self.metrics.snapshot()
    }

    pub fn apply_with_app_view<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_apply_txq(|txq| {
            txq.apply_with_app_view(
                lock,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_log_messages<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_apply_txq(|txq| {
            txq.apply_with_log_messages(
                lock,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_apply_journal_txq(|txq| {
            txq.apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission(
                lock, app, view, tx_source,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_apply_txq(|txq| {
            txq.apply_after_preflight_with_app_view(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_log_messages<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_apply_txq(|txq| {
            txq.apply_after_preflight_with_log_messages(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }
}
