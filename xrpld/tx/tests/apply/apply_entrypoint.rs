use std::cell::RefCell;

use protocol::{BatchTransactionFlags, Rules, SeqProxy, Ter, TxType};
use tx::{
    ApplyFlags, ApplyResult, ApplyTransactionResult, HasTxnType, TxConsequences,
    apply_transaction_flags, classify_apply_transaction_result, run_apply,
    run_apply_for_txn_source, run_apply_for_txn_type, run_apply_transaction,
    run_apply_transaction_flow_for_txn_source, run_apply_transaction_flow_for_txn_type,
    run_apply_transaction_for_txn_source, run_apply_transaction_for_txn_type,
};

#[derive(Clone)]
struct StubTxnSource {
    txn_type: TxType,
}

impl HasTxnType for StubTxnSource {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn apply_entrypoint_runs_preflight_preclaim_then_apply_shell() {
    let events = RefCell::new(Vec::new());

    let result = run_apply(
        &Rules::new(std::iter::empty()),
        || {
            events.borrow_mut().push("preflight");
            "pf"
        },
        |preflight| {
            assert_eq!(preflight, "pf");
            events.borrow_mut().push("preclaim");
            "pc"
        },
        |preclaim| {
            assert_eq!(preclaim, "pc");
            events.borrow_mut().push("apply");
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        },
    );

    assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, true));
    assert_eq!(events.into_inner(), vec!["preflight", "preclaim", "apply"]);
}

#[test]
fn apply_transaction_retry_assured_adds_retry_flag_before_apply() {
    let seen_flags = RefCell::new(Vec::new());

    let result = run_apply_transaction(true, ApplyFlags::FAIL_HARD, |flags| {
        seen_flags.borrow_mut().push(flags);
        Ok::<_, ()>(ApplyResult::new(Ter::TER_RETRY, false, false))
    });

    assert_eq!(result, ApplyTransactionResult::Retry);
    assert_eq!(
        seen_flags.into_inner(),
        vec![apply_transaction_flags(ApplyFlags::FAIL_HARD, true)]
    );
}

#[test]
fn apply_transaction_classification_matches_current_cpp_categories() {
    assert_eq!(
        classify_apply_transaction_result(ApplyResult::new(Ter::TES_SUCCESS, true, true)),
        ApplyTransactionResult::Success
    );
    assert_eq!(
        classify_apply_transaction_result(ApplyResult::new(Ter::TEF_EXCEPTION, false, false)),
        ApplyTransactionResult::Fail
    );
    assert_eq!(
        classify_apply_transaction_result(ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE, false, false)),
        ApplyTransactionResult::Fail
    );
    assert_eq!(
        classify_apply_transaction_result(ApplyResult::new(Ter::TER_RETRY, false, false)),
        ApplyTransactionResult::Retry
    );
}

