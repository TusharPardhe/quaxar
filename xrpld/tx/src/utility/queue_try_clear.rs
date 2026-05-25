//! Deterministic front half of `TxQ::tryClearAccountQueueUpThruTx(...)`.
//!
//! This now ports:
//! - the account-range selection and fee-gate decision,
//! - the mutating queued `MaybeTx::apply(...)` loop shape,
//! - the success cleanup after the current transaction applies.

use protocol::{SeqProxy, Ter};

use crate::{
    ApplyResult, FeeLevel64, MaybeTx, MaybeTxCore, PreclaimResult, TxQAccount, erase_account_range,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TryClearAccountPlan {
    pub queued_seq_proxies: Vec<SeqProxy>,
    pub queued_count: usize,
    pub target_was_already_queued: bool,
    pub total_fee_level_paid: FeeLevel64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryClearAccountFeeGate {
    RequiredFeeOverflow(TryClearAccountPlan),
    InsufficientFee {
        plan: TryClearAccountPlan,
        required_total_fee_level: FeeLevel64,
    },
    ClearQueue {
        plan: TryClearAccountPlan,
        required_total_fee_level: FeeLevel64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryClearQueuedBatchProgress {
    ContinueToCurrentTx,
    FallBackToNormal {
        failed_index: usize,
        result: ApplyResult,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TryClearSuccessCleanup {
    pub removed_seq_proxies: Vec<SeqProxy>,
    pub removed_replaced_target: bool,
    pub next_seq_proxy: Option<SeqProxy>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TryClearFinalization {
    pub current_result: ApplyResult,
    pub cleanup: Option<TryClearSuccessCleanup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryClearExecution {
    FallBackToNormal {
        failed_index: usize,
        result: ApplyResult,
    },
    CurrentTx(TryClearFinalization),
}

impl TryClearExecution {
    pub fn apply_result(&self) -> ApplyResult {
        match self {
            Self::FallBackToNormal { result, .. } => result.clone(),
            Self::CurrentTx(finalization) => finalization.current_result.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryClearAccountResult {
    RequiredFeeOverflow(TryClearAccountPlan),
    InsufficientFee {
        plan: TryClearAccountPlan,
        required_total_fee_level: FeeLevel64,
    },
    ClearQueue {
        plan: TryClearAccountPlan,
        required_total_fee_level: FeeLevel64,
        execution: TryClearExecution,
    },
}

impl TryClearAccountResult {
    pub fn apply_result(&self) -> ApplyResult {
        match self {
            Self::RequiredFeeOverflow(_) | Self::InsufficientFee { .. } => {
                ApplyResult::new(Ter::TEL_INSUF_FEE_P, false, false)
            }
            Self::ClearQueue { execution, .. } => execution.apply_result(),
        }
    }
}

pub fn evaluate_try_clear_account_fee_gate<Account, T, FeeLevelOf, RequiredTotalFeeLevel>(
    account: &TxQAccount<Account, T>,
    begin: SeqProxy,
    target: SeqProxy,
    fee_level_paid: FeeLevel64,
    mut fee_level_of: FeeLevelOf,
    required_total_fee_level: RequiredTotalFeeLevel,
) -> TryClearAccountFeeGate
where
    FeeLevelOf: FnMut(&MaybeTxCore<T>) -> FeeLevel64,
    RequiredTotalFeeLevel: FnOnce(usize) -> Option<FeeLevel64>,
{
    assert!(
        account.transactions.contains_key(&begin),
        "xrpl::TxQ::tryClearAccountQueueUpThruTx : non-empty accounts input"
    );

    let queued_seq_proxies = account
        .transactions
        .range(begin..)
        .take_while(|(seq_proxy, _)| **seq_proxy < target)
        .map(|(seq_proxy, _)| *seq_proxy)
        .collect::<Vec<_>>();

    let total_fee_level_paid =
        queued_seq_proxies
            .iter()
            .fold(fee_level_paid, |total, seq_proxy| {
                let queued = account
                    .transactions
                    .get(seq_proxy)
                    .expect("queued transaction in collected clear range must exist");
                total.saturating_add(fee_level_of(queued))
            });

    let plan = TryClearAccountPlan {
        queued_count: queued_seq_proxies.len(),
        target_was_already_queued: account.transactions.contains_key(&target),
        total_fee_level_paid,
        queued_seq_proxies,
    };

    let Some(required_total_fee_level) = required_total_fee_level(plan.queued_count + 1) else {
        return TryClearAccountFeeGate::RequiredFeeOverflow(plan);
    };

    if plan.total_fee_level_paid < required_total_fee_level {
        return TryClearAccountFeeGate::InsufficientFee {
            plan,
            required_total_fee_level,
        };
    }

    TryClearAccountFeeGate::ClearQueue {
        plan,
        required_total_fee_level,
    }
}

pub fn process_try_clear_queued_results<Tx, Account, Journal, ParentBatchId, I>(
    queued: &mut [MaybeTx<Tx, Account, Journal, ParentBatchId>],
    results: I,
) -> TryClearQueuedBatchProgress
where
    I: IntoIterator<Item = ApplyResult>,
{
    let mut results = results.into_iter();

    let progress = run_try_clear_queued_batch(queued, |_| {
        results.next().expect(
            "xrpl::TxQ::tryClearAccountQueueUpThruTx : queued result count matches applied range",
        )
    });

    if progress == TryClearQueuedBatchProgress::ContinueToCurrentTx {
        assert!(
            results.next().is_none(),
            "xrpl::TxQ::tryClearAccountQueueUpThruTx : no extra queued results"
        );
    }

    progress
}

pub fn run_try_clear_queued_batch<Tx, Account, Journal, ParentBatchId, ApplyQueued>(
    queued: &mut [MaybeTx<Tx, Account, Journal, ParentBatchId>],
    mut apply_queued: ApplyQueued,
) -> TryClearQueuedBatchProgress
where
    ApplyQueued: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    for (index, queued_tx) in queued.iter_mut().enumerate() {
        let result = apply_queued(queued_tx);
        queued_tx.record_apply_attempt_result(&result);

        if result.ter == Ter::TEF_NO_TICKET {
            continue;
        }

        if !result.applied {
            return TryClearQueuedBatchProgress::FallBackToNormal {
                failed_index: index,
                result,
            };
        }
    }

    TryClearQueuedBatchProgress::ContinueToCurrentTx
}

pub fn cleanup_after_try_clear_success<Account, T>(
    account: &mut TxQAccount<Account, T>,
    begin: SeqProxy,
    target: SeqProxy,
) -> TryClearSuccessCleanup {
    let mut range_cleanup = erase_account_range(account, begin, Some(target));
    let mut removed_seq_proxies = range_cleanup.removed_seq_proxies;
    let mut removed_replaced_target = false;

    if range_cleanup.next_seq_proxy == Some(target) {
        let removed = account.remove(target);
        assert!(
            removed,
            "xrpl::TxQ::tryClearAccountQueueUpThruTx : replacement entry removed"
        );
        removed_replaced_target = true;
        removed_seq_proxies.push(target);
        range_cleanup.next_seq_proxy = account
            .transactions
            .range(target..)
            .next()
            .map(|(seq, _)| *seq);
    }

    TryClearSuccessCleanup {
        removed_seq_proxies,
        removed_replaced_target,
        next_seq_proxy: range_cleanup.next_seq_proxy,
    }
}

pub fn finalize_try_clear_result<Account, T>(
    account: &mut TxQAccount<Account, T>,
    begin: SeqProxy,
    target: SeqProxy,
    current_result: ApplyResult,
) -> TryClearFinalization {
    let cleanup = current_result
        .applied
        .then(|| cleanup_after_try_clear_success(account, begin, target));

    TryClearFinalization {
        current_result,
        cleanup,
    }
}

pub fn run_try_clear_current_tx_after_batch<Account, T, RunCurrentApply>(
    account: &mut TxQAccount<Account, T>,
    begin: SeqProxy,
    target: SeqProxy,
    progress: TryClearQueuedBatchProgress,
    run_current_apply: RunCurrentApply,
) -> TryClearExecution
where
    RunCurrentApply: FnOnce() -> ApplyResult,
{
    match progress {
        TryClearQueuedBatchProgress::ContinueToCurrentTx => TryClearExecution::CurrentTx(
            finalize_try_clear_result(account, begin, target, run_current_apply()),
        ),
        TryClearQueuedBatchProgress::FallBackToNormal {
            failed_index,
            result,
        } => TryClearExecution::FallBackToNormal {
            failed_index,
            result,
        },
    }
}

pub fn run_try_clear_current_tx_after_batch_with_preclaim<
    Account,
    T,
    Tx,
    Journal,
    ParentBatchId,
    RunPreclaim,
    RunDoApply,
>(
    account: &mut TxQAccount<Account, T>,
    begin: SeqProxy,
    target: SeqProxy,
    progress: TryClearQueuedBatchProgress,
    run_preclaim: RunPreclaim,
    run_do_apply: RunDoApply,
) -> TryClearExecution
where
    RunPreclaim: FnOnce() -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunDoApply: FnOnce(PreclaimResult<Tx, Journal, ParentBatchId>) -> ApplyResult,
{
    match progress {
        TryClearQueuedBatchProgress::ContinueToCurrentTx => {
            let pcresult = run_preclaim();
            TryClearExecution::CurrentTx(finalize_try_clear_result(
                account,
                begin,
                target,
                run_do_apply(pcresult),
            ))
        }
        TryClearQueuedBatchProgress::FallBackToNormal {
            failed_index,
            result,
        } => TryClearExecution::FallBackToNormal {
            failed_index,
            result,
        },
    }
}

pub fn run_try_clear_account_queue_up_thru_tx<
    Tx,
    Account,
    Journal,
    ParentBatchId,
    RequiredTotalFeeLevel,
    ApplyQueued,
    RunCurrentApply,
>(
    account: &mut TxQAccount<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    begin: SeqProxy,
    target: SeqProxy,
    fee_level_paid: FeeLevel64,
    required_total_fee_level: RequiredTotalFeeLevel,
    apply_queued: ApplyQueued,
    run_current_apply: RunCurrentApply,
) -> TryClearAccountResult
where
    RequiredTotalFeeLevel: FnOnce(usize) -> Option<FeeLevel64>,
    ApplyQueued: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
    RunCurrentApply: FnOnce() -> ApplyResult,
{
    match evaluate_try_clear_account_fee_gate(
        account,
        begin,
        target,
        fee_level_paid,
        |queued| queued.payload.fee_level,
        required_total_fee_level,
    ) {
        TryClearAccountFeeGate::RequiredFeeOverflow(plan) => {
            TryClearAccountResult::RequiredFeeOverflow(plan)
        }
        TryClearAccountFeeGate::InsufficientFee {
            plan,
            required_total_fee_level,
        } => TryClearAccountResult::InsufficientFee {
            plan,
            required_total_fee_level,
        },
        TryClearAccountFeeGate::ClearQueue {
            plan,
            required_total_fee_level,
        } => {
            let progress =
                run_try_clear_queued_account_plan(account, &plan.queued_seq_proxies, apply_queued);
            let execution = run_try_clear_current_tx_after_batch(
                account,
                begin,
                target,
                progress,
                run_current_apply,
            );

            TryClearAccountResult::ClearQueue {
                plan,
                required_total_fee_level,
                execution,
            }
        }
    }
}

pub fn run_try_clear_account_queue_up_thru_tx_with_current_preclaim<
    Tx,
    Account,
    Journal,
    ParentBatchId,
    RequiredTotalFeeLevel,
    ApplyQueued,
    RunPreclaim,
    RunDoApply,
>(
    account: &mut TxQAccount<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    begin: SeqProxy,
    target: SeqProxy,
    fee_level_paid: FeeLevel64,
    required_total_fee_level: RequiredTotalFeeLevel,
    apply_queued: ApplyQueued,
    run_preclaim: RunPreclaim,
    run_do_apply: RunDoApply,
) -> TryClearAccountResult
where
    RequiredTotalFeeLevel: FnOnce(usize) -> Option<FeeLevel64>,
    ApplyQueued: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
    RunPreclaim: FnOnce() -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunDoApply: FnOnce(PreclaimResult<Tx, Journal, ParentBatchId>) -> ApplyResult,
{
    match evaluate_try_clear_account_fee_gate(
        account,
        begin,
        target,
        fee_level_paid,
        |queued| queued.payload.fee_level,
        required_total_fee_level,
    ) {
        TryClearAccountFeeGate::RequiredFeeOverflow(plan) => {
            TryClearAccountResult::RequiredFeeOverflow(plan)
        }
        TryClearAccountFeeGate::InsufficientFee {
            plan,
            required_total_fee_level,
        } => TryClearAccountResult::InsufficientFee {
            plan,
            required_total_fee_level,
        },
        TryClearAccountFeeGate::ClearQueue {
            plan,
            required_total_fee_level,
        } => {
            let progress =
                run_try_clear_queued_account_plan(account, &plan.queued_seq_proxies, apply_queued);
            let execution = run_try_clear_current_tx_after_batch_with_preclaim(
                account,
                begin,
                target,
                progress,
                run_preclaim,
                run_do_apply,
            );

            TryClearAccountResult::ClearQueue {
                plan,
                required_total_fee_level,
                execution,
            }
        }
    }
}

fn run_try_clear_queued_account_plan<Tx, Account, Journal, ParentBatchId, ApplyQueued>(
    account: &mut TxQAccount<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    queued_seq_proxies: &[SeqProxy],
    mut apply_queued: ApplyQueued,
) -> TryClearQueuedBatchProgress
where
    ApplyQueued: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    for (index, seq_proxy) in queued_seq_proxies.iter().copied().enumerate() {
        let queued_tx = &mut account
            .transactions
            .get_mut(&seq_proxy)
            .expect("queued transaction in collected clear range must exist")
            .payload;

        let result = apply_queued(queued_tx);
        queued_tx.record_apply_attempt_result(&result);

        if result.ter == Ter::TEF_NO_TICKET {
            continue;
        }

        if !result.applied {
            return TryClearQueuedBatchProgress::FallBackToNormal {
                failed_index: index,
                result,
            };
        }
    }

    TryClearQueuedBatchProgress::ContinueToCurrentTx
}

#[cfg(test)]
mod tests {
    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{
        TryClearAccountFeeGate, TryClearAccountPlan, TryClearExecution, TryClearFinalization,
        TryClearQueuedBatchProgress, TryClearSuccessCleanup, cleanup_after_try_clear_success,
        evaluate_try_clear_account_fee_gate, finalize_try_clear_result,
        process_try_clear_queued_results, run_try_clear_current_tx_after_batch,
        run_try_clear_queued_batch,
    };
    use crate::{
        ApplyFlags, ApplyResult, MaybeTx, MaybeTxCore, PreflightResult, TxConsequences, TxQAccount,
    };

    fn make_queued(
        seq_proxy: SeqProxy,
    ) -> MaybeTx<&'static str, &'static str, &'static str, &'static str> {
        MaybeTx::new(
            Uint256::from_u64(seq_proxy.value() as u64),
            55_u64,
            "acct",
            Some(120),
            seq_proxy,
            ApplyFlags::NONE,
            PreflightResult::new(
                "tx",
                None::<&str>,
                Rules::new(std::iter::empty()),
                TxConsequences::new(1, seq_proxy),
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            ),
        )
    }

    #[test]
    fn try_clear_fee_gate_collects_account_range_and_allows_clear_when_paid_enough() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(20_u64, TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        account.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new(15_u64, TxConsequences::new(1, SeqProxy::sequence(6))),
        );
        account.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new(30_u64, TxConsequences::new(1, SeqProxy::sequence(8))),
        );

        let result = evaluate_try_clear_account_fee_gate(
            &account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(8),
            50,
            |queued| queued.payload,
            |count| Some((count as u64) * 25),
        );

        assert_eq!(
            result,
            TryClearAccountFeeGate::ClearQueue {
                plan: TryClearAccountPlan {
                    queued_seq_proxies: vec![SeqProxy::sequence(5), SeqProxy::sequence(6)],
                    queued_count: 2,
                    target_was_already_queued: true,
                    total_fee_level_paid: 85,
                },
                required_total_fee_level: 75,
            }
        );
    }

    #[test]
    fn try_clear_fee_gate_falls_back_when_paid_total_is_too_small() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(20_u64, TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        account.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new(15_u64, TxConsequences::new(1, SeqProxy::sequence(6))),
        );

        let result = evaluate_try_clear_account_fee_gate(
            &account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(9),
            10,
            |queued| queued.payload,
            |count| Some((count as u64) * 20),
        );

        assert_eq!(
            result,
            TryClearAccountFeeGate::InsufficientFee {
                plan: TryClearAccountPlan {
                    queued_seq_proxies: vec![SeqProxy::sequence(5), SeqProxy::sequence(6)],
                    queued_count: 2,
                    target_was_already_queued: false,
                    total_fee_level_paid: 45,
                },
                required_total_fee_level: 60,
            }
        );
    }

    #[test]
    fn try_clear_fee_gate_reports_required_fee_overflow_without_claiming_clear() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(20_u64, TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let result = evaluate_try_clear_account_fee_gate(
            &account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(7),
            10,
            |queued| queued.payload,
            |_| None,
        );

        assert_eq!(
            result,
            TryClearAccountFeeGate::RequiredFeeOverflow(TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5)],
                queued_count: 1,
                target_was_already_queued: false,
                total_fee_level_paid: 30,
            })
        );
    }

    #[test]
    fn try_clear_fee_gate_saturates_paid_total_like_a_safe_u64_boundary() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(u64::MAX, TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let result = evaluate_try_clear_account_fee_gate(
            &account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(7),
            1,
            |queued| queued.payload,
            |_| Some(u64::MAX),
        );

        assert_eq!(
            result,
            TryClearAccountFeeGate::ClearQueue {
                plan: TryClearAccountPlan {
                    queued_seq_proxies: vec![SeqProxy::sequence(5)],
                    queued_count: 1,
                    target_was_already_queued: false,
                    total_fee_level_paid: u64::MAX,
                },
                required_total_fee_level: u64::MAX,
            }
        );
    }

    #[test]
    fn process_try_clear_queued_results_updates_retry_state_and_continues_past_no_ticket() {
        let mut queued = vec![
            make_queued(SeqProxy::ticket(2)),
            make_queued(SeqProxy::sequence(5)),
        ];

        let progress = process_try_clear_queued_results(
            &mut queued,
            [
                ApplyResult::new(Ter::TEF_NO_TICKET, false, false),
                ApplyResult::new(Ter::TES_SUCCESS, true, true),
            ],
        );

        assert_eq!(progress, TryClearQueuedBatchProgress::ContinueToCurrentTx);
        assert_eq!(queued[0].retries_remaining, 9);
        assert_eq!(queued[1].retries_remaining, 9);
        assert_eq!(queued[0].last_result, Some(Ter::TEF_NO_TICKET));
        assert_eq!(queued[1].last_result, Some(Ter::TES_SUCCESS));
    }

    #[test]
    fn process_try_clear_queued_results_falls_back_on_first_non_ticket_unapplied_result() {
        let mut queued = vec![
            make_queued(SeqProxy::sequence(5)),
            make_queued(SeqProxy::sequence(6)),
        ];

        let progress = process_try_clear_queued_results(
            &mut queued,
            [
                ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
                ApplyResult::new(Ter::TES_SUCCESS, true, true),
            ],
        );

        assert_eq!(
            progress,
            TryClearQueuedBatchProgress::FallBackToNormal {
                failed_index: 0,
                result: ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
            }
        );
        assert_eq!(queued[0].retries_remaining, 9);
        assert_eq!(queued[0].last_result, Some(Ter::TEF_EXCEPTION));
        assert_eq!(queued[1].retries_remaining, 10);
        assert_eq!(queued[1].last_result, None);
    }

    #[test]
    fn run_try_clear_queued_batch_updates_retry_state_and_continues_past_no_ticket() {
        let mut queued = vec![
            make_queued(SeqProxy::ticket(2)),
            make_queued(SeqProxy::sequence(5)),
        ];
        let mut calls = 0;

        let progress = run_try_clear_queued_batch(&mut queued, |_queued| {
            let result = match calls {
                0 => ApplyResult::new(Ter::TEF_NO_TICKET, false, false),
                1 => ApplyResult::new(Ter::TES_SUCCESS, true, true),
                _ => unreachable!("only two queued txs should be attempted"),
            };
            calls += 1;
            result
        });

        assert_eq!(progress, TryClearQueuedBatchProgress::ContinueToCurrentTx);
        assert_eq!(calls, 2);
        assert_eq!(queued[0].retries_remaining, 9);
        assert_eq!(queued[1].retries_remaining, 9);
        assert_eq!(queued[0].last_result, Some(Ter::TEF_NO_TICKET));
        assert_eq!(queued[1].last_result, Some(Ter::TES_SUCCESS));
    }

    #[test]
    fn run_try_clear_queued_batch_stops_after_first_non_ticket_unapplied_result() {
        let mut queued = vec![
            make_queued(SeqProxy::sequence(5)),
            make_queued(SeqProxy::sequence(6)),
        ];
        let mut calls = 0;

        let progress = run_try_clear_queued_batch(&mut queued, |_queued| {
            calls += 1;
            if calls == 1 {
                ApplyResult::new(Ter::TEF_EXCEPTION, false, false)
            } else {
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            }
        });

        assert_eq!(
            progress,
            TryClearQueuedBatchProgress::FallBackToNormal {
                failed_index: 0,
                result: ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
            }
        );
        assert_eq!(calls, 1);
        assert_eq!(queued[0].retries_remaining, 9);
        assert_eq!(queued[0].last_result, Some(Ter::TEF_EXCEPTION));
        assert_eq!(queued[1].retries_remaining, 10);
        assert_eq!(queued[1].last_result, None);
    }

    #[test]
    #[should_panic(expected = "queued result count matches applied range")]
    fn process_try_clear_queued_results_requires_a_result_for_each_attempted_tx() {
        let mut queued = vec![
            make_queued(SeqProxy::sequence(5)),
            make_queued(SeqProxy::sequence(6)),
        ];

        let _ = process_try_clear_queued_results(
            &mut queued,
            [ApplyResult::new(Ter::TES_SUCCESS, true, true)],
        );
    }

    #[test]
    fn cleanup_after_try_clear_success_removes_prior_range_and_replaced_target() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        account.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new("s6", TxConsequences::new(1, SeqProxy::sequence(6))),
        );
        account.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new("s8", TxConsequences::new(1, SeqProxy::sequence(8))),
        );
        account.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new("s9", TxConsequences::new(1, SeqProxy::sequence(9))),
        );

        let cleanup = cleanup_after_try_clear_success(
            &mut account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(8),
        );

        assert_eq!(
            cleanup,
            TryClearSuccessCleanup {
                removed_seq_proxies: vec![
                    SeqProxy::sequence(5),
                    SeqProxy::sequence(6),
                    SeqProxy::sequence(8),
                ],
                removed_replaced_target: true,
                next_seq_proxy: Some(SeqProxy::sequence(9)),
            }
        );
        assert_eq!(account.get_txn_count(), 1);
        assert!(account.transactions.contains_key(&SeqProxy::sequence(9)));
    }

    #[test]
    fn cleanup_after_try_clear_success_leaves_non_replacement_target_absent() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        account.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new("s6", TxConsequences::new(1, SeqProxy::sequence(6))),
        );
        account.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new("s9", TxConsequences::new(1, SeqProxy::sequence(9))),
        );

        let cleanup = cleanup_after_try_clear_success(
            &mut account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(8),
        );

        assert_eq!(
            cleanup,
            TryClearSuccessCleanup {
                removed_seq_proxies: vec![SeqProxy::sequence(5), SeqProxy::sequence(6)],
                removed_replaced_target: false,
                next_seq_proxy: Some(SeqProxy::sequence(9)),
            }
        );
        assert_eq!(account.get_txn_count(), 1);
        assert!(account.transactions.contains_key(&SeqProxy::sequence(9)));
    }

    #[test]
    fn finalize_try_clear_result_runs_cleanup_only_when_current_tx_applied() {
        let mut success_account = TxQAccount::new("acct");
        success_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        success_account.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new("s8", TxConsequences::new(1, SeqProxy::sequence(8))),
        );
        success_account.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new("s9", TxConsequences::new(1, SeqProxy::sequence(9))),
        );

        let success = finalize_try_clear_result(
            &mut success_account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(8),
            ApplyResult::new(Ter::TES_SUCCESS, true, true),
        );

        assert_eq!(
            success,
            TryClearFinalization {
                current_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                cleanup: Some(TryClearSuccessCleanup {
                    removed_seq_proxies: vec![SeqProxy::sequence(5), SeqProxy::sequence(8)],
                    removed_replaced_target: true,
                    next_seq_proxy: Some(SeqProxy::sequence(9)),
                }),
            }
        );
        assert_eq!(success_account.get_txn_count(), 1);

        let mut failure_account = TxQAccount::new("acct");
        failure_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        failure_account.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new("s8", TxConsequences::new(1, SeqProxy::sequence(8))),
        );

        let failure = finalize_try_clear_result(
            &mut failure_account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(8),
            ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
        );

        assert_eq!(
            failure,
            TryClearFinalization {
                current_result: ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
                cleanup: None,
            }
        );
        assert_eq!(failure_account.get_txn_count(), 2);
    }

    #[test]
    fn run_try_clear_current_tx_after_batch_short_circuits_batch_failure() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        let mut ran_current = false;

        let execution = run_try_clear_current_tx_after_batch(
            &mut account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(8),
            TryClearQueuedBatchProgress::FallBackToNormal {
                failed_index: 0,
                result: ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
            },
            || {
                ran_current = true;
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            },
        );

        assert!(!ran_current);
        assert_eq!(
            execution,
            TryClearExecution::FallBackToNormal {
                failed_index: 0,
                result: ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
            }
        );
    }

    #[test]
    fn run_try_clear_current_tx_after_batch_runs_current_and_cleanup_on_success() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        account.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new("s6", TxConsequences::new(1, SeqProxy::sequence(6))),
        );
        account.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new("s8", TxConsequences::new(1, SeqProxy::sequence(8))),
        );
        let mut ran_current = false;

        let execution = run_try_clear_current_tx_after_batch(
            &mut account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(8),
            TryClearQueuedBatchProgress::ContinueToCurrentTx,
            || {
                ran_current = true;
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            },
        );

        assert!(ran_current);
        assert_eq!(
            execution,
            TryClearExecution::CurrentTx(TryClearFinalization {
                current_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                cleanup: Some(TryClearSuccessCleanup {
                    removed_seq_proxies: vec![
                        SeqProxy::sequence(5),
                        SeqProxy::sequence(6),
                        SeqProxy::sequence(8),
                    ],
                    removed_replaced_target: true,
                    next_seq_proxy: None,
                }),
            })
        );
        assert!(account.transactions.is_empty());
    }

    #[test]
    fn run_try_clear_current_tx_after_batch_runs_current_without_cleanup_on_failure() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        account.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new("s8", TxConsequences::new(1, SeqProxy::sequence(8))),
        );
        let mut ran_current = false;

        let execution = run_try_clear_current_tx_after_batch(
            &mut account,
            SeqProxy::sequence(5),
            SeqProxy::sequence(8),
            TryClearQueuedBatchProgress::ContinueToCurrentTx,
            || {
                ran_current = true;
                ApplyResult::new(Ter::TER_RETRY, false, false)
            },
        );

        assert!(ran_current);
        assert_eq!(
            execution,
            TryClearExecution::CurrentTx(TryClearFinalization {
                current_result: ApplyResult::new(Ter::TER_RETRY, false, false),
                cleanup: None,
            })
        );
        assert_eq!(
            account.transactions.keys().copied().collect::<Vec<_>>(),
            vec![SeqProxy::sequence(5), SeqProxy::sequence(8)]
        );
    }
}
