//! Current Rust helper mirroring the invariant-check slice of
//! `Transactor::operator()()`.
//!
//! This module preserves the exact current branch policy:
//!
//! - skip all invariant work when `applied` is already false,
//! - always run `checkInvariants(result, fee)` once when `applied` is true,
//! - on `tecINVARIANT_FAILED`, reset once and only rerun invariants when the
//!   reset result is still `tes` or `tec`,
//! - carry the post-reset fee into that second invariant pass, and
//! - clear `applied` whenever the final invariant result is neither `tes` nor
//!   `tec`.

use protocol::{Ter, is_tec_claim, is_tes_success};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactorOperatorInvariantState<Fee> {
    pub result: Ter,
    pub applied: bool,
    pub fee: Fee,
}

pub fn run_transactor_operator_invariants<Fee>(
    result: Ter,
    applied: bool,
    fee: Fee,
    mut check_invariants: impl FnMut(Ter, &Fee) -> Ter,
    reset: impl FnOnce(Fee) -> (Ter, Fee),
) -> TransactorOperatorInvariantState<Fee> {
    if !applied {
        return TransactorOperatorInvariantState {
            result,
            applied,
            fee,
        };
    }

    let mut result = check_invariants(result, &fee);
    let mut fee = fee;

    if result == Ter::TEC_INVARIANT_FAILED {
        let (reset_result, reset_fee) = reset(fee);
        fee = reset_fee;

        if !is_tes_success(reset_result) {
            result = reset_result;
        }

        if is_tes_success(result) || is_tec_claim(result) {
            result = check_invariants(result, &fee);
        }
    }

    let applied = is_tes_success(result) || is_tec_claim(result);

    TransactorOperatorInvariantState {
        result,
        applied,
        fee,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use protocol::{Ter, trans_token};

    use super::{TransactorOperatorInvariantState, run_transactor_operator_invariants};

    #[test]
    fn transactor_operator_invariants_skips_checker_when_apply_already_failed() {
        let checked = Cell::new(false);

        let result = run_transactor_operator_invariants(
            Ter::TEC_CLAIM,
            false,
            10_i64,
            |_, _| {
                checked.set(true);
                Ter::TES_SUCCESS
            },
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
        assert!(!checked.get());
    }

    #[test]
    fn transactor_operator_invariants_clears_applied_on_tef() {
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
    fn transactor_operator_invariants_resets_and_rechecks_after_tec_invariant_failed() {
        let calls = RefCell::new(Vec::new());

        let result = run_transactor_operator_invariants(
            Ter::TEC_CLAIM,
            true,
            10_i64,
            |incoming, fee| {
                calls.borrow_mut().push((incoming, *fee));
                if calls.borrow().len() == 1 {
                    Ter::TEC_INVARIANT_FAILED
                } else {
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
        assert_eq!(
            calls.into_inner(),
            vec![(Ter::TEC_CLAIM, 10), (Ter::TEC_INVARIANT_FAILED, 12)]
        );
    }

    #[test]
    fn transactor_operator_invariants_skips_second_check_when_reset_fails() {
        let checked = Cell::new(0_u32);

        let result = run_transactor_operator_invariants(
            Ter::TEC_CLAIM,
            true,
            10_i64,
            |_, _| {
                checked.set(checked.get() + 1);
                Ter::TEC_INVARIANT_FAILED
            },
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
        assert_eq!(checked.get(), 1);
    }
}
