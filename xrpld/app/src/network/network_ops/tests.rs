use super::{
    AppNetworkOpsModeOwner, NETWORKOPS_HOLD_LEDGERS, NetworkOpsApplyBatchEntry,
    NetworkOpsApplyBatchStart, NetworkOpsApplyBatchTail, NetworkOpsApplyResultPreamble,
    NetworkOpsApplyStatusBranch, NetworkOpsAsyncDispatch, NetworkOpsBatchDispatch,
    NetworkOpsCurrentLedgerState, NetworkOpsDispatchState, NetworkOpsOperatingMode,
    NetworkOpsPreprocessDecision, NetworkOpsProcessDispatch, NetworkOpsProcessSetFrontDecision,
    NetworkOpsProcessSetOwnerSync, NetworkOpsRelayBranch, NetworkOpsRetryHoldBranch,
    NetworkOpsRuntimeState, NetworkOpsSetBuildDecision, NetworkOpsSubmitDecision,
    NetworkOpsSubmitFlowOutcome, NetworkOpsSyncBatchOutcome, NetworkOpsSyncDispatch,
    NetworkOpsSyncOwnerOutcome, NetworkOpsTransactionSetOutcome, SharedNetworkOpsState,
    classify_networkops_apply_status, format_preprocess_bad_signature_message,
    format_preprocess_cached_bad_message, format_submit_exception_message,
    format_submit_invalid_message, networkops_apply_flags, networkops_enforce_fail_hard,
    networkops_ledgers_left, no_transaction_to_process_message,
    normalize_operating_mode_for_validated_age, run_networkops_apply_batch_tail,
    run_networkops_apply_result_preamble, run_networkops_apply_status_branch,
    run_networkops_apply_txq_batch, run_networkops_begin_apply_batch,
    run_networkops_finish_apply_batch, run_networkops_local_keep,
    run_networkops_merge_pending_transactions, run_networkops_merge_submit_held,
    run_networkops_preprocess_transaction, run_networkops_preprocess_transaction_gate,
    run_networkops_process_transaction, run_networkops_process_transaction_set,
    run_networkops_process_transaction_set_entrypoint,
    run_networkops_process_transaction_set_front, run_networkops_process_transaction_set_owner,
    run_networkops_process_transaction_set_shell, run_networkops_process_transaction_set_stage,
    run_networkops_process_transaction_shell, run_networkops_relay_branch,
    run_networkops_retry_hold_branch, run_networkops_set_current_ledger_state,
    run_networkops_submit_transaction, run_networkops_submit_transaction_gate,
    run_networkops_transaction_async, run_networkops_transaction_async_owner,
    run_networkops_transaction_batch, run_networkops_transaction_batch_owner,
    run_networkops_transaction_sync, run_networkops_transaction_sync_batch,
    run_networkops_transaction_sync_batch_owner, run_networkops_transaction_sync_owner,
};
use protocol::{Ter, trans_token};
use std::cell::RefCell;
use std::time::Duration;
use tx::{ApplyFlags, ApplyResult, CheckValidityResult, Validity};
use xrpl_core::HashRouterFlags;

fn result(validity: Validity, reason: impl Into<String>) -> CheckValidityResult {
    CheckValidityResult {
        validity,
        reason: reason.into(),
        flags_to_set: HashRouterFlags::UNDEFINED,
    }
}

#[test]
fn operating_mode_strings_match_cpp_state_names() {
    assert_eq!(
        NetworkOpsOperatingMode::Disconnected.as_str(),
        "disconnected"
    );
    assert_eq!(NetworkOpsOperatingMode::Connected.as_str(), "connected");
    assert_eq!(NetworkOpsOperatingMode::Syncing.as_str(), "syncing");
    assert_eq!(NetworkOpsOperatingMode::Tracking.as_str(), "tracking");
    assert_eq!(NetworkOpsOperatingMode::Full.as_str(), "full");
}

#[test]
fn operating_mode_normalization_validated_age_gate() {
    assert_eq!(
        normalize_operating_mode_for_validated_age(
            NetworkOpsOperatingMode::Connected,
            Duration::from_secs(59),
            false,
        ),
        NetworkOpsOperatingMode::Syncing
    );
    assert_eq!(
        normalize_operating_mode_for_validated_age(
            NetworkOpsOperatingMode::Syncing,
            Duration::from_secs(60),
            false,
        ),
        NetworkOpsOperatingMode::Connected
    );
    assert_eq!(
        normalize_operating_mode_for_validated_age(
            NetworkOpsOperatingMode::Full,
            Duration::from_secs(1),
            true,
        ),
        NetworkOpsOperatingMode::Connected
    );
}

#[test]
fn shared_network_ops_state_tracks_live_operating_mode() {
    let state = SharedNetworkOpsState::default();
    assert_eq!(
        state.operating_mode(),
        NetworkOpsOperatingMode::Disconnected
    );
    assert_eq!(state.str_operating_mode(), "disconnected");

    state.set_operating_mode(NetworkOpsOperatingMode::Tracking);
    assert_eq!(state.operating_mode(), NetworkOpsOperatingMode::Tracking);
    assert_eq!(state.str_operating_mode(), "tracking");
}

#[test]
fn shared_network_ops_state_tracks_blocking_flags() {
    let state = SharedNetworkOpsState::default();
    assert!(!state.need_network_ledger());
    assert!(!state.amendment_blocked());
    assert!(!state.unl_blocked());

    state.set_need_network_ledger(true);
    state.set_amendment_blocked(true);
    state.set_unl_blocked(true);

    assert!(state.need_network_ledger());
    assert!(state.amendment_blocked());
    assert!(state.unl_blocked());
}

#[test]
fn shared_network_ops_state_is_full_requires_mode_and_no_network_ledger_gap() {
    let state = SharedNetworkOpsState::new(NetworkOpsOperatingMode::Full);
    assert!(state.is_full());

    state.set_need_network_ledger(true);
    assert!(!state.is_full());

    state.set_need_network_ledger(false);
    state.set_operating_mode(NetworkOpsOperatingMode::Tracking);
    assert!(!state.is_full());
}

#[test]
fn app_network_ops_mode_owner_normalizes_mode_setmode() {
    let state = std::sync::Arc::new(SharedNetworkOpsState::new(
        NetworkOpsOperatingMode::Disconnected,
    ));
    let owner = AppNetworkOpsModeOwner::new(
        state.clone(),
        std::sync::Arc::new(|| Duration::from_secs(30)),
    );

    assert_eq!(
        owner.set_operating_mode(NetworkOpsOperatingMode::Connected),
        NetworkOpsOperatingMode::Disconnected
    );
    assert_eq!(owner.operating_mode(), NetworkOpsOperatingMode::Syncing);

    state.set_amendment_blocked(true);
    owner.set_operating_mode(NetworkOpsOperatingMode::Full);
    assert_eq!(owner.operating_mode(), NetworkOpsOperatingMode::Connected);
}

