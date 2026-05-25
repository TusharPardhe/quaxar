use basics::base_uint::Uint256;
use std::cell::Cell;

use protocol::{
    Rules, SeqProxy, Ter, TxType, feature_single_asset_vault, get_current_transaction_rules,
    set_current_transaction_rules,
};
use tx::apply_steps_entrypoint::{
    run_preflight_for_txn_source_with_consequences, run_preflight_for_txn_type_with_consequences,
};
use tx::consequences::{TxConsequencesShape, build_tx_consequences};
use tx::{
    ApplyFlags, ApplyResult, HasTxnType, PreclaimResult, PreflightContext, PreflightResult,
    TxConsequences, UNKNOWN_TRANSACTION_TYPE_TER, UnknownTransactionType,
    run_calculate_base_fee_for_txn_source, run_calculate_base_fee_for_txn_type,
    run_calculate_base_fee_with_context, run_calculate_default_base_fee_with_context,
    run_do_apply_for_txn_source, run_do_apply_for_txn_type, run_do_apply_with_context,
    run_invoke_apply_for_txn_source, run_invoke_apply_for_txn_type,
    run_invoke_apply_result_for_txn_source, run_invoke_apply_result_for_txn_type,
    run_invoke_apply_result_with_context, run_invoke_apply_with_context,
    run_invoke_preclaim_for_txn_source, run_invoke_preclaim_for_txn_type,
    run_invoke_preclaim_with_context, run_invoke_preflight_for_txn_source,
    run_invoke_preflight_for_txn_source_with_consequences, run_invoke_preflight_for_txn_type,
    run_invoke_preflight_for_txn_type_with_consequences, run_invoke_preflight_with_consequences,
    run_invoke_preflight_with_context, run_preclaim_for_txn_source, run_preclaim_with_context,
    run_preflight_for_txn_source, run_preflight_with_context, run_with_txn_type_key,
    run_with_txn_type_source,
};

fn ledger_rules(seed: u8) -> Rules {
    Rules::from_ledger(
        [feature_single_asset_vault()],
        Uint256::from_array([seed; 32]),
        std::iter::empty(),
    )
}

fn sample_preflight(
    rules: Rules,
    flags: ApplyFlags,
    ter: Ter,
) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
    PreflightResult::new(
        "tx",
        ((flags & ApplyFlags::BATCH) == ApplyFlags::BATCH).then_some("batch"),
        rules,
        TxConsequences::new(12, SeqProxy::sequence(5)),
        flags,
        "journal",
        ter,
    )
}

fn sample_preclaim(
    ledger_seq: u32,
    flags: ApplyFlags,
    ter: Ter,
) -> PreclaimResult<&'static str, &'static str, &'static str> {
    PreclaimResult::new(
        ledger_seq,
        "tx",
        ((flags & ApplyFlags::BATCH) == ApplyFlags::BATCH).then_some("batch"),
        flags,
        "journal",
        ter,
    )
}

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
fn preflight_shell_preserves_successful_invoke_result() {
    let result = run_preflight_with_context(
        PreflightContext::<_, _, _, &str>::new(
            "registry",
            "tx",
            Rules::new(std::iter::empty()),
            ApplyFlags::RETRY,
            "journal",
        ),
        |_ctx| {
            Ok::<_, &str>((
                Ter::TES_SUCCESS,
                TxConsequences::new(9, SeqProxy::sequence(3)),
            ))
        },
        |_ctx| unreachable!("successful invoke should not use fallback"),
    );

    assert_eq!(result.ter, Ter::TES_SUCCESS);
    assert_eq!(result.consequences.fee(), 9);
    assert_eq!(result.flags, ApplyFlags::RETRY);
    assert_eq!(result.parent_batch_id, None);
}

