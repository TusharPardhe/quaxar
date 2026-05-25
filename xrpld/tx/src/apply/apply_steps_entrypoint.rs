//! Public `xrpl/tx/the transaction dispatch layer` preflight and preclaim shells.
//!
//! This ports the deterministic control flow above transaction-type dispatch:
//! preflight exception mapping, preclaim reflight when rules changed, and the
//! early-return path when preflight already failed.

use protocol::{NotTec, Rules, Ter, TxType, is_tes_success};

use crate::{
    ApplyContext, ApplyResult, HasTxnType, PreclaimContext, PreclaimResult, PreflightContext,
    PreflightResult, TxConsequences, txn_type_of,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownTransactionType<TxnType> {
    pub txn_type: TxnType,
}

impl<TxnType> UnknownTransactionType<TxnType> {
    pub fn new(txn_type: TxnType) -> Self {
        Self { txn_type }
    }
}

/// Current reference `temUNKNOWN` fallback used by the internal the transaction dispatch layer
/// transaction-type dispatch shells when no concrete transactor exists.
pub const UNKNOWN_TRANSACTION_TYPE_TER: Ter = Ter::TEM_UNKNOWN;

pub fn run_with_txn_type_key<R>(
    rules: &Rules,
    txn_type: TxType,
    dispatch: impl FnOnce(TxType) -> R,
) -> Result<R, UnknownTransactionType<TxType>> {
    crate::runtime::with_transaction_step_runtime(rules, || {
        if txn_type.is_dispatchable() {
            Ok(dispatch(txn_type))
        } else {
            Err(UnknownTransactionType::new(txn_type))
        }
    })
}

pub fn run_with_txn_type_source<Tx: HasTxnType + ?Sized, R>(
    rules: &Rules,
    tx: &Tx,
    dispatch: impl FnOnce(TxType) -> R,
) -> Result<R, UnknownTransactionType<TxType>> {
    run_with_txn_type_key(rules, txn_type_of(tx), dispatch)
}

pub fn run_preflight_with_context<Registry, Tx, Journal, ParentBatchId, E>(
    ctx: PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    invoke_preflight: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    ) -> Result<(NotTec, TxConsequences), E>,
    fallback_consequences: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    ) -> TxConsequences,
) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId> {
    let outcome = match invoke_preflight(&ctx) {
        Ok((ter, consequences)) => (ter, consequences),
        Err(_) => (Ter::TEF_EXCEPTION, fallback_consequences(&ctx)),
    };

    let PreflightContext {
        tx,
        rules,
        flags,
        parent_batch_id,
        journal,
        ..
    } = ctx;

    PreflightResult::new(
        tx,
        parent_batch_id,
        rules,
        outcome.1,
        flags,
        journal,
        outcome.0,
    )
}

pub fn run_preflight_for_txn_type_with_consequences<Registry, Tx, Journal, ParentBatchId, E>(
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
    run_preflight_with_context(
        ctx,
        |ctx| match run_with_txn_type_key(&ctx.rules, txn_type, |txn_type| {
            dispatch_preflight(ctx, txn_type)
        }) {
            Ok(Ok(ter)) => {
                let consequences = if is_tes_success(ter) {
                    dispatch_success_consequences(ctx, txn_type)
                } else {
                    TxConsequences::from_preflight_result(ter)
                };
                Ok((ter, consequences))
            }
            Ok(Err(_)) => Err(()),
            Err(_) => Ok((
                UNKNOWN_TRANSACTION_TYPE_TER,
                TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
            )),
        },
        fallback_consequences,
    )
}

pub fn run_preflight_for_txn_source_with_consequences<Registry, Tx, Journal, ParentBatchId, E>(
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
) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>
where
    Tx: HasTxnType,
{
    let txn_type = txn_type_of(&ctx.tx);
    run_preflight_for_txn_type_with_consequences(
        ctx,
        txn_type,
        dispatch_preflight,
        dispatch_success_consequences,
        fallback_consequences,
    )
}

pub fn run_preflight_for_txn_type<Registry, Tx, Journal, ParentBatchId, E>(
    ctx: PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    txn_type: TxType,
    dispatch: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<(NotTec, TxConsequences), E>,
    fallback_consequences: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    ) -> TxConsequences,
) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId> {
    run_preflight_with_context(
        ctx,
        |ctx| match run_with_txn_type_key(&ctx.rules, txn_type, |txn_type| dispatch(ctx, txn_type))
        {
            Ok(result) => result,
            Err(_) => Ok((
                UNKNOWN_TRANSACTION_TYPE_TER,
                TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
            )),
        },
        fallback_consequences,
    )
}

