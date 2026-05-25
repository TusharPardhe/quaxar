//! Current Rust helper mirroring the front of `Transactor::operator()()`.
//!
//! This module preserves the exact current front sequencing:
//!
//! - start from the preclaim result,
//! - only call `apply()` when preclaim succeeded,
//! - assert that neither preclaim nor apply yields `temUNKNOWN`,
//! - derive the initial `applied` bit from the pre-oversize result,
//! - read the fee after that initial apply decision, and
//! - override the result to `tecOVERSIZE` when metadata exceeds the cap
//!   without retroactively changing the already-derived `applied` bit.

use protocol::{Ter, is_tes_success};

pub const TRANSACTOR_OPERATOR_FRONT_ASSERT_MESSAGE: &str =
    "xrpl::Transactor::operator() : result is not temUNKNOWN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactorOperatorFront<Fee> {
    pub result: Ter,
    pub applied: bool,
    pub fee: Fee,
}

pub fn run_transactor_operator_front<Fee>(
    preclaim_result: Ter,
    apply: impl FnOnce() -> Ter,
    read_fee: impl FnOnce() -> Fee,
    metadata_size: usize,
    oversize_metadata_cap: usize,
) -> TransactorOperatorFront<Fee> {
    let mut result = preclaim_result;
    if is_tes_success(result) {
        result = apply();
    }

    assert!(
        result != Ter::TEM_UNKNOWN,
        "{TRANSACTOR_OPERATOR_FRONT_ASSERT_MESSAGE}"
    );

    let applied = is_tes_success(result);
    let fee = read_fee();

    if metadata_size > oversize_metadata_cap {
        result = Ter::TEC_OVERSIZE;
    }

    TransactorOperatorFront {
        result,
        applied,
        fee,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        panic::{AssertUnwindSafe, catch_unwind},
    };

    use protocol::{Ter, trans_token};

    use super::{
        TRANSACTOR_OPERATOR_FRONT_ASSERT_MESSAGE, TransactorOperatorFront,
        run_transactor_operator_front,
    };

    #[test]
    fn transactor_operator_front_skips_apply_after_preclaim_failure() {
        let apply_called = Cell::new(false);
        let fee_read = Cell::new(false);

        let result = run_transactor_operator_front(
            Ter::TER_NO_ACCOUNT,
            || {
                apply_called.set(true);
                Ter::TES_SUCCESS
            },
            || {
                fee_read.set(true);
                10_i64
            },
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
        assert!(!apply_called.get());
        assert!(fee_read.get());
    }

    #[test]
    fn transactor_operator_front_asserts_tem_unknown() {
        let panic = catch_unwind(AssertUnwindSafe(|| {
            let _ = run_transactor_operator_front(
                Ter::TES_SUCCESS,
                || Ter::TEM_UNKNOWN,
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
    fn transactor_operator_front_reads_fee_after_apply_result() {
        let trace = RefCell::new(Vec::new());

        let result = run_transactor_operator_front(
            Ter::TES_SUCCESS,
            || {
                trace.borrow_mut().push("apply");
                Ter::TEC_CLAIM
            },
            || {
                trace.borrow_mut().push("fee");
                22_i64
            },
            2,
            8,
        );

        assert_eq!(
            result,
            TransactorOperatorFront {
                result: Ter::TEC_CLAIM,
                applied: false,
                fee: 22,
            }
        );
        assert_eq!(trace.into_inner(), vec!["apply", "fee"]);
        assert_eq!(trans_token(result.result), "tecCLAIM");
    }

    #[test]
    fn transactor_operator_front_overrides_result_to_oversize_without_changing_applied() {
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
}