#[test]
fn preflight_shell_maps_exception_to_tefexception_and_fallback_consequences() {
    let result = run_preflight_with_context(
        PreflightContext::new_batch(
            "registry",
            "tx",
            "batch",
            Rules::new(std::iter::empty()),
            ApplyFlags::BATCH,
            "journal",
        ),
        |_ctx| Err::<(Ter, TxConsequences), &str>("boom"),
        |_ctx| TxConsequences::new(10, SeqProxy::sequence(5)),
    );

    assert_eq!(result.ter, Ter::TEF_EXCEPTION);
    assert_eq!(result.consequences.fee(), 10);
    assert_eq!(result.parent_batch_id, Some("batch"));
}

#[test]
fn preclaim_shell_reflights_then_invokes_preclaim() {
    let old_rules = ledger_rules(0x11);
    let new_rules = ledger_rules(0x22);
    let result = run_preclaim_with_context(
        sample_preflight(old_rules, ApplyFlags::RETRY, Ter::TES_SUCCESS),
        "registry",
        "view",
        &new_rules,
        7,
        |_preflight, rules| {
            sample_preflight(rules.clone(), ApplyFlags::FAIL_HARD, Ter::TES_SUCCESS)
        },
        |ctx| {
            assert_eq!(ctx.flags, ApplyFlags::FAIL_HARD);
            assert_eq!(ctx.parent_batch_id, None);
            Ok::<_, &str>(Ter::TEC_CLAIM)
        },
    );

    assert_eq!(result.ledger_seq, 7);
    assert_eq!(result.flags, ApplyFlags::FAIL_HARD);
    assert_eq!(result.ter, Ter::TEC_CLAIM);
    assert!(result.likely_to_claim_fee);
}

#[test]
fn preclaim_shell_returns_failed_preflight_without_invoking_preclaim() {
    let result = run_preclaim_with_context(
        sample_preflight(
            Rules::new(std::iter::empty()),
            ApplyFlags::NONE,
            Ter::TER_RETRY,
        ),
        "registry",
        "view",
        &Rules::new(std::iter::empty()),
        9,
        |_preflight, _rules| unreachable!("matching rules should skip reflight"),
        |_ctx| -> Result<Ter, &str> {
            unreachable!("failed preflight should skip invoke_preclaim")
        },
    );

    assert_eq!(result.ledger_seq, 9);
    assert_eq!(result.ter, Ter::TER_RETRY);
    assert!(!result.likely_to_claim_fee);
}

#[test]
fn do_apply_shell_returns_tefexception_for_stale_ledger_seq() {
    let result = run_do_apply_with_context(
        sample_preclaim(9, ApplyFlags::NONE, Ter::TES_SUCCESS),
        "registry",
        10,
        "base",
        "view",
        |_base, _tx| unreachable!("stale ledger should skip fee calculation"),
        |_ctx| -> Result<ApplyResult, &str> {
            unreachable!("stale ledger should skip invoke_apply")
        },
    );

    assert_eq!(result, ApplyResult::new(Ter::TEF_EXCEPTION, false, false));
}

#[test]
fn do_apply_shell_returns_non_fee_claiming_result_without_invoking_apply() {
    let result = run_do_apply_with_context(
        sample_preclaim(9, ApplyFlags::NONE, Ter::TER_RETRY),
        "registry",
        9,
        "base",
        "view",
        |_base, _tx| unreachable!("non-fee-claiming path should skip fee calculation"),
        |_ctx| -> Result<ApplyResult, &str> {
            unreachable!("non-fee-claiming path should skip invoke_apply")
        },
    );

    assert_eq!(result, ApplyResult::new(Ter::TER_RETRY, false, false));
}

#[test]
fn do_apply_shell_builds_apply_context_then_invokes_apply() {
    let result = run_do_apply_with_context(
        sample_preclaim(9, ApplyFlags::BATCH, Ter::TES_SUCCESS),
        "registry",
        9,
        String::from("base"),
        vec![1_i32],
        |base, tx| {
            assert_eq!(base, "base");
            assert_eq!(tx, &"tx");
            12_u64
        },
        |ctx| {
            assert_eq!(ctx.registry, "registry");
            assert_eq!(ctx.tx, "tx");
            assert_eq!(ctx.preclaim_result, Ter::TES_SUCCESS);
            assert_eq!(ctx.base_fee, 12_u64);
            assert_eq!(ctx.flags(), ApplyFlags::BATCH);
            assert_eq!(ctx.parent_batch_id, Some("batch"));
            assert_eq!(ctx.base(), "base");
            assert_eq!(ctx.view(), &vec![1]);
            Ok::<_, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
        },
    );

    assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, true));
}