pub fn run_preflight_for_txn_source<Registry, Tx, Journal, ParentBatchId, E>(
    ctx: PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    dispatch: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<(NotTec, TxConsequences), E>,
    fallback_consequences: impl FnOnce(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    ) -> TxConsequences,
) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>
where
    Tx: HasTxnType,
{
    let txn_type = txn_type_of(&ctx.tx);
    run_preflight_for_txn_type(ctx, txn_type, dispatch, fallback_consequences)
}

pub fn run_preclaim_with_context<Registry, View, Tx, Journal, ParentBatchId, E>(
    preflight_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    registry: Registry,
    view: View,
    current_rules: &Rules,
    current_ledger_seq: u32,
    rerun_preflight: impl FnOnce(
        &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        &Rules,
    ) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    invoke_preclaim: impl FnOnce(
        &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
    ) -> Result<Ter, E>,
) -> PreclaimResult<Tx, Journal, ParentBatchId>
where
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    let effective_preflight = if preflight_result.rules != *current_rules {
        rerun_preflight(&preflight_result, current_rules)
    } else {
        preflight_result
    };

    let ctx = PreclaimContext::new_with_parent_batch_id(
        registry,
        view,
        effective_preflight.ter,
        effective_preflight.tx,
        effective_preflight.flags,
        effective_preflight.parent_batch_id,
        effective_preflight.journal,
    );

    if !is_tes_success(ctx.preflight_result) {
        let preflight_result = ctx.preflight_result;
        return preclaim_result_from_context(ctx, current_ledger_seq, preflight_result);
    }

    match invoke_preclaim(&ctx) {
        Ok(ter) => preclaim_result_from_context(ctx, current_ledger_seq, ter),
        Err(_) => preclaim_result_from_context(ctx, current_ledger_seq, Ter::TEF_EXCEPTION),
    }
}

pub fn run_preclaim_for_txn_type<Registry, View, Tx, Journal, ParentBatchId, E>(
    preflight_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    txn_type: TxType,
    registry: Registry,
    view: View,
    current_rules: &Rules,
    current_ledger_seq: u32,
    rerun_preflight: impl FnOnce(
        &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        &Rules,
    ) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    dispatch: impl FnOnce(
        &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<Ter, E>,
) -> PreclaimResult<Tx, Journal, ParentBatchId>
where
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    run_preclaim_with_context(
        preflight_result,
        registry,
        view,
        current_rules,
        current_ledger_seq,
        rerun_preflight,
        |ctx| match run_with_txn_type_key(current_rules, txn_type, |txn_type| {
            dispatch(ctx, txn_type)
        }) {
            Ok(result) => result,
            Err(_) => Ok(UNKNOWN_TRANSACTION_TYPE_TER),
        },
    )
}

pub fn run_preclaim_for_txn_source<Registry, View, Tx, Journal, ParentBatchId, E>(
    preflight_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    registry: Registry,
    view: View,
    current_rules: &Rules,
    current_ledger_seq: u32,
    rerun_preflight: impl FnOnce(
        &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        &Rules,
    ) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    dispatch: impl FnOnce(
        &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<Ter, E>,
) -> PreclaimResult<Tx, Journal, ParentBatchId>
where
    Tx: HasTxnType + Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    let txn_type = txn_type_of(&preflight_result.tx);
    run_preclaim_for_txn_type(
        preflight_result,
        txn_type,
        registry,
        view,
        current_rules,
        current_ledger_seq,
        rerun_preflight,
        dispatch,
    )
}

pub fn run_do_apply_with_context<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId, E>(
    preclaim_result: PreclaimResult<Tx, Journal, ParentBatchId>,
    registry: Registry,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx) -> Fee,
    invoke_apply: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
    ) -> Result<ApplyResult, E>,
) -> ApplyResult {
    if let Some(result) = preclaim_result.early_apply_result(current_ledger_seq) {
        return result;
    }

    let PreclaimResult {
        tx,
        parent_batch_id,
        flags,
        journal,
        ter,
        ..
    } = preclaim_result;

    let base_fee = calculate_base_fee(&base, &tx);
    let mut ctx = match parent_batch_id {
        Some(parent_batch_id) => ApplyContext::new_batch(
            registry,
            base,
            view,
            parent_batch_id,
            tx,
            ter,
            base_fee,
            flags,
            journal,
        ),
        None => ApplyContext::new(registry, base, view, tx, ter, base_fee, flags, journal),
    };

    match invoke_apply(&mut ctx) {
        Ok(result) => result,
        Err(_) => ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
    }
}

