//! Deterministic early-exit guards near the top of `TxQ::apply(...)` after
//! direct apply is attempted.
//!
//! This ports only the missing-account, missing-ticket, and blocker-only
//! queue-admission rules before the broader queued-transaction path.

use protocol::{SeqProxy, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueApplyPrerequisite {
    Ready,
    MissingAccount,
    MissingTicketPast,
    MissingTicketFuture,
}

impl QueueApplyPrerequisite {
    pub const fn ter(self) -> Option<Ter> {
        match self {
            Self::Ready => None,
            Self::MissingAccount => Some(Ter::TER_NO_ACCOUNT),
            Self::MissingTicketPast => Some(Ter::TEF_NO_TICKET),
            Self::MissingTicketFuture => Some(Ter::TER_PRE_TICKET),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockerQueueAdmission {
    Allowed,
    RejectsCoResidentQueue,
    RejectsNonReplacementOfLoneEntry,
}

impl BlockerQueueAdmission {
    pub const fn ter(self) -> Option<Ter> {
        match self {
            Self::Allowed => None,
            Self::RejectsCoResidentQueue | Self::RejectsNonReplacementOfLoneEntry => {
                Some(Ter::TEL_CAN_NOT_QUEUE_BLOCKS)
            }
        }
    }
}

pub fn evaluate_queue_apply_prerequisite(
    account_exists: bool,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    ticket_exists: bool,
) -> QueueApplyPrerequisite {
    if !account_exists {
        return QueueApplyPrerequisite::MissingAccount;
    }

    if tx_seq_proxy.is_ticket() && !ticket_exists {
        if tx_seq_proxy.value() < account_seq_proxy.value() {
            return QueueApplyPrerequisite::MissingTicketPast;
        }
        return QueueApplyPrerequisite::MissingTicketFuture;
    }

    QueueApplyPrerequisite::Ready
}

pub fn evaluate_blocker_queue_admission(
    is_blocker: bool,
    account_tx_count: usize,
    tx_seq_proxy: SeqProxy,
    queued_front_seq_proxy: Option<SeqProxy>,
) -> BlockerQueueAdmission {
    if !is_blocker {
        return BlockerQueueAdmission::Allowed;
    }

    if account_tx_count > 1 {
        return BlockerQueueAdmission::RejectsCoResidentQueue;
    }

    if account_tx_count == 1
        && queued_front_seq_proxy.is_some_and(|queued_seq_proxy| queued_seq_proxy != tx_seq_proxy)
    {
        return BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry;
    }

    BlockerQueueAdmission::Allowed
}

#[cfg(test)]
mod tests {
    use protocol::{SeqProxy, Ter};

    use super::{
        BlockerQueueAdmission, QueueApplyPrerequisite, evaluate_blocker_queue_admission,
        evaluate_queue_apply_prerequisite,
    };

    #[test]
    fn queue_apply_prerequisite_requires_the_source_account_to_exist() {
        let result = evaluate_queue_apply_prerequisite(
            false,
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            false,
        );

        assert_eq!(result, QueueApplyPrerequisite::MissingAccount);
        assert_eq!(result.ter(), Some(Ter::TER_NO_ACCOUNT));
    }

    #[test]
    fn queue_apply_prerequisite_distinguishes_past_and_future_missing_tickets() {
        let past = evaluate_queue_apply_prerequisite(
            true,
            SeqProxy::sequence(8),
            SeqProxy::ticket(7),
            false,
        );
        let future = evaluate_queue_apply_prerequisite(
            true,
            SeqProxy::sequence(8),
            SeqProxy::ticket(9),
            false,
        );

        assert_eq!(past, QueueApplyPrerequisite::MissingTicketPast);
        assert_eq!(past.ter(), Some(Ter::TEF_NO_TICKET));
        assert_eq!(future, QueueApplyPrerequisite::MissingTicketFuture);
        assert_eq!(future.ter(), Some(Ter::TER_PRE_TICKET));
    }

    #[test]
    fn queue_apply_prerequisite_allows_sequence_transactions_and_existing_tickets() {
        assert_eq!(
            evaluate_queue_apply_prerequisite(
                true,
                SeqProxy::sequence(8),
                SeqProxy::sequence(8),
                false,
            ),
            QueueApplyPrerequisite::Ready
        );
        assert_eq!(
            evaluate_queue_apply_prerequisite(
                true,
                SeqProxy::sequence(8),
                SeqProxy::ticket(9),
                true,
            ),
            QueueApplyPrerequisite::Ready
        );
    }

    #[test]
    fn blocker_queue_admission_allows_non_blockers_and_valid_replacements() {
        assert_eq!(
            evaluate_blocker_queue_admission(false, 3, SeqProxy::sequence(5), None),
            BlockerQueueAdmission::Allowed
        );
        assert_eq!(
            evaluate_blocker_queue_admission(
                true,
                1,
                SeqProxy::sequence(5),
                Some(SeqProxy::sequence(5)),
            ),
            BlockerQueueAdmission::Allowed
        );
    }

    #[test]
    fn blocker_queue_admission_rejects_blockers_with_other_queued_transactions() {
        let crowded = evaluate_blocker_queue_admission(true, 2, SeqProxy::sequence(5), None);
        let non_replacement = evaluate_blocker_queue_admission(
            true,
            1,
            SeqProxy::sequence(5),
            Some(SeqProxy::sequence(7)),
        );

        assert_eq!(crowded, BlockerQueueAdmission::RejectsCoResidentQueue);
        assert_eq!(crowded.ter(), Some(Ter::TEL_CAN_NOT_QUEUE_BLOCKS));
        assert_eq!(
            non_replacement,
            BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry
        );
        assert_eq!(non_replacement.ter(), Some(Ter::TEL_CAN_NOT_QUEUE_BLOCKS));
    }
}
