//! `TxQ`-shaped method facade for the current landed public `xrpld` `TxQ`
//! boundaries.
//!
//! This layer is intentionally thin:
//! - one Rust-side owner object carries the landed accept state, journal sink,
//!   and explicit lock-scope boundary,
//! - callers still supply a lock token,
//! - callers still supply the live app/runtime and ledger-view inputs,
//! - the facade stays focused on the currently landed public member surfaces.

use std::fmt::Display;

use basics::mul_div::mul_div;
use protocol::SeqProxy;

use crate::{
    ApplyFlags, ClosedLedgerCandidate, ClosedLedgerMaintenanceWithMetrics, FeeLevel64,
    QueueAcceptEntryResult, QueueAcceptLedgerViewSource, QueueAcceptLiveApplyRuntime,
    QueueAcceptLockScope, QueueAcceptLockScopeOwner, QueueAcceptPreparedCallStep, TXQ_BASE_LEVEL,
    TxDetails, evaluate_required_fee_level,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueTxQAccountState<'a, Account> {
    Missing,
    Present {
        account: &'a Account,
        seq_proxy: SeqProxy,
    },
}

pub trait QueueTxQMetricsView {
    fn open_ledger_tx_count(&self) -> usize;
}

pub trait QueueTxQRpcView {
    fn ledger_current_index(&self) -> u32;
    fn open_ledger_tx_count(&self) -> usize;
    fn base_fee_drops(&self) -> u64;
}

pub trait QueueTxQRpcAppSource {
    type View: QueueTxQRpcView;

    fn current_rpc_view(&self) -> Option<Self::View>;
}

pub trait QueueTxQClosedLedgerView {
    fn ledger_seq(&self) -> u32;
}

pub trait QueueTxQClosedLedgerAppSource<View> {
    fn validated_fee_levels(&self, view: &View) -> Vec<FeeLevel64>;
}

pub trait QueueTxQRequiredFeeTxSource<Account> {
    fn account(&self) -> &Account;
}

pub trait QueueTxQRequiredFeeViewSource<Account, Tx> {
    fn open_ledger_tx_count(&self) -> usize;
    fn calculate_base_fee_drops(&self, tx: &Tx) -> u64;
    fn account_sequence(&self, account: &Account) -> Option<u32>;
}

