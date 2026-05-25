//! Public system-family preflight entrypoint shape above the landed generic
//! the transaction dispatch layer shells.
//!
//! This ports the public composition layer that:
//!
//! - routes only the current system-family transaction subset,
//! - preserves the generic preflight exception mapping to `TEF_EXCEPTION`,
//! - preserves the current unknown-transaction fallback to `temUNKNOWN` for
//!   non-system types,
//! - and lets callers compose the landed system invoke-preflight and system
//!   consequence builders without re-implementing the outer shell.

use protocol::{NotTec, SeqProxy, TxType, is_tes_success};

use crate::apply_steps_entrypoint::{
    run_preflight_for_txn_type_with_consequences, run_preflight_with_context,
};
use crate::system_invoke_preflight::run_system_invoke_preflight_for_txn_type_with_consequences;
use crate::{
    HasTxnType, PreflightContext, PreflightResult, SystemTransactorTxnType, SystemTxnType,
    TxConsequences, UNKNOWN_TRANSACTION_TYPE_TER, run_change_invoke_preflight_for_txn_type,
    run_system_make_tx_consequences_entrypoint_for_txn_type,
    run_with_system_transactor_txn_type_key, txn_type_of,
};

pub fn run_system_preflight_with_context<Registry, Tx, Journal, ParentBatchId, E>(
    ctx: PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    invoke_preflight: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    ) -> Result<(NotTec, TxConsequences), E>,
    fallback_consequences: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    ) -> TxConsequences,
) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId> {
    run_preflight_with_context(ctx, invoke_preflight, fallback_consequences)
}

#[allow(clippy::too_many_arguments)]
pub fn run_system_preflight_for_txn_type_with_consequences<
    Registry,
    Tx,
    Journal,
    ParentBatchId,
    E,
