//! Vault-family `Transactor::invokePreflight<T>` composition shell above the
//! already-landed vault metadata, vault preflight dispatch, and shared
//! transactor preflight shells.
//!
//! This ports the exact current ordering around:
//!
//! - the shared `featureSingleAssetVault` gate,
//! - the per-type `checkExtraFeatures(...)` split,
//! - the base-versus-create flags-mask split,
//! - `preflight1(...)`,
//! - the selected vault `preflight(...)`,
//! - `preflight2(...)`,
//! - and the current vault-family base `preflightSigValidated(...)` success tail.

use protocol::{NotTec, Ter, TxType, is_tes_success};

use crate::{
    HasTxnType, TxConsequences, UnknownTransactionType, VaultTxnType,
    get_vault_flags_mask_for_txn_type, run_vault_check_extra_features_for_txn_type,
    run_with_vault_txn_type_key,
};

#[allow(clippy::too_many_arguments)]
fn run_vault_invoke_preflight_for_vault_txn_type(
    vault_txn_type: VaultTxnType,
    single_asset_vault_enabled: bool,
    check_create_extra_features: impl FnOnce() -> bool,
    check_set_extra_features: impl FnOnce() -> bool,
    create_flags_mask: impl FnOnce() -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_create_preflight: impl FnOnce() -> NotTec,
    run_set_preflight: impl FnOnce() -> NotTec,
    run_delete_preflight: impl FnOnce() -> NotTec,
    run_deposit_preflight: impl FnOnce() -> NotTec,
    run_withdraw_preflight: impl FnOnce() -> NotTec,
    run_clawback_preflight: impl FnOnce() -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
) -> NotTec {
    let txn_type = match vault_txn_type {
        VaultTxnType::Create => TxType::VAULT_CREATE,
        VaultTxnType::Set => TxType::VAULT_SET,
        VaultTxnType::Delete => TxType::VAULT_DELETE,
        VaultTxnType::Deposit => TxType::VAULT_DEPOSIT,
        VaultTxnType::Withdraw => TxType::VAULT_WITHDRAW,
        VaultTxnType::Clawback => TxType::VAULT_CLAWBACK,
    };

    if !single_asset_vault_enabled {
        return Ter::TEM_DISABLED;
    }

    let extra_features_enabled = match run_vault_check_extra_features_for_txn_type(
        txn_type,
        check_create_extra_features,
        check_set_extra_features,
    ) {
        Ok(enabled) => enabled,
        Err(_) => unreachable!("vault txn type already selected"),
    };
    if !extra_features_enabled {
        return Ter::TEM_DISABLED;
    }

    let flag_mask = match get_vault_flags_mask_for_txn_type(txn_type, create_flags_mask) {
        Ok(mask) => mask,
        Err(_) => unreachable!("vault txn type already selected"),
    };
    let ret = run_preflight1(flag_mask);
    if !is_tes_success(ret) {
        return ret;
    }

    let ret = match vault_txn_type {
        VaultTxnType::Create => run_create_preflight(),
        VaultTxnType::Set => run_set_preflight(),
        VaultTxnType::Delete => run_delete_preflight(),
        VaultTxnType::Deposit => run_deposit_preflight(),
        VaultTxnType::Withdraw => run_withdraw_preflight(),
        VaultTxnType::Clawback => run_clawback_preflight(),
    };
    if !is_tes_success(ret) {
        return ret;
    }

    let ret = run_preflight2();
    if !is_tes_success(ret) {
        return ret;
    }

    Ter::TES_SUCCESS
}

