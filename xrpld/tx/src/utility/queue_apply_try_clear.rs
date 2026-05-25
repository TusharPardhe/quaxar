//! Heuristic gate that decides whether `TxQ::apply(...)` should attempt
//! `tryClearAccountQueueUpThruTx(...)`.
//!
//! This ports the deterministic admission checks around that clear-ahead
//! attempt plus the immediate control flow after a supplied
//! `tryClearAccountQueueUpThruTx(...)` result.

use protocol::SeqProxy;

use crate::{ApplyResult, FeeLevel64, MAYBE_TX_RETRIES_ALLOWED, TryClearAccountResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueApplyTryClearGate {
    Bypass,
    AttemptClearAhead,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueApplyTryClearStage {
    Bypass,
    ContinueAfterAttempt,
    ApplySandboxAndReturn(ApplyResult),
}

pub trait QueueApplyTryClearResult {
    fn apply_result(&self) -> ApplyResult;
}

impl QueueApplyTryClearResult for ApplyResult {
    fn apply_result(&self) -> ApplyResult {
        self.clone()
    }
}

impl QueueApplyTryClearResult for TryClearAccountResult {
    fn apply_result(&self) -> ApplyResult {
        self.apply_result()
    }
}

pub fn evaluate_queue_apply_try_clear_gate(
    tx_seq_proxy: SeqProxy,
    first_relevant_retries_remaining: Option<i32>,
    has_multi_txn: bool,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
    base_fee_level: FeeLevel64,
) -> QueueApplyTryClearGate {
    if tx_seq_proxy.is_seq()
        && first_relevant_retries_remaining == Some(MAYBE_TX_RETRIES_ALLOWED)
        && has_multi_txn
        && fee_level_paid > required_fee_level
        && required_fee_level > base_fee_level
    {
        return QueueApplyTryClearGate::AttemptClearAhead;
    }

    QueueApplyTryClearGate::Bypass
}

pub fn run_queue_apply_try_clear_stage<RunTryClear, ApplySandbox, TryClearResult>(
    gate: QueueApplyTryClearGate,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyTryClearStage
where
    RunTryClear: FnOnce() -> TryClearResult,
    ApplySandbox: FnOnce(),
    TryClearResult: QueueApplyTryClearResult,
{
    if gate != QueueApplyTryClearGate::AttemptClearAhead {
        return QueueApplyTryClearStage::Bypass;
    }

    let result = run_try_clear().apply_result();
    if !result.applied {
        return QueueApplyTryClearStage::ContinueAfterAttempt;
    }

    apply_sandbox();
    QueueApplyTryClearStage::ApplySandboxAndReturn(result)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::SeqProxy;
    use protocol::Ter;

    use super::{
        QueueApplyTryClearGate, QueueApplyTryClearResult, QueueApplyTryClearStage,
        evaluate_queue_apply_try_clear_gate, run_queue_apply_try_clear_stage,
    };
    use crate::{
        ApplyResult, MAYBE_TX_RETRIES_ALLOWED, TryClearAccountPlan, TryClearAccountResult,
        TryClearExecution, TryClearFinalization,
    };

    #[test]
    fn try_clear_gate_matches_current_cpp_attempt_conditions() {
        assert_eq!(
            evaluate_queue_apply_try_clear_gate(
                SeqProxy::sequence(7),
                Some(MAYBE_TX_RETRIES_ALLOWED),
                true,
                200,
                120,
                100,
            ),
            QueueApplyTryClearGate::AttemptClearAhead
        );
    }

    #[test]
    fn try_clear_gate_bypasses_tickets_or_missing_relevant_tail() {
        assert_eq!(
            evaluate_queue_apply_try_clear_gate(
                SeqProxy::ticket(7),
                Some(MAYBE_TX_RETRIES_ALLOWED),
                true,
                200,
                120,
                100,
            ),
            QueueApplyTryClearGate::Bypass
        );
        assert_eq!(
            evaluate_queue_apply_try_clear_gate(SeqProxy::sequence(7), None, true, 200, 120, 100,),
            QueueApplyTryClearGate::Bypass
        );
    }

    #[test]
    fn try_clear_gate_bypasses_when_multitxn_or_retry_budget_conditions_fail() {
        assert_eq!(
            evaluate_queue_apply_try_clear_gate(
                SeqProxy::sequence(7),
                Some(MAYBE_TX_RETRIES_ALLOWED),
                false,
                200,
                120,
                100,
            ),
            QueueApplyTryClearGate::Bypass
        );
        assert_eq!(
            evaluate_queue_apply_try_clear_gate(
                SeqProxy::sequence(7),
                Some(MAYBE_TX_RETRIES_ALLOWED - 1),
                true,
                200,
                120,
                100,
            ),
            QueueApplyTryClearGate::Bypass
        );
    }

    #[test]
    fn try_clear_gate_requires_strict_fee_headroom_above_queue_and_base_levels() {
        assert_eq!(
            evaluate_queue_apply_try_clear_gate(
                SeqProxy::sequence(7),
                Some(MAYBE_TX_RETRIES_ALLOWED),
                true,
                120,
                120,
                100,
            ),
            QueueApplyTryClearGate::Bypass
        );
        assert_eq!(
            evaluate_queue_apply_try_clear_gate(
                SeqProxy::sequence(7),
                Some(MAYBE_TX_RETRIES_ALLOWED),
                true,
                200,
                100,
                100,
            ),
            QueueApplyTryClearGate::Bypass
        );
    }

    #[test]
    fn try_clear_stage_skips_attempt_and_sandbox_when_gate_bypasses() {
        let ran_try_clear = Cell::new(false);
        let applied_sandbox = Cell::new(false);

        let stage = run_queue_apply_try_clear_stage(
            QueueApplyTryClearGate::Bypass,
            || {
                ran_try_clear.set(true);
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
            || {
                applied_sandbox.set(true);
            },
        );

        assert_eq!(stage, QueueApplyTryClearStage::Bypass);
        assert!(!ran_try_clear.get());
        assert!(!applied_sandbox.get());
    }

    #[test]
    fn try_clear_stage_continues_without_applying_sandbox_when_attempt_fails() {
        let ran_try_clear = Cell::new(false);
        let applied_sandbox = Cell::new(false);

        let stage = run_queue_apply_try_clear_stage(
            QueueApplyTryClearGate::AttemptClearAhead,
            || {
                ran_try_clear.set(true);
                ApplyResult::new(Ter::TER_RETRY, false, false)
            },
            || {
                applied_sandbox.set(true);
            },
        );

        assert_eq!(stage, QueueApplyTryClearStage::ContinueAfterAttempt);
        assert!(ran_try_clear.get());
        assert!(!applied_sandbox.get());
    }

    #[test]
    fn try_clear_stage_applies_sandbox_only_for_applied_result() {
        let ran_try_clear = Cell::new(false);
        let applied_sandbox = Cell::new(false);

        let stage = run_queue_apply_try_clear_stage(
            QueueApplyTryClearGate::AttemptClearAhead,
            || {
                ran_try_clear.set(true);
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
            || {
                applied_sandbox.set(true);
            },
        );

        assert_eq!(
            stage,
            QueueApplyTryClearStage::ApplySandboxAndReturn(ApplyResult::new(
                Ter::TES_SUCCESS,
                true,
                false,
            ))
        );
        assert!(ran_try_clear.get());
        assert!(applied_sandbox.get());
    }

    #[test]
    fn try_clear_stage_can_consume_structured_account_result_without_applying_sandbox() {
        let ran_try_clear = Cell::new(false);
        let applied_sandbox = Cell::new(false);

        let stage = run_queue_apply_try_clear_stage(
            QueueApplyTryClearGate::AttemptClearAhead,
            || {
                ran_try_clear.set(true);
                TryClearAccountResult::InsufficientFee {
                    plan: TryClearAccountPlan {
                        queued_seq_proxies: vec![SeqProxy::sequence(5)],
                        queued_count: 1,
                        target_was_already_queued: false,
                        total_fee_level_paid: 50,
                    },
                    required_total_fee_level: 60,
                }
            },
            || {
                applied_sandbox.set(true);
            },
        );

        assert_eq!(stage, QueueApplyTryClearStage::ContinueAfterAttempt);
        assert!(ran_try_clear.get());
        assert!(!applied_sandbox.get());
    }

    #[test]
    fn try_clear_result_trait_preserves_structured_clear_ahead_success_shape() {
        let result = TryClearAccountResult::ClearQueue {
            plan: TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5)],
                queued_count: 1,
                target_was_already_queued: false,
                total_fee_level_paid: 50,
            },
            required_total_fee_level: 40,
            execution: TryClearExecution::CurrentTx(TryClearFinalization {
                current_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                cleanup: None,
            }),
        };

        assert_eq!(
            QueueApplyTryClearResult::apply_result(&result),
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        );
    }
}
