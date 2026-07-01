//! Deterministic queue-position decisions inside `TxQ::canBeHeld(...)`.
//!
//! This ports only the sequence-order and account-full checks that depend on
//! `SeqProxy`, `TxConsequences`, and `TxQAccount`.

use std::ops::Bound::{Excluded, Unbounded};

use protocol::{SeqProxy, Ter};

use crate::{ApplyFlags, TxQAccount, any_apply_flags};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct QueueHoldPreflight {
    pub has_previous_txn_id: bool,
    pub has_account_txn_id: bool,
    pub has_delegate: bool,
    pub has_fee_sponsor: bool,
    pub flags: ApplyFlags,
    pub last_valid_ledger: Option<u32>,
}

impl QueueHoldPreflight {
    pub const fn new(
        has_previous_txn_id: bool,
        has_account_txn_id: bool,
        flags: ApplyFlags,
        last_valid_ledger: Option<u32>,
    ) -> Self {
        Self {
            has_previous_txn_id,
            has_account_txn_id,
            has_delegate: false,
            has_fee_sponsor: false,
            flags,
            last_valid_ledger,
        }
    }

    pub const fn with_delegate(mut self, has_delegate: bool) -> Self {
        self.has_delegate = has_delegate;
        self
    }

    pub const fn with_fee_sponsor(mut self, has_fee_sponsor: bool) -> Self {
        self.has_fee_sponsor = has_fee_sponsor;
        self
    }
}

/// Mirrors the deterministic front-half rejection checks in
/// `TxQ::canBeHeld(...)` before account-queue inspection.
pub fn check_hold_preconditions(
    preflight: QueueHoldPreflight,
    open_ledger_seq: u32,
    minimum_last_ledger_buffer: u32,
) -> Ter {
    if preflight.has_previous_txn_id
        || preflight.has_account_txn_id
        || any_apply_flags(preflight.flags & ApplyFlags::FAIL_HARD)
    {
        return Ter::TEL_CAN_NOT_QUEUE;
    }

    if preflight.has_delegate {
        return Ter::TEL_CAN_NOT_QUEUE;
    }

    // Disallow sponsored transactions from being queued (PR #7674).
    if preflight.has_fee_sponsor {
        return Ter::TEL_CAN_NOT_QUEUE;
    }

    // than letting Rust's checked addition panic in debug builds.
    let last_ledger_threshold = open_ledger_seq.wrapping_add(minimum_last_ledger_buffer);

    if preflight
        .last_valid_ledger
        .is_some_and(|last_valid| last_valid < last_ledger_threshold)
    {
        return Ter::TEL_CAN_NOT_QUEUE;
    }

    Ter::TES_SUCCESS
}

/// Mirrors the currently ported deterministic admission branches in
/// `TxQ::canBeHeld(...)` while explicitly stopping short of fee policy and
/// other broader queue-engine behavior.
pub fn check_hold_admission<Account, T>(
    preflight: QueueHoldPreflight,
    open_ledger_seq: u32,
    minimum_last_ledger_buffer: u32,
    tx_q_account: Option<&TxQAccount<Account, T>>,
    maximum_txn_per_account: usize,
    replaces_existing: bool,
    tx_seq_proxy: SeqProxy,
    account_seq_proxy: SeqProxy,
) -> Ter {
    let result = check_hold_preconditions(preflight, open_ledger_seq, minimum_last_ledger_buffer);
    if result != Ter::TES_SUCCESS {
        return result;
    }

    let Some(tx_q_account) = tx_q_account else {
        return Ter::TES_SUCCESS;
    };

    if replaces_existing {
        return Ter::TES_SUCCESS;
    }

    check_queue_full(
        tx_q_account,
        maximum_txn_per_account,
        tx_seq_proxy,
        account_seq_proxy,
    )
}

