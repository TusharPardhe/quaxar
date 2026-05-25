//! Public vault-family `preclaim(...)` composition shell above the landed
//! generic the transaction dispatch layer helpers.
//!
//! This ports the public composition layer that:
//!
//! - preserves the current preflight reflight reuse when rules change,
//! - preserves the current `tefEXCEPTION` mapping for thrown preclaim errors,
//! - preserves the current `temUNKNOWN` fallback for non-vault transaction
//!   types,
//! - and composes the landed generic preclaim shell at the public boundary.

use protocol::{Rules, Ter, TxType};

use crate::apply_steps_entrypoint::run_preclaim_for_txn_type;
use crate::{
    HasTxnType, PreclaimContext, PreclaimResult, PreflightResult, TxConsequences,
    UNKNOWN_TRANSACTION_TYPE_TER, run_with_vault_txn_type_key, txn_type_of,
};

pub fn run_vault_preclaim_for_txn_type<Registry, View, Tx, Journal, ParentBatchId, E>(
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
    dispatch_preclaim: impl FnOnce(
        &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
        TxType,
    ) -> Result<Ter, E>,
) -> PreclaimResult<Tx, Journal, ParentBatchId>
where
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    run_preclaim_for_txn_type(
        preflight_result,
        txn_type,
        registry,
        view,
        current_rules,
        current_ledger_seq,
        rerun_preflight,
        |ctx, txn_type| {
            if run_with_vault_txn_type_key(txn_type, |_| ()).is_err() {
                return Ok(UNKNOWN_TRANSACTION_TYPE_TER);
            }

            dispatch_preclaim(ctx, txn_type)
        },
    )
}

pub fn run_vault_preclaim_for_txn_source<Registry, View, Tx, Journal, ParentBatchId, E>(
    preflight_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    registry: Registry,
    view: View,
    current_rules: &Rules,
    current_ledger_seq: u32,
    rerun_preflight: impl FnOnce(
        &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        &Rules,
    ) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    dispatch_preclaim: impl FnOnce(
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
    run_vault_preclaim_for_txn_type(
        preflight_result,
        txn_type,
        registry,
        view,
        current_rules,
        current_ledger_seq,
        rerun_preflight,
        dispatch_preclaim,
    )
}
