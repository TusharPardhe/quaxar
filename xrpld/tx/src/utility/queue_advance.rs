//! Candidate-selection half of `TxQ::eraseAndAdvance(...)`.
//!
//! This ports the deterministic rule that decides whether the next candidate
//! after removal should come from the same account or from the next fee-ordered
//! queue position.

use basics::base_uint::Uint256;
use protocol::SeqProxy;

use crate::{FeeLevel64, MaybeTx, OrderCandidates};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueAdvanceCandidate {
    pub fee_level: FeeLevel64,
    pub tx_id: Uint256,
    pub seq_proxy: SeqProxy,
}

impl<Tx, Account, Journal, ParentBatchId> From<&MaybeTx<Tx, Account, Journal, ParentBatchId>>
    for QueueAdvanceCandidate
{
    fn from(value: &MaybeTx<Tx, Account, Journal, ParentBatchId>) -> Self {
        Self {
            fee_level: value.fee_level,
            tx_id: value.tx_id,
            seq_proxy: value.seq_proxy,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvanceTarget {
    End,
    FeeNext(QueueAdvanceCandidate),
    AccountNext(QueueAdvanceCandidate),
}

pub fn choose_next_after_erase(
    current: QueueAdvanceCandidate,
    current_is_first_for_account: bool,
    account_next: Option<QueueAdvanceCandidate>,
    fee_next: Option<QueueAdvanceCandidate>,
    order: &OrderCandidates,
) -> AdvanceTarget {
    assert!(
        current.seq_proxy.is_ticket() || current_is_first_for_account,
        "xrpl::TxQ::eraseAndAdvance : ticket or sequence"
    );

    if let Some(account_next) = account_next
        && account_next.seq_proxy > current.seq_proxy
        && fee_next.is_none_or(|fee_next| {
            order.compares_by_fee_and_tx_id(
                account_next.fee_level,
                account_next.tx_id,
                fee_next.fee_level,
                fee_next.tx_id,
            )
        })
    {
        return AdvanceTarget::AccountNext(account_next);
    }

    fee_next.map_or(AdvanceTarget::End, AdvanceTarget::FeeNext)
}

#[cfg(test)]
mod tests {
    use basics::base_uint::Uint256;
    use protocol::SeqProxy;

    use super::{AdvanceTarget, QueueAdvanceCandidate, choose_next_after_erase};
    use crate::OrderCandidates;

    fn candidate(seq_proxy: SeqProxy, tx_id: u64, fee_level: u64) -> QueueAdvanceCandidate {
        QueueAdvanceCandidate {
            fee_level,
            tx_id: Uint256::from_u64(tx_id),
            seq_proxy,
        }
    }

    #[test]
    fn chooses_next_account_candidate_when_it_would_sort_before_fee_next() {
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let current = candidate(SeqProxy::sequence(5), 1, 100);
        let account_next = candidate(SeqProxy::sequence(6), 9, 95);
        let fee_next = candidate(SeqProxy::ticket(1), 7, 90);

        assert_eq!(
            choose_next_after_erase(current, true, Some(account_next), Some(fee_next), &order),
            AdvanceTarget::AccountNext(account_next)
        );
    }

    #[test]
    fn chooses_fee_next_when_it_stays_ahead_of_next_account_candidate() {
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let current = candidate(SeqProxy::sequence(5), 1, 100);
        let account_next = candidate(SeqProxy::sequence(6), 9, 90);
        let fee_next = candidate(SeqProxy::ticket(1), 7, 95);

        assert_eq!(
            choose_next_after_erase(current, true, Some(account_next), Some(fee_next), &order),
            AdvanceTarget::FeeNext(fee_next)
        );
    }

    #[test]
    fn chooses_next_account_candidate_when_fee_queue_is_exhausted() {
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let current = candidate(SeqProxy::ticket(5), 1, 100);
        let account_next = candidate(SeqProxy::ticket(7), 9, 90);

        assert_eq!(
            choose_next_after_erase(current, false, Some(account_next), None, &order),
            AdvanceTarget::AccountNext(account_next)
        );
    }

    #[test]
    fn returns_end_when_no_follow_up_candidate_exists() {
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let current = candidate(SeqProxy::ticket(5), 1, 100);

        assert_eq!(
            choose_next_after_erase(current, false, None, None, &order),
            AdvanceTarget::End
        );
    }

    #[test]
    #[should_panic(expected = "xrpl::TxQ::eraseAndAdvance : ticket or sequence")]
    fn sequence_candidates_must_be_first_for_their_account() {
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let current = candidate(SeqProxy::sequence(5), 1, 100);

        let _ = choose_next_after_erase(current, false, None, None, &order);
    }
}
