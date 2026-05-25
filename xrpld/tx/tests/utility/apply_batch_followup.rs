//! Integration tests that pin the narrowed batch follow-up `apply.cpp` seam to
//! the currently ported C++ behavior.

use std::cell::{Cell, RefCell};

use protocol::{BatchTransactionFlags, Rules, SeqProxy, Ter, TxType};
use tx::{ApplyFlags, ApplyResult, ApplyTransactionResult, HasTxnType, TxConsequences};

#[derive(Clone)]
struct TxnTypeStubSource {
    txn_type: TxType,
}

impl HasTxnType for TxnTypeStubSource {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn tx_apply_batch_followup_only_runs_for_successful_outer_batch() {
    assert!(tx::apply_entrypoint::should_run_batch_followup(
        &ApplyResult::new(Ter::TES_SUCCESS, true, true),
        TxType::BATCH
    ));
    assert!(!tx::apply_entrypoint::should_run_batch_followup(
        &ApplyResult::new(Ter::TEC_CLAIM, true, true),
        TxType::BATCH
    ));
    assert!(!tx::apply_entrypoint::should_run_batch_followup(
        &ApplyResult::new(Ter::TES_SUCCESS, true, true),
        TxType::PAYMENT
    ));
}

#[test]
fn tx_apply_batch_transactions_returns_false_for_all_or_nothing_failure() {
    let seen = RefCell::new(Vec::new());

    let applied = tx::apply_entrypoint::run_apply_batch_transactions(
        BatchTransactionFlags::ALL_OR_NOTHING,
        ["first", "second"],
        |inner| {
            seen.borrow_mut().push(inner);
            Ok::<_, ()>(if inner == "first" {
                ApplyResult::new(Ter::TEC_CLAIM, true, true)
            } else {
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            })
        },
    )
    .unwrap();

    assert!(!applied);
    assert_eq!(seen.into_inner(), vec!["first"]);
}

#[test]
fn tx_apply_batch_transactions_breaks_after_first_success_in_only_one_mode() {
    let seen = RefCell::new(Vec::new());

    let applied = tx::apply_entrypoint::run_apply_batch_transactions(
        BatchTransactionFlags::ONLY_ONE,
        ["first", "second"],
        |inner| {
            seen.borrow_mut().push(inner);
            Ok::<_, ()>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
        },
    )
    .unwrap();

    assert!(applied);
    assert_eq!(seen.into_inner(), vec!["first"]);
}

#[test]
fn tx_apply_batch_transactions_with_view_merge_applies_per_tx_batch_view_before_all_or_nothing_abort()
 {
    let seen = RefCell::new(Vec::new());
    let merged = RefCell::new(Vec::new());

    let applied = tx::apply_entrypoint::run_apply_batch_transactions_with_view_merge(
        BatchTransactionFlags::ALL_OR_NOTHING,
        ["first", "second"],
        |inner| {
            seen.borrow_mut().push(inner);
            Ok::<_, ()>(if inner == "first" {
                (ApplyResult::new(Ter::TEC_CLAIM, true, true), inner)
            } else {
                (ApplyResult::new(Ter::TES_SUCCESS, true, true), inner)
            })
        },
        |per_tx_batch_view| {
            merged.borrow_mut().push(per_tx_batch_view);
            Ok::<_, ()>(())
        },
    )
    .unwrap();

    assert!(!applied);
    assert_eq!(seen.into_inner(), vec!["first"]);
    assert_eq!(merged.into_inner(), vec!["first"]);
}

#[test]
fn tx_apply_transaction_with_batch_followup_applies_whole_batch_only_when_inner_batch_returns_true()
{
    let seen_flags = RefCell::new(Vec::new());
    let followup_calls = Cell::new(0_u32);
    let whole_batch_applies = Cell::new(0_u32);

    let result = tx::apply_entrypoint::run_apply_transaction_with_batch_followup(
        TxType::BATCH,
        true,
        ApplyFlags::FAIL_HARD,
        |flags| {
            seen_flags.borrow_mut().push(flags);
            Ok::<_, ()>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
        },
        || {
            followup_calls.set(followup_calls.get() + 1);
            Ok::<_, ()>(true)
        },
        || {
            whole_batch_applies.set(whole_batch_applies.get() + 1);
            Ok::<_, ()>(())
        },
    );

    assert_eq!(result, ApplyTransactionResult::Success);
    assert_eq!(
        seen_flags.into_inner(),
        vec![ApplyFlags::FAIL_HARD | ApplyFlags::RETRY]
    );
    assert_eq!(followup_calls.get(), 1);
    assert_eq!(whole_batch_applies.get(), 1);
}

#[test]
fn tx_apply_transaction_with_batch_followup_maps_followup_error_to_fail() {
    let result = tx::apply_entrypoint::run_apply_transaction_with_batch_followup(
        TxType::BATCH,
        false,
        ApplyFlags::NONE,
        |_flags| Ok::<_, ()>(ApplyResult::new(Ter::TES_SUCCESS, true, true)),
        || Err::<bool, &str>("boom"),
        || Ok::<_, &str>(()),
    );

    assert_eq!(result, ApplyTransactionResult::Fail);
}

#[test]
fn tx_apply_transaction_flow_with_batch_followup_for_txn_source_runs_inner_batch_then_applies_whole_batch()
 {
    let inner_applied = RefCell::new(Vec::new());
    let whole_batch_applies = Cell::new(0_u32);

    let result =
        tx::apply_entrypoint::run_apply_transaction_flow_with_batch_followup_for_txn_source(
            "registry",
            TxnTypeStubSource {
                txn_type: TxType::BATCH,
            },
            None::<&str>,
            &Rules::new(std::iter::empty()),
            false,
            ApplyFlags::NONE,
            9,
            "base",
            "view",
            "journal",
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                Ok::<_, &str>((
                    Ter::TES_SUCCESS,
                    TxConsequences::new(7, SeqProxy::sequence(1)),
                ))
            },
            |_ctx| unreachable!("successful dispatch should not use fallback"),
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                Ok::<_, &str>(Ter::TES_SUCCESS)
            },
            |_base, _tx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                11_u64
            },
            || 0_u64,
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                Ok::<_, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
            },
            BatchTransactionFlags::UNTIL_FAILURE,
            ["first", "second"],
            |inner| {
                inner_applied.borrow_mut().push(inner);
                Ok::<_, &str>(if inner == "first" {
                    ApplyResult::new(Ter::TES_SUCCESS, true, true)
                } else {
                    ApplyResult::new(Ter::TEC_CLAIM, true, true)
                })
            },
            || {
                whole_batch_applies.set(whole_batch_applies.get() + 1);
                Ok::<_, &str>(())
            },
        );

    assert_eq!(result, ApplyTransactionResult::Success);
    assert_eq!(inner_applied.into_inner(), vec!["first", "second"]);
    assert_eq!(whole_batch_applies.get(), 1);
}

