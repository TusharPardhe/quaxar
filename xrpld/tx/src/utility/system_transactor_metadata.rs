//! Shared static system transactor metadata that the reference implementation system classes
//! expose through headers and the transaction dispatch layer.
//!
//! This ports the exact current behavior around:
//!
//! - selecting the current system-family transactor kind,
//! - mapping that kind to a protocol-owned `TxConsequences` factory,
//! - and providing the generic dispatch wrappers used by the `invoke*` shells.

use protocol::{SeqProxy, TxType};

use crate::{
    HasTxnType, TxConsequences, TxConsequencesCategory, UnknownTransactionType,
    run_ticket_create_make_tx_consequences, txn_type_of,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeSystemTxnType {
    Amendment,
    Fee,
    UnlModify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemTransactorTxnType {
    Change(ChangeSystemTxnType),
    Batch,
    TicketCreate,
    LedgerStateFix,
}

pub fn classify_system_transactor_txn_type(txn_type: TxType) -> Option<SystemTransactorTxnType> {
    match txn_type {
        TxType::AMENDMENT => Some(SystemTransactorTxnType::Change(
            ChangeSystemTxnType::Amendment,
        )),
        TxType::FEE => Some(SystemTransactorTxnType::Change(ChangeSystemTxnType::Fee)),
        TxType::UNL_MODIFY => Some(SystemTransactorTxnType::Change(
            ChangeSystemTxnType::UnlModify,
        )),
        TxType::BATCH => Some(SystemTransactorTxnType::Batch),
        TxType::TICKET_CREATE => Some(SystemTransactorTxnType::TicketCreate),
        TxType::LEDGER_STATE_FIX => Some(SystemTransactorTxnType::LedgerStateFix),
        _ => None,
    }
}

pub fn run_with_system_transactor_txn_type_key<R>(
    txn_type: TxType,
    dispatch: impl FnOnce(SystemTransactorTxnType) -> R,
) -> Result<R, UnknownTransactionType<TxType>> {
    match classify_system_transactor_txn_type(txn_type) {
        Some(system_txn_type) => Ok(dispatch(system_txn_type)),
        None => Err(UnknownTransactionType::new(txn_type)),
    }
}

pub fn run_with_system_transactor_txn_type_source<Tx: HasTxnType + ?Sized, R>(
    tx: &Tx,
    dispatch: impl FnOnce(SystemTransactorTxnType) -> R,
) -> Result<R, UnknownTransactionType<TxType>> {
    run_with_system_transactor_txn_type_key(txn_type_of(tx), dispatch)
}

pub const fn system_transactor_txn_consequences_category(
    txn_type: SystemTransactorTxnType,
) -> TxConsequencesCategory {
    match txn_type {
        SystemTransactorTxnType::Change(_)
        | SystemTransactorTxnType::Batch
        | SystemTransactorTxnType::TicketCreate
        | SystemTransactorTxnType::LedgerStateFix => TxConsequencesCategory::Normal,
    }
}

pub fn run_system_make_tx_consequences_for_txn_type(
    txn_type: TxType,
    fee_drops: u64,
    seq_proxy: SeqProxy,
    ticket_count: u32,
) -> Result<TxConsequences, UnknownTransactionType<TxType>> {
    run_with_system_transactor_txn_type_key(txn_type, |system_txn_type| match system_txn_type {
        SystemTransactorTxnType::TicketCreate => {
            run_ticket_create_make_tx_consequences(fee_drops, seq_proxy, ticket_count)
        }
        SystemTransactorTxnType::Change(_)
        | SystemTransactorTxnType::Batch
        | SystemTransactorTxnType::LedgerStateFix => TxConsequences::new(fee_drops, seq_proxy),
    })
}

pub fn run_system_make_tx_consequences_for_txn_source<Tx: HasTxnType + ?Sized>(
    tx: &Tx,
    fee_drops: u64,
    seq_proxy: SeqProxy,
    ticket_count: u32,
) -> Result<TxConsequences, UnknownTransactionType<TxType>> {
    run_system_make_tx_consequences_for_txn_type(
        txn_type_of(tx),
        fee_drops,
        seq_proxy,
        ticket_count,
    )
}

#[cfg(test)]
mod tests {
    use protocol::{SeqProxy, TxType};

    use super::{
        classify_system_transactor_txn_type, run_with_system_transactor_txn_type_key,
        run_with_system_transactor_txn_type_source,
    };
    use crate::{HasTxnType, UnknownTransactionType};

    struct StubTx {
        txn_type: TxType,
    }

    impl HasTxnType for StubTx {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    struct StubTicketSource {
        following_seq: SeqProxy,
        sequences_consumed: u32,
    }

    impl HasTxnType for StubTicketSource {
        fn txn_type(&self) -> TxType {
            TxType::TICKET_CREATE
        }
    }

    #[test]
    fn system_transactor_metadata_following_seq_calculates_next_sequence() {
        let ticket = StubTicketSource {
            following_seq: SeqProxy::sequence(5),
            sequences_consumed: 3,
        };
        let ticket_source = StubTicketSource {
            following_seq: SeqProxy::ticket(9),
            sequences_consumed: 2,
        };

        assert_eq!(ticket.sequences_consumed, 3);
        assert_eq!(ticket.following_seq, SeqProxy::sequence(5));
        assert_eq!(ticket_source.sequences_consumed, 2);
        assert_eq!(ticket_source.following_seq, SeqProxy::ticket(9));
    }

    #[test]
    fn system_transactor_classifier_rejects_non_system_transactions_subset() {
        assert_eq!(
            classify_system_transactor_txn_type(TxType::ESCROW_CREATE),
            None
        );
    }

    #[test]
    fn system_transactor_key_and_source_wrappers_dispatch_and_preserve_unknowns() {
        let tx = StubTx {
            txn_type: TxType::ESCROW_CREATE,
        };

        let result = run_with_system_transactor_txn_type_key(TxType::ESCROW_CREATE, |kind| kind);
        assert_eq!(
            result,
            Err(UnknownTransactionType::new(TxType::ESCROW_CREATE))
        );

        let result = run_with_system_transactor_txn_type_source(&tx, |kind| kind);
        assert_eq!(
            result,
            Err(UnknownTransactionType::new(TxType::ESCROW_CREATE))
        );
    }
}
