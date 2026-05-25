use std::cell::Cell;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, MaybeTx, MaybeTxCore, PreclaimResult, PreflightResult,
    TryClearAccountResult, TryClearExecution, TryClearFinalization, TryClearSuccessCleanup,
    TxConsequences, TxQAccount, run_try_clear_account_queue_up_thru_tx,
    run_try_clear_account_queue_up_thru_tx_with_current_preclaim,
};

fn make_preflight(
    seq_proxy: SeqProxy,
) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
    PreflightResult::new(
        "tx",
        None,
        Rules::new(std::iter::empty()),
        TxConsequences::new(1, seq_proxy),
        ApplyFlags::NONE,
        "journal",
        Ter::TES_SUCCESS,
    )
}

fn make_queued(
    seq_proxy: SeqProxy,
    fee_level: u64,
) -> MaybeTx<&'static str, &'static str, &'static str, &'static str> {
    MaybeTx::new(
        Uint256::from_u64(seq_proxy.value() as u64),
        fee_level,
        "acct",
        Some(120),
        seq_proxy,
        ApplyFlags::NONE,
        make_preflight(seq_proxy),
    )
}

#[test]
fn try_clear_account_wrapper_returns_insufficient_fee_without_running_apply_closures() {
    let mut account = TxQAccount::new("acct");
    account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(5), 20),
            TxConsequences::new(1, SeqProxy::sequence(5)),
        ),
    );
    account.add(
        SeqProxy::sequence(6),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(6), 15),
            TxConsequences::new(1, SeqProxy::sequence(6)),
        ),
    );

    let mut queued_apply_calls = 0;
    let mut current_apply_calls = 0;

    let result = run_try_clear_account_queue_up_thru_tx(
        &mut account,
        SeqProxy::sequence(5),
        SeqProxy::sequence(8),
        10,
        |count| Some((count as u64) * 20),
        |_queued| {
            queued_apply_calls += 1;
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        },
        || {
            current_apply_calls += 1;
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        },
    );

    assert_eq!(
        result,
        TryClearAccountResult::InsufficientFee {
            plan: tx::TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5), SeqProxy::sequence(6)],
                queued_count: 2,
                target_was_already_queued: false,
                total_fee_level_paid: 45,
            },
            required_total_fee_level: 60,
        }
    );
    assert_eq!(
        result.apply_result(),
        ApplyResult::new(Ter::TEL_INSUF_FEE_P, false, false)
    );
    assert_eq!(queued_apply_calls, 0);
    assert_eq!(current_apply_calls, 0);
    assert_eq!(
        account.transactions[&SeqProxy::sequence(5)]
            .payload
            .retries_remaining,
        10
    );
    assert_eq!(
        account.transactions[&SeqProxy::sequence(6)]
            .payload
            .retries_remaining,
        10
    );
}

#[test]
fn try_clear_account_wrapper_short_circuits_current_apply_after_queued_failure() {
    let mut account = TxQAccount::new("acct");
    account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(5), 20),
            TxConsequences::new(1, SeqProxy::sequence(5)),
        ),
    );
    account.add(
        SeqProxy::sequence(6),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(6), 15),
            TxConsequences::new(1, SeqProxy::sequence(6)),
        ),
    );

    let mut current_apply_calls = 0;

    let result = run_try_clear_account_queue_up_thru_tx(
        &mut account,
        SeqProxy::sequence(5),
        SeqProxy::sequence(8),
        30,
        |count| Some((count as u64) * 20),
        |_queued| ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
        || {
            current_apply_calls += 1;
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        },
    );

    assert_eq!(
        result,
        TryClearAccountResult::ClearQueue {
            plan: tx::TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5), SeqProxy::sequence(6)],
                queued_count: 2,
                target_was_already_queued: false,
                total_fee_level_paid: 65,
            },
            required_total_fee_level: 60,
            execution: TryClearExecution::FallBackToNormal {
                failed_index: 0,
                result: ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
            },
        }
    );
    assert_eq!(current_apply_calls, 0);
    assert_eq!(
        result.apply_result(),
        ApplyResult::new(Ter::TEF_EXCEPTION, false, false)
    );
    assert_eq!(
        account.transactions[&SeqProxy::sequence(5)]
            .payload
            .retries_remaining,
        9
    );
    assert_eq!(
        account.transactions[&SeqProxy::sequence(5)]
            .payload
            .last_result,
        Some(Ter::TEF_EXCEPTION)
    );
    assert_eq!(
        account.transactions[&SeqProxy::sequence(6)]
            .payload
            .retries_remaining,
        10
    );
}

