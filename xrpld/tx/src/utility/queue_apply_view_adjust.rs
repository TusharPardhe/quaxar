//! Staged `multiTxn` account adjustment inside `TxQ::apply(...)` before
//! `preclaim(...)`.
//!
//! This ports only the current `potentialTotalSpend` calculation, the
//! `XRPL_ASSERT` invariant on that value, and the temporary account sequence
//! selection used in the sandbox view.

use protocol::SeqProxy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueApplyViewAdjustment {
    pub potential_total_spend_drops: i64,
    pub adjusted_balance_drops: i64,
    pub applied_sequence_value: u32,
}

fn to_i64_drops(drops: u64, label: &str) -> i64 {
    i64::try_from(drops).unwrap_or_else(|_| {
        panic!("{label} exceeds signed XRPAmount range in narrowed TxQ apply port")
    })
}

pub fn evaluate_queue_apply_view_adjustment(
    total_fee_drops: u64,
    potential_spend_drops: u64,
    balance_drops: u64,
    reserve_drops: u64,
    base_fee_drops: u64,
    tx_seq_proxy: SeqProxy,
    next_queuable_seq: SeqProxy,
) -> QueueApplyViewAdjustment {
    let total_fee_drops = to_i64_drops(total_fee_drops, "total_fee_drops");
    let potential_spend_drops = to_i64_drops(potential_spend_drops, "potential_spend_drops");
    let balance_drops = to_i64_drops(balance_drops, "balance_drops");
    let reserve_drops = to_i64_drops(reserve_drops, "reserve_drops");
    let base_fee_drops = to_i64_drops(base_fee_drops, "base_fee_drops");

    let spendable_balance_drops = balance_drops - balance_drops.min(reserve_drops);
    let potential_total_spend_drops =
        total_fee_drops + spendable_balance_drops.min(potential_spend_drops);

    assert!(
        potential_total_spend_drops > 0
            || (potential_total_spend_drops == 0 && base_fee_drops == 0),
        "xrpl::TxQ::apply : total spend check"
    );

    QueueApplyViewAdjustment {
        potential_total_spend_drops,
        adjusted_balance_drops: balance_drops - potential_total_spend_drops,
        applied_sequence_value: if tx_seq_proxy.is_seq() {
            tx_seq_proxy.value()
        } else {
            next_queuable_seq.value()
        },
    }
}

#[cfg(test)]
mod tests {
    use protocol::SeqProxy;

    use super::{QueueApplyViewAdjustment, evaluate_queue_apply_view_adjustment};

    #[test]
    fn view_adjustment_matches_current_cpp_formula_for_sequence_transactions() {
        let adjustment = evaluate_queue_apply_view_adjustment(
            90,
            700,
            1_000,
            300,
            10,
            SeqProxy::sequence(11),
            SeqProxy::sequence(13),
        );

        assert_eq!(
            adjustment,
            QueueApplyViewAdjustment {
                potential_total_spend_drops: 790,
                adjusted_balance_drops: 210,
                applied_sequence_value: 11,
            }
        );
    }

    #[test]
    fn view_adjustment_uses_next_queuable_sequence_for_ticket_transactions() {
        let adjustment = evaluate_queue_apply_view_adjustment(
            40,
            150,
            500,
            200,
            10,
            SeqProxy::ticket(7),
            SeqProxy::sequence(23),
        );

        assert_eq!(adjustment.applied_sequence_value, 23);
    }

    #[test]
    fn view_adjustment_preserves_negative_sandbox_balances_when_reserve_gate_is_ignored() {
        let adjustment = evaluate_queue_apply_view_adjustment(
            900,
            1_000,
            1_000,
            50,
            10,
            SeqProxy::sequence(12),
            SeqProxy::sequence(12),
        );

        assert_eq!(
            adjustment,
            QueueApplyViewAdjustment {
                potential_total_spend_drops: 1_850,
                adjusted_balance_drops: -850,
                applied_sequence_value: 12,
            }
        );
    }

    #[test]
    fn view_adjustment_allows_zero_spend_only_when_base_fee_is_zero() {
        let adjustment = evaluate_queue_apply_view_adjustment(
            0,
            0,
            0,
            0,
            0,
            SeqProxy::sequence(1),
            SeqProxy::sequence(1),
        );

        assert_eq!(adjustment.potential_total_spend_drops, 0);
        assert_eq!(adjustment.adjusted_balance_drops, 0);
    }

    #[test]
    #[should_panic(expected = "xrpl::TxQ::apply : total spend check")]
    fn view_adjustment_rejects_zero_total_spend_when_base_fee_is_nonzero() {
        let _ = evaluate_queue_apply_view_adjustment(
            0,
            0,
            0,
            0,
            10,
            SeqProxy::sequence(1),
            SeqProxy::sequence(1),
        );
    }
}