#[test]
fn do_apply_shell_maps_apply_exception_to_tefexception() {
    let result = run_do_apply_with_context(
        sample_preclaim(9, ApplyFlags::NONE, Ter::TES_SUCCESS),
        "registry",
        9,
        "base",
        "view",
        |_base, _tx| 10_u64,
        |_ctx| Err::<ApplyResult, &str>("boom"),
    );

    assert_eq!(result, ApplyResult::new(Ter::TEF_EXCEPTION, false, false));
}

#[test]
fn invoke_preflight_shell_enters_step_runtime_and_returns_dispatched_result() {
    set_current_transaction_rules(None);
    let rules = ledger_rules(0x33);

    let (ter, consequences) = run_invoke_preflight_with_context(&rules, || {
        assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
        Ok::<_, UnknownTransactionType<&str>>((
            Ter::TES_SUCCESS,
            TxConsequences::new(12, SeqProxy::sequence(4)),
        ))
    });

    assert_eq!(ter, Ter::TES_SUCCESS);
    assert_eq!(consequences, TxConsequences::new(12, SeqProxy::sequence(4)));
    assert_eq!(get_current_transaction_rules(), None);
}

#[test]
fn invoke_preflight_shell_maps_unknown_type_to_temunknown() {
    let result = run_invoke_preflight_with_context(&Rules::new(std::iter::empty()), || {
        Err::<(Ter, TxConsequences), _>(UnknownTransactionType::new("unknown"))
    });

    assert_eq!(
        result,
        (
            UNKNOWN_TRANSACTION_TYPE_TER,
            TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
        )
    );
}

#[test]
fn invoke_preflight_with_consequences_builds_success_consequences_only_on_success() {
    let consequences_called = Cell::new(false);

    let result = run_invoke_preflight_with_consequences(
        &Rules::new(std::iter::empty()),
        || Ok::<_, UnknownTransactionType<&str>>(Ter::TES_SUCCESS),
        || {
            consequences_called.set(true);
            TxConsequences::new(15, SeqProxy::sequence(8))
        },
    );

    assert_eq!(
        result,
        (
            Ter::TES_SUCCESS,
            TxConsequences::new(15, SeqProxy::sequence(8))
        )
    );
    assert!(consequences_called.get());
}

#[test]
fn invoke_preflight_with_consequences_keeps_failure_consequences_on_failed_preflight() {
    let consequences_called = Cell::new(false);

    let result = run_invoke_preflight_with_consequences(
        &Rules::new(std::iter::empty()),
        || Ok::<_, UnknownTransactionType<&str>>(Ter::TEL_BAD_DOMAIN),
        || {
            consequences_called.set(true);
            TxConsequences::new(15, SeqProxy::sequence(8))
        },
    );

    assert_eq!(
        result,
        (
            Ter::TEL_BAD_DOMAIN,
            TxConsequences::from_preflight_result(Ter::TEL_BAD_DOMAIN),
        )
    );
    assert!(!consequences_called.get());
}

#[test]
fn invoke_preflight_for_txn_type_with_consequences_preserves_dispatch_and_failure_mapping() {
    let result = run_invoke_preflight_for_txn_type_with_consequences(
        &Rules::new(std::iter::empty()),
        TxType::PAYMENT,
        |txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            Ter::TES_SUCCESS
        },
        |txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            TxConsequences::new(9, SeqProxy::sequence(3))
        },
    );

    assert_eq!(
        result,
        (
            Ter::TES_SUCCESS,
            TxConsequences::new(9, SeqProxy::sequence(3))
        )
    );

    let failure = run_invoke_preflight_for_txn_type_with_consequences(
        &Rules::new(std::iter::empty()),
        TxType::PAYMENT,
        |_txn_type| Ter::TEM_INVALID_FLAG,
        |_txn_type| unreachable!("failed preflight must not build success consequences"),
    );

    assert_eq!(
        failure,
        (
            Ter::TEM_INVALID_FLAG,
            TxConsequences::from_preflight_result(Ter::TEM_INVALID_FLAG),
        )
    );
}

