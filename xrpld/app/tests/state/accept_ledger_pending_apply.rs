#[path = "../../src/state/accept_ledger_pending_apply.rs"]
mod accept_ledger_pending_apply_impl;

use accept_ledger_pending_apply_impl::{
    AcceptLedgerPendingApplyInputs, AcceptLedgerPendingApplyRuntime,
    run_accept_ledger_pending_apply,
};
use protocol::{Rules, SeqProxy, Ter, TxType};
use tx::{
    ApplyContext, ApplyFlags, ApplyResult, HasTxnType, PreclaimContext, PreflightContext,
    TxConsequences,
};

#[derive(Clone)]
struct StubTxnSource {
    txn_type: TxType,
}

impl HasTxnType for StubTxnSource {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

struct RecordingRuntime {
    events: Vec<&'static str>,
    contexts: Vec<(&'static str, Option<&'static str>, ApplyFlags, TxType)>,
    preflight_result: Ter,
    preclaim_result: Ter,
    apply_result: ApplyResult,
}

impl RecordingRuntime {
    fn success() -> Self {
        Self {
            events: Vec::new(),
            contexts: Vec::new(),
            preflight_result: Ter::TES_SUCCESS,
            preclaim_result: Ter::TES_SUCCESS,
            apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
        }
    }

    fn preclaim_retry() -> Self {
        Self {
            events: Vec::new(),
            contexts: Vec::new(),
            preflight_result: Ter::TES_SUCCESS,
            preclaim_result: Ter::TER_RETRY,
            apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
        }
    }

    fn with_preclaim_result(preclaim_result: Ter) -> Self {
        Self {
            preclaim_result,
            ..Self::success()
        }
    }
}

impl
    AcceptLedgerPendingApplyRuntime<
        &'static str,
        &'static str,
        &'static str,
        StubTxnSource,
        &'static str,
        &'static str,
    > for RecordingRuntime
{
    type Fee = u64;
    type PreflightError = &'static str;
    type PreclaimError = &'static str;
    type ApplyError = &'static str;

    fn dispatch_preflight(
        &mut self,
        ctx: &PreflightContext<&'static str, StubTxnSource, &'static str, &'static str>,
        txn_type: TxType,
    ) -> Result<(protocol::NotTec, TxConsequences), Self::PreflightError> {
        self.events.push("preflight");
        self.contexts
            .push(("preflight", ctx.parent_batch_id, ctx.flags, txn_type));
        assert_eq!(txn_type, TxType::PAYMENT);
        Ok((
            self.preflight_result,
            TxConsequences::new(7, SeqProxy::sequence(3)),
        ))
    }

    fn fallback_consequences(
        &mut self,
        _ctx: &PreflightContext<&'static str, StubTxnSource, &'static str, &'static str>,
    ) -> TxConsequences {
        self.events.push("fallback");
        TxConsequences::from_preflight_result(Ter::TEF_EXCEPTION)
    }

    fn dispatch_preclaim(
        &mut self,
        ctx: &PreclaimContext<
            &'static str,
            &'static str,
            StubTxnSource,
            &'static str,
            &'static str,
        >,
        txn_type: TxType,
    ) -> Result<Ter, Self::PreclaimError> {
        self.events.push("preclaim");
        self.contexts
            .push(("preclaim", ctx.parent_batch_id, ctx.flags, txn_type));
        assert_eq!(txn_type, TxType::PAYMENT);
        Ok(self.preclaim_result)
    }

    fn calculate_base_fee(
        &mut self,
        base: &&'static str,
        _tx: &StubTxnSource,
        txn_type: TxType,
    ) -> Self::Fee {
        self.events.push("fee");
        assert_eq!(*base, "base");
        assert_eq!(txn_type, TxType::PAYMENT);
        12
    }

    fn zero_fee(&mut self) -> Self::Fee {
        self.events.push("zero_fee");
        0
    }

    fn dispatch_apply(
        &mut self,
        ctx: &mut ApplyContext<
            &'static str,
            &'static str,
            &'static str,
            StubTxnSource,
            Self::Fee,
            &'static str,
            &'static str,
        >,
        txn_type: TxType,
    ) -> Result<ApplyResult, Self::ApplyError> {
        self.events.push("apply");
        self.contexts
            .push(("apply", ctx.parent_batch_id, ctx.flags(), txn_type));
        assert_eq!(ctx.base_fee, 12);
        assert_eq!(txn_type, TxType::PAYMENT);
        Ok(self.apply_result.clone())
    }
}

#[test]
fn accept_ledger_pending_apply_runs_full_tx_apply_flow() {
    let inputs = AcceptLedgerPendingApplyInputs::new(
        "registry",
        StubTxnSource {
            txn_type: TxType::PAYMENT,
        },
        Some("batch"),
        Rules::new(std::iter::empty()),
        ApplyFlags::FAIL_HARD | ApplyFlags::BATCH,
        9,
        "base",
        "view",
        "journal",
    );
    let mut runtime = RecordingRuntime::success();

    let result = run_accept_ledger_pending_apply(inputs, &mut runtime);

    assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, true));
    assert_eq!(
        runtime.events,
        vec!["preflight", "preclaim", "fee", "apply"]
    );
}