>(
    ctx: PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    txn_type: TxType,
    dispatch_preflight: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<NotTec, E>,
    dispatch_success_consequences: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> TxConsequences,
    fallback_consequences: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    ) -> TxConsequences,
) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId> {
    if run_with_system_transactor_txn_type_key(txn_type, |_| ()).is_err() {
        return run_system_preflight_with_context(
            ctx,
            |_ctx| {
                Ok::<(NotTec, TxConsequences), E>((
                    UNKNOWN_TRANSACTION_TYPE_TER,
                    TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
                ))
            },
            fallback_consequences,
        );
    }

    run_preflight_for_txn_type_with_consequences(
        ctx,
        txn_type,
        dispatch_preflight,
        dispatch_success_consequences,
        fallback_consequences,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_system_preflight_for_txn_source_with_consequences<
    Registry,
    Tx: HasTxnType,
    Journal,
    ParentBatchId,
    E,
>(
    ctx: PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    dispatch_preflight: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<NotTec, E>,
    dispatch_success_consequences: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> TxConsequences,
    fallback_consequences: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    ) -> TxConsequences,
) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId> {
    let txn_type = txn_type_of(&ctx.tx);
    run_system_preflight_for_txn_type_with_consequences(
        ctx,
        txn_type,
        dispatch_preflight,
        dispatch_success_consequences,
        fallback_consequences,
    )
}

fn build_system_success_consequences(
    txn_type: TxType,
    fee_drops: u64,
    seq_proxy: SeqProxy,
    ticket_count: u32,
) -> Result<TxConsequences, ()> {
    run_system_make_tx_consequences_entrypoint_for_txn_type(
        txn_type,
        fee_drops,
        seq_proxy,
        ticket_count,
    )
    .map_err(|_| ())
}

#[allow(clippy::too_many_arguments)]
fn run_system_public_invoke_preflight(
    txn_type: TxType,
    fee_drops: u64,
    seq_proxy: SeqProxy,
    ticket_count: u32,
    lending_protocol_enabled: bool,
    run_change_flags_mask: impl FnOnce() -> u32,
    run_preflight0: impl FnOnce(u32) -> NotTec,
    run_change_preflight: impl FnOnce() -> NotTec,
    feature_gate_enabled: impl FnOnce(SystemTxnType) -> bool,
    check_extra_features: impl FnOnce(SystemTxnType) -> bool,
    get_flags_mask: impl FnOnce(SystemTxnType) -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_preflight: impl FnOnce(SystemTxnType) -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
    run_preflight_sig_validated: impl FnOnce() -> NotTec,
) -> Result<(NotTec, TxConsequences), ()> {
    match run_with_system_transactor_txn_type_key(txn_type, |txn_type| txn_type) {
        Ok(SystemTransactorTxnType::Change(_)) => {
            let ter = run_change_invoke_preflight_for_txn_type(
                txn_type,
                lending_protocol_enabled,
                run_change_flags_mask,
                run_preflight0,
                run_change_preflight,
            )
            .map_err(|_| ())?;
            let consequences = if is_tes_success(ter) {
                build_system_success_consequences(txn_type, fee_drops, seq_proxy, ticket_count)?
            } else {
                TxConsequences::from_preflight_result(ter)
            };
            Ok((ter, consequences))
        }
        Ok(
            SystemTransactorTxnType::Batch
            | SystemTransactorTxnType::TicketCreate
            | SystemTransactorTxnType::LedgerStateFix,
        ) => run_system_invoke_preflight_for_txn_type_with_consequences(
            txn_type,
            feature_gate_enabled,
            check_extra_features,
            get_flags_mask,
            run_preflight1,
            run_preflight,
            run_preflight2,
            run_preflight_sig_validated,
            |_| {
                build_system_success_consequences(txn_type, fee_drops, seq_proxy, ticket_count)
                    .expect("classified system txn type should build system consequences")
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
pub fn run_system_preflight_for_txn_type<Registry, Tx, Journal, ParentBatchId>(
    ctx: PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    txn_type: TxType,
    fee_drops: u64,
    seq_proxy: SeqProxy,
    ticket_count: u32,
    lending_protocol_enabled: bool,
    run_change_flags_mask: impl FnOnce() -> u32,
    run_preflight0: impl FnOnce(u32) -> NotTec,
    run_change_preflight: impl FnOnce() -> NotTec,
    feature_gate_enabled: impl FnOnce(SystemTxnType) -> bool,
    check_extra_features: impl FnOnce(SystemTxnType) -> bool,
    get_flags_mask: impl FnOnce(SystemTxnType) -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_preflight: impl FnOnce(SystemTxnType) -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
    run_preflight_sig_validated: impl FnOnce() -> NotTec,
) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId> {
    run_system_preflight_with_context(
        ctx,
        |_| {
            run_system_public_invoke_preflight(
                txn_type,
                fee_drops,
                seq_proxy,
                ticket_count,
                lending_protocol_enabled,
                run_change_flags_mask,
                run_preflight0,
                run_change_preflight,
                feature_gate_enabled,
                check_extra_features,
                get_flags_mask,
                run_preflight1,
                run_preflight,
                run_preflight2,
                run_preflight_sig_validated,
            )
        },
        |_| TxConsequences::new(fee_drops, seq_proxy),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_system_preflight_for_txn_source<Registry, Tx, Journal, ParentBatchId>(
    ctx: PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    fee_drops: u64,
    seq_proxy: SeqProxy,
    ticket_count: u32,
    lending_protocol_enabled: bool,
    run_change_flags_mask: impl FnOnce() -> u32,
    run_preflight0: impl FnOnce(u32) -> NotTec,
    run_change_preflight: impl FnOnce() -> NotTec,
    feature_gate_enabled: impl FnOnce(SystemTxnType) -> bool,
    check_extra_features: impl FnOnce(SystemTxnType) -> bool,
    get_flags_mask: impl FnOnce(SystemTxnType) -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_preflight: impl FnOnce(SystemTxnType) -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
    run_preflight_sig_validated: impl FnOnce() -> NotTec,
) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>
where
    Tx: HasTxnType,
{
    let txn_type = txn_type_of(&ctx.tx);
    run_system_preflight_for_txn_type(
        ctx,
        txn_type,
        fee_drops,
        seq_proxy,
        ticket_count,
        lending_protocol_enabled,
        run_change_flags_mask,
        run_preflight0,
        run_change_preflight,
        feature_gate_enabled,
        check_extra_features,
        get_flags_mask,
        run_preflight1,
        run_preflight,
        run_preflight2,
        run_preflight_sig_validated,
    )
}