/// Mirrors the current `TxQ::apply(...)` sequence-position checks.
pub fn check_sequence_position<Account, T>(
    tx_q_account: &TxQAccount<Account, T>,
    tx_seq_proxy: SeqProxy,
    account_seq_proxy: SeqProxy,
    replaces_existing: bool,
) -> Ter {
    let prev_tx = tx_q_account.get_prev_tx(tx_seq_proxy);

    if prev_tx.is_none_or(|(prev_seq_proxy, _)| tx_seq_proxy < *prev_seq_proxy) {
        if tx_seq_proxy.is_seq() {
            if tx_seq_proxy < account_seq_proxy {
                return Ter::TEF_PAST_SEQ;
            }
            if tx_seq_proxy > account_seq_proxy {
                return Ter::TER_PRE_SEQ;
            }
        }
    } else if !replaces_existing
        && tx_seq_proxy.is_seq()
        && tx_q_account.next_queuable_seq(account_seq_proxy) != tx_seq_proxy
    {
        return Ter::TEL_CAN_NOT_QUEUE;
    }

    Ter::TES_SUCCESS
}

/// Mirrors the current `TxQ::canBeHeld(...)` queue-full admission check.
pub fn check_queue_full<Account, T>(
    tx_q_account: &TxQAccount<Account, T>,
    maximum_txn_per_account: usize,
    tx_seq_proxy: SeqProxy,
    account_seq_proxy: SeqProxy,
) -> Ter {
    if tx_q_account.get_txn_count() < maximum_txn_per_account {
        return Ter::TES_SUCCESS;
    }

    if tx_seq_proxy.is_ticket() {
        return Ter::TEL_CAN_NOT_QUEUE_FULL;
    }

    let next_queuable = tx_q_account.next_queuable_seq(account_seq_proxy);
    if tx_seq_proxy != next_queuable {
        return Ter::TEL_CAN_NOT_QUEUE_FULL;
    }

    if tx_q_account
        .transactions
        .range((Excluded(next_queuable), Unbounded))
        .next()
        .is_some_and(|(next_seq_proxy, _)| next_seq_proxy.is_seq())
    {
        return Ter::TES_SUCCESS;
    }

    Ter::TEL_CAN_NOT_QUEUE_FULL
}

#[cfg(test)]
mod tests {
    use super::{
        QueueHoldPreflight, check_hold_admission, check_hold_preconditions, check_queue_full,
        check_sequence_position,
    };
    use crate::{ApplyFlags, MaybeTxCore, TxConsequences, TxQAccount};
    use protocol::{SeqProxy, Ter};

    #[test]
    fn hold_preconditions_reject_unsupported_fields_and_fail_hard_flags() {
        assert_eq!(
            check_hold_preconditions(
                QueueHoldPreflight::new(true, false, ApplyFlags::NONE, None),
                100,
                2,
            ),
            Ter::TEL_CAN_NOT_QUEUE
        );
        assert_eq!(
            check_hold_preconditions(
                QueueHoldPreflight::new(false, true, ApplyFlags::NONE, None),
                100,
                2,
            ),
            Ter::TEL_CAN_NOT_QUEUE
        );
        assert_eq!(
            check_hold_preconditions(
                QueueHoldPreflight::new(false, false, ApplyFlags::FAIL_HARD, None),
                100,
                2,
            ),
            Ter::TEL_CAN_NOT_QUEUE
        );
    }

