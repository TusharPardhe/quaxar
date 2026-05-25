//! Vault-family `invokeApply(...)` dispatch shell above the landed vault
//! `doApply(...)` helpers.
//!
//! This ports the deterministic branch selection that the reference implementation performs for
//! the six vault transaction types plus the current `temUNKNOWN` fallback for
//! everything else.

use protocol::TxType;

use crate::{ApplyResult, HasTxnType, UNKNOWN_TRANSACTION_TYPE_TER, txn_type_of};

pub fn run_vault_invoke_apply_for_txn_type(
    txn_type: TxType,
    run_create_do_apply: impl FnOnce() -> ApplyResult,
    run_set_do_apply: impl FnOnce() -> ApplyResult,
    run_delete_do_apply: impl FnOnce() -> ApplyResult,
    run_deposit_do_apply: impl FnOnce() -> ApplyResult,
    run_withdraw_do_apply: impl FnOnce() -> ApplyResult,
    run_clawback_do_apply: impl FnOnce() -> ApplyResult,
) -> ApplyResult {
    run_vault_invoke_apply_result_for_txn_type(
        txn_type,
        || Ok::<ApplyResult, ()>(run_create_do_apply()),
        || Ok::<ApplyResult, ()>(run_set_do_apply()),
        || Ok::<ApplyResult, ()>(run_delete_do_apply()),
        || Ok::<ApplyResult, ()>(run_deposit_do_apply()),
        || Ok::<ApplyResult, ()>(run_withdraw_do_apply()),
        || Ok::<ApplyResult, ()>(run_clawback_do_apply()),
    )
    .expect("non-fallible vault invoke-apply wrapper should not produce an error")
}

pub fn run_vault_invoke_apply_for_txn_source<Tx: HasTxnType + ?Sized>(
    tx: &Tx,
    run_create_do_apply: impl FnOnce() -> ApplyResult,
    run_set_do_apply: impl FnOnce() -> ApplyResult,
    run_delete_do_apply: impl FnOnce() -> ApplyResult,
    run_deposit_do_apply: impl FnOnce() -> ApplyResult,
    run_withdraw_do_apply: impl FnOnce() -> ApplyResult,
    run_clawback_do_apply: impl FnOnce() -> ApplyResult,
) -> ApplyResult {
    run_vault_invoke_apply_for_txn_type(
        txn_type_of(tx),
        run_create_do_apply,
        run_set_do_apply,
        run_delete_do_apply,
        run_deposit_do_apply,
        run_withdraw_do_apply,
        run_clawback_do_apply,
    )
}

pub fn run_vault_invoke_apply_result_for_txn_type<E>(
    txn_type: TxType,
    run_create_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_set_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_delete_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_deposit_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_withdraw_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_clawback_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
) -> Result<ApplyResult, E> {
    match txn_type {
        TxType::VAULT_CREATE => run_create_do_apply(),
        TxType::VAULT_SET => run_set_do_apply(),
        TxType::VAULT_DELETE => run_delete_do_apply(),
        TxType::VAULT_DEPOSIT => run_deposit_do_apply(),
        TxType::VAULT_WITHDRAW => run_withdraw_do_apply(),
        TxType::VAULT_CLAWBACK => run_clawback_do_apply(),
        _ => Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)),
    }
}

pub fn run_vault_invoke_apply_result_for_txn_source<Tx: HasTxnType + ?Sized, E>(
    tx: &Tx,
    run_create_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_set_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_delete_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_deposit_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_withdraw_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_clawback_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
) -> Result<ApplyResult, E> {
    run_vault_invoke_apply_result_for_txn_type(
        txn_type_of(tx),
        run_create_do_apply,
        run_set_do_apply,
        run_delete_do_apply,
        run_deposit_do_apply,
        run_withdraw_do_apply,
        run_clawback_do_apply,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::{Ter, TxType};

    use super::{
        run_vault_invoke_apply_for_txn_source, run_vault_invoke_apply_for_txn_type,
        run_vault_invoke_apply_result_for_txn_source, run_vault_invoke_apply_result_for_txn_type,
    };
    use crate::{ApplyResult, HasTxnType, UNKNOWN_TRANSACTION_TYPE_TER};

    struct StubTx {
        txn_type: TxType,
    }

    impl HasTxnType for StubTx {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    #[test]
    fn vault_invoke_apply_routes_current_cpp_vault_do_apply_subset() {
        let trace = RefCell::new(Vec::new());

        let create = run_vault_invoke_apply_for_txn_type(
            TxType::VAULT_CREATE,
            || {
                trace.borrow_mut().push("create");
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
            || panic!("create should not dispatch to set"),
            || panic!("create should not dispatch to delete"),
            || panic!("create should not dispatch to deposit"),
            || panic!("create should not dispatch to withdraw"),
            || panic!("create should not dispatch to clawback"),
        );
        assert_eq!(create, ApplyResult::new(Ter::TES_SUCCESS, true, false));

        let withdraw = run_vault_invoke_apply_for_txn_type(
            TxType::VAULT_WITHDRAW,
            || panic!("withdraw should not dispatch to create"),
            || panic!("withdraw should not dispatch to set"),
            || panic!("withdraw should not dispatch to delete"),
            || panic!("withdraw should not dispatch to deposit"),
            || {
                trace.borrow_mut().push("withdraw");
                ApplyResult::new(Ter::TEC_INSUFFICIENT_RESERVE, false, false)
            },
            || panic!("withdraw should not dispatch to clawback"),
        );
        assert_eq!(
            withdraw,
            ApplyResult::new(Ter::TEC_INSUFFICIENT_RESERVE, false, false)
        );

        assert_eq!(trace.into_inner(), vec!["create", "withdraw"]);
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
        let tx = StubTx {
            txn_type: TxType::VAULT_SET,
        };

        let result = run_vault_invoke_apply_for_txn_source(
            &tx,
            || panic!("set should not dispatch to create"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            || panic!("set should not dispatch to delete"),
            || panic!("set should not dispatch to deposit"),
            || panic!("set should not dispatch to withdraw"),
            || panic!("set should not dispatch to clawback"),
        );

        assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, true));
    }

    #[test]
    fn vault_invoke_apply_result_wrapper_preserves_errors_from_selected_branch() {
        let result: Result<ApplyResult, &str> = run_vault_invoke_apply_result_for_txn_type(
            TxType::VAULT_CLAWBACK,
            || panic!("clawback should not dispatch to create"),
            || panic!("clawback should not dispatch to set"),
            || panic!("clawback should not dispatch to delete"),
            || panic!("clawback should not dispatch to deposit"),
            || panic!("clawback should not dispatch to withdraw"),
            || Err("boom"),
        );

        assert_eq!(result, Err("boom"));

        let tx = StubTx {
            txn_type: TxType::PAYMENT,
        };
        let unknown: Result<ApplyResult, &str> = run_vault_invoke_apply_result_for_txn_source(
            &tx,
            || panic!("unknown type should not dispatch to create"),
            || panic!("unknown type should not dispatch to set"),
            || panic!("unknown type should not dispatch to delete"),
            || panic!("unknown type should not dispatch to deposit"),
            || panic!("unknown type should not dispatch to withdraw"),
            || panic!("unknown type should not dispatch to clawback"),
        );

        assert_eq!(
            unknown,
            Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false))
        );
    }
}
