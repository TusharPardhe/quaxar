//! First `preclaim(...)` composition inside `TxQ::apply(...)`.
//!
//! This ports the exact deterministic control flow around `preclaim(...)`:
//! choose whether `preclaim(...)` should
//! run against the current view or the staged `multiTxn` open view, reject
//! immediately when the result is not likely to claim a fee, assert the
//! post-`preclaim(...)` minimum-fee invariant, and format the current trace
//! message before the later clear-ahead heuristic.

use std::fmt::Display;

use crate::{ApplyResult, FeeLevel64, PreclaimResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueApplyPreclaimViewSource {
    CurrentView,
    MultiTxnOpenView,
}

impl QueueApplyPreclaimViewSource {
    pub fn has_multi_txn(self) -> bool {
        matches!(self, Self::MultiTxnOpenView)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyPreclaimStage<Tx, Journal, ParentBatchId> {
    pub view_source: QueueApplyPreclaimViewSource,
    pub preclaim_result: PreclaimResult<Tx, Journal, ParentBatchId>,
    pub trace_message: String,
}

pub fn choose_queue_apply_preclaim_view_source(
    has_multi_txn: bool,
) -> QueueApplyPreclaimViewSource {
    if has_multi_txn {
        QueueApplyPreclaimViewSource::MultiTxnOpenView
    } else {
        QueueApplyPreclaimViewSource::CurrentView
    }
}

pub fn format_queue_apply_preclaim_trace_message<TxId, Account>(
    transaction_id: TxId,
    account: Account,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
    open_ledger_tx_count: usize,
) -> String
where
    TxId: Display,
    Account: Display,
{
    format!(
        "Transaction {} from account {} has fee level of {} needs at least {} to get in the open ledger, which has {} entries.",
        transaction_id, account, fee_level_paid, required_fee_level, open_ledger_tx_count
    )
}

pub fn run_queue_apply_preclaim_stage<Tx, Journal, ParentBatchId, RunPreclaim, TxId, Account>(
    view_source: QueueApplyPreclaimViewSource,
    fee_level_paid: FeeLevel64,
    base_level: FeeLevel64,
    required_fee_level: FeeLevel64,
    open_ledger_tx_count: usize,
    transaction_id: TxId,
    account: Account,
    run_preclaim: RunPreclaim,
) -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, ApplyResult>
where
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    TxId: Display,
    Account: Display,
{
    let preclaim_result = run_preclaim(view_source);

    if !preclaim_result.likely_to_claim_fee {
        return Err(ApplyResult::new(preclaim_result.ter, false, false));
    }

    assert!(
        fee_level_paid >= base_level,
        "xrpl::TxQ::apply : minimum fee"
    );

    Ok(QueueApplyPreclaimStage {
        view_source,
        trace_message: format_queue_apply_preclaim_trace_message(
            transaction_id,
            account,
            fee_level_paid,
            required_fee_level,
            open_ledger_tx_count,
        ),
        preclaim_result,
    })
}

#[cfg(test)]
mod tests {
    use protocol::Ter;

    use super::{
        QueueApplyPreclaimViewSource, choose_queue_apply_preclaim_view_source,
        format_queue_apply_preclaim_trace_message, run_queue_apply_preclaim_stage,
    };
    use crate::{ApplyFlags, ApplyResult, PreclaimResult};

    #[test]
    fn preclaim_stage_uses_current_view_when_multitxn_is_absent() {
        let stage = run_queue_apply_preclaim_stage(
            QueueApplyPreclaimViewSource::CurrentView,
            12,
            10,
            11,
            4,
            "tx",
            "acct",
            |source| {
                assert_eq!(source, QueueApplyPreclaimViewSource::CurrentView);
                PreclaimResult::new(
                    7,
                    "tx",
                    None::<&str>,
                    ApplyFlags::NONE,
                    (),
                    Ter::TES_SUCCESS,
                )
            },
        )
        .expect("preclaim should continue");

        assert_eq!(stage.view_source, QueueApplyPreclaimViewSource::CurrentView);
    }

    #[test]
    fn preclaim_stage_uses_multitxn_view_when_present() {
        let stage = run_queue_apply_preclaim_stage(
            QueueApplyPreclaimViewSource::MultiTxnOpenView,
            12,
            10,
            11,
            4,
            "tx",
            "acct",
            |source| {
                assert_eq!(source, QueueApplyPreclaimViewSource::MultiTxnOpenView);
                PreclaimResult::new(
                    7,
                    "tx",
                    None::<&str>,
                    ApplyFlags::NONE,
                    (),
                    Ter::TES_SUCCESS,
                )
            },
        )
        .expect("preclaim should continue");

        assert_eq!(
            stage.view_source,
            QueueApplyPreclaimViewSource::MultiTxnOpenView
        );
    }

    #[test]
    fn preclaim_stage_returns_exact_preclaim_rejection_when_fee_will_not_be_claimed() {
        let result = run_queue_apply_preclaim_stage(
            QueueApplyPreclaimViewSource::CurrentView,
            12,
            10,
            11,
            4,
            "tx",
            "acct",
            |_| PreclaimResult::new(7, "tx", None::<&str>, ApplyFlags::NONE, (), Ter::TER_RETRY),
        );

        assert_eq!(result, Err(ApplyResult::new(Ter::TER_RETRY, false, false)));
    }

    #[test]
    fn preclaim_stage_formats_fee_trace() {
        let stage = run_queue_apply_preclaim_stage(
            QueueApplyPreclaimViewSource::MultiTxnOpenView,
            27,
            10,
            15,
            42,
            "ABC",
            "rAccount",
            |_| {
                PreclaimResult::new(
                    7,
                    "tx",
                    None::<&str>,
                    ApplyFlags::NONE,
                    (),
                    Ter::TES_SUCCESS,
                )
            },
        )
        .expect("preclaim should continue");

        assert_eq!(
            stage.trace_message,
            "Transaction ABC from account rAccount has fee level of 27 needs at least 15 to get in the open ledger, which has 42 entries."
        );
    }

    #[test]
    #[should_panic(expected = "xrpl::TxQ::apply : minimum fee")]
    fn preclaim_stage_panics_when_post_preclaim_fee_is_below_base() {
        let _ = run_queue_apply_preclaim_stage(
            QueueApplyPreclaimViewSource::CurrentView,
            9,
            10,
            11,
            4,
            "tx",
            "acct",
            |_| {
                PreclaimResult::new(
                    7,
                    "tx",
                    None::<&str>,
                    ApplyFlags::NONE,
                    (),
                    Ter::TES_SUCCESS,
                )
            },
        );
    }

    #[test]
    fn preclaim_view_source_helper_matches_multitxn_presence() {
        assert_eq!(
            choose_queue_apply_preclaim_view_source(false),
            QueueApplyPreclaimViewSource::CurrentView
        );
        assert_eq!(
            choose_queue_apply_preclaim_view_source(true),
            QueueApplyPreclaimViewSource::MultiTxnOpenView
        );
        assert!(!QueueApplyPreclaimViewSource::CurrentView.has_multi_txn());
        assert!(QueueApplyPreclaimViewSource::MultiTxnOpenView.has_multi_txn());
    }

    #[test]
    fn preclaim_trace_message_helper_matches_current_cpp_shape() {
        assert_eq!(
            format_queue_apply_preclaim_trace_message("tx", "acct", 30, 12, 8),
            "Transaction tx from account acct has fee level of 30 needs at least 12 to get in the open ledger, which has 8 entries."
        );
    }
}
