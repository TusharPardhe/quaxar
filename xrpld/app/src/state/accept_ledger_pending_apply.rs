use std::cell::RefCell;

use protocol::{NotTec, Rules, Ter, TxType};
use tx::{
    ApplyContext, ApplyFlags, ApplyResult, HasTxnType, PreclaimContext, PreflightContext,
    TxConsequences, run_apply_for_txn_source,
};

pub struct AcceptLedgerPendingApplyInputs<Registry, BaseView, View, Tx, Journal, ParentBatchId> {
    pub registry: Registry,
    pub tx: Tx,
    pub parent_batch_id: Option<ParentBatchId>,
    pub current_rules: Rules,
    pub flags: ApplyFlags,
    pub current_ledger_seq: u32,
    pub base: BaseView,
    pub view: View,
    pub journal: Journal,
}

impl<Registry, BaseView, View, Tx, Journal, ParentBatchId>
    AcceptLedgerPendingApplyInputs<Registry, BaseView, View, Tx, Journal, ParentBatchId>
{
    pub fn new(
        registry: Registry,
        tx: Tx,
        parent_batch_id: Option<ParentBatchId>,
        current_rules: Rules,
        flags: ApplyFlags,
        current_ledger_seq: u32,
        base: BaseView,
        view: View,
        journal: Journal,
    ) -> Self {
        Self {
            registry,
            tx,
            parent_batch_id,
            current_rules,
            flags,
            current_ledger_seq,
            base,
            view,
            journal,
        }
    }
}

pub trait AcceptLedgerPendingApplyRuntime<Registry, BaseView, View, Tx, Journal, ParentBatchId> {
    type Fee;
    type PreflightError;
    type PreclaimError;
    type ApplyError;

    fn dispatch_preflight(
        &mut self,
        ctx: &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
        txn_type: TxType,
    ) -> Result<(NotTec, TxConsequences), Self::PreflightError>;

    fn fallback_consequences(
        &mut self,
        ctx: &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    ) -> TxConsequences;

    fn dispatch_preclaim(
        &mut self,
        ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
        txn_type: TxType,
    ) -> Result<Ter, Self::PreclaimError>;

    fn calculate_base_fee(&mut self, base: &BaseView, tx: &Tx, txn_type: TxType) -> Self::Fee;

    fn zero_fee(&mut self) -> Self::Fee;

    fn dispatch_apply(
        &mut self,
        ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Self::Fee, Journal, ParentBatchId>,
        txn_type: TxType,
    ) -> Result<ApplyResult, Self::ApplyError>;
}

pub fn run_accept_ledger_pending_apply<
    Registry,
    BaseView,
    View,
    Tx,
    Journal,
    ParentBatchId,
    Runtime,
>(
    inputs: AcceptLedgerPendingApplyInputs<Registry, BaseView, View, Tx, Journal, ParentBatchId>,
    runtime: &mut Runtime,
) -> ApplyResult
where
    Registry: Clone,
    View: Clone,
    Tx: HasTxnType + Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    Runtime: AcceptLedgerPendingApplyRuntime<Registry, BaseView, View, Tx, Journal, ParentBatchId>,
{
    let AcceptLedgerPendingApplyInputs {
        registry,
        tx,
        parent_batch_id,
        current_rules,
        flags,
        current_ledger_seq,
        base,
        view,
        journal,
    } = inputs;

    let runtime = RefCell::new(runtime);

    run_apply_for_txn_source(
        registry,
        tx,
        parent_batch_id,
        &current_rules,
        flags,
        current_ledger_seq,
        base,
        view,
        journal,
        |ctx, txn_type| runtime.borrow_mut().dispatch_preflight(ctx, txn_type),
        |ctx| runtime.borrow_mut().fallback_consequences(ctx),
        |ctx, txn_type| runtime.borrow_mut().dispatch_preclaim(ctx, txn_type),
        |base, tx, txn_type| runtime.borrow_mut().calculate_base_fee(base, tx, txn_type),
        || runtime.borrow_mut().zero_fee(),
        |ctx, txn_type| runtime.borrow_mut().dispatch_apply(ctx, txn_type),
    )
}
