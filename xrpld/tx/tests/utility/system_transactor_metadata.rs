//! Integration tests that pin the shared system-transactor metadata and
//! consequence-shaping seam to the current C++ system transaction behavior.

use protocol::{SeqProxy, TxType};
use tx::{
    ChangeSystemTxnType, HasTxnType, SystemTransactorTxnType, TxConsequencesCategory,
    UnknownTransactionType, classify_system_transactor_txn_type,
    run_system_make_tx_consequences_for_txn_source, run_system_make_tx_consequences_for_txn_type,
    run_with_system_transactor_txn_type_key, run_with_system_transactor_txn_type_source,
    system_transactor_txn_consequences_category,
};

struct TestTx {
    txn_type: TxType,
}

impl HasTxnType for TestTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn system_transactor_metadata_classifies_current_system_tx_set() {
    assert_eq!(
        classify_system_transactor_txn_type(TxType::AMENDMENT),
        Some(SystemTransactorTxnType::Change(
            ChangeSystemTxnType::Amendment
        ))
    );
    assert_eq!(
        classify_system_transactor_txn_type(TxType::BATCH),
        Some(SystemTransactorTxnType::Batch)
    );
    assert_eq!(
        classify_system_transactor_txn_type(TxType::TICKET_CREATE),
        Some(SystemTransactorTxnType::TicketCreate)
    );
    assert_eq!(
        classify_system_transactor_txn_type(TxType::LEDGER_STATE_FIX),
        Some(SystemTransactorTxnType::LedgerStateFix)
    );
}

#[test]
fn system_transactor_metadata_preserves_unknowns_subset() {
    let unknown = run_with_system_transactor_txn_type_key(TxType::PAYMENT, |txn_type| txn_type);

    assert_eq!(classify_system_transactor_txn_type(TxType::PAYMENT), None);
    assert_eq!(unknown, Err(UnknownTransactionType::new(TxType::PAYMENT)));
}

#[test]
fn system_transactor_metadata_source_wrapper_uses_txn_type() {
    let result = run_with_system_transactor_txn_type_source(
        &TestTx {
            txn_type: TxType::UNL_MODIFY,
        },
        |txn_type| txn_type,
    );

    assert_eq!(
        result,
        Ok(SystemTransactorTxnType::Change(
            ChangeSystemTxnType::UnlModify
        ))
    );
}

#[test]
fn system_transactor_metadata_tracks_current_consequences_factory_roles() {
    assert_eq!(
        system_transactor_txn_consequences_category(SystemTransactorTxnType::Change(
            ChangeSystemTxnType::Fee
        )),
        TxConsequencesCategory::Normal
    );
    assert_eq!(
        system_transactor_txn_consequences_category(SystemTransactorTxnType::Batch),
        TxConsequencesCategory::Normal
    );
    assert_eq!(
        system_transactor_txn_consequences_category(SystemTransactorTxnType::TicketCreate),
        TxConsequencesCategory::Normal
    );
    assert_eq!(
        system_transactor_txn_consequences_category(SystemTransactorTxnType::LedgerStateFix),
        TxConsequencesCategory::Normal
    );
}

#[test]
fn system_transactor_metadata_keeps_normal_paths_on_base_consequences() {
    let change =
        run_system_make_tx_consequences_for_txn_type(TxType::FEE, 12, SeqProxy::sequence(5), 9)
            .expect("fee should classify");
    let batch =
        run_system_make_tx_consequences_for_txn_type(TxType::BATCH, 12, SeqProxy::ticket(7), 9)
            .expect("batch should classify");

    assert_eq!(change.sequences_consumed(), 1);
    assert_eq!(change.following_seq(), SeqProxy::sequence(6));
    assert_eq!(batch.sequences_consumed(), 0);
    assert_eq!(batch.following_seq(), SeqProxy::ticket(7));
}

#[test]
fn system_transactor_metadata_uses_ticket_create_custom_consequences() {
    let ticket = run_system_make_tx_consequences_for_txn_type(
        TxType::TICKET_CREATE,
        12,
        SeqProxy::sequence(5),
        4,
    )
    .expect("ticket create should classify");
    let ticket_source = run_system_make_tx_consequences_for_txn_source(
        &TestTx {
            txn_type: TxType::TICKET_CREATE,
        },
        12,
        SeqProxy::ticket(9),
        2,
    )
    .expect("ticket create source should classify");

    assert_eq!(ticket.sequences_consumed(), 4);
    assert_eq!(ticket.following_seq(), SeqProxy::sequence(9));
    assert_eq!(ticket_source.sequences_consumed(), 2);
    assert_eq!(ticket_source.following_seq(), SeqProxy::ticket(11));
}
