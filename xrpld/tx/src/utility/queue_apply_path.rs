//! Account-path split inside `TxQ::apply(...)` after the replacement-fee
//! branch.
//!
//! This ports only the deterministic `acctTxCount == 0` versus queued-account
//! split, the early `tefPAST_SEQ` / `terPRE_SEQ` checks around that split, and
//! the current `requiresMultiTxn` decision.

use protocol::{SeqProxy, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueApplyPath {
    OpenLedger,
    QueuedAccount { requires_multi_txn: bool },
}

impl QueueApplyPath {
    pub const fn requires_multi_txn(self) -> bool {
        match self {
            Self::OpenLedger => false,
            Self::QueuedAccount { requires_multi_txn } => requires_multi_txn,
        }
    }
}

pub fn evaluate_queue_apply_path(
    account_tx_count: usize,
    tx_seq_proxy: SeqProxy,
    account_seq_proxy: SeqProxy,
    replaces_existing: bool,
) -> Result<QueueApplyPath, Ter> {
    if account_tx_count == 0 {
        if tx_seq_proxy.is_seq() {
            if account_seq_proxy > tx_seq_proxy {
                return Err(Ter::TEF_PAST_SEQ);
            }
            if account_seq_proxy < tx_seq_proxy {
                return Err(Ter::TER_PRE_SEQ);
            }
        }

        return Ok(QueueApplyPath::OpenLedger);
    }

    if account_seq_proxy > tx_seq_proxy {
        return Err(Ter::TEF_PAST_SEQ);
    }

    Ok(QueueApplyPath::QueuedAccount {
        requires_multi_txn: account_tx_count > 1 || !replaces_existing,
    })
}

#[cfg(test)]
mod tests {
    use protocol::{SeqProxy, Ter};

    use super::{QueueApplyPath, evaluate_queue_apply_path};

    #[test]
    fn open_ledger_path_rejects_past_and_future_sequences() {
        assert_eq!(
            evaluate_queue_apply_path(0, SeqProxy::sequence(4), SeqProxy::sequence(5), false,),
            Err(Ter::TEF_PAST_SEQ)
        );
        assert_eq!(
            evaluate_queue_apply_path(0, SeqProxy::sequence(6), SeqProxy::sequence(5), false,),
            Err(Ter::TER_PRE_SEQ)
        );
        assert_eq!(
            evaluate_queue_apply_path(0, SeqProxy::sequence(5), SeqProxy::sequence(5), false,),
            Ok(QueueApplyPath::OpenLedger)
        );
    }

    #[test]
    fn open_ledger_path_allows_ticketed_transactions_without_sequence_comparison() {
        assert_eq!(
            evaluate_queue_apply_path(0, SeqProxy::ticket(9), SeqProxy::sequence(5), false),
            Ok(QueueApplyPath::OpenLedger)
        );
    }

    #[test]
    fn queued_account_path_rejects_past_sequences() {
        assert_eq!(
            evaluate_queue_apply_path(1, SeqProxy::sequence(4), SeqProxy::sequence(5), false,),
            Err(Ter::TEF_PAST_SEQ)
        );
    }

    #[test]
    fn queued_account_path_sets_multitxn_requirement_from_count_and_replacement() {
        assert_eq!(
            evaluate_queue_apply_path(1, SeqProxy::sequence(5), SeqProxy::sequence(5), true,),
            Ok(QueueApplyPath::QueuedAccount {
                requires_multi_txn: false
            })
        );
        assert_eq!(
            evaluate_queue_apply_path(1, SeqProxy::sequence(5), SeqProxy::sequence(5), false,),
            Ok(QueueApplyPath::QueuedAccount {
                requires_multi_txn: true
            })
        );
        assert_eq!(
            evaluate_queue_apply_path(2, SeqProxy::sequence(5), SeqProxy::sequence(5), true,),
            Ok(QueueApplyPath::QueuedAccount {
                requires_multi_txn: true
            })
        );
    }

    #[test]
    fn apply_path_reports_multitxn_requirement_for_queued_accounts_only() {
        assert!(!QueueApplyPath::OpenLedger.requires_multi_txn());
        assert!(
            !QueueApplyPath::QueuedAccount {
                requires_multi_txn: false
            }
            .requires_multi_txn()
        );
        assert!(
            QueueApplyPath::QueuedAccount {
                requires_multi_txn: true
            }
            .requires_multi_txn()
        );
    }
}