#[test]
fn batch_followup_only_runs_for_successful_outer_batch() {
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
fn apply_batch_transactions_returns_false_for_all_or_nothing_failure() {
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
fn apply_batch_transactions_breaks_after_first_success_in_only_one_mode() {
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
fn apply_batch_transactions_with_view_merge_applies_per_tx_batch_view_before_all_or_nothing_abort()
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
fn apply_transaction_for_txn_type_dispatches_known_payment_shell() {
    let seen_flags = RefCell::new(Vec::new());

    let result = run_apply_transaction_for_txn_type(
        TxType::PAYMENT,
        true,
        ApplyFlags::FAIL_HARD,
        |flags, txn_type| {
            seen_flags.borrow_mut().push((flags, txn_type));
            Ok::<_, ()>(ApplyResult::new(Ter::TER_RETRY, false, false))
        },
    );

    assert_eq!(result, ApplyTransactionResult::Retry);
    assert_eq!(
        seen_flags.into_inner(),
        vec![(
            apply_transaction_flags(ApplyFlags::FAIL_HARD, true),
            TxType::PAYMENT
        )]
    );
}

#[test]
fn apply_transaction_for_txn_type_maps_hook_set_to_fail() {
    let result = run_apply_transaction_for_txn_type(
        TxType::HOOK_SET,
        false,
        ApplyFlags::NONE,
        |_flags, _txn_type| -> Result<ApplyResult, ()> {
            unreachable!("hook set should not dispatch")
        },
    );

    assert_eq!(result, ApplyTransactionResult::Fail);
}

#[test]
fn apply_transaction_for_txn_source_dispatches_known_payment_shell() {
    let tx = StubTxnSource {
        txn_type: TxType::PAYMENT,
    };
    let seen_flags = RefCell::new(Vec::new());

    let result = run_apply_transaction_for_txn_source(
        &tx,
        true,
        ApplyFlags::FAIL_HARD,
        |flags, txn_type| {
            seen_flags.borrow_mut().push((flags, txn_type));
            Ok::<_, ()>(ApplyResult::new(Ter::TER_RETRY, false, false))
        },
    );

    assert_eq!(result, ApplyTransactionResult::Retry);
    assert_eq!(
        seen_flags.into_inner(),
        vec![(
            apply_transaction_flags(ApplyFlags::FAIL_HARD, true),
            TxType::PAYMENT
        )]
    );
}

#[test]
fn apply_transaction_for_txn_source_maps_hook_set_to_fail() {
    let tx = StubTxnSource {
        txn_type: TxType::HOOK_SET,
    };

    let result = run_apply_transaction_for_txn_source(
        &tx,
        false,
        ApplyFlags::NONE,
        |_flags, _txn_type| -> Result<ApplyResult, ()> {
            unreachable!("hook set should not dispatch")
        },
    );

    assert_eq!(result, ApplyTransactionResult::Fail);
}

#[test]
fn apply_transaction_with_batch_followup_applies_whole_batch_only_when_inner_batch_returns_true() {
    let seen_flags = RefCell::new(Vec::new());
    let followup_calls = std::cell::Cell::new(0_u32);
    let whole_batch_applies = std::cell::Cell::new(0_u32);

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
fn apply_transaction_with_batch_followup_maps_followup_error_to_fail() {
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
fn apply_for_txn_type_dispatches_known_payment_shell() {
    let result = run_apply_for_txn_type(
        "registry",
        StubTxnSource {
            txn_type: TxType::PAYMENT,
        },
        Some("batch"),
        &Rules::new(std::iter::empty()),
        ApplyFlags::BATCH,
        9,
        "base",
        vec![1_i32],
        "journal",
        TxType::PAYMENT,
        |_ctx, txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            Ok::<_, &str>((
                Ter::TES_SUCCESS,
                TxConsequences::new(10, SeqProxy::sequence(4)),
            ))
        },
        |_ctx| unreachable!("successful dispatch should not use fallback"),
        |_ctx, txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            Ok::<_, &str>(Ter::TES_SUCCESS)
        },
        |_base, tx, txn_type| {
            assert_eq!(tx.txn_type, TxType::PAYMENT);
            assert_eq!(txn_type, TxType::PAYMENT);
            12_u64
        },
        || 0_u64,
        |ctx, txn_type| {
            assert_eq!(ctx.parent_batch_id, Some("batch"));
            assert_eq!(ctx.base_fee, 12_u64);
            assert_eq!(txn_type, TxType::PAYMENT);
            Ok::<_, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
        },
    );

    assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, true));
}

#[test]
fn apply_for_txn_source_uses_current_rules_for_preflight_apply_shell() {
    let new_rules = Rules::from_ledger(
        [protocol::feature_single_asset_vault()],
        basics::base_uint::Uint256::from_array([0x62; 32]),
        std::iter::empty(),
    );
    let preflight_calls = std::cell::Cell::new(0_u32);

    let result = run_apply_for_txn_source(
        "registry",
        StubTxnSource {
            txn_type: TxType::PAYMENT,
        },
        None::<&str>,
        &new_rules,
        ApplyFlags::FAIL_HARD,
        9,
        "base",
        "view",
        "journal",
        |ctx, txn_type| {
            let next_call = preflight_calls.get() + 1;
            preflight_calls.set(next_call);
            assert_eq!(txn_type, TxType::PAYMENT);
            assert_eq!(ctx.rules, new_rules);
            Ok::<_, &str>((
                Ter::TES_SUCCESS,
                TxConsequences::new(8, SeqProxy::sequence(2)),
            ))
        },
        |_ctx| unreachable!("successful dispatch should not use fallback"),
        |_ctx, txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            Ok::<_, &str>(Ter::TES_SUCCESS)
        },
        |_base, _tx, txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            11_u64
        },
        || 0_u64,
        |ctx, txn_type| {
            assert_eq!(ctx.base_fee, 11_u64);
            assert_eq!(txn_type, TxType::PAYMENT);
            Ok::<_, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
        },
    );

    assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, true));
    assert_eq!(preflight_calls.get(), 1);
}