pub fn run_do_apply_for_txn_type<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId, E>(
    preclaim_result: PreclaimResult<Tx, Journal, ParentBatchId>,
    rules: &Rules,
    txn_type: TxType,
    registry: Registry,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx, TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
    dispatch: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
        TxType,
    ) -> Result<ApplyResult, E>,
) -> ApplyResult {
    run_do_apply_with_context(
        preclaim_result,
        registry,
        current_ledger_seq,
        base,
        view,
        |base, tx| {
            run_calculate_base_fee_for_txn_type(
                rules,
                txn_type,
                |txn_type| calculate_base_fee(base, tx, txn_type),
                zero_fee,
            )
        },
        |ctx| {
            run_invoke_apply_result_for_txn_type(rules, txn_type, |txn_type| {
                dispatch(ctx, txn_type)
            })
        },
    )
}

pub fn run_do_apply_for_txn_source<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId, E>(
    preclaim_result: PreclaimResult<Tx, Journal, ParentBatchId>,
    rules: &Rules,
    registry: Registry,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx, TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
    dispatch: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
        TxType,
    ) -> Result<ApplyResult, E>,
) -> ApplyResult
where
    Tx: HasTxnType,
{
    let txn_type = txn_type_of(&preclaim_result.tx);
    run_do_apply_for_txn_type(
        preclaim_result,
        rules,
        txn_type,
        registry,
        current_ledger_seq,
        base,
        view,
        calculate_base_fee,
        zero_fee,
        dispatch,
    )
}

pub fn run_invoke_preflight_with_context<TxnType>(
    rules: &Rules,
    invoke_preflight: impl FnOnce() -> Result<(NotTec, TxConsequences), UnknownTransactionType<TxnType>>,
) -> (NotTec, TxConsequences) {
    crate::runtime::with_transaction_step_runtime(rules, || match invoke_preflight() {
        Ok(result) => result,
        Err(_) => (
            UNKNOWN_TRANSACTION_TYPE_TER,
            TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
        ),
    })
}

pub fn run_invoke_preflight_with_consequences<TxnType>(
    rules: &Rules,
    invoke_preflight: impl FnOnce() -> Result<NotTec, UnknownTransactionType<TxnType>>,
    success_consequences: impl FnOnce() -> TxConsequences,
) -> (NotTec, TxConsequences) {
    run_invoke_preflight_with_context(rules, || {
        invoke_preflight().map(|ter| {
            let consequences = if is_tes_success(ter) {
                success_consequences()
            } else {
                TxConsequences::from_preflight_result(ter)
            };
            (ter, consequences)
        })
    })
}

pub fn run_invoke_preflight_for_txn_type(
    rules: &Rules,
    txn_type: TxType,
    dispatch: impl FnOnce(TxType) -> (NotTec, TxConsequences),
) -> (NotTec, TxConsequences) {
    match run_with_txn_type_key(rules, txn_type, dispatch) {
        Ok(result) => result,
        Err(_) => (
            UNKNOWN_TRANSACTION_TYPE_TER,
            TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
        ),
    }
}

pub fn run_invoke_preflight_for_txn_type_with_consequences(
    rules: &Rules,
    txn_type: TxType,
    dispatch_preflight: impl FnOnce(TxType) -> NotTec,
    dispatch_success_consequences: impl FnOnce(TxType) -> TxConsequences,
) -> (NotTec, TxConsequences) {
    run_invoke_preflight_for_txn_type(rules, txn_type, |txn_type| {
        let ter = dispatch_preflight(txn_type);
        let consequences = if is_tes_success(ter) {
            dispatch_success_consequences(txn_type)
        } else {
            TxConsequences::from_preflight_result(ter)
        };
        (ter, consequences)
    })
}

pub fn run_invoke_preflight_for_txn_source<Tx: HasTxnType + ?Sized>(
    rules: &Rules,
    tx: &Tx,
    dispatch: impl FnOnce(TxType) -> (NotTec, TxConsequences),
) -> (NotTec, TxConsequences) {
    match run_with_txn_type_source(rules, tx, dispatch) {
        Ok(result) => result,
        Err(_) => (
            UNKNOWN_TRANSACTION_TYPE_TER,
            TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
        ),
    }
}