#[test]
fn invoke_preflight_for_txn_source_with_consequences_maps_unknown_type_to_temunknown() {
    let tx = StubTxnSource {
        txn_type: TxType::HOOK_SET,
    };

    let result = run_invoke_preflight_for_txn_source_with_consequences(
        &Rules::new(std::iter::empty()),
        &tx,
        |_txn_type| unreachable!("hook set should not dispatch"),
        |_txn_type| unreachable!("hook set should not build success consequences"),
    );

    assert_eq!(
        result,
        (
            UNKNOWN_TRANSACTION_TYPE_TER,
            TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
        )
    );
}

#[test]
fn preflight_for_txn_source_with_consequences_uses_success_builder() {
    set_current_transaction_rules(None);
    let rules = ledger_rules(0x49);

    let result = run_preflight_for_txn_source_with_consequences(
        PreflightContext::<_, _, _, &str>::new(
            "registry",
            StubTxnSource {
                txn_type: TxType::PAYMENT,
            },
            rules.clone(),
            ApplyFlags::RETRY,
            "journal",
        ),
        |ctx, txn_type| {
            assert_eq!(ctx.tx.txn_type(), TxType::PAYMENT);
            assert_eq!(txn_type, TxType::PAYMENT);
            assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
            Ok::<_, &str>(Ter::TES_SUCCESS)
        },
        |_ctx, txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            build_tx_consequences(9, SeqProxy::sequence(3), TxConsequencesShape::Blocker)
        },
        |_ctx| unreachable!("successful dispatch should not use fallback consequences"),
    );

    assert_eq!(result.ter, Ter::TES_SUCCESS);
    assert!(result.consequences.is_blocker());
    assert_eq!(get_current_transaction_rules(), None);
}

#[test]
fn preflight_for_txn_source_with_consequences_maps_dispatch_failure_to_tefexception() {
    let result = run_preflight_for_txn_source_with_consequences(
        PreflightContext::<_, _, _, &str>::new(
            "registry",
            StubTxnSource {
                txn_type: TxType::PAYMENT,
            },
            Rules::new(std::iter::empty()),
            ApplyFlags::NONE,
            "journal",
        ),
        |_ctx, _txn_type| Err::<Ter, &str>("boom"),
        |_ctx, _txn_type| unreachable!("failed dispatch should not use success consequences"),
        |_ctx| TxConsequences::new(10, SeqProxy::sequence(5)),
    );

    assert_eq!(result.ter, Ter::TEF_EXCEPTION);
    assert_eq!(
        result.consequences,
        TxConsequences::new(10, SeqProxy::sequence(5))
    );
}

#[test]
fn preflight_for_txn_source_with_consequences_still_maps_unknown_type_to_temunknown() {
    let result = run_preflight_for_txn_source_with_consequences(
        PreflightContext::<_, _, _, &str>::new(
            "registry",
            StubTxnSource {
                txn_type: TxType::HOOK_SET,
            },
            Rules::new(std::iter::empty()),
            ApplyFlags::NONE,
            "journal",
        ),
        |_ctx, _txn_type| -> Result<Ter, &str> { unreachable!("unknown type should not dispatch") },
        |_ctx, _txn_type| unreachable!("unknown type should not build success consequences"),
        |_ctx| unreachable!("unknown type should not use fallback consequences"),
    );

    assert_eq!(result.ter, UNKNOWN_TRANSACTION_TYPE_TER);
    assert_eq!(
        result.consequences,
        TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER)
    );
}

