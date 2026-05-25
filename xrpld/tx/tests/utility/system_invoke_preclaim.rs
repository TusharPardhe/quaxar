//! Integration tests that pin the system-family `invoke_preclaim(...)` shell
//! to the current C++ ordering across `Change`, `Batch`, `TicketCreate`, and
//! `LedgerStateFix`.

use std::cell::RefCell;

use protocol::{Ter, TxType, trans_token};
use tx::ledger_state_fix::LedgerStateFixType;
use tx::{
    ChangePreclaimFacts, HasTxnType, TicketCreatePreclaimFacts, UnknownTransactionType,
    run_system_invoke_preclaim_for_txn_source, run_system_invoke_preclaim_for_txn_type,
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
fn system_invoke_preclaim_uses_change_path_for_change_family() {
    let result = run_system_invoke_preclaim_for_txn_type(
        true,
        TxType::AMENDMENT,
        ChangePreclaimFacts::default(),
        TicketCreatePreclaimFacts::default(),
        LedgerStateFixType::NfTokenPageLink,
        true,
        || panic!("change path should skip seq"),
        || panic!("change path should skip prior"),
        || panic!("change path should skip permission"),
        || panic!("change path should skip sign"),
        || panic!("change path should skip batch sign"),
        || panic!("change path should skip base fee"),
        |_| panic!("change path should skip fee"),
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    )
    .expect("change family should be a known system transaction");

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn system_invoke_preclaim_preserves_batch_sign_order() {
    let trace = RefCell::new(Vec::new());

    let result = run_system_invoke_preclaim_for_txn_type(
        false,
        TxType::BATCH,
        ChangePreclaimFacts::default(),
        TicketCreatePreclaimFacts::default(),
        LedgerStateFixType::NfTokenPageLink,
        true,
        || {
            trace.borrow_mut().push("seq");
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("prior");
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("permission");
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("sign");
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("batch-sign");
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("base-fee");
            20_u64
        },
        |fee| {
            trace.borrow_mut().push("fee");
            assert_eq!(fee, 20);
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    )
    .expect("batch should be a known system transaction");

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        trace.into_inner(),
        vec![
            "seq",
            "prior",
            "permission",
            "sign",
            "batch-sign",
            "base-fee",
            "fee"
        ]
    );
}

#[test]
fn system_invoke_preclaim_routes_ticket_create_and_ledger_fix_helpers() {
    let ticket = run_system_invoke_preclaim_for_txn_type(
        false,
        TxType::TICKET_CREATE,
        ChangePreclaimFacts::default(),
        TicketCreatePreclaimFacts {
            account_exists: true,
            current_ticket_count: 4,
            requested_ticket_count: 2,
            consumes_ticket_sequence: false,
        },
        LedgerStateFixType::NfTokenPageLink,
        true,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || panic!("ticket path should skip batch sign"),
        || 20_u64,
        |_| Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    )
    .expect("ticket create should be a known system transaction");
    let ledger_fix = run_system_invoke_preclaim_for_txn_type(
        false,
        TxType::LEDGER_STATE_FIX,
        ChangePreclaimFacts::default(),
        TicketCreatePreclaimFacts::default(),
        LedgerStateFixType::NfTokenPageLink,
        false,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || panic!("ledger fix path should skip batch sign"),
        || 20_u64,
        |_| Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    )
    .expect("ledger state fix should be a known system transaction");

    assert_eq!(ticket, Ter::TES_SUCCESS);
    assert_eq!(ledger_fix, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(trans_token(ledger_fix), "tecOBJECT_NOT_FOUND");
}

#[test]
fn system_invoke_preclaim_preserves_unknowns_for_non_system_types_subset() {
    // AccountSet is a truly non-system type: it is not matched by any arm in
    // run_system_invoke_preclaim_for_txn_type and therefore produces the
    // UnknownTransactionType error that C++ would also produce when the system
    // transactor encounters a transaction it does not own.
    let tx = TestTx {
        txn_type: TxType::ACCOUNT_SET,
    };

    let result = run_system_invoke_preclaim_for_txn_source(
        false,
        &tx,
        ChangePreclaimFacts::default(),
        TicketCreatePreclaimFacts::default(),
        LedgerStateFixType::NfTokenPageLink,
        true,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || 20_u64,
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(
        result,
        Err(UnknownTransactionType::new(TxType::ACCOUNT_SET))
    );
}
