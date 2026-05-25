//! Integration tests that pin the system-family `invokeApply(...)` dispatch
//! shell to the current C++ transaction-type routing and unknown fallback.

use std::cell::RefCell;

use protocol::{Ter, TxType};
use tx::{
    ApplyResult, HasTxnType, SystemApplyTxnType, UNKNOWN_TRANSACTION_TYPE_TER,
    classify_system_apply_txn_type, run_system_invoke_apply_for_txn_source,
    run_system_invoke_apply_for_txn_type, run_with_system_apply_txn_source,
    run_with_system_apply_txn_type_key,
};

struct StubTx {
    txn_type: TxType,
}

impl HasTxnType for StubTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn system_invoke_apply_classifies_current_system_family() {
    assert_eq!(
        classify_system_apply_txn_type(TxType::AMENDMENT),
        Some(SystemApplyTxnType::Change)
    );
    assert_eq!(
        classify_system_apply_txn_type(TxType::FEE),
        Some(SystemApplyTxnType::Change)
    );
    assert_eq!(
        classify_system_apply_txn_type(TxType::UNL_MODIFY),
        Some(SystemApplyTxnType::Change)
    );
    assert_eq!(
        classify_system_apply_txn_type(TxType::BATCH),
        Some(SystemApplyTxnType::Batch)
    );
    assert_eq!(
        classify_system_apply_txn_type(TxType::TICKET_CREATE),
        Some(SystemApplyTxnType::TicketCreate)
    );
    assert_eq!(
        classify_system_apply_txn_type(TxType::LEDGER_STATE_FIX),
        Some(SystemApplyTxnType::LedgerStateFix)
    );
    assert_eq!(
        classify_system_apply_txn_type(TxType::PAYMENT),
        Some(SystemApplyTxnType::Payment)
    );
}

#[test]
fn system_invoke_apply_routes_each_system_transaction_type() {
    let trace = RefCell::new(Vec::new());

    let amendment = run_system_invoke_apply_for_txn_type(
        TxType::AMENDMENT,
        || {
            trace.borrow_mut().push("change:amendment");
            ApplyResult::new(Ter::TES_SUCCESS, true, false)
        },
        || panic!("amendment should not dispatch to batch"),
        || panic!("amendment should not dispatch to ticket create"),
        || panic!("amendment should not dispatch to ledger state fix"),
        || panic!("amendment should not dispatch to payment"),
        || panic!("amendment should not dispatch to offer create"),
        || panic!("amendment should not dispatch to offer cancel"),
        || panic!("amendment should not dispatch to trust set"),
        || panic!("amendment should not dispatch to nft create"),
        || panic!("amendment should not dispatch to nft cancel"),
        || panic!("amendment should not dispatch to nft accept"),
    );
    assert_eq!(amendment, ApplyResult::new(Ter::TES_SUCCESS, true, false));

    let batch = run_system_invoke_apply_for_txn_type(
        TxType::BATCH,
        || panic!("batch should not dispatch to change"),
        || {
            trace.borrow_mut().push("batch");
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        },
        || panic!("batch should not dispatch to ticket create"),
        || panic!("batch should not dispatch to ledger state fix"),
        || panic!("batch should not dispatch to payment"),
        || panic!("batch should not dispatch to offer create"),
        || panic!("batch should not dispatch to offer cancel"),
        || panic!("batch should not dispatch to trust set"),
        || panic!("batch should not dispatch to nft create"),
        || panic!("batch should not dispatch to nft cancel"),
        || panic!("batch should not dispatch to nft accept"),
    );
    assert_eq!(batch, ApplyResult::new(Ter::TES_SUCCESS, true, true));

    let ticket = run_system_invoke_apply_for_txn_type(
        TxType::TICKET_CREATE,
        || panic!("ticket create should not dispatch to change"),
        || panic!("ticket create should not dispatch to batch"),
        || {
            trace.borrow_mut().push("ticket");
            ApplyResult::new(Ter::TEC_DIR_FULL, false, false)
        },
        || panic!("ticket create should not dispatch to ledger state fix"),
        || panic!("should not dispatch to payment"),
        || panic!("should not dispatch to offer create"),
        || panic!("should not dispatch to offer cancel"),
        || panic!("should not dispatch to trust set"),
        || panic!("should not dispatch to nft create"),
        || panic!("should not dispatch to nft cancel"),
        || panic!("should not dispatch to nft accept"),
    );
    assert_eq!(ticket, ApplyResult::new(Ter::TEC_DIR_FULL, false, false));

    let ledger_fix = run_system_invoke_apply_for_txn_type(
        TxType::LEDGER_STATE_FIX,
        || panic!("ledger state fix should not dispatch to change"),
        || panic!("ledger state fix should not dispatch to batch"),
        || panic!("ledger state fix should not dispatch to ticket create"),
        || {
            trace.borrow_mut().push("ledger_fix");
            ApplyResult::new(Ter::TEC_OBJECT_NOT_FOUND, false, false)
        },
        || panic!("should not dispatch to payment"),
        || panic!("should not dispatch to offer create"),
        || panic!("should not dispatch to offer cancel"),
        || panic!("should not dispatch to trust set"),
        || panic!("should not dispatch to nft create"),
        || panic!("should not dispatch to nft cancel"),
        || panic!("should not dispatch to nft accept"),
    );
    assert_eq!(
        ledger_fix,
        ApplyResult::new(Ter::TEC_OBJECT_NOT_FOUND, false, false)
    );

    assert_eq!(
        trace.into_inner(),
        vec!["change:amendment", "batch", "ticket", "ledger_fix"]
    );
}