#[test]
fn preflight_for_txn_type_with_consequences_dispatches_without_source_lookup() {
    let result = run_preflight_for_txn_type_with_consequences(
        PreflightContext::<_, _, _, &str>::new(
            "registry",
            "tx",
            Rules::new(std::iter::empty()),
            ApplyFlags::NONE,
            "journal",
        ),
        TxType::PAYMENT,
        |_ctx, txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            Ok::<_, &str>(Ter::TES_SUCCESS)
        },
        |_ctx, txn_type| {
            assert_eq!(txn_type, TxType::PAYMENT);
            build_tx_consequences(
                7,
                SeqProxy::sequence(4),
                TxConsequencesShape::SequencesConsumed(2),
            )
        },
        |_ctx| unreachable!("successful dispatch should not use fallback consequences"),
    );

    assert_eq!(result.ter, Ter::TES_SUCCESS);
    assert_eq!(result.consequences.sequences_consumed(), 2);
    assert_eq!(result.consequences.following_seq(), SeqProxy::sequence(6));
}

#[test]
fn preflight_for_txn_source_dispatches_known_payment_public_shell() {
    set_current_transaction_rules(None);
    let rules = ledger_rules(0x46);

    let result = run_preflight_for_txn_source(
        PreflightContext::<_, _, _, &str>::new(
            "registry",
            StubTxnSource {
                txn_type: TxType::PAYMENT,
            },
            rules.clone(),
            ApplyFlags::RETRY,
            "journal",
        ),
        |ctx, txn_type| {
            assert_eq!(ctx.tx.txn_type(), TxType::PAYMENT);
            assert_eq!(txn_type, TxType::PAYMENT);
            assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
            Ok::<_, &str>((
                Ter::TES_SUCCESS,
                TxConsequences::new(9, SeqProxy::sequence(3)),
            ))
        },
        |_ctx| unreachable!("successful dispatch should not use fallback"),
    );

    assert_eq!(result.ter, Ter::TES_SUCCESS);
    assert_eq!(
        result.consequences,
        TxConsequences::new(9, SeqProxy::sequence(3))
    );
    assert_eq!(get_current_transaction_rules(), None);
}

#[test]
fn preflight_for_txn_source_maps_hook_set_to_temunknown_public_shell() {
    let result = run_preflight_for_txn_source(
        PreflightContext::<_, _, _, &str>::new(
            "registry",
            StubTxnSource {
                txn_type: TxType::HOOK_SET,
            },
            Rules::new(std::iter::empty()),
            ApplyFlags::NONE,
            "journal",
        ),
        |_ctx, _txn_type| -> Result<(Ter, TxConsequences), &str> {
            unreachable!("hook set should not dispatch")
        },
        |_ctx| unreachable!("unknown transaction type should not use exception fallback"),
    );

    assert_eq!(result.ter, UNKNOWN_TRANSACTION_TYPE_TER);
    assert_eq!(
        result.consequences,
        TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER)
    );
}

#[test]
fn preclaim_for_txn_source_dispatches_known_payment_public_shell() {
    set_current_transaction_rules(None);
    let rules = ledger_rules(0x47);
    let result = run_preclaim_for_txn_source(
        PreflightResult::new(
            StubTxnSource {
                txn_type: TxType::PAYMENT,
            },
            None::<&str>,
            rules.clone(),
            TxConsequences::new(12, SeqProxy::sequence(5)),
            ApplyFlags::FAIL_HARD,
            "journal",
            Ter::TES_SUCCESS,
        ),
        "registry",
        "view",
        &rules,
        7,
        |_preflight, _rules| unreachable!("matching rules should skip reflight"),
        |ctx, txn_type| {
            assert_eq!(ctx.tx.txn_type(), TxType::PAYMENT);
            assert_eq!(txn_type, TxType::PAYMENT);
            assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
            Ok::<_, &str>(Ter::TEC_CLAIM)
        },
    );

    assert_eq!(result.ledger_seq, 7);
    assert_eq!(result.ter, Ter::TEC_CLAIM);
    assert!(result.likely_to_claim_fee);
    assert_eq!(get_current_transaction_rules(), None);
}

