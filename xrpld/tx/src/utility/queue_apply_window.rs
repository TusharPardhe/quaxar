//! Account-queue window inspection immediately after the early
//! `TxQ::apply(...)` guards.
//!
//! This ports only the current `lower_bound(acctSeqProx)` tail-window view,
//! replacement lookup by exact `SeqProxy`, and the rule that an already-queued
//! blocker may only be replaced by that exact queued entry.

use protocol::{SeqProxy, Ter};

use crate::TxQAccount;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccountQueueWindow {
    pub account_is_in_queue: bool,
    pub first_relevant_seq_proxy: Option<SeqProxy>,
    pub relevant_tx_count: usize,
    pub replaces_existing: bool,
    pub front_is_blocker: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuedBlockerAdmission {
    Allowed,
    BlockedByQueuedBlocker,
}

impl QueuedBlockerAdmission {
    pub const fn ter(self) -> Option<Ter> {
        match self {
            Self::Allowed => None,
            Self::BlockedByQueuedBlocker => Some(Ter::TEL_CAN_NOT_QUEUE_BLOCKED),
        }
    }
}

pub fn inspect_account_queue_window<Account, T>(
    tx_q_account: Option<&TxQAccount<Account, T>>,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
) -> AccountQueueWindow {
    let Some(tx_q_account) = tx_q_account else {
        return AccountQueueWindow::default();
    };

    let mut relevant = tx_q_account.transactions.range(account_seq_proxy..);
    let first_relevant = relevant
        .next()
        .map(|(seq_proxy, queued)| (*seq_proxy, queued));
    let relevant_tx_count = first_relevant
        .map(|_| 1 + relevant.count())
        .unwrap_or_default();

    AccountQueueWindow {
        account_is_in_queue: true,
        first_relevant_seq_proxy: first_relevant.map(|(seq_proxy, _)| seq_proxy),
        relevant_tx_count,
        replaces_existing: tx_q_account.transactions.contains_key(&tx_seq_proxy),
        front_is_blocker: first_relevant
            .map(|(_, queued)| queued.consequences.is_blocker())
            .unwrap_or(false),
    }
}

pub fn evaluate_queued_blocker_admission(
    window: AccountQueueWindow,
    tx_seq_proxy: SeqProxy,
) -> QueuedBlockerAdmission {
    if window.relevant_tx_count == 1
        && window.front_is_blocker
        && window
            .first_relevant_seq_proxy
            .is_some_and(|front_seq_proxy| front_seq_proxy != tx_seq_proxy)
    {
        return QueuedBlockerAdmission::BlockedByQueuedBlocker;
    }

    QueuedBlockerAdmission::Allowed
}

#[cfg(test)]
mod tests {
    use protocol::{SeqProxy, Ter};

    use super::{
        AccountQueueWindow, QueuedBlockerAdmission, evaluate_queued_blocker_admission,
        inspect_account_queue_window,
    };
    use crate::{MaybeTxCore, TxConsequences, TxConsequencesCategory, TxQAccount};

    #[test]
    fn inspect_account_queue_window_returns_empty_state_when_account_is_not_queued() {
        let window = inspect_account_queue_window::<&str, &str>(
            None,
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
        );

        assert_eq!(window, AccountQueueWindow::default());
    }

    #[test]
    fn inspect_account_queue_window_uses_lower_bound_tail_but_replacement_checks_full_queue() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new("s7", TxConsequences::new(1, SeqProxy::sequence(7))),
        );
        account.add(
            SeqProxy::ticket(3),
            MaybeTxCore::new("t3", TxConsequences::new(1, SeqProxy::ticket(3))),
        );

        let active = inspect_account_queue_window(
            Some(&account),
            SeqProxy::sequence(6),
            SeqProxy::sequence(7),
        );
        assert_eq!(
            active,
            AccountQueueWindow {
                account_is_in_queue: true,
                first_relevant_seq_proxy: Some(SeqProxy::sequence(7)),
                relevant_tx_count: 2,
                replaces_existing: true,
                front_is_blocker: false,
            }
        );

        let mut slipped_account = TxQAccount::new("acct");
        slipped_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        slipped_account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new("s7", TxConsequences::new(1, SeqProxy::sequence(7))),
        );

        let slipped_only = inspect_account_queue_window(
            Some(&slipped_account),
            SeqProxy::sequence(8),
            SeqProxy::sequence(5),
        );
        assert_eq!(
            slipped_only,
            AccountQueueWindow {
                account_is_in_queue: true,
                first_relevant_seq_proxy: None,
                relevant_tx_count: 0,
                replaces_existing: true,
                front_is_blocker: false,
            }
        );
    }

    #[test]
    fn inspect_account_queue_window_reports_front_blocker_from_relevant_tail_only() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "blocker",
                TxConsequences::with_category(
                    1,
                    SeqProxy::sequence(5),
                    TxConsequencesCategory::Blocker,
                ),
            ),
        );
        account.add(
            SeqProxy::ticket(3),
            MaybeTxCore::new("t3", TxConsequences::new(1, SeqProxy::ticket(3))),
        );

        let visible = inspect_account_queue_window(
            Some(&account),
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
        );
        assert!(visible.front_is_blocker);

        let ignored = inspect_account_queue_window(
            Some(&account),
            SeqProxy::sequence(6),
            SeqProxy::ticket(3),
        );
        assert!(!ignored.front_is_blocker);
        assert_eq!(ignored.first_relevant_seq_proxy, Some(SeqProxy::ticket(3)));
        assert_eq!(ignored.relevant_tx_count, 1);
    }

    #[test]
    fn queued_blocker_admission_matches_current_cpp_rule() {
        let blocked = evaluate_queued_blocker_admission(
            AccountQueueWindow {
                account_is_in_queue: true,
                first_relevant_seq_proxy: Some(SeqProxy::sequence(5)),
                relevant_tx_count: 1,
                replaces_existing: false,
                front_is_blocker: true,
            },
            SeqProxy::sequence(7),
        );
        let replacement = evaluate_queued_blocker_admission(
            AccountQueueWindow {
                account_is_in_queue: true,
                first_relevant_seq_proxy: Some(SeqProxy::sequence(5)),
                relevant_tx_count: 1,
                replaces_existing: true,
                front_is_blocker: true,
            },
            SeqProxy::sequence(5),
        );
        let normal = evaluate_queued_blocker_admission(
            AccountQueueWindow {
                account_is_in_queue: true,
                first_relevant_seq_proxy: Some(SeqProxy::sequence(5)),
                relevant_tx_count: 2,
                replaces_existing: false,
                front_is_blocker: true,
            },
            SeqProxy::sequence(7),
        );

        assert_eq!(blocked, QueuedBlockerAdmission::BlockedByQueuedBlocker);
        assert_eq!(blocked.ter(), Some(Ter::TEL_CAN_NOT_QUEUE_BLOCKED));
        assert_eq!(replacement, QueuedBlockerAdmission::Allowed);
        assert_eq!(normal, QueuedBlockerAdmission::Allowed);
    }
}