#[test]
fn submit_rejects_inner_batch_before_router_lookup() {
    let calls = RefCell::new(Vec::new());

    let decision = run_networkops_submit_transaction_gate(
        true,
        true,
        || {
            calls.borrow_mut().push("flags");
            HashRouterFlags::UNDEFINED
        },
        || {
            calls.borrow_mut().push("check");
            Ok(result(Validity::Valid, ""))
        },
    );

    assert_eq!(decision, NetworkOpsSubmitDecision::RejectInnerBatch);
    assert!(calls.borrow().is_empty());
}

#[test]
fn preprocess_cached_bad_skips_later_checks() {
    let calls = RefCell::new(Vec::new());

    let decision = run_networkops_preprocess_transaction_gate(
        false,
        true,
        || {
            calls.borrow_mut().push("flags");
            HashRouterFlags::BAD
        },
        || {
            calls.borrow_mut().push("check");
            result(Validity::Valid, "")
        },
    );

    assert_eq!(decision, NetworkOpsPreprocessDecision::RejectCachedBad);
    assert_eq!(calls.into_inner(), vec!["flags"]);
}

#[test]
fn formatting_helpers_match_current_cpp_messages() {
    assert_eq!(
        format_submit_invalid_message("Invalid signature."),
        "Submitted transaction invalid: Invalid signature."
    );
    assert_eq!(
        format_submit_exception_message("ABC", "boom"),
        "Exception checking transaction ABC: boom"
    );
    assert_eq!(
        format_preprocess_cached_bad_message("ABC"),
        "ABC: cached bad!\n"
    );
    assert_eq!(
        format_preprocess_bad_signature_message("Transaction has bad signature."),
        "Transaction has bad signature: Transaction has bad signature."
    );
}

#[test]
fn apply_flags_match_current_cpp_bits() {
    assert_eq!(networkops_apply_flags(false, false), ApplyFlags::NONE);
    assert_eq!(networkops_apply_flags(true, false), ApplyFlags::UNLIMITED);
    assert_eq!(networkops_apply_flags(false, true), ApplyFlags::FAIL_HARD);
    assert_eq!(
        networkops_apply_flags(true, true),
        ApplyFlags::UNLIMITED | ApplyFlags::FAIL_HARD
    );
}

#[test]
fn apply_txq_batch_updates_entries_in_order() {
    let seen = RefCell::new(Vec::new());
    let mut entries = vec![
        NetworkOpsApplyBatchEntry::new(1u8, false, false, false),
        NetworkOpsApplyBatchEntry::new(2u8, true, true, true),
    ];

    let changed = run_networkops_apply_txq_batch(&mut entries, |tx, flags| {
        seen.borrow_mut().push((*tx, flags));
        match *tx {
            1 => ApplyResult::new(Ter::TER_QUEUED, false, false),
            2 => ApplyResult::new(Ter::TES_SUCCESS, true, false),
            _ => unreachable!(),
        }
    });

    assert!(changed);
    assert_eq!(
        seen.into_inner(),
        vec![
            (1u8, ApplyFlags::NONE),
            (2u8, ApplyFlags::UNLIMITED | ApplyFlags::FAIL_HARD),
        ]
    );
    assert_eq!(entries[0].result, Some(Ter::TER_QUEUED));
    assert!(!entries[0].applied);
    assert_eq!(entries[1].result, Some(Ter::TES_SUCCESS));
    assert!(entries[1].applied);
}

#[test]
fn apply_result_preamble_preserves_for_applied_entries() {
    let calls = RefCell::new(Vec::new());
    let mut entry = NetworkOpsApplyBatchEntry::new(7u8, false, false, false);
    entry.applied = true;
    entry.result = Some(Ter::TES_SUCCESS);

    let outcome = run_networkops_apply_result_preamble(
        &entry,
        |tx| calls.borrow_mut().push(format!("clear:{tx}")),
        |tx, result| {
            calls
                .borrow_mut()
                .push(format!("publish:{tx}:{}", trans_token(result)))
        },
        |tx| calls.borrow_mut().push(format!("applied:{tx}")),
        |tx, result| {
            calls
                .borrow_mut()
                .push(format!("result:{tx}:{}", trans_token(result)))
        },
        |tx| calls.borrow_mut().push(format!("bad:{tx}")),
    );

    assert_eq!(
        outcome,
        NetworkOpsApplyResultPreamble {
            published: true,
            malformed: false,
        }
    );
    assert_eq!(
        calls.into_inner(),
        vec![
            "clear:7".to_string(),
            "publish:7:tesSUCCESS".to_string(),
            "applied:7".to_string(),
            "result:7:tesSUCCESS".to_string(),
        ]
    );
}

#[test]
fn apply_result_preamble_marks_malformed_results_bad() {
    let calls = RefCell::new(Vec::new());
    let mut entry = NetworkOpsApplyBatchEntry::new(9u8, false, false, false);
    entry.applied = false;
    entry.result = Some(Ter::TEM_MALFORMED);

    let outcome = run_networkops_apply_result_preamble(
        &entry,
        |tx| calls.borrow_mut().push(format!("clear:{tx}")),
        |tx, result| {
            calls
                .borrow_mut()
                .push(format!("publish:{tx}:{}", trans_token(result)))
        },
        |tx| calls.borrow_mut().push(format!("applied:{tx}")),
        |tx, result| {
            calls
                .borrow_mut()
                .push(format!("result:{tx}:{}", trans_token(result)))
        },
        |tx| calls.borrow_mut().push(format!("bad:{tx}")),
    );

    assert_eq!(
        outcome,
        NetworkOpsApplyResultPreamble {
            published: false,
            malformed: true,
        }
    );
    assert_eq!(
        calls.into_inner(),
        vec![
            "clear:9".to_string(),
            "result:9:temMALFORMED".to_string(),
            "bad:9".to_string(),
        ]
    );
}