#[test]
fn tx_apply_transaction_flow_with_batch_view_merge_for_txn_source_merges_each_applied_inner_view_then_whole_batch()
 {
    let inner_applied = RefCell::new(Vec::new());
    let merged_views = RefCell::new(Vec::new());
    let whole_batch_applies = Cell::new(0_u32);

    let result =
        tx::apply_entrypoint::run_apply_transaction_flow_with_batch_view_merge_for_txn_source(
            "registry",
            TxnTypeStubSource {
                txn_type: TxType::BATCH,
            },
            None::<&str>,
            &Rules::new(std::iter::empty()),
            false,
            ApplyFlags::NONE,
            9,
            "base",
            "view",
            "journal",
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                Ok::<_, &str>((
                    Ter::TES_SUCCESS,
                    TxConsequences::new(7, SeqProxy::sequence(1)),
                ))
            },
            |_ctx| unreachable!("successful dispatch should not use fallback"),
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                Ok::<_, &str>(Ter::TES_SUCCESS)
            },
            |_base, _tx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                11_u64
            },
            || 0_u64,
            |_ctx, txn_type| {
                assert_eq!(txn_type, TxType::BATCH);
                Ok::<_, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
            },
            BatchTransactionFlags::UNTIL_FAILURE,
            ["first", "second"],
            |inner| {
                inner_applied.borrow_mut().push(inner);
                Ok::<_, &str>(if inner == "first" {
                    (
                        ApplyResult::new(Ter::TES_SUCCESS, true, true),
                        "merge-first",
                    )
                } else {
                    (ApplyResult::new(Ter::TEC_CLAIM, true, true), "merge-second")
                })
            },
            |per_tx_batch_view| {
                merged_views.borrow_mut().push(per_tx_batch_view);
                Ok::<_, &str>(())
            },
            || {
                whole_batch_applies.set(whole_batch_applies.get() + 1);
                Ok::<_, &str>(())
            },
        );

    assert_eq!(result, ApplyTransactionResult::Success);
    assert_eq!(inner_applied.into_inner(), vec!["first", "second"]);
    assert_eq!(
        merged_views.into_inner(),
        vec!["merge-first", "merge-second"]
    );
    assert_eq!(whole_batch_applies.get(), 1);
}