#[test]
fn preclaim_for_txn_source_maps_hook_set_to_temunknown_public_shell() {
    let result = run_preclaim_for_txn_source(
        PreflightResult::new(
            StubTxnSource {
                txn_type: TxType::HOOK_SET,
            },
            None::<&str>,
            Rules::new(std::iter::empty()),
            TxConsequences::new(12, SeqProxy::sequence(5)),
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        ),
        "registry",
        "view",
        &Rules::new(std::iter::empty()),
        9,
        |_preflight, _rules| unreachable!("matching rules should skip reflight"),
        |_ctx, _txn_type| -> Result<Ter, &str> { unreachable!("hook set should not dispatch") },
    );

    assert_eq!(result.ledger_seq, 9);
    assert_eq!(result.ter, UNKNOWN_TRANSACTION_TYPE_TER);
}

#[test]
fn do_apply_for_txn_source_dispatches_known_payment_public_shell() {
    set_current_transaction_rules(None);
    let rules = ledger_rules(0x48);
    let result = run_do_apply_for_txn_source(
        PreclaimResult::new(
            9,
            StubTxnSource {
                txn_type: TxType::PAYMENT,
            },
            Some("batch"),
            ApplyFlags::BATCH,
            "journal",
            Ter::TES_SUCCESS,
        ),
        &rules,
        "registry",
        9,
        String::from("base"),
        vec![1_i32],
        |base, tx, txn_type| {
            assert_eq!(base, "base");
            assert_eq!(tx.txn_type(), TxType::PAYMENT);
            assert_eq!(txn_type, TxType::PAYMENT);
            12_u64
        },
        || 0_u64,
        |ctx, txn_type| {
            assert_eq!(ctx.tx.txn_type(), TxType::PAYMENT);
            assert_eq!(txn_type, TxType::PAYMENT);
            assert_eq!(ctx.base_fee, 12_u64);
            assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
            Ok::<_, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, true))
        },
    );

    assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, true));
    assert_eq!(get_current_transaction_rules(), None);
}

#[test]
fn do_apply_for_txn_source_maps_hook_set_to_temunknown_public_shell() {
    let result = run_do_apply_for_txn_source(
        PreclaimResult::new(
            9,
            StubTxnSource {
                txn_type: TxType::HOOK_SET,
            },
            None::<&str>,
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        ),
        &Rules::new(std::iter::empty()),
        "registry",
        9,
        "base",
        "view",
        |_base, _tx, _txn_type| unreachable!("hook set should not dispatch"),
        || 0_u64,
        |_ctx, _txn_type| -> Result<ApplyResult, &str> {
            unreachable!("hook set should not dispatch")
        },
    );

    assert_eq!(
        result,
        ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)
    );
}

#[test]
fn do_apply_for_txn_type_dispatches_known_payment_public_shell() {
    set_current_transaction_rules(None);
    let rules = ledger_rules(0x4a);
    let result = run_do_apply_for_txn_type(
        PreclaimResult::new(
            9,
            "tx",
            None::<&str>,
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        ),
        &rules,
        TxType::PAYMENT,
        "registry",
        9,
        String::from("base"),
        vec![1_i32],
        |base, tx, txn_type| {
            assert_eq!(base, "base");
            assert_eq!(tx, &"tx");
            assert_eq!(txn_type, TxType::PAYMENT);
            12_u64
        },
        || 0_u64,
        |ctx, txn_type| {
            assert_eq!(ctx.registry, "registry");
            assert_eq!(ctx.tx, "tx");
            assert_eq!(txn_type, TxType::PAYMENT);
            assert_eq!(ctx.base_fee, 12_u64);
            assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
            Ok::<_, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, false))
        },
    );

    assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, false));
    assert_eq!(get_current_transaction_rules(), None);
}

#[test]
fn do_apply_for_txn_type_maps_hook_set_to_temunknown_public_shell() {
    let result = run_do_apply_for_txn_type(
        PreclaimResult::new(
            9,
            "tx",
            None::<&str>,
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        ),
        &Rules::new(std::iter::empty()),
        TxType::HOOK_SET,
        "registry",
        9,
        "base",
        "view",
        |_base, _tx, _txn_type| unreachable!("hook set should not calculate a base fee"),
        || 0_u64,
        |_ctx, _txn_type| -> Result<ApplyResult, &str> {
            unreachable!("hook set should not dispatch")
        },
    );

    assert_eq!(
        result,
        ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)
    );
}

