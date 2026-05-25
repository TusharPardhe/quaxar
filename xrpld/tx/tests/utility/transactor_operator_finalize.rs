//! Integration tests that pin the final apply/metadata slice of
//! `Transactor::operator()()` to the current C++ behavior.

use std::cell::RefCell;

use protocol::{Ter, trans_token};
use tx::{ApplyFlags, TransactorOperatorFinalize, run_transactor_operator_finalize};

#[test]
fn tx_transactor_operator_finalize_skips_tail_when_not_applied() {
    let result = run_transactor_operator_finalize(
        Ter::TEC_CLAIM,
        false,
        10_i64,
        ApplyFlags::NONE,
        false,
        |_| panic!("not-applied path should skip destroy"),
        |_| panic!("not-applied path should skip apply"),
    );

    assert_eq!(
        result,
        TransactorOperatorFinalize {
            result: Ter::TEC_CLAIM,
            applied: false,
            metadata: None::<&'static str>,
        }
    );
    assert_eq!(trans_token(result.result), "tecCLAIM");
}

#[test]
fn tx_transactor_operator_finalize_destroys_fee_before_apply_on_closed_ledger() {
    let trace = RefCell::new(Vec::new());

    let result = run_transactor_operator_finalize(
        Ter::TES_SUCCESS,
        true,
        12_i64,
        ApplyFlags::NONE,
        false,
        |fee| {
            assert_eq!(fee, 12);
            trace.borrow_mut().push("destroy");
        },
        |incoming| {
            assert_eq!(incoming, Ter::TES_SUCCESS);
            trace.borrow_mut().push("apply");
            "metadata"
        },
    );

    assert_eq!(
        result,
        TransactorOperatorFinalize {
            result: Ter::TES_SUCCESS,
            applied: true,
            metadata: Some("metadata"),
        }
    );
    assert_eq!(trace.into_inner(), vec!["destroy", "apply"]);
}

#[test]
fn tx_transactor_operator_finalize_skips_destroy_for_zero_fee() {
    let result = run_transactor_operator_finalize(
        Ter::TES_SUCCESS,
        true,
        0_i64,
        ApplyFlags::NONE,
        false,
        |_| panic!("zero fee should skip destroy"),
        |incoming| {
            assert_eq!(incoming, Ter::TES_SUCCESS);
            "metadata"
        },
    );

    assert_eq!(
        result,
        TransactorOperatorFinalize {
            result: Ter::TES_SUCCESS,
            applied: true,
            metadata: Some("metadata"),
        }
    );
}

#[test]
fn tx_transactor_operator_finalize_keeps_metadata_but_clears_applied_on_dry_run() {
    let result = run_transactor_operator_finalize(
        Ter::TES_SUCCESS,
        true,
        0_i64,
        ApplyFlags::DRY_RUN,
        true,
        |_| panic!("zero fee on open ledger should skip destroy"),
        |incoming| {
            assert_eq!(incoming, Ter::TES_SUCCESS);
            "metadata"
        },
    );

    assert_eq!(
        result,
        TransactorOperatorFinalize {
            result: Ter::TES_SUCCESS,
            applied: false,
            metadata: Some("metadata"),
        }
    );
}
