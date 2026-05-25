//! `requiresMultiTxn` preparation block inside `TxQ::apply(...)`.
//!
//! This preserves the current ordered composition for:
//! 1. path selection and early `tefPAST_SEQ` / `terPRE_SEQ` rejection,
//! 2. queued-account `canBeHeld(...)` and sequence-fit admission when a
//!    staged `multiTxn` view is required,
//! 3. queued-balance rejection before staged preclaim,
//! 4. staged view adjustment and no-`multiTxn` hold fallback shaping.
//!

use protocol::{SeqProxy, Ter};

use crate::{
    ApplyResult, QueueApplyAccountContext, QueueApplyBalanceDecision, QueueApplyHoldFallback,
    QueueApplyPath, QueueApplyViewAdjustment, QueueHoldPreflight, TxConsequences, TxQAccount,
    evaluate_queue_apply_balance, evaluate_queue_apply_hold_fallback,
    evaluate_queue_apply_multitxn_admission, evaluate_queue_apply_path,
    evaluate_queue_apply_view_adjustment,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueApplyMultiTxnInputs {
    pub preflight: QueueHoldPreflight,
    pub open_ledger_seq: u32,
    pub minimum_last_ledger_buffer: u32,
    pub maximum_txn_per_account: usize,
    pub account_seq_proxy: SeqProxy,
    pub tx_seq_proxy: SeqProxy,
    pub balance_drops: u64,
    pub reserve_drops: u64,
    pub base_fee_drops: u64,
    pub can_be_held_result: Ter,
    pub consequences: TxConsequences,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueApplyMultiTxnContext {
    pub path: QueueApplyPath,
    pub view_adjustment: Option<QueueApplyViewAdjustment>,
    pub hold_fallback: QueueApplyHoldFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueApplyMultiTxnStage {
    RejectPath(Ter),
    RejectBalance(QueueApplyBalanceDecision),
    Ready(QueueApplyMultiTxnContext),
}

impl QueueApplyMultiTxnStage {
    pub fn apply_result(&self) -> ApplyResult {
        match self {
            Self::RejectPath(ter) => ApplyResult::new(*ter, false, false),
            Self::RejectBalance(decision) => {
                ApplyResult::new(decision.ter().expect("balance rejection"), false, false)
            }
            Self::Ready(_) => ApplyResult::new(Ter::TES_SUCCESS, false, false),
        }
    }
}

pub fn run_queue_apply_multitxn_stage<Account, T>(
    tx_q_account: Option<&TxQAccount<Account, T>>,
    account_context: &QueueApplyAccountContext<Account>,
    inputs: QueueApplyMultiTxnInputs,
) -> QueueApplyMultiTxnStage {
    let path = match evaluate_queue_apply_path(
        account_context.window.relevant_tx_count,
        inputs.tx_seq_proxy,
        inputs.account_seq_proxy,
        account_context.window.replaces_existing,
    ) {
        Ok(path) => path,
        Err(ter) => return QueueApplyMultiTxnStage::RejectPath(ter),
    };

    let path = if path.requires_multi_txn() {
        let tx_q_account = tx_q_account.expect("xrpl::TxQ::apply : account in queue");
        match evaluate_queue_apply_multitxn_admission(
            path,
            inputs.preflight,
            inputs.open_ledger_seq,
            inputs.minimum_last_ledger_buffer,
            tx_q_account,
            inputs.maximum_txn_per_account,
            inputs.tx_seq_proxy,
            inputs.account_seq_proxy,
            account_context.window.replaces_existing,
        ) {
            Ok(path) => path,
            Err(ter) => return QueueApplyMultiTxnStage::RejectPath(ter),
        }
    } else {
        path
    };

    let view_adjustment = if path.requires_multi_txn() {
        let tx_q_account = tx_q_account.expect("xrpl::TxQ::apply : account in queue");
        let balance_decision = evaluate_queue_apply_balance(
            tx_q_account,
            inputs.account_seq_proxy,
            inputs.tx_seq_proxy,
            inputs.consequences,
            inputs.balance_drops,
            inputs.reserve_drops,
            inputs.base_fee_drops,
        );
        if balance_decision.ter().is_some() {
            return QueueApplyMultiTxnStage::RejectBalance(balance_decision);
        }

        Some(evaluate_queue_apply_view_adjustment(
            balance_decision.totals().total_fee_drops,
            balance_decision.totals().potential_spend_drops,
            inputs.balance_drops,
            inputs.reserve_drops,
            inputs.base_fee_drops,
            inputs.tx_seq_proxy,
            tx_q_account.next_queuable_seq(inputs.account_seq_proxy),
        ))
    } else {
        None
    };

    QueueApplyMultiTxnStage::Ready(QueueApplyMultiTxnContext {
        path,
        view_adjustment,
        hold_fallback: evaluate_queue_apply_hold_fallback(
            path.requires_multi_txn(),
            inputs.can_be_held_result,
        ),
    })
}

#[cfg(test)]
mod tests {
    use protocol::{SeqProxy, Ter};

    use super::{
        QueueApplyMultiTxnContext, QueueApplyMultiTxnInputs, QueueApplyMultiTxnStage,
        run_queue_apply_multitxn_stage,
    };
    use crate::{
        AccountQueueWindow, ApplyFlags, ApplyResult, MaybeTxCore, QueueApplyAccountContext,
        QueueApplyBalanceDecision, QueueApplyHoldFallback, QueueApplyPath,
        QueueApplyViewAdjustment, QueueHoldPreflight, TxConsequences, TxQAccount,
    };

    #[test]
    fn multitxn_stage_keeps_open_ledger_path_without_staged_adjustment() {
        let stage = run_queue_apply_multitxn_stage::<&str, &str>(
            None,
            &QueueApplyAccountContext {
                window: AccountQueueWindow::default(),
                first_relevant_retries_remaining: None,
                replacement_decision: None,
                replaced: None,
            },
            QueueApplyMultiTxnInputs {
                preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                open_ledger_seq: 100,
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                account_seq_proxy: SeqProxy::sequence(5),
                tx_seq_proxy: SeqProxy::sequence(5),
                balance_drops: 1_000,
                reserve_drops: 200,
                base_fee_drops: 10,
                can_be_held_result: Ter::TES_SUCCESS,
                consequences: TxConsequences::new(1, SeqProxy::sequence(5)),
            },
        );

        assert_eq!(
            stage,
            QueueApplyMultiTxnStage::Ready(QueueApplyMultiTxnContext {
                path: QueueApplyPath::OpenLedger,
                view_adjustment: None,
                hold_fallback: QueueApplyHoldFallback::HoldAllowed,
            })
        );
    }

    #[test]
    fn multitxn_stage_rejects_path_when_queued_admission_fails() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 1),
            ),
        );
        account.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new(
                "s6",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(6), 1),
            ),
        );

        let stage = run_queue_apply_multitxn_stage(
            Some(&account),
            &QueueApplyAccountContext {
                window: AccountQueueWindow {
                    account_is_in_queue: true,
                    first_relevant_seq_proxy: Some(SeqProxy::sequence(5)),
                    relevant_tx_count: 2,
                    replaces_existing: false,
                    front_is_blocker: false,
                },
                first_relevant_retries_remaining: Some(10),
                replacement_decision: None,
                replaced: None,
            },
            QueueApplyMultiTxnInputs {
                preflight: QueueHoldPreflight::new(true, false, ApplyFlags::NONE, None),
                open_ledger_seq: 100,
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                account_seq_proxy: SeqProxy::sequence(5),
                tx_seq_proxy: SeqProxy::sequence(8),
                balance_drops: 1_000,
                reserve_drops: 200,
                base_fee_drops: 10,
                can_be_held_result: Ter::TES_SUCCESS,
                consequences: TxConsequences::new(1, SeqProxy::sequence(8)),
            },
        );

        assert_eq!(
            stage,
            QueueApplyMultiTxnStage::RejectPath(Ter::TEL_CAN_NOT_QUEUE)
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE, false, false)
        );
    }

    #[test]
    fn multitxn_stage_rejects_balance_before_ready_context() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_potential_spend(70, SeqProxy::sequence(5), 100),
            ),
        );
        account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                "s7",
                TxConsequences::with_potential_spend(50, SeqProxy::sequence(7), 70),
            ),
        );

        let stage = run_queue_apply_multitxn_stage(
            Some(&account),
            &QueueApplyAccountContext {
                window: AccountQueueWindow {
                    account_is_in_queue: true,
                    first_relevant_seq_proxy: Some(SeqProxy::sequence(5)),
                    relevant_tx_count: 2,
                    replaces_existing: false,
                    front_is_blocker: false,
                },
                first_relevant_retries_remaining: Some(10),
                replacement_decision: None,
                replaced: None,
            },
            QueueApplyMultiTxnInputs {
                preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                open_ledger_seq: 100,
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                account_seq_proxy: SeqProxy::sequence(5),
                tx_seq_proxy: SeqProxy::sequence(6),
                balance_drops: 120,
                reserve_drops: 1_000,
                base_fee_drops: 10,
                can_be_held_result: Ter::TES_SUCCESS,
                consequences: TxConsequences::with_potential_spend(90, SeqProxy::sequence(6), 80),
            },
        );

        assert_eq!(
            stage,
            QueueApplyMultiTxnStage::RejectBalance(
                QueueApplyBalanceDecision::TotalFeesInFlightTooHigh(
                    crate::QueueApplyBalanceTotals {
                        total_fee_drops: 120,
                        potential_spend_drops: 170,
                    }
                )
            )
        );
    }

    #[test]
    fn multitxn_stage_carries_ready_multitxn_context() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_potential_spend(70, SeqProxy::sequence(5), 100),
            ),
        );
        account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                "s7",
                TxConsequences::with_potential_spend(50, SeqProxy::sequence(7), 70),
            ),
        );

        let stage = run_queue_apply_multitxn_stage(
            Some(&account),
            &QueueApplyAccountContext {
                window: AccountQueueWindow {
                    account_is_in_queue: true,
                    first_relevant_seq_proxy: Some(SeqProxy::sequence(5)),
                    relevant_tx_count: 2,
                    replaces_existing: false,
                    front_is_blocker: false,
                },
                first_relevant_retries_remaining: Some(10),
                replacement_decision: None,
                replaced: None,
            },
            QueueApplyMultiTxnInputs {
                preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                open_ledger_seq: 100,
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                account_seq_proxy: SeqProxy::sequence(5),
                tx_seq_proxy: SeqProxy::sequence(6),
                balance_drops: 1_000,
                reserve_drops: 200,
                base_fee_drops: 10,
                can_be_held_result: Ter::TES_SUCCESS,
                consequences: TxConsequences::with_potential_spend(90, SeqProxy::sequence(6), 80),
            },
        );

        assert_eq!(
            stage,
            QueueApplyMultiTxnStage::Ready(QueueApplyMultiTxnContext {
                path: QueueApplyPath::QueuedAccount {
                    requires_multi_txn: true,
                },
                view_adjustment: Some(QueueApplyViewAdjustment {
                    potential_total_spend_drops: 290,
                    adjusted_balance_drops: 710,
                    applied_sequence_value: 6,
                }),
                hold_fallback: QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            })
        );
    }
}