#[test]
fn apply_for_txn_source_maps_hook_set_to_temunknown_shell() {
    let result = run_apply_for_txn_source(
        "registry",
        StubTxnSource {
            txn_type: TxType::HOOK_SET,
        },
        None::<&str>,
        &Rules::new(std::iter::empty()),
        ApplyFlags::NONE,
        9,
        "base",
        "view",
        "journal",
        |_ctx, _txn_type| -> Result<(Ter, TxConsequences), &str> {
            unreachable!("hook set should not dispatch")
        },
        |_ctx| unreachable!("unknown type should not use exception fallback"),
        |_ctx, _txn_type| -> Result<Ter, &str> {
            unreachable!("failed preflight should skip preclaim dispatch")
        },
        |_base, _tx, _txn_type| unreachable!("failed preflight should skip fee calculation"),
        || 0_u64,
        |_ctx, _txn_type| -> Result<ApplyResult, &str> {
            unreachable!("failed preflight should skip apply dispatch")
        },
    );

    assert_eq!(
        result,
        ApplyResult::new(tx::UNKNOWN_TRANSACTION_TYPE_TER, false, false)
    );
}

#[test]
fn apply_transaction_flow_for_txn_type_classifies_success_shell() {
    let result = run_apply_transaction_flow_for_txn_type(
        "registry",
        StubTxnSource {
            txn_type: TxType::PAYMENT,
        },
        None::<&str>,
        &Rules::new(std::iter::empty()),
        true,
        ApplyFlags::FAIL_HARD,
        9,
        "base",
        "view",
        "journal",
        TxType::PAYMENT,
        |_ctx, txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            Ok::<_, &str>((
                Ter::TES_SUCCESS,
                TxConsequences::new(9, SeqProxy::sequence(3)),
            ))
        },
        |_ctx| unreachable!("successful dispatch should not use fallback"),
        |_ctx, txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            Ok::<_, &str>(Ter::TES_SUCCESS)
        },
        |_base, _tx, txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            12_u64
        },
        || 0_u64,
        |_ctx, txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            Ok::<_, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
        },
    );

    assert_eq!(result, ApplyTransactionResult::Success);
}

#[test]
fn apply_transaction_flow_for_txn_source_maps_hook_set_to_fail_shell() {
    let result = run_apply_transaction_flow_for_txn_source(
        "registry",
        StubTxnSource {
            txn_type: TxType::HOOK_SET,
        },
        None::<&str>,
        &Rules::new(std::iter::empty()),
        false,
        ApplyFlags::NONE,
        9,
        "base",
        "view",
        "journal",
        |_ctx, _txn_type| -> Result<(Ter, TxConsequences), &str> {
            unreachable!("hook set should not dispatch")
        },
        |_ctx| unreachable!("unknown type should not use exception fallback"),
        |_ctx, _txn_type| -> Result<Ter, &str> { unreachable!("hook set should not dispatch") },
        |_base, _tx, _txn_type| unreachable!("hook set should not dispatch"),
        || 0_u64,
        |_ctx, _txn_type| -> Result<ApplyResult, &str> {
            unreachable!("hook set should not dispatch")
        },
    );

    assert_eq!(result, ApplyTransactionResult::Fail);
}

#[test]
fn apply_transaction_flow_with_batch_followup_for_txn_source_runs_inner_batch_then_applies_whole_batch()
 {
    let inner_applied = RefCell::new(Vec::new());
    let whole_batch_applies = std::cell::Cell::new(0_u32);

    let result =
        tx::apply_entrypoint::run_apply_transaction_flow_with_batch_followup_for_txn_source(
            "registry",
            StubTxnSource {
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
fn apply_transaction_flow_with_batch_view_merge_for_txn_source_merges_each_applied_inner_view_then_whole_batch()
 {
    let inner_applied = RefCell::new(Vec::new());
    let merged_views = RefCell::new(Vec::new());
    let whole_batch_applies = std::cell::Cell::new(0_u32);

    let result =
        tx::apply_entrypoint::run_apply_transaction_flow_with_batch_view_merge_for_txn_source(
            "registry",
            StubTxnSource {
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
