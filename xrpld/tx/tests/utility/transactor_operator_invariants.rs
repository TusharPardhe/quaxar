//! Integration tests that pin the invariant-check slice of
//! `Transactor::operator()()` to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{TransactorOperatorInvariantState, run_transactor_operator_invariants};

#[test]
fn tx_transactor_operator_invariants_skips_checker_when_apply_already_failed() {
    let result = run_transactor_operator_invariants(
        Ter::TEC_CLAIM,
        false,
        10_i64,
        |_, _| panic!("applied=false should skip invariant checks"),
        |_| panic!("applied=false should skip reset"),
    );

    assert_eq!(
        result,
        TransactorOperatorInvariantState {
            result: Ter::TEC_CLAIM,
            applied: false,
            fee: 10,
        }
    );
    assert_eq!(trans_token(result.result), "tecCLAIM");
}

#[test]
fn tx_transactor_operator_invariants_clears_applied_on_tef() {
    let result = run_transactor_operator_invariants(
        Ter::TEC_CLAIM,
        true,
        10_i64,
        |incoming, fee| {
            assert_eq!(incoming, Ter::TEC_CLAIM);
            assert_eq!(*fee, 10);
            Ter::TEF_INVARIANT_FAILED
        },
        |_| panic!("tef invariant result should skip reset"),
    );

    assert_eq!(
        result,
        TransactorOperatorInvariantState {
            result: Ter::TEF_INVARIANT_FAILED,
            applied: false,
            fee: 10,
        }
    );
    assert_eq!(trans_token(result.result), "tefINVARIANT_FAILED");
}

#[test]
fn tx_transactor_operator_invariants_resets_and_rechecks_after_tec_invariant_failed() {
    let mut seen_first = false;

    let result = run_transactor_operator_invariants(
        Ter::TEC_CLAIM,
        true,
        10_i64,
        |incoming, fee| {
            if !seen_first {
                seen_first = true;
                assert_eq!(incoming, Ter::TEC_CLAIM);
                assert_eq!(*fee, 10);
                Ter::TEC_INVARIANT_FAILED
            } else {
                assert_eq!(incoming, Ter::TEC_INVARIANT_FAILED);
                assert_eq!(*fee, 12);
                Ter::TEC_CLAIM
            }
        },
        |fee| {
            assert_eq!(fee, 10);
            (Ter::TES_SUCCESS, 12)
        },
    );

    assert_eq!(
        result,
        TransactorOperatorInvariantState {
            result: Ter::TEC_CLAIM,
            applied: true,
            fee: 12,
        }
    );
}

#[test]
fn tx_transactor_operator_invariants_skips_second_check_when_reset_fails() {
    let result = run_transactor_operator_invariants(
        Ter::TEC_CLAIM,
        true,
        10_i64,
        |_, _| Ter::TEC_INVARIANT_FAILED,
        |fee| {
            assert_eq!(fee, 10);
            (Ter::TEF_EXCEPTION, 13)
        },
    );

    assert_eq!(
        result,
        TransactorOperatorInvariantState {
            result: Ter::TEF_EXCEPTION,
            applied: false,
            fee: 13,
        }
    );
    assert_eq!(trans_token(result.result), "tefEXCEPTION");
}