pub fn run_invoke_preflight_for_txn_source_with_consequences<Tx: HasTxnType + ?Sized>(
    rules: &Rules,
    tx: &Tx,
    dispatch_preflight: impl FnOnce(TxType) -> NotTec,
    dispatch_success_consequences: impl FnOnce(TxType) -> TxConsequences,
) -> (NotTec, TxConsequences) {
    run_invoke_preflight_for_txn_source(rules, tx, |txn_type| {
        let ter = dispatch_preflight(txn_type);
        let consequences = if is_tes_success(ter) {
            dispatch_success_consequences(txn_type)
        } else {
            TxConsequences::from_preflight_result(ter)
        };
        (ter, consequences)
    })
}

pub fn run_invoke_preclaim_with_context<TxnType>(
    rules: &Rules,
    invoke_preclaim: impl FnOnce() -> Result<Ter, UnknownTransactionType<TxnType>>,
) -> Ter {
    crate::runtime::with_transaction_step_runtime(rules, || {
        invoke_preclaim().unwrap_or(UNKNOWN_TRANSACTION_TYPE_TER)
    })
}

pub fn run_invoke_preclaim_for_txn_type(
    rules: &Rules,
    txn_type: TxType,
    dispatch: impl FnOnce(TxType) -> Ter,
) -> Ter {
    run_with_txn_type_key(rules, txn_type, dispatch).unwrap_or(UNKNOWN_TRANSACTION_TYPE_TER)
}

pub fn run_invoke_preclaim_for_txn_source<Tx: HasTxnType + ?Sized>(
    rules: &Rules,
    tx: &Tx,
    dispatch: impl FnOnce(TxType) -> Ter,
) -> Ter {
    run_with_txn_type_source(rules, tx, dispatch).unwrap_or(UNKNOWN_TRANSACTION_TYPE_TER)
}

pub fn run_invoke_apply_with_context<TxnType>(
    rules: &Rules,
    invoke_apply: impl FnOnce() -> Result<ApplyResult, UnknownTransactionType<TxnType>>,
) -> ApplyResult {
    crate::runtime::with_transaction_step_runtime(rules, || match invoke_apply() {
        Ok(result) => result,
        Err(_) => ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false),
    })
}

pub fn run_invoke_apply_result_with_context<TxnType, E>(
    rules: &Rules,
    invoke_apply: impl FnOnce() -> Result<Result<ApplyResult, E>, UnknownTransactionType<TxnType>>,
) -> Result<ApplyResult, E> {
    crate::runtime::with_transaction_step_runtime(rules, || match invoke_apply() {
        Ok(result) => result,
        Err(_) => Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)),
    })
}

pub fn run_invoke_apply_for_txn_type(
    rules: &Rules,
    txn_type: TxType,
    dispatch: impl FnOnce(TxType) -> ApplyResult,
) -> ApplyResult {
    run_with_txn_type_key(rules, txn_type, dispatch)
        .unwrap_or_else(|_| ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false))
}

pub fn run_invoke_apply_for_txn_source<Tx: HasTxnType + ?Sized>(
    rules: &Rules,
    tx: &Tx,
    dispatch: impl FnOnce(TxType) -> ApplyResult,
) -> ApplyResult {
    run_with_txn_type_source(rules, tx, dispatch)
        .unwrap_or_else(|_| ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false))
}

pub fn run_invoke_apply_result_for_txn_type<E>(
    rules: &Rules,
    txn_type: TxType,
    dispatch: impl FnOnce(TxType) -> Result<ApplyResult, E>,
) -> Result<ApplyResult, E> {
    run_with_txn_type_key(rules, txn_type, dispatch)
        .unwrap_or_else(|_| Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)))
}

pub fn run_invoke_apply_result_for_txn_source<Tx: HasTxnType + ?Sized, E>(
    rules: &Rules,
    tx: &Tx,
    dispatch: impl FnOnce(TxType) -> Result<ApplyResult, E>,
) -> Result<ApplyResult, E> {
    run_with_txn_type_source(rules, tx, dispatch)
        .unwrap_or_else(|_| Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)))
}

pub fn run_calculate_base_fee_with_context<View: ?Sized, Tx: ?Sized, Fee, TxnType>(
    rules: &Rules,
    view: &View,
    tx: &Tx,
    invoke_calculate_base_fee: impl FnOnce(&View, &Tx) -> Result<Fee, UnknownTransactionType<TxnType>>,
    zero_fee: impl FnOnce() -> Fee,
) -> Fee {
    crate::runtime::with_transaction_step_runtime(rules, || {
        match invoke_calculate_base_fee(view, tx) {
            Ok(fee) => fee,
            Err(_) => zero_fee(),
        }
    })
}

