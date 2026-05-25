//! Integration tests that pin the composed control-flow shell of
//! `Transactor::operator()()` to the current C++ behavior.

use std::cell::RefCell;

use protocol::{Ter, trans_token};
use tx::{ApplyFlags, TransactorOperatorResult, run_transactor_operator};

#[test]
fn tx_transactor_operator_runs_full_success_tail() {
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
fn tx_transactor_operator_reapplies_then_finalizes_tec_claim() {
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
            assert!(collection.offers);
            assert_eq!(fee, 10);
            (Ter::TEC_OVERSIZE, 14)
        },
        || {},
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
}

#[test]
fn tx_transactor_operator_stops_before_finalize_when_invariants_fail() {
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
        |_| panic!("failed invariant recovery should skip finalize apply"),
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
}

#[test]
fn tx_transactor_operator_preserves_metadata_but_clears_applied_on_dry_run() {
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
