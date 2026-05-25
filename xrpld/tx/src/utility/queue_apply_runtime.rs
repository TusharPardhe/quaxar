//! Top runtime plus `preflight(...)` orchestration in `TxQ::apply(...)`.
//!
//! This preserves the current top-level caller order:
//! 1. enter the transaction-apply runtime guard for the supplied rules,
//! 2. run `preflight(...)`,
//! 3. delegate the result into the landed preflight/entry carriers.

use protocol::{Rules, SeqProxy};

use crate::{
    ApplyFlags, PreflightResult, QueueApplyCallEnvelope, QueueApplyExecutionRuntime,
    QueueApplyOwnerShell, QueueApplyPreflightStage, QueueApplyQueuedStage,
    QueueApplyRuntimeEnvelope, QueueHoldPreflight, TxConsequences, run_queue_apply_preflight_stage,
    with_transaction_apply_runtime,
};

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_runtime<
    Account,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunPreflight,
    RunDirectApply,
    RunQueuedStage,
>(
    rules: &Rules,
    run_preflight: RunPreflight,
    account_exists: bool,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    ticket_exists: bool,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    RunPreflight: FnOnce() -> PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    RunDirectApply: FnOnce() -> Option<crate::DirectApplyExecution<Account, TxId>>,
    RunQueuedStage: FnOnce() -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    with_transaction_apply_runtime(rules, || {
        let preflight_result = run_preflight();
        run_queue_apply_preflight_stage(
            &preflight_result,
            account_exists,
            account_seq_proxy,
            tx_seq_proxy,
            ticket_exists,
            run_direct_apply,
            run_queued_stage,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_runtime_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    Runtime,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: protocol::Ter,
    runtime: &mut Runtime,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + std::fmt::Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + std::fmt::Display,
    TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: crate::QueueApplyObservedViewSource<Account>,
    Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    QueueApplyRuntimeEnvelope::new(runtime).apply_with_log_sinks(
        owner,
        call,
        hold_preflight,
        flags,
        consequences,
        can_be_held_result,
        trace,
        debug,
        info,
    )
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
    };

    use basics::number::{MantissaScale, get_mantissa_scale};
    use protocol::{
        Rules, SeqProxy, Ter, feature_universal_number, get_current_transaction_rules,
        get_st_number_switchover, set_current_transaction_rules, set_st_number_switchover,
    };

    use super::{run_queue_apply_with_runtime, run_queue_apply_with_runtime_and_log_sinks};
    use crate::{
        ApplyFlags, ApplyResult, DirectApplyAttemptResult, DirectApplyExecution, OrderCandidates,
        PreclaimResult, PreflightResult, QueueApplyCallEnvelope, QueueApplyEntryStage,
        QueueApplyExecutionRuntime, QueueApplyLiveOwner, QueueApplyObservedAccountLookup,
        QueueApplyObservedTicketLookup, QueueApplyObservedTxSource, QueueApplyObservedViewSource,
        QueueApplyOwnerShell, QueueApplyPreflightStage, QueueApplyRuntimeEnvelope,
        QueueApplyViewAdjustment, QueueFeeMetricsSnapshot, QueueHoldPreflight, QueueViews,
        TxConsequences,
    };
    use basics::base_uint::Uint256;

    #[derive(Debug)]
    struct TestTxSource<'a> {
        account: &'a String,
        transaction_id: &'static str,
        tx_id: Uint256,
        tx_seq_proxy: SeqProxy,
    }

    #[derive(Debug, Clone)]
    struct TestViewSource {
        rules: Rules,
        account_lookup: QueueApplyObservedAccountLookup,
        ticket_lookup: QueueApplyObservedTicketLookup,
        calculated_base_fee_drops: i64,
        fee_paid_drops: i64,
        default_base_fee_drops: i64,
        metrics_snapshot: QueueFeeMetricsSnapshot,
        open_ledger_tx_count: usize,
        open_ledger_seq: u32,
        reserve_drops: u64,
        base_fee_drops: u64,
    }

    #[derive(Debug, Default)]
    struct LoggingRuntime {
        preflight_calls: usize,
        direct_apply_calls: usize,
        prepare_multitxn_calls: usize,
        preclaim_calls: usize,
        try_clear_calls: usize,
        apply_sandbox_calls: usize,
        trace_messages: Vec<String>,
    }

    impl QueueApplyObservedTxSource for TestTxSource<'_> {
        type Account = String;
        type TransactionId = &'static str;

        fn account(&self) -> &Self::Account {
            self.account
        }

        fn transaction_id(&self) -> Self::TransactionId {
            self.transaction_id
        }

        fn tx_id(&self) -> Uint256 {
            self.tx_id
        }

        fn tx_seq_proxy(&self) -> SeqProxy {
            self.tx_seq_proxy
        }
    }

    impl crate::QueueApplyHoldPreflightTxSource for TestTxSource<'_> {
        fn has_previous_txn_id(&self) -> bool {
            false
        }

        fn has_account_txn_id(&self) -> bool {
            false
        }

        fn last_valid_ledger(&self) -> Option<u32> {
            Some(250)
        }
    }

    impl QueueApplyObservedViewSource<String> for TestViewSource {
        fn rules(&self) -> &Rules {
            &self.rules
        }

        fn account_lookup(&self, _account: &String) -> QueueApplyObservedAccountLookup {
            self.account_lookup
        }

        fn ticket_lookup(
            &self,
            _account: &String,
            _tx_seq_proxy: SeqProxy,
        ) -> QueueApplyObservedTicketLookup {
            self.ticket_lookup
        }

        fn calculated_base_fee_drops(&self) -> i64 {
            self.calculated_base_fee_drops
        }

        fn fee_paid_drops(&self) -> i64 {
            self.fee_paid_drops
        }

        fn default_base_fee_drops(&self) -> i64 {
            self.default_base_fee_drops
        }

        fn metrics_snapshot(&self) -> QueueFeeMetricsSnapshot {
            self.metrics_snapshot
        }

        fn open_ledger_tx_count(&self) -> usize {
            self.open_ledger_tx_count
        }

        fn open_ledger_seq(&self) -> u32 {
            self.open_ledger_seq
        }

        fn reserve_drops(&self) -> u64 {
            self.reserve_drops
        }

        fn base_fee_drops(&self) -> u64 {
            self.base_fee_drops
        }
    }

    impl QueueApplyExecutionRuntime<&'static str, &'static str, &'static str> for LoggingRuntime {
        fn run_preflight(
            &mut self,
        ) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
            self.preflight_calls += 1;
            PreflightResult::new(
                "tx",
                None::<&str>,
                Rules::new(std::iter::empty()),
                TxConsequences::new(1, SeqProxy::sequence(6)),
                ApplyFlags::NONE,
                "journal",
                Ter::TER_RETRY,
            )
        }

        fn trace(&mut self, message: &str) {
            self.trace_messages.push(message.to_owned());
        }

        fn direct_apply(&mut self) -> ApplyResult {
            self.direct_apply_calls += 1;
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        }

        fn prepare_multitxn(&mut self, _adjustment: QueueApplyViewAdjustment) -> bool {
            self.prepare_multitxn_calls += 1;
            true
        }

        fn run_preclaim(
            &mut self,
            _view_source: crate::QueueApplyPreclaimViewSource,
        ) -> PreclaimResult<&'static str, &'static str, &'static str> {
            self.preclaim_calls += 1;
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        }

        fn run_try_clear(&mut self) -> ApplyResult {
            self.try_clear_calls += 1;
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        }

        fn apply_sandbox(&mut self) {
            self.apply_sandbox_calls += 1;
        }
    }

    fn test_runtime() -> LoggingRuntime {
        LoggingRuntime::default()
    }

    fn test_owner_shell() -> QueueApplyOwnerShell<String, &'static str, &'static str, &'static str>
    {
        QueueApplyOwnerShell::new(QueueApplyLiveOwner::new(
            2,
            10,
            25,
            None,
            OrderCandidates::new(Uint256::from_u64(0)),
            QueueViews::new(BTreeMap::new(), vec![]),
        ))
    }

    #[test]
    fn runtime_log_sink_wrapper_matches_runtime_envelope_for_preflight_rejects() {
        let account = String::from("acct");
        let tx_source = TestTxSource {
            account: &account,
            transaction_id: "ABC123",
            tx_id: Uint256::from_u64(9),
            tx_seq_proxy: SeqProxy::sequence(6),
        };
        let view_source = TestViewSource {
            rules: Rules::new(std::iter::empty()),
            account_lookup: QueueApplyObservedAccountLookup::Present {
                sequence: 5,
                balance_drops: 1_000,
            },
            ticket_lookup: QueueApplyObservedTicketLookup::Missing,
            calculated_base_fee_drops: 10,
            fee_paid_drops: 1,
            default_base_fee_drops: 10,
            metrics_snapshot: QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: crate::TXQ_BASE_LEVEL * 500,
            },
            open_ledger_tx_count: 4,
            open_ledger_seq: 100,
            reserve_drops: 200,
            base_fee_drops: 10,
        };
        let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
        let hold_preflight = QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250));
        let flags = ApplyFlags::NONE;
        let consequences = TxConsequences::new(1, SeqProxy::sequence(6));
        let can_be_held_result = Ter::TES_SUCCESS;

        let mut runtime_a = test_runtime();
        let mut owner_a = test_owner_shell();
        let emitted_a = RefCell::new(Vec::new());
        let runtime_stage = run_queue_apply_with_runtime_and_log_sinks(
            &mut owner_a,
            &call,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            &mut runtime_a,
            |message| emitted_a.borrow_mut().push(format!("trace:{message}")),
            |message| emitted_a.borrow_mut().push(format!("debug:{message}")),
            |message| emitted_a.borrow_mut().push(format!("info:{message}")),
        );

        let mut runtime_b = test_runtime();
        let mut owner_b = test_owner_shell();
        let emitted_b = RefCell::new(Vec::new());
        let envelope_stage = QueueApplyRuntimeEnvelope::new(&mut runtime_b).apply_with_log_sinks(
            &mut owner_b,
            &call,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            |message| emitted_b.borrow_mut().push(format!("trace:{message}")),
            |message| emitted_b.borrow_mut().push(format!("debug:{message}")),
            |message| emitted_b.borrow_mut().push(format!("info:{message}")),
        );

        assert_eq!(runtime_stage, envelope_stage);
        assert_eq!(runtime_a.preflight_calls, 1);
        assert_eq!(runtime_b.preflight_calls, 1);
        assert_eq!(runtime_a.direct_apply_calls, 0);
        assert_eq!(runtime_b.direct_apply_calls, 0);
        assert!(runtime_a.trace_messages.is_empty());
        assert!(runtime_b.trace_messages.is_empty());
        assert!(emitted_a.into_inner().is_empty());
        assert!(emitted_b.into_inner().is_empty());
    }

    #[test]
    fn runtime_wrapper_runs_preflight_inside_transaction_apply_runtime() {
        set_current_transaction_rules(None);
        set_st_number_switchover(true);

        let rules = Rules::new([feature_universal_number()]);
        let ran_direct_apply = Cell::new(false);
        let ran_queued = Cell::new(false);

        let stage = run_queue_apply_with_runtime::<
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            _,
            _,
            _,
        >(
            &rules,
            || {
                assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
                assert!(get_st_number_switchover());
                assert_eq!(get_mantissa_scale(), MantissaScale::Small);
                PreflightResult::new(
                    "tx",
                    None,
                    Rules::new(std::iter::empty()),
                    "normal",
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TER_RETRY,
                )
            },
            true,
            SeqProxy::sequence(8),
            SeqProxy::sequence(8),
            false,
            || {
                ran_direct_apply.set(true);
                None
            },
            || {
                ran_queued.set(true);
                crate::QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                ))
            },
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                Ter::TER_RETRY,
                false,
                false,
            ))
        );
        assert!(!ran_direct_apply.get());
        assert!(!ran_queued.get());
        assert_eq!(get_current_transaction_rules(), None);
        assert!(get_st_number_switchover());
        assert_eq!(get_mantissa_scale(), MantissaScale::Large);
    }

    #[test]
    fn runtime_wrapper_delegates_success_into_landed_preflight_and_entry_stages() {
        let rules = Rules::new(std::iter::empty());
        let direct = DirectApplyExecution {
            transaction_id: "ABC123",
            attempt: DirectApplyAttemptResult {
                apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                removed_replacement: None::<crate::FeeQueueKey<&'static str>>,
            },
        };

        let stage = run_queue_apply_with_runtime(
            &rules,
            || {
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    "normal",
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            true,
            SeqProxy::sequence(8),
            SeqProxy::sequence(8),
            false,
            || Some(direct.clone()),
            || unreachable!("direct apply should return first"),
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(direct))
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        );
    }
}
