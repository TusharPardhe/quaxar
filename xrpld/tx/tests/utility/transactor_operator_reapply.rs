//! Integration tests that pin the fail-hard/reapply policy slice of
//! `Transactor::operator()()` to the current C++ behavior.

use std::cell::Cell;

use protocol::{Ter, trans_token};
use tx::{
    ApplyFlags, TransactorOperatorReapplyCollection, TransactorOperatorReapplyState,
    run_transactor_operator_reapply,
};

#[test]
fn tx_transactor_operator_reapply_discards_fail_hard_tec_without_reset() {
    let discarded = Cell::new(false);

    let result = run_transactor_operator_reapply(
        Ter::TEC_CLAIM,
        true,
        10_i64,
        ApplyFlags::FAIL_HARD,
        || discarded.set(true),
        |_, _| panic!("fail-hard path should skip reapply"),
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
}

#[test]
fn tx_transactor_operator_reapply_reapplies_hard_fail_claims() {
    let result = run_transactor_operator_reapply(
        Ter::TEC_CLAIM,
        false,
        10_i64,
        ApplyFlags::NONE,
        || panic!("claim reapply should skip discard"),
        |collection, fee| {
            assert_eq!(
                collection,
                TransactorOperatorReapplyCollection {
                    offers: false,
                    trust_lines: false,
                    nftoken_offers: false,
                    credentials: false,
                }
            );
            assert_eq!(fee, 10);
            (Ter::TEC_CLAIM, 12)
        },
        || panic!("claim reapply should skip offer removal"),
        || panic!("claim reapply should skip line removal"),
        || panic!("claim reapply should skip nft offer removal"),
        || panic!("claim reapply should skip credential removal"),
    );

    assert_eq!(
        result,
        TransactorOperatorReapplyState {
            result: Ter::TEC_CLAIM,
            applied: true,
            fee: 12,
        }
    );
}

#[test]
fn tx_transactor_operator_reapply_removes_offers_for_oversize() {
    let removed = Cell::new(false);

    let result = run_transactor_operator_reapply(
        Ter::TEC_OVERSIZE,
        true,
        15_i64,
        ApplyFlags::RETRY,
        || panic!("oversize path should skip discard"),
        |collection, fee| {
            assert!(collection.offers);
            assert_eq!(fee, 15);
            (Ter::TEC_OVERSIZE, 18)
        },
        || removed.set(true),
        || panic!("oversize path should skip line removal"),
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
    assert!(removed.get());
}

#[test]
fn tx_transactor_operator_reapply_removes_lines_for_incomplete() {
    let removed = Cell::new(false);

    let result = run_transactor_operator_reapply(
        Ter::TEC_INCOMPLETE,
        true,
        9_i64,
        ApplyFlags::RETRY,
        || panic!("incomplete path should skip discard"),
        |collection, fee| {
            assert!(collection.trust_lines);
            assert_eq!(fee, 9);
            (Ter::TEC_INCOMPLETE, 11)
        },
        || panic!("incomplete path should skip offer removal"),
        || removed.set(true),
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
    assert!(removed.get());
}