#[test]
fn apply_status_classification_branch_order() {
    assert_eq!(
        classify_networkops_apply_status(Ter::TES_SUCCESS),
        NetworkOpsApplyStatusBranch::Included
    );
    assert_eq!(
        classify_networkops_apply_status(Ter::TEF_PAST_SEQ),
        NetworkOpsApplyStatusBranch::Obsolete
    );
    assert_eq!(
        classify_networkops_apply_status(Ter::TER_QUEUED),
        NetworkOpsApplyStatusBranch::Queued
    );
    assert_eq!(
        classify_networkops_apply_status(Ter::TER_RETRY),
        NetworkOpsApplyStatusBranch::RetryCandidate
    );
    assert_eq!(
        classify_networkops_apply_status(Ter::TEM_MALFORMED),
        NetworkOpsApplyStatusBranch::Invalid
    );
    assert_eq!(
        classify_networkops_apply_status(Ter::TEC_EXPIRED),
        NetworkOpsApplyStatusBranch::Invalid
    );
}

#[test]
fn apply_status_branch_runs_queued_side_effects_in() {
    let calls = RefCell::new(Vec::new());
    let mut entry = NetworkOpsApplyBatchEntry::new(5u8, false, false, false);
    entry.result = Some(Ter::TER_QUEUED);

    let branch = run_networkops_apply_status_branch(
        &entry,
        |tx| calls.borrow_mut().push(format!("included:{tx}")),
        |tx| calls.borrow_mut().push(format!("status_included:{tx}")),
        |tx| calls.borrow_mut().push(format!("status_obsolete:{tx}")),
        |tx| calls.borrow_mut().push(format!("status_held:{tx}")),
        |tx| calls.borrow_mut().push(format!("add_held:{tx}")),
        |tx| calls.borrow_mut().push(format!("queued:{tx}")),
        |tx| calls.borrow_mut().push(format!("kept:{tx}")),
        |tx| calls.borrow_mut().push(format!("status_invalid:{tx}")),
    );

    assert_eq!(branch, NetworkOpsApplyStatusBranch::Queued);
    assert_eq!(
        calls.into_inner(),
        vec![
            "status_held:5".to_string(),
            "add_held:5".to_string(),
            "queued:5".to_string(),
            "kept:5".to_string(),
        ]
    );
}

#[test]
fn apply_status_branch_leaves_retry_candidate_to_later_hold_logic() {
    let calls = RefCell::new(Vec::new());
    let mut entry = NetworkOpsApplyBatchEntry::new(6u8, false, false, false);
    entry.result = Some(Ter::TER_RETRY);

    let branch = run_networkops_apply_status_branch(
        &entry,
        |tx| calls.borrow_mut().push(format!("included:{tx}")),
        |tx| calls.borrow_mut().push(format!("status_included:{tx}")),
        |tx| calls.borrow_mut().push(format!("status_obsolete:{tx}")),
        |tx| calls.borrow_mut().push(format!("status_held:{tx}")),
        |tx| calls.borrow_mut().push(format!("add_held:{tx}")),
        |tx| calls.borrow_mut().push(format!("queued:{tx}")),
        |tx| calls.borrow_mut().push(format!("kept:{tx}")),
        |tx| calls.borrow_mut().push(format!("status_invalid:{tx}")),
    );

    assert_eq!(branch, NetworkOpsApplyStatusBranch::RetryCandidate);
    assert!(calls.borrow().is_empty());
}

#[test]
fn retry_hold_branch_holds_local_without_router_flag() {
    let calls = RefCell::new(Vec::new());
    let entry = NetworkOpsApplyBatchEntry::new(8u8, false, true, false);

    let branch = run_networkops_retry_hold_branch(
        &entry,
        100,
        None,
        |tx| {
            calls.borrow_mut().push(format!("flag:{tx}"));
            false
        },
        |tx| calls.borrow_mut().push(format!("status_held:{tx}")),
        |tx| calls.borrow_mut().push(format!("add_held:{tx}")),
        |tx| calls.borrow_mut().push(format!("kept:{tx}")),
    );

    assert_eq!(
        branch,
        NetworkOpsRetryHoldBranch::Held { ledgers_left: None }
    );
    assert_eq!(
        calls.into_inner(),
        vec![
            "status_held:8".to_string(),
            "add_held:8".to_string(),
            "kept:8".to_string(),
        ]
    );
}

#[test]
fn retry_hold_branch_holds_when_last_ledger_is_near() {
    let calls = RefCell::new(Vec::new());
    let entry = NetworkOpsApplyBatchEntry::new(9u8, false, false, false);

    let branch = run_networkops_retry_hold_branch(
        &entry,
        100,
        Some(100 + NETWORKOPS_HOLD_LEDGERS),
        |tx| {
            calls.borrow_mut().push(format!("flag:{tx}"));
            false
        },
        |tx| calls.borrow_mut().push(format!("status_held:{tx}")),
        |tx| calls.borrow_mut().push(format!("add_held:{tx}")),
        |tx| calls.borrow_mut().push(format!("kept:{tx}")),
    );

    assert_eq!(
        branch,
        NetworkOpsRetryHoldBranch::Held {
            ledgers_left: Some(NETWORKOPS_HOLD_LEDGERS),
        }
    );
    assert_eq!(
        calls.into_inner(),
        vec![
            "status_held:9".to_string(),
            "add_held:9".to_string(),
            "kept:9".to_string(),
        ]
    );
}

#[test]
fn retry_hold_branch_falls_back_to_router_flag_once() {
    let calls = RefCell::new(Vec::new());
    let entry = NetworkOpsApplyBatchEntry::new(10u8, false, false, false);

    let branch = run_networkops_retry_hold_branch(
        &entry,
        100,
        None,
        |tx| {
            calls.borrow_mut().push(format!("flag:{tx}"));
            true
        },
        |tx| calls.borrow_mut().push(format!("status_held:{tx}")),
        |tx| calls.borrow_mut().push(format!("add_held:{tx}")),
        |tx| calls.borrow_mut().push(format!("kept:{tx}")),
    );

    assert_eq!(
        branch,
        NetworkOpsRetryHoldBranch::Held { ledgers_left: None }
    );
    assert_eq!(
        calls.into_inner(),
        vec![
            "flag:10".to_string(),
            "status_held:10".to_string(),
            "add_held:10".to_string(),
            "kept:10".to_string(),
        ]
    );
}

#[test]
fn retry_hold_branch_skips_side_effects_for_fail_hard() {
    let calls = RefCell::new(Vec::new());
    let entry = NetworkOpsApplyBatchEntry::new(11u8, false, true, true);

    let branch = run_networkops_retry_hold_branch(
        &entry,
        100,
        Some(101),
        |tx| {
            calls.borrow_mut().push(format!("flag:{tx}"));
            true
        },
        |tx| calls.borrow_mut().push(format!("status_held:{tx}")),
        |tx| calls.borrow_mut().push(format!("add_held:{tx}")),
        |tx| calls.borrow_mut().push(format!("kept:{tx}")),
    );

    assert_eq!(branch, NetworkOpsRetryHoldBranch::FailHard);
    assert!(calls.borrow().is_empty());
}

