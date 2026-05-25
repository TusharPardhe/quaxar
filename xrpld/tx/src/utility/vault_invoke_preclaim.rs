//! Vault-family `invoke_preclaim(...)` composition shell above the
//! already-landed shared transactor preclaim wrapper and per-transaction vault
//! `preclaim(...)` helpers.
//!
//! This ports the exact current ordering around:
//!
//! - selecting the six vault transaction types from the larger `TxType` set,
//! - skipping all transactor prechecks when `sfAccount` is zero,
//! - otherwise running the shared `checkSeqProxy(...)`,
//!   `checkPriorTxAndLastLedger(...)`, `checkPermission(...)`, and
//!   `checkSign(...)` sequence,
//! - calculating the base fee only after those checks succeed,
//! - running `checkFee(...)` only after that base-fee calculation,
//! - and falling through to the selected landed vault `preclaim(...)` helper
//!   only when all earlier steps succeed.

use protocol::{NotTec, Ter, TxType};

use crate::{
    HasTxnType, UnknownTransactionType, VaultTxnType, run_transactor_invoke_preclaim,
    run_with_vault_txn_type_key,
};

#[allow(clippy::too_many_arguments)]
pub fn run_vault_invoke_preclaim_for_txn_type<Fee>(
    account_is_zero: bool,
    txn_type: TxType,
    check_seq_proxy: impl FnOnce() -> NotTec,
    check_prior_tx_and_last_ledger: impl FnOnce() -> NotTec,
    check_permission: impl FnOnce() -> NotTec,
    check_sign: impl FnOnce() -> NotTec,
    calculate_base_fee: impl FnOnce() -> Fee,
    check_fee: impl FnOnce(Fee) -> Ter,
    run_create_preclaim: impl FnOnce() -> Ter,
    run_set_preclaim: impl FnOnce() -> Ter,
    run_delete_preclaim: impl FnOnce() -> Ter,
    run_deposit_preclaim: impl FnOnce() -> Ter,
    run_withdraw_preclaim: impl FnOnce() -> Ter,
    run_clawback_preclaim: impl FnOnce() -> Ter,
) -> Result<Ter, UnknownTransactionType<TxType>> {
    run_with_vault_txn_type_key(txn_type, |vault_txn_type| {
        run_transactor_invoke_preclaim(
            account_is_zero,
            check_seq_proxy,
            check_prior_tx_and_last_ledger,
            check_permission,
            check_sign,
            calculate_base_fee,
            check_fee,
            || match vault_txn_type {
                VaultTxnType::Create => run_create_preclaim(),
                VaultTxnType::Set => run_set_preclaim(),
                VaultTxnType::Delete => run_delete_preclaim(),
                VaultTxnType::Deposit => run_deposit_preclaim(),
                VaultTxnType::Withdraw => run_withdraw_preclaim(),
                VaultTxnType::Clawback => run_clawback_preclaim(),
            },
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_vault_invoke_preclaim_for_txn_source<Tx: HasTxnType + ?Sized, Fee>(
    account_is_zero: bool,
    tx: &Tx,
    check_seq_proxy: impl FnOnce() -> NotTec,
    check_prior_tx_and_last_ledger: impl FnOnce() -> NotTec,
    check_permission: impl FnOnce() -> NotTec,
    check_sign: impl FnOnce() -> NotTec,
    calculate_base_fee: impl FnOnce() -> Fee,
    check_fee: impl FnOnce(Fee) -> Ter,
    run_create_preclaim: impl FnOnce() -> Ter,
    run_set_preclaim: impl FnOnce() -> Ter,
    run_delete_preclaim: impl FnOnce() -> Ter,
    run_deposit_preclaim: impl FnOnce() -> Ter,
    run_withdraw_preclaim: impl FnOnce() -> Ter,
    run_clawback_preclaim: impl FnOnce() -> Ter,
) -> Result<Ter, UnknownTransactionType<TxType>> {
    run_vault_invoke_preclaim_for_txn_type(
        account_is_zero,
        tx.txn_type(),
        check_seq_proxy,
        check_prior_tx_and_last_ledger,
        check_permission,
        check_sign,
        calculate_base_fee,
        check_fee,
        run_create_preclaim,
        run_set_preclaim,
        run_delete_preclaim,
        run_deposit_preclaim,
        run_withdraw_preclaim,
        run_clawback_preclaim,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::{Ter, TxType};

    use super::{run_vault_invoke_preclaim_for_txn_source, run_vault_invoke_preclaim_for_txn_type};
    use crate::{HasTxnType, UnknownTransactionType};

    struct TestTx {
        txn_type: TxType,
    }

    impl HasTxnType for TestTx {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    #[test]
    fn vault_invoke_preclaim_skips_shared_checks_when_account_is_zero() {
        let result = run_vault_invoke_preclaim_for_txn_type(
            true,
            TxType::VAULT_CREATE,
            || panic!("zero account should skip seq-proxy"),
            || panic!("zero account should skip prior-tx"),
            || panic!("zero account should skip permission"),
            || panic!("zero account should skip sign"),
            || panic!("zero account should skip base-fee"),
            |_| panic!("zero account should skip fee"),
            || Ter::TEC_OBJECT_NOT_FOUND,
            || panic!("wrong vault preclaim selected"),
            || panic!("wrong vault preclaim selected"),
            || panic!("wrong vault preclaim selected"),
            || panic!("wrong vault preclaim selected"),
            || panic!("wrong vault preclaim selected"),
        );

        assert_eq!(result, Ok(Ter::TEC_OBJECT_NOT_FOUND));
    }

    #[test]
    fn vault_invoke_preclaim_preserves_current() {
        let trace = RefCell::new(Vec::new());

        let result = run_vault_invoke_preclaim_for_txn_type(
            false,
            TxType::VAULT_WITHDRAW,
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
                trace.borrow_mut().push("base-fee");
                20_u64
            },
            |fee| {
                trace.borrow_mut().push("fee");
                assert_eq!(fee, 20);
                Ter::TES_SUCCESS
            },
            || panic!("wrong vault preclaim selected"),
            || panic!("wrong vault preclaim selected"),
            || panic!("wrong vault preclaim selected"),
            || panic!("wrong vault preclaim selected"),
            || {
                trace.borrow_mut().push("withdraw-preclaim");
                Ter::TES_SUCCESS
            },
            || panic!("wrong vault preclaim selected"),
        );

        assert_eq!(result, Ok(Ter::TES_SUCCESS));
        assert_eq!(
            trace.into_inner(),
            vec![
                "seq",
                "prior",
                "permission",
                "sign",
                "base-fee",
                "fee",
                "withdraw-preclaim"
            ]
        );
    }

    #[test]
    fn vault_invoke_preclaim_returns_first_shared_failure_unchanged() {
        let result = run_vault_invoke_preclaim_for_txn_type(
            false,
            TxType::VAULT_SET,
            || Ter::TES_SUCCESS,
            || Ter::TEF_WRONG_PRIOR,
            || panic!("prior failure should skip permission"),
            || panic!("prior failure should skip sign"),
            || panic!("prior failure should skip base-fee"),
            |_| panic!("prior failure should skip fee"),
            || panic!("prior failure should skip set preclaim"),
            || panic!("wrong vault preclaim selected"),
            || panic!("wrong vault preclaim selected"),
            || panic!("wrong vault preclaim selected"),
            || panic!("wrong vault preclaim selected"),
            || panic!("wrong vault preclaim selected"),
        );

        assert_eq!(result, Ok(Ter::TEF_WRONG_PRIOR));
    }

    #[test]
    fn vault_invoke_preclaim_for_txn_source_maps_non_vault_types_to_unknown() {
        let tx = TestTx {
            txn_type: TxType::PAYMENT,
        };

        let result = run_vault_invoke_preclaim_for_txn_source(
            false,
            &tx,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || 20_u64,
            |_| Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Err(UnknownTransactionType::new(TxType::PAYMENT)));
    }
}
