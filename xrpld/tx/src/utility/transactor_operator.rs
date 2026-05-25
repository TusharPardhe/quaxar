//! Current Rust helper mirroring the top-level control-flow shell of
//! `Transactor::operator()()`.
//!
//! This composes the already-landed front, reapply, invariant, and finalize
//! shells into the current Rust transactor flow.

use crate::{
    ApplyFlags, TransactorOperatorFinalize, TransactorOperatorInvariantState,
    TransactorOperatorReapplyCollection, run_transactor_operator_finalize,
    run_transactor_operator_front, run_transactor_operator_invariants,
    run_transactor_operator_reapply,
};
use protocol::Ter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactorOperatorResult<Fee, Meta> {
    pub result: Ter,
    pub applied: bool,
    pub fee: Fee,
    pub metadata: Option<Meta>,
}

#[allow(clippy::too_many_arguments)]
pub fn run_transactor_operator<Fee, Meta>(
    preclaim_result: Ter,
    apply: impl FnOnce() -> Ter,
    read_fee: impl FnOnce() -> Fee,
    metadata_size: usize,
    oversize_metadata_cap: usize,
    flags: ApplyFlags,
    view_open: bool,
    discard: impl FnOnce(),
    visit_and_reset: impl FnOnce(TransactorOperatorReapplyCollection, Fee) -> (Ter, Fee),
    remove_unfunded_offers: impl FnOnce(),
    remove_deleted_trust_lines: impl FnOnce(),
    remove_expired_nftoken_offers: impl FnOnce(),
    remove_expired_credentials: impl FnOnce(),
    check_invariants: impl FnMut(Ter, &Fee) -> Ter,
    reset_invariants: impl FnOnce(Fee) -> (Ter, Fee),
    destroy_xrp: impl FnOnce(Fee),
    finalize_apply: impl FnOnce(Ter) -> Meta,
) -> TransactorOperatorResult<Fee, Meta>
where
    Fee: Clone + Default + PartialEq + PartialOrd,
{
    let front = run_transactor_operator_front(
        preclaim_result,
        apply,
        read_fee,
        metadata_size,
        oversize_metadata_cap,
    );

    let reapply = run_transactor_operator_reapply(
        front.result,
        front.applied,
        front.fee,
        flags,
        discard,
        visit_and_reset,
        remove_unfunded_offers,
        remove_deleted_trust_lines,
        remove_expired_nftoken_offers,
        remove_expired_credentials,
    );

    let TransactorOperatorInvariantState {
        result,
        applied,
        fee,
    } = run_transactor_operator_invariants(
        reapply.result,
        reapply.applied,
        reapply.fee,
        check_invariants,
        reset_invariants,
    );

    let TransactorOperatorFinalize {
        result,
        applied,
        metadata,
    } = run_transactor_operator_finalize(
        result,
        applied,
        fee.clone(),
        flags,
        view_open,
        destroy_xrp,
        finalize_apply,
    );

    TransactorOperatorResult {
        result,
        applied,
        fee,
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use protocol::{Ter, trans_token};

    use crate::ApplyFlags;

    use super::{TransactorOperatorResult, run_transactor_operator};

    #[test]
    fn transactor_operator_runs_full_success_tail() {
        let trace = RefCell::new(Vec::new());

        let result = run_transactor_operator(
            Ter::TES_SUCCESS,
            || {
                trace.borrow_mut().push("apply");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("fee");
                12_i64
            },
            2,
            8,
            ApplyFlags::NONE,
            false,
            || panic!("success path should skip discard"),
            |_, _| panic!("success path should skip reapply reset"),
            || panic!("success path should skip offer removal"),
            || panic!("success path should skip line removal"),
            || panic!("success path should skip nft offer removal"),
            || panic!("success path should skip credential removal"),
            |incoming, fee| {
                trace.borrow_mut().push("invariants");
                assert_eq!(incoming, Ter::TES_SUCCESS);
                assert_eq!(*fee, 12);
                Ter::TES_SUCCESS
            },
            |_| panic!("success path should skip invariant reset"),
            |fee| {
                trace.borrow_mut().push("destroy");
                assert_eq!(fee, 12);
            },
            |incoming| {
                trace.borrow_mut().push("finalize_apply");
                assert_eq!(incoming, Ter::TES_SUCCESS);
                "metadata"
            },
        );

        assert_eq!(
            result,
            TransactorOperatorResult {
                result: Ter::TES_SUCCESS,
                applied: true,
                fee: 12,
                metadata: Some("metadata"),
            }
        );
        assert_eq!(
            trace.into_inner(),
            vec!["apply", "fee", "invariants", "destroy", "finalize_apply"]
        );
    }

    #[test]
    fn transactor_operator_reapplies_then_finalizes_tec_claim() {
        let visited = Cell::new(false);
        let removed = Cell::new(false);

        let result = run_transactor_operator(
            Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || 10_i64,
            9,
            8,
            ApplyFlags::RETRY,
            true,
            || panic!("oversize retry path should skip discard"),
            |collection, fee| {
                visited.set(true);
                assert!(collection.offers);
                assert_eq!(fee, 10);
                (Ter::TEC_OVERSIZE, 14)
            },
            || removed.set(true),
            || panic!("oversize path should skip line removal"),
            || panic!("oversize path should skip nft offer removal"),
            || panic!("oversize path should skip credential removal"),
            |incoming, fee| {
                assert_eq!(incoming, Ter::TEC_OVERSIZE);
                assert_eq!(*fee, 14);
                Ter::TEC_OVERSIZE
            },
            |_| panic!("successful tec invariant pass should skip reset"),
            |_| panic!("open ledger should skip destroy"),
            |incoming| {
                assert_eq!(incoming, Ter::TEC_OVERSIZE);
                "metadata"
            },
        );

        assert_eq!(
            result,
            TransactorOperatorResult {
                result: Ter::TEC_OVERSIZE,
                applied: true,
                fee: 14,
                metadata: Some("metadata"),
            }
        );
        assert_eq!(trans_token(result.result), "tecOVERSIZE");
        assert!(visited.get());
        assert!(removed.get());
    }

    #[test]
    fn transactor_operator_stops_before_finalize_when_invariants_fail() {
        let finalized = Cell::new(false);

        let result = run_transactor_operator(
            Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || 9_i64,
            2,
            8,
            ApplyFlags::NONE,
            false,
            || panic!("invariant-failure path should skip discard"),
            |_, _| panic!("invariant-failure path should skip reapply"),
            || panic!("invariant-failure path should skip offer removal"),
            || panic!("invariant-failure path should skip line removal"),
            || panic!("invariant-failure path should skip nft offer removal"),
            || panic!("invariant-failure path should skip credential removal"),
            |_, _| Ter::TEC_INVARIANT_FAILED,
            |fee| {
                assert_eq!(fee, 9);
                (Ter::TEF_EXCEPTION, 11)
            },
            |_| panic!("failed invariant recovery should skip destroy"),
            |_| {
                finalized.set(true);
                "metadata"
            },
        );

        assert_eq!(
            result,
            TransactorOperatorResult {
                result: Ter::TEF_EXCEPTION,
                applied: false,
                fee: 11,
                metadata: None::<&'static str>,
            }
        );
        assert!(!finalized.get());
    }

    #[test]
    fn transactor_operator_preserves_metadata_but_clears_applied_on_dry_run() {
        let result = run_transactor_operator(
            Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || 0_i64,
            2,
            8,
            ApplyFlags::DRY_RUN,
            true,
            || panic!("dry-run success should skip discard"),
            |_, _| panic!("dry-run success should skip reapply"),
            || panic!("dry-run success should skip offer removal"),
            || panic!("dry-run success should skip line removal"),
            || panic!("dry-run success should skip nft offer removal"),
            || panic!("dry-run success should skip credential removal"),
            |incoming, fee| {
                assert_eq!(incoming, Ter::TES_SUCCESS);
                assert_eq!(*fee, 0);
                Ter::TES_SUCCESS
            },
            |_| panic!("dry-run success should skip invariant reset"),
            |_| panic!("open ledger zero fee should skip destroy"),
            |incoming| {
                assert_eq!(incoming, Ter::TES_SUCCESS);
                "metadata"
            },
        );

        assert_eq!(
            result,
            TransactorOperatorResult {
                result: Ter::TES_SUCCESS,
                applied: false,
                fee: 0,
                metadata: Some("metadata"),
            }
        );
    }
}