#[test]
fn local_keep_respects_enforce_fail_hard_gate() {
    let calls = RefCell::new(Vec::new());
    let entry = NetworkOpsApplyBatchEntry::new(12u8, false, true, true);

    assert!(networkops_enforce_fail_hard(true, Ter::TEF_FAILURE));
    assert!(!run_networkops_local_keep(
        &entry,
        Ter::TEF_FAILURE,
        |tx| calls.borrow_mut().push(format!("push:{tx}")),
        |tx| calls.borrow_mut().push(format!("kept:{tx}")),
    ));
    assert!(calls.borrow().is_empty());
}

#[test]
fn local_keep_pushes_and_keeps_non_fail_hard_local() {
    let calls = RefCell::new(Vec::new());
    let entry = NetworkOpsApplyBatchEntry::new(13u8, false, true, false);

    assert!(run_networkops_local_keep(
        &entry,
        Ter::TER_RETRY,
        |tx| calls.borrow_mut().push(format!("push:{tx}")),
        |tx| calls.borrow_mut().push(format!("kept:{tx}")),
    ));
    assert_eq!(
        calls.into_inner(),
        vec!["push:13".to_string(), "kept:13".to_string()]
    );
    assert_eq!(
        networkops_ledgers_left(Some(5), 10),
        Some(5u32.wrapping_sub(10))
    );
}

#[test]
fn relay_branch_skips_router_when_not_eligible() {
    let calls = RefCell::new(Vec::new());
    let entry = NetworkOpsApplyBatchEntry::new(14u8, false, true, true);

    let branch = run_networkops_relay_branch(
        &entry,
        false,
        Ter::TEF_FAILURE,
        false,
        |tx| {
            calls.borrow_mut().push(format!("should_relay:{tx}"));
            Some(1u8)
        },
        |tx, deferred, skip| {
            calls
                .borrow_mut()
                .push(format!("relay:{tx}:{deferred}:{skip}"));
        },
        |tx| calls.borrow_mut().push(format!("broadcast:{tx}")),
    );

    assert_eq!(branch, NetworkOpsRelayBranch::SkippedEligibility);
    assert!(calls.borrow().is_empty());
}

#[test]
fn relay_branch_relays_non_full_local() {
    let calls = RefCell::new(Vec::new());
    let entry = NetworkOpsApplyBatchEntry::new(15u8, false, true, false);

    let branch = run_networkops_relay_branch(
        &entry,
        false,
        Ter::TER_RETRY,
        false,
        |tx| {
            calls.borrow_mut().push(format!("should_relay:{tx}"));
            Some(7u8)
        },
        |tx, deferred, skip| {
            calls
                .borrow_mut()
                .push(format!("relay:{tx}:{deferred}:{skip}"));
        },
        |tx| calls.borrow_mut().push(format!("broadcast:{tx}")),
    );

    assert_eq!(branch, NetworkOpsRelayBranch::Relayed { deferred: false });
    assert_eq!(
        calls.into_inner(),
        vec![
            "should_relay:15".to_string(),
            "relay:15:false:7".to_string(),
            "broadcast:15".to_string(),
        ]
    );
}

#[test]
fn relay_branch_marks_queued_transactions_deferred() {
    let calls = RefCell::new(Vec::new());
    let entry = NetworkOpsApplyBatchEntry::new(16u8, false, false, false);

    let branch = run_networkops_relay_branch(
        &entry,
        true,
        Ter::TER_QUEUED,
        false,
        |tx| {
            calls.borrow_mut().push(format!("should_relay:{tx}"));
            Some(8u8)
        },
        |tx, deferred, skip| {
            calls
                .borrow_mut()
                .push(format!("relay:{tx}:{deferred}:{skip}"));
        },
        |tx| calls.borrow_mut().push(format!("broadcast:{tx}")),
    );

    assert_eq!(branch, NetworkOpsRelayBranch::Relayed { deferred: true });
    assert_eq!(
        calls.into_inner(),
        vec![
            "should_relay:16".to_string(),
            "relay:16:true:8".to_string(),
            "broadcast:16".to_string(),
        ]
    );
}

#[test]
fn relay_branch_skips_inner_batch_after_router() {
    let calls = RefCell::new(Vec::new());
    let mut entry = NetworkOpsApplyBatchEntry::new(17u8, false, false, false);
    entry.applied = true;

    let branch = run_networkops_relay_branch(
        &entry,
        true,
        Ter::TES_SUCCESS,
        true,
        |tx| {
            calls.borrow_mut().push(format!("should_relay:{tx}"));
            Some(9u8)
        },
        |tx, deferred, skip| {
            calls
                .borrow_mut()
                .push(format!("relay:{tx}:{deferred}:{skip}"));
        },
        |tx| calls.borrow_mut().push(format!("broadcast:{tx}")),
    );

    assert_eq!(branch, NetworkOpsRelayBranch::InnerBatchSuppressed);
    assert_eq!(calls.into_inner(), vec!["should_relay:17".to_string()]);
}

#[test]
fn set_current_ledger_state_skips_missing_validated_ledger() {
    let calls = RefCell::new(Vec::new());
    let entry = NetworkOpsApplyBatchEntry::new(18u8, false, false, false);

    assert!(!run_networkops_set_current_ledger_state(
        &entry,
        None,
        |tx| {
            calls.borrow_mut().push(format!("get:{tx}"));
            NetworkOpsCurrentLedgerState {
                fee: 1u64,
                account_seq: 2u32,
                available_seq: 3u32,
            }
        },
        |tx, ledger, state| {
            calls.borrow_mut().push(format!(
                "set:{tx}:{ledger}:{}:{}:{}",
                state.fee, state.account_seq, state.available_seq
            ));
        },
    ));
    assert!(calls.borrow().is_empty());
}

