//! Queued-balance gate for `TxQ::apply(...)` before `multiTxn` construction.
//!
//! This helper ports the queued-tail fee/spend accumulation and the
//! `telCAN_NOT_QUEUE_BALANCE` rejection rule.

use protocol::{SeqProxy, Ter};

use crate::{TxConsequences, TxQAccount};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueApplyBalanceTotals {
    pub total_fee_drops: u64,
    pub potential_spend_drops: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueApplyBalanceDecision {
    Allowed(QueueApplyBalanceTotals),
    TotalFeesInFlightTooHigh(QueueApplyBalanceTotals),
}

impl QueueApplyBalanceDecision {
    pub const fn totals(self) -> QueueApplyBalanceTotals {
        match self {
            Self::Allowed(totals) | Self::TotalFeesInFlightTooHigh(totals) => totals,
        }
    }

    pub const fn ter(self) -> Option<Ter> {
        match self {
            Self::Allowed(_) => None,
            Self::TotalFeesInFlightTooHigh(_) => Some(Ter::TEL_CAN_NOT_QUEUE_BALANCE),
        }
    }
}

pub fn evaluate_queue_apply_balance<Account, T>(
    tx_q_account: &TxQAccount<Account, T>,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    candidate_consequences: TxConsequences,
    balance_drops: u64,
    reserve_drops: u64,
    base_fee_drops: u64,
) -> QueueApplyBalanceDecision {
    let mut totals = QueueApplyBalanceTotals::default();
    let mut relevant = tx_q_account
        .transactions
        .range(account_seq_proxy..)
        .peekable();

    while let Some((seq_proxy, queued)) = relevant.next() {
        if *seq_proxy != tx_seq_proxy {
            totals.total_fee_drops = totals
                .total_fee_drops
                .saturating_add(queued.consequences.fee());
            totals.potential_spend_drops = totals
                .potential_spend_drops
                .saturating_add(queued.consequences.potential_spend());
        } else if relevant.peek().is_some() {
            totals.total_fee_drops = totals
                .total_fee_drops
                .saturating_add(candidate_consequences.fee());
            totals.potential_spend_drops = totals
                .potential_spend_drops
                .saturating_add(candidate_consequences.potential_spend());
        }
    }

    if totals.total_fee_drops >= balance_drops
        || (reserve_drops > base_fee_drops.saturating_mul(10)
            && totals.total_fee_drops >= reserve_drops)
    {
        return QueueApplyBalanceDecision::TotalFeesInFlightTooHigh(totals);
    }

    QueueApplyBalanceDecision::Allowed(totals)
}

#[cfg(test)]
mod tests {
    use protocol::{SeqProxy, Ter};

    use super::{QueueApplyBalanceDecision, QueueApplyBalanceTotals, evaluate_queue_apply_balance};
    use crate::{MaybeTxCore, TxConsequences, TxQAccount};

    #[test]
    fn balance_gate_accumulates_relevant_tail_and_replacement_in_middle() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_potential_spend(10, SeqProxy::sequence(5), 100),
            ),
        );
        account.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new(
                "s6",
                TxConsequences::with_potential_spend(20, SeqProxy::sequence(6), 200),
            ),
        );
        account.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new(
                "s8",
                TxConsequences::with_potential_spend(30, SeqProxy::sequence(8), 300),
            ),
        );

        let decision = evaluate_queue_apply_balance(
            &account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(6),
            TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
            1_000,
            500,
            10,
        );

        assert_eq!(
            decision,
            QueueApplyBalanceDecision::Allowed(QueueApplyBalanceTotals {
                total_fee_drops: 90,
                potential_spend_drops: 900,
            })
        );
        assert_eq!(decision.ter(), None);
    }

    #[test]
    fn balance_gate_does_not_count_replacement_when_it_is_the_last_relevant_entry() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_potential_spend(10, SeqProxy::sequence(5), 100),
            ),
        );
        account.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new(
                "s6",
                TxConsequences::with_potential_spend(20, SeqProxy::sequence(6), 200),
            ),
        );

        let decision = evaluate_queue_apply_balance(
            &account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(6),
            TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
            1_000,
            500,
            10,
        );

        assert_eq!(
            decision.totals(),
            QueueApplyBalanceTotals {
                total_fee_drops: 10,
                potential_spend_drops: 100,
            }
        );
    }

    #[test]
    fn balance_gate_rejects_when_total_fees_reach_balance_or_reserve_threshold() {
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
                TxConsequences::with_potential_spend(50, SeqProxy::sequence(7), 100),
            ),
        );

        let balance_fail = evaluate_queue_apply_balance(
            &account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(9),
            TxConsequences::with_potential_spend(1, SeqProxy::sequence(9), 1),
            120,
            1_000,
            10,
        );
        assert_eq!(
            balance_fail,
            QueueApplyBalanceDecision::TotalFeesInFlightTooHigh(QueueApplyBalanceTotals {
                total_fee_drops: 120,
                potential_spend_drops: 200,
            })
        );
        assert_eq!(balance_fail.ter(), Some(Ter::TEL_CAN_NOT_QUEUE_BALANCE));

        let reserve_fail = evaluate_queue_apply_balance(
            &account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(9),
            TxConsequences::with_potential_spend(1, SeqProxy::sequence(9), 1),
            1_000,
            100,
            5,
        );
        assert_eq!(
            reserve_fail,
            QueueApplyBalanceDecision::TotalFeesInFlightTooHigh(QueueApplyBalanceTotals {
                total_fee_drops: 120,
                potential_spend_drops: 200,
            })
        );
    }

    #[test]
    fn balance_gate_ignores_reserve_when_it_is_not_far_above_base_fee() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_potential_spend(70, SeqProxy::sequence(5), 100),
            ),
        );

        let decision = evaluate_queue_apply_balance(
            &account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(9),
            TxConsequences::with_potential_spend(1, SeqProxy::sequence(9), 1),
            1_000,
            100,
            10,
        );

        assert_eq!(
            decision,
            QueueApplyBalanceDecision::Allowed(QueueApplyBalanceTotals {
                total_fee_drops: 70,
                potential_spend_drops: 100,
            })
        );
    }
}