#[test]
fn invoke_preclaim_shell_maps_unknown_type_to_temunknown() {
    let result = run_invoke_preclaim_with_context(&Rules::new(std::iter::empty()), || {
        Err::<Ter, _>(UnknownTransactionType::new("unknown"))
    });

    assert_eq!(result, UNKNOWN_TRANSACTION_TYPE_TER);
}

#[test]
fn invoke_apply_shell_maps_unknown_type_to_temunknown() {
    let result = run_invoke_apply_with_context(&Rules::new(std::iter::empty()), || {
        Err::<ApplyResult, _>(UnknownTransactionType::new("unknown"))
    });

    assert_eq!(
        result,
        ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)
    );
}

#[test]
fn invoke_apply_result_shell_maps_unknown_type_to_temunknown() {
    let result = run_invoke_apply_result_with_context(&Rules::new(std::iter::empty()), || {
        Err::<Result<ApplyResult, &str>, _>(UnknownTransactionType::new("unknown"))
    });

    assert_eq!(
        result,
        Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false))
    );
}

#[test]
fn txn_type_key_dispatches_known_payment_like_transactions_macro() {
    set_current_transaction_rules(None);
    let rules = ledger_rules(0x44);

    let observed = run_with_txn_type_key(&rules, TxType::PAYMENT, |txn_type| {
        assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
        txn_type
    });

    assert_eq!(observed, Ok(TxType::PAYMENT));
    assert_eq!(get_current_transaction_rules(), None);
}

#[test]
fn txn_type_key_rejects_protocol_only_hook_set_dispatch_gap() {
    let observed = run_with_txn_type_key(
        &Rules::new(std::iter::empty()),
        TxType::HOOK_SET,
        |_txn_type| unreachable!("hook set is not in transactions.macro"),
    );

    assert_eq!(observed, Err(UnknownTransactionType::new(TxType::HOOK_SET)));
}

#[test]
fn txn_type_source_dispatches_known_payment_like_transactions_macro() {
    set_current_transaction_rules(None);
    let rules = ledger_rules(0x45);
    let tx = StubTxnSource {
        txn_type: TxType::PAYMENT,
    };

    let observed = run_with_txn_type_source(&rules, &tx, |txn_type| {
        assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
        txn_type
    });

    assert_eq!(observed, Ok(TxType::PAYMENT));
    assert_eq!(get_current_transaction_rules(), None);
}

#[test]
fn invoke_preflight_for_txn_type_maps_hook_set_to_temunknown() {
    let observed = run_invoke_preflight_for_txn_type(
        &Rules::new(std::iter::empty()),
        TxType::HOOK_SET,
        |_txn_type| unreachable!("hook set should not dispatch"),
    );

    assert_eq!(
        observed,
        (
            UNKNOWN_TRANSACTION_TYPE_TER,
            TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
        )
    );
}

#[test]
fn invoke_preflight_for_txn_source_maps_hook_set_to_temunknown() {
    let tx = StubTxnSource {
        txn_type: TxType::HOOK_SET,
    };

    let observed =
        run_invoke_preflight_for_txn_source(&Rules::new(std::iter::empty()), &tx, |_txn_type| {
            unreachable!("hook set should not dispatch")
        });

    assert_eq!(
        observed,
        (
            UNKNOWN_TRANSACTION_TYPE_TER,
            TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER),
        )
    );
}

#[test]
fn invoke_preclaim_for_txn_source_maps_hook_set_to_temunknown() {
    let tx = StubTxnSource {
        txn_type: TxType::HOOK_SET,
    };

    let observed =
        run_invoke_preclaim_for_txn_source(&Rules::new(std::iter::empty()), &tx, |_txn_type| {
            unreachable!("hook set should not dispatch")
        });

    assert_eq!(observed, UNKNOWN_TRANSACTION_TYPE_TER);
}