#[test]
fn set_current_ledger_state_applies_snapshot() {
    let calls = RefCell::new(Vec::new());
    let entry = NetworkOpsApplyBatchEntry::new(19u8, false, false, false);

    assert!(run_networkops_set_current_ledger_state(
        &entry,
        Some(700),
        |tx| {
            calls.borrow_mut().push(format!("get:{tx}"));
            NetworkOpsCurrentLedgerState {
                fee: 10u64,
                account_seq: 11u32,
                available_seq: 12u32,
            }
        },
        |tx, ledger, state| {
            calls.borrow_mut().push(format!(
                "set:{tx}:{ledger}:{}:{}:{}",
                state.fee, state.account_seq, state.available_seq
            ));
        },
    ));
    assert_eq!(
        calls.into_inner(),
        vec!["get:19".to_string(), "set:19:700:10:11:12".to_string()]
    );
}

#[test]
fn begin_apply_batch_swaps_sets_running_and_unlocks() {
    let calls = RefCell::new(Vec::new());
    let mut pending = vec![1u8, 2u8];

    let (transactions, start) =
        run_networkops_begin_apply_batch(&mut pending, NetworkOpsDispatchState::Scheduled, || {
            calls.borrow_mut().push("unlock".to_string())
        });

    assert_eq!(transactions, vec![1, 2]);
    assert!(pending.is_empty());
    assert_eq!(
        start,
        NetworkOpsApplyBatchStart {
            taken_transactions: 2,
            dispatch_state: NetworkOpsDispatchState::Running,
        }
    );
    assert_eq!(calls.into_inner(), vec!["unlock".to_string()]);
}

#[test]
fn finish_apply_batch_relocks_before_tail() {
    let calls = RefCell::new(Vec::new());
    let transactions = vec![
        NetworkOpsApplyBatchEntry::new(20u8, false, false, false),
        NetworkOpsApplyBatchEntry::new(21u8, false, false, false),
    ];
    let mut pending = vec![1u8];
    let mut submit_held = vec![2u8];

    let tail = run_networkops_finish_apply_batch(
        &transactions,
        &mut pending,
        &mut submit_held,
        || calls.borrow_mut().push("relock".to_string()),
        |tx| calls.borrow_mut().push(format!("clear:{tx}")),
        || calls.borrow_mut().push("notify".to_string()),
    );

    assert_eq!(
        tail,
        NetworkOpsApplyBatchTail {
            cleared: 2,
            pending_transactions: 2,
            dispatch_state: NetworkOpsDispatchState::None,
        }
    );
    assert_eq!(pending, vec![1, 2]);
    assert!(submit_held.is_empty());
    assert_eq!(
        calls.into_inner(),
        vec![
            "relock".to_string(),
            "clear:20".to_string(),
            "clear:21".to_string(),
            "notify".to_string(),
        ]
    );
}

#[test]
fn merge_pending_transactions_appends_in_order() {
    let mut pending = vec![1u8];
    let mut incoming = vec![2u8, 3u8];

    assert_eq!(
        run_networkops_merge_pending_transactions(&mut pending, &mut incoming),
        3
    );
    assert_eq!(pending, vec![1, 2, 3]);
    assert!(incoming.is_empty());
}

#[test]
fn merge_submit_held_swaps_into_empty_pending() {
    let mut pending = Vec::<u8>::new();
    let mut submit_held = vec![1u8, 2u8];

    assert_eq!(
        run_networkops_merge_submit_held(&mut pending, &mut submit_held),
        2
    );
    assert_eq!(pending, vec![1, 2]);
    assert!(submit_held.is_empty());
}

#[test]
fn merge_submit_held_appends_into_existing_pending() {
    let mut pending = vec![1u8, 2u8];
    let mut submit_held = vec![3u8, 4u8];

    assert_eq!(
        run_networkops_merge_submit_held(&mut pending, &mut submit_held),
        4
    );
    assert_eq!(pending, vec![1, 2, 3, 4]);
    assert!(submit_held.is_empty());
}

#[test]
fn process_transaction_set_owner_merges_and_starts_sync() {
    let calls = RefCell::new(Vec::new());
    let mut pending = vec![10u8];

    let outcome = run_networkops_process_transaction_set_owner(
        [1u8, 2u8],
        &mut pending,
        |tx| *tx == 2,
        |tx| {
            calls.borrow_mut().push(format!("stage:{tx}"));
            tx
        },
        |tx| *tx == 10 || *tx == 1,
        |sync| {
            let NetworkOpsProcessSetOwnerSync {
                added_count,
                had_pending_before,
                has_applying_after_merge,
            } = sync;
            calls.borrow_mut().push(format!(
                "sync:{added_count}:{had_pending_before}:{has_applying_after_merge}"
            ));
        },
    );

    assert_eq!(
        outcome,
        NetworkOpsTransactionSetOutcome::SyncBatch { added_count: 1 }
    );
    assert_eq!(pending, vec![10, 1]);
    assert_eq!(
        calls.into_inner(),
        vec!["stage:1".to_string(), "sync:1:true:true".to_string()]
    );
}

#[test]
fn process_transaction_set_owner_reports_no_transactions() {
    let calls = RefCell::new(Vec::new());
    let mut pending = Vec::<u8>::new();

    let outcome = run_networkops_process_transaction_set_owner(
        Vec::<u8>::new(),
        &mut pending,
        |_tx| false,
        |tx| tx,
        |_tx| true,
        |_sync| calls.borrow_mut().push("sync".to_string()),
    );

    assert_eq!(outcome, NetworkOpsTransactionSetOutcome::NoTransactions);
    assert!(pending.is_empty());
    assert!(calls.borrow().is_empty());
}

#[test]
fn process_transaction_set_front_preserves_replaced_candidate() {
    let calls = RefCell::new(Vec::new());
    let bad_flag_count = RefCell::new(0usize);

    let candidates = run_networkops_process_transaction_set_front(
        [1u8, 2u8, 3u8],
        |input| match input {
            1 => NetworkOpsProcessSetFrontDecision::RejectInvalid {
                reason: "boom".to_string(),
                set_bad_flag: true,
            },
            2 => NetworkOpsProcessSetFrontDecision::RejectPreprocess,
            3 => NetworkOpsProcessSetFrontDecision::Candidate(30u8),
            _ => unreachable!(),
        },
        |reason| calls.borrow_mut().push(format!("trace:{reason}")),
        || *bad_flag_count.borrow_mut() += 1,
    );

    assert_eq!(candidates, vec![30]);
    assert_eq!(calls.into_inner(), vec!["trace:boom".to_string()]);
    assert_eq!(*bad_flag_count.borrow(), 1);
}