impl<T> QueueTxQMetricsView for T
where
    T: QueueAcceptLedgerViewSource + ?Sized,
{
    fn open_ledger_tx_count(&self) -> usize {
        QueueAcceptLedgerViewSource::open_ledger_tx_count(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueTxQMetrics {
    pub tx_count: usize,
    pub tx_q_max_size: Option<usize>,
    pub tx_in_ledger: usize,
    pub tx_per_ledger: usize,
    pub reference_fee_level: FeeLevel64,
    pub min_processing_fee_level: FeeLevel64,
    pub med_fee_level: FeeLevel64,
    pub open_ledger_fee_level: FeeLevel64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueTxQRequiredFeeAndSeq {
    pub required_fee_drops: i64,
    pub account_seq: u32,
    pub available_seq: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueTxQRpcLevels {
    pub reference_level: String,
    pub minimum_level: String,
    pub median_level: String,
    pub open_ledger_level: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueTxQRpcDrops {
    pub base_fee: String,
    pub median_fee: String,
    pub minimum_fee: String,
    pub open_ledger_fee: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueTxQRpcReport {
    pub ledger_current_index: u32,
    pub expected_ledger_size: String,
    pub current_ledger_size: String,
    pub current_queue_size: String,
    pub max_queue_size: Option<String>,
    pub levels: QueueTxQRpcLevels,
    pub drops: QueueTxQRpcDrops,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptTxQ<Account, Tx, Journal, ParentBatchId, Sink> {
    owner: QueueAcceptLockScopeOwner<Account, Tx, Journal, ParentBatchId, Sink>,
}

impl<Account, Tx, Journal, ParentBatchId, Sink>
    QueueAcceptTxQ<Account, Tx, Journal, ParentBatchId, Sink>
{
    pub const fn new(
        owner: QueueAcceptLockScopeOwner<Account, Tx, Journal, ParentBatchId, Sink>,
    ) -> Self {
        Self { owner }
    }

    pub fn lock_scope_owner(
        &self,
    ) -> &QueueAcceptLockScopeOwner<Account, Tx, Journal, ParentBatchId, Sink> {
        &self.owner
    }

    pub fn lock_scope_owner_mut(
        &mut self,
    ) -> &mut QueueAcceptLockScopeOwner<Account, Tx, Journal, ParentBatchId, Sink> {
        &mut self.owner
    }

    fn live_owner(&self) -> &crate::QueueAcceptLiveOwner<Account, Tx, Journal, ParentBatchId> {
        self.owner.owner().owner().owner()
    }

    fn live_owner_mut(
        &mut self,
    ) -> &mut crate::QueueAcceptLiveOwner<Account, Tx, Journal, ParentBatchId> {
        self.owner.owner_mut().owner_mut().owner_mut()
    }

    fn available_seq_for_account(&self, account: &Account, account_seq: u32) -> u32
    where
        Account: Ord,
    {
        self.live_owner()
            .views()
            .accounts
            .get(account)
            .filter(|queued_account| !queued_account.transactions.is_empty())
            .map_or(account_seq, |queued_account| {
                queued_account
                    .next_queuable_seq(SeqProxy::sequence(account_seq))
                    .value()
            })
    }

    fn build_metrics(&self, tx_in_ledger: usize) -> QueueTxQMetrics {
        let live_owner = self.live_owner();
        let snapshot = live_owner.metrics().snapshot();
        let fee_order = &live_owner.views().fee_order;
        let is_full = live_owner
            .current_max_size()
            .is_some_and(|max_size| fee_order.len() >= max_size);

        QueueTxQMetrics {
            tx_count: fee_order.len(),
            tx_q_max_size: live_owner.current_max_size(),
            tx_in_ledger,
            tx_per_ledger: snapshot.txns_expected,
            reference_fee_level: TXQ_BASE_LEVEL,
            min_processing_fee_level: if is_full {
                fee_order
                    .last()
                    .map(|entry| entry.candidate.fee_level.wrapping_add(1))
                    .expect("xrpl::TxQ::getMetrics : full queue must have a trailing fee entry")
            } else {
                TXQ_BASE_LEVEL
            },
            med_fee_level: snapshot.escalation_multiplier,
            open_ledger_fee_level: evaluate_required_fee_level(
                snapshot,
                tx_in_ledger,
                ApplyFlags::NONE,
            ),
        }
    }
}

impl<Account, Tx, Journal, ParentBatchId, Sink>
    QueueAcceptTxQ<Account, Tx, Journal, ParentBatchId, Sink>
where
    Account: Clone + Display + Ord + PartialEq,
    Sink: crate::QueueAcceptJournalSink,
{
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
        self.owner.accept(lock, app, view)
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
        self.owner.prepare_accept(lock, view)
    }

    pub fn next_queuable_seq<Lock>(
        &self,
        _lock: &mut Lock,
        account_state: QueueTxQAccountState<'_, Account>,
    ) -> SeqProxy
    where
        Lock: QueueAcceptLockScope,
    {
        let QueueTxQAccountState::Present { account, seq_proxy } = account_state else {
            return SeqProxy::sequence(0);
        };

        self.live_owner()
            .views()
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
        Tx: Clone,
    {
        self.live_owner()
            .views()
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
        Tx: Clone,
    {
        let views = self.live_owner().views();

        views
            .fee_order
            .iter()
            .map(|entry| {
                views
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
        let snapshot = self.live_owner().metrics().snapshot();
        let required_fee_level =
            evaluate_required_fee_level(snapshot, view.open_ledger_tx_count(), ApplyFlags::NONE);
        let base_fee_drops = view.calculate_base_fee_drops(tx);
        let account_seq = view.account_sequence(tx.account()).unwrap_or(0);
        let available_seq = if account_seq == 0 {
            0
        } else {
            self.available_seq_for_account(tx.account(), account_seq)
        };

        QueueTxQRequiredFeeAndSeq {
            required_fee_drops: mul_div(required_fee_level, base_fee_drops, TXQ_BASE_LEVEL)
                .and_then(|fee| i64::try_from(fee).ok())
                .unwrap_or(i64::MAX),
            account_seq,
            available_seq,
        }
    }

    pub fn get_metrics<Lock, View>(&self, _lock: &mut Lock, view: &View) -> QueueTxQMetrics
    where
        Lock: QueueAcceptLockScope,
        View: QueueTxQMetricsView,
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
            TXQ_BASE_LEVEL,
        )
        .unwrap_or(u64::MAX);

        if effective_base_fee_drops != 0
            && mul_div(open_ledger_fee, TXQ_BASE_LEVEL, effective_base_fee_drops)
                .unwrap_or(FeeLevel64::MAX)
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
            levels: QueueTxQRpcLevels {
                reference_level: metrics.reference_fee_level.to_string(),
                minimum_level: metrics.min_processing_fee_level.to_string(),
                median_level: metrics.med_fee_level.to_string(),
                open_ledger_level: metrics.open_ledger_fee_level.to_string(),
            },
            drops: QueueTxQRpcDrops {
                base_fee: base_fee_drops.to_string(),
                median_fee: mul_div(metrics.med_fee_level, base_fee_drops, TXQ_BASE_LEVEL)
                    .unwrap_or(u64::MAX)
                    .to_string(),
                minimum_fee: mul_div(
                    metrics.min_processing_fee_level,
                    minimum_fee_base_drops,
                    TXQ_BASE_LEVEL,
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
        _lock: &mut Lock,
        app: &App,
        view: &View,
        time_leap: bool,
    ) -> ClosedLedgerMaintenanceWithMetrics<Account>
    where
        Lock: QueueAcceptLockScope,
        App: QueueTxQClosedLedgerAppSource<View>,
        View: QueueTxQClosedLedgerView,
    {
        let validated_fee_levels = app.validated_fee_levels(view);
        let ledger_seq = view.ledger_seq();
        let by_fee_candidates = self
            .live_owner()
            .views()
            .fee_order
            .iter()
            .map(|entry| {
                let last_valid = self
                    .live_owner()
                    .views()
                    .accounts
                    .get(&entry.key.account)
                    .and_then(|account| account.transactions.get(&entry.key.seq_proxy))
                    .map(|queued| queued.payload.last_valid)
                    .expect("xrpl::TxQ::processClosedLedger : candidate found in account");

                ClosedLedgerCandidate {
                    account: entry.key.account.clone(),
                    seq_proxy: entry.key.seq_proxy,
                    last_valid,
                }
            })
            .collect::<Vec<_>>();

        self.live_owner_mut().process_closed_ledger(
            &validated_fee_levels,
            by_fee_candidates,
            ledger_seq,
            time_leap,
        )
    }
}