#[test]
fn batch_inner_apply_preserves_parent_context_and_flags_through_shared_pipeline() {
    // Parity: ../rippled/src/libxrpl/tx/apply.cpp::applyBatchTransactions
    // invokes apply(..., parentBatchId, tx, TapBatch, ...); the overload in
    // ../rippled/src/libxrpl/tx/applySteps.cpp::preflight retains that parent
    // in both PreflightResult and PreclaimResult before doApply consumes it.
    let flags = ApplyFlags::FAIL_HARD | ApplyFlags::BATCH;
    let inputs = AcceptLedgerPendingApplyInputs::new(
        "registry",
        StubTxnSource {
            txn_type: TxType::PAYMENT,
        },
        Some("outer-batch-id"),
        Rules::new(std::iter::empty()),
        flags,
        9,
        "base",
        "view",
        "journal",
    );
    let mut runtime = RecordingRuntime::success();

    let result = run_accept_ledger_pending_apply(inputs, &mut runtime);

    assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, true));
    assert_eq!(
        runtime.contexts,
        vec![
            ("preflight", Some("outer-batch-id"), flags, TxType::PAYMENT),
            ("preclaim", Some("outer-batch-id"), flags, TxType::PAYMENT),
            ("apply", Some("outer-batch-id"), flags, TxType::PAYMENT),
        ],
        "the parent Batch identity and TapBatch-equivalent flags are metadata \
         carried through the canonical inner transaction pipeline"
    );
}

#[test]
fn accept_ledger_pending_apply_does_not_mark_preclaim_retry_as_applied() {
    let inputs = AcceptLedgerPendingApplyInputs::new(
        "registry",
        StubTxnSource {
            txn_type: TxType::PAYMENT,
        },
        None::<&'static str>,
        Rules::new(std::iter::empty()),
        ApplyFlags::NONE,
        9,
        "base",
        "view",
        "journal",
    );
    let mut runtime = RecordingRuntime::preclaim_retry();

    let result = run_accept_ledger_pending_apply(inputs, &mut runtime);

    assert_eq!(result, ApplyResult::new(Ter::TER_RETRY, false, false));
    assert_eq!(runtime.events, vec!["preflight", "preclaim"]);
}

#[test]
fn standalone_semantic_rejections_do_not_reach_mutation() {
    // Parity: ../rippled/src/libxrpl/tx/applySteps.cpp::invokePreclaim checks
    // sequence and signature admission before returning to doApply. A terminal
    // standalone/open-replay result must therefore never call the mutation hook.
    for (name, preclaim) in [
        ("invalid signature", Ter::TEM_BAD_SIGNATURE),
        ("stale sequence", Ter::TER_PRE_SEQ),
    ] {
        let inputs = AcceptLedgerPendingApplyInputs::new(
            "registry",
            StubTxnSource {
                txn_type: TxType::PAYMENT,
            },
            None::<&'static str>,
            Rules::new(std::iter::empty()),
            ApplyFlags::NONE,
            9,
            "base",
            "view",
            "journal",
        );
        let mut runtime = RecordingRuntime::with_preclaim_result(preclaim);

        let result = run_accept_ledger_pending_apply(inputs, &mut runtime);

        assert_eq!(result, ApplyResult::new(preclaim, false, false), "{name}");
        assert_eq!(runtime.events, vec!["preflight", "preclaim"], "{name}");
        assert!(
            runtime
                .contexts
                .iter()
                .all(|(stage, _, _, _)| *stage != "apply"),
            "{name} must be rejected before mutation"
        );
    }
}

#[test]
fn accept_ledger_pending_apply_maps_unknown_transaction_type_to_temunknown() {
    let inputs = AcceptLedgerPendingApplyInputs::new(
        "registry",
        StubTxnSource {
            txn_type: TxType::HOOK_SET,
        },
        None::<&'static str>,
        Rules::new(std::iter::empty()),
        ApplyFlags::NONE,
        9,
        "base",
        "view",
        "journal",
    );
    let mut runtime = RecordingRuntime::success();

    let result = run_accept_ledger_pending_apply(inputs, &mut runtime);

    assert_eq!(
        result,
        ApplyResult::new(tx::UNKNOWN_TRANSACTION_TYPE_TER, false, false)
    );
    assert!(runtime.events.is_empty());
}