#[test]
fn process_transaction_set_shell_composes_front_and_owner() {
    let calls = RefCell::new(Vec::new());
    let bad_flag_count = RefCell::new(0usize);
    let mut pending = vec![100u8];

    let outcome = run_networkops_process_transaction_set_shell(
        [1u8, 2u8, 3u8],
        &mut pending,
        |input| match input {
            1 => NetworkOpsProcessSetFrontDecision::RejectInvalid {
                reason: "bad".to_string(),
                set_bad_flag: true,
            },
            2 => NetworkOpsProcessSetFrontDecision::RejectPreprocess,
            3 => NetworkOpsProcessSetFrontDecision::Candidate(30u8),
            _ => unreachable!(),
        },
        |reason| calls.borrow_mut().push(format!("trace:{reason}")),
        || *bad_flag_count.borrow_mut() += 1,
        |tx| *tx == 40,
        |tx| {
            calls.borrow_mut().push(format!("stage:{tx}"));
            tx
        },
        |tx| *tx == 100 || *tx == 30,
        |sync| {
            let NetworkOpsProcessSetOwnerSync {
                added_count,
                had_pending_before,
                has_applying_after_merge,
            } = sync;
            calls.borrow_mut().push(format!(
                "sync:{added_count}:{had_pending_before}:{has_applying_after_merge}"
            ));
        },
    );

    assert_eq!(
        outcome,
        NetworkOpsTransactionSetOutcome::SyncBatch { added_count: 1 }
    );
    assert_eq!(pending, vec![100, 30]);
    assert_eq!(*bad_flag_count.borrow(), 1);
    assert_eq!(
        calls.into_inner(),
        vec![
            "trace:bad".to_string(),
            "stage:30".to_string(),
            "sync:1:true:true".to_string(),
        ]
    );
}

#[test]
fn process_transaction_set_entrypoint_makes_load_event_before_shell() {
    let calls = RefCell::new(Vec::new());
    let bad_flag_count = RefCell::new(0usize);
    let mut pending = vec![100u8];

    let outcome = run_networkops_process_transaction_set_entrypoint(
        || calls.borrow_mut().push("event".to_string()),
        [1u8, 2u8, 3u8],
        &mut pending,
        |input| match input {
            1 => NetworkOpsProcessSetFrontDecision::RejectInvalid {
                reason: "bad".to_string(),
                set_bad_flag: true,
            },
            2 => NetworkOpsProcessSetFrontDecision::RejectPreprocess,
            3 => NetworkOpsProcessSetFrontDecision::Candidate(30u8),
            _ => unreachable!(),
        },
        |reason| calls.borrow_mut().push(format!("trace:{reason}")),
        || *bad_flag_count.borrow_mut() += 1,
        |tx| *tx == 40,
        |tx| {
            calls.borrow_mut().push(format!("stage:{tx}"));
            tx
        },
        |tx| *tx == 100 || *tx == 30,
        |sync| {
            let NetworkOpsProcessSetOwnerSync {
                added_count,
                had_pending_before,
                has_applying_after_merge,
            } = sync;
            calls.borrow_mut().push(format!(
                "sync:{added_count}:{had_pending_before}:{has_applying_after_merge}"
            ));
        },
    );

    assert_eq!(
        outcome,
        NetworkOpsTransactionSetOutcome::SyncBatch { added_count: 1 }
    );
    assert_eq!(pending, vec![100, 30]);
    assert_eq!(*bad_flag_count.borrow(), 1);
    assert_eq!(
        calls.into_inner(),
        vec![
            "event".to_string(),
            "trace:bad".to_string(),
            "stage:30".to_string(),
            "sync:1:true:true".to_string(),
        ]
    );
}

#[test]
fn transaction_batch_owner_acquires_lock_before_dispatch() {
    let calls = RefCell::new(Vec::new());
    let remaining = RefCell::new(2usize);

    let dispatch = run_networkops_transaction_batch_owner(
        NetworkOpsDispatchState::None,
        || calls.borrow_mut().push("lock".to_string()),
        || *remaining.borrow() > 0,
        || {
            *remaining.borrow_mut() -= 1;
            calls.borrow_mut().push("apply".to_string());
        },
    );

    assert_eq!(
        dispatch,
        NetworkOpsBatchDispatch::AppliedLoop { iterations: 2 }
    );
    assert_eq!(
        calls.into_inner(),
        vec!["lock".to_string(), "apply".to_string(), "apply".to_string()]
    );
}

#[test]
fn apply_batch_tail_clears_then_merges_then_notifies() {
    let calls = RefCell::new(Vec::new());
    let transactions = vec![
        NetworkOpsApplyBatchEntry::new(20u8, false, false, false),
        NetworkOpsApplyBatchEntry::new(21u8, false, false, false),
    ];
    let mut pending = vec![1u8];
    let mut submit_held = vec![2u8, 3u8];

    let tail = run_networkops_apply_batch_tail(
        &transactions,
        &mut pending,
        &mut submit_held,
        |tx| calls.borrow_mut().push(format!("clear:{tx}")),
        || calls.borrow_mut().push("notify".to_string()),
    );

    assert_eq!(
        tail,
        NetworkOpsApplyBatchTail {
            cleared: 2,
            pending_transactions: 3,
            dispatch_state: NetworkOpsDispatchState::None,
        }
    );
    assert_eq!(pending, vec![1, 2, 3]);
    assert!(submit_held.is_empty());
    assert_eq!(
        calls.into_inner(),
        vec![
            "clear:20".to_string(),
            "clear:21".to_string(),
            "notify".to_string(),
        ]
    );
}

#[test]
fn apply_batch_tail_notifies_even_without_submit_held() {
    let calls = RefCell::new(Vec::new());
    let transactions = vec![NetworkOpsApplyBatchEntry::new(22u8, false, false, false)];
    let mut pending = vec![5u8];
    let mut submit_held = Vec::<u8>::new();

    let tail = run_networkops_apply_batch_tail(
        &transactions,
        &mut pending,
        &mut submit_held,
        |tx| calls.borrow_mut().push(format!("clear:{tx}")),
        || calls.borrow_mut().push("notify".to_string()),
    );

    assert_eq!(
        tail,
        NetworkOpsApplyBatchTail {
            cleared: 1,
            pending_transactions: 1,
            dispatch_state: NetworkOpsDispatchState::None,
        }
    );
    assert_eq!(pending, vec![5]);
    assert!(submit_held.is_empty());
    assert_eq!(
        calls.into_inner(),
        vec!["clear:22".to_string(), "notify".to_string()]
    );
}

