//! Integration tests that pin the front `Transactor::operator()()` shell to
//! the current C++ behavior.

use std::panic::{AssertUnwindSafe, catch_unwind};

use protocol::{Ter, trans_token};
use tx::{
    TRANSACTOR_OPERATOR_FRONT_ASSERT_MESSAGE, TransactorOperatorFront,
    run_transactor_operator_front,
};

#[test]
fn tx_transactor_operator_front_skips_apply_after_preclaim_failure() {
    let result = run_transactor_operator_front(
        Ter::TER_NO_ACCOUNT,
        || panic!("preclaim failure should skip apply"),
        || 10_i64,
        2,
        8,
    );

    assert_eq!(
        result,
        TransactorOperatorFront {
            result: Ter::TER_NO_ACCOUNT,
            applied: false,
            fee: 10,
        }
    );
    assert_eq!(trans_token(result.result), "terNO_ACCOUNT");
}

#[test]
fn tx_transactor_operator_front_asserts_tem_unknown() {
    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = run_transactor_operator_front(
            Ter::TEM_UNKNOWN,
            || panic!("temUNKNOWN preclaim result should skip apply"),
            || panic!("temUNKNOWN should assert before fee read"),
            2,
            8,
        );
    }))
    .expect_err("temUNKNOWN should assert");

    let message = if let Some(message) = panic.downcast_ref::<String>() {
        message.as_str()
    } else if let Some(message) = panic.downcast_ref::<&'static str>() {
        message
    } else {
        panic!("unexpected panic payload");
    };

    assert!(message.contains(TRANSACTOR_OPERATOR_FRONT_ASSERT_MESSAGE));
}

#[test]
fn tx_transactor_operator_front_uses_apply_result_when_preclaim_succeeds() {
    let result =
        run_transactor_operator_front(Ter::TES_SUCCESS, || Ter::TES_SUCCESS, || 12_i64, 2, 8);

    assert_eq!(
        result,
        TransactorOperatorFront {
            result: Ter::TES_SUCCESS,
            applied: true,
            fee: 12,
        }
    );
    assert_eq!(trans_token(result.result), "tesSUCCESS");
}

#[test]
fn tx_transactor_operator_front_overrides_to_oversize_after_fee_capture() {
    let result =
        run_transactor_operator_front(Ter::TES_SUCCESS, || Ter::TES_SUCCESS, || 15_i64, 9, 8);

    assert_eq!(
        result,
        TransactorOperatorFront {
            result: Ter::TEC_OVERSIZE,
            applied: true,
            fee: 15,
        }
    );
    assert_eq!(trans_token(result.result), "tecOVERSIZE");
}
