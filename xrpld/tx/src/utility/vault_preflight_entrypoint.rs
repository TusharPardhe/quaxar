//! Public vault-family preflight entrypoint shape above the landed generic
//! the transaction dispatch layer shells.
//!
//! This ports the public composition layer that:
//!
//! - preserves the generic preflight exception mapping to `TEF_EXCEPTION`,
//! - preserves the current unknown-transaction fallback to `temUNKNOWN`,
//! - and composes the landed vault invoke-preflight and consequence builders
//!   without re-implementing the outer shell.

use protocol::{NotTec, SeqProxy, TxType};

use crate::apply_steps_entrypoint::run_preflight_with_context;
use crate::vault_invoke_preflight::run_vault_invoke_preflight_for_txn_type_with_consequences;
use crate::{
    HasTxnType, PreflightContext, PreflightResult, TxConsequences, UNKNOWN_TRANSACTION_TYPE_TER,
    run_vault_make_tx_consequences_for_txn_type, run_with_vault_txn_type_key, txn_type_of,
};

#[allow(clippy::too_many_arguments)]
fn run_vault_public_invoke_preflight(
    single_asset_vault_enabled: bool,
    txn_type: TxType,
    fee_drops: u64,
    seq_proxy: SeqProxy,
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
) -> Result<(NotTec, TxConsequences), ()> {
    match run_with_vault_txn_type_key(txn_type, |txn_type| txn_type) {
        Ok(_) => run_vault_invoke_preflight_for_txn_type_with_consequences(
            single_asset_vault_enabled,
            txn_type,
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
            |_| {
                run_vault_make_tx_consequences_for_txn_type(txn_type, fee_drops, seq_proxy)
                    .expect("classified vault txn type should build vault consequences")
            },
        )
        .map_err(|_| ()),
        Err(_) => Ok((
            UNKNOWN_TRANSACTION_TYPE_TER,
            TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_vault_public_preflight_for_txn_type<Registry, Tx, Journal, ParentBatchId>(
    ctx: PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    single_asset_vault_enabled: bool,
    txn_type: TxType,
    fee_drops: u64,
    seq_proxy: SeqProxy,
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
) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId> {
    run_preflight_with_context(
        ctx,
        |_| {
            run_vault_public_invoke_preflight(
                single_asset_vault_enabled,
                txn_type,
                fee_drops,
                seq_proxy,
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
        },
        |_| TxConsequences::new(fee_drops, seq_proxy),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_vault_public_preflight_for_txn_source<Registry, Tx, Journal, ParentBatchId>(
    ctx: PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    single_asset_vault_enabled: bool,
    fee_drops: u64,
    seq_proxy: SeqProxy,
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
) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>
where
    Tx: HasTxnType,
{
    let txn_type = txn_type_of(&ctx.tx);
    run_vault_public_preflight_for_txn_type(
        ctx,
        single_asset_vault_enabled,
        txn_type,
        fee_drops,
        seq_proxy,
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
