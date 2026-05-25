//! Raw ledger-read and fee-input facts that the current top-level
//! `TxQ::apply(...)` caller gathers before it can enter the landed wrapper
//! chain.
//!
//! This lowers already-observed caller facts into the explicit Rust input
//! bundles that now drive:
//! 1. runtime + preflight + direct-apply flow,
//! 2. queued-stage flow,
//! 3. shared fee-context derivation.

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};

use crate::{
    ApplyFlags, OrderCandidates, QueueApplyCallInputs, QueueApplyFeeContextInputs,
    QueueApplyQueuedWithFeeContextInputs, QueueApplyTopWithDirectApplyInputs,
    QueueApplyTopWithFeeContextInputs, QueueApplyTopWithQueuedStageInputs, QueueFeeMetricsSnapshot,
    QueueHoldPreflight,
};

pub const MISSING_ACCOUNT_SEQ_PROXY: SeqProxy = SeqProxy::sequence(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueApplyLedgerAccountState {
    Missing,
    Present {
        seq_proxy: SeqProxy,
        balance_drops: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueApplyLedgerTicketState {
    NotRequired,
    Present,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueApplyLedgerReadState {
    pub account: QueueApplyLedgerAccountState,
    pub ticket: QueueApplyLedgerTicketState,
}

impl QueueApplyLedgerReadState {
    pub fn account_exists(self) -> bool {
        matches!(self.account, QueueApplyLedgerAccountState::Present { .. })
    }

    pub fn account_seq_proxy_for_top_call(self) -> SeqProxy {
        match self.account {
            QueueApplyLedgerAccountState::Missing => MISSING_ACCOUNT_SEQ_PROXY,
            QueueApplyLedgerAccountState::Present { seq_proxy, .. } => seq_proxy,
        }
    }

    pub fn ticket_exists_for_top_call(self) -> bool {
        !matches!(self.ticket, QueueApplyLedgerTicketState::Missing)
    }

    pub fn balance_drops_for_queued_stage(self) -> u64 {
        match self.account {
            QueueApplyLedgerAccountState::Missing => 0,
            QueueApplyLedgerAccountState::Present { balance_drops, .. } => balance_drops,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueApplyTopReadInputs<'a, Account, TxId> {
    pub rules: &'a Rules,
    pub account: &'a Account,
    pub transaction_id: TxId,
    pub tx_seq_proxy: SeqProxy,
    pub ledger_read: QueueApplyLedgerReadState,
    pub calculated_base_fee_drops: i64,
    pub fee_paid_drops: i64,
    pub default_base_fee_drops: i64,
    pub metrics_snapshot: QueueFeeMetricsSnapshot,
    pub open_ledger_tx_count: usize,
    pub flags: ApplyFlags,
    pub preflight: QueueHoldPreflight,
    pub is_blocker: bool,
    pub open_ledger_seq: u32,
    pub minimum_last_ledger_buffer: u32,
    pub maximum_txn_per_account: usize,
    pub retry_sequence_percent: u32,
    pub queue_is_full: bool,
    pub reserve_drops: u64,
    pub base_fee_drops: u64,
    pub can_be_held_result: Ter,
    pub last_valid: Option<u32>,
    pub order: &'a OrderCandidates,
    pub tx_id: Uint256,
}

pub fn build_queue_apply_top_with_queued_stage_inputs<'a, Account: Clone, TxId>(
    inputs: QueueApplyTopReadInputs<'a, Account, TxId>,
) -> QueueApplyTopWithQueuedStageInputs<'a, Account, TxId> {
    let call = QueueApplyCallInputs::new(
        inputs.rules,
        inputs.ledger_read.account_exists(),
        inputs.ledger_read.account_seq_proxy_for_top_call(),
        inputs.tx_seq_proxy,
        inputs.ledger_read.ticket_exists_for_top_call(),
    );
    let fee_context_inputs = QueueApplyFeeContextInputs {
        calculated_base_fee_drops: inputs.calculated_base_fee_drops,
        fee_paid_drops: inputs.fee_paid_drops,
        default_base_fee_drops: inputs.default_base_fee_drops,
        metrics_snapshot: inputs.metrics_snapshot,
        open_ledger_tx_count: inputs.open_ledger_tx_count,
        flags: inputs.flags,
    };

    QueueApplyTopWithQueuedStageInputs::new(
        QueueApplyTopWithDirectApplyInputs::new(
            QueueApplyTopWithFeeContextInputs::new(call, fee_context_inputs),
            inputs.transaction_id,
            inputs.account,
        ),
        QueueApplyQueuedWithFeeContextInputs {
            account: inputs.account.clone(),
            preflight: inputs.preflight,
            is_blocker: inputs.is_blocker,
            open_ledger_seq: inputs.open_ledger_seq,
            minimum_last_ledger_buffer: inputs.minimum_last_ledger_buffer,
            maximum_txn_per_account: inputs.maximum_txn_per_account,
            retry_sequence_percent: inputs.retry_sequence_percent,
            queue_is_full: inputs.queue_is_full,
            balance_drops: inputs.ledger_read.balance_drops_for_queued_stage(),
            reserve_drops: inputs.reserve_drops,
            base_fee_drops: inputs.base_fee_drops,
            can_be_held_result: inputs.can_be_held_result,
            open_ledger_tx_count: inputs.open_ledger_tx_count,
            tx_id: inputs.tx_id,
            last_valid: inputs.last_valid,
            flags: inputs.flags,
            order: inputs.order,
        },
    )
}

#[cfg(test)]
mod tests {
    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{
        MISSING_ACCOUNT_SEQ_PROXY, QueueApplyLedgerAccountState, QueueApplyLedgerReadState,
        QueueApplyLedgerTicketState, QueueApplyTopReadInputs,
        build_queue_apply_top_with_queued_stage_inputs,
    };
    use crate::{ApplyFlags, OrderCandidates, QueueFeeMetricsSnapshot, QueueHoldPreflight};

    #[test]
    fn missing_account_lowers_to_guarded_placeholder_state() {
        let rules = Rules::new(std::iter::empty());
        let account = "acct";
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let built = build_queue_apply_top_with_queued_stage_inputs(QueueApplyTopReadInputs {
            rules: &rules,
            account: &account,
            transaction_id: "ABC123",
            tx_seq_proxy: SeqProxy::sequence(8),
            ledger_read: QueueApplyLedgerReadState {
                account: QueueApplyLedgerAccountState::Missing,
                ticket: QueueApplyLedgerTicketState::NotRequired,
            },
            calculated_base_fee_drops: 10,
            fee_paid_drops: 20,
            default_base_fee_drops: 10,
            metrics_snapshot: QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: crate::TXQ_BASE_LEVEL * 500,
            },
            open_ledger_tx_count: 4,
            flags: ApplyFlags::NONE,
            preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
            is_blocker: false,
            open_ledger_seq: 100,
            minimum_last_ledger_buffer: 2,
            maximum_txn_per_account: 10,
            retry_sequence_percent: 25,
            queue_is_full: false,
            reserve_drops: 200,
            base_fee_drops: 10,
            can_be_held_result: Ter::TES_SUCCESS,
            last_valid: Some(250),
            order: &order,
            tx_id: Uint256::from_u64(9),
        });

        assert!(!built.direct_apply.top.call.account_exists);
        assert_eq!(
            built.direct_apply.top.call.account_seq_proxy,
            MISSING_ACCOUNT_SEQ_PROXY
        );
        assert!(built.direct_apply.top.call.ticket_exists);
        assert_eq!(built.queued.balance_drops, 0);
    }

    #[test]
    fn present_account_and_missing_ticket_preserve_read_side_facts() {
        let rules = Rules::new(std::iter::empty());
        let account = "acct";
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let built = build_queue_apply_top_with_queued_stage_inputs(QueueApplyTopReadInputs {
            rules: &rules,
            account: &account,
            transaction_id: "ABC123",
            tx_seq_proxy: SeqProxy::ticket(9),
            ledger_read: QueueApplyLedgerReadState {
                account: QueueApplyLedgerAccountState::Present {
                    seq_proxy: SeqProxy::sequence(8),
                    balance_drops: 1_000,
                },
                ticket: QueueApplyLedgerTicketState::Missing,
            },
            calculated_base_fee_drops: 10,
            fee_paid_drops: 20,
            default_base_fee_drops: 10,
            metrics_snapshot: QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: crate::TXQ_BASE_LEVEL * 500,
            },
            open_ledger_tx_count: 4,
            flags: ApplyFlags::NONE,
            preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
            is_blocker: true,
            open_ledger_seq: 100,
            minimum_last_ledger_buffer: 2,
            maximum_txn_per_account: 10,
            retry_sequence_percent: 25,
            queue_is_full: true,
            reserve_drops: 200,
            base_fee_drops: 10,
            can_be_held_result: Ter::TER_PRE_SEQ,
            last_valid: Some(250),
            order: &order,
            tx_id: Uint256::from_u64(9),
        });

        assert!(built.direct_apply.top.call.account_exists);
        assert_eq!(
            built.direct_apply.top.call.account_seq_proxy,
            SeqProxy::sequence(8)
        );
        assert!(!built.direct_apply.top.call.ticket_exists);
        assert_eq!(
            built.direct_apply.top.call.tx_seq_proxy,
            SeqProxy::ticket(9)
        );
        assert_eq!(built.queued.balance_drops, 1_000);
        assert!(built.queued.is_blocker);
        assert!(built.queued.queue_is_full);
        assert_eq!(built.queued.can_be_held_result, Ter::TER_PRE_SEQ);
    }
}
