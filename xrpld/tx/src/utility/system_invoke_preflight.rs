//! System-family `Transactor::invokePreflight<T>` dispatch layer.
//!
//! This ports the deterministic selection and composition shell around:
//!
//! - routing the current system-family transaction subset by `TxType`,
//! - preserving the regular `invokePreflight<T>` order for Batch,
//!   TicketCreate, and LedgerStateFix,
//! - preserving the `Change` specialization order that starts with
//!   `preflight0(...)`,
//! - and keeping unknown or non-system transaction types mapped to
//!   `UnknownTransactionType`.

use protocol::{NotTec, Ter, TxType, is_tes_success};

use crate::{HasTxnType, TxConsequences, UnknownTransactionType, txn_type_of};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemTxnType {
    Batch,
    TicketCreate,
    LedgerStateFix,
}

pub fn classify_system_txn_type(txn_type: TxType) -> Option<SystemTxnType> {
    match txn_type {
        TxType::BATCH => Some(SystemTxnType::Batch),
        TxType::TICKET_CREATE => Some(SystemTxnType::TicketCreate),
        TxType::LEDGER_STATE_FIX => Some(SystemTxnType::LedgerStateFix),
        _ => None,
    }
}

pub fn run_with_system_txn_type_key<R>(
    txn_type: TxType,
    dispatch: impl FnOnce(SystemTxnType) -> R,
) -> Result<R, UnknownTransactionType<TxType>> {
    match classify_system_txn_type(txn_type) {
        Some(system_txn_type) => Ok(dispatch(system_txn_type)),
        None => Err(UnknownTransactionType::new(txn_type)),
    }
}

pub fn run_with_system_txn_type_source<Tx: HasTxnType + ?Sized, R>(
    tx: &Tx,
    dispatch: impl FnOnce(SystemTxnType) -> R,
) -> Result<R, UnknownTransactionType<TxType>> {
    run_with_system_txn_type_key(txn_type_of(tx), dispatch)
}

fn run_system_invoke_preflight_for_system_txn_type(
    system_txn_type: SystemTxnType,
    feature_gate_enabled: impl FnOnce(SystemTxnType) -> bool,
    check_extra_features: impl FnOnce(SystemTxnType) -> bool,
    get_flags_mask: impl FnOnce(SystemTxnType) -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_preflight: impl FnOnce(SystemTxnType) -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
    run_preflight_sig_validated: impl FnOnce() -> NotTec,
) -> NotTec {
    if matches!(
        system_txn_type,
        SystemTxnType::Batch | SystemTxnType::LedgerStateFix
    ) && !feature_gate_enabled(system_txn_type)
    {
        return Ter::TEM_DISABLED;
    }

    if !check_extra_features(system_txn_type) {
        return Ter::TEM_DISABLED;
    }

    let ret = run_preflight1(get_flags_mask(system_txn_type));
    if !is_tes_success(ret) {
        return ret;
    }

    let ret = run_preflight(system_txn_type);
    if !is_tes_success(ret) {
        return ret;
    }

    let ret = run_preflight2();
    if !is_tes_success(ret) {
        return ret;
    }

    let ret = run_preflight_sig_validated();
    if !is_tes_success(ret) {
        return ret;
    }

    Ter::TES_SUCCESS
}

