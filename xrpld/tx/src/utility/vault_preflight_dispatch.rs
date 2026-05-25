//! Vault-family preflight dispatch surface above the already-landed
//! per-transaction vault preflight helpers.
//!
//! This ports the exact current static dispatch behavior around:
//!
//! - selecting the six vault transaction types from the larger `TxType` set,
//! - applying the shared `featureSingleAssetVault` gate from
//!   `transactions.macro`,
//! - running `checkExtraFeatures(...)` only for `VaultCreate` and `VaultSet`,
//! - returning the current base `Transactor::getFlagsMask(...)` result for the
//!   five vault types that do not override it,
//! - and dispatching to the already-landed per-type vault preflight helpers
//!   with `temDISABLED` short-circuiting before the selected preflight runs.

use protocol::{NotTec, Ter, TxType};

use crate::{
    HasTxnType, TRANSACTOR_FULLY_CANONICAL_SIGNATURE_FLAG, UnknownTransactionType, VaultTxnType,
    run_transactor_get_flags_mask, run_with_vault_txn_type_key, run_with_vault_txn_type_source,
};

pub const FULLY_CANONICAL_SIGNATURE_FLAG: u32 = TRANSACTOR_FULLY_CANONICAL_SIGNATURE_FLAG;
pub const VAULT_BASE_FLAGS_MASK: u32 = run_transactor_get_flags_mask();

pub fn run_vault_check_extra_features_for_txn_type(
    txn_type: TxType,
    check_create_extra_features: impl FnOnce() -> bool,
    check_set_extra_features: impl FnOnce() -> bool,
) -> Result<bool, UnknownTransactionType<TxType>> {
    run_with_vault_txn_type_key(txn_type, |vault_txn_type| match vault_txn_type {
        VaultTxnType::Create => check_create_extra_features(),
        VaultTxnType::Set => check_set_extra_features(),
        VaultTxnType::Delete
        | VaultTxnType::Deposit
        | VaultTxnType::Withdraw
        | VaultTxnType::Clawback => true,
    })
}

pub fn run_vault_check_extra_features_for_txn_source<Tx: HasTxnType + ?Sized>(
    tx: &Tx,
    check_create_extra_features: impl FnOnce() -> bool,
    check_set_extra_features: impl FnOnce() -> bool,
) -> Result<bool, UnknownTransactionType<TxType>> {
    run_with_vault_txn_type_source(tx, |vault_txn_type| match vault_txn_type {
        VaultTxnType::Create => check_create_extra_features(),
        VaultTxnType::Set => check_set_extra_features(),
        VaultTxnType::Delete
        | VaultTxnType::Deposit
        | VaultTxnType::Withdraw
        | VaultTxnType::Clawback => true,
    })
}

pub fn get_vault_flags_mask_for_txn_type(
    txn_type: TxType,
    create_flags_mask: impl FnOnce() -> u32,
) -> Result<u32, UnknownTransactionType<TxType>> {
    run_with_vault_txn_type_key(txn_type, |vault_txn_type| match vault_txn_type {
        VaultTxnType::Create => create_flags_mask(),
        VaultTxnType::Set
        | VaultTxnType::Delete
        | VaultTxnType::Deposit
        | VaultTxnType::Withdraw
        | VaultTxnType::Clawback => VAULT_BASE_FLAGS_MASK,
    })
}

