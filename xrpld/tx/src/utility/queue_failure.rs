//! Deterministic failed-candidate policy inside `TxQ::accept(...)`.
//!
//! This ports only the queue-state decision logic after a queued transaction
//! fails to apply.

use protocol::{SeqProxy, Ter, is_tef_failure, is_tem_malformed};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PenaltyUpdate {
    None,
    MarkDropPenalty,
    MarkRetryPenalty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailedCandidateAction {
    RemoveCurrent,
    KeepQueued,
    DropCurrentTicket,
    DropLastFromAccount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailedCandidateDecision {
    pub action: FailedCandidateAction,
    pub penalty_update: PenaltyUpdate,
    pub next_retries_remaining: i32,
    pub last_result: Option<Ter>,
}

pub fn decide_failed_candidate(
    txn_result: Ter,
    retries_remaining: i32,
    account_retry_penalty: bool,
    account_drop_penalty: bool,
    account_txn_count: usize,
    queue_nearly_full: bool,
    seq_proxy: SeqProxy,
) -> FailedCandidateDecision {
    if is_tef_failure(txn_result) || is_tem_malformed(txn_result) || retries_remaining <= 0 {
        let penalty_update = if retries_remaining <= 0 {
            PenaltyUpdate::MarkRetryPenalty
        } else {
            PenaltyUpdate::MarkDropPenalty
        };

        return FailedCandidateDecision {
            action: FailedCandidateAction::RemoveCurrent,
            penalty_update,
            next_retries_remaining: retries_remaining,
            last_result: None,
        };
    }

    let next_retries_remaining = if account_retry_penalty && retries_remaining > 2 {
        1
    } else {
        retries_remaining - 1
    };

    let action = if account_drop_penalty && account_txn_count > 1 && queue_nearly_full {
        if seq_proxy.is_ticket() {
            FailedCandidateAction::DropCurrentTicket
        } else {
            FailedCandidateAction::DropLastFromAccount
        }
    } else {
        FailedCandidateAction::KeepQueued
    };

    FailedCandidateDecision {
        action,
        penalty_update: PenaltyUpdate::None,
        next_retries_remaining,
        last_result: Some(txn_result),
    }
}

#[cfg(test)]
mod tests {
    use super::{FailedCandidateAction, PenaltyUpdate, decide_failed_candidate};
    use protocol::{SeqProxy, Ter};

    #[test]
    fn hard_failures_remove_current_and_mark_drop_penalty() {
        let tef = decide_failed_candidate(
            Ter::TEF_EXCEPTION,
            5,
            false,
            false,
            1,
            false,
            SeqProxy::sequence(5),
        );
        let tem = decide_failed_candidate(
            Ter::TEM_MALFORMED,
            5,
            false,
            false,
            1,
            false,
            SeqProxy::sequence(5),
        );

        assert_eq!(tef.action, FailedCandidateAction::RemoveCurrent);
        assert_eq!(tef.penalty_update, PenaltyUpdate::MarkDropPenalty);
        assert_eq!(tef.last_result, None);

        assert_eq!(tem.action, FailedCandidateAction::RemoveCurrent);
        assert_eq!(tem.penalty_update, PenaltyUpdate::MarkDropPenalty);
    }

    #[test]
    fn exhausted_retries_remove_current_and_mark_retry_penalty() {
        let decision = decide_failed_candidate(
            Ter::TER_RETRY,
            0,
            false,
            false,
            1,
            false,
            SeqProxy::sequence(5),
        );

        assert_eq!(decision.action, FailedCandidateAction::RemoveCurrent);
        assert_eq!(decision.penalty_update, PenaltyUpdate::MarkRetryPenalty);
    }

    #[test]
    fn exhausted_retries_take_retry_penalty_even_when_result_is_hard_failure() {
        let decision = decide_failed_candidate(
            Ter::TEF_EXCEPTION,
            0,
            false,
            false,
            1,
            false,
            SeqProxy::sequence(5),
        );

        assert_eq!(decision.action, FailedCandidateAction::RemoveCurrent);
        assert_eq!(decision.penalty_update, PenaltyUpdate::MarkRetryPenalty);
        assert_eq!(decision.last_result, None);
    }

    #[test]
    fn soft_failures_update_retry_count_and_record_last_result() {
        let retry_penalty = decide_failed_candidate(
            Ter::TER_RETRY,
            7,
            true,
            false,
            1,
            false,
            SeqProxy::sequence(5),
        );
        let normal = decide_failed_candidate(
            Ter::TER_RETRY,
            2,
            false,
            false,
            1,
            false,
            SeqProxy::sequence(5),
        );

        assert_eq!(retry_penalty.action, FailedCandidateAction::KeepQueued);
        assert_eq!(retry_penalty.next_retries_remaining, 1);
        assert_eq!(retry_penalty.last_result, Some(Ter::TER_RETRY));

        assert_eq!(normal.action, FailedCandidateAction::KeepQueued);
        assert_eq!(normal.next_retries_remaining, 1);
        assert_eq!(normal.last_result, Some(Ter::TER_RETRY));
    }

    #[test]
    fn near_full_drop_penalty_drops_ticket_or_last_account_item() {
        let ticket =
            decide_failed_candidate(Ter::TER_RETRY, 4, false, true, 2, true, SeqProxy::ticket(4));
        let sequence = decide_failed_candidate(
            Ter::TER_RETRY,
            4,
            false,
            true,
            2,
            true,
            SeqProxy::sequence(4),
        );

        assert_eq!(ticket.action, FailedCandidateAction::DropCurrentTicket);
        assert_eq!(sequence.action, FailedCandidateAction::DropLastFromAccount);
    }
}
