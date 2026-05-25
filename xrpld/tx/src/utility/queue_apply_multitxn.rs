//! Pre-`multiTxn` admission checks inside `TxQ::apply(...)`.
//!
//! This ports only the current rule that queued-account paths rerun
//! `canBeHeld(...)` and the sequence-fit check iff that queued path requires a
//! `multiTxn` view.

use protocol::{SeqProxy, Ter};

use crate::{
    QueueApplyPath, QueueHoldPreflight, TxQAccount, check_hold_admission, check_sequence_position,
};

pub fn evaluate_queue_apply_multitxn_admission<Account, T>(
    path: QueueApplyPath,
    preflight: QueueHoldPreflight,
    open_ledger_seq: u32,
    minimum_last_ledger_buffer: u32,
    tx_q_account: &TxQAccount<Account, T>,
    maximum_txn_per_account: usize,
    tx_seq_proxy: SeqProxy,
    account_seq_proxy: SeqProxy,
    replaces_existing: bool,
) -> Result<QueueApplyPath, Ter> {
    if !path.requires_multi_txn() {
        return Ok(path);
    }

    let ter = check_hold_admission(
        preflight,
        open_ledger_seq,
        minimum_last_ledger_buffer,
        Some(tx_q_account),
        maximum_txn_per_account,
        replaces_existing,
        tx_seq_proxy,
        account_seq_proxy,
    );
    if ter != Ter::TES_SUCCESS {
        return Err(ter);
    }

    let ter = check_sequence_position(
        tx_q_account,
        tx_seq_proxy,
        account_seq_proxy,
        replaces_existing,
    );
    if ter != Ter::TES_SUCCESS {
        return Err(ter);
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use protocol::{SeqProxy, Ter};

    use super::evaluate_queue_apply_multitxn_admission;
    use crate::{
        ApplyFlags, MaybeTxCore, QueueApplyPath, QueueHoldPreflight, TxConsequences, TxQAccount,
    };

    #[test]
    fn open_ledger_and_single_replacement_paths_bypass_multitxn_admission_checks() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let invalid_preflight =
            QueueHoldPreflight::new(true, false, ApplyFlags::FAIL_HARD, Some(1));

        assert_eq!(
            evaluate_queue_apply_multitxn_admission(
                QueueApplyPath::OpenLedger,
                invalid_preflight,
                100,
                2,
                &account,
                1,
                SeqProxy::sequence(9),
                SeqProxy::sequence(5),
                false,
            ),
            Ok(QueueApplyPath::OpenLedger)
        );
        assert_eq!(
            evaluate_queue_apply_multitxn_admission(
                QueueApplyPath::QueuedAccount {
                    requires_multi_txn: false
                },
                invalid_preflight,
                100,
                2,
                &account,
                1,
                SeqProxy::sequence(9),
                SeqProxy::sequence(5),
                true,
            ),
            Ok(QueueApplyPath::QueuedAccount {
                requires_multi_txn: false
            })
        );
    }

    #[test]
    fn multitxn_path_runs_hold_admission_before_sequence_fit_checks() {
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

        assert_eq!(
            evaluate_queue_apply_multitxn_admission(
                QueueApplyPath::QueuedAccount {
                    requires_multi_txn: true
                },
                QueueHoldPreflight::new(true, false, ApplyFlags::NONE, None),
                100,
                2,
                &account,
                2,
                SeqProxy::sequence(8),
                SeqProxy::sequence(5),
                false,
            ),
            Err(Ter::TEL_CAN_NOT_QUEUE)
        );
    }

    #[test]
    fn multitxn_path_propagates_sequence_fit_failures_after_hold_admission_succeeds() {
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
            evaluate_queue_apply_multitxn_admission(
                QueueApplyPath::QueuedAccount {
                    requires_multi_txn: true
                },
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                100,
                2,
                &account,
                3,
                SeqProxy::sequence(8),
                SeqProxy::sequence(5),
                false,
            ),
            Err(Ter::TEL_CAN_NOT_QUEUE)
        );
    }

    #[test]
    fn multitxn_path_allows_replacement_or_gap_fill_when_checks_pass() {
        let mut replacement_account = TxQAccount::new("acct");
        replacement_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 1),
            ),
        );
        replacement_account.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new(
                "s6",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(6), 1),
            ),
        );

        assert_eq!(
            evaluate_queue_apply_multitxn_admission(
                QueueApplyPath::QueuedAccount {
                    requires_multi_txn: true
                },
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                100,
                2,
                &replacement_account,
                2,
                SeqProxy::sequence(6),
                SeqProxy::sequence(5),
                true,
            ),
            Ok(QueueApplyPath::QueuedAccount {
                requires_multi_txn: true
            })
        );

        let mut gap_fill_account = TxQAccount::new("acct");
        gap_fill_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 1),
            ),
        );
        gap_fill_account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                "s7",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(7), 1),
            ),
        );

        assert_eq!(
            evaluate_queue_apply_multitxn_admission(
                QueueApplyPath::QueuedAccount {
                    requires_multi_txn: true
                },
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                100,
                2,
                &gap_fill_account,
                2,
                SeqProxy::sequence(6),
                SeqProxy::sequence(5),
                false,
            ),
            Ok(QueueApplyPath::QueuedAccount {
                requires_multi_txn: true
            })
        );
    }
}