    #[test]
    fn hold_preconditions_enforce_last_valid_buffer() {
        assert_eq!(
            check_hold_preconditions(
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(101)),
                100,
                2,
            ),
            Ter::TEL_CAN_NOT_QUEUE
        );
        assert_eq!(
            check_hold_preconditions(
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(102)),
                100,
                2,
            ),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            check_hold_preconditions(
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                100,
                2,
            ),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn hold_preconditions_wrap_the_last_valid_threshold_unsigned_math() {
        assert_eq!(
            check_hold_preconditions(
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(9)),
                u32::MAX - 1,
                10,
            ),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            check_hold_preconditions(
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(1)),
                u32::MAX - 1,
                10,
            ),
            Ter::TEL_CAN_NOT_QUEUE
        );
    }

    #[test]
    fn hold_preconditions_reject_delegated_transactions() {
        let preflight = QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(200))
            .with_delegate(true);
        assert_eq!(
            check_hold_preconditions(preflight, 100, 2),
            Ter::TEL_CAN_NOT_QUEUE
        );

        let no_delegate = QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(200))
            .with_delegate(false);
        assert_eq!(
            check_hold_preconditions(no_delegate, 100, 2),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn hold_admission_allows_missing_account_and_replacements() {
        let overfull = {
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
            account
        };

        assert_eq!(
            check_hold_admission(
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                100,
                2,
                None::<&TxQAccount<&str, &str>>,
                2,
                false,
                SeqProxy::sequence(7),
                SeqProxy::sequence(5),
            ),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            check_hold_admission(
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                100,
                2,
                Some(&overfull),
                2,
                true,
                SeqProxy::ticket(9),
                SeqProxy::sequence(5),
            ),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn hold_admission_reuses_queue_full_rules_for_existing_accounts() {
        let mut gap_account = TxQAccount::new("acct");
        gap_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 1),
            ),
        );
        gap_account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                "s7",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(7), 1),
            ),
        );

        assert_eq!(
            check_hold_admission(
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                100,
                2,
                Some(&gap_account),
                3,
                false,
                SeqProxy::sequence(6),
                SeqProxy::sequence(5),
            ),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            check_hold_admission(
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                100,
                2,
                Some(&gap_account),
                2,
                false,
                SeqProxy::sequence(6),
                SeqProxy::sequence(5),
            ),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            check_hold_admission(
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                100,
                2,
                Some(&gap_account),
                2,
                false,
                SeqProxy::ticket(1),
                SeqProxy::sequence(5),
            ),
            Ter::TEL_CAN_NOT_QUEUE_FULL
        );
    }

    #[test]
    fn sequence_position_rejects_front_sequence_before_account_sequence() {
        let account = TxQAccount::<_, &str>::new("acct");

        assert_eq!(
            check_sequence_position(
                &account,
                SeqProxy::sequence(4),
                SeqProxy::sequence(5),
                false,
            ),
            Ter::TEF_PAST_SEQ
        );
    }

    #[test]
    fn sequence_position_rejects_front_sequence_after_account_sequence() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::ticket(2),
            MaybeTxCore::new("ticket", TxConsequences::new(1, SeqProxy::ticket(2))),
        );

        assert_eq!(
            check_sequence_position(
                &account,
                SeqProxy::sequence(6),
                SeqProxy::sequence(5),
                false,
            ),
            Ter::TER_PRE_SEQ
        );
    }

    #[test]
    fn sequence_position_requires_non_replacement_to_fill_next_gap() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 1),
            ),
        );
        account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                "s7",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(7), 1),
            ),
        );

        assert_eq!(
            check_sequence_position(
                &account,
                SeqProxy::sequence(8),
                SeqProxy::sequence(5),
                false,
            ),
            Ter::TEL_CAN_NOT_QUEUE
        );
        assert_eq!(
            check_sequence_position(&account, SeqProxy::sequence(8), SeqProxy::sequence(5), true,),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn queue_full_allows_under_limit_accounts() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        assert_eq!(
            check_queue_full(&account, 2, SeqProxy::sequence(6), SeqProxy::sequence(5)),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn queue_full_rejects_ticket_and_non_gap_sequences() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 1),
            ),
        );
        account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                "s7",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(7), 1),
            ),
        );

        assert_eq!(
            check_queue_full(&account, 2, SeqProxy::ticket(1), SeqProxy::sequence(5)),
            Ter::TEL_CAN_NOT_QUEUE_FULL
        );
        assert_eq!(
            check_queue_full(&account, 2, SeqProxy::sequence(8), SeqProxy::sequence(5)),
            Ter::TEL_CAN_NOT_QUEUE_FULL
        );
    }

    #[test]
    fn queue_full_allows_real_gap_but_rejects_tail_topping() {
        let mut gap_account = TxQAccount::new("acct");
        gap_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 1),
            ),
        );
        gap_account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                "s7",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(7), 1),
            ),
        );

        assert_eq!(
            check_queue_full(
                &gap_account,
                2,
                SeqProxy::sequence(6),
                SeqProxy::sequence(5),
            ),
            Ter::TES_SUCCESS
        );

        let mut tail_account = TxQAccount::new("acct");
        tail_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 1),
            ),
        );
        tail_account.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new(
                "s6",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(6), 1),
            ),
        );

        assert_eq!(
            check_queue_full(
                &tail_account,
                2,
                SeqProxy::sequence(7),
                SeqProxy::sequence(5),
            ),
            Ter::TEL_CAN_NOT_QUEUE_FULL
        );
    }
}