#[test]
fn system_invoke_apply_maps_unknown_transaction_types_to_temunknown() {
    let result = run_system_invoke_apply_for_txn_type(
        TxType::ESCROW_CREATE, // Use a non-system type that is now known but not dispatched here
        || panic!("unknown transaction type should not dispatch to change"),
        || panic!("unknown transaction type should not dispatch to batch"),
        || panic!("unknown transaction type should not dispatch to ticket create"),
        || panic!("unknown transaction type should not dispatch to ledger state fix"),
        || panic!("should not dispatch to payment"),
        || panic!("should not dispatch to offer create"),
        || panic!("should not dispatch to offer cancel"),
        || panic!("should not dispatch to trust set"),
        || panic!("should not dispatch to nft create"),
        || panic!("should not dispatch to nft cancel"),
        || panic!("should not dispatch to nft accept"),
    );

    assert_eq!(
        result,
        ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)
    );
    assert_eq!(result.ter, Ter::TEM_UNKNOWN);
}

#[test]
fn system_invoke_apply_source_wrapper_uses_txn_type_from_source() {
    let tx = StubTx {
        txn_type: TxType::FEE,
    };

    let result = run_system_invoke_apply_for_txn_source(
        &tx,
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || panic!("fee should not dispatch to batch"),
        || panic!("fee should not dispatch to ticket create"),
        || panic!("fee should not dispatch to ledger state fix"),
        || panic!("fee should not dispatch to payment"),
        || panic!("fee should not dispatch to offer create"),
        || panic!("fee should not dispatch to offer cancel"),
        || panic!("fee should not dispatch to trust set"),
        || panic!("fee should not dispatch to nft create"),
        || panic!("fee should not dispatch to nft cancel"),
        || panic!("fee should not dispatch to nft accept"),
    );

    assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, false));

    let routed = run_with_system_apply_txn_type_key(TxType::BATCH, |kind| kind);
    assert_eq!(routed, Ok(SystemApplyTxnType::Batch));

    let routed = run_with_system_apply_txn_source(&tx, |kind| kind);
    assert_eq!(routed, Ok(SystemApplyTxnType::Change));

    assert_eq!(
        run_with_system_apply_txn_type_key(TxType::HOOK_SET, |_kind| 99_u8),
        Err(tx::UnknownTransactionType::new(TxType::HOOK_SET))
    );
}