#[test]
fn submit_transaction_returns_early_when_network_ledger_is_needed() {
    let calls = RefCell::new(Vec::new());

    let outcome = run_networkops_submit_transaction(
        true,
        || {
            calls.borrow_mut().push("gate");
            NetworkOpsSubmitDecision::Accept
        },
        || calls.borrow_mut().push("construct"),
        || calls.borrow_mut().push("enqueue"),
    );

    assert_eq!(outcome, NetworkOpsSubmitFlowOutcome::NeedNetworkLedger);
    assert!(calls.borrow().is_empty());
}

#[test]
fn submit_transaction_constructs_before_enqueue() {
    let calls = RefCell::new(Vec::new());

    let outcome = run_networkops_submit_transaction(
        false,
        || {
            calls.borrow_mut().push("gate");
            NetworkOpsSubmitDecision::Accept
        },
        || calls.borrow_mut().push("construct"),
        || calls.borrow_mut().push("enqueue"),
    );

    assert_eq!(outcome, NetworkOpsSubmitFlowOutcome::Queued);
    assert_eq!(calls.into_inner(), vec!["gate", "construct", "enqueue"]);
}

#[test]
fn preprocess_transaction_continue_canonicalizes() {
    let calls = RefCell::new(Vec::new());

    let accepted = run_networkops_preprocess_transaction(
        NetworkOpsPreprocessDecision::Continue,
        |_result| calls.borrow_mut().push("set"),
        || calls.borrow_mut().push("flag"),
        || calls.borrow_mut().push("canonicalize"),
    );

    assert!(accepted);
    assert_eq!(calls.into_inner(), vec!["canonicalize"]);
}

#[test]
fn preprocess_transaction_reject_inner_batch_sets_result_then_bad_flag() {
    let calls = RefCell::new(Vec::new());

    let accepted = run_networkops_preprocess_transaction(
        NetworkOpsPreprocessDecision::RejectInnerBatch,
        |result| {
            assert_eq!(result, Ter::TEM_INVALID_FLAG);
            calls.borrow_mut().push("set");
        },
        || calls.borrow_mut().push("flag"),
        || calls.borrow_mut().push("canonicalize"),
    );

    assert!(!accepted);
    assert_eq!(calls.into_inner(), vec!["set", "flag"]);
}

#[test]
fn process_transaction_rejects_before_sync_async_split() {
    let calls = RefCell::new(Vec::new());

    let dispatch = run_networkops_process_transaction(
        || {
            calls.borrow_mut().push("pre");
            false
        },
        true,
        || calls.borrow_mut().push("sync"),
        || calls.borrow_mut().push("async"),
    );

    assert_eq!(dispatch, NetworkOpsProcessDispatch::Rejected);
    assert_eq!(calls.into_inner(), vec!["pre"]);
}

#[test]
fn transaction_async_schedules_batch_only_from_none_state() {
    let calls = RefCell::new(Vec::new());

    let (dispatch, state) = run_networkops_transaction_async(
        false,
        NetworkOpsDispatchState::None,
        || calls.borrow_mut().push("push"),
        || calls.borrow_mut().push("set"),
        || {
            calls.borrow_mut().push("job");
            true
        },
    );

    assert_eq!(dispatch, NetworkOpsAsyncDispatch::Scheduled);
    assert_eq!(state, NetworkOpsDispatchState::Scheduled);
    assert_eq!(calls.into_inner(), vec!["push", "set", "job"]);
}

#[test]
fn transaction_async_owner_acquires_lock_before_staging() {
    let calls = RefCell::new(Vec::new());

    let (dispatch, state) = run_networkops_transaction_async_owner(
        false,
        NetworkOpsDispatchState::None,
        || calls.borrow_mut().push("lock"),
        || calls.borrow_mut().push("push"),
        || calls.borrow_mut().push("set"),
        || {
            calls.borrow_mut().push("job");
            true
        },
    );

    assert_eq!(dispatch, NetworkOpsAsyncDispatch::Scheduled);
    assert_eq!(state, NetworkOpsDispatchState::Scheduled);
    assert_eq!(calls.into_inner(), vec!["lock", "push", "set", "job"]);
}

#[test]
fn process_transaction_shell_makes_load_event_before_preprocess() {
    let calls = RefCell::new(Vec::new());

    let dispatch = run_networkops_process_transaction_shell(
        || calls.borrow_mut().push("event"),
        || {
            calls.borrow_mut().push("pre");
            true
        },
        true,
        || calls.borrow_mut().push("sync"),
        || calls.borrow_mut().push("async"),
    );

    assert_eq!(dispatch, NetworkOpsProcessDispatch::Sync);
    assert_eq!(calls.into_inner(), vec!["event", "pre", "sync"]);
}

#[test]
fn transaction_sync_stages_only_when_not_already_applying() {
    let calls = RefCell::new(Vec::new());

    let dispatch = run_networkops_transaction_sync(
        false,
        || calls.borrow_mut().push("stage"),
        || calls.borrow_mut().push("sync"),
    );

    assert_eq!(dispatch, NetworkOpsSyncDispatch::Staged);
    assert_eq!(calls.into_inner(), vec!["stage", "sync"]);
}

#[test]
fn transaction_sync_owner_stages_then_runs_batch() {
    let calls = RefCell::new(Vec::new());
    let applying = RefCell::new(false);
    let mut lock = 4u8;

    let outcome = run_networkops_transaction_sync_owner(
        NetworkOpsDispatchState::None,
        false,
        &mut lock,
        |lock| {
            *lock += 1;
            calls.borrow_mut().push(format!("stage:{lock}"));
        },
        || {
            *applying.borrow_mut() = true;
            calls.borrow_mut().push("set".to_string());
        },
        |_lock| {
            calls
                .borrow_mut()
                .push(format!("retry:{}", *applying.borrow()));
            applying.replace(false)
        },
        |lock| calls.borrow_mut().push(format!("wait:{lock}")),
        |lock| calls.borrow_mut().push(format!("apply:{lock}")),
        |lock| {
            calls.borrow_mut().push(format!("has:{lock}"));
            false
        },
        |lock| {
            calls.borrow_mut().push(format!("job:{lock}"));
            false
        },
    );

    assert_eq!(
        outcome,
        NetworkOpsSyncOwnerOutcome {
            dispatch: NetworkOpsSyncDispatch::Staged,
            batch: NetworkOpsSyncBatchOutcome {
                waited: 0,
                applied: 2,
                scheduled: false,
                dispatch_state: NetworkOpsDispatchState::None,
            },
        }
    );
    assert_eq!(lock, 5);
    assert_eq!(
        calls.into_inner(),
        vec![
            "stage:5".to_string(),
            "set".to_string(),
            "apply:5".to_string(),
            "has:5".to_string(),
            "retry:true".to_string(),
            "apply:5".to_string(),
            "has:5".to_string(),
            "retry:false".to_string(),
        ]
    );
}