#[test]
fn try_clear_account_wrapper_cleans_up_only_after_current_success() {
    let mut account = TxQAccount::new("acct");
    account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(5), 20),
            TxConsequences::new(1, SeqProxy::sequence(5)),
        ),
    );
    account.add(
        SeqProxy::sequence(6),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(6), 15),
            TxConsequences::new(1, SeqProxy::sequence(6)),
        ),
    );
    account.add(
        SeqProxy::sequence(8),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(8), 25),
            TxConsequences::new(1, SeqProxy::sequence(8)),
        ),
    );
    account.add(
        SeqProxy::sequence(9),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(9), 10),
            TxConsequences::new(1, SeqProxy::sequence(9)),
        ),
    );

    let result = run_try_clear_account_queue_up_thru_tx(
        &mut account,
        SeqProxy::sequence(5),
        SeqProxy::sequence(8),
        30,
        |count| Some((count as u64) * 20),
        |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, true),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
    );

    assert_eq!(
        result,
        TryClearAccountResult::ClearQueue {
            plan: tx::TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5), SeqProxy::sequence(6)],
                queued_count: 2,
                target_was_already_queued: true,
                total_fee_level_paid: 65,
            },
            required_total_fee_level: 60,
            execution: TryClearExecution::CurrentTx(TryClearFinalization {
                current_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                cleanup: Some(TryClearSuccessCleanup {
                    removed_seq_proxies: vec![
                        SeqProxy::sequence(5),
                        SeqProxy::sequence(6),
                        SeqProxy::sequence(8),
                    ],
                    removed_replaced_target: true,
                    next_seq_proxy: Some(SeqProxy::sequence(9)),
                }),
            }),
        }
    );
    assert_eq!(
        result.apply_result(),
        ApplyResult::new(Ter::TES_SUCCESS, true, true)
    );
    assert_eq!(
        account.transactions.keys().copied().collect::<Vec<_>>(),
        vec![SeqProxy::sequence(9)]
    );
}

#[test]
fn try_clear_account_wrapper_keeps_queue_when_current_apply_fails() {
    let mut account = TxQAccount::new("acct");
    account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(5), 20),
            TxConsequences::new(1, SeqProxy::sequence(5)),
        ),
    );
    account.add(
        SeqProxy::sequence(8),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(8), 25),
            TxConsequences::new(1, SeqProxy::sequence(8)),
        ),
    );

    let result = run_try_clear_account_queue_up_thru_tx(
        &mut account,
        SeqProxy::sequence(5),
        SeqProxy::sequence(8),
        30,
        |count| Some((count as u64) * 20),
        |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, true),
        || ApplyResult::new(Ter::TER_RETRY, false, false),
    );

    assert_eq!(
        result,
        TryClearAccountResult::ClearQueue {
            plan: tx::TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5)],
                queued_count: 1,
                target_was_already_queued: true,
                total_fee_level_paid: 50,
            },
            required_total_fee_level: 40,
            execution: TryClearExecution::CurrentTx(TryClearFinalization {
                current_result: ApplyResult::new(Ter::TER_RETRY, false, false),
                cleanup: None,
            }),
        }
    );
    assert_eq!(
        account.transactions.keys().copied().collect::<Vec<_>>(),
        vec![SeqProxy::sequence(5), SeqProxy::sequence(8)]
    );
}