#[allow(clippy::too_many_arguments)]
pub fn run_system_invoke_preflight_for_txn_type(
    txn_type: TxType,
    feature_gate_enabled: impl FnOnce(SystemTxnType) -> bool,
    check_extra_features: impl FnOnce(SystemTxnType) -> bool,
    get_flags_mask: impl FnOnce(SystemTxnType) -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_preflight: impl FnOnce(SystemTxnType) -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
    run_preflight_sig_validated: impl FnOnce() -> NotTec,
) -> Result<NotTec, UnknownTransactionType<TxType>> {
    run_with_system_txn_type_key(txn_type, |system_txn_type| {
        run_system_invoke_preflight_for_system_txn_type(
            system_txn_type,
            feature_gate_enabled,
            check_extra_features,
            get_flags_mask,
            run_preflight1,
            run_preflight,
            run_preflight2,
            run_preflight_sig_validated,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_system_invoke_preflight_for_txn_type_with_consequences(
    txn_type: TxType,
    feature_gate_enabled: impl FnOnce(SystemTxnType) -> bool,
    check_extra_features: impl FnOnce(SystemTxnType) -> bool,
    get_flags_mask: impl FnOnce(SystemTxnType) -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_preflight: impl FnOnce(SystemTxnType) -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
    run_preflight_sig_validated: impl FnOnce() -> NotTec,
    run_success_consequences: impl FnOnce(SystemTxnType) -> TxConsequences,
) -> Result<(NotTec, TxConsequences), UnknownTransactionType<TxType>> {
    run_with_system_txn_type_key(txn_type, |system_txn_type| {
        let ter = run_system_invoke_preflight_for_system_txn_type(
            system_txn_type,
            feature_gate_enabled,
            check_extra_features,
            get_flags_mask,
            run_preflight1,
            run_preflight,
            run_preflight2,
            run_preflight_sig_validated,
        );
        let consequences = if is_tes_success(ter) {
            run_success_consequences(system_txn_type)
        } else {
            TxConsequences::from_preflight_result(ter)
        };
        (ter, consequences)
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_system_invoke_preflight_for_txn_source<Tx: HasTxnType + ?Sized>(
    tx: &Tx,
    feature_gate_enabled: impl FnOnce(SystemTxnType) -> bool,
    check_extra_features: impl FnOnce(SystemTxnType) -> bool,
    get_flags_mask: impl FnOnce(SystemTxnType) -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_preflight: impl FnOnce(SystemTxnType) -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
    run_preflight_sig_validated: impl FnOnce() -> NotTec,
) -> Result<NotTec, UnknownTransactionType<TxType>> {
    run_system_invoke_preflight_for_txn_type(
        txn_type_of(tx),
        feature_gate_enabled,
        check_extra_features,
        get_flags_mask,
        run_preflight1,
        run_preflight,
        run_preflight2,
        run_preflight_sig_validated,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_system_invoke_preflight_for_txn_source_with_consequences<Tx: HasTxnType + ?Sized>(
    tx: &Tx,
    feature_gate_enabled: impl FnOnce(SystemTxnType) -> bool,
    check_extra_features: impl FnOnce(SystemTxnType) -> bool,
    get_flags_mask: impl FnOnce(SystemTxnType) -> u32,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_preflight: impl FnOnce(SystemTxnType) -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
    run_preflight_sig_validated: impl FnOnce() -> NotTec,
    run_success_consequences: impl FnOnce(SystemTxnType) -> TxConsequences,
) -> Result<(NotTec, TxConsequences), UnknownTransactionType<TxType>> {
    run_system_invoke_preflight_for_txn_type_with_consequences(
        txn_type_of(tx),
        feature_gate_enabled,
        check_extra_features,
        get_flags_mask,
        run_preflight1,
        run_preflight,
        run_preflight2,
        run_preflight_sig_validated,
        run_success_consequences,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_change_invoke_preflight_for_txn_type(
    txn_type: TxType,
    lending_protocol_enabled: bool,
    run_change_flags_mask: impl FnOnce() -> u32,
    run_preflight0: impl FnOnce(u32) -> NotTec,
    run_change_preflight: impl FnOnce() -> NotTec,
) -> Result<NotTec, UnknownTransactionType<TxType>> {
    match txn_type {
        TxType::AMENDMENT | TxType::FEE | TxType::UNL_MODIFY => {
            let flag_mask = if lending_protocol_enabled {
                run_change_flags_mask()
            } else {
                0
            };

            let ret = run_preflight0(flag_mask);
            if !is_tes_success(ret) {
                return Ok(ret);
            }

            let ret = run_change_preflight();
            if !is_tes_success(ret) {
                return Ok(ret);
            }

            Ok(Ter::TES_SUCCESS)
        }
        other => Err(UnknownTransactionType::new(other)),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_change_invoke_preflight_for_txn_source<Tx: HasTxnType + ?Sized>(
    tx: &Tx,
    lending_protocol_enabled: bool,
    run_change_flags_mask: impl FnOnce() -> u32,
    run_preflight0: impl FnOnce(u32) -> NotTec,
    run_change_preflight: impl FnOnce() -> NotTec,
) -> Result<NotTec, UnknownTransactionType<TxType>> {
    run_change_invoke_preflight_for_txn_type(
        txn_type_of(tx),
        lending_protocol_enabled,
        run_change_flags_mask,
        run_preflight0,
        run_change_preflight,
    )
}
