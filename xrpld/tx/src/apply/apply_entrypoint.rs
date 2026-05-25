//! Top `xrpl/tx/the reference source` control-flow shell.
//!
//! This ports the deterministic closure ordering and the public
//! `applyTransaction(...)` result classification.

use protocol::{
    BatchTransactionFlags, NotTec, Rules, Ter, TxType, is_tec_claim, is_tef_failure,
    is_tem_malformed, is_tes_success,
};

use crate::{
    ApplyContext, ApplyFlags, ApplyResult, HasTxnType, PreclaimContext, PreflightContext,
    TxConsequences, UNKNOWN_TRANSACTION_TYPE_TER, run_do_apply_for_txn_type,
    run_preclaim_for_txn_type, run_preflight_for_txn_type, txn_type_of,
    with_transaction_apply_runtime,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyTransactionResult {
    Success,
    Fail,
    Retry,
}

pub fn apply_transaction_flags(flags: ApplyFlags, retry_assured: bool) -> ApplyFlags {
    if retry_assured {
        flags | ApplyFlags::RETRY
    } else {
        flags
    }
}

pub fn should_run_batch_followup(result: &ApplyResult, txn_type: TxType) -> bool {
    result.applied && is_tes_success(result.ter) && txn_type == TxType::BATCH
}

pub fn run_apply_batch_transactions<InnerTx, E>(
    batch_flags: BatchTransactionFlags,
    inner_transactions: impl IntoIterator<Item = InnerTx>,
    mut apply_one_transaction: impl FnMut(InnerTx) -> Result<ApplyResult, E>,
) -> Result<bool, E> {
    run_apply_batch_transactions_with_view_merge(
        batch_flags,
        inner_transactions,
        |inner_transaction| apply_one_transaction(inner_transaction).map(|result| (result, ())),
        |_| Ok(()),
    )
}

pub fn run_apply_batch_transactions_with_view_merge<InnerTx, PerTxBatchView, E>(
    batch_flags: BatchTransactionFlags,
    inner_transactions: impl IntoIterator<Item = InnerTx>,
    mut apply_one_transaction: impl FnMut(InnerTx) -> Result<(ApplyResult, PerTxBatchView), E>,
    mut apply_per_tx_batch_view: impl FnMut(PerTxBatchView) -> Result<(), E>,
) -> Result<bool, E> {
    let mut applied = 0_usize;

    for inner_transaction in inner_transactions {
        let (result, per_tx_batch_view) = apply_one_transaction(inner_transaction)?;
        debug_assert_eq!(
            result.applied,
            is_tes_success(result.ter) || is_tec_claim(result.ter),
            "inner batch transaction applied state should match the reference implementation the reference source rules",
        );

        if result.applied {
            apply_per_tx_batch_view(per_tx_batch_view)?;
            applied += 1;
        }

        if !is_tes_success(result.ter) {
            if batch_flags.contains(BatchTransactionFlags::ALL_OR_NOTHING) {
                return Ok(false);
            }

            if batch_flags.contains(BatchTransactionFlags::UNTIL_FAILURE) {
                break;
            }
        } else if batch_flags.contains(BatchTransactionFlags::ONLY_ONE) {
            break;
        }
    }

    Ok(applied != 0)
}

pub fn run_apply<PreflightResult, PreclaimResult>(
    rules: &Rules,
    run_preflight: impl FnOnce() -> PreflightResult,
    run_preclaim: impl FnOnce(PreflightResult) -> PreclaimResult,
    run_do_apply: impl FnOnce(PreclaimResult) -> ApplyResult,
) -> ApplyResult {
    with_transaction_apply_runtime(rules, || {
        let preflight = run_preflight();
        let preclaim = run_preclaim(preflight);
        run_do_apply(preclaim)
    })
}

fn build_preflight_context<Registry, Tx, Journal, ParentBatchId>(
    registry: Registry,
    tx: Tx,
    parent_batch_id: Option<ParentBatchId>,
    rules: &Rules,
    flags: ApplyFlags,
    journal: Journal,
) -> PreflightContext<Registry, Tx, Journal, ParentBatchId> {
    match parent_batch_id {
        Some(parent_batch_id) => PreflightContext::new_batch(
            registry,
            tx,
            parent_batch_id,
            rules.clone(),
            flags,
            journal,
        ),
        None => PreflightContext::new(registry, tx, rules.clone(), flags, journal),
    }
}

pub fn run_apply_for_txn_type<
    Registry,
    BaseView,
    View,
    Tx,
    Fee,
    Journal,
    ParentBatchId,
    PreflightE,
    PreclaimE,
    ApplyE,
    PreflightDispatch,
    FallbackConsequences,
>(
    registry: Registry,
    tx: Tx,
    parent_batch_id: Option<ParentBatchId>,
    current_rules: &Rules,
    flags: ApplyFlags,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    journal: Journal,
    txn_type: TxType,
    dispatch_preflight: PreflightDispatch,
    fallback_consequences: FallbackConsequences,
    dispatch_preclaim: impl FnOnce(
        &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<Ter, PreclaimE>,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx, TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
    dispatch_apply: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
        TxType,
    ) -> Result<ApplyResult, ApplyE>,
) -> ApplyResult
where
    Registry: Clone,
    View: Clone,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PreflightDispatch: Fn(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<(NotTec, TxConsequences), PreflightE>,
    FallbackConsequences:
        Fn(&PreflightContext<Registry, Tx, Journal, ParentBatchId>) -> TxConsequences,
{
    let preflight_registry = registry.clone();
    let preclaim_registry = registry.clone();
    let rerun_registry = registry.clone();
    let apply_registry = registry;
    let preclaim_view = view.clone();

    run_apply(
        current_rules,
        || {
            let ctx = build_preflight_context(
                preflight_registry,
                tx,
                parent_batch_id,
                current_rules,
                flags,
                journal,
            );
            run_preflight_for_txn_type(
                ctx,
                txn_type,
                |ctx, txn_type| dispatch_preflight(ctx, txn_type),
                |ctx| fallback_consequences(ctx),
            )
        },
        |preflight_result| {
            run_preclaim_for_txn_type(
                preflight_result,
                txn_type,
                preclaim_registry,
                preclaim_view,
                current_rules,
                current_ledger_seq,
                |preflight_result, rules| {
                    let ctx = build_preflight_context(
                        rerun_registry.clone(),
                        preflight_result.tx.clone(),
                        preflight_result.parent_batch_id.clone(),
                        rules,
                        preflight_result.flags,
                        preflight_result.journal.clone(),
                    );
                    run_preflight_for_txn_type(
                        ctx,
                        txn_type,
                        |ctx, txn_type| dispatch_preflight(ctx, txn_type),
                        |ctx| fallback_consequences(ctx),
                    )
                },
                dispatch_preclaim,
            )
        },
        |preclaim_result| {
            run_do_apply_for_txn_type(
                preclaim_result,
                current_rules,
                txn_type,
                apply_registry,
                current_ledger_seq,
                base,
                view,
                calculate_base_fee,
                zero_fee,
                dispatch_apply,
            )
        },
    )
}

pub fn run_apply_for_txn_source<
    Registry,
    BaseView,
    View,
    Tx,
    Fee,
    Journal,
    ParentBatchId,
    PreflightE,
    PreclaimE,
    ApplyE,
    PreflightDispatch,
    FallbackConsequences,
>(
    registry: Registry,
    tx: Tx,
    parent_batch_id: Option<ParentBatchId>,
    current_rules: &Rules,
    flags: ApplyFlags,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    journal: Journal,
    dispatch_preflight: PreflightDispatch,
    fallback_consequences: FallbackConsequences,
    dispatch_preclaim: impl FnOnce(
        &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<Ter, PreclaimE>,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx, TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
    dispatch_apply: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
        TxType,
    ) -> Result<ApplyResult, ApplyE>,
) -> ApplyResult
where
    Registry: Clone,
    View: Clone,
    Tx: HasTxnType + Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PreflightDispatch: Fn(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<(NotTec, TxConsequences), PreflightE>,
    FallbackConsequences:
        Fn(&PreflightContext<Registry, Tx, Journal, ParentBatchId>) -> TxConsequences,
{
    let txn_type = txn_type_of(&tx);
    run_apply_for_txn_type(
        registry,
        tx,
        parent_batch_id,
        current_rules,
        flags,
        current_ledger_seq,
        base,
        view,
        journal,
        txn_type,
        dispatch_preflight,
        fallback_consequences,
        dispatch_preclaim,
        calculate_base_fee,
        zero_fee,
        dispatch_apply,
    )
}

pub fn run_apply_transaction_flow_for_txn_type<
    Registry,
    BaseView,
    View,
    Tx,
    Fee,
    Journal,
    ParentBatchId,
    PreflightE,
    PreclaimE,
    ApplyE,
    PreflightDispatch,
    FallbackConsequences,
>(
    registry: Registry,
    tx: Tx,
    parent_batch_id: Option<ParentBatchId>,
    current_rules: &Rules,
    retry_assured: bool,
    flags: ApplyFlags,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    journal: Journal,
    txn_type: TxType,
    dispatch_preflight: PreflightDispatch,
    fallback_consequences: FallbackConsequences,
    dispatch_preclaim: impl FnOnce(
        &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<Ter, PreclaimE>,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx, TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
    dispatch_apply: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
        TxType,
    ) -> Result<ApplyResult, ApplyE>,
) -> ApplyTransactionResult
where
    Registry: Clone,
    View: Clone,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PreflightDispatch: Fn(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<(NotTec, TxConsequences), PreflightE>,
    FallbackConsequences:
        Fn(&PreflightContext<Registry, Tx, Journal, ParentBatchId>) -> TxConsequences,
{
    let flags = apply_transaction_flags(flags, retry_assured);
    classify_apply_transaction_result(run_apply_for_txn_type(
        registry,
        tx,
        parent_batch_id,
        current_rules,
        flags,
        current_ledger_seq,
        base,
        view,
        journal,
        txn_type,
        dispatch_preflight,
        fallback_consequences,
        dispatch_preclaim,
        calculate_base_fee,
        zero_fee,
        dispatch_apply,
    ))
}

pub fn run_apply_transaction_flow_for_txn_source<
    Registry,
    BaseView,
    View,
    Tx,
    Fee,
    Journal,
    ParentBatchId,
    PreflightE,
    PreclaimE,
    ApplyE,
    PreflightDispatch,
    FallbackConsequences,
>(
    registry: Registry,
    tx: Tx,
    parent_batch_id: Option<ParentBatchId>,
    current_rules: &Rules,
    retry_assured: bool,
    flags: ApplyFlags,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    journal: Journal,
    dispatch_preflight: PreflightDispatch,
    fallback_consequences: FallbackConsequences,
    dispatch_preclaim: impl FnOnce(
        &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<Ter, PreclaimE>,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx, TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
    dispatch_apply: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
        TxType,
    ) -> Result<ApplyResult, ApplyE>,
) -> ApplyTransactionResult
where
    Registry: Clone,
    View: Clone,
    Tx: HasTxnType + Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PreflightDispatch: Fn(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<(NotTec, TxConsequences), PreflightE>,
    FallbackConsequences:
        Fn(&PreflightContext<Registry, Tx, Journal, ParentBatchId>) -> TxConsequences,
{
    let flags = apply_transaction_flags(flags, retry_assured);
    classify_apply_transaction_result(run_apply_for_txn_source(
        registry,
        tx,
        parent_batch_id,
        current_rules,
        flags,
        current_ledger_seq,
        base,
        view,
        journal,
        dispatch_preflight,
        fallback_consequences,
        dispatch_preclaim,
        calculate_base_fee,
        zero_fee,
        dispatch_apply,
    ))
}

pub fn classify_apply_transaction_result(result: ApplyResult) -> ApplyTransactionResult {
    if result.applied {
        ApplyTransactionResult::Success
    } else if is_tef_failure(result.ter) || is_tem_malformed(result.ter) || is_tel_local(result.ter)
    {
        ApplyTransactionResult::Fail
    } else {
        ApplyTransactionResult::Retry
    }
}

pub fn run_apply_transaction<E>(
    retry_assured: bool,
    flags: ApplyFlags,
    run_apply: impl FnOnce(ApplyFlags) -> Result<ApplyResult, E>,
) -> ApplyTransactionResult {
    let flags = apply_transaction_flags(flags, retry_assured);
    match run_apply(flags) {
        Ok(result) => classify_apply_transaction_result(result),
        Err(_) => ApplyTransactionResult::Fail,
    }
}

pub fn run_apply_transaction_with_batch_followup<ApplyE, BatchE>(
    txn_type: TxType,
    retry_assured: bool,
    flags: ApplyFlags,
    run_apply: impl FnOnce(ApplyFlags) -> Result<ApplyResult, ApplyE>,
    run_batch_followup: impl FnOnce() -> Result<bool, BatchE>,
    apply_whole_batch: impl FnOnce() -> Result<(), BatchE>,
) -> ApplyTransactionResult {
    let flags = apply_transaction_flags(flags, retry_assured);
    match run_apply(flags) {
        Ok(result) => {
            if result.applied {
                if should_run_batch_followup(&result, txn_type) {
                    match run_batch_followup() {
                        Ok(true) => {
                            if apply_whole_batch().is_err() {
                                return ApplyTransactionResult::Fail;
                            }
                        }
                        Ok(false) => {}
                        Err(_) => return ApplyTransactionResult::Fail,
                    }
                }

                ApplyTransactionResult::Success
            } else {
                classify_apply_transaction_result(result)
            }
        }
        Err(_) => ApplyTransactionResult::Fail,
    }
}

pub fn run_apply_transaction_for_txn_type<E>(
    txn_type: TxType,
    retry_assured: bool,
    flags: ApplyFlags,
    run_apply: impl FnOnce(ApplyFlags, TxType) -> Result<ApplyResult, E>,
) -> ApplyTransactionResult {
    run_apply_transaction(retry_assured, flags, |flags| {
        if txn_type.is_dispatchable() {
            run_apply(flags, txn_type)
        } else {
            Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false))
        }
    })
}

pub fn run_apply_transaction_for_txn_source<Tx: HasTxnType + ?Sized, E>(
    tx: &Tx,
    retry_assured: bool,
    flags: ApplyFlags,
    run_apply: impl FnOnce(ApplyFlags, TxType) -> Result<ApplyResult, E>,
) -> ApplyTransactionResult {
    run_apply_transaction_for_txn_type(txn_type_of(tx), retry_assured, flags, run_apply)
}

pub fn run_apply_transaction_flow_with_batch_followup_for_txn_type<
    Registry,
    BaseView,
    View,
    Tx,
    Fee,
    Journal,
    ParentBatchId,
    InnerTx,
    InnerTransactions,
    PreflightE,
    PreclaimE,
    ApplyE,
    BatchE,
    PreflightDispatch,
    FallbackConsequences,
>(
    registry: Registry,
    tx: Tx,
    parent_batch_id: Option<ParentBatchId>,
    current_rules: &Rules,
    retry_assured: bool,
    flags: ApplyFlags,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    journal: Journal,
    txn_type: TxType,
    dispatch_preflight: PreflightDispatch,
    fallback_consequences: FallbackConsequences,
    dispatch_preclaim: impl FnOnce(
        &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<Ter, PreclaimE>,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx, TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
    dispatch_apply: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
        TxType,
    ) -> Result<ApplyResult, ApplyE>,
    batch_flags: BatchTransactionFlags,
    inner_transactions: InnerTransactions,
    apply_batch_transaction: impl FnMut(InnerTx) -> Result<ApplyResult, BatchE>,
    apply_whole_batch: impl FnOnce() -> Result<(), BatchE>,
) -> ApplyTransactionResult
where
    Registry: Clone,
    View: Clone,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    InnerTransactions: IntoIterator<Item = InnerTx>,
    PreflightDispatch: Fn(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<(NotTec, TxConsequences), PreflightE>,
    FallbackConsequences:
        Fn(&PreflightContext<Registry, Tx, Journal, ParentBatchId>) -> TxConsequences,
{
    let mut apply_batch_transaction = apply_batch_transaction;
    run_apply_transaction_with_batch_followup(
        txn_type,
        retry_assured,
        flags,
        |flags| {
            Ok::<ApplyResult, ApplyE>(run_apply_for_txn_type(
                registry,
                tx,
                parent_batch_id,
                current_rules,
                flags,
                current_ledger_seq,
                base,
                view,
                journal,
                txn_type,
                dispatch_preflight,
                fallback_consequences,
                dispatch_preclaim,
                calculate_base_fee,
                zero_fee,
                dispatch_apply,
            ))
        },
        move || {
            run_apply_batch_transactions(batch_flags, inner_transactions, |inner_transaction| {
                apply_batch_transaction(inner_transaction)
            })
        },
        apply_whole_batch,
    )
}

pub fn run_apply_transaction_flow_with_batch_followup_for_txn_source<
    Registry,
    BaseView,
    View,
    Tx,
    Fee,
    Journal,
    ParentBatchId,
    InnerTx,
    InnerTransactions,
    PreflightE,
    PreclaimE,
    ApplyE,
    BatchE,
    PreflightDispatch,
    FallbackConsequences,
>(
    registry: Registry,
    tx: Tx,
    parent_batch_id: Option<ParentBatchId>,
    current_rules: &Rules,
    retry_assured: bool,
    flags: ApplyFlags,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    journal: Journal,
    dispatch_preflight: PreflightDispatch,
    fallback_consequences: FallbackConsequences,
    dispatch_preclaim: impl FnOnce(
        &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<Ter, PreclaimE>,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx, TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
    dispatch_apply: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
        TxType,
    ) -> Result<ApplyResult, ApplyE>,
    batch_flags: BatchTransactionFlags,
    inner_transactions: InnerTransactions,
    apply_batch_transaction: impl FnMut(InnerTx) -> Result<ApplyResult, BatchE>,
    apply_whole_batch: impl FnOnce() -> Result<(), BatchE>,
) -> ApplyTransactionResult
where
    Registry: Clone,
    View: Clone,
    Tx: HasTxnType + Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    InnerTransactions: IntoIterator<Item = InnerTx>,
    PreflightDispatch: Fn(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<(NotTec, TxConsequences), PreflightE>,
    FallbackConsequences:
        Fn(&PreflightContext<Registry, Tx, Journal, ParentBatchId>) -> TxConsequences,
{
    let txn_type = txn_type_of(&tx);
    run_apply_transaction_flow_with_batch_followup_for_txn_type(
        registry,
        tx,
        parent_batch_id,
        current_rules,
        retry_assured,
        flags,
        current_ledger_seq,
        base,
        view,
        journal,
        txn_type,
        dispatch_preflight,
        fallback_consequences,
        dispatch_preclaim,
        calculate_base_fee,
        zero_fee,
        dispatch_apply,
        batch_flags,
        inner_transactions,
        apply_batch_transaction,
        apply_whole_batch,
    )
}

pub fn run_apply_transaction_flow_with_batch_view_merge_for_txn_type<
    Registry,
    BaseView,
    View,
    Tx,
    Fee,
    Journal,
    ParentBatchId,
    InnerTx,
    InnerTransactions,
    PerTxBatchView,
    PreflightE,
    PreclaimE,
    ApplyE,
    BatchE,
    PreflightDispatch,
    FallbackConsequences,
>(
    registry: Registry,
    tx: Tx,
    parent_batch_id: Option<ParentBatchId>,
    current_rules: &Rules,
    retry_assured: bool,
    flags: ApplyFlags,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    journal: Journal,
    txn_type: TxType,
    dispatch_preflight: PreflightDispatch,
    fallback_consequences: FallbackConsequences,
    dispatch_preclaim: impl FnOnce(
        &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<Ter, PreclaimE>,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx, TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
    dispatch_apply: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
        TxType,
    ) -> Result<ApplyResult, ApplyE>,
    batch_flags: BatchTransactionFlags,
    inner_transactions: InnerTransactions,
    apply_batch_transaction: impl FnMut(InnerTx) -> Result<(ApplyResult, PerTxBatchView), BatchE>,
    apply_per_tx_batch_view: impl FnMut(PerTxBatchView) -> Result<(), BatchE>,
    apply_whole_batch: impl FnOnce() -> Result<(), BatchE>,
) -> ApplyTransactionResult
where
    Registry: Clone,
    View: Clone,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    InnerTransactions: IntoIterator<Item = InnerTx>,
    PreflightDispatch: Fn(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<(NotTec, TxConsequences), PreflightE>,
    FallbackConsequences:
        Fn(&PreflightContext<Registry, Tx, Journal, ParentBatchId>) -> TxConsequences,
{
    let mut apply_batch_transaction = apply_batch_transaction;
    let mut apply_per_tx_batch_view = apply_per_tx_batch_view;
    run_apply_transaction_with_batch_followup(
        txn_type,
        retry_assured,
        flags,
        |flags| {
            Ok::<ApplyResult, ApplyE>(run_apply_for_txn_type(
                registry,
                tx,
                parent_batch_id,
                current_rules,
                flags,
                current_ledger_seq,
                base,
                view,
                journal,
                txn_type,
                dispatch_preflight,
                fallback_consequences,
                dispatch_preclaim,
                calculate_base_fee,
                zero_fee,
                dispatch_apply,
            ))
        },
        move || {
            run_apply_batch_transactions_with_view_merge(
                batch_flags,
                inner_transactions,
                |inner_transaction| apply_batch_transaction(inner_transaction),
                |per_tx_batch_view| apply_per_tx_batch_view(per_tx_batch_view),
            )
        },
        apply_whole_batch,
    )
}

pub fn run_apply_transaction_flow_with_batch_view_merge_for_txn_source<
    Registry,
    BaseView,
    View,
    Tx,
    Fee,
    Journal,
    ParentBatchId,
    InnerTx,
    InnerTransactions,
    PerTxBatchView,
    PreflightE,
    PreclaimE,
    ApplyE,
    BatchE,
    PreflightDispatch,
    FallbackConsequences,
>(
    registry: Registry,
    tx: Tx,
    parent_batch_id: Option<ParentBatchId>,
    current_rules: &Rules,
    retry_assured: bool,
    flags: ApplyFlags,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    journal: Journal,
    dispatch_preflight: PreflightDispatch,
    fallback_consequences: FallbackConsequences,
    dispatch_preclaim: impl FnOnce(
        &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<Ter, PreclaimE>,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx, TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
    dispatch_apply: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
        TxType,
    ) -> Result<ApplyResult, ApplyE>,
    batch_flags: BatchTransactionFlags,
    inner_transactions: InnerTransactions,
    apply_batch_transaction: impl FnMut(InnerTx) -> Result<(ApplyResult, PerTxBatchView), BatchE>,
    apply_per_tx_batch_view: impl FnMut(PerTxBatchView) -> Result<(), BatchE>,
    apply_whole_batch: impl FnOnce() -> Result<(), BatchE>,
) -> ApplyTransactionResult
where
    Registry: Clone,
    View: Clone,
    Tx: HasTxnType + Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    InnerTransactions: IntoIterator<Item = InnerTx>,
    PreflightDispatch: Fn(
        &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<(NotTec, TxConsequences), PreflightE>,
    FallbackConsequences:
        Fn(&PreflightContext<Registry, Tx, Journal, ParentBatchId>) -> TxConsequences,
{
    let txn_type = txn_type_of(&tx);
    run_apply_transaction_flow_with_batch_view_merge_for_txn_type(
        registry,
        tx,
        parent_batch_id,
        current_rules,
        retry_assured,
        flags,
        current_ledger_seq,
        base,
        view,
        journal,
        txn_type,
        dispatch_preflight,
        fallback_consequences,
        dispatch_preclaim,
        calculate_base_fee,
        zero_fee,
        dispatch_apply,
        batch_flags,
        inner_transactions,
        apply_batch_transaction,
        apply_per_tx_batch_view,
        apply_whole_batch,
    )
}

const fn is_tel_local(code: Ter) -> bool {
    code.to_int() < Ter::TEM_MALFORMED.to_int()
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::{
        ApplyTransactionResult, apply_transaction_flags, classify_apply_transaction_result,
        run_apply, run_apply_batch_transactions, run_apply_batch_transactions_with_view_merge,
        run_apply_for_txn_source, run_apply_for_txn_type, run_apply_transaction,
        run_apply_transaction_flow_with_batch_followup_for_txn_source,
        run_apply_transaction_flow_with_batch_view_merge_for_txn_source,
        run_apply_transaction_for_txn_source, run_apply_transaction_for_txn_type,
        run_apply_transaction_with_batch_followup, should_run_batch_followup,
    };
    use crate::{
        ApplyFlags, ApplyResult, HasTxnType, TxConsequences, UNKNOWN_TRANSACTION_TYPE_TER,
    };
    use protocol::{BatchTransactionFlags, Rules, SeqProxy, Ter, TxType};

    #[derive(Clone)]
    struct StubTxnSource {
        txn_type: TxType,
    }

    impl HasTxnType for StubTxnSource {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    #[test]
    fn apply_flags_add_retry_when_retry_assured_apply_transaction() {
        assert_eq!(
            apply_transaction_flags(ApplyFlags::FAIL_HARD, true),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY
        );
        assert_eq!(
            apply_transaction_flags(ApplyFlags::FAIL_HARD, false),
            ApplyFlags::FAIL_HARD
        );
    }

    #[test]
    fn run_apply_preserves_preflight_then_preclaim_then_do_apply_order() {
        let events = RefCell::new(Vec::new());
        let result = run_apply(
            &Rules::new(std::iter::empty()),
            || {
                events.borrow_mut().push("preflight");
                "pf"
            },
            |preflight| {
                assert_eq!(preflight, "pf");
                events.borrow_mut().push("preclaim");
                "pc"
            },
            |preclaim| {
                assert_eq!(preclaim, "pc");
                events.borrow_mut().push("apply");
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            },
        );

        assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, true));
        assert_eq!(events.into_inner(), vec!["preflight", "preclaim", "apply"]);
    }

    #[test]
    fn apply_transaction_result_classification_matches_current_cpp_categories() {
        assert_eq!(
            classify_apply_transaction_result(ApplyResult::new(Ter::TES_SUCCESS, true, true)),
            ApplyTransactionResult::Success
        );
        assert_eq!(
            classify_apply_transaction_result(ApplyResult::new(Ter::TEF_EXCEPTION, false, false)),
            ApplyTransactionResult::Fail
        );
        assert_eq!(
            classify_apply_transaction_result(ApplyResult::new(
                Ter::TEL_CAN_NOT_QUEUE,
                false,
                false
            )),
            ApplyTransactionResult::Fail
        );
        assert_eq!(
            classify_apply_transaction_result(ApplyResult::new(Ter::TER_RETRY, false, false)),
            ApplyTransactionResult::Retry
        );
        assert_eq!(
            classify_apply_transaction_result(ApplyResult::new(Ter::TEC_CLAIM, false, false)),
            ApplyTransactionResult::Retry
        );
    }

    #[test]
    fn batch_followup_only_runs_for_successful_outer_batch() {
        assert!(should_run_batch_followup(
            &ApplyResult::new(Ter::TES_SUCCESS, true, true),
            TxType::BATCH
        ));
        assert!(!should_run_batch_followup(
            &ApplyResult::new(Ter::TEC_CLAIM, true, true),
            TxType::BATCH
        ));
        assert!(!should_run_batch_followup(
            &ApplyResult::new(Ter::TES_SUCCESS, true, true),
            TxType::PAYMENT
        ));
    }

    #[test]
    fn apply_batch_transactions_returns_false_for_all_or_nothing_failure() {
        let seen = RefCell::new(Vec::new());

        let applied = run_apply_batch_transactions(
            BatchTransactionFlags::ALL_OR_NOTHING,
            ["first", "second"],
            |inner| {
                seen.borrow_mut().push(inner);
                Ok::<_, ()>(if inner == "first" {
                    ApplyResult::new(Ter::TEC_CLAIM, true, true)
                } else {
                    ApplyResult::new(Ter::TES_SUCCESS, true, true)
                })
            },
        )
        .unwrap();

        assert!(!applied);
        assert_eq!(seen.into_inner(), vec!["first"]);
    }

    #[test]
    fn apply_batch_transactions_breaks_after_first_success_in_only_one_mode() {
        let seen = RefCell::new(Vec::new());

        let applied = run_apply_batch_transactions(
            BatchTransactionFlags::ONLY_ONE,
            ["first", "second"],
            |inner| {
                seen.borrow_mut().push(inner);
                Ok::<_, ()>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
            },
        )
        .unwrap();

        assert!(applied);
        assert_eq!(seen.into_inner(), vec!["first"]);
    }

    #[test]
    fn apply_batch_transactions_with_view_merge_applies_per_tx_batch_view_before_all_or_nothing_abort()
     {
        let seen = RefCell::new(Vec::new());
        let merged = RefCell::new(Vec::new());

        let applied = run_apply_batch_transactions_with_view_merge(
            BatchTransactionFlags::ALL_OR_NOTHING,
            ["first", "second"],
            |inner| {
                seen.borrow_mut().push(inner);
                Ok::<_, ()>(if inner == "first" {
                    (ApplyResult::new(Ter::TEC_CLAIM, true, true), inner)
                } else {
                    (ApplyResult::new(Ter::TES_SUCCESS, true, true), inner)
                })
            },
            |per_tx_batch_view| {
                merged.borrow_mut().push(per_tx_batch_view);
                Ok::<_, ()>(())
            },
        )
        .unwrap();

        assert!(!applied);
        assert_eq!(seen.into_inner(), vec!["first"]);
        assert_eq!(merged.into_inner(), vec!["first"]);
    }

    #[test]
    fn apply_batch_transactions_with_view_merge_propagates_merge_failure_and_stops() {
        let seen = RefCell::new(Vec::new());
        let merged = RefCell::new(Vec::new());
        let merge_calls = std::cell::Cell::new(0_u32);

        let error = run_apply_batch_transactions_with_view_merge(
            BatchTransactionFlags::UNTIL_FAILURE,
            ["first", "second"],
            |inner| {
                seen.borrow_mut().push(inner);
                Ok::<_, &str>((ApplyResult::new(Ter::TES_SUCCESS, true, true), inner))
            },
            |per_tx_batch_view| {
                merged.borrow_mut().push(per_tx_batch_view);
                let next_call = merge_calls.get() + 1;
                merge_calls.set(next_call);
                if next_call == 1 {
                    Err::<(), _>("boom")
                } else {
                    Ok::<_, &str>(())
                }
            },
        )
        .expect_err("merge failure should propagate immediately");

        assert_eq!(error, "boom");
        assert_eq!(seen.into_inner(), vec!["first"]);
        assert_eq!(merged.into_inner(), vec!["first"]);
    }

    #[test]
    fn apply_transaction_maps_apply_errors_to_fail_exception_catch() {
        let result = run_apply_transaction(true, ApplyFlags::FAIL_HARD, |_flags| {
            Err::<ApplyResult, &str>("boom")
        });

        assert_eq!(result, ApplyTransactionResult::Fail);
    }

    #[test]
    fn apply_transaction_for_txn_type_dispatches_payment_public_shell() {
        let seen = RefCell::new(Vec::new());

        let result = run_apply_transaction_for_txn_type(
            TxType::PAYMENT,
            true,
            ApplyFlags::FAIL_HARD,
            |flags, txn_type| {
                seen.borrow_mut().push((flags, txn_type));
                Ok::<_, &str>(ApplyResult::new(Ter::TER_RETRY, false, false))
            },
        );

        assert_eq!(result, ApplyTransactionResult::Retry);
        assert_eq!(
            seen.into_inner(),
            vec![(
                apply_transaction_flags(ApplyFlags::FAIL_HARD, true),
                TxType::PAYMENT
            )]
        );
    }

    #[test]
    fn apply_transaction_for_txn_type_maps_protocol_only_hook_set_to_fail() {
        let result = run_apply_transaction_for_txn_type(
            TxType::HOOK_SET,
            false,
            ApplyFlags::NONE,
            |_flags, _txn_type| -> Result<ApplyResult, &str> {
                unreachable!("hook set should not dispatch")
            },
        );

        assert_eq!(result, ApplyTransactionResult::Fail);
    }

    #[test]
    fn apply_transaction_for_txn_source_dispatches_known_payment() {
        let tx = StubTxnSource {
            txn_type: TxType::PAYMENT,
        };
        let seen = RefCell::new(Vec::new());

        let result = run_apply_transaction_for_txn_source(
            &tx,
            true,
            ApplyFlags::FAIL_HARD,
            |flags, txn_type| {
                seen.borrow_mut().push((flags, txn_type));
                Ok::<_, &str>(ApplyResult::new(Ter::TER_RETRY, false, false))
            },
        );

        assert_eq!(result, ApplyTransactionResult::Retry);
        assert_eq!(
            seen.into_inner(),
            vec![(
                apply_transaction_flags(ApplyFlags::FAIL_HARD, true),
                TxType::PAYMENT
            )]
        );
    }

    #[test]
    fn apply_transaction_for_txn_source_maps_protocol_only_hook_set_to_fail() {
        let tx = StubTxnSource {
            txn_type: TxType::HOOK_SET,
        };

        let result = run_apply_transaction_for_txn_source(
            &tx,
            false,
            ApplyFlags::NONE,
            |_flags, _txn_type| -> Result<ApplyResult, &str> {
                unreachable!("hook set should not dispatch")
            },
        );

        assert_eq!(result, ApplyTransactionResult::Fail);
    }

    #[test]
    fn apply_transaction_with_batch_followup_applies_whole_batch_only_when_inner_batch_returns_true()
     {
        let seen_flags = RefCell::new(Vec::new());
        let followup_calls = Cell::new(0_u32);
        let whole_batch_applies = Cell::new(0_u32);

        let result = run_apply_transaction_with_batch_followup(
            TxType::BATCH,
            true,
            ApplyFlags::FAIL_HARD,
            |flags| {
                seen_flags.borrow_mut().push(flags);
                Ok::<_, ()>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
            },
            || {
                followup_calls.set(followup_calls.get() + 1);
                Ok::<_, ()>(true)
            },
            || {
                whole_batch_applies.set(whole_batch_applies.get() + 1);
                Ok::<_, ()>(())
            },
        );

        assert_eq!(result, ApplyTransactionResult::Success);
        assert_eq!(
            seen_flags.into_inner(),
            vec![ApplyFlags::FAIL_HARD | ApplyFlags::RETRY]
        );
        assert_eq!(followup_calls.get(), 1);
        assert_eq!(whole_batch_applies.get(), 1);
    }

    #[test]
    fn apply_transaction_with_batch_followup_maps_followup_error_to_fail() {
        let result = run_apply_transaction_with_batch_followup(
            TxType::BATCH,
            false,
            ApplyFlags::NONE,
            |_flags| Ok::<_, ()>(ApplyResult::new(Ter::TES_SUCCESS, true, true)),
            || Err::<bool, &str>("boom"),
            || Ok::<_, &str>(()),
        );

        assert_eq!(result, ApplyTransactionResult::Fail);
    }

    #[test]
    fn apply_for_txn_type_dispatches_known_payment_shell() {
        let tx = StubTxnSource {
            txn_type: TxType::PAYMENT,
        };
        let result = run_apply_for_txn_type(
            "registry",
            tx,
            Some("batch"),
            &Rules::new(std::iter::empty()),
            ApplyFlags::BATCH,
            9,
            "base",
            vec![1_i32],
            "journal",
            TxType::PAYMENT,
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::PAYMENT);
                Ok::<_, &str>((
                    Ter::TES_SUCCESS,
                    TxConsequences::new(10, SeqProxy::sequence(4)),
                ))
            },
            |_ctx| unreachable!("successful dispatch should not use fallback"),
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::PAYMENT);
                Ok::<_, &str>(Ter::TES_SUCCESS)
            },
            |_base, tx, txn_type| {
                assert_eq!(tx.txn_type, TxType::PAYMENT);
                assert_eq!(txn_type, TxType::PAYMENT);
                12_u64
            },
            || 0_u64,
            |ctx, txn_type| {
                assert_eq!(ctx.parent_batch_id, Some("batch"));
                assert_eq!(ctx.base_fee, 12_u64);
                assert_eq!(txn_type, TxType::PAYMENT);
                Ok::<_, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
            },
        );

        assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, true));
    }

    #[test]
    fn apply_for_txn_source_maps_hook_set_to_temunknown_shell() {
        let result = run_apply_for_txn_source(
            "registry",
            StubTxnSource {
                txn_type: TxType::HOOK_SET,
            },
            None::<&str>,
            &Rules::new(std::iter::empty()),
            ApplyFlags::NONE,
            9,
            "base",
            "view",
            "journal",
            |_ctx, _txn_type| -> Result<(Ter, TxConsequences), &str> {
                unreachable!("hook set should not dispatch")
            },
            |_ctx| unreachable!("unknown type should not use exception fallback"),
            |_ctx, _txn_type| -> Result<Ter, &str> {
                unreachable!("failed preflight should skip preclaim dispatch")
            },
            |_base, _tx, _txn_type| unreachable!("failed preflight should skip fee calculation"),
            || 0_u64,
            |_ctx, _txn_type| -> Result<ApplyResult, &str> {
                unreachable!("failed preflight should skip apply dispatch")
            },
        );

        assert_eq!(
            result,
            ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)
        );
    }

    #[test]
    fn apply_transaction_flow_with_batch_followup_for_txn_source_runs_inner_batch_then_applies_whole_batch()
     {
        let inner_applied = RefCell::new(Vec::new());
        let whole_batch_applies = Cell::new(0_u32);

        let result = run_apply_transaction_flow_with_batch_followup_for_txn_source(
            "registry",
            StubTxnSource {
                txn_type: TxType::BATCH,
            },
            None::<&str>,
            &Rules::new(std::iter::empty()),
            false,
            ApplyFlags::NONE,
            9,
            "base",
            "view",
            "journal",
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                Ok::<_, &str>((
                    Ter::TES_SUCCESS,
                    TxConsequences::new(7, SeqProxy::sequence(1)),
                ))
            },
            |_ctx| unreachable!("successful dispatch should not use fallback"),
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                Ok::<_, &str>(Ter::TES_SUCCESS)
            },
            |_base, _tx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                11_u64
            },
            || 0_u64,
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                Ok::<_, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
            },
            BatchTransactionFlags::UNTIL_FAILURE,
            ["first", "second"],
            |inner| {
                inner_applied.borrow_mut().push(inner);
                Ok::<_, &str>(if inner == "first" {
                    ApplyResult::new(Ter::TES_SUCCESS, true, true)
                } else {
                    ApplyResult::new(Ter::TEC_CLAIM, true, true)
                })
            },
            || {
                whole_batch_applies.set(whole_batch_applies.get() + 1);
                Ok::<_, &str>(())
            },
        );

        assert_eq!(result, ApplyTransactionResult::Success);
        assert_eq!(inner_applied.into_inner(), vec!["first", "second"]);
        assert_eq!(whole_batch_applies.get(), 1);
    }

    #[test]
    fn apply_transaction_flow_with_batch_view_merge_for_txn_source_merges_each_applied_inner_view_then_whole_batch()
     {
        let inner_applied = RefCell::new(Vec::new());
        let merged_views = RefCell::new(Vec::new());
        let whole_batch_applies = Cell::new(0_u32);

        let result = run_apply_transaction_flow_with_batch_view_merge_for_txn_source(
            "registry",
            StubTxnSource {
                txn_type: TxType::BATCH,
            },
            None::<&str>,
            &Rules::new(std::iter::empty()),
            false,
            ApplyFlags::NONE,
            9,
            "base",
            "view",
            "journal",
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                Ok::<_, &str>((
                    Ter::TES_SUCCESS,
                    TxConsequences::new(7, SeqProxy::sequence(1)),
                ))
            },
            |_ctx| unreachable!("successful dispatch should not use fallback"),
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                Ok::<_, &str>(Ter::TES_SUCCESS)
            },
            |_base, _tx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                11_u64
            },
            || 0_u64,
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                Ok::<_, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
            },
            BatchTransactionFlags::UNTIL_FAILURE,
            ["first", "second"],
            |inner| {
                inner_applied.borrow_mut().push(inner);
                Ok::<_, &str>(if inner == "first" {
                    (
                        ApplyResult::new(Ter::TES_SUCCESS, true, true),
                        "merge-first",
                    )
                } else {
                    (ApplyResult::new(Ter::TEC_CLAIM, true, true), "merge-second")
                })
            },
            |per_tx_batch_view| {
                merged_views.borrow_mut().push(per_tx_batch_view);
                Ok::<_, &str>(())
            },
            || {
                whole_batch_applies.set(whole_batch_applies.get() + 1);
                Ok::<_, &str>(())
            },
        );

        assert_eq!(result, ApplyTransactionResult::Success);
        assert_eq!(inner_applied.into_inner(), vec!["first", "second"]);
        assert_eq!(
            merged_views.into_inner(),
            vec!["merge-first", "merge-second"]
        );
        assert_eq!(whole_batch_applies.get(), 1);
    }
}