pub fn get_vault_flags_mask_for_txn_source<Tx: HasTxnType + ?Sized>(
    tx: &Tx,
    create_flags_mask: impl FnOnce() -> u32,
) -> Result<u32, UnknownTransactionType<TxType>> {
    run_with_vault_txn_type_source(tx, |vault_txn_type| match vault_txn_type {
        VaultTxnType::Create => create_flags_mask(),
        VaultTxnType::Set
        | VaultTxnType::Delete
        | VaultTxnType::Deposit
        | VaultTxnType::Withdraw
        | VaultTxnType::Clawback => VAULT_BASE_FLAGS_MASK,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_vault_preflight_for_txn_type(
    single_asset_vault_enabled: bool,
    txn_type: TxType,
    check_create_extra_features: impl FnOnce() -> bool,
    check_set_extra_features: impl FnOnce() -> bool,
    run_create_preflight: impl FnOnce() -> NotTec,
    run_set_preflight: impl FnOnce() -> NotTec,
    run_delete_preflight: impl FnOnce() -> NotTec,
    run_deposit_preflight: impl FnOnce() -> NotTec,
    run_withdraw_preflight: impl FnOnce() -> NotTec,
    run_clawback_preflight: impl FnOnce() -> NotTec,
) -> Result<NotTec, UnknownTransactionType<TxType>> {
    run_with_vault_txn_type_key(txn_type, |vault_txn_type| {
        if !single_asset_vault_enabled {
            return Ter::TEM_DISABLED;
        }

        let extra_features_enabled = match vault_txn_type {
            VaultTxnType::Create => check_create_extra_features(),
            VaultTxnType::Set => check_set_extra_features(),
            VaultTxnType::Delete
            | VaultTxnType::Deposit
            | VaultTxnType::Withdraw
            | VaultTxnType::Clawback => true,
        };
        if !extra_features_enabled {
            return Ter::TEM_DISABLED;
        }

        match vault_txn_type {
            VaultTxnType::Create => run_create_preflight(),
            VaultTxnType::Set => run_set_preflight(),
            VaultTxnType::Delete => run_delete_preflight(),
            VaultTxnType::Deposit => run_deposit_preflight(),
            VaultTxnType::Withdraw => run_withdraw_preflight(),
            VaultTxnType::Clawback => run_clawback_preflight(),
        }
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_vault_preflight_for_txn_source<Tx: HasTxnType + ?Sized>(
    single_asset_vault_enabled: bool,
    tx: &Tx,
    check_create_extra_features: impl FnOnce() -> bool,
    check_set_extra_features: impl FnOnce() -> bool,
    run_create_preflight: impl FnOnce() -> NotTec,
    run_set_preflight: impl FnOnce() -> NotTec,
    run_delete_preflight: impl FnOnce() -> NotTec,
    run_deposit_preflight: impl FnOnce() -> NotTec,
    run_withdraw_preflight: impl FnOnce() -> NotTec,
    run_clawback_preflight: impl FnOnce() -> NotTec,
) -> Result<NotTec, UnknownTransactionType<TxType>> {
    run_vault_preflight_for_txn_type(
        single_asset_vault_enabled,
        tx.txn_type(),
        check_create_extra_features,
        check_set_extra_features,
        run_create_preflight,
        run_set_preflight,
        run_delete_preflight,
        run_deposit_preflight,
        run_withdraw_preflight,
        run_clawback_preflight,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use protocol::{INNER_BATCH_TRANSACTION_FLAG, Ter, TxType};

    use super::{
        FULLY_CANONICAL_SIGNATURE_FLAG, VAULT_BASE_FLAGS_MASK, get_vault_flags_mask_for_txn_source,
        get_vault_flags_mask_for_txn_type, run_vault_check_extra_features_for_txn_source,
        run_vault_check_extra_features_for_txn_type, run_vault_preflight_for_txn_source,
        run_vault_preflight_for_txn_type,
    };
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
    fn vault_extra_features_dispatches_create_and_set_only() {
        let create_called = Cell::new(false);
        let set_called = Cell::new(false);

        let create = run_vault_check_extra_features_for_txn_type(
            TxType::VAULT_CREATE,
            || {
                create_called.set(true);
                false
            },
            || {
                set_called.set(true);
                true
            },
        );

        assert_eq!(create, Ok(false));
        assert!(create_called.get());
        assert!(!set_called.get());
    }

    #[test]
    fn vault_extra_features_skips_wrappers_for_other_vault_types() {
        let create_called = Cell::new(false);
        let set_called = Cell::new(false);

        let deposit = run_vault_check_extra_features_for_txn_type(
            TxType::VAULT_DEPOSIT,
            || {
                create_called.set(true);
                false
            },
            || {
                set_called.set(true);
                false
            },
        );

        assert_eq!(deposit, Ok(true));
        assert!(!create_called.get());
        assert!(!set_called.get());
    }

    #[test]
    fn vault_flags_mask_keeps_current_create_override_and_base_default_split() {
        let create_mask = get_vault_flags_mask_for_txn_type(TxType::VAULT_CREATE, || 0x3ffc_ffff);
        let delete_mask = get_vault_flags_mask_for_txn_type(TxType::VAULT_DELETE, || {
            panic!("base vault transactors should not call create flags helper")
        });

        assert_eq!(FULLY_CANONICAL_SIGNATURE_FLAG, 0x8000_0000);
        assert_eq!(INNER_BATCH_TRANSACTION_FLAG, 0x4000_0000);
        assert_eq!(VAULT_BASE_FLAGS_MASK, 0x3fff_ffff);
        assert_eq!(create_mask, Ok(0x3ffc_ffff));
        assert_eq!(delete_mask, Ok(0x3fff_ffff));
    }

    #[test]
    fn vault_preflight_dispatch_short_circuits_temdisabled_before_selected_preflight() {
        let trace = RefCell::new(Vec::new());

        let result = run_vault_preflight_for_txn_type(
            false,
            TxType::VAULT_CREATE,
            || {
                trace.borrow_mut().push("create-extra");
                true
            },
            || {
                trace.borrow_mut().push("set-extra");
                true
            },
            || {
                trace.borrow_mut().push("create-preflight");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("set-preflight");
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ok(Ter::TEM_DISABLED));
        assert!(trace.borrow().is_empty());
    }

    #[test]
    fn vault_preflight_dispatch_runs_selected_extra_features_before_preflight() {
        let trace = RefCell::new(Vec::new());

        let result = run_vault_preflight_for_txn_type(
            true,
            TxType::VAULT_SET,
            || {
                trace.borrow_mut().push("create-extra");
                true
            },
            || {
                trace.borrow_mut().push("set-extra");
                true
            },
            || {
                trace.borrow_mut().push("create-preflight");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("set-preflight");
                Ter::TEM_MALFORMED
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ok(Ter::TEM_MALFORMED));
        assert_eq!(trace.into_inner(), vec!["set-extra", "set-preflight"]);
    }

    #[test]
    fn vault_preflight_dispatch_source_and_mask_wrappers_preserve_unknowns_subset() {
        let tx = TestTx {
            txn_type: TxType::BATCH,
        };

        let extra = run_vault_check_extra_features_for_txn_source(&tx, || true, || true);
        let mask = get_vault_flags_mask_for_txn_source(&tx, || 0x3ffc_ffff);
        let preflight = run_vault_preflight_for_txn_source(
            true,
            &tx,
            || true,
            || true,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(extra, Err(UnknownTransactionType::new(TxType::BATCH)));
        assert_eq!(mask, Err(UnknownTransactionType::new(TxType::BATCH)));
        assert_eq!(preflight, Err(UnknownTransactionType::new(TxType::BATCH)));
    }
}