#[test]
fn transaction_sync_batch_waits_while_running() {
    let calls = RefCell::new(Vec::new());
    let retry = RefCell::new(vec![true, false].into_iter());

    let outcome = run_networkops_transaction_sync_batch(
        NetworkOpsDispatchState::Running,
        || retry.borrow_mut().next().unwrap_or(false),
        || calls.borrow_mut().push("wait"),
        || calls.borrow_mut().push("apply"),
        || false,
        || false,
    );

    assert_eq!(
        outcome,
        NetworkOpsSyncBatchOutcome {
            waited: 2,
            applied: 0,
            scheduled: false,
            dispatch_state: NetworkOpsDispatchState::Running,
        }
    );
    assert_eq!(calls.into_inner(), vec!["wait", "wait"]);
}

#[test]
fn transaction_sync_batch_owner_threads_lock() {
    let calls = RefCell::new(Vec::new());
    let retry = RefCell::new(vec![true, false].into_iter());
    let mut lock = 9u8;

    let outcome = run_networkops_transaction_sync_batch_owner(
        NetworkOpsDispatchState::Running,
        &mut lock,
        |lock| {
            calls.borrow_mut().push(format!("retry:{lock}"));
            retry.borrow_mut().next().unwrap_or(false)
        },
        |lock| {
            *lock += 1;
            calls.borrow_mut().push(format!("wait:{lock}"));
        },
        |lock| calls.borrow_mut().push(format!("apply:{lock}")),
        |lock| {
            calls.borrow_mut().push(format!("has:{lock}"));
            false
        },
        |lock| {
            calls.borrow_mut().push(format!("job:{lock}"));
            false
        },
    );

    assert_eq!(
        outcome,
        NetworkOpsSyncBatchOutcome {
            waited: 2,
            applied: 0,
            scheduled: false,
            dispatch_state: NetworkOpsDispatchState::Running,
        }
    );
    assert_eq!(lock, 11);
    assert_eq!(
        calls.into_inner(),
        vec![
            "wait:10".to_string(),
            "retry:10".to_string(),
            "wait:11".to_string(),
            "retry:11".to_string(),
        ]
    );
}

#[test]
fn transaction_batch_returns_early_when_already_running() {
    let calls = RefCell::new(Vec::new());

    let dispatch = run_networkops_transaction_batch(
        NetworkOpsDispatchState::Running,
        || {
            calls.borrow_mut().push("has");
            true
        },
        || calls.borrow_mut().push("apply"),
    );

    assert_eq!(dispatch, NetworkOpsBatchDispatch::AlreadyRunning);
    assert!(calls.borrow().is_empty());
}

#[test]
fn process_transaction_set_filters_invalid_and_preprocess_rejections() {
    let traces = RefCell::new(Vec::new());
    let bad_flag_count = RefCell::new(0usize);

    let candidates = run_networkops_process_transaction_set(
        [1, 2, 3],
        |tx| match tx {
            1 => NetworkOpsSetBuildDecision::RejectInvalid {
                reason: "boom".to_string(),
                set_bad_flag: true,
            },
            2 => NetworkOpsSetBuildDecision::RejectPreprocess,
            3 => NetworkOpsSetBuildDecision::Candidate,
            _ => unreachable!(),
        },
        |reason| traces.borrow_mut().push(reason.to_string()),
        || *bad_flag_count.borrow_mut() += 1,
    );

    assert_eq!(candidates, vec![3]);
    assert_eq!(traces.into_inner(), vec!["boom"]);
    assert_eq!(*bad_flag_count.borrow(), 1);
}

#[test]
fn process_transaction_set_stage_reports_no_transactions_networkops_test() {
    let merged = RefCell::new(None::<Vec<u8>>);
    let sync_called = RefCell::new(false);

    let outcome = run_networkops_process_transaction_set_stage(
        Vec::<u8>::new(),
        true,
        |_tx| false,
        |_tx| panic!("setApplying must not run without candidates"),
        |transactions| *merged.borrow_mut() = Some(transactions),
        || *sync_called.borrow_mut() = true,
    );

    assert_eq!(outcome, NetworkOpsTransactionSetOutcome::NoTransactions);
    assert_eq!(merged.into_inner(), Some(Vec::new()));
    assert!(!*sync_called.borrow());
    assert_eq!(
        no_transaction_to_process_message(),
        "No transaction to process!"
    );
}

#[test]
fn runtime_state_async_updates_owned_queue_and_dispatch_owner() {
    let calls = RefCell::new(Vec::new());
    let mut owner = NetworkOpsRuntimeState::<u8>::default();

    let dispatch = owner.transaction_async(
        false,
        7,
        || calls.borrow_mut().push("set".to_string()),
        || {
            calls.borrow_mut().push("job".to_string());
            true
        },
    );

    assert_eq!(dispatch, NetworkOpsAsyncDispatch::Scheduled);
    assert_eq!(owner.pending_transactions(), &[7]);
    assert_eq!(owner.dispatch_state(), NetworkOpsDispatchState::Scheduled);
    assert_eq!(
        calls.into_inner(),
        vec!["set".to_string(), "job".to_string()]
    );
}

#[test]
fn runtime_state_finish_apply_batch_clears_merges_and_resets_dispatch_owner() {
    let calls = RefCell::new(Vec::new());
    let mut owner =
        NetworkOpsRuntimeState::new(vec![1u8], vec![2u8, 3u8], NetworkOpsDispatchState::Running);
    let transactions = vec![NetworkOpsApplyBatchEntry::new(20u8, false, false, false)];

    let tail = owner.finish_apply_batch(
        &transactions,
        || calls.borrow_mut().push("relock".to_string()),
        |tx| calls.borrow_mut().push(format!("clear:{tx}")),
        || calls.borrow_mut().push("notify".to_string()),
    );

    assert_eq!(
        tail,
        NetworkOpsApplyBatchTail {
            cleared: 1,
            pending_transactions: 3,
            dispatch_state: NetworkOpsDispatchState::None,
        }
    );
    assert_eq!(owner.pending_transactions(), &[1, 2, 3]);
    assert!(owner.submit_held().is_empty());
    assert_eq!(owner.dispatch_state(), NetworkOpsDispatchState::None);
    assert_eq!(
        calls.into_inner(),
        vec![
            "relock".to_string(),
            "clear:20".to_string(),
            "notify".to_string(),
        ]
    );
}