#[test]
fn invoke_apply_for_txn_source_maps_hook_set_to_temunknown() {
    let tx = StubTxnSource {
        txn_type: TxType::HOOK_SET,
    };

    let observed =
        run_invoke_apply_for_txn_source(&Rules::new(std::iter::empty()), &tx, |_txn_type| {
            unreachable!("hook set should not dispatch")
        });

    assert_eq!(
        observed,
        ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)
    );
}

#[test]
fn invoke_preclaim_for_txn_type_maps_hook_set_to_temunknown() {
    let observed = run_invoke_preclaim_for_txn_type(
        &Rules::new(std::iter::empty()),
        TxType::HOOK_SET,
        |_txn_type| unreachable!("hook set should not dispatch"),
    );

    assert_eq!(observed, UNKNOWN_TRANSACTION_TYPE_TER);
}

#[test]
fn invoke_apply_for_txn_type_maps_hook_set_to_temunknown() {
    let observed = run_invoke_apply_for_txn_type(
        &Rules::new(std::iter::empty()),
        TxType::HOOK_SET,
        |_txn_type| unreachable!("hook set should not dispatch"),
    );

    assert_eq!(
        observed,
        ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)
    );
}

#[test]
fn invoke_apply_result_for_txn_type_maps_hook_set_to_temunknown() {
    let observed: Result<ApplyResult, &str> = run_invoke_apply_result_for_txn_type(
        &Rules::new(std::iter::empty()),
        TxType::HOOK_SET,
        |_txn_type| unreachable!("hook set should not dispatch"),
    );

    assert_eq!(
        observed,
        Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false))
    );
}

#[test]
fn invoke_apply_result_for_txn_source_maps_hook_set_to_temunknown() {
    let tx = StubTxnSource {
        txn_type: TxType::HOOK_SET,
    };

    let observed: Result<ApplyResult, &str> =
        run_invoke_apply_result_for_txn_source(&Rules::new(std::iter::empty()), &tx, |_txn_type| {
            unreachable!("hook set should not dispatch")
        });

    assert_eq!(
        observed,
        Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false))
    );
}

#[test]
fn calculate_base_fee_for_txn_type_maps_hook_set_to_zero() {
    let observed = run_calculate_base_fee_for_txn_type(
        &Rules::new(std::iter::empty()),
        TxType::HOOK_SET,
        |_txn_type| unreachable!("hook set should not dispatch"),
        || 0_u64,
    );

    assert_eq!(observed, 0_u64);
}

#[test]
fn calculate_base_fee_for_txn_source_maps_hook_set_to_zero() {
    let tx = StubTxnSource {
        txn_type: TxType::HOOK_SET,
    };

    let observed = run_calculate_base_fee_for_txn_source(
        &Rules::new(std::iter::empty()),
        &tx,
        |_txn_type| unreachable!("hook set should not dispatch"),
        || 0_u64,
    );

    assert_eq!(observed, 0_u64);
}

#[test]
fn calculate_base_fee_shell_returns_dispatched_fee() {
    let fee = run_calculate_base_fee_with_context(
        &Rules::new(std::iter::empty()),
        "view",
        "tx",
        |view, tx| {
            assert_eq!(view, "view");
            assert_eq!(tx, "tx");
            Ok::<_, UnknownTransactionType<&str>>(12_u64)
        },
        || 0_u64,
    );

    assert_eq!(fee, 12_u64);
}

#[test]
fn calculate_base_fee_shell_maps_unknown_txn_type_to_zero() {
    let fee = run_calculate_base_fee_with_context(
        &Rules::new(std::iter::empty()),
        "view",
        "tx",
        |_view, _tx| Err::<u64, _>(UnknownTransactionType::new("unknown")),
        || 0_u64,
    );

    assert_eq!(fee, 0_u64);
}

#[test]
fn calculate_default_base_fee_shell_delegates() {
    let fee = run_calculate_default_base_fee_with_context("view", "tx", |view, tx| {
        assert_eq!(view, "view");
        assert_eq!(tx, "tx");
        9_u64
    });

    assert_eq!(fee, 9_u64);
}