pub fn run_calculate_base_fee_for_txn_type<Fee>(
    rules: &Rules,
    txn_type: TxType,
    dispatch: impl FnOnce(TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
) -> Fee {
    run_with_txn_type_key(rules, txn_type, dispatch).unwrap_or_else(|_| zero_fee())
}

pub fn run_calculate_base_fee_for_txn_source<Tx: HasTxnType + ?Sized, Fee>(
    rules: &Rules,
    tx: &Tx,
    dispatch: impl FnOnce(TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
) -> Fee {
    run_with_txn_type_source(rules, tx, dispatch).unwrap_or_else(|_| zero_fee())
}

pub fn run_calculate_default_base_fee_with_context<View: ?Sized, Tx: ?Sized, Fee>(
    view: &View,
    tx: &Tx,
    calculate_default_base_fee: impl FnOnce(&View, &Tx) -> Fee,
) -> Fee {
    calculate_default_base_fee(view, tx)
}

fn preclaim_result_from_context<Registry, View, Tx, Journal, ParentBatchId>(
    ctx: PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
    current_ledger_seq: u32,
    ter: Ter,
) -> PreclaimResult<Tx, Journal, ParentBatchId> {
    let PreclaimContext {
        tx,
        parent_batch_id,
        flags,
        journal,
        ..
    } = ctx;

    PreclaimResult::new(current_ledger_seq, tx, parent_batch_id, flags, journal, ter)
}

#[cfg(test)]
mod tests {
    use basics::base_uint::Uint256;
    use protocol::{
        Rules, SeqProxy, Ter, TxType, feature_single_asset_vault, get_current_transaction_rules,
        set_current_transaction_rules,
    };

    use super::{
        UNKNOWN_TRANSACTION_TYPE_TER, UnknownTransactionType,
        run_calculate_base_fee_for_txn_source, run_calculate_base_fee_for_txn_type,
        run_calculate_base_fee_with_context, run_calculate_default_base_fee_with_context,
        run_do_apply_with_context, run_invoke_apply_for_txn_source, run_invoke_apply_for_txn_type,
        run_invoke_apply_result_for_txn_source, run_invoke_apply_result_for_txn_type,
        run_invoke_apply_result_with_context, run_invoke_apply_with_context,
        run_invoke_preclaim_for_txn_source, run_invoke_preclaim_for_txn_type,
        run_invoke_preclaim_with_context, run_invoke_preflight_for_txn_source,
        run_invoke_preflight_for_txn_type, run_invoke_preflight_with_context,
        run_preclaim_with_context, run_preflight_with_context, run_with_txn_type_key,
        run_with_txn_type_source,
    };
    use crate::{
        ApplyFlags, ApplyResult, HasTxnType, PreclaimResult, PreflightContext, PreflightResult,
        TxConsequences,
    };

    fn ledger_rules(seed: u8) -> Rules {
        Rules::from_ledger(
            [feature_single_asset_vault()],
            Uint256::from_array([seed; 32]),
            std::iter::empty(),
        )
    }

    fn sample_preflight(
        rules: Rules,
        flags: ApplyFlags,
        ter: Ter,
    ) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
        PreflightResult::new(
            "tx",
            ((flags & ApplyFlags::BATCH) == ApplyFlags::BATCH).then_some("batch"),
            rules,
            TxConsequences::new(12, SeqProxy::sequence(5)),
            flags,
            "journal",
            ter,
        )
    }

    fn sample_preclaim(
        ledger_seq: u32,
        flags: ApplyFlags,
        ter: Ter,
    ) -> PreclaimResult<&'static str, &'static str, &'static str> {
        PreclaimResult::new(
            ledger_seq,
            "tx",
            ((flags & ApplyFlags::BATCH) == ApplyFlags::BATCH).then_some("batch"),
            flags,
            "journal",
            ter,
        )
    }

    struct StubTxnSource {
        txn_type: TxType,
    }

    impl HasTxnType for StubTxnSource {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    #[test]
    fn preflight_shell_maps_exceptions_to_tefexception() {
        let result = run_preflight_with_context(
            PreflightContext::new_batch(
                "registry",
                "tx",
                "batch",
                Rules::new(std::iter::empty()),
                ApplyFlags::BATCH,
                "journal",
            ),
            |_ctx| Err::<(Ter, TxConsequences), &str>("boom"),
            |_ctx| TxConsequences::new(10, SeqProxy::sequence(5)),
        );

        assert_eq!(result.ter, Ter::TEF_EXCEPTION);
        assert_eq!(result.consequences.fee(), 10);
        assert_eq!(result.parent_batch_id, Some("batch"));
    }

    #[test]
    fn preclaim_shell_reflights_when_rules_change() {
        let old_rules = ledger_rules(0x11);
        let new_rules = ledger_rules(0x22);
        let preflight = sample_preflight(old_rules, ApplyFlags::RETRY, Ter::TES_SUCCESS);

        let result = run_preclaim_with_context(
            preflight,
            "registry",
            "view",
            &new_rules,
            7,
            |_preflight, rules| {
                sample_preflight(rules.clone(), ApplyFlags::FAIL_HARD, Ter::TES_SUCCESS)
            },
            |ctx| {
                assert_eq!(ctx.flags, ApplyFlags::FAIL_HARD);
                assert_eq!(ctx.parent_batch_id, None);
                Ok::<_, &str>(Ter::TEC_CLAIM)
            },
        );

        assert_eq!(result.ledger_seq, 7);
        assert_eq!(result.flags, ApplyFlags::FAIL_HARD);
        assert_eq!(result.ter, Ter::TEC_CLAIM);
        assert!(result.likely_to_claim_fee);
    }

    #[test]
    fn preclaim_shell_returns_preflight_failure_without_invoking_preclaim() {
        let result = run_preclaim_with_context(
            sample_preflight(
                Rules::new(std::iter::empty()),
                ApplyFlags::NONE,
                Ter::TER_RETRY,
            ),
            "registry",
            "view",
            &Rules::new(std::iter::empty()),
            9,
            |_preflight, _rules| unreachable!("matching rules should skip reflight"),
            |_ctx| -> Result<Ter, &str> {
                unreachable!("failed preflight should skip invoke_preclaim")
            },
        );

        assert_eq!(result.ledger_seq, 9);
        assert_eq!(result.ter, Ter::TER_RETRY);
        assert!(!result.likely_to_claim_fee);
    }

    #[test]
    fn do_apply_shell_returns_tefexception_when_ledger_seq_changed() {
        let result = run_do_apply_with_context(
            sample_preclaim(9, ApplyFlags::NONE, Ter::TES_SUCCESS),
            "registry",
            10,
            "base",
            "view",
            |_base, _tx| unreachable!("stale ledger should not calculate fees"),
            |_ctx| -> Result<ApplyResult, &str> {
                unreachable!("stale ledger should not invoke apply")
            },
        );

        assert_eq!(result, ApplyResult::new(Ter::TEF_EXCEPTION, false, false));
    }

    #[test]
    fn do_apply_shell_returns_non_fee_claiming_preclaim_without_invoking_apply() {
        let result = run_do_apply_with_context(
            sample_preclaim(9, ApplyFlags::NONE, Ter::TER_RETRY),
            "registry",
            9,
            "base",
            "view",
            |_base, _tx| unreachable!("non-fee-claiming path should skip fee calculation"),
            |_ctx| -> Result<ApplyResult, &str> {
                unreachable!("non-fee-claiming path should skip invoke_apply")
            },
        );

        assert_eq!(result, ApplyResult::new(Ter::TER_RETRY, false, false));
    }

    #[test]
    fn do_apply_shell_builds_apply_context_then_invokes_apply() {
        let result = run_do_apply_with_context(
            sample_preclaim(9, ApplyFlags::BATCH, Ter::TES_SUCCESS),
            "registry",
            9,
            String::from("base"),
            vec![1_i32],
            |base, tx| {
                assert_eq!(base, "base");
                assert_eq!(tx, &"tx");
                12_u64
            },
            |ctx| {
                assert_eq!(ctx.registry, "registry");
                assert_eq!(ctx.tx, "tx");
                assert_eq!(ctx.preclaim_result, Ter::TES_SUCCESS);
                assert_eq!(ctx.base_fee, 12_u64);
                assert_eq!(ctx.flags(), ApplyFlags::BATCH);
                assert_eq!(ctx.parent_batch_id, Some("batch"));
                assert_eq!(ctx.base(), "base");
                assert_eq!(ctx.view(), &vec![1]);
                Ok::<_, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
            },
        );

        assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, true));
    }

    #[test]
    fn do_apply_shell_maps_apply_errors_to_tefexception() {
        let result = run_do_apply_with_context(
            sample_preclaim(9, ApplyFlags::NONE, Ter::TES_SUCCESS),
            "registry",
            9,
            "base",
            "view",
            |_base, _tx| 10_u64,
            |_ctx| Err::<ApplyResult, &str>("boom"),
        );

        assert_eq!(result, ApplyResult::new(Ter::TEF_EXCEPTION, false, false));
    }

    #[test]
    fn invoke_preflight_shell_enters_step_runtime_and_returns_dispatched_result() {
        set_current_transaction_rules(None);
        let rules = ledger_rules(0x33);

        let (ter, consequences) = run_invoke_preflight_with_context(&rules, || {
            assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
            Ok::<_, UnknownTransactionType<&str>>((
                Ter::TES_SUCCESS,
                TxConsequences::new(12, SeqProxy::sequence(4)),
            ))
        });

        assert_eq!(ter, Ter::TES_SUCCESS);
        assert_eq!(consequences, TxConsequences::new(12, SeqProxy::sequence(4)));
        assert_eq!(get_current_transaction_rules(), None);
    }

    #[test]
    fn invoke_preflight_shell_maps_unknown_type_to_temunknown() {
        let result = run_invoke_preflight_with_context(&Rules::new(std::iter::empty()), || {
            Err::<(Ter, TxConsequences), _>(UnknownTransactionType::new("unknown"))
        });

        assert_eq!(
            result,
            (
                UNKNOWN_TRANSACTION_TYPE_TER,
                TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
            )
        );
    }

    #[test]
    fn invoke_preclaim_shell_maps_unknown_type_to_temunknown() {
        let result = run_invoke_preclaim_with_context(&Rules::new(std::iter::empty()), || {
            Err::<Ter, _>(UnknownTransactionType::new("unknown"))
        });

        assert_eq!(result, UNKNOWN_TRANSACTION_TYPE_TER);
    }

    #[test]
    fn invoke_apply_shell_maps_unknown_type_to_temunknown() {
        let result = run_invoke_apply_with_context(&Rules::new(std::iter::empty()), || {
            Err::<ApplyResult, _>(UnknownTransactionType::new("unknown"))
        });

        assert_eq!(
            result,
            ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)
        );
    }

    #[test]
    fn invoke_apply_result_shell_maps_unknown_type_to_temunknown() {
        let result = run_invoke_apply_result_with_context(&Rules::new(std::iter::empty()), || {
            Err::<Result<ApplyResult, &str>, _>(UnknownTransactionType::new("unknown"))
        });

        assert_eq!(
            result,
            Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false))
        );
    }

    #[test]
    fn txn_type_key_dispatches_known_payment_like_transactions_macro() {
        set_current_transaction_rules(None);
        let rules = ledger_rules(0x44);

        let observed = run_with_txn_type_key(&rules, TxType::PAYMENT, |txn_type| {
            assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
            txn_type
        });

        assert_eq!(observed, Ok(TxType::PAYMENT));
        assert_eq!(get_current_transaction_rules(), None);
    }

    #[test]
    fn txn_type_key_rejects_protocol_only_hook_set_dispatch_gap() {
        let observed = run_with_txn_type_key(
            &Rules::new(std::iter::empty()),
            TxType::HOOK_SET,
            |_txn_type| unreachable!("hook set is not in transactions.macro"),
        );

        assert_eq!(observed, Err(UnknownTransactionType::new(TxType::HOOK_SET)));
    }

    #[test]
    fn txn_type_source_dispatches_known_payment_like_transactions_macro() {
        set_current_transaction_rules(None);
        let rules = ledger_rules(0x45);
        let tx = StubTxnSource {
            txn_type: TxType::PAYMENT,
        };

        let observed = run_with_txn_type_source(&rules, &tx, |txn_type| {
            assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
            txn_type
        });

        assert_eq!(observed, Ok(TxType::PAYMENT));
        assert_eq!(get_current_transaction_rules(), None);
    }

    #[test]
    fn invoke_preflight_for_txn_type_maps_hook_set_to_temunknown() {
        let observed = run_invoke_preflight_for_txn_type(
            &Rules::new(std::iter::empty()),
            TxType::HOOK_SET,
            |_txn_type| unreachable!("hook set should not dispatch"),
        );

        assert_eq!(
            observed,
            (
                UNKNOWN_TRANSACTION_TYPE_TER,
                TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
            )
        );
    }

    #[test]
    fn invoke_preflight_for_txn_source_maps_hook_set_to_temunknown() {
        let tx = StubTxnSource {
            txn_type: TxType::HOOK_SET,
        };

        let observed = run_invoke_preflight_for_txn_source(
            &Rules::new(std::iter::empty()),
            &tx,
            |_txn_type| unreachable!("hook set should not dispatch"),
        );

        assert_eq!(
            observed,
            (
                UNKNOWN_TRANSACTION_TYPE_TER,
                TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
            )
        );
    }

    #[test]
    fn invoke_preclaim_for_txn_type_maps_hook_set_to_temunknown() {
        let observed = run_invoke_preclaim_for_txn_type(
            &Rules::new(std::iter::empty()),
            TxType::HOOK_SET,
            |_txn_type| unreachable!("hook set should not dispatch"),
        );

        assert_eq!(observed, UNKNOWN_TRANSACTION_TYPE_TER);
    }

    #[test]
    fn invoke_preclaim_for_txn_source_maps_hook_set_to_temunknown() {
        let tx = StubTxnSource {
            txn_type: TxType::HOOK_SET,
        };

        let observed =
            run_invoke_preclaim_for_txn_source(&Rules::new(std::iter::empty()), &tx, |_txn_type| {
                unreachable!("hook set should not dispatch")
            });

        assert_eq!(observed, UNKNOWN_TRANSACTION_TYPE_TER);
    }

    #[test]
    fn invoke_apply_for_txn_type_maps_hook_set_to_temunknown() {
        let observed = run_invoke_apply_for_txn_type(
            &Rules::new(std::iter::empty()),
            TxType::HOOK_SET,
            |_txn_type| unreachable!("hook set should not dispatch"),
        );

        assert_eq!(
            observed,
            ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)
        );
    }

    #[test]
    fn invoke_apply_for_txn_source_maps_hook_set_to_temunknown() {
        let tx = StubTxnSource {
            txn_type: TxType::HOOK_SET,
        };

        let observed =
            run_invoke_apply_for_txn_source(&Rules::new(std::iter::empty()), &tx, |_txn_type| {
                unreachable!("hook set should not dispatch")
            });

        assert_eq!(
            observed,
            ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)
        );
    }

    #[test]
    fn invoke_apply_result_for_txn_type_maps_hook_set_to_temunknown() {
        let observed: Result<ApplyResult, &str> = run_invoke_apply_result_for_txn_type(
            &Rules::new(std::iter::empty()),
            TxType::HOOK_SET,
            |_txn_type| unreachable!("hook set should not dispatch"),
        );

        assert_eq!(
            observed,
            Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false))
        );
    }

    #[test]
    fn invoke_apply_result_for_txn_source_maps_hook_set_to_temunknown() {
        let tx = StubTxnSource {
            txn_type: TxType::HOOK_SET,
        };

        let observed: Result<ApplyResult, &str> = run_invoke_apply_result_for_txn_source(
            &Rules::new(std::iter::empty()),
            &tx,
            |_txn_type| unreachable!("hook set should not dispatch"),
        );

        assert_eq!(
            observed,
            Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false))
        );
    }

    #[test]
    fn calculate_base_fee_for_txn_type_maps_hook_set_to_zero() {
        let observed = run_calculate_base_fee_for_txn_type(
            &Rules::new(std::iter::empty()),
            TxType::HOOK_SET,
            |_txn_type| unreachable!("hook set should not dispatch"),
            || 0_u64,
        );

        assert_eq!(observed, 0_u64);
    }

    #[test]
    fn calculate_base_fee_for_txn_source_maps_hook_set_to_zero() {
        let tx = StubTxnSource {
            txn_type: TxType::HOOK_SET,
        };

        let observed = run_calculate_base_fee_for_txn_source(
            &Rules::new(std::iter::empty()),
            &tx,
            |_txn_type| unreachable!("hook set should not dispatch"),
            || 0_u64,
        );

        assert_eq!(observed, 0_u64);
    }

    #[test]
    fn calculate_base_fee_shell_preserves_dispatched_fee() {
        let rules = Rules::new(std::iter::empty());
        let fee = run_calculate_base_fee_with_context(
            &rules,
            "view",
            "tx",
            |view, tx| {
                assert_eq!(view, "view");
                assert_eq!(tx, "tx");
                Ok::<_, UnknownTransactionType<&str>>(12_u64)
            },
            || 0_u64,
        );

        assert_eq!(fee, 12_u64);
    }

    #[test]
    fn calculate_base_fee_shell_maps_unknown_txn_type_to_zero_fee() {
        let fee = run_calculate_base_fee_with_context(
            &Rules::new(std::iter::empty()),
            "view",
            "tx",
            |_view, _tx| Err::<u64, _>(UnknownTransactionType::new("unknown")),
            || 0_u64,
        );

        assert_eq!(fee, 0_u64);
    }

    #[test]
    fn calculate_default_base_fee_shell_delegates_directly() {
        let fee = run_calculate_default_base_fee_with_context("view", "tx", |view, tx| {
            assert_eq!(view, "view");
            assert_eq!(tx, "tx");
            9_u64
        });

        assert_eq!(fee, 9_u64);
    }
}
