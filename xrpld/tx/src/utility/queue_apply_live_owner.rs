//! Live-owner wrapper for the current `xrpld` `TxQ::apply(...)` boundary.
//!
//! This layer moves the current TxQ-owned apply state into one Rust owner:
//! - fee metrics,
//! - queue admission policy values,
//! - current queue max size,
//! - the fee-order tie-break comparator,
//! - synchronized queue/account views.

use std::fmt::Display;

use protocol::{SeqProxy, Ter};

use crate::{
    ApplyFlags, MISSING_ACCOUNT_SEQ_PROXY, MaybeTx, OrderCandidates, PreclaimResult,
    PreflightResult, QueueApplyAccountStage, QueueApplyAfterPreflightSourceInputs,
    QueueApplyFeeContext, QueueApplyFeeContextInputs, QueueApplyMultiTxnInputs,
    QueueApplyMultiTxnStage, QueueApplyObservedAccountLookup, QueueApplyObservedQueue,
    QueueApplyObservedTxSource, QueueApplyObservedViewSource, QueueApplyPreclaimStage,
    QueueApplyPreclaimViewSource, QueueApplyPreflightStage, QueueApplyPreparedPostPreclaimInputs,
    QueueApplyPreparedPreclaimInputs, QueueApplyPreparedQueuedFlowStage,
    QueueApplyQueuedWithFeeContextInputs, QueueApplyTopFromSourcesInputs,
    QueueApplyTopWithLogMessagesResult, QueueApplyTryClearResult, QueueApplyViewAdjustment,
    QueueFeeMetricsState, QueueHoldPreflight, QueueViews, TxConsequences, TxQSetup,
    check_hold_admission, derive_queue_apply_prepared_flow_stage,
    derive_queue_apply_prepared_post_preclaim_inputs, evaluate_queue_apply_fee_context,
    run_queue_apply_account_stage, run_queue_apply_after_preflight_from_sources,
    run_queue_apply_after_preflight_with_caller_preclaim_and_log_sinks_from_sources,
    run_queue_apply_after_preflight_with_caller_preclaim_from_sources,
    run_queue_apply_after_preflight_with_log_messages_from_sources,
    run_queue_apply_after_preflight_with_log_sinks_from_sources, run_queue_apply_multitxn_stage,
    run_queue_apply_top_with_caller_preclaim_and_log_sinks_from_sources,
    run_queue_apply_top_with_caller_preclaim_from_sources,
    run_queue_apply_top_with_log_messages_from_sources,
    run_queue_apply_top_with_log_sinks_from_sources,
    run_queue_apply_top_with_queued_stage_from_sources,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueApplyLiveOwnerQueuedFrontStage<Account> {
    Account {
        account_seq_proxy: SeqProxy,
        fee_context: QueueApplyFeeContext,
        stage: QueueApplyAccountStage<Account>,
    },
    MultiTxn {
        account_seq_proxy: SeqProxy,
        fee_context: QueueApplyFeeContext,
        stage: QueueApplyMultiTxnStage,
    },
}

fn derive_after_preflight_prepared_flow_stage_from_live_owner<
    'a,
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxSource,
    ViewSource,
    PrepareMultiTxn,
>(
    views: &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    order: &'a OrderCandidates,
    minimum_last_ledger_buffer: u32,
    maximum_txn_per_account: usize,
    retry_sequence_percent: u32,
    current_max_size: Option<usize>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    hold_preflight: QueueHoldPreflight,
    preflight_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
) -> QueueApplyPreparedQueuedFlowStage<'a, Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxSource: QueueApplyObservedTxSource<Account = Account>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
{
    let account_lookup = view_source.account_lookup(tx_source.account());
    let account_seq_proxy = match account_lookup {
        QueueApplyObservedAccountLookup::Missing => MISSING_ACCOUNT_SEQ_PROXY,
        QueueApplyObservedAccountLookup::Present { sequence, .. } => SeqProxy::sequence(sequence),
    };
    let balance_drops = match account_lookup {
        QueueApplyObservedAccountLookup::Missing => 0,
        QueueApplyObservedAccountLookup::Present { balance_drops, .. } => balance_drops,
    };
    let fee_context = evaluate_queue_apply_fee_context(QueueApplyFeeContextInputs {
        calculated_base_fee_drops: view_source.calculated_base_fee_drops(),
        fee_paid_drops: view_source.fee_paid_drops(),
        default_base_fee_drops: view_source.default_base_fee_drops(),
        metrics_snapshot: view_source.metrics_snapshot(),
        open_ledger_tx_count: view_source.open_ledger_tx_count(),
        flags: preflight_result.flags,
    });
    let tx_q_account = views.accounts.get(tx_source.account());
    let can_be_held_result = check_hold_admission(
        hold_preflight,
        view_source.open_ledger_seq(),
        minimum_last_ledger_buffer,
        tx_q_account,
        maximum_txn_per_account,
        tx_q_account.is_some_and(|queued_account| {
            queued_account
                .transactions
                .contains_key(&tx_source.tx_seq_proxy())
        }),
        tx_source.tx_seq_proxy(),
        account_seq_proxy,
    );

    derive_queue_apply_prepared_flow_stage(
        views,
        account_seq_proxy,
        tx_source.tx_seq_proxy(),
        QueueApplyQueuedWithFeeContextInputs {
            account: tx_source.account().clone(),
            preflight: hold_preflight,
            is_blocker: preflight_result.consequences.is_blocker(),
            open_ledger_seq: view_source.open_ledger_seq(),
            minimum_last_ledger_buffer,
            maximum_txn_per_account,
            retry_sequence_percent,
            queue_is_full: current_max_size
                .is_some_and(|max_size| views.fee_order.len() >= max_size),
            balance_drops,
            reserve_drops: view_source.reserve_drops(),
            base_fee_drops: view_source.base_fee_drops(),
            can_be_held_result,
            open_ledger_tx_count: view_source.open_ledger_tx_count(),
            tx_id: tx_source.tx_id(),
            last_valid: hold_preflight.last_valid_ledger,
            flags: preflight_result.flags,
            order,
        },
        fee_context,
        preflight_result,
        prepare_multitxn,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId> {
    metrics: QueueFeeMetricsState,
    minimum_last_ledger_buffer: u32,
    maximum_txn_per_account: usize,
    retry_sequence_percent: u32,
    current_max_size: Option<usize>,
    order: OrderCandidates,
    views: QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
}

impl<Account, Tx, Journal, ParentBatchId> QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId> {
    pub fn new(
        minimum_last_ledger_buffer: u32,
        maximum_txn_per_account: usize,
        retry_sequence_percent: u32,
        current_max_size: Option<usize>,
        order: OrderCandidates,
        views: QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> Self {
        Self::new_with_metrics(
            TxQSetup::default().fee_metrics_state(),
            minimum_last_ledger_buffer,
            maximum_txn_per_account,
            retry_sequence_percent,
            current_max_size,
            order,
            views,
        )
    }

    pub fn new_with_metrics(
        metrics: QueueFeeMetricsState,
        minimum_last_ledger_buffer: u32,
        maximum_txn_per_account: usize,
        retry_sequence_percent: u32,
        current_max_size: Option<usize>,
        order: OrderCandidates,
        views: QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> Self {
        Self {
            metrics,
            minimum_last_ledger_buffer,
            maximum_txn_per_account,
            retry_sequence_percent,
            current_max_size,
            order,
            views,
        }
    }

    pub fn new_from_setup(
        setup: &TxQSetup,
        current_max_size: Option<usize>,
        order: OrderCandidates,
        views: QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> Self {
        Self::new_with_metrics(
            setup.fee_metrics_state(),
            setup.minimum_last_ledger_buffer,
            usize::try_from(setup.maximum_txn_per_account).unwrap_or(usize::MAX),
            setup.retry_sequence_percent,
            current_max_size,
            order,
            views,
        )
    }

    pub fn metrics(&self) -> &QueueFeeMetricsState {
        &self.metrics
    }

    pub fn metrics_mut(&mut self) -> &mut QueueFeeMetricsState {
        &mut self.metrics
    }

    pub const fn minimum_last_ledger_buffer(&self) -> u32 {
        self.minimum_last_ledger_buffer
    }

    pub const fn maximum_txn_per_account(&self) -> usize {
        self.maximum_txn_per_account
    }

    pub const fn retry_sequence_percent(&self) -> u32 {
        self.retry_sequence_percent
    }

    pub const fn current_max_size(&self) -> Option<usize> {
        self.current_max_size
    }

    pub const fn order(&self) -> &OrderCandidates {
        &self.order
    }

    pub fn views(&self) -> &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>> {
        &self.views
    }

    pub fn views_mut(
        &mut self,
    ) -> &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>> {
        &mut self.views
    }

    pub fn observed_queue(&self, can_be_held_result: Ter) -> QueueApplyObservedQueue<'_> {
        QueueApplyObservedQueue {
            minimum_last_ledger_buffer: self.minimum_last_ledger_buffer,
            maximum_txn_per_account: self.maximum_txn_per_account,
            retry_sequence_percent: self.retry_sequence_percent,
            queue_is_full: self
                .current_max_size
                .is_some_and(|max_size| self.views.fee_order.len() >= max_size),
            can_be_held_result,
            order: &self.order,
        }
    }

    fn with_top_from_sources_inputs<R, Build>(
        &mut self,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        build: Build,
    ) -> R
    where
        Build: FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            QueueApplyTopFromSourcesInputs<'_>,
        ) -> R,
    {
        let order = self.order;
        let queue = QueueApplyObservedQueue {
            minimum_last_ledger_buffer: self.minimum_last_ledger_buffer,
            maximum_txn_per_account: self.maximum_txn_per_account,
            retry_sequence_percent: self.retry_sequence_percent,
            queue_is_full: self
                .current_max_size
                .is_some_and(|max_size| self.views.fee_order.len() >= max_size),
            can_be_held_result,
            order: &order,
        };

        build(
            &mut self.views,
            QueueApplyTopFromSourcesInputs::new(hold_preflight, flags, consequences, queue),
        )
    }

    fn with_after_preflight_source_inputs<R, Build>(
        &mut self,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        build: Build,
    ) -> R
    where
        Build: FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            QueueApplyAfterPreflightSourceInputs<'_>,
        ) -> R,
    {
        let order = self.order;
        let queue = QueueApplyObservedQueue {
            minimum_last_ledger_buffer: self.minimum_last_ledger_buffer,
            maximum_txn_per_account: self.maximum_txn_per_account,
            retry_sequence_percent: self.retry_sequence_percent,
            queue_is_full: self
                .current_max_size
                .is_some_and(|max_size| self.views.fee_order.len() >= max_size),
            can_be_held_result,
            order: &order,
        };

        build(
            &mut self.views,
            QueueApplyAfterPreflightSourceInputs::new(hold_preflight, queue),
        )
    }

    fn with_top_from_sources<TxSource, ViewSource, R, Run>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        run: Run,
    ) -> R
    where
        Run: FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            &TxSource,
            &ViewSource,
            QueueApplyTopFromSourcesInputs<'_>,
        ) -> R,
    {
        self.with_top_from_sources_inputs(
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            |views, inputs| run(views, tx_source, view_source, inputs),
        )
    }

    fn with_top_from_sources_caller_preclaim<TxSource, ViewSource, R, Run>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        run: Run,
    ) -> R
    where
        Run: FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            &TxSource,
            &ViewSource,
            QueueApplyTopFromSourcesInputs<'_>,
        ) -> R,
    {
        self.with_top_from_sources_inputs(
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            |views, inputs| run(views, tx_source, view_source, inputs),
        )
    }

    fn with_top_from_sources_caller_preclaim_log_sinks<TxSource, ViewSource, R, Run>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        run: Run,
    ) -> R
    where
        Run: FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            &TxSource,
            &ViewSource,
            QueueApplyTopFromSourcesInputs<'_>,
        ) -> R,
    {
        self.with_top_from_sources_caller_preclaim(
            tx_source,
            view_source,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            run,
        )
    }

    fn with_top_from_sources_log_sinks<TxSource, ViewSource, R, Run>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        run: Run,
    ) -> R
    where
        Run: FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            &TxSource,
            &ViewSource,
            QueueApplyTopFromSourcesInputs<'_>,
        ) -> R,
    {
        self.with_top_from_sources(
            tx_source,
            view_source,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            run,
        )
    }

    fn with_top_from_sources_log_messages<TxSource, ViewSource, R, Run>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        run: Run,
    ) -> R
    where
        Run: FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            &TxSource,
            &ViewSource,
            QueueApplyTopFromSourcesInputs<'_>,
        ) -> R,
    {
        self.with_top_from_sources(
            tx_source,
            view_source,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            run,
        )
    }

    fn with_after_preflight_from_sources<TxSource, ViewSource, R, Run>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run: Run,
    ) -> R
    where
        Run: FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            &TxSource,
            &ViewSource,
            QueueApplyAfterPreflightSourceInputs<'_>,
        ) -> R,
    {
        self.with_after_preflight_source_inputs(
            hold_preflight,
            can_be_held_result,
            |views, inputs| run(views, tx_source, view_source, inputs),
        )
    }

    fn with_after_preflight_from_sources_caller_preclaim<TxSource, ViewSource, R, Run>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run: Run,
    ) -> R
    where
        Run: FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            &TxSource,
            &ViewSource,
            QueueApplyAfterPreflightSourceInputs<'_>,
        ) -> R,
    {
        self.with_after_preflight_source_inputs(
            hold_preflight,
            can_be_held_result,
            |views, inputs| run(views, tx_source, view_source, inputs),
        )
    }

    fn with_after_preflight_from_sources_caller_preclaim_log_sinks<TxSource, ViewSource, R, Run>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run: Run,
    ) -> R
    where
        Run: FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            &TxSource,
            &ViewSource,
            QueueApplyAfterPreflightSourceInputs<'_>,
        ) -> R,
    {
        self.with_after_preflight_from_sources_caller_preclaim(
            tx_source,
            view_source,
            hold_preflight,
            can_be_held_result,
            run,
        )
    }

    fn with_after_preflight_from_sources_log_sinks<TxSource, ViewSource, R, Run>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run: Run,
    ) -> R
    where
        Run: FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            &TxSource,
            &ViewSource,
            QueueApplyAfterPreflightSourceInputs<'_>,
        ) -> R,
    {
        self.with_after_preflight_from_sources(
            tx_source,
            view_source,
            hold_preflight,
            can_be_held_result,
            run,
        )
    }

    fn with_after_preflight_from_sources_log_messages<TxSource, ViewSource, R, Run>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run: Run,
    ) -> R
    where
        Run: FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            &TxSource,
            &ViewSource,
            QueueApplyAfterPreflightSourceInputs<'_>,
        ) -> R,
    {
        self.with_after_preflight_from_sources(
            tx_source,
            view_source,
            hold_preflight,
            can_be_held_result,
            run,
        )
    }

    fn account_lookup_for_view<TxSource, ViewSource>(
        &self,
        tx_source: &TxSource,
        view_source: &ViewSource,
    ) -> QueueApplyObservedAccountLookup
    where
        Account: Ord,
        TxSource: QueueApplyObservedTxSource<Account = Account>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        view_source.account_lookup(tx_source.account())
    }

    fn account_seq_proxy_from_lookup(account_lookup: QueueApplyObservedAccountLookup) -> SeqProxy {
        match account_lookup {
            QueueApplyObservedAccountLookup::Missing => MISSING_ACCOUNT_SEQ_PROXY,
            QueueApplyObservedAccountLookup::Present { sequence, .. } => {
                SeqProxy::sequence(sequence)
            }
        }
    }

    fn balance_drops_from_lookup(account_lookup: QueueApplyObservedAccountLookup) -> u64 {
        match account_lookup {
            QueueApplyObservedAccountLookup::Missing => 0,
            QueueApplyObservedAccountLookup::Present { balance_drops, .. } => balance_drops,
        }
    }

    fn fee_context_for_view<ViewSource>(
        &self,
        view_source: &ViewSource,
        flags: ApplyFlags,
    ) -> QueueApplyFeeContext
    where
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        evaluate_queue_apply_fee_context(QueueApplyFeeContextInputs {
            calculated_base_fee_drops: view_source.calculated_base_fee_drops(),
            fee_paid_drops: view_source.fee_paid_drops(),
            default_base_fee_drops: view_source.default_base_fee_drops(),
            metrics_snapshot: view_source.metrics_snapshot(),
            open_ledger_tx_count: view_source.open_ledger_tx_count(),
            flags,
        })
    }

    pub fn derive_can_be_held_result<TxSource, ViewSource>(
        &self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
    ) -> Ter
    where
        Account: Ord,
        TxSource: QueueApplyObservedTxSource<Account = Account>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        let account_lookup = self.account_lookup_for_view(tx_source, view_source);
        let tx_seq_proxy = tx_source.tx_seq_proxy();
        let tx_q_account = self.views.accounts.get(tx_source.account());

        check_hold_admission(
            hold_preflight,
            view_source.open_ledger_seq(),
            self.minimum_last_ledger_buffer,
            tx_q_account,
            self.maximum_txn_per_account,
            tx_q_account.is_some_and(|queued_account| {
                queued_account.transactions.contains_key(&tx_seq_proxy)
            }),
            tx_seq_proxy,
            Self::account_seq_proxy_from_lookup(account_lookup),
        )
    }

    pub fn derive_queued_front_stage<TxSource, ViewSource>(
        &self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
    ) -> QueueApplyLiveOwnerQueuedFrontStage<Account>
    where
        Account: Clone + Display + Ord + PartialEq,
        TxSource: QueueApplyObservedTxSource<Account = Account>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        let account_lookup = self.account_lookup_for_view(tx_source, view_source);
        let account_seq_proxy = Self::account_seq_proxy_from_lookup(account_lookup);
        let fee_context = self.fee_context_for_view(view_source, flags);
        let account_stage = run_queue_apply_account_stage(
            &self.views,
            tx_source.account(),
            account_seq_proxy,
            tx_source.tx_seq_proxy(),
            consequences.is_blocker(),
            fee_context.fee_level_paid,
            self.retry_sequence_percent,
        );

        let QueueApplyAccountStage::Ready(account_context) = &account_stage else {
            return QueueApplyLiveOwnerQueuedFrontStage::Account {
                account_seq_proxy,
                fee_context,
                stage: account_stage,
            };
        };

        let can_be_held_result =
            self.derive_can_be_held_result(tx_source, view_source, hold_preflight);
        let tx_q_account = self.views.accounts.get(tx_source.account());
        let stage = run_queue_apply_multitxn_stage(
            tx_q_account,
            account_context,
            QueueApplyMultiTxnInputs {
                preflight: hold_preflight,
                open_ledger_seq: view_source.open_ledger_seq(),
                minimum_last_ledger_buffer: self.minimum_last_ledger_buffer,
                maximum_txn_per_account: self.maximum_txn_per_account,
                account_seq_proxy,
                tx_seq_proxy: tx_source.tx_seq_proxy(),
                balance_drops: Self::balance_drops_from_lookup(account_lookup),
                reserve_drops: view_source.reserve_drops(),
                base_fee_drops: view_source.base_fee_drops(),
                can_be_held_result,
                consequences,
            },
        );

        QueueApplyLiveOwnerQueuedFrontStage::MultiTxn {
            account_seq_proxy,
            fee_context,
            stage,
        }
    }

    pub fn derive_after_preflight_prepared_flow_stage<'a, TxSource, ViewSource, PrepareMultiTxn>(
        &'a self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        preflight_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        prepare_multitxn: PrepareMultiTxn,
    ) -> QueueApplyPreparedQueuedFlowStage<'a, Account, Tx, Journal, ParentBatchId>
    where
        Account: Clone + Display + Ord + PartialEq,
        TxSource: QueueApplyObservedTxSource<Account = Account>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    {
        derive_after_preflight_prepared_flow_stage_from_live_owner(
            self.views(),
            &self.order,
            self.minimum_last_ledger_buffer,
            self.maximum_txn_per_account,
            self.retry_sequence_percent,
            self.current_max_size,
            tx_source,
            view_source,
            hold_preflight,
            preflight_result,
            prepare_multitxn,
        )
    }

    pub fn derive_prepared_post_preclaim_inputs<'a>(
        prepared_flow: QueueApplyPreparedQueuedFlowStage<'a, Account, Tx, Journal, ParentBatchId>,
        preclaim: QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
    ) -> Option<QueueApplyPreparedPostPreclaimInputs<'a, Account, Tx, Journal, ParentBatchId>>
    where
        Account: Clone,
    {
        let QueueApplyPreparedQueuedFlowStage::Flow { prepared, .. } = prepared_flow else {
            return None;
        };

        Some(derive_queue_apply_prepared_post_preclaim_inputs(
            prepared.tx_seq_proxy,
            prepared.first_relevant_retries_remaining,
            prepared.fee_level_paid,
            prepared.base_level,
            prepared.required_fee_level,
            prepared.hold_fallback,
            prepared.full_queue_decision,
            prepared.replaced,
            prepared.account,
            prepared.tx_id,
            prepared.last_valid,
            prepared.flags,
            prepared.pf_result,
            prepared.order,
            preclaim,
        ))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_live_owner_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    owner: &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    owner.with_top_from_sources_log_sinks(
        tx_source,
        view_source,
        hold_preflight,
        flags,
        consequences,
        can_be_held_result,
        |views, tx_source, view_source, inputs| {
            run_queue_apply_top_with_queued_stage_from_sources(
                views,
                tx_source,
                view_source,
                inputs,
                run_preflight,
                trace,
                apply,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_live_owner_from_sources_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    owner: &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    owner.with_top_from_sources_log_messages(
        tx_source,
        view_source,
        hold_preflight,
        flags,
        consequences,
        can_be_held_result,
        |views, tx_source, view_source, inputs| {
            run_queue_apply_top_with_log_messages_from_sources(
                views,
                tx_source,
                view_source,
                inputs,
                run_preflight,
                trace,
                apply,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_live_owner_from_sources_and_caller_preclaim<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaimStage,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    owner: &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim_stage: RunPreclaimStage,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    owner.with_top_from_sources_caller_preclaim_log_sinks(
        tx_source,
        view_source,
        hold_preflight,
        flags,
        consequences,
        can_be_held_result,
        |views, tx_source, view_source, inputs| {
            run_queue_apply_top_with_caller_preclaim_from_sources(
                views,
                tx_source,
                view_source,
                inputs,
                run_preflight,
                trace,
                apply,
                prepare_multitxn,
                run_preclaim_stage,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_live_owner_from_sources_and_caller_preclaim_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    DebugFn,
    InfoFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaimStage,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    owner: &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    run_preflight: RunPreflight,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim_stage: RunPreclaimStage,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    owner.with_top_from_sources_caller_preclaim(
        tx_source,
        view_source,
        hold_preflight,
        flags,
        consequences,
        can_be_held_result,
        |views, tx_source, view_source, inputs| {
            run_queue_apply_top_with_caller_preclaim_and_log_sinks_from_sources(
                views,
                tx_source,
                view_source,
                inputs,
                run_preflight,
                trace,
                debug,
                info,
                apply,
                prepare_multitxn,
                run_preclaim_stage,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_live_owner_from_sources_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    DebugFn,
    InfoFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    owner: &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    run_preflight: RunPreflight,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    owner.with_top_from_sources(
        tx_source,
        view_source,
        hold_preflight,
        flags,
        consequences,
        can_be_held_result,
        |views, tx_source, view_source, inputs| {
            run_queue_apply_top_with_log_sinks_from_sources(
                views,
                tx_source,
                view_source,
                inputs,
                run_preflight,
                trace,
                debug,
                info,
                apply,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_live_owner_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    owner: &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    owner.with_after_preflight_from_sources_log_sinks(
        tx_source,
        view_source,
        hold_preflight,
        can_be_held_result,
        |views, tx_source, view_source, inputs| {
            run_queue_apply_after_preflight_from_sources(
                views,
                tx_source,
                view_source,
                preflight_result,
                inputs,
                trace,
                apply,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    owner: &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    owner.with_after_preflight_from_sources_log_messages(
        tx_source,
        view_source,
        hold_preflight,
        can_be_held_result,
        |views, tx_source, view_source, inputs| {
            run_queue_apply_after_preflight_with_log_messages_from_sources(
                views,
                tx_source,
                view_source,
                preflight_result,
                inputs,
                trace,
                apply,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaimStage,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    owner: &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim_stage: RunPreclaimStage,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    owner.with_after_preflight_from_sources_caller_preclaim_log_sinks(
        tx_source,
        view_source,
        hold_preflight,
        can_be_held_result,
        |views, tx_source, view_source, inputs| {
            run_queue_apply_after_preflight_with_caller_preclaim_from_sources(
                views,
                tx_source,
                view_source,
                preflight_result,
                inputs,
                trace,
                apply,
                prepare_multitxn,
                run_preclaim_stage,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    DebugFn,
    InfoFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaimStage,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    owner: &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim_stage: RunPreclaimStage,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    owner.with_after_preflight_from_sources_caller_preclaim(
        tx_source,
        view_source,
        hold_preflight,
        can_be_held_result,
        |views, tx_source, view_source, inputs| {
            run_queue_apply_after_preflight_with_caller_preclaim_and_log_sinks_from_sources(
                views,
                tx_source,
                view_source,
                preflight_result,
                inputs,
                trace,
                debug,
                info,
                apply,
                prepare_multitxn,
                run_preclaim_stage,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    DebugFn,
    InfoFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    owner: &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    owner.with_after_preflight_from_sources(
        tx_source,
        view_source,
        hold_preflight,
        can_be_held_result,
        |views, tx_source, view_source, inputs| {
            run_queue_apply_after_preflight_with_log_sinks_from_sources(
                views,
                tx_source,
                view_source,
                preflight_result,
                inputs,
                trace,
                debug,
                info,
                apply,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}
