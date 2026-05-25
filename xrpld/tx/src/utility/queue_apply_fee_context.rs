//! Deterministic fee-context shaping for current `TxQ::apply(...)` and
//! `TxQ::tryDirectApply(...)`
//! before deeper branch decisions.
//!
//! This exposes one explicit pure carrier for:
//! 1. the paid fee level,
//! 2. the current required open-ledger fee level,
//! 3. the reference/base level used by later minimum-fee assertions.

use crate::{
    ApplyFlags, FeeLevel64, QueueFeeLevelPaidInputs, QueueFeeMetricsSnapshot, TXQ_BASE_LEVEL,
    evaluate_fee_level_paid, evaluate_required_fee_level,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueApplyFeeContextInputs {
    pub calculated_base_fee_drops: i64,
    pub fee_paid_drops: i64,
    pub default_base_fee_drops: i64,
    pub metrics_snapshot: QueueFeeMetricsSnapshot,
    pub open_ledger_tx_count: usize,
    pub flags: ApplyFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueApplyFeeContext {
    pub base_level: FeeLevel64,
    pub fee_level_paid: FeeLevel64,
    pub required_fee_level: FeeLevel64,
}

pub fn evaluate_queue_apply_fee_context(
    inputs: QueueApplyFeeContextInputs,
) -> QueueApplyFeeContext {
    QueueApplyFeeContext {
        base_level: TXQ_BASE_LEVEL,
        fee_level_paid: evaluate_fee_level_paid(QueueFeeLevelPaidInputs {
            calculated_base_fee_drops: inputs.calculated_base_fee_drops,
            fee_paid_drops: inputs.fee_paid_drops,
            default_base_fee_drops: inputs.default_base_fee_drops,
        }),
        required_fee_level: evaluate_required_fee_level(
            inputs.metrics_snapshot,
            inputs.open_ledger_tx_count,
            inputs.flags,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        QueueApplyFeeContext, QueueApplyFeeContextInputs, evaluate_queue_apply_fee_context,
    };
    use crate::{
        ApplyFlags, QueueFeeLevelPaidInputs, QueueFeeMetricsSnapshot, TXQ_BASE_LEVEL,
        evaluate_fee_level_paid, evaluate_required_fee_level,
    };

    #[test]
    fn fee_context_matches_current_paid_and_required_fee_helper_composition() {
        let snapshot = QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        };
        let inputs = QueueApplyFeeContextInputs {
            calculated_base_fee_drops: 10,
            fee_paid_drops: 25,
            default_base_fee_drops: 10,
            metrics_snapshot: snapshot,
            open_ledger_tx_count: 40,
            flags: ApplyFlags::FAIL_HARD,
        };

        assert_eq!(
            evaluate_queue_apply_fee_context(inputs),
            QueueApplyFeeContext {
                base_level: TXQ_BASE_LEVEL,
                fee_level_paid: evaluate_fee_level_paid(QueueFeeLevelPaidInputs {
                    calculated_base_fee_drops: 10,
                    fee_paid_drops: 25,
                    default_base_fee_drops: 10,
                }),
                required_fee_level: evaluate_required_fee_level(
                    snapshot,
                    40,
                    ApplyFlags::FAIL_HARD,
                ),
            }
        );
    }

    #[test]
    fn fee_context_keeps_base_level_explicit_for_later_apply_guards() {
        let snapshot = QueueFeeMetricsSnapshot {
            txns_expected: 64,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        };

        let context = evaluate_queue_apply_fee_context(QueueApplyFeeContextInputs {
            calculated_base_fee_drops: 0,
            fee_paid_drops: 0,
            default_base_fee_drops: 10,
            metrics_snapshot: snapshot,
            open_ledger_tx_count: 1,
            flags: ApplyFlags::NONE,
        });

        assert_eq!(context.base_level, TXQ_BASE_LEVEL);
        assert_eq!(context.fee_level_paid, TXQ_BASE_LEVEL);
        assert_eq!(context.required_fee_level, TXQ_BASE_LEVEL);
    }
}
