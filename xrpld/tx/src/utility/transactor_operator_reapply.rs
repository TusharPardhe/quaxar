//! Current Rust helper mirroring the fail-hard and reapply policy slice of
//! `Transactor::operator()()`.
//!
//! This module preserves the exact current branch policy:
//!
//! - `tapFAIL_HARD` turns any `tec` result into a discard plus `applied = false`
//!   without attempting reapply work,
//! - otherwise the current reapply branch is entered for `tecOVERSIZE`,
//!   `tecKILLED`, `tecINCOMPLETE`, `tecEXPIRED`, and any `tec` result that is a
//!   hard-fail claim under the current flags,
//! - the visit/reset callback receives collection flags derived from the
//!   pre-reset result,
//! - the post-reset removal callbacks are selected from the post-reset result,
//!   and
//! - `applied` is recomputed from the post-reset result.

use protocol::{Ter, is_tec_claim};

use crate::{ApplyFlags, is_tec_claim_hard_fail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactorOperatorReapplyCollection {
    pub offers: bool,
    pub trust_lines: bool,
    pub nftoken_offers: bool,
    pub credentials: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactorOperatorReapplyState<Fee> {
    pub result: Ter,
    pub applied: bool,
    pub fee: Fee,
}

#[allow(clippy::too_many_arguments)]
pub fn run_transactor_operator_reapply<Fee>(
    result: Ter,
    applied: bool,
    fee: Fee,
    flags: ApplyFlags,
    discard: impl FnOnce(),
    visit_and_reset: impl FnOnce(TransactorOperatorReapplyCollection, Fee) -> (Ter, Fee),
    remove_unfunded_offers: impl FnOnce(),
    remove_deleted_trust_lines: impl FnOnce(),
    remove_expired_nftoken_offers: impl FnOnce(),
    remove_expired_credentials: impl FnOnce(),
) -> TransactorOperatorReapplyState<Fee> {
    if is_tec_claim(result) && (flags & ApplyFlags::FAIL_HARD) == ApplyFlags::FAIL_HARD {
        discard();
        return TransactorOperatorReapplyState {
            result,
            applied: false,
            fee,
        };
    }

    let needs_reapply = matches!(
        result,
        Ter::TEC_OVERSIZE | Ter::TEC_KILLED | Ter::TEC_INCOMPLETE | Ter::TEC_EXPIRED
    ) || is_tec_claim_hard_fail(result, flags);

    if !needs_reapply {
        return TransactorOperatorReapplyState {
            result,
            applied,
            fee,
        };
    }

    let collection = TransactorOperatorReapplyCollection {
        offers: matches!(result, Ter::TEC_OVERSIZE | Ter::TEC_KILLED),
        trust_lines: result == Ter::TEC_INCOMPLETE,
        nftoken_offers: result == Ter::TEC_EXPIRED,
        credentials: result == Ter::TEC_EXPIRED,
    };

    let (result, fee) = visit_and_reset(collection, fee);

    if matches!(result, Ter::TEC_OVERSIZE | Ter::TEC_KILLED) {
        remove_unfunded_offers();
    }
    if result == Ter::TEC_EXPIRED {
        remove_expired_nftoken_offers();
    }
    if result == Ter::TEC_INCOMPLETE {
        remove_deleted_trust_lines();
    }
    if result == Ter::TEC_EXPIRED {
        remove_expired_credentials();
    }

    TransactorOperatorReapplyState {
        result,
        applied: is_tec_claim(result),
        fee,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use protocol::{Ter, trans_token};

    use crate::ApplyFlags;

    use super::{
        TransactorOperatorReapplyCollection, TransactorOperatorReapplyState,
        run_transactor_operator_reapply,
    };

    #[test]
    fn transactor_operator_reapply_discards_fail_hard_tec_without_reset() {
        let discarded = Cell::new(false);
        let reset_called = Cell::new(false);

        let result = run_transactor_operator_reapply(
            Ter::TEC_CLAIM,
            true,
            10_i64,
            ApplyFlags::FAIL_HARD,
            || discarded.set(true),
            |_, _| {
                reset_called.set(true);
                (Ter::TES_SUCCESS, 0)
            },
            || panic!("fail-hard path should skip offer removal"),
            || panic!("fail-hard path should skip line removal"),
            || panic!("fail-hard path should skip nft offer removal"),
            || panic!("fail-hard path should skip credential removal"),
        );

        assert_eq!(
            result,
            TransactorOperatorReapplyState {
                result: Ter::TEC_CLAIM,
                applied: false,
                fee: 10,
            }
        );
        assert_eq!(trans_token(result.result), "tecCLAIM");
        assert!(discarded.get());
        assert!(!reset_called.get());
    }

    #[test]
    fn transactor_operator_reapply_reapplies_hard_fail_claims_without_collectors() {
        let reset_calls = RefCell::new(Vec::new());

        let result = run_transactor_operator_reapply(
            Ter::TEC_CLAIM,
            false,
            10_i64,
            ApplyFlags::NONE,
            || panic!("hard-fail-claim path should skip discard"),
            |collection, fee| {
                reset_calls.borrow_mut().push(collection);
                assert_eq!(fee, 10);
                (Ter::TEC_CLAIM, 12)
            },
            || panic!("plain claim reapply should skip offer removal"),
            || panic!("plain claim reapply should skip line removal"),
            || panic!("plain claim reapply should skip nft offer removal"),
            || panic!("plain claim reapply should skip credential removal"),
        );

        assert_eq!(
            result,
            TransactorOperatorReapplyState {
                result: Ter::TEC_CLAIM,
                applied: true,
                fee: 12,
            }
        );
        assert_eq!(
            reset_calls.into_inner(),
            vec![TransactorOperatorReapplyCollection {
                offers: false,
                trust_lines: false,
                nftoken_offers: false,
                credentials: false,
            }]
        );
    }

    #[test]
    fn transactor_operator_reapply_collects_and_removes_offers_for_oversize() {
        let offers_removed = Cell::new(false);

        let result = run_transactor_operator_reapply(
            Ter::TEC_OVERSIZE,
            true,
            15_i64,
            ApplyFlags::RETRY,
            || panic!("oversize path should skip fail-hard discard"),
            |collection, fee| {
                assert!(collection.offers);
                assert!(!collection.trust_lines);
                assert!(!collection.nftoken_offers);
                assert!(!collection.credentials);
                assert_eq!(fee, 15);
                (Ter::TEC_OVERSIZE, 18)
            },
            || offers_removed.set(true),
            || panic!("oversize path should skip trust-line removal"),
            || panic!("oversize path should skip nft offer removal"),
            || panic!("oversize path should skip credential removal"),
        );

        assert_eq!(
            result,
            TransactorOperatorReapplyState {
                result: Ter::TEC_OVERSIZE,
                applied: true,
                fee: 18,
            }
        );
        assert!(offers_removed.get());
    }

    #[test]
    fn transactor_operator_reapply_collects_and_removes_expired_objects() {
        let nft_removed = Cell::new(false);
        let credentials_removed = Cell::new(false);

        let result = run_transactor_operator_reapply(
            Ter::TEC_EXPIRED,
            true,
            20_i64,
            ApplyFlags::RETRY,
            || panic!("expired path should skip fail-hard discard"),
            |collection, fee| {
                assert!(!collection.offers);
                assert!(!collection.trust_lines);
                assert!(collection.nftoken_offers);
                assert!(collection.credentials);
                assert_eq!(fee, 20);
                (Ter::TEC_EXPIRED, 21)
            },
            || panic!("expired path should skip offer removal"),
            || panic!("expired path should skip trust-line removal"),
            || nft_removed.set(true),
            || credentials_removed.set(true),
        );

        assert_eq!(
            result,
            TransactorOperatorReapplyState {
                result: Ter::TEC_EXPIRED,
                applied: true,
                fee: 21,
            }
        );
        assert!(nft_removed.get());
        assert!(credentials_removed.get());
    }

    #[test]
    fn transactor_operator_reapply_collects_and_removes_lines_for_incomplete() {
        let lines_removed = Cell::new(false);

        let result = run_transactor_operator_reapply(
            Ter::TEC_INCOMPLETE,
            true,
            9_i64,
            ApplyFlags::RETRY,
            || panic!("incomplete path should skip fail-hard discard"),
            |collection, fee| {
                assert!(!collection.offers);
                assert!(collection.trust_lines);
                assert!(!collection.nftoken_offers);
                assert!(!collection.credentials);
                assert_eq!(fee, 9);
                (Ter::TEC_INCOMPLETE, 11)
            },
            || panic!("incomplete path should skip offer removal"),
            || lines_removed.set(true),
            || panic!("incomplete path should skip nft offer removal"),
            || panic!("incomplete path should skip credential removal"),
        );

        assert_eq!(
            result,
            TransactorOperatorReapplyState {
                result: Ter::TEC_INCOMPLETE,
                applied: true,
                fee: 11,
            }
        );
        assert!(lines_removed.get());
    }
}
