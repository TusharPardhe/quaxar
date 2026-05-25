//! Integration tests that pin the vault-family `invokeApply` shell to the
//! current C++ vault transaction behavior.

use std::cell::RefCell;

use protocol::{Ter, TxType};
use tx::vault_invoke_apply::{
    run_vault_invoke_apply_for_txn_source, run_vault_invoke_apply_for_txn_type,
    run_vault_invoke_apply_result_for_txn_type,
};
use tx::{ApplyResult, HasTxnType, UNKNOWN_TRANSACTION_TYPE_TER};

struct TestTx {
    txn_type: TxType,
}

impl HasTxnType for TestTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn vault_invoke_apply_routes_to_the_selected_vault_do_apply_branch() {
    let trace = RefCell::new(Vec::new());

    let result = run_vault_invoke_apply_for_txn_type(
        TxType::VAULT_DEPOSIT,
        || panic!("deposit should not dispatch to create"),
        || panic!("deposit should not dispatch to set"),
        || panic!("deposit should not dispatch to delete"),
        || {
            trace.borrow_mut().push("deposit");
            ApplyResult::new(Ter::TES_SUCCESS, true, false)
        },
        || panic!("deposit should not dispatch to withdraw"),
        || panic!("deposit should not dispatch to clawback"),
    );

    assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, false));
    assert_eq!(trace.into_inner(), vec!["deposit"]);
}

#[test]
fn vault_invoke_apply_maps_unknown_transaction_types_to_temunknown() {
    let result = run_vault_invoke_apply_for_txn_type(
        TxType::PAYMENT,
        || panic!("unknown type should not dispatch to create"),
        || panic!("unknown type should not dispatch to set"),
        || panic!("unknown type should not dispatch to delete"),
        || panic!("unknown type should not dispatch to deposit"),
        || panic!("unknown type should not dispatch to withdraw"),
        || panic!("unknown type should not dispatch to clawback"),
    );

    assert_eq!(
        result,
        ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)
    );
}

#[test]
fn vault_invoke_apply_source_wrapper_reads_txn_type_from_source() {
    let result = run_vault_invoke_apply_for_txn_source(
        &TestTx {
            txn_type: TxType::VAULT_CLAWBACK,
        },
        || panic!("clawback should not dispatch to create"),
        || panic!("clawback should not dispatch to set"),
        || panic!("clawback should not dispatch to delete"),
        || panic!("clawback should not dispatch to deposit"),
        || panic!("clawback should not dispatch to withdraw"),
        || ApplyResult::new(Ter::TEC_NO_PERMISSION, false, false),
    );

    assert_eq!(
        result,
        ApplyResult::new(Ter::TEC_NO_PERMISSION, false, false)
    );
}

#[test]
fn vault_invoke_apply_result_wrapper_preserves_selected_branch_errors() {
    let result: Result<ApplyResult, &str> = run_vault_invoke_apply_result_for_txn_type(
        TxType::VAULT_SET,
        || panic!("set should not dispatch to create"),
        || Err("boom"),
        || panic!("set should not dispatch to delete"),
        || panic!("set should not dispatch to deposit"),
        || panic!("set should not dispatch to withdraw"),
        || panic!("set should not dispatch to clawback"),
    );

    assert_eq!(result, Err("boom"));
}
