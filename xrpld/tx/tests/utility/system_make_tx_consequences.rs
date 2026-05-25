//! Integration tests that pin the system-family consequence entrypoint to the
//! current C++ routing and unknown fallback.

use protocol::{SeqProxy, TxType};
use tx::system_make_tx_consequences::{
    run_system_make_tx_consequences_entrypoint_for_txn_source,
    run_system_make_tx_consequences_entrypoint_for_txn_type,
};
use tx::{HasTxnType, TxConsequences, UnknownTransactionType};

struct StubTx {
    txn_type: TxType,
}

impl HasTxnType for StubTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn system_make_tx_consequences_routes_ticket_create_and_normal_system_types() {
    let change = run_system_make_tx_consequences_entrypoint_for_txn_type(
        TxType::AMENDMENT,
        12,
        SeqProxy::sequence(5),
        99,
    )
    .expect("change should be known");
    let batch = run_system_make_tx_consequences_entrypoint_for_txn_type(
        TxType::BATCH,
        12,
        SeqProxy::ticket(7),
        99,
    )
    .expect("batch should be known");
    let ticket = run_system_make_tx_consequences_entrypoint_for_txn_type(
        TxType::TICKET_CREATE,
        12,
        SeqProxy::sequence(3),
        4,
    )
    .expect("ticket create should be known");
    let ledger_fix = run_system_make_tx_consequences_entrypoint_for_txn_type(
        TxType::LEDGER_STATE_FIX,
        12,
        SeqProxy::ticket(9),
        99,
    )
    .expect("ledger state fix should be known");

    assert_eq!(change, TxConsequences::new(12, SeqProxy::sequence(5)));
    assert_eq!(batch, TxConsequences::new(12, SeqProxy::ticket(7)));
    assert_eq!(
        ticket,
        TxConsequences::with_sequences_consumed(12, SeqProxy::sequence(3), 4)
    );
    assert_eq!(ledger_fix, TxConsequences::new(12, SeqProxy::ticket(9)));
}

#[test]
fn system_make_tx_consequences_preserves_unknown_fallback() {
    let result = run_system_make_tx_consequences_entrypoint_for_txn_type(
        TxType::PAYMENT,
        12,
        SeqProxy::sequence(5),
        2,
    );

    assert_eq!(result, Err(UnknownTransactionType::new(TxType::PAYMENT)));
}

#[test]
fn system_make_tx_consequences_source_wrapper_uses_txn_type_from_source() {
    let tx = StubTx {
        txn_type: TxType::TICKET_CREATE,
    };

    let result =
        run_system_make_tx_consequences_entrypoint_for_txn_source(&tx, 20, SeqProxy::ticket(11), 2)
            .expect("ticket create should be known");

    assert_eq!(
        result,
        TxConsequences::with_sequences_consumed(20, SeqProxy::ticket(11), 2)
    );
}
