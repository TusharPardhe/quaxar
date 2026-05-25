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
    preclaim_result: Ter,
    apply_result: ApplyResult,
}

impl RecordingRuntime {
    fn success() -> Self {
        Self {
            events: Vec::new(),
            preclaim_result: Ter::TES_SUCCESS,
            apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
        }
    }

    fn preclaim_retry() -> Self {
        Self {
            events: Vec::new(),
            preclaim_result: Ter::TER_RETRY,
            apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
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
        _ctx: &PreflightContext<&'static str, StubTxnSource, &'static str, &'static str>,
        txn_type: TxType,
    ) -> Result<(protocol::NotTec, TxConsequences), Self::PreflightError> {
        self.events.push("preflight");
        assert_eq!(txn_type, TxType::PAYMENT);
        Ok((
            Ter::TES_SUCCESS,
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
        _ctx: &PreclaimContext<
            &'static str,
            &'static str,
            StubTxnSource,
            &'static str,
            &'static str,
        >,
        txn_type: TxType,
    ) -> Result<Ter, Self::PreclaimError> {
        self.events.push("preclaim");
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