#[test]
fn try_clear_account_wrapper_with_current_preclaim_short_circuits_before_preclaim_or_apply() {
    let mut account = TxQAccount::new("acct");
    account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(5), 20),
            TxConsequences::new(1, SeqProxy::sequence(5)),
        ),
    );
    account.add(
        SeqProxy::sequence(6),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(6), 15),
            TxConsequences::new(1, SeqProxy::sequence(6)),
        ),
    );

    let preclaim_calls = Cell::new(0);
    let apply_calls = Cell::new(0);

    let result = run_try_clear_account_queue_up_thru_tx_with_current_preclaim(
        &mut account,
        SeqProxy::sequence(5),
        SeqProxy::sequence(8),
        30,
        |count| Some((count as u64) * 20),
        |_queued| ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
        || {
            preclaim_calls.set(preclaim_calls.get() + 1);
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        |_pcresult| {
            apply_calls.set(apply_calls.get() + 1);
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        },
    );

    assert_eq!(
        result,
        TryClearAccountResult::ClearQueue {
            plan: tx::TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5), SeqProxy::sequence(6)],
                queued_count: 2,
                target_was_already_queued: false,
                total_fee_level_paid: 65,
            },
            required_total_fee_level: 60,
            execution: TryClearExecution::FallBackToNormal {
                failed_index: 0,
                result: ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
            },
        }
    );
    assert_eq!(preclaim_calls.get(), 0);
    assert_eq!(apply_calls.get(), 0);
}

#[test]
fn try_clear_account_wrapper_with_current_preclaim_runs_preclaim_then_apply() {
    let mut account = TxQAccount::new("acct");
    account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(5), 20),
            TxConsequences::new(1, SeqProxy::sequence(5)),
        ),
    );
    account.add(
        SeqProxy::sequence(6),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(6), 15),
            TxConsequences::new(1, SeqProxy::sequence(6)),
        ),
    );
    account.add(
        SeqProxy::sequence(8),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(8), 25),
            TxConsequences::new(1, SeqProxy::sequence(8)),
        ),
    );
    account.add(
        SeqProxy::sequence(9),
        MaybeTxCore::new(
            make_queued(SeqProxy::sequence(9), 10),
            TxConsequences::new(1, SeqProxy::sequence(9)),
        ),
    );

    let preclaim_calls = Cell::new(0);
    let apply_calls = Cell::new(0);

    let result = run_try_clear_account_queue_up_thru_tx_with_current_preclaim(
        &mut account,
        SeqProxy::sequence(5),
        SeqProxy::sequence(8),
        30,
        |count| Some((count as u64) * 20),
        |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, true),
        || {
            preclaim_calls.set(preclaim_calls.get() + 1);
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        |pcresult| {
            apply_calls.set(apply_calls.get() + 1);
            assert_eq!(pcresult.ter, Ter::TES_SUCCESS);
            assert_eq!(preclaim_calls.get(), 1);
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        },
    );

    assert_eq!(preclaim_calls.get(), 1);
    assert_eq!(apply_calls.get(), 1);
    assert_eq!(
        result,
        TryClearAccountResult::ClearQueue {
            plan: tx::TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5), SeqProxy::sequence(6)],
                queued_count: 2,
                target_was_already_queued: true,
                total_fee_level_paid: 65,
            },
            required_total_fee_level: 60,
            execution: TryClearExecution::CurrentTx(TryClearFinalization {
                current_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                cleanup: Some(TryClearSuccessCleanup {
                    removed_seq_proxies: vec![
                        SeqProxy::sequence(5),
                        SeqProxy::sequence(6),
                        SeqProxy::sequence(8),
                    ],
                    removed_replaced_target: true,
                    next_seq_proxy: Some(SeqProxy::sequence(9)),
                }),
            }),
        }
    );
}
