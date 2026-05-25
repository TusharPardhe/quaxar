//! Pre-apply gate for `TxQ::accept(...)`.
//!
//! This ports only the deterministic decision that says whether the current
//! queued candidate should be skipped because it is not the first sequence for
//! its account, should stop the fee-ordered walk because it is below the
//! required fee level, or is eligible to proceed to the later apply branch.

use protocol::SeqProxy;

use crate::FeeLevel64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptCandidateGate {
    SkipNotFirst,
    TryApply,
    StopInsufficientFee,
}

pub fn evaluate_accept_candidate(
    candidate_seq_proxy: SeqProxy,
    account_front_seq_proxy: SeqProxy,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
) -> AcceptCandidateGate {
    if candidate_seq_proxy.is_seq() && candidate_seq_proxy > account_front_seq_proxy {
        return AcceptCandidateGate::SkipNotFirst;
    }

    if fee_level_paid >= required_fee_level {
        AcceptCandidateGate::TryApply
    } else {
        AcceptCandidateGate::StopInsufficientFee
    }
}

#[cfg(test)]
mod tests {
    use protocol::SeqProxy;

    use super::{AcceptCandidateGate, evaluate_accept_candidate};

    #[test]
    fn later_sequence_candidates_are_skipped_until_the_front_moves() {
        assert_eq!(
            evaluate_accept_candidate(SeqProxy::sequence(7), SeqProxy::sequence(5), 100, 90),
            AcceptCandidateGate::SkipNotFirst
        );
    }

    #[test]
    fn ticket_candidates_do_not_take_the_sequence_skip_path() {
        assert_eq!(
            evaluate_accept_candidate(SeqProxy::ticket(7), SeqProxy::sequence(5), 100, 90),
            AcceptCandidateGate::TryApply
        );
    }

    #[test]
    fn candidates_with_enough_fee_are_eligible_to_try_apply() {
        assert_eq!(
            evaluate_accept_candidate(SeqProxy::sequence(5), SeqProxy::sequence(5), 90, 90),
            AcceptCandidateGate::TryApply
        );
    }

    #[test]
    fn candidates_below_required_fee_stop_iteration() {
        assert_eq!(
            evaluate_accept_candidate(SeqProxy::sequence(5), SeqProxy::sequence(5), 89, 90),
            AcceptCandidateGate::StopInsufficientFee
        );
    }
}