#[allow(clippy::too_many_arguments)]
pub fn run_vault_invoke_preflight_for_txn_type(
    single_asset_vault_enabled: bool,
    txn_type: TxType,
    check_create_extra_features: impl FnOnce() -> bool,
    check_set_extra_features: impl FnOnce() -> bool,
    create_flags_mask: impl FnOnce() -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_create_preflight: impl FnOnce() -> NotTec,
    run_set_preflight: impl FnOnce() -> NotTec,
    run_delete_preflight: impl FnOnce() -> NotTec,
    run_deposit_preflight: impl FnOnce() -> NotTec,
    run_withdraw_preflight: impl FnOnce() -> NotTec,
    run_clawback_preflight: impl FnOnce() -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
) -> Result<NotTec, UnknownTransactionType<TxType>> {
    run_with_vault_txn_type_key(txn_type, |vault_txn_type| {
        run_vault_invoke_preflight_for_vault_txn_type(
            vault_txn_type,
            single_asset_vault_enabled,
            check_create_extra_features,
            check_set_extra_features,
            create_flags_mask,
            run_preflight1,
            run_create_preflight,
            run_set_preflight,
            run_delete_preflight,
            run_deposit_preflight,
            run_withdraw_preflight,
            run_clawback_preflight,
            run_preflight2,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_vault_invoke_preflight_for_txn_type_with_consequences(
    single_asset_vault_enabled: bool,
    txn_type: TxType,
    check_create_extra_features: impl FnOnce() -> bool,
    check_set_extra_features: impl FnOnce() -> bool,
    create_flags_mask: impl FnOnce() -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_create_preflight: impl FnOnce() -> NotTec,
    run_set_preflight: impl FnOnce() -> NotTec,
    run_delete_preflight: impl FnOnce() -> NotTec,
    run_deposit_preflight: impl FnOnce() -> NotTec,
    run_withdraw_preflight: impl FnOnce() -> NotTec,
    run_clawback_preflight: impl FnOnce() -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
    run_success_consequences: impl FnOnce(VaultTxnType) -> TxConsequences,
) -> Result<(NotTec, TxConsequences), UnknownTransactionType<TxType>> {
    run_with_vault_txn_type_key(txn_type, |vault_txn_type| {
        let ter = run_vault_invoke_preflight_for_vault_txn_type(
            vault_txn_type,
            single_asset_vault_enabled,
            check_create_extra_features,
            check_set_extra_features,
            create_flags_mask,
            run_preflight1,
            run_create_preflight,
            run_set_preflight,
            run_delete_preflight,
            run_deposit_preflight,
            run_withdraw_preflight,
            run_clawback_preflight,
            run_preflight2,
        );
        let consequences = if is_tes_success(ter) {
            run_success_consequences(vault_txn_type)
        } else {
            TxConsequences::from_preflight_result(ter)
        };
        (ter, consequences)
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_vault_invoke_preflight_for_txn_source<Tx: HasTxnType + ?Sized>(
    single_asset_vault_enabled: bool,
    tx: &Tx,
    check_create_extra_features: impl FnOnce() -> bool,
    check_set_extra_features: impl FnOnce() -> bool,
    create_flags_mask: impl FnOnce() -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_create_preflight: impl FnOnce() -> NotTec,
    run_set_preflight: impl FnOnce() -> NotTec,
    run_delete_preflight: impl FnOnce() -> NotTec,
    run_deposit_preflight: impl FnOnce() -> NotTec,
    run_withdraw_preflight: impl FnOnce() -> NotTec,
    run_clawback_preflight: impl FnOnce() -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
) -> Result<NotTec, UnknownTransactionType<TxType>> {
    run_vault_invoke_preflight_for_txn_type(
        single_asset_vault_enabled,
        tx.txn_type(),
        check_create_extra_features,
        check_set_extra_features,
        create_flags_mask,
        run_preflight1,
        run_create_preflight,
        run_set_preflight,
        run_delete_preflight,
        run_deposit_preflight,
        run_withdraw_preflight,
        run_clawback_preflight,
        run_preflight2,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_vault_invoke_preflight_for_txn_source_with_consequences<Tx: HasTxnType + ?Sized>(
    single_asset_vault_enabled: bool,
    tx: &Tx,
    check_create_extra_features: impl FnOnce() -> bool,
    check_set_extra_features: impl FnOnce() -> bool,
    create_flags_mask: impl FnOnce() -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_create_preflight: impl FnOnce() -> NotTec,
    run_set_preflight: impl FnOnce() -> NotTec,
    run_delete_preflight: impl FnOnce() -> NotTec,
    run_deposit_preflight: impl FnOnce() -> NotTec,
    run_withdraw_preflight: impl FnOnce() -> NotTec,
    run_clawback_preflight: impl FnOnce() -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
    run_success_consequences: impl FnOnce(VaultTxnType) -> TxConsequences,
) -> Result<(NotTec, TxConsequences), UnknownTransactionType<TxType>> {
    run_vault_invoke_preflight_for_txn_type_with_consequences(
        single_asset_vault_enabled,
        tx.txn_type(),
        check_create_extra_features,
        check_set_extra_features,
        create_flags_mask,
        run_preflight1,
        run_create_preflight,
        run_set_preflight,
        run_delete_preflight,
        run_deposit_preflight,
        run_withdraw_preflight,
        run_clawback_preflight,
        run_preflight2,
        run_success_consequences,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::{SeqProxy, Ter, TxType};

    use super::{
        run_vault_invoke_preflight_for_txn_source,
        run_vault_invoke_preflight_for_txn_source_with_consequences,
        run_vault_invoke_preflight_for_txn_type,
        run_vault_invoke_preflight_for_txn_type_with_consequences,
    };
    use crate::{HasTxnType, TxConsequences, UnknownTransactionType};

    struct TestTx {
        txn_type: TxType,
    }

    impl HasTxnType for TestTx {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    #[test]
    fn vault_invoke_preflight_short_circuits_feature_gate_before_other_steps() {
        let result = run_vault_invoke_preflight_for_txn_type(
            false,
            TxType::VAULT_CREATE,
            || panic!("feature gate should skip create extra-features"),
            || panic!("feature gate should skip set extra-features"),
            || panic!("feature gate should skip create flags mask"),
            |_| panic!("feature gate should skip preflight1"),
            || panic!("feature gate should skip create preflight"),
            || panic!("feature gate should skip set preflight"),
            || panic!("feature gate should skip delete preflight"),
            || panic!("feature gate should skip deposit preflight"),
            || panic!("feature gate should skip withdraw preflight"),
            || panic!("feature gate should skip clawback preflight"),
            || panic!("feature gate should skip preflight2"),
        );

        assert_eq!(result, Ok(Ter::TEM_DISABLED));
    }

    #[test]
    fn vault_invoke_preflight_preserves_current_cpp_step_order() {
        let trace = RefCell::new(Vec::new());

        let result = run_vault_invoke_preflight_for_txn_type(
            true,
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
                trace.borrow_mut().push("create-flags");
                0x3ffc_ffff
            },
            |mask| {
                trace.borrow_mut().push("preflight1");
                assert_eq!(mask, 0x3ffc_ffff);
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("create-preflight");
                Ter::TES_SUCCESS
            },
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || {
                trace.borrow_mut().push("preflight2");
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ok(Ter::TES_SUCCESS));
        assert_eq!(
            trace.into_inner(),
            vec![
                "create-extra",
                "create-flags",
                "preflight1",
                "create-preflight",
                "preflight2"
            ]
        );
    }

    #[test]
    fn vault_invoke_preflight_returns_first_failure_unchanged() {
        let create_extra_failure = run_vault_invoke_preflight_for_txn_type(
            true,
            TxType::VAULT_CREATE,
            || false,
            || true,
            || panic!("failed create extra-features should skip flags mask"),
            |_| panic!("failed create extra-features should skip preflight1"),
            || panic!("failed create extra-features should skip create preflight"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("failed create extra-features should skip preflight2"),
        );
        let preflight1_failure = run_vault_invoke_preflight_for_txn_type(
            true,
            TxType::VAULT_DELETE,
            || true,
            || true,
            || panic!("delete path should not evaluate create flags mask"),
            |_| Ter::TEM_INVALID_FLAG,
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("preflight1 failure should skip delete preflight"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("preflight1 failure should skip preflight2"),
        );
        let preflight2_failure = run_vault_invoke_preflight_for_txn_type(
            true,
            TxType::VAULT_WITHDRAW,
            || true,
            || true,
            || panic!("withdraw path should not evaluate create flags mask"),
            |_| Ter::TES_SUCCESS,
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || Ter::TES_SUCCESS,
            || panic!("wrong vault preflight selected"),
            || Ter::TEM_INVALID,
        );

        assert_eq!(create_extra_failure, Ok(Ter::TEM_DISABLED));
        assert_eq!(preflight1_failure, Ok(Ter::TEM_INVALID_FLAG));
        assert_eq!(preflight2_failure, Ok(Ter::TEM_INVALID));
    }

    #[test]
    fn vault_invoke_preflight_uses_base_flags_mask_for_non_create_types() {
        let observed = RefCell::new(None);

        let result = run_vault_invoke_preflight_for_txn_type(
            true,
            TxType::VAULT_SET,
            || true,
            || true,
            || panic!("set path should not evaluate create flags mask"),
            |mask| {
                *observed.borrow_mut() = Some(mask);
                Ter::TES_SUCCESS
            },
            || panic!("wrong vault preflight selected"),
            || Ter::TES_SUCCESS,
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ok(Ter::TES_SUCCESS));
        assert_eq!(*observed.borrow(), Some(0x3fff_ffff));
    }

    #[test]
    fn vault_invoke_preflight_source_wrapper_preserves_unknowns_subset() {
        let tx = TestTx {
            txn_type: TxType::BATCH,
        };

        let result = run_vault_invoke_preflight_for_txn_source(
            true,
            &tx,
            || true,
            || true,
            || 0x3ffc_ffff,
            |_| Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Err(UnknownTransactionType::new(TxType::BATCH)));
    }

    #[test]
    fn vault_invoke_preflight_with_consequences_builds_success_consequences_only_on_success() {
        let consequences_called = RefCell::new(false);

        let result = run_vault_invoke_preflight_for_txn_type_with_consequences(
            true,
            TxType::VAULT_CREATE,
            || true,
            || true,
            || 0x3ffc_ffff,
            |_| Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            |vault_txn_type| {
                *consequences_called.borrow_mut() = true;
                assert_eq!(vault_txn_type, super::VaultTxnType::Create);
                TxConsequences::new(12, SeqProxy::sequence(5))
            },
        );

        assert_eq!(
            result,
            Ok((
                Ter::TES_SUCCESS,
                TxConsequences::new(12, SeqProxy::sequence(5))
            ))
        );
        assert!(*consequences_called.borrow());
    }

    #[test]
    fn vault_invoke_preflight_with_consequences_maps_failure_consequences() {
        let consequences_called = RefCell::new(false);

        let result = run_vault_invoke_preflight_for_txn_type_with_consequences(
            true,
            TxType::VAULT_DELETE,
            || true,
            || true,
            || panic!("delete path should not read create flags mask"),
            |_| Ter::TEM_INVALID_FLAG,
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            || panic!("wrong vault preflight selected"),
            |_vault_txn_type| {
                *consequences_called.borrow_mut() = true;
                TxConsequences::new(12, SeqProxy::sequence(5))
            },
        );

        assert_eq!(
            result,
            Ok((
                Ter::TEM_INVALID_FLAG,
                TxConsequences::from_preflight_result(Ter::TEM_INVALID_FLAG)
            ))
        );
        assert!(!*consequences_called.borrow());
    }

    #[test]
    fn vault_invoke_preflight_source_with_consequences_uses_txn_type_from_source_subset() {
        let tx = TestTx {
            txn_type: TxType::VAULT_WITHDRAW,
        };

        let result = run_vault_invoke_preflight_for_txn_source_with_consequences(
            true,
            &tx,
            || true,
            || true,
            || 0x3ffc_ffff,
            |_| Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            |vault_txn_type| {
                assert_eq!(vault_txn_type, super::VaultTxnType::Withdraw);
                TxConsequences::new(7, SeqProxy::ticket(9))
            },
        );

        assert_eq!(
            result,
            Ok((
                Ter::TES_SUCCESS,
                TxConsequences::new(7, SeqProxy::ticket(9))
            ))
        );
    }
}
