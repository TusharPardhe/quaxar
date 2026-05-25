//! Public system-family `doApply(...)` composition shell above the landed
//! generic the transaction dispatch layer helpers.
//!
//! This ports the public composition layer that:
//!
//! - preserves the current stale-ledger early-exit behavior,
//! - preserves the current `tefEXCEPTION` mapping for thrown apply errors,
//! - preserves the current `temUNKNOWN` fallback for non-system transaction
//!   types,
//! - and composes the landed system invoke-apply helper without re-encoding
//!   the family routing at the caller boundary.

use protocol::{Rules, TxType};

use crate::apply_steps_entrypoint::run_do_apply_for_txn_type;
use crate::system_invoke_apply::run_with_system_apply_txn_type_key;
use crate::{
    ApplyContext, ApplyResult, HasTxnType, PreclaimResult, UNKNOWN_TRANSACTION_TYPE_TER,
    txn_type_of,
};

pub fn run_system_apply_for_txn_type<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId, E>(
    preclaim_result: PreclaimResult<Tx, Journal, ParentBatchId>,
    txn_type: TxType,
    rules: &Rules,
    registry: Registry,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx, TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
    dispatch_apply: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
        TxType,
    ) -> Result<ApplyResult, E>,
) -> ApplyResult
where
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
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
        |ctx, txn_type| {
            if run_with_system_apply_txn_type_key(txn_type, |_| ()).is_err() {
                return Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false));
            }

            dispatch_apply(ctx, txn_type)
        },
    )
}

pub fn run_system_apply_for_txn_source<
    Registry,
    BaseView,
    View,
    Tx,
    Fee,
    Journal,
    ParentBatchId,
    E,
>(
    preclaim_result: PreclaimResult<Tx, Journal, ParentBatchId>,
    rules: &Rules,
    registry: Registry,
    current_ledger_seq: u32,
    base: BaseView,
    view: View,
    calculate_base_fee: impl FnOnce(&BaseView, &Tx, TxType) -> Fee,
    zero_fee: impl FnOnce() -> Fee,
    dispatch_apply: impl FnOnce(
        &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
        TxType,
    ) -> Result<ApplyResult, E>,
) -> ApplyResult
where
    Tx: HasTxnType + Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    let txn_type = txn_type_of(&preclaim_result.tx);
    run_system_apply_for_txn_type(
        preclaim_result,
        txn_type,
        rules,
        registry,
        current_ledger_seq,
        base,
        view,
        calculate_base_fee,
        zero_fee,
        dispatch_apply,
    )
}
